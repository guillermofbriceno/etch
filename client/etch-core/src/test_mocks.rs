use std::collections::VecDeque;
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
    /// Keep returning `connect_result` instead of failing after the first call.
    /// Needed by tests that drive more than one successful connection.
    pub repeat_connect_result: bool,
    pub profile_response: (Option<String>, Option<String>),
    pub media_response: Result<Vec<u8>, String>,
    /// Events sent through `internal_tx` during `connect()`.
    pub internal_events: Vec<InternalEvent>,
    /// When set, `connect()` waits on this before returning. Lets a test hold a
    /// connection in flight and observe whether the engine stays responsive.
    pub connect_gate: Option<oneshot::Receiver<()>>,
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
            repeat_connect_result: false,
            profile_response: (None, None),
            media_response: Ok(vec![0xDE, 0xAD]),
            internal_events: Vec::new(),
            connect_gate: None,
        }
    }

    pub fn with_connect_result(mut self, outcome: ConnectOutcome) -> Self {
        self.connect_result = outcome;
        self
    }

    /// Return `outcome` from every `connect()` call, not just the first.
    pub fn with_repeating_connect_result(mut self, outcome: ConnectOutcome) -> Self {
        self.connect_result = outcome;
        self.repeat_connect_result = true;
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

    pub fn with_connect_gate(mut self, gate: oneshot::Receiver<()>) -> Self {
        self.connect_gate = Some(gate);
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
        if let Some(gate) = self.connect_gate.take() {
            let _ = gate.await;
        }
        for event in self.internal_events.drain(..) {
            let _ = internal_tx.send(event).await;
        }
        if self.repeat_connect_result {
            return self.connect_result.clone();
        }
        std::mem::replace(&mut self.connect_result, ConnectOutcome::Failed)
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
    pub launched_channel_paths: Mutex<Vec<Option<String>>>,
    pub commands: Mutex<Vec<MumbleCommand>>,
    pub shutdown_count: Mutex<u32>,
    pub launch_error: Mutex<bool>,
}

pub struct MockVoice {
    pub state: Arc<MockVoiceState>,
    /// Event batches sent through `internal_tx` during successive `launch()` calls.
    launch_event_batches: VecDeque<Vec<InternalEvent>>,
    /// 1-based index of the `launch()` call that should fail, if any.
    failing_launch: Option<u32>,
    /// `launch()` calls made so far, counted whether they failed or not.
    launch_calls: u32,
}

impl MockVoice {
    pub fn new() -> Self {
        Self {
            state: Arc::new(MockVoiceState {
                launched_with: Mutex::new(Vec::new()),
                launched_channel_paths: Mutex::new(Vec::new()),
                commands: Mutex::new(Vec::new()),
                shutdown_count: Mutex::new(0),
                launch_error: Mutex::new(false),
            }),
            launch_event_batches: VecDeque::new(),
            failing_launch: None,
            launch_calls: 0,
        }
    }

    /// Queue a batch of events to emit during the next `launch()` call.
    /// Call multiple times to queue events for successive launches.
    pub fn with_internal_events(mut self, events: Vec<InternalEvent>) -> Self {
        self.launch_event_batches.push_back(events);
        self
    }

    /// Fail the `nth` `launch()` call (1-based) and no other. Lets a test put
    /// a launch failure in the middle of a run without racing the engine task
    /// for the `launch_error` flag.
    pub fn with_failing_launch(mut self, nth: u32) -> Self {
        self.failing_launch = Some(nth);
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
        channel_path: Option<&str>,
    ) -> Result<(), CoreError> {
        self.launch_calls += 1;
        if self.failing_launch == Some(self.launch_calls)
            || *self.state.launch_error.lock().unwrap()
        {
            return Err(CoreError::InvalidConfig { message: "mock launch failure".into() });
        }
        self.state.launched_with.lock().unwrap().push(creds);
        self.state.launched_channel_paths.lock().unwrap().push(channel_path.map(String::from));
        if let Some(events) = self.launch_event_batches.pop_front() {
            for event in events {
                let _ = internal_tx.send(event).await;
            }
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
