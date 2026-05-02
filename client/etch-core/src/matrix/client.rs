use matrix_sdk::{
    Client,
    ruma::{UserId,
        api::client::uiaa,
        events::room::message::RoomMessageEventContent,
        events::room::member::{StrippedRoomMemberEvent, OriginalSyncRoomMemberEvent},
        RoomId},
    Room,
};
use matrix_sdk::store::RoomLoadSettings;
use matrix_sdk::event_handler::Ctx;
use tokio::sync::mpsc;
use crate::events::{InternalEvent, InternalMatrixEvent, CoreEvent, MatrixEvent, SystemEvent};
use crate::commands::ServerConnectionForm;
use crate::matrix;
use crate::models::{RoomInfo, RoomType};
use std::path::Path;
use matrix_sdk::attachment::AttachmentConfig;
// TODO: enable once keyring backend is configured
// use keyring::Entry;
// use rand::RngExt;
// use rand::distr::Alphanumeric;

/// Guess the MIME type for a file, correcting known misidentifications.
///
/// `mime_guess` maps several source-code extensions to media types (e.g. `.ts`
/// becomes `video/mp2t`). This function falls back to `application/octet-stream`
/// for any extension that is obviously not a real video/audio file.
fn sanitize_mime(path: &Path) -> mime_guess::mime::Mime {
    use mime_guess::mime;

    let guess = mime_guess::from_path(path).first_or_octet_stream();

    // If mime_guess says it's video or audio, verify via a small allowlist of
    // extensions that are genuinely media files. Anything else (e.g. .ts, .m4)
    // gets demoted to octet-stream so the frontend renders it as a file download.
    let is_misidentified = match (guess.type_(), guess.subtype().as_str()) {
        (mime::VIDEO, sub) => !matches!(sub, "mp4" | "webm" | "ogg" | "quicktime" | "x-matroska" | "x-msvideo" | "mpeg"),
        (mime::AUDIO, sub) => !matches!(sub, "mpeg" | "ogg" | "wav" | "webm" | "aac" | "flac" | "mp4" | "x-flac"),
        _ => false,
    };

    if is_misidentified {
        mime::APPLICATION_OCTET_STREAM
    } else {
        guess
    }
}

pub enum ConnectionResult {
    Ok(Client),
    NeedsPassword,
    #[allow(dead_code)]
    Error(String)
}

// TODO: enable once keyring backend is configured
// fn get_or_create_passphrase() -> anyhow::Result<String> {
//     let entry = Entry::new("etch_matrix_store", "etch_core")?;
//     match entry.get_password() {
//         Ok(pw) => Ok(pw),
//         Err(keyring::Error::NoEntry) => {
//             let pw: String = rand::rng()
//                 .sample_iter(&Alphanumeric)
//                 .take(32)
//                 .map(char::from)
//                 .collect();
//             entry.set_password(&pw)?;
//             Ok(pw)
//         }
//         Err(e) => Err(e.into()),
//     }
// }
//
// fn load_session() -> anyhow::Result<Option<String>> {
//     let entry = Entry::new("etch_session", "etch_core")?;
//     match entry.get_password() {
//         Ok(json) => Ok(Some(json)),
//         Err(keyring::Error::NoEntry) => Ok(None),
//         Err(e) => Err(e.into()),
//     }
// }
//
// fn save_session(json: &str) -> anyhow::Result<()> {
//     let entry = Entry::new("etch_session", "etch_core")?;
//     entry.set_password(json)?;
//     Ok(())
// }

async fn build_matrix_client(
    form: &ServerConnectionForm,
    store_path: impl AsRef<Path>,
) -> anyhow::Result<Client> {
    let builder = Client::builder().sqlite_store(store_path, None);
    match &form.homeserver_url {
        Some(url) => Ok(builder.homeserver_url(url).build().await?),
        None => Ok(builder
            .server_name_or_homeserver_url(format!("{}:{}", form.hostname, form.port))
            .build().await?),
    }
}

pub async fn start_matrix_client(tx: mpsc::Sender<InternalEvent>, event_tx: mpsc::Sender<CoreEvent>, conn_form: ServerConnectionForm, data_dir: &Path) -> anyhow::Result<ConnectionResult> {
    let user_id = UserId::parse(format!("@{}:{}", conn_form.username, conn_form.hostname))?;

    let server_dir = data_dir.join("servers").join(format!("{}@{}", conn_form.username, conn_form.hostname));
    std::fs::create_dir_all(&server_dir)?;

    // TODO: use get_or_create_passphrase() once keyring backend is configured
    let client = build_matrix_client(&conn_form, server_dir.join("matrix_store")).await?;

    let session_path = server_dir.join("session.json");
    let mut need_fresh_login = true;

    if session_path.exists() {
        match (|| async {
            let session_json = std::fs::read_to_string(&session_path)?;
            let session = serde_json::from_str(&session_json)?;
            client.matrix_auth().restore_session(session, RoomLoadSettings::default()).await?;
            anyhow::Ok(())
        })().await {
            Ok(()) => { need_fresh_login = false; }
            Err(e) => {
                log::warn!("Stale session for {}, starting fresh: {e}", conn_form.hostname);
                let _ = std::fs::remove_file(&session_path);
                let _ = std::fs::remove_dir_all(server_dir.join("matrix_store"));

                // Rebuild client with a clean store
                let client_fresh = build_matrix_client(&conn_form, server_dir.join("matrix_store")).await?;
                return start_fresh_login(client_fresh, user_id, conn_form, &session_path, tx, event_tx).await;
            }
        }
    }

    if need_fresh_login {
        return start_fresh_login(client, user_id, conn_form, &session_path, tx, event_tx).await;
    }

    register_event_handlers(&client, tx, event_tx);
    Ok(ConnectionResult::Ok(client))
}

async fn start_fresh_login(
    client: Client,
    user_id: matrix_sdk::ruma::OwnedUserId,
    conn_form: ServerConnectionForm,
    session_path: &Path,
    tx: mpsc::Sender<InternalEvent>,
    event_tx: mpsc::Sender<CoreEvent>,
) -> anyhow::Result<ConnectionResult> {
    let Some(password) = conn_form.password else {
        return Ok(ConnectionResult::NeedsPassword);
    };

    client.matrix_auth().login_username(&user_id, &password).send().await?;
    let session = client.matrix_auth().session().unwrap();
    let session_json = serde_json::to_string(&session)?;
    std::fs::write(session_path, session_json)?;

    // Bootstrap cross-signing (establishes session trust)
    if let Err(e) = client.encryption().bootstrap_cross_signing(None).await {
        // WARNING: Retrying with password auth will OVERWRITE existing cross-signing
        // keys on the server, invalidating all other verified sessions for this account.
        log::warn!("Cross-signing bootstrap failed ({e}), retrying with password auth — this will reset existing cross-signing keys");
        if let Some(response) = e.as_uiaa_response() {
            let mut password_auth = uiaa::Password::new(
                uiaa::UserIdentifier::UserIdOrLocalpart(user_id.to_string()),
                password,
            );
            password_auth.session = response.session.clone();

            if let Err(e) = client.encryption().bootstrap_cross_signing(Some(uiaa::AuthData::Password(password_auth))).await {
                log::error!("Failed to bootstrap cross-signing with password auth: {e}");
            }
        } else {
            log::error!("Cross-signing bootstrap failed (non-UIA error): {e}");
        }
    }

    register_event_handlers(&client, tx, event_tx);
    Ok(ConnectionResult::Ok(client))
}

fn register_event_handlers(client: &Client, tx: mpsc::Sender<InternalEvent>, event_tx: mpsc::Sender<CoreEvent>) {
    client.add_event_handler_context(tx.clone());
    client.add_event_handler_context(event_tx.clone());
    client.add_event_handler(
        |ev: StrippedRoomMemberEvent,
         room: Room,
         client: Client,
         Ctx(tx): Ctx<mpsc::Sender<InternalEvent>>,
         Ctx(event_tx): Ctx<mpsc::Sender<CoreEvent>>| async move {
            if ev.state_key != client.user_id().unwrap() {
                return;
            }
            log::info!("Auto-accepting invite to room {}", room.room_id());
            if let Err(e) = room.join().await {
                log::error!("Failed to accept invite to {}: {:?}", room.room_id(), e);
                return;
            }

            // Use is_direct from the invite event itself — most reliable
            // source since room.is_direct() may not be synced yet.
            let room_type = if ev.content.is_direct == Some(true) {
                RoomType::Dm
            } else {
                match matrix::build_room_info(&room).await {
                    Ok(info) => info.etch_room_type,
                    Err(_) => RoomType::Text,
                }
            };
            let room_info = RoomInfo {
                id: room.room_id().to_string(),
                display_name: room.display_name().await.map(|n| n.to_string()).unwrap_or_default(),
                etch_room_type: room_type,
                channel_id: None,
                is_default: false,
                unread_count: 0,
                is_encrypted: room.latest_encryption_state().await.map(|s| s.is_encrypted()).unwrap_or(false),
                avatar_url: room.avatar_url().map(|u| u.to_string()),
            };
            let _ = event_tx.send(
                CoreEvent::Matrix(MatrixEvent::DmCreated(room_info))
            ).await;

            // Ask the engine to subscribe via TimelineManager so the
            // timeline is registered and pagination/reactions work.
            let _ = tx.send(InternalEvent::Matrix(
                InternalMatrixEvent::SubscribeToRoom(room.room_id().to_owned())
            )).await;
        },
    );

    client.add_event_handler(
        |ev: OriginalSyncRoomMemberEvent,
         Ctx(event_tx): Ctx<mpsc::Sender<CoreEvent>>| async move {
            let prev = ev.unsigned.prev_content.as_ref();
            let new_display = ev.content.displayname.as_deref();
            let new_avatar = ev.content.avatar_url.as_ref().map(|u| u.to_string());
            let old_display = prev.and_then(|p| p.displayname.as_deref());
            let old_avatar = prev.and_then(|p| p.avatar_url.as_ref().map(|u| u.to_string()));

            if new_display == old_display && new_avatar == old_avatar {
                return;
            }

            let username = ev.state_key.localpart().to_string();
            let _ = event_tx.send(CoreEvent::System(SystemEvent::UserProfileChanged {
                username,
                display_name: ev.content.displayname.clone(),
                avatar_url: new_avatar,
            })).await;
        },
    );
}

pub async fn send_message(text: String, html_body: Option<String>, room_id_str: String, attachment_path: Option<String>, client: &Client) {
    let room_id = RoomId::parse(&room_id_str).unwrap();
    let room = client.get_room(&room_id).unwrap();

    match attachment_path {
        Some(path) => {
            let path = Path::new(&path);
            let data = match std::fs::read(path) {
                Ok(bytes) => bytes,
                Err(e) => {
                    log::error!("Failed to read attachment file: {:?}", e);
                    return;
                }
            };
            let filename = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("attachment");
            let content_type = sanitize_mime(path);

            if let Err(e) = room.send_attachment(filename, &content_type, data, AttachmentConfig::new()).await {
                log::error!("Failed to send attachment: {:?}", e);
            }
        }
        None => {
            let content = match html_body {
                Some(html) => RoomMessageEventContent::text_html(text, html),
                None => RoomMessageEventContent::text_plain(text),
            };
            if let Err(e) = room.send(content).await {
                log::error!("Failed to send message: {:?}", e);
            }
        }
    }
}
