use crate::types::ProfileProperty;
use anyhow::{Result, bail};
use reqwest::{Client, StatusCode};
use std::fmt::Write as _;
use std::sync::LazyLock;
use std::time::Duration;
use tracing::debug;

static PROFILE_HTTP_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .connect_timeout(Duration::from_millis(750))
        .timeout(Duration::from_secs(2))
        .user_agent("VibeCraft/0.1")
        .build()
        .expect("profile http client")
});

pub(super) async fn fetch_profile_properties(
    username: &str,
    uuid: [u8; 16],
) -> Vec<ProfileProperty> {
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

pub(super) fn uuid_without_dashes(uuid: [u8; 16]) -> String {
    let mut formatted = String::with_capacity(32);
    for byte in uuid {
        let _ = write!(&mut formatted, "{byte:02x}");
    }
    formatted
}

pub(super) fn offline_uuid(username: &str) -> [u8; 16] {
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
