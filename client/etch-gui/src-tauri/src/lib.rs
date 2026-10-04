mod media;
mod sfx;

use etch_core::init_core;
use etch_core::attachment::{self, Inspection, UploadLimits};
use etch_core::commands::{CoreCommand, MediaRequest};
use etch_core::temp_uploads::TempUploads;
use tauri::{AppHandle, Manager, State};
use tauri::Emitter;
use tauri_plugin_updater::UpdaterExt;
use std::io::Cursor;
use std::path::Path;
use log::LevelFilter;
use simplelog::{CombinedLogger, TermLogger, WriteLogger, ConfigBuilder, TerminalMode, ColorChoice};
use time::macros::format_description;

use sfx::SfxPlayer;

/// The `Option` lets exit drop the sender, which is how the engine learns to flush
/// settings and stop.
pub struct TauriState {
    core_tx: std::sync::Mutex<Option<tokio::sync::mpsc::Sender<CoreCommand>>>,
}

impl TauriState {
    fn new(core_tx: tokio::sync::mpsc::Sender<CoreCommand>) -> Self {
        Self { core_tx: std::sync::Mutex::new(Some(core_tx)) }
    }

    /// Cloned out so the lock is never held across the send's await.
    fn sender(&self) -> Option<tokio::sync::mpsc::Sender<CoreCommand>> {
        self.core_tx.lock().ok().and_then(|guard| guard.clone())
    }

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
async fn paste_clipboard_image(temp_uploads: State<'_, TempUploads>) -> Result<Option<(String, u64)>, String> {
    let temp_uploads = temp_uploads.inner().clone();
    tokio::task::spawn_blocking(move || {
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

        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).map_err(|e| e.to_string())?;
        let bytes = buf.into_inner();
        let size = bytes.len() as u64;
        let path = temp_uploads.create("image.png", &bytes).map_err(|e| e.to_string())?;

        Ok(Some((path.to_string_lossy().into_owned(), size)))
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn inspect_attachment(name: String, size: u64, limits: Option<UploadLimits>) -> Inspection {
    attachment::inspect(&name, size, limits)
}

/// The body is the file's bytes; its percent-encoded name travels in the `file-name` header.
#[tauri::command]
async fn save_pasted_file(
    request: tauri::ipc::Request<'_>,
    temp_uploads: State<'_, TempUploads>,
) -> Result<String, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected the file's bytes".into());
    };
    let name = request.headers().get("file-name")
        .and_then(|value| value.to_str().ok())
        .and_then(media::percent_decode)
        .unwrap_or_default();
    let (temp_uploads, bytes) = (temp_uploads.inner().clone(), bytes.clone());
    tokio::task::spawn_blocking(move || temp_uploads.create(&name, &bytes))
        .await
        .map_err(|e| e.to_string())?
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn discard_temp_upload(path: String, temp_uploads: State<'_, TempUploads>) -> Result<(), String> {
    let temp_uploads = temp_uploads.inner().clone();
    tokio::task::spawn_blocking(move || temp_uploads.discard(Path::new(&path)))
        .await
        .map_err(|e| e.to_string())
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

/// Must cover the engine's own bounded drain.
const ENGINE_SHUTDOWN_WAIT: std::time::Duration = std::time::Duration::from_secs(6);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<CoreCommand>(32);

    // Signalled after the engine has flushed settings; exit waits on it.
    let (engine_done_tx, engine_done_rx) = std::sync::mpsc::sync_channel::<()>(1);

    // Created here because the protocol handler is registered before `setup` runs.
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
                let mxc_url = media::mxc_url(request.uri(), cfg!(windows));

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
            let temp_uploads = TempUploads::new(std::env::temp_dir());
            app.manage(temp_uploads.clone());
            // `setup` runs outside a Tokio context, and `init_core` spawns tasks.
            let (mut core_handle, engine) = tauri::async_runtime::block_on(async {
                init_core(data_dir, resource_dir, cmd_tx, cmd_rx, media_rx, temp_uploads, logger)
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
            inspect_attachment,
            save_pasted_file,
            discard_temp_upload,
            play_sfx,
            load_custom_css,
            check_for_update
        ])
        .build(tauri::generate_context!())
        .expect("Error while building Tauri")
        .run(move |app_handle, event| {
            // Exit is the engine's last chance to write out in-memory settings.
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

#[cfg(test)]
mod tests {
    use tauri::ipc::Origin;

    // Other fs permissions the app holds include `remove`, so only the explicit denial keeps it from the webview.
    #[test]
    fn the_webview_is_never_allowed_to_delete_a_file() {
        let mut context: tauri::Context<tauri::Wry> = tauri::generate_context!(test = true);
        let authority = context.runtime_authority_mut();
        let allowed = |command: &str| authority.resolve_access(command, "main", "main", &Origin::Local).is_some();

        assert!(allowed("plugin:fs|stat"), "the composer reads a picked file's size, so this command name resolves");
        assert!(!allowed("plugin:fs|remove"), "deleting is the shell's alone, and only for files Etch created");
    }
}
