use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use std::time::Duration;
use matrix_sdk::config::SyncSettings;
use matrix_sdk::media::{MediaRequestParameters, MediaFormat};
use matrix_sdk::ruma::api::client::room::create_room::v3::{Request as CreateRoomRequest, RoomPreset};
use matrix_sdk::ruma::api::client::{account::change_password, uiaa};
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk::ruma::UserId;
use crate::commands::{MatrixCommand, ServerConnectionForm};
use crate::events::{CoreEvent, MatrixEvent, InternalEvent, InternalMatrixEvent};
use crate::matrix::client::{start_matrix_client, ConnectionResult};
use crate::matrix::timeline::TimelineManager;
use crate::models::{ConnectOutcome, RoomInfo, RoomType};
use crate::traits::MatrixBackend;
use crate::matrix;

use std::path::PathBuf;

pub struct MatrixService {
    pub client: Option<matrix_sdk::Client>,
    pub timeline_manager: TimelineManager,
    event_tx: mpsc::Sender<CoreEvent>,
    sync_handle: Option<JoinHandle<()>>,
    data_dir: PathBuf,
}

impl MatrixService {
    pub fn new(event_tx: mpsc::Sender<CoreEvent>, data_dir: PathBuf) -> Self {
        let timeline_manager = TimelineManager::new(event_tx.clone());
        Self {
            client: None,
            timeline_manager,
            event_tx,
            sync_handle: None,
            data_dir,
        }
    }

    async fn find_existing_dm(client: &matrix_sdk::Client, target: &UserId) -> Option<String> {
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
    async fn query_member_device_keys(client: &matrix_sdk::Client, rooms: &[RoomInfo]) {
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
        client: Option<&matrix_sdk::Client>,
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

impl MatrixBackend for MatrixService {
    async fn connect(
        &mut self,
        form: ServerConnectionForm,
        internal_tx: mpsc::Sender<InternalEvent>,
    ) -> ConnectOutcome {
        match start_matrix_client(internal_tx.clone(), self.event_tx.clone(), form, &self.data_dir).await {
            Ok(ConnectionResult::Ok(client)) => {
                self.client = Some(client.clone());

                let homeserver = client.homeserver().to_string();
                let homeserver = homeserver.trim_end_matches('/').to_string();
                let _ = self.event_tx.send(
                    CoreEvent::Matrix(MatrixEvent::HomeserverResolved(homeserver))
                ).await;

                if let Some(user_id) = client.user_id() {
                    let username = user_id.localpart().to_string();
                    let matrix_id = user_id.to_string();
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

                if let Some(handle) = self.sync_handle.take() {
                    handle.abort();
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
                    return ConnectOutcome::Failed;
                }

                // Phase 2: settle — fetches full state for rooms joined from invites
                if let Err(e) = client.sync_once(initial_settings).await {
                    log::error!("Settlement sync failed: {:?}", e);
                    return ConnectOutcome::Failed;
                }

                let mut voice_server = None;
                match matrix::fetch_rooms(&client).await {
                    Ok(rooms) => {
                        log::debug!("Rooms list: {:?}", rooms);

                        // Discover voice server from default room
                        voice_server = matrix::find_voice_server(&client, &rooms).await;
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
                        self.sync_handle = Some(tokio::spawn(async move {
                            let reason = match matrix::sync_loop(sync_client, Duration::from_secs(30)).await {
                                Ok(()) => "Sync ended".to_string(),
                                Err(e) => format!("Sync error: {}", e),
                            };
                            let _ = itx.send(InternalEvent::Matrix(
                                InternalMatrixEvent::Disconnected(reason),
                            )).await;
                        }));

                        // Paginate in background (slow, doesn't need to block connect)
                        let timeline_arcs = self.timeline_manager.timeline_arcs();
                        tokio::spawn(async move {
                            for (room_id, timeline) in &timeline_arcs {
                                if let Err(e) = timeline.paginate_backwards(20).await {
                                    log::error!("Pagination error for room {}: {:?}", room_id, e);
                                }
                            }
                        });
                    }

                    _unhandled => {
                        log::error!("Error getting initial room list");
                    }
                };

                ConnectOutcome::Connected(voice_server)
            }

            Ok(ConnectionResult::NeedsPassword) => {
                let _ = self.event_tx.send(
                    CoreEvent::Matrix(MatrixEvent::PasswordRequest)
                ).await;
                ConnectOutcome::NeedsPassword
            }

            Err(e) => {
                log::error!("Error attempting to start Matrix client: {:?}", e);
                ConnectOutcome::Failed
            }

            _unhandled => {
                log::error!("Error attempting to start Matrix client");
                ConnectOutcome::Failed
            }
        }
    }

    async fn handle_command(&mut self, cmd: MatrixCommand) {
        match cmd {
            MatrixCommand::SendMessage(msg) => {
                log::debug!("[MATRIX] TX -> {}: {}", msg.room_id, msg.text);
                if msg.attachment_path.is_some() {
                    // Attachments still go through Room::send_attachment
                    if let Some(client) = self.client.clone() {
                        matrix::send_message(msg.text, msg.html_body, msg.room_id, msg.attachment_path, &client).await;
                    }
                } else {
                    // Text messages go through Timeline::send for immediate local echo
                    use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
                    let content: matrix_sdk::ruma::events::AnyMessageLikeEventContent = match &msg.html_body {
                        Some(html) => RoomMessageEventContent::text_html(&msg.text, html).into(),
                        None => RoomMessageEventContent::text_plain(&msg.text).into(),
                    };
                    if !self.timeline_manager.send_message(&msg.room_id, content).await {
                        log::warn!("No timeline for room {}, falling back to Room::send", msg.room_id);
                        if let Some(client) = self.client.clone() {
                            matrix::send_message(msg.text, msg.html_body, msg.room_id, None, &client).await;
                        }
                    }
                }
            }
            MatrixCommand::ToggleReaction { room_id, event_id, key } => {
                self.timeline_manager.toggle_reaction(&room_id, &event_id, &key).await;
            }
            MatrixCommand::SetDisplayName(name) => {
                let Some(client) = self.client.clone() else { return };
                if let Err(e) = client.account().set_display_name(Some(&name)).await {
                    log::error!("Failed to set display name: {:?}", e);
                }
            }
            MatrixCommand::SetAvatar(path) => {
                let Some(client) = self.client.clone() else { return };
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
                let Some(client) = self.client.clone() else { return };
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
                let Some(client) = self.client.clone() else { return };
                let Ok(rid) = matrix_sdk::ruma::RoomId::parse(&room_id) else { return };
                let Ok(eid) = matrix_sdk::ruma::EventId::parse(&event_id) else { return };
                // Spawn as a background task so slow receipt RPCs don't block
                // the command loop and delay subsequent commands.
                tokio::spawn(async move {
                    if let Some(room) = client.get_room(&rid) {
                        if let Err(e) = room.send_single_receipt(
                            matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType::Read,
                            matrix_sdk::ruma::events::receipt::ReceiptThread::Unthreaded,
                            eid,
                        ).await {
                            log::error!("Failed to send read receipt: {:?}", e);
                        }
                    }
                });
            }
            MatrixCommand::EnableEncryption { room_id } => {
                let Some(client) = self.client.clone() else { return };
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
                let Some(client) = self.client.clone() else { return };
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
                    if let Ok(rid) = matrix_sdk::ruma::RoomId::parse(&existing_room_id) {
                        if let Some(room) = client.get_room(&rid) {
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
                        if let Ok(rid) = matrix_sdk::ruma::RoomId::parse(&room_id) {
                            if let Some(room) = client.get_room(&rid) {
                                let event_tx = self.event_tx.clone();
                                let media_sources = self.timeline_manager.media_sources.clone();
                                tokio::spawn(async move {
                                    TimelineManager::subscribe_and_paginate(
                                        event_tx, &room, &rid, 20, media_sources,
                                    ).await;
                                });
                            }
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
        let Some(client) = &self.client else { return (None, None) };
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
        let client = self.client.clone();
        let sources = self.timeline_manager.media_sources.clone();
        tokio::spawn(async move {
            let result = MatrixService::fetch_media_static(
                client.as_ref(), &sources, &mxc_url,
            ).await;
            let _ = respond.send(result);
        });
    }

    async fn subscribe_to_room(&mut self, room_id: &str) {
        let Some(client) = &self.client else { return };
        let Ok(rid) = matrix_sdk::ruma::RoomId::parse(room_id) else { return };
        let Some(room) = client.get_room(&rid) else { return };
        self.timeline_manager.subscribe_to_room(&room).await;
    }

    async fn reset(&mut self) {
        if let Some(handle) = self.sync_handle.take() {
            handle.abort();
        }
        self.timeline_manager.clear();
        self.client = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reset_aborts_sync_and_clears_state() {
        let tmp = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::channel(1);
        let mut service = MatrixService::new(tx, tmp.path().to_path_buf());

        // Simulate a live session: a running sync task and cached media sources.
        let sync_handle = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        });
        let abort_handle = sync_handle.abort_handle();
        service.sync_handle = Some(sync_handle);
        {
            let mut sources = service.timeline_manager.media_sources.write().unwrap();
            let uri = matrix_sdk::ruma::OwnedMxcUri::from("mxc://stale".to_owned());
            sources.insert("mxc://stale".into(), MediaSource::Plain(uri));
        }

        service.reset().await;

        // The sync handle was taken and abort() called on it.
        assert!(service.sync_handle.is_none(), "sync handle should be consumed");
        // Yield so the runtime can process the cancellation.
        tokio::task::yield_now().await;
        assert!(abort_handle.is_finished(), "sync task should be aborted");

        assert!(service.client.is_none(), "client should be cleared");

        let sources = service.timeline_manager.media_sources.read().unwrap();
        assert!(sources.get("mxc://stale").is_none(), "media sources should be cleared");
    }
}
