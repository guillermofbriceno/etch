use tokio::sync::{broadcast, mpsc, oneshot};
use std::sync::Arc;
use std::time::Duration;
use matrix_sdk::Client;
use matrix_sdk::config::SyncSettings;
use matrix_sdk::media::{MediaRequestParameters, MediaFormat};
use matrix_sdk::ruma::api::client::room::create_room::v3::{Request as CreateRoomRequest, RoomPreset};
use matrix_sdk::ruma::api::client::{account::change_password, uiaa};
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk::ruma::{RoomId, UserId};
use matrix_sdk::send_queue::SendQueueRoomError;
use crate::commands::{AttachmentSend, MatrixCommand, ServerConnectionForm};
use crate::events::{CoreEvent, MatrixEvent, InternalEvent, InternalMatrixEvent};
use crate::matrix::attachment;
use crate::temp_files::TempFiles;
use crate::matrix::client::{log_out, start_matrix_client, ConnectionResult};
use crate::matrix::encryption::{self, EncryptionWorker};
use crate::matrix::name_colors::{self, NameColorResolver, NameColorSender};
use crate::matrix::retry::credentials_rejected;
use crate::matrix::saved_session::{self, session_path};
use crate::matrix::timeline::TimelineManager;
use crate::models::{ConnectOutcome, RoomInfo, RoomType};
use crate::scripting::ScriptDispatcher;
use crate::task::AbortOnDrop;
use crate::traits::MatrixBackend;
use crate::matrix;

use std::path::PathBuf;

/// How long a sync long-poll is left open before the server answers it empty.
const SYNC_POLL_TIMEOUT: Duration = Duration::from_secs(30);

const SIGN_OUT_UNREACHABLE: &str =
    "Could not reach the server, so this device is still signed in. Check your connection and try again.";
const SIGN_OUT_NOT_CONNECTED: &str =
    "Etch is not connected to the server, so this device is still signed in. Try again once it is connected.";

fn sign_out_failure(error: &matrix_sdk::HttpError) -> String {
    match error {
        matrix_sdk::HttpError::Reqwest(_) => SIGN_OUT_UNREACHABLE.into(),
        other => format!("Signing out failed, so this device is still signed in: {other}"),
    }
}

/// The SDK reports `http://host/` for a client built from `http://host`.
fn normalize_url(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

/// Identifies the session a client belongs to, read off the client because the form's
/// MXID is only a prediction.
#[derive(Clone, PartialEq, Eq)]
struct SessionKey {
    user_id: String,
    homeserver: String,
}

impl SessionKey {
    fn of(client: &Client, form: &ServerConnectionForm) -> Self {
        let user_id = match client.user_id() {
            Some(id) => id.to_string(),
            None => {
                // Fall back to the form's prediction when the client has no session.
                log::warn!("Matrix client has no user ID; keying its session off the connection form");
                format!("@{}:{}", form.username, form.hostname)
            }
        };
        Self { user_id, homeserver: normalize_url(client.homeserver().as_str()) }
    }

    /// A cheap pre-check that can rule reuse out but never in, since the form only
    /// predicts the MXID.
    fn could_serve(&self, form: &ServerConnectionForm) -> bool {
        if self.user_id != format!("@{}:{}", form.username, form.hostname) {
            return false;
        }
        match &form.homeserver_url {
            Some(url) => self.homeserver == normalize_url(url),
            // Without an explicit URL, `hostname` is already pinned by the MXID comparison.
            None => true,
        }
    }
}

impl std::fmt::Display for SessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} via {}", self.user_id, self.homeserver)
    }
}

/// The Matrix client and the tasks tied to its lifetime, kept across reconnects because
/// each new client leaks about 18 file descriptors.
/// An `Idle` client may serve reads but nothing that writes.
enum MatrixSession {
    None,
    Idle { key: SessionKey, client: Client, name_colors: NameColorResolver, encryption: EncryptionWorker },
    /// Owning its tasks means leaving `Live` aborts them instead of stranding them
    /// against a replaced client.
    Live {
        key: SessionKey,
        client: Client,
        name_colors: NameColorResolver,
        encryption: EncryptionWorker,
        _sync: AbortOnDrop,
        _pagination: AbortOnDrop,
        _send_errors: AbortOnDrop,
    },
}

impl MatrixSession {
    /// Also available while `Idle`, so media fetches work between connections.
    fn client(&self) -> Option<&Client> {
        match self {
            Self::None => None,
            Self::Idle { client, .. } | Self::Live { client, .. } => Some(client),
        }
    }

    fn name_colors(&self) -> Option<&NameColorSender> {
        match self {
            Self::None => None,
            Self::Idle { name_colors, .. } | Self::Live { name_colors, .. } => Some(name_colors.sender()),
        }
    }

    fn encryption(&self) -> Option<&EncryptionWorker> {
        match self {
            Self::None => None,
            Self::Idle { encryption, .. } | Self::Live { encryption, .. } => Some(encryption),
        }
    }

    fn live_client(&self) -> Option<&Client> {
        match self {
            Self::Live { client, .. } => Some(client),
            _ => None,
        }
    }

    fn key(&self) -> Option<&SessionKey> {
        match self {
            Self::None => None,
            Self::Idle { key, .. } | Self::Live { key, .. } => Some(key),
        }
    }

    fn reuse_for(&mut self, form: &ServerConnectionForm) -> Option<Client> {
        match self.key() {
            Some(key) if key.could_serve(form) => {
                log::debug!("Reusing the Matrix client for {}", key);
            }
            _ => return None,
        }
        self.stand_down();
        self.client().cloned()
    }

    fn install(&mut self, key: SessionKey, client: Client, event_tx: mpsc::Sender<CoreEvent>) {
        let name_colors = NameColorResolver::spawn(client.clone(), event_tx.clone());
        let encryption = EncryptionWorker::spawn(client.clone(), event_tx);
        *self = Self::Idle { key, client, name_colors, encryption };
    }

    fn go_live(&mut self, sync: AbortOnDrop, pagination: AbortOnDrop, send_errors: AbortOnDrop) {
        let (key, client, name_colors, encryption) = match std::mem::replace(self, Self::None) {
            Self::Idle { key, client, name_colors, encryption }
            | Self::Live { key, client, name_colors, encryption, .. } => (key, client, name_colors, encryption),
            Self::None => {
                log::error!("No Matrix session to bring live; stopping the tasks just started");
                return;
            }
        };
        *self = Self::Live {
            key, client, name_colors, encryption,
            _sync: sync, _pagination: pagination, _send_errors: send_errors,
        };
    }

    fn stand_down(&mut self) {
        match std::mem::replace(self, Self::None) {
            Self::Live { key, client, name_colors, encryption, .. } => {
                *self = Self::Idle { key, client, name_colors, encryption };
            }
            other => *self = other,
        }
    }

    fn invalidate(&mut self) {
        *self = Self::None;
    }
}

async fn report_send_errors(
    mut errors: broadcast::Receiver<SendQueueRoomError>,
    event_tx: mpsc::Sender<CoreEvent>,
) {
    loop {
        match errors.recv().await {
            Ok(SendQueueRoomError { room_id, error, is_recoverable }) => {
                log::warn!("Failed to send to {room_id} (recoverable: {is_recoverable}): {error}");
                let reason = attachment::send_failure_reason(&error, is_recoverable);
                let event = MatrixEvent::SendFailed { room_id: room_id.to_string(), reason };
                if event_tx.send(CoreEvent::Matrix(event)).await.is_err() {
                    return;
                }
            }
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                log::warn!("Missed {missed} send queue errors");
            }
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

pub struct MatrixService {
    session: MatrixSession,
    timeline_manager: TimelineManager,
    event_tx: mpsc::Sender<CoreEvent>,
    data_dir: PathBuf,
    temp_files: TempFiles,
    /// Set once the stores no session uses have been removed, which is only safe before
    /// this process has built its first client.
    stores_cleaned: bool,
}

impl MatrixService {
    pub fn new(
        event_tx: mpsc::Sender<CoreEvent>,
        data_dir: PathBuf,
        dispatcher: Arc<ScriptDispatcher>,
        temp_files: TempFiles,
    ) -> Self {
        let timeline_manager = TimelineManager::new(event_tx.clone(), dispatcher);
        Self {
            session: MatrixSession::None,
            timeline_manager,
            event_tx,
            data_dir,
            temp_files,
            stores_cleaned: false,
        }
    }

    /// A failed send disables its room's queue, so sending there again retries what it held back, in order.
    fn resume_send_queue(client: &Client, room_id: &str) {
        let Some(room) = RoomId::parse(room_id).ok().and_then(|id| client.get_room(&id)) else { return };
        let queue = room.send_queue();
        if !queue.is_enabled() {
            log::info!("Resuming the send queue for {room_id}");
            queue.set_enabled(true);
        }
    }

    async fn send_attachment(&self, request: AttachmentSend) {
        let AttachmentSend { room_id, path, compress, media_info } = request;
        let outcome = match self.serving_client("SendAttachment") {
            Some(client) => {
                let server_limit = attachment::server_upload_limit(&client).await;
                match attachment::prepare(&path, compress, server_limit, media_info, &self.temp_files).await {
                    Ok(prepared) => {
                        Self::resume_send_queue(&client, &room_id);
                        attachment::send(&client, &room_id, prepared).await
                    }
                    Err(reason) => Err(reason),
                }
            }
            None => {
                attachment::discard(&self.temp_files, &path).await;
                Err(attachment::NOT_CONNECTED.into())
            }
        };
        let Err(reason) = outcome else { return };

        let file_name = attachment::display_name(&path);
        log::warn!("Did not send {file_name} to {room_id}: {reason}");
        let _ = self.event_tx.send(CoreEvent::Matrix(MatrixEvent::AttachmentFailed {
            room_id, file_name, reason,
        })).await;
    }

    async fn find_existing_dm(client: &Client, target: &UserId) -> Option<String> {
        for room in client.joined_rooms() {
            if !room.is_direct().await.unwrap_or(false) {
                continue;
            }
            let Ok(members) = room.members(matrix_sdk::RoomMemberships::ACTIVE).await else {
                continue;
            };
            if members.iter().any(|m| m.user_id() == target) {
                return Some(room.room_id().to_string());
            }
        }
        None
    }

    /// Force a /keys/query for all members of encrypted rooms so the crypto
    /// store has their device keys. Prevents UTDs when the sender was offline
    /// while the recipient registered their device.
    async fn query_member_device_keys(client: &Client, rooms: &[RoomInfo]) {
        let enc = client.encryption();
        let own_user = client.user_id().map(|u| u.to_owned());
        for room_info in rooms {
            if !room_info.is_encrypted { continue; }
            let Ok(room_id) = matrix_sdk::ruma::RoomId::parse(&room_info.id) else { continue };
            let Some(room) = client.get_room(&room_id) else { continue };
            let Ok(members) = room.members(matrix_sdk::RoomMemberships::ACTIVE).await else { continue };
            for member in &members {
                if Some(member.user_id()) == own_user.as_deref() { continue; }
                let _ = enc.request_user_identity(member.user_id()).await;
            }
        }
    }

    pub async fn fetch_media_static(
        client: Option<&Client>,
        sources: &crate::matrix::timeline::MediaSourceMap,
        mxc_url: &str,
    ) -> Result<Vec<u8>, String> {
        let client = client.ok_or("Not connected")?;

        let source = match sources.read().expect("media source lock").get(mxc_url).cloned() {
            Some(s) => s,
            None => {
                let uri = matrix_sdk::ruma::OwnedMxcUri::from(mxc_url.to_owned());
                MediaSource::Plain(uri)
            }
        };

        let params = MediaRequestParameters { source, format: MediaFormat::File };
        client.media().get_media_content(&params, true)
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|e| format!("Failed to fetch media: {e}"))
    }
}

impl MatrixService {
    /// Reuses the cached client when it belongs to the requested session, else builds one.
    async fn prepare_session(
        &mut self,
        form: &ServerConnectionForm,
        internal_tx: &mpsc::Sender<InternalEvent>,
    ) -> Result<Client, ConnectOutcome> {
        if let Some(client) = self.session.reuse_for(form) {
            return Ok(client);
        }

        // Release the old client first so its stores close before the new ones open.
        self.session.invalidate();

        if !std::mem::replace(&mut self.stores_cleaned, true) {
            saved_session::remove_unreferenced_stores(&self.data_dir);
        }

        match start_matrix_client(
            internal_tx.clone(), self.event_tx.clone(), form.clone(), &self.data_dir,
        ).await {
            Ok(ConnectionResult::Ok(client)) => {
                let key = SessionKey::of(&client, form);
                log::debug!("Built a new Matrix client for {}", key);
                self.session.install(key, client.clone(), self.event_tx.clone());
                Ok(client)
            }

            Ok(ConnectionResult::NeedsPassword) => {
                let _ = self.event_tx.send(
                    CoreEvent::Matrix(MatrixEvent::PasswordRequest)
                ).await;
                Err(ConnectOutcome::NeedsPassword)
            }

            Ok(ConnectionResult::Error(msg)) => {
                log::error!("Matrix client returned error: {msg}");
                Err(ConnectOutcome::Failed)
            }

            Err(e) => {
                log::error!("Error attempting to start Matrix client: {:?}", e);
                Err(ConnectOutcome::Failed)
            }
        }
    }

    /// Without this a revoked token fails every sync and each retry is handed the same dead client.
    fn forget_rejected_session(&mut self, form: &ServerConnectionForm, err: &matrix_sdk::Error) {
        if !credentials_rejected(err.client_api_error_kind()) {
            return;
        }
        log::warn!(
            "The server rejected our Matrix credentials ({err}); \
             discarding the saved session so the next attempt logs in again",
        );
        self.discard_saved_session(form);
    }

    /// Also deletes `session.json`, since `start_matrix_client` restores it without
    /// checking the token.
    fn discard_saved_session(&mut self, form: &ServerConnectionForm) {
        self.session.invalidate();

        let path = session_path(&self.data_dir, form);
        if let Err(e) = std::fs::remove_file(&path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::error!("Failed to remove the rejected session file {}: {e}", path.display());
        }
    }

    async fn subscribe(&mut self, room: &matrix_sdk::Room) {
        let Some(name_colors) = self.session.name_colors().cloned() else {
            log::warn!("Not subscribing to {}: there is no Matrix session", room.room_id());
            return;
        };
        self.timeline_manager.subscribe_to_room(room, name_colors).await;
    }

    /// Hands the request to the worker without waiting for its round trips, and answers
    /// one the worker will never see, because the frontend is waiting on an outcome.
    async fn ask_encryption(&self, command: &str, request: encryption::Request) {
        let reason = if self.serving_client(command).is_none() {
            encryption::NOT_CONNECTED
        } else if self.session.encryption().is_some_and(|worker| worker.ask(request)) {
            return;
        } else {
            encryption::BUSY
        };
        let _ = self.event_tx.send(
            CoreEvent::Matrix(MatrixEvent::EncryptionActionFailed { reason: reason.into() })
        ).await;
    }

    fn serving_client(&self, command: &str) -> Option<Client> {
        match self.session.live_client() {
            Some(client) => Some(client.clone()),
            None => {
                log::warn!("Ignoring {command}: the Matrix session is not connected");
                None
            }
        }
    }
}

impl MatrixBackend for MatrixService {
    async fn connect(
        &mut self,
        form: ServerConnectionForm,
        internal_tx: mpsc::Sender<InternalEvent>,
        generation: u64,
    ) -> ConnectOutcome {
        let client = match self.prepare_session(&form, &internal_tx).await {
            Ok(client) => client,
            Err(outcome) => return outcome,
        };
        // Subscribed first because a send left over from an earlier run resumes as soon as its room's timeline opens.
        let send_errors = client.send_queue().subscribe_errors();

        let homeserver = client.homeserver().to_string();
        let homeserver = homeserver.trim_end_matches('/').to_string();
        let _ = self.event_tx.send(
            CoreEvent::Matrix(MatrixEvent::HomeserverResolved(homeserver))
        ).await;

        if let Some(user_id) = client.user_id() {
            let username = user_id.localpart().to_string();
            let matrix_id = user_id.to_string();
            self.timeline_manager.set_local_user_id(matrix_id.clone());
            let (display_name, avatar_url) = match client.account().fetch_user_profile_of(user_id).await {
                Ok(profile) => (
                    profile.get("displayname").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    profile.get("avatar_url").and_then(|v| v.as_str()).map(|s| s.to_string()),
                ),
                Err(e) => {
                    log::warn!("Failed to fetch user profile: {e}");
                    (None, None)
                }
            };
            let _ = self.event_tx.send(
                CoreEvent::Matrix(MatrixEvent::CurrentUser { username, matrix_id, display_name, avatar_url })
            ).await;
        }

        let name_color = name_colors::settable_on(&client).await;
        let _ = self.event_tx.send(
            CoreEvent::Matrix(MatrixEvent::Capabilities { name_color })
        ).await;

        // The frontend dropped the status with the rest of the session's state, and the worker only speaks on a change.
        if let Some(worker) = self.session.encryption() {
            worker.ask(encryption::Request::Announce);
        }

        // Enable the event cache BEFORE syncing so events from
        // sync_once are captured even without active Timeline subscriptions.
        if let Err(e) = client.event_cache().subscribe() {
            log::error!("Failed to enable event cache: {:?}", e);
        }

        let initial_settings = SyncSettings::default()
            .timeout(Duration::from_secs(0));

        // Phase 1: initial sync — discovers rooms and triggers invite auto-accepts
        if let Err(e) = client.sync_once(initial_settings.clone()).await {
            log::error!("Initial sync failed: {:?}", e);
            self.forget_rejected_session(&form, &e);
            return ConnectOutcome::Failed;
        }

        // Phase 2: settle — fetches full state for rooms joined from invites
        if let Err(e) = client.sync_once(initial_settings).await {
            log::error!("Settlement sync failed: {:?}", e);
            self.forget_rejected_session(&form, &e);
            return ConnectOutcome::Failed;
        }

        // No room list means no sync loop, so the session must not report a connection.
        let rooms = match matrix::fetch_rooms(&client).await {
            Ok(rooms) => rooms,
            Err(e) => {
                log::error!("Error getting initial room list: {e}");
                return ConnectOutcome::Failed;
            }
        };
        log::debug!("Rooms list: {:?}", rooms);

        // Discover voice server from default room
        let voice_server = matrix::find_voice_server(&client, &rooms).await;
        if let Some(ref vs) = voice_server {
            log::info!("Discovered voice server from Matrix: {}:{}", vs.host, vs.port);
        } else {
            log::info!("No etch.voice_server state event found in default room");
        }

        // Send channel list immediately so UI is responsive
        let _ = self.event_tx.send(
            CoreEvent::Matrix(MatrixEvent::ChannelList(rooms.clone()))
        ).await;

        // Subscribe to all timelines BEFORE starting the sync loop
        // so that events arriving via sync flow through the diff streams.
        for room_info in &rooms {
            if let Ok(room_id) = matrix_sdk::ruma::RoomId::parse(&room_info.id)
                && let Some(room) = client.get_room(&room_id)
            {
                self.subscribe(&room).await;
            }
        }

        // Ensure the crypto store has device keys for members of
        // encrypted rooms. Without this, messages sent while the
        // other party was offline produce UTDs because the SDK
        // never fetched their device keys. Runs synchronously so
        // keys are available before any messages can be sent.
        Self::query_member_device_keys(&client, &rooms).await;

        // Now start the sync loop — new events will hit the subscriptions above
        let sync_client = client.clone();
        let itx = internal_tx.clone();
        let sync = AbortOnDrop::new(tokio::spawn(async move {
            let end = matrix::sync_loop(sync_client, SYNC_POLL_TIMEOUT, itx.clone(), generation)
                .await;
            let _ = itx.send(InternalEvent::Matrix(
                InternalMatrixEvent::Disconnected { generation, end },
            )).await;
        }));

        // Paginate in background (slow, doesn't need to block connect)
        let timeline_arcs = self.timeline_manager.timeline_arcs();
        let pagination = AbortOnDrop::new(tokio::spawn(async move {
            for (room_id, timeline) in &timeline_arcs {
                if let Err(e) = timeline.paginate_backwards(20).await {
                    log::error!("Pagination error for room {}: {:?}", room_id, e);
                }
            }
        }));

        let send_errors = AbortOnDrop::new(tokio::spawn(report_send_errors(send_errors, self.event_tx.clone())));

        self.session.go_live(sync, pagination, send_errors);

        ConnectOutcome::Connected(voice_server)
    }

    async fn handle_command(&mut self, cmd: MatrixCommand) {
        match cmd {
            MatrixCommand::SendMessage(msg) => {
                log::debug!("[MATRIX] TX -> {}: {}", msg.room_id, msg.text);
                let Some(client) = self.serving_client("SendMessage") else { return };
                Self::resume_send_queue(&client, &msg.room_id);
                // Text messages go through Timeline::send for immediate local echo
                use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
                let content: matrix_sdk::ruma::events::AnyMessageLikeEventContent = match &msg.html_body {
                    Some(html) => RoomMessageEventContent::text_html(&msg.text, html).into(),
                    None => RoomMessageEventContent::text_plain(&msg.text).into(),
                };
                if !self.timeline_manager.send_message(&msg.room_id, content).await {
                    log::warn!("No timeline for room {}, falling back to Room::send", msg.room_id);
                    matrix::send_message(msg.text, msg.html_body, msg.room_id, &client).await;
                }
            }
            MatrixCommand::SendAttachment(request) => self.send_attachment(request).await,
            MatrixCommand::EditMessage { room_id, event_id, text, html_body } => {
                if self.serving_client("EditMessage").is_none() { return }
                self.timeline_manager.edit_message(&room_id, &event_id, &text, html_body.as_deref()).await;
            }
            MatrixCommand::RedactMessage { room_id, event_id } => {
                if self.serving_client("RedactMessage").is_none() { return }
                self.timeline_manager.redact_message(&room_id, &event_id).await;
            }
            MatrixCommand::ToggleReaction { room_id, event_id, key } => {
                if self.serving_client("ToggleReaction").is_none() { return }
                self.timeline_manager.toggle_reaction(&room_id, &event_id, &key).await;
            }
            MatrixCommand::SetDisplayName(name) => {
                let Some(client) = self.serving_client("SetDisplayName") else { return };
                if let Err(e) = client.account().set_display_name(Some(&name)).await {
                    log::error!("Failed to set display name: {:?}", e);
                }
            }
            MatrixCommand::SetAvatar(path) => {
                let Some(client) = self.serving_client("SetAvatar") else { return };
                let data = match std::fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        log::error!("Failed to read avatar file: {:?}", e);
                        return;
                    }
                };
                let mime = mime_guess::from_path(&path).first_or_octet_stream();
                if let Err(e) = client.account().upload_avatar(&mime, data).await {
                    log::error!("Failed to upload avatar: {:?}", e);
                }
            }
            MatrixCommand::SetNameColor(color) => {
                let Some(client) = self.serving_client("SetNameColor") else { return };
                let written = match color {
                    Some(color) => client.account().set_profile_field(color.to_profile()).await,
                    None => client.account().delete_profile_field(name_colors::field_name()).await,
                };
                if let Err(e) = written {
                    log::error!("Failed to set name color: {:?}", e);
                    return;
                }
                if let Some(user_id) = client.user_id()
                    && let Some(resolver) = self.session.name_colors()
                {
                    resolver.learned(user_id.to_string(), color);
                }
            }
            MatrixCommand::ResolveNameColors(user_ids) => match self.session.name_colors() {
                Some(resolver) => resolver.lookup(user_ids),
                None => log::warn!("Ignoring ResolveNameColors: there is no Matrix session"),
            },
            MatrixCommand::ChangePassword { current_password, new_password } => {
                let Some(client) = self.serving_client("ChangePassword") else { return };
                let user_id = client.user_id().map(|u| u.to_string()).unwrap_or_default();

                let mut request = change_password::v3::Request::new(new_password);
                let password_auth = uiaa::Password::new(
                    uiaa::UserIdentifier::UserIdOrLocalpart(user_id),
                    current_password,
                );
                request.auth = Some(uiaa::AuthData::Password(password_auth));

                match client.send(request).await {
                    Ok(_) => log::info!("Password changed successfully"),
                    Err(e) => log::error!("Failed to change password: {:?}", e),
                }
            }
            MatrixCommand::PaginateBackwards { room_id } => {
                if let Ok(rid) = matrix_sdk::ruma::OwnedRoomId::try_from(room_id.as_str()) {
                    let has_more = self.timeline_manager.paginate_backwards(&rid, 20).await;
                    let _ = self.event_tx.send(
                        CoreEvent::Matrix(MatrixEvent::PaginationComplete(room_id, has_more))
                    ).await;
                }
            }
            MatrixCommand::SendReadReceipt { room_id, event_id } => {
                let Some(client) = self.serving_client("SendReadReceipt") else { return };
                let Ok(rid) = matrix_sdk::ruma::RoomId::parse(&room_id) else { return };
                let Ok(eid) = matrix_sdk::ruma::EventId::parse(&event_id) else { return };
                // Spawn as a background task so slow receipt RPCs don't block
                // the command loop and delay subsequent commands.
                tokio::spawn(async move {
                    if let Some(room) = client.get_room(&rid)
                        && let Err(e) = room.send_single_receipt(
                            matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType::Read,
                            matrix_sdk::ruma::events::receipt::ReceiptThread::Unthreaded,
                            eid,
                        ).await
                    {
                        log::error!("Failed to send read receipt: {:?}", e);
                    }
                });
            }
            MatrixCommand::CreateRecoveryKey => {
                self.ask_encryption("CreateRecoveryKey", encryption::Request::CreateRecoveryKey).await;
            }
            MatrixCommand::ConfirmRecoveryKeySaved => {
                self.ask_encryption("ConfirmRecoveryKeySaved", encryption::Request::ConfirmRecoveryKeySaved).await;
            }
            MatrixCommand::SubmitRecoveryKey { key } => {
                self.ask_encryption("SubmitRecoveryKey", encryption::Request::SubmitRecoveryKey(key)).await;
            }
            MatrixCommand::ResetEncryption { password } => {
                self.ask_encryption("ResetEncryption", encryption::Request::Reset { password }).await;
            }
            MatrixCommand::EnableEncryption { room_id } => {
                let Some(client) = self.serving_client("EnableEncryption") else { return };
                let Ok(rid) = matrix_sdk::ruma::RoomId::parse(&room_id) else {
                    log::error!("Invalid room_id for EnableEncryption: {}", room_id);
                    return;
                };
                let Some(room) = client.get_room(&rid) else {
                    log::error!("Room not found for EnableEncryption: {}", room_id);
                    return;
                };
                let content = matrix_sdk::ruma::events::room::encryption::RoomEncryptionEventContent::with_recommended_defaults();
                match room.send_state_event(content).await {
                    Ok(_) => log::info!("Encryption enabled for room {}", room_id),
                    Err(e) => log::error!("Failed to enable encryption for room {}: {:?}", room_id, e),
                }
            }
            MatrixCommand::CreateDirectMessage { target_user_id } => {
                let Some(client) = self.serving_client("CreateDirectMessage") else { return };
                let Ok(target) = UserId::parse(&target_user_id) else {
                    log::error!("Invalid user ID for DM: {}", target_user_id);
                    return;
                };

                // Verify the target user exists on the homeserver before creating a room.
                // This prevents DM attempts with Mumble-only users who have no Matrix account.
                // Also capture their display name for the DM room info.
                let (display_name, avatar_url) = match client.account().fetch_user_profile_of(&target).await {
                    Ok(profile) => (
                        profile.get("displayname").and_then(|v| v.as_str()).map(|s| s.to_string())
                            .unwrap_or_else(|| target.localpart().to_string()),
                        profile.get("avatar_url").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    ),
                    Err(_) => {
                        log::error!("Cannot message {}: user not found on the server", target.localpart());
                        return;
                    }
                };

                // If a DM room with this user already exists, reuse it
                if let Some(existing_room_id) = Self::find_existing_dm(&client, &target).await {
                    if let Ok(rid) = matrix_sdk::ruma::RoomId::parse(&existing_room_id)
                        && let Some(room) = client.get_room(&rid)
                    {
                        let is_encrypted = room.latest_encryption_state().await
                            .map(|s| s.is_encrypted()).unwrap_or(false);
                        let unread = room.unread_notification_counts();
                        let room_info = RoomInfo {
                            id: existing_room_id,
                            display_name: display_name.clone(),
                            etch_room_type: RoomType::Dm,
                            channel_id: None,
                            is_default: false,
                            unread_count: unread.notification_count,
                            is_encrypted,
                            avatar_url: avatar_url.clone(),
                        };
                        let _ = self.event_tx.send(
                            CoreEvent::Matrix(MatrixEvent::DmCreated(room_info))
                        ).await;
                    }
                    return;
                }

                let mut request = CreateRoomRequest::new();
                request.preset = Some(RoomPreset::TrustedPrivateChat);
                request.is_direct = true;
                request.invite = vec![target.to_owned()];
                request.initial_state.push(
                    matrix_sdk::ruma::events::InitialStateEvent::with_empty_state_key(
                        matrix_sdk::ruma::events::room::encryption::RoomEncryptionEventContent::with_recommended_defaults(),
                    ).to_raw_any(),
                );

                match client.create_room(request).await {
                    Ok(response) => {
                        let room_id = response.room_id().to_string();
                        let is_encrypted = match client.get_room(response.room_id()) {
                            Some(room) => room.latest_encryption_state().await
                                .map(|s| s.is_encrypted()).unwrap_or(false),
                            None => false,
                        };
                        let room_info = RoomInfo {
                            id: room_id.clone(),
                            display_name,
                            etch_room_type: RoomType::Dm,
                            channel_id: None,
                            is_default: false,
                            unread_count: 0,
                            is_encrypted,
                            avatar_url,
                        };
                        let _ = self.event_tx.send(
                            CoreEvent::Matrix(MatrixEvent::DmCreated(room_info))
                        ).await;

                        // Fetch the target user's device keys so we can
                        // encrypt for them immediately.
                        let _ = client.encryption()
                            .request_user_identity(&target).await;

                        if let Ok(rid) = matrix_sdk::ruma::RoomId::parse(&room_id)
                            && let Some(room) = client.get_room(&rid)
                        {
                            // Manager-owned, so a reconnect cannot double the stream.
                            self.subscribe(&room).await;
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to create DM with {}: {:?}", target_user_id, e);
                    }
                }
            }
        }
    }

    async fn resolve_user_profile(&self, username: &str) -> (Option<String>, Option<String>) {
        let Some(client) = self.session.client() else { return (None, None) };
        let homeserver = client.homeserver();
        let domain = homeserver.host_str().unwrap_or_default();
        let user_id_str = format!("@{}:{}", username, domain);
        let Ok(user_id) = UserId::parse(&user_id_str) else { return (None, None) };

        match client.account().fetch_user_profile_of(&user_id).await {
            Ok(profile) => (
                profile.get("displayname").and_then(|v| v.as_str()).map(|s| s.to_string()),
                profile.get("avatar_url").and_then(|v| v.as_str()).map(|s| s.to_string()),
            ),
            Err(e) => {
                log::warn!("Failed to resolve profile for {}: {}", username, e);
                (None, None)
            }
        }
    }

    fn spawn_media_fetch(
        &self,
        mxc_url: String,
        respond: oneshot::Sender<Result<Vec<u8>, String>>,
    ) {
        let client = self.session.client().cloned();
        let sources = self.timeline_manager.media_sources.clone();
        tokio::spawn(async move {
            let result = MatrixService::fetch_media_static(
                client.as_ref(), &sources, &mxc_url,
            ).await;
            let _ = respond.send(result);
        });
    }

    async fn subscribe_to_room(&mut self, room_id: &str) {
        let Some(client) = self.session.client().cloned() else { return };
        let Ok(rid) = matrix_sdk::ruma::RoomId::parse(room_id) else { return };
        let Some(room) = client.get_room(&rid) else { return };
        self.subscribe(&room).await;
    }

    async fn reset(&mut self) {
        self.session.stand_down();
        self.timeline_manager.clear();
    }

    /// A sign out that fails keeps the client and the saved login, so reconnecting needs no password.
    async fn sign_out(&mut self, form: ServerConnectionForm) -> Result<(), String> {
        // Stopped first: the logout invalidates the token, and a sync still running would
        // report the session as rejected by the server.
        self.session.stand_down();

        match self.session.client().cloned() {
            Some(client) => {
                if let Err(e) = log_out(&client).await {
                    // A token the server no longer knows means the device is already gone.
                    if !credentials_rejected(e.client_api_error_kind()) {
                        log::warn!("Could not sign out: {e}");
                        return Err(sign_out_failure(&e));
                    }
                }
            }
            // With no client to log it out, forgetting a saved login would leave its device on the server.
            None if session_path(&self.data_dir, &form).exists() => {
                return Err(SIGN_OUT_NOT_CONNECTED.into());
            }
            None => {}
        }

        self.discard_saved_session(&form);
        self.timeline_manager.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::commands::ChatMessageSend;
    use crate::matrix::name_colors::{NameColor, UserNameColor};
    use crate::matrix::test_server::{unreachable_client, CannedHomeserver};
    use matrix_sdk::ruma::api::MatrixVersion;
    use serde_json::json;

    fn test_form() -> ServerConnectionForm {
        ServerConnectionForm {
            username: "alice".into(),
            hostname: "example.com".into(),
            port: "8448".into(),
            password: None,
            mumble_host: None,
            mumble_port: None,
            mumble_username: None,
            mumble_password: None,
            homeserver_url: Some("http://127.0.0.1:1".into()),
        }
    }

    async fn offline_client(store: &std::path::Path) -> Client {
        Client::builder()
            .homeserver_url("http://127.0.0.1:1")
            .sqlite_store(store.join("matrix_store"), None)
            .build()
            .await
            .expect("client should build without contacting the server")
    }

    fn service(dir: &std::path::Path) -> MatrixService {
        let (tx, _rx) = mpsc::channel(1);
        let dispatcher = Arc::new(ScriptDispatcher::empty());
        MatrixService::new(tx, dir.to_path_buf(), dispatcher, TempFiles::new(dir.to_path_buf()))
    }

    fn parked_task() -> (AbortOnDrop, tokio::task::AbortHandle) {
        let handle = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        });
        let abort_handle = handle.abort_handle();
        (AbortOnDrop::new(handle), abort_handle)
    }

    async fn connected_service(
        dir: &std::path::Path,
    ) -> (MatrixService, tokio::task::AbortHandle, tokio::task::AbortHandle) {
        let mut service = service(dir);
        let client = offline_client(dir).await;
        let key = SessionKey::of(&client, &test_form());
        let (sync, sync_abort) = parked_task();
        let (pagination, pagination_abort) = parked_task();
        let (send_errors, _) = parked_task();
        service.session.install(key, client, service.event_tx.clone());
        service.session.go_live(sync, pagination, send_errors);
        (service, sync_abort, pagination_abort)
    }

    /// Left running, the pagination task keeps the client alive.
    #[tokio::test]
    async fn reset_aborts_sync_and_clears_state() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut service, sync_abort, pagination_abort) = connected_service(tmp.path()).await;

        {
            let mut sources = service.timeline_manager.media_sources.write().unwrap();
            let uri = matrix_sdk::ruma::OwnedMxcUri::from("mxc://stale".to_owned());
            sources.insert("mxc://stale".into(), MediaSource::Plain(uri));
        }

        service.reset().await;

        assert!(service.session.live_client().is_none(), "the session should have stood down");
        // Yield so the runtime can process the cancellation.
        tokio::task::yield_now().await;
        assert!(sync_abort.is_finished(), "sync task should be aborted");
        assert!(pagination_abort.is_finished(), "pagination task should be aborted");

        let sources = service.timeline_manager.media_sources.read().unwrap();
        assert!(sources.get("mxc://stale").is_none(), "media sources should be cleared");
    }

    #[tokio::test]
    async fn reset_keeps_the_client_for_reuse() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut service, _sync_abort, _pagination_abort) = connected_service(tmp.path()).await;

        service.reset().await;

        assert!(service.session.client().is_some(), "reset must keep the client for the next attempt");
    }

    #[tokio::test]
    async fn acquire_reuses_only_for_a_matching_session() {
        let tmp = tempfile::tempdir().unwrap();
        let (internal_tx, _internal_rx) = mpsc::channel(1);
        let mut service = service(tmp.path());
        let form = test_form();

        let cached = offline_client(tmp.path()).await;
        let key = SessionKey { user_id: "@alice:example.com".into(), homeserver: "http://127.0.0.1:1".into() };
        service.session.install(key, cached, service.event_tx.clone());

        service.prepare_session(&form, &internal_tx).await
            .expect("a matching session should reuse the cached client");
        assert!(service.session.client().is_some(), "the session should still hold the client");

        // The build that follows cannot complete (no saved session or password), which
        // is all this asserts.
        let other = ServerConnectionForm { username: "bob".into(), ..form.clone() };
        let outcome = service.prepare_session(&other, &internal_tx).await;
        assert!(outcome.is_err(), "a different session must not reuse the cached client");
        assert!(service.session.client().is_none(), "the stale client should have been released");
    }

    #[tokio::test]
    async fn a_session_key_only_serves_its_own_account_and_homeserver() {
        let tmp = tempfile::tempdir().unwrap();
        let client = offline_client(tmp.path()).await;
        let form = test_form();

        // The offline client has no session, so the MXID falls back to the form.
        let key = SessionKey::of(&client, &form);
        assert_eq!(key.homeserver, "http://127.0.0.1:1", "the URL should be normalised");

        assert!(key.could_serve(&form), "the key should serve the form it was built from");
        assert!(
            !key.could_serve(&ServerConnectionForm { username: "bob".into(), ..form.clone() }),
            "a different account must not reuse this client",
        );
        assert!(
            !key.could_serve(&ServerConnectionForm {
                homeserver_url: Some("http://127.0.0.1:2".into()), ..form.clone()
            }),
            "a different homeserver must not reuse this client",
        );
        assert!(
            key.could_serve(&ServerConnectionForm { homeserver_url: None, ..form.clone() }),
            "without an explicit URL the MXID's hostname is what pins the server",
        );

        let server = CannedHomeserver::ok().await;
        let signed_in = server.client_for("@alice:example.com").await;
        let predicted_as_bob = ServerConnectionForm { username: "bob".into(), ..form };
        assert_eq!(
            SessionKey::of(&signed_in, &predicted_as_bob).user_id, "@alice:example.com",
            "a signed-in client is keyed by its own MXID, not the form's prediction",
        );
    }

    #[tokio::test]
    async fn a_rejected_token_forces_the_next_attempt_to_log_in_again() {
        let tmp = tempfile::tempdir().unwrap();
        let (internal_tx, _internal_rx) = mpsc::channel(16);
        let mut service = service(tmp.path());
        let server = CannedHomeserver::rejecting_the_token().await;
        let form = ServerConnectionForm { homeserver_url: Some(server.url.clone()), ..test_form() };

        let client = server.client_for("@alice:example.com").await;
        service.session.install(SessionKey::of(&client, &form), client, service.event_tx.clone());

        let saved = session_path(tmp.path(), &form);
        std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
        std::fs::write(&saved, r#"{
            "user_id": "@alice:example.com",
            "device_id": "TESTDEVICE",
            "access_token": "revoked-token"
        }"#).unwrap();

        let outcome = service.connect(form.clone(), internal_tx.clone(), 1).await;

        assert!(matches!(outcome, ConnectOutcome::Failed), "a rejected token cannot connect");
        assert!(server.requests_to("/sync") > 0, "the rejection should have come from the server");
        assert!(service.session.client().is_none(), "the rejected client must be dropped");
        assert!(!saved.exists(), "the rejected session file must be removed");

        let outcome = service.prepare_session(&form, &internal_tx).await;
        assert!(
            matches!(outcome, Err(ConnectOutcome::NeedsPassword)),
            "the next attempt should have rebuilt and reached the login path",
        );
    }

    fn save_login(dir: &std::path::Path, form: &ServerConnectionForm) -> PathBuf {
        let saved = session_path(dir, form);
        std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
        std::fs::write(
            &saved,
            r#"{"user_id":"@alice:example.com","device_id":"TESTDEVICE","access_token":"token"}"#,
        ).unwrap();
        saved
    }

    /// Returns the handle of the session's sync task.
    fn go_live(service: &mut MatrixService, client: Client, form: &ServerConnectionForm) -> tokio::task::AbortHandle {
        let (sync, sync_abort) = parked_task();
        let (pagination, _) = parked_task();
        let (send_errors, _) = parked_task();
        service.session.install(SessionKey::of(&client, form), client, service.event_tx.clone());
        service.session.go_live(sync, pagination, send_errors);
        sync_abort
    }

    #[tokio::test]
    async fn signing_out_logs_the_device_out_and_forgets_the_session() {
        // The second server has already removed the device, which leaves nothing to stay signed in to.
        for server in [CannedHomeserver::ok().await, CannedHomeserver::rejecting_the_token().await] {
            let tmp = tempfile::tempdir().unwrap();
            let form = ServerConnectionForm { homeserver_url: Some(server.url.clone()), ..test_form() };
            let mut service = service(tmp.path());
            let saved = save_login(tmp.path(), &form);
            let sync = go_live(&mut service, server.client_for("@alice:example.com").await, &form);

            let outcome = service.sign_out(form).await;

            assert_eq!(outcome, Ok(()));
            assert_eq!(server.requests_to("/logout"), 1, "the server should be asked to remove the device");
            assert!(!saved.exists(), "a login left on disk would be restored at the next start");
            assert!(service.session.client().is_none(), "the client holds the token, so it must go");
            tokio::task::yield_now().await;
            assert!(sync.is_finished(), "the session's tasks should have stopped with it");
        }
    }

    /// Signs out a live session of `client`, whose server will not carry it out, and returns the reason given.
    async fn sign_out_the_server_does_not_carry_out(client: Client, form: &ServerConnectionForm) -> String {
        let tmp = tempfile::tempdir().unwrap();
        let (internal_tx, _internal_rx) = mpsc::channel(1);
        let mut service = service(tmp.path());
        let saved = save_login(tmp.path(), form);
        go_live(&mut service, client, form);

        let outcome = tokio::time::timeout(Duration::from_secs(5), service.sign_out(form.clone())).await
            .expect("the Matrix actor serves nothing else until a sign out answers, so it must not keep trying");

        assert!(saved.exists(), "the device is still on the server, so its login must stay usable");
        service.prepare_session(form, &internal_tx).await
            .expect("the session should be there to reconnect with, without a password");
        outcome.expect_err("a device still on the server must not be reported as signed out")
    }

    #[tokio::test]
    async fn a_sign_out_the_server_does_not_carry_out_keeps_the_login_and_is_not_tried_again() {
        let unreachable = unreachable_client("@alice:example.com").await;
        assert_eq!(sign_out_the_server_does_not_carry_out(unreachable, &test_form()).await, SIGN_OUT_UNREACHABLE);

        // An answer the SDK would by default keep retrying for minutes.
        let server = CannedHomeserver::answering(|request| {
            if request.contains("/logout") {
                ("503 Service Unavailable", r#"{"errcode":"M_UNKNOWN","error":"Unavailable"}"#)
            } else {
                ("200 OK", "{}")
            }
        }).await;
        let form = ServerConnectionForm { homeserver_url: Some(server.url.clone()), ..test_form() };
        let reason = sign_out_the_server_does_not_carry_out(
            server.client_that_retries("@alice:example.com").await, &form,
        ).await;

        assert!(reason.contains("still signed in"), "the user should be told where that leaves them, got {reason:?}");
        assert_eq!(server.requests_to("/logout"), 1, "one attempt, however the server answers");
    }

    #[tokio::test]
    async fn a_write_command_is_ignored_while_the_session_is_not_live() {
        let tmp = tempfile::tempdir().unwrap();
        let server = CannedHomeserver::ok().await;
        let mut service = service(tmp.path());
        let client = server.client_for("@alice:example.com").await;
        let (sync, _sync_abort) = parked_task();
        let (pagination, _pagination_abort) = parked_task();
        let (send_errors, _) = parked_task();
        service.session.install(SessionKey::of(&client, &test_form()), client, service.event_tx.clone());
        service.session.go_live(sync, pagination, send_errors);

        service.handle_command(MatrixCommand::SetDisplayName("while live".into())).await;
        let while_live = server.requests_to("/displayname");
        assert!(while_live > 0, "a live session should have sent the write to the server");

        service.reset().await;
        service.handle_command(MatrixCommand::SetDisplayName("mid-reconnect".into())).await;

        assert_eq!(
            server.requests_to("/displayname"), while_live,
            "a session that is not live must not send writes to the server",
        );
        assert!(
            service.session.client().is_some(),
            "read-only work should still have a client to use",
        );
    }

    #[tokio::test]
    async fn a_message_to_a_room_the_client_cannot_resolve_is_dropped_without_stopping_the_service() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut service, _sync_abort, _pagination_abort) = connected_service(tmp.path()).await;
        let (event_tx, mut event_rx) = mpsc::channel(16);
        service.event_tx = event_tx;

        for room_id in ["not a room id", "!unknown:example.com"] {
            service.handle_command(MatrixCommand::SendMessage(ChatMessageSend {
                room_id: room_id.into(),
                text: "hello".into(),
                html_body: None,
            })).await;
        }

        service.handle_command(MatrixCommand::PaginateBackwards {
            room_id: "!unknown:example.com".into(),
        }).await;
        assert!(
            matches!(
                event_rx.try_recv(),
                Ok(CoreEvent::Matrix(MatrixEvent::PaginationComplete(_, false))),
            ),
            "the service should still answer the command that follows",
        );
    }

    #[tokio::test]
    async fn a_reconnect_stops_the_previous_session_tasks() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut service, sync_abort, pagination_abort) = connected_service(tmp.path()).await;
        let (internal_tx, _internal_rx) = mpsc::channel(1);

        let reused = service.prepare_session(&test_form(), &internal_tx).await
            .expect("the same session should be reusable");
        drop(reused);
        tokio::task::yield_now().await;

        assert!(sync_abort.is_finished(), "the previous sync task should be aborted");
        assert!(pagination_abort.is_finished(), "the previous pagination task should be aborted");
    }

    fn send_attachment_command(room_id: &str, path: &std::path::Path) -> MatrixCommand {
        MatrixCommand::SendAttachment(AttachmentSend {
            room_id: room_id.into(),
            path: path.to_path_buf(),
            compress: false,
            media_info: None,
        })
    }

    fn temp_upload(service: &MatrixService, name: &str, len: usize) -> std::path::PathBuf {
        service.temp_files.create(name, &vec![0u8; len]).unwrap()
    }

    fn attachment_failure(event_rx: &mut mpsc::Receiver<CoreEvent>) -> (String, String, String) {
        loop {
            match event_rx.try_recv() {
                Ok(CoreEvent::Matrix(MatrixEvent::AttachmentFailed { room_id, file_name, reason })) => {
                    return (room_id, file_name, reason);
                }
                Ok(_) => continue,
                Err(e) => panic!("no AttachmentFailed was emitted: {e:?}"),
            }
        }
    }

    #[tokio::test]
    async fn an_attachment_over_the_homeservers_limit_is_reported_and_its_temp_directory_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let server = CannedHomeserver::start("200 OK", r#"{"m.upload.size":3145728}"#).await;
        let client = server.client_for("@alice:example.com").await;
        let mut service = service(tmp.path());
        let (event_tx, mut event_rx) = mpsc::channel(16);
        service.event_tx = event_tx;
        let (sync, _sync_abort) = parked_task();
        let (pagination, _pagination_abort) = parked_task();
        let (send_errors, _) = parked_task();
        service.session.install(SessionKey::of(&client, &test_form()), client, service.event_tx.clone());
        service.session.go_live(sync, pagination, send_errors);
        let path = temp_upload(&service, "clip.mp4", 3_565_158);

        service.handle_command(send_attachment_command("!a:b", &path)).await;

        assert_eq!(
            attachment_failure(&mut event_rx),
            ("!a:b".into(), "clip.mp4".into(), "it is 3.4 MB and the server's limit is 3 MB".into()),
        );
        assert!(!path.parent().unwrap().exists(), "the temp directory should be removed");
    }

    #[tokio::test]
    async fn an_attachment_for_an_unknown_room_is_reported_and_its_temp_directory_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut service, _sync_abort, _pagination_abort) = connected_service(tmp.path()).await;
        let (event_tx, mut event_rx) = mpsc::channel(16);
        service.event_tx = event_tx;
        let path = temp_upload(&service, "photo.png", 10);

        service.handle_command(send_attachment_command("!unknown:example.com", &path)).await;

        let (_, file_name, reason) = attachment_failure(&mut event_rx);
        assert_eq!((file_name.as_str(), reason.as_str()), ("photo.png", "the room could not be found"));
        assert!(!path.parent().unwrap().exists(), "the temp directory should be removed");
    }

    #[tokio::test]
    async fn an_unreadable_attachment_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut service, _sync_abort, _pagination_abort) = connected_service(tmp.path()).await;
        let (event_tx, mut event_rx) = mpsc::channel(16);
        service.event_tx = event_tx;

        service.handle_command(send_attachment_command("!a:b", &tmp.path().join("missing.pdf"))).await;

        let (_, file_name, reason) = attachment_failure(&mut event_rx);
        assert_eq!((file_name.as_str(), reason.as_str()), ("missing.pdf", "the file could not be read"));
    }

    #[tokio::test]
    async fn an_attachment_is_refused_and_cleaned_up_while_the_session_is_not_live() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut service, _sync_abort, _pagination_abort) = connected_service(tmp.path()).await;
        let (event_tx, mut event_rx) = mpsc::channel(16);
        service.event_tx = event_tx;
        service.reset().await;
        let path = temp_upload(&service, "photo.png", 10);

        service.handle_command(send_attachment_command("!a:b", &path)).await;

        let (_, _, reason) = attachment_failure(&mut event_rx);
        assert_eq!(reason, "not connected to the server");
        assert!(!path.parent().unwrap().exists(), "the temp directory should be removed");
    }

    #[tokio::test]
    async fn an_attachment_the_send_queue_refuses_is_reported_and_its_temp_file_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let server = CannedHomeserver::start(
            "200 OK", r#"{"next_batch":"s1","rooms":{"leave":{"!left:example.com":{}}}}"#,
        ).await;
        let client = server.client_for("@alice:example.com").await;
        client.sync_once(SyncSettings::default().timeout(Duration::ZERO)).await
            .expect("the canned sync should make the room known");
        let mut service = service(tmp.path());
        let (event_tx, mut event_rx) = mpsc::channel(16);
        service.event_tx = event_tx;
        let (sync, _sync_abort) = parked_task();
        let (pagination, _pagination_abort) = parked_task();
        let (send_errors, _) = parked_task();
        service.session.install(SessionKey::of(&client, &test_form()), client, service.event_tx.clone());
        service.session.go_live(sync, pagination, send_errors);
        let path = temp_upload(&service, "photo.png", 10);

        service.handle_command(send_attachment_command("!left:example.com", &path)).await;

        let (room_id, _, reason) = attachment_failure(&mut event_rx);
        assert_eq!((room_id.as_str(), reason.as_str()), ("!left:example.com", "you are not in this room"));
        assert!(!path.parent().unwrap().exists(), "the temp directory should be removed");
    }

    /// The room of the next `SendFailed`, or `None` if none arrives in time.
    async fn send_failure(event_rx: &mut mpsc::Receiver<CoreEvent>) -> Option<String> {
        send_failure_and_reason(event_rx).await.map(|(room_id, _)| room_id)
    }

    async fn send_failure_and_reason(event_rx: &mut mpsc::Receiver<CoreEvent>) -> Option<(String, String)> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(CoreEvent::Matrix(MatrixEvent::SendFailed { room_id, reason })) = event_rx.recv().await {
                    return (room_id, reason);
                }
            }
        }).await.ok()
    }

    #[tokio::test]
    async fn a_send_that_fails_after_queuing_is_reported_and_the_next_send_to_the_room_is_tried() {
        let tmp = tempfile::tempdir().unwrap();
        // Every request gets this sync body, so every send the queue attempts in the room fails.
        let server = CannedHomeserver::start(
            "200 OK", r#"{"next_batch":"s1","rooms":{"join":{"!joined:example.com":{}}}}"#,
        ).await;
        let form = ServerConnectionForm { homeserver_url: Some(server.url.clone()), ..test_form() };
        let mut service = service(tmp.path());
        let (event_tx, mut event_rx) = mpsc::channel(64);
        service.event_tx = event_tx;
        let (internal_tx, _internal_rx) = mpsc::channel(64);
        let client = server.client_for("@alice:example.com").await;
        service.session.install(SessionKey::of(&client, &form), client, service.event_tx.clone());
        let outcome = service.connect(form, internal_tx, 1).await;
        assert!(matches!(outcome, ConnectOutcome::Connected(_)), "the canned server should accept the connect");

        let path = temp_upload(&service, "photo.png", 10);
        service.handle_command(send_attachment_command("!joined:example.com", &path)).await;
        assert_eq!(
            send_failure(&mut event_rx).await.as_deref(), Some("!joined:example.com"),
            "the failed upload should be reported",
        );
        let room_id = RoomId::parse("!joined:example.com").unwrap();
        let room = service.session.live_client().and_then(|client| client.get_room(&room_id)).unwrap();
        assert!(!room.send_queue().is_enabled(), "the failure should have disabled the room's queue");

        service.handle_command(MatrixCommand::SendMessage(ChatMessageSend {
            room_id: "!joined:example.com".into(),
            text: "hello".into(),
            html_body: None,
        })).await;
        assert_eq!(
            send_failure(&mut event_rx).await.as_deref(), Some("!joined:example.com"),
            "the next message should have been tried, not held in the disabled queue",
        );
        assert!(!room.send_queue().is_enabled(), "that failure should have disabled the queue again");

        let path = temp_upload(&service, "photo.png", 10);
        service.handle_command(send_attachment_command("!joined:example.com", &path)).await;
        assert_eq!(
            send_failure(&mut event_rx).await.as_deref(), Some("!joined:example.com"),
            "the next attachment should have been tried too",
        );
    }

    #[tokio::test]
    async fn a_send_the_server_was_too_busy_to_take_is_held_and_goes_out_first_with_the_next_send() {
        // One body that answers a sync, the media config, an upload and an event send alike.
        const SUCCESS: &str = r#"{"next_batch":"s1","rooms":{"join":{"!joined:example.com":{}}},"m.upload.size":52428800,"content_uri":"mxc://example.com/uploaded","event_id":"$sent"}"#;
        let tmp = tempfile::tempdir().unwrap();
        let unavailable = Arc::new(AtomicBool::new(false));
        let server = CannedHomeserver::answering({
            let unavailable = unavailable.clone();
            move |request| {
                if unavailable.load(Ordering::SeqCst) {
                    ("503 Service Unavailable", "{}")
                } else if request.contains("/state/m.room.encryption") {
                    ("404 Not Found", r#"{"errcode":"M_NOT_FOUND","error":"Event not found"}"#)
                } else {
                    ("200 OK", SUCCESS)
                }
            }
        }).await;
        let form = ServerConnectionForm { homeserver_url: Some(server.url.clone()), ..test_form() };
        let mut service = service(tmp.path());
        let (event_tx, mut event_rx) = mpsc::channel(64);
        service.event_tx = event_tx;
        let (internal_tx, _internal_rx) = mpsc::channel(64);
        let client = server.client_for("@alice:example.com").await;
        service.session.install(SessionKey::of(&client, &form), client, service.event_tx.clone());
        let outcome = service.connect(form, internal_tx, 1).await;
        assert!(matches!(outcome, ConnectOutcome::Connected(_)), "the canned server should accept the connect");
        // Asked while the server still answers, so the next request to meet the outage is the upload itself.
        let client = service.session.live_client().cloned().expect("the session should be live");
        assert_eq!(attachment::server_upload_limit(&client).await, Some(52_428_800));

        unavailable.store(true, Ordering::SeqCst);
        let path = temp_upload(&service, "photo.png", 10);
        service.handle_command(send_attachment_command("!joined:example.com", &path)).await;
        assert_eq!(
            send_failure_and_reason(&mut event_rx).await,
            Some((
                "!joined:example.com".into(),
                "the server could not be reached; it will be retried when you next send to this room".into(),
            )),
        );

        unavailable.store(false, Ordering::SeqCst);
        let already_logged = server.requests().len();
        service.handle_command(MatrixCommand::SendMessage(ChatMessageSend {
            room_id: "!joined:example.com".into(),
            text: "hello".into(),
            html_body: None,
        })).await;

        let delivered = || -> Vec<&'static str> {
            server.requests()[already_logged..].iter().filter_map(|request| {
                if request.contains("/media/v3/upload") {
                    Some("upload")
                } else if request.contains("/send/m.room.message/") {
                    Some("event")
                } else {
                    None
                }
            }).collect()
        };
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            while delivered().len() < 3 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await;
        assert_eq!(
            delivered(), ["upload", "event", "event"],
            "the held attachment should go out first, then the message that resumed the queue",
        );
    }

    fn name_color(hex: &str) -> NameColor {
        serde_json::from_value(json!({ "color": hex })).expect("a valid color")
    }

    #[tokio::test]
    async fn a_name_color_is_written_to_the_profile_only_while_live() {
        let tmp = tempfile::tempdir().unwrap();
        let server = CannedHomeserver::ok().await;
        let mut service = service(tmp.path());
        let client = server.client_for("@alice:example.com").await;
        let (sync, _sync_abort) = parked_task();
        let (pagination, _pagination_abort) = parked_task();
        let (send_errors, _) = parked_task();
        service.session.install(SessionKey::of(&client, &test_form()), client, service.event_tx.clone());
        service.session.go_live(sync, pagination, send_errors);

        service.handle_command(MatrixCommand::SetNameColor(Some(name_color("#62baf7")))).await;
        service.handle_command(MatrixCommand::SetNameColor(None)).await;
        let writes: Vec<String> = server.requests().iter()
            .filter(|line| line.contains("/profile/") && line.contains("etch.name_color"))
            .map(|line| line.split(' ').next().unwrap_or_default().to_string())
            .collect();
        assert_eq!(writes, ["PUT", "DELETE"], "setting writes the field and clearing deletes it");

        service.reset().await;
        service.handle_command(MatrixCommand::SetNameColor(Some(name_color("#c7b14d")))).await;
        assert_eq!(
            server.requests_to("etch.name_color"), 2,
            "a session that is not live must not send writes to the server",
        );
    }

    #[tokio::test]
    async fn name_color_lookups_are_answered_while_the_session_is_idle() {
        let tmp = tempfile::tempdir().unwrap();
        let server = CannedHomeserver::start("200 OK", r##"{"etch.name_color":{"color":"#51caa4"}}"##).await;
        let mut service = service(tmp.path());
        let (event_tx, mut event_rx) = mpsc::channel(16);
        service.event_tx = event_tx;
        let client = server.client_with_version("@alice:example.com", MatrixVersion::V1_16).await;
        service.session.install(SessionKey::of(&client, &test_form()), client, service.event_tx.clone());

        service.handle_command(MatrixCommand::ResolveNameColors(vec!["@bob:example.com".into()])).await;

        let event = tokio::time::timeout(Duration::from_secs(10), event_rx.recv())
            .await
            .expect("the name color resolver should have answered")
            .expect("the event channel closed");
        let CoreEvent::Matrix(MatrixEvent::NameColors(answers)) = event else {
            panic!("expected name colors, got {event:?}");
        };
        assert_eq!(
            answers,
            vec![UserNameColor { user_id: "@bob:example.com".into(), color: Some(name_color("#51caa4")) }],
        );
    }
}
