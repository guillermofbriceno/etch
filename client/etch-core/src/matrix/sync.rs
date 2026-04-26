use matrix_sdk::{
    Client, config::SyncSettings,
    ruma::events::StateEventType,
};
use serde_json::Value;
use matrix_sdk::deserialized_responses::RawAnySyncOrStrippedState;
use matrix_sdk::room::Room;

use crate::models::{RoomInfo, RoomType, VoiceServerConfig};

pub async fn build_room_info(room: &Room) -> anyhow::Result<RoomInfo> {
    let config = get_room_config(room).await?;
    let unread = room.unread_notification_counts();
    Ok(RoomInfo {
        id: room.room_id().to_string(),
        display_name: room.display_name().await?.to_string(),
        etch_room_type: config.room_type,
        channel_id: config.channel_id,
        is_default: config.is_default,
        unread_count: unread.notification_count,
        is_encrypted: room.latest_encryption_state().await.map(|s| s.is_encrypted()).unwrap_or(false),
    })
}

pub async fn fetch_rooms(client: &Client) -> anyhow::Result<Vec<RoomInfo>> {
    let mut rooms_model: Vec<RoomInfo> = vec![];
    for room in client.joined_rooms() {
        rooms_model.push(build_room_info(&room).await?);
    }
    Ok(rooms_model)
}

pub async fn sync_loop(client: Client) -> anyhow::Result<()> {
    log::debug!("Entering matrix sync loop");
    client.sync(SyncSettings::default()).await?;
    log::debug!("Exited matrix sync loop");
    Ok(())
}

struct RoomConfig {
    room_type: RoomType,
    channel_id: Option<u32>,
    is_default: bool,
}

async fn get_room_config(room: &Room) -> anyhow::Result<RoomConfig> {
    match room
        .get_state_event(StateEventType::from("etch.room_config"), "").await?
    {
        Some(raw_event) => {
            let raw_json = match raw_event {
                RawAnySyncOrStrippedState::Sync(e) => e.json().to_string(),
                RawAnySyncOrStrippedState::Stripped(e) => e.json().to_string(),
            };
            let json: Value = serde_json::from_str(&raw_json)?;
            let content = &json["content"];

            let room_type = match content["room_type"].as_str() {
                Some("voice") => RoomType::Voice,
                _ => RoomType::Text,
            };

            let channel_id = content["channel_id"].as_u64().map(|v| v as u32);
            let is_default = content["is_default"].as_bool().unwrap_or(false);

            Ok(RoomConfig { room_type, channel_id, is_default })
        }
        None => {
            let room_type = if room.is_direct().await.unwrap_or(false) {
                RoomType::Dm
            } else {
                RoomType::Text
            };
            Ok(RoomConfig { room_type, channel_id: None, is_default: false })
        }
    }
}

async fn get_voice_server_config(room: &Room) -> anyhow::Result<Option<VoiceServerConfig>> {
    match room
        .get_state_event(StateEventType::from("etch.voice_server"), "").await?
    {
        Some(raw_event) => {
            let raw_json = match raw_event {
                RawAnySyncOrStrippedState::Sync(e) => e.json().to_string(),
                RawAnySyncOrStrippedState::Stripped(e) => e.json().to_string(),
            };
            let json: Value = serde_json::from_str(&raw_json)?;
            let content = &json["content"];

            let host = content["host"].as_str()
                .ok_or_else(|| anyhow::anyhow!("etch.voice_server missing 'host' field"))?
                .to_string();
            let port = content["port"].as_u64().unwrap_or(64738) as u16;
            let password = content["password"].as_str().map(|s| s.to_string());

            Ok(Some(VoiceServerConfig { host, port, username: None, password }))
        }
        None => Ok(None),
    }
}

pub async fn find_voice_server(client: &Client, rooms: &[RoomInfo]) -> Option<VoiceServerConfig> {
    let default_room_info = rooms.iter().find(|r| r.is_default)?;
    let room_id = matrix_sdk::ruma::RoomId::parse(&default_room_info.id).ok()?;
    let room = client.get_room(&room_id)?;

    match get_voice_server_config(&room).await {
        Ok(config) => config,
        Err(e) => {
            log::warn!("Failed to read etch.voice_server from default room {}: {}", default_room_info.id, e);
            None
        }
    }
}
