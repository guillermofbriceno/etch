use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::settings;

/// Dispatches event scripts configured in settings.
///
/// Caches the `event_scripts` map at construction time (avoids per-event disk I/O)
/// and enforces a per-event-type debounce window to prevent process storms.
pub struct ScriptDispatcher {
    scripts: HashMap<String, String>,
    last_fired: Mutex<HashMap<String, Instant>>,
    debounce: Duration,
}

impl ScriptDispatcher {
    /// Load event scripts from settings on disk. Call once at startup.
    pub fn new(data_dir: &Path) -> Self {
        let settings = settings::load(data_dir);
        Self {
            scripts: settings.event_scripts,
            last_fired: Mutex::new(HashMap::new()),
            debounce: Duration::from_millis(500),
        }
    }

    /// Construct an empty dispatcher (no scripts configured). Useful for tests.
    #[cfg(test)]
    pub fn empty() -> Self {
        Self {
            scripts: HashMap::new(),
            last_fired: Mutex::new(HashMap::new()),
            debounce: Duration::from_millis(500),
        }
    }

    /// Construct a dispatcher with explicit scripts and debounce. Useful for
    /// unit tests that need to verify fire/debounce behavior.
    #[cfg(test)]
    pub fn with_scripts(scripts: HashMap<String, String>, debounce: Duration) -> Self {
        Self {
            scripts,
            last_fired: Mutex::new(HashMap::new()),
            debounce,
        }
    }

    /// Fire the script configured for `event`, if any.
    ///
    /// Sets `ETCH_{KEY}=value` environment variables for each entry in `vars`.
    /// Debounces: skips if the same event type fired within the last 500ms.
    /// The spawned process is killed after 60 seconds.
    ///
    /// No-op on Windows.
    #[cfg(not(target_os = "windows"))]
    pub fn fire(&self, event: &str, vars: &[(&str, &str)]) {
        let Some(script) = self.scripts.get(event) else {
            log::debug!("[scripting] No script configured for '{}'", event);
            return;
        };

        // Debounce check
        {
            let mut last = self.last_fired.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            if let Some(&prev) = last.get(event)
                && now.duration_since(prev) < self.debounce
            {
                log::debug!("[scripting] Debounced '{}' (too soon)", event);
                return;
            }
            last.insert(event.to_string(), now);
        }

        log::info!("[scripting] Firing '{}': {}", event, script);
        let script = script.clone();
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| (format!("ETCH_{}", k), v.to_string()))
            .collect();

        tokio::spawn(async move {
            let mut child = match tokio::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(&script)
                .envs(vars)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .spawn()
            {
                Ok(child) => child,
                Err(e) => {
                    log::warn!("[scripting] Failed to spawn script for '{}': {}", script, e);
                    return;
                }
            };

            match tokio::time::timeout(Duration::from_secs(60), child.wait()).await {
                Ok(Ok(status)) => {
                    if !status.success() {
                        log::warn!("[scripting] Script exited with {}: {}", status, script);
                    }
                }
                Ok(Err(e)) => {
                    log::warn!("[scripting] Error waiting on script: {}", e);
                }
                Err(_) => {
                    log::warn!("[scripting] Script timed out after 60s, killing: {}", script);
                    let _ = child.kill().await;
                }
            }
        });
    }

    #[cfg(target_os = "windows")]
    pub fn fire(&self, _event: &str, _vars: &[(&str, &str)]) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dispatcher_with(events: &[(&str, &str)]) -> ScriptDispatcher {
        let scripts = events
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        ScriptDispatcher::with_scripts(scripts, Duration::from_millis(500))
    }

    #[test]
    fn empty_dispatcher_has_no_scripts() {
        let d = ScriptDispatcher::empty();
        assert!(d.scripts.is_empty());
    }

    #[test]
    fn new_loads_scripts_from_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let settings = crate::settings::Settings {
            event_scripts: [
                ("new_message".to_string(), "echo hello".to_string()),
                ("user_join".to_string(), "echo joined".to_string()),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        crate::settings::save(tmp.path(), &settings);

        let d = ScriptDispatcher::new(tmp.path());
        assert_eq!(d.scripts.len(), 2);
        assert_eq!(d.scripts["new_message"], "echo hello");
        assert_eq!(d.scripts["user_join"], "echo joined");
    }

    #[test]
    fn new_with_no_settings_file_yields_empty_scripts() {
        let tmp = tempfile::tempdir().unwrap();
        let d = ScriptDispatcher::new(tmp.path());
        assert!(d.scripts.is_empty());
    }

    // --- Tests below spawn real processes and only apply on unix ---

    #[cfg(unix)]
    #[test]
    fn fire_noop_for_unconfigured_event() {
        // Returns before tokio::spawn (no runtime needed).
        let d = dispatcher_with(&[("other_event", "echo hi")]);
        d.fire("nonexistent", &[]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fire_runs_configured_script() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("fired");

        let d = dispatcher_with(&[(
            "test_event",
            &format!("touch {}", marker.display()),
        )]);
        d.fire("test_event", &[]);

        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(marker.exists(), "Script should have created the marker file");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fire_passes_env_variables() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("env_out");

        let d = dispatcher_with(&[(
            "test_event",
            &format!("echo -n \"$ETCH_USER:$ETCH_ROOM\" > {}", out.display()),
        )]);
        d.fire("test_event", &[("USER", "alice"), ("ROOM", "!abc:example.com")]);

        tokio::time::sleep(Duration::from_millis(500)).await;
        let content = std::fs::read_to_string(&out).unwrap();
        assert_eq!(content, "alice:!abc:example.com");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn debounce_suppresses_rapid_calls() {
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("counter");
        std::fs::write(&counter, "0").unwrap();

        let script = format!(
            "n=$(cat {}); echo $((n + 1)) > {}",
            counter.display(),
            counter.display()
        );
        let d = dispatcher_with(&[("test_event", &script)]);

        d.fire("test_event", &[]);
        d.fire("test_event", &[]); // debounced
        d.fire("test_event", &[]); // debounced

        tokio::time::sleep(Duration::from_millis(500)).await;
        let n: i32 = std::fs::read_to_string(&counter)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(n, 1, "Only the first call should have fired");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn debounce_allows_different_event_types() {
        let dir = tempfile::tempdir().unwrap();
        let marker_a = dir.path().join("fired_a");
        let marker_b = dir.path().join("fired_b");

        let d = dispatcher_with(&[
            ("event_a", &format!("touch {}", marker_a.display())),
            ("event_b", &format!("touch {}", marker_b.display())),
        ]);
        d.fire("event_a", &[]);
        d.fire("event_b", &[]); // different event type, not debounced

        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(marker_a.exists(), "event_a should have fired");
        assert!(marker_b.exists(), "event_b should have fired");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn debounce_allows_after_window_expires() {
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("counter");
        std::fs::write(&counter, "0").unwrap();

        let script = format!(
            "n=$(cat {}); echo $((n + 1)) > {}",
            counter.display(),
            counter.display()
        );
        let scripts: HashMap<String, String> =
            [("test_event".to_string(), script)].into_iter().collect();
        // Short debounce so the test doesn't need a long sleep.
        let d = ScriptDispatcher::with_scripts(scripts, Duration::from_millis(50));

        d.fire("test_event", &[]);
        // Wait for the first script to finish and the debounce to expire.
        tokio::time::sleep(Duration::from_millis(300)).await;
        d.fire("test_event", &[]); // should fire again

        tokio::time::sleep(Duration::from_millis(500)).await;
        let n: i32 = std::fs::read_to_string(&counter)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(n, 2, "Both calls should have fired after debounce expired");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_script_does_not_panic() {
        let d = dispatcher_with(&[("test_event", "exit 1")]);
        d.fire("test_event", &[]);
        // Give the spawned task time to observe the non-zero exit.
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
