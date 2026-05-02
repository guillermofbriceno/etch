use std::path::Path;
use serde::{Deserialize, Serialize};
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
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        if std::fs::write(&tmp_path, &json).is_ok() {
            let _ = std::fs::rename(&tmp_path, &path);
        }
    }
}

pub fn update_bookmarks(data_dir: &Path, bookmarks: Vec<ServerBookmark>) {
    let mut settings = load(data_dir);
    settings.bookmarks = bookmarks;
    save(data_dir, &settings);
}

pub fn set_transmission_mode(data_dir: &Path, mode: String) {
    let mut settings = load(data_dir);
    settings.transmission_mode = Some(mode);
    save(data_dir, &settings);
}

pub fn set_vad_threshold(data_dir: &Path, value: f64) {
    let mut settings = load(data_dir);
    settings.vad_threshold = Some(value);
    save(data_dir, &settings);
}

pub fn set_voice_hold(data_dir: &Path, value: i64) {
    let mut settings = load(data_dir);
    settings.voice_hold = Some(value);
    save(data_dir, &settings);
}

pub fn set_use_mumble_settings(data_dir: &Path, value: bool) {
    let mut settings = load(data_dir);
    settings.use_mumble_settings = Some(value);
    save(data_dir, &settings);
}

pub fn set_deafen_suppresses_notifs(data_dir: &Path, value: bool) {
    let mut settings = load(data_dir);
    settings.deafen_suppresses_notifs = Some(value);
    save(data_dir, &settings);
}

pub fn hide_dm(data_dir: &Path, room_id: String) {
    let mut settings = load(data_dir);
    if !settings.hidden_dms.contains(&room_id) {
        settings.hidden_dms.push(room_id);
    }
    save(data_dir, &settings);
}

pub fn unhide_dm(data_dir: &Path, room_id: &str) {
    let mut settings = load(data_dir);
    settings.hidden_dms.retain(|id| id != room_id);
    save(data_dir, &settings);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ServerBookmark;

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
        let mut s = Settings::default();
        s.transmission_mode = Some("continuous".into());
        s.vad_threshold = Some(0.5);
        s.voice_hold = Some(300);
        s.use_mumble_settings = Some(true);

        save(tmp.path(), &s);
        let loaded = load(tmp.path());

        assert_eq!(loaded.transmission_mode.as_deref(), Some("continuous"));
        assert_eq!(loaded.vad_threshold, Some(0.5));
        assert_eq!(loaded.voice_hold, Some(300));
        assert_eq!(loaded.use_mumble_settings, Some(true));
    }

    #[test]
    fn update_bookmarks_replaces_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let bm1 = ServerBookmark {
            id: "1".into(), label: "First".into(), address: "a.com".into(),
            port: 8448, username: "alice".into(), auto_connect: false,
            mumble_host: None, mumble_port: None, mumble_username: None, mumble_password: None,
        };
        update_bookmarks(tmp.path(), vec![bm1]);
        assert_eq!(load(tmp.path()).bookmarks.len(), 1);

        let bm2 = ServerBookmark {
            id: "2".into(), label: "Second".into(), address: "b.com".into(),
            port: 8448, username: "bob".into(), auto_connect: true,
            mumble_host: None, mumble_port: None, mumble_username: None, mumble_password: None,
        };
        let bm3 = ServerBookmark {
            id: "3".into(), label: "Third".into(), address: "c.com".into(),
            port: 8448, username: "carol".into(), auto_connect: false,
            mumble_host: None, mumble_port: None, mumble_username: None, mumble_password: None,
        };
        update_bookmarks(tmp.path(), vec![bm2, bm3]);
        let loaded = load(tmp.path());
        assert_eq!(loaded.bookmarks.len(), 2);
        assert_eq!(loaded.bookmarks[0].label, "Second");
        assert_eq!(loaded.bookmarks[1].label, "Third");
    }

    #[test]
    fn hide_dm_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        hide_dm(tmp.path(), "!room:example.com".into());
        hide_dm(tmp.path(), "!room:example.com".into());
        let loaded = load(tmp.path());
        assert_eq!(loaded.hidden_dms.len(), 1);
    }

    #[test]
    fn unhide_dm_removes_only_target() {
        let tmp = tempfile::tempdir().unwrap();
        hide_dm(tmp.path(), "!room1:example.com".into());
        hide_dm(tmp.path(), "!room2:example.com".into());
        unhide_dm(tmp.path(), "!room1:example.com");
        let loaded = load(tmp.path());
        assert_eq!(loaded.hidden_dms, vec!["!room2:example.com"]);
    }

    #[test]
    fn set_transmission_mode_persists() {
        let tmp = tempfile::tempdir().unwrap();
        set_transmission_mode(tmp.path(), "push_to_talk".into());
        assert_eq!(load(tmp.path()).transmission_mode.as_deref(), Some("push_to_talk"));
    }

    #[test]
    fn set_vad_threshold_persists() {
        let tmp = tempfile::tempdir().unwrap();
        set_vad_threshold(tmp.path(), 0.75);
        assert_eq!(load(tmp.path()).vad_threshold, Some(0.75));
    }

    #[test]
    fn set_voice_hold_persists() {
        let tmp = tempfile::tempdir().unwrap();
        set_voice_hold(tmp.path(), 500);
        assert_eq!(load(tmp.path()).voice_hold, Some(500));
    }

    #[test]
    fn set_deafen_suppresses_notifs_persists() {
        let tmp = tempfile::tempdir().unwrap();
        set_deafen_suppresses_notifs(tmp.path(), false);
        assert_eq!(load(tmp.path()).deafen_suppresses_notifs, Some(false));
    }

    #[test]
    fn deafen_suppresses_notifs_defaults_to_none() {
        let tmp = tempfile::tempdir().unwrap();
        let s = load(tmp.path());
        assert_eq!(s.deafen_suppresses_notifs, None);
    }

    #[test]
    fn settings_mutations_preserve_other_fields() {
        let tmp = tempfile::tempdir().unwrap();
        // Set several fields
        set_transmission_mode(tmp.path(), "continuous".into());
        hide_dm(tmp.path(), "!room:example.com".into());
        set_vad_threshold(tmp.path(), 0.3);

        // Verify all three survive
        let loaded = load(tmp.path());
        assert_eq!(loaded.transmission_mode.as_deref(), Some("continuous"));
        assert_eq!(loaded.hidden_dms.len(), 1);
        assert_eq!(loaded.vad_threshold, Some(0.3));
    }
}

