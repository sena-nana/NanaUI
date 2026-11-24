//! Host-owned, policy-gated buffered HTTP(S) transport.

mod policy;
#[cfg(test)]
mod tests;
#[cfg(feature = "fetch")]
mod transport;
#[cfg(feature = "fetch")]
mod worker;

use std::fmt;
#[cfg(feature = "fetch")]
use std::io;
#[cfg(feature = "fetch")]
use std::net::{Shutdown, TcpStream};
use std::sync::Arc;
#[cfg(feature = "fetch")]
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub use policy::FetchPolicy;
#[cfg(feature = "fetch")]
pub use worker::NativeFetchHost;

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

/// Everything about a response except its body.
///
/// Split out so a streaming host can hand the response line and headers to the
/// consumer before any body byte has arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchHead {
    pub url: String,
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub redirected: bool,
}

impl FetchHead {
    /// Rejoin a head and a fully-read body.
    pub fn with_body(self, body: Vec<u8>) -> FetchResponse {
        FetchResponse {
            url: self.url,
            status: self.status,
            status_text: self.status_text,
            headers: self.headers,
            body,
            redirected: self.redirected,
        }
    }
}

/// Incremental receiver for a streaming fetch.
///
/// [`FetchSink::head`] is called exactly once, before any [`FetchSink::chunk`].
/// Returning an error from either aborts the transfer, so a consumer that has
/// gone away (or that enforces its own limit) stops the read rather than paying
/// for bytes nobody will take.
pub trait FetchSink {
    fn head(&mut self, head: FetchHead) -> Result<(), FetchError>;
    fn chunk(&mut self, bytes: &[u8]) -> Result<(), FetchError>;
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
    Cancelled,
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

/// Cooperative cancellation shared by the request, worker, and host
/// transport. Hosts with an interruptible backend should observe it while
/// connecting, following redirects, and reading the response body.
#[derive(Debug, Default)]
struct FetchCancellationState {
    cancelled: AtomicBool,
    #[cfg(feature = "fetch")]
    sockets: Mutex<Vec<TcpStream>>,
}

#[derive(Debug, Clone, Default)]
pub struct FetchCancellation {
    state: Arc<FetchCancellationState>,
}

impl FetchCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.state.cancelled.store(true, Ordering::Release);
        #[cfg(feature = "fetch")]
        if let Ok(sockets) = self.state.sockets.lock() {
            for socket in sockets.iter() {
                let _ = socket.shutdown(Shutdown::Both);
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    pub fn check(&self) -> Result<(), FetchError> {
        if self.is_cancelled() {
            Err(FetchError::new(
                FetchErrorKind::Cancelled,
                "fetch was cancelled",
            ))
        } else {
            Ok(())
        }
    }

    #[cfg(feature = "fetch")]
    pub(super) fn register_socket(&self, socket: &TcpStream) -> io::Result<()> {
        let registered = socket.try_clone()?;
        if self.is_cancelled() {
            let _ = registered.shutdown(Shutdown::Both);
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "fetch was cancelled",
            ));
        }
        self.state
            .sockets
            .lock()
            .map_err(|_| io::Error::other("fetch cancellation state poisoned"))?
            .push(registered);
        Ok(())
    }
}

pub trait FetchHost: Send + Sync + fmt::Debug {
    fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError>;

    fn fetch_cancellable(
        &self,
        request: FetchRequest,
        cancellation: FetchCancellation,
    ) -> Result<FetchResponse, FetchError> {
        cancellation.check()?;
        self.fetch(request)
    }

    /// Stream a response into `sink`.
    ///
    /// The default buffers through [`Self::fetch_cancellable`] and delivers the
    /// whole body as a single chunk, so an application host that only knows how
    /// to return a complete response keeps working — it simply does not stream.
    ///
    /// The response cap still applies, taken from [`Self::policy`] — the host's
    /// own declared limit, not somebody else's. Without this a host that
    /// implements only `fetch` would hand the caller an unbounded body while its own
    /// `FetchPolicy` says otherwise.
    fn fetch_streaming(
        &self,
        request: FetchRequest,
        cancellation: FetchCancellation,
        sink: &mut dyn FetchSink,
    ) -> Result<(), FetchError> {
        let response = self.fetch_cancellable(request, cancellation)?;
        let max = self.policy().max_response_bytes;
        if response.body.len() > max {
            return Err(FetchError::new(
                FetchErrorKind::ResponseTooLarge,
                format!("fetch response body exceeds {max} bytes"),
            ));
        }
        let body = response.body;
        sink.head(FetchHead {
            url: response.url,
            status: response.status,
            status_text: response.status_text,
            headers: response.headers,
            redirected: response.redirected,
        })?;
        if !body.is_empty() {
            sink.chunk(&body)?;
        }
        Ok(())
    }

    fn policy(&self) -> &FetchPolicy;
}

pub type SharedFetchHost = Arc<dyn FetchHost>;

pub fn shared_fetch_host(host: impl FetchHost + 'static) -> SharedFetchHost {
    Arc::new(host)
}
