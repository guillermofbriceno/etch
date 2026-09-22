use std::sync::Mutex;
use std::time::Duration;

use matrix_sdk::{
    Client, LoopCtrl, config::SyncSettings,
    ruma::events::StateEventType,
};
use serde_json::Value;
use matrix_sdk::deserialized_responses::RawAnySyncOrStrippedState;
use matrix_sdk::room::Room;
use tokio::sync::mpsc;

use crate::events::{InternalEvent, InternalMatrixEvent, SyncEnd};
use crate::matrix::retry::{SyncFailure, SyncRetryPolicy, SyncStep};
use crate::models::{RoomInfo, RoomType, VoiceServerConfig};

pub async fn build_room_info(room: &Room) -> anyhow::Result<RoomInfo> {
    let config = get_room_config(room).await?;
    let unread = room.unread_notification_counts();

    let avatar_url = match room.avatar_url() {
        Some(url) => Some(url.to_string()),
        None if matches!(config.room_type, RoomType::Dm) => {
            // DM rooms don't have a room-level avatar; use the other member's profile avatar.
            room.members(matrix_sdk::RoomMemberships::ACTIVE).await
                .ok()
                .and_then(|members| {
                    let own_id = room.own_user_id();
                    members.into_iter()
                        .find(|m| m.user_id() != own_id)
                        .and_then(|m| m.avatar_url().map(|u| u.to_string()))
                })
        }
        None => None,
    };

    Ok(RoomInfo {
        id: room.room_id().to_string(),
        display_name: room.display_name().await?.to_string(),
        etch_room_type: config.room_type,
        channel_id: config.channel_id,
        is_default: config.is_default,
        unread_count: unread.notification_count,
        is_encrypted: room.latest_encryption_state().await.map(|s| s.is_encrypted()).unwrap_or(false),
        avatar_url,
    })
}

pub async fn fetch_rooms(client: &Client) -> anyhow::Result<Vec<RoomInfo>> {
    let mut rooms_model: Vec<RoomInfo> = vec![];
    for room in client.joined_rooms() {
        rooms_model.push(build_room_info(&room).await?);
    }
    Ok(rooms_model)
}

/// Keep `client` syncing until the session is genuinely over, riding out the
/// failures that are not.
///
/// This is the *driver*: every decision it makes is `SyncRetryPolicy`'s, and
/// all it does is act on them -- wait, announce, carry on, or stop. What makes
/// that worth spelling out is what it replaced. `Client::sync` delegates to
/// `sync_with_callback`, whose callback only ever sees successful responses;
/// the first error propagates straight out of the call. So one dropped
/// 30-second long poll -- a laptop lid, a NAT timeout, a dropped route --
/// ended the sync loop for good, and the engine, having nothing better to go
/// on than "disconnected", tore the whole session down and cold-connected a
/// client whose token was never in doubt.
///
/// `sync_with_result_callback` hands the callback the `Result`, so an error
/// can be answered with `LoopCtrl::Continue` and the loop carries on from the
/// same `since` token. Nothing is torn down for a retry: the client, its
/// timelines, its subscriptions and its sqlite handles all stay exactly where
/// they are, and the UI keeps the data it is showing.
///
/// Returns only when the loop is over for good; `SyncEnd` says which kind of
/// over it is.
pub async fn sync_loop(
    client: Client,
    poll_timeout: Duration,
    internal_tx: mpsc::Sender<InternalEvent>,
) -> SyncEnd {
    log::debug!("Entering matrix sync loop (poll_timeout={poll_timeout:?})");

    // The SDK takes a `Fn` callback, so the policy and the verdict it reaches
    // have to live outside it behind a lock the callback borrows. Neither lock
    // is ever held across an await: the decision is taken, the guard dropped,
    // and only then does the driver wait or send. Shared references are copied
    // into each call's future, which is the shape the SDK's own example uses.
    let policy = Mutex::new(SyncRetryPolicy::new());
    let verdict: Mutex<Option<SyncEnd>> = Mutex::new(None);
    let (policy, verdict, tx) = (&policy, &verdict, &internal_tx);

    let result = client
        .sync_with_result_callback(SyncSettings::default().timeout(poll_timeout), |result| async move {
            let reason = result.as_ref().err().map(|e| format!("Sync error: {e}"));
            let failure = result.as_ref().err().map(SyncFailure::of);

            let step = {
                let mut policy = policy.lock().expect("sync retry policy lock");
                policy.observe(failure)
            };

            // Every arm but `Proceed` has a failure behind it, so the reason
            // is present wherever it is used. The fallback keeps that from
            // being an unwrap.
            let reason = move || reason.unwrap_or_else(|| "Sync error".to_string());

            match step {
                SyncStep::Proceed => Ok(LoopCtrl::Continue),

                SyncStep::Recovered => {
                    log::info!("Matrix sync recovered; the session was never dropped");
                    let _ = tx.send(InternalEvent::Matrix(
                        InternalMatrixEvent::SyncRecovered,
                    )).await;
                    Ok(LoopCtrl::Continue)
                }

                SyncStep::Retry { attempt, delay } => {
                    let reason = reason();
                    log::warn!(
                        "Matrix sync failed ({reason}); retry {attempt} of {} in {delay:?}, \
                         session left intact",
                        SyncRetryPolicy::MAX_RETRIES,
                    );
                    // Told once per degraded stretch, not once per retry: the
                    // engine only needs to know the connection went from fine
                    // to not, and a healthy client syncs every 30 seconds for
                    // as long as the app is open.
                    if attempt == 1 {
                        let _ = tx.send(InternalEvent::Matrix(
                            InternalMatrixEvent::SyncDegraded { reason },
                        )).await;
                    }
                    tokio::time::sleep(delay).await;
                    Ok(LoopCtrl::Continue)
                }

                SyncStep::GiveUp { failures } => {
                    let reason = format!(
                        "{} (gave up after {failures} consecutive sync failures)",
                        reason(),
                    );
                    log::error!("Matrix sync gave up retrying: {reason}");
                    *verdict.lock().expect("sync verdict lock") =
                        Some(SyncEnd::RetriesExhausted { reason });
                    Ok(LoopCtrl::Break)
                }

                SyncStep::SessionInvalidated => {
                    let reason = reason();
                    log::error!("The homeserver rejected our Matrix credentials: {reason}");
                    *verdict.lock().expect("sync verdict lock") =
                        Some(SyncEnd::SessionInvalidated { reason });
                    Ok(LoopCtrl::Break)
                }
            }
        })
        .await;

    let end = verdict.lock().expect("sync verdict lock").take().unwrap_or_else(|| {
        // `sync_stream` never ends of its own accord and the callback only
        // breaks after recording a verdict, so this is unreachable in
        // practice. It is reported as an exhausted retry rather than quietly
        // treated as success, because whatever happened the client is no
        // longer syncing and the reconnect path is what fixes that.
        let reason = match result {
            Ok(()) => "Sync loop ended without a verdict".to_string(),
            Err(e) => format!("Sync error: {e}"),
        };
        log::warn!("Matrix sync loop ended unexpectedly: {reason}");
        SyncEnd::RetriesExhausted { reason }
    });

    log::debug!("Exited matrix sync loop: {end:?}");
    end
}

struct RoomConfig {
    room_type: RoomType,
    channel_id: Option<u32>,
    is_default: bool,
}

async fn get_room_config(room: &Room) -> anyhow::Result<RoomConfig> {
    match room
        .get_state_event(StateEventType::from("etch.room_config"), "").await?
    {
        Some(raw_event) => {
            let raw_json = match raw_event {
                RawAnySyncOrStrippedState::Sync(e) => e.json().to_string(),
                RawAnySyncOrStrippedState::Stripped(e) => e.json().to_string(),
            };
            let json: Value = serde_json::from_str(&raw_json)?;
            let content = &json["content"];

            let room_type = match content["room_type"].as_str() {
                Some("voice") => RoomType::Voice,
                _ => RoomType::Text,
            };

            let channel_id = content["channel_id"].as_u64().map(|v| v as u32);
            let is_default = content["is_default"].as_bool().unwrap_or(false);

            Ok(RoomConfig { room_type, channel_id, is_default })
        }
        None => {
            let room_type = if room.is_direct().await.unwrap_or(false) {
                RoomType::Dm
            } else {
                RoomType::Text
            };
            Ok(RoomConfig { room_type, channel_id: None, is_default: false })
        }
    }
}

async fn get_voice_server_config(room: &Room) -> anyhow::Result<Option<VoiceServerConfig>> {
    match room
        .get_state_event(StateEventType::from("etch.voice_server"), "").await?
    {
        Some(raw_event) => {
            let raw_json = match raw_event {
                RawAnySyncOrStrippedState::Sync(e) => e.json().to_string(),
                RawAnySyncOrStrippedState::Stripped(e) => e.json().to_string(),
            };
            let json: Value = serde_json::from_str(&raw_json)?;
            let content = &json["content"];

            let host = content["host"].as_str()
                .ok_or_else(|| anyhow::anyhow!("etch.voice_server missing 'host' field"))?
                .to_string();
            let port = content["port"].as_u64().unwrap_or(64738) as u16;
            let password = content["password"].as_str().map(|s| s.to_string());

            Ok(Some(VoiceServerConfig { host, port, username: None, password }))
        }
        None => Ok(None),
    }
}

pub async fn find_voice_server(client: &Client, rooms: &[RoomInfo]) -> Option<VoiceServerConfig> {
    let default_room_info = rooms.iter().find(|r| r.is_default)?;
    let room_id = matrix_sdk::ruma::RoomId::parse(&default_room_info.id).ok()?;
    let room = client.get_room(&room_id)?;

    match get_voice_server_config(&room).await {
        Ok(config) => config,
        Err(e) => {
            log::warn!("Failed to read etch.voice_server from default room {}: {}", default_room_info.id, e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matrix::retry::SyncRetryPolicy;

    /// A client pointed at a port nothing is listening on. Every sync it
    /// attempts fails with a transport error -- exactly the failure that used
    /// to end the loop on its first occurrence -- without a server, a mock, or
    /// a dependency to stand one up.
    async fn client_with_nowhere_to_sync() -> Client {
        Client::builder()
            .homeserver_url("http://127.0.0.1:1")
            .build()
            .await
            .expect("client should build without contacting the server")
    }

    fn drain(rx: &mut mpsc::Receiver<InternalEvent>) -> Vec<InternalEvent> {
        let mut out = Vec::new();
        while let Ok(event) = rx.try_recv() {
            out.push(event);
        }
        out
    }

    /// The bug, at the level of the driver: a sync that fails has to be tried
    /// again, and again, up to the policy's ceiling -- not abandoned on the
    /// first error the way `Client::sync` abandons it.
    ///
    /// Runs on a paused clock, so the policy's real 122 seconds of backoff
    /// cost the test nothing. What is not faked is the failure: the requests
    /// are genuinely attempted and genuinely refused.
    #[tokio::test(start_paused = true)]
    async fn a_failing_sync_is_retried_to_the_ceiling_before_it_gives_up() {
        let client = client_with_nowhere_to_sync().await;
        let (tx, _rx) = mpsc::channel(16);

        let end = sync_loop(client, Duration::from_millis(1), tx).await;

        match &end {
            SyncEnd::RetriesExhausted { reason } => assert!(
                reason.contains(&format!(
                    "gave up after {} consecutive sync failures",
                    SyncRetryPolicy::MAX_RETRIES + 1,
                )),
                "the loop should have made every retry the policy allows, got {reason:?}",
            ),
            other => panic!("a transport failure is not a session invalidation, got {other:?}"),
        }
    }

    /// While retrying, the loop says so once and then stays quiet. It must
    /// never report a disconnect mid-retry: that is the event the engine turns
    /// into a teardown.
    #[tokio::test(start_paused = true)]
    async fn retrying_reports_degradation_once_and_nothing_else() {
        let client = client_with_nowhere_to_sync().await;
        let (tx, mut rx) = mpsc::channel(16);

        let end = sync_loop(client, Duration::from_millis(1), tx).await;
        let reported = drain(&mut rx);

        let degraded = reported.iter().filter(|e| matches!(
            e, InternalEvent::Matrix(InternalMatrixEvent::SyncDegraded { .. })
        )).count();
        assert_eq!(
            degraded, 1,
            "a degraded stretch is announced once, not once per retry; got {reported:?}",
        );

        assert!(
            !reported.iter().any(|e| matches!(
                e, InternalEvent::Matrix(InternalMatrixEvent::SyncRecovered)
            )),
            "nothing recovered here; got {reported:?}",
        );
        assert!(
            !reported.iter().any(|e| matches!(
                e, InternalEvent::Matrix(InternalMatrixEvent::Disconnected(_))
            )),
            "the driver reports the end by returning it, not on the channel; got {reported:?}",
        );

        // The verdict is still the return value, so the caller has exactly one
        // place to read it from.
        assert!(matches!(end, SyncEnd::RetriesExhausted { .. }));
    }
}
