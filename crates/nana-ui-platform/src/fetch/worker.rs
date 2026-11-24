use std::fmt;
use std::io::Read as _;
use std::time::Duration;

use ureq::unversioned::transport::{ConnectProxyConnector, Connector as _, RustlsConnector};
use url::Url;

use super::policy::exact_origin;
use super::transport::{CancellableResolver, CancellableTcpConnector};
use super::{
    FetchCancellation, FetchError, FetchErrorKind, FetchHead, FetchHost, FetchPolicy, FetchRequest,
    FetchResponse, FetchSink,
};

/// Collects a streamed response back into one buffered [`FetchResponse`].
#[derive(Debug, Default)]
struct BufferingSink {
    head: Option<FetchHead>,
    body: Vec<u8>,
}

impl FetchSink for BufferingSink {
    fn head(&mut self, head: FetchHead) -> Result<(), FetchError> {
        self.head = Some(head);
        Ok(())
    }

    fn chunk(&mut self, bytes: &[u8]) -> Result<(), FetchError> {
        self.body.extend_from_slice(bytes);
        Ok(())
    }
}

/// Blocking `ureq` implementation. Callers must execute it away from the UI
/// thread.
#[derive(Clone)]
pub struct NativeFetchHost {
    policy: FetchPolicy,
    config: ureq::config::Config,
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
        Self { policy, config }
    }

    fn one_request(
        &self,
        url: &Url,
        method: &str,
        headers: &[(String, String)],
        body: &[u8],
        timeout: Duration,
        cancellation: &FetchCancellation,
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
        let connector =
            ().chain(ConnectProxyConnector::default())
                .chain(CancellableTcpConnector::new(cancellation.clone()))
                .chain(RustlsConnector::default());
        let agent = ureq::Agent::with_parts(
            self.config.clone(),
            connector,
            CancellableResolver::new(cancellation.clone()),
        );
        let request = agent
            .configure_request(request)
            .timeout_global(Some(timeout))
            .build();
        agent.run(request).map_err(|error| {
            if cancellation.is_cancelled() {
                FetchError::new(FetchErrorKind::Cancelled, "fetch was cancelled")
            } else {
                map_ureq_error(error)
            }
        })
    }

    fn fetch_impl(
        &self,
        request: FetchRequest,
        cancellation: &FetchCancellation,
    ) -> Result<FetchResponse, FetchError> {
        let mut sink = BufferingSink::default();
        self.stream_impl(request, cancellation, &mut sink)?;
        let head = sink.head.ok_or_else(|| {
            FetchError::new(FetchErrorKind::Network, "fetch produced no response head")
        })?;
        Ok(head.with_body(sink.body))
    }

    /// The one transport path. Buffered fetches run it with a collecting sink,
    /// so redirect re-authorization, header stripping and the size caps cannot
    /// drift between the two modes.
    fn stream_impl(
        &self,
        request: FetchRequest,
        cancellation: &FetchCancellation,
        sink: &mut dyn FetchSink,
    ) -> Result<(), FetchError> {
        cancellation.check()?;
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
            cancellation.check()?;
            self.policy.authorize(&url)?;
            let previous_origin = exact_origin(&url)?;
            let remaining = self
                .policy
                .timeout
                .checked_sub(started.elapsed())
                .ok_or_else(|| {
                    FetchError::new(FetchErrorKind::Timeout, "fetch exceeded its total timeout")
                })?;
            let mut response =
                self.one_request(&url, &method, &headers, &body, remaining, cancellation)?;
            cancellation.check()?;
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
            let status_text = response
                .status()
                .canonical_reason()
                .unwrap_or_default()
                .to_string();
            // Hand over the head before reading any body byte: that is the whole
            // point of streaming, and the buffered sink does not care when it
            // arrives.
            sink.head(FetchHead {
                url: url.to_string(),
                status,
                status_text,
                headers: response_headers,
                redirected: redirects > 0,
            })?;

            let mut total = 0usize;
            let mut reader = response.body_mut().as_reader();
            let mut chunk = [0_u8; 64 * 1024];
            loop {
                cancellation.check()?;
                let read = reader.read(&mut chunk).map_err(|error| {
                    FetchError::new(
                        FetchErrorKind::Network,
                        format!("fetch body read failed: {error}"),
                    )
                })?;
                if read == 0 {
                    break;
                }
                total = total.saturating_add(read);
                // The cap stays cumulative across chunks, so streaming cannot be
                // used to slip past a limit the buffered path enforces.
                if total > self.policy.max_response_bytes {
                    return Err(FetchError::new(
                        FetchErrorKind::ResponseTooLarge,
                        format!(
                            "fetch response body exceeds {} bytes",
                            self.policy.max_response_bytes
                        ),
                    ));
                }
                sink.chunk(&chunk[..read])?;
            }
            cancellation.check()?;
            return Ok(());
        }
    }
}

impl FetchHost for NativeFetchHost {
    fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.fetch_impl(request, &FetchCancellation::new())
    }

    fn fetch_cancellable(
        &self,
        request: FetchRequest,
        cancellation: FetchCancellation,
    ) -> Result<FetchResponse, FetchError> {
        self.fetch_impl(request, &cancellation)
    }

    fn fetch_streaming(
        &self,
        request: FetchRequest,
        cancellation: FetchCancellation,
        sink: &mut dyn FetchSink,
    ) -> Result<(), FetchError> {
        self.stream_impl(request, &cancellation, sink)
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
