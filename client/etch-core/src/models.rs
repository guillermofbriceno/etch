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
        // `Failed` is the only arm that carries a retry count, and a fresh
        // state is not it, so there is nothing to read a count off.
        let state = ConnectionState::Disconnected;
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

    /// The retry count is carried by `Failed` and by no other arm.
    ///
    /// There is deliberately no accessor that answers it for a whole
    /// `ConnectionState`: `Connecting` is an attempt in progress, so any such
    /// accessor would have to report 0 however many attempts had already
    /// failed. The authoritative counter is `MatrixConnection.retries`; the
    /// field here is the display snapshot that crosses to the frontend.
    #[test]
    fn only_the_failed_arm_carries_a_retry_count() {
        let failed = ConnectionState::Failed {
            reason: "err".into(),
            retries: 5,
            retry_in_secs: 32,
        };
        assert!(matches!(failed, ConnectionState::Failed { retries: 5, .. }));
        assert!(failed.is_failed());

        assert!(!matches!(ConnectionState::Connected, ConnectionState::Failed { .. }));
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
