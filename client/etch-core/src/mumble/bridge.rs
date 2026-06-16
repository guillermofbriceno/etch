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
                // The rename event from the plugin carries no parent, but the
                // ChannelState we emit fully replaces the channel on the consumer
                // side. Re-use the parent we already track so a rename does not
                // reparent the channel to root.
                let parent = self.channel_tree.get(&(id as u32)).map_or(0, |info| info.parent_id);
                if let Some(info) = self.channel_tree.get_mut(&(id as u32)) {
                    info.name = name.clone();
                }
                let _ = tx.send(mumble(MumbleEvent::ChannelState {
                    id: id as u32,
                    name,
                    parent,
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

// Behavioral tests for the BridgeEvent -> CoreEvent/InternalEvent translation
// layer. Each test drives `translate` with a single bridge event and asserts on
// both the emitted events and the resulting BridgeState, exercising the real
// state machine rather than mocked stand-ins.
#[cfg(test)]
mod translate_tests {
    use super::*;
    use bridge_types::{Channel, User};

    fn new_state() -> BridgeState {
        BridgeState::new(Arc::new(crate::scripting::ScriptDispatcher::new(
            std::path::Path::new("/tmp"),
        )))
    }

    fn drain<T>(rx: &mut mpsc::Receiver<T>) -> Vec<T> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            out.push(ev);
        }
        out
    }

    /// Drive a single event through `translate`, returning everything it emitted
    /// on the public (core) and internal channels.
    async fn feed(state: &mut BridgeState, event: BridgeEvent) -> (Vec<CoreEvent>, Vec<InternalEvent>) {
        let (tx, mut rx) = mpsc::channel(128);
        let (itx, mut irx) = mpsc::channel(128);
        state.translate(&tx, &itx, event).await;
        (drain(&mut rx), drain(&mut irx))
    }

    fn channel_states(evs: &[CoreEvent]) -> Vec<(u32, String, u32)> {
        evs.iter()
            .filter_map(|e| match e {
                CoreEvent::Mumble(MumbleEvent::ChannelState { id, name, parent }) => {
                    Some((*id, name.clone(), *parent))
                }
                _ => None,
            })
            .collect()
    }

    fn user(session: u32, channel_id: i32, mute_state: u32, deaf_state: u32) -> User {
        User {
            session,
            name: format!("user{session}"),
            channel_id,
            mute_state,
            deaf_state,
            volume_adjustment: 1.0,
        }
    }

    #[tokio::test]
    async fn server_sync_populates_state_and_announces_connected() {
        let mut state = new_state();
        let event = BridgeEvent::ServerSync {
            local_session: 7,
            channels: vec![
                Channel { id: 0, name: "Root".into(), parent: 0 },
                Channel { id: 1, name: "Voice".into(), parent: 0 },
                Channel { id: 2, name: "General".into(), parent: 1 },
            ],
            users: vec![user(7, 2, 0x02, 0x00)],
        };
        let (core, internal) = feed(&mut state, event).await;

        // State is recorded for later channel-path / membership logic.
        assert_eq!(state.local_session, Some(7));
        assert_eq!(state.session_channels.get(&7), Some(&2));
        assert_eq!(state.session_names.get(&7).map(String::as_str), Some("user7"));
        assert_eq!(state.channel_tree.get(&2).map(|c| c.parent_id), Some(1));

        // Every channel is forwarded with its real parent intact.
        let chans = channel_states(&core);
        assert!(chans.contains(&(1, "Voice".to_string(), 0)));
        assert!(chans.contains(&(2, "General".to_string(), 1)));

        // The local user's self-mute bit (0x02) is decoded; deaf bit is clear.
        let user_state = core.iter().find_map(|e| match e {
            CoreEvent::Mumble(MumbleEvent::UserState { session_id, self_mute, self_deaf, .. }) if *session_id == 7 => {
                Some((*self_mute, *self_deaf))
            }
            _ => None,
        });
        assert_eq!(user_state, Some((Some(true), Some(false))));

        // Sync is bracketed by a Connected transition on both channels.
        assert!(core.iter().any(|e| matches!(
            e,
            CoreEvent::Mumble(MumbleEvent::ConnectionState(ConnectionState::Connected))
        )));
        assert!(internal
            .iter()
            .any(|e| matches!(e, InternalEvent::Mumble(InternalMumbleEvent::Connected))));
        assert!(internal.iter().any(|e| matches!(
            e,
            InternalEvent::Mumble(InternalMumbleEvent::UserJoined { session_id: 7, .. })
        )));
    }

    #[tokio::test]
    async fn channel_rename_preserves_existing_parent() {
        let mut state = new_state();
        // A channel nested under parent 1.
        feed(
            &mut state,
            BridgeEvent::ChannelAdded {
                channel: Channel { id: 2, name: "General".into(), parent: 1 },
            },
        )
        .await;

        let (core, _) = feed(
            &mut state,
            BridgeEvent::ChannelRenamed { id: 2, name: "Lounge".into() },
        )
        .await;

        // The rename must not move the channel: parent stays 1, not 0.
        let chans = channel_states(&core);
        assert_eq!(
            chans,
            vec![(2, "Lounge".to_string(), 1)],
            "rename emitted the wrong parent (a 0 here reparents the channel to root in the UI)"
        );
        // Internal tree keeps the correct parent too.
        assert_eq!(state.channel_tree.get(&2).map(|c| c.parent_id), Some(1));
    }

    #[tokio::test]
    async fn channel_removed_drops_from_tree() {
        let mut state = new_state();
        feed(
            &mut state,
            BridgeEvent::ChannelAdded {
                channel: Channel { id: 5, name: "Temp".into(), parent: 0 },
            },
        )
        .await;
        assert!(state.channel_tree.contains_key(&5));

        let (core, _) = feed(&mut state, BridgeEvent::ChannelRemoved { id: 5 }).await;

        assert!(!state.channel_tree.contains_key(&5));
        assert!(core
            .iter()
            .any(|e| matches!(e, CoreEvent::Mumble(MumbleEvent::ChannelRemoved(5)))));
    }

    #[tokio::test]
    async fn mute_state_decodes_self_bit_only() {
        // 0x02 is MUMBLE_MS_SELF_MUTED; 0x01 is a different (server) bit and must
        // not be read as a self-mute.
        for (raw, expected) in [(0x00u32, false), (0x02, true), (0x01, false), (0x03, true)] {
            let mut state = new_state();
            state.local_session = Some(9);
            let (core, internal) =
                feed(&mut state, BridgeEvent::UserMuteStateChanged { session: 9, mute_state: raw }).await;

            let emitted = core.iter().find_map(|e| match e {
                CoreEvent::Mumble(MumbleEvent::UserState { self_mute, .. }) => Some(*self_mute),
                _ => None,
            });
            assert_eq!(emitted, Some(Some(expected)), "mute_state={raw:#04x}");

            // Local session changes are mirrored on the internal channel.
            let internal_mute = internal.iter().find_map(|e| match e {
                InternalEvent::Mumble(InternalMumbleEvent::LocalMuteChanged(v)) => Some(*v),
                _ => None,
            });
            assert_eq!(internal_mute, Some(expected), "mute_state={raw:#04x}");
        }
    }

    #[tokio::test]
    async fn remote_user_mute_change_does_not_emit_local_event() {
        let mut state = new_state();
        state.local_session = Some(1);
        let (_, internal) =
            feed(&mut state, BridgeEvent::UserMuteStateChanged { session: 2, mute_state: 0x02 }).await;
        assert!(
            !internal
                .iter()
                .any(|e| matches!(e, InternalEvent::Mumble(InternalMumbleEvent::LocalMuteChanged(_)))),
            "a remote user's mute change must not be reported as the local mute state"
        );
    }

    #[tokio::test]
    async fn local_user_move_reports_encoded_channel_path() {
        let mut state = new_state();
        state.local_session = Some(4);
        state.channel_tree.insert(0, ChannelInfo { name: "Root".into(), parent_id: 0 });
        state.channel_tree.insert(1, ChannelInfo { name: "Voice Chat".into(), parent_id: 0 });
        state.channel_tree.insert(2, ChannelInfo { name: "General".into(), parent_id: 1 });

        let (_, internal) =
            feed(&mut state, BridgeEvent::UserMoved { session: 4, channel_id: 2 }).await;

        assert_eq!(state.session_channels.get(&4), Some(&2));
        let path = internal.iter().find_map(|e| match e {
            InternalEvent::Mumble(InternalMumbleEvent::LocalChannelChanged { channel_path }) => {
                Some(channel_path.clone())
            }
            _ => None,
        });
        // Space is percent-encoded; root is omitted from the path.
        assert_eq!(path, Some("Voice%20Chat/General".to_string()));
    }

    #[tokio::test]
    async fn talking_state_passive_is_not_talking() {
        for (state_in, expected) in [
            (TalkingState::Passive, false),
            (TalkingState::Talking, true),
            (TalkingState::Whispering, true),
            (TalkingState::Shouting, true),
            (TalkingState::TalkingMuted, true),
        ] {
            let mut state = new_state();
            let (core, _) =
                feed(&mut state, BridgeEvent::UserTalking { session: 3, state: state_in }).await;
            let talking = core.iter().find_map(|e| match e {
                CoreEvent::Mumble(MumbleEvent::UserTalking { talking, .. }) => Some(*talking),
                _ => None,
            });
            assert_eq!(talking, Some(expected), "state={state_in:?}");
        }
    }

    #[tokio::test]
    async fn transmission_mode_maps_to_expected_string() {
        for (mode, expected) in [
            (TransmissionMode::VoiceActivation, "voice_activation"),
            (TransmissionMode::Continuous, "continuous"),
            (TransmissionMode::PushToTalk, "push_to_talk"),
        ] {
            let mut state = new_state();
            let (core, _) =
                feed(&mut state, BridgeEvent::TransmissionModeChanged { mode }).await;
            let s = core.iter().find_map(|e| match e {
                CoreEvent::Mumble(MumbleEvent::TransmissionModeChanged(s)) => Some(s.clone()),
                _ => None,
            });
            assert_eq!(s.as_deref(), Some(expected));
        }
    }
}
