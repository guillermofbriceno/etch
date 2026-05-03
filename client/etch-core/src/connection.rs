use tokio::sync::mpsc;
use tokio::time::{Duration, Instant, Sleep};
use crate::events::{CoreEvent, InternalEvent, MatrixEvent};
use crate::commands::ServerConnectionForm;
use crate::models::{ConnectOutcome, ConnectionState, VoiceServerConfig};
use crate::traits::MatrixBackend;

use std::pin::Pin;

fn matrix_conn_event(s: ConnectionState) -> CoreEvent {
    CoreEvent::Matrix(MatrixEvent::ConnectionState(s))
}

pub(crate) struct MatrixConnection {
    pub state: ConnectionState,
    pub form: Option<ServerConnectionForm>,
}

impl MatrixConnection {
    pub fn new() -> Self {
        Self {
            state: ConnectionState::Disconnected,
            form: None,
        }
    }

    pub async fn schedule_retry(
        &mut self,
        timer: &mut Pin<Box<Sleep>>,
        reason: String,
        event_tx: &mpsc::Sender<CoreEvent>,
    ) {
        let new = self.state.next_failure(reason);
        if let ConnectionState::Failed { retry_in_secs, .. } = &new {
            timer.as_mut().reset(Instant::now() + Duration::from_secs(*retry_in_secs));
        }
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
