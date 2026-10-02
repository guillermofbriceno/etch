use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngDecoder, PngEncoder};
use image::codecs::webp::WebPDecoder;
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{ColorType, DynamicImage, ImageDecoder, ImageFormat, ImageReader};

const TEMP_PREFIX: &str = "etch-paste-";
const MAX_EDGE: u32 = 2048;
const JPEG_QUALITY: u8 = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Jpeg,
    Png,
}

impl OutputFormat {
    pub(crate) fn for_alpha(has_meaningful_alpha: bool) -> Self {
        if has_meaningful_alpha {
            Self::Png
        } else {
            Self::Jpeg
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
        }
    }
}

/// Only the directory form counts: older versions sent files named `etch-paste-*`, and a
/// user re-sharing one of those must not lose it.
pub(crate) fn temp_upload_dir(path: &Path) -> Option<&Path> {
    path.parent().filter(|dir| {
        dir.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with(TEMP_PREFIX))
    })
}

pub(crate) fn target_dimensions(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= max_edge {
        return (width, height);
    }
    let scale = |edge: u32| {
        let scaled =
            (u64::from(edge) * u64::from(max_edge) + u64::from(longest) / 2) / u64::from(longest);
        (scaled as u32).max(1)
    };
    (scale(width), scale(height))
}

pub(crate) fn has_meaningful_alpha(img: &DynamicImage) -> bool {
    match img {
        DynamicImage::ImageLumaA8(buf) => buf.pixels().any(|p| p.0[1] < u8::MAX),
        DynamicImage::ImageRgba8(buf) => buf.pixels().any(|p| p.0[3] < u8::MAX),
        other if other.color().has_alpha() => other.to_rgba16().pixels().any(|p| p.0[3] < u16::MAX),
        _ => false,
    }
}

pub(crate) fn is_compressible(format: Option<ImageFormat>, animated: bool) -> bool {
    match format {
        Some(ImageFormat::Jpeg | ImageFormat::Bmp | ImageFormat::Tiff) => true,
        Some(ImageFormat::Png | ImageFormat::WebP) => !animated,
        _ => false,
    }
}

pub(crate) fn is_animated(format: ImageFormat, data: &[u8]) -> bool {
    match format {
        ImageFormat::Png => PngDecoder::new(Cursor::new(data))
            .and_then(|d| d.is_apng())
            .unwrap_or(false),
        ImageFormat::WebP => WebPDecoder::new(Cursor::new(data)).is_ok_and(|d| d.has_animation()),
        _ => false,
    }
}

pub(crate) fn output_file_name(input: &Path, format: OutputFormat) -> String {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();
    let stem: &str = if stem.is_empty() { "image" } else { &stem };
    format!("{stem}.{}", format.extension())
}

/// `Ok(None)` means the input should be uploaded as is; it is then left untouched.
pub(crate) fn compress_image_file(input: &Path, temp_root: &Path) -> io::Result<Option<PathBuf>> {
    let data = std::fs::read(input)?;
    let Some((bytes, format)) = compress_bytes(&data) else {
        return Ok(None);
    };
    let out = write_temp_file(temp_root, &output_file_name(input, format), &bytes)?;
    remove_temp_input(input);
    Ok(Some(out))
}

pub(crate) fn write_temp_file(
    temp_root: &Path,
    file_name: &str,
    bytes: &[u8],
) -> io::Result<PathBuf> {
    let dir = create_temp_dir(temp_root)?;
    let path = dir.join(file_name);
    if let Err(e) = std::fs::write(&path, bytes) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(path)
}

fn compress_bytes(data: &[u8]) -> Option<(Vec<u8>, OutputFormat)> {
    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()?;
    let format = reader.format();
    let animated = format.is_some_and(|f| is_animated(f, data));
    if !is_compressible(format, animated) {
        return None;
    }

    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).ok()?;
    // The re-encoded file carries no EXIF, so the rotation has to be baked into the pixels.
    img.apply_orientation(orientation);

    let output = OutputFormat::for_alpha(has_meaningful_alpha(&img));
    let img = to_output_color(img, output);
    let (width, height) = target_dimensions(img.width(), img.height(), MAX_EDGE);
    let img = if (width, height) == (img.width(), img.height()) {
        img
    } else {
        img.resize_exact(width, height, FilterType::Lanczos3)
    };

    let bytes = encode(&img, output).ok()?;
    (bytes.len() < data.len()).then_some((bytes, output))
}

fn to_output_color(img: DynamicImage, output: OutputFormat) -> DynamicImage {
    let gray = matches!(
        img.color(),
        ColorType::L8 | ColorType::L16 | ColorType::La8 | ColorType::La16
    );
    match (output, gray) {
        (OutputFormat::Jpeg, true) => DynamicImage::ImageLuma8(img.into_luma8()),
        (OutputFormat::Jpeg, false) => DynamicImage::ImageRgb8(img.into_rgb8()),
        (OutputFormat::Png, true) => DynamicImage::ImageLumaA8(img.into_luma_alpha8()),
        (OutputFormat::Png, false) => DynamicImage::ImageRgba8(img.into_rgba8()),
    }
}

fn encode(img: &DynamicImage, output: OutputFormat) -> image::ImageResult<Vec<u8>> {
    let mut buf = Vec::new();
    match output {
        OutputFormat::Jpeg => {
            img.write_with_encoder(JpegEncoder::new_with_quality(&mut buf, JPEG_QUALITY))?
        }
        OutputFormat::Png => img.write_with_encoder(PngEncoder::new_with_quality(
            &mut buf,
            CompressionType::Default,
            PngFilter::Adaptive,
        ))?,
    }
    Ok(buf)
}

fn create_temp_dir(temp_root: &Path) -> io::Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for _ in 0..16 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = temp_root.join(format!("{TEMP_PREFIX}{}-{nanos}-{n}", std::process::id()));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free temp directory name",
    ))
}

fn remove_temp_input(path: &Path) {
    if let Some(dir) = temp_upload_dir(path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::webp::WebPEncoder;
    use image::{GenericImageView, ImageBuffer, LumaA, Rgb, RgbImage, Rgba, RgbaImage};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("etch-gui-test-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn entries(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn uncompressed_png(img: &DynamicImage) -> Vec<u8> {
        let mut buf = Vec::new();
        img.write_with_encoder(PngEncoder::new_with_quality(
            &mut buf,
            CompressionType::Uncompressed,
            PngFilter::NoFilter,
        ))
        .unwrap();
        buf
    }

    fn encoded(img: &DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, format).unwrap();
        buf.into_inner()
    }

    fn opaque(width: u32, height: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |x, y| {
            Rgb([
                (x % 256) as u8,
                (y % 256) as u8,
                ((x * 7 + y * 13) % 256) as u8,
            ])
        }))
    }

    fn translucent(width: u32, height: u32) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_fn(width, height, |x, y| {
            Rgba([
                (x % 256) as u8,
                (y % 256) as u8,
                90,
                if x < width / 2 { 0 } else { 255 },
            ])
        }))
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    fn noise(width: u32, height: u32) -> DynamicImage {
        let mut state = 0x9E37_79B9u32;
        DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |_, _| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            Rgb([state as u8, (state >> 8) as u8, (state >> 16) as u8])
        }))
    }

    fn apng(img: &DynamicImage) -> Vec<u8> {
        let png = uncompressed_png(img);
        let mut actl = 8u32.to_be_bytes().to_vec();
        actl.extend_from_slice(b"acTL");
        actl.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0]);
        actl.extend_from_slice(&crc32(&actl[4..]).to_be_bytes());
        // The acTL chunk has to come before the first IDAT; IHDR ends 33 bytes in.
        let mut out = png[..33].to_vec();
        out.extend_from_slice(&actl);
        out.extend_from_slice(&png[33..]);
        out
    }

    fn riff_chunk(tag: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = tag.to_vec();
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn animated_webp(img: &DynamicImage) -> Vec<u8> {
        let mut still = Vec::new();
        img.write_with_encoder(WebPEncoder::new_lossless(&mut still))
            .unwrap();
        let len = u32::from_le_bytes(still[16..20].try_into().unwrap()) as usize;
        let bitstream = &still[20..20 + len];

        let mut vp8x = vec![0x02, 0, 0, 0];
        let mut anmf = vec![0; 6];
        for edge in [img.width() - 1, img.height() - 1] {
            vp8x.extend_from_slice(&edge.to_le_bytes()[..3]);
            anmf.extend_from_slice(&edge.to_le_bytes()[..3]);
        }
        anmf.extend_from_slice(&[100, 0, 0, 0]);
        anmf.extend(riff_chunk(b"VP8L", bitstream));

        let mut body = b"WEBP".to_vec();
        body.extend(riff_chunk(b"VP8X", &vp8x));
        body.extend(riff_chunk(b"ANIM", &[0; 6]));
        body.extend(riff_chunk(b"ANMF", &anmf));
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend(body);
        out
    }

    fn jpeg_with_orientation(img: &DynamicImage, orientation: u8) -> Vec<u8> {
        let mut jpeg = Vec::new();
        img.write_with_encoder(JpegEncoder::new_with_quality(&mut jpeg, 100))
            .unwrap();
        // A TIFF header and a one-entry directory holding the Orientation tag, 0x0112.
        let mut exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
        exif.extend_from_slice(&[orientation, 0, 0, 0, 0, 0, 0, 0]);
        let mut out = jpeg[..2].to_vec();
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&(exif.len() as u16 + 2).to_be_bytes());
        out.extend_from_slice(&exif);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    #[test]
    fn target_dimensions_scale_only_down_keep_the_aspect_ratio_and_never_reach_zero() {
        assert_eq!(target_dimensions(800, 600, 2048), (800, 600));
        assert_eq!(target_dimensions(2048, 1000, 2048), (2048, 1000));
        assert_eq!(target_dimensions(4096, 3072, 2048), (2048, 1536));
        assert_eq!(target_dimensions(3000, 6000, 2048), (1024, 2048));
        assert_eq!(target_dimensions(100_000, 3, 2048), (2048, 1));
        assert_eq!(target_dimensions(3, 100_000, 2048), (1, 2048));
    }

    #[test]
    fn alpha_is_meaningful_only_when_some_pixel_is_not_opaque() {
        assert!(!has_meaningful_alpha(&opaque(4, 4)));

        let mut rgba = RgbaImage::from_pixel(4, 4, Rgba([10, 20, 30, 255]));
        assert!(!has_meaningful_alpha(&DynamicImage::ImageRgba8(rgba.clone())));
        rgba.put_pixel(3, 3, Rgba([10, 20, 30, 254]));
        assert!(has_meaningful_alpha(&DynamicImage::ImageRgba8(rgba)));

        let mut gray = ImageBuffer::from_pixel(4, 4, LumaA([100u8, 255]));
        assert!(!has_meaningful_alpha(&DynamicImage::ImageLumaA8(gray.clone())));
        gray.put_pixel(0, 0, LumaA([100, 0]));
        assert!(has_meaningful_alpha(&DynamicImage::ImageLumaA8(gray)));

        let mut rgba16 = ImageBuffer::from_pixel(4, 4, Rgba([10u16, 20, 30, u16::MAX]));
        assert!(!has_meaningful_alpha(&DynamicImage::ImageRgba16(rgba16.clone())));
        rgba16.put_pixel(1, 2, Rgba([10, 20, 30, u16::MAX - 1]));
        assert!(has_meaningful_alpha(&DynamicImage::ImageRgba16(rgba16)));
    }

    #[test]
    fn write_temp_file_uses_a_fresh_prefixed_directory() {
        let root = TestDir::new();
        let a = write_temp_file(root.path(), "image.png", b"a").unwrap();
        let b = write_temp_file(root.path(), "image.png", b"b").unwrap();
        assert_ne!(a.parent(), b.parent());
        for path in [&a, &b] {
            assert_eq!(path.file_name().unwrap(), "image.png");
            assert_eq!(path.parent().unwrap().parent().unwrap(), root.path());
            assert!(temp_upload_dir(path).is_some());
        }
        assert_eq!(std::fs::read(&a).unwrap(), b"a");
        assert_eq!(std::fs::read(&b).unwrap(), b"b");
    }

    #[test]
    fn a_large_opaque_image_becomes_a_jpeg_within_the_size_limit() {
        let root = TestDir::new();
        let input = root.path().join("vacation.png");
        std::fs::write(&input, uncompressed_png(&opaque(3000, 1000))).unwrap();

        let out = compress_image_file(&input, root.path())
            .unwrap()
            .expect("compressed");

        assert_eq!(out.file_name().unwrap(), "vacation.jpg");
        assert!(temp_upload_dir(&out).is_some());
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(image::guess_format(&bytes).unwrap(), ImageFormat::Jpeg);
        assert_eq!(
            image::load_from_memory(&bytes).unwrap().dimensions(),
            (2048, 683)
        );
        assert!(input.exists());
    }

    #[test]
    fn an_image_with_transparency_becomes_a_png_that_keeps_it() {
        let root = TestDir::new();
        let input = root.path().join("logo.png");
        std::fs::write(&input, uncompressed_png(&translucent(2500, 400))).unwrap();

        let out = compress_image_file(&input, root.path())
            .unwrap()
            .expect("compressed");

        assert_eq!(out.file_name().unwrap(), "logo.png");
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(image::guess_format(&bytes).unwrap(), ImageFormat::Png);
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!(decoded.dimensions(), (2048, 328));
        assert!(has_meaningful_alpha(&decoded));
    }

    #[test]
    fn every_still_format_the_frontend_offers_to_compress_is_re_encoded() {
        let root = TestDir::new();
        let img = noise(64, 64);
        let mut webp = Vec::new();
        img.write_with_encoder(WebPEncoder::new_lossless(&mut webp))
            .unwrap();
        for (name, bytes) in [
            ("photo.jpg", jpeg_with_orientation(&img, 1)),
            ("photo.webp", webp),
            ("photo.bmp", encoded(&img, ImageFormat::Bmp)),
            ("photo.tiff", encoded(&img, ImageFormat::Tiff)),
        ] {
            let input = root.path().join(name);
            std::fs::write(&input, bytes).unwrap();

            let out = compress_image_file(&input, root.path()).unwrap();

            assert_eq!(
                out.and_then(|path| path.file_name().map(|n| n.to_os_string())),
                Some("photo.jpg".into()),
                "{name}"
            );
        }
    }

    #[test]
    fn a_photo_stored_sideways_comes_out_upright() {
        let root = TestDir::new();
        let input = root.path().join("portrait.jpg");
        std::fs::write(&input, jpeg_with_orientation(&noise(400, 200), 6)).unwrap();

        let out = compress_image_file(&input, root.path())
            .unwrap()
            .expect("compressed");

        let decoded = image::load_from_memory(&std::fs::read(&out).unwrap()).unwrap();
        assert_eq!(decoded.dimensions(), (200, 400));
    }

    #[test]
    fn a_file_that_cannot_or_should_not_be_re_encoded_is_returned_untouched() {
        let root = TestDir::new();
        let frame = noise(300, 300);
        let solid = DynamicImage::ImageRgb8(RgbImage::from_pixel(16, 16, Rgb([200, 10, 10])));
        for (name, bytes) in [
            ("party.gif", encoded(&opaque(32, 32), ImageFormat::Gif)),
            ("animated.png", apng(&frame)),
            ("animated.webp", animated_webp(&frame)),
            (
                "not-an-image.png",
                b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec(),
            ),
            ("already-small.png", encoded(&solid, ImageFormat::Png)),
        ] {
            let input = root.path().join(name);
            std::fs::write(&input, &bytes).unwrap();
            let before = root.entries();

            assert_eq!(
                compress_image_file(&input, root.path()).unwrap(),
                None,
                "{name}"
            );
            assert_eq!(std::fs::read(&input).unwrap(), bytes, "{name}");
            assert_eq!(root.entries(), before, "{name}");
        }
    }

    #[test]
    fn a_replaced_input_named_like_a_legacy_temp_file_is_kept() {
        let root = TestDir::new();
        let input = root.path().join("etch-paste-4.png");
        std::fs::write(&input, uncompressed_png(&opaque(300, 300))).unwrap();

        let out = compress_image_file(&input, root.path())
            .unwrap()
            .expect("compressed");

        assert!(input.exists(), "a flat etch-paste file is the user's own");
        assert_eq!(out.file_name().unwrap(), "etch-paste-4.jpg");
    }

    #[test]
    fn a_replaced_directory_temp_input_is_deleted_with_its_directory() {
        let root = TestDir::new();
        let input = write_temp_file(
            root.path(),
            "image.png",
            &uncompressed_png(&opaque(300, 300)),
        )
        .unwrap();
        let input_dir = input.parent().unwrap().to_path_buf();

        let out = compress_image_file(&input, root.path())
            .unwrap()
            .expect("compressed");

        assert!(!input_dir.exists());
        assert_eq!(out.file_name().unwrap(), "image.jpg");
        assert_eq!(
            root.entries(),
            vec![out
                .parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()]
        );
    }
}
