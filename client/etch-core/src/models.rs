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
    fn next_failure_exponential_backoff() {
        let state = ConnectionState::Disconnected;

        let s1 = state.next_failure("err".into());
        assert!(matches!(s1, ConnectionState::Failed { retries: 1, retry_in_secs: 2, .. }));

        let s2 = s1.next_failure("err".into());
        assert!(matches!(s2, ConnectionState::Failed { retries: 2, retry_in_secs: 4, .. }));

        let s3 = s2.next_failure("err".into());
        assert!(matches!(s3, ConnectionState::Failed { retries: 3, retry_in_secs: 8, .. }));

        let s4 = s3.next_failure("err".into());
        assert!(matches!(s4, ConnectionState::Failed { retries: 4, retry_in_secs: 16, .. }));

        let s5 = s4.next_failure("err".into());
        assert!(matches!(s5, ConnectionState::Failed { retries: 5, retry_in_secs: 32, .. }));
    }

    #[test]
    fn next_failure_caps_at_60_seconds() {
        let state = ConnectionState::Disconnected;
        // 2^6 = 64, should be capped to 60
        let mut s = state;
        for _ in 0..6 {
            s = s.next_failure("err".into());
        }
        assert!(matches!(s, ConnectionState::Failed { retries: 6, retry_in_secs: 60, .. }));

        // Further retries stay at 60
        let s7 = s.next_failure("err".into());
        assert!(matches!(s7, ConnectionState::Failed { retries: 7, retry_in_secs: 60, .. }));
    }

    #[test]
    fn connected_state_resets_retries() {
        let failed = ConnectionState::Failed {
            reason: "err".into(),
            retries: 5,
            retry_in_secs: 32,
        };
        assert_eq!(failed.retries(), 5);

        let connected = ConnectionState::Connected;
        assert_eq!(connected.retries(), 0);

        // Starting fresh failures from non-failed state begins at retry 1
        let new_fail = connected.next_failure("new err".into());
        assert!(matches!(new_fail, ConnectionState::Failed { retries: 1, retry_in_secs: 2, .. }));
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

#[derive(Clone, Debug)]
pub struct VoiceServerConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}
