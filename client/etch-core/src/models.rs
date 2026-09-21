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

}

/// Seconds to wait before retry number `retries`, doubling each time and
/// levelling off at a minute.
///
/// The caller owns the retry count. It cannot be derived from the current
/// `ConnectionState`, because an attempt in progress is `Connecting`, which
/// has no count to derive from; reading it back from the state would reset
/// the backoff on every attempt and pin it at the first step forever.
pub fn backoff_secs(retries: u32) -> u64 {
    std::cmp::min(2u64.saturating_pow(retries), 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_state_defaults() {
        let state = ConnectionState::Disconnected;
        assert_eq!(state.retries(), 0);
        assert!(!state.is_failed());
    }

    #[test]
    fn backoff_doubles_per_retry() {
        assert_eq!(backoff_secs(1), 2);
        assert_eq!(backoff_secs(2), 4);
        assert_eq!(backoff_secs(3), 8);
        assert_eq!(backoff_secs(4), 16);
        assert_eq!(backoff_secs(5), 32);
    }

    #[test]
    fn backoff_caps_at_60_seconds() {
        // 2^6 = 64, capped to 60, and it stays there.
        assert_eq!(backoff_secs(6), 60);
        assert_eq!(backoff_secs(7), 60);
        assert_eq!(backoff_secs(50), 60);
        // Far enough out that 2^retries no longer fits in a u64.
        assert_eq!(backoff_secs(u32::MAX), 60);
    }

    #[test]
    fn failed_state_reports_its_retry_count() {
        let failed = ConnectionState::Failed {
            reason: "err".into(),
            retries: 5,
            retry_in_secs: 32,
        };
        assert_eq!(failed.retries(), 5);
        assert!(failed.is_failed());

        assert_eq!(ConnectionState::Connected.retries(), 0);
        assert!(!ConnectionState::Connected.is_failed());
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
    pub edited: bool,
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
    pub is_encrypted: bool,
    pub avatar_url: Option<String>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceServerConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Clone, Debug)]
pub enum ConnectOutcome {
    Connected(Option<VoiceServerConfig>),
    NeedsPassword,
    Failed,
}
