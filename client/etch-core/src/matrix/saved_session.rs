use std::path::{Component, Path, PathBuf};

use anyhow::Context;
use matrix_sdk::authentication::matrix::MatrixSession;
use serde::{Deserialize, Serialize};

use crate::commands::ServerConnectionForm;

const SESSION_FILE: &str = "session.json";
const STORES_DIR: &str = "stores";
/// Where every session kept its store before each login was given its own.
const LEGACY_STORE: &str = "matrix_store";

pub(crate) fn server_dir(data_dir: &Path, form: &ServerConnectionForm) -> PathBuf {
    data_dir.join("servers").join(format!("{}@{}", form.username, form.hostname))
}

pub(crate) fn session_file(server_dir: &Path) -> PathBuf {
    server_dir.join(SESSION_FILE)
}

/// `restore_session` never validates a saved session, so deleting this file is the only
/// way to force a fresh login.
pub(crate) fn session_path(data_dir: &Path, form: &ServerConnectionForm) -> PathBuf {
    session_file(&server_dir(data_dir, form))
}

/// A login together with the store it is bound to, since a store only works for the
/// device that created it.
#[derive(Serialize, Deserialize)]
pub(crate) struct SavedSession {
    #[serde(flatten)]
    pub session: MatrixSession,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    store: Option<String>,
}

impl SavedSession {
    pub fn new(session: MatrixSession, store: &FreshStore) -> Self {
        Self { session, store: Some(store.name.clone()) }
    }

    /// `Ok(None)` when there is no session file.
    pub fn load(server_dir: &Path) -> anyhow::Result<Option<Self>> {
        let json = match std::fs::read_to_string(session_file(server_dir)) {
            Ok(json) => json,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let saved: Self = serde_json::from_str(&json)?;
        if let Some(store) = &saved.store {
            let mut components = Path::new(store).components();
            let is_one_name = matches!(components.next(), Some(Component::Normal(_)))
                && components.next().is_none();
            anyhow::ensure!(is_one_name, "the session names a store outside its own directory");
        }
        Ok(Some(saved))
    }

    pub fn save(&self, server_dir: &Path) -> anyhow::Result<()> {
        let path = session_file(server_dir);
        let tmp = path.with_extension("json.tmp");
        let written = std::fs::write(&tmp, serde_json::to_string(self)?)
            .and_then(|()| std::fs::rename(&tmp, &path));
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        written.with_context(|| format!("could not save the session to {}", path.display()))
    }

    pub fn store_dir(&self, server_dir: &Path) -> PathBuf {
        match &self.store {
            Some(name) => server_dir.join(STORES_DIR).join(name),
            None => server_dir.join(LEGACY_STORE),
        }
    }
}

/// A store directory created for one login, removed again unless a session comes to name it.
pub(crate) struct FreshStore {
    path: PathBuf,
    name: String,
    kept: bool,
}

impl FreshStore {
    pub fn create(server_dir: &Path) -> std::io::Result<Self> {
        let stores = server_dir.join(STORES_DIR);
        std::fs::create_dir_all(&stores)?;
        let mut n = 1u32;
        loop {
            let name = n.to_string();
            let path = stores.join(&name);
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path, name, kept: false }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
                Err(e) => return Err(e),
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn keep(&mut self) {
        self.kept = true;
    }
}

impl Drop for FreshStore {
    fn drop(&mut self) {
        if self.kept {
            return;
        }
        // A client that still holds the files open makes this fail on Windows; the next startup removes it.
        if let Err(e) = std::fs::remove_dir_all(&self.path) {
            log::debug!("Leaving the unused store {} for the next startup: {e}", self.path.display());
        }
    }
}

/// Must run before any client is built: a store whose login is still in progress has no
/// session naming it yet.
pub(crate) fn remove_unreferenced_stores(data_dir: &Path) {
    let Ok(servers) = std::fs::read_dir(data_dir.join("servers")) else { return };
    for server in servers.flatten() {
        let server_dir = server.path();
        if is_directory(&server_dir) {
            remove_unreferenced_stores_of(&server_dir);
        }
    }
}

fn remove_unreferenced_stores_of(server_dir: &Path) {
    let referenced = match SavedSession::load(server_dir) {
        Ok(saved) => saved.map(|saved| saved.store_dir(server_dir)),
        Err(e) => {
            // It may still name a store, so nothing here is provably unused.
            log::warn!("Not cleaning up {}: its session is unreadable ({e})", server_dir.display());
            return;
        }
    };

    let mut stores = vec![server_dir.join(LEGACY_STORE)];
    if let Ok(entries) = std::fs::read_dir(server_dir.join(STORES_DIR)) {
        stores.extend(entries.flatten().map(|entry| entry.path()));
    }

    for store in stores {
        if referenced.as_ref() == Some(&store) || !is_directory(&store) {
            continue;
        }
        match std::fs::remove_dir_all(&store) {
            Ok(()) => log::info!("Removed the store {}, which no session uses", store.display()),
            Err(e) => log::warn!("Could not remove the unused store {}: {e}", store.display()),
        }
    }
}

/// A symlink is not a directory here, so nothing outside the data directory is ever followed.
fn is_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAVED_BEFORE_PER_LOGIN_STORES: &str =
        r#"{"user_id":"@alice:example.com","device_id":"TESTDEVICE","access_token":"token"}"#;

    fn session() -> MatrixSession {
        serde_json::from_str(SAVED_BEFORE_PER_LOGIN_STORES).unwrap()
    }

    fn children(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|entries| {
                entries.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn a_session_saved_before_per_login_stores_still_opens_matrix_store() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(session_file(tmp.path()), SAVED_BEFORE_PER_LOGIN_STORES).unwrap();

        let saved = SavedSession::load(tmp.path()).unwrap().expect("the session file should be read");

        assert_eq!(saved.store_dir(tmp.path()), tmp.path().join("matrix_store"));
        assert_eq!(saved.session.meta.device_id.as_str(), "TESTDEVICE");
    }

    #[test]
    fn a_fresh_login_gets_a_store_of_its_own_and_gives_it_back_if_it_never_uses_it() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("matrix_store")).unwrap();

        let mut used = FreshStore::create(tmp.path()).unwrap();
        let abandoned = FreshStore::create(tmp.path()).unwrap();
        assert_ne!(used.path(), abandoned.path(), "two logins must never share a store");
        assert!(used.path().is_dir() && abandoned.path().is_dir());

        SavedSession::new(session(), &used).save(tmp.path()).unwrap();
        used.keep();
        let (used_path, abandoned_path) = (used.path().to_path_buf(), abandoned.path().to_path_buf());
        drop((used, abandoned));
        let (used, abandoned) = (used_path, abandoned_path);

        let saved = SavedSession::load(tmp.path()).unwrap().expect("the session was just saved");
        assert_eq!(saved.store_dir(tmp.path()), used, "the session should name the store it logged in with");
        assert!(used.is_dir(), "a store a session names must survive");
        assert!(!abandoned.exists(), "a store no session came to name should be removed");
        assert!(tmp.path().join("matrix_store").is_dir(), "an earlier login's store is not this login's to remove");
    }

    #[test]
    fn startup_removes_the_stores_no_session_names_and_nothing_else() {
        let tmp = tempfile::tempdir().unwrap();
        let servers = tmp.path().join("servers");
        let make = |server: &str, dirs: &[&str]| {
            let dir = servers.join(server);
            for name in dirs {
                std::fs::create_dir_all(dir.join(name)).unwrap();
            }
            dir
        };

        let current = make("current@example.com", &["matrix_store", "stores/1", "stores/2", "notes"]);
        std::fs::write(
            session_file(&current),
            r#"{"user_id":"@current:example.com","device_id":"D","access_token":"t","store":"2"}"#,
        ).unwrap();
        std::fs::write(current.join("stores/readme.txt"), "not a store").unwrap();

        let older = make("older@example.com", &["matrix_store", "stores/1"]);
        std::fs::write(session_file(&older), SAVED_BEFORE_PER_LOGIN_STORES).unwrap();

        let stuck = make("stuck@example.com", &["matrix_store"]);

        let unreadable = make("unreadable@example.com", &["matrix_store", "stores/1"]);
        std::fs::write(session_file(&unreadable), "{ not json").unwrap();

        remove_unreferenced_stores(tmp.path());

        assert_eq!(children(&current), ["notes", "session.json", "stores"]);
        assert_eq!(children(&current.join("stores")), ["2", "readme.txt"]);
        assert_eq!(children(&older), ["matrix_store", "session.json", "stores"]);
        assert_eq!(children(&older.join("stores")), [] as [&str; 0]);
        assert_eq!(children(&stuck), [] as [&str; 0], "a store left without any session should be removed");
        assert_eq!(children(&unreadable), ["matrix_store", "session.json", "stores"]);
        assert_eq!(children(&unreadable.join("stores")), ["1"], "an unreadable session proves nothing unused");
    }
}
