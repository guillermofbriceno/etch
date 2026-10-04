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
use serde::{Deserialize, Serialize};

use crate::commands::OutgoingMediaInfo;
use crate::matrix::compress::{self, OutputFormat};
use crate::temp_files::TempFiles;

const MIB: u64 = 1024 * 1024;
const COMPRESS_THRESHOLD_BYTES: u64 = 256_000;

pub(crate) const NOT_CONNECTED: &str = "not connected to the server";
const UNREADABLE: &str = "the file could not be read";
const UNKNOWN_ROOM: &str = "the room could not be found";
const NOT_JOINED: &str = "you are not in this room";
const UPLOAD_FAILED: &str = "the upload failed";
const HELD_FOR_RETRY: &str = "the server could not be reached; it will be retried when you next send to this room";
const NOT_ACCEPTED: &str = "the server did not accept it";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UploadCategory {
    Image,
    Other,
}

impl UploadCategory {
    pub(crate) fn of(content_type: &Mime) -> Self {
        if content_type.type_() == mime::IMAGE && content_type.subtype() != mime::GIF {
            Self::Image
        } else {
            Self::Other
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct UploadLimits {
    pub image_bytes: u64,
    pub other_bytes: u64,
}

impl UploadLimits {
    pub(crate) const ETCH_CAPS: Self = Self { image_bytes: 5 * MIB, other_bytes: 2 * MIB };

    pub(crate) fn effective(server_limit: Option<u64>) -> Self {
        let caps = Self::ETCH_CAPS;
        match server_limit {
            Some(server) => Self {
                image_bytes: caps.image_bytes.min(server),
                other_bytes: caps.other_bytes.min(server),
            },
            None => caps,
        }
    }

    pub(crate) fn for_category(&self, category: UploadCategory) -> u64 {
        match category {
            UploadCategory::Image => self.image_bytes,
            UploadCategory::Other => self.other_bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum Verdict {
    Accept { compress_offered: bool },
    MustCompress,
    Reject { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Inspection {
    pub mimetype: String,
    pub limit: u64,
    pub verdict: Verdict,
}

/// The composer's preflight; `None` limits mean the session has not reported its own yet.
pub fn inspect(file_name: &str, size: u64, limits: Option<UploadLimits>) -> Inspection {
    let content_type = sanitize_mime(Path::new(file_name));
    let limits = limits.unwrap_or(UploadLimits::ETCH_CAPS);
    let limit = limits.for_category(UploadCategory::of(&content_type));
    Inspection {
        verdict: verdict(&content_type, size, limit),
        mimetype: content_type.essence_str().to_owned(),
        limit,
    }
}

fn verdict(content_type: &Mime, size: u64, limit: u64) -> Verdict {
    let compressible = is_compressible(content_type);
    if size <= limit {
        Verdict::Accept { compress_offered: compressible && size > COMPRESS_THRESHOLD_BYTES }
    } else if compressible {
        Verdict::MustCompress
    } else {
        Verdict::Reject { reason: over_limit_reason(size, limit, false) }
    }
}

fn is_compressible(content_type: &Mime) -> bool {
    content_type.type_() == mime::IMAGE
        && matches!(content_type.subtype().as_str(), "jpeg" | "png" | "webp" | "bmp" | "tiff")
}

/// Integer rounding, half up, so the frontend can reproduce the text exactly.
pub(crate) fn format_mb(bytes: u64) -> String {
    if bytes.is_multiple_of(MIB) {
        return format!("{} MB", bytes / MIB);
    }
    let tenths = (u128::from(bytes) * 10 + u128::from(MIB / 2)) / u128::from(MIB);
    format!("{}.{} MB", tenths / 10, tenths % 10)
}

fn over_limit_reason(size: u64, limit: u64, after_compression: bool) -> String {
    format!(
        "it is {}{} and the limit for this kind of file is {}",
        format_mb(size),
        if after_compression { " after compression" } else { "" },
        format_mb(limit),
    )
}

fn could_not_compress_reason(limit: u64) -> String {
    format!("it could not be compressed to fit the {} limit", format_mb(limit))
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
    limits: UploadLimits,
    media: Option<OutgoingMediaInfo>,
    temp_files: &TempFiles,
) -> Result<Attachment, String> {
    let prepared = prepare_file(path, compress_requested, limits, media).await;
    discard(temp_files, path).await;
    prepared
}

async fn prepare_file(
    path: &Path,
    compress_requested: bool,
    limits: UploadLimits,
    media: Option<OutgoingMediaInfo>,
) -> Result<Attachment, String> {
    let content_type = sanitize_mime(path);
    let limit = limits.for_category(UploadCategory::of(&content_type));
    let size = tokio::fs::metadata(path).await.map_err(|e| unreadable(path, e))?.len();
    let (must_compress, should_compress) = match verdict(&content_type, size, limit) {
        Verdict::Reject { reason } => return Err(reason),
        Verdict::MustCompress => (true, true),
        Verdict::Accept { compress_offered } => (false, compress_requested && compress_offered),
    };
    let data = tokio::fs::read(path).await.map_err(|e| unreadable(path, e))?;

    let (data, compressed) = if should_compress {
        on_blocking_pool(data, compress::compress).await
    } else {
        (data, None)
    };
    if compressed.is_none() && must_compress {
        return Err(could_not_compress_reason(limit));
    }
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
    if uploaded > limit {
        return Err(over_limit_reason(uploaded, limit, after_compression));
    }

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

pub(crate) async fn fetch_upload_limits(client: &Client) -> UploadLimits {
    match client.load_or_fetch_max_upload_size().await {
        Ok(server_limit) => UploadLimits::effective(Some(u64::from(server_limit))),
        Err(e) => {
            log::warn!("Could not read the homeserver's upload limit, using Etch's own: {e}");
            UploadLimits::ETCH_CAPS
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

    #[test]
    fn each_kind_of_file_is_held_to_its_own_limit_and_offered_compression_only_where_it_helps() {
        let accept = |compress_offered| Verdict::Accept { compress_offered };
        let reject = |reason: &str| Verdict::Reject { reason: reason.into() };
        for (name, size, expected) in [
            ("photo.png", 256_000, accept(false)),
            ("photo.png", 256_001, accept(true)),
            ("photo.webp", 5 * MIB, accept(true)),
            ("Photo.JPG", 5 * MIB + 1, Verdict::MustCompress),
            ("photo.heic", 7 * MIB, reject("it is 7 MB and the limit for this kind of file is 5 MB")),
            ("clip.mp4", 2 * MIB, accept(false)),
            ("clip.mp4", 3_565_158, reject("it is 3.4 MB and the limit for this kind of file is 2 MB")),
            ("party.gif", MIB, accept(false)),
            ("party.gif", 3 * MIB, reject("it is 3 MB and the limit for this kind of file is 2 MB")),
        ] {
            assert_eq!(inspect(name, size, None).verdict, expected, "{name} at {size} bytes");
        }

        let lowered = UploadLimits { image_bytes: 3 * MIB, other_bytes: MIB };
        assert_eq!(
            inspect("photo.png", 4 * MIB, Some(lowered)),
            Inspection { mimetype: "image/png".into(), limit: 3 * MIB, verdict: Verdict::MustCompress },
        );
    }

    /// The frontend's formatMB test uses the same sizes, because the two texts must agree byte for byte.
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
        assert_eq!(send_failure_reason(&err, false), "it is 3 MB and the limit for this kind of file is 1 MB");
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

        let attachment = prepare(&path, true, UploadLimits::ETCH_CAPS, None, &temp_files).await
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

        prepare(&sent, true, UploadLimits::ETCH_CAPS, None, &temp_files).await.expect("a small image should load");
        prepare(&rejected, true, UploadLimits::ETCH_CAPS, None, &temp_files).await.expect_err("a large file should be rejected");

        assert!(sent.exists() && rejected.exists(), "only a file Etch created may be deleted");
    }

    #[tokio::test]
    async fn an_image_over_its_limit_is_sent_compressed_and_every_image_reports_its_upright_size() {
        let tmp = tempfile::tempdir().unwrap();
        let temp_files = temp_files(tmp.path());
        let sideways = jpeg_with_orientation(&opaque(3000, 1000), 6);
        let sized = |info: &AttachmentInfo| match info {
            AttachmentInfo::Image(info) => (info.width, info.height),
            other => panic!("expected image info, got {other:?}"),
        };
        let px = |n: u32| Some(UInt::from(n));

        let tight = UploadLimits { image_bytes: sideways.len() as u64 - 1, other_bytes: MIB };
        let path = temp_files.create("photo.jpeg", &sideways).unwrap();
        let attachment = prepare(&path, false, tight, None, &temp_files).await
            .expect("an image over its limit should be compressed to fit");

        assert_eq!((attachment.file_name.as_str(), &attachment.content_type), ("photo.jpg", &mime::IMAGE_JPEG));
        assert!(attachment.data.len() as u64 <= tight.image_bytes);
        assert_eq!(sized(&attachment.info), (px(683), px(2048)));

        let roomy = UploadLimits { image_bytes: 20 * MIB, other_bytes: MIB };
        let path = temp_files.create("photo.jpeg", &sideways).unwrap();
        let attachment = prepare(&path, false, roomy, None, &temp_files).await
            .expect("an image within its limit should be sent as is");

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
        let attachment = prepare(&path, true, UploadLimits::ETCH_CAPS, None, &temp_files).await
            .expect("an image within its limit should be sent");

        assert_eq!((attachment.file_name.as_str(), &attachment.content_type), ("photo.jpg", &mime::IMAGE_JPEG));
        assert!(attachment.data.len() < large.len(), "the user asked for the smaller file");

        // The composer sends `compress: true` whenever its checkbox is hidden, which it is for a small image.
        let path = temp_files.create("photo.png", &small).unwrap();
        let attachment = prepare(&path, true, UploadLimits::ETCH_CAPS, None, &temp_files).await
            .expect("a small image should be sent");

        assert_eq!((attachment.file_name.as_str(), &attachment.content_type), ("photo.png", &mime::IMAGE_PNG));
        assert_eq!(attachment.data, small, "a small image must reach the room untouched");
    }

    #[tokio::test]
    async fn an_image_still_over_its_limit_after_compression_is_refused_and_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let temp_files = temp_files(tmp.path());
        let noisy = uncompressed_png(&noise(400, 400));
        let compressed = compress::compress(&noisy).expect("noise should still shrink as a JPEG").bytes.len() as u64;
        let limits = UploadLimits { image_bytes: 100_000, other_bytes: 100_000 };
        assert!(compressed > limits.image_bytes, "the fixture has to stay over the limit once compressed");
        let path = temp_files.create("photo.png", &noisy).unwrap();

        let reason = prepare(&path, true, limits, None, &temp_files).await
            .expect_err("nothing over the limit may be handed to the send queue");

        assert_eq!(
            reason,
            format!("it is {} after compression and the limit for this kind of file is 0.1 MB", format_mb(compressed)),
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

    /// The other side of these shapes is `Inspection` in attachments.ts and `UploadLimits` in stores/uploads.ts.
    #[test]
    fn an_inspection_takes_the_frontends_limits_and_answers_in_the_shape_it_reads() {
        let limits: UploadLimits = serde_json::from_value(json!({ "image_bytes": 3_145_728, "other_bytes": 1_048_576 }))
            .expect("the limits the frontend stores should be understood");
        for (name, size, mimetype, limit, verdict) in [
            ("photo.png", 300_000, "image/png", 3_145_728, json!({ "type": "Accept", "compress_offered": true })),
            ("photo.png", 4 * MIB, "image/png", 3_145_728, json!({ "type": "MustCompress" })),
            ("clip.mp4", 2 * MIB, "video/mp4", 1_048_576, json!({
                "type": "Reject",
                "reason": "it is 2 MB and the limit for this kind of file is 1 MB",
            })),
        ] {
            assert_eq!(
                serde_json::to_value(inspect(name, size, Some(limits))).unwrap(),
                json!({ "mimetype": mimetype, "limit": limit, "verdict": verdict }),
                "{name} at {size} bytes",
            );
        }
    }

    #[tokio::test]
    async fn without_a_homeserver_limit_the_caps_apply() {
        let server = CannedHomeserver::ok().await;
        let client = server.client_for("@alice:example.com").await;

        assert_eq!(fetch_upload_limits(&client).await, UploadLimits::ETCH_CAPS);
    }
}
