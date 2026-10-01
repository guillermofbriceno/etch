use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

use crate::commands::{MatrixCommand, MumbleCommand, ServerConnectionForm};
use crate::error::CoreError;
use crate::events::{InternalEvent, InternalMatrixEvent};
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
    pub repeat_connect_result: bool,
    pub profile_response: (Option<String>, Option<String>),
    pub media_response: Result<Vec<u8>, String>,
    /// Holds a connection in flight so a test can check the engine stays responsive.
    pub connect_gate: Option<oneshot::Receiver<()>>,
    /// Sent by the first connect before it returns, as a sync task it spawned would.
    pub reports_during_connect: Vec<fn(u64) -> InternalMatrixEvent>,
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
            connect_gate: None,
            reports_during_connect: Vec::new(),
        }
    }

    pub fn with_connect_result(mut self, outcome: ConnectOutcome) -> Self {
        self.connect_result = outcome;
        self
    }

    pub fn with_repeating_connect_result(mut self, outcome: ConnectOutcome) -> Self {
        self.connect_result = outcome;
        self.repeat_connect_result = true;
        self
    }

    pub fn with_profile_response(mut self, display_name: Option<String>, avatar_url: Option<String>) -> Self {
        self.profile_response = (display_name, avatar_url);
        self
    }

    pub fn with_connect_gate(mut self, gate: oneshot::Receiver<()>) -> Self {
        self.connect_gate = Some(gate);
        self
    }

    pub fn with_reports_during_connect(
        mut self,
        reports: Vec<fn(u64) -> InternalMatrixEvent>,
    ) -> Self {
        self.reports_during_connect = reports;
        self
    }
}

impl MatrixBackend for MockMatrix {
    async fn connect(
        &mut self,
        _form: ServerConnectionForm,
        internal_tx: mpsc::Sender<InternalEvent>,
        generation: u64,
    ) -> ConnectOutcome {
        self.state.call_log.lock().unwrap().push(MockCall::Connect);
        if let Some(gate) = self.connect_gate.take() {
            let _ = gate.await;
        }
        for report in std::mem::take(&mut self.reports_during_connect) {
            let _ = internal_tx.send(InternalEvent::Matrix(report(generation))).await;
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
    failing_launch: Option<u32>,
    launch_calls: u32,
    certs: HashMap<(String, u16), String>,
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
            certs: HashMap::new(),
        }
    }

    /// Queue a batch of events to emit during the next `launch()` call.
    /// Call multiple times to queue events for successive launches.
    pub fn with_internal_events(mut self, events: Vec<InternalEvent>) -> Self {
        self.launch_event_batches.push_back(events);
        self
    }

    pub fn with_failing_launch(mut self, nth: u32) -> Self {
        self.failing_launch = Some(nth);
        self
    }

    /// The fingerprint a probe of `host:port` returns; an unlisted server fails the probe.
    pub fn presenting_cert(mut self, host: &str, port: u16, fingerprint: &str) -> Self {
        self.certs.insert((host.to_string(), port), fingerprint.to_string());
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

    async fn probe_cert(&self, host: &str, port: u16) -> Result<String, CoreError> {
        self.certs.get(&(host.to_string(), port)).cloned().ok_or_else(|| CoreError::CertProbe {
            message: format!("mock has no cert for {host}:{port}"),
        })
    }

    async fn shutdown(&mut self) {
        *self.state.shutdown_count.lock().unwrap() += 1;
    }
}
