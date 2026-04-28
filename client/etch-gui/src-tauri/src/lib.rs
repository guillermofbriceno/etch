mod sfx;

use etch_core::init_core;
use etch_core::commands::CoreCommand;
use tauri::{AppHandle, Manager, State};
use tauri::Emitter;
use tauri_plugin_updater::UpdaterExt;
use std::io::Cursor;

use sfx::SfxPlayer;

pub struct TauriState {
    pub core_tx: tokio::sync::mpsc::Sender<CoreCommand>,
}

#[tauri::command]
async fn core_command(command: CoreCommand, state: State<'_, TauriState>) -> Result<(), String> {
    state.core_tx.send(command).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn play_sfx(name: String, volume: f32, state: State<'_, SfxPlayer>) {
    state.play(&name, volume);
}

#[tauri::command]
fn paste_clipboard_image() -> Result<Option<String>, String> {
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
    std::fs::write(&path, buf.into_inner()).map_err(|e| e.to_string())?;

    Ok(Some(path.to_string_lossy().into_owned()))
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Create the command channel early so the protocol handler
    // (registered on the Builder, before setup) can send FetchMedia.
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<CoreCommand>(32);
    let core_tx_for_protocol = cmd_tx.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .register_asynchronous_uri_scheme_protocol("etch-media", move |_ctx, request, responder| {
            let core_tx = core_tx_for_protocol.clone();
            tauri::async_runtime::spawn(async move {
                let uri = request.uri();
                let host = uri.host().unwrap_or_default();
                let media_id = uri.path().trim_start_matches('/');
                let mxc_url = format!("mxc://{}/{}", host, media_id);

                let (tx, rx) = tokio::sync::oneshot::channel();
                let _ = core_tx.send(CoreCommand::FetchMedia { mxc_url, respond: tx }).await;

                match rx.await {
                    Ok(Ok(bytes)) => {
                        responder.respond(
                            tauri::http::Response::builder()
                                .header("content-type", "application/octet-stream")
                                .body(bytes)
                                .unwrap()
                        );
                    }
                    Ok(Err(e)) => {
                        let body = format!("Media fetch error: {e}").into_bytes();
                        responder.respond(
                            tauri::http::Response::builder()
                                .status(502)
                                .body(body)
                                .unwrap()
                        );
                    }
                    Err(_) => {
                        responder.respond(
                            tauri::http::Response::builder()
                                .status(502)
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
            let (mut core_handle, engine) = init_core(data_dir, resource_dir, cmd_tx, cmd_rx);
            app.manage(TauriState { core_tx: core_handle.cmd_tx });
            app.manage(SfxPlayer::new());

            tauri::async_runtime::spawn(async move {
                engine.run().await;
            });

            tauri::async_runtime::spawn(async move {
                while let Some(event) = core_handle.event_rx.recv().await {
                    app_handle.emit("core_event", &event).unwrap();
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core_command,
            paste_clipboard_image,
            play_sfx,
            check_for_update
        ])
        .run(tauri::generate_context!())
        .expect("Error while running Tauri");
}
