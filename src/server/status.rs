use crate::cursor::Cursor;
use crate::protocol::{read_packet, write_packet, write_string, write_var_i32};
use crate::types::Config;
use anyhow::{Result, bail};
use serde_json::json;
use tokio::net::TcpStream;
use tracing::debug;

pub(super) async fn handle_status(mut stream: TcpStream, config: Config) -> Result<()> {
    let request = read_packet(&mut stream).await?;
    let mut cursor = Cursor::new(&request);
    let packet_id = cursor.read_var_i32()?;
    if packet_id != 0x00 {
        bail!("expected status request, got packet id {packet_id}");
    }

    // Server-list ping gets only the minimal status shape vanilla clients need.
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
