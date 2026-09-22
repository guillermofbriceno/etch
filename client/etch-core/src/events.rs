use serde::Serialize;
use crate::models::{ConnectionState, RoomInfo};
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

/// Why the Matrix sync loop stopped.
///
/// This used to be a bare `String` on `Disconnected`, which made a revoked
/// access token and a single dropped long poll the same event. They are not
/// the same thing: one ends the session, the other ends one HTTP request.
/// Conflating them is what turned every network blip into a full teardown --
/// the engine could only read "disconnected" and had to assume the worst.
///
/// The loop now retries transient failures in place (see
/// `crate::matrix::retry`), so by the time one of these is sent the question
/// has already been settled: either the server disowned us, or retrying was
/// given a fair run and did not work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncEnd {
    /// The homeserver rejected our credentials. The session is over: the next
    /// attempt has to log in again, not merely re-sync.
    SessionInvalidated { reason: String },
    /// Retrying through transient failures ran out the policy's ceiling.
    /// Nothing here says the session is invalid -- the network was out for
    /// longer than the loop is willing to wait -- so the cold reconnect path
    /// takes over as the backstop.
    RetriesExhausted { reason: String },
}

impl SyncEnd {
    /// The text shown to the user beneath a failed connection.
    pub fn reason(&self) -> &str {
        match self {
            Self::SessionInvalidated { reason } | Self::RetriesExhausted { reason } => reason,
        }
    }
}

#[derive(Debug)]
pub enum InternalMatrixEvent {
    Connected,
    /// The sync loop hit a failure it is retrying through. Nothing has been
    /// torn down: the client, its timelines and its subscriptions are all
    /// still live, and a later `SyncRecovered` may be all that follows.
    SyncDegraded { reason: String },
    /// Sync is working again after a `SyncDegraded`, without a reconnect.
    SyncRecovered,
    /// The sync loop has stopped for good. See `SyncEnd`.
    Disconnected(SyncEnd),
    SubscribeToRoom(matrix_sdk::ruma::OwnedRoomId),
    /// How a connect the engine dispatched to the Matrix actor ended.
    ///
    /// `generation` identifies the attempt. A result the engine has moved past
    /// can still arrive here -- a superseding connect races the one it
    /// replaced -- and is discarded on arrival.
    ConnectFinished {
        generation: u64,
        outcome: crate::models::ConnectOutcome,
    },
    /// The Matrix profile behind a user who joined voice, looked up off the
    /// engine's loop. Carries the voice fields it was asked with so the engine
    /// needs no table of outstanding lookups to pair the answer back up.
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
    /// The voice actor is about to kill the running Mumble and spawn a new
    /// one. Sent before the replacement rather than after it so that a
    /// `Connected` from the process coming up is credited to this launch.
    LaunchStarted { generation: u64 },
    /// How a launch the engine dispatched ended. The credentials stay with
    /// the engine, which dispatched them; only the verdict travels back.
    LaunchFinished {
        generation: u64,
        outcome: LaunchOutcome,
    },
}

/// How a voice launch ended. The credentials it was for stay with the engine,
/// which dispatched it; only the verdict travels back.
#[derive(Debug)]
pub enum LaunchOutcome {
    Launched,
    Failed,
    /// The server presented a certificate other than the one stored for it.
    /// Mumble was left alone and the user has been prompted.
    CertChanged,
}

#[derive(Debug)]
pub enum InternalSystemEvent {
    /// Answered once the engine has drained every command sent ahead of it and
    /// has no dispatched work outstanding.
    ///
    /// Test-only. Now that a connect runs concurrently with the loop, "the
    /// engine has finished with what I just sent" is no longer the same as
    /// "the call returned", and a test that needs to order a connect against
    /// what depends on it has to be able to ask. Travelling on the internal
    /// event channel rather than a channel of its own is what makes the
    /// answer exact: events already queued ahead of it are handled first.
    #[cfg(test)]
    Barrier(tokio::sync::oneshot::Sender<()>),
}
