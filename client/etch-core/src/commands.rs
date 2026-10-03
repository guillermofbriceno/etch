use std::path::PathBuf;

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
    SendAttachment(AttachmentSend),
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
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct AttachmentSend {
    pub room_id: String,
    pub path: PathBuf,
    pub compress: bool,
    /// Measured by the frontend for video and audio; core reads an image's own size.
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
