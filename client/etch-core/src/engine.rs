use tokio::sync::mpsc;
use tokio::time::{sleep, Duration, Instant, Sleep};
use crate::actor::{LaunchRequest, MatrixHandle, MatrixRequest, VoiceHandle, VoiceRequest};
use crate::connection::MatrixConnection;
use crate::events::{CoreEvent, InternalEvent, InternalMatrixEvent, InternalMumbleEvent, InternalSystemEvent, LaunchOutcome, MumbleEvent, SyncEnd, SystemEvent};
use crate::commands::{CoreCommand, MediaRequest, MumbleCommand, ServerConnectionForm, SystemCommand};
use crate::models::{ConnectOutcome, ConnectionState, VoiceServerConfig};
use crate::settings::{Settings, SettingsStore};
use crate::traits::{MatrixBackend, VoiceService};

use std::path::PathBuf;
use std::pin::Pin;

/// Depth of the engine's own internal event channel.
const INTERNAL_QUEUE: usize = 100;

/// How long a shutdown waits for work already dispatched.
///
/// Closing the command channel means "no more work is coming", not "drop what
/// you are holding". A connect still running in the Matrix actor reports back
/// here, and its voice launch is dispatched off that reply, so returning the
/// moment the channel closes would cut the sequence in half. The bound is what
/// keeps quit prompt regardless: in the ordinary case nothing is outstanding
/// and shutdown is immediate, and a connect caught mid-flight cannot hold the
/// app open longer than this.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Voice state tracked in memory for restoration after Mumble restarts.
/// Reset when connecting to a new server; preserved across process restarts
/// on the same server.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct VoiceSessionState {
    pub channel_path: Option<String>,
    pub muted: bool,
    pub deafened: bool,
}

/// Where the voice session actually is. One value so "what we asked for" and
/// "what is up" cannot disagree.
#[derive(Debug)]
pub(crate) enum VoiceSession {
    /// No voice server known yet.
    Idle,
    /// Creds known, Mumble not up. Covers never-launched, launch-failed, and
    /// the session having dropped.
    Down { creds: VoiceServerConfig },
    /// Launch issued, waiting for Mumble to report Connected.
    Launching { creds: VoiceServerConfig },
    /// Waiting on the user to accept a changed server certificate.
    AwaitingCert {
        creds: VoiceServerConfig,
        show_gui: bool,
        extra_args: String,
    },
    /// Mumble is joined to `creds`.
    Up { creds: VoiceServerConfig },
}

impl VoiceSession {
    /// The voice server this session is about, for every arm that knows one.
    pub(crate) fn creds(&self) -> Option<&VoiceServerConfig> {
        match self {
            VoiceSession::Idle => None,
            VoiceSession::Down { creds }
            | VoiceSession::Launching { creds }
            | VoiceSession::AwaitingCert { creds, .. }
            | VoiceSession::Up { creds } => Some(creds),
        }
    }

    /// The arm's name, for log lines. Spelled out rather than `Debug`-printed
    /// because the credentials carry a password.
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

/// A connect the engine has dispatched and not yet had an answer for.
///
/// Its presence is the "one at a time" gate. The form is kept because the
/// voice launch that follows a successful connect is resolved from it, and
/// the answer arrives long after the command that carried it is gone.
struct PendingConnect {
    generation: u64,
    form: ServerConnectionForm,
    /// Fires to stop the attempt. Sent when a newer connect supersedes this
    /// one, so the old attempt is dropped where it stands rather than left to
    /// run to completion against a session that has been replaced.
    cancel: tokio::sync::oneshot::Sender<()>,
    started: Instant,
    /// Deepest the control channel got while this attempt was in flight.
    ///
    /// The number this whole change is about. While the connect was awaited
    /// on the loop, nothing drained `cmd_rx` for its duration, so whatever
    /// the user did during it piled up here and, once the channel filled,
    /// blocked their next `invoke` outright. With the connect in an actor the
    /// loop keeps draining, so this should read zero.
    peak_control_depth: usize,
}

/// A voice launch the engine has dispatched and not yet had an answer for.
struct PendingLaunch {
    generation: u64,
    creds: VoiceServerConfig,
    show_gui: bool,
    extra_args: String,
}

/// Coordinates the subsystems; owns none of them.
///
/// Each service lives in a task of its own (see `crate::actor`), so the engine
/// holds channels rather than the services themselves. That is what lets this
/// loop keep serving the UI while a connect -- several network round trips and
/// a TLS probe -- runs to completion elsewhere. What the engine does own is
/// the state that says where each subsystem stands, and the sequencing between
/// them: a Matrix connect landing is what dispatches the voice launch.
pub struct CoreEngine {
    pub(crate) cmd_rx: mpsc::Receiver<CoreCommand>,
    /// Media fetches, on their own channel with its own capacity. Handing one
    /// over never waits, so this arm cannot hold up the loop; what the split
    /// buys is that a burst of image loads cannot eat the control channel's
    /// slots.
    pub(crate) media_rx: mpsc::Receiver<MediaRequest>,
    pub(crate) event_tx: mpsc::Sender<CoreEvent>,

    /// Results from the actors, and reports from the processes they own.
    /// Owned as a field rather than made in `run` so that everything the
    /// engine dispatches has somewhere to answer from the moment it exists.
    internal_tx: mpsc::Sender<InternalEvent>,
    internal_rx: mpsc::Receiver<InternalEvent>,

    matrix: MatrixHandle,
    voice_service: VoiceHandle,
    conn: MatrixConnection,
    pub(crate) data_dir: PathBuf,
    /// The settings, owned here. Reads are from memory; writes go to memory
    /// and are persisted off this loop.
    pub(crate) settings: SettingsStore,
    /// Voice state persisted across Mumble client restarts.
    pub(crate) voice_session: VoiceSessionState,
    /// Where the one voice session stands right now.
    pub(crate) voice: VoiceSession,

    pending_connect: Option<PendingConnect>,
    pending_launch: Option<PendingLaunch>,
    connect_generation: u64,
    launch_generation: u64,

    /// Barrier requests waiting on the engine to settle. See
    /// `InternalSystemEvent::Barrier`.
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
            settings: SettingsStore::from_loaded(data_dir.clone(), settings),
            data_dir,
            voice_session: VoiceSessionState::default(),
            voice: VoiceSession::Idle,
            pending_connect: None,
            pending_launch: None,
            connect_generation: 0,
            launch_generation: 0,
            #[cfg(test)]
            pending_barriers: Vec::new(),
        }
    }

    pub async fn run(mut self) {
        let mut retry_timer: Pin<Box<Sleep>> = Box::pin(sleep(Duration::MAX));

        loop {
            if let Some(pending) = &mut self.pending_connect {
                pending.peak_control_depth = pending.peak_control_depth.max(self.cmd_rx.len());
            }

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

                // --- Media data path ---
                //
                // Separate from the control channel above so that loading a
                // timeline's worth of images cannot fill the queue the UI's
                // commands arrive on. Handing a fetch to the Matrix actor never
                // waits, so this arm never parks the loop.
                Some(request) = self.media_rx.recv() => {
                    self.matrix.fetch_media(request.mxc_url, request.respond);
                }

                Some(internal_event) = self.internal_rx.recv() => {
                    self.handle_internal_event(internal_event, &mut retry_timer).await;
                    self.answer_barriers();
                }

                // --- Retry timer ---
                //
                // Gated on there being no attempt in flight. Without that, a
                // connect slower than the backoff would keep superseding
                // itself and never finish.
                _ = &mut retry_timer, if self.conn.state.is_failed() && self.pending_connect.is_none() => {
                    if let Some(form) = self.conn.form.clone() {
                        log::info!("Retrying Matrix connection (attempt {})", self.conn.retries + 1);
                        self.connect_to_server(&form, &mut retry_timer).await;
                    }
                }
            }
        }

        self.shut_down(&mut retry_timer).await;
    }

    /// Let go of the subsystems and flush what is only in memory.
    ///
    /// Work already dispatched is given until `SHUTDOWN_GRACE` to report back,
    /// because a connect's voice launch is dispatched off the connect's own
    /// reply and cutting the loop here would leave that half undone. Then each
    /// actor is told no more requests are coming and given the rest of the
    /// grace to run out the ones it has -- the Matrix commands the user issued
    /// last are sitting in that queue.
    async fn shut_down(mut self, retry_timer: &mut Pin<Box<Sleep>>) {
        let deadline = Instant::now() + SHUTDOWN_GRACE;

        while !self.settled() {
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

        // Anything the actors emitted on their way out. Without this, events
        // queued during that last stretch of work would be dropped.
        while let Ok(event) = self.internal_rx.try_recv() {
            self.handle_internal_event(event, retry_timer).await;
        }

        // Settings are written off this loop, so the last change may still be
        // in the coalescing window. This waits for it: nothing the user set is
        // lost by closing the app.
        self.settings.shutdown().await;
    }

    /// Nothing the engine dispatched is still outstanding.
    fn settled(&self) -> bool {
        self.pending_connect.is_none() && self.pending_launch.is_none()
    }

    #[cfg(test)]
    fn answer_barriers(&mut self) {
        // A barrier means "everything I sent before this is done", so a
        // command still queued ahead of it counts as outstanding too.
        if !self.settled() || !self.cmd_rx.is_empty() {
            return;
        }
        for reply in self.pending_barriers.drain(..) {
            let _ = reply.send(());
        }
    }

    #[cfg(not(test))]
    fn answer_barriers(&self) {}

    /// Kill a subsystem's actor the way a panic in it would. See
    /// `MatrixHandle::kill_for_test`.
    #[cfg(test)]
    pub(crate) async fn kill_matrix_actor(&self) {
        self.matrix.kill_for_test().await;
    }

    #[cfg(test)]
    pub(crate) async fn kill_voice_actor(&self) {
        self.voice_service.kill_for_test().await;
    }

    /// A sender for the channel the actors report on. Lets a test stand in for
    /// a subsystem and deliver an event the way the real one would.
    #[cfg(test)]
    pub(crate) fn internal_sender(&self) -> mpsc::Sender<InternalEvent> {
        self.internal_tx.clone()
    }

    async fn handle_system_command(
        &mut self,
        cmd: SystemCommand,
        retry_timer: &mut Pin<Box<Sleep>>,
    ) {
        match cmd {
            SystemCommand::ConnectToServer(form) => {
                self.conn.form = Some(form.clone());
                self.conn.retries = 0;
                self.connect_to_server(&form, retry_timer).await;
            }
            SystemCommand::LoadSettings => {
                let s = self.settings.get().clone();
                let _ = self.event_tx.send(CoreEvent::System(
                    SystemEvent::SettingsLoaded(s.clone()),
                )).await;

                if let Some(bm) = s.bookmarks.iter().find(|b| b.auto_connect) {
                    let form = ServerConnectionForm::from(bm);
                    self.conn.form = Some(form.clone());
                    self.connect_to_server(&form, retry_timer).await;
                }
            }
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
                let db_path = self.data_dir.join("mumble/mumble.sqlite");
                if let Err(e) = crate::mumble::cert::store_cert(&db_path, &host, port, &fingerprint) {
                    log::error!("Failed to store accepted cert: {:?}", e);
                    return;
                }
                log::info!("User accepted new cert for {}:{}", host, port);
                // Resume the stashed voice launch. Anything else stays as it
                // was: there is nothing to resume.
                match std::mem::replace(&mut self.voice, VoiceSession::Idle) {
                    VoiceSession::AwaitingCert { creds, show_gui, extra_args } => {
                        // The prompt is answered, so the session is no longer
                        // waiting on it -- but Mumble is still not up until
                        // the launch says so.
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
                    // A sync failure the loop is riding out. Nothing has been
                    // torn down and nothing is being reconnected, so no
                    // `ServerReset` is emitted and no connect is dispatched:
                    // the UI keeps every timeline it is showing and is only
                    // told the connection is working on something.
                    InternalMatrixEvent::SyncDegraded { reason } => {
                        log::warn!(
                            "Matrix sync degraded ({reason}); retrying in place, session untouched",
                        );
                        if self.sync_health_is_current() {
                            self.conn.degraded(&self.event_tx).await;
                        }
                    }
                    InternalMatrixEvent::SyncRecovered => {
                        log::info!("Matrix sync recovered without a reconnect");
                        if self.sync_health_is_current() {
                            self.conn.recovered(&self.event_tx).await;
                        }
                    }
                    // The loop is over. `SyncEnd` is what tells the two kinds
                    // of over apart -- a session the server disowned, and a
                    // session that simply could not reach it -- which a bare
                    // string never could.
                    InternalMatrixEvent::Disconnected(end) => {
                        match &end {
                            SyncEnd::SessionInvalidated { reason } => log::error!(
                                "Matrix session invalidated by the server: {reason}",
                            ),
                            SyncEnd::RetriesExhausted { reason } => log::warn!(
                                "Matrix sync stopped after retrying: {reason}",
                            ),
                        }
                        // Both end at the same place for now -- the cold
                        // reconnect is the backstop for either -- but they no
                        // longer arrive here as the same event, which is what
                        // the reason the user reads, the log severity above,
                        // and any later resume path all need.
                        self.conn.schedule_retry(
                            retry_timer, end.reason().to_string(), &self.event_tx,
                        ).await;
                    }
                    InternalMatrixEvent::ConnectFinished { generation, outcome } => {
                        self.finish_connect(generation, outcome, retry_timer).await;
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
                        // Resolving the profile is a homeserver round trip, so
                        // it goes to the actor and comes back as
                        // `VoiceUserResolved` rather than being awaited here.
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
                        // The Mumble process is being replaced right now, so
                        // from here a Connected belongs to this launch.
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
                        // Only a launch we issued can complete. In any other
                        // arm this is some other Mumble -- most likely the one
                        // still joined to the previous server, reconnecting on
                        // its own -- and crediting it to the credentials we
                        // last asked for would record a live session on a
                        // server Mumble has never reached.
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
                        // The settings and mute/deafen restoration below are
                        // about the Mumble process that just came up, so they
                        // run either way.
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
                        if self.voice_session.muted {
                            self.send_voice_command(MumbleCommand::MuteSelf(true)).await;
                        }
                        if self.voice_session.deafened {
                            self.send_voice_command(MumbleCommand::DeafenSelf(true)).await;
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
                }
            }
            InternalEvent::System(evt) => self.handle_internal_system_event(evt),
        }
    }

    #[cfg(test)]
    fn handle_internal_system_event(&mut self, evt: InternalSystemEvent) {
        match evt {
            InternalSystemEvent::Barrier(reply) => self.pending_barriers.push(reply),
        }
    }

    /// `InternalSystemEvent` is uninhabited outside tests: nothing can
    /// construct one, so there is nothing to handle.
    #[cfg(not(test))]
    fn handle_internal_system_event(&mut self, evt: InternalSystemEvent) {
        match evt {}
    }

    async fn send_voice_command(&self, cmd: MumbleCommand) {
        let _ = self.voice_service.send(VoiceRequest::Command(cmd)).await;
    }

    async fn dispatch_mumble_command(&mut self, cmd: MumbleCommand) {
        // Record the setting regardless of whether Mumble is connected. These
        // arrive one per slider event; they update memory and the write is
        // coalesced elsewhere, so none of this touches the disk here.
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

    /// Start a connection attempt and return; the answer arrives as
    /// `InternalMatrixEvent::ConnectFinished`.
    ///
    /// Only one attempt runs at a time. A request arriving while one is in
    /// flight supersedes it rather than queueing behind it: the user asking
    /// again, or picking a different server, means the attempt underway is
    /// the wrong one, and finishing it first would connect them somewhere
    /// they have already left. The old attempt is cancelled where it stands.
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

        // Ordered deliberately: the frontend clears its session stores on
        // `ServerReset`, and everything the new session produces is emitted by
        // the actor, which is only told to start below. Both travel on
        // `event_tx`, so the reset cannot be overtaken by the data it exists
        // to make room for.
        let _ = self.event_tx.send(CoreEvent::System(SystemEvent::ServerReset)).await;
        self.conn.begin(&self.event_tx).await;

        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        self.pending_connect = Some(PendingConnect {
            generation,
            form: form.clone(),
            cancel: cancel_tx,
            started: Instant::now(),
            peak_control_depth: 0,
        });

        let dispatched = self.matrix.send(MatrixRequest::Connect {
            form: form.clone(),
            internal_tx: self.internal_tx.clone(),
            generation,
            cancel: cancel_rx,
        }).await;

        // A request the actor never took is an attempt that will never
        // answer, and `pending_connect` is what gates the retry timer. Take
        // it through the same path a returned `Failed` would, so the state
        // reaches the frontend and the retry re-arms instead of the app
        // going quietly unable to connect.
        if !dispatched {
            log::error!("Matrix actor is not accepting requests; failing connect #{generation}");
            self.finish_connect(generation, ConnectOutcome::Failed, retry_timer).await;
        }

        // Explicit Mumble config in the bookmark: the voice server is already
        // known, so the launch does not have to wait for Matrix to discover it.
        if form.mumble_host.is_some() {
            self.resolve_and_launch_voice(form, None, false, "").await;
        }
    }

    /// May a report from a sync loop about the health of its session move the
    /// connection state?
    ///
    /// Only when no connect is in flight. A connect owns the connection state
    /// for as long as it runs, and the report may well come from the sync task
    /// of the very session that connect is replacing -- the event can already
    /// be queued when the actor aborts the task. Letting a superseded session
    /// speak would put the connection state behind the truth at exactly the
    /// moment it matters most.
    fn sync_health_is_current(&self) -> bool {
        if self.pending_connect.is_some() {
            log::debug!("Ignoring a sync health report: a connect is already in flight");
            return false;
        }
        true
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
            // The attempt we are actually waiting on is still running.
            self.pending_connect = Some(pending);
            return;
        }

        log::info!(
            "Matrix connect #{generation} took {:?}; peak control queue depth during it: {}; \
             queue depth after it: control={}, media={}",
            pending.started.elapsed(),
            pending.peak_control_depth,
            self.cmd_rx.len(),
            self.media_rx.len(),
        );

        let voice_server = self.conn.settle(outcome, retry_timer, &self.event_tx).await;

        // A bookmark with an explicit Mumble host already launched at dispatch.
        if matches!(self.conn.state, ConnectionState::Connected) && pending.form.mumble_host.is_none() {
            self.resolve_and_launch_voice(&pending.form, voice_server, false, "").await;
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
            // `LaunchStarted` already moved the session to `Launching`, and a
            // `Connected` may have moved it on to `Up` since. Either is more
            // current than anything this could say.
            LaunchOutcome::Launched => {}
            LaunchOutcome::Failed => {
                self.voice = VoiceSession::Down { creds: pending.creds };
            }
            LaunchOutcome::CertChanged => {
                // Mumble was left alone, so it is not on this server and until
                // the user decides it will not be. A later reconnect has to
                // re-attempt rather than assume voice is fine.
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

        // A live Mumble session on the same server needs nothing done to it.
        // This is the common case on a Matrix reconnect: the voice server has
        // not moved, and relaunching would drop the user out of voice for
        // several seconds over an unrelated sync hiccup. Only `Up` is a live
        // session: every other arm means Mumble is not joined to `new_creds`,
        // whatever credentials we last asked for.
        if matches!(self.voice, VoiceSession::Up { ref creds } if creds == &new_creds) {
            log::debug!(
                "Voice already connected to {}:{}, keeping the session",
                new_creds.host, new_creds.port,
            );
            return;
        }

        // A different voice server means nothing about the old session carries
        // over: channel, mute and deafen are all specific to where we were.
        if self.voice.creds() != Some(&new_creds) {
            self.voice_session = VoiceSessionState::default();
        }

        self.launch_voice(new_creds, show_gui, extra_args).await;
    }

    /// Dispatch a voice launch and return; the answer arrives as
    /// `InternalMumbleEvent::LaunchFinished`.
    ///
    /// `self.voice` is deliberately left alone here. The launch begins with a
    /// TLS probe of the server's certificate, during which Mumble has not been
    /// touched and is still joined to wherever it was; claiming otherwise
    /// would let a `Connected` from that older process be credited to a server
    /// it has never reached. The actor says when the replacement actually
    /// starts.
    async fn launch_voice(&mut self, creds: VoiceServerConfig, show_gui: bool, extra_args: &str) {
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
            channel_path: self.voice_session.channel_path.clone(),
            internal_tx: self.internal_tx.clone(),
            generation,
        })).await;

        // As for a connect: an unserved request would leave `pending_launch`
        // outstanding forever. This is the same path the actor reporting a
        // failed launch takes, so the session lands on `Down` with its
        // credentials kept and the next reconnect tries again.
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

    /// Drives a running engine one command at a time.
    ///
    /// A connect no longer completes before `run()` takes the next command off
    /// the channel, so "send a burst and look at the result" no longer says
    /// what a test means by it: a command that depends on a connect having
    /// landed has to be sent after it lands, not merely after it starts.
    /// `step` is that -- it sends a command and waits for the engine to settle
    /// -- and it is exact rather than timed, because every piece of work the
    /// engine dispatches reports back to it.
    struct EngineDriver {
        cmd_tx: mpsc::Sender<CoreCommand>,
        internal_tx: mpsc::Sender<InternalEvent>,
        event_rx: mpsc::Receiver<CoreEvent>,
        engine: tokio::task::JoinHandle<()>,
    }

    impl EngineDriver {
        fn start(engine: CoreEngine, cmd_tx: mpsc::Sender<CoreCommand>, event_rx: mpsc::Receiver<CoreEvent>) -> Self {
            let internal_tx = engine.internal_sender();
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

        /// Deliver an internal event the way a subsystem would.
        async fn inject(&self, event: InternalEvent) {
            self.internal_tx.send(event).await.expect("engine already stopped");
        }

        /// Wait until the engine has drained every command sent so far and has
        /// nothing it dispatched still outstanding.
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

        /// Close the command channel, let the engine shut down, and collect
        /// everything it emitted.
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

    /// Send commands to the engine, letting each one settle before the next,
    /// then shut it down and collect emitted events.
    ///
    /// Settling between commands is what the loop used to do for free by
    /// running everything to completion inline. It is not a sleep: the engine
    /// answers the barrier once it has no dispatched work left.
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

    /// Write a settings.json for the engine to pick up when it is built.
    fn seed_settings(
        data_dir: &std::path::Path,
        change: impl FnOnce(&mut settings::Settings),
    ) {
        let mut s = settings::load(data_dir);
        change(&mut s);
        settings::save(data_dir, &s);
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

        // oneshot resolves once the engine services the request
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

    /// The media data path must not consume the control channel's capacity.
    ///
    /// One `etch-media` request is raised per image the webview loads, so
    /// opening a picture-heavy room bursts more requests than the control
    /// channel has slots. While they shared a channel, that burst filled it
    /// and the next UI command could not even be enqueued -- the frontend's
    /// `invoke` blocked until the engine drained it, which it cannot do while
    /// parked in a connect.
    #[tokio::test]
    async fn a_burst_of_media_requests_leaves_the_control_channel_usable() {
        const CONTROL_CAPACITY: usize = 32;
        const IMAGES: usize = CONTROL_CAPACITY * 4;

        let tmp = tempfile::tempdir().unwrap();
        // Hold the engine inside a connect: the loop is parked, so nothing is
        // drained from either channel for the duration.
        let (_gate_tx, gate_rx) = tokio::sync::oneshot::channel();
        let matrix = MockMatrix::new().with_connect_gate(gate_rx);

        let (cmd_tx, cmd_rx) = mpsc::channel(CONTROL_CAPACITY);
        let (media_tx, media_rx) = mpsc::channel(256);
        let (event_tx, mut event_rx) = mpsc::channel(100);
        let engine = CoreEngine::new(
            cmd_rx, media_rx, event_tx, matrix, MockVoice::new(),
            tmp.path().to_path_buf(), settings::load(tmp.path()),
        );
        let engine_handle = tokio::spawn(async move { engine.run().await });

        cmd_tx.send(CoreCommand::System(SystemCommand::ConnectToServer(connect_form())))
            .await.unwrap();
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

        // Keep the response ends alive so nothing is dropped early.
        let mut pending = Vec::new();
        for i in 0..IMAGES {
            let (tx, rx) = tokio::sync::oneshot::channel();
            pending.push(rx);
            media_tx.try_send(MediaRequest {
                mxc_url: format!("mxc://example.com/{i}"),
                respond: tx,
            }).unwrap_or_else(|_| panic!("media request {i} was refused"));
        }

        // The user clicks mute while those images are still outstanding.
        let control = cmd_tx.try_send(CoreCommand::System(SystemCommand::MuteMic(true)));

        engine_handle.abort();
        assert!(
            control.is_ok(),
            "{IMAGES} queued media requests blocked a UI command on a \
             {CONTROL_CAPACITY}-slot control channel",
        );
    }

    /// Dragging the VAD slider sends one command per event. Each used to cost
    /// a full read-modify-write of settings.json, synchronously, on this loop.
    /// They must now cost memory writes plus the writer's two coalesced file
    /// writes -- one on the leading edge of the burst, one on the trailing.
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
            performed <= 2,
            "{EVENTS} slider events cost {performed} settings writes; \
             the drag should have collapsed to a leading and a trailing write",
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
    async fn matrix_disconnect_triggers_retry() {
        let tmp = tempfile::tempdir().unwrap();
        let matrix = MockMatrix::new().with_internal_events(vec![
            InternalEvent::Matrix(InternalMatrixEvent::Disconnected(
                SyncEnd::RetriesExhausted { reason: "test disconnect".into() },
            )),
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

    // --- Sync health: degrade in place, tear down only when it is over ---

    /// Everything the engine emitted, in order, for a test that needs to reason
    /// about what came after what.
    fn conn_states(events: &[CoreEvent]) -> Vec<&ConnectionState> {
        events.iter().filter_map(|e| match e {
            CoreEvent::Matrix(MatrixEvent::ConnectionState(s)) => Some(s),
            _ => None,
        }).collect()
    }

    /// Connect, then hand the engine `events` the way the sync task would, let
    /// it sit for `linger`, and return everything it emitted throughout.
    ///
    /// `linger` is what makes "nothing was torn down" a claim about the future
    /// rather than about the instant after the injection. The engine schedules
    /// a teardown on a timer, so a test that shuts down immediately would pass
    /// against code that had scheduled one; waiting past the shortest backoff
    /// (`backoff_secs(1)`, two seconds) is what makes the absence real.
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

    /// Long enough for the shortest retry backoff to have fired if one had
    /// been scheduled.
    const PAST_THE_FIRST_BACKOFF: Duration = Duration::from_millis(2_500);

    /// The bug, at the level of the engine. A sync request that failed and is
    /// being retried must cost the user nothing: no `ServerReset`, so the
    /// frontend keeps its stores, and no second `reset()` on the backend, so
    /// the timelines and subscriptions behind them stay subscribed. The only
    /// thing that may happen is the UI being told the connection is working
    /// on something.
    #[tokio::test]
    async fn a_transient_sync_failure_does_not_tear_the_session_down() {
        let tmp = tempfile::tempdir().unwrap();
        let (events, matrix_state) = connect_then_inject(tmp.path(), vec![
            InternalEvent::Matrix(InternalMatrixEvent::SyncDegraded {
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

    /// And when the retry works, the session comes back where it was: still
    /// the same connection, still the same timelines, reported as `Connected`
    /// again without a reconnect in between.
    #[tokio::test]
    async fn a_recovered_sync_returns_to_connected_without_reconnecting() {
        let tmp = tempfile::tempdir().unwrap();
        let (events, matrix_state) = connect_then_inject(tmp.path(), vec![
            InternalEvent::Matrix(InternalMatrixEvent::SyncDegraded { reason: "blip".into() }),
            InternalEvent::Matrix(InternalMatrixEvent::SyncRecovered),
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
            "expected connect, degrade, recover -- got {states:?}",
        );
    }

    /// The backstop. Once the sync loop has exhausted its retries the session
    /// really is over, and the engine has to go back to the path it always
    /// had: schedule a retry and report the failure with the reason the sync
    /// loop gave.
    #[tokio::test]
    async fn a_sync_loop_that_gave_up_falls_back_to_the_reconnect_path() {
        let tmp = tempfile::tempdir().unwrap();
        let (events, _) = connect_then_inject(tmp.path(), vec![
            InternalEvent::Matrix(InternalMatrixEvent::Disconnected(
                SyncEnd::RetriesExhausted { reason: "out of retries".into() },
            )),
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

    /// A session the server disowned takes the same path -- but it arrives as
    /// a different value, which is the whole point of typing the signal. A
    /// bare string could not tell the engine whether the token was rejected or
    /// the network merely blinked, and so it had to assume the worst about
    /// both.
    #[tokio::test]
    async fn an_invalidated_session_is_distinguishable_from_exhausted_retries() {
        let tmp = tempfile::tempdir().unwrap();
        let (events, _) = connect_then_inject(tmp.path(), vec![
            InternalEvent::Matrix(InternalMatrixEvent::Disconnected(
                SyncEnd::SessionInvalidated { reason: "M_UNKNOWN_TOKEN".into() },
            )),
        ], Duration::ZERO).await;

        assert!(
            matches!(
                conn_states(&events).last(),
                Some(ConnectionState::Failed { reason, retries: 1, .. }) if reason == "M_UNKNOWN_TOKEN",
            ),
            "a rejected session must still drive the disconnect-and-retry path, got {:?}",
            conn_states(&events),
        );

        // The two ends are separate variants carrying separate reasons, so the
        // engine can act on the difference; today it reports it.
        assert_ne!(
            SyncEnd::SessionInvalidated { reason: "x".into() },
            SyncEnd::RetriesExhausted { reason: "x".into() },
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

    // --- Voice relaunch on reconnect ---

    /// A Matrix sync hiccup says nothing about the voice server. Relaunching
    /// Mumble anyway drops the user out of voice for several seconds, which in
    /// one five-day session happened 28 times without a single voice-side
    /// failure to justify it.
    #[tokio::test]
    async fn reconnect_keeps_a_live_voice_session_on_the_same_server() {
        let tmp = tempfile::tempdir().unwrap();
        let matrix = MockMatrix::new().with_repeating_connect_result(ConnectOutcome::Connected(None));
        // The first launch reports the voice session as established.
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

    /// Voice that is down must still be brought back by a reconnect.
    #[tokio::test]
    async fn reconnect_relaunches_voice_when_it_is_not_connected() {
        let tmp = tempfile::tempdir().unwrap();
        let matrix = MockMatrix::new().with_repeating_connect_result(ConnectOutcome::Connected(None));
        // No Connected event, so voice never comes up.
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

    /// Losing the voice connection re-arms the relaunch.
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

    /// A different voice server is a different session: relaunch, and do not
    /// carry the old channel/mute/deafen state over to it.
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

    /// A launch kills the running Mumble before it spawns the new one, so a
    /// launch that fails leaves voice down, not where it was. Recording the
    /// requested credentials as a live session hides that: the next reconnect
    /// takes the skip branch and voice stays dead until the app restarts.
    #[tokio::test]
    async fn a_failed_launch_is_retried_on_the_next_reconnect() {
        let tmp = tempfile::tempdir().unwrap();
        let matrix = MockMatrix::new().with_repeating_connect_result(ConnectOutcome::Connected(None));
        // Launch 1 brings voice up on the first server. Launch 2, onto the
        // second server, fails to spawn. Launch 3 must still be attempted.
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

        // The failed launch records nothing, so a recovered session shows up
        // as the second recorded launch.
        let launches = voice_state.launched_with.lock().unwrap();
        assert_eq!(
            launches.len(), 2,
            "a reconnect after a failed launch should try again, got {:?}",
            launches,
        );
        assert_eq!(launches[1].host, "other.example.com");
    }

    // --- Event loop responsiveness ---

    /// A connection attempt makes several network round trips. It must not be
    /// awaited inline on the select loop: while the loop is parked, nothing
    /// drains `cmd_rx`, so the bounded channel from the Tauri layer fills and
    /// every subsequent `invoke` hangs, which the user sees as a frozen UI.
    ///
    /// The connect now runs in the Matrix actor and reports back as an
    /// internal event, so the loop is free for the whole of it.
    #[tokio::test]
    async fn engine_keeps_serving_commands_while_a_connect_is_in_flight() {
        let tmp = tempfile::tempdir().unwrap();
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel();
        let matrix = MockMatrix::new().with_connect_gate(gate_rx);

        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let (_media_tx, media_rx) = mpsc::channel(32);
        let (event_tx, mut event_rx) = mpsc::channel(100);
        let engine = CoreEngine::new(
            cmd_rx, media_rx, event_tx, matrix, MockVoice::new(),
            tmp.path().to_path_buf(), settings::load(tmp.path()),
        );
        let engine_handle = tokio::spawn(async move { engine.run().await });

        // Start a connect that will not complete until the gate is released.
        cmd_tx.send(CoreCommand::System(SystemCommand::ConnectToServer(connect_form())))
            .await.unwrap();

        // Wait until the engine is genuinely inside the connect.
        let saw_connecting = timeout(Duration::from_secs(2), async {
            while let Some(event) = event_rx.recv().await {
                if matches!(
                    event,
                    CoreEvent::Matrix(MatrixEvent::ConnectionState(ConnectionState::Connecting))
                ) {
                    return true;
                }
            }
            false
        }).await.expect("engine never reported Connecting");
        assert!(saw_connecting);

        // An unrelated command issued now must still be served.
        cmd_tx.send(CoreCommand::System(SystemCommand::LoadSettings)).await.unwrap();
        let served = timeout(Duration::from_secs(2), async {
            while let Some(event) = event_rx.recv().await {
                if matches!(event, CoreEvent::System(SystemEvent::SettingsLoaded(_))) {
                    return true;
                }
            }
            false
        }).await;

        let _ = gate_tx.send(());
        drop(cmd_tx);
        let _ = timeout(Duration::from_secs(5), engine_handle).await;

        assert!(
            matches!(served, Ok(true)),
            "engine stopped serving commands while a connect was in flight",
        );
    }

    /// The property behind the test above, stated directly: a burst of UI
    /// commands issued during a connect is served *while* the connect is
    /// still running, not afterwards.
    ///
    /// The burst is deliberately several times the control channel's depth.
    /// A loop parked in the connect would fill that channel and then block
    /// the sender, so the burst would not even finish being issued -- which
    /// is exactly what the frozen UI was.
    #[tokio::test]
    async fn a_burst_of_commands_is_served_before_an_in_flight_connect_completes() {
        const CONTROL_CAPACITY: usize = 32;
        const BURST: usize = CONTROL_CAPACITY * 4;

        let tmp = tempfile::tempdir().unwrap();
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel();
        let matrix = MockMatrix::new().with_connect_gate(gate_rx);
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

        // The user carries on using the app: a slider drag, then mute.
        let issued = timeout(Duration::from_secs(5), async {
            for i in 0..BURST {
                cmd_tx.send(CoreCommand::Mumble(MumbleCommand::SetVadThreshold(i as f64 / 1000.0)))
                    .await.unwrap();
            }
        }).await;
        assert!(
            issued.is_ok(),
            "{BURST} commands could not even be enqueued on a {CONTROL_CAPACITY}-slot \
             channel while a connect was in flight",
        );

        // All of them must be served while the connect is still gated.
        let served = timeout(Duration::from_secs(5), async {
            loop {
                if voice_state.commands.lock().unwrap().len() >= BURST {
                    return;
                }
                tokio::task::yield_now().await;
            }
        }).await;
        assert!(
            served.is_ok(),
            "only {} of {BURST} commands were served before the connect completed",
            voice_state.commands.lock().unwrap().len(),
        );

        // And the control channel is not merely draining eventually -- it is
        // empty, which is the number the instrumentation reports during a
        // connect.
        assert_eq!(
            cmd_tx.capacity(), cmd_tx.max_capacity(),
            "the control channel still had commands backed up behind the connect",
        );

        let _ = gate_tx.send(());
        drop(cmd_tx);
        let _ = timeout(Duration::from_secs(5), engine_handle).await;
    }

    /// Asking to connect again while an attempt is in flight supersedes it.
    ///
    /// Queueing instead would be wrong twice over: the user asking again, or
    /// picking a different server, means the attempt underway is the one they
    /// no longer want, and a first attempt that never returns would hold the
    /// second one behind it forever. So the first attempt here is held open
    /// and never released -- the engine can only reach Connected if the
    /// supersede genuinely stopped it rather than waited on it.
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

        // Wait until the first attempt is genuinely inside `connect()`, so
        // that what the second one supersedes is work actually underway.
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
            "the second attempt should have connected, and only it -- the first \
             never returned, so reaching Connected at all means it was stopped",
        );

        assert_eq!(
            voice_state.launched_with.lock().unwrap().len(), 1,
            "only the attempt that completed should have launched voice",
        );
    }

    /// A request the Matrix actor cannot take has to be reported as a failed
    /// attempt, not left outstanding.
    ///
    /// `pending_connect` is what gates the retry timer, so a dispatch that
    /// quietly went nowhere would leave the app unable to connect ever again,
    /// with nothing but one log line to say so. Reaching `Failed` puts that
    /// in front of the user and re-arms the retry; a retry that finds the
    /// actor still gone simply fails again on a backoff, which is correct.
    #[tokio::test]
    async fn a_connect_that_cannot_be_dispatched_fails_and_re_arms_the_retry() {
        let tmp = tempfile::tempdir().unwrap();
        let (engine, cmd_tx, event_rx) = build_engine(MockMatrix::new(), MockVoice::new(), tmp.path());
        engine.kill_matrix_actor().await;
        let driver = EngineDriver::start(engine, cmd_tx, event_rx);

        // Settling at all is half the assertion: the barrier is only answered
        // once nothing dispatched is still outstanding.
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

    /// The same for the voice side: a launch the actor cannot take clears
    /// `pending_launch` and records the session as down, by the same path a
    /// launch that failed inside the actor takes.
    #[tokio::test]
    async fn a_voice_launch_that_cannot_be_dispatched_does_not_stay_outstanding() {
        let tmp = tempfile::tempdir().unwrap();
        let voice = MockVoice::new();
        let voice_state = voice.state.clone();
        let (engine, cmd_tx, event_rx) = build_engine(MockMatrix::new(), voice, tmp.path());
        engine.kill_voice_actor().await;
        let driver = EngineDriver::start(engine, cmd_tx, event_rx);

        // An explicit Mumble host, so the connect dispatches a launch.
        let mut form = connect_form();
        form.mumble_host = Some("voice.example.com".into());
        driver.step(CoreCommand::System(SystemCommand::ConnectToServer(form))).await;
        let events = driver.finish().await;

        assert!(
            voice_state.launched_with.lock().unwrap().is_empty(),
            "the actor was gone, so nothing can have been launched",
        );
        // The dead voice actor must not have taken the Matrix side with it.
        assert!(
            events.iter().any(|e| matches!(e,
                CoreEvent::Matrix(MatrixEvent::ConnectionState(ConnectionState::Connected))
            )),
            "a failed voice dispatch must not stop the Matrix connect from landing",
        );
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

    /// A cert prompt blocks the launch, so Mumble is still sitting on the
    /// server it was already joined to, not the one whose cert changed.
    /// Counting that as a live session on the new server strands the user on
    /// the old one: the next reconnect skips, and the prompt never fires
    /// again, so there is nothing left to accept.
    #[tokio::test]
    async fn a_pending_cert_prompt_does_not_count_as_a_live_session() {
        let (first_port, first_fp) = start_tls_server().await;
        let (second_port, _second_fp) = start_tls_server().await;
        let tmp = tempfile::tempdir().unwrap();
        // The first server's cert is the one we have stored; the second's is not.
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
        // The only launch that gets through is the first one, and it brings
        // voice up on the first server.
        let voice = MockVoice::new().with_internal_events(vec![
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

    /// While a cert prompt is up we deliberately did not launch, so the Mumble
    /// that is still running is joined to the *old* server. If that process
    /// re-joins on its own -- the old server bounces, Mumble reconnects -- the
    /// bridge reports Connected. Crediting that to the credentials we asked
    /// for records a live session on a server Mumble has never reached, and
    /// the next reconnect skips the blocked launch all over again.
    #[tokio::test]
    async fn a_voice_connect_while_awaiting_a_cert_does_not_mark_the_session_live() {
        let (first_port, first_fp) = start_tls_server().await;
        let (second_port, _second_fp) = start_tls_server().await;
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

        // No launch reports Connected by itself: every Connected below is
        // injected, so it is unambiguous which one the engine is reacting to.
        let voice = MockVoice::new();
        let voice_state = voice.state.clone();
        let matrix = MockMatrix::new().with_repeating_connect_result(ConnectOutcome::Connected(None));
        let (engine, cmd_tx, event_rx) = build_engine(matrix, voice, tmp.path());
        let driver = EngineDriver::start(engine, cmd_tx, event_rx);

        // Voice comes up on the first server.
        driver.step(CoreCommand::System(SystemCommand::ConnectToServer(voice_form(first_port)))).await;
        driver.inject(InternalEvent::Mumble(InternalMumbleEvent::Connected)).await;
        driver.settle().await;

        // The second server's cert has changed, so its launch is blocked on
        // the user. Mumble is untouched, still on the first server.
        driver.step(CoreCommand::System(SystemCommand::ConnectToServer(voice_form(second_port)))).await;

        // That still-running Mumble re-joins the first server by itself.
        driver.inject(InternalEvent::Mumble(InternalMumbleEvent::Connected)).await;
        driver.settle().await;

        // The reconnect must still re-attempt the launch the user never approved.
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
        // Nothing else is sent, so close the command channel now: the engine
        // is only being run below to drain what is dispatched here.
        drop(cmd_tx);

        // Simulate state saved from a previous session
        engine.voice_session = VoiceSessionState {
            channel_path: Some("Voice/General".into()),
            muted: true,
            deafened: true,
        };

        // Connecting to a server resolves fresh credentials, and none of the
        // old session's channel/mute/deafen belongs to them.
        engine.resolve_and_launch_voice(&connect_form(), None, false, "").await;
        assert_eq!(engine.voice_session, VoiceSessionState::default());

        // Run the engine out so the dispatched launch reaches the mock. The
        // command channel is already closed, so this is just the shutdown
        // drain finishing what was dispatched.
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
