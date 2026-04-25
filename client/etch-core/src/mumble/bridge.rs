use bridge_types::{MumbleCommand as BridgeCommand, MumbleEvent as BridgeEvent, TalkingState, TransmissionMode};
use crate::events::{CoreEvent, InternalEvent, InternalMumbleEvent, MumbleEvent};
use crate::models::ConnectionState;
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
) -> std::io::Result<(String, mpsc::Sender<BridgeCommand>, tokio::task::JoinHandle<()>)> {
    let sock_name = format!("etch-bridge-{}", std::process::id());
    let name = sock_name.clone().to_ns_name::<GenericNamespaced>()?;
    let listener = ListenerOptions::new()
        .name(name)
        .create_tokio()?;

    log::info!("Bridge listener started on: {}", sock_name);

    let (cmd_tx, cmd_rx) = mpsc::channel::<BridgeCommand>(64);

    let handle = tokio::spawn(async move {
        if let Err(e) = accept_loop(listener, event_tx, internal_tx, cmd_rx).await {
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
) -> std::io::Result<()> {
    // Accept one connection (the plugin connects once)
    let stream = listener.accept().await?;
    log::info!("Bridge plugin connected");

    let (reader, mut writer): (RecvHalf, SendHalf) = stream.split();

    // Writer task — sends commands to the plugin
    let writer_handle = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&cmd) {
                if writer.write_all(json.as_bytes()).await.is_err()
                    || writer.write_all(b"\n").await.is_err()
                    || writer.flush().await.is_err()
                {
                    log::warn!("[bridge] Write to plugin failed");
                    break;
                }
            }
        }
    });

    // Reader — reads events from the plugin
    let buf_reader = BufReader::new(reader);
    let mut lines = buf_reader.lines();

    while let Some(line) = lines.next_line().await? {
        if line.is_empty() { continue; }
        match serde_json::from_str::<BridgeEvent>(&line) {
            Ok(bridge_event) => {
                log::trace!("[bridge] {:?}", bridge_event);
                translate(&event_tx, &internal_tx, bridge_event).await;
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

async fn translate(tx: &mpsc::Sender<CoreEvent>, itx: &mpsc::Sender<InternalEvent>, event: BridgeEvent) {
    match event {
        BridgeEvent::ServerConnected => {
            let _ = tx.send(mumble(MumbleEvent::ConnectionState(ConnectionState::Connecting))).await;
        }
        BridgeEvent::ServerSync { local_session, channels, users } => {
            let _ = tx.send(mumble(MumbleEvent::LocalSession(local_session))).await;
            for ch in channels {
                let _ = tx.send(mumble(MumbleEvent::ChannelState {
                    id: ch.id as u32,
                    name: ch.name,
                    parent: ch.parent as u32,
                })).await;
            }
            for u in users {
                let volume_db = 20.0 * u.volume_adjustment.log10();
                let _ = itx.send(internal_mumble(InternalMumbleEvent::UserJoined {
                    session_id: u.session,
                    name: u.name,
                    channel_id: u.channel_id as u32,
                    self_mute: (u.mute_state & 0x02) != 0,
                    self_deaf: (u.deaf_state & 0x02) != 0,
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
            let _ = itx.send(internal_mumble(InternalMumbleEvent::UserJoined {
                session_id: user.session,
                name: user.name,
                channel_id: user.channel_id as u32,
                self_mute: (user.mute_state & 0x02) != 0,
                self_deaf: (user.deaf_state & 0x02) != 0,
                volume_db,
            })).await;
        }
        BridgeEvent::UserDisconnected { session } => {
            let _ = tx.send(mumble(MumbleEvent::UserRemoved(session))).await;
        }
        BridgeEvent::UserMoved { session, channel_id } => {
            let _ = tx.send(mumble(MumbleEvent::UserState {
                session_id: session,
                name: None,
                display_name: None,
                avatar_url: None,
                channel_id: Some(channel_id as u32),
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
            let _ = tx.send(mumble(MumbleEvent::ChannelState {
                id: channel.id as u32,
                name: channel.name,
                parent: channel.parent as u32,
            })).await;
        }
        BridgeEvent::ChannelRemoved { id } => {
            let _ = tx.send(mumble(MumbleEvent::ChannelRemoved(id as u32))).await;
        }
        BridgeEvent::ChannelRenamed { id, name } => {
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
