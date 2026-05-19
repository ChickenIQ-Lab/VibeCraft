use crate::constants::*;
use crate::cursor::Cursor;
use crate::protocol::{pack_position, read_packet, write_packet, write_string, write_var_i32};
use crate::types::{
    BLOCK_ITEM_PLACEMENTS, BlockPlacement, BlockPlacementKind, Config, NEXT_ENTITY_ID,
    ONLINE_PLAYERS, OnlinePlayer, PersistedInventoryItem, PersistedPlayerData, ProfileProperty,
    WORLD_BLOCKS,
};
use anyhow::{Context, Result, bail};
use reqwest::{Client, StatusCode};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::{LazyLock, atomic::Ordering};
use tokio::fs;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::{Duration, interval};
use tracing::{debug, info, warn};

#[derive(serde::Deserialize)]
struct PersistedPlayerDataDisk {
    #[serde(flatten)]
    player: PersistedPlayerData,
    #[serde(default)]
    hotbar_items: Vec<Option<i32>>,
}

static PROFILE_HTTP_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .connect_timeout(Duration::from_millis(750))
        .timeout(Duration::from_secs(2))
        .user_agent("VibeCraft/0.1")
        .build()
        .expect("profile http client")
});

pub(crate) async fn handle_connection(mut stream: TcpStream, config: Config) -> Result<()> {
    let packet = read_packet(&mut stream).await?;
    let mut cursor = Cursor::new(&packet);
    let packet_id = cursor.read_var_i32()?;
    if packet_id != 0x00 {
        bail!("expected handshake packet, got packet id {packet_id}");
    }

    let client_protocol = cursor.read_var_i32()?;
    let server_addr = cursor.read_string()?;
    let server_port = cursor.read_u16()?;
    let next_state = cursor.read_var_i32()?;
    debug!(
        client_protocol,
        server_addr, server_port, next_state, "handshake"
    );

    // The handshake only chooses which protocol state should handle the next packet.
    match next_state {
        1 => handle_status(stream, config).await,
        2 => handle_login(stream, config).await,
        other => bail!("unsupported next state {other}"),
    }
}

async fn handle_status(mut stream: TcpStream, config: Config) -> Result<()> {
    let request = read_packet(&mut stream).await?;
    let mut cursor = Cursor::new(&request);
    let packet_id = cursor.read_var_i32()?;
    if packet_id != 0x00 {
        bail!("expected status request, got packet id {packet_id}");
    }

    let status = json!({
        "version": { "name": config.version_name, "protocol": config.protocol },
        "players": { "max": config.max_players, "online": 0, "sample": [] },
        "description": { "text": config.motd },
        "enforcesSecureChat": false,
        "previewsChat": false,
    });

    let mut response = Vec::new();
    write_var_i32(&mut response, 0x00);
    write_string(&mut response, &status.to_string())?;
    write_packet(&mut stream, &response).await?;

    match read_packet(&mut stream).await {
        Ok(ping) => {
            let mut cursor = Cursor::new(&ping);
            if cursor.read_var_i32()? == 0x01 {
                let payload = cursor.read_i64()?;
                let mut pong = Vec::new();
                write_var_i32(&mut pong, 0x01);
                pong.extend_from_slice(&payload.to_be_bytes());
                write_packet(&mut stream, &pong).await?;
            }
        }
        Err(err) => debug!(error = %err, "client skipped ping"),
    }

    Ok(())
}

async fn handle_login(mut stream: TcpStream, config: Config) -> Result<()> {
    let login_start = read_packet(&mut stream).await?;
    let mut cursor = Cursor::new(&login_start);
    let packet_id = cursor.read_var_i32()?;
    if packet_id != 0x00 {
        bail!("expected login start, got packet id {packet_id}");
    }

    let username = cursor.read_string()?;
    let uuid = cursor
        .read_uuid()
        .unwrap_or_else(|_| offline_uuid(&username));
    let mut saved_player = match load_player_data(uuid).await {
        Ok(player) => player,
        Err(err) => {
            warn!(player = %uuid_without_dashes(uuid), error = %err, "failed to load player data");
            PersistedPlayerData::default()
        }
    };
    saved_player.held_slot = saved_player.held_slot.clamp(0, 8);
    let chunk_x = (saved_player.x.floor() as i32).div_euclid(16);
    let chunk_z = (saved_player.z.floor() as i32).div_euclid(16);
    let profile_properties = fetch_profile_properties(&username, uuid).await;
    let entity_id = NEXT_ENTITY_ID.fetch_add(1, Ordering::Relaxed);
    info!(%username, "player logging in");

    send_login_success(&mut stream, uuid, &username, &profile_properties).await?;
    wait_for_packet_id(&mut stream, 0x03, "login acknowledged").await?;
    run_configuration(&mut stream, &config).await?;
    enter_world(&mut stream, entity_id, &saved_player).await?;

    let (reader, writer) = stream.into_split();
    let (sender, receiver) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        if let Err(err) = write_packets(writer, receiver).await {
            debug!(error = %err, "writer closed");
        }
    });

    let player = OnlinePlayer {
        entity_id,
        uuid,
        username,
        profile_properties,
        sender: sender.clone(),
        x: saved_player.x,
        y: saved_player.y,
        z: saved_player.z,
        y_rot: saved_player.y_rot,
        x_rot: saved_player.x_rot,
        on_ground: saved_player.on_ground,
        loaded_chunks: initial_chunk_set(chunk_x, chunk_z),
        held_slot: saved_player.held_slot,
        inventory_slots: saved_player.inventory_slots,
    };
    register_player(&player).await?;

    let result = keep_player_connected(reader, sender, uuid).await;
    unregister_player(player.uuid, player.entity_id).await?;
    result
}

async fn send_login_success(
    stream: &mut TcpStream,
    uuid: [u8; 16],
    username: &str,
    profile_properties: &[ProfileProperty],
) -> Result<()> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, 0x02);
    packet.extend_from_slice(&uuid);
    write_string(&mut packet, username)?;
    // Login success seeds the local client profile, including cape data.
    write_profile_properties(&mut packet, profile_properties)?;
    write_packet(stream, &packet).await
}

async fn fetch_profile_properties(username: &str, uuid: [u8; 16]) -> Vec<ProfileProperty> {
    match try_fetch_profile_properties(username, uuid).await {
        Ok(Some(properties)) => properties,
        Ok(None) => Vec::new(),
        Err(err) => {
            debug!(%username, error = %err, "failed to fetch profile properties");
            Vec::new()
        }
    }
}

async fn try_fetch_profile_properties(
    username: &str,
    uuid: [u8; 16],
) -> Result<Option<Vec<ProfileProperty>>> {
    // Mirror Mojang profile properties into player info so vanilla clients can resolve skins.
    if let Some(properties) = fetch_session_profile_properties(&uuid_without_dashes(uuid)).await? {
        return Ok(Some(properties));
    }

    let Some(profile_id) = fetch_profile_id_by_name(username).await? else {
        return Ok(None);
    };
    fetch_session_profile_properties(&profile_id).await
}

async fn fetch_profile_id_by_name(username: &str) -> Result<Option<String>> {
    let response = PROFILE_HTTP_CLIENT
        .get(format!(
            "https://api.mojang.com/users/profiles/minecraft/{username}"
        ))
        .send()
        .await?;

    match response.status() {
        StatusCode::NO_CONTENT | StatusCode::NOT_FOUND => return Ok(None),
        status if !status.is_success() => bail!("profile lookup failed with status {status}"),
        _ => {}
    }

    let body: serde_json::Value = response.json().await?;
    Ok(body
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned))
}

async fn fetch_session_profile_properties(
    profile_id: &str,
) -> Result<Option<Vec<ProfileProperty>>> {
    let response = PROFILE_HTTP_CLIENT
        .get(format!(
            "https://sessionserver.mojang.com/session/minecraft/profile/{profile_id}?unsigned=false"
        ))
        .send()
        .await?;

    match response.status() {
        StatusCode::NO_CONTENT | StatusCode::NOT_FOUND => return Ok(None),
        status if !status.is_success() => {
            bail!("session profile lookup failed with status {status}")
        }
        _ => {}
    }

    let body: serde_json::Value = response.json().await?;
    let Some(properties) = body.get("properties").and_then(serde_json::Value::as_array) else {
        return Ok(None);
    };

    let mut result = Vec::new();
    for property in properties {
        let Some(name) = property.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(value) = property.get("value").and_then(serde_json::Value::as_str) else {
            continue;
        };
        result.push(ProfileProperty {
            name: name.to_owned(),
            value: value.to_owned(),
            signature: property
                .get("signature")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
        });
    }

    Ok((!result.is_empty()).then_some(result))
}

fn uuid_without_dashes(uuid: [u8; 16]) -> String {
    let mut formatted = String::with_capacity(32);
    for byte in uuid {
        let _ = write!(&mut formatted, "{byte:02x}");
    }
    formatted
}

fn player_data_path(uuid: [u8; 16]) -> PathBuf {
    // Keep tiny reconnect state beside the repo so restarts reuse the same files.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("players")
        .join(format!("{}.json", uuid_without_dashes(uuid)))
}

async fn load_player_data(uuid: [u8; 16]) -> Result<PersistedPlayerData> {
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

async fn save_player_data(player: &OnlinePlayer) -> Result<()> {
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

fn write_profile_properties(out: &mut Vec<u8>, properties: &[ProfileProperty]) -> Result<()> {
    write_var_i32(out, properties.len() as i32);
    for property in properties {
        write_string(out, &property.name)?;
        write_string(out, &property.value)?;
        if let Some(signature) = &property.signature {
            out.push(1);
            write_string(out, signature)?;
        } else {
            out.push(0);
        }
    }
    Ok(())
}

async fn run_configuration(stream: &mut TcpStream, config: &Config) -> Result<()> {
    let _ = read_packet(stream).await;

    // Vanilla clients need these minimal configuration packets before play packets are valid.
    send_known_packs(stream, &config.version_name).await?;
    let _selected_packs = read_packet(stream).await?;
    send_required_registries(stream).await?;
    send_enabled_features(stream).await?;
    send_required_tags(stream).await?;
    send_finish_configuration(stream).await?;
    wait_for_packet_id(stream, 0x03, "finish configuration acknowledgement").await
}

async fn send_required_registries(stream: &mut TcpStream) -> Result<()> {
    // Send compact registries with just enough entries for a vanilla client to join.
    send_registry_data(stream, "minecraft:damage_type", DAMAGE_TYPES).await?;
    send_registry_data(stream, "minecraft:dimension_type", &["minecraft:overworld"]).await?;
    send_registry_data(stream, "minecraft:worldgen/biome", &["minecraft:plains"]).await?;
    send_registry_data(
        stream,
        "minecraft:timeline",
        &["minecraft:day", "minecraft:moon", "minecraft:early_game"],
    )
    .await?;
    send_registry_data(stream, "minecraft:world_clock", &["minecraft:overworld"]).await?;
    send_registry_data(
        stream,
        "minecraft:instrument",
        &[
            "minecraft:ponder_goat_horn",
            "minecraft:sing_goat_horn",
            "minecraft:seek_goat_horn",
            "minecraft:feel_goat_horn",
            "minecraft:admire_goat_horn",
            "minecraft:call_goat_horn",
            "minecraft:yearn_goat_horn",
            "minecraft:dream_goat_horn",
        ],
    )
    .await?;
    send_registry_data(stream, "minecraft:banner_pattern", BANNER_PATTERNS).await?;
    send_registry_data(stream, "minecraft:jukebox_song", JUKEBOX_SONGS).await?;
    send_registry_data(
        stream,
        "minecraft:cat_sound_variant",
        &["minecraft:classic", "minecraft:royal"],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:cat_variant",
        &[
            "minecraft:tabby",
            "minecraft:black",
            "minecraft:red",
            "minecraft:siamese",
            "minecraft:british_shorthair",
            "minecraft:calico",
            "minecraft:persian",
            "minecraft:ragdoll",
            "minecraft:white",
            "minecraft:jellie",
            "minecraft:all_black",
        ],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:chicken_sound_variant",
        &["minecraft:classic", "minecraft:picky"],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:chicken_variant",
        &["minecraft:temperate", "minecraft:warm", "minecraft:cold"],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:cow_sound_variant",
        &["minecraft:classic", "minecraft:moody"],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:cow_variant",
        &["minecraft:temperate", "minecraft:warm", "minecraft:cold"],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:frog_variant",
        &["minecraft:temperate", "minecraft:warm", "minecraft:cold"],
    )
    .await?;
    send_registry_data(stream, "minecraft:painting_variant", &["minecraft:kebab"]).await?;
    send_registry_data(
        stream,
        "minecraft:pig_sound_variant",
        &["minecraft:classic", "minecraft:big", "minecraft:mini"],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:pig_variant",
        &["minecraft:temperate", "minecraft:warm", "minecraft:cold"],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:wolf_sound_variant",
        &[
            "minecraft:classic",
            "minecraft:big",
            "minecraft:cute",
            "minecraft:grumpy",
            "minecraft:puglin",
            "minecraft:sad",
            "minecraft:angry",
        ],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:wolf_variant",
        &[
            "minecraft:pale",
            "minecraft:spotted",
            "minecraft:snowy",
            "minecraft:black",
            "minecraft:ashen",
            "minecraft:rusty",
            "minecraft:woods",
            "minecraft:chestnut",
            "minecraft:striped",
        ],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:zombie_nautilus_variant",
        &["minecraft:temperate"],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:trim_material",
        &[
            "minecraft:quartz",
            "minecraft:iron",
            "minecraft:netherite",
            "minecraft:redstone",
            "minecraft:copper",
            "minecraft:gold",
            "minecraft:emerald",
            "minecraft:diamond",
            "minecraft:lapis",
            "minecraft:amethyst",
            "minecraft:resin",
        ],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:trim_pattern",
        &[
            "minecraft:sentry",
            "minecraft:dune",
            "minecraft:coast",
            "minecraft:wild",
            "minecraft:ward",
            "minecraft:eye",
            "minecraft:vex",
            "minecraft:tide",
            "minecraft:snout",
            "minecraft:rib",
            "minecraft:spire",
            "minecraft:wayfinder",
            "minecraft:shaper",
            "minecraft:silence",
            "minecraft:raiser",
            "minecraft:host",
            "minecraft:flow",
            "minecraft:bolt",
        ],
    )
    .await
}

async fn send_registry_data(
    stream: &mut TcpStream,
    registry: &str,
    entries: &[&str],
) -> Result<()> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, 0x07);
    write_string(&mut packet, registry)?;
    write_var_i32(&mut packet, entries.len() as i32);
    for entry in entries {
        write_string(&mut packet, entry)?;
        // False means the entry uses no inline NBT payload.
        packet.push(0);
    }
    write_packet(stream, &packet).await
}

async fn send_known_packs(stream: &mut TcpStream, version: &str) -> Result<()> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, 0x0e);
    write_var_i32(&mut packet, 1);
    write_string(&mut packet, "minecraft")?;
    write_string(&mut packet, "core")?;
    write_string(&mut packet, version)?;
    write_packet(stream, &packet).await
}

async fn send_enabled_features(stream: &mut TcpStream) -> Result<()> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, 0x0c);
    write_var_i32(&mut packet, 1);
    write_string(&mut packet, "minecraft:vanilla")?;
    write_packet(stream, &packet).await
}

async fn send_required_tags(stream: &mut TcpStream) -> Result<()> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, 0x0d);
    write_var_i32(&mut packet, 3);
    write_string(&mut packet, "minecraft:damage_type")?;
    write_var_i32(&mut packet, 3);
    write_tag_ids(
        &mut packet,
        DAMAGE_TYPES,
        "minecraft:is_fire",
        &[
            "minecraft:in_fire",
            "minecraft:campfire",
            "minecraft:on_fire",
            "minecraft:lava",
            "minecraft:hot_floor",
            "minecraft:unattributed_fireball",
            "minecraft:fireball",
        ],
    )?;
    write_tag_ids(
        &mut packet,
        DAMAGE_TYPES,
        "minecraft:is_explosion",
        &[
            "minecraft:fireworks",
            "minecraft:explosion",
            "minecraft:player_explosion",
            "minecraft:bad_respawn_point",
        ],
    )?;
    write_tag_ids(
        &mut packet,
        DAMAGE_TYPES,
        "minecraft:bypasses_shield",
        &[
            "minecraft:on_fire",
            "minecraft:in_wall",
            "minecraft:cramming",
            "minecraft:drown",
            "minecraft:fly_into_wall",
            "minecraft:generic",
            "minecraft:wither",
            "minecraft:dragon_breath",
            "minecraft:starve",
            "minecraft:fall",
            "minecraft:ender_pearl",
            "minecraft:freeze",
            "minecraft:stalagmite",
            "minecraft:magic",
            "minecraft:indirect_magic",
            "minecraft:out_of_world",
            "minecraft:generic_kill",
            "minecraft:sonic_boom",
            "minecraft:outside_border",
            "minecraft:cactus",
            "minecraft:campfire",
            "minecraft:dry_out",
            "minecraft:falling_anvil",
            "minecraft:falling_stalactite",
            "minecraft:hot_floor",
            "minecraft:in_fire",
            "minecraft:lava",
            "minecraft:lightning_bolt",
            "minecraft:sweet_berry_bush",
        ],
    )?;

    write_string(&mut packet, "minecraft:timeline")?;
    write_var_i32(&mut packet, 1);
    write_string(&mut packet, "minecraft:in_overworld")?;
    write_var_i32(&mut packet, 3);
    write_var_i32(&mut packet, 0);
    write_var_i32(&mut packet, 1);
    write_var_i32(&mut packet, 2);

    write_string(&mut packet, "minecraft:banner_pattern")?;
    write_var_i32(&mut packet, 10);
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/flower",
        &["minecraft:flower"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/creeper",
        &["minecraft:creeper"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/skull",
        &["minecraft:skull"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/mojang",
        &["minecraft:mojang"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/globe",
        &["minecraft:globe"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/piglin",
        &["minecraft:piglin"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/flow",
        &["minecraft:flow"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/guster",
        &["minecraft:guster"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/field_masoned",
        &["minecraft:bricks"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/bordure_indented",
        &["minecraft:curly_border"],
    )?;
    write_packet(stream, &packet).await
}

fn write_tag_ids(out: &mut Vec<u8>, registry: &[&str], tag: &str, values: &[&str]) -> Result<()> {
    write_string(out, tag)?;
    write_var_i32(out, values.len() as i32);
    for value in values {
        let index = registry
            .iter()
            .position(|entry| entry == value)
            .with_context(|| format!("unknown registry entry {value} for tag {tag}"))?;
        write_var_i32(out, index as i32);
    }
    Ok(())
}

async fn send_finish_configuration(stream: &mut TcpStream) -> Result<()> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, 0x03);
    write_packet(stream, &packet).await
}

async fn enter_world(
    stream: &mut TcpStream,
    entity_id: i32,
    player: &PersistedPlayerData,
) -> Result<()> {
    let chunk_x = (player.x.floor() as i32).div_euclid(16);
    let chunk_z = (player.z.floor() as i32).div_euclid(16);

    // Send the absolute spawn position before the heavy chunk batch so the
    // client does not spend its first seconds at the void floor waiting for
    // terrain packets to finish.
    send_play_login(stream, entity_id).await?;
    send_level_chunks_load_start(stream).await?;
    send_chunk_cache_center(stream, chunk_x, chunk_z).await?;
    send_chunk_cache_radius(stream, VIEW_DISTANCE).await?;
    send_default_spawn(stream).await?;
    send_player_position(stream, player).await?;
    send_player_inventory(stream, player).await?;
    send_superflat_chunks(stream, chunk_x, chunk_z).await
}

async fn send_play_login(stream: &mut TcpStream, entity_id: i32) -> Result<()> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_LOGIN_PACKET_ID);
    packet.extend_from_slice(&entity_id.to_be_bytes());
    packet.push(0);
    write_var_i32(&mut packet, 1);
    write_string(&mut packet, DIMENSION)?;
    write_var_i32(&mut packet, 20);
    write_var_i32(&mut packet, VIEW_DISTANCE);
    write_var_i32(&mut packet, VIEW_DISTANCE);
    packet.push(0);
    packet.push(1);
    packet.push(0);
    write_var_i32(&mut packet, 0);
    write_string(&mut packet, DIMENSION)?;
    packet.extend_from_slice(&0i64.to_be_bytes());
    packet.push(1);
    packet.push(255);
    packet.push(0);
    packet.push(1);
    packet.push(0);
    write_var_i32(&mut packet, 0);
    write_var_i32(&mut packet, 63);
    packet.push(0);
    write_packet(stream, &packet).await
}

async fn send_chunk_cache_center(stream: &mut TcpStream, x: i32, z: i32) -> Result<()> {
    write_packet(stream, &chunk_cache_center_packet(x, z)).await
}

async fn send_chunk_cache_radius(stream: &mut TcpStream, radius: i32) -> Result<()> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_SET_CHUNK_CACHE_RADIUS_PACKET_ID);
    write_var_i32(&mut packet, radius);
    write_packet(stream, &packet).await
}

async fn send_level_chunks_load_start(stream: &mut TcpStream) -> Result<()> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_GAME_EVENT_PACKET_ID);
    packet.push(13);
    packet.extend_from_slice(&0f32.to_be_bytes());
    write_packet(stream, &packet).await
}

async fn send_default_spawn(stream: &mut TcpStream) -> Result<()> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_SET_DEFAULT_SPAWN_POSITION_PACKET_ID);
    write_string(&mut packet, DIMENSION)?;
    packet.extend_from_slice(&pack_position(0, SPAWN_Y as i32, 0).to_be_bytes());
    packet.extend_from_slice(&0f32.to_be_bytes());
    packet.extend_from_slice(&0f32.to_be_bytes());
    write_packet(stream, &packet).await
}

async fn send_player_position(stream: &mut TcpStream, player: &PersistedPlayerData) -> Result<()> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_PLAYER_POSITION_PACKET_ID);
    write_var_i32(&mut packet, 1);
    packet.extend_from_slice(&player.x.to_be_bytes());
    packet.extend_from_slice(&player.y.to_be_bytes());
    packet.extend_from_slice(&player.z.to_be_bytes());
    packet.extend_from_slice(&0.0f64.to_be_bytes());
    packet.extend_from_slice(&0.0f64.to_be_bytes());
    packet.extend_from_slice(&0.0f64.to_be_bytes());
    packet.extend_from_slice(&player.y_rot.to_be_bytes());
    packet.extend_from_slice(&player.x_rot.to_be_bytes());
    packet.extend_from_slice(&0i32.to_be_bytes());
    write_packet(stream, &packet).await
}

async fn send_player_inventory(stream: &mut TcpStream, player: &PersistedPlayerData) -> Result<()> {
    // Creative slot updates use inventory-menu slots 1..45, so reconnects replay that layout.
    for (slot, item) in player.inventory_slots.iter().enumerate().skip(1) {
        write_packet(
            stream,
            &container_set_slot_packet(slot as i16, item.as_ref()),
        )
        .await?;
    }
    write_packet(stream, &set_held_slot_packet(player.held_slot)).await
}

async fn send_superflat_chunks(stream: &mut TcpStream, center_x: i32, center_z: i32) -> Result<()> {
    // Batch packets tell the client how many chunks belong to this initial view.
    write_packet(stream, &chunk_batch_start_packet()).await?;
    let mut sent = 0;
    for z in center_z - VIEW_DISTANCE..=center_z + VIEW_DISTANCE {
        for x in center_x - VIEW_DISTANCE..=center_x + VIEW_DISTANCE {
            write_packet(stream, &flat_chunk_packet(x, z).await).await?;
            sent += 1;
        }
    }
    write_packet(stream, &chunk_batch_finished_packet(sent)).await?;
    Ok(())
}

fn initial_chunk_set(center_x: i32, center_z: i32) -> HashSet<(i32, i32)> {
    let mut chunks = HashSet::new();
    for z in center_z - VIEW_DISTANCE..=center_z + VIEW_DISTANCE {
        for x in center_x - VIEW_DISTANCE..=center_x + VIEW_DISTANCE {
            chunks.insert((x, z));
        }
    }
    chunks
}

async fn flat_chunk_packet(x: i32, z: i32) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_LEVEL_CHUNK_WITH_LIGHT_PACKET_ID);
    packet.extend_from_slice(&x.to_be_bytes());
    packet.extend_from_slice(&z.to_be_bytes());
    write_heightmaps(&mut packet);
    let data = flat_chunk_data(x, z).await;
    write_var_i32(&mut packet, data.len() as i32);
    packet.extend_from_slice(&data);
    write_var_i32(&mut packet, 0);
    write_full_bright_light(&mut packet);
    packet
}

async fn flat_chunk_data(chunk_x: i32, chunk_z: i32) -> Vec<u8> {
    let mut data = Vec::new();
    let world_blocks = WORLD_BLOCKS.lock().await;

    // Sections -4..20 cover the vanilla overworld height range for this protocol.
    for section_y in -4..20 {
        let mut values = [0i32; 4096];

        if section_y == 3 {
            // Base terrain sits just below spawn: air, bedrock, then dirt.
            for dy in 0..16 {
                let state = if dy < 12 {
                    0
                } else if dy == 12 {
                    BLOCK_STATE_BEDROCK
                } else {
                    BLOCK_STATE_DIRT
                };
                for dz in 0..16 {
                    for dx in 0..16 {
                        values[(dy * 16 + dz) * 16 + dx] = state;
                    }
                }
            }
        } else if section_y == 4 {
            for dz in 0..16 {
                for dx in 0..16 {
                    values[(0 * 16 + dz) * 16 + dx] = BLOCK_STATE_GRASS_BLOCK;
                }
            }
            if chunk_x == 0 && chunk_z == 0 {
                // A small tree at spawn makes fresh chunks visibly non-empty.
                for dy in 1..=4 {
                    values[(dy * 16 + 4) * 16 + 4] = BLOCK_STATE_OAK_LOG;
                }
                for dy in 3..=5 {
                    for dz in 2..=6 {
                        for dx in 2..=6 {
                            if dx == 4 && dz == 4 && dy <= 4 {
                                continue;
                            }
                            values[(dy * 16 + dz) * 16 + dx] = BLOCK_STATE_OAK_LEAVES;
                        }
                    }
                }
                for dx in 3..=5 {
                    for dz in 3..=5 {
                        values[(6 * 16 + dz) * 16 + dx] = BLOCK_STATE_OAK_LEAVES;
                    }
                }
            }
        }

        // Player edits override generated terrain inside this chunk section.
        let base_y = section_y * 16;
        let base_x = chunk_x * 16;
        let base_z = chunk_z * 16;

        for dy in 0..16 {
            let global_y = base_y + dy;
            for dz in 0..16 {
                let global_z = base_z + dz;
                for dx in 0..16 {
                    let global_x = base_x + dx;
                    if let Some(&state_id) = world_blocks.get(&(global_x, global_y, global_z)) {
                        values[(dy as usize * 16 + dz as usize) * 16 + dx as usize] = state_id;
                    }
                }
            }
        }

        write_section(&mut data, &values);
    }
    data
}

fn write_section(data: &mut Vec<u8>, values: &[i32; 4096]) {
    let mut non_air = 0;
    let mut palette = Vec::new();
    let mut indices = [0u16; 4096];

    // Chunk sections store a local palette plus packed indices into that palette.
    for i in 0..4096 {
        let state = values[i];
        if state != 0 {
            non_air += 1;
        }
        if let Some(pos) = palette.iter().position(|&x| x == state) {
            indices[i] = pos as u16;
        } else {
            indices[i] = palette.len() as u16;
            palette.push(state);
        }
    }

    let non_air_count = non_air as i16;
    data.extend_from_slice(&non_air_count.to_be_bytes());
    // This toy world does not emit fluid blocks yet, but the protocol still expects the count.
    data.extend_from_slice(&0i16.to_be_bytes());

    // Block states container.
    if palette.len() == 1 {
        data.push(0); // 0 bits per block
        write_var_i32(data, palette[0]);
    } else {
        let mut bits_per_block = 4;
        while (1 << bits_per_block) < palette.len() {
            bits_per_block += 1;
        }
        if bits_per_block < 4 {
            bits_per_block = 4;
        }

        if bits_per_block > 8 {
            bits_per_block = 15;
            data.push(15);
        } else {
            data.push(bits_per_block as u8);
            write_var_i32(data, palette.len() as i32);
            for &state in &palette {
                write_var_i32(data, state);
            }
        }

        let longs = pack_indices(&indices, bits_per_block as u8, 4096);
        for value in longs {
            data.extend_from_slice(&value.to_be_bytes());
        }
    }

    // Biomes container: one plains biome value covers the whole section.
    data.push(0);
    write_var_i32(data, 0);
}

fn pack_indices(indices: &[u16], bits_per_block: u8, count: usize) -> Vec<u64> {
    if bits_per_block == 0 {
        return Vec::new();
    }
    let blocks_per_long = 64 / bits_per_block as usize;
    let num_longs = (count + blocks_per_long - 1) / blocks_per_long;
    let mut packed = vec![0u64; num_longs];

    // Minecraft packs indices little-endian within each u64.
    for i in 0..count {
        let long_index = i / blocks_per_long;
        let bit_offset = (i % blocks_per_long) * bits_per_block as usize;
        packed[long_index] |= (indices[i] as u64) << bit_offset;
    }
    packed
}

fn write_heightmaps(out: &mut Vec<u8>) {
    // Two heightmaps, 256 columns each, with 9-bit heights packed into 37 longs.
    write_var_i32(out, 2);
    write_var_i32(out, 4);
    write_var_i32(out, 37);
    for value in packed_heightmap(65) {
        out.extend_from_slice(&value.to_be_bytes());
    }
    write_var_i32(out, 1);
    write_var_i32(out, 37);
    for value in packed_heightmap(65) {
        out.extend_from_slice(&value.to_be_bytes());
    }
}

fn packed_heightmap(height: u64) -> [u64; 37] {
    let mut values = [0u64; 37];
    for index in 0..256 {
        let bit_index = index * 9;
        let long_index = bit_index / 64;
        let bit_offset = bit_index % 64;
        values[long_index] |= height << bit_offset;
        if bit_offset > 55 {
            values[long_index + 1] |= height >> (64 - bit_offset);
        }
    }
    values
}

fn write_full_bright_light(out: &mut Vec<u8>) {
    // Treat every light section as full-bright so the server does no lighting work.
    write_bitset(out, &[0x03ff_ffff]);
    write_bitset(out, &[]);
    write_bitset(out, &[]);
    write_bitset(out, &[0x03ff_ffff]);
    write_var_i32(out, 26);
    for _ in 0..26 {
        write_var_i32(out, 2048);
        out.extend(std::iter::repeat_n(0xff, 2048));
    }
    write_var_i32(out, 0);
}

fn write_bitset(out: &mut Vec<u8>, longs: &[i64]) {
    write_var_i32(out, longs.len() as i32);
    for value in longs {
        out.extend_from_slice(&value.to_be_bytes());
    }
}

async fn wait_for_packet_id(stream: &mut TcpStream, expected: i32, name: &str) -> Result<()> {
    loop {
        let packet = read_packet(stream).await?;
        let mut cursor = Cursor::new(&packet);
        let packet_id = cursor.read_var_i32()?;
        if packet_id == expected {
            return Ok(());
        }
        debug!(packet_id, expected, name, "ignoring packet while waiting");
    }
}

async fn keep_player_connected<R: AsyncRead + Unpin>(
    mut stream: R,
    sender: mpsc::UnboundedSender<Vec<u8>>,
    uuid: [u8; 16],
) -> Result<()> {
    // Keep one heartbeat timer alive so frequent movement packets do not reset it.
    let mut keep_alive = interval(Duration::from_secs(10));
    keep_alive.tick().await;

    loop {
        tokio::select! {
            result = read_packet(&mut stream) => {
                let packet = result?;
                let mut cursor = Cursor::new(&packet);
                let packet_id = cursor.read_var_i32()?;
                handle_play_packet(packet_id, &mut cursor, uuid).await?;
                debug!(packet_id, "play packet received");
            }
            _ = keep_alive.tick() => {
                let _ = sender.send(keep_alive_packet());
            }
        }
    }
}

async fn handle_play_packet(
    cursor_packet_id: i32,
    cursor: &mut Cursor<'_>,
    uuid: [u8; 16],
) -> Result<()> {
    // Only packets that change this toy world are decoded; the rest are ignored.
    match cursor_packet_id {
        SERVERBOUND_SET_CARRIED_ITEM_PACKET_ID => {
            update_held_slot(uuid, cursor.read_i16()?).await?
        }
        SERVERBOUND_SET_CREATIVE_MODE_SLOT_PACKET_ID => {
            let slot = cursor.read_i16()?;
            let encoded = cursor.remaining().to_vec();
            let count = cursor.read_var_i32()?;
            let item = if count > 0 {
                Some(PersistedInventoryItem {
                    item_id: cursor.read_var_i32()?,
                    count,
                    encoded,
                })
            } else {
                None
            };
            update_inventory_slot(uuid, slot, item).await?;
        }
        SERVERBOUND_PLAYER_ACTION_PACKET_ID => {
            let action = cursor.read_var_i32()?;
            let pos = cursor.read_block_pos()?;
            let _direction = cursor.read_u8()?;
            let _sequence = cursor.read_var_i32()?;
            match action {
                0 | 2 => set_world_block(pos, 0).await?,
                // This action swaps the selected hotbar slot with the offhand slot.
                6 => swap_held_with_offhand(uuid).await?,
                _ => {}
            }
        }
        SERVERBOUND_USE_ITEM_ON_PACKET_ID => {
            let hand = cursor.read_var_i32()?;
            let pos = cursor.read_block_pos()?;
            let face = cursor.read_var_i32()?;
            let _click_x = cursor.read_f32()?;
            let _click_y = cursor.read_f32()?;
            let _click_z = cursor.read_f32()?;
            let _inside = cursor.read_bool()?;
            let _border = cursor.read_bool()?;
            let _sequence = cursor.read_var_i32()?;
            if !interact_with_block(uuid, pos).await? {
                let (dx, dy, dz) = direction_offset(face);
                place_hand_block(uuid, hand, (pos.0 + dx, pos.1 + dy, pos.2 + dz)).await?;
            }
        }
        SERVERBOUND_MOVE_PLAYER_POS_PACKET_ID => {
            let x = cursor.read_f64()?;
            let y = cursor.read_f64()?;
            let z = cursor.read_f64()?;
            let flags = cursor.read_u8()?;
            update_player_state(uuid, Some(x), Some(y), Some(z), None, None, flags & 1 != 0)
                .await?;
        }
        SERVERBOUND_MOVE_PLAYER_POS_ROT_PACKET_ID => {
            let x = cursor.read_f64()?;
            let y = cursor.read_f64()?;
            let z = cursor.read_f64()?;
            let y_rot = cursor.read_f32()?;
            let x_rot = cursor.read_f32()?;
            let flags = cursor.read_u8()?;
            update_player_state(
                uuid,
                Some(x),
                Some(y),
                Some(z),
                Some(y_rot),
                Some(x_rot),
                flags & 1 != 0,
            )
            .await?;
        }
        SERVERBOUND_MOVE_PLAYER_ROT_PACKET_ID => {
            let y_rot = cursor.read_f32()?;
            let x_rot = cursor.read_f32()?;
            let flags = cursor.read_u8()?;
            update_player_state(
                uuid,
                None,
                None,
                None,
                Some(y_rot),
                Some(x_rot),
                flags & 1 != 0,
            )
            .await?;
        }
        SERVERBOUND_MOVE_PLAYER_STATUS_ONLY_PACKET_ID => {
            let flags = cursor.read_u8()?;
            update_player_state(uuid, None, None, None, None, None, flags & 1 != 0).await?;
        }
        _ => {}
    }
    Ok(())
}

fn direction_offset(direction: i32) -> (i32, i32, i32) {
    match direction {
        0 => (0, -1, 0),
        1 => (0, 1, 0),
        2 => (0, 0, -1),
        3 => (0, 0, 1),
        4 => (-1, 0, 0),
        5 => (1, 0, 0),
        _ => (0, 1, 0),
    }
}

fn keep_alive_packet() -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_KEEP_ALIVE_PACKET_ID);
    packet.extend_from_slice(&0i64.to_be_bytes());
    packet
}

fn player_skin_parts_mask() -> u8 {
    // Turn on cape and the usual outer layers until we track client preferences.
    0x7f
}

fn player_entity_metadata_packet(player: &OnlinePlayer) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_SET_ENTITY_DATA_PACKET_ID);
    write_var_i32(&mut packet, player.entity_id);

    // Avatar metadata index 16 is the displayed skin parts bitmask, serialized as BYTE.
    packet.push(16);
    write_var_i32(&mut packet, 0);
    packet.push(player_skin_parts_mask());
    packet.push(0xff);
    packet
}

async fn write_packets<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut receiver: mpsc::UnboundedReceiver<Vec<u8>>,
) -> Result<()> {
    while let Some(packet) = receiver.recv().await {
        write_packet(&mut writer, &packet).await?;
    }
    Ok(())
}

async fn register_player(player: &OnlinePlayer) -> Result<()> {
    let mut online = ONLINE_PLAYERS.lock().await;
    let others = online.clone();
    online.push(player.clone());
    drop(online);

    let self_metadata = player_entity_metadata_packet(player);
    let _ = player.sender.send(self_metadata.clone());

    // The joiner needs all existing players; existing players need only the joiner.
    let mut everyone = others.clone();
    everyone.push(player.clone());
    let _ = player.sender.send(player_info_update_packet(&everyone)?);
    for other in &others {
        let _ = player.sender.send(add_player_entity_packet(other));
        let _ = player.sender.send(player_entity_metadata_packet(other));
        let _ = other
            .sender
            .send(player_info_update_packet(std::slice::from_ref(player))?);
        let _ = other.sender.send(add_player_entity_packet(player));
        let _ = other.sender.send(self_metadata.clone());
    }
    Ok(())
}

async fn unregister_player(uuid: [u8; 16], entity_id: i32) -> Result<()> {
    let mut online = ONLINE_PLAYERS.lock().await;
    let player_to_save = online.iter().find(|player| player.uuid == uuid).cloned();
    online.retain(|player| player.uuid != uuid);
    let remaining = online.clone();
    drop(online);

    if let Some(player) = player_to_save
        && let Err(err) = save_player_data(&player).await
    {
        warn!(player = %uuid_without_dashes(uuid), error = %err, "failed to save player data");
    }

    let remove_info = player_info_remove_packet(uuid);
    let remove_entity = remove_entities_packet(entity_id);
    for player in remaining {
        let _ = player.sender.send(remove_info.clone());
        let _ = player.sender.send(remove_entity.clone());
    }
    Ok(())
}

fn player_info_update_packet(players: &[OnlinePlayer]) -> Result<Vec<u8>> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_PLAYER_INFO_UPDATE_PACKET_ID);
    packet.push(0xff);
    write_var_i32(&mut packet, players.len() as i32);
    for player in players {
        packet.extend_from_slice(&player.uuid);
        write_string(&mut packet, &player.username)?;
        write_profile_properties(&mut packet, &player.profile_properties)?;
        packet.push(0);
        write_var_i32(&mut packet, 1);
        packet.push(1);
        write_var_i32(&mut packet, 0);
        packet.push(0);
        write_var_i32(&mut packet, 0);
        packet.push(1);
    }
    Ok(packet)
}

fn add_player_entity_packet(player: &OnlinePlayer) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_ADD_ENTITY_PACKET_ID);
    write_var_i32(&mut packet, player.entity_id);
    packet.extend_from_slice(&player.uuid);
    write_var_i32(&mut packet, PLAYER_ENTITY_TYPE_ID);
    packet.extend_from_slice(&player.x.to_be_bytes());
    packet.extend_from_slice(&player.y.to_be_bytes());
    packet.extend_from_slice(&player.z.to_be_bytes());
    packet.push(0);
    packet.push(pack_degrees(player.x_rot));
    packet.push(pack_degrees(player.y_rot));
    packet.push(pack_degrees(player.y_rot));
    write_var_i32(&mut packet, 0);
    packet
}

fn player_info_remove_packet(uuid: [u8; 16]) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_PLAYER_INFO_REMOVE_PACKET_ID);
    write_var_i32(&mut packet, 1);
    packet.extend_from_slice(&uuid);
    packet
}

fn container_set_slot_packet(slot: i16, item: Option<&PersistedInventoryItem>) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_CONTAINER_SET_SLOT_PACKET_ID);
    write_var_i32(&mut packet, 0);
    write_var_i32(&mut packet, 0);
    packet.extend_from_slice(&slot.to_be_bytes());
    write_optional_item_stack(&mut packet, item);
    packet
}

fn set_held_slot_packet(slot: i16) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_SET_HELD_SLOT_PACKET_ID);
    write_var_i32(&mut packet, slot as i32);
    packet
}

fn write_optional_item_stack(packet: &mut Vec<u8>, item: Option<&PersistedInventoryItem>) {
    match item {
        Some(item) => {
            if item.encoded.is_empty() {
                packet.extend_from_slice(&basic_item_stack_bytes(item.count, item.item_id));
            } else {
                packet.extend_from_slice(&item.encoded);
            }
        }
        None => write_var_i32(packet, 0),
    }
}

fn basic_item_stack_bytes(count: i32, item_id: i32) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, count);
    write_var_i32(&mut packet, item_id);
    write_var_i32(&mut packet, 0);
    write_var_i32(&mut packet, 0);
    packet
}

fn remove_entities_packet(entity_id: i32) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_REMOVE_ENTITIES_PACKET_ID);
    write_var_i32(&mut packet, 1);
    write_var_i32(&mut packet, entity_id);
    packet
}

fn chunk_batch_start_packet() -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, CHUNK_BATCH_START_PACKET_ID);
    packet
}

fn chunk_batch_finished_packet(batch_size: i32) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, CHUNK_BATCH_FINISHED_PACKET_ID);
    write_var_i32(&mut packet, batch_size);
    packet
}

fn chunk_cache_center_packet(x: i32, z: i32) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_SET_CHUNK_CACHE_CENTER_PACKET_ID);
    write_var_i32(&mut packet, x);
    write_var_i32(&mut packet, z);
    packet
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_chunk_section_writes_both_counts() {
        let mut data = Vec::new();
        let values = [0; 4096];

        write_section(&mut data, &values);

        assert_eq!(&data[..4], &[0, 0, 0, 0]);
        assert_eq!(data.len(), 8);
    }

    #[test]
    fn chunk_section_writes_non_air_then_fluid_counts() {
        let mut data = Vec::new();
        let mut values = [0; 4096];
        values[0] = BLOCK_STATE_GRASS_BLOCK;

        write_section(&mut data, &values);

        assert_eq!(&data[..2], &1i16.to_be_bytes());
        assert_eq!(&data[2..4], &0i16.to_be_bytes());
    }
}

async fn update_player_state(
    uuid: [u8; 16],
    x: Option<f64>,
    y: Option<f64>,
    z: Option<f64>,
    y_rot: Option<f32>,
    x_rot: Option<f32>,
    on_ground: bool,
) -> Result<()> {
    let mut online = ONLINE_PLAYERS.lock().await;
    let Some(index) = online.iter().position(|player| player.uuid == uuid) else {
        return Ok(());
    };
    let old_chunk_x = (online[index].x.floor() as i32).div_euclid(16);
    let old_chunk_z = (online[index].z.floor() as i32).div_euclid(16);
    if let Some(x) = x {
        online[index].x = x;
    }
    if let Some(y) = y {
        online[index].y = y;
    }
    if let Some(z) = z {
        online[index].z = z;
    }
    if let Some(y_rot) = y_rot {
        online[index].y_rot = y_rot;
    }
    if let Some(x_rot) = x_rot {
        online[index].x_rot = x_rot;
    }
    online[index].on_ground = on_ground;
    let new_chunk_x = (online[index].x.floor() as i32).div_euclid(16);
    let new_chunk_z = (online[index].z.floor() as i32).div_euclid(16);

    let mut self_packets = Vec::new();
    if old_chunk_x != new_chunk_x || old_chunk_z != new_chunk_z {
        // Crossing a chunk border moves the client cache window and streams any new chunks.
        self_packets.push(chunk_cache_center_packet(new_chunk_x, new_chunk_z));
        let mut fresh = Vec::new();
        for z in new_chunk_z - VIEW_DISTANCE..=new_chunk_z + VIEW_DISTANCE {
            for x in new_chunk_x - VIEW_DISTANCE..=new_chunk_x + VIEW_DISTANCE {
                if online[index].loaded_chunks.insert((x, z)) {
                    fresh.push((x, z));
                }
            }
        }
        if !fresh.is_empty() {
            self_packets.push(chunk_batch_start_packet());
            for &(x, z) in &fresh {
                self_packets.push(flat_chunk_packet(x, z).await);
            }
            self_packets.push(chunk_batch_finished_packet((self_packets.len() - 2) as i32));
        }
    }

    let packet = entity_position_sync_packet(&online[index]);
    let head_packet = rotate_head_packet(&online[index]);
    let entity_id = online[index].entity_id;
    let self_sender = online[index].sender.clone();
    // Position packets go to other players only; this player already authored the move.
    for player in online.iter() {
        if player.entity_id == entity_id {
            continue;
        }
        let _ = player.sender.send(packet.clone());
        let _ = player.sender.send(head_packet.clone());
    }
    drop(online);
    for packet in self_packets {
        let _ = self_sender.send(packet);
    }
    Ok(())
}

async fn update_held_slot(uuid: [u8; 16], slot: i16) -> Result<()> {
    let mut online = ONLINE_PLAYERS.lock().await;
    if let Some(player) = online.iter_mut().find(|player| player.uuid == uuid)
        && (0..9).contains(&slot)
    {
        player.held_slot = slot;
    }
    Ok(())
}

async fn update_inventory_slot(
    uuid: [u8; 16],
    slot: i16,
    item: Option<PersistedInventoryItem>,
) -> Result<()> {
    let mut online = ONLINE_PLAYERS.lock().await;
    if let Some(player) = online.iter_mut().find(|player| player.uuid == uuid)
        && slot > 0
        && let Some(saved_slot) = player.inventory_slots.get_mut(slot as usize)
    {
        *saved_slot = item;
    }
    Ok(())
}

async fn swap_held_with_offhand(uuid: [u8; 16]) -> Result<()> {
    let mut online = ONLINE_PLAYERS.lock().await;
    let Some(player) = online.iter_mut().find(|player| player.uuid == uuid) else {
        return Ok(());
    };

    let held_slot = PLAYER_HOTBAR_SLOT_START + player.held_slot as usize;
    let Some(held_item) = player.inventory_slots.get(held_slot).cloned() else {
        return Ok(());
    };
    let Some(offhand_item) = player.inventory_slots.get(PLAYER_OFFHAND_SLOT).cloned() else {
        return Ok(());
    };

    player.inventory_slots[held_slot] = offhand_item;
    player.inventory_slots[PLAYER_OFFHAND_SLOT] = held_item;
    let sender = player.sender.clone();
    let held_after = player.inventory_slots[held_slot].clone();
    let offhand_after = player.inventory_slots[PLAYER_OFFHAND_SLOT].clone();
    drop(online);

    let _ = sender.send(container_set_slot_packet(
        held_slot as i16,
        held_after.as_ref(),
    ));
    let _ = sender.send(container_set_slot_packet(
        PLAYER_OFFHAND_SLOT as i16,
        offhand_after.as_ref(),
    ));
    Ok(())
}

async fn interact_with_block(uuid: [u8; 16], pos: (i32, i32, i32)) -> Result<bool> {
    let player_is_online = ONLINE_PLAYERS
        .lock()
        .await
        .iter()
        .any(|player| player.uuid == uuid);
    if !player_is_online {
        return Ok(false);
    }

    let Some(state_id) = WORLD_BLOCKS.lock().await.get(&pos).copied() else {
        return Ok(false);
    };

    if let Some(next) = get_toggled_state(state_id) {
        set_world_block(pos, next).await?;

        // Doors are two blocks, so both halves must stay in the same open state.
        if is_door_lower(state_id) {
            let upper_pos = (pos.0, pos.1 + 1, pos.2);
            if let Some(upper_state) = WORLD_BLOCKS.lock().await.get(&upper_pos).copied() {
                if is_door_upper(upper_state) {
                    if let Some(upper_next) = get_toggled_state(upper_state) {
                        set_world_block(upper_pos, upper_next).await?;
                    }
                }
            }
        } else if is_door_upper(state_id) {
            let lower_pos = (pos.0, pos.1 - 1, pos.2);
            if let Some(lower_state) = WORLD_BLOCKS.lock().await.get(&lower_pos).copied() {
                if is_door_lower(lower_state) {
                    if let Some(lower_next) = get_toggled_state(lower_state) {
                        set_world_block(lower_pos, lower_next).await?;
                    }
                }
            }
        }

        return Ok(true);
    }

    Ok(false)
}

const TOGGLE_RULES: &[(i32, i32, i32)] = &[
    // State IDs in these ranges alternate closed/open or unpowered/powered by step.
    (4590, 4653, 2),
    (11822, 11885, 2),
    (11886, 11949, 2),
    (11950, 12013, 2),
    (12014, 12077, 2),
    (12142, 12205, 2),
    (12206, 12269, 2),
    (12078, 12141, 2),
    (12270, 12333, 2),
    (19148, 19211, 2),
    (19212, 19275, 2),
    (5962, 6025, 4),
    (6026, 6089, 4),
    (6090, 6153, 4),
    (6154, 6217, 4),
    (6218, 6281, 4),
    (6346, 6409, 4),
    (6410, 6473, 4),
    (6282, 6345, 4),
    (6474, 6537, 4),
    (18748, 18811, 4),
    (18812, 18875, 4),
    (6997, 7028, 2),
    (11310, 11341, 2),
    (11342, 11373, 2),
    (11374, 11405, 2),
    (11406, 11437, 2),
    (11470, 11501, 2),
    (11502, 11533, 2),
    (11438, 11469, 2),
    (11534, 11565, 2),
    (18876, 18907, 2),
    (18908, 18939, 2),
    (5626, 5649, 1),
    (8611, 8634, 1),
    (8635, 8658, 1),
    (8659, 8682, 1),
    (8683, 8706, 1),
    (8707, 8730, 1),
    (8755, 8778, 1),
    (8779, 8802, 1),
    (8731, 8754, 1),
    (8803, 8826, 1),
    (19100, 19123, 1),
    (19124, 19147, 1),
    (5748, 5771, 1),
    (20374, 20397, 1),
];

fn toggled_state_for_rule(state_id: i32, start: i32, end: i32, step: i32) -> Option<i32> {
    if !(start..=end).contains(&state_id) {
        return None;
    }

    let bucket = (state_id - start) / step;
    // Even buckets move forward to the paired state; odd buckets move back.
    Some(if bucket % 2 == 0 {
        state_id + step
    } else {
        state_id - step
    })
}

pub fn get_toggled_state(state_id: i32) -> Option<i32> {
    TOGGLE_RULES
        .iter()
        .find_map(|&(start, end, step)| toggled_state_for_rule(state_id, start, end, step))
}

pub fn is_door_upper(state_id: i32) -> bool {
    // Door upper and lower halves use separate ranges, so both halves must be handled.
    matches!(state_id,
        4590..=4597 | 4606..=4613 | 4622..=4629 | 4638..=4645 | 11822..=11829 | 11838..=11845 | 11854..=11861 | 11870..=11877 | 11886..=11893 | 11902..=11909 | 11918..=11925 | 11934..=11941 | 11950..=11957 | 11966..=11973 | 11982..=11989 | 11998..=12005 | 12014..=12021 | 12030..=12037 | 12046..=12053 | 12062..=12069 | 12078..=12085 | 12094..=12101 | 12110..=12117 | 12126..=12133 | 12142..=12149 | 12158..=12165 | 12174..=12181 | 12190..=12197 | 12206..=12213 | 12222..=12229 | 12238..=12245 | 12254..=12261 | 12270..=12277 | 12286..=12293 | 12302..=12309 | 12318..=12325 | 19148..=19155 | 19164..=19171 | 19180..=19187 | 19196..=19203 | 19212..=19219 | 19228..=19235 | 19244..=19251 | 19260..=19267)
}

pub fn is_door_lower(state_id: i32) -> bool {
    matches!(state_id,
        4598..=4605 | 4614..=4621 | 4630..=4637 | 4646..=4653 | 11830..=11837 | 11846..=11853 | 11862..=11869 | 11878..=11885 | 11894..=11901 | 11910..=11917 | 11926..=11933 | 11942..=11949 | 11958..=11965 | 11974..=11981 | 11990..=11997 | 12006..=12013 | 12022..=12029 | 12038..=12045 | 12054..=12061 | 12070..=12077 | 12086..=12093 | 12102..=12109 | 12118..=12125 | 12134..=12141 | 12150..=12157 | 12166..=12173 | 12182..=12189 | 12198..=12205 | 12214..=12221 | 12230..=12237 | 12246..=12253 | 12262..=12269 | 12278..=12285 | 12294..=12301 | 12310..=12317 | 12326..=12333 | 19156..=19163 | 19172..=19179 | 19188..=19195 | 19204..=19211 | 19220..=19227 | 19236..=19243 | 19252..=19259 | 19268..=19275)
}

fn is_slab_state_id(state_id: i32) -> bool {
    matches!(
        state_id,
        13333
            | 13339
            | 13345
            | 13351
            | 13357
            | 13363
            | 13369
            | 13375
            | 13381
            | 13387
            | 13393
            | 21037
            | 21043
            | 13399
            | 13405
            | 13411
            | 13417
            | 13423
            | 13429
            | 13435
            | 13441
            | 13447
            | 13453
            | 13459
            | 13465
            | 13471
            | 13477
            | 12877
            | 12883
            | 12889
            | 16419
            | 16425
            | 16431
            | 16437
            | 16443
            | 16449
            | 16455
            | 16461
            | 16467
            | 16473
            | 16479
            | 16485
            | 28010
            | 28421
            | 29243
            | 28832
            | 22239
            | 22740
            | 9006
            | 23867
            | 24279
    )
}

fn block_name_for_state_id(state_id: i32) -> Option<&'static str> {
    BLOCK_ITEM_PLACEMENTS
        .values()
        .find(|placement| {
            placement.lower_state_id == state_id || placement.upper_state_id == state_id
        })
        .map(|placement| placement.block_name)
}

fn double_slab_state_for(block_name: &str) -> Option<i32> {
    match block_name {
        "minecraft:oak_slab" => Some(13333),
        "minecraft:spruce_slab" => Some(13339),
        "minecraft:birch_slab" => Some(13345),
        "minecraft:jungle_slab" => Some(13351),
        "minecraft:acacia_slab" => Some(13357),
        "minecraft:cherry_slab" => Some(13363),
        "minecraft:dark_oak_slab" => Some(13369),
        "minecraft:pale_oak_slab" => Some(13375),
        "minecraft:mangrove_slab" => Some(13381),
        "minecraft:bamboo_slab" => Some(13387),
        "minecraft:bamboo_mosaic_slab" => Some(13393),
        "minecraft:crimson_slab" => Some(21037),
        "minecraft:warped_slab" => Some(21043),
        "minecraft:stone_slab" => Some(13399),
        "minecraft:smooth_stone_slab" => Some(13405),
        "minecraft:sandstone_slab" => Some(13411),
        "minecraft:cut_sandstone_slab" => Some(13417),
        "minecraft:petrified_oak_slab" => Some(13423),
        "minecraft:cobblestone_slab" => Some(13429),
        "minecraft:brick_slab" => Some(13435),
        "minecraft:stone_brick_slab" => Some(13441),
        "minecraft:mud_brick_slab" => Some(13447),
        "minecraft:nether_brick_slab" => Some(13453),
        "minecraft:quartz_slab" => Some(13459),
        "minecraft:red_sandstone_slab" => Some(13465),
        "minecraft:cut_red_sandstone_slab" => Some(13471),
        "minecraft:purpur_slab" => Some(13477),
        "minecraft:prismarine_slab" => Some(12877),
        "minecraft:prismarine_brick_slab" => Some(12883),
        "minecraft:dark_prismarine_slab" => Some(12889),
        "minecraft:polished_granite_slab" => Some(16419),
        "minecraft:smooth_red_sandstone_slab" => Some(16425),
        "minecraft:mossy_stone_brick_slab" => Some(16431),
        "minecraft:polished_diorite_slab" => Some(16437),
        "minecraft:mossy_cobblestone_slab" => Some(16443),
        "minecraft:end_stone_brick_slab" => Some(16449),
        "minecraft:smooth_sandstone_slab" => Some(16455),
        "minecraft:smooth_quartz_slab" => Some(16461),
        "minecraft:granite_slab" => Some(16467),
        "minecraft:andesite_slab" => Some(16473),
        "minecraft:red_nether_brick_slab" => Some(16479),
        "minecraft:polished_andesite_slab" => Some(16485),
        "minecraft:diorite_slab" => Some(16491),
        "minecraft:cobbled_deepslate_slab" => Some(28010),
        "minecraft:polished_deepslate_slab" => Some(28421),
        "minecraft:deepslate_brick_slab" => Some(29243),
        "minecraft:deepslate_tile_slab" => Some(28832),
        "minecraft:blackstone_slab" => Some(22239),
        "minecraft:polished_blackstone_slab" => Some(22740),
        "minecraft:polished_blackstone_brick_slab" => Some(9006),
        _ => None,
    }
}

fn block_intersects_player(player: &OnlinePlayer, pos: (i32, i32, i32), state_id: i32) -> bool {
    let block_min_x = pos.0 as f64;
    let block_min_y = pos.1 as f64;
    let block_min_z = pos.2 as f64;
    let block_max_y = if is_slab_state_id(state_id) {
        block_min_y + 0.5
    } else {
        block_min_y + 1.0
    };
    let block_max_x = block_min_x + 1.0;
    let block_max_z = block_min_z + 1.0;

    // Vanilla player collision is roughly 0.6 blocks wide and 1.8 blocks tall.
    let player_min_x = player.x - 0.3;
    let player_max_x = player.x + 0.3;
    let player_min_y = player.y;
    let player_max_y = player.y + 1.8;
    let player_min_z = player.z - 0.3;
    let player_max_z = player.z + 0.3;
    player_min_x < block_max_x
        && player_max_x > block_min_x
        && player_min_y < block_max_y
        && player_max_y > block_min_y
        && player_min_z < block_max_z
        && player_max_z > block_min_z
}

fn can_place_block_at(player: &OnlinePlayer, pos: (i32, i32, i32), state_id: i32) -> bool {
    !block_intersects_player(player, pos, state_id)
}

pub(crate) fn load_block_item_placements() -> HashMap<i32, BlockPlacement> {
    // Bad rows are ignored so a partial generated CSV can still boot the server.
    include_str!("block_items.csv")
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(5, ',');
            let item_id = parts.next()?.parse().ok()?;
            let lower_state_id = parts.next()?.parse().ok()?;
            let upper_state_id = parts.next()?.parse().ok()?;
            let kind = match parts.next()? {
                "single" => BlockPlacementKind::Single,
                "double" => BlockPlacementKind::Double,
                "bed" => BlockPlacementKind::Bed,
                _ => return None,
            };
            let block_name = parts.next()?;
            Some((
                item_id,
                BlockPlacement {
                    block_name,
                    lower_state_id,
                    upper_state_id,
                    kind,
                },
            ))
        })
        .collect()
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

async fn place_hand_block(uuid: [u8; 16], hand: i32, pos: (i32, i32, i32)) -> Result<()> {
    let Some(player) = ONLINE_PLAYERS
        .lock()
        .await
        .iter()
        .find(|player| player.uuid == uuid)
        .cloned()
    else {
        return Ok(());
    };
    // Use-item-on sends 0 for main hand and 1 for offhand.
    let inventory_slot = match hand {
        0 => PLAYER_HOTBAR_SLOT_START + player.held_slot as usize,
        1 => PLAYER_OFFHAND_SLOT,
        _ => return Ok(()),
    };
    let held_item = player
        .inventory_slots
        .get(inventory_slot)
        .and_then(|item| item.as_ref())
        .map(|item| item.item_id);
    let yaw = player.y_rot;
    let Some(item_id) = held_item else {
        return Ok(());
    };
    let Some(placement) = BLOCK_ITEM_PLACEMENTS.get(&item_id).copied() else {
        return Ok(());
    };
    if let Some(existing_state) = WORLD_BLOCKS.lock().await.get(&pos).copied()
        && block_name_for_state_id(existing_state) == Some(placement.block_name)
        && let Some(double_state) = double_slab_state_for(placement.block_name)
    {
        // Placing the same slab into an occupied slab block upgrades it to a double slab.
        set_world_block(pos, double_state).await?;
        return Ok(());
    }

    if !can_place_block_at(&player, pos, placement.lower_state_id) {
        return Ok(());
    }

    set_world_block(pos, placement.lower_state_id).await?;
    match placement.kind {
        BlockPlacementKind::Single => {}
        BlockPlacementKind::Double => {
            if placement.upper_state_id >= 0 {
                // Tall blocks use the second state ID one block above the base.
                let upper = (pos.0, pos.1 + 1, pos.2);
                if can_place_block_at(&player, upper, placement.upper_state_id) {
                    set_world_block(upper, placement.upper_state_id).await?;
                }
            }
        }
        BlockPlacementKind::Bed => {
            if placement.upper_state_id >= 0 {
                // Beds extend horizontally from the clicked foot block.
                let (dx, dz) = horizontal_offset_from_yaw(yaw);
                let upper = (pos.0 + dx, pos.1, pos.2 + dz);
                if can_place_block_at(&player, upper, placement.upper_state_id) {
                    set_world_block(upper, placement.upper_state_id).await?;
                }
            }
        }
    }
    Ok(())
}

fn horizontal_offset_from_yaw(yaw: f32) -> (i32, i32) {
    match (((yaw.rem_euclid(360.0) + 45.0) / 90.0) as i32) & 3 {
        0 => (0, 1),
        1 => (-1, 0),
        2 => (0, -1),
        _ => (1, 0),
    }
}

async fn set_world_block(pos: (i32, i32, i32), block_state_id: i32) -> Result<()> {
    let mut world = WORLD_BLOCKS.lock().await;
    world.insert(pos, block_state_id);
    drop(world);

    // Store the edit once, then fan the same block update packet to every client.
    let packet = block_update_packet(pos, block_state_id);
    let online = ONLINE_PLAYERS.lock().await;
    for player in online.iter() {
        let _ = player.sender.send(packet.clone());
    }
    Ok(())
}

fn block_update_packet((x, y, z): (i32, i32, i32), block_state_id: i32) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_BLOCK_UPDATE_PACKET_ID);
    packet.extend_from_slice(&pack_position(x, y, z).to_be_bytes());
    write_var_i32(&mut packet, block_state_id);
    packet
}

fn entity_position_sync_packet(player: &OnlinePlayer) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_ENTITY_POSITION_SYNC_PACKET_ID);
    write_var_i32(&mut packet, player.entity_id);
    packet.extend_from_slice(&player.x.to_be_bytes());
    packet.extend_from_slice(&player.y.to_be_bytes());
    packet.extend_from_slice(&player.z.to_be_bytes());
    packet.extend_from_slice(&0.0f64.to_be_bytes());
    packet.extend_from_slice(&0.0f64.to_be_bytes());
    packet.extend_from_slice(&0.0f64.to_be_bytes());
    packet.extend_from_slice(&player.y_rot.to_be_bytes());
    packet.extend_from_slice(&player.x_rot.to_be_bytes());
    packet.push(u8::from(player.on_ground));
    packet
}

fn rotate_head_packet(player: &OnlinePlayer) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_ROTATE_HEAD_PACKET_ID);
    write_var_i32(&mut packet, player.entity_id);
    packet.push(pack_degrees(player.y_rot));
    packet
}

fn pack_degrees(value: f32) -> u8 {
    (((value.rem_euclid(360.0) * 256.0) / 360.0) as i32 & 0xff) as u8
}

fn offline_uuid(username: &str) -> [u8; 16] {
    let mut hash: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    for byte in format!("OfflinePlayer:{username}").bytes() {
        hash ^= byte as u128;
        hash = hash.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
    }
    let mut uuid = hash.to_be_bytes();
    uuid[6] = (uuid[6] & 0x0f) | 0x30;
    uuid[8] = (uuid[8] & 0x3f) | 0x80;
    uuid
}
