use serde::Deserialize;
use crate::models::ServerBookmark;

// gui -> core
#[derive(Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum CoreCommand {
    Matrix(MatrixCommand),
    Mumble(MumbleCommand),
    System(SystemCommand),
    #[serde(skip)]
    FetchMedia {
        mxc_url: String,
        respond: tokio::sync::oneshot::Sender<Result<Vec<u8>, String>>,
    },
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum MatrixCommand {
    SendMessage(ChatMessageSend),
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
    HideDm { room_id: String },
    UnhideDm { room_id: String },
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
}
