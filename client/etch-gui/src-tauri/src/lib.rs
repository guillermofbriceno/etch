mod sfx;

use etch_core::init_core;
use etch_core::commands::{CoreCommand, MediaRequest};
use tauri::{AppHandle, Manager, State};
use tauri::Emitter;
use tauri_plugin_updater::UpdaterExt;
use std::io::Cursor;
use std::path::Path;
use log::LevelFilter;
use simplelog::{CombinedLogger, TermLogger, WriteLogger, ConfigBuilder, TerminalMode, ColorChoice};
use time::macros::format_description;

use sfx::SfxPlayer;

/// The command channel's sending end, held for the life of the process.
///
/// Wrapped in an `Option` so that exiting can drop it. That is the only way
/// the engine learns the app is closing: `run()` returns when this channel
/// closes, and the settings it holds in memory are flushed on the way out.
/// While it stayed alive to the end of the process, that flush never ran.
pub struct TauriState {
    core_tx: std::sync::Mutex<Option<tokio::sync::mpsc::Sender<CoreCommand>>>,
}

impl TauriState {
    fn new(core_tx: tokio::sync::mpsc::Sender<CoreCommand>) -> Self {
        Self { core_tx: std::sync::Mutex::new(Some(core_tx)) }
    }

    /// A sender to use for one command. Cloned out rather than borrowed so
    /// the lock is never held across the send's await.
    fn sender(&self) -> Option<tokio::sync::mpsc::Sender<CoreCommand>> {
        self.core_tx.lock().ok().and_then(|guard| guard.clone())
    }

    /// Close the command channel, which is what tells the engine to shut down.
    fn close(&self) {
        if let Ok(mut guard) = self.core_tx.lock() {
            *guard = None;
        }
    }
}

#[tauri::command]
async fn core_command(command: CoreCommand, state: State<'_, TauriState>) -> Result<(), String> {
    let tx = state.sender().ok_or("core is shutting down")?;
    tx.send(command).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn play_sfx(name: String, volume: f32, state: State<'_, SfxPlayer>) {
    state.play(&name, volume);
}

#[tauri::command]
fn load_custom_css(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read CSS from {path}: {e}"))
}

#[tauri::command]
async fn paste_clipboard_image() -> Result<Option<(String, u64)>, String> {
    tokio::task::spawn_blocking(|| {
        let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        let img_data = match clipboard.get_image() {
            Ok(data) => data,
            Err(_) => return Ok(None),
        };

        let img = image::RgbaImage::from_raw(
            img_data.width as u32,
            img_data.height as u32,
            img_data.bytes.into_owned(),
        ).ok_or("Failed to create image from clipboard data")?;

        let path = std::env::temp_dir().join(format!("etch-paste-{}.png", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()));

        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).map_err(|e| e.to_string())?;
        let bytes = buf.into_inner();
        let size = bytes.len() as u64;
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;

        Ok(Some((path.to_string_lossy().into_owned(), size)))
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn compress_image(path: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let data = std::fs::read(&path).map_err(|e| e.to_string())?;

        let img = image::ImageReader::new(Cursor::new(&data))
            .with_guessed_format()
            .map_err(|e| e.to_string())?
            .decode()
            .map_err(|e| e.to_string())?;

        let stem = Path::new(&path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("image");

        let out_path = std::env::temp_dir().join(format!("etch-paste-{}.jpg", stem));

        let mut buf = Cursor::new(Vec::new());
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 80);
        img.write_with_encoder(encoder).map_err(|e| e.to_string())?;
        std::fs::write(&out_path, buf.into_inner()).map_err(|e| e.to_string())?;

        // Clean up the original temp file if it was from a paste
        if Path::new(&path)
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|f| f.starts_with("etch-paste-"))
        {
            let _ = std::fs::remove_file(&path);
        }

        Ok(out_path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Clone, serde::Serialize)]
#[serde(tag = "type", content = "data")]
enum UpdateEvent {
    Available { version: String },
    Ready,
    UpToDate,
}

#[tauri::command]
async fn check_for_update(app: AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater.check().await.map_err(|e| e.to_string())?;

    match update {
        Some(update) => {
            let version = update.version.clone();
            app.emit("update_event", UpdateEvent::Available { version }).unwrap();

            let app_handle = app.clone();
            update.download_and_install(
                |_chunk_length, _content_length| {},
                move || {
                    app_handle.emit("update_event", UpdateEvent::Ready).unwrap();
                },
            ).await.map_err(|e| e.to_string())?;
        }
        None => {
            app.emit("update_event", UpdateEvent::UpToDate).unwrap();
        }
    }

    Ok(())
}

/// Truncate the log file if it exceeds the size limit, keeping the tail.
/// Writes to a temporary file first, then renames for atomicity.
fn rotate_log_file(log_path: &Path) {
    const MAX_LOG_SIZE: u64 = 2 * 1024 * 1024;
    const KEEP_BYTES: usize = 1024 * 1024;

    let meta = match std::fs::metadata(log_path) {
        Ok(m) => m,
        Err(_) => return,
    };
    if meta.len() <= MAX_LOG_SIZE {
        return;
    }
    let contents = match std::fs::read(log_path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let tail = &contents[contents.len().saturating_sub(KEEP_BYTES)..];
    let start = tail.iter().position(|&b| b == b'\n').map(|i| i + 1).unwrap_or(0);

    let tmp_path = log_path.with_extension("log.tmp");
    if std::fs::write(&tmp_path, &tail[start..]).is_ok() {
        let _ = std::fs::rename(&tmp_path, log_path);
    }
}

/// Build the combined logger (terminal + file).
fn build_logger(log_path: &Path) -> Box<dyn log::Log> {
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .expect("Failed to open log file");

    let log_config = ConfigBuilder::default()
        .set_target_level(LevelFilter::Error)
        .set_time_format_custom(format_description!(
            "[year]-[month]-[day] [hour]:[minute]:[second]"
        ))
        .set_time_offset_to_local()
        .unwrap_or_else(|b| b)
        .build();

    // The global max-level gate is set by ForwardingLogger::init in
    // etch-core (ETCH_LOG env / build profile). Both backends pass
    // everything the global gate allows.
    CombinedLogger::new(vec![
        TermLogger::new(LevelFilter::max(), log_config.clone(), TerminalMode::Stderr, ColorChoice::Auto),
        WriteLogger::new(LevelFilter::max(), log_config, log_file),
    ])
}

/// Register GTK enter/leave events so the frontend can suppress sidebar
/// peek when the cursor leaves the window.
#[cfg(target_os = "linux")]
fn setup_cursor_events(app: &tauri::App) {
    let Some(win) = app.get_webview_window("main") else {
        log::warn!("Could not find main window for cursor events");
        return;
    };
    let leave_handle = app.handle().clone();
    let enter_handle = app.handle().clone();
    let result = win.with_webview(move |webview| {
        use gtk::prelude::{WidgetExt, WidgetExtManual};
        let widget = webview.inner();
        if let Some(toplevel) = widget.toplevel() {
            toplevel.add_events(
                gtk::gdk::EventMask::ENTER_NOTIFY_MASK | gtk::gdk::EventMask::LEAVE_NOTIFY_MASK,
            );
            toplevel.connect_leave_notify_event(move |_, _| {
                let _ = leave_handle.emit("cursor-left-window", ());
                gtk::glib::Propagation::Proceed
            });
            toplevel.connect_enter_notify_event(move |_, _| {
                let _ = enter_handle.emit("cursor-entered-window", ());
                gtk::glib::Propagation::Proceed
            });
        }
    });
    if let Err(e) = result {
        log::warn!("Failed to set up GTK cursor events: {:?}", e);
    }
}

/// How long exiting waits for the engine to finish shutting down.
///
/// The engine bounds its own drain, so this only has to be long enough to
/// cover it. When nothing is in flight -- the ordinary case -- the engine
/// returns immediately and this wait costs nothing.
const ENGINE_SHUTDOWN_WAIT: std::time::Duration = std::time::Duration::from_secs(6);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Control commands from the UI. Small on purpose: these are user actions,
    // and a backlog of them means something is wrong.
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<CoreCommand>(32);

    // Signalled once the engine's `run()` has returned, which is after it has
    // flushed settings. Exiting waits on this so the flush actually happens.
    let (engine_done_tx, engine_done_rx) = std::sync::mpsc::sync_channel::<()>(1);

    // Media fetches, on their own channel. The `etch-media` protocol handler
    // raises one per image the webview loads, so a freshly opened room can
    // burst well past the control channel's capacity; sharing a queue with UI
    // commands meant those bursts blocked `invoke`. Created here rather than
    // in `init_core` because the handler is registered on the Builder, before
    // `setup` runs.
    let (media_tx, media_rx) = tokio::sync::mpsc::channel::<MediaRequest>(256);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .register_asynchronous_uri_scheme_protocol("etch-media", move |_ctx, request, responder| {
            let media_tx = media_tx.clone();
            tauri::async_runtime::spawn(async move {
                let uri = request.uri();
                let raw_host = uri.host().unwrap_or_default();
                let raw_path = uri.path().trim_start_matches('/');

                // On Windows, wry reverts http://<scheme>.localhost/<path>
                // back to <scheme>://localhost/<path>, so the URI host is
                // "localhost" and the real Matrix server is the first path segment.
                let mxc_url = if raw_host == "localhost" {
                    format!("mxc://{}", raw_path)
                } else {
                    format!("mxc://{}/{}", raw_host, raw_path)
                };

                let (tx, rx) = tokio::sync::oneshot::channel();
                let _ = media_tx.send(MediaRequest { mxc_url, respond: tx }).await;

                match rx.await {
                    Ok(Ok(bytes)) => {
                        responder.respond(
                            tauri::http::Response::builder()
                                .header("content-type", "application/octet-stream")
                                .header("access-control-allow-origin", "*")
                                .body(bytes)
                                .unwrap()
                        );
                    }
                    Ok(Err(e)) => {
                        let body = format!("Media fetch error: {e}").into_bytes();
                        responder.respond(
                            tauri::http::Response::builder()
                                .status(502)
                                .header("access-control-allow-origin", "*")
                                .body(body)
                                .unwrap()
                        );
                    }
                    Err(_) => {
                        responder.respond(
                            tauri::http::Response::builder()
                                .status(502)
                                .header("access-control-allow-origin", "*")
                                .body(b"Media fetch channel closed".to_vec())
                                .unwrap()
                        );
                    }
                }
            });
        })
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let data_dir = app.path().app_data_dir().expect("Failed to get app data dir");
            let resource_dir = app.path().resource_dir().expect("Failed to get resource dir");

            std::fs::create_dir_all(&data_dir).expect("Failed to create data dir");
            let log_path = data_dir.join("etch.log");
            rotate_log_file(&log_path);
            let logger = build_logger(&log_path);

            let sfx_player = SfxPlayer::new(&data_dir);
            // Inside the runtime: building the engine spawns a task per
            // subsystem, and `setup` runs on the main thread, which is not
            // otherwise in a Tokio context.
            let (mut core_handle, engine) = tauri::async_runtime::block_on(async {
                init_core(data_dir, resource_dir, cmd_tx, cmd_rx, media_rx, logger)
            });
            app.manage(TauriState::new(core_handle.cmd_tx));
            app.manage(sfx_player);

            tauri::async_runtime::spawn(async move {
                engine.run().await;
                let _ = engine_done_tx.send(());
            });

            tauri::async_runtime::spawn(async move {
                while let Some(event) = core_handle.event_rx.recv().await {
                    app_handle.emit("core_event", &event).unwrap();
                }
            });

            #[cfg(target_os = "linux")]
            setup_cursor_events(app);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core_command,
            paste_clipboard_image,
            compress_image,
            play_sfx,
            load_custom_css,
            check_for_update
        ])
        .build(tauri::generate_context!())
        .expect("Error while building Tauri")
        .run(move |app_handle, event| {
            // Quitting is the only chance the engine gets to write out
            // settings that are still only in memory, so the channel it reads
            // from has to be closed and the engine given a moment to drain.
            if let tauri::RunEvent::Exit = event {
                log::info!("Exiting: closing the core command channel");
                app_handle.state::<TauriState>().close();
                let waiting_since = std::time::Instant::now();
                if engine_done_rx.recv_timeout(ENGINE_SHUTDOWN_WAIT).is_err() {
                    log::warn!(
                        "Engine did not shut down within {:?}; exiting anyway",
                        ENGINE_SHUTDOWN_WAIT,
                    );
                } else {
                    log::info!("Engine shut down in {:?}", waiting_since.elapsed());
                }
            }
        });
}
