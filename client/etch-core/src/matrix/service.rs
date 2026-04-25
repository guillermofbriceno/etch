use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use std::time::Duration;
use matrix_sdk::config::SyncSettings;
use matrix_sdk::ruma::api::client::room::create_room::v3::{Request as CreateRoomRequest, RoomPreset};
use matrix_sdk::ruma::api::client::{account::change_password, uiaa};
use matrix_sdk::ruma::UserId;
use crate::commands::{MatrixCommand, ServerConnectionForm};
use crate::events::{CoreEvent, MatrixEvent, InternalEvent, InternalMatrixEvent};
use crate::matrix::client::{start_matrix_client, ConnectionResult};
use crate::matrix::timeline::TimelineManager;
use crate::models::{RoomInfo, RoomType, VoiceServerConfig};
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

    pub async fn connect(
        &mut self,
        form: ServerConnectionForm,
        internal_tx: mpsc::Sender<InternalEvent>,
    ) -> (bool, Option<VoiceServerConfig>) {
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
                    return (false, None);
                }

                // Phase 2: settle — fetches full state for rooms joined from invites
                if let Err(e) = client.sync_once(initial_settings).await {
                    log::error!("Settlement sync failed: {:?}", e);
                    return (false, None);
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

                        // Now start the sync loop — new events will hit the subscriptions above
                        let sync_client = client.clone();
                        let itx = internal_tx.clone();
                        self.sync_handle = Some(tokio::spawn(async move {
                            let reason = match matrix::sync_loop(sync_client).await {
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

                (true, voice_server)
            }

            Ok(ConnectionResult::NeedsPassword) => {
                let _ = self.event_tx.send(
                    CoreEvent::Matrix(MatrixEvent::PasswordRequest)
                ).await;
                (false, None)
            }

            Err(e) => {
                log::error!("Error attempting to start Matrix client: {:?}", e);
                (false, None)
            }

            _unhandled => {
                log::error!("Error attempting to start Matrix client");
                (false, None)
            }
        }
    }

    pub async fn handle_command(&mut self, cmd: MatrixCommand) {
        match cmd {
            MatrixCommand::SendMessage(msg) => {
                log::debug!("[MATRIX] TX -> {}: {}", msg.room_id, msg.text);
                if let Some(client) = self.client.clone() {
                    matrix::send_message(msg.text, msg.html_body, msg.room_id, msg.attachment_path, &client).await;
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
                if let Some(room) = client.get_room(&rid) {
                    if let Err(e) = room.send_single_receipt(
                        matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType::Read,
                        matrix_sdk::ruma::events::receipt::ReceiptThread::Unthreaded,
                        eid,
                    ).await {
                        log::error!("Failed to send read receipt: {:?}", e);
                    }
                }
            }
            MatrixCommand::CreateDirectMessage { target_user_id } => {
                let Some(client) = self.client.clone() else { return };
                let Ok(target) = UserId::parse(&target_user_id) else {
                    log::error!("Invalid user ID for DM: {}", target_user_id);
                    return;
                };

                let mut request = CreateRoomRequest::new();
                request.preset = Some(RoomPreset::TrustedPrivateChat);
                request.is_direct = true;
                request.invite = vec![target.to_owned()];

                match client.create_room(request).await {
                    Ok(response) => {
                        let room_id = response.room_id().to_string();
                        let display_name = target.localpart().to_string();
                        let room_info = RoomInfo {
                            id: room_id.clone(),
                            display_name,
                            etch_room_type: RoomType::Dm,
                            channel_id: None,
                            is_default: false,
                            unread_count: 0,
                        };
                        let _ = self.event_tx.send(
                            CoreEvent::Matrix(MatrixEvent::DmCreated(room_info))
                        ).await;

                        // Subscribe to the new room's timeline in the background
                        if let Ok(rid) = matrix_sdk::ruma::RoomId::parse(&room_id) {
                            if let Some(room) = client.get_room(&rid) {
                                let event_tx = self.event_tx.clone();
                                tokio::spawn(async move {
                                    TimelineManager::subscribe_and_paginate(
                                        event_tx, &room, &rid, 20,
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

    /// Look up a user's Matrix profile by their localpart username.
    /// Returns (display_name, avatar_url).
    pub async fn resolve_user_profile(&self, username: &str) -> (Option<String>, Option<String>) {
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
}
