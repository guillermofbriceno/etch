use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use matrix_sdk::Client;
use matrix_sdk::attachment::{
    AttachmentConfig, AttachmentInfo, BaseAudioInfo, BaseFileInfo, BaseImageInfo, BaseVideoInfo,
};
use matrix_sdk::media::MediaError;
use matrix_sdk::ruma::{RoomId, UInt};
use mime_guess::mime::{self, Mime};

use crate::commands::OutgoingMediaInfo;

const MIB: u64 = 1024 * 1024;
const TEMP_PREFIX: &str = "etch-paste-";

pub(crate) const NOT_CONNECTED: &str = "not connected to the server";
const UNREADABLE: &str = "the file could not be read";
const UNKNOWN_ROOM: &str = "the room could not be found";
const UPLOAD_FAILED: &str = "the upload failed";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UploadLimits {
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

    /// The error is the reason shown to the user.
    pub(crate) fn check(&self, content_type: &Mime, size: u64) -> Result<(), String> {
        let limit = self.for_category(UploadCategory::of(content_type));
        if size > limit {
            Err(over_limit_reason(size, limit))
        } else {
            Ok(())
        }
    }
}

/// Integer rounding, half up, so the frontend can reproduce the text exactly.
pub(crate) fn format_mb(bytes: u64) -> String {
    if bytes.is_multiple_of(MIB) {
        return format!("{} MB", bytes / MIB);
    }
    let tenths = (u128::from(bytes) * 10 + u128::from(MIB / 2)) / u128::from(MIB);
    format!("{}.{} MB", tenths / 10, tenths % 10)
}

fn over_limit_reason(size: u64, limit: u64) -> String {
    format!(
        "it is {} and the limit for this kind of file is {}",
        format_mb(size),
        format_mb(limit),
    )
}

fn send_failure_reason(err: &matrix_sdk::Error) -> String {
    match err {
        matrix_sdk::Error::Media(MediaError::MediaTooLargeToUpload { max, current }) => {
            over_limit_reason(u64::from(*current), u64::from(*max))
        }
        _ => UPLOAD_FAILED.into(),
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

/// Only the directory form counts: older versions sent files named `etch-paste-*`, and a
/// user re-sharing one of those must not lose it.
pub(crate) fn temp_upload_dir(path: &Path) -> Option<&Path> {
    path.parent().filter(|dir| {
        dir.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(TEMP_PREFIX))
    })
}

pub(crate) async fn discard_if_temp(path: &Path) {
    let Some(dir) = temp_upload_dir(path) else { return };
    if let Err(e) = tokio::fs::remove_file(path).await
        && e.kind() != ErrorKind::NotFound
    {
        log::warn!("Failed to remove the temp upload {}: {e}", path.display());
    }
    // Not `remove_dir_all`: a directory holding anything else is left alone.
    if let Err(e) = tokio::fs::remove_dir(dir).await
        && e.kind() != ErrorKind::NotFound
    {
        log::warn!("Failed to remove the temp upload directory {}: {e}", dir.display());
    }
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

/// Removes an Etch temp file whatever the outcome.
pub(crate) async fn load(
    path: &Path,
    limits: UploadLimits,
    media: Option<&OutgoingMediaInfo>,
) -> Result<Attachment, String> {
    let read = read_within_limit(path, limits).await;
    discard_if_temp(path).await;
    let (content_type, data) = read?;
    let info = attachment_info(&content_type, data.len() as u64, media);
    Ok(Attachment { file_name: display_name(path), content_type, data, info })
}

async fn read_within_limit(path: &Path, limits: UploadLimits) -> Result<(Mime, Vec<u8>), String> {
    let content_type = sanitize_mime(path);
    let size = tokio::fs::metadata(path).await.map_err(|e| unreadable(path, e))?.len();
    limits.check(&content_type, size)?;
    let data = tokio::fs::read(path).await.map_err(|e| unreadable(path, e))?;
    // The file may have grown since the metadata was read, and the cap is on what is uploaded.
    limits.check(&content_type, data.len() as u64)?;
    Ok((content_type, data))
}

fn unreadable(path: &Path, e: std::io::Error) -> String {
    log::error!("Failed to read the attachment {}: {e}", path.display());
    UNREADABLE.into()
}

pub(crate) async fn send(client: &Client, room_id: &str, attachment: Attachment) -> Result<(), String> {
    let room = RoomId::parse(room_id).ok().and_then(|id| client.get_room(&id));
    let Some(room) = room else {
        log::warn!("Not sending an attachment to room {room_id}: it is not known to this client");
        return Err(UNKNOWN_ROOM.into());
    };
    let config = AttachmentConfig::new().info(attachment.info);
    room.send_attachment(&attachment.file_name, &attachment.content_type, attachment.data, config)
        .await
        .map(|_| ())
        .map_err(|e| {
            log::error!("Failed to send an attachment to {room_id}: {e:?}");
            send_failure_reason(&e)
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
    use super::*;
    use crate::matrix::test_server::CannedHomeserver;
    use std::path::PathBuf;

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
    fn still_images_are_images_and_everything_else_is_other() {
        assert_eq!(UploadCategory::of(&mime("image/png")), UploadCategory::Image);
        assert_eq!(UploadCategory::of(&mime("image/jpeg")), UploadCategory::Image);
        assert_eq!(UploadCategory::of(&mime("image/webp")), UploadCategory::Image);
        assert_eq!(UploadCategory::of(&mime("image/gif")), UploadCategory::Other);
        assert_eq!(UploadCategory::of(&mime("video/mp4")), UploadCategory::Other);
        assert_eq!(UploadCategory::of(&mime("audio/ogg")), UploadCategory::Other);
        assert_eq!(UploadCategory::of(&mime("application/pdf")), UploadCategory::Other);
    }

    #[test]
    fn the_etch_caps_are_five_and_two_mebibytes() {
        assert_eq!(UploadLimits::ETCH_CAPS, UploadLimits { image_bytes: 5_242_880, other_bytes: 2_097_152 });
    }

    #[test]
    fn the_effective_limit_is_the_smaller_of_the_cap_and_the_server_limit() {
        assert_eq!(UploadLimits::effective(None), UploadLimits::ETCH_CAPS);
        assert_eq!(UploadLimits::effective(Some(100 * MIB)), UploadLimits::ETCH_CAPS);
        assert_eq!(
            UploadLimits::effective(Some(3 * MIB)),
            UploadLimits { image_bytes: 3 * MIB, other_bytes: 2 * MIB },
        );
        assert_eq!(
            UploadLimits::effective(Some(MIB)),
            UploadLimits { image_bytes: MIB, other_bytes: MIB },
        );
    }

    #[test]
    fn a_file_exactly_at_the_limit_is_accepted_and_one_byte_more_is_not() {
        let limits = UploadLimits::ETCH_CAPS;
        assert_eq!(limits.check(&mime("image/png"), 5_242_880), Ok(()));
        assert!(limits.check(&mime("image/png"), 5_242_881).is_err());
        assert_eq!(limits.check(&mime("video/mp4"), 2_097_152), Ok(()));
        assert!(limits.check(&mime("video/mp4"), 2_097_153).is_err());
        assert_eq!(limits.check(&mime("application/pdf"), 0), Ok(()));
    }

    #[test]
    fn a_gif_is_held_to_the_limit_for_other_files() {
        assert_eq!(
            UploadLimits::ETCH_CAPS.check(&mime("image/gif"), 3 * MIB),
            Err("it is 3 MB and the limit for this kind of file is 2 MB".into()),
        );
    }

    #[test]
    fn the_reason_gives_both_sizes_in_mb() {
        assert_eq!(
            UploadLimits::ETCH_CAPS.check(&mime("video/mp4"), 3_565_158),
            Err("it is 3.4 MB and the limit for this kind of file is 2 MB".into()),
        );
        assert_eq!(
            UploadLimits::effective(Some(1_500_000)).check(&mime("image/png"), 2 * MIB),
            Err("it is 2 MB and the limit for this kind of file is 1.4 MB".into()),
        );
    }

    #[test]
    fn sizes_show_one_decimal_only_when_not_whole() {
        assert_eq!(format_mb(2 * MIB), "2 MB");
        assert_eq!(format_mb(5 * MIB), "5 MB");
        assert_eq!(format_mb(MIB + MIB / 2), "1.5 MB");
        assert_eq!(format_mb(50_000_000), "47.7 MB");
    }

    #[test]
    fn a_size_halfway_between_tenths_rounds_up() {
        assert_eq!(format_mb(1_310_720), "1.3 MB");
        assert_eq!(format_mb(2_359_296), "2.3 MB");
    }

    #[test]
    fn a_size_just_over_a_whole_number_keeps_its_decimal() {
        assert_eq!(format_mb(2_097_153), "2.0 MB");
    }

    #[test]
    fn an_sdk_size_rejection_reads_like_our_own() {
        let err = matrix_sdk::Error::Media(MediaError::MediaTooLargeToUpload {
            max: UInt::new(MIB).unwrap(),
            current: UInt::new(3 * MIB).unwrap(),
        });
        assert_eq!(send_failure_reason(&err), "it is 3 MB and the limit for this kind of file is 1 MB");
        assert_eq!(send_failure_reason(&matrix_sdk::Error::InsufficientData), "the upload failed");
    }

    #[test]
    fn a_file_in_an_etch_paste_directory_is_a_temp_upload() {
        assert_eq!(
            temp_upload_dir(Path::new("/tmp/etch-paste-abc/photo.png")),
            Some(Path::new("/tmp/etch-paste-abc")),
        );
        assert_eq!(
            temp_upload_dir(Path::new("/tmp/etch-paste-abc/etch-paste-1.png")),
            Some(Path::new("/tmp/etch-paste-abc")),
        );
    }

    #[test]
    fn a_flat_etch_paste_file_is_not_a_temp_upload() {
        assert_eq!(temp_upload_dir(Path::new("/tmp/etch-paste-1234.png")), None);
        assert_eq!(temp_upload_dir(Path::new("/home/someone/Downloads/etch-paste-1700000000000.png")), None);
        assert_eq!(temp_upload_dir(Path::new("etch-paste-1.png")), None);
    }

    #[test]
    fn other_files_are_not_temp_uploads() {
        assert_eq!(temp_upload_dir(Path::new("/home/someone/photo.png")), None);
        assert_eq!(temp_upload_dir(Path::new("/tmp/etch-paste/photo.png")), None);
        assert_eq!(temp_upload_dir(Path::new("/tmp/etch-paste-abc/nested/photo.png")), None);
        assert_eq!(temp_upload_dir(Path::new("/tmp/my-etch-paste-1/photo.png")), None);
    }

    #[test]
    fn media_types_are_sniffed_from_the_extension_but_source_code_is_not_video() {
        assert_eq!(sanitize_mime(Path::new("clip.mp4")), mime("video/mp4"));
        assert_eq!(sanitize_mime(Path::new("song.flac")), mime("audio/flac"));
        assert_eq!(sanitize_mime(Path::new("photo.png")), mime("image/png"));
        assert_eq!(sanitize_mime(Path::new("main.ts")), mime::APPLICATION_OCTET_STREAM);
        assert_eq!(sanitize_mime(Path::new("no-extension")), mime::APPLICATION_OCTET_STREAM);
    }

    #[test]
    fn an_m4a_voice_memo_is_sent_as_mp4_audio_with_its_duration() {
        let content_type = sanitize_mime(Path::new("Voice Memo.m4a"));
        assert_eq!(content_type, mime("audio/mp4"));
        let AttachmentInfo::Audio(info) = attachment_info(&content_type, 10, Some(&media(0, 0, 4_000))) else {
            panic!("an m4a should be sent as audio");
        };
        assert_eq!(info.duration, Some(Duration::from_secs(4)));
    }

    #[test]
    fn an_m4v_clip_is_sent_as_mp4_video_with_its_duration() {
        let content_type = sanitize_mime(Path::new("clip.m4v"));
        assert_eq!(content_type, mime("video/mp4"));
        let AttachmentInfo::Video(info) = attachment_info(&content_type, 10, Some(&media(640, 360, 2_500))) else {
            panic!("an m4v should be sent as video");
        };
        assert_eq!(info.duration, Some(Duration::from_millis(2_500)));
    }

    #[test]
    fn an_image_carries_its_size_and_dimensions() {
        let AttachmentInfo::Image(info) = attachment_info(&mime("image/png"), 1234, Some(&media(640, 480, 9))) else {
            panic!("a PNG should be sent as an image");
        };
        assert_eq!(info.width, Some(UInt::from(640u32)));
        assert_eq!(info.height, Some(UInt::from(480u32)));
        assert_eq!(info.size, Some(UInt::from(1234u32)));
    }

    #[test]
    fn a_gif_is_still_sent_as_an_image() {
        let info = attachment_info(&mime("image/gif"), 10, Some(&media(32, 32, 0)));
        assert!(matches!(info, AttachmentInfo::Image(_)), "the SDK sends a GIF as m.image, got {info:?}");
    }

    #[test]
    fn a_video_carries_its_size_dimensions_and_duration() {
        let AttachmentInfo::Video(info) = attachment_info(&mime("video/mp4"), 99, Some(&media(1920, 1080, 12_500))) else {
            panic!("an MP4 should be sent as a video");
        };
        assert_eq!(info.width, Some(UInt::from(1920u32)));
        assert_eq!(info.height, Some(UInt::from(1080u32)));
        assert_eq!(info.duration, Some(Duration::from_millis(12_500)));
        assert_eq!(info.size, Some(UInt::from(99u32)));
    }

    #[test]
    fn audio_carries_its_size_and_duration() {
        let AttachmentInfo::Audio(info) = attachment_info(&mime("audio/ogg"), 77, Some(&media(1, 1, 3_000))) else {
            panic!("an Ogg file should be sent as audio");
        };
        assert_eq!(info.duration, Some(Duration::from_secs(3)));
        assert_eq!(info.size, Some(UInt::from(77u32)));
    }

    #[test]
    fn any_other_file_carries_only_its_size() {
        let AttachmentInfo::File(info) = attachment_info(&mime("application/pdf"), 5, Some(&media(1, 1, 1))) else {
            panic!("a PDF should be sent as a file");
        };
        assert_eq!(info.size, Some(UInt::from(5u32)));
    }

    #[test]
    fn values_the_frontend_did_not_measure_are_left_out() {
        let AttachmentInfo::Video(info) = attachment_info(&mime("video/webm"), 8, None) else {
            panic!("a WebM should be sent as a video");
        };
        assert_eq!((info.width, info.height, info.duration), (None, None, None));
        assert_eq!(info.size, Some(UInt::from(8u32)));

        let partial = OutgoingMediaInfo { width: Some(10), ..Default::default() };
        let AttachmentInfo::Image(info) = attachment_info(&mime("image/jpeg"), 8, Some(&partial)) else {
            panic!("a JPEG should be sent as an image");
        };
        assert_eq!((info.width, info.height), (Some(UInt::from(10u32)), None));
    }

    #[test]
    fn a_file_without_a_name_is_called_attachment() {
        assert_eq!(display_name(Path::new("/tmp/etch-paste-abc/clip.mp4")), "clip.mp4");
        assert_eq!(display_name(Path::new("/")), "attachment");
    }

    #[tokio::test]
    async fn a_temp_upload_in_its_own_directory_is_read_and_removed_with_the_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("etch-paste-abc");
        let path = dir.join("photo.png");
        write_file(&path, 1000);

        let attachment = load(&path, UploadLimits::ETCH_CAPS, Some(&media(4, 3, 0))).await
            .expect("a small image should load");

        assert_eq!(attachment.file_name, "photo.png");
        assert_eq!(attachment.content_type, mime("image/png"));
        assert_eq!(attachment.data.len(), 1000);
        assert!(matches!(attachment.info, AttachmentInfo::Image(ref i) if i.size == Some(UInt::from(1000u32))));
        assert!(!path.exists(), "the temp file should be removed");
        assert!(!dir.exists(), "its etch-paste directory should be removed");
    }

    #[tokio::test]
    async fn a_temp_upload_over_the_limit_is_rejected_and_still_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("etch-paste-abc").join("clip.mp4");
        write_file(&path, 3_565_158);

        let result = load(&path, UploadLimits::ETCH_CAPS, None).await;

        assert_eq!(result.unwrap_err(), "it is 3.4 MB and the limit for this kind of file is 2 MB");
        assert!(!path.exists(), "a rejected temp file should still be removed");
    }

    #[tokio::test]
    async fn a_temp_directory_holding_anything_else_is_left_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("etch-paste-abc");
        let path = dir.join("clip.mp4");
        write_file(&path, 10);
        write_file(&dir.join("other.txt"), 10);

        load(&path, UploadLimits::ETCH_CAPS, None).await.expect("a small clip should load");

        assert!(!path.exists(), "the uploaded file should be removed");
        assert!(dir.join("other.txt").exists(), "nothing else in the directory may be deleted");
    }

    #[tokio::test]
    async fn a_file_the_user_picked_is_never_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let small = tmp.path().join("notes.pdf");
        let large = tmp.path().join("big.pdf");
        write_file(&small, 10);
        write_file(&large, 3 * MIB as usize);

        load(&small, UploadLimits::ETCH_CAPS, None).await.expect("a small file should load");
        load(&large, UploadLimits::ETCH_CAPS, None).await.expect_err("a large file should be rejected");

        assert!(small.exists() && large.exists(), "only Etch temp files may be deleted");
    }

    #[tokio::test]
    async fn a_saved_file_named_like_a_legacy_temp_file_is_never_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let small = tmp.path().join("etch-paste-1700000000000.png");
        let large = tmp.path().join("etch-paste-1700000000001.mp4");
        write_file(&small, 10);
        write_file(&large, 3 * MIB as usize);

        load(&small, UploadLimits::ETCH_CAPS, None).await.expect("a small image should load");
        load(&large, UploadLimits::ETCH_CAPS, None).await.expect_err("a large clip should be rejected");

        assert!(small.exists() && large.exists(), "a flat etch-paste file is the user's own");
    }

    #[tokio::test]
    async fn a_missing_file_is_reported_as_unreadable() {
        let tmp = tempfile::tempdir().unwrap();
        let path: PathBuf = tmp.path().join("etch-paste-gone").join("photo.png");

        let result = load(&path, UploadLimits::ETCH_CAPS, None).await;

        assert_eq!(result.unwrap_err(), "the file could not be read");
    }

    #[tokio::test]
    async fn the_homeserver_limit_lowers_the_caps() {
        let server = CannedHomeserver::start("200 OK", r#"{"m.upload.size":1048576}"#).await;
        let client = server.client_for("@alice:example.com").await;

        assert_eq!(fetch_upload_limits(&client).await, UploadLimits { image_bytes: MIB, other_bytes: MIB });
        assert!(server.requests_to("/config") > 0, "the limit should come from the media config endpoint");
    }

    #[tokio::test]
    async fn without_a_homeserver_limit_the_caps_apply() {
        let server = CannedHomeserver::ok().await;
        let client = server.client_for("@alice:example.com").await;

        assert_eq!(fetch_upload_limits(&client).await, UploadLimits::ETCH_CAPS);
    }
}
