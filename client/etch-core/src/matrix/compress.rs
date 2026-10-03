use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngDecoder, PngEncoder};
use image::codecs::webp::WebPDecoder;
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{ColorType, DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use mime_guess::mime::{self, Mime};

const MAX_EDGE: u32 = 2048;
const JPEG_QUALITY: u8 = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Jpeg,
    Png,
}

impl OutputFormat {
    fn for_alpha(has_meaningful_alpha: bool) -> Self {
        if has_meaningful_alpha {
            Self::Png
        } else {
            Self::Jpeg
        }
    }

    pub(crate) fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
        }
    }

    pub(crate) fn content_type(self) -> Mime {
        match self {
            Self::Jpeg => mime::IMAGE_JPEG,
            Self::Png => mime::IMAGE_PNG,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Compressed {
    pub bytes: Vec<u8>,
    pub format: OutputFormat,
    pub width: u32,
    pub height: u32,
}

/// `None` means the input should be uploaded as is.
pub(crate) fn compress(data: &[u8]) -> Option<Compressed> {
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
    (bytes.len() < data.len()).then_some(Compressed { bytes, format: output, width, height })
}

/// Reads only the header, and reports the size the image is displayed at.
pub(crate) fn dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let mut decoder = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()?
        .into_decoder()
        .ok()?;
    let (width, height) = decoder.dimensions();
    let sideways = matches!(
        decoder.orientation().unwrap_or(Orientation::NoTransforms),
        Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH
    );
    Some(if sideways { (height, width) } else { (width, height) })
}

fn target_dimensions(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
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

fn has_meaningful_alpha(img: &DynamicImage) -> bool {
    match img {
        DynamicImage::ImageLumaA8(buf) => buf.pixels().any(|p| p.0[1] < u8::MAX),
        DynamicImage::ImageRgba8(buf) => buf.pixels().any(|p| p.0[3] < u8::MAX),
        other if other.color().has_alpha() => other.to_rgba16().pixels().any(|p| p.0[3] < u16::MAX),
        _ => false,
    }
}

fn is_compressible(format: Option<ImageFormat>, animated: bool) -> bool {
    match format {
        Some(ImageFormat::Jpeg | ImageFormat::Bmp | ImageFormat::Tiff) => true,
        Some(ImageFormat::Png | ImageFormat::WebP) => !animated,
        _ => false,
    }
}

fn is_animated(format: ImageFormat, data: &[u8]) -> bool {
    match format {
        ImageFormat::Png => PngDecoder::new(Cursor::new(data))
            .and_then(|d| d.is_apng())
            .unwrap_or(false),
        ImageFormat::WebP => WebPDecoder::new(Cursor::new(data)).is_ok_and(|d| d.has_animation()),
        _ => false,
    }
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

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use image::codecs::webp::WebPEncoder;
    use image::{Rgb, RgbImage, Rgba, RgbaImage};

    pub fn uncompressed_png(img: &DynamicImage) -> Vec<u8> {
        let mut buf = Vec::new();
        img.write_with_encoder(PngEncoder::new_with_quality(
            &mut buf,
            CompressionType::Uncompressed,
            PngFilter::NoFilter,
        ))
        .unwrap();
        buf
    }

    pub fn encoded(img: &DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, format).unwrap();
        buf.into_inner()
    }

    pub fn opaque(width: u32, height: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |x, y| {
            Rgb([
                (x % 256) as u8,
                (y % 256) as u8,
                ((x * 7 + y * 13) % 256) as u8,
            ])
        }))
    }

    pub fn translucent(width: u32, height: u32) -> DynamicImage {
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

    pub fn noise(width: u32, height: u32) -> DynamicImage {
        let mut state = 0x9E37_79B9u32;
        DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |_, _| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            Rgb([state as u8, (state >> 8) as u8, (state >> 16) as u8])
        }))
    }

    pub fn apng(img: &DynamicImage) -> Vec<u8> {
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

    pub fn lossless_webp(img: &DynamicImage) -> Vec<u8> {
        let mut webp = Vec::new();
        img.write_with_encoder(WebPEncoder::new_lossless(&mut webp))
            .unwrap();
        webp
    }

    pub fn animated_webp(img: &DynamicImage) -> Vec<u8> {
        let still = lossless_webp(img);
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

    pub fn jpeg_with_orientation(img: &DynamicImage, orientation: u8) -> Vec<u8> {
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
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use image::{GenericImageView, ImageBuffer, LumaA, Rgb, RgbImage, Rgba, RgbaImage};

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
    fn a_large_opaque_image_becomes_a_jpeg_within_the_size_limit() {
        let out = compress(&uncompressed_png(&opaque(3000, 1000))).expect("compressed");

        assert_eq!(out.format, OutputFormat::Jpeg);
        assert_eq!(image::guess_format(&out.bytes).unwrap(), ImageFormat::Jpeg);
        assert_eq!(image::load_from_memory(&out.bytes).unwrap().dimensions(), (2048, 683));
        assert_eq!((out.width, out.height), (2048, 683));
    }

    #[test]
    fn an_image_with_transparency_becomes_a_png_that_keeps_it() {
        let out = compress(&uncompressed_png(&translucent(2500, 400))).expect("compressed");

        assert_eq!(out.format, OutputFormat::Png);
        assert_eq!(image::guess_format(&out.bytes).unwrap(), ImageFormat::Png);
        let decoded = image::load_from_memory(&out.bytes).unwrap();
        assert_eq!(decoded.dimensions(), (2048, 328));
        assert!(has_meaningful_alpha(&decoded));
    }

    #[test]
    fn every_still_format_core_offers_to_compress_is_re_encoded() {
        let img = noise(64, 64);
        for (name, bytes) in [
            ("photo.jpg", jpeg_with_orientation(&img, 1)),
            ("photo.webp", lossless_webp(&img)),
            ("photo.bmp", encoded(&img, ImageFormat::Bmp)),
            ("photo.tiff", encoded(&img, ImageFormat::Tiff)),
        ] {
            assert_eq!(compress(&bytes).map(|out| out.format), Some(OutputFormat::Jpeg), "{name}");
        }
    }

    #[test]
    fn a_photo_stored_sideways_comes_out_upright() {
        let out = compress(&jpeg_with_orientation(&noise(400, 200), 6)).expect("compressed");

        let decoded = image::load_from_memory(&out.bytes).unwrap();
        assert_eq!(decoded.dimensions(), (200, 400));
        assert_eq!((out.width, out.height), (200, 400));
    }

    #[test]
    fn a_file_that_cannot_or_should_not_be_re_encoded_is_left_alone() {
        let frame = noise(300, 300);
        let solid = DynamicImage::ImageRgb8(RgbImage::from_pixel(16, 16, Rgb([200, 10, 10])));
        for (name, bytes) in [
            ("party.gif", encoded(&opaque(32, 32), ImageFormat::Gif)),
            ("animated.png", apng(&frame)),
            ("animated.webp", animated_webp(&frame)),
            ("not-an-image.png", b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec()),
            ("already-small.png", encoded(&solid, ImageFormat::Png)),
        ] {
            assert!(compress(&bytes).is_none(), "{name}");
        }
    }
}
