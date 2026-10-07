use tokio::sync::mpsc;
use tokio::time::{sleep, Duration, Instant, Sleep};
use crate::actor::{LaunchRequest, MatrixHandle, MatrixRequest, VoiceHandle, VoiceRequest};
use crate::connection::MatrixConnection;
use crate::events::{CoreEvent, InternalEvent, InternalMatrixEvent, InternalMumbleEvent, LaunchOutcome, MumbleEvent, SyncEnd, SystemEvent};
use crate::commands::{CoreCommand, MediaRequest, MumbleCommand, ServerConnectionForm, SystemCommand};
use crate::models::{ConnectOutcome, ConnectionState, VoiceServerConfig};
use crate::settings::{Settings, SettingsStore};
use crate::traits::{MatrixBackend, VoiceService};

use std::path::PathBuf;
use std::pin::Pin;

const INTERNAL_QUEUE: usize = 100;

/// Bounds how long quitting waits for dispatched work, so a stuck launch cannot hold the app open.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

const SIGN_OUT_UNSERVED: &str =
    "Etch could not sign out, so this device is still signed in. Restart Etch and try again.";

/// Voice state tracked in memory for restoration after Mumble restarts.
/// Reset when connecting to a new server; preserved across process restarts
/// on the same server.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct VoiceRestoreState {
    pub channel_path: Option<String>,
    pub muted: bool,
    pub deafened: bool,
}

#[derive(Debug)]
pub(crate) enum VoiceSession {
    Idle,
    /// Creds known, Mumble not up (never launched, failed, or dropped).
    Down { creds: VoiceServerConfig },
    Launching { creds: VoiceServerConfig },
    AwaitingCert {
        creds: VoiceServerConfig,
        show_gui: bool,
        extra_args: String,
    },
    Up { creds: VoiceServerConfig },
}

impl VoiceSession {
    pub(crate) fn creds(&self) -> Option<&VoiceServerConfig> {
        match self {
            VoiceSession::Idle => None,
            VoiceSession::Down { creds }
            | VoiceSession::Launching { creds }
            | VoiceSession::AwaitingCert { creds, .. }
            | VoiceSession::Up { creds } => Some(creds),
        }
    }

    /// Not `Debug`-printed because the credentials carry a password.
    pub(crate) fn state_name(&self) -> &'static str {
        match self {
            VoiceSession::Idle => "Idle",
            VoiceSession::Down { .. } => "Down",
            VoiceSession::Launching { .. } => "Launching",
            VoiceSession::AwaitingCert { .. } => "AwaitingCert",
            VoiceSession::Up { .. } => "Up",
        }
    }
}

/// The form is kept because the voice launch after a successful connect is resolved from it.
struct PendingConnect {
    generation: u64,
    form: ServerConnectionForm,
    cancel: tokio::sync::oneshot::Sender<()>,
    started: Instant,
    /// The connect spawns its sync task before it answers, so its session can report first.
    held_sync: Option<SyncReport>,
}

enum SyncReport {
    Degraded,
    Recovered,
    Ended(SyncEnd),
}

/// A sign out the engine has dispatched and not yet had an answer for.
#[derive(Default)]
struct PendingSignOut {
    /// A connect asked for meanwhile, started once the sign out has answered so the two cannot interleave.
    then_connect: Option<ServerConnectionForm>,
}

/// A voice launch the engine has dispatched and not yet had an answer for.
struct PendingLaunch {
    generation: u64,
    creds: VoiceServerConfig,
    show_gui: bool,
    extra_args: String,
}

/// Coordinates the subsystems; owns none of them.
pub struct CoreEngine {
    pub(crate) cmd_rx: mpsc::Receiver<CoreCommand>,
    pub(crate) media_rx: mpsc::Receiver<MediaRequest>,
    pub(crate) event_tx: mpsc::Sender<CoreEvent>,

    internal_tx: mpsc::Sender<InternalEvent>,
    internal_rx: mpsc::Receiver<InternalEvent>,

    matrix: MatrixHandle,
    voice_service: VoiceHandle,
    conn: MatrixConnection,
    pub(crate) settings: SettingsStore,
    /// Voice state persisted across Mumble client restarts.
    pub(crate) voice_restore: VoiceRestoreState,
    pub(crate) voice: VoiceSession,

    pending_connect: Option<PendingConnect>,
    pending_launch: Option<PendingLaunch>,
    pending_sign_out: Option<PendingSignOut>,
    connect_generation: u64,
    /// The connect generation whose sync reports are current: set when that connect settles
    /// `Connected`, cleared when its sync ends or another connect starts.
    live_generation: Option<u64>,
    launch_generation: u64,

    /// Set at the top of `shut_down` so a connect landing during the grace cannot start voice.
    shutting_down: bool,

    #[cfg(test)]
    pending_barriers: Vec<tokio::sync::oneshot::Sender<()>>,
}

pub struct CoreHandle {
    pub cmd_tx: mpsc::Sender<CoreCommand>,
    pub event_rx: mpsc::Receiver<CoreEvent>,
}

impl CoreEngine {
    pub fn new<M: MatrixBackend + 'static, V: VoiceService + 'static>(
        cmd_rx: mpsc::Receiver<CoreCommand>,
        media_rx: mpsc::Receiver<MediaRequest>,
        event_tx: mpsc::Sender<CoreEvent>,
        matrix: M,
        voice: V,
        data_dir: PathBuf,
        settings: Settings,
    ) -> Self {
        let (internal_tx, internal_rx) = mpsc::channel(INTERNAL_QUEUE);
        Self {
            cmd_rx,
            media_rx,
            matrix: MatrixHandle::spawn(matrix),
            voice_service: VoiceHandle::spawn(voice, data_dir.clone(), event_tx.clone()),
            event_tx,
            internal_tx,
            internal_rx,
            conn: MatrixConnection::new(),
            settings: SettingsStore::from_loaded(data_dir, settings),
            voice_restore: VoiceRestoreState::default(),
            voice: VoiceSession::Idle,
            pending_connect: None,
            pending_launch: None,
            pending_sign_out: None,
            connect_generation: 0,
            live_generation: None,
            launch_generation: 0,
            shutting_down: false,
            #[cfg(test)]
            pending_barriers: Vec::new(),
        }
    }

    pub async fn run(mut self) {
        let mut retry_timer: Pin<Box<Sleep>> = Box::pin(sleep(Duration::MAX));

        loop {
            tokio::select! {
                cmd = self.cmd_rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    match cmd {
                        CoreCommand::Matrix(matrix_cmd) => {
                            let _ = self.matrix.send(MatrixRequest::Command(matrix_cmd)).await;
                        }
                        CoreCommand::Mumble(mumble_cmd) => {
                            self.dispatch_mumble_command(mumble_cmd).await;
                        }
                        CoreCommand::System(cmd) => {
                            self.handle_system_command(cmd, &mut retry_timer).await;
                        }
                    }
                    // Drain internal events that arrived during command processing
                    // so they're handled before the next command.
                    while let Ok(event) = self.internal_rx.try_recv() {
                        self.handle_internal_event(event, &mut retry_timer).await;
                    }
                    self.answer_barriers();
                }

                Some(request) = self.media_rx.recv() => {
                    self.matrix.fetch_media(request.mxc_url, request.respond);
                }

                Some(internal_event) = self.internal_rx.recv() => {
                    self.handle_internal_event(internal_event, &mut retry_timer).await;
                    self.answer_barriers();
                }

                // Gated on no attempt in flight, or a connect slower than the backoff
                // would keep superseding itself.
                _ = &mut retry_timer, if self.conn.state.is_failed()
                    // An elapsed timer with nothing to retry would fire on every turn of the loop.
                    && self.conn.form.is_some()
                    && self.pending_connect.is_none()
                    && self.pending_sign_out.is_none() =>
                {
                    if let Some(form) = self.conn.form.clone() {
                        log::info!("Retrying Matrix connection (attempt {})", self.conn.retries + 1);
                        self.connect_to_server(&form, &mut retry_timer).await;
                    }
                }
            }
        }

        self.shut_down(&mut retry_timer).await;
    }

    /// An outstanding connect is deliberately not waited for: with `launch_voice` suppressed it has
    /// nothing left to do, and waiting would burn grace that `matrix.finish` needs.
    async fn shut_down(mut self, retry_timer: &mut Pin<Box<Sleep>>) {
        self.shutting_down = true;
        let deadline = Instant::now() + SHUTDOWN_GRACE;

        while self.pending_launch.is_some() {
            tokio::select! {
                Some(event) = self.internal_rx.recv() => {
                    self.handle_internal_event(event, retry_timer).await;
                }
                _ = tokio::time::sleep_until(deadline) => {
                    log::warn!("Shutdown grace expired with dispatched work still outstanding");
                    break;
                }
            }
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        self.matrix.finish(remaining).await;
        let remaining = deadline.saturating_duration_since(Instant::now());
        self.voice_service.finish(remaining).await;

        while let Ok(event) = self.internal_rx.try_recv() {
            self.handle_internal_event(event, retry_timer).await;
        }

        self.settings.shutdown().await;
    }

    /// A no-op outside tests; shutdown waits on a narrower condition than a barrier does.
    fn answer_barriers(&mut self) {
        #[cfg(test)]
        if self.pending_connect.is_none()
            && self.pending_launch.is_none()
            && self.pending_sign_out.is_none()
            && self.cmd_rx.is_empty()
        {
            for reply in self.pending_barriers.drain(..) {
                let _ = reply.send(());
            }
        }
    }

    async fn handle_system_command(
        &mut self,
        cmd: SystemCommand,
        retry_timer: &mut Pin<Box<Sleep>>,
    ) {
        match cmd {
            SystemCommand::ConnectToServer(form) => {
                self.connect_on_request(form, retry_timer).await;
            }
            SystemCommand::LoadSettings => {
                let s = self.settings.get().clone();
                let _ = self.event_tx.send(CoreEvent::System(
                    SystemEvent::SettingsLoaded(s.clone()),
                )).await;

                if let Some(bm) = s.bookmarks.iter().find(|b| b.auto_connect) {
                    self.connect_on_request(ServerConnectionForm::from(bm), retry_timer).await;
                }
            }
            SystemCommand::SignOut => self.sign_out(retry_timer).await,
            SystemCommand::SaveBookmarks(bookmarks) => {
                self.settings.update(|s| s.bookmarks = bookmarks);
                let s = self.settings.get().clone();
                let _ = self.event_tx.send(CoreEvent::System(
                    SystemEvent::SettingsLoaded(s),
                )).await;
            }
            SystemCommand::MuteMic(muted) => {
                let _ = self.voice_service.send(VoiceRequest::Command(MumbleCommand::MuteSelf(muted))).await;
            }
            SystemCommand::Deafen(deafened) => {
                let _ = self.voice_service.send(VoiceRequest::Command(MumbleCommand::DeafenSelf(deafened))).await;
            }
            SystemCommand::OpenMumbleGui(extra_args) => {
                if let Some(creds) = self.voice.creds().cloned() {
                    self.launch_voice(creds, true, &extra_args).await;
                }
            }
            SystemCommand::RestartMumble(extra_args) => {
                if let Some(creds) = self.voice.creds().cloned() {
                    self.launch_voice(creds, false, &extra_args).await;
                }
            }
            SystemCommand::SetLogLevel(level) => {
                crate::logger::set_level(&level);
            }
            SystemCommand::TestError => {
                log::error!("Test error triggered from Developer Options");
            }
            SystemCommand::SetDeafenSuppressesNotifs(value) => {
                self.settings.update(|s| s.deafen_suppresses_notifs = Some(value));
            }
            SystemCommand::HideDm { room_id } => {
                self.settings.update(|s| s.hide_dm(room_id));
            }
            SystemCommand::UnhideDm { room_id } => {
                self.settings.update(|s| s.unhide_dm(&room_id));
            }
            SystemCommand::AcceptMumbleCert { host, port, fingerprint } => {
                log::info!("User accepted new cert for {}:{}", host, port);
                // Sent to the voice actor because a contended sqlite write would block
                // this loop for rusqlite's busy timeout.
                // The actor serves requests in order, so the write lands before the
                // launch below reads the fingerprint back.
                if !self.voice_service.send(VoiceRequest::AcceptCert {
                    host: host.clone(), port, fingerprint,
                }).await {
                    log::error!(
                        "Voice actor is not accepting requests; the cert accepted for \
                         {host}:{port} was not stored",
                    );
                }
                // Resume the launch even if the write failed, so the session cannot
                // stick in `AwaitingCert`.
                match std::mem::replace(&mut self.voice, VoiceSession::Idle) {
                    VoiceSession::AwaitingCert { creds, show_gui, extra_args } => {
                        self.voice = VoiceSession::Down { creds: creds.clone() };
                        self.launch_voice(creds, show_gui, &extra_args).await;
                    }
                    other => self.voice = other,
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
                        let _ = self.matrix.send(MatrixRequest::Subscribe(room_id.to_string())).await;
                    }
                    InternalMatrixEvent::SyncDegraded { generation, reason } => {
                        log::warn!(
                            "Matrix sync #{generation} degraded ({reason}); retrying in place, \
                             session untouched",
                        );
                        self.on_sync_report(generation, SyncReport::Degraded, retry_timer).await;
                    }
                    InternalMatrixEvent::SyncRecovered { generation } => {
                        log::info!("Matrix sync #{generation} recovered without a reconnect");
                        self.on_sync_report(generation, SyncReport::Recovered, retry_timer).await;
                    }
                    InternalMatrixEvent::Disconnected { generation, end } => {
                        match &end {
                            SyncEnd::SessionInvalidated { reason } => log::error!(
                                "Matrix session #{generation} invalidated by the server: {reason}",
                            ),
                            SyncEnd::RetriesExhausted { reason } => log::warn!(
                                "Matrix sync #{generation} stopped after retrying: {reason}",
                            ),
                        }
                        self.on_sync_report(generation, SyncReport::Ended(end), retry_timer).await;
                    }
                    InternalMatrixEvent::ConnectFinished { generation, outcome } => {
                        self.finish_connect(generation, outcome, retry_timer).await;
                    }
                    InternalMatrixEvent::SignOutFinished { outcome } => {
                        self.finish_sign_out(outcome, retry_timer).await;
                    }
                    InternalMatrixEvent::VoiceUserResolved {
                        session_id, name, volume_db, display_name, avatar_url,
                    } => {
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
                }
            }
            InternalEvent::Mumble(evt) => {
                match evt {
                    InternalMumbleEvent::UserJoined { session_id, name, volume_db } => {
                        let _ = self.matrix.send(MatrixRequest::ResolveVoiceUser {
                            session_id,
                            name,
                            volume_db,
                            internal_tx: self.internal_tx.clone(),
                        }).await;
                    }
                    InternalMumbleEvent::ConnectionLost { reason } => {
                        log::info!("Voice connection lost: {}", reason);
                        self.voice = match self.voice.creds().cloned() {
                            Some(creds) => VoiceSession::Down { creds },
                            None => VoiceSession::Idle,
                        };
                    }
                    InternalMumbleEvent::LaunchStarted { generation } => {
                        let Some(pending) = &self.pending_launch else { return };
                        if pending.generation != generation {
                            return;
                        }
                        self.voice = VoiceSession::Launching { creds: pending.creds.clone() };
                    }
                    InternalMumbleEvent::LaunchFinished { generation, outcome } => {
                        self.finish_launch(generation, outcome).await;
                    }
                    InternalMumbleEvent::Connected => {
                        // Only a launch we issued can complete; any other Connected is
                        // a Mumble still on the previous server.
                        let launched = match &self.voice {
                            VoiceSession::Launching { creds } => Some(creds.clone()),
                            other => {
                                log::warn!(
                                    "Voice reported Connected with no launch outstanding (session is {})",
                                    other.state_name(),
                                );
                                None
                            }
                        };
                        if let Some(creds) = launched {
                            self.voice = VoiceSession::Up { creds };
                        }
                        let (use_mumble_settings, mode, vad_threshold, voice_hold) = {
                            let s = self.settings.get();
                            (s.use_mumble_settings, s.transmission_mode.clone(), s.vad_threshold, s.voice_hold)
                        };
                        if use_mumble_settings != Some(true) {
                            if let Some(mode) = mode {
                                self.send_voice_command(MumbleCommand::SetTransmissionMode(mode)).await;
                            }
                            if let Some(value) = vad_threshold {
                                self.send_voice_command(MumbleCommand::SetVadThreshold(value)).await;
                            }
                            if let Some(value) = voice_hold {
                                self.send_voice_command(MumbleCommand::SetVoiceHold(value)).await;
                            }
                        }
                        // Restore mute/deafen from the previous session. Send each flag
                        // independently rather than leaning on deafen's implicit mute: an
                        // explicitly muted user must stay muted after they later undeafen.
                        if self.voice_restore.muted {
                            self.send_voice_command(MumbleCommand::MuteSelf(true)).await;
                        }
                        if self.voice_restore.deafened {
                            self.send_voice_command(MumbleCommand::DeafenSelf(true)).await;
                        }
                    }
                    InternalMumbleEvent::LocalChannelChanged { channel_path } => {
                        self.voice_restore.channel_path = Some(channel_path);
                    }
                    InternalMumbleEvent::LocalMuteChanged(muted) => {
                        self.voice_restore.muted = muted;
                    }
                    InternalMumbleEvent::LocalDeafChanged(deafened) => {
                        self.voice_restore.deafened = deafened;
                    }
                }
            }
            InternalEvent::System(evt) => match evt {
                #[cfg(test)]
                crate::events::InternalSystemEvent::Barrier(reply) => self.pending_barriers.push(reply),
            },
        }
    }

    async fn send_voice_command(&self, cmd: MumbleCommand) {
        let _ = self.voice_service.send(VoiceRequest::Command(cmd)).await;
    }

    async fn dispatch_mumble_command(&mut self, cmd: MumbleCommand) {
        match &cmd {
            MumbleCommand::SetTransmissionMode(mode) => {
                let mode = mode.clone();
                self.settings.update(move |s| s.transmission_mode = Some(mode));
            }
            &MumbleCommand::SetVadThreshold(value) => {
                self.settings.update(move |s| s.vad_threshold = Some(value));
            }
            &MumbleCommand::SetVoiceHold(value) => {
                self.settings.update(move |s| s.voice_hold = Some(value));
            }
            &MumbleCommand::SetUseMumbleSettings(value) => {
                self.settings.update(move |s| s.use_mumble_settings = Some(value));
                return;
            }
            _ => {}
        }

        self.send_voice_command(cmd).await;
    }

    /// A connect asked for while a sign out is in flight waits for its answer, so the two
    /// cannot interleave.
    async fn connect_on_request(
        &mut self,
        form: ServerConnectionForm,
        retry_timer: &mut Pin<Box<Sleep>>,
    ) {
        if let Some(signing_out) = &mut self.pending_sign_out {
            signing_out.then_connect = Some(form);
            return;
        }
        self.conn.form = Some(form.clone());
        self.conn.retries = 0;
        self.connect_to_server(&form, retry_timer).await;
    }

    /// Served behind whatever the actor is already doing, a connect included, so a login
    /// still being made is logged out instead of being left on the server.
    async fn sign_out(&mut self, retry_timer: &mut Pin<Box<Sleep>>) {
        if self.pending_sign_out.is_some() {
            log::debug!("Ignoring a sign out: one is already in flight");
            return;
        }
        self.pending_sign_out = Some(PendingSignOut::default());

        let Some(form) = self.conn.form.clone() else {
            self.finish_sign_out(Ok(()), retry_timer).await;
            return;
        };
        let dispatched = self.matrix.send(MatrixRequest::SignOut {
            form,
            internal_tx: self.internal_tx.clone(),
        }).await;

        // An undispatched request never answers; fail it here so the frontend is not left waiting.
        if !dispatched {
            log::error!("Matrix actor is not accepting requests; failing the sign out");
            self.finish_sign_out(Err(SIGN_OUT_UNSERVED.into()), retry_timer).await;
        }
    }

    async fn finish_sign_out(
        &mut self,
        outcome: Result<(), String>,
        retry_timer: &mut Pin<Box<Sleep>>,
    ) {
        let Some(signing_out) = self.pending_sign_out.take() else {
            log::debug!("Discarding a sign out result: none was outstanding");
            return;
        };
        let next = signing_out.then_connect.filter(|_| !self.shutting_down);

        match outcome {
            Ok(()) => {
                log::info!("Signed out");
                // A report the session sent on its way out must not arm a retry.
                self.live_generation = None;
                let _ = self.event_tx.send(CoreEvent::System(SystemEvent::ServerReset)).await;
                self.conn.disconnect(&self.event_tx).await;
                let _ = self.event_tx.send(CoreEvent::System(SystemEvent::SignedOut)).await;
                self.end_voice().await;
                if let Some(form) = next {
                    self.connect_on_request(form, retry_timer).await;
                }
            }
            Err(reason) => {
                log::warn!("Sign out failed: {reason}");
                // The actor stopped the session's sync before it tried, so the session is
                // brought back the way a dropped one is; its saved login needs no password.
                match (next, self.conn.form.clone()) {
                    (Some(form), _) => self.connect_on_request(form, retry_timer).await,
                    (None, Some(form)) if !self.shutting_down => {
                        self.connect_to_server(&form, retry_timer).await;
                    }
                    _ => {}
                }
                // After the reconnect's `ServerReset`, so the frontend shows the reason on the session it now has.
                let _ = self.event_tx.send(CoreEvent::System(
                    SystemEvent::SignOutFailed { reason },
                )).await;
            }
        }
    }

    /// A reconnect leaves voice alone; this is for the user leaving the server, so the
    /// state that outlives a Mumble restart goes with it.
    async fn end_voice(&mut self) {
        self.voice_restore = VoiceRestoreState::default();
        let was_launching = self.pending_launch.take().is_some();
        if !was_launching && matches!(self.voice, VoiceSession::Idle) {
            return;
        }
        self.voice = VoiceSession::Idle;
        if !self.voice_service.send(VoiceRequest::Shutdown).await && !self.shutting_down {
            log::error!("Voice actor is not accepting requests; Mumble was not stopped");
        }
    }

    /// A new request supersedes an attempt in flight rather than queueing, since the
    /// user has moved on from it.
    async fn connect_to_server(
        &mut self,
        form: &ServerConnectionForm,
        retry_timer: &mut Pin<Box<Sleep>>,
    ) {
        if let Some(previous) = self.pending_connect.take() {
            log::info!(
                "Superseding Matrix connect #{} after {:?}",
                previous.generation,
                previous.started.elapsed(),
            );
            let _ = previous.cancel.send(());
        }

        self.connect_generation += 1;
        let generation = self.connect_generation;

        // `ServerReset` must be sent before the actor starts, so it cannot overtake the
        // new session's data.
        let _ = self.event_tx.send(CoreEvent::System(SystemEvent::ServerReset)).await;
        self.conn.begin(&self.event_tx).await;
        // The actor resets the backend before connecting, which ends the live session.
        self.live_generation = None;

        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        self.pending_connect = Some(PendingConnect {
            generation,
            form: form.clone(),
            cancel: cancel_tx,
            started: Instant::now(),
            held_sync: None,
        });

        let dispatched = self.matrix.send(MatrixRequest::Connect {
            form: form.clone(),
            internal_tx: self.internal_tx.clone(),
            generation,
            cancel: cancel_rx,
        }).await;

        // An undispatched request never answers; fail it here so the retry timer re-arms.
        if !dispatched {
            log::error!("Matrix actor is not accepting requests; failing connect #{generation}");
            self.finish_connect(generation, ConnectOutcome::Failed, retry_timer).await;
        }

        if form.mumble_host.is_some() {
            self.resolve_and_launch_voice(form, None, false, "").await;
        }
    }

    async fn on_sync_report(
        &mut self,
        generation: u64,
        report: SyncReport,
        retry_timer: &mut Pin<Box<Sleep>>,
    ) {
        if let Some(pending) = &mut self.pending_connect
            && pending.generation == generation
        {
            // An end is final for the session, so no later health report may displace it.
            if !matches!(pending.held_sync, Some(SyncReport::Ended(_))) {
                pending.held_sync = Some(report);
            }
            return;
        }
        if self.live_generation != Some(generation) {
            log::debug!("Ignoring a sync report from session #{generation}: it is not live");
            return;
        }
        self.apply_sync_report(report, retry_timer).await;
    }

    async fn apply_sync_report(&mut self, report: SyncReport, retry_timer: &mut Pin<Box<Sleep>>) {
        match report {
            SyncReport::Degraded => self.conn.degraded(&self.event_tx).await,
            SyncReport::Recovered => self.conn.recovered(&self.event_tx).await,
            SyncReport::Ended(end) => {
                self.live_generation = None;
                self.conn.schedule_retry(retry_timer, end.reason().to_string(), &self.event_tx)
                    .await;
            }
        }
    }

    async fn finish_connect(
        &mut self,
        generation: u64,
        outcome: ConnectOutcome,
        retry_timer: &mut Pin<Box<Sleep>>,
    ) {
        let Some(pending) = self.pending_connect.take() else {
            log::debug!("Discarding connect #{generation}: nothing was outstanding");
            return;
        };
        if pending.generation != generation {
            log::info!("Discarding superseded connect #{generation}");
            self.pending_connect = Some(pending);
            return;
        }

        log::info!("Matrix connect #{generation} took {:?}", pending.started.elapsed());

        let voice_server = self.conn.settle(outcome, retry_timer, &self.event_tx).await;
        if !matches!(self.conn.state, ConnectionState::Connected) {
            return;
        }
        self.live_generation = Some(generation);

        if pending.form.mumble_host.is_none() {
            self.resolve_and_launch_voice(&pending.form, voice_server, false, "").await;
        }
        // Applied after the voice decision, as if it had arrived just after the result.
        if let Some(report) = pending.held_sync {
            self.apply_sync_report(report, retry_timer).await;
        }
    }

    async fn finish_launch(&mut self, generation: u64, outcome: LaunchOutcome) {
        let Some(pending) = self.pending_launch.take() else {
            log::debug!("Discarding voice launch #{generation}: nothing was outstanding");
            return;
        };
        if pending.generation != generation {
            log::info!("Discarding superseded voice launch #{generation}");
            self.pending_launch = Some(pending);
            return;
        }

        match outcome {
            // `LaunchStarted` and `Connected` may already have moved the session on.
            LaunchOutcome::Launched => {}
            LaunchOutcome::Failed => {
                self.voice = VoiceSession::Down { creds: pending.creds };
            }
            LaunchOutcome::CertChanged => {
                // Mumble was left alone and is not on this server, so a later reconnect
                // must re-attempt.
                self.voice = VoiceSession::AwaitingCert {
                    creds: pending.creds,
                    show_gui: pending.show_gui,
                    extra_args: pending.extra_args,
                };
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
    ) {
        let new_creds = Self::resolve_mumble_credentials(form, voice_server);

        // Only `Up` is live; relaunching on a sync hiccup would drop the user out of
        // voice for seconds.
        if matches!(self.voice, VoiceSession::Up { ref creds } if creds == &new_creds) {
            log::debug!(
                "Voice already connected to {}:{}, keeping the session",
                new_creds.host, new_creds.port,
            );
            return;
        }

        if self.voice.creds() != Some(&new_creds) {
            self.voice_restore = VoiceRestoreState::default();
        }

        self.launch_voice(new_creds, show_gui, extra_args).await;
    }

    /// `self.voice` is left alone until `LaunchStarted`, because Mumble is still on the
    /// old server during the TLS probe.
    ///
    /// Refuses once shutdown has begun, before `pending_launch` is set, or `shut_down`
    /// would wait out the grace.
    async fn launch_voice(&mut self, creds: VoiceServerConfig, show_gui: bool, extra_args: &str) {
        if self.shutting_down {
            log::info!(
                "Not starting a voice session for {}:{}: the app is shutting down",
                creds.host, creds.port,
            );
            return;
        }

        self.launch_generation += 1;
        let generation = self.launch_generation;
        if let Some(previous) = self.pending_launch.replace(PendingLaunch {
            generation,
            creds: creds.clone(),
            show_gui,
            extra_args: extra_args.to_string(),
        }) {
            log::info!("Voice launch #{} superseded by #{generation}", previous.generation);
        }

        let dispatched = self.voice_service.send(VoiceRequest::Launch(LaunchRequest {
            creds,
            show_gui,
            extra_args: extra_args.to_string(),
            channel_path: self.voice_restore.channel_path.clone(),
            internal_tx: self.internal_tx.clone(),
            generation,
        })).await;

        if !dispatched {
            log::error!("Voice actor is not accepting requests; failing launch #{generation}");
            self.finish_launch(generation, LaunchOutcome::Failed).await;
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

    /// `step` sends a command and waits for the engine to settle, since a connect no
    /// longer finishes before the next command is read.
    struct EngineDriver {
        cmd_tx: mpsc::Sender<CoreCommand>,
        internal_tx: mpsc::Sender<InternalEvent>,
        event_rx: mpsc::Receiver<CoreEvent>,
        engine: tokio::task::JoinHandle<()>,
    }

    impl EngineDriver {
        fn start(engine: CoreEngine, cmd_tx: mpsc::Sender<CoreCommand>, event_rx: mpsc::Receiver<CoreEvent>) -> Self {
            let internal_tx = engine.internal_tx.clone();
            Self {
                cmd_tx,
                internal_tx,
                event_rx,
                engine: tokio::spawn(async move { engine.run().await }),
            }
        }

        async fn send(&self, cmd: CoreCommand) {
            self.cmd_tx.send(cmd).await.expect("engine already stopped");
        }

        async fn inject(&self, event: InternalEvent) {
            self.internal_tx.send(event).await.expect("engine already stopped");
        }

        async fn settle(&self) {
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            self.internal_tx
                .send(InternalEvent::System(InternalSystemEvent::Barrier(reply_tx)))
                .await
                .expect("engine already stopped");
            timeout(Duration::from_secs(5), reply_rx)
                .await
                .expect("engine did not settle within 5s")
                .expect("engine dropped the barrier");
        }

        async fn step(&self, cmd: CoreCommand) {
            self.send(cmd).await;
            self.settle().await;
        }

        async fn finish(mut self) -> Vec<CoreEvent> {
            drop(self.cmd_tx);
            timeout(Duration::from_secs(5), self.engine)
                .await
                .expect("engine did not shut down within 5s")
                .expect("engine task panicked");

            let mut events = Vec::new();
            while let Ok(event) = self.event_rx.try_recv() {
                events.push(event);
            }
            events
        }
    }

    fn build_engine(
        matrix: MockMatrix,
        voice: MockVoice,
        data_dir: &std::path::Path,
    ) -> (CoreEngine, mpsc::Sender<CoreCommand>, mpsc::Receiver<CoreEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let (_media_tx, media_rx) = mpsc::channel(32);
        let (event_tx, event_rx) = mpsc::channel(100);
        let engine = CoreEngine::new(
            cmd_rx, media_rx, event_tx, matrix, voice,
            data_dir.to_path_buf(), settings::load(data_dir),
        );
        (engine, cmd_tx, event_rx)
    }

    async fn run_commands(
        matrix: MockMatrix,
        voice: MockVoice,
        data_dir: &std::path::Path,
        commands: Vec<CoreCommand>,
    ) -> (Vec<CoreEvent>, Arc<MockMatrixState>, Arc<MockVoiceState>) {
        let matrix_state = matrix.state.clone();
        let voice_state = voice.state.clone();

        let (engine, cmd_tx, event_rx) = build_engine(matrix, voice, data_dir);
        let driver = EngineDriver::start(engine, cmd_tx, event_rx);

        for cmd in commands {
            driver.step(cmd).await;
        }

        (driver.finish().await, matrix_state, voice_state)
    }

    fn seed_settings(
        data_dir: &std::path::Path,
        change: impl FnOnce(&mut settings::Settings),
    ) {
        let mut s = settings::load(data_dir);
        change(&mut s);
        settings::save(data_dir, &s).unwrap();
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
        seed_settings(tmp.path(), |s| {
            s.hide_dm("!room1:example.com".into());
            s.hide_dm("!room2:example.com".into());
        });

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
        let (media_tx, media_rx) = mpsc::channel(32);
        let (event_tx, _event_rx) = mpsc::channel(100);
        let matrix = MockMatrix::new();
        let voice = MockVoice::new();
        let engine = CoreEngine::new(
            cmd_rx, media_rx, event_tx, matrix, voice,
            tmp.path().to_path_buf(), settings::load(tmp.path()),
        );

        let engine_handle = tokio::spawn(async move { engine.run().await });

        media_tx.send(MediaRequest {
            mxc_url: "mxc://example.com/abc".into(),
            respond: respond_tx,
        }).await.unwrap();

        let result = timeout(Duration::from_secs(2), respond_rx).await
            .expect("timed out waiting for media response")
            .expect("oneshot dropped");

        assert_eq!(result.unwrap(), vec![0xDE, 0xAD]);

        drop(cmd_tx);
        timeout(Duration::from_secs(2), engine_handle)
            .await
            .expect("engine did not shut down")
            .expect("engine task panicked");
    }

    #[tokio::test]
    async fn a_media_request_that_does_not_fit_is_refused_rather_than_left_waiting() {
        const ACTOR_QUEUE: usize = 256;
        const OVERFLOW: usize = 4;

        let tmp = tempfile::tempdir().unwrap();
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel();
        let matrix = MockMatrix::new().with_connect_gate(gate_rx);

        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let (media_tx, media_rx) = mpsc::channel(ACTOR_QUEUE + OVERFLOW);
        let (event_tx, mut event_rx) = mpsc::channel(100);
        let engine = CoreEngine::new(
            cmd_rx, media_rx, event_tx, matrix, MockVoice::new(),
            tmp.path().to_path_buf(), settings::load(tmp.path()),
        );
        let engine_handle = tokio::spawn(async move { engine.run().await });

        // The actor serves no media while a connect holds it, so its queue fills.
        cmd_tx.send(CoreCommand::System(SystemCommand::ConnectToServer(connect_form())))
            .await.unwrap();
        wait_for_connecting(&mut event_rx).await;

        let mut replies = Vec::new();
        for i in 0..ACTOR_QUEUE + OVERFLOW {
            let (tx, rx) = tokio::sync::oneshot::channel();
            replies.push(rx);
            media_tx.send(MediaRequest { mxc_url: format!("mxc://example.com/{i}"), respond: tx })
                .await.unwrap();
        }

        let overflow = replies.split_off(ACTOR_QUEUE);
        for reply in overflow {
            let answer = timeout(Duration::from_secs(2), reply).await
                .expect("a media request that did not fit was left waiting")
                .expect("the reply was dropped unanswered");
            assert!(answer.is_err(), "an overflowing request must be refused, got {answer:?}");
        }

        let _ = gate_tx.send(());
        for reply in replies {
            let answer = timeout(Duration::from_secs(2), reply).await
                .expect("a queued media request was never served")
                .expect("the reply was dropped unanswered");
            assert_eq!(answer, Ok(vec![0xDE, 0xAD]));
        }

        drop(cmd_tx);
        let _ = timeout(Duration::from_secs(5), engine_handle).await;
    }

    /// The bound is three, not two, because a burst can straddle a coalescing window on
    /// a loaded runner.
    #[tokio::test]
    async fn a_slider_drag_does_not_cost_a_write_per_event() {
        const EVENTS: usize = 200;
        let tmp = tempfile::tempdir().unwrap();

        let (cmd_tx, cmd_rx) = mpsc::channel(EVENTS + 1);
        let (_media_tx, media_rx) = mpsc::channel(32);
        let (event_tx, _event_rx) = mpsc::channel(100);
        let engine = CoreEngine::new(
            cmd_rx, media_rx, event_tx, MockMatrix::new(), MockVoice::new(),
            tmp.path().to_path_buf(), settings::load(tmp.path()),
        );
        let writes = engine.settings.write_counter();
        let engine_handle = tokio::spawn(async move { engine.run().await });

        for i in 0..EVENTS {
            cmd_tx.send(CoreCommand::Mumble(MumbleCommand::SetVadThreshold(i as f64 / 1000.0)))
                .await.unwrap();
        }
        drop(cmd_tx);
        timeout(Duration::from_secs(5), engine_handle)
            .await
            .expect("engine did not shut down")
            .expect("engine task panicked");

        let performed = writes.count();
        assert!(
            performed <= 3,
            "{EVENTS} slider events cost {performed} settings writes; \
             the drag should have collapsed to a couple of coalesced writes",
        );
        assert_eq!(
            settings::load(tmp.path()).vad_threshold,
            Some((EVENTS - 1) as f64 / 1000.0),
            "the value the drag ended on must be what is on disk",
        );
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

        let creds = CoreEngine::resolve_mumble_credentials(&form, voice_server);
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

        let creds = CoreEngine::resolve_mumble_credentials(&form, voice_server);
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

        let creds = CoreEngine::resolve_mumble_credentials(&form, None);
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
        seed_settings(tmp.path(), |s| {
            s.transmission_mode = Some("push_to_talk".into());
            s.vad_threshold = Some(0.42);
            s.voice_hold = Some(250);
        });

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
        seed_settings(tmp.path(), |s| {
            s.transmission_mode = Some("push_to_talk".into());
            s.vad_threshold = Some(0.42);
            s.voice_hold = Some(250);
            s.use_mumble_settings = Some(true);
        });

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
        seed_settings(tmp.path(), |s| s.bookmarks = vec![bookmark]);

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

    async fn wait_for_connecting(event_rx: &mut mpsc::Receiver<CoreEvent>) {
        timeout(Duration::from_secs(2), async {
            while let Some(e) = event_rx.recv().await {
                if matches!(
                    e,
                    CoreEvent::Matrix(MatrixEvent::ConnectionState(ConnectionState::Connecting))
                ) {
                    return;
                }
            }
            panic!("engine never reported Connecting");
        }).await.expect("engine never reported Connecting");
    }

    fn conn_states(events: &[CoreEvent]) -> Vec<&ConnectionState> {
        events.iter().filter_map(|e| match e {
            CoreEvent::Matrix(MatrixEvent::ConnectionState(s)) => Some(s),
            _ => None,
        }).collect()
    }

    /// A non-zero `linger` runs under `start_paused` and must exceed the first backoff,
    /// so a scheduled teardown would have fired.
    async fn connect_then_inject(
        data_dir: &std::path::Path,
        events: Vec<InternalEvent>,
        linger: Duration,
    ) -> (Vec<CoreEvent>, Arc<MockMatrixState>) {
        let matrix = MockMatrix::new().with_repeating_connect_result(
            ConnectOutcome::Connected(None),
        );
        let matrix_state = matrix.state.clone();
        let (engine, cmd_tx, event_rx) = build_engine(matrix, MockVoice::new(), data_dir);
        let driver = EngineDriver::start(engine, cmd_tx, event_rx);

        driver.step(CoreCommand::System(SystemCommand::ConnectToServer(connect_form()))).await;
        for event in events {
            driver.inject(event).await;
            driver.settle().await;
        }
        if !linger.is_zero() {
            tokio::time::sleep(linger).await;
            driver.settle().await;
        }

        (driver.finish().await, matrix_state)
    }

    /// Longer than `backoff_secs(1)`, so a scheduled retry would have fired.
    const PAST_THE_FIRST_BACKOFF: Duration = Duration::from_millis(2_500);

    #[tokio::test(start_paused = true)]
    async fn a_transient_sync_failure_does_not_tear_the_session_down() {
        let tmp = tempfile::tempdir().unwrap();
        let (events, matrix_state) = connect_then_inject(tmp.path(), vec![
            InternalEvent::Matrix(InternalMatrixEvent::SyncDegraded {
                generation: 1,
                reason: "Sync error: error sending request".into(),
            }),
        ], PAST_THE_FIRST_BACKOFF).await;

        use crate::test_mocks::MockCall;
        assert_eq!(
            matrix_state.call_log.lock().unwrap().as_slice(),
            &[MockCall::Reset, MockCall::Connect],
            "a retried sync failure must not reset the backend or reconnect it",
        );

        let resets = events.iter().filter(|e| matches!(
            e, CoreEvent::System(SystemEvent::ServerReset)
        )).count();
        assert_eq!(
            resets, 1,
            "only the connect itself may emit ServerReset; the degradation must not",
        );

        assert!(
            matches!(conn_states(&events).last(), Some(ConnectionState::Connecting)),
            "the UI should be told the connection is degraded, got {:?}",
            conn_states(&events),
        );
        assert!(
            !events.iter().any(|e| matches!(
                e, CoreEvent::Matrix(MatrixEvent::ConnectionState(ConnectionState::Failed { .. }))
            )),
            "a failure being retried is not a failed connection",
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_recovered_sync_returns_to_connected_without_reconnecting() {
        let tmp = tempfile::tempdir().unwrap();
        let (events, matrix_state) = connect_then_inject(tmp.path(), vec![
            InternalEvent::Matrix(InternalMatrixEvent::SyncDegraded {
                generation: 1,
                reason: "blip".into(),
            }),
            InternalEvent::Matrix(InternalMatrixEvent::SyncRecovered { generation: 1 }),
        ], PAST_THE_FIRST_BACKOFF).await;

        use crate::test_mocks::MockCall;
        assert_eq!(
            matrix_state.call_log.lock().unwrap().as_slice(),
            &[MockCall::Reset, MockCall::Connect],
            "recovering from a blip must not have cost a reconnect",
        );

        let states = conn_states(&events);
        assert!(
            matches!(
                states.as_slice(),
                [ConnectionState::Connecting, ConnectionState::Connected,
                 ConnectionState::Connecting, ConnectionState::Connected],
            ),
            "expected connect, degrade, recover, got {states:?}",
        );
    }

    #[tokio::test]
    async fn a_sync_loop_that_gave_up_falls_back_to_the_reconnect_path() {
        let tmp = tempfile::tempdir().unwrap();
        let (events, _) = connect_then_inject(tmp.path(), vec![
            InternalEvent::Matrix(InternalMatrixEvent::Disconnected {
                generation: 1,
                end: SyncEnd::RetriesExhausted { reason: "out of retries".into() },
            }),
        ], Duration::ZERO).await;

        assert!(
            matches!(
                conn_states(&events).last(),
                Some(ConnectionState::Failed { reason, retries: 1, .. }) if reason == "out of retries",
            ),
            "giving up must schedule a retry and carry its reason, got {:?}",
            conn_states(&events),
        );
    }

    #[tokio::test]
    async fn an_invalidated_session_is_distinguishable_from_exhausted_retries() {
        let tmp = tempfile::tempdir().unwrap();
        let (events, _) = connect_then_inject(tmp.path(), vec![
            InternalEvent::Matrix(InternalMatrixEvent::Disconnected {
                generation: 1,
                end: SyncEnd::SessionInvalidated { reason: "M_UNKNOWN_TOKEN".into() },
            }),
        ], Duration::ZERO).await;

        assert!(
            matches!(
                conn_states(&events).last(),
                Some(ConnectionState::Failed { reason, retries: 1, .. }) if reason == "M_UNKNOWN_TOKEN",
            ),
            "a rejected session must still drive the disconnect-and-retry path, got {:?}",
            conn_states(&events),
        );

        assert_ne!(
            SyncEnd::SessionInvalidated { reason: "x".into() },
            SyncEnd::RetriesExhausted { reason: "x".into() },
        );
    }

    fn sign_out() -> CoreCommand {
        CoreCommand::System(SystemCommand::SignOut)
    }

    fn session_rejected(generation: u64) -> InternalEvent {
        InternalEvent::Matrix(InternalMatrixEvent::Disconnected {
            generation,
            end: SyncEnd::SessionInvalidated { reason: "M_UNKNOWN_TOKEN".into() },
        })
    }

    fn position(events: &[CoreEvent], what: &str, is: impl Fn(&CoreEvent) -> bool) -> usize {
        events.iter().position(is).unwrap_or_else(|| panic!("the engine never emitted {what}"))
    }

    #[tokio::test(start_paused = true)]
    async fn signing_out_leaves_the_connection_disconnected_voice_ended_and_nothing_retrying() {
        use crate::test_mocks::MockCall;

        let tmp = tempfile::tempdir().unwrap();
        let (answer_sign_out, sign_out_held) = tokio::sync::oneshot::channel();
        let matrix = MockMatrix::new()
            .with_repeating_connect_result(ConnectOutcome::Connected(None))
            .with_sign_out_gate(sign_out_held);
        let matrix_state = matrix.state.clone();
        let voice = MockVoice::new().with_internal_events(vec![
            InternalEvent::Mumble(InternalMumbleEvent::Connected),
            InternalEvent::Mumble(InternalMumbleEvent::LocalChannelChanged {
                channel_path: "Voice/General".into(),
            }),
            InternalEvent::Mumble(InternalMumbleEvent::LocalMuteChanged(true)),
        ]).with_internal_events(vec![
            // The point at which a mute carried over from before would be sent again.
            InternalEvent::Mumble(InternalMumbleEvent::Connected),
        ]);
        let voice_state = voice.state.clone();
        let (engine, cmd_tx, event_rx) = build_engine(matrix, voice, tmp.path());
        let driver = EngineDriver::start(engine, cmd_tx, event_rx);

        driver.step(CoreCommand::System(SystemCommand::ConnectToServer(connect_form()))).await;
        // The sync has given up, so a retry is armed for the sign out to disarm.
        driver.inject(InternalEvent::Matrix(InternalMatrixEvent::Disconnected {
            generation: 1,
            end: SyncEnd::RetriesExhausted { reason: "out of retries".into() },
        })).await;
        driver.settle().await;

        // Held past the moment of that retry, which would otherwise log back in behind the sign out.
        driver.send(sign_out()).await;
        tokio::time::sleep(PAST_THE_FIRST_BACKOFF).await;
        answer_sign_out.send(()).unwrap();
        driver.settle().await;
        tokio::time::sleep(PAST_THE_FIRST_BACKOFF).await;
        driver.settle().await;
        assert_eq!(
            matrix_state.call_log.lock().unwrap().as_slice(),
            &[MockCall::Reset, MockCall::Connect, MockCall::SignOut],
            "nothing may reconnect a user who is signing out or has signed out",
        );
        assert_eq!(*voice_state.shutdown_count.lock().unwrap(), 1, "signing out leaves voice as well");

        driver.step(CoreCommand::System(SystemCommand::ConnectToServer(connect_form()))).await;
        let events = driver.finish().await;

        let signed_out = position(&events, "SignedOut", |e| {
            matches!(e, CoreEvent::System(SystemEvent::SignedOut))
        });
        let disconnected = position(&events, "a Disconnected connection state", |e| {
            matches!(e, CoreEvent::Matrix(MatrixEvent::ConnectionState(ConnectionState::Disconnected)))
        });
        assert!(
            matches!(events[disconnected - 1], CoreEvent::System(SystemEvent::ServerReset))
                && signed_out == disconnected + 1,
            "the frontend should see the session reset, then disconnected, then signed out",
        );
        assert!(
            events.iter().any(|e| matches!(
                e, CoreEvent::Mumble(MumbleEvent::ConnectionState(ConnectionState::Disconnected))
            )),
            "the frontend must be told voice ended, since a killed Mumble may not say so itself",
        );
        assert_eq!(
            voice_state.launched_channel_paths.lock().unwrap().as_slice(),
            &[None, None],
            "the channel remembered for a Mumble restart must not survive a sign out",
        );
        assert!(
            !voice_state.commands.lock().unwrap().contains(&MumbleCommand::MuteSelf(true)),
            "nor may the mute",
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_sign_out_reconnects_and_a_finished_one_is_not_taken_for_a_rejected_session() {
        use crate::test_mocks::MockCall;

        let tmp = tempfile::tempdir().unwrap();
        let matrix = MockMatrix::new()
            .with_repeating_connect_result(ConnectOutcome::Connected(None))
            .with_sign_out_result(Err("the server could not be reached".into()));
        let matrix_state = matrix.state.clone();
        let voice = MockVoice::new();
        let voice_state = voice.state.clone();
        let (engine, cmd_tx, event_rx) = build_engine(matrix, voice, tmp.path());
        let driver = EngineDriver::start(engine, cmd_tx, event_rx);

        driver.step(CoreCommand::System(SystemCommand::ConnectToServer(connect_form()))).await;
        driver.step(sign_out()).await;
        assert_eq!(
            *voice_state.shutdown_count.lock().unwrap(), 0,
            "a sign out that failed must leave voice alone",
        );

        driver.step(sign_out()).await;
        // What a sync that outlived the logout would report: the server no longer knows its token.
        driver.inject(session_rejected(2)).await;
        driver.settle().await;
        tokio::time::sleep(PAST_THE_FIRST_BACKOFF).await;
        driver.settle().await;

        let events = driver.finish().await;

        assert!(
            matches!(
                conn_states(&events).as_slice(),
                [ConnectionState::Connecting, ConnectionState::Connected,
                 ConnectionState::Connecting, ConnectionState::Connected,
                 ConnectionState::Disconnected],
            ),
            "the failed sign out should reconnect, and the one that succeeded should end \
             disconnected rather than failed, got {:?}",
            conn_states(&events),
        );
        assert_eq!(
            matrix_state.call_log.lock().unwrap().as_slice(),
            &[
                MockCall::Reset, MockCall::Connect,
                MockCall::SignOut, MockCall::Reset, MockCall::Connect,
                MockCall::SignOut,
            ],
            "nothing may try to connect again once the sign out has succeeded",
        );

        let reconnect_reset = events.iter().enumerate()
            .filter(|(_, e)| matches!(e, CoreEvent::System(SystemEvent::ServerReset)))
            .map(|(at, _)| at)
            .nth(1)
            .expect("the reconnect should have reset the session");
        let failed = position(&events, "SignOutFailed", |e| matches!(
            e, CoreEvent::System(SystemEvent::SignOutFailed { reason }) if reason == "the server could not be reached"
        ));
        assert!(
            reconnect_reset < failed,
            "the reason must follow the reset, which clears what the frontend holds",
        );
    }

    #[tokio::test]
    async fn a_connect_asked_for_during_a_sign_out_starts_once_it_has_answered() {
        let tmp = tempfile::tempdir().unwrap();
        let (gate, gate_rx) = tokio::sync::oneshot::channel();
        let matrix = MockMatrix::new()
            .with_repeating_connect_result(ConnectOutcome::Connected(None))
            .with_sign_out_gate(gate_rx);
        let (engine, cmd_tx, event_rx) = build_engine(matrix, MockVoice::new(), tmp.path());
        let driver = EngineDriver::start(engine, cmd_tx, event_rx);

        driver.step(CoreCommand::System(SystemCommand::ConnectToServer(connect_form()))).await;
        driver.send(sign_out()).await;
        driver.send(CoreCommand::System(SystemCommand::ConnectToServer(connect_form()))).await;
        // Long enough for the engine to have taken both commands while the sign out is held.
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        gate.send(()).unwrap();
        driver.settle().await;

        let events = driver.finish().await;
        assert!(
            matches!(
                conn_states(&events).as_slice(),
                [ConnectionState::Connecting, ConnectionState::Connected,
                 ConnectionState::Disconnected,
                 ConnectionState::Connecting, ConnectionState::Connected],
            ),
            "the sign out must be over before the next connect shows, got {:?}",
            conn_states(&events),
        );
    }

    #[tokio::test]
    async fn reconnect_resets_each_time() {
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

    #[tokio::test]
    async fn reconnect_keeps_a_live_voice_session_on_the_same_server() {
        let tmp = tempfile::tempdir().unwrap();
        let matrix = MockMatrix::new().with_repeating_connect_result(ConnectOutcome::Connected(None));
        let voice = MockVoice::new().with_internal_events(vec![
            InternalEvent::Mumble(InternalMumbleEvent::Connected),
        ]);

        let (_events, _, voice_state) = run_commands(
            matrix, voice, tmp.path(),
            vec![
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
            ],
        ).await;

        let launches = voice_state.launched_with.lock().unwrap();
        assert_eq!(
            launches.len(), 1,
            "a reconnect to the same voice server should not relaunch Mumble, got {:?}",
            launches,
        );
    }

    #[tokio::test]
    async fn reconnect_relaunches_voice_when_it_is_not_connected() {
        let tmp = tempfile::tempdir().unwrap();
        let matrix = MockMatrix::new().with_repeating_connect_result(ConnectOutcome::Connected(None));
        let voice = MockVoice::new();

        let (_events, _, voice_state) = run_commands(
            matrix, voice, tmp.path(),
            vec![
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
            ],
        ).await;

        let launches = voice_state.launched_with.lock().unwrap();
        assert_eq!(launches.len(), 2, "voice that is down should be relaunched");
    }

    #[tokio::test]
    async fn reconnect_relaunches_after_the_voice_connection_drops() {
        let tmp = tempfile::tempdir().unwrap();
        let matrix = MockMatrix::new().with_repeating_connect_result(ConnectOutcome::Connected(None));
        let voice = MockVoice::new()
            .with_internal_events(vec![
                InternalEvent::Mumble(InternalMumbleEvent::Connected),
                InternalEvent::Mumble(InternalMumbleEvent::ConnectionLost {
                    reason: "voice server went away".into(),
                }),
            ]);

        let (_events, _, voice_state) = run_commands(
            matrix, voice, tmp.path(),
            vec![
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
            ],
        ).await;

        let launches = voice_state.launched_with.lock().unwrap();
        assert_eq!(launches.len(), 2, "a dropped voice session should be relaunched");
    }

    #[tokio::test]
    async fn connecting_to_a_different_voice_server_relaunches() {
        let tmp = tempfile::tempdir().unwrap();
        let matrix = MockMatrix::new().with_repeating_connect_result(ConnectOutcome::Connected(None));
        let voice = MockVoice::new().with_internal_events(vec![
            InternalEvent::Mumble(InternalMumbleEvent::Connected),
        ]);

        let mut second = connect_form();
        second.mumble_host = Some("other.example.com".into());

        let (_events, _, voice_state) = run_commands(
            matrix, voice, tmp.path(),
            vec![
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
                CoreCommand::System(SystemCommand::ConnectToServer(second)),
            ],
        ).await;

        let launches = voice_state.launched_with.lock().unwrap();
        assert_eq!(launches.len(), 2, "a different voice server should relaunch");
        assert_eq!(launches[1].host, "other.example.com");
    }

    #[tokio::test]
    async fn a_failed_launch_is_retried_on_the_next_reconnect() {
        let tmp = tempfile::tempdir().unwrap();
        let matrix = MockMatrix::new().with_repeating_connect_result(ConnectOutcome::Connected(None));
        let voice = MockVoice::new()
            .with_internal_events(vec![
                InternalEvent::Mumble(InternalMumbleEvent::Connected),
            ])
            .with_failing_launch(2);

        let mut second = connect_form();
        second.mumble_host = Some("other.example.com".into());

        let (_events, _, voice_state) = run_commands(
            matrix, voice, tmp.path(),
            vec![
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
                CoreCommand::System(SystemCommand::ConnectToServer(second.clone())),
                CoreCommand::System(SystemCommand::ConnectToServer(second)),
            ],
        ).await;

        // A failed launch records nothing, so a recovered one is the second recorded launch.
        let launches = voice_state.launched_with.lock().unwrap();
        assert_eq!(
            launches.len(), 2,
            "a reconnect after a failed launch should try again, got {:?}",
            launches,
        );
        assert_eq!(launches[1].host, "other.example.com");
    }

    /// Matrix commands queue behind the connect in the Matrix actor, so the burst is
    /// larger than that actor's queue as well as the engine's.
    #[tokio::test]
    async fn a_burst_of_commands_during_a_connect_neither_stalls_the_engine_nor_is_lost() {
        const CONTROL_CAPACITY: usize = 32;
        const BURST: usize = CONTROL_CAPACITY * 4;

        let tmp = tempfile::tempdir().unwrap();
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel();
        let matrix = MockMatrix::new().with_connect_gate(gate_rx);
        let matrix_state = matrix.state.clone();
        let voice = MockVoice::new();
        let voice_state = voice.state.clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(CONTROL_CAPACITY);
        let (_media_tx, media_rx) = mpsc::channel(32);
        let (event_tx, mut event_rx) = mpsc::channel(1000);
        let engine = CoreEngine::new(
            cmd_rx, media_rx, event_tx, matrix, voice,
            tmp.path().to_path_buf(), settings::load(tmp.path()),
        );
        let engine_handle = tokio::spawn(async move { engine.run().await });

        cmd_tx.send(CoreCommand::System(SystemCommand::ConnectToServer(connect_form())))
            .await.unwrap();
        wait_for_connecting(&mut event_rx).await;

        let issued = timeout(Duration::from_secs(5), async {
            for i in 0..BURST {
                cmd_tx.send(CoreCommand::Matrix(MatrixCommand::SendReadReceipt {
                    room_id: "!room:example.com".into(),
                    event_id: format!("${i}"),
                })).await.unwrap();
                cmd_tx.send(CoreCommand::Mumble(MumbleCommand::SetVadThreshold(i as f64 / 1000.0)))
                    .await.unwrap();
            }
            cmd_tx.send(CoreCommand::System(SystemCommand::LoadSettings)).await.unwrap();
        }).await;
        assert!(
            issued.is_ok(),
            "the engine stopped taking commands while a connect was in flight",
        );

        let served = timeout(Duration::from_secs(5), async {
            while let Some(event) = event_rx.recv().await {
                if matches!(event, CoreEvent::System(SystemEvent::SettingsLoaded(_))) {
                    return;
                }
            }
        }).await;
        assert!(served.is_ok(), "a command behind the burst was not answered during the connect");
        assert_eq!(voice_state.commands.lock().unwrap().len(), BURST);

        let _ = gate_tx.send(());
        drop(cmd_tx);
        timeout(Duration::from_secs(5), engine_handle)
            .await
            .expect("engine did not shut down within 5s")
            .expect("engine task panicked");

        let receipts: Vec<String> = matrix_state.commands.lock().unwrap().iter()
            .filter_map(|c| match c {
                MatrixCommand::SendReadReceipt { event_id, .. } => Some(event_id.clone()),
                _ => None,
            })
            .collect();
        let expected: Vec<String> = (0..BURST).map(|i| format!("${i}")).collect();
        assert_eq!(receipts, expected, "commands held during the connect must all be served, in order");
    }

    /// The first attempt is never released, so reaching Connected means the supersede stopped it.
    #[tokio::test]
    async fn a_second_connect_request_supersedes_the_one_in_flight() {
        use crate::test_mocks::MockCall;

        let tmp = tempfile::tempdir().unwrap();
        let (_gate_tx, gate_rx) = tokio::sync::oneshot::channel();
        let matrix = MockMatrix::new()
            .with_repeating_connect_result(ConnectOutcome::Connected(None))
            .with_connect_gate(gate_rx);
        let matrix_state = matrix.state.clone();
        let voice = MockVoice::new();
        let voice_state = voice.state.clone();

        let (engine, cmd_tx, event_rx) = build_engine(matrix, voice, tmp.path());
        let driver = EngineDriver::start(engine, cmd_tx, event_rx);

        driver.send(CoreCommand::System(SystemCommand::ConnectToServer(connect_form()))).await;

        timeout(Duration::from_secs(2), async {
            loop {
                if matrix_state.call_log.lock().unwrap().contains(&MockCall::Connect) {
                    return;
                }
                tokio::task::yield_now().await;
            }
        }).await.expect("the first attempt never reached connect()");

        driver.step(CoreCommand::System(SystemCommand::ConnectToServer(connect_form()))).await;
        let events = driver.finish().await;

        assert_eq!(
            matrix_state.call_log.lock().unwrap().as_slice(),
            &[MockCall::Reset, MockCall::Connect, MockCall::Reset, MockCall::Connect],
            "both attempts should have started, each with its own reset",
        );

        let connected = events.iter().filter(|e| matches!(e,
            CoreEvent::Matrix(MatrixEvent::ConnectionState(ConnectionState::Connected))
        )).count();
        assert_eq!(
            connected, 1,
            "the second attempt should have connected, and only it, since the first \
             never returned, so reaching Connected at all means it was stopped",
        );

        assert_eq!(
            voice_state.launched_with.lock().unwrap().len(), 1,
            "only the attempt that completed should have launched voice",
        );
    }

    #[tokio::test]
    async fn a_connect_that_cannot_be_dispatched_fails_and_re_arms_the_retry() {
        let tmp = tempfile::tempdir().unwrap();
        let (engine, cmd_tx, event_rx) = build_engine(MockMatrix::new(), MockVoice::new(), tmp.path());
        engine.matrix.kill_for_test().await;
        let driver = EngineDriver::start(engine, cmd_tx, event_rx);

        // Settling is half the assertion: barriers answer only once nothing dispatched
        // is outstanding.
        driver.step(CoreCommand::System(SystemCommand::ConnectToServer(connect_form()))).await;
        let events = driver.finish().await;

        let states: Vec<_> = events.iter().filter_map(|e| match e {
            CoreEvent::Matrix(MatrixEvent::ConnectionState(s)) => Some(s),
            _ => None,
        }).collect();
        assert!(
            matches!(states.last(), Some(ConnectionState::Failed { retries: 1, .. })),
            "a connect that could not be dispatched must reach Failed with the \
             retry armed, got {:?}",
            states,
        );
    }

    #[tokio::test]
    async fn a_voice_launch_that_cannot_be_dispatched_does_not_stay_outstanding() {
        let tmp = tempfile::tempdir().unwrap();
        let voice = MockVoice::new();
        let voice_state = voice.state.clone();
        let (engine, cmd_tx, event_rx) = build_engine(MockMatrix::new(), voice, tmp.path());
        engine.voice_service.kill_for_test().await;
        let driver = EngineDriver::start(engine, cmd_tx, event_rx);

        let mut form = connect_form();
        form.mumble_host = Some("voice.example.com".into());
        driver.step(CoreCommand::System(SystemCommand::ConnectToServer(form))).await;
        let events = driver.finish().await;

        assert!(
            voice_state.launched_with.lock().unwrap().is_empty(),
            "the actor was gone, so nothing can have been launched",
        );
        assert!(
            events.iter().any(|e| matches!(e,
                CoreEvent::Matrix(MatrixEvent::ConnectionState(ConnectionState::Connected))
            )),
            "a failed voice dispatch must not stop the Matrix connect from landing",
        );
    }

    /// Every route to a launch passes through `launch_voice`, and it must return before
    /// `pending_launch` is set or shutdown waits out the grace.
    #[tokio::test]
    async fn a_launch_asked_for_during_shutdown_is_neither_dispatched_nor_recorded() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut engine, _cmd_tx, _event_rx) =
            build_engine(MockMatrix::new(), MockVoice::new(), tmp.path());

        engine.shutting_down = true;
        engine.launch_voice(
            VoiceServerConfig {
                host: "voice.example.com".into(),
                port: 64738,
                username: Some("alice".into()),
                password: None,
            },
            false,
            "",
        ).await;

        assert!(
            engine.pending_launch.is_none(),
            "a suppressed launch must not be left outstanding: shutdown drains \
             until nothing is, so this would cost the full grace period",
        );
        assert_eq!(
            engine.launch_generation, 0,
            "nothing was dispatched, so no launch generation should have been spent",
        );
    }

    /// Starts from a live session #1, which the connect (#2) replaces.
    async fn replaced_session_report_during_a_connect(
        outcome: ConnectOutcome,
        report: InternalMatrixEvent,
    ) -> Vec<CoreEvent> {
        let tmp = tempfile::tempdir().unwrap();
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel();
        let matrix = MockMatrix::new()
            .with_connect_result(outcome)
            .with_connect_gate(gate_rx);
        let (mut engine, cmd_tx, mut event_rx) =
            build_engine(matrix, MockVoice::new(), tmp.path());
        engine.connect_generation = 1;
        engine.live_generation = Some(1);
        engine.conn.state = ConnectionState::Connected;
        let internal_tx = engine.internal_tx.clone();
        let engine_handle = tokio::spawn(async move { engine.run().await });

        cmd_tx.send(CoreCommand::System(SystemCommand::ConnectToServer(connect_form())))
            .await.unwrap();
        wait_for_connecting(&mut event_rx).await;

        // Sent on the channel the connect reports on, so it is handled before the result.
        internal_tx.send(InternalEvent::Matrix(report)).await.unwrap();
        let _ = gate_tx.send(());

        drop(cmd_tx);
        timeout(Duration::from_secs(5), engine_handle)
            .await
            .expect("engine did not shut down within 5s")
            .expect("engine task panicked");
        let mut events = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    #[tokio::test]
    async fn a_sync_report_from_the_replaced_session_cannot_mark_a_connect_in_flight_connected() {
        let events = replaced_session_report_during_a_connect(
            ConnectOutcome::Failed,
            InternalMatrixEvent::SyncRecovered { generation: 1 },
        ).await;

        let states = conn_states(&events);
        assert!(
            matches!(states.as_slice(), [ConnectionState::Failed { .. }]),
            "only the connect's own outcome may settle it, got Connecting then {states:?}",
        );
    }

    #[tokio::test]
    async fn the_replaced_session_ending_cannot_fail_a_connect_in_flight() {
        let events = replaced_session_report_during_a_connect(
            ConnectOutcome::Failed,
            InternalMatrixEvent::Disconnected {
                generation: 1,
                end: SyncEnd::RetriesExhausted { reason: "the old session's sync gave up".into() },
            },
        ).await;

        let states = conn_states(&events);
        assert!(
            matches!(
                states.as_slice(),
                [ConnectionState::Failed { reason, retries: 1, retry_in_secs: 2 }]
                    if reason == "Connection failed",
            ),
            "only the connect's own failure may count toward the backoff, got Connecting \
             then {states:?}",
        );
    }

    async fn own_report_before_the_connect_answers(
        data_dir: &std::path::Path,
        reports: Vec<fn(u64) -> InternalMatrixEvent>,
        linger: Duration,
    ) -> (Vec<CoreEvent>, Arc<MockMatrixState>) {
        let matrix = MockMatrix::new()
            .with_repeating_connect_result(ConnectOutcome::Connected(None))
            .with_reports_during_connect(reports);
        let matrix_state = matrix.state.clone();
        let (engine, cmd_tx, event_rx) = build_engine(matrix, MockVoice::new(), data_dir);
        let driver = EngineDriver::start(engine, cmd_tx, event_rx);

        driver.step(CoreCommand::System(SystemCommand::ConnectToServer(connect_form()))).await;
        tokio::time::sleep(linger).await;
        driver.settle().await;

        (driver.finish().await, matrix_state)
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_that_ends_before_its_connect_answers_is_retried() {
        let tmp = tempfile::tempdir().unwrap();
        let (events, matrix_state) = own_report_before_the_connect_answers(
            tmp.path(),
            vec![|generation| InternalMatrixEvent::Disconnected {
                generation,
                end: SyncEnd::RetriesExhausted { reason: "the new session's sync gave up".into() },
            }],
            PAST_THE_FIRST_BACKOFF,
        ).await;

        let states = conn_states(&events);
        assert!(
            matches!(
                states.as_slice(),
                [ConnectionState::Connecting, ConnectionState::Connected,
                 ConnectionState::Failed { reason, retries: 1, .. }, ConnectionState::Connecting,
                 ConnectionState::Connected]
                    if reason == "the new session's sync gave up",
            ),
            "a dead sync loop must not be left looking connected, got {states:?}",
        );

        use crate::test_mocks::MockCall;
        assert_eq!(
            matrix_state.call_log.lock().unwrap().iter()
                .filter(|call| **call == MockCall::Connect).count(),
            2,
            "the ended session should have been reconnected by the retry",
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_held_session_end_is_not_displaced_by_a_later_health_report() {
        let tmp = tempfile::tempdir().unwrap();
        let (events, _) = own_report_before_the_connect_answers(
            tmp.path(),
            vec![
                |generation| InternalMatrixEvent::Disconnected {
                    generation,
                    end: SyncEnd::RetriesExhausted { reason: "the new session's sync gave up".into() },
                },
                |generation| InternalMatrixEvent::SyncRecovered { generation },
            ],
            Duration::ZERO,
        ).await;

        let states = conn_states(&events);
        assert!(
            matches!(
                states.as_slice(),
                [ConnectionState::Connecting, ConnectionState::Connected,
                 ConnectionState::Failed { reason, retries: 1, .. }]
                    if reason == "the new session's sync gave up",
            ),
            "the session ended, so it must settle Failed with a retry, got {states:?}",
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_degradation_reported_before_its_connect_answers_is_applied_once_it_lands() {
        let tmp = tempfile::tempdir().unwrap();
        let (events, _) = own_report_before_the_connect_answers(
            tmp.path(),
            vec![|generation| InternalMatrixEvent::SyncDegraded {
                generation,
                reason: "Sync error: error sending request".into(),
            }],
            Duration::ZERO,
        ).await;

        let states = conn_states(&events);
        assert!(
            matches!(
                states.as_slice(),
                [ConnectionState::Connecting, ConnectionState::Connected,
                 ConnectionState::Connecting],
            ),
            "the new session is degraded and the UI should say so, got {states:?}",
        );
    }

    #[tokio::test]
    async fn a_result_from_a_superseded_connect_is_not_taken_for_the_current_one() {
        let tmp = tempfile::tempdir().unwrap();
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel();
        let matrix = MockMatrix::new()
            .with_connect_result(ConnectOutcome::Failed)
            .with_connect_gate(gate_rx);
        let voice = MockVoice::new();
        let voice_state = voice.state.clone();
        let (engine, cmd_tx, mut event_rx) = build_engine(matrix, voice, tmp.path());
        let internal_tx = engine.internal_tx.clone();
        let engine_handle = tokio::spawn(async move { engine.run().await });

        cmd_tx.send(CoreCommand::System(SystemCommand::ConnectToServer(connect_form())))
            .await.unwrap();
        wait_for_connecting(&mut event_rx).await;

        internal_tx.send(InternalEvent::Matrix(InternalMatrixEvent::ConnectFinished {
            generation: 0,
            outcome: ConnectOutcome::Connected(None),
        })).await.unwrap();
        let _ = gate_tx.send(());

        drop(cmd_tx);
        timeout(Duration::from_secs(5), engine_handle)
            .await
            .expect("engine did not shut down within 5s")
            .expect("engine task panicked");
        let mut events = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            events.push(event);
        }

        assert!(
            matches!(conn_states(&events).as_slice(), [ConnectionState::Failed { .. }]),
            "the stale success must be discarded and the real failure kept, got {:?}",
            conn_states(&events),
        );
        assert!(
            voice_state.launched_with.lock().unwrap().is_empty(),
            "a stale success must not start voice",
        );
    }

    #[tokio::test]
    async fn a_result_from_a_superseded_launch_leaves_the_current_one_outstanding() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut engine, _cmd_tx, _event_rx) =
            build_engine(MockMatrix::new(), MockVoice::new(), tmp.path());
        let creds = |host: &str| VoiceServerConfig {
            host: host.into(),
            port: 64738,
            username: Some("alice".into()),
            password: None,
        };

        engine.launch_voice(creds("first.example.com"), false, "").await;
        engine.launch_voice(creds("second.example.com"), false, "").await;
        engine.finish_launch(1, LaunchOutcome::Failed).await;

        assert!(
            matches!(&engine.pending_launch, Some(p) if p.generation == 2),
            "the newer launch must still be awaited",
        );
        assert_eq!(
            engine.voice.state_name(), "Idle",
            "the superseded launch's failure must not be recorded against the session",
        );
    }

    #[tokio::test]
    async fn asking_to_connect_again_restarts_the_backoff() {
        let tmp = tempfile::tempdir().unwrap();
        let (events, _, _) = run_commands(
            MockMatrix::new().with_repeating_connect_result(ConnectOutcome::Failed),
            MockVoice::new(),
            tmp.path(),
            vec![
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
                CoreCommand::System(SystemCommand::ConnectToServer(connect_form())),
            ],
        ).await;

        let failures: Vec<_> = conn_states(&events).into_iter()
            .filter_map(|s| match s {
                ConnectionState::Failed { retries, retry_in_secs, .. } => Some((*retries, *retry_in_secs)),
                _ => None,
            })
            .collect();
        assert_eq!(
            failures, vec![(1, 2), (1, 2)],
            "a connect the user asked for must not inherit the previous backoff",
        );
    }

    // --- Certificate check tests ---

    /// A voice server address and the fingerprint `voice_presenting` makes it present.
    fn fake_cert(port: u16) -> (u16, String) {
        (port, format!("fingerprint-of-{port}"))
    }

    fn voice_presenting(certs: &[(u16, &str)]) -> MockVoice {
        certs.iter().fold(MockVoice::new(), |voice, (port, fingerprint)| {
            voice.presenting_cert("127.0.0.1", *port, fingerprint)
        })
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
        let (port, real_fp) = fake_cert(64001);
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
            voice_presenting(&[(port, &real_fp)]),
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
        let (port, real_fp) = fake_cert(64001);
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
            voice_presenting(&[(port, &real_fp)]),
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
        let (port, real_fp) = fake_cert(64001);
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
            voice_presenting(&[(port, &real_fp)]),
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
        let (port, real_fp) = fake_cert(64001);
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
            voice_presenting(&[(port, &real_fp)]),
            tmp.path(),
            vec![CoreCommand::System(SystemCommand::ConnectToServer(form))],
        ).await;

        // No CertificateChanged
        assert!(!events.iter().any(|e| matches!(e, CoreEvent::Mumble(MumbleEvent::CertificateChanged { .. }))));

        // Voice launched normally
        let launches = voice_state.launched_with.lock().unwrap();
        assert_eq!(launches.len(), 1);
    }

    #[tokio::test]
    async fn a_pending_cert_prompt_does_not_count_as_a_live_session() {
        let (first_port, first_fp) = fake_cert(64001);
        let (second_port, second_fp) = fake_cert(64002);
        let tmp = tempfile::tempdir().unwrap();
        seed_cert_db(tmp.path(), "127.0.0.1", first_port, &first_fp);
        seed_cert_db(tmp.path(), "127.0.0.1", second_port, "wrong_fingerprint");

        let voice_form = |port: u16| ServerConnectionForm {
            username: "alice".into(),
            hostname: "matrix.example.com".into(),
            port: "8448".into(),
            password: None,
            mumble_host: Some("127.0.0.1".into()),
            mumble_port: Some(port),
            mumble_username: None,
            mumble_password: None,
            homeserver_url: None,
        };

        let matrix = MockMatrix::new().with_repeating_connect_result(ConnectOutcome::Connected(None));
        let voice = voice_presenting(&[(first_port, &first_fp), (second_port, &second_fp)]).with_internal_events(vec![
            InternalEvent::Mumble(InternalMumbleEvent::Connected),
        ]);

        let (events, _, voice_state) = run_commands(
            matrix, voice, tmp.path(),
            vec![
                CoreCommand::System(SystemCommand::ConnectToServer(voice_form(first_port))),
                CoreCommand::System(SystemCommand::ConnectToServer(voice_form(second_port))),
                CoreCommand::System(SystemCommand::ConnectToServer(voice_form(second_port))),
            ],
        ).await;

        let prompts = events.iter().filter(|e| matches!(e,
            CoreEvent::Mumble(MumbleEvent::CertificateChanged { .. })
        )).count();
        assert_eq!(
            prompts, 2,
            "the reconnect should re-attempt the blocked launch and prompt again",
        );

        let launches = voice_state.launched_with.lock().unwrap();
        assert_eq!(
            launches.len(), 1,
            "voice must not launch onto a server whose cert was never accepted, got {:?}",
            launches,
        );
        assert_eq!(launches[0].port, first_port, "the only launch is the first server's");
    }

    #[tokio::test]
    async fn a_voice_connect_while_awaiting_a_cert_does_not_mark_the_session_live() {
        let (first_port, first_fp) = fake_cert(64001);
        let (second_port, second_fp) = fake_cert(64002);
        let tmp = tempfile::tempdir().unwrap();
        seed_cert_db(tmp.path(), "127.0.0.1", first_port, &first_fp);
        seed_cert_db(tmp.path(), "127.0.0.1", second_port, "wrong_fingerprint");

        let voice_form = |port: u16| ServerConnectionForm {
            username: "alice".into(),
            hostname: "matrix.example.com".into(),
            port: "8448".into(),
            password: None,
            mumble_host: Some("127.0.0.1".into()),
            mumble_port: Some(port),
            mumble_username: None,
            mumble_password: None,
            homeserver_url: None,
        };

        // Connected events are injected, so it is clear which one the engine reacts to.
        let voice = voice_presenting(&[(first_port, &first_fp), (second_port, &second_fp)]);
        let voice_state = voice.state.clone();
        let matrix = MockMatrix::new().with_repeating_connect_result(ConnectOutcome::Connected(None));
        let (engine, cmd_tx, event_rx) = build_engine(matrix, voice, tmp.path());
        let driver = EngineDriver::start(engine, cmd_tx, event_rx);

        driver.step(CoreCommand::System(SystemCommand::ConnectToServer(voice_form(first_port)))).await;
        driver.inject(InternalEvent::Mumble(InternalMumbleEvent::Connected)).await;
        driver.settle().await;

        driver.step(CoreCommand::System(SystemCommand::ConnectToServer(voice_form(second_port)))).await;

        driver.inject(InternalEvent::Mumble(InternalMumbleEvent::Connected)).await;
        driver.settle().await;

        driver.step(CoreCommand::System(SystemCommand::ConnectToServer(voice_form(second_port)))).await;

        let events = driver.finish().await;

        let prompts = events.iter().filter(|e| matches!(e,
            CoreEvent::Mumble(MumbleEvent::CertificateChanged { .. })
        )).count();
        assert_eq!(
            prompts, 2,
            "a Connected with no launch outstanding must not pass for the blocked one",
        );

        let launches = voice_state.launched_with.lock().unwrap();
        assert_eq!(
            launches.len(), 1,
            "voice must not launch onto a server whose cert was never accepted, got {:?}",
            launches,
        );
        assert_eq!(launches[0].port, first_port, "the only launch is the first server's");
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
        let (mut engine, cmd_tx, _event_rx) = build_engine(MockMatrix::new(), voice, tmp.path());
        // Close the channel now; the engine is only run below to drain the dispatched launch.
        drop(cmd_tx);

        // Simulate state saved from a previous session
        engine.voice_restore = VoiceRestoreState {
            channel_path: Some("Voice/General".into()),
            muted: true,
            deafened: true,
        };

        engine.resolve_and_launch_voice(&connect_form(), None, false, "").await;
        assert_eq!(engine.voice_restore, VoiceRestoreState::default());

        timeout(Duration::from_secs(5), tokio::spawn(async move { engine.run().await }))
            .await
            .expect("engine did not shut down within 5s")
            .expect("engine task panicked");

        // Voice launched with no channel path
        let paths = voice_state.launched_channel_paths.lock().unwrap();
        assert_eq!(paths.len(), 1);
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
