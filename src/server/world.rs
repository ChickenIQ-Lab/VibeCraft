use super::packets::{
    available_commands_packet, chunk_batch_finished_packet, chunk_batch_start_packet,
    chunk_cache_center_packet, container_set_slot_packet, player_abilities_packet,
    player_position_packet, set_held_slot_packet,
};
use crate::constants::*;
use crate::protocol::{pack_position, write_packet, write_string, write_var_i32};
use crate::types::{PersistedPlayerData, WORLD_BLOCKS};
use anyhow::Result;
use std::collections::HashSet;
use tokio::net::TcpStream;

pub(super) async fn enter_world(
    stream: &mut TcpStream,
    entity_id: i32,
    player: &PersistedPlayerData,
) -> Result<()> {
    let chunk_x = (player.x.floor() as i32).div_euclid(16);
    let chunk_z = (player.z.floor() as i32).div_euclid(16);

    // Send the absolute spawn position before the heavy chunk batch so the
    // client does not spend its first seconds at the void floor waiting for
    // terrain packets to finish.
    send_play_login(stream, entity_id, player.game_mode).await?;
    write_packet(stream, &player_abilities_packet(player.game_mode)).await?;
    write_packet(stream, &available_commands_packet()).await?;
    send_level_chunks_load_start(stream).await?;
    send_chunk_cache_center(stream, chunk_x, chunk_z).await?;
    send_chunk_cache_radius(stream, VIEW_DISTANCE).await?;
    send_default_spawn(stream).await?;
    send_player_position(stream, player).await?;
    send_player_inventory(stream, player).await?;
    send_superflat_chunks(stream, chunk_x, chunk_z).await
}

async fn send_play_login(
    stream: &mut TcpStream,
    entity_id: i32,
    game_mode: crate::types::GameMode,
) -> Result<()> {
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
    packet.push(game_mode.id() as u8);
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
    write_packet(
        stream,
        &player_position_packet(player.x, player.y, player.z, player.y_rot, player.x_rot),
    )
    .await
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

pub(super) fn initial_chunk_set(center_x: i32, center_z: i32) -> HashSet<(i32, i32)> {
    let mut chunks = HashSet::new();
    for z in center_z - VIEW_DISTANCE..=center_z + VIEW_DISTANCE {
        for x in center_x - VIEW_DISTANCE..=center_x + VIEW_DISTANCE {
            chunks.insert((x, z));
        }
    }
    chunks
}

pub(super) async fn flat_chunk_packet(x: i32, z: i32) -> Vec<u8> {
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

pub(super) fn generated_block_state_at((x, y, z): (i32, i32, i32)) -> i32 {
    if y == 60 {
        return BLOCK_STATE_BEDROCK;
    }
    if (61..=63).contains(&y) {
        return BLOCK_STATE_DIRT;
    }
    if y == 64 {
        return BLOCK_STATE_GRASS_BLOCK;
    }
    if x == 4 && z == 4 && (65..=68).contains(&y) {
        return BLOCK_STATE_OAK_LOG;
    }
    if (2..=6).contains(&x) && (2..=6).contains(&z) && (67..=69).contains(&y) {
        return BLOCK_STATE_OAK_LEAVES;
    }
    if (3..=5).contains(&x) && (3..=5).contains(&z) && y == 70 {
        return BLOCK_STATE_OAK_LEAVES;
    }
    0
}

async fn flat_chunk_data(chunk_x: i32, chunk_z: i32) -> Vec<u8> {
    let mut data = Vec::new();
    let world_blocks = WORLD_BLOCKS.lock().await;

    // Sections -4..20 cover the vanilla overworld height range for this protocol.
    for section_y in -4..20 {
        let mut values = [0i32; 4096];

        // Generated terrain comes first; player edits override it below.
        let base_y = section_y * 16;
        let base_x = chunk_x * 16;
        let base_z = chunk_z * 16;

        for dy in 0..16 {
            let global_y = base_y + dy;
            for dz in 0..16 {
                let global_z = base_z + dz;
                for dx in 0..16 {
                    let global_x = base_x + dx;
                    values[(dy as usize * 16 + dz as usize) * 16 + dx as usize] =
                        generated_block_state_at((global_x, global_y, global_z));
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
