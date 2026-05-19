mod configuration;
mod login;
mod packets;
mod play;
mod profile;
mod state;
mod status;
mod storage;
mod world;

use crate::cursor::Cursor;
use crate::protocol::read_packet;
use crate::types::Config;
use anyhow::{Result, bail};
use tokio::net::TcpStream;
use tracing::debug;

pub(crate) use state::load_block_item_placements;

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
        1 => status::handle_status(stream, config).await,
        2 => login::handle_login(stream, config).await,
        other => bail!("unsupported next state {other}"),
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
