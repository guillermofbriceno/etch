//! Each subsystem lives in a task that owns it, so a slow connect or launch never
//! blocks the engine's event loop.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use crate::commands::{MatrixCommand, MumbleCommand, ServerConnectionForm};
use crate::events::{CoreEvent, InternalEvent, InternalMatrixEvent, InternalMumbleEvent, LaunchOutcome, MumbleEvent};
use crate::models::VoiceServerConfig;
use crate::task::AbortOnDrop;
use crate::traits::{MatrixBackend, VoiceService};

/// Matches the Tauri control channel depth, so the engine can forward a full channel
/// without blocking.
const CONTROL_QUEUE: usize = 32;

/// Separate from control so a burst of image loads cannot starve commands or block the engine.
const MEDIA_QUEUE: usize = 256;

pub(crate) enum MatrixRequest {
    /// Reset and connect are one request so nothing can be dispatched between them.
    Connect {
        form: ServerConnectionForm,
        internal_tx: mpsc::Sender<InternalEvent>,
        generation: u64,
        /// Fired when a newer attempt supersedes this one.
        cancel: oneshot::Receiver<()>,
    },
    Command(MatrixCommand),
    Subscribe(String),
    ResolveVoiceUser {
        session_id: u32,
        name: String,
        volume_db: f32,
        internal_tx: mpsc::Sender<InternalEvent>,
    },
}

pub(crate) struct MediaFetch {
    pub mxc_url: String,
    pub respond: oneshot::Sender<Result<Vec<u8>, String>>,
}

/// The engine's end of the Matrix actor.
pub(crate) struct MatrixHandle {
    /// `None` once shutdown has closed it.
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

    /// `false` means the actor is gone and the request will never be served.
    #[must_use = "a request the actor never took may leave the engine waiting on it"]
    pub(crate) async fn send(&self, request: MatrixRequest) -> bool {
        let Some(tx) = &self.control_tx else { return false };
        if tx.send(request).await.is_err() {
            log::warn!("Matrix actor has stopped; dropping a request");
            return false;
        }
        true
    }

    /// Never awaits: a fetch that does not fit is answered with an error instead of
    /// parking the engine.
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

    pub(crate) async fn finish(&mut self, grace: Duration) {
        self.control_tx = None;
        self.media_tx = None;
        self.task.join_within(grace).await;
    }

    /// Simulates a panicked actor: task gone, receiver dropped.
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
            // Control first so image loads cannot starve user commands.
            biased;

            request = control_rx.recv() => match request {
                Some(request) => request,
                None => break,
            },

            fetch = media_rx.recv(), if media_open => {
                match fetch {
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
                    // Cancellation wins a tie: the engine has moved on to a newer attempt.
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

pub(crate) enum VoiceRequest {
    Launch(LaunchRequest),
    Command(MumbleCommand),
    /// Served in order so the launch queued behind it reads the stored fingerprint.
    AcceptCert {
        host: String,
        port: u16,
        fingerprint: String,
    },
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
            VoiceRequest::AcceptCert { host, port, fingerprint } => {
                let db_path = mumble_db_path(&data_dir);
                match crate::mumble::cert::store_cert(&db_path, &host, port, &fingerprint) {
                    Ok(()) => log::info!("Stored the accepted cert for {host}:{port}"),
                    // The queued launch will see the old fingerprint and re-prompt.
                    Err(e) => log::error!("Failed to store the accepted cert for {host}:{port}: {e:?}"),
                }
            }
            VoiceRequest::Launch(request) => {
                let outcome = launch(&mut service, &data_dir, &event_tx, &request).await;
                let _ = request.internal_tx.send(InternalEvent::Mumble(
                    InternalMumbleEvent::LaunchFinished { generation: request.generation, outcome },
                )).await;
            }
        }
    }
}

/// Shared with the spawned Mumble client via `database_location` in `mumble-conf.json`.
fn mumble_db_path(data_dir: &Path) -> PathBuf {
    data_dir.join("mumble/mumble.sqlite")
}

async fn launch<V: VoiceService>(
    service: &mut V,
    data_dir: &Path,
    event_tx: &mpsc::Sender<CoreEvent>,
    request: &LaunchRequest,
) -> LaunchOutcome {
    let creds = &request.creds;
    let db_path = mumble_db_path(data_dir);

    match crate::mumble::cert::probe_server_cert(&creds.host, creds.port).await {
        Ok(fingerprint) => match crate::mumble::cert::get_stored_cert(&db_path, &creds.host, creds.port) {
            None => {
                // TOFU: first sight of this server, trust and store.
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
                return LaunchOutcome::CertChanged;
            }
        },
        Err(e) => {
            // Probe failure is not fatal: proceed anyway.
            log::warn!("Cert probe failed for {}:{}: {:?}", creds.host, creds.port, e);
        }
    }

    // Must precede the replacement so the engine attributes the next `Connected` to this launch.
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
