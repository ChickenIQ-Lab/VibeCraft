use super::state::{reset_server_data, send_system_message, set_player_game_mode};
use crate::types::GameMode;
use anyhow::Result;

pub(super) async fn handle_command(uuid: [u8; 16], command: &str) -> Result<()> {
    let mut parts = command.split_whitespace();
    let Some(name) = parts.next() else {
        return Ok(());
    };

    match name {
        "gamemode" => handle_gamemode(uuid, parts.collect()).await,
        "reset" if parts.next().is_none() => reset_server_data().await,
        _ => send_system_message(uuid, "Unknown command").await,
    }
}

async fn handle_gamemode(uuid: [u8; 16], args: Vec<&str>) -> Result<()> {
    let [mode_arg] = args.as_slice() else {
        return send_system_message(uuid, "Usage: /gamemode <mode>").await;
    };
    let Some(game_mode) = GameMode::parse(mode_arg) else {
        return send_system_message(uuid, "Unknown game mode").await;
    };

    set_player_game_mode(uuid, game_mode).await?;
    send_system_message(uuid, &format!("Set game mode to {}", game_mode.name())).await
}
