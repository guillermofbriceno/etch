use tokio::sync::mpsc;
use tokio::time::{Duration, Instant, Sleep};
use crate::events::{CoreEvent, MatrixEvent};
use crate::commands::ServerConnectionForm;
use crate::models::{backoff_secs, ConnectOutcome, ConnectionState, VoiceServerConfig};

use std::pin::Pin;

fn matrix_conn_event(s: ConnectionState) -> CoreEvent {
    CoreEvent::Matrix(MatrixEvent::ConnectionState(s))
}

pub(crate) struct MatrixConnection {
    pub state: ConnectionState,
    pub form: Option<ServerConnectionForm>,
    /// Consecutive failed attempts, owned here rather than read back from
    /// `state`: an attempt in progress is `Connecting`, which would report
    /// zero and flatten the backoff. Cleared on a successful connection and
    /// whenever the user asks to connect.
    pub retries: u32,
}

impl MatrixConnection {
    pub fn new() -> Self {
        Self {
            state: ConnectionState::Disconnected,
            form: None,
            retries: 0,
        }
    }

    pub async fn schedule_retry(
        &mut self,
        timer: &mut Pin<Box<Sleep>>,
        reason: String,
        event_tx: &mpsc::Sender<CoreEvent>,
    ) {
        self.retries = self.retries.saturating_add(1);
        let retry_in_secs = backoff_secs(self.retries);
        timer.as_mut().reset(Instant::now() + Duration::from_secs(retry_in_secs));

        let new = ConnectionState::Failed { reason, retries: self.retries, retry_in_secs };
        self.state = new.clone();
        let _ = event_tx.send(matrix_conn_event(new)).await;
    }

    /// An attempt has been dispatched.
    ///
    /// `Connecting` is also the gate on starting another one: for as long as
    /// the state says an attempt is in flight, the retry timer stays disarmed
    /// and a fresh request supersedes rather than runs alongside.
    pub async fn begin(&mut self, event_tx: &mpsc::Sender<CoreEvent>) {
        self.state = ConnectionState::Connecting;
        let _ = event_tx.send(matrix_conn_event(ConnectionState::Connecting)).await;
    }

    /// The live session's sync loop is failing and retrying in place.
    ///
    /// Reported as `Connecting` because that is the vocabulary the frontend
    /// already has for "working on it, nothing to see yet", and because the
    /// alternative -- a new `ConnectionState` variant -- would change a
    /// contract the UI has no need to learn. Nothing else about the session
    /// moves: the retry machinery is untouched, because there is nothing here
    /// to retry from. The sync loop is doing the retrying.
    ///
    /// Only a `Connected` session can degrade. A report arriving against any
    /// other state belongs to a session that has already been superseded, and
    /// applying it would be actively harmful: `Failed` is what arms the retry
    /// timer, and overwriting it with `Connecting` would disarm it for good.
    pub async fn degraded(&mut self, event_tx: &mpsc::Sender<CoreEvent>) {
        if !matches!(self.state, ConnectionState::Connected) {
            log::debug!("Ignoring a sync degradation reported against {:?}", self.state);
            return;
        }
        self.state = ConnectionState::Connecting;
        let _ = event_tx.send(matrix_conn_event(ConnectionState::Connecting)).await;
    }

    /// The live session's sync loop is working again, without a reconnect.
    ///
    /// The counterpart of `degraded`, and guarded the same way: only a session
    /// that reads as `Connecting` has anything to recover from. The retry
    /// count is deliberately left alone -- no connection attempt failed, so
    /// there is no accumulated backoff for this to clear.
    pub async fn recovered(&mut self, event_tx: &mpsc::Sender<CoreEvent>) {
        if !matches!(self.state, ConnectionState::Connecting) {
            log::debug!("Ignoring a sync recovery reported against {:?}", self.state);
            return;
        }
        self.state = ConnectionState::Connected;
        let _ = event_tx.send(matrix_conn_event(ConnectionState::Connected)).await;
    }

    /// An attempt has come back. Returns the voice server it discovered, if it
    /// got far enough to discover one.
    pub async fn settle(
        &mut self,
        outcome: ConnectOutcome,
        timer: &mut Pin<Box<Sleep>>,
        event_tx: &mpsc::Sender<CoreEvent>,
    ) -> Option<VoiceServerConfig> {
        match outcome {
            ConnectOutcome::Connected(voice_server) => {
                self.retries = 0;
                self.state = ConnectionState::Connected;
                let _ = event_tx.send(matrix_conn_event(ConnectionState::Connected)).await;
                voice_server
            }
            ConnectOutcome::NeedsPassword => {
                // Don't retry; the user is being prompted for a password.
                self.state = ConnectionState::Disconnected;
                None
            }
            ConnectOutcome::Failed => {
                self.schedule_retry(timer, "Connection failed".into(), event_tx).await;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ConnectOutcome;
    use tokio::time::sleep;

    /// Consecutive failed connection attempts must escalate the backoff.
    /// An attempt in flight is `Connecting`, which reports zero retries; if
    /// that transition is allowed to clobber the counter the backoff is
    /// pinned at the first step forever and the client hammers the server
    /// every 2s indefinitely.
    #[tokio::test]
    async fn consecutive_failures_escalate_backoff() {
        let (event_tx, _event_rx) = mpsc::channel(100);
        let mut timer: Pin<Box<Sleep>> = Box::pin(sleep(Duration::from_secs(3600)));
        let mut conn = MatrixConnection::new();

        let mut observed = Vec::new();
        for _ in 0..4 {
            conn.begin(&event_tx).await;
            conn.settle(ConnectOutcome::Failed, &mut timer, &event_tx).await;
            match &conn.state {
                ConnectionState::Failed { retries, retry_in_secs, .. } => {
                    observed.push((*retries, *retry_in_secs));
                }
                other => panic!("expected Failed after a failed connect, got {:?}", other),
            }
        }

        assert_eq!(
            observed,
            vec![(1, 2), (2, 4), (3, 8), (4, 16)],
            "backoff must escalate across consecutive failures",
        );
    }

    /// A successful connection clears the accumulated backoff so the next
    /// disconnect starts over at the shortest delay.
    #[tokio::test]
    async fn success_resets_backoff() {
        let (event_tx, _event_rx) = mpsc::channel(100);
        let mut timer: Pin<Box<Sleep>> = Box::pin(sleep(Duration::from_secs(3600)));
        let mut conn = MatrixConnection::new();

        for _ in 0..3 {
            conn.begin(&event_tx).await;
            conn.settle(ConnectOutcome::Failed, &mut timer, &event_tx).await;
        }
        assert_eq!(conn.state.retries(), 3, "three failures should accumulate");

        conn.begin(&event_tx).await;
        conn.settle(ConnectOutcome::Connected(None), &mut timer, &event_tx).await;
        assert!(matches!(conn.state, ConnectionState::Connected));

        conn.begin(&event_tx).await;
        conn.settle(ConnectOutcome::Failed, &mut timer, &event_tx).await;
        assert!(
            matches!(conn.state, ConnectionState::Failed { retries: 1, retry_in_secs: 2, .. }),
            "backoff should restart after a successful connection, got {:?}",
            conn.state,
        );
    }

    /// A degraded sync loop is reported with the vocabulary the frontend
    /// already has, and recovering from it costs no reconnect. The retry count
    /// must survive both: no connection attempt failed, so there is no
    /// accumulated backoff for either to touch.
    #[tokio::test]
    async fn degrading_and_recovering_move_between_connected_and_connecting() {
        let (event_tx, mut event_rx) = mpsc::channel(100);
        let mut timer: Pin<Box<Sleep>> = Box::pin(sleep(Duration::from_secs(3600)));
        let mut conn = MatrixConnection::new();

        conn.begin(&event_tx).await;
        conn.settle(ConnectOutcome::Failed, &mut timer, &event_tx).await;
        conn.begin(&event_tx).await;
        conn.settle(ConnectOutcome::Connected(None), &mut timer, &event_tx).await;
        while event_rx.try_recv().is_ok() {}

        conn.degraded(&event_tx).await;
        assert!(matches!(conn.state, ConnectionState::Connecting), "got {:?}", conn.state);

        conn.recovered(&event_tx).await;
        assert!(matches!(conn.state, ConnectionState::Connected), "got {:?}", conn.state);

        let mut announced = Vec::new();
        while let Ok(CoreEvent::Matrix(MatrixEvent::ConnectionState(s))) = event_rx.try_recv() {
            announced.push(s);
        }
        assert!(
            matches!(
                announced.as_slice(),
                [ConnectionState::Connecting, ConnectionState::Connected],
            ),
            "the frontend should see the degradation and the recovery, got {announced:?}",
        );
    }

    /// A sync report from a session a newer connect has already replaced must
    /// not move the connection state. The sync task is aborted when the actor
    /// resets, but its last event can already be sitting in the engine's
    /// queue -- and `Failed` is what arms the retry timer, so letting a dead
    /// session overwrite it with `Connecting` would disarm the retry for good
    /// and leave the app permanently unable to reconnect.
    #[tokio::test]
    async fn a_stale_sync_report_cannot_disarm_the_retry() {
        let (event_tx, mut event_rx) = mpsc::channel(100);
        let mut timer: Pin<Box<Sleep>> = Box::pin(sleep(Duration::from_secs(3600)));
        let mut conn = MatrixConnection::new();

        conn.begin(&event_tx).await;
        conn.settle(ConnectOutcome::Failed, &mut timer, &event_tx).await;
        assert!(conn.state.is_failed(), "the retry timer is armed off this state");
        while event_rx.try_recv().is_ok() {}

        conn.degraded(&event_tx).await;
        assert!(
            conn.state.is_failed(),
            "a stale degradation must not disarm the retry, got {:?}",
            conn.state,
        );

        conn.recovered(&event_tx).await;
        assert!(
            conn.state.is_failed(),
            "nor may a stale recovery claim the connection is up, got {:?}",
            conn.state,
        );

        assert!(
            event_rx.try_recv().is_err(),
            "an ignored report must not reach the frontend either",
        );
    }

    /// `Connecting` is the gate the engine reads to decide whether an attempt
    /// is already in flight, so beginning one has to leave the state saying so
    /// -- and must not touch the accumulated retry count on the way.
    #[tokio::test]
    async fn beginning_an_attempt_reports_connecting_without_clearing_the_backoff() {
        let (event_tx, mut event_rx) = mpsc::channel(100);
        let mut timer: Pin<Box<Sleep>> = Box::pin(sleep(Duration::from_secs(3600)));
        let mut conn = MatrixConnection::new();

        conn.begin(&event_tx).await;
        conn.settle(ConnectOutcome::Failed, &mut timer, &event_tx).await;
        assert_eq!(conn.retries, 1);

        conn.begin(&event_tx).await;
        assert!(
            matches!(conn.state, ConnectionState::Connecting),
            "an attempt in flight must read as Connecting, got {:?}",
            conn.state,
        );
        assert_eq!(conn.retries, 1, "beginning an attempt must not clear the backoff");

        let mut announced = Vec::new();
        while let Ok(CoreEvent::Matrix(MatrixEvent::ConnectionState(s))) = event_rx.try_recv() {
            announced.push(s);
        }
        assert!(
            matches!(announced.last(), Some(ConnectionState::Connecting)),
            "the frontend must be told the attempt started, got {:?}",
            announced,
        );
    }
}
