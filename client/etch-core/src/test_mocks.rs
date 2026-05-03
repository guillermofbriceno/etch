use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

use crate::commands::{MatrixCommand, MumbleCommand, ServerConnectionForm};
use crate::error::CoreError;
use crate::events::InternalEvent;
use crate::models::{ConnectOutcome, VoiceServerConfig};
use crate::traits::{MatrixBackend, VoiceService};

/// Labels for trait methods called on the mock, recorded in call order.
/// Used to verify sequencing constraints (e.g., reset before connect).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MockCall {
    Reset,
    Connect,
}

/// Shared state for `MockMatrix`. Clone the `Arc` before moving the
/// mock into the engine so tests can inspect recorded calls afterward.
pub struct MockMatrixState {
    /// Commands dispatched via `handle_command`.
    pub commands: Mutex<Vec<MatrixCommand>>,
    /// Room IDs passed to `subscribe_to_room`.
    pub subscribe_calls: Mutex<Vec<String>>,
    /// Ordered log of trait method calls for sequencing assertions.
    pub call_log: Mutex<Vec<MockCall>>,
}

pub struct MockMatrix {
    pub state: Arc<MockMatrixState>,
    pub connect_result: ConnectOutcome,
    pub profile_response: (Option<String>, Option<String>),
    pub media_response: Result<Vec<u8>, String>,
    /// Events sent through `internal_tx` during `connect()`.
    pub internal_events: Vec<InternalEvent>,
}

impl MockMatrix {
    pub fn new() -> Self {
        Self {
            state: Arc::new(MockMatrixState {
                commands: Mutex::new(Vec::new()),
                subscribe_calls: Mutex::new(Vec::new()),
                call_log: Mutex::new(Vec::new()),
            }),
            connect_result: ConnectOutcome::Connected(None),
            profile_response: (None, None),
            media_response: Ok(vec![0xDE, 0xAD]),
            internal_events: Vec::new(),
        }
    }

    pub fn with_connect_result(mut self, outcome: ConnectOutcome) -> Self {
        self.connect_result = outcome;
        self
    }

    pub fn with_profile_response(mut self, display_name: Option<String>, avatar_url: Option<String>) -> Self {
        self.profile_response = (display_name, avatar_url);
        self
    }

    pub fn with_internal_events(mut self, events: Vec<InternalEvent>) -> Self {
        self.internal_events = events;
        self
    }
}

impl MatrixBackend for MockMatrix {
    async fn connect(
        &mut self,
        _form: ServerConnectionForm,
        internal_tx: mpsc::Sender<InternalEvent>,
    ) -> ConnectOutcome {
        self.state.call_log.lock().unwrap().push(MockCall::Connect);
        for event in self.internal_events.drain(..) {
            let _ = internal_tx.send(event).await;
        }
        self.connect_result.clone()
    }

    async fn handle_command(&mut self, cmd: MatrixCommand) {
        // std::sync::Mutex is correct here: the lock is never held across
        // an .await point, so it won't block the tokio runtime.
        self.state.commands.lock().unwrap().push(cmd);
    }

    async fn resolve_user_profile(
        &self,
        _username: &str,
    ) -> (Option<String>, Option<String>) {
        self.profile_response.clone()
    }

    fn spawn_media_fetch(
        &self,
        _mxc_url: String,
        respond: oneshot::Sender<Result<Vec<u8>, String>>,
    ) {
        let _ = respond.send(self.media_response.clone());
    }

    async fn subscribe_to_room(&mut self, room_id: &str) {
        self.state.subscribe_calls.lock().unwrap().push(room_id.to_string());
    }

    async fn reset(&mut self) {
        self.state.call_log.lock().unwrap().push(MockCall::Reset);
    }
}

/// Shared state for `MockVoice`. Clone the `Arc` before moving the
/// mock into the engine so tests can inspect recorded calls afterward.
pub struct MockVoiceState {
    pub launched_with: Mutex<Vec<VoiceServerConfig>>,
    pub commands: Mutex<Vec<MumbleCommand>>,
    pub shutdown_count: Mutex<u32>,
    pub launch_error: Mutex<bool>,
}

pub struct MockVoice {
    pub state: Arc<MockVoiceState>,
    /// Events sent through `internal_tx` during `launch()`.
    pub internal_events: Vec<InternalEvent>,
}

impl MockVoice {
    pub fn new() -> Self {
        Self {
            state: Arc::new(MockVoiceState {
                launched_with: Mutex::new(Vec::new()),
                commands: Mutex::new(Vec::new()),
                shutdown_count: Mutex::new(0),
                launch_error: Mutex::new(false),
            }),
            internal_events: Vec::new(),
        }
    }

    pub fn with_internal_events(mut self, events: Vec<InternalEvent>) -> Self {
        self.internal_events = events;
        self
    }
}

impl VoiceService for MockVoice {
    async fn launch(
        &mut self,
        creds: VoiceServerConfig,
        internal_tx: mpsc::Sender<InternalEvent>,
        _show_gui: bool,
        _extra_args: &str,
    ) -> Result<(), CoreError> {
        if *self.state.launch_error.lock().unwrap() {
            return Err(CoreError::InvalidConfig { message: "mock launch failure".into() });
        }
        self.state.launched_with.lock().unwrap().push(creds);
        for event in self.internal_events.drain(..) {
            let _ = internal_tx.send(event).await;
        }
        Ok(())
    }

    async fn send_command(&mut self, cmd: MumbleCommand) {
        self.state.commands.lock().unwrap().push(cmd);
    }

    async fn shutdown(&mut self) {
        *self.state.shutdown_count.lock().unwrap() += 1;
    }
}
