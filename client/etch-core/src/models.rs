use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Serialize, Debug)]
#[serde(tag = "type", content = "data")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Failed { reason: String, retries: u32, retry_in_secs: u64 },
}

impl ConnectionState {
    pub fn retries(&self) -> u32 {
        match self {
            ConnectionState::Failed { retries, .. } => *retries,
            _ => 0,
        }
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, ConnectionState::Failed { .. })
    }

    pub fn next_failure(&self, reason: String) -> ConnectionState {
        let retries = self.retries() + 1;
        let retry_in_secs = std::cmp::min(2u64.pow(retries), 60);
        ConnectionState::Failed { reason, retries, retry_in_secs }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SenderProfile {
      pub display_name: Option<String>,
      pub avatar_url: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct MediaInfo {
    pub mxc_url: String,
    pub mimetype: String,
    pub size: u64,
    pub width: u64,
    pub height: u64,
    pub duration: u128,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ChatMessageReceive {
    pub id: String,
    pub sender: String,
    pub body: String,
    pub html_body: Option<String>,
    pub media: Option<MediaInfo>,

    pub timestamp: u128,
    // emoji key → list of sender user IDs (aggregated from m.reaction events)
    pub reactions: HashMap<String, Vec<String>>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum RoomType {
    Voice,
    Text,
    Dm,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct RoomInfo {
    pub id: String,
    pub display_name: String,
    pub etch_room_type: RoomType,
    pub channel_id: Option<u32>,
    pub is_default: bool,
    pub unread_count: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ServerBookmark {
    pub id: String,
    pub label: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub auto_connect: bool,
    #[serde(default)]
    pub mumble_host: Option<String>,
    #[serde(default)]
    pub mumble_port: Option<u16>,
    #[serde(default)]
    pub mumble_username: Option<String>,
    #[serde(default)]
    pub mumble_password: Option<String>,
}

#[derive(Clone, Debug)]
pub struct VoiceServerConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}
