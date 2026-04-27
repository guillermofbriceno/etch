mod sfx;

use etch_core::init_core;
use etch_core::commands::CoreCommand;
use etch_core::events::CoreEvent;
use tauri::{AppHandle, Manager, State};
use tauri::Emitter;
use tauri_plugin_updater::UpdaterExt;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use sfx::SfxPlayer;

pub struct TauriState {
    pub core_tx: tokio::sync::mpsc::Sender<CoreCommand>,
}

/// Shared state for the media proxy protocol handler.
/// Updated when the backend emits `HomeserverResolved`.
pub struct MediaProxyState {
    homeserver_url: Mutex<Option<String>>,
    http_client: reqwest::Client,
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

fn update_media_proxy_url(proxy: &MediaProxyState, event: &CoreEvent) {
    if let CoreEvent::Matrix(etch_core::events::MatrixEvent::HomeserverResolved(url)) = event {
        *proxy.homeserver_url.lock().unwrap() = Some(url.clone());
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let media_proxy = Arc::new(MediaProxyState {
        homeserver_url: Mutex::new(None),
        http_client: reqwest::Client::new(),
    });

    let proxy_for_protocol = media_proxy.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .register_asynchronous_uri_scheme_protocol("etch-media", move |_ctx, request, responder| {
            let proxy = proxy_for_protocol.clone();
            tauri::async_runtime::spawn(async move {
                let result = handle_media_request(&proxy, &request).await;
                match result {
                    Ok(response) => responder.respond(response),
                    Err(e) => {
                        let body = format!("Media proxy error: {e}").into_bytes();
                        responder.respond(
                            tauri::http::Response::builder()
                                .status(502)
                                .body(body)
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
            let (mut core_handle, engine) = init_core(data_dir, resource_dir);
            app.manage(TauriState { core_tx: core_handle.cmd_tx });
            app.manage(SfxPlayer::new());

            tauri::async_runtime::spawn(async move {
                engine.run().await;
            });

            let proxy_for_events = media_proxy.clone();
            tauri::async_runtime::spawn(async move {
                while let Some(event) = core_handle.event_rx.recv().await {
                    update_media_proxy_url(&proxy_for_events, &event);
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

/// Handles `etch-media://server_name/media_id` requests by proxying
/// to the Matrix media endpoint via the Rust HTTP client.
async fn handle_media_request(
    proxy: &MediaProxyState,
    request: &tauri::http::Request<Vec<u8>>,
) -> Result<tauri::http::Response<Vec<u8>>, String> {
    let homeserver = proxy.homeserver_url.lock().unwrap().clone()
        .ok_or("Homeserver URL not yet resolved")?;

    // URL format: etch-media://server_name/media_id
    let uri = request.uri();
    let host = uri.host().ok_or("Missing server_name in media URL")?;
    let media_id = uri.path().trim_start_matches('/');
    if media_id.is_empty() {
        return Err("Missing media_id in media URL".into());
    }

    let url = format!(
        "{}/_matrix/media/v3/download/{}/{}",
        homeserver, host, media_id,
    );

    let resp = proxy.http_client.get(&url).send().await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    let status = resp.status().as_u16();
    let content_type = resp.headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let bytes = resp.bytes().await
        .map_err(|e| format!("Failed to read response body: {e}"))?;

    tauri::http::Response::builder()
        .status(status)
        .header("content-type", content_type)
        .body(bytes.to_vec())
        .map_err(|e| format!("Failed to build response: {e}"))
}
