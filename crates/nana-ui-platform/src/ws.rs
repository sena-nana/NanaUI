//! Host-owned, policy-gated WebSocket transport boundary.
//!
//! The framework reserves the interface: origin policy, host ops, and the JS
//! `WebSocket` shim. The transport itself is application-owned — NanaUI ships
//! no default implementation, and without a host-injected [`WebSocketHost`]
//! the JS surface reports itself unavailable.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use url::Url;

const DEFAULT_SOCKET_MESSAGE_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsOpenRequest {
    pub url: String,
    pub protocols: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
}

impl WsMessage {
    pub fn len(&self) -> usize {
        match self {
            WsMessage::Text(text) => text.len(),
            WsMessage::Binary(bytes) => bytes.len(),
        }
    }
}

/// Inbound connection event pushed by the host transport into [`WsSink`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsEvent {
    Open,
    Message(WsMessage),
    Error(String),
    Closed {
        code: u16,
        reason: String,
        was_clean: bool,
    },
}

/// Framework-owned receiving side of one connection. The framework binds a
/// sink to a connection id before [`WebSocketHost::open`] runs; transports
/// only push events, delivery to JS happens on the next host frame pump.
pub trait WsSink: Send + Sync {
    fn emit(&self, event: WsEvent);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsErrorKind {
    Policy,
    InvalidRequest,
    Network,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsError {
    pub kind: WsErrorKind,
    pub message: String,
}

impl WsError {
    pub fn new(kind: WsErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for WsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for WsError {}

/// Security and resource limits applied before a connection opens.
///
/// Origins use URL origin serialization (`scheme://host[:port]`) and are
/// matched exactly. The default policy authorizes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketPolicy {
    allowed_origins: BTreeSet<String>,
    pub max_message_bytes: usize,
}

impl Default for SocketPolicy {
    fn default() -> Self {
        Self {
            allowed_origins: BTreeSet::new(),
            max_message_bytes: DEFAULT_SOCKET_MESSAGE_LIMIT,
        }
    }
}

impl SocketPolicy {
    pub fn allow_origin(&mut self, origin: &str) -> Result<&mut Self, WsError> {
        let url = Url::parse(origin).map_err(|error| {
            WsError::new(
                WsErrorKind::InvalidRequest,
                format!("invalid socket origin `{origin}`: {error}"),
            )
        })?;
        if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
            return Err(WsError::new(
                WsErrorKind::InvalidRequest,
                format!("socket policy requires an origin, not a URL path: `{origin}`"),
            ));
        }
        let serialized = exact_socket_origin(&url)?;
        self.allowed_origins.insert(serialized);
        Ok(self)
    }

    pub fn with_allowed_origin(mut self, origin: &str) -> Result<Self, WsError> {
        self.allow_origin(origin)?;
        Ok(self)
    }

    pub fn allowed_origins(&self) -> impl Iterator<Item = &str> {
        self.allowed_origins.iter().map(String::as_str)
    }

    pub fn authorize(&self, url: &Url) -> Result<(), WsError> {
        let origin = exact_socket_origin(url)?;
        if self.allowed_origins.contains(&origin) {
            Ok(())
        } else {
            Err(WsError::new(
                WsErrorKind::Policy,
                format!("socket origin `{origin}` is not authorized by the host"),
            ))
        }
    }

    /// Parse-then-authorize convenience for callers that hold a raw URL.
    pub fn authorize_str(&self, url: &str) -> Result<(), WsError> {
        let parsed = Url::parse(url).map_err(|error| {
            WsError::new(
                WsErrorKind::InvalidRequest,
                format!("invalid socket URL `{url}`: {error}"),
            )
        })?;
        self.authorize(&parsed)
    }
}

fn exact_socket_origin(url: &Url) -> Result<String, WsError> {
    if !matches!(url.scheme(), "ws" | "wss") || url.host_str().is_none() {
        return Err(WsError::new(
            WsErrorKind::Policy,
            format!("sockets only support WS(S) origins: `{url}`"),
        ));
    }
    Ok(url.origin().ascii_serialization())
}

/// Application-owned WebSocket transport.
///
/// The framework assigns connection ids and supplies a per-connection sink.
/// [`open`](Self::open) must not block the calling (UI/JS) thread: transports
/// spawn their own I/O and report progress through the sink. Implementations
/// enforce [`policy`](Self::policy) limits on inbound traffic and emit
/// [`WsEvent::Closed`] exactly once per connection.
pub trait WebSocketHost: Send + Sync + fmt::Debug {
    fn open(&self, id: u64, request: WsOpenRequest, sink: Arc<dyn WsSink>) -> Result<(), WsError>;

    fn send(&self, id: u64, message: WsMessage) -> Result<(), WsError>;

    fn close(&self, id: u64, code: u16, reason: &str) -> Result<(), WsError>;

    fn policy(&self) -> &SocketPolicy;
}

pub type SharedWebSocketHost = Arc<dyn WebSocketHost>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_denies_every_origin() {
        let policy = SocketPolicy::default();
        let error = policy
            .authorize(&Url::parse("wss://example.com/chat").unwrap())
            .unwrap_err();
        assert_eq!(error.kind, WsErrorKind::Policy);
    }

    #[test]
    fn policy_matches_normalized_origin_not_path() {
        let policy = SocketPolicy::default()
            .with_allowed_origin("wss://example.com:443")
            .unwrap();
        policy
            .authorize(&Url::parse("wss://example.com/chat?room=1").unwrap())
            .unwrap();
        assert!(
            policy
                .authorize(&Url::parse("wss://api.example.com/chat").unwrap())
                .is_err()
        );
    }

    #[test]
    fn policy_rejects_non_socket_schemes() {
        let mut policy = SocketPolicy::default();
        assert_eq!(
            policy.allow_origin("https://example.com").unwrap_err().kind,
            WsErrorKind::Policy
        );
        let policy = policy.with_allowed_origin("wss://example.com").unwrap();
        assert_eq!(
            policy
                .authorize_str("https://example.com/chat")
                .unwrap_err()
                .kind,
            WsErrorKind::Policy
        );
    }
}
