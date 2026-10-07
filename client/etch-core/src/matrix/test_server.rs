use std::sync::{Arc, Mutex};

use matrix_sdk::Client;
use matrix_sdk::authentication::matrix::MatrixSession;
use matrix_sdk::config::RequestConfig;
use matrix_sdk::ruma::api::MatrixVersion;
use matrix_sdk::store::RoomLoadSettings;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A local homeserver stand-in that answers each request by its request line and records
/// what it was sent.
pub(crate) struct CannedHomeserver {
    pub url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl CannedHomeserver {
    pub fn start(status: &'static str, body: &'static str) -> impl Future<Output = Self> {
        Self::answering(move |_| (status, body))
    }

    /// `respond` maps a request line to the status and body to answer it with.
    pub async fn answering(
        respond: impl Fn(&str) -> (&'static str, &'static str) + Send + 'static,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let request_line = read_request(&mut socket).await;
                let (status, body) = respond(&request_line);
                log.lock().unwrap().push(request_line);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{body}",
                    body.len(),
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        Self { url, requests }
    }

    pub fn ok() -> impl Future<Output = Self> {
        Self::start("200 OK", "{}")
    }

    pub fn rejecting_the_token() -> impl Future<Output = Self> {
        Self::start("401 Unauthorized", r#"{"errcode":"M_UNKNOWN_TOKEN","error":"Unknown token","soft_logout":true}"#)
    }

    /// The request line of everything received so far, such as `GET /_matrix/... HTTP/1.1`.
    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }

    /// Counts by path because a restored client also makes crypto requests of its own.
    pub fn requests_to(&self, path: &str) -> usize {
        self.requests().iter().filter(|line| line.contains(path)).count()
    }

    pub async fn client_for(&self, user_id: &str) -> Client {
        self.client_with_version(user_id, MatrixVersion::V1_1).await
    }

    pub async fn client_with_version(&self, user_id: &str, version: MatrixVersion) -> Client {
        logged_in_client(&self.url, user_id, version, RequestConfig::new().disable_retry()).await
    }

    /// Retries a failed request as the app's own client does, where the others make each call one request.
    pub async fn client_that_retries(&self, user_id: &str) -> Client {
        logged_in_client(&self.url, user_id, MatrixVersion::V1_1, RequestConfig::new()).await
    }
}

/// A logged-in client whose homeserver refuses every connection.
pub(crate) fn unreachable_client(user_id: &str) -> impl Future<Output = Client> {
    logged_in_client("http://127.0.0.1:1", user_id, MatrixVersion::V1_1, RequestConfig::new().disable_retry())
}

/// Pins the versions so the client never probes `/versions`.
async fn logged_in_client(homeserver: &str, user_id: &str, version: MatrixVersion, requests: RequestConfig) -> Client {
    let client = Client::builder()
        .homeserver_url(homeserver)
        .server_versions([version])
        .request_config(requests)
        .build()
        .await
        .expect("client should build without contacting the server");
    let session: MatrixSession = serde_json::from_value(serde_json::json!({
        "user_id": user_id,
        "device_id": "TESTDEVICE",
        "access_token": "token",
    })).unwrap();
    client.matrix_auth().restore_session(session, RoomLoadSettings::default()).await.unwrap();
    client
}

/// Consumes the whole request so closing the socket does not reset it under the response.
async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = socket.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            return request_line(&request);
        }
        request.extend_from_slice(&chunk[..n]);
        let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
        let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
        let body_len = headers.lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|len| len.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if request.len() >= end + 4 + body_len {
            return request_line(&request);
        }
    }
}

fn request_line(request: &[u8]) -> String {
    String::from_utf8_lossy(request).lines().next().unwrap_or_default().to_string()
}
