use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender, TrySendError};
use nana_js_engine::{HostApiRegistry, HostValue, JsException};
use nana_ui_platform::{
    FetchCancellation, FetchError, FetchHead, FetchRequest, FetchSink, SharedFetchHost,
};

use crate::SharedWebApiState;

#[derive(Debug)]
struct FetchJob {
    id: u64,
    request: FetchRequest,
    cancellation: FetchCancellation,
}

/// One step of a response's delivery.
///
/// `Head` arrives once and resolves the `fetch()` promise, then zero or more
/// `Chunk`s, then exactly one `End`. A transport failure before the head is an
/// `End` with no preceding `Head`.
#[derive(Debug)]
pub(crate) enum FetchEvent {
    Head { id: u64, head: FetchHead },
    Chunk { id: u64, bytes: Vec<u8> },
    End { id: u64, error: Option<FetchError> },
}

impl FetchEvent {
    pub fn id(&self) -> u64 {
        match self {
            Self::Head { id, .. } | Self::Chunk { id, .. } | Self::End { id, .. } => *id,
        }
    }

    pub fn is_end(&self) -> bool {
        matches!(self, Self::End { .. })
    }

    pub fn into_host_value(self) -> HostValue {
        let mut value = BTreeMap::new();
        value.insert("id".into(), HostValue::Number(self.id() as f64));
        match self {
            Self::Head { head, .. } => {
                value.insert("kind".into(), HostValue::String("head".into()));
                value.insert("url".into(), HostValue::String(head.url));
                value.insert("status".into(), HostValue::Number(head.status as f64));
                value.insert("statusText".into(), HostValue::String(head.status_text));
                value.insert(
                    "headers".into(),
                    HostValue::Array(
                        head.headers
                            .into_iter()
                            .map(|(name, value)| {
                                HostValue::Array(vec![
                                    HostValue::String(name),
                                    HostValue::String(value),
                                ])
                            })
                            .collect(),
                    ),
                );
                value.insert("redirected".into(), HostValue::Bool(head.redirected));
            }
            Self::Chunk { bytes, .. } => {
                value.insert("kind".into(), HostValue::String("chunk".into()));
                value.insert("bytes".into(), HostValue::Bytes(bytes));
            }
            Self::End { error, .. } => {
                value.insert("kind".into(), HostValue::String("end".into()));
                match error {
                    None => {
                        value.insert("ok".into(), HostValue::Bool(true));
                    }
                    Some(error) => {
                        value.insert("ok".into(), HostValue::Bool(false));
                        value.insert(
                            "error".into(),
                            HostValue::Object(
                                [
                                    (
                                        "kind".into(),
                                        HostValue::String(format!("{:?}", error.kind)),
                                    ),
                                    ("message".into(), HostValue::String(error.message)),
                                ]
                                .into_iter()
                                .collect(),
                            ),
                        );
                    }
                }
            }
        }
        HostValue::Object(value)
    }
}

/// Forwards a streaming response onto the completion channel.
///
/// A send failure means the engine side is gone; reporting it as an error stops
/// the worker reading a body nobody will receive.
struct ChannelSink<'a> {
    id: u64,
    events: &'a Sender<FetchEvent>,
}

impl FetchSink for ChannelSink<'_> {
    fn head(&mut self, head: FetchHead) -> Result<(), FetchError> {
        self.send(FetchEvent::Head { id: self.id, head })
    }

    fn chunk(&mut self, bytes: &[u8]) -> Result<(), FetchError> {
        self.send(FetchEvent::Chunk {
            id: self.id,
            bytes: bytes.to_vec(),
        })
    }
}

impl ChannelSink<'_> {
    fn send(&self, event: FetchEvent) -> Result<(), FetchError> {
        self.events.send(event).map_err(|_| {
            FetchError::new(
                nana_ui_platform::FetchErrorKind::Cancelled,
                "fetch consumer went away",
            )
        })
    }
}

/// Bounded blocking worker pool. Only [`Self::drain_completions`] exposes
/// results, so JS callbacks remain on the engine/UI thread.
pub(crate) struct FetchRuntime {
    jobs: Sender<FetchJob>,
    completions: Receiver<FetchEvent>,
    cancelled: BTreeSet<u64>,
    cancellations: BTreeMap<u64, FetchCancellation>,
    next_id: u64,
    active: usize,
}

impl std::fmt::Debug for FetchRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FetchRuntime")
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl FetchRuntime {
    pub fn new(host: SharedFetchHost) -> Self {
        let worker_count = host.policy().worker_count.max(1);
        let (jobs_tx, jobs_rx) = crossbeam_channel::bounded(worker_count * 2);
        let (completion_tx, completion_rx) = crossbeam_channel::unbounded();
        for index in 0..worker_count {
            let jobs = jobs_rx.clone();
            let completions = completion_tx.clone();
            let host = Arc::clone(&host);
            std::thread::Builder::new()
                .name(format!("nana-fetch-{index}"))
                .spawn(move || fetch_worker(host, jobs, completions))
                .expect("spawn Nana fetch worker");
        }
        Self {
            jobs: jobs_tx,
            completions: completion_rx,
            cancelled: BTreeSet::new(),
            cancellations: BTreeMap::new(),
            next_id: 1,
            active: 0,
        }
    }

    pub fn start(&mut self, request: FetchRequest) -> Result<u64, JsException> {
        let id = self.next_id;
        self.next_id += 1;
        let cancellation = FetchCancellation::new();
        match self.jobs.try_send(FetchJob {
            id,
            request,
            cancellation: cancellation.clone(),
        }) {
            Ok(()) => {
                self.cancellations.insert(id, cancellation);
                self.active += 1;
                Ok(id)
            }
            Err(TrySendError::Full(_)) => Err(JsException::new(
                "fetch worker queue is full; retry after pending requests complete",
            )),
            Err(TrySendError::Disconnected(_)) => {
                Err(JsException::new("fetch worker pool is unavailable"))
            }
        }
    }

    pub fn cancel(&mut self, id: u64) {
        if let Some(cancellation) = self.cancellations.get(&id) {
            cancellation.cancel();
        }
        self.cancelled.insert(id);
    }

    pub fn drain_completions(&mut self) -> Vec<FetchEvent> {
        let mut due = Vec::new();
        while let Ok(event) = self.completions.try_recv() {
            let id = event.id();
            // Only the terminating event retires the request; head and chunk
            // events for the same id arrive before it.
            if event.is_end() {
                self.active -= 1;
                self.cancellations.remove(&id);
            }
            // A cancelled request keeps draining so its worker can finish, but
            // nothing reaches JS: its promise was already rejected.
            if self.cancelled.contains(&id) {
                if event.is_end() {
                    self.cancelled.remove(&id);
                }
                continue;
            }
            due.push(event);
        }
        due
    }

    /// Cancel every in-flight request.
    ///
    /// Their completions are still drained later and discarded, the same way a
    /// single [`Self::cancel`] behaves. Used when the JS that issued them is
    /// being replaced and nothing is left to resolve their promises.
    pub fn cancel_all(&mut self) {
        let ids: Vec<u64> = self.cancellations.keys().copied().collect();
        for id in ids {
            self.cancel(id);
        }
    }

    pub fn has_pending(&self) -> bool {
        self.active > 0
    }
}

fn fetch_worker(host: SharedFetchHost, jobs: Receiver<FetchJob>, completions: Sender<FetchEvent>) {
    while let Ok(job) = jobs.recv() {
        let mut sink = ChannelSink {
            id: job.id,
            events: &completions,
        };
        let result = host.fetch_streaming(job.request, job.cancellation, &mut sink);
        if completions
            .send(FetchEvent::End {
                id: job.id,
                error: result.err(),
            })
            .is_err()
        {
            break;
        }
    }
}

pub(crate) fn register_fetch_host_ops(api: &mut HostApiRegistry, state: SharedWebApiState) {
    {
        let state = Arc::clone(&state);
        api.register("fetchStart", move |args| {
            let request = parse_request(args.first())?;
            let mut guard = state
                .lock()
                .map_err(|_| JsException::new("web-api state poisoned"))?;
            Ok(HostValue::Number(guard.fetch.start(request)? as f64))
        });
    }
    api.register("fetchCancel", move |args| {
        let id = args
            .first()
            .and_then(HostValue::as_f64)
            .ok_or_else(|| JsException::new("fetchCancel requires a request id"))?
            as u64;
        let mut guard = state
            .lock()
            .map_err(|_| JsException::new("web-api state poisoned"))?;
        guard.fetch.cancel(id);
        Ok(HostValue::Null)
    });
}

fn parse_request(value: Option<&HostValue>) -> Result<FetchRequest, JsException> {
    let object = value
        .and_then(HostValue::as_object)
        .ok_or_else(|| JsException::new("fetchStart requires a request object"))?;
    let url = object
        .get("url")
        .and_then(HostValue::as_str)
        .ok_or_else(|| JsException::new("fetch request URL is required"))?
        .to_string();
    let method = object
        .get("method")
        .and_then(HostValue::as_str)
        .unwrap_or("GET")
        .to_string();
    let headers = match object.get("headers") {
        Some(HostValue::Array(entries)) => entries
            .iter()
            .map(|entry| match entry {
                HostValue::Array(pair) if pair.len() == 2 => {
                    let name = pair[0]
                        .as_str()
                        .ok_or_else(|| JsException::new("fetch header name must be a string"))?;
                    let value = pair[1]
                        .as_str()
                        .ok_or_else(|| JsException::new("fetch header value must be a string"))?;
                    Ok((name.to_string(), value.to_string()))
                }
                _ => Err(JsException::new(
                    "fetch headers must contain [name, value] pairs",
                )),
            })
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
        _ => return Err(JsException::new("fetch headers must be an array")),
    };
    let body = match object.get("body") {
        Some(HostValue::Bytes(bytes)) => bytes.clone(),
        Some(HostValue::Array(bytes)) => bytes
            .iter()
            .map(|value| {
                let byte = value
                    .as_f64()
                    .ok_or_else(|| JsException::new("fetch body must contain bytes"))?;
                if !(0.0..=255.0).contains(&byte) || byte.fract() != 0.0 {
                    return Err(JsException::new("fetch body contains an invalid byte"));
                }
                Ok(byte as u8)
            })
            .collect::<Result<Vec<_>, _>>()?,
        None | Some(HostValue::Null) => Vec::new(),
        _ => return Err(JsException::new("fetch body must be a byte array")),
    };
    Ok(FetchRequest {
        url,
        method,
        headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use nana_ui_platform::{
        FetchCancellation, FetchError, FetchErrorKind, FetchHost, FetchPolicy, FetchResponse,
        shared_fetch_host,
    };

    use super::*;

    #[test]
    fn fetch_bridge_preserves_binary_request_and_response_bodies() {
        let request = parse_request(Some(&HostValue::Object(BTreeMap::from([
            (
                "url".into(),
                HostValue::String("https://example.test/upload".into()),
            ),
            ("method".into(), HostValue::String("POST".into())),
            ("headers".into(), HostValue::Array(Vec::new())),
            ("body".into(), HostValue::Bytes(vec![0, 1, 127, 255])),
        ]))))
        .unwrap();
        assert_eq!(request.body, vec![0, 1, 127, 255]);

        let value = FetchEvent::Chunk {
            id: 1,
            bytes: vec![255, 0, 128],
        }
        .into_host_value();
        let object = value.as_object().unwrap();
        assert_eq!(
            object.get("kind").and_then(HostValue::as_str),
            Some("chunk")
        );
        assert_eq!(
            object.get("bytes"),
            Some(&HostValue::Bytes(vec![255, 0, 128])),
            "body bytes stay on the binary channel, not base64"
        );
    }

    #[derive(Debug)]
    struct BlockingHost {
        policy: FetchPolicy,
        released: Arc<AtomicBool>,
    }

    impl FetchHost for BlockingHost {
        fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
            while !self.released.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            Ok(FetchResponse {
                url: request.url,
                status: 200,
                status_text: "OK".into(),
                headers: Vec::new(),
                body: b"done".to_vec(),
                redirected: false,
            })
        }

        fn policy(&self) -> &FetchPolicy {
            &self.policy
        }

        fn fetch_cancellable(
            &self,
            request: FetchRequest,
            cancellation: FetchCancellation,
        ) -> Result<FetchResponse, FetchError> {
            while !self.released.load(Ordering::Acquire) {
                if cancellation.is_cancelled() {
                    return Err(FetchError::new(
                        FetchErrorKind::Cancelled,
                        "fixture fetch cancelled",
                    ));
                }
                std::thread::yield_now();
            }
            self.fetch(request)
        }
    }

    #[test]
    fn blocking_fetch_never_blocks_starting_thread() {
        let released = Arc::new(AtomicBool::new(false));
        let mut runtime = FetchRuntime::new(shared_fetch_host(BlockingHost {
            policy: FetchPolicy::default(),
            released: Arc::clone(&released),
        }));
        let start = Instant::now();
        let id = runtime
            .start(FetchRequest::get("https://example.test"))
            .unwrap();
        assert!(start.elapsed() < Duration::from_millis(100));
        assert!(runtime.drain_completions().is_empty());

        released.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(1);
        // A buffered host still arrives as head -> chunk -> end, because the
        // default `fetch_streaming` delivers its whole body as one chunk.
        let mut kinds = Vec::new();
        let mut body = Vec::new();
        loop {
            for event in runtime.drain_completions() {
                assert_eq!(event.id(), id);
                match &event {
                    FetchEvent::Head { head, .. } => {
                        assert_eq!(head.status, 200);
                        kinds.push("head");
                    }
                    FetchEvent::Chunk { bytes, .. } => {
                        body.extend_from_slice(bytes);
                        kinds.push("chunk");
                    }
                    FetchEvent::End { error, .. } => {
                        assert!(error.is_none());
                        kinds.push("end");
                    }
                }
            }
            if kinds.last() == Some(&"end") {
                break;
            }
            assert!(Instant::now() < deadline, "fetch worker did not complete");
            std::thread::yield_now();
        }
        assert_eq!(kinds, vec!["head", "chunk", "end"]);
        assert_eq!(body, b"done");
    }

    #[test]
    fn cancelled_fetch_completion_is_not_delivered() {
        let released = Arc::new(AtomicBool::new(false));
        let mut runtime = FetchRuntime::new(shared_fetch_host(BlockingHost {
            policy: FetchPolicy::default(),
            released: Arc::clone(&released),
        }));
        let id = runtime
            .start(FetchRequest::get("https://example.test"))
            .unwrap();
        runtime.cancel(id);
        let deadline = Instant::now() + Duration::from_secs(1);
        while runtime.has_pending() {
            assert!(runtime.drain_completions().is_empty());
            assert!(
                Instant::now() < deadline,
                "cancelled fetch transport did not stop"
            );
            std::thread::yield_now();
        }
        assert!(runtime.drain_completions().is_empty());
    }
}
