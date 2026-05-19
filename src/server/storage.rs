use super::profile::uuid_without_dashes;
use crate::constants::PLAYER_INVENTORY_SLOT_COUNT;
use crate::types::{
    ONLINE_PLAYERS, OnlinePlayer, PersistedInventoryItem, PersistedPlayerData, WORLD_BLOCKS,
};
use anyhow::{Context, Result, ensure};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::fs;
use tokio::sync::Mutex;

static NEXT_SAVE_ID: AtomicU64 = AtomicU64::new(1);
static PLAYER_SAVE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static WORLD_SAVE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

// Persisted files start with a short magic string so bad files fail loudly.
const PLAYER_SAVE_MAGIC: &[u8; 8] = b"VCPPLYR2";
const LEGACY_PLAYER_SAVE_MAGIC: &[u8; 8] = b"VCPPLYR1";
const WORLD_SAVE_MAGIC: &[u8; 8] = b"VCPWRLD1";

#[derive(serde::Deserialize)]
struct LegacyPersistedPlayerData {
    x: f64,
    y: f64,
    z: f64,
    y_rot: f32,
    x_rot: f32,
    on_ground: bool,
    held_slot: i16,
    inventory_slots: Vec<Option<PersistedInventoryItem>>,
}

impl From<LegacyPersistedPlayerData> for PersistedPlayerData {
    fn from(data: LegacyPersistedPlayerData) -> Self {
        Self {
            x: data.x,
            y: data.y,
            z: data.z,
            y_rot: data.y_rot,
            x_rot: data.x_rot,
            on_ground: data.on_ground,
            held_slot: data.held_slot,
            inventory_slots: data.inventory_slots,
            game_mode: Default::default(),
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
struct PersistedWorldData {
    blocks: Vec<PersistedWorldBlock>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct PersistedWorldBlock {
    x: i32,
    y: i32,
    z: i32,
    state_id: i32,
}

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data")
}

fn player_data_path(uuid: [u8; 16]) -> PathBuf {
    // Keep reconnect state beside the repo so restarts reuse the same files.
    data_dir()
        .join("players")
        .join(format!("{}.bin", uuid_without_dashes(uuid)))
}

fn world_blocks_path() -> PathBuf {
    data_dir().join("world").join("blocks.bin")
}

pub(super) async fn load_world_blocks() -> Result<usize> {
    let path = world_blocks_path();
    let data = match fs::read(&path).await {
        Ok(bytes) => decode_binary_save(&path, &bytes, WORLD_SAVE_MAGIC)?,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };
    let blocks = world_blocks_from_disk(data);
    let loaded = blocks.len();

    let mut world = WORLD_BLOCKS.lock().await;
    *world = blocks;
    Ok(loaded)
}

pub(super) async fn save_world_blocks() -> Result<()> {
    let _save_guard = WORLD_SAVE_LOCK.lock().await;
    let path = world_blocks_path();
    let world = WORLD_BLOCKS.lock().await;
    let data = world_blocks_to_disk(&world);
    drop(world);

    write_binary_atomic(&path, WORLD_SAVE_MAGIC, &data).await
}

pub(super) async fn save_online_players() -> Result<()> {
    let players = ONLINE_PLAYERS.lock().await.clone();
    for player in players {
        save_player_data(&player).await?;
    }
    Ok(())
}

pub(super) async fn reset_persistent_data() -> Result<()> {
    let _player_guard = PLAYER_SAVE_LOCK.lock().await;
    let _world_guard = WORLD_SAVE_LOCK.lock().await;

    remove_dir_if_exists(&data_dir().join("players")).await?;
    remove_dir_if_exists(&data_dir().join("world")).await
}

pub(super) async fn load_player_data(uuid: [u8; 16]) -> Result<PersistedPlayerData> {
    let path = player_data_path(uuid);
    let mut data: PersistedPlayerData = match fs::read(&path).await {
        Ok(bytes) => decode_player_save(&path, &bytes)?,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(PersistedPlayerData::default()),
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };
    data.inventory_slots = normalize_inventory_slots(data.inventory_slots);

    Ok(data)
}

pub(super) async fn save_player_data(player: &OnlinePlayer) -> Result<()> {
    let _save_guard = PLAYER_SAVE_LOCK.lock().await;
    let path = player_data_path(player.uuid);
    let data = PersistedPlayerData {
        x: player.x,
        y: player.y,
        z: player.z,
        y_rot: player.y_rot,
        x_rot: player.x_rot,
        on_ground: player.on_ground,
        held_slot: player.held_slot,
        inventory_slots: player.inventory_slots.clone(),
        game_mode: player.game_mode,
    };

    write_binary_atomic(&path, PLAYER_SAVE_MAGIC, &data).await
}

async fn remove_dir_if_exists(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("failed to remove {}", path.display())),
    }
}

async fn write_binary_atomic<T: Serialize>(path: &Path, magic: &[u8], value: &T) -> Result<()> {
    let bytes = encode_binary_save(magic, value)?;
    write_bytes_atomic(path, &bytes).await
}

fn encode_binary_save<T: Serialize>(magic: &[u8], value: &T) -> Result<Vec<u8>> {
    let mut bytes = Vec::from(magic);
    let encoded = bincode::serialize(value).context("failed to encode persisted data")?;
    bytes.extend_from_slice(&encoded);
    Ok(bytes)
}

fn decode_binary_save<T: DeserializeOwned>(path: &Path, bytes: &[u8], magic: &[u8]) -> Result<T> {
    ensure!(
        bytes.starts_with(magic),
        "{} has wrong persistence magic",
        path.display()
    );
    bincode::deserialize(&bytes[magic.len()..])
        .with_context(|| format!("failed to decode {}", path.display()))
}

fn decode_player_save(path: &Path, bytes: &[u8]) -> Result<PersistedPlayerData> {
    if bytes.starts_with(PLAYER_SAVE_MAGIC) {
        return decode_binary_save(path, bytes, PLAYER_SAVE_MAGIC);
    }
    if bytes.starts_with(LEGACY_PLAYER_SAVE_MAGIC) {
        let legacy: LegacyPersistedPlayerData =
            decode_binary_save(path, bytes, LEGACY_PLAYER_SAVE_MAGIC)?;
        return Ok(legacy.into());
    }

    ensure!(false, "{} has wrong persistence magic", path.display());
    unreachable!()
}

async fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let tmp_path = temporary_path(path);
    fs::write(&tmp_path, bytes)
        .await
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;

    if let Err(err) = fs::rename(&tmp_path, path).await {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(err).with_context(|| format!("failed to replace {}", path.display()));
    }

    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let save_id = NEXT_SAVE_ID.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("save.bin");
    path.with_file_name(format!(".{file_name}.{save_id}.tmp"))
}

fn normalize_inventory_slots(
    mut slots: Vec<Option<PersistedInventoryItem>>,
) -> Vec<Option<PersistedInventoryItem>> {
    slots.resize(PLAYER_INVENTORY_SLOT_COUNT, None);
    slots.truncate(PLAYER_INVENTORY_SLOT_COUNT);
    slots[0] = None;
    for slot in slots.iter_mut().skip(1) {
        if matches!(slot, Some(item) if item.count <= 0) {
            *slot = None;
        }
    }
    slots
}

fn world_blocks_from_disk(data: PersistedWorldData) -> HashMap<(i32, i32, i32), i32> {
    data.blocks
        .into_iter()
        .map(|block| ((block.x, block.y, block.z), block.state_id))
        .collect()
}

fn world_blocks_to_disk(world: &HashMap<(i32, i32, i32), i32>) -> PersistedWorldData {
    let mut blocks: Vec<_> = world
        .iter()
        .map(|(&(x, y, z), &state_id)| PersistedWorldBlock { x, y, z, state_id })
        .collect();
    blocks.sort_by_key(|block| (block.x, block.y, block.z));
    PersistedWorldData { blocks }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_blocks_are_saved_in_position_order() {
        let mut world = HashMap::new();
        world.insert((4, 65, 1), 137);
        world.insert((1, 64, 4), 9);

        let data = world_blocks_to_disk(&world);

        assert_eq!(data.blocks[0].x, 1);
        assert_eq!(data.blocks[1].x, 4);
    }

    #[test]
    fn world_blocks_load_duplicate_positions_last_write_wins() {
        let data = PersistedWorldData {
            blocks: vec![
                PersistedWorldBlock {
                    x: 1,
                    y: 64,
                    z: 1,
                    state_id: 9,
                },
                PersistedWorldBlock {
                    x: 1,
                    y: 64,
                    z: 1,
                    state_id: 0,
                },
            ],
        };

        let world = world_blocks_from_disk(data);

        assert_eq!(world.get(&(1, 64, 1)), Some(&0));
    }

    #[test]
    fn binary_saves_carry_magic_header() {
        let data = PersistedWorldData {
            blocks: vec![PersistedWorldBlock {
                x: 1,
                y: 64,
                z: 1,
                state_id: 9,
            }],
        };

        let bytes = encode_binary_save(WORLD_SAVE_MAGIC, &data).unwrap();
        let decoded: PersistedWorldData =
            decode_binary_save(Path::new("blocks.bin"), &bytes, WORLD_SAVE_MAGIC).unwrap();

        assert!(bytes.starts_with(WORLD_SAVE_MAGIC));
        assert_eq!(decoded.blocks[0].state_id, 9);
    }
}
