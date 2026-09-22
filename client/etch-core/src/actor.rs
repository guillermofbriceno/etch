//! Each subsystem in a task that owns it.
//!
//! `MatrixBackend::connect` and `VoiceService::launch` take `&mut self`, and
//! both make several network round trips. Awaited inline on the engine's
//! `select!` loop, that borrow is a lock on the whole engine: nothing drains
//! the command channel for as long as the call runs, so the bounded queue from
//! the Tauri layer fills and every `invoke` behind it blocks. The user sees a
//! frozen UI for the length of a connect.
//!
//! The obvious fix -- move the traits to `&self` and put a `Mutex` inside each
//! service -- only swaps one exclusion mechanism for another, and a lock held
//! across an await is the same stall wearing a different hat. So instead each
//! service moves into a task that owns it outright. `&mut self` stays exactly
//! as it is, because the actor is the only thing that can reach the service;
//! there is no lock anywhere. The engine keeps a channel and stops owning
//! subsystems, and every result comes back as an `InternalEvent` on the
//! channel the engine already drains.
//!
//! Requests to one actor are served strictly in order, which is what keeps the
//! orderings the rest of the program relies on. A reset and the connect that
//! follows it travel as one request so nothing can be dispatched between them,
//! and a `MatrixCommand` issued after a connect is still handled after it.

use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use crate::commands::{MatrixCommand, MumbleCommand, ServerConnectionForm};
use crate::events::{CoreEvent, InternalEvent, InternalMatrixEvent, InternalMumbleEvent, LaunchOutcome, MumbleEvent};
use crate::models::VoiceServerConfig;
use crate::task::AbortOnDrop;
use crate::traits::{MatrixBackend, VoiceService};

/// Depth of an actor's control queue.
///
/// Sized to match the control channel the Tauri layer feeds the engine: these
/// are one-per-user-action requests plus the occasional connect, so a backlog
/// deeper than that means something is wrong rather than merely busy. Keeping
/// the two the same means the engine can forward a full control channel into
/// an actor without ever finding the far end full.
const CONTROL_QUEUE: usize = 32;

/// Depth of the Matrix actor's media queue.
///
/// Media is a data path, not a control path: the webview raises one request
/// per image, so opening a picture-heavy room bursts far past anything the
/// control queue is sized for. It gets its own queue, as deep as the one the
/// protocol handler fills, so a burst of images can be forwarded without the
/// engine ever waiting on a full channel -- which would park the loop the rest
/// of this module exists to keep free.
const MEDIA_QUEUE: usize = 256;

// ---------------------------------------------------------------------------
// Matrix
// ---------------------------------------------------------------------------

/// Work for the Matrix service. Served in the order it is dispatched.
pub(crate) enum MatrixRequest {
    /// Clear the previous session and connect a new one.
    ///
    /// Reset and connect are one request rather than two so that nothing can
    /// be dispatched between them: the point of the reset is that no state
    /// from the old session survives into the new one.
    Connect {
        form: ServerConnectionForm,
        internal_tx: mpsc::Sender<InternalEvent>,
        /// Identifies this attempt. Echoed back so the engine can tell a
        /// result it is still waiting for from one it has moved past.
        generation: u64,
        /// Fired by the engine when a newer attempt supersedes this one. The
        /// in-flight connect is then dropped where it stands rather than left
        /// to run to completion and mutate a session the engine has replaced.
        cancel: oneshot::Receiver<()>,
    },
    Command(MatrixCommand),
    Subscribe(String),
    /// Look up the Matrix profile behind a user who just joined voice. A
    /// homeserver round trip, so it cannot be done on the engine's loop.
    ResolveVoiceUser {
        session_id: u32,
        name: String,
        volume_db: f32,
        internal_tx: mpsc::Sender<InternalEvent>,
    },
}

/// A request for the bytes behind an `mxc://` URI. On its own queue; see
/// `MEDIA_QUEUE`.
pub(crate) struct MediaFetch {
    pub mxc_url: String,
    pub respond: oneshot::Sender<Result<Vec<u8>, String>>,
}

/// The engine's end of the Matrix actor.
pub(crate) struct MatrixHandle {
    /// `None` once shutdown has closed it, which is how the actor is told
    /// there is no more work coming.
    control_tx: Option<mpsc::Sender<MatrixRequest>>,
    media_tx: Option<mpsc::Sender<MediaFetch>>,
    task: AbortOnDrop,
}

impl MatrixHandle {
    pub(crate) fn spawn<M: MatrixBackend + 'static>(service: M) -> Self {
        let (control_tx, control_rx) = mpsc::channel(CONTROL_QUEUE);
        let (media_tx, media_rx) = mpsc::channel(MEDIA_QUEUE);
        let task = AbortOnDrop::new(tokio::spawn(matrix_actor(service, control_rx, media_rx)));
        Self { control_tx: Some(control_tx), media_tx: Some(media_tx), task }
    }

    /// Hand a request to the actor. `false` means the actor is gone and the
    /// request will never be served.
    ///
    /// Reported rather than swallowed because some requests leave the engine
    /// waiting on an answer that would now never come. The caller is the only
    /// thing that knows whether this one did.
    #[must_use = "a request the actor never took may leave the engine waiting on it"]
    pub(crate) async fn send(&self, request: MatrixRequest) -> bool {
        let Some(tx) = &self.control_tx else { return false };
        if tx.send(request).await.is_err() {
            log::warn!("Matrix actor has stopped; dropping a request");
            return false;
        }
        true
    }

    /// Hand a media fetch over without waiting.
    ///
    /// Never awaits: a room's worth of images must not be able to park the
    /// engine on a full queue. A fetch that will not fit is answered with an
    /// error, which the webview renders as a failed image rather than a hang.
    pub(crate) fn fetch_media(&self, mxc_url: String, respond: oneshot::Sender<Result<Vec<u8>, String>>) {
        let Some(tx) = &self.media_tx else {
            let _ = respond.send(Err("Matrix actor has stopped".into()));
            return;
        };
        if let Err(e) = tx.try_send(MediaFetch { mxc_url, respond }) {
            let (reason, request) = match e {
                mpsc::error::TrySendError::Full(r) => ("media queue is full", r),
                mpsc::error::TrySendError::Closed(r) => ("Matrix actor has stopped", r),
            };
            log::warn!("Dropping media fetch for {}: {}", request.mxc_url, reason);
            let _ = request.respond.send(Err(reason.into()));
        }
    }

    /// Stop accepting requests and let the actor run out what is already
    /// queued, giving up after `grace`.
    pub(crate) async fn finish(&mut self, grace: Duration) {
        self.control_tx = None;
        self.media_tx = None;
        self.task.join_within(grace).await;
    }

    /// Leave the actor in the state a panic inside it would: the task gone
    /// and the receiver dropped, so the next dispatch finds a closed channel.
    /// Unwinding drops the receiver just as aborting does, so this is the
    /// same situation without needing something to actually panic.
    #[cfg(test)]
    pub(crate) async fn kill_for_test(&self) {
        self.task.abort();
        if let Some(tx) = &self.control_tx {
            tx.closed().await;
        }
    }
}

async fn matrix_actor<M: MatrixBackend>(
    mut service: M,
    mut control_rx: mpsc::Receiver<MatrixRequest>,
    mut media_rx: mpsc::Receiver<MediaFetch>,
) {
    let mut media_open = true;
    loop {
        let request = tokio::select! {
            // Control first: a flood of image loads must not be able to
            // starve the commands the user is issuing.
            biased;

            request = control_rx.recv() => match request {
                Some(request) => request,
                None => break,
            },

            fetch = media_rx.recv(), if media_open => {
                match fetch {
                    // Only spawns, so this is never where the actor spends time.
                    Some(fetch) => {
                        service.spawn_media_fetch(fetch.mxc_url, fetch.respond);
                        continue;
                    }
                    None => {
                        media_open = false;
                        continue;
                    }
                }
            },
        };

        match request {
            MatrixRequest::Connect { form, internal_tx, generation, cancel } => {
                let attempt = async {
                    service.reset().await;
                    service.connect(form, internal_tx.clone()).await
                };
                tokio::pin!(attempt);

                let outcome = tokio::select! {
                    // Cancellation wins a tie: if both are ready the engine
                    // has already moved on to a newer attempt.
                    biased;
                    _ = cancel => None,
                    outcome = &mut attempt => Some(outcome),
                };

                match outcome {
                    Some(outcome) => {
                        let _ = internal_tx.send(InternalEvent::Matrix(
                            InternalMatrixEvent::ConnectFinished { generation, outcome },
                        )).await;
                    }
                    None => {
                        log::info!(
                            "Matrix connect #{generation} was superseded; stopped it where it stood",
                        );
                    }
                }
            }
            MatrixRequest::Command(cmd) => service.handle_command(cmd).await,
            MatrixRequest::Subscribe(room_id) => service.subscribe_to_room(&room_id).await,
            MatrixRequest::ResolveVoiceUser { session_id, name, volume_db, internal_tx } => {
                let (display_name, avatar_url) = service.resolve_user_profile(&name).await;
                let _ = internal_tx.send(InternalEvent::Matrix(
                    InternalMatrixEvent::VoiceUserResolved {
                        session_id, name, volume_db, display_name, avatar_url,
                    },
                )).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Voice
// ---------------------------------------------------------------------------

/// Work for the voice service. Served in the order it is dispatched.
pub(crate) enum VoiceRequest {
    Launch(LaunchRequest),
    Command(MumbleCommand),
}

pub(crate) struct LaunchRequest {
    pub creds: VoiceServerConfig,
    pub show_gui: bool,
    pub extra_args: String,
    pub channel_path: Option<String>,
    pub internal_tx: mpsc::Sender<InternalEvent>,
    pub generation: u64,
}

/// The engine's end of the voice actor.
pub(crate) struct VoiceHandle {
    tx: Option<mpsc::Sender<VoiceRequest>>,
    task: AbortOnDrop,
}

impl VoiceHandle {
    pub(crate) fn spawn<V: VoiceService + 'static>(
        service: V,
        data_dir: PathBuf,
        event_tx: mpsc::Sender<CoreEvent>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(CONTROL_QUEUE);
        let task = AbortOnDrop::new(tokio::spawn(voice_actor(service, rx, data_dir, event_tx)));
        Self { tx: Some(tx), task }
    }

    /// Hand a request to the actor. `false` means the actor is gone and the
    /// request will never be served. See `MatrixHandle::send`.
    #[must_use = "a request the actor never took may leave the engine waiting on it"]
    pub(crate) async fn send(&self, request: VoiceRequest) -> bool {
        let Some(tx) = &self.tx else { return false };
        if tx.send(request).await.is_err() {
            log::warn!("Voice actor has stopped; dropping a request");
            return false;
        }
        true
    }

    pub(crate) async fn finish(&mut self, grace: Duration) {
        self.tx = None;
        self.task.join_within(grace).await;
    }

    /// See `MatrixHandle::kill_for_test`.
    #[cfg(test)]
    pub(crate) async fn kill_for_test(&self) {
        self.task.abort();
        if let Some(tx) = &self.tx {
            tx.closed().await;
        }
    }
}

async fn voice_actor<V: VoiceService>(
    mut service: V,
    mut rx: mpsc::Receiver<VoiceRequest>,
    data_dir: PathBuf,
    event_tx: mpsc::Sender<CoreEvent>,
) {
    while let Some(request) = rx.recv().await {
        match request {
            VoiceRequest::Command(cmd) => service.send_command(cmd).await,
            VoiceRequest::Launch(request) => {
                let outcome = launch(&mut service, &data_dir, &event_tx, &request).await;
                let _ = request.internal_tx.send(InternalEvent::Mumble(
                    InternalMumbleEvent::LaunchFinished { generation: request.generation, outcome },
                )).await;
            }
        }
    }
}

/// Check the server's certificate, then replace the Mumble process.
///
/// The probe is a TLS handshake with its own timeout, which is why this is
/// here and not on the engine's loop: it is the single longest thing the
/// connect path can wait on.
async fn launch<V: VoiceService>(
    service: &mut V,
    data_dir: &std::path::Path,
    event_tx: &mpsc::Sender<CoreEvent>,
    request: &LaunchRequest,
) -> LaunchOutcome {
    let creds = &request.creds;
    let db_path = data_dir.join("mumble/mumble.sqlite");

    match crate::mumble::cert::probe_server_cert(&creds.host, creds.port).await {
        Ok(fingerprint) => match crate::mumble::cert::get_stored_cert(&db_path, &creds.host, creds.port) {
            None => {
                // TOFU: first time seeing this server, store and proceed.
                log::info!("First connection to {}:{}, storing cert fingerprint", creds.host, creds.port);
                if let Err(e) = crate::mumble::cert::store_cert(&db_path, &creds.host, creds.port, &fingerprint) {
                    log::warn!("Failed to store cert: {:?}", e);
                }
            }
            Some(ref stored) if stored == &fingerprint => {
                log::debug!("Cert fingerprint matches for {}:{}", creds.host, creds.port);
            }
            Some(_) => {
                log::warn!("Certificate changed for {}:{}, awaiting user approval", creds.host, creds.port);
                let _ = event_tx.send(CoreEvent::Mumble(MumbleEvent::CertificateChanged {
                    host: creds.host.clone(),
                    port: creds.port,
                    new_fingerprint: fingerprint,
                })).await;
                // Mumble has not been touched, so it is still joined to
                // wherever it was. The engine stashes the launch for the user
                // to approve.
                return LaunchOutcome::CertChanged;
            }
        },
        Err(e) => {
            // Probe failed (network issue, etc.) -- log and proceed anyway.
            log::warn!("Cert probe failed for {}:{}: {:?}", creds.host, creds.port, e);
        }
    }

    // Past this point the running Mumble is about to be killed and replaced,
    // so say so before it happens: a `Connected` that arrives while the new
    // process comes up belongs to this launch, and the engine needs to know
    // that before it sees it.
    let _ = request.internal_tx.send(InternalEvent::Mumble(
        InternalMumbleEvent::LaunchStarted { generation: request.generation },
    )).await;

    match service.launch(
        creds.clone(),
        request.internal_tx.clone(),
        request.show_gui,
        &request.extra_args,
        request.channel_path.as_deref(),
    ).await {
        Ok(()) => LaunchOutcome::Launched,
        Err(e) => {
            log::error!("Failed to launch voice: {:?}", e);
            LaunchOutcome::Failed
        }
    }
}
