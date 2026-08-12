//! Host-owned, policy-gated buffered HTTP(S) transport.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use url::Url;

pub const DEFAULT_FETCH_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_FETCH_BODY_LIMIT: usize = 16 * 1024 * 1024;
pub const DEFAULT_FETCH_REDIRECTS: usize = 5;
pub const DEFAULT_FETCH_WORKERS: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRequest {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl FetchRequest {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchResponse {
    pub url: String,
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub redirected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchErrorKind {
    Policy,
    InvalidRequest,
    Network,
    Timeout,
    RequestTooLarge,
    ResponseTooLarge,
    Redirect,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchError {
    pub kind: FetchErrorKind,
    pub message: String,
}

impl FetchError {
    pub fn new(kind: FetchErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(FetchErrorKind::Unsupported, message)
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FetchError {}

/// Security and resource limits applied by the host before every network hop.
///
/// Origins use URL origin serialization (`scheme://host[:port]`) and are
/// matched exactly. The default policy authorizes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPolicy {
    allowed_origins: BTreeSet<String>,
    pub timeout: Duration,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub max_redirects: usize,
    pub worker_count: usize,
}

impl Default for FetchPolicy {
    fn default() -> Self {
        Self {
            allowed_origins: BTreeSet::new(),
            timeout: DEFAULT_FETCH_TIMEOUT,
            max_request_bytes: DEFAULT_FETCH_BODY_LIMIT,
            max_response_bytes: DEFAULT_FETCH_BODY_LIMIT,
            max_redirects: DEFAULT_FETCH_REDIRECTS,
            worker_count: DEFAULT_FETCH_WORKERS,
        }
    }
}

impl FetchPolicy {
    pub fn allow_origin(&mut self, origin: &str) -> Result<&mut Self, FetchError> {
        let url = Url::parse(origin).map_err(|error| {
            FetchError::new(
                FetchErrorKind::InvalidRequest,
                format!("invalid fetch origin `{origin}`: {error}"),
            )
        })?;
        if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
            return Err(FetchError::new(
                FetchErrorKind::InvalidRequest,
                format!("fetch policy requires an origin, not a URL path: `{origin}`"),
            ));
        }
        let serialized = exact_origin(&url)?;
        self.allowed_origins.insert(serialized);
        Ok(self)
    }

    pub fn with_allowed_origin(mut self, origin: &str) -> Result<Self, FetchError> {
        self.allow_origin(origin)?;
        Ok(self)
    }

    pub fn allowed_origins(&self) -> impl Iterator<Item = &str> {
        self.allowed_origins.iter().map(String::as_str)
    }

    pub fn authorize(&self, url: &Url) -> Result<(), FetchError> {
        let origin = exact_origin(url)?;
        if self.allowed_origins.contains(&origin) {
            Ok(())
        } else {
            Err(FetchError::new(
                FetchErrorKind::Policy,
                format!("fetch origin `{origin}` is not authorized by the host"),
            ))
        }
    }
}

fn exact_origin(url: &Url) -> Result<String, FetchError> {
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(FetchError::new(
            FetchErrorKind::Policy,
            format!("fetch only supports HTTP(S) origins: `{url}`"),
        ));
    }
    Ok(url.origin().ascii_serialization())
}

pub trait FetchHost: Send + Sync + fmt::Debug {
    fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError>;
    fn policy(&self) -> &FetchPolicy;
}

pub type SharedFetchHost = Arc<dyn FetchHost>;

pub fn shared_fetch_host(host: impl FetchHost + 'static) -> SharedFetchHost {
    Arc::new(host)
}

/// Blocking `ureq` implementation. Callers must execute it away from UI/JS
/// threads; `nana-ui-web-api` supplies that worker boundary.
#[derive(Clone)]
pub struct NativeFetchHost {
    policy: FetchPolicy,
    agent: ureq::Agent,
}

impl fmt::Debug for NativeFetchHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeFetchHost")
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl NativeFetchHost {
    pub fn new(policy: FetchPolicy) -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .build();
        Self {
            policy,
            agent: ureq::Agent::new_with_config(config),
        }
    }

    fn one_request(
        &self,
        url: &Url,
        method: &str,
        headers: &[(String, String)],
        body: &[u8],
        timeout: Duration,
    ) -> Result<ureq::http::Response<ureq::Body>, FetchError> {
        let method = ureq::http::Method::from_bytes(method.as_bytes()).map_err(|error| {
            FetchError::new(
                FetchErrorKind::InvalidRequest,
                format!("invalid HTTP method `{method}`: {error}"),
            )
        })?;
        if matches!(method.as_str(), "CONNECT" | "TRACE" | "TRACK") {
            return Err(FetchError::new(
                FetchErrorKind::InvalidRequest,
                format!("HTTP method `{method}` is not supported"),
            ));
        }
        let mut builder = ureq::http::Request::builder()
            .method(method)
            .uri(url.as_str());
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        let request = builder.body(body.to_vec()).map_err(|error| {
            FetchError::new(
                FetchErrorKind::InvalidRequest,
                format!("invalid fetch request: {error}"),
            )
        })?;
        let request = self
            .agent
            .configure_request(request)
            .timeout_global(Some(timeout))
            .build();
        self.agent.run(request).map_err(map_ureq_error)
    }
}

impl FetchHost for NativeFetchHost {
    fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        if request.body.len() > self.policy.max_request_bytes {
            return Err(FetchError::new(
                FetchErrorKind::RequestTooLarge,
                format!(
                    "fetch request body exceeds {} bytes",
                    self.policy.max_request_bytes
                ),
            ));
        }

        let mut url = Url::parse(&request.url).map_err(|error| {
            FetchError::new(
                FetchErrorKind::InvalidRequest,
                format!("invalid fetch URL `{}`: {error}", request.url),
            )
        })?;
        let mut method = request.method.to_ascii_uppercase();
        let mut headers = request.headers;
        let mut body = request.body;
        let mut redirects = 0usize;
        let started = std::time::Instant::now();

        loop {
            self.policy.authorize(&url)?;
            let previous_origin = exact_origin(&url)?;
            let remaining = self
                .policy
                .timeout
                .checked_sub(started.elapsed())
                .ok_or_else(|| {
                    FetchError::new(FetchErrorKind::Timeout, "fetch exceeded its total timeout")
                })?;
            let mut response = self.one_request(&url, &method, &headers, &body, remaining)?;
            let status = response.status().as_u16();
            if matches!(status, 301 | 302 | 303 | 307 | 308)
                && let Some(location) = response.headers().get(ureq::http::header::LOCATION)
            {
                if redirects >= self.policy.max_redirects {
                    return Err(FetchError::new(
                        FetchErrorKind::Redirect,
                        format!(
                            "fetch exceeded the {} redirect limit",
                            self.policy.max_redirects
                        ),
                    ));
                }
                let location = location.to_str().map_err(|error| {
                    FetchError::new(
                        FetchErrorKind::Redirect,
                        format!("invalid redirect Location header: {error}"),
                    )
                })?;
                let next = url.join(location).map_err(|error| {
                    FetchError::new(
                        FetchErrorKind::Redirect,
                        format!("invalid redirect URL `{location}`: {error}"),
                    )
                })?;
                self.policy.authorize(&next)?;
                if exact_origin(&next)? != previous_origin {
                    headers.retain(|(name, _)| {
                        !name.eq_ignore_ascii_case("authorization")
                            && !name.eq_ignore_ascii_case("proxy-authorization")
                    });
                }
                if status == 303 || ((status == 301 || status == 302) && method == "POST") {
                    method = "GET".into();
                    body.clear();
                    headers.retain(|(name, _)| {
                        !name.eq_ignore_ascii_case("content-length")
                            && !name.eq_ignore_ascii_case("content-type")
                    });
                }
                url = next;
                redirects += 1;
                continue;
            }

            if let Some(length) = response
                .headers()
                .get(ureq::http::header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<usize>().ok())
                && length > self.policy.max_response_bytes
            {
                return Err(FetchError::new(
                    FetchErrorKind::ResponseTooLarge,
                    format!(
                        "fetch response body exceeds {} bytes",
                        self.policy.max_response_bytes
                    ),
                ));
            }
            let response_headers = response
                .headers()
                .iter()
                .filter(|(name, _)| !name.as_str().eq_ignore_ascii_case("set-cookie"))
                .map(|(name, value)| {
                    (
                        name.as_str().to_string(),
                        value.to_str().unwrap_or_default().to_string(),
                    )
                })
                .collect();
            let bytes = response
                .body_mut()
                .with_config()
                .limit(self.policy.max_response_bytes.saturating_add(1) as u64)
                .read_to_vec()
                .map_err(|error| match error {
                    ureq::Error::BodyExceedsLimit(_) => FetchError::new(
                        FetchErrorKind::ResponseTooLarge,
                        format!(
                            "fetch response body exceeds {} bytes",
                            self.policy.max_response_bytes
                        ),
                    ),
                    other => map_ureq_error(other),
                })?;
            if bytes.len() > self.policy.max_response_bytes {
                return Err(FetchError::new(
                    FetchErrorKind::ResponseTooLarge,
                    format!(
                        "fetch response body exceeds {} bytes",
                        self.policy.max_response_bytes
                    ),
                ));
            }
            let status_text = response
                .status()
                .canonical_reason()
                .unwrap_or_default()
                .to_string();
            return Ok(FetchResponse {
                url: url.to_string(),
                status,
                status_text,
                headers: response_headers,
                body: bytes,
                redirected: redirects > 0,
            });
        }
    }

    fn policy(&self) -> &FetchPolicy {
        &self.policy
    }
}

fn map_ureq_error(error: ureq::Error) -> FetchError {
    let kind = if matches!(error, ureq::Error::Timeout(_)) {
        FetchErrorKind::Timeout
    } else {
        FetchErrorKind::Network
    };
    FetchError::new(kind, format!("fetch network error: {error}"))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;

    use super::*;

    fn read_request(stream: &mut TcpStream) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 2048];
        let mut expected = None;
        loop {
            let read = stream.read(&mut chunk).expect("read loopback request");
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..read]);
            if expected.is_none()
                && let Some(split) = bytes.windows(4).position(|part| part == b"\r\n\r\n")
            {
                let head = String::from_utf8_lossy(&bytes[..split]);
                let length = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                expected = Some(split + 4 + length);
            }
            if expected.is_some_and(|expected| bytes.len() >= expected) {
                break;
            }
        }
        bytes
    }

    fn one_response_server(
        response: Vec<u8>,
    ) -> (String, mpsc::Receiver<Vec<u8>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback server");
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = mpsc::channel();
        let join = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept loopback request");
            let request = read_request(&mut stream);
            request_tx.send(request).unwrap();
            stream
                .write_all(&response)
                .expect("write loopback response");
        });
        (format!("http://{address}"), request_rx, join)
    }

    #[test]
    fn default_policy_denies_every_origin() {
        let policy = FetchPolicy::default();
        let error = policy
            .authorize(&Url::parse("https://example.com/a").unwrap())
            .unwrap_err();
        assert_eq!(error.kind, FetchErrorKind::Policy);
    }

    #[test]
    fn policy_matches_normalized_origin_not_path() {
        let policy = FetchPolicy::default()
            .with_allowed_origin("https://example.com:443")
            .unwrap();
        policy
            .authorize(&Url::parse("https://example.com/path?q=1").unwrap())
            .unwrap();
        assert!(
            policy
                .authorize(&Url::parse("https://api.example.com/path").unwrap())
                .is_err()
        );
    }

    #[test]
    fn native_fetch_preserves_method_headers_body_and_http_error_response() {
        let (origin, request_rx, join) = one_response_server(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 3\r\nContent-Type: application/octet-stream\r\nX-Test: response\r\nConnection: close\r\n\r\n\0\x01\xff"
                .to_vec(),
        );
        let policy = FetchPolicy::default().with_allowed_origin(&origin).unwrap();
        let host = NativeFetchHost::new(policy);
        let response = host
            .fetch(FetchRequest {
                url: format!("{origin}/items"),
                method: "PATCH".into(),
                headers: vec![("X-Request".into(), "nana".into())],
                body: br#"{"enabled":true}"#.to_vec(),
            })
            .unwrap();
        let request = String::from_utf8(request_rx.recv().unwrap()).unwrap();
        join.join().unwrap();

        assert!(request.starts_with("PATCH /items HTTP/1.1\r\n"));
        assert!(request.to_ascii_lowercase().contains("x-request: nana"));
        assert!(request.ends_with(r#"{"enabled":true}"#));
        assert_eq!(response.status, 404);
        assert_eq!(response.body, vec![0, 1, 255]);
        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| name == "x-test" && value == "response")
        );
    }

    #[test]
    fn native_fetch_follows_authorized_redirect_and_buffers_json() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let origin = format!("http://{address}");
        let join = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let _ = read_request(&mut first);
            first
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            let (mut second, _) = listener.accept().unwrap();
            let request = read_request(&mut second);
            assert!(String::from_utf8_lossy(&request).starts_with("GET /final HTTP/1.1\r\n"));
            let body = br#"{"source":"nana"}"#;
            write!(
                second,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            second.write_all(body).unwrap();
        });
        let policy = FetchPolicy::default().with_allowed_origin(&origin).unwrap();
        let response = NativeFetchHost::new(policy)
            .fetch(FetchRequest::get(format!("{origin}/redirect")))
            .unwrap();
        join.join().unwrap();

        assert!(response.redirected);
        assert!(response.url.ends_with("/final"));
        let json: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(json["source"], "nana");
    }

    #[test]
    fn redirect_to_another_origin_is_reauthorized() {
        let forbidden = TcpListener::bind("127.0.0.1:0").unwrap();
        let forbidden_origin = format!("http://{}", forbidden.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {forbidden_origin}/secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .into_bytes();
        let (origin, _request_rx, join) = one_response_server(response);
        let policy = FetchPolicy::default().with_allowed_origin(&origin).unwrap();
        let error = NativeFetchHost::new(policy)
            .fetch(FetchRequest::get(format!("{origin}/redirect")))
            .unwrap_err();
        join.join().unwrap();
        drop(forbidden);
        assert_eq!(error.kind, FetchErrorKind::Policy);
    }

    #[test]
    fn native_fetch_enforces_timeout_and_body_limits() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let origin = format!("http://{address}");
        let join = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_request(&mut stream);
            thread::sleep(Duration::from_millis(150));
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        });
        let mut timeout_policy = FetchPolicy::default().with_allowed_origin(&origin).unwrap();
        timeout_policy.timeout = Duration::from_millis(25);
        let error = NativeFetchHost::new(timeout_policy)
            .fetch(FetchRequest::get(format!("{origin}/slow")))
            .unwrap_err();
        join.join().unwrap();
        assert_eq!(error.kind, FetchErrorKind::Timeout);

        let (origin, _request_rx, join) = one_response_server(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nlarge".to_vec(),
        );
        let mut response_policy = FetchPolicy::default().with_allowed_origin(&origin).unwrap();
        response_policy.max_response_bytes = 4;
        let error = NativeFetchHost::new(response_policy)
            .fetch(FetchRequest::get(format!("{origin}/large")))
            .unwrap_err();
        join.join().unwrap();
        assert_eq!(error.kind, FetchErrorKind::ResponseTooLarge);

        let mut request_policy = FetchPolicy::default()
            .with_allowed_origin("https://example.com")
            .unwrap();
        request_policy.max_request_bytes = 2;
        let error = NativeFetchHost::new(request_policy)
            .fetch(FetchRequest {
                url: "https://example.com/upload".into(),
                method: "POST".into(),
                headers: Vec::new(),
                body: vec![1, 2, 3],
            })
            .unwrap_err();
        assert_eq!(error.kind, FetchErrorKind::RequestTooLarge);
    }
}
