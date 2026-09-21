use tokio::sync::mpsc;
use tokio::time::{Duration, Instant, Sleep};
use crate::events::{CoreEvent, InternalEvent, MatrixEvent};
use crate::commands::ServerConnectionForm;
use crate::models::{backoff_secs, ConnectOutcome, ConnectionState, VoiceServerConfig};
use crate::traits::MatrixBackend;

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

    pub async fn attempt_connect<M: MatrixBackend>(
        &mut self,
        timer: &mut Pin<Box<Sleep>>,
        service: &mut M,
        form: ServerConnectionForm,
        internal_tx: mpsc::Sender<InternalEvent>,
        event_tx: &mpsc::Sender<CoreEvent>,
    ) -> Option<VoiceServerConfig> {
        self.state = ConnectionState::Connecting;
        let _ = event_tx.send(matrix_conn_event(ConnectionState::Connecting)).await;

        match service.connect(form, internal_tx).await {
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
    use crate::test_mocks::MockMatrix;
    use tokio::time::sleep;

    fn test_form() -> ServerConnectionForm {
        ServerConnectionForm {
            username: "alice".into(),
            hostname: "example.com".into(),
            port: "8448".into(),
            password: None,
            mumble_host: None,
            mumble_port: None,
            mumble_username: None,
            mumble_password: None,
            homeserver_url: None,
        }
    }

    /// Consecutive failed connection attempts must escalate the backoff.
    /// `attempt_connect` moves through `Connecting`, which reports zero
    /// retries; if that transition is allowed to clobber the counter the
    /// backoff is pinned at the first step forever and the client hammers
    /// the server every 2s indefinitely.
    #[tokio::test]
    async fn consecutive_failures_escalate_backoff() {
        let (event_tx, _event_rx) = mpsc::channel(100);
        let (internal_tx, _internal_rx) = mpsc::channel(100);
        let mut timer: Pin<Box<Sleep>> = Box::pin(sleep(Duration::from_secs(3600)));
        let mut conn = MatrixConnection::new();

        let mut observed = Vec::new();
        for _ in 0..4 {
            let mut matrix = MockMatrix::new().with_connect_result(ConnectOutcome::Failed);
            conn.attempt_connect(
                &mut timer, &mut matrix, test_form(), internal_tx.clone(), &event_tx,
            ).await;
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
        let (internal_tx, _internal_rx) = mpsc::channel(100);
        let mut timer: Pin<Box<Sleep>> = Box::pin(sleep(Duration::from_secs(3600)));
        let mut conn = MatrixConnection::new();

        for _ in 0..3 {
            let mut matrix = MockMatrix::new().with_connect_result(ConnectOutcome::Failed);
            conn.attempt_connect(
                &mut timer, &mut matrix, test_form(), internal_tx.clone(), &event_tx,
            ).await;
        }
        assert_eq!(conn.state.retries(), 3, "three failures should accumulate");

        let mut matrix = MockMatrix::new().with_connect_result(ConnectOutcome::Connected(None));
        conn.attempt_connect(
            &mut timer, &mut matrix, test_form(), internal_tx.clone(), &event_tx,
        ).await;
        assert!(matches!(conn.state, ConnectionState::Connected));

        let mut matrix = MockMatrix::new().with_connect_result(ConnectOutcome::Failed);
        conn.attempt_connect(
            &mut timer, &mut matrix, test_form(), internal_tx, &event_tx,
        ).await;
        assert!(
            matches!(conn.state, ConnectionState::Failed { retries: 1, retry_in_secs: 2, .. }),
            "backoff should restart after a successful connection, got {:?}",
            conn.state,
        );
    }
}
