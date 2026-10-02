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
    fn each_kind_of_file_is_held_to_its_own_limit_to_the_byte() {
        let limits = UploadLimits::ETCH_CAPS;
        assert_eq!(limits.check(&mime("image/png"), 5 * MIB), Ok(()));
        assert!(limits.check(&mime("image/png"), 5 * MIB + 1).is_err());
        assert_eq!(limits.check(&mime("video/mp4"), 2 * MIB), Ok(()));
        assert_eq!(
            limits.check(&mime("video/mp4"), 3_565_158),
            Err("it is 3.4 MB and the limit for this kind of file is 2 MB".into()),
        );
        assert_eq!(
            limits.check(&mime("image/gif"), 3 * MIB),
            Err("it is 3 MB and the limit for this kind of file is 2 MB".into()),
            "a GIF is held to the limit for other files, not the one for images",
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
        assert_eq!(send_failure_reason(&err), "it is 3 MB and the limit for this kind of file is 1 MB");
        assert_eq!(send_failure_reason(&matrix_sdk::Error::InsufficientData), "the upload failed");
    }

    #[test]
    fn only_a_file_directly_inside_an_etch_paste_directory_is_a_temp_upload() {
        assert_eq!(
            temp_upload_dir(Path::new("/tmp/etch-paste-abc/photo.png")),
            Some(Path::new("/tmp/etch-paste-abc")),
        );
        for not_temp in [
            "/tmp/etch-paste-1234.png",
            "etch-paste-1.png",
            "/home/someone/photo.png",
            "/tmp/etch-paste/photo.png",
            "/tmp/etch-paste-abc/nested/photo.png",
            "/tmp/my-etch-paste-1/photo.png",
        ] {
            assert_eq!(temp_upload_dir(Path::new(not_temp)), None, "{not_temp}");
        }
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
    async fn a_file_of_the_users_own_is_never_removed_even_one_named_like_a_legacy_temp_file() {
        let tmp = tempfile::tempdir().unwrap();
        let sent = tmp.path().join("etch-paste-1700000000000.png");
        let rejected = tmp.path().join("big.pdf");
        write_file(&sent, 10);
        write_file(&rejected, 3 * MIB as usize);

        load(&sent, UploadLimits::ETCH_CAPS, None).await.expect("a small image should load");
        load(&rejected, UploadLimits::ETCH_CAPS, None).await.expect_err("a large file should be rejected");

        assert!(sent.exists() && rejected.exists(), "only a file in an etch-paste directory may be deleted");
    }

    #[tokio::test]
    async fn without_a_homeserver_limit_the_caps_apply() {
        let server = CannedHomeserver::ok().await;
        let client = server.client_for("@alice:example.com").await;

        assert_eq!(fetch_upload_limits(&client).await, UploadLimits::ETCH_CAPS);
    }
}
