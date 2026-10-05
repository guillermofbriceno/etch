use anyhow::Context;
use matrix_sdk::{
    Client,
    encryption::{BackupDownloadStrategy, EncryptionSettings},
    ruma::{UserId,
        api::client::{keys::get_keys, uiaa},
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
use crate::matrix::saved_session::{self, FreshStore, SavedSession};
use crate::models::{RoomInfo, RoomType};
use std::path::Path;
// TODO: enable once keyring backend is configured
// use keyring::Entry;
// use rand::RngExt;
// use rand::distr::Alphanumeric;

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
    // TODO: use get_or_create_passphrase() once keyring backend is configured
    let builder = Client::builder()
        .sqlite_store(store_path, None)
        .with_encryption_settings(EncryptionSettings {
            backup_download_strategy: BackupDownloadStrategy::OneShot,
            ..Default::default()
        });
    match &form.homeserver_url {
        Some(url) => Ok(builder.homeserver_url(url).build().await?),
        None => Ok(builder
            .server_name_or_homeserver_url(format!("{}:{}", form.hostname, form.port))
            .build().await?),
    }
}

pub async fn start_matrix_client(tx: mpsc::Sender<InternalEvent>, event_tx: mpsc::Sender<CoreEvent>, conn_form: ServerConnectionForm, data_dir: &Path) -> anyhow::Result<ConnectionResult> {
    let user_id = UserId::parse(format!("@{}:{}", conn_form.username, conn_form.hostname))?;

    let server_dir = saved_session::server_dir(data_dir, &conn_form);
    std::fs::create_dir_all(&server_dir)?;

    match SavedSession::load(&server_dir) {
        Ok(Some(saved)) => {
            let client = build_matrix_client(&conn_form, saved.store_dir(&server_dir)).await?;
            match client.matrix_auth().restore_session(saved.session, RoomLoadSettings::default()).await {
                Ok(()) => {
                    register_event_handlers(&client, tx, event_tx);
                    return Ok(ConnectionResult::Ok(client));
                }
                Err(e) => log::warn!("Stale session for {}, starting fresh: {e}", conn_form.hostname),
            }
            forget_session(&server_dir);
        }
        Ok(None) => {}
        Err(e) => {
            log::warn!("Unreadable session for {}, starting fresh: {e}", conn_form.hostname);
            forget_session(&server_dir);
        }
    }

    start_fresh_login(user_id, conn_form, &server_dir, tx, event_tx).await
}

/// The store the session named is left where it is: a client may still have it open, so
/// it is removed at the next startup instead.
fn forget_session(server_dir: &Path) {
    let _ = std::fs::remove_file(saved_session::session_file(server_dir));
}

async fn start_fresh_login(
    user_id: matrix_sdk::ruma::OwnedUserId,
    conn_form: ServerConnectionForm,
    server_dir: &Path,
    tx: mpsc::Sender<InternalEvent>,
    event_tx: mpsc::Sender<CoreEvent>,
) -> anyhow::Result<ConnectionResult> {
    let Some(password) = conn_form.password.clone() else {
        return Ok(ConnectionResult::NeedsPassword);
    };

    // Declared before the client so the client is dropped, and its files closed, first.
    let mut store = FreshStore::create(server_dir)?;
    let client = build_matrix_client(&conn_form, store.path()).await?;

    if let Err(e) = log_in_and_save(&client, &user_id, &password, &store, server_dir).await {
        log_out_abandoned_login(&client).await;
        return Err(e);
    }
    store.keep();

    bootstrap_identity(&client, &user_id, password).await;

    register_event_handlers(&client, tx, event_tx);
    Ok(ConnectionResult::Ok(client))
}

async fn log_in_and_save(
    client: &Client,
    user_id: &UserId,
    password: &str,
    store: &FreshStore,
    server_dir: &Path,
) -> anyhow::Result<()> {
    client.matrix_auth()
        .login_username(user_id, password)
        .initial_device_display_name(&device_display_name())
        .send()
        .await?;
    let session = client.matrix_auth().session().context("the login left no session to save")?;
    SavedSession::new(session, store).save(server_dir)
}

fn device_display_name() -> String {
    let os = match std::env::consts::OS {
        "linux" => "Linux",
        "windows" => "Windows",
        "macos" => "macOS",
        other => other,
    };
    format!("Etch ({os})")
}

/// The engine retries a failed connect with the same password, so a login the server
/// accepted but that failed here would otherwise leave one more device behind per attempt.
async fn log_out_abandoned_login(client: &Client) {
    if client.access_token().is_none() {
        return;
    }
    match client.matrix_auth().logout().await {
        Ok(_) => log::info!("Logged out the device of a login that could not be completed"),
        Err(e) => log::warn!("Could not log out the device of a login that could not be completed: {e}"),
    }
}

/// Creates the account's cross-signing identity when it has none, and never replaces one.
async fn bootstrap_identity(client: &Client, user_id: &UserId, password: String) {
    let encryption = client.encryption();
    let Err(e) = encryption.bootstrap_cross_signing_if_needed(None).await else { return };
    let Some(challenge) = e.as_uiaa_response() else {
        log::error!("Cross-signing bootstrap failed (non-UIA error): {e}");
        return;
    };

    // A challenge can also mean another device created an identity in the meantime, which the retry would overwrite.
    match account_has_identity(client, user_id).await {
        Ok(false) => {}
        Ok(true) => {
            log::warn!("The account gained a cross-signing identity during login; leaving it in place");
            return;
        }
        Err(e) => {
            log::error!("Could not tell whether the account has a cross-signing identity ({e}); not uploading one");
            return;
        }
    }

    let mut password_auth = uiaa::Password::new(
        uiaa::UserIdentifier::UserIdOrLocalpart(user_id.to_string()),
        password,
    );
    password_auth.session = challenge.session.clone();
    // Not `_if_needed`: the challenged attempt already stored the new identity locally, so that would upload nothing.
    if let Err(e) = encryption.bootstrap_cross_signing(Some(uiaa::AuthData::Password(password_auth))).await {
        log::error!("Failed to bootstrap cross-signing with password auth: {e}");
    }
}

/// Asked of the server directly, because the SDK's own lookup would find the identity a
/// challenged upload left in the local store.
async fn account_has_identity(client: &Client, user_id: &UserId) -> matrix_sdk::HttpResult<bool> {
    let mut request = get_keys::v3::Request::new();
    request.device_keys.insert(user_id.to_owned(), Vec::new());
    Ok(client.send(request).await?.master_keys.contains_key(user_id))
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

pub async fn send_message(text: String, html_body: Option<String>, room_id_str: String, client: &Client) {
    let Ok(room_id) = RoomId::parse(&room_id_str) else {
        log::warn!("Not sending a message to an invalid room ID: {room_id_str}");
        return;
    };
    let Some(room) = client.get_room(&room_id) else {
        log::warn!("Not sending a message to room {room_id_str}: it is not known to this client");
        return;
    };

    let content = match html_body {
        Some(html) => RoomMessageEventContent::text_html(text, html),
        None => RoomMessageEventContent::text_plain(text),
    };
    if let Err(e) = room.send(content).await {
        log::error!("Failed to send message: {:?}", e);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::matrix::test_server::CannedHomeserver;

    /// Answers a login, and the key uploads that follow one, by request line.
    fn accept_login(request: &str) -> (&'static str, &'static str) {
        if request.contains("/client/versions") {
            ("200 OK", r#"{"versions":["v1.11"]}"#)
        } else if request.contains("/login") {
            ("200 OK", r#"{"user_id":"@alice:example.com","device_id":"FRESHDEVICE","access_token":"token"}"#)
        } else if request.contains("/keys/upload") {
            ("200 OK", r#"{"one_time_key_counts":{"signed_curve25519":50}}"#)
        } else {
            ("200 OK", "{}")
        }
    }

    fn form(server: &CannedHomeserver, password: Option<&str>) -> ServerConnectionForm {
        ServerConnectionForm {
            username: "alice".into(),
            hostname: "example.com".into(),
            port: "8448".into(),
            password: password.map(Into::into),
            mumble_host: None,
            mumble_port: None,
            mumble_username: None,
            mumble_password: None,
            homeserver_url: Some(server.url.clone()),
        }
    }

    async fn start(form: ServerConnectionForm, data_dir: &Path) -> anyhow::Result<ConnectionResult> {
        let (tx, _rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(1);
        start_matrix_client(tx, event_tx, form, data_dir).await
    }

    #[tokio::test]
    async fn a_login_that_cannot_be_saved_locally_is_logged_out_again() {
        let tmp = tempfile::tempdir().unwrap();
        let server = CannedHomeserver::answering(accept_login).await;
        let form = form(&server, Some("password"));
        // A directory where the session file belongs makes the save fail once the server has accepted the login.
        let server_dir = saved_session::server_dir(tmp.path(), &form);
        std::fs::create_dir_all(saved_session::session_file(&server_dir)).unwrap();

        let outcome = start(form, tmp.path()).await;

        assert!(outcome.is_err(), "a login that was not saved must not be handed back as a session");
        assert_eq!(server.requests_to("/login"), 1, "the server should have accepted one login");
        assert_eq!(
            server.requests_to("/logout"), 1,
            "the device that login registered must not be left behind",
        );
    }

    #[tokio::test]
    async fn a_saved_login_is_restored_from_its_own_store_on_the_next_start() {
        let tmp = tempfile::tempdir().unwrap();
        let server = CannedHomeserver::answering(accept_login).await;

        let Ok(ConnectionResult::Ok(first)) = start(form(&server, Some("password")), tmp.path()).await else {
            panic!("the canned server should accept the login");
        };
        let device_key = first.encryption().ed25519_key().await;
        drop(first);

        let Ok(ConnectionResult::Ok(restored)) = start(form(&server, None), tmp.path()).await else {
            panic!("the saved session should be restored without asking for the password");
        };

        assert_eq!(server.requests_to("/login"), 1, "restoring must not log in again");
        assert!(device_key.is_some(), "the login should have created the device's keys");
        assert_eq!(
            restored.encryption().ed25519_key().await, device_key,
            "the session must come back with the store it logged in with",
        );
    }

    /// How many times the identity is uploaded when the server challenges the first attempt.
    async fn identity_uploads_when_challenged(another_device_created_one: bool) -> usize {
        const CHALLENGE: &str = r#"{"flows":[{"stages":["m.login.password"]}],"params":{},"session":"challenge"}"#;
        const HAS_IDENTITY: &str = r#"{"master_keys":{"@alice:example.com":{}}}"#;

        let tmp = tempfile::tempdir().unwrap();
        let challenged = Arc::new(AtomicBool::new(false));
        let server = CannedHomeserver::answering({
            let challenged = challenged.clone();
            move |request| {
                if request.contains("/keys/device_signing/upload") {
                    if challenged.swap(true, Ordering::SeqCst) {
                        ("200 OK", "{}")
                    } else {
                        ("401 Unauthorized", CHALLENGE)
                    }
                } else if request.contains("/keys/query")
                    && another_device_created_one
                    && challenged.load(Ordering::SeqCst)
                {
                    ("200 OK", HAS_IDENTITY)
                } else {
                    accept_login(request)
                }
            }
        }).await;

        let outcome = start(form(&server, Some("password")), tmp.path()).await;

        assert!(matches!(outcome, Ok(ConnectionResult::Ok(_))), "the login itself should succeed either way");
        server.requests_to("/keys/device_signing/upload")
    }

    #[tokio::test]
    async fn a_challenged_identity_upload_is_retried_only_while_the_account_has_no_identity() {
        assert_eq!(
            identity_uploads_when_challenged(false).await, 2,
            "an account without an identity should get one once the password is supplied",
        );
        assert_eq!(
            identity_uploads_when_challenged(true).await, 1,
            "an identity another device created in the meantime must not be overwritten",
        );
    }
}
