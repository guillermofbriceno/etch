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
    fn mxc_url_uses_the_host_as_the_server() {
        assert_eq!(
            mxc_url(&uri("etch-media://example.org/AbC123"), false),
            "mxc://example.org/AbC123"
        );
    }

    #[test]
    fn mxc_url_ignores_the_query() {
        assert_eq!(
            mxc_url(
                &uri("etch-media://example.org/AbC123?mime=image%2Fpng"),
                false
            ),
            "mxc://example.org/AbC123"
        );
    }

    #[test]
    fn mxc_url_keeps_a_server_port() {
        assert_eq!(
            mxc_url(&uri("etch-media://example.org:8448/AbC123"), false),
            "mxc://example.org:8448/AbC123"
        );
    }

    #[test]
    fn mxc_url_treats_a_localhost_authority_as_the_server_outside_windows() {
        assert_eq!(
            mxc_url(&uri("etch-media://localhost/AbC123"), false),
            "mxc://localhost/AbC123"
        );
        assert_eq!(
            mxc_url(
                &uri("etch-media://localhost:8448/AbC123?mime=image%2Fpng"),
                false
            ),
            "mxc://localhost:8448/AbC123"
        );
    }

    #[test]
    fn mxc_url_reads_the_server_from_the_path_on_the_windows_localhost_form() {
        assert_eq!(
            mxc_url(&uri("etch-media://localhost/example.org/AbC123"), true),
            "mxc://example.org/AbC123"
        );
        assert_eq!(
            mxc_url(
                &uri("etch-media://localhost/example.org/AbC123?mime=video%2Fmp4"),
                true
            ),
            "mxc://example.org/AbC123"
        );
        assert_eq!(
            mxc_url(
                &uri("etch-media://localhost/example.org:8448/AbC123?mime=video%2Fmp4"),
                true
            ),
            "mxc://example.org:8448/AbC123"
        );
    }

    #[test]
    fn mxc_url_reads_a_localhost_server_from_the_path_on_windows() {
        assert_eq!(
            mxc_url(&uri("etch-media://localhost/localhost/AbC123"), true),
            "mxc://localhost/AbC123"
        );
    }

    #[test]
    fn mime_hint_is_read_and_percent_decoded() {
        assert_eq!(
            mime_hint(Some("mime=image%2Fpng")).as_deref(),
            Some("image/png")
        );
        assert_eq!(
            mime_hint(Some("x=1&mime=video%2Fmp4")).as_deref(),
            Some("video/mp4")
        );
        assert_eq!(
            mime_hint(Some("mime=image%2Fsvg%2Bxml")).as_deref(),
            Some("image/svg+xml")
        );
        assert_eq!(
            mime_hint(Some("mime=audio/ogg")).as_deref(),
            Some("audio/ogg")
        );
    }

    #[test]
    fn mime_hint_is_absent_without_a_valid_mime_parameter() {
        assert_eq!(mime_hint(None), None);
        assert_eq!(mime_hint(Some("")), None);
        assert_eq!(mime_hint(Some("type=image%2Fpng")), None);
        assert_eq!(mime_hint(Some("mime=image%2")), None);
        assert_eq!(mime_hint(Some("mime=image%zzpng")), None);
        assert_eq!(mime_hint(Some("mime=%ff%fe")), None);
    }

    #[test]
    fn content_type_passes_image_video_and_audio_hints() {
        assert_eq!(content_type(Some("image/png")), "image/png");
        assert_eq!(content_type(Some("image/jpeg")), "image/jpeg");
        assert_eq!(content_type(Some("video/mp4")), "video/mp4");
        assert_eq!(content_type(Some("audio/ogg")), "audio/ogg");
        assert_eq!(content_type(Some("IMAGE/WebP")), "image/webp");
        assert_eq!(
            content_type(Some("video/mp4; codecs=\"avc1\"")),
            "video/mp4"
        );
    }

    #[test]
    fn content_type_falls_back_to_octet_stream_for_anything_else() {
        for hint in [
            None,
            Some("image/svg+xml"),
            Some("IMAGE/SVG+XML"),
            Some("text/html"),
            Some("application/pdf"),
            Some("application/octet-stream"),
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
    fn parse_range_reads_a_bounded_range() {
        assert_eq!(
            parse_range(Some("bytes=0-99"), 1000),
            ByteRange::Partial { start: 0, end: 99 }
        );
        assert_eq!(
            parse_range(Some("bytes=10-10"), 1000),
            ByteRange::Partial { start: 10, end: 10 }
        );
        assert_eq!(
            parse_range(Some("Bytes=1-2"), 1000),
            ByteRange::Partial { start: 1, end: 2 }
        );
    }

    #[test]
    fn parse_range_clamps_an_end_past_the_body() {
        assert_eq!(
            parse_range(Some("bytes=500-5000"), 1000),
            ByteRange::Partial {
                start: 500,
                end: 999
            }
        );
        assert_eq!(
            parse_range(Some("bytes=0-99999999999999999999999"), 1000),
            ByteRange::Partial { start: 0, end: 999 }
        );
    }

    #[test]
    fn parse_range_reads_an_open_ended_range() {
        assert_eq!(
            parse_range(Some("bytes=100-"), 1000),
            ByteRange::Partial {
                start: 100,
                end: 999
            }
        );
        assert_eq!(
            parse_range(Some("bytes=0-"), 1000),
            ByteRange::Partial { start: 0, end: 999 }
        );
    }

    #[test]
    fn parse_range_reads_a_suffix_range() {
        assert_eq!(
            parse_range(Some("bytes=-100"), 1000),
            ByteRange::Partial {
                start: 900,
                end: 999
            }
        );
        assert_eq!(
            parse_range(Some("bytes=-5000"), 1000),
            ByteRange::Partial { start: 0, end: 999 }
        );
    }

    #[test]
    fn parse_range_rejects_ranges_outside_the_body() {
        assert_eq!(
            parse_range(Some("bytes=1000-"), 1000),
            ByteRange::Unsatisfiable
        );
        assert_eq!(
            parse_range(Some("bytes=1000-1200"), 1000),
            ByteRange::Unsatisfiable
        );
        assert_eq!(
            parse_range(Some("bytes=99999999999999999999999-"), 1000),
            ByteRange::Unsatisfiable
        );
        assert_eq!(
            parse_range(Some("bytes=-0"), 1000),
            ByteRange::Unsatisfiable
        );
        assert_eq!(parse_range(Some("bytes=0-"), 0), ByteRange::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=-5"), 0), ByteRange::Unsatisfiable);
    }

    #[test]
    fn parse_range_serves_the_whole_body_for_malformed_headers() {
        for header in [
            "",
            "bytes",
            "bytes=",
            "bytes=-",
            "bytes=abc",
            "bytes=5-3",
            "bytes=1-2-3",
            "bytes=+1-2",
            "bytes=0x10-",
            "items=0-1",
            "0-1",
        ] {
            assert_eq!(
                parse_range(Some(header), 1000),
                ByteRange::Full,
                "header {header:?}"
            );
        }
        assert_eq!(parse_range(None, 1000), ByteRange::Full);
    }

    #[test]
    fn parse_range_serves_the_whole_body_for_multiple_ranges() {
        assert_eq!(parse_range(Some("bytes=0-1,5-6"), 1000), ByteRange::Full);
        assert_eq!(parse_range(Some("bytes=0-1, -5"), 1000), ByteRange::Full);
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
    fn a_suffix_range_request_gets_the_tail() {
        let response = media_response(b"0123456789".to_vec(), Some("bytes=-3"), "audio/ogg");
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.body(), b"789");
        assert_eq!(
            header_value(&response, header::CONTENT_RANGE),
            Some("bytes 7-9/10")
        );
        assert_eq!(header_value(&response, header::CONTENT_LENGTH), Some("3"));
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

    #[test]
    fn a_multi_range_request_gets_the_whole_body() {
        let response = media_response(b"0123456789".to_vec(), Some("bytes=0-1,4-5"), "video/mp4");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.body(), b"0123456789");
        assert_eq!(header_value(&response, header::CONTENT_LENGTH), Some("10"));
    }
}
