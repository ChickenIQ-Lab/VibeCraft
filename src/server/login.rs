use super::configuration::run_configuration;
use super::packets::write_profile_properties;
use super::play::{keep_player_connected, write_packets};
use super::profile::{fetch_profile_properties, offline_uuid, uuid_without_dashes};
use super::state::{register_player, unregister_player};
use super::storage::load_player_data;
use super::world::{enter_world, initial_chunk_set};
use crate::cursor::Cursor;
use crate::protocol::{read_packet, write_packet, write_string, write_var_i32};
use crate::types::{Config, NEXT_ENTITY_ID, OnlinePlayer, PersistedPlayerData, ProfileProperty};
use anyhow::{Result, bail};
use std::sync::atomic::Ordering;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::{info, warn};

pub(super) async fn handle_login(mut stream: TcpStream, config: Config) -> Result<()> {
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
    super::wait_for_packet_id(&mut stream, 0x03, "login acknowledged").await?;
    run_configuration(&mut stream, &config).await?;
    enter_world(&mut stream, entity_id, &saved_player).await?;

    let (reader, writer) = stream.into_split();
    let (sender, receiver) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        if let Err(err) = write_packets(writer, receiver).await {
            tracing::debug!(error = %err, "writer closed");
        }
    });

    // The online snapshot stores enough state to save reconnect data and fan out packets.
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
