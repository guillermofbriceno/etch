use serde::Serialize;
use crate::models::{ConnectionState, RoomInfo, ServerBookmark};
use crate::matrix::timeline::TimelineEntry;

// core -> gui
#[derive(Serialize)]
#[serde(tag = "type", content = "data")]
pub enum CoreEvent {
    Matrix(MatrixEvent),
    Mumble(MumbleEvent),
    System(SystemEvent),
}

#[derive(Serialize)]
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

#[derive(Serialize)]
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
    ConnectionState(ConnectionState),
}

#[derive(Serialize)]
#[serde(tag = "type", content = "data")]
pub enum SystemEvent {
    ConnectionLost,
    BookmarksLoaded(Vec<ServerBookmark>),
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

#[derive(Debug)]
pub enum InternalMatrixEvent {
    Connected,
    Disconnected(String),
}

#[derive(Debug)]
pub enum InternalMumbleEvent {
    Connected,
    ConnectionLost { reason: String },
    UserJoined {
        session_id: u32,
        name: String,
        channel_id: u32,
        self_mute: bool,
        self_deaf: bool,
        volume_db: f32,
    },
}

#[derive(Debug)]
pub enum InternalSystemEvent {
}
