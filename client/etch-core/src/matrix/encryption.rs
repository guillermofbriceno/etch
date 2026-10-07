//! Whether this device can take part in encrypted rooms, and the recovery key that makes
//! a new device able to.

use std::fmt;

use futures_util::StreamExt;
use matrix_sdk::encryption::recovery::{RecoveryError, RecoveryState};
use matrix_sdk::encryption::secret_storage::SecretStorageError;
use matrix_sdk::encryption::{CrossSigningResetAuthType, VerificationState};
use matrix_sdk::ruma::api::client::error::ErrorKind;
use matrix_sdk::ruma::api::client::uiaa;
use matrix_sdk::{Client, HttpError};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::events::{CoreEvent, MatrixEvent};
use crate::task::AbortOnDrop;

const REQUEST_QUEUE: usize = 8;

const STATE_NOT_KNOWN: &str =
    "Etch does not know this account's encryption state yet. Try again in a moment.";
const DEVICE_NOT_VERIFIED: &str =
    "A recovery key can only be created on a device that can already read your encrypted messages.";
pub(crate) const WRONG_KEY: &str = "That recovery key is not correct. Check it and try again.";
const NO_RECOVERY: &str = "This account has no recovery key to enter.";
const BACKUP_BELONGS_ELSEWHERE: &str =
    "This account already has a message backup that this device cannot use. \
     Enter the recovery key that belongs to it, or reset encryption.";
pub(crate) const WRONG_PASSWORD: &str = "The password is not correct.";
const NEEDS_BROWSER_APPROVAL: &str =
    "This server asks for approval in a browser to reset encryption, which Etch cannot do.";
const UNREACHABLE: &str = "Could not reach the server. Check your connection and try again.";
pub(crate) const NOT_CONNECTED: &str = "Etch is not connected to the server.";
pub(crate) const BUSY: &str = "Etch is still working on the last request. Try again in a moment.";

/// A recovery key or a password, which must not reach a log through `{:?}`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    fn expose(&self) -> &str {
        &self.0
    }
}

impl From<String> for Secret {
    fn from(secret: String) -> Self {
        Self(secret)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}

/// What the user has to do, if anything, before this device can read and send in
/// encrypted rooms.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum EncryptionStatus {
    Unknown,
    Ready,
    /// The account has no recovery key that the user was able to save.
    NeedsRecoverySetup,
    /// Created but not yet confirmed as saved; nothing else holds the key.
    RecoveryKeyPending { key: Secret },
    NeedsRecoveryKey,
    /// Only another device or a reset can help: there is no recovery to enter a key for.
    NeedsVerifiedDevice,
}

/// What became of the last recovery key this device created.
#[derive(Debug, PartialEq)]
enum CreatedKey {
    /// None was created here, or the user confirmed having saved it.
    Settled,
    Shown(Secret),
    /// The app stopped before the user confirmed saving it, so the account's recovery has a key nobody holds.
    Lost,
}

impl EncryptionStatus {
    fn of(verification: VerificationState, recovery: RecoveryState, created: &CreatedKey) -> Self {
        use {RecoveryState as R, VerificationState as V};

        if let CreatedKey::Shown(key) = created {
            return Self::RecoveryKeyPending { key: key.clone() };
        }
        match (verification, recovery) {
            (V::Unknown, _) | (_, R::Unknown) => Self::Unknown,
            (_, R::Incomplete) | (V::Unverified, R::Enabled) => Self::NeedsRecoveryKey,
            (V::Verified, R::Enabled) if *created == CreatedKey::Lost => Self::NeedsRecoverySetup,
            (V::Verified, R::Enabled) => Self::Ready,
            (V::Verified, R::Disabled) => Self::NeedsRecoverySetup,
            (V::Unverified, R::Disabled) => Self::NeedsVerifiedDevice,
        }
    }
}

#[derive(Debug, PartialEq)]
enum KeyCreation {
    First,
    Replacement,
}

impl KeyCreation {
    /// Refused unless this device holds every secret, because a key created without them
    /// would replace a recovery that has them with one that does not.
    fn for_state(verification: VerificationState, recovery: RecoveryState) -> Result<Self, &'static str> {
        use {RecoveryState as R, VerificationState as V};

        match (verification, recovery) {
            (V::Unknown, _) | (_, R::Unknown) => Err(STATE_NOT_KNOWN),
            (V::Verified, R::Disabled) => Ok(Self::First),
            (V::Verified, R::Enabled) => Ok(Self::Replacement),
            _ => Err(DEVICE_NOT_VERIFIED),
        }
    }
}

#[derive(Clone, Copy)]
enum Action {
    CreateKey,
    EnterKey,
    Reset,
}

fn failure_reason(action: Action, error: &RecoveryError) -> String {
    let sdk_error = match error {
        RecoveryError::BackupExistsOnServer => return BACKUP_BELONGS_ELSEWHERE.into(),
        RecoveryError::SecretStorage(SecretStorageError::SecretStorageKey(_)) => return WRONG_KEY.into(),
        RecoveryError::SecretStorage(SecretStorageError::MissingKeyInfo { .. }) => return NO_RECOVERY.into(),
        RecoveryError::Sdk(e) | RecoveryError::SecretStorage(SecretStorageError::Sdk(e)) => Some(e),
        _ => None,
    };

    if let Some(e) = sdk_error {
        let refused = e.as_uiaa_response().is_some()
            || matches!(e.client_api_error_kind(), Some(ErrorKind::Forbidden { .. }));
        if matches!(action, Action::Reset) && refused {
            return WRONG_PASSWORD.into();
        }
        if let matrix_sdk::Error::Http(http) = e
            && matches!(**http, HttpError::Reqwest(_))
        {
            return UNREACHABLE.into();
        }
    }

    let what = match action {
        Action::CreateKey => "Creating the recovery key failed",
        Action::EnterKey => "The recovery key could not be used",
        Action::Reset => "Resetting encryption failed",
    };
    format!("{what}: {error}")
}

pub(crate) enum Request {
    Announce,
    CreateRecoveryKey,
    ConfirmRecoveryKeySaved,
    SubmitRecoveryKey(Secret),
    Reset { password: Secret },
}

/// One per Matrix client, owned by the session that owns the client, so a recovery key
/// that was created but not yet saved survives a reconnect.
/// Every request except `Announce` is answered once, with success or the reason it failed.
pub(crate) struct EncryptionWorker {
    requests: mpsc::Sender<Request>,
    _task: AbortOnDrop,
}

impl EncryptionWorker {
    pub(crate) fn spawn(client: Client, event_tx: mpsc::Sender<CoreEvent>) -> Self {
        let (requests, rx) = mpsc::channel(REQUEST_QUEUE);
        let task = AbortOnDrop::new(tokio::spawn(run(client, rx, event_tx)));
        Self { requests, _task: task }
    }

    /// Never waits; `false` means the worker did not take the request.
    pub(crate) fn ask(&self, request: Request) -> bool {
        match self.requests.try_send(request) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                log::warn!("Dropping an encryption request: the worker's queue is full");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                log::debug!("Dropping an encryption request: its worker has stopped");
                false
            }
        }
    }
}

async fn run(client: Client, mut requests: mpsc::Receiver<Request>, event_tx: mpsc::Sender<CoreEvent>) {
    let encryption = client.encryption();
    encryption.wait_for_e2ee_initialization_tasks().await;
    // A fresh login creates the identity, but a login cut short before that leaves a restored session without one.
    if let Err(e) = encryption.bootstrap_cross_signing_if_needed(None).await {
        log::warn!("Could not make sure the account has a cross-signing identity: {e}");
    }
    // The SDK only re-reads whether this device is verified after a key query, and a login signs the device after its last one.
    if let Some(user_id) = client.user_id()
        && let Err(e) = encryption.request_user_identity(user_id).await
    {
        log::warn!("Could not refresh this device's verification state: {e}");
    }

    let recovery = encryption.recovery();
    let mut verification_changes = encryption.verification_state();
    let mut recovery_changes = std::pin::pin!(recovery.state_stream().fuse());
    let mut created = if unsaved_key_marked(&client).await { CreatedKey::Lost } else { CreatedKey::Settled };
    let mut announced: Option<EncryptionStatus> = None;
    let mut answer: Option<Result<(), String>> = None;

    loop {
        let status = EncryptionStatus::of(verification_changes.get(), recovery.state(), &created);
        if announced.as_ref() != Some(&status) {
            log::info!("Encryption status is now {status:?}");
            let event = MatrixEvent::EncryptionStatus(status.clone());
            if event_tx.send(CoreEvent::Matrix(event)).await.is_err() {
                return;
            }
            announced = Some(status);
        }
        // After the status, so the frontend already holds the state its request led to.
        if let Some(outcome) = answer.take() {
            let event = match outcome {
                Ok(()) => MatrixEvent::EncryptionActionSucceeded,
                Err(reason) => {
                    log::warn!("An encryption request failed: {reason}");
                    MatrixEvent::EncryptionActionFailed { reason }
                }
            };
            if event_tx.send(CoreEvent::Matrix(event)).await.is_err() {
                return;
            }
        }

        tokio::select! {
            request = requests.recv() => {
                let Some(request) = request else { return };
                match request {
                    // A reconnect cleared the frontend's copy, so the status goes out again even if unchanged.
                    Request::Announce => announced = None,
                    Request::ConfirmRecoveryKeySaved => {
                        if matches!(created, CreatedKey::Shown(_)) {
                            mark_key_unsaved(&client, false).await;
                            created = CreatedKey::Settled;
                        }
                        answer = Some(Ok(()));
                    }
                    Request::CreateRecoveryKey => answer = Some(create_recovery_key(&client, &mut created).await),
                    Request::SubmitRecoveryKey(key) => {
                        let recovered = recovery.recover(key.expose().trim()).await;
                        answer = Some(recovered.map_err(|e| failure_reason(Action::EnterKey, &e)));
                    }
                    Request::Reset { password } => {
                        let outcome = reset(&client, &password).await;
                        if outcome.is_ok() {
                            // The reset deleted the recovery that key belonged to.
                            mark_key_unsaved(&client, false).await;
                            created = CreatedKey::Settled;
                        }
                        answer = Some(outcome);
                    }
                }
            },
            Some(_) = verification_changes.next() => {},
            Some(_) = recovery_changes.next() => {},
        }
    }
}

/// Kept in the login's own store, so it ends with the login it describes.
const UNSAVED_KEY_MARKER: &[u8] = b"etch.recovery_key_unsaved";

async fn unsaved_key_marked(client: &Client) -> bool {
    match client.state_store().get_custom_value(UNSAVED_KEY_MARKER).await {
        Ok(marker) => marker.is_some(),
        Err(e) => {
            log::warn!("Could not read whether the last recovery key was saved: {e}");
            false
        }
    }
}

async fn mark_key_unsaved(client: &Client, unsaved: bool) {
    let store = client.state_store();
    let written = if unsaved {
        store.set_custom_value_no_read(UNSAVED_KEY_MARKER, vec![1]).await
    } else {
        store.remove_custom_value(UNSAVED_KEY_MARKER).await.map(drop)
    };
    if let Err(e) = written {
        log::warn!("Could not record whether the recovery key was saved: {e}");
    }
}

async fn create_recovery_key(client: &Client, created: &mut CreatedKey) -> Result<(), String> {
    let encryption = client.encryption();
    let recovery = encryption.recovery();
    let creation = KeyCreation::for_state(encryption.verification_state().get(), recovery.state())?;
    // Marked before the key exists, so a run that ends in between cannot report a key nobody saw as saved.
    mark_key_unsaved(client, true).await;
    let key = match creation {
        KeyCreation::First => recovery.enable().await,
        KeyCreation::Replacement => recovery.reset_key().await,
    };
    match key {
        Ok(key) => {
            *created = CreatedKey::Shown(Secret::from(key));
            Ok(())
        }
        Err(e) => {
            if *created == CreatedKey::Settled {
                mark_key_unsaved(client, false).await;
            }
            Err(failure_reason(Action::CreateKey, &e))
        }
    }
}

async fn reset(client: &Client, password: &Secret) -> Result<(), String> {
    let failed = |e: RecoveryError| failure_reason(Action::Reset, &e);

    let Some(handle) = client.encryption().recovery().reset_identity().await.map_err(failed)? else {
        return Ok(());
    };
    match handle.auth_type() {
        CrossSigningResetAuthType::Uiaa(challenge) => {
            let user_id = client.user_id().map(ToString::to_string).unwrap_or_default();
            let mut auth = uiaa::Password::new(
                uiaa::UserIdentifier::UserIdOrLocalpart(user_id),
                password.expose().to_owned(),
            );
            auth.session = challenge.session.clone();
            handle.reset(Some(uiaa::AuthData::Password(auth))).await.map_err(failed)
        }
        CrossSigningResetAuthType::OAuth(_) => {
            handle.cancel().await;
            Err(NEEDS_BROWSER_APPROVAL.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matrix::test_server::CannedHomeserver;

    use RecoveryState as R;
    use VerificationState as V;

    fn key() -> Secret {
        Secret::from("EsTc recovery key".to_owned())
    }

    #[test]
    fn the_status_follows_whether_the_device_is_verified_and_recovery_is_set_up() {
        use EncryptionStatus as S;

        let table = [
            (V::Unknown, R::Unknown, S::Unknown),
            (V::Unknown, R::Enabled, S::Unknown),
            (V::Unknown, R::Incomplete, S::Unknown),
            (V::Verified, R::Unknown, S::Unknown),
            (V::Unverified, R::Unknown, S::Unknown),
            (V::Verified, R::Enabled, S::Ready),
            (V::Verified, R::Disabled, S::NeedsRecoverySetup),
            (V::Verified, R::Incomplete, S::NeedsRecoveryKey),
            (V::Unverified, R::Incomplete, S::NeedsRecoveryKey),
            (V::Unverified, R::Enabled, S::NeedsRecoveryKey),
            (V::Unverified, R::Disabled, S::NeedsVerifiedDevice),
        ];
        for (verification, recovery, expected) in table {
            assert_eq!(
                EncryptionStatus::of(verification, recovery, &CreatedKey::Settled), expected,
                "for a device that is {verification:?} with recovery {recovery:?}",
            );
        }

        for (verification, recovery) in [(V::Verified, R::Enabled), (V::Unknown, R::Unknown), (V::Unverified, R::Disabled)] {
            assert_eq!(
                EncryptionStatus::of(verification, recovery, &CreatedKey::Shown(key())),
                S::RecoveryKeyPending { key: key() },
                "a key nobody has confirmed saving must stay on screen whatever else changes",
            );
        }

        assert_eq!(
            EncryptionStatus::of(V::Verified, R::Enabled, &CreatedKey::Lost), S::NeedsRecoverySetup,
            "a key the app stopped showing before it was saved is not a key the user has",
        );
        assert_eq!(
            EncryptionStatus::of(V::Unverified, R::Enabled, &CreatedKey::Lost), S::NeedsRecoveryKey,
            "a lost key changes nothing for a device that could not have created one",
        );
    }

    #[test]
    fn a_recovery_key_is_only_created_where_every_secret_is_at_hand() {
        assert_eq!(KeyCreation::for_state(V::Verified, R::Disabled), Ok(KeyCreation::First));
        assert_eq!(KeyCreation::for_state(V::Verified, R::Enabled), Ok(KeyCreation::Replacement));

        for (verification, recovery) in [
            (V::Unverified, R::Disabled),
            (V::Unverified, R::Enabled),
            (V::Unverified, R::Incomplete),
            (V::Verified, R::Incomplete),
        ] {
            assert_eq!(
                KeyCreation::for_state(verification, recovery), Err(DEVICE_NOT_VERIFIED),
                "a device that is {verification:?} with recovery {recovery:?} would upload a recovery missing secrets",
            );
        }
        assert_eq!(KeyCreation::for_state(V::Unknown, R::Disabled), Err(STATE_NOT_KNOWN));
        assert_eq!(KeyCreation::for_state(V::Verified, R::Unknown), Err(STATE_NOT_KNOWN));
    }

    #[derive(Debug, PartialEq)]
    enum Report {
        Status(EncryptionStatus),
        Succeeded,
        Failed(String),
    }

    async fn next_report(events: &mut mpsc::Receiver<CoreEvent>) -> Report {
        let event = tokio::time::timeout(std::time::Duration::from_secs(10), events.recv()).await
            .expect("the worker should have reported something")
            .expect("the worker should still be running");
        match event {
            CoreEvent::Matrix(MatrixEvent::EncryptionStatus(status)) => Report::Status(status),
            CoreEvent::Matrix(MatrixEvent::EncryptionActionSucceeded) => Report::Succeeded,
            CoreEvent::Matrix(MatrixEvent::EncryptionActionFailed { reason }) => Report::Failed(reason),
            other => panic!("the worker reports nothing else, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_worker_answers_each_request_once_and_repeats_its_status_only_when_asked() {
        // The server reports no recovery for the account, and nothing that would show this device as signed.
        let server = CannedHomeserver::answering(|request| {
            if request.contains("/account_data/") {
                ("404 Not Found", r#"{"errcode":"M_NOT_FOUND","error":"Not found"}"#)
            } else {
                ("200 OK", "{}")
            }
        }).await;
        let (event_tx, mut events) = mpsc::channel(8);
        let worker = EncryptionWorker::spawn(server.client_for("@alice:example.com").await, event_tx);
        assert_eq!(next_report(&mut events).await, Report::Status(EncryptionStatus::NeedsVerifiedDevice));

        assert!(worker.ask(Request::Announce));
        assert_eq!(
            next_report(&mut events).await, Report::Status(EncryptionStatus::NeedsVerifiedDevice),
            "a reconnect clears the frontend's copy, so an unchanged status must be sent again",
        );

        assert!(worker.ask(Request::SubmitRecoveryKey(key())));
        assert_eq!(next_report(&mut events).await, Report::Failed(NO_RECOVERY.to_owned()));

        assert!(worker.ask(Request::CreateRecoveryKey));
        assert_eq!(
            next_report(&mut events).await, Report::Failed(DEVICE_NOT_VERIFIED.to_owned()),
            "a key created here would hold none of the account's secrets, and no status may come between two answers",
        );

        assert!(worker.ask(Request::ConfirmRecoveryKeySaved));
        assert_eq!(
            next_report(&mut events).await, Report::Succeeded,
            "a request that changed nothing is still answered, and with an answer, not a status",
        );
    }

    #[test]
    fn a_secret_does_not_appear_when_it_is_debug_formatted() {
        let status = EncryptionStatus::RecoveryKeyPending { key: key() };
        let event = CoreEvent::Matrix(MatrixEvent::EncryptionStatus(status));

        let printed = format!("{event:?} {:?}", key());

        assert!(!printed.contains("EsTc"), "the key leaked into {printed}");
        assert_eq!(
            serde_json::to_value(key()).unwrap(), serde_json::json!("EsTc recovery key"),
            "the frontend must still receive the key itself",
        );
    }
}
