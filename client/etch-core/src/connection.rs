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
    /// Owned here rather than read from `state`, since `Connecting` reports zero
    /// retries and would flatten the backoff.
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

    pub async fn begin(&mut self, event_tx: &mpsc::Sender<CoreEvent>) {
        self.state = ConnectionState::Connecting;
        let _ = event_tx.send(matrix_conn_event(ConnectionState::Connecting)).await;
    }

    /// The user has left the server, so the form goes too: nothing may reconnect with it.
    pub async fn disconnect(&mut self, event_tx: &mpsc::Sender<CoreEvent>) {
        self.form = None;
        self.retries = 0;
        self.state = ConnectionState::Disconnected;
        let _ = event_tx.send(matrix_conn_event(ConnectionState::Disconnected)).await;
    }

    /// Only a `Connected` session can degrade; `Failed` arms the retry timer and a
    /// stale report must not overwrite it.
    pub async fn degraded(&mut self, event_tx: &mpsc::Sender<CoreEvent>) {
        if !matches!(self.state, ConnectionState::Connected) {
            log::debug!("Ignoring a sync degradation reported against {:?}", self.state);
            return;
        }
        self.state = ConnectionState::Connecting;
        let _ = event_tx.send(matrix_conn_event(ConnectionState::Connecting)).await;
    }

    pub async fn recovered(&mut self, event_tx: &mpsc::Sender<CoreEvent>) {
        if !matches!(self.state, ConnectionState::Connecting) {
            log::debug!("Ignoring a sync recovery reported against {:?}", self.state);
            return;
        }
        self.state = ConnectionState::Connected;
        let _ = event_tx.send(matrix_conn_event(ConnectionState::Connected)).await;
    }

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

    /// An attempt in flight is `Connecting`, which must not clobber the retry counter
    /// or the backoff never escalates.
    #[tokio::test]
    async fn consecutive_failures_escalate_backoff() {
        let (event_tx, mut event_rx) = mpsc::channel(100);
        let mut timer: Pin<Box<Sleep>> = Box::pin(sleep(Duration::from_secs(3600)));
        let mut conn = MatrixConnection::new();

        let mut observed = Vec::new();
        for _ in 0..4 {
            conn.begin(&event_tx).await;
            let mut announced = Vec::new();
            while let Ok(CoreEvent::Matrix(MatrixEvent::ConnectionState(s))) = event_rx.try_recv() {
                announced.push(s);
            }
            assert!(
                matches!(announced.last(), Some(ConnectionState::Connecting)),
                "the frontend must be told the attempt started, got {announced:?}",
            );

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

    #[tokio::test]
    async fn success_resets_backoff() {
        let (event_tx, _event_rx) = mpsc::channel(100);
        let mut timer: Pin<Box<Sleep>> = Box::pin(sleep(Duration::from_secs(3600)));
        let mut conn = MatrixConnection::new();

        for _ in 0..3 {
            conn.begin(&event_tx).await;
            conn.settle(ConnectOutcome::Failed, &mut timer, &event_tx).await;
        }
        assert!(
            matches!(conn.state, ConnectionState::Failed { retries: 3, .. }),
            "three failures should accumulate, got {:?}",
            conn.state,
        );

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

    /// A stale session's last event must not overwrite `Failed`, or the retry timer is
    /// disarmed for good.
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
}
