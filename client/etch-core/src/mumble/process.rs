use std::path::{Path, PathBuf};
use std::process::Stdio;
use sha1::{Sha1, Digest};
use snafu::ResultExt;
use bridge_types::MumbleCommand as BridgeCommand;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use crate::error::*;
use crate::events::{CoreEvent, InternalEvent};

pub struct MumbleProcess {
    child: Child,
    pub sock_name: String,
    pub cmd_tx: mpsc::Sender<BridgeCommand>,
    _bridge_handle: tokio::task::JoinHandle<()>,
    /// On Windows, holds the Job Object that kills Mumble when etch exits.
    /// Must stay alive for the duration of the process.
    #[cfg(target_os = "windows")]
    _job: win32job::Job,
}

impl MumbleProcess {
    /// Start the bridge listener, then spawn the Mumble client pointed at the
    /// given server.  Returns once the process is running (not once it connects).
    pub async fn spawn(
        host: &str,
        port: u16,
        username: &str,
        password: Option<&str>,
        event_tx: mpsc::Sender<CoreEvent>,
        internal_tx: mpsc::Sender<InternalEvent>,
        show_gui: bool,
        extra_args: &str,
        data_dir: &Path,
        resource_dir: &Path,
    ) -> Result<Self, CoreError> {
        // 1. Build mumble:// URL
        let url = match password {
            Some(pw) => format!("mumble://{}:{}@{}:{}", username, pw, host, port),
            None => format!("mumble://{}@{}:{}", username, host, port),
        };

        log::info!("Spawning Mumble: {} (gui={})", url, show_gui);

        // 2. Ensure mumble config dir exists with seed files (before starting bridge
        //    so a failure here doesn't leave an orphaned socket)
        let mumble_config_dir = data_dir.join("mumble");
        init_mumble_config(&mumble_config_dir, resource_dir)?;

        // 3. Start bridge listener
        let (sock_name, cmd_tx, bridge_handle) = crate::mumble::bridge::start(event_tx, internal_tx)
            .context(BridgeStartSnafu)?;

        // 4. Parse extra args
        let extra: Vec<&str> = extra_args.split_whitespace().filter(|s| !s.is_empty()).collect();

        // 5. Spawn Mumble with the bridge socket env var
        let mumble_path = mumble_bin(resource_dir);
        let path = mumble_path.display().to_string();
        let mut child = spawn_child(&mumble_path, &url, &sock_name, show_gui, &extra, &mumble_config_dir)
            .context(MumbleSpawnSnafu { path })?;

        log::info!("Mumble process started (pid {})", child.id().unwrap_or(0));

        // Log Mumble's stdout and stderr output
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(async move {
                let reader = tokio::io::BufReader::new(stdout);
                let mut lines = tokio::io::AsyncBufReadExt::lines(reader);
                while let Ok(Some(line)) = lines.next_line().await {
                    log::debug!("[mumble] {}", line);
                }
            });
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let reader = tokio::io::BufReader::new(stderr);
                let mut lines = tokio::io::AsyncBufReadExt::lines(reader);
                while let Ok(Some(line)) = lines.next_line().await {
                    log::warn!("[mumble] {}", line);
                }
            });
        }

        // 6. Platform-specific: ensure Mumble dies when etch dies
        #[cfg(target_os = "windows")]
        let _job = {
            let mut job = win32job::Job::create()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
                .context(MumbleSpawnSnafu { path: "win32job" })?;
            let mut info = job.query_extended_limit_info()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
                .context(MumbleSpawnSnafu { path: "win32job" })?;
            info.limit_kill_on_job_close();
            job.set_extended_limit_info(&mut info)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
                .context(MumbleSpawnSnafu { path: "win32job" })?;
            let pid = child.id().unwrap_or(0);
            let handle = unsafe {
                windows_sys::Win32::System::Threading::OpenProcess(
                    windows_sys::Win32::System::Threading::PROCESS_ALL_ACCESS,
                    0, // don't inherit
                    pid,
                )
            };
            if handle.is_null() {
                return Err(std::io::Error::last_os_error())
                    .context(MumbleSpawnSnafu { path: "win32job: OpenProcess" });
            }
            job.assign_process(handle as isize)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
                .context(MumbleSpawnSnafu { path: "win32job" })?;
            job
        };

        Ok(Self {
            child,
            sock_name,
            cmd_tx,
            _bridge_handle: bridge_handle,
            #[cfg(target_os = "windows")]
            _job,
        })
    }

    pub async fn kill(&mut self) {
        let _ = self.child.kill().await;
        self._bridge_handle.abort();
        log::info!("Mumble process killed");
    }
}

impl Drop for MumbleProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        self._bridge_handle.abort();
    }
}

/// Copy seed config files into the mumble config dir if they don't already exist,
/// then patch absolute paths into the config.
fn init_mumble_config(config_dir: &Path, resource_dir: &Path) -> Result<(), CoreError> {
    std::fs::create_dir_all(config_dir)
        .context(CreateDirSnafu { path: config_dir })?;

    // Copy seed files
    let seed_dir = seed_resource_dir(resource_dir);
    for filename in &["mumble-conf.json", "mumble.sqlite"] {
        let dest = config_dir.join(filename);
        if !dest.exists() {
            let src = seed_dir.join(filename);
            if src.exists() {
                std::fs::copy(&src, &dest)
                    .context(CopyFileSnafu { src: &src, dest: &dest })?;
                log::info!("Seeded mumble config: {}", dest.display());
            }
        }
    }

    // Install plugin
    let plugins_dir = mumble_plugins_dir(config_dir);
    let plugin_dest = plugins_dir.join(BRIDGE_LIB);
    let plugin_src = plugin_build_path(resource_dir);
    let plugin_src = plugin_src.canonicalize()
        .context(PluginNotFoundSnafu { path: &plugin_src })?;

    std::fs::create_dir_all(&plugins_dir)
        .context(CreateDirSnafu { path: &plugins_dir })?;
    std::fs::copy(&plugin_src, &plugin_dest)
        .context(CopyFileSnafu { src: &plugin_src, dest: &plugin_dest })?;
    log::info!("Installed plugin → {}", plugin_dest.display());

    // Patch config JSON
    let conf_path = config_dir.join("mumble-conf.json");
    if !conf_path.exists() {
        return Ok(());
    }

    let contents = std::fs::read_to_string(&conf_path)
        .context(ReadFileSnafu { path: &conf_path })?;

    let mut json: serde_json::Value = serde_json::from_str(&contents)
        .context(ParseConfigSnafu { path: &conf_path })?;

    let mut changed = false;

    // Patch database_location
    let db_path = config_dir.join("mumble.sqlite");
    if let Some(misc) = json.get_mut("misc").and_then(|m| m.as_object_mut()) {
        let current = misc.get("database_location").and_then(|v| v.as_str()).unwrap_or("");
        if current.is_empty() || current != db_path.to_string_lossy().as_ref() {
            misc.insert("database_location".to_string(), serde_json::Value::String(db_path.to_string_lossy().into_owned()));
            log::info!("Patched database_location → {}", db_path.display());
            changed = true;
        }
    }

    // Register plugin in config
    let plugin_canonical = plugin_dest.canonicalize()
        .context(ReadFileSnafu { path: &plugin_dest })?;
    // On Windows, canonicalize() adds a \\?\ prefix and uses backslashes.
    // Mumble (Qt) uses forward slashes everywhere, so we must match that
    // for both the stored path and the SHA1 hash to agree with Mumble's.
    let path_str = plugin_canonical.to_string_lossy().to_string();
    #[cfg(target_os = "windows")]
    let path_str = path_str.strip_prefix(r"\\?\").unwrap_or(&path_str).replace('\\', "/");
    let hash = format!("{:x}", Sha1::digest(path_str.as_bytes()));

    let root = json.as_object_mut()
        .ok_or_else(|| InvalidConfigSnafu { message: format!("Expected JSON object in '{}'", conf_path.display()) }.build())?;
    let plugins = root
        .entry("plugins")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let plugins = plugins.as_object_mut()
        .ok_or_else(|| InvalidConfigSnafu { message: format!("Expected 'plugins' to be an object in '{}'", conf_path.display()) }.build())?;

    if !plugins.contains_key(&hash) {
        plugins.insert(hash.clone(), serde_json::json!({
            "enabled": true,
            "keyboard_monitoring_allowed": false,
            "path": path_str,
            "positional_data_enabled": false
        }));
        log::info!("Registered plugin {} → {}", hash, path_str);
        changed = true;
    }

    // Fix dirty shutdown flag to prevent Mumble's backup prompt dialog
    if json.get("mumble_has_quit_normally") == Some(&serde_json::Value::Bool(false)) {
        log::error!("Mumble did not quit normally last time — resetting flag to suppress backup prompt");
        json["mumble_has_quit_normally"] = serde_json::Value::Bool(true);
        changed = true;
    }

    // Atomic write via tmp + rename
    if changed {
        let patched = serde_json::to_string_pretty(&json)
            .map_err(|e| InvalidConfigSnafu { message: format!("Serializing config: {}", e) }.build())?;
        let tmp_path = config_dir.join("mumble-conf.json.tmp");
        std::fs::write(&tmp_path, &patched)
            .context(WriteFileSnafu { path: &tmp_path })?;
        std::fs::rename(&tmp_path, &conf_path)
            .context(RenameFileSnafu { src: &tmp_path, dest: &conf_path })?;
    }

    Ok(())
}

/// Returns the path to the seed config files (mumble-conf.json, mumble.sqlite).
/// In debug builds, uses the source tree; in release, uses the Tauri resource directory.
fn seed_resource_dir(resource_dir: &Path) -> PathBuf {
    if cfg!(debug_assertions) {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/resources"))
    } else {
        resource_dir.join("resources")
    }
}

/// Plugin library filename (platform-specific).
const BRIDGE_LIB: &str = if cfg!(target_os = "windows") { "etch_bridge.dll" } else { "libetch_bridge.so" };

/// Returns the path to the built plugin shared library (source for copying).
fn plugin_build_path(resource_dir: &Path) -> PathBuf {
    if cfg!(debug_assertions) {
        PathBuf::from(format!("{}/../../target/debug/{}", env!("CARGO_MANIFEST_DIR"), BRIDGE_LIB))
    } else {
        resource_dir.join(format!("bundled/{}", BRIDGE_LIB))
    }
}

/// Returns the Mumble plugins directory where the plugin must be installed.
/// This must be inside the etch-controlled config dir so Mumble (launched
/// with -c pointing there) scans it on startup.
fn mumble_plugins_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("Plugins")
}

/// Returns the path to the Mumble binary.
/// In debug builds, uses the local dev build; in release, uses the bundled binary.
fn mumble_bin(resource_dir: &Path) -> PathBuf {
    if cfg!(debug_assertions) {
        if cfg!(target_os = "windows") {
            PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../mumble/build/mumble.exe"))
        } else {
            PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../mumble/build/mumble"))
        }
    } else if cfg!(target_os = "windows") {
        // Windows portable ZIP has flat structure (no bin/ subdir)
        resource_dir.join("bundled/mumble/mumble.exe")
    } else {
        resource_dir.join("bundled/mumble/bin/mumble")
    }
}

// --- Platform-specific spawn ---

#[cfg(unix)]
fn spawn_child(mumble_path: &Path, url: &str, sock_name: &str, show_gui: bool, extra: &[&str], config_dir: &Path) -> std::io::Result<Child> {
    let config_file = config_dir.join("mumble-conf.json");

    // Point Qt at the bundled plugins and shared libs (e.g. bundled/mumble/plugins, bundled/mumble/lib)
    let mumble_root = mumble_path.parent().and_then(|p| p.parent()).unwrap_or(Path::new("."));
    let qt_plugin_path = mumble_root.join("plugins");
    let lib_path = mumble_root.join("lib");
    let ld_library_path = match std::env::var("LD_LIBRARY_PATH") {
        Ok(existing) => format!("{}:{}", lib_path.display(), existing),
        Err(_) => lib_path.display().to_string(),
    };

    let mut cmd = Command::new(mumble_path);
    if !show_gui {
        cmd.arg("-platform").arg("offscreen");
    }
    cmd.arg(url)
        .arg ("--skip-settings-backup-prompt")
        .arg("-c").arg(&config_file)
        .arg("-m")
        .args(extra)
        .env("ETCH_BRIDGE_SOCK", sock_name)
        .env("QT_PLUGIN_PATH", &qt_plugin_path)
        .env("LD_LIBRARY_PATH", &ld_library_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
        cmd.spawn()
    }
}

#[cfg(windows)]
fn spawn_child(mumble_path: &Path, url: &str, sock_name: &str, show_gui: bool, extra: &[&str], config_dir: &Path) -> std::io::Result<Child> {
    let config_file = config_dir.join("mumble-conf.json");
    let mut cmd = Command::new(mumble_path);
    if !show_gui {
        cmd.arg("-platform").arg("offscreen");
    }
    cmd.arg(url)
        .arg ("--skip-settings-backup-prompt")
        .arg("-c").arg(&config_file)
        .arg("-m")
        .args(extra)
        .env("ETCH_BRIDGE_SOCK", sock_name)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}
