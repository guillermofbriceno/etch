use tauri::http::{header, Response, StatusCode, Uri};

const FALLBACK_CONTENT_TYPE: &str = "application/octet-stream";

/// `on_windows` is a parameter so both URL forms are testable on any host.
pub(crate) fn mxc_url(uri: &Uri, on_windows: bool) -> String {
    let authority = uri.authority().map(|a| a.as_str()).unwrap_or_default();
    let path = uri.path().trim_start_matches('/');
    // On Windows, wry turns http://etch-media.localhost/<server>/<id> back into etch-media://localhost/<server>/<id>.
    if on_windows && authority == "localhost" {
        format!("mxc://{path}")
    } else {
        format!("mxc://{authority}/{path}")
    }
}

pub(crate) fn mime_hint(query: Option<&str>) -> Option<String> {
    query?
        .split('&')
        .find_map(|pair| pair.strip_prefix("mime="))
        .and_then(percent_decode)
}

pub(crate) fn content_type(mime_hint: Option<&str>) -> String {
    mime_hint
        .and_then(allowed_media_type)
        .unwrap_or_else(|| FALLBACK_CONTENT_TYPE.to_owned())
}

fn allowed_media_type(hint: &str) -> Option<String> {
    let essence = hint.split(';').next()?.trim().to_ascii_lowercase();
    let (kind, subtype) = essence.split_once('/')?;
    let is_token = !subtype.is_empty() && subtype.bytes().all(is_tchar);
    let allowed = match kind {
        "image" => subtype != "svg+xml",
        "video" | "audio" => true,
        _ => false,
    };
    (is_token && allowed).then_some(essence)
}

fn is_tchar(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

fn percent_decode(s: &str) -> Option<String> {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = hex(*bytes.get(i + 1)?)?;
            let lo = hex(*bytes.get(i + 2)?)?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ByteRange {
    Full,
    Partial { start: u64, end: u64 },
    Unsatisfiable,
}

/// Anything other than a single well-formed byte range is ignored, which serves the whole body.
pub(crate) fn parse_range(header: Option<&str>, len: u64) -> ByteRange {
    let Some((unit, spec)) = header.and_then(|h| h.trim().split_once('=')) else {
        return ByteRange::Full;
    };
    if !unit.trim().eq_ignore_ascii_case("bytes") || spec.contains(',') {
        return ByteRange::Full;
    }
    let Some((first, last)) = spec.trim().split_once('-') else {
        return ByteRange::Full;
    };
    let (first, last) = (first.trim(), last.trim());
    match (parse_position(first), parse_position(last)) {
        (Some(start), Some(end)) if start <= end => bounded_range(start, end, len),
        (Some(start), None) if last.is_empty() => bounded_range(start, u64::MAX, len),
        (None, Some(suffix)) if first.is_empty() => {
            if suffix == 0 || len == 0 {
                ByteRange::Unsatisfiable
            } else {
                ByteRange::Partial {
                    start: len.saturating_sub(suffix),
                    end: len - 1,
                }
            }
        }
        _ => ByteRange::Full,
    }
}

fn bounded_range(start: u64, end: u64, len: u64) -> ByteRange {
    if start >= len {
        ByteRange::Unsatisfiable
    } else {
        ByteRange::Partial {
            start,
            end: end.min(len - 1),
        }
    }
}

fn parse_position(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(s.parse().unwrap_or(u64::MAX))
}

pub(crate) fn media_response(
    body: Vec<u8>,
    range: Option<&str>,
    content_type: &str,
) -> Response<Vec<u8>> {
    let len = body.len() as u64;
    let builder = Response::builder()
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCEPT_RANGES, "bytes");
    let (builder, body) = match parse_range(range, len) {
        ByteRange::Full => (
            builder
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type),
            body,
        ),
        ByteRange::Partial { start, end } => {
            let mut body = body;
            body.truncate(end as usize + 1);
            body.drain(..start as usize);
            (
                builder
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header(header::CONTENT_TYPE, content_type)
                    .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}")),
                body,
            )
        }
        ByteRange::Unsatisfiable => (
            builder
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{len}")),
            Vec::new(),
        ),
    };
    builder
        .header(header::CONTENT_LENGTH, body.len())
        .body(body)
        .expect("media response headers are built from validated values")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(s: &str) -> Uri {
        s.parse().unwrap()
    }

    fn header_value(response: &Response<Vec<u8>>, name: header::HeaderName) -> Option<&str> {
        response.headers().get(name).map(|v| v.to_str().unwrap())
    }

    #[test]
    fn mxc_url_takes_the_server_from_the_authority_or_on_windows_from_the_path() {
        for (request, on_windows, expected) in [
            (
                "etch-media://example.org/AbC123?mime=image%2Fpng",
                false,
                "mxc://example.org/AbC123",
            ),
            (
                "etch-media://example.org:8448/AbC123",
                false,
                "mxc://example.org:8448/AbC123",
            ),
            (
                "etch-media://localhost/AbC123",
                false,
                "mxc://localhost/AbC123",
            ),
            (
                "etch-media://localhost/example.org:8448/AbC123?mime=video%2Fmp4",
                true,
                "mxc://example.org:8448/AbC123",
            ),
            (
                "etch-media://localhost/localhost/AbC123",
                true,
                "mxc://localhost/AbC123",
            ),
        ] {
            assert_eq!(mxc_url(&uri(request), on_windows), expected, "{request}");
        }
    }

    #[test]
    fn mime_hint_is_the_percent_decoded_mime_parameter_or_nothing() {
        assert_eq!(
            mime_hint(Some("x=1&mime=video%2Fmp4")).as_deref(),
            Some("video/mp4")
        );
        for query in [
            None,
            Some(""),
            Some("type=image%2Fpng"),
            Some("mime=image%2"),
            Some("mime=image%zzpng"),
            Some("mime=%ff%fe"),
        ] {
            assert_eq!(mime_hint(query), None, "query {query:?}");
        }
    }

    #[test]
    fn only_a_clean_image_video_or_audio_hint_becomes_the_content_type() {
        assert_eq!(content_type(Some("IMAGE/WebP")), "image/webp");
        assert_eq!(content_type(Some("audio/ogg")), "audio/ogg");
        assert_eq!(
            content_type(Some("video/mp4; codecs=\"avc1\"")),
            "video/mp4"
        );
        for hint in [
            None,
            Some("image/svg+xml"),
            Some("IMAGE/SVG+XML"),
            Some("text/html"),
            Some("application/pdf"),
            Some("image/"),
            Some("image"),
            Some(""),
            Some("image/png\r\nx-injected: 1"),
            Some("video/mp 4"),
        ] {
            assert_eq!(content_type(hint), FALLBACK_CONTENT_TYPE, "hint {hint:?}");
        }
    }

    #[test]
    fn parse_range_clamps_one_byte_range_to_the_body_and_ignores_anything_else() {
        let partial = |start, end| ByteRange::Partial { start, end };
        for (header, len, expected) in [
            ("bytes=0-99", 1000, partial(0, 99)),
            ("Bytes=10-10", 1000, partial(10, 10)),
            ("bytes=500-5000", 1000, partial(500, 999)),
            ("bytes=0-99999999999999999999999", 1000, partial(0, 999)),
            ("bytes=100-", 1000, partial(100, 999)),
            ("bytes=-100", 1000, partial(900, 999)),
            ("bytes=-5000", 1000, partial(0, 999)),
            ("bytes=1000-", 1000, ByteRange::Unsatisfiable),
            ("bytes=1000-1200", 1000, ByteRange::Unsatisfiable),
            (
                "bytes=99999999999999999999999-",
                1000,
                ByteRange::Unsatisfiable,
            ),
            ("bytes=-0", 1000, ByteRange::Unsatisfiable),
            ("bytes=0-", 0, ByteRange::Unsatisfiable),
            ("bytes=-5", 0, ByteRange::Unsatisfiable),
            ("", 1000, ByteRange::Full),
            ("bytes", 1000, ByteRange::Full),
            ("bytes=", 1000, ByteRange::Full),
            ("bytes=-", 1000, ByteRange::Full),
            ("bytes=abc", 1000, ByteRange::Full),
            ("bytes=5-3", 1000, ByteRange::Full),
            ("bytes=1-2-3", 1000, ByteRange::Full),
            ("bytes=+1-2", 1000, ByteRange::Full),
            ("bytes=0x10-", 1000, ByteRange::Full),
            ("items=0-1", 1000, ByteRange::Full),
            ("0-1", 1000, ByteRange::Full),
            ("bytes=0-1,5-6", 1000, ByteRange::Full),
            ("bytes=0-1, -5", 1000, ByteRange::Full),
        ] {
            assert_eq!(parse_range(Some(header), len), expected, "header {header:?}");
        }
        assert_eq!(parse_range(None, 1000), ByteRange::Full);
    }

    #[test]
    fn a_request_without_a_range_gets_the_whole_body() {
        let response = media_response(b"0123456789".to_vec(), None, "video/mp4");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.body(), b"0123456789");
        assert_eq!(
            header_value(&response, header::CONTENT_TYPE),
            Some("video/mp4")
        );
        assert_eq!(header_value(&response, header::CONTENT_LENGTH), Some("10"));
        assert_eq!(
            header_value(&response, header::ACCEPT_RANGES),
            Some("bytes")
        );
        assert_eq!(header_value(&response, header::CONTENT_RANGE), None);
    }

    #[test]
    fn a_range_request_gets_the_slice() {
        let response = media_response(b"0123456789".to_vec(), Some("bytes=2-5"), "video/mp4");
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.body(), b"2345");
        assert_eq!(
            header_value(&response, header::CONTENT_RANGE),
            Some("bytes 2-5/10")
        );
        assert_eq!(header_value(&response, header::CONTENT_LENGTH), Some("4"));
        assert_eq!(
            header_value(&response, header::ACCEPT_RANGES),
            Some("bytes")
        );
        assert_eq!(
            header_value(&response, header::CONTENT_TYPE),
            Some("video/mp4")
        );
    }

    #[test]
    fn an_unsatisfiable_range_gets_416() {
        let response = media_response(b"0123456789".to_vec(), Some("bytes=10-"), "video/mp4");
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert!(response.body().is_empty());
        assert_eq!(
            header_value(&response, header::CONTENT_RANGE),
            Some("bytes */10")
        );
        assert_eq!(header_value(&response, header::CONTENT_LENGTH), Some("0"));
    }
}
