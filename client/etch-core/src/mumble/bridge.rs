use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use bridge_types::{MumbleCommand as BridgeCommand, MumbleEvent as BridgeEvent, TalkingState, TransmissionMode};
use crate::events::{CoreEvent, InternalEvent, InternalMumbleEvent, MumbleEvent};
use crate::models::ConnectionState;
use crate::scripting::ScriptDispatcher;
use interprocess::local_socket::{
    prelude::*,
    traits::tokio::{Listener as ListenerExt, Stream as StreamExt},
    GenericNamespaced,
    ListenerOptions,
    tokio::{Listener, RecvHalf, SendHalf},
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

/// Start a local socket listener. Returns the socket name (for passing to the
/// Mumble process via ETCH_BRIDGE_SOCK), a command sender for writing to the
/// plugin, and a task handle.
pub fn start(
    event_tx: mpsc::Sender<CoreEvent>,
    internal_tx: mpsc::Sender<InternalEvent>,
    dispatcher: Arc<ScriptDispatcher>,
) -> std::io::Result<(String, mpsc::Sender<BridgeCommand>, tokio::task::JoinHandle<()>)> {
    let sock_name = format!("etch-bridge-{}", std::process::id());
    let name = sock_name.clone().to_ns_name::<GenericNamespaced>()?;
    let listener = ListenerOptions::new()
        .name(name)
        .create_tokio()?;

    log::info!("Bridge listener started on: {}", sock_name);

    let (cmd_tx, cmd_rx) = mpsc::channel::<BridgeCommand>(64);

    let handle = tokio::spawn(async move {
        if let Err(e) = accept_loop(listener, event_tx, internal_tx, cmd_rx, dispatcher).await {
            log::error!("Bridge listener error: {}", e);
        }
    });

    Ok((sock_name, cmd_tx, handle))
}

async fn accept_loop(
    listener: Listener,
    event_tx: mpsc::Sender<CoreEvent>,
    internal_tx: mpsc::Sender<InternalEvent>,
    mut cmd_rx: mpsc::Receiver<BridgeCommand>,
    dispatcher: Arc<ScriptDispatcher>,
) -> std::io::Result<()> {
    // Accept one connection (the plugin connects once)
    let stream = listener.accept().await?;
    log::info!("Bridge plugin connected");

    let (reader, mut writer): (RecvHalf, SendHalf) = stream.split();

    // Writer task — sends commands to the plugin
    let writer_handle = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&cmd)
                && (writer.write_all(json.as_bytes()).await.is_err()
                    || writer.write_all(b"\n").await.is_err()
                    || writer.flush().await.is_err())
            {
                log::warn!("[bridge] Write to plugin failed");
                break;
            }
        }
    });

    // Reader — reads events from the plugin
    let buf_reader = BufReader::new(reader);
    let mut lines = buf_reader.lines();
    let mut state = BridgeState::new(dispatcher);

    while let Some(line) = lines.next_line().await? {
        if line.is_empty() { continue; }
        match serde_json::from_str::<BridgeEvent>(&line) {
            Ok(bridge_event) => {
                log::trace!("[bridge] {:?}", bridge_event);
                state.translate(&event_tx, &internal_tx, bridge_event).await;
            }
            Err(e) => {
                log::warn!("[bridge] Bad event: {} — {}", e, line);
            }
        }
    }

    log::info!("Bridge plugin disconnected");
    writer_handle.abort();
    let _ = event_tx.send(mumble(MumbleEvent::ConnectionState(ConnectionState::Disconnected))).await;
    Ok(())
}

// ==================== EVENT TRANSLATION ====================

fn mumble(me: MumbleEvent) -> CoreEvent {
    CoreEvent::Mumble(me)
}

fn internal_mumble(me: InternalMumbleEvent) -> InternalEvent {
    InternalEvent::Mumble(me)
}

struct ChannelInfo {
    name: String,
    parent_id: u32,
}

struct BridgeState {
    session_names: HashMap<u32, String>,
    session_channels: HashMap<u32, u32>,
    channel_tree: HashMap<u32, ChannelInfo>,
    local_session: Option<u32>,
    dispatcher: Arc<ScriptDispatcher>,
}

impl BridgeState {
    fn new(dispatcher: Arc<ScriptDispatcher>) -> Self {
        Self {
            session_names: HashMap::new(),
            session_channels: HashMap::new(),
            channel_tree: HashMap::new(),
            local_session: None,
            dispatcher,
        }
    }

    /// Build a URL-safe channel path by walking from `channel_id` up to the
    /// root (id 0). The root channel itself is omitted from the path.
    fn channel_path(&self, channel_id: u32) -> String {
        let mut segments = Vec::new();
        let mut current = channel_id;
        let mut visited = HashSet::new();
        while let Some(info) = self.channel_tree.get(&current) {
            if current == 0 || !visited.insert(current) { break; }
            segments.push(encode_path_segment(&info.name));
            current = info.parent_id;
        }
        segments.reverse();
        segments.join("/")
    }

    fn local_channel(&self) -> Option<u32> {
        self.local_session.and_then(|ls| self.session_channels.get(&ls).copied())
    }

    async fn translate(
        &mut self,
        tx: &mpsc::Sender<CoreEvent>,
        itx: &mpsc::Sender<InternalEvent>,
        event: BridgeEvent,
    ) {
        match event {
            BridgeEvent::ServerConnected => {
                let _ = tx.send(mumble(MumbleEvent::ConnectionState(ConnectionState::Connecting))).await;
            }
            BridgeEvent::ServerSync { local_session, channels, users } => {
                self.local_session = Some(local_session);
                self.channel_tree.clear();
                let _ = tx.send(mumble(MumbleEvent::LocalSession(local_session))).await;
                for ch in channels {
                    self.channel_tree.insert(ch.id as u32, ChannelInfo { name: ch.name.clone(), parent_id: ch.parent as u32 });
                    let _ = tx.send(mumble(MumbleEvent::ChannelState {
                        id: ch.id as u32,
                        name: ch.name,
                        parent: ch.parent as u32,
                    })).await;
                }
                for u in users {
                    let volume_db = 20.0 * u.volume_adjustment.log10();
                    self.session_names.insert(u.session, u.name.clone());
                    self.session_channels.insert(u.session, u.channel_id as u32);
                    let _ = tx.send(mumble(MumbleEvent::UserState {
                        session_id: u.session,
                        name: None,
                        display_name: None,
                        avatar_url: None,
                        channel_id: Some(u.channel_id as u32),
                        self_mute: Some((u.mute_state & 0x02) != 0),
                        self_deaf: Some((u.deaf_state & 0x02) != 0),
                        hash: None,
                    })).await;
                    let _ = itx.send(internal_mumble(InternalMumbleEvent::UserJoined {
                        session_id: u.session,
                        name: u.name,
                        volume_db,
                    })).await;
                }
                let _ = tx.send(mumble(MumbleEvent::ConnectionState(ConnectionState::Connected))).await;
                let _ = itx.send(internal_mumble(InternalMumbleEvent::Connected)).await;
            }
            BridgeEvent::ServerDisconnected => {
                let _ = tx.send(mumble(MumbleEvent::ConnectionState(ConnectionState::Disconnected))).await;
            }
            BridgeEvent::UserConnected { user } => {
                let volume_db = 20.0 * user.volume_adjustment.log10();
                self.session_names.insert(user.session, user.name.clone());
                self.session_channels.insert(user.session, user.channel_id as u32);
                if self.local_channel() == Some(user.channel_id as u32) {
                    self.dispatcher.fire("user_join", &[("USER", &user.name)]);
                }
                let _ = tx.send(mumble(MumbleEvent::UserState {
                    session_id: user.session,
                    name: None,
                    display_name: None,
                    avatar_url: None,
                    channel_id: Some(user.channel_id as u32),
                    self_mute: Some((user.mute_state & 0x02) != 0),
                    self_deaf: Some((user.deaf_state & 0x02) != 0),
                    hash: None,
                })).await;
                let _ = itx.send(internal_mumble(InternalMumbleEvent::UserJoined {
                    session_id: user.session,
                    name: user.name,
                    volume_db,
                })).await;
            }
            BridgeEvent::UserDisconnected { session } => {
                let was_in_local_ch = {
                    let local_ch = self.local_channel();
                    let user_ch = self.session_channels.remove(&session);
                    local_ch.is_some() && local_ch == user_ch
                };
                if let Some(name) = self.session_names.remove(&session)
                    && was_in_local_ch
                {
                    self.dispatcher.fire("user_leave", &[("USER", &name)]);
                }
                let _ = tx.send(mumble(MumbleEvent::UserRemoved(session))).await;
            }
            BridgeEvent::UserMoved { session, channel_id } => {
                let new_ch = channel_id as u32;
                let old_ch = self.session_channels.insert(session, new_ch);
                let local_ch = self.local_channel();

                // Only fire scripts for other users moving in/out of our channel
                if Some(session) != self.local_session
                    && let Some(local_ch) = local_ch
                    && let Some(name) = self.session_names.get(&session)
                {
                    if new_ch == local_ch && old_ch != Some(local_ch) {
                        self.dispatcher.fire("user_join", &[("USER", name)]);
                    } else if old_ch == Some(local_ch) && new_ch != local_ch {
                        self.dispatcher.fire("user_leave", &[("USER", name)]);
                    }
                }

                // Track local user's channel for reconnect restoration
                if Some(session) == self.local_session {
                    let path = self.channel_path(new_ch);
                    let _ = itx.send(internal_mumble(InternalMumbleEvent::LocalChannelChanged { channel_path: path })).await;
                }

                let _ = tx.send(mumble(MumbleEvent::UserState {
                    session_id: session,
                    name: None,
                    display_name: None,
                    avatar_url: None,
                    channel_id: Some(new_ch),
                    self_mute: None,
                    self_deaf: None,
                    hash: None,
                })).await;
            }
            BridgeEvent::UserTalking { session, state } => {
                let talking = !matches!(state, TalkingState::Passive);
                let _ = tx.send(mumble(MumbleEvent::UserTalking {
                    session_id: session,
                    talking,
                })).await;
            }
            BridgeEvent::UserMuteStateChanged { session, mute_state } => {
                // Bit 1 (0x02) = MUMBLE_MS_SELF_MUTED
                let self_mute = (mute_state & 0x02) != 0;
                if Some(session) == self.local_session {
                    let _ = itx.send(internal_mumble(InternalMumbleEvent::LocalMuteChanged(self_mute))).await;
                }
                let _ = tx.send(mumble(MumbleEvent::UserState {
                    session_id: session,
                    name: None,
                    display_name: None,
                    avatar_url: None,
                    channel_id: None,
                    self_mute: Some(self_mute),
                    self_deaf: None,
                    hash: None,
                })).await;
            }
            BridgeEvent::UserDeafStateChanged { session, deaf_state } => {
                // Bit 1 (0x02) = MUMBLE_DS_SELF_DEAFENED
                let self_deaf = (deaf_state & 0x02) != 0;
                if Some(session) == self.local_session {
                    let _ = itx.send(internal_mumble(InternalMumbleEvent::LocalDeafChanged(self_deaf))).await;
                }
                let _ = tx.send(mumble(MumbleEvent::UserState {
                    session_id: session,
                    name: None,
                    display_name: None,
                    avatar_url: None,
                    channel_id: None,
                    self_mute: None,
                    self_deaf: Some(self_deaf),
                    hash: None,
                })).await;
            }
            BridgeEvent::ChannelAdded { channel } => {
                self.channel_tree.insert(channel.id as u32, ChannelInfo { name: channel.name.clone(), parent_id: channel.parent as u32 });
                let _ = tx.send(mumble(MumbleEvent::ChannelState {
                    id: channel.id as u32,
                    name: channel.name,
                    parent: channel.parent as u32,
                })).await;
            }
            BridgeEvent::ChannelRemoved { id } => {
                self.channel_tree.remove(&(id as u32));
                let _ = tx.send(mumble(MumbleEvent::ChannelRemoved(id as u32))).await;
            }
            BridgeEvent::ChannelRenamed { id, name } => {
                if let Some(info) = self.channel_tree.get_mut(&(id as u32)) {
                    info.name = name.clone();
                }
                let _ = tx.send(mumble(MumbleEvent::ChannelState {
                    id: id as u32,
                    name,
                    parent: 0,
                })).await;
            }
            BridgeEvent::TransmissionModeChanged { mode } => {
                let mode_str = match mode {
                    TransmissionMode::VoiceActivation => "voice_activation",
                    TransmissionMode::Continuous => "continuous",
                    TransmissionMode::PushToTalk => "push_to_talk",
                };
                let _ = tx.send(mumble(MumbleEvent::TransmissionModeChanged(mode_str.to_string()))).await;
            }
            BridgeEvent::VadThresholdChanged { value } => {
                let _ = tx.send(mumble(MumbleEvent::VadThresholdChanged(value))).await;
            }
            BridgeEvent::VoiceHoldChanged { value } => {
                let _ = tx.send(mumble(MumbleEvent::VoiceHoldChanged(value))).await;
            }
        }
    }
}

/// Percent-encode a single URL path segment, preserving unreserved characters
/// per RFC 3986 (alphanumeric, `-`, `.`, `_`, `~`).
fn encode_path_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push(char::from(HEX[(byte >> 4) as usize]));
                out.push(char::from(HEX[(byte & 0x0f) as usize]));
            }
        }
    }
    out
}

const HEX: [u8; 16] = *b"0123456789ABCDEF";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_unreserved_passthrough() {
        let input = "AZaz09-._~";
        assert_eq!(encode_path_segment(input), input);
    }

    #[test]
    fn encode_space_and_slash() {
        assert_eq!(encode_path_segment("hello world"), "hello%20world");
        assert_eq!(encode_path_segment("a/b"), "a%2Fb");
    }

    #[test]
    fn encode_multibyte_utf8() {
        // Euro sign U+20AC is 3 bytes: 0xE2, 0x82, 0xAC
        assert_eq!(encode_path_segment("€"), "%E2%82%AC");
    }

    #[test]
    fn encode_empty_string() {
        assert_eq!(encode_path_segment(""), "");
    }

    #[test]
    fn channel_path_stops_at_root() {
        let mut state = BridgeState::new(Arc::new(crate::scripting::ScriptDispatcher::new(std::path::Path::new("/tmp"))));
        state.channel_tree.insert(0, ChannelInfo { name: "Root".into(), parent_id: 0 });
        state.channel_tree.insert(1, ChannelInfo { name: "Voice".into(), parent_id: 0 });
        state.channel_tree.insert(2, ChannelInfo { name: "General".into(), parent_id: 1 });
        assert_eq!(state.channel_path(2), "Voice/General");
        assert_eq!(state.channel_path(1), "Voice");
        assert_eq!(state.channel_path(0), "");
    }

    #[test]
    fn channel_path_handles_cycle() {
        let mut state = BridgeState::new(Arc::new(crate::scripting::ScriptDispatcher::new(std::path::Path::new("/tmp"))));
        state.channel_tree.insert(3, ChannelInfo { name: "A".into(), parent_id: 5 });
        state.channel_tree.insert(5, ChannelInfo { name: "B".into(), parent_id: 3 });
        // Should terminate instead of looping forever
        let path = state.channel_path(3);
        assert!(!path.is_empty());
    }
}
