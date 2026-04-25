use std::path::Path;
use serde::{Deserialize, Serialize};
use crate::models::ServerBookmark;

#[derive(Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub bookmarks: Vec<ServerBookmark>,
    #[serde(default)]
    pub mumble_initialized: bool,
    #[serde(default)]
    pub transmission_mode: Option<String>,
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

