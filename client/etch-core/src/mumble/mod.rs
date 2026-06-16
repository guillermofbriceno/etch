pub mod bridge;
pub mod cert;
pub mod process;
pub mod service;

/// Percent-encode a string, preserving only the RFC 3986 unreserved set
/// (alphanumeric, `-`, `.`, `_`, `~`); every other byte becomes `%XX`.
/// Shared by URL path-segment encoding and userinfo (username/password)
/// encoding — anywhere a value goes into a `mumble://` URL component.
pub(crate) fn percent_encode_unreserved(s: &str) -> String {
    const HEX: [u8; 16] = *b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push(char::from(HEX[(byte >> 4) as usize]));
                out.push(char::from(HEX[(byte & 0x0f) as usize]));
            }
        }
    }
    out
}
