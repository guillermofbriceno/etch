use tokio::sync::mpsc;
use tokio::time::{Duration, Instant, Sleep};
use crate::events::{CoreEvent, InternalEvent, MatrixEvent};
use crate::commands::ServerConnectionForm;
use crate::models::ConnectionState;
use crate::matrix;

use std::pin::Pin;

pub(crate) fn matrix_conn_event(s: ConnectionState) -> CoreEvent {
    CoreEvent::Matrix(MatrixEvent::ConnectionState(s))
}

pub(crate) async fn schedule_retry(
    state: &mut ConnectionState,
    timer: &mut Pin<Box<Sleep>>,
    reason: String,
    event_tx: &mpsc::Sender<CoreEvent>,
    wrap: fn(ConnectionState) -> CoreEvent,
) {
    let new = state.next_failure(reason);
    if let ConnectionState::Failed { retry_in_secs, .. } = &new {
        timer.as_mut().reset(Instant::now() + Duration::from_secs(*retry_in_secs));
    }
    *state = new.clone();
    let _ = event_tx.send(wrap(new)).await;
}

pub(crate) async fn attempt_matrix_connect(
    state: &mut ConnectionState,
    timer: &mut Pin<Box<Sleep>>,
    service: &mut matrix::MatrixService,
    form: ServerConnectionForm,
    internal_tx: mpsc::Sender<InternalEvent>,
    event_tx: &mpsc::Sender<CoreEvent>,
) {
    *state = ConnectionState::Connecting;
    let _ = event_tx.send(matrix_conn_event(ConnectionState::Connecting)).await;

    if service.connect(form, internal_tx).await {
        *state = ConnectionState::Connected;
        let _ = event_tx.send(matrix_conn_event(ConnectionState::Connected)).await;
    } else {
        schedule_retry(state, timer, "Connection failed".into(), event_tx, matrix_conn_event).await;
    }
}
