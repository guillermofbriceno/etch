use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::models::ServerBookmark;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub bookmarks: Vec<ServerBookmark>,
    #[serde(default)]
    pub mumble_initialized: bool,
    #[serde(default)]
    pub transmission_mode: Option<String>,
    #[serde(default)]
    pub vad_threshold: Option<f64>,
    #[serde(default)]
    pub voice_hold: Option<i64>,
    #[serde(default)]
    pub use_mumble_settings: Option<bool>,
    #[serde(default)]
    pub hidden_dms: Vec<String>,
    #[serde(default)]
    pub deafen_suppresses_notifs: Option<bool>,
    #[serde(default)]
    pub sfx_paths: HashMap<String, String>,
    #[serde(default)]
    pub custom_css: Option<String>,
    #[serde(default)]
    pub event_scripts: HashMap<String, String>,
}

pub fn load(data_dir: &Path) -> Settings {
    let path = data_dir.join("settings.json");
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

pub fn save(data_dir: &Path, settings: &Settings) {
    let path = data_dir.join("settings.json");
    let tmp_path = data_dir.join("settings.json.tmp");

    let _ = std::fs::create_dir_all(data_dir);
    if let Ok(json) = serde_json::to_string_pretty(settings)
        && std::fs::write(&tmp_path, &json).is_ok()
    {
        let _ = std::fs::rename(&tmp_path, &path);
    }
}

impl Settings {
    pub fn hide_dm(&mut self, room_id: String) {
        if !self.hidden_dms.contains(&room_id) {
            self.hidden_dms.push(room_id);
        }
    }

    pub fn unhide_dm(&mut self, room_id: &str) {
        self.hidden_dms.retain(|id| id != room_id);
    }
}

/// Bounds how long the trailing write of a burst can be delayed.
const WRITE_COALESCE_WINDOW: Duration = Duration::from_millis(250);

#[derive(Clone, Default)]
pub(crate) struct WriteCounter(Arc<AtomicUsize>);

impl WriteCounter {
    fn record(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn count(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
}

/// The app has no setters for these fields, so a write carries forward the hand-edited
/// file contents.
fn keep_externally_owned_fields(settings: &mut Settings, on_disk: Settings) {
    settings.mumble_initialized = on_disk.mumble_initialized;
    settings.sfx_paths = on_disk.sfx_paths;
    settings.custom_css = on_disk.custom_css;
    settings.event_scripts = on_disk.event_scripts;
}

fn persist(data_dir: &Path, mut settings: Settings) {
    keep_externally_owned_fields(&mut settings, load(data_dir));
    save(data_dir, &settings);
}

/// Owns `settings.json`: reads come from memory and writes go through a debounced
/// background writer that `shutdown` flushes.
pub struct SettingsStore {
    data_dir: PathBuf,
    current: Settings,
    revision: u64,
    tx: watch::Sender<(u64, Settings)>,
    /// Held until the writer starts so `update` always has a live receiver.
    idle_rx: Option<watch::Receiver<(u64, Settings)>>,
    writer: Option<JoinHandle<()>>,
    writes: WriteCounter,
    coalesce: Duration,
}

impl SettingsStore {
    pub fn open(data_dir: &Path) -> Self {
        Self::from_loaded(data_dir.to_path_buf(), load(data_dir))
    }

    /// Lets startup read the file once and share the values with the script dispatcher.
    pub fn from_loaded(data_dir: PathBuf, settings: Settings) -> Self {
        Self::with_coalesce_window(data_dir, settings, WRITE_COALESCE_WINDOW)
    }

    fn with_coalesce_window(data_dir: PathBuf, settings: Settings, coalesce: Duration) -> Self {
        let (tx, rx) = watch::channel((0, settings.clone()));
        Self {
            data_dir,
            current: settings,
            revision: 0,
            tx,
            idle_rx: Some(rx),
            writer: None,
            writes: WriteCounter::default(),
            coalesce,
        }
    }

    pub fn get(&self) -> &Settings {
        &self.current
    }

    /// Must be called within a Tokio runtime: the first call starts the writer task.
    pub fn update(&mut self, change: impl FnOnce(&mut Settings)) {
        change(&mut self.current);
        self.revision += 1;
        let _ = self.tx.send((self.revision, self.current.clone()));

        if self.writer.is_none()
            && let Some(rx) = self.idle_rx.take()
        {
            self.writer = Some(tokio::spawn(writer_task(
                self.data_dir.clone(),
                rx,
                self.coalesce,
                self.writes.clone(),
            )));
        }
    }

    #[cfg(test)]
    pub(crate) fn write_counter(&self) -> WriteCounter {
        self.writes.clone()
    }

    pub async fn shutdown(self) {
        let Self { tx, writer, .. } = self;
        drop(tx);
        if let Some(writer) = writer
            && let Err(e) = writer.await
        {
            log::warn!("Settings writer did not finish cleanly: {e}");
        }
    }
}

/// Debounced on both edges: the first change after idle is written immediately, the
/// rest of a burst in one trailing write.
async fn writer_task(
    data_dir: PathBuf,
    mut rx: watch::Receiver<(u64, Settings)>,
    coalesce: Duration,
    writes: WriteCounter,
) {
    let mut written: u64 = 0;

    loop {
        // An unseen value wins over a dropped sender, so the final change is still written.
        let store_live = rx.changed().await.is_ok();

        write_pending(&data_dir, &mut rx, &mut written, &writes).await;

        if !store_live {
            return;
        }

        // Cut short if the store goes away so shutdown is not held up.
        let deadline = tokio::time::Instant::now() + coalesce;
        let mut store_still_live = true;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                changed = rx.changed() => {
                    if changed.is_err() {
                        store_still_live = false;
                        break;
                    }
                }
            }
        }

        write_pending(&data_dir, &mut rx, &mut written, &writes).await;

        if !store_still_live {
            return;
        }
    }
}

async fn write_pending(
    data_dir: &Path,
    rx: &mut watch::Receiver<(u64, Settings)>,
    written: &mut u64,
    writes: &WriteCounter,
) {
    let (revision, snapshot) = rx.borrow_and_update().clone();
    if revision <= *written {
        return;
    }

    let dir = data_dir.to_path_buf();
    match tokio::task::spawn_blocking(move || persist(&dir, snapshot)).await {
        Ok(()) => {
            *written = revision;
            writes.record();
            log::debug!(
                "Settings written (revision {revision}, {} write(s) this session)",
                writes.count(),
            );
        }
        Err(e) => log::warn!("Settings write failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ServerBookmark;

    fn bookmark(id: &str, label: &str, username: &str, auto_connect: bool) -> ServerBookmark {
        ServerBookmark {
            id: id.into(), label: label.into(), address: "example.com".into(),
            port: 8448, username: username.into(), auto_connect,
            mumble_host: None, mumble_port: None, mumble_username: None, mumble_password: None,
        }
    }

    async fn store_round_trip(
        data_dir: &Path,
        change: impl FnOnce(&mut Settings),
    ) -> Settings {
        let mut store = SettingsStore::open(data_dir);
        store.update(change);
        store.shutdown().await;
        load(data_dir)
    }

    #[test]
    fn load_returns_default_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let s = load(tmp.path());
        assert!(s.bookmarks.is_empty());
        assert!(!s.mumble_initialized);
        assert!(s.hidden_dms.is_empty());
    }

    #[test]
    fn save_and_load_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Settings {
            transmission_mode: Some("continuous".into()),
            vad_threshold: Some(0.5),
            voice_hold: Some(300),
            use_mumble_settings: Some(true),
            ..Default::default()
        };

        save(tmp.path(), &s);
        let loaded = load(tmp.path());

        assert_eq!(loaded.transmission_mode.as_deref(), Some("continuous"));
        assert_eq!(loaded.vad_threshold, Some(0.5));
        assert_eq!(loaded.voice_hold, Some(300));
        assert_eq!(loaded.use_mumble_settings, Some(true));
    }

    #[tokio::test]
    async fn update_bookmarks_replaces_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let after_first = store_round_trip(tmp.path(), |s| {
            s.bookmarks = vec![bookmark("1", "First", "alice", false)];
        }).await;
        assert_eq!(after_first.bookmarks.len(), 1);

        let loaded = store_round_trip(tmp.path(), |s| {
            s.bookmarks = vec![
                bookmark("2", "Second", "bob", true),
                bookmark("3", "Third", "carol", false),
            ];
        }).await;
        assert_eq!(loaded.bookmarks.len(), 2);
        assert_eq!(loaded.bookmarks[0].label, "Second");
        assert_eq!(loaded.bookmarks[1].label, "Third");
    }

    #[tokio::test]
    async fn hide_dm_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let loaded = store_round_trip(tmp.path(), |s| {
            s.hide_dm("!room:example.com".into());
            s.hide_dm("!room:example.com".into());
        }).await;
        assert_eq!(loaded.hidden_dms.len(), 1);
    }

    #[tokio::test]
    async fn unhide_dm_removes_only_target() {
        let tmp = tempfile::tempdir().unwrap();
        let loaded = store_round_trip(tmp.path(), |s| {
            s.hide_dm("!room1:example.com".into());
            s.hide_dm("!room2:example.com".into());
            s.unhide_dm("!room1:example.com");
        }).await;
        assert_eq!(loaded.hidden_dms, vec!["!room2:example.com"]);
    }

    #[tokio::test]
    async fn set_transmission_mode_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let loaded = store_round_trip(tmp.path(), |s| {
            s.transmission_mode = Some("push_to_talk".into());
        }).await;
        assert_eq!(loaded.transmission_mode.as_deref(), Some("push_to_talk"));
    }

    #[tokio::test]
    async fn set_vad_threshold_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let loaded = store_round_trip(tmp.path(), |s| s.vad_threshold = Some(0.75)).await;
        assert_eq!(loaded.vad_threshold, Some(0.75));
    }

    #[tokio::test]
    async fn set_voice_hold_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let loaded = store_round_trip(tmp.path(), |s| s.voice_hold = Some(500)).await;
        assert_eq!(loaded.voice_hold, Some(500));
    }

    #[tokio::test]
    async fn set_deafen_suppresses_notifs_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let loaded = store_round_trip(tmp.path(), |s| {
            s.deafen_suppresses_notifs = Some(false);
        }).await;
        assert_eq!(loaded.deafen_suppresses_notifs, Some(false));
    }

    #[test]
    fn deafen_suppresses_notifs_defaults_to_none() {
        let tmp = tempfile::tempdir().unwrap();
        let s = load(tmp.path());
        assert_eq!(s.deafen_suppresses_notifs, None);
    }

    #[test]
    fn sfx_paths_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = Settings::default();
        s.sfx_paths.insert("new_notif".into(), "/home/user/sounds/ping.wav".into());
        s.sfx_paths.insert("user_join".into(), "/home/user/sounds/hello.wav".into());

        save(tmp.path(), &s);
        let loaded = load(tmp.path());
        assert_eq!(loaded.sfx_paths.len(), 2);
        assert_eq!(loaded.sfx_paths["new_notif"], "/home/user/sounds/ping.wav");
        assert_eq!(loaded.sfx_paths["user_join"], "/home/user/sounds/hello.wav");
    }

    #[test]
    fn custom_css_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Settings {
            custom_css: Some("/home/user/theme.css".into()),
            ..Default::default()
        };

        save(tmp.path(), &s);
        let loaded = load(tmp.path());
        assert_eq!(loaded.custom_css.as_deref(), Some("/home/user/theme.css"));
    }

    #[test]
    fn custom_css_defaults_to_none() {
        let tmp = tempfile::tempdir().unwrap();
        let s = load(tmp.path());
        assert!(s.custom_css.is_none());
    }

    #[test]
    fn sfx_paths_defaults_to_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let s = load(tmp.path());
        assert!(s.sfx_paths.is_empty());
    }

    #[test]
    fn event_scripts_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = Settings::default();
        s.event_scripts.insert("user_join".into(), "echo hello".into());
        s.event_scripts.insert("new_message".into(), "notify-send \"$ETCH_USER\"".into());

        save(tmp.path(), &s);
        let loaded = load(tmp.path());
        assert_eq!(loaded.event_scripts.len(), 2);
        assert_eq!(loaded.event_scripts["user_join"], "echo hello");
        assert_eq!(loaded.event_scripts["new_message"], "notify-send \"$ETCH_USER\"");
    }

    #[test]
    fn event_scripts_defaults_to_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let s = load(tmp.path());
        assert!(s.event_scripts.is_empty());
    }

    #[tokio::test]
    async fn settings_mutations_preserve_other_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = SettingsStore::open(tmp.path());
        store.update(|s| s.transmission_mode = Some("continuous".into()));
        store.update(|s| s.hide_dm("!room:example.com".into()));
        store.update(|s| s.vad_threshold = Some(0.3));
        store.shutdown().await;

        let loaded = load(tmp.path());
        assert_eq!(loaded.transmission_mode.as_deref(), Some("continuous"));
        assert_eq!(loaded.hidden_dms.len(), 1);
        assert_eq!(loaded.vad_threshold, Some(0.3));
    }


    /// The file changing under the store must not change what it reports for app-owned fields.
    #[test]
    fn reads_come_from_memory_not_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        save(tmp.path(), &Settings { vad_threshold: Some(0.1), ..Default::default() });

        let store = SettingsStore::open(tmp.path());
        assert_eq!(store.get().vad_threshold, Some(0.1));

        save(tmp.path(), &Settings { vad_threshold: Some(0.9), ..Default::default() });
        assert_eq!(
            store.get().vad_threshold, Some(0.1),
            "the store, not the file, is the source of truth once it is open",
        );
    }

    /// A slider drag must not cost a write per event; the bound is two because the
    /// writer fires on both edges.
    #[tokio::test]
    async fn a_burst_of_changes_does_not_cost_a_write_each() {
        const EVENTS: usize = 200;
        let tmp = tempfile::tempdir().unwrap();

        let mut store = SettingsStore::open(tmp.path());
        let writes = store.write_counter();
        for i in 0..EVENTS {
            store.update(|s| s.vad_threshold = Some(i as f64 / 1000.0));
        }
        store.shutdown().await;

        let performed = writes.count();
        assert!(
            performed <= 2,
            "{EVENTS} changes in one burst cost {performed} file writes; \
             the burst should have collapsed to a leading and a trailing write",
        );
        assert_eq!(
            load(tmp.path()).vad_threshold,
            Some((EVENTS - 1) as f64 / 1000.0),
            "the coalesced write must carry the last value, not an earlier one",
        );
    }

    #[tokio::test]
    async fn the_last_change_before_shutdown_reaches_disk() {
        let tmp = tempfile::tempdir().unwrap();

        let mut store = SettingsStore::open(tmp.path());
        store.update(|s| s.vad_threshold = Some(0.11));
        // No pause: shutdown lands inside the coalescing window.
        store.update(|s| s.voice_hold = Some(420));
        store.shutdown().await;

        let loaded = load(tmp.path());
        assert_eq!(loaded.vad_threshold, Some(0.11));
        assert_eq!(loaded.voice_hold, Some(420));
    }

    /// Hand edits to fields the app has no setter for must survive the next write.
    #[tokio::test]
    async fn writes_do_not_clobber_hand_edited_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = SettingsStore::open(tmp.path());

        let mut hand_edited = Settings::default();
        hand_edited.event_scripts.insert("user_join".into(), "echo hi".into());
        hand_edited.sfx_paths.insert("mute".into(), "/sounds/mute.wav".into());
        hand_edited.custom_css = Some("/themes/dark.css".into());
        save(tmp.path(), &hand_edited);

        store.update(|s| s.vad_threshold = Some(0.42));
        store.shutdown().await;

        let loaded = load(tmp.path());
        assert_eq!(loaded.vad_threshold, Some(0.42), "the app's own change must land");
        assert_eq!(loaded.event_scripts.get("user_join").map(String::as_str), Some("echo hi"));
        assert_eq!(loaded.sfx_paths.get("mute").map(String::as_str), Some("/sounds/mute.wav"));
        assert_eq!(loaded.custom_css.as_deref(), Some("/themes/dark.css"));
    }

    async fn settings_on_disk_within(
        data_dir: &Path,
        within: Duration,
        done: impl Fn(&Settings) -> bool,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            if done(&load(data_dir)) {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// A discrete change must reach disk immediately; the window is far longer than the
    /// wait, so a trailing-edge-only writer fails.
    #[tokio::test]
    async fn an_isolated_change_is_written_without_waiting_out_the_window() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = SettingsStore::with_coalesce_window(
            tmp.path().to_path_buf(),
            load(tmp.path()),
            Duration::from_secs(30),
        );

        store.update(|s| s.bookmarks = vec![bookmark("1", "Only", "alice", false)]);

        let landed = settings_on_disk_within(tmp.path(), Duration::from_secs(2), |s| {
            s.bookmarks.len() == 1 && s.bookmarks[0].label == "Only"
        }).await;
        assert!(
            landed,
            "an isolated change had not reached disk 2s in, with a 30s coalescing window",
        );

        store.shutdown().await;
    }

    /// Opening and closing the app without changes must not rewrite the file.
    #[tokio::test]
    async fn shutdown_without_changes_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SettingsStore::open(tmp.path());
        let writes = store.write_counter();
        store.shutdown().await;

        assert_eq!(writes.count(), 0);
        assert!(
            !tmp.path().join("settings.json").exists(),
            "an untouched store must not create the file",
        );
    }
}
