use super::packets::basic_item_stack_bytes;
use super::profile::uuid_without_dashes;
use crate::constants::{PLAYER_HOTBAR_SLOT_START, PLAYER_INVENTORY_SLOT_COUNT};
use crate::types::{OnlinePlayer, PersistedInventoryItem, PersistedPlayerData};
use anyhow::{Context, Result};
use std::io::ErrorKind;
use std::path::PathBuf;
use tokio::fs;

#[derive(serde::Deserialize)]
struct PersistedPlayerDataDisk {
    #[serde(flatten)]
    player: PersistedPlayerData,
    #[serde(default)]
    hotbar_items: Vec<Option<i32>>,
}

fn player_data_path(uuid: [u8; 16]) -> PathBuf {
    // Keep tiny reconnect state beside the repo so restarts reuse the same files.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("players")
        .join(format!("{}.json", uuid_without_dashes(uuid)))
}

pub(super) async fn load_player_data(uuid: [u8; 16]) -> Result<PersistedPlayerData> {
    let path = player_data_path(uuid);
    let bytes = match fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(PersistedPlayerData::default()),
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };
    let mut data: PersistedPlayerDataDisk = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    data.player.inventory_slots = normalize_inventory_slots(data.player.inventory_slots);

    // Older save files only tracked hotbar item IDs, so lift them into menu slots 36..44.
    for (offset, item_id) in data.hotbar_items.into_iter().take(9).enumerate() {
        let slot = PLAYER_HOTBAR_SLOT_START + offset;
        if data.player.inventory_slots[slot].is_none() {
            data.player.inventory_slots[slot] = item_id.map(|item_id| PersistedInventoryItem {
                item_id,
                count: 1,
                encoded: basic_item_stack_bytes(1, item_id),
            });
        }
    }

    Ok(data.player)
}

pub(super) async fn save_player_data(player: &OnlinePlayer) -> Result<()> {
    let path = player_data_path(player.uuid);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let data = PersistedPlayerData {
        x: player.x,
        y: player.y,
        z: player.z,
        y_rot: player.y_rot,
        x_rot: player.x_rot,
        on_ground: player.on_ground,
        held_slot: player.held_slot,
        inventory_slots: player.inventory_slots.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&data).context("failed to serialize player data")?;
    fs::write(&path, bytes)
        .await
        .with_context(|| format!("failed to write {}", path.display()))
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
        } else if let Some(item) = slot.as_mut()
            && item.encoded.is_empty()
        {
            item.encoded = basic_item_stack_bytes(item.count, item.item_id);
        }
    }
    slots
}
