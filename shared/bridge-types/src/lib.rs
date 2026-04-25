use serde::{Serialize, Deserialize};

// ==================== Plugin → Core ====================

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MumbleEvent {
    /// Full state snapshot, fired after Mumble finishes initial sync.
    ServerSync {
        local_session: u32,
        channels: Vec<Channel>,
        users: Vec<User>,
    },

    ServerConnected,
    ServerDisconnected,

    // --- User events ---
    UserConnected { user: User },
    UserDisconnected { session: u32 },
    UserMoved { session: u32, channel_id: i32 },
    UserTalking { session: u32, state: TalkingState },
    UserMuteStateChanged { session: u32, mute_state: u32 },
    UserDeafStateChanged { session: u32, deaf_state: u32 },

    // --- Channel events ---
    ChannelAdded { channel: Channel },
    ChannelRemoved { id: i32 },
    ChannelRenamed { id: i32, name: String },

    // --- Settings events ---
    TransmissionModeChanged { mode: TransmissionMode },
    VadThresholdChanged { value: f64 },
    VoiceHoldChanged { value: i64 },
}

// ==================== Core → Plugin ====================

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MumbleCommand {
    SwitchChannel { channel_id: i32 },
    SetMuted { muted: bool },
    SetDeafened { deafened: bool },
    SetUserVolume { session: u32, volume: f32 },
    SetTransmissionMode { mode: TransmissionMode },
    SetVadThreshold { value: f64 },
    SetVoiceHold { value: i64 },
}

// ==================== Shared types ====================

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct User {
    pub session: u32,
    pub name: String,
    pub channel_id: i32,
    pub mute_state: u32,
    pub deaf_state: u32,
    pub volume_adjustment: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Channel {
    pub id: i32,
    pub name: String,
    pub parent: i32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransmissionMode {
    VoiceActivation,
    Continuous,
    PushToTalk,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TalkingState {
    Passive,
    Talking,
    Whispering,
    Shouting,
    TalkingMuted,
}
