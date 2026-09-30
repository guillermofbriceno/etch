use serde::Deserialize;
use crate::models::ServerBookmark;

// gui -> core
#[derive(Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum CoreCommand {
    Matrix(MatrixCommand),
    Mumble(MumbleCommand),
    System(SystemCommand),
}

/// Not a `CoreCommand`: it carries a `oneshot::Sender` and travels on its own channel,
/// so images cannot crowd out control commands.
pub struct MediaRequest {
    pub mxc_url: String,
    pub respond: tokio::sync::oneshot::Sender<Result<Vec<u8>, String>>,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum MatrixCommand {
    SendMessage(ChatMessageSend),
    EditMessage { room_id: String, event_id: String, text: String, html_body: Option<String> },
    RedactMessage { room_id: String, event_id: String },
    ToggleReaction { room_id: String, event_id: String, key: String },
    CreateDirectMessage { target_user_id: String },
    SetDisplayName(String),
    SetAvatar(String),
    ChangePassword { current_password: String, new_password: String },
    SendReadReceipt { room_id: String, event_id: String },
    PaginateBackwards { room_id: String },
    EnableEncryption { room_id: String },
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum MumbleCommand {
    SwitchChannel(u32),
    MuteSelf(bool),
    DeafenSelf(bool),
    SetUserVolume { session_id: u32, volume_db: f32 },
    SetTransmissionMode(String),
    SetVadThreshold(f64),
    SetVoiceHold(i64),
    SetUseMumbleSettings(bool),
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum SystemCommand {
    ConnectToServer(ServerConnectionForm),
    LoadSettings,
    SaveBookmarks(Vec<ServerBookmark>),
    MuteMic(bool),
    Deafen(bool),
    OpenMumbleGui(String),
    RestartMumble(String),
    SetLogLevel(String),
    TestError,
    SetDeafenSuppressesNotifs(bool),
    HideDm { room_id: String },
    UnhideDm { room_id: String },
    AcceptMumbleCert { host: String, port: u16, fingerprint: String },
}

#[derive(Clone, Deserialize)]
pub struct ServerConnectionForm {
    pub username: String,
    pub hostname: String,
    pub port: String,
    pub password: Option<String>,
    pub mumble_host: Option<String>,
    pub mumble_port: Option<u16>,
    pub mumble_username: Option<String>,
    pub mumble_password: Option<String>,
    /// Explicit homeserver URL (e.g. "http://localhost:6167"). When set,
    /// bypasses server name discovery and connects directly. The `hostname`
    /// field is still used for user ID construction (@user:hostname).
    #[serde(default)]
    pub homeserver_url: Option<String>,
}

impl From<&crate::models::ServerBookmark> for ServerConnectionForm {
    fn from(bm: &crate::models::ServerBookmark) -> Self {
        Self {
            username: bm.username.clone(),
            hostname: bm.address.clone(),
            port: bm.port.to_string(),
            password: None,
            mumble_host: bm.mumble_host.clone(),
            mumble_port: bm.mumble_port,
            mumble_username: bm.mumble_username.clone(),
            mumble_password: bm.mumble_password.clone(),
            homeserver_url: None,
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct ChatMessageSend {
    pub room_id: String,
    pub text: String,
    pub html_body: Option<String>,
    pub attachment_path: Option<String>,
    #[serde(default)]
    pub media_info: Option<OutgoingMediaInfo>,
}

#[derive(Debug, PartialEq, Deserialize, Default)]
pub struct OutgoingMediaInfo {
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send_message(json: &str) -> ChatMessageSend {
        match serde_json::from_str(json).expect("SendMessage should deserialize") {
            MatrixCommand::SendMessage(msg) => msg,
            other => panic!("expected SendMessage, got {other:?}"),
        }
    }

    #[test]
    fn a_message_without_media_info_still_deserializes() {
        let msg = send_message(r#"{"type":"SendMessage","data":{
            "room_id":"!a:b","text":"hi","html_body":null,"attachment_path":null
        }}"#);
        assert_eq!(msg, ChatMessageSend {
            room_id: "!a:b".into(),
            text: "hi".into(),
            html_body: None,
            attachment_path: None,
            media_info: None,
        });
    }

    #[test]
    fn a_null_media_info_is_none() {
        let msg = send_message(r#"{"type":"SendMessage","data":{
            "room_id":"!a:b","text":"","html_body":null,"attachment_path":"/tmp/photo.png","media_info":null
        }}"#);
        assert_eq!(msg.media_info, None);
    }

    #[test]
    fn media_info_carries_what_the_frontend_measured() {
        let msg = send_message(r#"{"type":"SendMessage","data":{
            "room_id":"!a:b","text":"","html_body":null,"attachment_path":"/tmp/clip.mp4",
            "media_info":{"width":1920,"height":1080,"duration_ms":12500}
        }}"#);
        assert_eq!(msg.attachment_path.as_deref(), Some("/tmp/clip.mp4"));
        assert_eq!(msg.media_info, Some(OutgoingMediaInfo {
            width: Some(1920),
            height: Some(1080),
            duration_ms: Some(12_500),
        }));
    }

    #[test]
    fn unmeasured_media_values_may_be_null_or_absent() {
        let msg = send_message(r#"{"type":"SendMessage","data":{
            "room_id":"!a:b","text":"","html_body":null,"attachment_path":"/tmp/song.ogg",
            "media_info":{"width":null,"duration_ms":3000}
        }}"#);
        assert_eq!(msg.media_info, Some(OutgoingMediaInfo {
            width: None,
            height: None,
            duration_ms: Some(3000),
        }));
    }
}
