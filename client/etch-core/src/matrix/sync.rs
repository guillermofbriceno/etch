use matrix_sdk::{
    Client, config::SyncSettings,
    ruma::events::StateEventType,
};
use serde_json::Value;
use matrix_sdk::deserialized_responses::RawAnySyncOrStrippedState;
use matrix_sdk::room::Room;

use crate::models::RoomInfo;
use crate::models::RoomType;

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
