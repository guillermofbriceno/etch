//! Integration tests: real MatrixService, mock voice, disposable Docker servers.
//!
//! Run with: cargo test -p etch-core --features integration-tests -- --test-threads=1
//! Or via the orchestrator: ./tests/integration/run.sh

use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::commands::{CoreCommand, MatrixCommand, ChatMessageSend, ServerConnectionForm, SystemCommand};
use crate::engine::CoreEngine;
use crate::events::{CoreEvent, MatrixEvent, SystemEvent};
use crate::matrix::service::MatrixService;
use crate::matrix::timeline::TimelineEntryKind;
use crate::models::{ConnectionState, RoomInfo, RoomType};
use crate::test_mocks::MockVoice;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

struct TestHarness {
    cmd_tx: mpsc::Sender<CoreCommand>,
    event_rx: mpsc::Receiver<CoreEvent>,
    engine_handle: tokio::task::JoinHandle<()>,
    // Held so the temp directory outlives the engine. Fields are dropped in
    // declaration order, so _data_dir is dropped after engine_handle.
    _data_dir: tempfile::TempDir,
}

impl TestHarness {
    fn new() -> Self {
        let data_dir = tempfile::tempdir().expect("failed to create temp dir");
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let (event_tx, event_rx) = mpsc::channel(256);

        let matrix = MatrixService::new(event_tx.clone(), data_dir.path().to_path_buf());
        let voice = MockVoice::new();
        let engine = CoreEngine::new(
            cmd_rx, event_tx, matrix, voice, data_dir.path().to_path_buf(),
        );
        let engine_handle = tokio::spawn(engine.run());

        Self { cmd_tx, event_rx, engine_handle, _data_dir: data_dir }
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
        self.send(CoreCommand::System(SystemCommand::ConnectToServer(
            test_connection_form(),
        ))).await;

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
            attachment_path: None,
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
        attachment_path: None,
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
