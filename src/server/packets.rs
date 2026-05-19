use crate::constants::*;
use crate::protocol::{pack_position, write_string, write_var_i32};
use crate::types::{OnlinePlayer, PersistedInventoryItem, ProfileProperty};
use anyhow::Result;

pub(super) fn write_profile_properties(
    out: &mut Vec<u8>,
    properties: &[ProfileProperty],
) -> Result<()> {
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

pub(super) fn keep_alive_packet() -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_KEEP_ALIVE_PACKET_ID);
    packet.extend_from_slice(&0i64.to_be_bytes());
    packet
}

fn player_skin_parts_mask() -> u8 {
    // Turn on cape and the usual outer layers until we track client preferences.
    0x7f
}

pub(super) fn player_entity_metadata_packet(player: &OnlinePlayer) -> Vec<u8> {
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

pub(super) fn player_info_update_packet(players: &[OnlinePlayer]) -> Result<Vec<u8>> {
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

pub(super) fn add_player_entity_packet(player: &OnlinePlayer) -> Vec<u8> {
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

pub(super) fn player_info_remove_packet(uuid: [u8; 16]) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_PLAYER_INFO_REMOVE_PACKET_ID);
    write_var_i32(&mut packet, 1);
    packet.extend_from_slice(&uuid);
    packet
}

pub(super) fn container_set_slot_packet(
    slot: i16,
    item: Option<&PersistedInventoryItem>,
) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_CONTAINER_SET_SLOT_PACKET_ID);
    write_var_i32(&mut packet, 0);
    write_var_i32(&mut packet, 0);
    packet.extend_from_slice(&slot.to_be_bytes());
    write_optional_item_stack(&mut packet, item);
    packet
}

pub(super) fn set_held_slot_packet(slot: i16) -> Vec<u8> {
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

pub(super) fn basic_item_stack_bytes(count: i32, item_id: i32) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, count);
    write_var_i32(&mut packet, item_id);
    write_var_i32(&mut packet, 0);
    write_var_i32(&mut packet, 0);
    packet
}

pub(super) fn remove_entities_packet(entity_id: i32) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_REMOVE_ENTITIES_PACKET_ID);
    write_var_i32(&mut packet, 1);
    write_var_i32(&mut packet, entity_id);
    packet
}

pub(super) fn chunk_batch_start_packet() -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, CHUNK_BATCH_START_PACKET_ID);
    packet
}

pub(super) fn chunk_batch_finished_packet(batch_size: i32) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, CHUNK_BATCH_FINISHED_PACKET_ID);
    write_var_i32(&mut packet, batch_size);
    packet
}

pub(super) fn chunk_cache_center_packet(x: i32, z: i32) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_SET_CHUNK_CACHE_CENTER_PACKET_ID);
    write_var_i32(&mut packet, x);
    write_var_i32(&mut packet, z);
    packet
}

pub(super) fn block_update_packet((x, y, z): (i32, i32, i32), block_state_id: i32) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_BLOCK_UPDATE_PACKET_ID);
    packet.extend_from_slice(&pack_position(x, y, z).to_be_bytes());
    write_var_i32(&mut packet, block_state_id);
    packet
}

pub(super) fn entity_position_sync_packet(player: &OnlinePlayer) -> Vec<u8> {
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

pub(super) fn rotate_head_packet(player: &OnlinePlayer) -> Vec<u8> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, PLAY_ROTATE_HEAD_PACKET_ID);
    write_var_i32(&mut packet, player.entity_id);
    packet.push(pack_degrees(player.y_rot));
    packet
}

fn pack_degrees(value: f32) -> u8 {
    (((value.rem_euclid(360.0) * 256.0) / 360.0) as i32 & 0xff) as u8
}
