use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use matrix_sdk::Client;
use matrix_sdk::attachment::{
    AttachmentConfig, AttachmentInfo, BaseAudioInfo, BaseFileInfo, BaseImageInfo, BaseVideoInfo,
};
use matrix_sdk::media::MediaError;
use matrix_sdk::ruma::{RoomId, UInt};
use matrix_sdk::send_queue::RoomSendQueueError;
use mime_guess::mime::{self, Mime};
use serde::Serialize;

use crate::commands::OutgoingMediaInfo;
use crate::matrix::compress::{self, OutputFormat};
use crate::temp_files::TempFiles;

const MIB: u64 = 1024 * 1024;
const COMPRESS_THRESHOLD_BYTES: u64 = 256_000;
const LIMIT_WAIT: Duration = Duration::from_secs(5);

pub(crate) const NOT_CONNECTED: &str = "not connected to the server";
const UNREADABLE: &str = "the file could not be read";
const UNKNOWN_ROOM: &str = "the room could not be found";
const NOT_JOINED: &str = "you are not in this room";
const UPLOAD_FAILED: &str = "the upload failed";
const HELD_FOR_RETRY: &str = "the server could not be reached; it will be retried when you next send to this room";
const NOT_ACCEPTED: &str = "the server did not accept it";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Inspection {
    pub mimetype: String,
    pub compress_offered: bool,
}

/// What the composer needs to know about a file before it is sent.
pub fn inspect(file_name: &str, size: u64) -> Inspection {
    let content_type = sanitize_mime(Path::new(file_name));
    Inspection {
        compress_offered: compress_offered(&content_type, size),
        mimetype: content_type.essence_str().to_owned(),
    }
}

fn compress_offered(content_type: &Mime, size: u64) -> bool {
    let compressible = content_type.type_() == mime::IMAGE
        && matches!(content_type.subtype().as_str(), "jpeg" | "png" | "webp" | "bmp" | "tiff");
    compressible && size > COMPRESS_THRESHOLD_BYTES
}

pub(crate) fn format_mb(bytes: u64) -> String {
    if bytes.is_multiple_of(MIB) {
        return format!("{} MB", bytes / MIB);
    }
    let tenths = (u128::from(bytes) * 10 + u128::from(MIB / 2)) / u128::from(MIB);
    format!("{}.{} MB", tenths / 10, tenths % 10)
}

fn over_limit_reason(size: u64, limit: u64, after_compression: bool) -> String {
    format!(
        "it is {}{} and the server's limit is {}",
        format_mb(size),
        if after_compression { " after compression" } else { "" },
        format_mb(limit),
    )
}

/// `None` is a limit the homeserver has not told us, which leaves the check to the SDK.
fn check_limit(size: u64, limit: Option<u64>, after_compression: bool) -> Result<(), String> {
    match limit {
        Some(limit) if size > limit => Err(over_limit_reason(size, limit, after_compression)),
        _ => Ok(()),
    }
}

/// For a failure the send queue reports after it has taken the request, which may be a text message.
pub(crate) fn send_failure_reason(err: &matrix_sdk::Error, is_recoverable: bool) -> String {
    match err {
        matrix_sdk::Error::Media(MediaError::MediaTooLargeToUpload { max, current }) => {
            over_limit_reason(u64::from(*current), u64::from(*max), false)
        }
        _ if is_recoverable => HELD_FOR_RETRY.into(),
        _ => NOT_ACCEPTED.into(),
    }
}

/// `mime_guess` maps several source-code extensions to media types (`.ts` becomes
/// `video/mp2t`), so anything outside a small allowlist is sent as a plain file.
pub(crate) fn sanitize_mime(path: &Path) -> Mime {
    let guess = mime_guess::from_path(path).first_or_octet_stream();
    // Non-standard names for MP4 audio and video, which the allowlist below would demote.
    let guess = match guess.essence_str() {
        "audio/m4a" => "audio/mp4".parse().expect("a valid media type"),
        "video/x-m4v" => "video/mp4".parse().expect("a valid media type"),
        _ => guess,
    };

    let is_misidentified = match (guess.type_(), guess.subtype().as_str()) {
        (mime::VIDEO, sub) => !matches!(sub, "mp4" | "webm" | "ogg" | "quicktime" | "x-matroska" | "x-msvideo" | "mpeg"),
        (mime::AUDIO, sub) => !matches!(sub, "mpeg" | "ogg" | "wav" | "webm" | "aac" | "flac" | "mp4" | "x-flac"),
        _ => false,
    };

    if is_misidentified {
        mime::APPLICATION_OCTET_STREAM
    } else {
        guess
    }
}

pub(crate) async fn discard(temp_files: &TempFiles, path: &Path) {
    let (temp_files, path) = (temp_files.clone(), path.to_owned());
    let _ = tokio::task::spawn_blocking(move || temp_files.discard(&path)).await;
}

/// Keyed on the top-level type because that is what the SDK picks the message type
/// from, and it discards info that does not match.
pub(crate) fn attachment_info(
    content_type: &Mime,
    size: u64,
    media: Option<&OutgoingMediaInfo>,
) -> AttachmentInfo {
    let size = UInt::new(size);
    let width = media.and_then(|m| m.width).map(UInt::from);
    let height = media.and_then(|m| m.height).map(UInt::from);
    let duration = media.and_then(|m| m.duration_ms).map(Duration::from_millis);
    match content_type.type_() {
        mime::IMAGE => AttachmentInfo::Image(BaseImageInfo { width, height, size, ..Default::default() }),
        mime::VIDEO => AttachmentInfo::Video(BaseVideoInfo { duration, width, height, size, ..Default::default() }),
        mime::AUDIO => AttachmentInfo::Audio(BaseAudioInfo { duration, size, ..Default::default() }),
        _ => AttachmentInfo::File(BaseFileInfo { size }),
    }
}

pub(crate) fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".into())
}

#[derive(Debug)]
pub(crate) struct Attachment {
    pub file_name: String,
    pub content_type: Mime,
    pub data: Vec<u8>,
    pub info: AttachmentInfo,
}

/// Discards the file whatever the outcome, which only removes one Etch created.
pub(crate) async fn prepare(
    path: &Path,
    compress_requested: bool,
    server_limit: Option<u64>,
    media: Option<OutgoingMediaInfo>,
    temp_files: &TempFiles,
) -> Result<Attachment, String> {
    let prepared = prepare_file(path, compress_requested, server_limit, media).await;
    discard(temp_files, path).await;
    prepared
}

async fn prepare_file(
    path: &Path,
    compress_requested: bool,
    server_limit: Option<u64>,
    media: Option<OutgoingMediaInfo>,
) -> Result<Attachment, String> {
    let content_type = sanitize_mime(path);
    let size = tokio::fs::metadata(path).await.map_err(|e| unreadable(path, e))?.len();
    let should_compress = compress_requested && compress_offered(&content_type, size);
    // A file that will not get smaller is refused before it is read into memory.
    if !should_compress {
        check_limit(size, server_limit, false)?;
    }
    let data = tokio::fs::read(path).await.map_err(|e| unreadable(path, e))?;

    let (data, compressed) = if should_compress {
        on_blocking_pool(data, compress::compress).await
    } else {
        (data, None)
    };
    let after_compression = compressed.is_some();
    let (file_name, content_type, data, pixels) = match compressed {
        Some(out) => (
            compressed_file_name(path, out.format),
            out.format.content_type(),
            out.bytes,
            Some((out.width, out.height)),
        ),
        None => (display_name(path), content_type, data, None),
    };

    // Checked on what is uploaded, since the file may also have grown since its size was read.
    let uploaded = data.len() as u64;
    check_limit(uploaded, server_limit, after_compression)?;

    let (data, pixels) = if pixels.is_none() && content_type.type_() == mime::IMAGE {
        on_blocking_pool(data, compress::dimensions).await
    } else {
        (data, pixels)
    };
    let media = match pixels {
        Some((width, height)) => Some(OutgoingMediaInfo { width: Some(width), height: Some(height), duration_ms: None }),
        None => media,
    };
    let info = attachment_info(&content_type, uploaded, media.as_ref());
    Ok(Attachment { file_name, content_type, data, info })
}

/// Decoding is CPU work, so it runs on the blocking pool; the bytes come back either way.
async fn on_blocking_pool<T: Send + 'static>(
    data: Vec<u8>,
    work: fn(&[u8]) -> Option<T>,
) -> (Vec<u8>, Option<T>) {
    let data = Arc::new(data);
    let input = data.clone();
    let result = tokio::task::spawn_blocking(move || work(&input))
        .await
        .unwrap_or_else(|e| {
            log::warn!("Processing an attached image did not finish: {e}");
            None
        });
    (Arc::unwrap_or_clone(data), result)
}

fn compressed_file_name(path: &Path, format: OutputFormat) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy()).unwrap_or_default();
    let stem: &str = if stem.is_empty() { "image" } else { &stem };
    format!("{stem}.{}", format.extension())
}

fn unreadable(path: &Path, e: std::io::Error) -> String {
    log::warn!("Failed to read the attachment {}: {e}", path.display());
    UNREADABLE.into()
}

/// Returns once the file is queued; a failure after that arrives through the send queue's error stream.
pub(crate) async fn send(client: &Client, room_id: &str, attachment: Attachment) -> Result<(), String> {
    let room = RoomId::parse(room_id).ok().and_then(|id| client.get_room(&id));
    let Some(room) = room else {
        log::warn!("Not sending an attachment to room {room_id}: it is not known to this client");
        return Err(UNKNOWN_ROOM.into());
    };
    let config = AttachmentConfig::new().info(attachment.info);
    room.send_queue()
        .send_attachment(attachment.file_name, attachment.content_type, attachment.data, config)
        .await
        .map(|_| ())
        .map_err(|e| {
            log::warn!("Failed to queue an attachment for {room_id}: {e:?}");
            match e {
                RoomSendQueueError::RoomNotJoined | RoomSendQueueError::RoomDisappeared => NOT_JOINED.into(),
                _ => UPLOAD_FAILED.into(),
            }
        })
}

/// The SDK caches the answer, so only a client's first attachment asks the homeserver.
/// `None` when it does not answer in time; the SDK checks again as it uploads.
pub(crate) async fn server_upload_limit(client: &Client) -> Option<u64> {
    match tokio::time::timeout(LIMIT_WAIT, client.load_or_fetch_max_upload_size()).await {
        Ok(Ok(limit)) => Some(u64::from(limit)),
        Ok(Err(e)) => {
            log::warn!("Could not read the homeserver's upload limit: {e}");
            None
        }
        Err(_) => {
            log::warn!("The homeserver did not report its upload limit within {LIMIT_WAIT:?}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::commands::{AttachmentSend, CoreCommand, MatrixCommand};
    use crate::matrix::compress::fixtures::{jpeg_with_orientation, noise, opaque, uncompressed_png};
    use crate::matrix::test_server::CannedHomeserver;

    fn mime(s: &str) -> Mime {
        s.parse().unwrap()
    }

    fn media(width: u32, height: u32, duration_ms: u64) -> OutgoingMediaInfo {
        OutgoingMediaInfo { width: Some(width), height: Some(height), duration_ms: Some(duration_ms) }
    }

    fn write_file(path: &Path, len: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![0u8; len]).unwrap();
    }

    #[tokio::test]
    async fn only_the_servers_limit_refuses_a_file_and_only_when_it_is_known() {
        let tmp = tempfile::tempdir().unwrap();
        let temp_files = temp_files(tmp.path());
        let clip = tmp.path().join("clip.mp4");
        write_file(&clip, 3 * MIB as usize);

        let reason = prepare(&clip, true, Some(2 * MIB), None, &temp_files).await
            .expect_err("a file over the server's limit must not be queued");
        assert_eq!(reason, "it is 3 MB and the server's limit is 2 MB");

        for limit in [Some(3 * MIB), None] {
            let attachment = prepare(&clip, true, limit, None, &temp_files).await
                .unwrap_or_else(|reason| panic!("a limit of {limit:?} should let 3 MB through: {reason}"));
            assert_eq!(attachment.data.len() as u64, 3 * MIB);
        }
    }

    #[test]
    fn sizes_round_to_tenths_half_up_and_drop_the_decimal_only_when_whole() {
        assert_eq!(format_mb(2 * MIB), "2 MB");
        assert_eq!(format_mb(2 * MIB + 1), "2.0 MB");
        assert_eq!(format_mb(1_310_720), "1.3 MB");
        assert_eq!(format_mb(2_359_296), "2.3 MB");
        assert_eq!(format_mb(3_565_158), "3.4 MB");
        assert_eq!(format_mb(50_000_000), "47.7 MB");
    }

    #[test]
    fn an_sdk_size_rejection_reads_like_our_own() {
        let err = matrix_sdk::Error::Media(MediaError::MediaTooLargeToUpload {
            max: UInt::new(MIB).unwrap(),
            current: UInt::new(3 * MIB).unwrap(),
        });
        assert_eq!(send_failure_reason(&err, false), "it is 3 MB and the server's limit is 1 MB");
        assert_eq!(send_failure_reason(&matrix_sdk::Error::InsufficientData, false), "the server did not accept it");
    }

    #[test]
    fn real_media_keeps_its_type_and_source_code_is_not_mistaken_for_it() {
        for (name, expected) in [
            ("clip.mp4", "video/mp4"),
            ("song.flac", "audio/flac"),
            ("Voice Memo.m4a", "audio/mp4"),
            ("clip.m4v", "video/mp4"),
            ("main.ts", "application/octet-stream"),
        ] {
            assert_eq!(sanitize_mime(Path::new(name)), mime(expected), "{name}");
        }
    }

    #[test]
    fn the_info_is_the_kind_the_sdk_keeps_for_the_type_and_carries_what_was_measured() {
        let measured = media(640, 480, 2_500);
        let (width, height) = (Some(UInt::from(640u32)), Some(UInt::from(480u32)));
        let duration = Some(Duration::from_millis(2_500));
        let size = Some(UInt::from(99u32));

        for image in ["image/png", "image/gif"] {
            let AttachmentInfo::Image(info) = attachment_info(&mime(image), 99, Some(&measured)) else {
                panic!("{image} should be sent as an image");
            };
            assert_eq!((info.width, info.height, info.size), (width, height, size), "{image}");
        }

        let AttachmentInfo::Video(info) = attachment_info(&mime("video/mp4"), 99, Some(&measured)) else {
            panic!("an MP4 should be sent as a video");
        };
        assert_eq!((info.width, info.height, info.duration, info.size), (width, height, duration, size));

        let AttachmentInfo::Audio(info) = attachment_info(&mime("audio/ogg"), 99, Some(&measured)) else {
            panic!("an Ogg file should be sent as audio");
        };
        assert_eq!((info.duration, info.size), (duration, size));

        let AttachmentInfo::File(info) = attachment_info(&mime("application/pdf"), 99, Some(&measured)) else {
            panic!("a PDF should be sent as a file");
        };
        assert_eq!(info.size, size);

        let AttachmentInfo::Video(info) = attachment_info(&mime("video/webm"), 99, None) else {
            panic!("a WebM should be sent as a video");
        };
        assert_eq!((info.width, info.height, info.duration, info.size), (None, None, None, size));
    }

    fn temp_files(root: &Path) -> TempFiles {
        TempFiles::new(root.to_path_buf())
    }

    #[tokio::test]
    async fn a_temp_upload_is_read_and_then_removed_with_its_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let temp_files = temp_files(tmp.path());
        let path = temp_files.create("photo.png", &[0u8; 1000]).unwrap();

        let attachment = prepare(&path, true, None, None, &temp_files).await
            .expect("a small image should load");

        assert_eq!(attachment.file_name, "photo.png");
        assert_eq!(attachment.content_type, mime("image/png"));
        assert_eq!(attachment.data.len(), 1000);
        assert!(matches!(attachment.info, AttachmentInfo::Image(ref i) if i.size == Some(UInt::from(1000u32))));
        assert!(!path.exists(), "the temp file should be removed");
        assert!(!path.parent().unwrap().exists(), "its directory should be removed");
    }

    #[tokio::test]
    async fn a_file_of_the_users_own_is_never_removed_whatever_its_folder_is_called() {
        let tmp = tempfile::tempdir().unwrap();
        let temp_files = temp_files(tmp.path());
        let ours = temp_files.create("ours.png", &[0u8; 10]).unwrap();
        let sent = ours.parent().unwrap().join("photo.png");
        let rejected = tmp.path().join("etch-paste-abc").join("big.pdf");
        write_file(&sent, 10);
        write_file(&rejected, 3 * MIB as usize);

        prepare(&sent, true, Some(2 * MIB), None, &temp_files).await.expect("a small image should load");
        prepare(&rejected, true, Some(2 * MIB), None, &temp_files).await.expect_err("a large file should be rejected");

        assert!(sent.exists() && rejected.exists(), "only a file Etch created may be deleted");
    }

    #[tokio::test]
    async fn an_image_reports_its_upright_size_whether_or_not_it_was_compressed() {
        let tmp = tempfile::tempdir().unwrap();
        let temp_files = temp_files(tmp.path());
        let sideways = jpeg_with_orientation(&noise(3000, 1000), 6);
        assert!(sideways.len() as u64 > COMPRESS_THRESHOLD_BYTES, "the fixture has to be worth compressing");
        let sized = |info: &AttachmentInfo| match info {
            AttachmentInfo::Image(info) => (info.width, info.height),
            other => panic!("expected image info, got {other:?}"),
        };
        let px = |n: u32| Some(UInt::from(n));

        let path = temp_files.create("photo.jpeg", &sideways).unwrap();
        let attachment = prepare(&path, true, None, None, &temp_files).await
            .expect("a large image should be sent compressed");

        assert_eq!((attachment.file_name.as_str(), &attachment.content_type), ("photo.jpg", &mime::IMAGE_JPEG));
        assert!(attachment.data.len() < sideways.len());
        assert_eq!(sized(&attachment.info), (px(683), px(2048)));

        let path = temp_files.create("photo.jpeg", &sideways).unwrap();
        let attachment = prepare(&path, false, None, None, &temp_files).await
            .expect("an image should be sent as is when compression is declined");

        assert_eq!(attachment.data, sideways);
        assert_eq!(sized(&attachment.info), (px(1000), px(3000)), "measured from the header, turned upright");
    }

    #[tokio::test]
    async fn a_requested_compression_is_applied_above_the_offer_threshold_and_ignored_below_it() {
        let tmp = tempfile::tempdir().unwrap();
        let temp_files = temp_files(tmp.path());
        let large = uncompressed_png(&opaque(600, 400));
        let small = uncompressed_png(&opaque(200, 200));
        assert!(large.len() as u64 > COMPRESS_THRESHOLD_BYTES && small.len() as u64 <= COMPRESS_THRESHOLD_BYTES);

        let path = temp_files.create("photo.png", &large).unwrap();
        let attachment = prepare(&path, true, None, None, &temp_files).await
            .expect("a large image should be sent");

        assert_eq!((attachment.file_name.as_str(), &attachment.content_type), ("photo.jpg", &mime::IMAGE_JPEG));
        assert!(attachment.data.len() < large.len(), "the user asked for the smaller file");

        // The composer sends `compress: true` whenever its checkbox is hidden, which it is for a small image.
        let path = temp_files.create("photo.png", &small).unwrap();
        let attachment = prepare(&path, true, None, None, &temp_files).await
            .expect("a small image should be sent");

        assert_eq!((attachment.file_name.as_str(), &attachment.content_type), ("photo.png", &mime::IMAGE_PNG));
        assert_eq!(attachment.data, small, "a small image must reach the room untouched");
    }

    #[tokio::test]
    async fn an_image_over_the_servers_limit_is_sent_only_if_compression_brings_it_under() {
        let tmp = tempfile::tempdir().unwrap();
        let temp_files = temp_files(tmp.path());
        let noisy = uncompressed_png(&noise(400, 400));
        let original = noisy.len() as u64;
        let compressed = compress::compress(&noisy).expect("noise should still shrink as a JPEG").bytes.len() as u64;
        assert!(compressed > 100_000, "the fixture has to stay over the tighter limit once compressed");

        let path = temp_files.create("photo.png", &noisy).unwrap();
        let attachment = prepare(&path, true, Some(original - 1), None, &temp_files).await
            .expect("compression brings this image under the limit");
        assert_eq!(attachment.data.len() as u64, compressed);

        let path = temp_files.create("photo.png", &noisy).unwrap();
        let reason = prepare(&path, false, Some(original - 1), None, &temp_files).await
            .expect_err("compression was declined, so the image is over the limit");
        assert_eq!(
            reason,
            format!("it is {} and the server's limit is {}", format_mb(original), format_mb(original - 1)),
        );

        let path = temp_files.create("photo.png", &noisy).unwrap();
        let reason = prepare(&path, true, Some(100_000), None, &temp_files).await
            .expect_err("nothing over the limit may be handed to the send queue");
        assert_eq!(
            reason,
            format!("it is {} after compression and the server's limit is 0.1 MB", format_mb(compressed)),
        );
        assert!(!path.exists(), "the temp file should be removed");
    }

    /// The other side of this shape is `MatrixCommand` in the frontend's ipc.ts.
    #[test]
    fn the_send_attachment_json_the_frontend_sends_is_the_command_core_runs() {
        let audio_only = OutgoingMediaInfo { width: None, height: None, duration_ms: Some(3_500) };
        for (media_info, expected) in [
            (json!(null), None),
            (json!({ "width": 1280, "height": 720, "duration_ms": 4000 }), Some(media(1280, 720, 4_000))),
            (json!({ "width": null, "height": null, "duration_ms": 3500 }), Some(audio_only)),
        ] {
            let sent = json!({
                "type": "Matrix",
                "data": {
                    "type": "SendAttachment",
                    "data": {
                        "room_id": "!room:example.org",
                        "path": "/home/user/clip.mp4",
                        "compress": true,
                        "media_info": media_info,
                    },
                },
            });

            let CoreCommand::Matrix(command) = serde_json::from_value(sent.clone())
                .unwrap_or_else(|e| panic!("{sent} should be a command: {e}"))
            else {
                panic!("{sent} should be a Matrix command");
            };
            assert_eq!(command, MatrixCommand::SendAttachment(AttachmentSend {
                room_id: "!room:example.org".into(),
                path: "/home/user/clip.mp4".into(),
                compress: true,
                media_info: expected,
            }));
        }
    }

    /// The other side of this shape is `Inspection` in attachments.ts.
    #[test]
    fn an_inspection_offers_compression_only_for_a_large_still_image_in_the_shape_the_composer_reads() {
        for (name, size, mimetype, compress_offered) in [
            ("photo.png", 256_000, "image/png", false),
            ("Photo.JPG", 256_001, "image/jpeg", true),
            ("party.gif", 3 * MIB, "image/gif", false),
            ("clip.mp4", 3 * MIB, "video/mp4", false),
        ] {
            assert_eq!(
                serde_json::to_value(inspect(name, size)).unwrap(),
                json!({ "mimetype": mimetype, "compress_offered": compress_offered }),
                "{name} at {size} bytes",
            );
        }
    }

    #[tokio::test]
    async fn the_limit_is_the_homeservers_own_and_unknown_when_it_does_not_say() {
        let silent = CannedHomeserver::ok().await;
        assert_eq!(server_upload_limit(&silent.client_for("@alice:example.com").await).await, None);

        let reporting = CannedHomeserver::start("200 OK", r#"{"m.upload.size":3145728}"#).await;
        assert_eq!(server_upload_limit(&reporting.client_for("@alice:example.com").await).await, Some(3 * MIB));
    }
}
