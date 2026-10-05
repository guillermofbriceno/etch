//! Integration tests: real MatrixService, mock voice, disposable Docker servers.
//!
//! Run with: cargo test -p etch-core --features integration-tests -- --test-threads=1
//! Or via the orchestrator: ./tests/integration/run.sh

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::commands::{
    AttachmentSend, ChatMessageSend, CoreCommand, MatrixCommand, MediaRequest, OutgoingMediaInfo,
    ServerConnectionForm, SystemCommand,
};
use crate::engine::CoreEngine;
use crate::events::{CoreEvent, MatrixEvent, SystemEvent};
use crate::matrix::name_colors::{self, Fetched, NameColor, UserNameColor};
use crate::matrix::service::MatrixService;
use crate::matrix::timeline::{TimelineEntry, TimelineEntryKind};
use crate::models::{ConnectionState, MediaInfo, RoomInfo, RoomType};
use crate::scripting::ScriptDispatcher;
use crate::test_mocks::MockVoice;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

struct TestHarness {
    cmd_tx: mpsc::Sender<CoreCommand>,
    media_tx: mpsc::Sender<MediaRequest>,
    event_rx: mpsc::Receiver<CoreEvent>,
    engine_handle: tokio::task::JoinHandle<()>,
    // Held so the temp directory outlives the engine. Fields are dropped in
    // declaration order, so _data_dir is dropped after engine_handle.
    _data_dir: tempfile::TempDir,
}

/// Logs only `etch_core` targets to stderr; matrix-sdk is too chatty.
fn init_test_logging() {
    use std::sync::Once;
    static ONCE: Once = Once::new();

    struct StderrLog;
    impl log::Log for StderrLog {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.target().starts_with("etch_core")
        }
        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                eprintln!("[{} {}] {}", record.level(), record.target(), record.args());
            }
        }
        fn flush(&self) {}
    }

    ONCE.call_once(|| {
        if log::set_boxed_logger(Box::new(StderrLog)).is_ok() {
            log::set_max_level(log::LevelFilter::Info);
        }
    });
}

impl TestHarness {
    fn new() -> Self {
        init_test_logging();
        let data_dir = tempfile::tempdir().expect("failed to create temp dir");
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let (media_tx, media_rx) = mpsc::channel(256);
        let (event_tx, event_rx) = mpsc::channel(256);

        let settings = crate::settings::load(data_dir.path());
        let dispatcher = Arc::new(ScriptDispatcher::from_settings(&settings));
        let temp_files = crate::temp_files::TempFiles::new(data_dir.path().to_path_buf());
        let matrix = MatrixService::new(event_tx.clone(), data_dir.path().to_path_buf(), dispatcher, temp_files);
        let voice = MockVoice::new();
        let engine = CoreEngine::new(
            cmd_rx, media_rx, event_tx, matrix, voice,
            data_dir.path().to_path_buf(), settings,
        );
        let engine_handle = tokio::spawn(engine.run());

        Self { cmd_tx, media_tx, event_rx, engine_handle, _data_dir: data_dir }
    }

    async fn send(&self, cmd: CoreCommand) {
        self.cmd_tx.send(cmd).await.expect("engine already stopped");
    }

    /// Wait for an event matching `predicate`. Non-matching events are
    /// discarded (but tracked for diagnostics). Returns the value extracted
    /// by the predicate, or panics on timeout with a summary of all
    /// received events.
    async fn expect_event<F, T>(&mut self, predicate: F, dur: Duration) -> T
    where
        F: Fn(&CoreEvent) -> Option<T>,
    {
        let mut discarded: Vec<String> = Vec::new();
        let deadline = tokio::time::Instant::now() + dur;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                panic!(
                    "timed out waiting for expected event after {:?}\n\
                     events received but discarded ({}):\n  {}",
                    dur,
                    discarded.len(),
                    if discarded.is_empty() {
                        "(none -- no events arrived at all)".to_string()
                    } else {
                        discarded.join("\n  ")
                    },
                );
            }
            match timeout(remaining, self.event_rx.recv()).await {
                Ok(Some(event)) => {
                    if let Some(val) = predicate(&event) {
                        return val;
                    }
                    discarded.push(Self::summarize_event(&event));
                }
                Ok(None) => panic!(
                    "event channel closed before expected event arrived\n\
                     events received but discarded ({}):\n  {}",
                    discarded.len(),
                    discarded.join("\n  "),
                ),
                Err(_) => panic!(
                    "timed out waiting for expected event after {:?}\n\
                     events received but discarded ({}):\n  {}",
                    dur,
                    discarded.len(),
                    if discarded.is_empty() {
                        "(none -- no events arrived at all)".to_string()
                    } else {
                        discarded.join("\n  ")
                    },
                ),
            }
        }
    }

    /// One-line summary of a CoreEvent for diagnostic output.
    fn summarize_event(event: &CoreEvent) -> String {
        match event {
            CoreEvent::Matrix(m) => match m {
                MatrixEvent::TimelineAppend(rid, entries) =>
                    format!("TimelineAppend({}, {} entries)", rid, entries.len()),
                MatrixEvent::TimelinePushBack(rid, e) =>
                    format!("TimelinePushBack({}, {})", rid, Self::entry_summary(e)),
                MatrixEvent::TimelinePushFront(rid, e) =>
                    format!("TimelinePushFront({}, {})", rid, Self::entry_summary(e)),
                MatrixEvent::TimelineInsert(rid, idx, e) =>
                    format!("TimelineInsert({}, idx={}, {})", rid, idx, Self::entry_summary(e)),
                MatrixEvent::TimelineSet(rid, idx, e) =>
                    format!("TimelineSet({}, idx={}, {})", rid, idx, Self::entry_summary(e)),
                MatrixEvent::TimelineRemove(rid, idx) =>
                    format!("TimelineRemove({}, idx={})", rid, idx),
                MatrixEvent::TimelineCleared(rid) =>
                    format!("TimelineCleared({})", rid),
                MatrixEvent::TimelineReset(rid, entries) =>
                    format!("TimelineReset({}, {} entries)", rid, entries.len()),
                MatrixEvent::ConnectionState(s) =>
                    format!("ConnectionState({:?})", s),
                other => format!("{:?}", other),
            },
            other => format!("{:?}", other),
        }
    }

    fn entry_summary(entry: &crate::matrix::timeline::TimelineEntry) -> String {
        match &entry.kind {
            TimelineEntryKind::Message(msg) => {
                let body: String = msg.body.chars().take(60).collect();
                format!("Message(id={}, body={:?})", msg.id, body)
            }
            TimelineEntryKind::Redacted => "Redacted".into(),
            TimelineEntryKind::DayDivider(_) => "DayDivider".into(),
            TimelineEntryKind::ReadMarker => "ReadMarker".into(),
            TimelineEntryKind::StateEvent(s) => format!("StateEvent({:?})", s),
            TimelineEntryKind::Other => "Other".into(),
        }
    }

    /// Connect to the test server and wait for the channel list and
    /// Connected state. Returns the room list.
    async fn connect(&mut self) -> Vec<RoomInfo> {
        self.connect_as(test_connection_form()).await
    }

    async fn connect_as(&mut self, form: ServerConnectionForm) -> Vec<RoomInfo> {
        self.send(CoreCommand::System(SystemCommand::ConnectToServer(form))).await;

        let rooms: Vec<RoomInfo> = self.expect_event(|e| match e {
            CoreEvent::Matrix(MatrixEvent::ChannelList(rooms)) => Some(rooms.clone()),
            _ => None,
        }, CONNECT_TIMEOUT).await;

        self.expect_event(|e| match e {
            CoreEvent::Matrix(MatrixEvent::ConnectionState(ConnectionState::Connected)) => Some(()),
            _ => None,
        }, CONNECT_TIMEOUT).await;

        rooms
    }

    /// Find a room by display name in a list.
    fn find_room<'a>(rooms: &'a [RoomInfo], name: &str) -> &'a RoomInfo {
        rooms.iter()
            .find(|r| r.display_name == name)
            .unwrap_or_else(|| panic!("Room '{}' not found in channel list", name))
    }

    /// Send a message with a unique body and return the body string.
    async fn send_unique_message(&self, room_id: &str, prefix: &str) -> String {
        let unique_body = format!(
            "{}-{}",
            prefix,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        );
        self.send(CoreCommand::Matrix(MatrixCommand::SendMessage(ChatMessageSend {
            room_id: room_id.to_string(),
            text: unique_body.clone(),
            html_body: None,
        }))).await;
        unique_body
    }

    /// Wait for a message containing `body_target` in the given room's timeline.
    /// Returns the message's event ID.
    async fn expect_timeline_message(&mut self, room_id: &str, body_target: &str) -> String {
        let rid = room_id.to_string();
        let target = body_target.to_string();
        self.expect_event(move |e| {
            let (event_rid, entries) = match e {
                CoreEvent::Matrix(MatrixEvent::TimelineAppend(rid, entries)) =>
                    (rid, entries.as_slice()),
                CoreEvent::Matrix(MatrixEvent::TimelineReset(rid, entries)) =>
                    (rid, entries.as_slice()),
                CoreEvent::Matrix(MatrixEvent::TimelinePushBack(rid, entry))
                | CoreEvent::Matrix(MatrixEvent::TimelinePushFront(rid, entry))
                | CoreEvent::Matrix(MatrixEvent::TimelineInsert(rid, _, entry))
                | CoreEvent::Matrix(MatrixEvent::TimelineSet(rid, _, entry)) =>
                    (rid, std::slice::from_ref(entry)),
                _ => return None,
            };
            if event_rid != &rid { return None; }
            for entry in entries {
                if let TimelineEntryKind::Message(msg) = &entry.kind {
                    if msg.body.contains(&target) {
                        return Some(msg.id.clone());
                    }
                }
            }
            None
        }, EVENT_TIMEOUT).await
    }

    /// Waits for the copy the server accepted; the local echo before it has no event ID yet.
    async fn expect_attachment(&mut self, room_id: &str, file_name: &str) -> MediaInfo {
        let (room_id, file_name) = (room_id.to_string(), file_name.to_string());
        self.expect_event(move |e| {
            let (event_room, entries) = timeline_entries(e)?;
            if event_room != room_id { return None; }
            entries.iter().find_map(|entry| match &entry.kind {
                TimelineEntryKind::Message(msg) if msg.body == file_name && msg.id.starts_with('$') => {
                    msg.media.clone()
                }
                _ => None,
            })
        }, EVENT_TIMEOUT).await
    }

    async fn fetch_media(&self, mxc_url: &str) -> Vec<u8> {
        let (respond, response) = tokio::sync::oneshot::channel();
        self.media_tx.send(MediaRequest { mxc_url: mxc_url.to_string(), respond }).await
            .expect("engine already stopped");
        timeout(EVENT_TIMEOUT, response).await
            .expect("the media fetch was not answered in time")
            .expect("the media fetch was dropped")
            .unwrap_or_else(|e| panic!("{mxc_url} should download: {e}"))
    }

    /// Non-blocking drain: collect message bodies from any timeline events
    /// already buffered for `room_id`.
    fn drain_timeline_messages(&mut self, room_id: &str, out: &mut Vec<String>) {
        while let Ok(event) = self.event_rx.try_recv() {
            Self::collect_message_bodies(&event, room_id, out);
        }
    }

    /// Like `expect_event`, but also collects message bodies from timeline
    /// events for `room_id` while waiting for the predicate to match.
    async fn expect_event_collecting<F, T>(
        &mut self,
        room_id: &str,
        out: &mut Vec<String>,
        predicate: F,
        dur: Duration,
    ) -> T
    where
        F: Fn(&CoreEvent) -> Option<T>,
    {
        let deadline = tokio::time::Instant::now() + dur;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                panic!("timed out waiting for expected event");
            }
            match timeout(remaining, self.event_rx.recv()).await {
                Ok(Some(event)) => {
                    Self::collect_message_bodies(&event, room_id, out);
                    if let Some(val) = predicate(&event) {
                        return val;
                    }
                }
                Ok(None) => panic!("event channel closed before expected event arrived"),
                Err(_) => panic!("timed out waiting for expected event"),
            }
        }
    }

    /// Extract message bodies from a timeline event for the given room.
    fn collect_message_bodies(event: &CoreEvent, room_id: &str, out: &mut Vec<String>) {
        let (rid, entries) = match event {
            CoreEvent::Matrix(MatrixEvent::TimelineAppend(rid, entries)) =>
                (rid.as_str(), entries.as_slice()),
            CoreEvent::Matrix(MatrixEvent::TimelinePushBack(rid, entry)) =>
                (rid.as_str(), std::slice::from_ref(entry)),
            CoreEvent::Matrix(MatrixEvent::TimelinePushFront(rid, entry)) =>
                (rid.as_str(), std::slice::from_ref(entry)),
            CoreEvent::Matrix(MatrixEvent::TimelineInsert(rid, _, entry)) =>
                (rid.as_str(), std::slice::from_ref(entry)),
            CoreEvent::Matrix(MatrixEvent::TimelineSet(rid, _, entry)) =>
                (rid.as_str(), std::slice::from_ref(entry)),
            CoreEvent::Matrix(MatrixEvent::TimelineReset(rid, entries)) =>
                (rid.as_str(), entries.as_slice()),
            _ => return,
        };
        if rid != room_id { return; }
        for entry in entries {
            if let TimelineEntryKind::Message(msg) = &entry.kind {
                out.push(msg.body.clone());
            }
        }
    }

    /// Drop the command sender and wait for the engine to finish.
    async fn shutdown(self) {
        drop(self.cmd_tx);
        timeout(Duration::from_secs(10), self.engine_handle)
            .await
            .expect("engine did not shut down within 10s")
            .expect("engine task panicked");
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_connection_form() -> ServerConnectionForm {
    let url = std::env::var("ETCH_INTEG_MATRIX_URL")
        .unwrap_or_else(|_| "http://localhost:6167".into());
    let server_name = std::env::var("ETCH_INTEG_SERVER_NAME")
        .unwrap_or_else(|_| "localhost".into());

    ServerConnectionForm {
        username: "alice".into(),
        hostname: server_name,
        port: "6167".into(),
        password: Some("alice_password".into()),
        homeserver_url: Some(url),
        mumble_host: None,
        mumble_port: None,
        mumble_username: None,
        mumble_password: None,
    }
}

fn bob_connection_form() -> ServerConnectionForm {
    ServerConnectionForm {
        username: "bob".into(),
        password: Some("bob_password".into()),
        ..test_connection_form()
    }
}

fn timeline_entries(event: &CoreEvent) -> Option<(&str, &[TimelineEntry])> {
    match event {
        CoreEvent::Matrix(MatrixEvent::TimelineAppend(room_id, entries))
        | CoreEvent::Matrix(MatrixEvent::TimelineReset(room_id, entries)) => Some((room_id, entries)),
        CoreEvent::Matrix(MatrixEvent::TimelinePushBack(room_id, entry))
        | CoreEvent::Matrix(MatrixEvent::TimelinePushFront(room_id, entry))
        | CoreEvent::Matrix(MatrixEvent::TimelineInsert(room_id, _, entry))
        | CoreEvent::Matrix(MatrixEvent::TimelineSet(room_id, _, entry)) => {
            Some((room_id, std::slice::from_ref(entry)))
        }
        _ => None,
    }
}

/// A plain client for another provisioned user, to read profiles from outside the engine.
async fn logged_in_reader(username: &str, password: &str) -> matrix_sdk::Client {
    let form = test_connection_form();
    let client = matrix_sdk::Client::builder()
        .homeserver_url(form.homeserver_url.expect("the test form names the homeserver"))
        .build()
        .await
        .expect("reader client should build");
    client.matrix_auth().login_username(username, password).send().await
        .expect("reader should log in");
    client
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Read back by a second user, because the sender's own copy is served from its media cache.
#[tokio::test(flavor = "multi_thread")]
async fn an_attachment_reaches_another_user_intact_with_its_type_size_and_measurements() {
    use crate::matrix::compress::fixtures::{encoded, opaque};

    let mut bob = TestHarness::new();
    bob.connect_as(bob_connection_form()).await;
    let mut alice = TestHarness::new();
    let rooms = alice.connect().await;

    let files = tempfile::tempdir().unwrap();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let photo = encoded(&opaque(40, 30), image::ImageFormat::Png);
    let clip: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
    let measured = OutgoingMediaInfo { width: Some(1280), height: Some(720), duration_ms: Some(4_000) };

    for (room, name, bytes, media_info, (mimetype, width, height, duration)) in [
        ("Test Text", format!("photo-{stamp}.png"), photo, None, ("image/png", 40, 30, 0)),
        ("Encrypted Room", format!("clip-{stamp}.mp4"), clip, Some(measured), ("video/mp4", 1280, 720, 4_000)),
    ] {
        let room_id = TestHarness::find_room(&rooms, room).id.clone();
        let path = files.path().join(&name);
        std::fs::write(&path, &bytes).unwrap();

        alice.send(CoreCommand::Matrix(MatrixCommand::SendAttachment(AttachmentSend {
            room_id: room_id.clone(),
            path,
            compress: true,
            media_info,
        }))).await;

        let sent = alice.expect_attachment(&room_id, &name).await;
        let received = bob.expect_attachment(&room_id, &name).await;
        for (who, media) in [("its sender", &sent), ("another user", &received)] {
            assert_eq!(
                (media.mimetype.as_str(), media.size, media.width, media.height, media.duration),
                (mimetype, bytes.len() as u64, width, height, duration),
                "{name} in {room}, as {who} sees it",
            );
        }
        let downloaded = bob.fetch_media(&received.mxc_url).await;
        assert!(
            downloaded == bytes,
            "{name} in {room}: another user downloaded {} bytes that are not the {} that were sent",
            downloaded.len(), bytes.len(),
        );
    }

    alice.shutdown().await;
    bob.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_receives_channel_list() {
    let mut h = TestHarness::new();
    h.send(CoreCommand::System(SystemCommand::ConnectToServer(
        test_connection_form(),
    ))).await;

    // ChannelList is emitted before ConnectionState::Connected, so wait for
    // it directly rather than waiting for Connected first.
    let rooms: Vec<RoomInfo> = h.expect_event(|e| match e {
        CoreEvent::Matrix(MatrixEvent::ChannelList(rooms)) => Some(rooms.clone()),
        _ => None,
    }, CONNECT_TIMEOUT).await;

    assert!(rooms.len() >= 7, "Expected >= 7 provisioned rooms, got {}", rooms.len());
    let names: Vec<&str> = rooms.iter().map(|r| r.display_name.as_str()).collect();
    assert!(names.contains(&"Lobby"), "Missing 'Lobby' room. Got: {:?}", names);
    assert!(names.contains(&"Test Text"), "Missing 'Test Text' room. Got: {:?}", names);
    assert!(names.contains(&"Encrypted Room"), "Missing 'Encrypted Room'. Got: {:?}", names);

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_receives_current_user() {
    let mut h = TestHarness::new();
    h.send(CoreCommand::System(SystemCommand::ConnectToServer(
        test_connection_form(),
    ))).await;

    let server_name = std::env::var("ETCH_INTEG_SERVER_NAME")
        .unwrap_or_else(|_| "localhost".into());

    let (username, matrix_id) = h.expect_event(|e| match e {
        CoreEvent::Matrix(MatrixEvent::CurrentUser { username, matrix_id, .. }) =>
            Some((username.clone(), matrix_id.clone())),
        _ => None,
    }, CONNECT_TIMEOUT).await;

    assert_eq!(username, "alice");
    assert_eq!(matrix_id, format!("@alice:{server_name}"));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_wrong_password_fails() {
    let mut h = TestHarness::new();
    let mut form = test_connection_form();
    form.password = Some("wrong_password".into());

    h.send(CoreCommand::System(SystemCommand::ConnectToServer(form))).await;

    h.expect_event(|e| match e {
        CoreEvent::Matrix(MatrixEvent::ConnectionState(ConnectionState::Failed { .. })) => Some(()),
        _ => None,
    }, CONNECT_TIMEOUT).await;

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn send_message_appears_in_timeline() {
    let mut h = TestHarness::new();
    let rooms = h.connect().await;
    let room = TestHarness::find_room(&rooms, "Test Text");

    let body = h.send_unique_message(&room.id, "integ-test").await;
    h.expect_timeline_message(&room.id, &body).await;

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn encrypted_room_is_marked_encrypted() {
    let mut h = TestHarness::new();
    h.send(CoreCommand::System(SystemCommand::ConnectToServer(
        test_connection_form(),
    ))).await;

    let rooms: Vec<RoomInfo> = h.expect_event(|e| match e {
        CoreEvent::Matrix(MatrixEvent::ChannelList(rooms)) => Some(rooms.clone()),
        _ => None,
    }, CONNECT_TIMEOUT).await;

    let enc_room = rooms.iter()
        .find(|r| r.display_name == "Encrypted Room")
        .expect("'Encrypted Room' not found in channel list");
    assert!(enc_room.is_encrypted, "Room should be marked encrypted");

    // Verify the unencrypted room is not marked encrypted.
    let text_room = rooms.iter()
        .find(|r| r.display_name == "Test Text")
        .expect("'Test Text' room not found");
    assert!(!text_room.is_encrypted, "Text room should not be encrypted");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn send_message_in_encrypted_room() {
    let mut h = TestHarness::new();
    let rooms = h.connect().await;
    let room = TestHarness::find_room(&rooms, "Encrypted Room");

    let body = h.send_unique_message(&room.id, "encrypted-test").await;
    h.expect_timeline_message(&room.id, &body).await;

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn channel_list_has_correct_room_types() {
    let mut h = TestHarness::new();
    let rooms = h.connect().await;

    // Voice rooms should have channel_id set.
    let lobby = TestHarness::find_room(&rooms, "Lobby");
    assert!(matches!(lobby.etch_room_type, RoomType::Voice));
    assert_eq!(lobby.channel_id, Some(0));
    assert!(lobby.is_default, "Lobby should be the default room");

    let gd1 = TestHarness::find_room(&rooms, "General Discussion 1");
    assert!(matches!(gd1.etch_room_type, RoomType::Voice));
    assert_eq!(gd1.channel_id, Some(1));
    assert!(!gd1.is_default);

    // Text room should have no channel_id.
    let text = TestHarness::find_room(&rooms, "Test Text");
    assert!(matches!(text.etch_room_type, RoomType::Text));
    assert_eq!(text.channel_id, None);
    assert!(!text.is_default);

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn create_dm_produces_encrypted_room() {
    let mut h = TestHarness::new();
    h.connect().await;

    let server_name = std::env::var("ETCH_INTEG_SERVER_NAME")
        .unwrap_or_else(|_| "localhost".into());

    h.send(CoreCommand::Matrix(MatrixCommand::CreateDirectMessage {
        target_user_id: format!("@bob:{server_name}"),
    })).await;

    let dm_room = h.expect_event(|e| match e {
        CoreEvent::Matrix(MatrixEvent::DmCreated(room)) => Some(room.clone()),
        _ => None,
    }, EVENT_TIMEOUT).await;

    assert!(matches!(dm_room.etch_room_type, RoomType::Dm));
    assert!(dm_room.is_encrypted, "DMs should be encrypted by default");
    assert!(
        dm_room.display_name.starts_with("bob"),
        "DM display name should reference bob, got: {:?}",
        dm_room.display_name,
    );

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn backwards_pagination_loads_full_history() {
    // The provisioning script seeds 500 messages (seed-msg-0000 through
    // seed-msg-0499) into the "Test Text" room. The pagination page size
    // is 20. This test paginates backwards repeatedly until the server
    // reports no more history, collecting all message bodies, then verifies
    // the full sequence was received.
    //
    // We avoid the `connect()` helper here because it discards timeline
    // events while waiting for ChannelList/Connected. Instead we connect
    // manually and collect timeline messages throughout the entire flow.
    let mut h = TestHarness::new();
    let mut seen_bodies: Vec<String> = Vec::new();

    h.send(CoreCommand::System(SystemCommand::ConnectToServer(
        test_connection_form(),
    ))).await;

    // Wait for ChannelList, collecting timeline messages along the way.
    let rooms: Vec<RoomInfo> = h.expect_event_collecting(
        // We don't know the room ID yet, so collect from all rooms using
        // an empty string that won't match. We'll get the initial items
        // from the Connected wait below once we know the room ID.
        "",
        &mut Vec::new(),
        |e| match e {
            CoreEvent::Matrix(MatrixEvent::ChannelList(rooms)) => Some(rooms.clone()),
            _ => None,
        },
        CONNECT_TIMEOUT,
    ).await;

    let room = TestHarness::find_room(&rooms, "Test Text");
    let room_id = room.id.clone();

    // Wait for Connected, collecting timeline messages for our target room.
    h.expect_event_collecting(
        &room_id,
        &mut seen_bodies,
        |e| match e {
            CoreEvent::Matrix(MatrixEvent::ConnectionState(ConnectionState::Connected)) =>
                Some(()),
            _ => None,
        },
        CONNECT_TIMEOUT,
    ).await;

    // Give background pagination a moment to deliver items, then drain.
    tokio::time::sleep(Duration::from_millis(500)).await;
    h.drain_timeline_messages(&room_id, &mut seen_bodies);

    // Paginate until the server says there's no more history.
    // Timeline diff events arrive asynchronously (via a spawned task that
    // processes the SDK's VectorDiff stream), so they may lag behind
    // PaginationComplete. We collect what we can during pagination, then
    // drain remaining events after the loop.
    let mut pages = 0;
    loop {
        h.send(CoreCommand::Matrix(MatrixCommand::PaginateBackwards {
            room_id: room_id.clone(),
        })).await;

        let rid_clone = room_id.clone();
        let has_more = h.expect_event_collecting(
            &room_id,
            &mut seen_bodies,
            move |e| match e {
                CoreEvent::Matrix(MatrixEvent::PaginationComplete(rid, has_more))
                    if *rid == rid_clone => Some(*has_more),
                _ => None,
            },
            Duration::from_secs(30),
        ).await;

        pages += 1;

        if !has_more {
            break;
        }
    }

    // The diff stream task delivers timeline items asynchronously.
    // Drain remaining events until the stream is quiet.
    loop {
        match timeout(Duration::from_secs(2), h.event_rx.recv()).await {
            Ok(Some(event)) => {
                TestHarness::collect_message_bodies(&event, &room_id, &mut seen_bodies);
            }
            _ => break,
        }
    }

    // Extract only the seed messages and sort them to verify the full set.
    let mut seed_msgs: Vec<u32> = seen_bodies.iter()
        .filter_map(|b| b.strip_prefix("seed-msg-"))
        .filter_map(|n| n.parse().ok())
        .collect();
    seed_msgs.sort();
    seed_msgs.dedup();

    assert!(
        pages >= 2,
        "Expected multiple pagination pages, got {pages}",
    );
    assert_eq!(
        seed_msgs.len(), 500,
        "Expected 500 unique seed messages, got {}. Range: {:?}..={:?}",
        seed_msgs.len(),
        seed_msgs.first(),
        seed_msgs.last(),
    );
    assert_eq!(seed_msgs.first(), Some(&0));
    assert_eq!(seed_msgs.last(), Some(&499));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn set_display_name_emits_profile_change() {
    let mut h = TestHarness::new();
    h.connect().await;

    let new_name = format!(
        "Alice Test {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() % 10000,
    );

    h.send(CoreCommand::Matrix(
        MatrixCommand::SetDisplayName(new_name.clone()),
    )).await;

    h.expect_event(|e| match e {
        CoreEvent::System(SystemEvent::UserProfileChanged {
            username, display_name, ..
        }) if username == "alice" && display_name.as_deref() == Some(new_name.as_str()) =>
            Some(()),
        _ => None,
    }, EVENT_TIMEOUT).await;

    h.shutdown().await;
}

// Disabled: Conduwuit does not break /sync long-polls for m.reaction events
// sent by the same client, so the reaction echo never arrives within the test
// timeout. This is likely a Conduwuit bug (the watcher mechanism should notify
// the sync handler, but doesn't for reactions). Re-enable once fixed upstream.
// See also: https://spec.matrix.org/v1.10/client-server-api/#get_matrixclientv3sync
#[cfg(feature = "conduwuit-reaction-echo-fixed")]
#[tokio::test(flavor = "multi_thread")]
async fn toggle_reaction_updates_timeline() {
    let mut h = TestHarness::new();
    let rooms = h.connect().await;
    let room = TestHarness::find_room(&rooms, "Test Text");

    let body = h.send_unique_message(&room.id, "react-test").await;
    let event_id = h.expect_timeline_message(&room.id, &body).await;

    h.send(CoreCommand::Matrix(MatrixCommand::ToggleReaction {
        room_id: room.id.clone(),
        event_id,
        key: "\u{1f44d}".into(), // thumbs up
    })).await;

    // The reaction should appear as a timeline update with the emoji in
    // the message's reactions map.
    let rid = room.id.clone();
    h.expect_event(move |e| {
        let (event_rid, entries) = match e {
            CoreEvent::Matrix(MatrixEvent::TimelineAppend(rid, entries)) =>
                (rid, entries.as_slice()),
            CoreEvent::Matrix(MatrixEvent::TimelineReset(rid, entries)) =>
                (rid, entries.as_slice()),
            CoreEvent::Matrix(MatrixEvent::TimelinePushBack(rid, entry))
            | CoreEvent::Matrix(MatrixEvent::TimelinePushFront(rid, entry))
            | CoreEvent::Matrix(MatrixEvent::TimelineInsert(rid, _, entry))
            | CoreEvent::Matrix(MatrixEvent::TimelineSet(rid, _, entry)) =>
                (rid, std::slice::from_ref(entry)),
            _ => return None,
        };
        if event_rid != &rid { return None; }
        for entry in entries {
            if let TimelineEntryKind::Message(msg) = &entry.kind {
                if msg.body.contains(&body) && msg.reactions.contains_key("\u{1f44d}") {
                    return Some(());
                }
            }
        }
        None
    }, EVENT_TIMEOUT).await;

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn send_html_message_appears_in_timeline() {
    let mut h = TestHarness::new();
    let rooms = h.connect().await;
    let room = TestHarness::find_room(&rooms, "Test Text");

    let unique_tag = format!(
        "html-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    );
    let html_body = format!("<b>{}</b>", unique_tag);

    h.send(CoreCommand::Matrix(MatrixCommand::SendMessage(ChatMessageSend {
        room_id: room.id.clone(),
        text: unique_tag.clone(),
        html_body: Some(html_body),
    }))).await;

    h.expect_timeline_message(&room.id, &unique_tag).await;

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn create_duplicate_dm_reuses_room() {
    let mut h = TestHarness::new();
    h.connect().await;

    let server_name = std::env::var("ETCH_INTEG_SERVER_NAME")
        .unwrap_or_else(|_| "localhost".into());
    let target = format!("@bob:{server_name}");

    // Create the first DM.
    h.send(CoreCommand::Matrix(MatrixCommand::CreateDirectMessage {
        target_user_id: target.clone(),
    })).await;

    let first_dm = h.expect_event(|e| match e {
        CoreEvent::Matrix(MatrixEvent::DmCreated(room)) => Some(room.clone()),
        _ => None,
    }, EVENT_TIMEOUT).await;

    // Create a second DM with the same user.
    h.send(CoreCommand::Matrix(MatrixCommand::CreateDirectMessage {
        target_user_id: target.clone(),
    })).await;

    let second_dm = h.expect_event(|e| match e {
        CoreEvent::Matrix(MatrixEvent::DmCreated(room)) => Some(room.clone()),
        _ => None,
    }, EVENT_TIMEOUT).await;

    assert_eq!(
        first_dm.id, second_dm.id,
        "Creating a DM with the same user twice should reuse the existing room",
    );

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn enable_encryption_on_unencrypted_room() {
    let mut h = TestHarness::new();
    let rooms = h.connect().await;

    // Use "Plaintext Room" (a dedicated room for this test) rather than
    // "Test Text", because enabling encryption is a permanent server-side
    // mutation. Tests run alphabetically with --test-threads=1, so
    // mutating "Test Text" here would break encrypted_room_is_marked_encrypted
    // which asserts that "Test Text" is NOT encrypted.
    let room = TestHarness::find_room(&rooms, "Plaintext Room");
    assert!(!room.is_encrypted, "Pre-condition: Plaintext Room should start unencrypted");

    h.send(CoreCommand::Matrix(MatrixCommand::EnableEncryption {
        room_id: room.id.clone(),
    })).await;

    // After enabling encryption, verify the room is still functional by
    // sending a message and confirming it echoes back.
    let body = h.send_unique_message(&room.id, "post-encrypt").await;
    h.expect_timeline_message(&room.id, &body).await;

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn send_read_receipt_does_not_error() {
    let mut h = TestHarness::new();
    let rooms = h.connect().await;
    let room = TestHarness::find_room(&rooms, "Test Text");

    // Send a message so we have a known event ID.
    let body = h.send_unique_message(&room.id, "receipt-test").await;
    let event_id = h.expect_timeline_message(&room.id, &body).await;

    // Sending a read receipt should not crash or produce a system error.
    h.send(CoreCommand::Matrix(MatrixCommand::SendReadReceipt {
        room_id: room.id.clone(),
        event_id,
    })).await;

    // Verify the engine is still healthy by sending another message.
    let body2 = h.send_unique_message(&room.id, "after-receipt").await;
    h.expect_timeline_message(&room.id, &body2).await;

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn edit_message_is_processed_by_sdk() {
    let mut h = TestHarness::new();
    let rooms = h.connect().await;
    let room = TestHarness::find_room(&rooms, "Test Text");

    // Send a message and wait for it to appear. The initial event may be a
    // local echo with a transaction ID rather than a server-assigned event_id,
    // so we wait for a second emission (TimelineSet) that carries the real $-id.
    let original_body = h.send_unique_message(&room.id, "edit-test").await;
    h.expect_timeline_message(&room.id, &original_body).await;

    // Wait for the server-confirmed event_id (starts with $).
    let body_clone = original_body.clone();
    let event_id = h.expect_event(move |e| {
        let entry = match e {
            CoreEvent::Matrix(MatrixEvent::TimelineSet(_, _, entry)) => entry,
            CoreEvent::Matrix(MatrixEvent::TimelinePushBack(_, entry)) => entry,
            _ => return None,
        };
        if let TimelineEntryKind::Message(msg) = &entry.kind {
            if msg.body.contains(&body_clone) && msg.id.starts_with('$') {
                return Some(msg.id.clone());
            }
        }
        None
    }, EVENT_TIMEOUT).await;

    // Drain any residual events from the send confirmation flow so that
    // the next expect_event can only match events triggered by the edit.
    tokio::time::sleep(Duration::from_millis(500)).await;
    while h.event_rx.try_recv().is_ok() {}

    // Edit the message.
    h.send(CoreCommand::Matrix(MatrixCommand::EditMessage {
        room_id: room.id.clone(),
        event_id: event_id.clone(),
        text: format!("{}-edited", original_body),
        html_body: None,
    })).await;

    // The SDK processes the edit and emits timeline mutations (TimelineSet
    // or Remove+PushBack). The edited body may not appear until the server
    // confirms via /sync, but any re-emission of our event proves the SDK
    // accepted the edit.
    h.expect_event(move |e| {
        let entry = match e {
            CoreEvent::Matrix(MatrixEvent::TimelineSet(_, _, entry)) => entry,
            CoreEvent::Matrix(MatrixEvent::TimelinePushBack(_, entry)) => entry,
            _ => return None,
        };
        if let TimelineEntryKind::Message(msg) = &entry.kind {
            if msg.id == event_id {
                return Some(());
            }
        }
        None
    }, EVENT_TIMEOUT).await;

    // Verify the engine is still healthy.
    let body2 = h.send_unique_message(&room.id, "after-edit").await;
    h.expect_timeline_message(&room.id, &body2).await;

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn redact_message_removes_from_timeline() {
    let mut h = TestHarness::new();
    let rooms = h.connect().await;
    let room = TestHarness::find_room(&rooms, "Test Text");

    // Send a message and wait for it to appear.
    let body = h.send_unique_message(&room.id, "redact-test").await;
    let event_id = h.expect_timeline_message(&room.id, &body).await;

    // Redact the message.
    h.send(CoreCommand::Matrix(MatrixCommand::RedactMessage {
        room_id: room.id.clone(),
        event_id: event_id.clone(),
    })).await;

    // The SDK should emit either a TimelineSet with Redacted kind, a
    // TimelineRemove, or a TimelineSet/PushBack referencing the event_id
    // (the SDK processes redactions optimistically).
    let rid = room.id.clone();
    let eid = event_id.clone();
    h.expect_event(move |e| {
        match e {
            CoreEvent::Matrix(MatrixEvent::TimelineSet(_, _, entry)) => {
                if matches!(&entry.kind, TimelineEntryKind::Redacted) {
                    return Some(());
                }
                if let TimelineEntryKind::Message(msg) = &entry.kind {
                    if msg.id == eid {
                        return Some(());
                    }
                }
            }
            CoreEvent::Matrix(MatrixEvent::TimelineRemove(event_rid, _))
                if *event_rid == rid =>
            {
                return Some(());
            }
            _ => {}
        }
        None
    }, EVENT_TIMEOUT).await;

    // Verify the engine is still healthy.
    let body2 = h.send_unique_message(&room.id, "after-redact").await;
    h.expect_timeline_message(&room.id, &body2).await;

    h.shutdown().await;
}

#[cfg(target_os = "linux")]
fn open_fd_count() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .expect("failed to read /proc/self/fd")
        .count()
}

/// A background task left over from the previous connection pins the old client's
/// sqlite and HTTP pools.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn repeated_reconnects_do_not_leak_file_descriptors() {
    const WARMUP_CYCLES: usize = 2;
    const MEASURED_CYCLES: usize = 6;
    const MAX_FDS_PER_CYCLE: usize = 3;

    async fn settle() {
        tokio::time::sleep(Duration::from_secs(3)).await;
    }

    let mut h = TestHarness::new();

    for _ in 0..WARMUP_CYCLES {
        h.connect().await;
    }
    settle().await;
    let baseline = open_fd_count();

    let mut samples = Vec::new();
    for _ in 0..MEASURED_CYCLES {
        h.connect().await;
        settle().await;
        samples.push(open_fd_count());
    }

    let final_count = *samples.last().unwrap();
    let growth = final_count.saturating_sub(baseline);
    let budget = MAX_FDS_PER_CYCLE * MEASURED_CYCLES;

    println!(
        "fd baseline after {} warmup cycles: {}\nper-cycle samples: {:?}\ngrowth over {} cycles: {} (budget {})",
        WARMUP_CYCLES, baseline, samples, MEASURED_CYCLES, growth, budget,
    );
    h.shutdown().await;

    assert!(
        growth <= budget,
        "file descriptors grew by {} over {} reconnects (~{:.1}/cycle); \
         expected no sustained growth. baseline={}, samples={:?}",
        growth, MEASURED_CYCLES, growth as f64 / MEASURED_CYCLES as f64, baseline, samples,
    );
}


/// A leftover diff task from a previous subscription would deliver each message once per reconnect.
#[tokio::test(flavor = "multi_thread")]
async fn reconnecting_does_not_duplicate_timeline_events() {
    const RECONNECTS: usize = 2;

    async fn deliveries(h: &mut TestHarness, room_id: &str, prefix: &str) -> usize {
        let body = h.send_unique_message(room_id, prefix).await;
        h.expect_timeline_message(room_id, &body).await;

        tokio::time::sleep(Duration::from_secs(3)).await;
        let mut bodies = Vec::new();
        h.drain_timeline_messages(room_id, &mut bodies);

        1 + bodies.iter().filter(|b| b.contains(&body)).count()
    }

    let mut h = TestHarness::new();
    let rooms = h.connect().await;
    let room_id = TestHarness::find_room(&rooms, "Test Text").id.clone();

    let baseline = deliveries(&mut h, &room_id, "baseline").await;

    for _ in 0..RECONNECTS {
        h.connect().await;
    }
    let after_reconnects = deliveries(&mut h, &room_id, "reconnected").await;

    h.shutdown().await;

    assert_eq!(
        after_reconnects, baseline,
        "a message was delivered {} times after {} reconnects but {} times before; \
         each reconnect left its predecessor's timeline subscription running",
        after_reconnects, RECONNECTS, baseline,
    );
}

/// The DM is created with admin so no earlier test has made it and the create path runs.
#[tokio::test(flavor = "multi_thread")]
async fn reconnecting_does_not_duplicate_events_in_a_created_dm() {
    async fn deliveries(h: &mut TestHarness, room_id: &str, prefix: &str) -> usize {
        let body = h.send_unique_message(room_id, prefix).await;
        h.expect_timeline_message(room_id, &body).await;

        tokio::time::sleep(Duration::from_secs(3)).await;
        let mut bodies = Vec::new();
        h.drain_timeline_messages(room_id, &mut bodies);

        1 + bodies.iter().filter(|b| b.contains(&body)).count()
    }

    let mut h = TestHarness::new();
    h.connect().await;

    let server_name = std::env::var("ETCH_INTEG_SERVER_NAME")
        .unwrap_or_else(|_| "localhost".into());
    h.send(CoreCommand::Matrix(MatrixCommand::CreateDirectMessage {
        target_user_id: format!("@admin:{server_name}"),
    })).await;
    let dm = h.expect_event(|e| match e {
        CoreEvent::Matrix(MatrixEvent::DmCreated(room)) => Some(room.clone()),
        _ => None,
    }, EVENT_TIMEOUT).await;

    let baseline = deliveries(&mut h, &dm.id, "dm-baseline").await;
    h.connect().await;
    let after_reconnect = deliveries(&mut h, &dm.id, "dm-reconnected").await;

    h.shutdown().await;

    assert_eq!(
        after_reconnect, baseline,
        "a message in a DM created this session was delivered {after_reconnect} times after a \
         reconnect but {baseline} before; the DM has more than one timeline stream",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn name_color_round_trips_through_the_profile() {
    let mut h = TestHarness::new();
    let server_name = std::env::var("ETCH_INTEG_SERVER_NAME")
        .unwrap_or_else(|_| "localhost".into());
    let alice = format!("@alice:{server_name}");
    let blue: NameColor = serde_json::from_value(serde_json::json!({ "color": "#62BAF7" })).unwrap();

    h.send(CoreCommand::System(SystemCommand::ConnectToServer(
        test_connection_form(),
    ))).await;
    let settable = h.expect_event(|e| match e {
        CoreEvent::Matrix(MatrixEvent::Capabilities { name_color }) => Some(*name_color),
        _ => None,
    }, CONNECT_TIMEOUT).await;
    assert!(settable, "the test homeserver supports extended profiles, so the color is settable");
    h.expect_event(|e| match e {
        CoreEvent::Matrix(MatrixEvent::ConnectionState(ConnectionState::Connected)) => Some(()),
        _ => None,
    }, CONNECT_TIMEOUT).await;

    let reader = logged_in_reader("bob", "bob_password").await;

    h.send(CoreCommand::Matrix(MatrixCommand::SetNameColor(Some(blue)))).await;
    let set = UserNameColor { user_id: alice.clone(), color: Some(blue) };
    h.expect_event(|e| match e {
        CoreEvent::Matrix(MatrixEvent::NameColors(answers)) if answers.contains(&set) => Some(()),
        _ => None,
    }, EVENT_TIMEOUT).await;
    assert_eq!(
        name_colors::fetch(reader.clone(), alice.clone()).await,
        Fetched::Value(Some(blue)),
        "the color should read back",
    );
    let stored = reader.account()
        .fetch_profile_field_of(matrix_sdk::ruma::UserId::parse(&alice).unwrap(), name_colors::field_name())
        .await
        .expect("the profile field should be readable")
        .expect("the profile field should be set");
    assert_eq!(*stored.value(), serde_json::json!({ "color": "#62baf7" }), "the profile holds lowercase hex");

    h.send(CoreCommand::Matrix(MatrixCommand::SetNameColor(None))).await;
    let cleared = UserNameColor { user_id: alice.clone(), color: None };
    h.expect_event(|e| match e {
        CoreEvent::Matrix(MatrixEvent::NameColors(answers)) if answers.contains(&cleared) => Some(()),
        _ => None,
    }, EVENT_TIMEOUT).await;
    assert_eq!(
        name_colors::fetch(reader.clone(), alice.clone()).await,
        Fetched::Value(None),
        "a cleared color should read as none, as the server's answer rather than a failed fetch",
    );

    h.shutdown().await;
}

/// A user no earlier test has logged in as, so the account starts without an identity or devices.
async fn register_fresh_user(prefix: &str) -> ServerConnectionForm {
    use matrix_sdk::ruma::api::client::{account::register::v3::Request, uiaa};

    let form = test_connection_form();
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock should be past 1970");
    let username = format!("{prefix}{}", since_epoch.as_millis());
    let password = "fresh_user_password";

    let client = matrix_sdk::Client::builder()
        .homeserver_url(form.homeserver_url.as_deref().expect("the test form names the homeserver"))
        .build()
        .await
        .expect("registration client should build");
    let mut request = Request::new();
    request.username = Some(username.clone());
    request.password = Some(password.into());
    request.inhibit_login = true;
    request.auth = Some(uiaa::AuthData::RegistrationToken(uiaa::RegistrationToken::new("devtoken".into())));
    client.matrix_auth().register(request).await.expect("the test homeserver should register a new user");

    ServerConnectionForm { username, password: Some(password.into()), ..form }
}

/// The account's master key and each device with whether the identity has signed it, as
/// another user's client judges them.
async fn identity_seen_by(
    onlooker: &matrix_sdk::Client,
    owner: &matrix_sdk::ruma::UserId,
) -> (Option<String>, Vec<(String, bool)>) {
    let identity = onlooker.encryption().request_user_identity(owner).await
        .expect("the key query should succeed");
    let master_key = identity.and_then(|i| i.master_key().get_first_key().map(|key| key.to_base64()));
    let devices = onlooker.encryption().get_user_devices(owner).await
        .expect("the owner's devices should be known after a key query");
    let mut devices: Vec<(String, bool)> = devices.devices()
        .map(|device| (device.device_id().to_string(), device.is_cross_signed_by_owner()))
        .collect();
    devices.sort();
    (master_key, devices)
}

async fn device_ids(client: &matrix_sdk::Client) -> Vec<matrix_sdk::ruma::OwnedDeviceId> {
    let mut ids: Vec<_> = client.devices().await.expect("the device list should be readable")
        .devices.into_iter().map(|device| device.device_id).collect();
    ids.sort();
    ids
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_login_leaves_the_identity_alone_and_the_first_device_verified() {
    let form = register_fresh_user("identity").await;
    let owner = matrix_sdk::ruma::UserId::parse(format!("@{}:{}", form.username, form.hostname)).unwrap();
    let onlooker = logged_in_reader("admin", "admin_password").await;

    let mut first = TestHarness::new();
    first.connect_as(form.clone()).await;
    let (identity, devices) = identity_seen_by(&onlooker, &owner).await;
    let identity = identity.expect("the first login should have created the account's identity");
    let [(first_device, true)] = devices.as_slice() else {
        panic!("the first login should be the one device, signed by the identity, got {devices:?}");
    };

    let mut second = TestHarness::new();
    second.connect_as(form).await;
    let (identity_after, devices_after) = identity_seen_by(&onlooker, &owner).await;

    assert_eq!(
        identity_after.as_deref(), Some(identity.as_str()),
        "a later login must not replace the account's identity",
    );
    assert_eq!(devices_after.len(), 2, "the second login should be a second device, got {devices_after:?}");
    assert!(
        devices_after.contains(&(first_device.clone(), true)),
        "the first device must still be signed by the identity, got {devices_after:?}",
    );

    first.shutdown().await;
    second.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn logging_in_again_after_the_token_is_revoked_connects_without_adding_a_device() {
    use matrix_sdk::ruma::api::client::{device::delete_device, uiaa};

    let form = test_connection_form();
    let password = form.password.clone().expect("the test form carries the password");
    // Its own login is one of the devices counted below, before and after.
    let observer = logged_in_reader(&form.username, &password).await;
    let before = device_ids(&observer).await;

    let mut h = TestHarness::new();
    h.connect_as(form.clone()).await;
    let connected = device_ids(&observer).await;
    let etch_device = connected.iter().find(|id| !before.contains(id))
        .expect("connecting should have registered a device")
        .clone();

    let challenge = observer.send(delete_device::v3::Request::new(etch_device.clone())).await
        .expect_err("deleting a device should ask for the password");
    let mut auth = uiaa::Password::new(
        uiaa::UserIdentifier::UserIdOrLocalpart(form.username.clone()),
        password,
    );
    auth.session = challenge.as_uiaa_response().and_then(|info| info.session.clone());
    let mut request = delete_device::v3::Request::new(etch_device.clone());
    request.auth = Some(uiaa::AuthData::Password(auth));
    observer.send(request).await.expect("the device should be deleted");

    // The engine only learns of it from its next sync, which can be a whole long poll away.
    h.expect_event(|e| match e {
        CoreEvent::Matrix(MatrixEvent::ConnectionState(state)) if state.is_failed() => Some(()),
        _ => None,
    }, Duration::from_secs(45)).await;
    h.expect_event(|e| match e {
        CoreEvent::Matrix(MatrixEvent::ConnectionState(ConnectionState::Connected)) => Some(()),
        _ => None,
    }, CONNECT_TIMEOUT).await;

    let after = device_ids(&observer).await;
    assert!(!after.contains(&etch_device), "the revoked device should be gone, got {after:?}");
    assert_eq!(
        after.len(), connected.len(),
        "logging in again should replace the revoked device, not add to it, got {after:?}",
    );

    h.shutdown().await;
}
