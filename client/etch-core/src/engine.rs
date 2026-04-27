use tokio::sync::mpsc;
use tokio::time::{sleep, Duration, Sleep};
use crate::connection;
use crate::events::{CoreEvent, InternalEvent, InternalMatrixEvent, InternalMumbleEvent, MumbleEvent, SystemEvent};
use bridge_types::MumbleCommand as BridgeCommand;
use crate::commands::{CoreCommand, MumbleCommand, ServerConnectionForm, SystemCommand};
use crate::models::{ConnectionState, VoiceServerConfig};
use crate::mumble;
use crate::settings;
use crate::matrix;

use std::path::PathBuf;
use std::pin::Pin;

pub struct CoreEngine {
    pub cmd_rx: mpsc::Receiver<CoreCommand>,
    pub event_tx: mpsc::Sender<CoreEvent>,

    matrix_service: matrix::MatrixService,
    mumble_process: Option<mumble::process::MumbleProcess>,
    mumble_credentials: Option<VoiceServerConfig>,
    pub data_dir: PathBuf,
    pub resource_dir: PathBuf,
}

pub struct CoreHandle {
    pub cmd_tx: mpsc::Sender<CoreCommand>,
    pub event_rx: mpsc::Receiver<CoreEvent>,
}

impl CoreEngine {
    pub fn new(
        cmd_rx: mpsc::Receiver<CoreCommand>,
        event_tx: mpsc::Sender<CoreEvent>,
        data_dir: PathBuf,
        resource_dir: PathBuf,
    ) -> Self {
        let matrix_service = matrix::MatrixService::new(event_tx.clone(), data_dir.clone());
        Self {
            cmd_rx,
            event_tx,
            matrix_service,
            mumble_process: None,
            mumble_credentials: None,
            resource_dir,
            data_dir,
        }
    }

    pub async fn run(mut self) {
        let (internal_tx, mut internal_rx) = mpsc::channel::<InternalEvent>(100);

        // --- Connection FSM state ---
        let mut matrix_state = ConnectionState::Disconnected;
        let mut matrix_timer: Pin<Box<Sleep>> = Box::pin(sleep(Duration::MAX));
        let mut connection_form: Option<ServerConnectionForm> = None;

        loop {
            tokio::select! {
                Some(cmd) = self.cmd_rx.recv() => {
                    match cmd {
                        CoreCommand::Matrix(matrix_cmd) => {
                            self.matrix_service.handle_command(matrix_cmd).await;
                        }
                        CoreCommand::Mumble(mumble_cmd) => {
                            self.send_bridge_command(mumble_cmd).await;
                        }
                        CoreCommand::System(sys_cmd) => {
                            match sys_cmd {
                                SystemCommand::ConnectToServer(form) => {
                                    connection_form = Some(form.clone());
                                    self.connect_to_server(
                                        &form, &mut matrix_state, &mut matrix_timer,
                                        internal_tx.clone(),
                                    ).await;
                                }
                                SystemCommand::LoadSettings => {
                                    let s = settings::load(&self.data_dir);
                                    let _ = self.event_tx.send(CoreEvent::System(
                                        SystemEvent::SettingsLoaded(s.clone()),
                                    )).await;

                                    if let Some(bm) = s.bookmarks.iter().find(|b| b.auto_connect) {
                                        let form = ServerConnectionForm::from(bm);
                                        connection_form = Some(form.clone());
                                        self.connect_to_server(
                                            &form, &mut matrix_state, &mut matrix_timer,
                                            internal_tx.clone(),
                                        ).await;
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
                                    self.send_bridge_command(MumbleCommand::MuteSelf(muted)).await;
                                }
                                SystemCommand::Deafen(deafened) => {
                                    self.send_bridge_command(MumbleCommand::DeafenSelf(deafened)).await;
                                }
                                SystemCommand::OpenMumbleGui(extra_args) => {
                                    if let Some(ref creds) = self.mumble_credentials {
                                        self.spawn_mumble(creds.clone(), true, &extra_args, internal_tx.clone()).await;
                                    }
                                }
                                SystemCommand::RestartMumble(extra_args) => {
                                    if let Some(ref creds) = self.mumble_credentials {
                                        self.spawn_mumble(creds.clone(), false, &extra_args, internal_tx.clone()).await;
                                    }
                                }
                                SystemCommand::SetLogLevel(level) => {
                                    crate::logger::set_level(&level);
                                }
                                SystemCommand::TestError => {
                                    log::error!("Test error triggered from Developer Options");
                                }
                            }
                        }
                    }
                }

                Some(internal_event) = internal_rx.recv() => {
                    match internal_event {
                        InternalEvent::Matrix(evt) => {
                            match evt {
                                InternalMatrixEvent::Connected => {
                                    log::debug!("Internal: Matrix connected");
                                }
                                InternalMatrixEvent::SubscribeToRoom(room_id) => {
                                    if let Some(client) = &self.matrix_service.client {
                                        if let Some(room) = client.get_room(&room_id) {
                                            self.matrix_service.timeline_manager.subscribe_to_room(&room).await;
                                        }
                                    }
                                }
                                InternalMatrixEvent::Disconnected(reason) => {
                                    log::warn!("Matrix disconnected: {}", reason);
                                    connection::schedule_retry(
                                        &mut matrix_state, &mut matrix_timer,
                                        reason, &self.event_tx, connection::matrix_conn_event,
                                    ).await;
                                }
                            }
                        }
                        InternalEvent::Mumble(evt) => {
                            match evt {
                                InternalMumbleEvent::UserJoined { session_id, name, channel_id, self_mute, self_deaf, volume_db } => {
                                    let (display_name, avatar_url) = self.matrix_service.resolve_user_profile(&name).await;
                                    let _ = self.event_tx.send(CoreEvent::Mumble(MumbleEvent::UserState {
                                        session_id,
                                        name: Some(name),
                                        display_name,
                                        avatar_url,
                                        channel_id: Some(channel_id),
                                        self_mute: Some(self_mute),
                                        self_deaf: Some(self_deaf),
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
                                            self.send_bridge_command(MumbleCommand::SetTransmissionMode(mode)).await;
                                        }
                                        if let Some(value) = s.vad_threshold {
                                            self.send_bridge_command(MumbleCommand::SetVadThreshold(value)).await;
                                        }
                                        if let Some(value) = s.voice_hold {
                                            self.send_bridge_command(MumbleCommand::SetVoiceHold(value)).await;
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

                // --- Retry timers ---

                _ = &mut matrix_timer, if matrix_state.is_failed() => {
                    if let Some(form) = connection_form.clone() {
                        log::info!("Retrying Matrix connection (attempt {})", matrix_state.retries() + 1);
                        self.connect_to_server(
                            &form, &mut matrix_state, &mut matrix_timer,
                            internal_tx.clone(),
                        ).await;
                    }
                }
            }
        }
    }

    async fn send_bridge_command(&self, cmd: MumbleCommand) {
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

        if let Some(ref proc) = self.mumble_process {
            let bridge_cmd = match cmd {
                MumbleCommand::SwitchChannel(id) => BridgeCommand::SwitchChannel { channel_id: id as i32 },
                MumbleCommand::MuteSelf(muted) => BridgeCommand::SetMuted { muted },
                MumbleCommand::DeafenSelf(deafened) => BridgeCommand::SetDeafened { deafened },
                MumbleCommand::SetUserVolume { session_id, volume_db } => {
                    // Convert dB offset to linear factor: 10^(dB/20)
                    let volume = 10.0_f32.powf(volume_db / 20.0);
                    BridgeCommand::SetUserVolume { session: session_id, volume }
                }
                MumbleCommand::SetTransmissionMode(ref mode_str) => {
                    let mode = match mode_str.as_str() {
                        "continuous" => bridge_types::TransmissionMode::Continuous,
                        "push_to_talk" => bridge_types::TransmissionMode::PushToTalk,
                        _ => bridge_types::TransmissionMode::VoiceActivation,
                    };
                    BridgeCommand::SetTransmissionMode { mode }
                }
                MumbleCommand::SetVadThreshold(value) => {
                    BridgeCommand::SetVadThreshold { value }
                }
                MumbleCommand::SetVoiceHold(value) => {
                    BridgeCommand::SetVoiceHold { value }
                }
                MumbleCommand::SetUseMumbleSettings(_) => unreachable!(),
            };
            let _ = proc.cmd_tx.send(bridge_cmd).await;
        }
    }

    async fn connect_to_server(
        &mut self,
        form: &ServerConnectionForm,
        matrix_state: &mut ConnectionState,
        matrix_timer: &mut Pin<Box<Sleep>>,
        internal_tx: mpsc::Sender<InternalEvent>,
    ) {
        if form.mumble_host.is_some() {
            // Explicit Mumble config in bookmark: launch immediately, don't wait for Matrix
            self.resolve_and_launch_mumble(form, None, false, "", internal_tx.clone()).await;
            connection::attempt_matrix_connect(
                matrix_state, matrix_timer,
                &mut self.matrix_service, form.clone(),
                internal_tx, &self.event_tx,
            ).await;
        } else {
            // No explicit Mumble config: wait for Matrix to discover voice server
            let voice_server = connection::attempt_matrix_connect(
                matrix_state, matrix_timer,
                &mut self.matrix_service, form.clone(),
                internal_tx.clone(), &self.event_tx,
            ).await;
            if matches!(matrix_state, ConnectionState::Connected) {
                self.resolve_and_launch_mumble(form, voice_server, false, "", internal_tx).await;
            }
        }
    }

    fn resolve_mumble_credentials(
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

    async fn resolve_and_launch_mumble(
        &mut self,
        form: &ServerConnectionForm,
        voice_server: Option<VoiceServerConfig>,
        show_gui: bool,
        extra_args: &str,
        internal_tx: mpsc::Sender<InternalEvent>,
    ) {
        let creds = Self::resolve_mumble_credentials(form, voice_server);
        self.mumble_credentials = Some(creds.clone());
        self.spawn_mumble(creds, show_gui, extra_args, internal_tx).await;
    }

    async fn spawn_mumble(&mut self, creds: VoiceServerConfig, show_gui: bool, extra_args: &str, internal_tx: mpsc::Sender<InternalEvent>) {
        if let Some(ref mut proc) = self.mumble_process {
            proc.kill().await;
        }

        let username = creds.username.as_deref().unwrap_or("unknown");
        match mumble::process::MumbleProcess::spawn(
            &creds.host, creds.port, username, creds.password.as_deref(),
            self.event_tx.clone(), internal_tx, show_gui, extra_args,
            &self.data_dir, &self.resource_dir,
        ).await {
            Ok(proc) => {
                log::info!("Mumble launched, bridge socket: {}", proc.sock_name);
                self.mumble_process = Some(proc);
            }
            Err(e) => {
                log::error!("Failed to spawn Mumble: {:?}", e);
            }
        }
    }
}
