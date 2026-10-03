use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The files Etch wrote so they could be uploaded. Only a path created here can be
/// discarded, so a file the user picked is never deleted.
#[derive(Clone)]
pub struct TempUploads {
    root: PathBuf,
    created: Arc<Mutex<HashSet<PathBuf>>>,
}

impl TempUploads {
    pub fn new(root: PathBuf) -> Self {
        Self { root, created: Arc::default() }
    }

    /// Writes `bytes` to a new file, in a directory made for it, named after `file_name`.
    pub fn create(&self, file_name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
        let dir = self.fresh_dir()?;
        let path = dir.join(base_name(file_name));
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(e) => {
                let _ = std::fs::remove_dir(&dir);
                return Err(e);
            }
        };
        if let Err(e) = file.write_all(bytes) {
            drop(file);
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_dir(&dir);
            return Err(e);
        }
        self.created.lock().expect("temp uploads lock").insert(path.clone());
        Ok(path)
    }

    /// Does nothing to a path this registry did not create.
    pub fn discard(&self, path: &Path) {
        if !self.created.lock().expect("temp uploads lock").remove(path) {
            return;
        }
        if let Err(e) = std::fs::remove_file(path)
            && e.kind() != io::ErrorKind::NotFound
        {
            log::warn!("Failed to remove the temp upload {}: {e}", path.display());
        }
        // Not `remove_dir_all`: only the file was ours, so anything else in there stays.
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir(dir);
        }
    }

    // `create_dir` fails on a directory that already exists, so this never adopts one.
    fn fresh_dir(&self) -> io::Result<PathBuf> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..16 {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = self.root.join(format!("etch-upload-{}-{nanos}-{n}", std::process::id()));
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok(dir),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "no free temp directory name"))
    }
}

fn base_name(file_name: &str) -> &std::ffi::OsStr {
    Path::new(file_name)
        .file_name()
        .unwrap_or(std::ffi::OsStr::new("attachment"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> (tempfile::TempDir, TempUploads) {
        let root = tempfile::tempdir().unwrap();
        let uploads = TempUploads::new(root.path().to_path_buf());
        (root, uploads)
    }

    #[test]
    fn a_created_file_is_removed_with_its_directory_when_discarded() {
        let (root, uploads) = registry();
        let path = uploads.create("photo.png", b"pixels").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"pixels");
        assert_eq!(path.file_name().unwrap(), "photo.png");

        uploads.discard(&path);

        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_file_it_did_not_create_is_never_deleted() {
        let (root, uploads) = registry();
        let ours = uploads.create("photo.png", b"ours").unwrap();
        let our_dir = ours.parent().unwrap();

        let lookalike_dir = root.path().join("etch-upload-1-1-1");
        std::fs::create_dir(&lookalike_dir).unwrap();
        let in_lookalike_dir = lookalike_dir.join("photo.png");
        let beside_ours = our_dir.join("notes.txt");
        let elsewhere = root.path().join("holiday.png");
        for user_file in [&in_lookalike_dir, &beside_ours, &elsewhere] {
            std::fs::write(user_file, b"the user's").unwrap();
            uploads.discard(user_file);
            assert!(user_file.exists(), "{} was deleted", user_file.display());
        }

        uploads.discard(&ours);
        assert!(!ours.exists());
        assert!(beside_ours.exists(), "a file the user put beside ours must survive");
    }

    #[test]
    fn a_path_can_be_discarded_only_once() {
        let (_root, uploads) = registry();
        let path = uploads.create("photo.png", b"ours").unwrap();
        uploads.discard(&path);

        std::fs::create_dir(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"the user's, written later at the same path").unwrap();
        uploads.discard(&path);

        assert!(path.exists());
    }

    #[test]
    fn the_file_name_cannot_escape_the_new_directory() {
        let (root, uploads) = registry();
        let victim = root.path().join("victim.txt");
        std::fs::write(&victim, b"the user's").unwrap();

        let path = uploads.create("../victim.txt", b"ours").unwrap();

        assert_ne!(path, victim);
        assert_eq!(path.parent().unwrap().parent().unwrap(), root.path());
        assert_eq!(std::fs::read(&victim).unwrap(), b"the user's");
        assert!(uploads.create("..", b"ours").unwrap().starts_with(root.path()));
    }
}
