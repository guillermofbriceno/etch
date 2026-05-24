use tokio::sync::mpsc;
use tokio::time::{sleep, Duration, Sleep};
use crate::connection::MatrixConnection;
use crate::events::{CoreEvent, InternalEvent, InternalMatrixEvent, InternalMumbleEvent, MumbleEvent, SystemEvent};
use crate::commands::{CoreCommand, MumbleCommand, ServerConnectionForm, SystemCommand};
use crate::models::{ConnectionState, VoiceServerConfig};
use crate::settings;
use crate::traits::{MatrixBackend, VoiceService};

use std::path::PathBuf;
use std::pin::Pin;

/// Voice state tracked in memory for restoration after Mumble restarts.
/// Reset when connecting to a new server; preserved across process restarts
/// on the same server.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct VoiceSessionState {
    pub channel_path: Option<String>,
    pub muted: bool,
    pub deafened: bool,
}

pub struct CoreEngine<M, V> {
    pub(crate) cmd_rx: mpsc::Receiver<CoreCommand>,
    pub(crate) event_tx: mpsc::Sender<CoreEvent>,

    matrix: M,
    voice: V,
    conn: MatrixConnection,
    mumble_credentials: Option<VoiceServerConfig>,
    pub(crate) data_dir: PathBuf,
    /// Stashed launch params while waiting for user to accept a changed cert.
    pending_cert_launch: Option<(VoiceServerConfig, bool, String, mpsc::Sender<InternalEvent>)>,
    /// Voice state persisted across Mumble client restarts.
    pub(crate) voice_session: VoiceSessionState,
}

pub struct CoreHandle {
    pub cmd_tx: mpsc::Sender<CoreCommand>,
    pub event_rx: mpsc::Receiver<CoreEvent>,
}

impl<M: MatrixBackend, V: VoiceService> CoreEngine<M, V> {
    pub fn new(
        cmd_rx: mpsc::Receiver<CoreCommand>,
        event_tx: mpsc::Sender<CoreEvent>,
        matrix: M,
        voice: V,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            cmd_rx,
            event_tx,
            matrix,
            voice,
            conn: MatrixConnection::new(),
            mumble_credentials: None,
            data_dir,
            pending_cert_launch: None,
            voice_session: VoiceSessionState::default(),
        }
    }

    pub async fn run(mut self) {
        let (internal_tx, mut internal_rx) = mpsc::channel::<InternalEvent>(100);
        let mut retry_timer: Pin<Box<Sleep>> = Box::pin(sleep(Duration::MAX));

        loop {
            tokio::select! {
                cmd = self.cmd_rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    match cmd {
                        CoreCommand::Matrix(matrix_cmd) => {
                            self.matrix.handle_command(matrix_cmd).await;
                        }
                        CoreCommand::Mumble(mumble_cmd) => {
                            self.dispatch_mumble_command(mumble_cmd).await;
                        }
                        CoreCommand::FetchMedia { mxc_url, respond } => {
                            self.matrix.spawn_media_fetch(mxc_url, respond);
                        }
                        CoreCommand::System(cmd) => {
                            self.handle_system_command(cmd, &mut retry_timer, internal_tx.clone()).await;
                        }
                    }
                    // Drain internal events that arrived during command processing
                    // so they're handled before the next command.
                    while let Ok(event) = internal_rx.try_recv() {
                        self.handle_internal_event(event, &mut retry_timer).await;
                    }
                }

                Some(internal_event) = internal_rx.recv() => {
                    self.handle_internal_event(internal_event, &mut retry_timer).await;
                }

                // --- Retry timer ---

                _ = &mut retry_timer, if self.conn.state.is_failed() => {
                    if let Some(form) = self.conn.form.clone() {
                        log::info!("Retrying Matrix connection (attempt {})", self.conn.state.retries() + 1);
                        self.connect_to_server(&form, &mut retry_timer, internal_tx.clone()).await;
                    }
                }
            }
        }

        // Drain any internal events that arrived during the last command.
        // Without this, events queued by Matrix/Mumble services during
        // command processing could be dropped on shutdown.
        while let Ok(event) = internal_rx.try_recv() {
            self.handle_internal_event(event, &mut retry_timer).await;
        }
    }

    async fn handle_system_command(
        &mut self,
        cmd: SystemCommand,
        retry_timer: &mut Pin<Box<Sleep>>,
        internal_tx: mpsc::Sender<InternalEvent>,
    ) {
        match cmd {
            SystemCommand::ConnectToServer(form) => {
                self.voice_session = VoiceSessionState::default();
                self.conn.form = Some(form.clone());
                self.connect_to_server(&form, retry_timer, internal_tx).await;
            }
            SystemCommand::LoadSettings => {
                let s = settings::load(&self.data_dir);
                let _ = self.event_tx.send(CoreEvent::System(
                    SystemEvent::SettingsLoaded(s.clone()),
                )).await;

                if let Some(bm) = s.bookmarks.iter().find(|b| b.auto_connect) {
                    let form = ServerConnectionForm::from(bm);
                    self.conn.form = Some(form.clone());
                    self.connect_to_server(&form, retry_timer, internal_tx).await;
                }
            }
            SystemCommand::SaveBookmarks(bookmarks) => {
                settings::update_bookmarks(&self.data_dir, bookmarks);
                let s = settings::load(&self.data_dir);
                let _ = self.event_tx.send(CoreEvent::System(
                    SystemEvent::SettingsLoaded(s),
                )).await;
            }
            SystemCommand::MuteMic(muted) => {
                self.voice.send_command(MumbleCommand::MuteSelf(muted)).await;
            }
            SystemCommand::Deafen(deafened) => {
                self.voice.send_command(MumbleCommand::DeafenSelf(deafened)).await;
            }
            SystemCommand::OpenMumbleGui(extra_args) => {
                if let Some(ref creds) = self.mumble_credentials {
                    self.launch_voice(creds.clone(), true, &extra_args, internal_tx).await;
                }
            }
            SystemCommand::RestartMumble(extra_args) => {
                if let Some(ref creds) = self.mumble_credentials {
                    self.launch_voice(creds.clone(), false, &extra_args, internal_tx).await;
                }
            }
            SystemCommand::SetLogLevel(level) => {
                crate::logger::set_level(&level);
            }
            SystemCommand::TestError => {
                log::error!("Test error triggered from Developer Options");
            }
            SystemCommand::SetDeafenSuppressesNotifs(value) => {
                settings::set_deafen_suppresses_notifs(&self.data_dir, value);
            }
            SystemCommand::HideDm { room_id } => {
                settings::hide_dm(&self.data_dir, room_id);
            }
            SystemCommand::UnhideDm { room_id } => {
                settings::unhide_dm(&self.data_dir, &room_id);
            }
            SystemCommand::AcceptMumbleCert { host, port, fingerprint } => {
                let db_path = self.data_dir.join("mumble/mumble.sqlite");
                if let Err(e) = crate::mumble::cert::store_cert(&db_path, &host, port, &fingerprint) {
                    log::error!("Failed to store accepted cert: {:?}", e);
                    return;
                }
                log::info!("User accepted new cert for {}:{}", host, port);
                // Resume the stashed voice launch
                if let Some((creds, show_gui, extra_args, itx)) = self.pending_cert_launch.take()
                    && let Err(e) = self.voice.launch(creds, itx, show_gui, &extra_args, self.voice_session.channel_path.as_deref()).await
                {
                    log::error!("Failed to launch voice after cert accept: {:?}", e);
                }
            }
        }
    }

    async fn handle_internal_event(
        &mut self,
        event: InternalEvent,
        retry_timer: &mut Pin<Box<Sleep>>,
    ) {
        match event {
            InternalEvent::Matrix(evt) => {
                match evt {
                    InternalMatrixEvent::Connected => {
                        log::debug!("Internal: Matrix connected");
                    }
                    InternalMatrixEvent::SubscribeToRoom(room_id) => {
                        self.matrix.subscribe_to_room(room_id.as_str()).await;
                    }
                    InternalMatrixEvent::Disconnected(reason) => {
                        log::warn!("Matrix disconnected: {}", reason);
                        self.conn.schedule_retry(retry_timer, reason, &self.event_tx).await;
                    }
                }
            }
            InternalEvent::Mumble(evt) => {
                match evt {
                    InternalMumbleEvent::UserJoined { session_id, name, volume_db } => {
                        let (display_name, avatar_url) = self.matrix.resolve_user_profile(&name).await;
                        let _ = self.event_tx.send(CoreEvent::Mumble(MumbleEvent::UserState {
                            session_id,
                            name: Some(name),
                            display_name,
                            avatar_url,
                            channel_id: None,
                            self_mute: None,
                            self_deaf: None,
                            hash: None,
                        })).await;
                        let _ = self.event_tx.send(CoreEvent::Mumble(MumbleEvent::UserVolume {
                            session_id,
                            volume_db,
                        })).await;
                    }
                    InternalMumbleEvent::Connected => {
                        let s = settings::load(&self.data_dir);
                        if s.use_mumble_settings != Some(true) {
                            if let Some(mode) = s.transmission_mode {
                                self.voice.send_command(MumbleCommand::SetTransmissionMode(mode)).await;
                            }
                            if let Some(value) = s.vad_threshold {
                                self.voice.send_command(MumbleCommand::SetVadThreshold(value)).await;
                            }
                            if let Some(value) = s.voice_hold {
                                self.voice.send_command(MumbleCommand::SetVoiceHold(value)).await;
                            }
                        }
                        // Restore mute/deafen from previous session.
                        // Deafen implies mute in Mumble, so only send one.
                        if self.voice_session.deafened {
                            self.voice.send_command(MumbleCommand::DeafenSelf(true)).await;
                        } else if self.voice_session.muted {
                            self.voice.send_command(MumbleCommand::MuteSelf(true)).await;
                        }
                    }
                    InternalMumbleEvent::LocalChannelChanged { channel_path } => {
                        self.voice_session.channel_path = Some(channel_path);
                    }
                    InternalMumbleEvent::LocalMuteChanged(muted) => {
                        self.voice_session.muted = muted;
                    }
                    InternalMumbleEvent::LocalDeafChanged(deafened) => {
                        self.voice_session.deafened = deafened;
                    }
                    _ => {
                        log::debug!("Internal Mumble event: {:?}", evt);
                    }
                }
            }
            InternalEvent::System(evt) => {
                log::debug!("Internal System event: {:?}", evt);
            }
        }
    }

    async fn dispatch_mumble_command(&mut self, cmd: MumbleCommand) {
        // Persist settings regardless of whether Mumble is connected
        match &cmd {
            MumbleCommand::SetTransmissionMode(mode) => settings::set_transmission_mode(&self.data_dir, mode.clone()),
            MumbleCommand::SetVadThreshold(value) => settings::set_vad_threshold(&self.data_dir, *value),
            MumbleCommand::SetVoiceHold(value) => settings::set_voice_hold(&self.data_dir, *value),
            MumbleCommand::SetUseMumbleSettings(value) => {
                settings::set_use_mumble_settings(&self.data_dir, *value);
                return;
            }
            _ => {}
        }

        self.voice.send_command(cmd).await;
    }

    async fn connect_to_server(
        &mut self,
        form: &ServerConnectionForm,
        retry_timer: &mut Pin<Box<Sleep>>,
        internal_tx: mpsc::Sender<InternalEvent>,
    ) {
        let _ = self.event_tx.send(CoreEvent::System(SystemEvent::ServerReset)).await;
        self.matrix.reset().await;

        if form.mumble_host.is_some() {
            // Explicit Mumble config in bookmark: launch immediately, don't wait for Matrix
            self.resolve_and_launch_voice(form, None, false, "", internal_tx.clone()).await;
            self.conn.attempt_connect(
                retry_timer, &mut self.matrix, form.clone(),
                internal_tx, &self.event_tx,
            ).await;
        } else {
            // No explicit Mumble config: wait for Matrix to discover voice server
            let voice_server = self.conn.attempt_connect(
                retry_timer, &mut self.matrix, form.clone(),
                internal_tx.clone(), &self.event_tx,
            ).await;
            if matches!(self.conn.state, ConnectionState::Connected) {
                self.resolve_and_launch_voice(form, voice_server, false, "", internal_tx).await;
            }
        }
    }

    pub(crate) fn resolve_mumble_credentials(
        form: &ServerConnectionForm,
        voice_server: Option<VoiceServerConfig>,
    ) -> VoiceServerConfig {
        // Priority: bookmark explicit > state event > fallback defaults
        VoiceServerConfig {
            host: form.mumble_host.clone()
                .or_else(|| voice_server.as_ref().map(|vs| vs.host.clone()))
                .unwrap_or_else(|| form.hostname.clone()),
            port: form.mumble_port
                .or_else(|| voice_server.as_ref().map(|vs| vs.port))
                .unwrap_or(64738),
            username: Some(form.mumble_username.clone()
                .unwrap_or_else(|| form.username.clone())),
            password: form.mumble_password.clone()
                .or_else(|| voice_server.and_then(|vs| vs.password)),
        }
    }

    async fn resolve_and_launch_voice(
        &mut self,
        form: &ServerConnectionForm,
        voice_server: Option<VoiceServerConfig>,
        show_gui: bool,
        extra_args: &str,
        internal_tx: mpsc::Sender<InternalEvent>,
    ) {
        let creds = Self::resolve_mumble_credentials(form, voice_server);
        self.mumble_credentials = Some(creds.clone());
        self.launch_voice(creds, show_gui, extra_args, internal_tx).await;
    }

    async fn launch_voice(&mut self, creds: VoiceServerConfig, show_gui: bool, extra_args: &str, internal_tx: mpsc::Sender<InternalEvent>) {
        // Pre-check: probe the server certificate and compare against stored value
        let db_path = self.data_dir.join("mumble/mumble.sqlite");
        match crate::mumble::cert::probe_server_cert(&creds.host, creds.port).await {
            Ok(fingerprint) => {
                let stored = crate::mumble::cert::get_stored_cert(&db_path, &creds.host, creds.port);
                match stored {
                    None => {
                        // TOFU: first time seeing this server, store and proceed
                        log::info!("First connection to {}:{}, storing cert fingerprint", creds.host, creds.port);
                        if let Err(e) = crate::mumble::cert::store_cert(&db_path, &creds.host, creds.port, &fingerprint) {
                            log::warn!("Failed to store cert: {:?}", e);
                        }
                    }
                    Some(ref stored_fp) if stored_fp == &fingerprint => {
                        log::debug!("Cert fingerprint matches for {}:{}", creds.host, creds.port);
                    }
                    Some(_) => {
                        // Certificate has changed -- prompt the user
                        log::warn!("Certificate changed for {}:{}, awaiting user approval", creds.host, creds.port);
                        let _ = self.event_tx.send(CoreEvent::Mumble(MumbleEvent::CertificateChanged {
                            host: creds.host.clone(),
                            port: creds.port,
                            new_fingerprint: fingerprint,
                        })).await;
                        // Stash credentials so AcceptMumbleCert can resume the launch
                        self.pending_cert_launch = Some((creds, show_gui, extra_args.to_string(), internal_tx));
                        return;
                    }
                }
            }
            Err(e) => {
                // Probe failed (network issue, etc.) -- log and proceed anyway
                log::warn!("Cert probe failed for {}:{}: {:?}", creds.host, creds.port, e);
            }
        }

        if let Err(e) = self.voice.launch(creds, internal_tx, show_gui, extra_args, self.voice_session.channel_path.as_deref()).await {
            log::error!("Failed to launch voice: {:?}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_mocks::{MockMatrix, MockMatrixState, MockVoice, MockVoiceState};
    use crate::models::ConnectOutcome;
    use crate::commands::*;
    use crate::events::*;
    use crate::settings;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tokio::time::timeout;
    use std::time::Duration;

    /// Send commands to the engine, wait for it to process them all, then
    /// collect emitted events.
    ///
    /// Dropping the command sender causes `run()` to break out of the select
    /// loop after processing all buffered commands, making this fully
    /// deterministic with no sleeps.
    async fn run_commands(
        matrix: MockMatrix,
        voice: MockVoice,
        data_dir: &std::path::Path,
        commands: Vec<CoreCommand>,
    ) -> (Vec<CoreEvent>, Arc<MockMatrixState>, Arc<MockVoiceState>) {
        let matrix_state = matrix.state.clone();
        let voice_state = voice.state.clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let (event_tx, mut event_rx) = mpsc::channel(100);

        let engine = CoreEngine::new(cmd_rx, event_tx, matrix, voice, data_dir.to_path_buf());
        let engine_handle = tokio::spawn(async move { engine.run().await });

        for cmd in commands {
            cmd_tx.send(cmd).await.unwrap();
        }
        drop(cmd_tx);

        timeout(Duration::from_secs(2), engine_handle)
            .await
            .expect("engine did not shut down within 2s")
            .expect("engine task panicked");

        let mut events = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            events.push(event);
        }

        (events, matrix_state, voice_state)
    }

    #[tokio::test]
    async fn load_settings_emits_settings_loaded() {
        let tmp = tempfile::tempdir().unwrap();
        let (events, _, _) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::LoadSettings)],
        ).await;

        let has_settings_loaded = events.iter().any(|e| matches!(e, CoreEvent::System(SystemEvent::SettingsLoaded(_))));
        assert!(has_settings_loaded, "Expected SettingsLoaded event");
    }

    #[tokio::test]
    async fn save_bookmarks_persists_and_emits() {
        let tmp = tempfile::tempdir().unwrap();
        let bookmark = crate::models::ServerBookmark {
            id: "test".into(),
            label: "Test Server".into(),
            address: "example.com".into(),
            port: 8448,
            username: "alice".into(),
            auto_connect: false,
            mumble_host: None,
            mumble_port: None,
            mumble_username: None,
            mumble_password: None,
        };

        let (events, _, _) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::SaveBookmarks(vec![bookmark]))],
        ).await;

        // Should emit SettingsLoaded
        let has_settings = events.iter().any(|e| matches!(e, CoreEvent::System(SystemEvent::SettingsLoaded(_))));
        assert!(has_settings, "Expected SettingsLoaded event after SaveBookmarks");

        // Should persist to disk
        let loaded = settings::load(tmp.path());
        assert_eq!(loaded.bookmarks.len(), 1);
        assert_eq!(loaded.bookmarks[0].label, "Test Server");
    }

    #[tokio::test]
    async fn hide_dm_persists_to_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let _ = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![
                CoreCommand::System(SystemCommand::HideDm { room_id: "!room1:example.com".into() }),
                CoreCommand::System(SystemCommand::HideDm { room_id: "!room2:example.com".into() }),
            ],
        ).await;

        let loaded = settings::load(tmp.path());
        assert_eq!(loaded.hidden_dms.len(), 2);
        assert!(loaded.hidden_dms.contains(&"!room1:example.com".to_string()));
    }

    #[tokio::test]
    async fn unhide_dm_removes_from_settings() {
        let tmp = tempfile::tempdir().unwrap();
        // Pre-populate with hidden DMs
        settings::hide_dm(tmp.path(), "!room1:example.com".into());
        settings::hide_dm(tmp.path(), "!room2:example.com".into());

        let _ = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::UnhideDm { room_id: "!room1:example.com".into() })],
        ).await;

        let loaded = settings::load(tmp.path());
        assert_eq!(loaded.hidden_dms.len(), 1);
        assert_eq!(loaded.hidden_dms[0], "!room2:example.com");
    }

    #[tokio::test]
    async fn set_transmission_mode_persists_and_forwards() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, _, voice) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::Mumble(MumbleCommand::SetTransmissionMode("continuous".into()))],
        ).await;

        // Should persist
        let loaded = settings::load(tmp.path());
        assert_eq!(loaded.transmission_mode.as_deref(), Some("continuous"));

        // Should forward to voice
        let cmds = voice.commands.lock().unwrap();
        assert!(cmds.iter().any(|c| matches!(c, MumbleCommand::SetTransmissionMode(m) if m == "continuous")));
    }

    #[tokio::test]
    async fn set_use_mumble_settings_persists_without_forwarding() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, _, voice) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::Mumble(MumbleCommand::SetUseMumbleSettings(true))],
        ).await;

        // Should persist
        let loaded = settings::load(tmp.path());
        assert_eq!(loaded.use_mumble_settings, Some(true));

        // Should NOT forward to voice
        let cmds = voice.commands.lock().unwrap();
        assert!(cmds.is_empty(), "SetUseMumbleSettings should not be forwarded to voice");
    }

    #[tokio::test]
    async fn mute_mic_forwards_to_voice() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, _, voice) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::MuteMic(true))],
        ).await;

        let cmds = voice.commands.lock().unwrap();
        assert!(cmds.iter().any(|c| matches!(c, MumbleCommand::MuteSelf(true))));
    }

    #[tokio::test]
    async fn deafen_forwards_to_voice() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, _, voice) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::Deafen(true))],
        ).await;

        let cmds = voice.commands.lock().unwrap();
        assert!(cmds.iter().any(|c| matches!(c, MumbleCommand::DeafenSelf(true))));
    }

    #[tokio::test]
    async fn fetch_media_delegates_to_matrix() {
        let tmp = tempfile::tempdir().unwrap();
        let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();

        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let (event_tx, _event_rx) = mpsc::channel(100);
        let matrix = MockMatrix::new();
        let voice = MockVoice::new();
        let engine = CoreEngine::new(cmd_rx, event_tx, matrix, voice, tmp.path().to_path_buf());

        let engine_handle = tokio::spawn(async move { engine.run().await });

        cmd_tx.send(CoreCommand::FetchMedia {
            mxc_url: "mxc://example.com/abc".into(),
            respond: respond_tx,
        }).await.unwrap();
        drop(cmd_tx);

        // oneshot resolves once the engine processes the command
        let result = timeout(Duration::from_secs(2), respond_rx).await
            .expect("timed out waiting for media response")
            .expect("oneshot dropped");

        assert_eq!(result.unwrap(), vec![0xDE, 0xAD]);

        timeout(Duration::from_secs(2), engine_handle)
            .await
            .expect("engine did not shut down")
            .expect("engine task panicked");
    }

    #[tokio::test]
    async fn connect_success_launches_voice() {
        let tmp = tempfile::tempdir().unwrap();
        let form = ServerConnectionForm {
            username: "alice".into(),
            hostname: "matrix.example.com".into(),
            port: "8448".into(),
            password: None,
            mumble_host: None,
            mumble_port: None,
            mumble_username: None,
            mumble_password: None,
            homeserver_url: None,
        };

        let (_, _, voice) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::ConnectToServer(form))],
        ).await;

        // Voice should have been launched with fallback credentials
        let launches = voice.launched_with.lock().unwrap();
        assert_eq!(launches.len(), 1);
        assert_eq!(launches[0].host, "matrix.example.com");
        assert_eq!(launches[0].port, 64738);
        assert_eq!(launches[0].username.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn connect_failure_emits_failed_state() {
        let tmp = tempfile::tempdir().unwrap();
        let form = ServerConnectionForm {
            username: "alice".into(),
            hostname: "matrix.example.com".into(),
            port: "8448".into(),
            password: None,
            mumble_host: None,
            mumble_port: None,
            mumble_username: None,
            mumble_password: None,
            homeserver_url: None,
        };

        let (events, _, voice) = run_commands(
            MockMatrix::new().with_connect_result(ConnectOutcome::Failed),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::ConnectToServer(form))],
        ).await;

        // Should have Connecting then Failed events, in that order
        let conn_states: Vec<_> = events.iter().filter_map(|e| match e {
            CoreEvent::Matrix(MatrixEvent::ConnectionState(s)) => Some(s),
            _ => None,
        }).collect();

        assert!(conn_states.len() >= 2, "Expected at least Connecting + Failed events, got {}", conn_states.len());
        assert!(matches!(conn_states[0], ConnectionState::Connecting));
        assert!(matches!(conn_states[1], ConnectionState::Failed { .. }));

        // Voice should NOT have been launched
        let launches = voice.launched_with.lock().unwrap();
        assert!(launches.is_empty(), "Voice should not launch on failed connection");
    }

    #[tokio::test]
    async fn connect_with_explicit_mumble_host() {
        let tmp = tempfile::tempdir().unwrap();
        let form = ServerConnectionForm {
            username: "alice".into(),
            hostname: "matrix.example.com".into(),
            port: "8448".into(),
            password: None,
            mumble_host: Some("mumble.example.com".into()),
            mumble_port: Some(64738),
            mumble_username: Some("alice_voice".into()),
            mumble_password: Some("secret".into()),
            homeserver_url: None,
        };

        let (_, _, voice) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::ConnectToServer(form))],
        ).await;

        let launches = voice.launched_with.lock().unwrap();
        assert_eq!(launches.len(), 1);
        assert_eq!(launches[0].host, "mumble.example.com");
        assert_eq!(launches[0].username.as_deref(), Some("alice_voice"));
        assert_eq!(launches[0].password.as_deref(), Some("secret"));
    }

    // --- resolve_mumble_credentials unit tests ---

    #[test]
    fn resolve_creds_bookmark_takes_priority() {
        let form = ServerConnectionForm {
            username: "alice".into(),
            hostname: "matrix.example.com".into(),
            port: "8448".into(),
            password: None,
            mumble_host: Some("mumble.example.com".into()),
            mumble_port: Some(12345),
            mumble_username: Some("alice_voice".into()),
            mumble_password: Some("secret".into()),
            homeserver_url: None,
        };
        let voice_server = Some(VoiceServerConfig {
            host: "voice.example.com".into(),
            port: 64738,
            username: Some("different".into()),
            password: Some("other_pass".into()),
        });

        let creds = CoreEngine::<MockMatrix, MockVoice>::resolve_mumble_credentials(&form, voice_server);
        assert_eq!(creds.host, "mumble.example.com");
        assert_eq!(creds.port, 12345);
        assert_eq!(creds.username.as_deref(), Some("alice_voice"));
        assert_eq!(creds.password.as_deref(), Some("secret"));
    }

    #[test]
    fn resolve_creds_falls_back_to_voice_server() {
        let form = ServerConnectionForm {
            username: "alice".into(),
            hostname: "matrix.example.com".into(),
            port: "8448".into(),
            password: None,
            mumble_host: None,
            mumble_port: None,
            mumble_username: None,
            mumble_password: None,
            homeserver_url: None,
        };
        let voice_server = Some(VoiceServerConfig {
            host: "voice.example.com".into(),
            port: 55555,
            username: None,
            password: Some("vs_pass".into()),
        });

        let creds = CoreEngine::<MockMatrix, MockVoice>::resolve_mumble_credentials(&form, voice_server);
        assert_eq!(creds.host, "voice.example.com");
        assert_eq!(creds.port, 55555);
        assert_eq!(creds.username.as_deref(), Some("alice")); // falls back to form.username
        assert_eq!(creds.password.as_deref(), Some("vs_pass"));
    }

    #[test]
    fn resolve_creds_falls_back_to_defaults() {
        let form = ServerConnectionForm {
            username: "alice".into(),
            hostname: "matrix.example.com".into(),
            port: "8448".into(),
            password: None,
            mumble_host: None,
            mumble_port: None,
            mumble_username: None,
            mumble_password: None,
            homeserver_url: None,
        };

        let creds = CoreEngine::<MockMatrix, MockVoice>::resolve_mumble_credentials(&form, None);
        assert_eq!(creds.host, "matrix.example.com");
        assert_eq!(creds.port, 64738);
        assert_eq!(creds.username.as_deref(), Some("alice"));
        assert!(creds.password.is_none());
    }

    // --- Internal event handling tests ---

    fn connect_form() -> ServerConnectionForm {
        ServerConnectionForm {
            username: "alice".into(),
            hostname: "matrix.example.com".into(),
            port: "8448".into(),
            password: None,
            mumble_host: None,
            mumble_port: None,
            mumble_username: None,
            mumble_password: None,
            homeserver_url: None,
        }
    }

    #[tokio::test]
    async fn mumble_connected_applies_saved_voice_settings() {
        let tmp = tempfile::tempdir().unwrap();
        settings::set_transmission_mode(tmp.path(), "push_to_talk".into());
        settings::set_vad_threshold(tmp.path(), 0.42);
        settings::set_voice_hold(tmp.path(), 250);

        let voice = MockVoice::new().with_internal_events(vec![
            InternalEvent::Mumble(InternalMumbleEvent::Connected),
        ]);

        let (_, _, voice_state) = run_commands(
            MockMatrix::new(),
            voice,
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::ConnectToServer(connect_form()))],
        ).await;

        let cmds = voice_state.commands.lock().unwrap();
        assert!(cmds.contains(&MumbleCommand::SetTransmissionMode("push_to_talk".into())));
        assert!(cmds.contains(&MumbleCommand::SetVadThreshold(0.42)));
        assert!(cmds.contains(&MumbleCommand::SetVoiceHold(250)));
    }

    #[tokio::test]
    async fn mumble_connected_skips_when_use_mumble_settings_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        settings::set_transmission_mode(tmp.path(), "push_to_talk".into());
        settings::set_vad_threshold(tmp.path(), 0.42);
        settings::set_voice_hold(tmp.path(), 250);
        settings::set_use_mumble_settings(tmp.path(), true);

        let voice = MockVoice::new().with_internal_events(vec![
            InternalEvent::Mumble(InternalMumbleEvent::Connected),
        ]);

        let (_, _, voice_state) = run_commands(
            MockMatrix::new(),
            voice,
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::ConnectToServer(connect_form()))],
        ).await;

        let cmds = voice_state.commands.lock().unwrap();
        let settings_cmds: Vec<_> = cmds.iter().filter(|c| matches!(c,
            MumbleCommand::SetTransmissionMode(_) |
            MumbleCommand::SetVadThreshold(_) |
            MumbleCommand::SetVoiceHold(_)
        )).collect();
        assert!(settings_cmds.is_empty(),
            "No voice settings should be applied when use_mumble_settings is enabled");
    }

    #[tokio::test]
    async fn user_joined_emits_enriched_user_state() {
        let tmp = tempfile::tempdir().unwrap();

        let matrix = MockMatrix::new()
            .with_profile_response(Some("Alice".into()), Some("mxc://example.com/avatar".into()));

        let voice = MockVoice::new().with_internal_events(vec![
            InternalEvent::Mumble(InternalMumbleEvent::UserJoined {
                session_id: 42,
                name: "alice".into(),
                volume_db: -3.5,
            }),
        ]);

        let (events, _, _) = run_commands(
            matrix,
            voice,
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::ConnectToServer(connect_form()))],
        ).await;

        let user_state = events.iter().find_map(|e| match e {
            CoreEvent::Mumble(MumbleEvent::UserState {
                session_id, display_name, avatar_url, ..
            }) if *session_id == 42 => Some((display_name.clone(), avatar_url.clone())),
            _ => None,
        });
        assert_eq!(
            user_state,
            Some((Some("Alice".into()), Some("mxc://example.com/avatar".into()))),
            "UserState should contain resolved profile data",
        );

        let volume = events.iter().find_map(|e| match e {
            CoreEvent::Mumble(MumbleEvent::UserVolume {
                session_id, volume_db,
            }) if *session_id == 42 => Some(*volume_db),
            _ => None,
        });
        assert_eq!(volume, Some(-3.5), "UserVolume should carry the stored volume");
    }

    #[tokio::test]
    async fn auto_connect_bookmark_triggers_connection() {
        let tmp = tempfile::tempdir().unwrap();

        let bookmark = crate::models::ServerBookmark {
            id: "auto".into(),
            label: "Auto Server".into(),
            address: "auto.example.com".into(),
            port: 8448,
            username: "bob".into(),
            auto_connect: true,
            mumble_host: None,
            mumble_port: None,
            mumble_username: None,
            mumble_password: None,
        };
        settings::update_bookmarks(tmp.path(), vec![bookmark]);

        let (events, _, voice_state) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::LoadSettings)],
        ).await;

        assert!(events.iter().any(|e| matches!(e,
            CoreEvent::System(SystemEvent::SettingsLoaded(_))
        )), "Expected SettingsLoaded event");

        assert!(events.iter().any(|e| matches!(e,
            CoreEvent::Matrix(MatrixEvent::ConnectionState(ConnectionState::Connecting))
        )), "Expected Connecting state from auto-connect bookmark");

        let launches = voice_state.launched_with.lock().unwrap();
        assert_eq!(launches.len(), 1);
        assert_eq!(launches[0].host, "auto.example.com");
        assert_eq!(launches[0].username.as_deref(), Some("bob"));
    }

    #[tokio::test]
    async fn set_vad_threshold_persists_and_forwards() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, _, voice) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::Mumble(MumbleCommand::SetVadThreshold(0.65))],
        ).await;

        let loaded = settings::load(tmp.path());
        assert_eq!(loaded.vad_threshold, Some(0.65));

        let cmds = voice.commands.lock().unwrap();
        assert!(cmds.iter().any(|c| matches!(c, MumbleCommand::SetVadThreshold(v) if *v == 0.65)));
    }

    #[tokio::test]
    async fn set_voice_hold_persists_and_forwards() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, _, voice) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::Mumble(MumbleCommand::SetVoiceHold(200))],
        ).await;

        let loaded = settings::load(tmp.path());
        assert_eq!(loaded.voice_hold, Some(200));

        let cmds = voice.commands.lock().unwrap();
        assert!(cmds.iter().any(|c| matches!(c, MumbleCommand::SetVoiceHold(200))));
    }

    #[tokio::test]
    async fn matrix_command_forwarded_to_backend() {
        let tmp = tempfile::tempdir().unwrap();
        let matrix = MockMatrix::new();
        let matrix_state = matrix.state.clone();

        let _ = run_commands(
            matrix,
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::Matrix(MatrixCommand::SendReadReceipt {
                room_id: "!room:example.com".into(),
                event_id: "$event123".into(),
            })],
        ).await;

        let cmds = matrix_state.commands.lock().unwrap();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(&cmds[0], MatrixCommand::SendReadReceipt { .. }));
    }

    #[tokio::test]
    async fn matrix_disconnect_triggers_retry() {
        let tmp = tempfile::tempdir().unwrap();
        let matrix = MockMatrix::new().with_internal_events(vec![
            InternalEvent::Matrix(InternalMatrixEvent::Disconnected("test disconnect".into())),
        ]);

        let (events, _, _) = run_commands(
            matrix,
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::ConnectToServer(connect_form()))],
        ).await;

        // After a successful connect, the mock fires an InternalMatrixEvent::Disconnected.
        // The engine should schedule a retry, emitting a Failed connection state.
        let has_failed = events.iter().any(|e| matches!(e,
            CoreEvent::Matrix(MatrixEvent::ConnectionState(ConnectionState::Failed { .. }))
        ));
        assert!(has_failed, "Disconnection should schedule a retry with Failed state");
    }

    #[tokio::test]
    async fn open_mumble_gui_launches_with_cached_creds() {
        let tmp = tempfile::tempdir().unwrap();

        // Connect first to cache credentials, then send OpenMumbleGui.
        let (_, _, voice_state) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
                CoreCommand::System(SystemCommand::OpenMumbleGui(String::new())),
            ],
        ).await;

        // First launch from connect, second from OpenMumbleGui.
        let launches = voice_state.launched_with.lock().unwrap();
        assert_eq!(launches.len(), 2, "Expected 2 voice launches (connect + OpenMumbleGui)");
    }

    #[tokio::test]
    async fn restart_mumble_launches_with_cached_creds() {
        let tmp = tempfile::tempdir().unwrap();

        let (_, _, voice_state) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
                CoreCommand::System(SystemCommand::RestartMumble(String::new())),
            ],
        ).await;

        let launches = voice_state.launched_with.lock().unwrap();
        assert_eq!(launches.len(), 2, "Expected 2 voice launches (connect + RestartMumble)");
    }

    // --- ServerReset tests ---

    #[tokio::test]
    async fn connect_resets_backend_before_connecting() {
        // The whole point of ServerReset: stale state from the previous
        // session must be cleared before any new state can accumulate.
        // Verify the mock sees reset() before connect() in the call log.
        let tmp = tempfile::tempdir().unwrap();
        let (events, matrix_state, _) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::ConnectToServer(connect_form()))],
        ).await;

        use crate::test_mocks::MockCall;
        let log = matrix_state.call_log.lock().unwrap();
        assert_eq!(
            log.as_slice(),
            &[MockCall::Reset, MockCall::Connect],
            "reset() must be called exactly once, before connect()"
        );

        // The frontend also needs the ServerReset event to clear its stores,
        // and it must arrive before the Connecting state so the UI doesn't
        // briefly show stale data.
        let reset_pos = events.iter().position(|e| matches!(e,
            CoreEvent::System(SystemEvent::ServerReset)
        ));
        let connecting_pos = events.iter().position(|e| matches!(e,
            CoreEvent::Matrix(MatrixEvent::ConnectionState(ConnectionState::Connecting))
        ));
        assert!(
            reset_pos.unwrap() < connecting_pos.unwrap(),
            "ServerReset event must precede Connecting state"
        );
    }

    #[tokio::test]
    async fn reconnect_resets_each_time() {
        // Switching servers (or reconnecting to the same one) must clear
        // state from the previous session every time, not just the first.
        let tmp = tempfile::tempdir().unwrap();
        let (_, matrix_state, _) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
            ],
        ).await;

        use crate::test_mocks::MockCall;
        let log = matrix_state.call_log.lock().unwrap();
        assert_eq!(
            log.as_slice(),
            &[MockCall::Reset, MockCall::Connect, MockCall::Reset, MockCall::Connect],
            "Each connection attempt must go through reset-then-connect"
        );
    }

    #[tokio::test]
    async fn open_mumble_gui_noop_without_cached_creds() {
        let tmp = tempfile::tempdir().unwrap();

        // Send OpenMumbleGui without connecting first -- should be a no-op.
        let (_, _, voice_state) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::OpenMumbleGui(String::new()))],
        ).await;

        let launches = voice_state.launched_with.lock().unwrap();
        assert!(launches.is_empty(), "OpenMumbleGui without prior connect should not launch voice");
    }

    // --- Certificate check tests ---

    /// Helper: start a local TLS server on a random port using a self-signed cert.
    /// Returns (port, SHA1 fingerprint of the cert's DER).
    async fn start_tls_server() -> (u16, String) {
        use sha1::{Sha1, Digest};
        use std::sync::Arc;

        let _ = rustls::crypto::ring::default_provider().install_default();

        let key_pair = rcgen::KeyPair::generate().unwrap();
        let params = rcgen::CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
        let cert = params.self_signed(&key_pair).unwrap();
        let cert_der = cert.der().to_vec();
        let key_der = key_pair.serialize_der();
        let fingerprint = format!("{:x}", Sha1::digest(&cert_der));

        let server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls_pki_types::CertificateDer::from(cert_der)],
                rustls_pki_types::PrivateKeyDer::try_from(key_der).unwrap(),
            ).unwrap();

        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            // Accept connections in a loop until the test ends
            while let Ok((stream, _)) = listener.accept().await {
                let acc = acceptor.clone();
                tokio::spawn(async move {
                    // Complete the TLS handshake, then drop
                    let _ = acc.accept(stream).await;
                });
            }
        });

        (port, fingerprint)
    }

    /// Helper: create a mumble.sqlite with the cert table in a temp dir.
    fn seed_cert_db(data_dir: &std::path::Path, host: &str, port: u16, digest: &str) {
        let mumble_dir = data_dir.join("mumble");
        std::fs::create_dir_all(&mumble_dir).unwrap();
        let db_path = mumble_dir.join("mumble.sqlite");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cert (id INTEGER PRIMARY KEY AUTOINCREMENT, hostname TEXT, port INTEGER, digest TEXT);
             CREATE UNIQUE INDEX IF NOT EXISTS cert_host_port ON cert(hostname, port);"
        ).unwrap();
        conn.execute(
            "INSERT INTO cert (hostname, port, digest) VALUES (?1, ?2, ?3)",
            rusqlite::params![host, port as i64, digest],
        ).unwrap();
    }

    #[tokio::test]
    async fn cert_mismatch_blocks_launch_and_emits_event() {
        let (port, _real_fp) = start_tls_server().await;
        let tmp = tempfile::tempdir().unwrap();
        seed_cert_db(tmp.path(), "127.0.0.1", port, "wrong_fingerprint");

        let form = ServerConnectionForm {
            username: "testuser".into(),
            hostname: "matrix.example.com".into(),
            port: "8448".into(),
            password: None,
            mumble_host: Some("127.0.0.1".into()),
            mumble_port: Some(port),
            mumble_username: None,
            mumble_password: None,
            homeserver_url: None,
        };

        let (events, _, voice_state) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::ConnectToServer(form))],
        ).await;

        // Should have emitted CertificateChanged
        let cert_event = events.iter().find_map(|e| match e {
            CoreEvent::Mumble(MumbleEvent::CertificateChanged { host, port: p, new_fingerprint }) =>
                Some((host.clone(), *p, new_fingerprint.clone())),
            _ => None,
        });
        assert!(cert_event.is_some(), "Expected CertificateChanged event");
        let (host, p, _fp) = cert_event.unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(p, port);

        // Voice should NOT have been launched
        let launches = voice_state.launched_with.lock().unwrap();
        assert!(launches.is_empty(), "Voice should not launch when cert mismatches");
    }

    #[tokio::test]
    async fn accept_cert_stores_and_launches_voice() {
        let (port, real_fp) = start_tls_server().await;
        let tmp = tempfile::tempdir().unwrap();
        seed_cert_db(tmp.path(), "127.0.0.1", port, "wrong_fingerprint");

        let form = ServerConnectionForm {
            username: "testuser".into(),
            hostname: "matrix.example.com".into(),
            port: "8448".into(),
            password: None,
            mumble_host: Some("127.0.0.1".into()),
            mumble_port: Some(port),
            mumble_username: None,
            mumble_password: None,
            homeserver_url: None,
        };

        let (events, _, voice_state) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![
                CoreCommand::System(SystemCommand::ConnectToServer(form)),
                CoreCommand::System(SystemCommand::AcceptMumbleCert {
                    host: "127.0.0.1".into(),
                    port,
                    fingerprint: real_fp.clone(),
                }),
            ],
        ).await;

        // CertificateChanged should still have been emitted
        assert!(events.iter().any(|e| matches!(e, CoreEvent::Mumble(MumbleEvent::CertificateChanged { .. }))));

        // Voice SHOULD have been launched after accept
        let launches = voice_state.launched_with.lock().unwrap();
        assert_eq!(launches.len(), 1, "Voice should launch after cert accept");
        assert_eq!(launches[0].host, "127.0.0.1");
        assert_eq!(launches[0].port, port);

        // Cert should be stored in the DB
        let db_path = tmp.path().join("mumble/mumble.sqlite");
        let stored = crate::mumble::cert::get_stored_cert(&db_path, "127.0.0.1", port);
        assert_eq!(stored, Some(real_fp), "Accepted fingerprint should be persisted");
    }

    #[tokio::test]
    async fn cert_first_use_stores_and_launches() {
        let (port, real_fp) = start_tls_server().await;
        let tmp = tempfile::tempdir().unwrap();
        // Create DB with cert table but NO entry for this host
        seed_cert_db(tmp.path(), "other.host", 9999, "irrelevant");

        let form = ServerConnectionForm {
            username: "testuser".into(),
            hostname: "matrix.example.com".into(),
            port: "8448".into(),
            password: None,
            mumble_host: Some("127.0.0.1".into()),
            mumble_port: Some(port),
            mumble_username: None,
            mumble_password: None,
            homeserver_url: None,
        };

        let (events, _, voice_state) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::ConnectToServer(form))],
        ).await;

        // No CertificateChanged -- TOFU should auto-accept
        assert!(!events.iter().any(|e| matches!(e, CoreEvent::Mumble(MumbleEvent::CertificateChanged { .. }))),
            "TOFU should not emit CertificateChanged");

        // Voice should have launched
        let launches = voice_state.launched_with.lock().unwrap();
        assert_eq!(launches.len(), 1, "Voice should launch on first use");

        // Cert should be stored
        let db_path = tmp.path().join("mumble/mumble.sqlite");
        let stored = crate::mumble::cert::get_stored_cert(&db_path, "127.0.0.1", port);
        assert_eq!(stored, Some(real_fp), "TOFU should store the fingerprint");
    }

    #[tokio::test]
    async fn cert_match_launches_without_event() {
        let (port, real_fp) = start_tls_server().await;
        let tmp = tempfile::tempdir().unwrap();
        // Pre-store the CORRECT fingerprint
        seed_cert_db(tmp.path(), "127.0.0.1", port, &real_fp);

        let form = ServerConnectionForm {
            username: "testuser".into(),
            hostname: "matrix.example.com".into(),
            port: "8448".into(),
            password: None,
            mumble_host: Some("127.0.0.1".into()),
            mumble_port: Some(port),
            mumble_username: None,
            mumble_password: None,
            homeserver_url: None,
        };

        let (events, _, voice_state) = run_commands(
            MockMatrix::new(),
            MockVoice::new(),
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::ConnectToServer(form))],
        ).await;

        // No CertificateChanged
        assert!(!events.iter().any(|e| matches!(e, CoreEvent::Mumble(MumbleEvent::CertificateChanged { .. }))));

        // Voice launched normally
        let launches = voice_state.launched_with.lock().unwrap();
        assert_eq!(launches.len(), 1);
    }

    // --- Voice state restoration tests ---

    #[tokio::test]
    async fn reconnect_restores_mute_and_deafen() {
        let tmp = tempfile::tempdir().unwrap();

        // First launch: emit state changes.
        // Second launch (RestartMumble): emit Connected to trigger restore.
        let voice = MockVoice::new()
            .with_internal_events(vec![
                InternalEvent::Mumble(InternalMumbleEvent::LocalMuteChanged(true)),
                InternalEvent::Mumble(InternalMumbleEvent::LocalDeafChanged(true)),
            ])
            .with_internal_events(vec![
                InternalEvent::Mumble(InternalMumbleEvent::Connected),
            ]);

        let (_, _, voice_state) = run_commands(
            MockMatrix::new(),
            voice,
            tmp.path(),
            vec![
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
                CoreCommand::System(SystemCommand::RestartMumble(String::new())),
            ],
        ).await;

        let cmds = voice_state.commands.lock().unwrap();
        assert!(cmds.contains(&MumbleCommand::MuteSelf(true)),
            "Expected MuteSelf(true) restore command, got: {:?}", *cmds);
        assert!(cmds.contains(&MumbleCommand::DeafenSelf(true)),
            "Expected DeafenSelf(true) restore command, got: {:?}", *cmds);
    }

    #[tokio::test]
    async fn reconnect_passes_channel_path_in_launch() {
        let tmp = tempfile::tempdir().unwrap();

        // First launch: user moves to a channel.
        let voice = MockVoice::new()
            .with_internal_events(vec![
                InternalEvent::Mumble(InternalMumbleEvent::LocalChannelChanged {
                    channel_path: "Voice/General".into(),
                }),
            ]);

        let (_, _, voice_state) = run_commands(
            MockMatrix::new(),
            voice,
            tmp.path(),
            vec![
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
                CoreCommand::System(SystemCommand::RestartMumble(String::new())),
            ],
        ).await;

        let paths = voice_state.launched_channel_paths.lock().unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], None, "First launch should have no channel path");
        assert_eq!(paths[1], Some("Voice/General".into()),
            "Second launch should include saved channel path");
    }

    #[tokio::test]
    async fn connect_to_server_clears_saved_voice_state() {
        let tmp = tempfile::tempdir().unwrap();

        let voice = MockVoice::new();
        let voice_state = voice.state.clone();
        let (_, cmd_rx) = mpsc::channel(32);
        let (event_tx, _) = mpsc::channel(100);
        let mut engine = CoreEngine::new(cmd_rx, event_tx, MockMatrix::new(), voice, tmp.path().to_path_buf());

        // Simulate state saved from a previous session
        engine.voice_session = VoiceSessionState {
            channel_path: Some("Voice/General".into()),
            muted: true,
            deafened: true,
        };

        // ConnectToServer should clear all saved state
        let mut retry_timer = Box::pin(sleep(Duration::from_secs(3600)));
        let (itx, _) = mpsc::channel(32);
        engine.handle_system_command(
            SystemCommand::ConnectToServer(connect_form()),
            &mut retry_timer,
            itx,
        ).await;

        assert_eq!(engine.voice_session, VoiceSessionState::default());

        // Voice launched with no channel path
        let paths = voice_state.launched_channel_paths.lock().unwrap();
        assert_eq!(paths[0], None, "ConnectToServer should launch with no channel path");
    }

    #[tokio::test]
    async fn unmute_before_crash_does_not_restore_mute() {
        let tmp = tempfile::tempdir().unwrap();

        // First launch: mute then unmute before crash.
        // Second launch: emit Connected to trigger restore.
        let voice = MockVoice::new()
            .with_internal_events(vec![
                InternalEvent::Mumble(InternalMumbleEvent::LocalMuteChanged(true)),
                InternalEvent::Mumble(InternalMumbleEvent::LocalMuteChanged(false)),
            ])
            .with_internal_events(vec![
                InternalEvent::Mumble(InternalMumbleEvent::Connected),
            ]);

        let (_, _, voice_state) = run_commands(
            MockMatrix::new(),
            voice,
            tmp.path(),
            vec![
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
                CoreCommand::System(SystemCommand::RestartMumble(String::new())),
            ],
        ).await;

        let cmds = voice_state.commands.lock().unwrap();
        assert!(!cmds.contains(&MumbleCommand::MuteSelf(true)),
            "MuteSelf(true) should NOT be sent when user unmuted before crash, got: {:?}", *cmds);
    }

    #[tokio::test]
    async fn multiple_channel_moves_restores_only_last() {
        let tmp = tempfile::tempdir().unwrap();

        // User moves through three channels before crash.
        let voice = MockVoice::new()
            .with_internal_events(vec![
                InternalEvent::Mumble(InternalMumbleEvent::LocalChannelChanged {
                    channel_path: "Voice/Alpha".into(),
                }),
                InternalEvent::Mumble(InternalMumbleEvent::LocalChannelChanged {
                    channel_path: "Voice/Beta".into(),
                }),
                InternalEvent::Mumble(InternalMumbleEvent::LocalChannelChanged {
                    channel_path: "Voice/Gamma".into(),
                }),
            ]);

        let (_, _, voice_state) = run_commands(
            MockMatrix::new(),
            voice,
            tmp.path(),
            vec![
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
                CoreCommand::System(SystemCommand::RestartMumble(String::new())),
            ],
        ).await;

        let paths = voice_state.launched_channel_paths.lock().unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[1], Some("Voice/Gamma".into()),
            "Should restore only the last channel the user was in");
    }

    #[tokio::test]
    async fn deafen_only_restores_without_mute() {
        let tmp = tempfile::tempdir().unwrap();

        // User deafens but never explicitly mutes (Mumble UI auto-mutes on
        // deafen, but the bridge reports them as separate state changes).
        let voice = MockVoice::new()
            .with_internal_events(vec![
                InternalEvent::Mumble(InternalMumbleEvent::LocalDeafChanged(true)),
            ])
            .with_internal_events(vec![
                InternalEvent::Mumble(InternalMumbleEvent::Connected),
            ]);

        let (_, _, voice_state) = run_commands(
            MockMatrix::new(),
            voice,
            tmp.path(),
            vec![
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
                CoreCommand::System(SystemCommand::RestartMumble(String::new())),
            ],
        ).await;

        let cmds = voice_state.commands.lock().unwrap();
        assert!(cmds.contains(&MumbleCommand::DeafenSelf(true)),
            "DeafenSelf(true) should be restored, got: {:?}", *cmds);
        assert!(!cmds.contains(&MumbleCommand::MuteSelf(true)),
            "MuteSelf(true) should NOT be sent when only deafen was set, got: {:?}", *cmds);
    }
}
