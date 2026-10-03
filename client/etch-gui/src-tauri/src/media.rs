use tauri::http::Uri;

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

pub(crate) fn percent_decode(s: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(s: &str) -> Uri {
        s.parse().unwrap()
    }

    #[test]
    fn mxc_url_takes_the_server_from_the_authority_or_on_windows_from_the_path() {
        for (request, on_windows, expected) in [
            (
                "etch-media://example.org/AbC123",
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
                "etch-media://localhost/example.org:8448/AbC123",
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
    fn percent_decode_reads_utf8_names_and_rejects_malformed_escapes() {
        assert_eq!(percent_decode("r%C3%A9sum%C3%A9.pdf").as_deref(), Some("résumé.pdf"));
        for malformed in ["photo%2", "photo%zz.png", "%ff%fe"] {
            assert_eq!(percent_decode(malformed), None, "{malformed}");
        }
    }
}
