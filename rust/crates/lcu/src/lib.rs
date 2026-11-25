pub mod cache;
pub mod websocket;

use base64::Engine;
use cache::{CacheMetrics, JsonCache, Start};
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::{Client, Url};
use serde_json::Value;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionInfo {
    pub pid: u32,
    pub port: u16,
    pub server: String,
    pub epoch: u64,
}

#[derive(Debug)]
pub enum LcuError {
    NotConnected,
    Http(reqwest::Error),
    Json(reqwest::Error),
    InvalidAuth,
    Unauthorized,
    Asset(String),
    Unsupported(&'static str),
}

impl fmt::Display for LcuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConnected => formatter.write_str("LCU is not connected"),
            Self::Http(error) => write!(formatter, "LCU HTTP error: {error}"),
            Self::Json(error) => write!(formatter, "LCU JSON error: {error}"),
            Self::InvalidAuth => formatter.write_str("invalid LCU authorization header"),
            Self::Unauthorized => formatter.write_str("SGP authentication failed"),
            Self::Asset(message) => write!(formatter, "asset error: {message}"),
            Self::Unsupported(operation) => write!(formatter, "unsupported operation: {operation}"),
        }
    }
}

impl std::error::Error for LcuError {}

#[derive(Clone)]
pub struct LcuClient {
    port: u16,
    client: Client,
    cache: Arc<JsonCache>,
}

impl LcuClient {
    pub fn new(port: u16, token: &str) -> Result<Self, LcuError> {
        let encoded = base64::engine::general_purpose::STANDARD.encode(format!("riot:{token}"));
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Basic {encoded}"))
                .map_err(|_| LcuError::InvalidAuth)?,
        );
        let client = Client::builder()
            .default_headers(headers)
            .no_proxy()
            .http1_only()
            .danger_accept_invalid_certs(true)
            .connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_secs(3))
            .pool_max_idle_per_host(2)
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .map_err(LcuError::Http)?;
        Ok(Self {
            port,
            client,
            cache: Arc::new(JsonCache::default()),
        })
    }

    pub fn url(&self, path: &str) -> String {
        format!("https://127.0.0.1:{}{path}", self.port)
    }

    pub async fn get(&self, path: &str) -> Result<reqwest::Response, LcuError> {
        self.client
            .get(self.url(path))
            .send()
            .await
            .map_err(LcuError::Http)
    }

    pub async fn get_json(&self, path: &str) -> Result<Value, LcuError> {
        let key = self.url(path);
        let ttl = JsonCache::ttl_for(path);
        loop {
            match self.cache.start(&key) {
                Start::Cached(value) => return Ok(value),
                Start::Wait(notify) => {
                    let notified = notify.notified();
                    notified.await;
                }
                Start::Leader => break,
            }
        }

        match self.fetch_json(path).await {
            Ok(value) => {
                let size = serde_json::to_vec(&value)
                    .map(|data| data.len())
                    .unwrap_or(0);
                self.cache.complete(&key, value.clone(), size, ttl);
                Ok(value)
            }
            Err(error) => {
                self.cache.fail(&key);
                Err(error)
            }
        }
    }

    pub fn cache_metrics(&self) -> CacheMetrics {
        self.cache.metrics()
    }

    pub async fn get_json_fresh(&self, path: &str) -> Result<Value, LcuError> {
        parse_json_response(self.get(path).await?).await
    }

    async fn fetch_json(&self, path: &str) -> Result<Value, LcuError> {
        let response = self.get(path).await?;
        parse_json_response(response).await
    }

    pub async fn get_bytes(&self, path: &str, max_bytes: usize) -> Result<Vec<u8>, LcuError> {
        let response = self.get(path).await?;
        if !response.status().is_success() {
            return Err(LcuError::Http(response.error_for_status().unwrap_err()));
        }
        if let Some(length) = response.content_length() {
            if length > max_bytes as u64 {
                return Err(LcuError::Asset("LCU asset exceeds byte budget".to_string()));
            }
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(LcuError::Http)?;
            if bytes.len().saturating_add(chunk.len()) > max_bytes {
                return Err(LcuError::Asset("LCU asset exceeds byte budget".to_string()));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    pub async fn post_json(&self, path: &str, data: Option<Value>) -> Result<Value, LcuError> {
        let mut request = self.client.post(self.url(path));
        if let Some(data) = data {
            request = request.json(&data);
        }
        parse_json_response(request.send().await.map_err(LcuError::Http)?).await
    }

    pub async fn put_json(&self, path: &str, data: Option<Value>) -> Result<Value, LcuError> {
        let mut request = self.client.put(self.url(path));
        if let Some(data) = data {
            request = request.json(&data);
        }
        parse_json_response(request.send().await.map_err(LcuError::Http)?).await
    }

    pub async fn patch_json(&self, path: &str, data: Option<Value>) -> Result<Value, LcuError> {
        let mut request = self.client.patch(self.url(path));
        if let Some(data) = data {
            request = request.json(&data);
        }
        parse_json_response(request.send().await.map_err(LcuError::Http)?).await
    }

    pub async fn delete(&self, path: &str) -> Result<Value, LcuError> {
        parse_json_response(
            self.client
                .delete(self.url(path))
                .send()
                .await
                .map_err(LcuError::Http)?,
        )
        .await
    }
}

async fn parse_json_response(response: reqwest::Response) -> Result<Value, LcuError> {
    let response = response.error_for_status().map_err(LcuError::Http)?;
    if response.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(Value::Null);
    }
    response.json().await.map_err(LcuError::Json)
}

#[derive(Clone)]
pub struct SgpClient {
    client: Client,
    base_url: String,
}

impl SgpClient {
    pub fn new(server: &str) -> Result<Self, LcuError> {
        let server = server.to_ascii_lowercase();
        let host = if matches!(server.as_str(), "hn1" | "hn10" | "bgp2") {
            format!("{server}-k8s-sgp.lol.qq.com")
        } else {
            format!("{server}-sgp.lol.qq.com")
        };
        let client = Client::builder()
            .http1_only()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(8))
            .pool_max_idle_per_host(2)
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .map_err(LcuError::Http)?;
        Ok(Self {
            client,
            base_url: format!("https://{host}:21019"),
        })
    }

    pub async fn get(&self, path: &str, access_token: &str) -> Result<reqwest::Response, LcuError> {
        self.client
            .get(format!("{}{path}", self.base_url))
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(LcuError::Http)
    }

    pub async fn get_json(
        &self,
        path: &str,
        access_token: &str,
        params: Option<&Value>,
    ) -> Result<Value, LcuError> {
        let mut retried = false;
        loop {
            match self.get_json_once(path, access_token, params).await {
                Err(error) if !retried && should_retry_sgp(&error) => {
                    retried = true;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                result => return result,
            }
        }
    }

    pub async fn get_json_once(
        &self,
        path: &str,
        access_token: &str,
        params: Option<&Value>,
    ) -> Result<Value, LcuError> {
        self.send_json(path, access_token, params).await
    }

    async fn send_json(
        &self,
        path: &str,
        access_token: &str,
        params: Option<&Value>,
    ) -> Result<Value, LcuError> {
        let mut request = self
            .client
            .get(format!("{}{path}", self.base_url))
            .bearer_auth(access_token);
        if let Some(params) = params {
            request = request.query(params);
        }
        let response = request.send().await.map_err(LcuError::Http)?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(LcuError::Unauthorized);
        }
        parse_json_response(response).await
    }
}

pub fn should_retry_sgp(error: &LcuError) -> bool {
    match error {
        LcuError::Http(error) => {
            error.is_timeout()
                || error.is_connect()
                || error.status().is_some_and(|status| {
                    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
                })
        }
        _ => false,
    }
}

#[derive(Clone)]
pub struct ExternalClient {
    client: Client,
}

impl ExternalClient {
    pub fn new(proxy: Option<&str>) -> Result<Self, LcuError> {
        let mut builder = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(15))
            .pool_max_idle_per_host(4)
            .redirect(reqwest::redirect::Policy::none())
            .tcp_keepalive(Duration::from_secs(30));
        if let Some(proxy) = proxy {
            let proxy = reqwest::Proxy::all(proxy).map_err(LcuError::Http)?;
            builder = builder.proxy(proxy);
        }
        Ok(Self {
            client: builder.build().map_err(LcuError::Http)?,
        })
    }

    pub async fn get_json(&self, url: &str) -> Result<Value, LcuError> {
        let response = self.client.get(url).send().await.map_err(LcuError::Http)?;
        response
            .error_for_status()
            .map_err(LcuError::Http)?
            .json()
            .await
            .map_err(LcuError::Json)
    }

    pub async fn get_legacy_json(
        &self,
        url: &str,
        allowed_host: &str,
        params: &Value,
        max_bytes: usize,
    ) -> Result<Value, LcuError> {
        let url = Url::parse(url).map_err(|error| LcuError::Asset(error.to_string()))?;
        let host = url
            .host_str()
            .ok_or_else(|| LcuError::Asset("external host is missing".to_string()))?;
        if !matches!(url.scheme(), "http" | "https")
            || !host.eq_ignore_ascii_case(allowed_host)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
        {
            return Err(LcuError::Asset(
                "legacy external host is not allowed".to_string(),
            ));
        }
        let response = self
            .client
            .get(url)
            .query(params)
            .send()
            .await
            .map_err(LcuError::Http)?;
        let response = response.error_for_status().map_err(LcuError::Http)?;
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            return Err(LcuError::Asset(
                "external response exceeds byte budget".to_string(),
            ));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(LcuError::Http)?;
            if bytes.len().saturating_add(chunk.len()) > max_bytes {
                return Err(LcuError::Asset(
                    "external response exceeds byte budget".to_string(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|error| LcuError::Asset(error.to_string()))
    }
}

#[derive(Clone)]
pub struct AssetClient {
    client: Client,
    allowed_hosts: Vec<String>,
}

impl AssetClient {
    pub fn new(proxy: Option<&str>, allowed_hosts: Vec<String>) -> Result<Self, LcuError> {
        let mut builder = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(20))
            .pool_max_idle_per_host(2)
            .redirect(reqwest::redirect::Policy::none());
        if let Some(proxy) = proxy {
            builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(LcuError::Http)?);
        }
        Ok(Self {
            client: builder.build().map_err(LcuError::Http)?,
            allowed_hosts: allowed_hosts
                .into_iter()
                .map(|host| host.to_ascii_lowercase())
                .collect(),
        })
    }

    fn host_allowed(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.allowed_hosts
            .iter()
            .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
    }

    pub async fn download(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>, LcuError> {
        let url = Url::parse(url).map_err(|error| LcuError::Asset(error.to_string()))?;
        if url.scheme() != "https" {
            return Err(LcuError::Asset("only HTTPS assets are allowed".to_string()));
        }
        let host = url
            .host_str()
            .ok_or_else(|| LcuError::Asset("asset host is missing".to_string()))?;
        if !self.host_allowed(host) {
            return Err(LcuError::Asset(format!(
                "asset host is not allowed: {host}"
            )));
        }

        let response = self.client.get(url).send().await.map_err(LcuError::Http)?;
        if !response.status().is_success() {
            return Err(LcuError::Asset(format!(
                "asset returned HTTP {}",
                response.status()
            )));
        }
        if let Some(length) = response.content_length() {
            if length > max_bytes as u64 {
                return Err(LcuError::Asset("asset exceeds byte budget".to_string()));
            }
        }

        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(LcuError::Http)?;
            if bytes.len().saturating_add(chunk.len()) > max_bytes {
                return Err(LcuError::Asset("asset exceeds byte budget".to_string()));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn_sgp_mock(
        responses: Vec<(u16, &'static str)>,
    ) -> (
        String,
        Arc<std::sync::atomic::AtomicUsize>,
        std::thread::JoinHandle<()>,
    ) {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let worker_hits = Arc::clone(&hits);
        let handle = std::thread::spawn(move || {
            'responses: for (status, body) in responses {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
                let (mut socket, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            if std::time::Instant::now() >= deadline {
                                break 'responses;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                        Err(error) => panic!("mock SGP accept failed: {error}"),
                    }
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0_u8; 4096];
                let _ = socket.read(&mut request).unwrap();
                worker_hits.fetch_add(1, Ordering::SeqCst);
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).unwrap();
            }
        });
        (format!("http://{address}"), hits, handle)
    }

    fn mock_sgp_client(base_url: String) -> SgpClient {
        SgpClient {
            client: Client::builder().build().unwrap(),
            base_url,
        }
    }

    #[tokio::test]
    async fn sgp_retries_server_errors_once() {
        let (base_url, hits, worker) = spawn_sgp_mock(vec![
            (503, r#"{"error":"unavailable"}"#),
            (200, r#"{"ok":true}"#),
        ]);
        let client = mock_sgp_client(base_url);
        let result = client.get_json("/test", "token", None).await.unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
        worker.join().unwrap();
    }

    #[tokio::test]
    async fn sgp_does_not_retry_client_errors_or_unauthorized() {
        let (base_url, hits, worker) = spawn_sgp_mock(vec![
            (400, r#"{"error":"bad request"}"#),
            (400, r#"{"error":"bad request"}"#),
        ]);
        let client = mock_sgp_client(base_url);
        assert!(client.get_json("/test", "token", None).await.is_err());
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
        worker.join().unwrap();

        let (base_url, hits, worker) = spawn_sgp_mock(vec![
            (401, r#"{"error":"unauthorized"}"#),
            (401, r#"{"error":"unauthorized"}"#),
        ]);
        let client = mock_sgp_client(base_url);
        assert!(matches!(
            client.get_json("/test", "token", None).await,
            Err(LcuError::Unauthorized)
        ));
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
        worker.join().unwrap();
    }

    #[tokio::test]
    async fn sgp_does_not_retry_invalid_json() {
        let (base_url, hits, worker) =
            spawn_sgp_mock(vec![(200, "not-json"), (200, r#"{"ok":true}"#)]);
        let client = mock_sgp_client(base_url);
        assert!(client.get_json("/test", "token", None).await.is_err());
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
        worker.join().unwrap();
    }

    #[test]
    fn sgp_uses_tencent_host_mapping() {
        for (server, host) in [
            ("HN1", "hn1-k8s-sgp.lol.qq.com"),
            ("HN10", "hn10-k8s-sgp.lol.qq.com"),
            ("BGP2", "bgp2-k8s-sgp.lol.qq.com"),
            ("EUW1", "euw1-sgp.lol.qq.com"),
            ("NA1", "na1-sgp.lol.qq.com"),
        ] {
            let client = SgpClient::new(server).unwrap();
            assert!(client.base_url.contains(host), "{}", client.base_url);
        }
        let client = SgpClient::new("euw1").unwrap();
        assert!(client.base_url.contains("euw1-sgp.lol.qq.com"));
    }

    #[test]
    fn asset_hosts_are_allowlisted() {
        let client = AssetClient::new(None, vec!["example.com".to_string()]).unwrap();
        assert!(client.host_allowed("example.com"));
        assert!(client.host_allowed("cdn.example.com"));
        assert!(!client.host_allowed("evil.example.net"));
    }

    #[test]
    fn isolated_clients_stay_within_idle_memory_budget() {
        let _lcu = LcuClient::new(55123, "test-token").unwrap();
        let _sgp = SgpClient::new("HN1").unwrap();
        let _external = ExternalClient::new(None).unwrap();
        let memory = seraphine_platform::current_process_memory().unwrap();
        println!(
            "isolated clients: working_set={} private_usage={}",
            memory.working_set_bytes, memory.private_usage_bytes
        );
        assert!(memory.working_set_bytes < 10 * 1024 * 1024);
        assert!(memory.private_usage_bytes < 10 * 1024 * 1024);
    }
}
