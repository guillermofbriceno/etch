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
    "A recovery key can only be created on a device that is already verified.";
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
    NeedsRecoverySetup,
    /// Created but not yet confirmed as saved; nothing else holds the key.
    RecoveryKeyPending { key: Secret },
    NeedsRecoveryKey,
    /// Only another device or a reset can help: there is no recovery to enter a key for.
    NeedsVerifiedDevice,
}

impl EncryptionStatus {
    fn of(verification: VerificationState, recovery: RecoveryState, pending_key: Option<&Secret>) -> Self {
        use {RecoveryState as R, VerificationState as V};

        if let Some(key) = pending_key {
            return Self::RecoveryKeyPending { key: key.clone() };
        }
        match (verification, recovery) {
            (V::Unknown, _) | (_, R::Unknown) => Self::Unknown,
            (_, R::Incomplete) | (V::Unverified, R::Enabled) => Self::NeedsRecoveryKey,
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
    let mut pending_key: Option<Secret> = None;
    let mut announced: Option<EncryptionStatus> = None;

    loop {
        let status = EncryptionStatus::of(verification_changes.get(), recovery.state(), pending_key.as_ref());
        if announced.as_ref() != Some(&status) {
            log::info!("Encryption status is now {status:?}");
            let event = MatrixEvent::EncryptionStatus(status.clone());
            if event_tx.send(CoreEvent::Matrix(event)).await.is_err() {
                return;
            }
            announced = Some(status);
        }

        let failure = tokio::select! {
            request = requests.recv() => match request {
                None => return,
                Some(Request::Announce) => {
                    announced = None;
                    None
                }
                Some(Request::ConfirmRecoveryKeySaved) => {
                    pending_key = None;
                    None
                }
                Some(Request::CreateRecoveryKey) => match create_recovery_key(&client).await {
                    Ok(key) => {
                        pending_key = Some(key);
                        None
                    }
                    Err(reason) => Some(reason),
                },
                Some(Request::SubmitRecoveryKey(key)) => recovery.recover(key.expose().trim()).await
                    .err()
                    .map(|e| failure_reason(Action::EnterKey, &e)),
                Some(Request::Reset { password }) => match reset(&client, &password).await {
                    Ok(()) => {
                        // It unlocked the recovery the reset just deleted.
                        pending_key = None;
                        None
                    }
                    Err(reason) => Some(reason),
                },
            },
            Some(_) = verification_changes.next() => None,
            Some(_) = recovery_changes.next() => None,
        };

        if let Some(reason) = failure {
            log::warn!("An encryption request failed: {reason}");
            let event = MatrixEvent::EncryptionActionFailed { reason };
            if event_tx.send(CoreEvent::Matrix(event)).await.is_err() {
                return;
            }
        }
    }
}

async fn create_recovery_key(client: &Client) -> Result<Secret, String> {
    let encryption = client.encryption();
    let recovery = encryption.recovery();
    let created = match KeyCreation::for_state(encryption.verification_state().get(), recovery.state())? {
        KeyCreation::First => recovery.enable().await,
        KeyCreation::Replacement => recovery.reset_key().await,
    };
    created.map(Secret::from).map_err(|e| failure_reason(Action::CreateKey, &e))
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
                EncryptionStatus::of(verification, recovery, None), expected,
                "for a device that is {verification:?} with recovery {recovery:?}",
            );
        }

        for (verification, recovery) in [(V::Verified, R::Enabled), (V::Unknown, R::Unknown), (V::Unverified, R::Disabled)] {
            assert_eq!(
                EncryptionStatus::of(verification, recovery, Some(&key())),
                S::RecoveryKeyPending { key: key() },
                "a key nobody has confirmed saving must stay on screen whatever else changes",
            );
        }
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

    /// The worker's next report: a status, or the reason a request failed.
    async fn next_report(events: &mut mpsc::Receiver<CoreEvent>) -> Result<EncryptionStatus, String> {
        let event = tokio::time::timeout(std::time::Duration::from_secs(10), events.recv()).await
            .expect("the worker should have reported something")
            .expect("the worker should still be running");
        match event {
            CoreEvent::Matrix(MatrixEvent::EncryptionStatus(status)) => Ok(status),
            CoreEvent::Matrix(MatrixEvent::EncryptionActionFailed { reason }) => Err(reason),
            other => panic!("the worker reports nothing else, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_worker_repeats_its_status_when_asked_and_says_why_a_request_failed() {
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
        assert_eq!(next_report(&mut events).await, Ok(EncryptionStatus::NeedsVerifiedDevice));

        assert!(worker.ask(Request::Announce));
        assert_eq!(
            next_report(&mut events).await, Ok(EncryptionStatus::NeedsVerifiedDevice),
            "a reconnect clears the frontend's copy, so an unchanged status must be sent again",
        );

        assert!(worker.ask(Request::SubmitRecoveryKey(key())));
        assert_eq!(next_report(&mut events).await, Err(NO_RECOVERY.to_owned()));

        assert!(worker.ask(Request::CreateRecoveryKey));
        assert_eq!(
            next_report(&mut events).await, Err(DEVICE_NOT_VERIFIED.to_owned()),
            "a key created here would hold none of the account's secrets",
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
