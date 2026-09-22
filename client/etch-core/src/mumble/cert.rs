use std::path::Path;
use std::time::Duration;
use sha1::{Sha1, Digest};
use tokio::net::TcpStream;
use crate::error::*;

/// Ceiling on a whole probe: name resolution, TCP connect and TLS handshake.
///
/// Neither a TCP connect nor a TLS handshake bounds itself, so against a black
/// hole this would otherwise wait out the kernel's SYN retries, or forever on a
/// peer that connects and then goes quiet. The probe runs on the engine's event
/// loop, so that wait stalls every queued frontend command.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// TLS-connect to host:port (accepting any cert), return SHA1 hex of the
/// DER-encoded leaf certificate.
///
/// Gives up after `PROBE_TIMEOUT`. Callers treat that like any other probe
/// failure: the fingerprint check is skipped and the connection proceeds.
pub async fn probe_server_cert(host: &str, port: u16) -> Result<String, CoreError> {
    let addr = format!("{}:{}", host, port);

    tokio::time::timeout(PROBE_TIMEOUT, probe(host, &addr))
        .await
        .map_err(|_| CertProbeSnafu {
            message: format!("Timed out after {:?} probing {}", PROBE_TIMEOUT, addr),
        }.build())?
}

async fn probe(host: &str, addr: &str) -> Result<String, CoreError> {
    let tcp = TcpStream::connect(addr).await
        .map_err(|e| CertProbeSnafu { message: format!("TCP connect to {}: {}", addr, e) }.build())?;

    let tls_connector = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| CertProbeSnafu { message: format!("Building TLS connector: {}", e) }.build())?;

    let connector = tokio_native_tls::TlsConnector::from(tls_connector);

    let tls_stream = connector.connect(host, tcp).await
        .map_err(|e| CertProbeSnafu { message: format!("TLS handshake with {}: {}", addr, e) }.build())?;

    let cert = tls_stream.get_ref().peer_certificate()
        .map_err(|e| CertProbeSnafu { message: format!("Reading peer cert from {}: {}", addr, e) }.build())?
        .ok_or_else(|| CertProbeSnafu { message: format!("No peer certificate from {}", addr) }.build())?;

    let der = cert.to_der()
        .map_err(|e| CertProbeSnafu { message: format!("Encoding cert to DER: {}", e) }.build())?;

    Ok(format!("{:x}", Sha1::digest(&der)))
}

/// Read the stored fingerprint for a given host:port from mumble.sqlite's cert table.
pub fn get_stored_cert(db_path: &Path, host: &str, port: u16) -> Option<String> {
    let conn = rusqlite::Connection::open(db_path).ok()?;
    conn.query_row(
        "SELECT digest FROM cert WHERE hostname = ?1 AND port = ?2",
        rusqlite::params![host, port as i64],
        |row| row.get::<_, String>(0),
    ).ok()
}

/// Insert or update the cert fingerprint in mumble.sqlite.
pub fn store_cert(db_path: &Path, host: &str, port: u16, digest: &str) -> Result<(), CoreError> {
    let conn = rusqlite::Connection::open(db_path)
        .map_err(|e| CertProbeSnafu { message: format!("Opening {}: {}", db_path.display(), e) }.build())?;

    conn.execute(
        "INSERT INTO cert (hostname, port, digest) VALUES (?1, ?2, ?3) \
         ON CONFLICT(hostname, port) DO UPDATE SET digest = excluded.digest",
        rusqlite::params![host, port as i64, digest],
    ).map_err(|e| CertProbeSnafu { message: format!("Writing cert to {}: {}", db_path.display(), e) }.build())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn create_test_db() -> NamedTempFile {
        let file = NamedTempFile::new().unwrap();
        let conn = rusqlite::Connection::open(file.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE cert (id INTEGER PRIMARY KEY AUTOINCREMENT, hostname TEXT, port INTEGER, digest TEXT);
             CREATE UNIQUE INDEX cert_host_port ON cert(hostname, port);"
        ).unwrap();
        file
    }

    #[test]
    fn get_stored_cert_returns_none_for_empty_db() {
        let db = create_test_db();
        assert_eq!(get_stored_cert(db.path(), "example.com", 64738), None);
    }

    #[test]
    fn store_and_retrieve_cert() {
        let db = create_test_db();
        store_cert(db.path(), "example.com", 64738, "abcdef1234567890").unwrap();
        assert_eq!(
            get_stored_cert(db.path(), "example.com", 64738),
            Some("abcdef1234567890".to_string()),
        );
    }

    #[test]
    fn store_cert_overwrites_existing() {
        let db = create_test_db();
        store_cert(db.path(), "example.com", 64738, "old_fingerprint").unwrap();
        store_cert(db.path(), "example.com", 64738, "new_fingerprint").unwrap();
        assert_eq!(
            get_stored_cert(db.path(), "example.com", 64738),
            Some("new_fingerprint".to_string()),
        );
    }

    #[test]
    fn different_hosts_are_independent() {
        let db = create_test_db();
        store_cert(db.path(), "host-a.com", 64738, "fp_a").unwrap();
        store_cert(db.path(), "host-b.com", 64738, "fp_b").unwrap();
        assert_eq!(get_stored_cert(db.path(), "host-a.com", 64738), Some("fp_a".to_string()));
        assert_eq!(get_stored_cert(db.path(), "host-b.com", 64738), Some("fp_b".to_string()));
    }

    #[test]
    fn different_ports_are_independent() {
        let db = create_test_db();
        store_cert(db.path(), "example.com", 64738, "fp_default").unwrap();
        store_cert(db.path(), "example.com", 64739, "fp_other").unwrap();
        assert_eq!(get_stored_cert(db.path(), "example.com", 64738), Some("fp_default".to_string()));
        assert_eq!(get_stored_cert(db.path(), "example.com", 64739), Some("fp_other".to_string()));
    }

    /// A server that completes the TCP handshake but never speaks TLS must not
    /// stall the probe forever. `probe_server_cert` runs on the engine's event
    /// loop, so an unbounded hang there freezes every queued frontend command.
    #[tokio::test]
    async fn probe_gives_up_on_a_server_that_never_completes_the_handshake() {
        // Accept connections and then do nothing, holding the socket open.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let _server = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });

        let probe = probe_server_cert("127.0.0.1", port);
        let outcome = tokio::time::timeout(PROBE_TIMEOUT * 2, probe).await;

        assert!(
            outcome.is_ok(),
            "probe_server_cert must time out on its own rather than hang the caller",
        );
        assert!(
            outcome.unwrap().is_err(),
            "a stalled handshake should surface as a probe error",
        );
    }

    #[test]
    fn get_stored_cert_returns_none_for_missing_db() {
        let result = get_stored_cert(Path::new("/nonexistent/path/mumble.sqlite"), "x", 1);
        assert_eq!(result, None);
    }

}
