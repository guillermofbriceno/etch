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

/// Keeps syncing through transient errors, which `Client::sync` would propagate and end
/// the loop on; returns only when `SyncEnd` says the session is over.
pub async fn sync_loop(
    client: Client,
    poll_timeout: Duration,
    internal_tx: mpsc::Sender<InternalEvent>,
    generation: u64,
) -> SyncEnd {
    log::debug!("Entering matrix sync loop (poll_timeout={poll_timeout:?})");

    // The SDK callback is `Fn`, so policy and verdict sit behind locks that are never
    // held across an await.
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

            let reason = move || reason.unwrap_or_else(|| "Sync error".to_string());

            match step {
                SyncStep::Proceed => Ok(LoopCtrl::Continue),

                SyncStep::Recovered => {
                    log::info!("Matrix sync recovered; the session was never dropped");
                    let _ = tx.send(InternalEvent::Matrix(
                        InternalMatrixEvent::SyncRecovered { generation },
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
                    // Once per degraded stretch, not per retry.
                    if attempt == 1 {
                        let _ = tx.send(InternalEvent::Matrix(
                            InternalMatrixEvent::SyncDegraded { generation, reason },
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
        // Unreachable in practice; reported as exhausted so the reconnect path repairs
        // a client that stopped syncing.
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
    use crate::matrix::retry::{SyncRetryPolicy, SyncStep};
    use crate::matrix::test_server::CannedHomeserver;
    use matrix_sdk::config::RequestConfig;
    use matrix_sdk::ruma::api::MatrixVersion;

    fn drain(rx: &mut mpsc::Receiver<InternalEvent>) -> Vec<InternalEvent> {
        let mut out = Vec::new();
        while let Ok(event) = rx.try_recv() {
            out.push(event);
        }
        out
    }

    fn policy_backoff_total() -> Duration {
        let mut policy = SyncRetryPolicy::new();
        (0..SyncRetryPolicy::MAX_RETRIES).map(|_| match policy.observe(Some(SyncFailure::Transient)) {
            SyncStep::Retry { delay, .. } => delay,
            other => panic!("expected a retry, got {other:?}"),
        }).sum()
    }

    /// Runs on a paused clock so the policy's real backoff costs nothing, with a client
    /// that fails before any network I/O, since I/O lets the paused clock jump ahead.
    #[tokio::test(start_paused = true)]
    async fn a_failing_sync_is_retried_with_backoff_to_the_ceiling_and_reported_degraded_once() {
        let client = Client::builder()
            .homeserver_url("http://127.0.0.1:1")
            .server_versions([MatrixVersion::V1_1])
            .request_config(RequestConfig::new().disable_retry())
            .build()
            .await
            .expect("client should build without contacting the server");
        let (tx, mut rx) = mpsc::channel(16);
        let started = tokio::time::Instant::now();

        let end = sync_loop(client, Duration::from_millis(1), tx, 7).await;
        let elapsed = started.elapsed();
        let reported = drain(&mut rx);

        match &end {
            SyncEnd::RetriesExhausted { reason } => assert!(
                reason.contains(&format!(
                    "gave up after {} consecutive sync failures",
                    SyncRetryPolicy::MAX_RETRIES + 1,
                )),
                "the loop should have made every retry the policy allows, got {reason:?}",
            ),
            other => panic!("a failure without an errcode is not a session invalidation, got {other:?}"),
        }
        assert!(
            elapsed >= policy_backoff_total(),
            "the loop should have waited out the policy's backoff, took {elapsed:?}",
        );

        let degraded = reported.iter().filter(|e| matches!(
            e, InternalEvent::Matrix(InternalMatrixEvent::SyncDegraded { generation: 7, .. })
        )).count();
        assert_eq!(
            degraded, 1,
            "a degraded stretch is announced once, not once per retry, under the session's \
             generation; got {reported:?}",
        );
        assert!(
            !reported.iter().any(|e| matches!(
                e, InternalEvent::Matrix(InternalMatrixEvent::SyncRecovered { .. })
            )),
            "nothing recovered here; got {reported:?}",
        );
        assert!(
            !reported.iter().any(|e| matches!(
                e, InternalEvent::Matrix(InternalMatrixEvent::Disconnected { .. })
            )),
            "the driver reports the end by returning it, not on the channel; got {reported:?}",
        );
    }

    /// On the real clock, because a paused one can time the request out before the
    /// server's answer arrives.
    #[tokio::test]
    async fn a_rejected_token_ends_the_sync_loop_without_retrying() {
        let server = CannedHomeserver::rejecting_the_token().await;
        let client = server.client_for("@alice:example.com").await;
        let (tx, mut rx) = mpsc::channel(16);

        let end = tokio::time::timeout(
            Duration::from_secs(10),
            sync_loop(client, Duration::from_millis(1), tx, 1),
        ).await.expect("a rejected token should end the loop, not wait out a retry");
        let reported = drain(&mut rx);

        assert!(
            matches!(end, SyncEnd::SessionInvalidated { .. }),
            "a rejected token means the session is over, got {end:?}",
        );
        assert_eq!(
            server.requests_to("/sync"), 1,
            "a rejected token must not be retried; the server saw {:?}", server.requests(),
        );
        assert!(
            !reported.iter().any(|e| matches!(
                e, InternalEvent::Matrix(InternalMatrixEvent::SyncDegraded { .. })
            )),
            "a rejected token is not a degraded connection; got {reported:?}",
        );
    }
}
