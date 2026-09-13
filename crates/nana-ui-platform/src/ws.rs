//! Host-owned, policy-gated WebSocket transport boundary.
//!
//! The framework reserves the interface: origin policy, host ops, and the JS
//! `WebSocket` shim. Desktop builds include [`NativeWebSocketHost`]; applications
//! may replace it with another [`WebSocketHost`] implementation.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

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
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

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

pub fn shared_websocket_host(policy: SocketPolicy) -> SharedWebSocketHost {
    Arc::new(NativeWebSocketHost::new(policy))
}

#[derive(Debug)]
enum SocketCommand {
    Send(tungstenite::Message),
    Close(u16, String),
}

#[derive(Debug)]
pub struct NativeWebSocketHost {
    policy: SocketPolicy,
    connections: Arc<Mutex<BTreeMap<u64, Sender<SocketCommand>>>>,
}
fn set_socket_timeout(stream: &mut tungstenite::stream::MaybeTlsStream<std::net::TcpStream>) {
    match stream {
        tungstenite::stream::MaybeTlsStream::Plain(s) => {
            let _ = s.set_read_timeout(Some(std::time::Duration::from_millis(50)));
        }
        tungstenite::stream::MaybeTlsStream::Rustls(s) => {
            let _ = s
                .sock
                .set_read_timeout(Some(std::time::Duration::from_millis(50)));
        }
        _ => {}
    }
}
impl NativeWebSocketHost {
    pub fn new(policy: SocketPolicy) -> Self {
        Self {
            policy,
            connections: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
}
impl WebSocketHost for NativeWebSocketHost {
    fn open(&self, id: u64, request: WsOpenRequest, sink: Arc<dyn WsSink>) -> Result<(), WsError> {
        let url = Url::parse(&request.url)
            .map_err(|e| WsError::new(WsErrorKind::InvalidRequest, e.to_string()))?;
        self.policy.authorize(&url)?;
        use tungstenite::client::IntoClientRequest;
        let mut handshake = request
            .url
            .clone()
            .into_client_request()
            .map_err(|e| WsError::new(WsErrorKind::InvalidRequest, e.to_string()))?;
        if !request.protocols.is_empty() {
            let value = request.protocols.join(", ");
            let header = tungstenite::http::HeaderValue::from_str(&value)
                .map_err(|e| WsError::new(WsErrorKind::InvalidRequest, e.to_string()))?;
            handshake
                .headers_mut()
                .insert("Sec-WebSocket-Protocol", header);
        }
        let (tx, rx): (Sender<SocketCommand>, Receiver<SocketCommand>) = std::sync::mpsc::channel();
        let mut registry = self.connections.lock().unwrap();
        if registry.contains_key(&id) {
            return Err(WsError::new(
                WsErrorKind::InvalidRequest,
                "WebSocket connection id is already in use",
            ));
        }
        registry.insert(id, tx);
        drop(registry);
        let connections = Arc::clone(&self.connections);
        let max_message_bytes = self.policy.max_message_bytes;
        std::thread::spawn(move || {
            // Do not follow HTTP redirects here: the policy was evaluated for
            // the requested origin and must not silently expand to a new one.
            let result = tungstenite::client::connect_with_config(handshake, None, 0);
            match result {
                Ok((mut socket, _)) => {
                    set_socket_timeout(socket.get_mut());
                    sink.emit(WsEvent::Open);
                    'connection: loop {
                        while let Ok(command) = rx.try_recv() {
                            match command {
                                SocketCommand::Send(message) => {
                                    if let Err(error) = socket.send(message) {
                                        let reason = error.to_string();
                                        sink.emit(WsEvent::Error(reason.clone()));
                                        sink.emit(WsEvent::Closed {
                                            code: 1006,
                                            reason,
                                            was_clean: false,
                                        });
                                        break 'connection;
                                    }
                                }
                                SocketCommand::Close(code, reason) => {
                                    let frame = tungstenite::protocol::CloseFrame {
                                        code: tungstenite::protocol::frame::coding::CloseCode::from(
                                            code,
                                        ),
                                        reason: reason.clone().into(),
                                    };
                                    match socket.close(Some(frame)) {
                                        Ok(()) => sink.emit(WsEvent::Closed {
                                            code,
                                            reason,
                                            was_clean: true,
                                        }),
                                        Err(error) => {
                                            let failure = error.to_string();
                                            sink.emit(WsEvent::Error(failure.clone()));
                                            sink.emit(WsEvent::Closed {
                                                code: 1006,
                                                reason: failure,
                                                was_clean: false,
                                            });
                                        }
                                    }
                                    break 'connection;
                                }
                            }
                        }
                        match socket.read() {
                            Ok(tungstenite::Message::Text(value))
                                if value.len() <= max_message_bytes =>
                            {
                                sink.emit(WsEvent::Message(WsMessage::Text(value.to_string())))
                            }
                            Ok(tungstenite::Message::Binary(value))
                                if value.len() <= max_message_bytes =>
                            {
                                sink.emit(WsEvent::Message(WsMessage::Binary(value.to_vec())))
                            }
                            Ok(tungstenite::Message::Text(_))
                            | Ok(tungstenite::Message::Binary(_)) => {
                                let reason =
                                    format!("WebSocket message exceeds {max_message_bytes} bytes");
                                let close = tungstenite::protocol::CloseFrame {
                                    code: tungstenite::protocol::frame::coding::CloseCode::Size,
                                    reason: reason.clone().into(),
                                };
                                let _ = socket.close(Some(close));
                                sink.emit(WsEvent::Error(reason.clone()));
                                sink.emit(WsEvent::Closed {
                                    code: 1009,
                                    reason,
                                    was_clean: false,
                                });
                                break 'connection;
                            }
                            Ok(tungstenite::Message::Close(frame)) => {
                                let (code, reason) = frame
                                    .map(|f| (u16::from(f.code), f.reason.to_string()))
                                    .unwrap_or((1000, String::new()));
                                sink.emit(WsEvent::Closed {
                                    code,
                                    reason,
                                    was_clean: true,
                                });
                                break 'connection;
                            }
                            Ok(_) => {}
                            Err(tungstenite::Error::ConnectionClosed) => {
                                sink.emit(WsEvent::Closed {
                                    code: 1000,
                                    reason: String::new(),
                                    was_clean: true,
                                });
                                break 'connection;
                            }
                            Err(tungstenite::Error::Io(ref io))
                                if io.kind() == std::io::ErrorKind::WouldBlock
                                    || io.kind() == std::io::ErrorKind::TimedOut => {}
                            Err(error) => {
                                sink.emit(WsEvent::Error(error.to_string()));
                                sink.emit(WsEvent::Closed {
                                    code: 1006,
                                    reason: error.to_string(),
                                    was_clean: false,
                                });
                                break 'connection;
                            }
                        }
                    }
                }
                Err(e) => {
                    sink.emit(WsEvent::Error(e.to_string()));
                    sink.emit(WsEvent::Closed {
                        code: 1006,
                        reason: e.to_string(),
                        was_clean: false,
                    });
                }
            }
            connections.lock().unwrap().remove(&id);
        });
        let _ = id;
        Ok(())
    }
    fn send(&self, id: u64, message: WsMessage) -> Result<(), WsError> {
        if message.len() > self.policy.max_message_bytes {
            return Err(WsError::new(
                WsErrorKind::InvalidRequest,
                format!(
                    "WebSocket message exceeds {} bytes",
                    self.policy.max_message_bytes
                ),
            ));
        }
        let m = match message {
            WsMessage::Text(v) => tungstenite::Message::Text(v.into()),
            WsMessage::Binary(v) => tungstenite::Message::Binary(v.into()),
        };
        self.connections
            .lock()
            .unwrap()
            .get(&id)
            .ok_or_else(|| WsError::new(WsErrorKind::Network, "unknown connection"))?
            .send(SocketCommand::Send(m))
            .map_err(|_| WsError::new(WsErrorKind::Network, "connection closed"))
    }
    fn close(&self, id: u64, code: u16, reason: &str) -> Result<(), WsError> {
        let tx = self
            .connections
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or_else(|| WsError::new(WsErrorKind::Network, "unknown connection"))?;
        tx.send(SocketCommand::Close(code, reason.to_string()))
            .map_err(|_| WsError::new(WsErrorKind::Network, "connection closed"))
    }
    fn policy(&self) -> &SocketPolicy {
        &self.policy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

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

    #[test]
    fn native_host_round_trips_text_and_closes() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let message = socket.read().unwrap();
            socket.send(message).unwrap();
            let _ = socket.close(None);
        });
        let policy = SocketPolicy::default()
            .with_allowed_origin(&format!("ws://127.0.0.1:{port}"))
            .unwrap();
        let host = NativeWebSocketHost::new(policy);
        let (sender, receiver) = mpsc::channel();
        #[derive(Debug)]
        struct Sink(std::sync::mpsc::Sender<WsEvent>);
        impl WsSink for Sink {
            fn emit(&self, event: WsEvent) {
                let _ = self.0.send(event);
            }
        }
        host.open(
            1,
            WsOpenRequest {
                url: format!("ws://127.0.0.1:{port}/echo"),
                protocols: vec![],
            },
            Arc::new(Sink(sender)),
        )
        .unwrap();
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            WsEvent::Open
        );
        host.send(1, WsMessage::Text("hello".into())).unwrap();
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            WsEvent::Message(WsMessage::Text("hello".into()))
        );
        host.close(1, 1000, "done").unwrap();
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            WsEvent::Closed { code: 1000, .. }
        ));
        server.join().unwrap();
    }

    #[test]
    fn native_host_rejects_oversized_outbound_messages_before_network_io() {
        let mut policy = SocketPolicy::default();
        policy.allow_origin("ws://127.0.0.1").unwrap();
        policy.max_message_bytes = 3;
        let host = NativeWebSocketHost::new(policy);
        let error = host
            .send(42, WsMessage::Text("toolong".into()))
            .unwrap_err();
        assert_eq!(error.kind, WsErrorKind::InvalidRequest);
        assert!(error.message.contains("exceeds 3 bytes"));
    }
}
