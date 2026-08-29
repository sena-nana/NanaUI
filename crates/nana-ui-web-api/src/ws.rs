use std::collections::BTreeMap;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use nana_js_engine::{HostApiRegistry, HostValue, JsException};
use nana_ui_platform::{SharedWebSocketHost, WsEvent, WsMessage, WsOpenRequest, WsSink};

use crate::SharedWebApiState;

/// Bounded wake used by [`crate::WebApiState::next_wakeup`] while any socket
/// connection is Connecting/Open/Closing, mirroring the in-flight fetch wake.
pub(crate) const SOCKET_WAKE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(8);

/// Close code synthesized when a host transport rejects a close request and
/// can therefore never deliver its own `Closed` event.
const ABNORMAL_CLOSE_CODE: u16 = 1006;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SocketState {
    Connecting,
    Open,
    Closing,
}

#[derive(Debug)]
pub(crate) struct SocketEvent {
    pub id: u64,
    pub event: WsEvent,
}

impl SocketEvent {
    pub fn into_host_value(self) -> HostValue {
        let mut value = BTreeMap::new();
        value.insert("id".into(), HostValue::Number(self.id as f64));
        match self.event {
            WsEvent::Open => {
                value.insert("kind".into(), HostValue::String("open".into()));
            }
            WsEvent::Message(WsMessage::Text(text)) => {
                value.insert("kind".into(), HostValue::String("message".into()));
                value.insert("data".into(), HostValue::String(text));
            }
            WsEvent::Message(WsMessage::Binary(bytes)) => {
                value.insert("kind".into(), HostValue::String("message".into()));
                value.insert("bytes".into(), HostValue::Bytes(bytes));
            }
            WsEvent::Error(message) => {
                value.insert("kind".into(), HostValue::String("error".into()));
                value.insert("message".into(), HostValue::String(message));
            }
            WsEvent::Closed {
                code,
                reason,
                was_clean,
            } => {
                value.insert("kind".into(), HostValue::String("close".into()));
                value.insert("code".into(), HostValue::Number(code as f64));
                value.insert("reason".into(), HostValue::String(reason));
                value.insert("wasClean".into(), HostValue::Bool(was_clean));
            }
        }
        HostValue::Object(value)
    }
}

#[derive(Debug)]
struct SocketEventSink {
    id: u64,
    sender: Sender<SocketEvent>,
}

impl WsSink for SocketEventSink {
    fn emit(&self, event: WsEvent) {
        let _ = self.sender.send(SocketEvent { id: self.id, event });
    }
}

/// Connection bookkeeping for the reserved WebSocket surface. Transports are
/// application-owned; this runtime only assigns ids, forwards requests, and
/// queues inbound events for the per-frame drain so JS callbacks stay on the
/// engine/UI thread.
pub(crate) struct SocketRuntime {
    host: Option<SharedWebSocketHost>,
    events: Receiver<SocketEvent>,
    event_sender: Sender<SocketEvent>,
    connections: BTreeMap<u64, SocketState>,
    next_id: u64,
}

impl std::fmt::Debug for SocketRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SocketRuntime")
            .field("connections", &self.connections)
            .finish_non_exhaustive()
    }
}

impl SocketRuntime {
    pub fn new() -> Self {
        let (event_sender, events) = crossbeam_channel::unbounded();
        Self {
            host: None,
            events,
            event_sender,
            connections: BTreeMap::new(),
            next_id: 1,
        }
    }

    pub fn set_host(&mut self, host: Option<SharedWebSocketHost>) {
        self.host = host;
    }

    fn host(&self) -> Result<&SharedWebSocketHost, JsException> {
        self.host.as_ref().ok_or_else(|| {
            JsException::new(
                "WebSocket is not available: the application did not provide a socket host",
            )
        })
    }

    pub fn open(&mut self, request: WsOpenRequest) -> Result<u64, JsException> {
        let host = self.host()?.clone();
        host.policy()
            .authorize_str(&request.url)
            .map_err(|error| JsException::new(error.message))?;
        let id = self.next_id;
        self.next_id += 1;
        let sink: Arc<dyn WsSink> = Arc::new(SocketEventSink {
            id,
            sender: self.event_sender.clone(),
        });
        host.open(id, request, sink)
            .map_err(|error| JsException::new(error.message))?;
        self.connections.insert(id, SocketState::Connecting);
        Ok(id)
    }

    pub fn send(&mut self, id: u64, message: WsMessage) -> Result<(), JsException> {
        let host = self.host()?.clone();
        match self.connections.get(&id) {
            Some(SocketState::Open) => {}
            Some(_) => return Err(JsException::new("WebSocket is not open")),
            None => return Err(JsException::new("unknown WebSocket connection id")),
        }
        let limit = host.policy().max_message_bytes;
        if message.len() > limit {
            return Err(JsException::new(format!(
                "WebSocket message exceeds {} bytes",
                limit
            )));
        }
        host.send(id, message)
            .map_err(|error| JsException::new(error.message))
    }

    pub fn close(&mut self, id: u64, code: u16, reason: &str) -> Result<(), JsException> {
        let host = self.host()?.clone();
        match self.connections.get(&id) {
            Some(_) => {}
            None => return Err(JsException::new("unknown WebSocket connection id")),
        }
        self.connections.insert(id, SocketState::Closing);
        if let Err(error) = host.close(id, code, reason) {
            // The transport does not track this id, so its `Closed` event can
            // never arrive — synthesize one so `onclose` still fires once.
            self.connections.remove(&id);
            let _ = self.event_sender.send(SocketEvent {
                id,
                event: WsEvent::Closed {
                    code: ABNORMAL_CLOSE_CODE,
                    reason: error.message,
                    was_clean: false,
                },
            });
        }
        Ok(())
    }

    pub fn drain_events(&mut self) -> Vec<SocketEvent> {
        let mut due = Vec::new();
        while let Ok(item) = self.events.try_recv() {
            match &item.event {
                WsEvent::Open => {
                    self.connections.insert(item.id, SocketState::Open);
                }
                WsEvent::Closed { .. } => {
                    self.connections.remove(&item.id);
                }
                WsEvent::Message(_) | WsEvent::Error(_) => {}
            }
            due.push(item);
        }
        due
    }

    pub fn has_active(&self) -> bool {
        !self.connections.is_empty()
    }
}

pub(crate) fn register_socket_host_ops(api: &mut HostApiRegistry, state: SharedWebApiState) {
    {
        let state = Arc::clone(&state);
        api.register("wsOpen", move |args| {
            let request = parse_open_request(args.first())?;
            let mut guard = state
                .lock()
                .map_err(|_| JsException::new("web-api state poisoned"))?;
            Ok(HostValue::Number(guard.socket.open(request)? as f64))
        });
    }
    {
        let state = Arc::clone(&state);
        api.register("wsSend", move |args| {
            let (id, message) = parse_send(args.first())?;
            let mut guard = state
                .lock()
                .map_err(|_| JsException::new("web-api state poisoned"))?;
            guard.socket.send(id, message)?;
            Ok(HostValue::Null)
        });
    }
    {
        let state = Arc::clone(&state);
        api.register("wsClose", move |args| {
            let object = args
                .first()
                .and_then(HostValue::as_object)
                .ok_or_else(|| JsException::new("wsClose requires a socket object"))?;
            let id = object
                .get("id")
                .and_then(HostValue::as_f64)
                .ok_or_else(|| JsException::new("wsClose requires a socket id"))?
                as u64;
            let code = object
                .get("code")
                .and_then(HostValue::as_f64)
                .map(|code| code as u16)
                .unwrap_or(1000);
            let reason = object
                .get("reason")
                .and_then(HostValue::as_str)
                .unwrap_or_default()
                .to_string();
            let mut guard = state
                .lock()
                .map_err(|_| JsException::new("web-api state poisoned"))?;
            guard.socket.close(id, code, &reason)?;
            Ok(HostValue::Null)
        });
    }
}

fn parse_open_request(value: Option<&HostValue>) -> Result<WsOpenRequest, JsException> {
    let object = value
        .and_then(HostValue::as_object)
        .ok_or_else(|| JsException::new("wsOpen requires a socket request object"))?;
    let url = object
        .get("url")
        .and_then(HostValue::as_str)
        .ok_or_else(|| JsException::new("WebSocket URL is required"))?
        .to_string();
    let protocols = match object.get("protocols") {
        Some(HostValue::Array(entries)) => entries
            .iter()
            .map(|entry| {
                entry
                    .as_str()
                    .map(String::from)
                    .ok_or_else(|| JsException::new("WebSocket subprotocols must be strings"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
        _ => return Err(JsException::new("WebSocket subprotocols must be an array")),
    };
    Ok(WsOpenRequest { url, protocols })
}

fn parse_send(value: Option<&HostValue>) -> Result<(u64, WsMessage), JsException> {
    let object = value
        .and_then(HostValue::as_object)
        .ok_or_else(|| JsException::new("wsSend requires a socket payload object"))?;
    let id = object
        .get("id")
        .and_then(HostValue::as_f64)
        .ok_or_else(|| JsException::new("wsSend requires a socket id"))? as u64;
    let message = match object.get("kind").and_then(HostValue::as_str) {
        Some("text") => WsMessage::Text(
            object
                .get("data")
                .and_then(HostValue::as_str)
                .ok_or_else(|| JsException::new("wsSend text payload must be a string"))?
                .to_string(),
        ),
        Some("binary") => WsMessage::Binary(
            object
                .get("data")
                .and_then(HostValue::as_bytes)
                .ok_or_else(|| JsException::new("wsSend binary payload must be bytes"))?
                .to_vec(),
        ),
        _ => return Err(JsException::new("wsSend kind must be text or binary")),
    };
    Ok((id, message))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use nana_ui_platform::{SocketPolicy, WebSocketHost, WsError};

    use super::*;

    #[derive(Default)]
    struct FixtureInner {
        sinks: BTreeMap<u64, Arc<dyn WsSink>>,
        sent: Vec<(u64, WsMessage)>,
        closed: Vec<(u64, u16, String)>,
    }

    impl std::fmt::Debug for FixtureInner {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("FixtureInner")
                .field("sent", &self.sent)
                .field("closed", &self.closed)
                .finish_non_exhaustive()
        }
    }

    #[derive(Debug)]
    struct FixtureSocketHost {
        policy: SocketPolicy,
        inner: Mutex<FixtureInner>,
    }

    impl FixtureSocketHost {
        fn new(policy: SocketPolicy) -> Self {
            Self {
                policy,
                inner: Mutex::new(FixtureInner::default()),
            }
        }

        fn sink(&self, id: u64) -> Arc<dyn WsSink> {
            self.inner.lock().unwrap().sinks[&id].clone()
        }
    }

    impl WebSocketHost for FixtureSocketHost {
        fn open(
            &self,
            id: u64,
            _request: WsOpenRequest,
            sink: Arc<dyn WsSink>,
        ) -> Result<(), WsError> {
            self.inner.lock().unwrap().sinks.insert(id, sink);
            Ok(())
        }

        fn send(&self, id: u64, message: WsMessage) -> Result<(), WsError> {
            self.inner.lock().unwrap().sent.push((id, message));
            Ok(())
        }

        fn close(&self, id: u64, code: u16, reason: &str) -> Result<(), WsError> {
            self.inner
                .lock()
                .unwrap()
                .closed
                .push((id, code, reason.into()));
            Ok(())
        }

        fn policy(&self) -> &SocketPolicy {
            &self.policy
        }
    }

    fn open_args(url: &str) -> HostValue {
        HostValue::Object(BTreeMap::from([
            ("url".into(), HostValue::String(url.into())),
            (
                "protocols".into(),
                HostValue::Array(vec![HostValue::String("chat.v1".into())]),
            ),
        ]))
    }

    fn registered_state(
        host: FixtureSocketHost,
    ) -> (
        Arc<Mutex<crate::WebApiState>>,
        HostApiRegistry,
        Arc<FixtureSocketHost>,
    ) {
        let fixture = Arc::new(host);
        let state = crate::shared_web_api_state();
        state
            .lock()
            .unwrap()
            .set_socket_host(Some(Arc::clone(&fixture) as SharedWebSocketHost));
        let mut api = HostApiRegistry::new();
        crate::register_web_api_host_ops(&mut api, Arc::clone(&state));
        (state, api, fixture)
    }

    #[test]
    fn ws_open_reports_unavailable_without_a_socket_host() {
        let state = crate::shared_web_api_state();
        let mut api = HostApiRegistry::new();
        crate::register_web_api_host_ops(&mut api, state);
        let error = api
            .call("wsOpen", &[open_args("wss://example.com/chat")])
            .expect_err("no socket host must fail");
        assert!(error.message.contains("WebSocket is not available"));
    }

    #[test]
    fn ws_open_enforces_policy_before_touching_the_transport() {
        let (_state, api, _fixture) =
            registered_state(FixtureSocketHost::new(SocketPolicy::default()));
        let error = api
            .call("wsOpen", &[open_args("wss://example.com/chat")])
            .expect_err("deny-all policy must reject");
        assert!(error.message.contains("not authorized by the host"));
    }

    #[test]
    fn socket_events_reach_the_drain_in_order_and_clean_up_state() {
        let (state, api, fixture) = registered_state(FixtureSocketHost::new(
            SocketPolicy::default()
                .with_allowed_origin("wss://example.com")
                .unwrap(),
        ));
        let id = api
            .call("wsOpen", &[open_args("wss://example.com/chat")])
            .unwrap()
            .as_f64()
            .unwrap() as u64;
        {
            let guard = state.lock().unwrap();
            assert!(guard.socket.has_active());
            assert!(guard.next_wakeup(std::time::Instant::now()).is_some());
        }

        fixture.sink(id).emit(WsEvent::Open);
        fixture
            .sink(id)
            .emit(WsEvent::Message(WsMessage::Text("hello".into())));
        fixture
            .sink(id)
            .emit(WsEvent::Message(WsMessage::Binary(vec![1, 2, 255])));
        fixture.sink(id).emit(WsEvent::Closed {
            code: 1000,
            reason: "done".into(),
            was_clean: true,
        });

        let events = state.lock().unwrap().drain_socket_events();
        assert_eq!(events.len(), 4);
        assert_eq!(
            events[0].as_object().unwrap()["kind"],
            HostValue::String("open".into())
        );
        assert_eq!(
            events[1].as_object().unwrap()["data"],
            HostValue::String("hello".into())
        );
        assert_eq!(
            events[2].as_object().unwrap()["bytes"],
            HostValue::Bytes(vec![1, 2, 255])
        );
        assert_eq!(
            events[3].as_object().unwrap()["wasClean"],
            HostValue::Bool(true)
        );
        let guard = state.lock().unwrap();
        assert!(!guard.socket.has_active());
        assert!(guard.next_wakeup(std::time::Instant::now()).is_none());
    }

    #[test]
    fn ws_send_requires_an_open_connection_and_reaches_the_transport() {
        let (state, api, fixture) = registered_state(FixtureSocketHost::new(
            SocketPolicy::default()
                .with_allowed_origin("wss://example.com")
                .unwrap(),
        ));
        let id = api
            .call("wsOpen", &[open_args("wss://example.com/chat")])
            .unwrap()
            .as_f64()
            .unwrap() as u64;

        let payload = HostValue::Object(BTreeMap::from([
            ("id".into(), HostValue::Number(id as f64)),
            ("kind".into(), HostValue::String("text".into())),
            ("data".into(), HostValue::String("ping".into())),
        ]));
        let error = api
            .call("wsSend", &[payload.clone()])
            .expect_err("send while connecting must fail");
        assert!(error.message.contains("not open"));

        fixture.sink(id).emit(WsEvent::Open);
        state.lock().unwrap().drain_socket_events();
        api.call("wsSend", &[payload]).unwrap();
        assert_eq!(
            fixture.inner.lock().unwrap().sent.clone(),
            vec![(id, WsMessage::Text("ping".into()))]
        );
    }

    #[test]
    fn ws_close_marks_closing_and_unknown_ids_fail() {
        let (state, api, _fixture) = registered_state(FixtureSocketHost::new(
            SocketPolicy::default()
                .with_allowed_origin("wss://example.com")
                .unwrap(),
        ));
        let id = api
            .call("wsOpen", &[open_args("wss://example.com/chat")])
            .unwrap()
            .as_f64()
            .unwrap() as u64;
        api.call(
            "wsClose",
            &[HostValue::Object(BTreeMap::from([
                ("id".into(), HostValue::Number(id as f64)),
                ("code".into(), HostValue::Number(1000.0)),
                ("reason".into(), HostValue::String("bye".into())),
            ]))],
        )
        .unwrap();
        assert!(state.lock().unwrap().socket.has_active());

        let error = api
            .call(
                "wsSend",
                &[HostValue::Object(BTreeMap::from([
                    ("id".into(), HostValue::Number(id as f64)),
                    ("kind".into(), HostValue::String("text".into())),
                    ("data".into(), HostValue::String("x".into())),
                ]))],
            )
            .expect_err("send while closing must fail");
        assert!(error.message.contains("not open"));

        let error = api
            .call(
                "wsClose",
                &[HostValue::Object(BTreeMap::from([(
                    "id".into(),
                    HostValue::Number(9999.0),
                )]))],
            )
            .expect_err("unknown id must fail");
        assert!(error.message.contains("unknown WebSocket connection id"));
    }

    #[test]
    fn ws_send_enforces_the_message_limit() {
        let mut policy = SocketPolicy::default();
        policy.allow_origin("wss://example.com").unwrap();
        policy.max_message_bytes = 4;
        let (state, api, fixture) = registered_state(FixtureSocketHost::new(policy));
        let id = api
            .call("wsOpen", &[open_args("wss://example.com/chat")])
            .unwrap()
            .as_f64()
            .unwrap() as u64;
        fixture.sink(id).emit(WsEvent::Open);
        state.lock().unwrap().drain_socket_events();
        let error = api
            .call(
                "wsSend",
                &[HostValue::Object(BTreeMap::from([
                    ("id".into(), HostValue::Number(id as f64)),
                    ("kind".into(), HostValue::String("text".into())),
                    ("data".into(), HostValue::String("too long".into())),
                ]))],
            )
            .expect_err("oversized message must fail");
        assert!(error.message.contains("exceeds 4 bytes"));
    }
}
