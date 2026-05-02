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

pub struct CoreEngine<M, V> {
    pub(crate) cmd_rx: mpsc::Receiver<CoreCommand>,
    pub(crate) event_tx: mpsc::Sender<CoreEvent>,

    matrix: M,
    voice: V,
    conn: MatrixConnection,
    mumble_credentials: Option<VoiceServerConfig>,
    pub(crate) data_dir: PathBuf,
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
        if let Err(e) = self.voice.launch(creds, internal_tx, show_gui, extra_args).await {
            log::error!("Failed to launch voice: {:?}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_mocks::{MockMatrix, MockMatrixState, MockVoice, MockVoiceState};
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
            MockMatrix::new().with_connect_result(false, None),
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
}
