use super::packets::{
    add_player_entity_packet, block_update_packet, chunk_batch_finished_packet,
    chunk_batch_start_packet, chunk_cache_center_packet, container_set_slot_packet,
    entity_position_sync_packet, player_entity_metadata_packet, player_info_remove_packet,
    player_info_update_packet, remove_entities_packet, rotate_head_packet,
};
use super::profile::uuid_without_dashes;
use super::storage::save_player_data;
use super::world::flat_chunk_packet;
use crate::constants::*;
use crate::types::{
    BLOCK_ITEM_PLACEMENTS, BlockPlacement, BlockPlacementKind, ONLINE_PLAYERS, OnlinePlayer,
    PersistedInventoryItem, WORLD_BLOCKS,
};
use anyhow::Result;
use std::collections::HashMap;
use tracing::warn;

pub(super) async fn register_player(player: &OnlinePlayer) -> Result<()> {
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

pub(super) async fn unregister_player(uuid: [u8; 16], entity_id: i32) -> Result<()> {
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

pub(super) async fn update_player_state(
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

pub(super) async fn update_held_slot(uuid: [u8; 16], slot: i16) -> Result<()> {
    let mut online = ONLINE_PLAYERS.lock().await;
    if let Some(player) = online.iter_mut().find(|player| player.uuid == uuid)
        && (0..9).contains(&slot)
    {
        player.held_slot = slot;
    }
    Ok(())
}

pub(super) async fn update_inventory_slot(
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

pub(super) async fn swap_held_with_offhand(uuid: [u8; 16]) -> Result<()> {
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

pub(super) async fn interact_with_block(uuid: [u8; 16], pos: (i32, i32, i32)) -> Result<bool> {
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

fn get_toggled_state(state_id: i32) -> Option<i32> {
    TOGGLE_RULES
        .iter()
        .find_map(|&(start, end, step)| toggled_state_for_rule(state_id, start, end, step))
}

fn is_door_upper(state_id: i32) -> bool {
    // Door upper and lower halves use separate ranges, so both halves must be handled.
    matches!(state_id,
        4590..=4597 | 4606..=4613 | 4622..=4629 | 4638..=4645 | 11822..=11829 | 11838..=11845 | 11854..=11861 | 11870..=11877 | 11886..=11893 | 11902..=11909 | 11918..=11925 | 11934..=11941 | 11950..=11957 | 11966..=11973 | 11982..=11989 | 11998..=12005 | 12014..=12021 | 12030..=12037 | 12046..=12053 | 12062..=12069 | 12078..=12085 | 12094..=12101 | 12110..=12117 | 12126..=12133 | 12142..=12149 | 12158..=12165 | 12174..=12181 | 12190..=12197 | 12206..=12213 | 12222..=12229 | 12238..=12245 | 12254..=12261 | 12270..=12277 | 12286..=12293 | 12302..=12309 | 12318..=12325 | 19148..=19155 | 19164..=19171 | 19180..=19187 | 19196..=19203 | 19212..=19219 | 19228..=19235 | 19244..=19251 | 19260..=19267)
}

fn is_door_lower(state_id: i32) -> bool {
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
    include_str!("../block_items.csv")
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

pub(super) async fn place_hand_block(
    uuid: [u8; 16],
    hand: i32,
    pos: (i32, i32, i32),
) -> Result<()> {
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

pub(super) async fn set_world_block(pos: (i32, i32, i32), block_state_id: i32) -> Result<()> {
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
