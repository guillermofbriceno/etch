use tokio::sync::{mpsc, oneshot};
use std::sync::Arc;
use std::time::Duration;
use matrix_sdk::Client;
use matrix_sdk::config::SyncSettings;
use matrix_sdk::media::{MediaRequestParameters, MediaFormat};
use matrix_sdk::ruma::api::client::room::create_room::v3::{Request as CreateRoomRequest, RoomPreset};
use matrix_sdk::ruma::api::client::{account::change_password, uiaa};
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk::ruma::UserId;
use crate::commands::{MatrixCommand, ServerConnectionForm};
use crate::events::{CoreEvent, MatrixEvent, InternalEvent, InternalMatrixEvent};
use crate::matrix::client::{session_path, start_matrix_client, ConnectionResult};
use crate::matrix::retry::credentials_rejected;
use crate::matrix::timeline::TimelineManager;
use crate::models::{ConnectOutcome, RoomInfo, RoomType};
use crate::scripting::ScriptDispatcher;
use crate::task::AbortOnDrop;
use crate::traits::MatrixBackend;
use crate::matrix;

use std::path::PathBuf;

/// How long a sync long-poll is left open before the server answers it empty.
const SYNC_POLL_TIMEOUT: Duration = Duration::from_secs(30);

/// Compare homeserver URLs without tripping over a trailing slash: the SDK
/// reports `http://host/` for a client built from `http://host`.
fn normalize_url(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

/// Identifies the Matrix session a client belongs to: the account it is logged
/// in as, and the homeserver it talks to. Two connection attempts that agree on
/// this can share one client.
///
/// Both halves are read off the client, never off the connection form. The form
/// carries only a *predicted* MXID, `@username:hostname`, and
/// `start_matrix_client` may hand back a client that restored a saved session
/// belonging to a different user; a key taken from the form would then claim a
/// client for an account it does not serve.
#[derive(Clone, PartialEq, Eq)]
struct SessionKey {
    user_id: String,
    homeserver: String,
}

impl SessionKey {
    /// The session a built client actually belongs to.
    fn of(client: &Client, form: &ServerConnectionForm) -> Self {
        let user_id = match client.user_id() {
            Some(id) => id.to_string(),
            None => {
                // Only reachable if `start_matrix_client` returned a client
                // that neither restored a session nor logged in. Fall back to
                // the form's prediction so the session still has a key, and
                // say so rather than pretending this was authoritative.
                log::warn!("Matrix client has no user ID; keying its session off the connection form");
                format!("@{}:{}", form.username, form.hostname)
            }
        };
        Self { user_id, homeserver: normalize_url(client.homeserver().as_str()) }
    }

    /// Could a client with this key serve what `form` is asking for?
    ///
    /// This is the cheap pre-check that decides whether reuse is worth
    /// attempting at all, and it runs before any client for the new request
    /// exists. It can rule reuse out but never in: the form only predicts an
    /// MXID, so a match is evidence that the same account was *asked for*,
    /// which is as much as the form knows. The key on the other side of the
    /// comparison is the authoritative one, taken from the cached client.
    fn could_serve(&self, form: &ServerConnectionForm) -> bool {
        if self.user_id != format!("@{}:{}", form.username, form.hostname) {
            return false;
        }
        match &form.homeserver_url {
            Some(url) => self.homeserver == normalize_url(url),
            // Without an explicit URL the homeserver is whatever discovery
            // resolves `hostname` to, and `hostname` is already pinned by the
            // MXID comparison above.
            None => true,
        }
    }
}

impl std::fmt::Display for SessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} via {}", self.user_id, self.homeserver)
    }
}

/// The Matrix client together with everything whose lifetime is tied to it.
///
/// **The client is kept across a reconnect rather than rebuilt.** Each client
/// opens four sqlite stores, so building one per attempt costs roughly 18 file
/// descriptors that only come back when the client is genuinely dropped, and
/// anything still holding a clone of it pins that cost for the life of the
/// process. Reusing one client per session takes the question off the table.
///
/// That is also why `reset` leaves a client behind: `Idle` is what "reset"
/// means here. The session still owns a client, but it is not serving —
/// its timelines have been cleared and its sync loop stopped. Nothing that
/// writes to the homeserver may run against an `Idle` client, because the
/// result would land somewhere the rest of the program is no longer watching.
/// Read-only work is free to use it; that is the point of keeping it.
///
/// The one thing that does drop the client is the server rejecting its
/// credentials; see `MatrixService::forget_rejected_session`.
enum MatrixSession {
    /// No client built yet, or the last one was invalidated.
    None,
    /// A client exists for this session but is not serving: `reset` has run, or
    /// a connect attempt has not completed.
    Idle { key: SessionKey, client: Client },
    /// Connected and syncing. Both tasks are owned here so that leaving `Live`
    /// — by reconnecting, resetting, or dropping the service — stops them
    /// instead of leaving them running against a client that is about to be
    /// replaced. A stranded sync task keeps syncing the cached client and
    /// announces a disconnect when it ends; a stranded pagination task holds
    /// `Arc<Timeline>` clones that pin the client it paginates.
    Live {
        key: SessionKey,
        client: Client,
        _sync: AbortOnDrop,
        _pagination: AbortOnDrop,
    },
}

impl MatrixSession {
    /// The client for work that only reads. Available while `Idle` so media
    /// fetches and profile lookups keep working between connections.
    fn client(&self) -> Option<&Client> {
        match self {
            Self::None => None,
            Self::Idle { client, .. } | Self::Live { client, .. } => Some(client),
        }
    }

    /// The client for work that writes to the homeserver. Only a serving
    /// session has one.
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

    /// Hand back the cached client if it belongs to the session `form` is
    /// asking for, standing down first: a new connection supersedes whatever
    /// the previous one had running.
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

    /// Adopt a freshly built client. The session is not serving yet.
    fn install(&mut self, key: SessionKey, client: Client) {
        *self = Self::Idle { key, client };
    }

    /// Start serving, taking ownership of the tasks that do the serving.
    fn go_live(&mut self, sync: AbortOnDrop, pagination: AbortOnDrop) {
        let (key, client) = match std::mem::replace(self, Self::None) {
            Self::Idle { key, client } | Self::Live { key, client, .. } => (key, client),
            Self::None => {
                // Nothing to attach the tasks to; dropping them here stops
                // them rather than leaving them running unowned.
                log::error!("No Matrix session to bring live; stopping the tasks just started");
                return;
            }
        };
        *self = Self::Live { key, client, _sync: sync, _pagination: pagination };
    }

    /// Stop serving but keep the client for the next attempt. Dropping the
    /// `Live` variant aborts the tasks it owned.
    fn stand_down(&mut self) {
        match std::mem::replace(self, Self::None) {
            Self::Live { key, client, .. } => *self = Self::Idle { key, client },
            other => *self = other,
        }
    }

    /// Forget the client entirely, so the next attempt builds a new one.
    fn invalidate(&mut self) {
        *self = Self::None;
    }
}

/// Dropping a `MatrixService` needs no `Drop` of its own: every task it owns is
/// held through an `AbortOnDrop`, inside `session` or inside the per-room
/// entries of `timeline_manager`, so dropping those fields tears the tasks down.
pub struct MatrixService {
    session: MatrixSession,
    timeline_manager: TimelineManager,
    event_tx: mpsc::Sender<CoreEvent>,
    data_dir: PathBuf,
    dispatcher: Arc<ScriptDispatcher>,
}

impl MatrixService {
    pub fn new(event_tx: mpsc::Sender<CoreEvent>, data_dir: PathBuf, dispatcher: Arc<ScriptDispatcher>) -> Self {
        let timeline_manager = TimelineManager::new(event_tx.clone(), dispatcher.clone());
        Self {
            session: MatrixSession::None,
            timeline_manager,
            event_tx,
            data_dir,
            dispatcher,
        }
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
    /// Produce the client for a connection attempt, reusing the cached one when
    /// it already belongs to the requested session, and leaving the session
    /// holding that client either way.
    ///
    /// Reuse is what keeps reconnects from leaking (see `MatrixSession`). A
    /// client is only built when there is nothing to reuse: the first
    /// connection of a session, a switch to a different account or homeserver,
    /// or the attempt after the server rejected our credentials.
    async fn prepare_session(
        &mut self,
        form: &ServerConnectionForm,
        internal_tx: &mpsc::Sender<InternalEvent>,
    ) -> Result<Client, ConnectOutcome> {
        if let Some(client) = self.session.reuse_for(form) {
            return Ok(client);
        }

        // Nothing reusable. Release what is held before building, so the old
        // client's stores are closed rather than held open alongside the new
        // client's.
        self.session.invalidate();

        match start_matrix_client(
            internal_tx.clone(), self.event_tx.clone(), form.clone(), &self.data_dir,
        ).await {
            Ok(ConnectionResult::Ok(client)) => {
                let key = SessionKey::of(&client, form);
                log::debug!("Built a new Matrix client for {}", key);
                self.session.install(key, client.clone());
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

    /// React to a sync failure the server blamed on our credentials.
    ///
    /// Without this the cache never invalidates: a revoked access token fails
    /// every sync, the engine schedules a retry, and the retry is handed the
    /// same dead client forever.
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

    /// Drop the cached client *and* the saved login session.
    ///
    /// Dropping the client alone would not help. `start_matrix_client` restores
    /// `session.json` without ever checking it against the server, so a rebuilt
    /// client presents the same dead access token and fails in the same way.
    /// Removing the file is what sends the next attempt down the login path,
    /// which either logs in with the password on the form or asks the user for
    /// one. The store is deliberately left alone: it holds the crypto and room
    /// state that a re-login should not have to rebuild.
    fn discard_saved_session(&mut self, form: &ServerConnectionForm) {
        self.session.invalidate();

        let path = session_path(&self.data_dir, form);
        if let Err(e) = std::fs::remove_file(&path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::error!("Failed to remove the rejected session file {}: {e}", path.display());
        }
    }

    /// The client for a command that writes to the homeserver, or `None` when
    /// the session is not serving one. See `MatrixSession`.
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
    ) -> ConnectOutcome {
        let client = match self.prepare_session(&form, &internal_tx).await {
            Ok(client) => client,
            Err(outcome) => return outcome,
        };

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

        // Without a room list there is nothing to subscribe to and no sync loop
        // to start, so the session cannot go live. Reporting a connection here
        // would leave the engine believing it is connected to something that
        // will never deliver an event or ever ask to be retried.
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
                self.timeline_manager.subscribe_to_room(&room).await;
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
            // Returns only once retrying in place has been ruled out; see
            // `matrix::sync_loop`. Until then this task reports degradation
            // and recovery on the same channel and the session stays up.
            let end = matrix::sync_loop(sync_client, SYNC_POLL_TIMEOUT, itx.clone()).await;
            let _ = itx.send(InternalEvent::Matrix(
                InternalMatrixEvent::Disconnected(end),
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

        self.session.go_live(sync, pagination);

        ConnectOutcome::Connected(voice_server)
    }

    async fn handle_command(&mut self, cmd: MatrixCommand) {
        match cmd {
            MatrixCommand::SendMessage(msg) => {
                log::debug!("[MATRIX] TX -> {}: {}", msg.room_id, msg.text);
                let Some(client) = self.serving_client("SendMessage") else { return };
                if msg.attachment_path.is_some() {
                    // Attachments still go through Room::send_attachment
                    matrix::send_message(msg.text, msg.html_body, msg.room_id, msg.attachment_path, &client).await;
                } else {
                    // Text messages go through Timeline::send for immediate local echo
                    use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
                    let content: matrix_sdk::ruma::events::AnyMessageLikeEventContent = match &msg.html_body {
                        Some(html) => RoomMessageEventContent::text_html(&msg.text, html).into(),
                        None => RoomMessageEventContent::text_plain(&msg.text).into(),
                    };
                    if !self.timeline_manager.send_message(&msg.room_id, content).await {
                        log::warn!("No timeline for room {}, falling back to Room::send", msg.room_id);
                        matrix::send_message(msg.text, msg.html_body, msg.room_id, None, &client).await;
                    }
                }
            }
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

                        // Subscribe to the new room's timeline in the background
                        if let Ok(rid) = matrix_sdk::ruma::RoomId::parse(&room_id)
                            && let Some(room) = client.get_room(&rid)
                        {
                            let event_tx = self.event_tx.clone();
                            let media_sources = self.timeline_manager.media_sources.clone();
                            let dispatcher = self.dispatcher.clone();
                            let local_user_id = self.timeline_manager.local_user_id().map(|s| s.to_string());
                            tokio::spawn(async move {
                                TimelineManager::subscribe_and_paginate(
                                    event_tx, &room, &rid, 20, media_sources,
                                    dispatcher, local_user_id,
                                ).await;
                            });
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
        self.timeline_manager.subscribe_to_room(&room).await;
    }

    async fn reset(&mut self) {
        self.session.stand_down();
        self.timeline_manager.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ChatMessageSend;

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

    /// Build a client without contacting a server, for lifecycle assertions.
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
        MatrixService::new(tx, dir.to_path_buf(), dispatcher)
    }

    /// A task that will not finish on its own, so that an abort is observable.
    fn parked_task() -> (AbortOnDrop, tokio::task::AbortHandle) {
        let handle = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        });
        let abort_handle = handle.abort_handle();
        (AbortOnDrop::new(handle), abort_handle)
    }

    /// Put a service into the state a completed connection leaves it in.
    async fn connected_service(
        dir: &std::path::Path,
    ) -> (MatrixService, tokio::task::AbortHandle, tokio::task::AbortHandle) {
        let mut service = service(dir);
        let client = offline_client(dir).await;
        let key = SessionKey::of(&client, &test_form());
        let (sync, sync_abort) = parked_task();
        let (pagination, pagination_abort) = parked_task();
        service.session = MatrixSession::Live { key, client, _sync: sync, _pagination: pagination };
        (service, sync_abort, pagination_abort)
    }

    #[tokio::test]
    async fn reset_aborts_sync_and_clears_state() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut service, sync_abort, _pagination_abort) = connected_service(tmp.path()).await;

        {
            let mut sources = service.timeline_manager.media_sources.write().unwrap();
            let uri = matrix_sdk::ruma::OwnedMxcUri::from("mxc://stale".to_owned());
            sources.insert("mxc://stale".into(), MediaSource::Plain(uri));
        }

        service.reset().await;

        // The session is no longer serving, so it no longer owns the task.
        assert!(service.session.live_client().is_none(), "the session should have stood down");
        // Yield so the runtime can process the cancellation.
        tokio::task::yield_now().await;
        assert!(sync_abort.is_finished(), "sync task should be aborted");

        let sources = service.timeline_manager.media_sources.read().unwrap();
        assert!(sources.get("mxc://stale").is_none(), "media sources should be cleared");
    }

    /// The pagination task holds `Arc<Timeline>` clones for rooms that a
    /// reconnect is about to resubscribe. Left running it paginates timelines
    /// nothing is listening to any more, and keeps the client alive doing it.
    #[tokio::test]
    async fn reset_aborts_the_pagination_task_too() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut service, _sync_abort, pagination_abort) = connected_service(tmp.path()).await;

        service.reset().await;
        tokio::task::yield_now().await;

        assert!(pagination_abort.is_finished(), "pagination task should be aborted");
    }

    /// `reset` prepares for the next connection attempt, and that attempt is
    /// meant to reuse the client rather than build a second one. Clearing the
    /// client here is what made every reconnect leak a set of sqlite pools.
    #[tokio::test]
    async fn reset_keeps_the_client_for_reuse() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut service, _sync_abort, _pagination_abort) = connected_service(tmp.path()).await;

        service.reset().await;

        assert!(service.session.client().is_some(), "reset must keep the client for the next attempt");
    }

    /// The cached client is only reusable for the session it belongs to.
    #[tokio::test]
    async fn acquire_reuses_only_for_a_matching_session() {
        let tmp = tempfile::tempdir().unwrap();
        let (internal_tx, _internal_rx) = mpsc::channel(1);
        let mut service = service(tmp.path());
        let form = test_form();

        let cached = offline_client(tmp.path()).await;
        let key = SessionKey { user_id: "@alice:example.com".into(), homeserver: "http://127.0.0.1:1".into() };
        service.session.install(key, cached.clone());

        let reused = service.prepare_session(&form, &internal_tx).await
            .expect("a matching session should reuse the cached client");
        assert!(
            reused.homeserver() == cached.homeserver(),
            "the cached client should have been handed back",
        );
        assert!(service.session.client().is_some(), "the session should still hold the client");

        // A different account on the same homeserver is a different session, so
        // the cached client is dropped rather than reused. The build that
        // follows cannot complete (there is no saved session and no password),
        // which is all this asserts.
        let other = ServerConnectionForm { username: "bob".into(), ..form.clone() };
        let outcome = service.prepare_session(&other, &internal_tx).await;
        assert!(outcome.is_err(), "a different session must not reuse the cached client");
        assert!(service.session.client().is_none(), "the stale client should have been released");
    }

    /// The key that decides reuse has to distinguish accounts and homeservers,
    /// and has to tolerate the trailing slash the SDK adds to a homeserver URL.
    #[tokio::test]
    async fn a_session_key_only_serves_its_own_account_and_homeserver() {
        let tmp = tempfile::tempdir().unwrap();
        let client = offline_client(tmp.path()).await;
        let form = test_form();

        // The offline client has no session, so the MXID falls back to the
        // form's prediction; the homeserver still comes from the client, which
        // reports it with a trailing slash.
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
    }

    /// A revoked access token has to cost us both the cached client and the
    /// saved session, or the connection never comes back: the retry is handed
    /// the same dead client, and rebuilding one only restores the same dead
    /// token from `session.json`, because `restore_session` never checks it
    /// against the server. Either half left in place is a permanent 60s loop.
    #[tokio::test]
    async fn a_rejected_token_forces_the_next_attempt_to_log_in_again() {
        let tmp = tempfile::tempdir().unwrap();
        let (internal_tx, _internal_rx) = mpsc::channel(1);
        let mut service = service(tmp.path());
        let form = test_form();

        let client = offline_client(tmp.path()).await;
        service.session.install(SessionKey::of(&client, &form), client);

        // A saved session that `restore_session` would happily accept, because
        // it never asks the server whether the token inside it still works.
        let saved = session_path(tmp.path(), &form);
        std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
        std::fs::write(&saved, r#"{
            "user_id": "@alice:example.com",
            "device_id": "TESTDEVICE",
            "access_token": "revoked-token"
        }"#).unwrap();

        service.discard_saved_session(&form);

        assert!(service.session.client().is_none(), "the rejected client must be dropped");
        assert!(!saved.exists(), "the rejected session file must be removed");

        // With both gone the next attempt goes through `start_matrix_client`,
        // which finds no session to restore and asks for a password rather than
        // handing back a client that will be rejected again.
        let outcome = service.prepare_session(&form, &internal_tx).await;
        assert!(
            matches!(outcome, Err(ConnectOutcome::NeedsPassword)),
            "the next attempt should have rebuilt and reached the login path",
        );
    }

    /// A command that arrives between `reset` and a completed reconnect must
    /// not write. The client outlives a reset now, so without a `Live` guard a
    /// send reaches a client whose timelines were just cleared: the timeline
    /// send finds no room, falls back to `Room::send`, and unwraps a room the
    /// client does not have. This test passing at all is the assertion.
    #[tokio::test]
    async fn a_write_command_is_ignored_while_the_session_is_not_live() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut service, _sync_abort, _pagination_abort) = connected_service(tmp.path()).await;

        service.reset().await;

        service.handle_command(MatrixCommand::SendMessage(ChatMessageSend {
            room_id: "!room:example.com".into(),
            text: "sent mid-reconnect".into(),
            html_body: None,
            attachment_path: None,
        })).await;

        assert!(service.session.live_client().is_none(), "the session must still not be serving");
        assert!(
            service.session.client().is_some(),
            "read-only work should still have a client to use",
        );
    }

    /// Going live has to take ownership of the tasks. A `Live` session that did
    /// not own them would leave a sync loop running on the cached client after
    /// the next reconnect replaced it, and that loop announces a disconnect
    /// when it finally ends.
    #[tokio::test]
    async fn a_reconnect_stops_the_previous_session_tasks() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut service, sync_abort, pagination_abort) = connected_service(tmp.path()).await;
        let (internal_tx, _internal_rx) = mpsc::channel(1);

        // A second connection attempt for the same session reuses the client.
        let reused = service.prepare_session(&test_form(), &internal_tx).await
            .expect("the same session should be reusable");
        drop(reused);
        tokio::task::yield_now().await;

        assert!(sync_abort.is_finished(), "the previous sync task should be aborted");
        assert!(pagination_abort.is_finished(), "the previous pagination task should be aborted");
    }
}
