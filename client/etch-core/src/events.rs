use serde::Serialize;
use crate::models::{ConnectionState, RoomInfo};
use crate::matrix::name_colors::UserNameColor;
use crate::matrix::timeline::TimelineEntry;

// core -> gui
#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum CoreEvent {
    Matrix(MatrixEvent),
    Mumble(MumbleEvent),
    System(SystemEvent),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum MatrixEvent {
    TimelineAppend(String, Vec<TimelineEntry>),
    TimelinePushBack(String, TimelineEntry),
    TimelinePushFront(String, TimelineEntry),
    TimelineInsert(String, usize, TimelineEntry),
    TimelineSet(String, usize, TimelineEntry),
    TimelineRemove(String, usize),
    TimelineCleared(String),
    TimelineReset(String, Vec<TimelineEntry>),
    ChannelList(Vec<RoomInfo>),
    DmCreated(RoomInfo),
    HomeserverResolved(String),
    CurrentUser { username: String, matrix_id: String, display_name: Option<String>, avatar_url: Option<String> },
    PasswordRequest,
    PaginationComplete(String, bool),
    ConnectionState(ConnectionState),
    AttachmentFailed { room_id: String, file_name: String, reason: String },
    SendFailed { room_id: String, reason: String },
    /// Answers to `ResolveNameColors`, and changes nobody asked about.
    NameColors(Vec<UserNameColor>),
    Capabilities { name_color: bool },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum MumbleEvent {
    LocalSession(u32),
    UserState { session_id: u32, name: Option<String>, display_name: Option<String>, avatar_url: Option<String>, channel_id: Option<u32>, self_mute: Option<bool>, self_deaf: Option<bool>, hash: Option<String> },
    UserRemoved(u32),
    UserTalking { session_id: u32, talking: bool },
    UserVolume { session_id: u32, volume_db: f32 },
    ChannelState { id: u32, name: String, parent: u32 },
    ChannelRemoved(u32),
    TransmissionModeChanged(String),
    VadThresholdChanged(f64),
    VoiceHoldChanged(i64),
    CertificateChanged { host: String, port: u16, new_fingerprint: String },
    ConnectionState(ConnectionState),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum SystemEvent {
    ServerReset,
    ConnectionLost,
    SettingsLoaded(crate::settings::Settings),
    LogError { message: String, target: String },
    UserProfileChanged { username: String, display_name: Option<String>, avatar_url: Option<String> },
}

// internal process -> core
#[derive(Debug)]
pub enum InternalEvent {
    Matrix(InternalMatrixEvent),
    Mumble(InternalMumbleEvent),
    System(InternalSystemEvent),
}

/// Distinguishes a revoked session from transient failures the sync loop already retried through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncEnd {
    /// The session is over: the next attempt must log in again, not just re-sync.
    SessionInvalidated { reason: String },
    /// The network was out longer than the loop would wait; the session may still be valid.
    RetriesExhausted { reason: String },
}

impl SyncEnd {
    pub fn reason(&self) -> &str {
        match self {
            Self::SessionInvalidated { reason } | Self::RetriesExhausted { reason } => reason,
        }
    }
}

#[derive(Debug)]
pub enum InternalMatrixEvent {
    Connected,
    /// Nothing has been torn down; a later `SyncRecovered` may be all that follows.
    /// Each sync report carries the generation of the connect that started its session.
    SyncDegraded { generation: u64, reason: String },
    SyncRecovered { generation: u64 },
    Disconnected { generation: u64, end: SyncEnd },
    SubscribeToRoom(matrix_sdk::ruma::OwnedRoomId),
    /// A result from a superseded attempt can still arrive and is discarded by `generation`.
    ConnectFinished {
        generation: u64,
        outcome: crate::models::ConnectOutcome,
    },
    VoiceUserResolved {
        session_id: u32,
        name: String,
        volume_db: f32,
        display_name: Option<String>,
        avatar_url: Option<String>,
    },
}

#[derive(Debug)]
pub enum InternalMumbleEvent {
    Connected,
    ConnectionLost { reason: String },
    UserJoined {
        session_id: u32,
        name: String,
        volume_db: f32,
    },
    LocalChannelChanged { channel_path: String },
    LocalMuteChanged(bool),
    LocalDeafChanged(bool),
    /// Sent before the replacement so a `Connected` from the new process is credited to
    /// this launch.
    LaunchStarted { generation: u64 },
    LaunchFinished {
        generation: u64,
        outcome: LaunchOutcome,
    },
}

#[derive(Debug)]
pub enum LaunchOutcome {
    Launched,
    Failed,
    /// Mumble was left alone and the user has been prompted.
    CertChanged,
}

#[derive(Debug)]
pub enum InternalSystemEvent {
    /// Answered once the engine has drained every earlier event; sent on the internal
    /// channel so the answer is exact.
    #[cfg(test)]
    Barrier(tokio::sync::oneshot::Sender<()>),
}
