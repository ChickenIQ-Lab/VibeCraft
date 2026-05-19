use super::packets::keep_alive_packet;
use super::state::{
    interact_with_block, place_hand_block, swap_held_with_offhand, update_held_slot,
    update_inventory_slot, update_player_state,
};
use crate::constants::*;
use crate::cursor::Cursor;
use crate::protocol::{read_packet, write_packet};
use crate::types::PersistedInventoryItem;
use anyhow::Result;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio::time::{Duration, interval};
use tracing::debug;

pub(super) async fn write_packets<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut receiver: mpsc::UnboundedReceiver<Vec<u8>>,
) -> Result<()> {
    while let Some(packet) = receiver.recv().await {
        write_packet(&mut writer, &packet).await?;
    }
    Ok(())
}

pub(super) async fn keep_player_connected<R: AsyncRead + Unpin>(
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
                0 | 2 => super::state::set_world_block(pos, 0).await?,
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
