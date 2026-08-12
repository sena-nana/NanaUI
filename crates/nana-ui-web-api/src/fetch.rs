use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use base64::Engine as _;
use crossbeam_channel::{Receiver, Sender, TrySendError};
use nana_js_engine::{HostApiRegistry, HostValue, JsException};
use nana_ui_platform::{FetchError, FetchRequest, FetchResponse, SharedFetchHost};

use crate::SharedWebApiState;

#[derive(Debug)]
struct FetchJob {
    id: u64,
    request: FetchRequest,
}

#[derive(Debug)]
pub(crate) struct FetchCompletion {
    pub id: u64,
    pub result: Result<FetchResponse, FetchError>,
}

impl FetchCompletion {
    pub fn into_host_value(self) -> HostValue {
        let mut value = BTreeMap::new();
        value.insert("id".into(), HostValue::Number(self.id as f64));
        match self.result {
            Ok(response) => {
                value.insert("ok".into(), HostValue::Bool(true));
                value.insert("response".into(), response_to_host_value(response));
            }
            Err(error) => {
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
        HostValue::Object(value)
    }
}

fn response_to_host_value(response: FetchResponse) -> HostValue {
    HostValue::Object(
        [
            ("url".into(), HostValue::String(response.url)),
            ("status".into(), HostValue::Number(response.status as f64)),
            ("statusText".into(), HostValue::String(response.status_text)),
            (
                "headers".into(),
                HostValue::Array(
                    response
                        .headers
                        .into_iter()
                        .map(|(name, value)| {
                            HostValue::Array(vec![
                                HostValue::String(name),
                                HostValue::String(value),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "bodyBase64".into(),
                HostValue::String(base64::engine::general_purpose::STANDARD.encode(response.body)),
            ),
            ("redirected".into(), HostValue::Bool(response.redirected)),
        ]
        .into_iter()
        .collect(),
    )
}

/// Bounded blocking worker pool. Only [`Self::drain_completions`] exposes
/// results, so JS callbacks remain on the engine/UI thread.
pub(crate) struct FetchRuntime {
    jobs: Sender<FetchJob>,
    completions: Receiver<FetchCompletion>,
    cancelled: BTreeSet<u64>,
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
            next_id: 1,
            active: 0,
        }
    }

    pub fn start(&mut self, request: FetchRequest) -> Result<u64, JsException> {
        let id = self.next_id;
        self.next_id += 1;
        match self.jobs.try_send(FetchJob { id, request }) {
            Ok(()) => {
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
        self.cancelled.insert(id);
    }

    pub fn drain_completions(&mut self) -> Vec<FetchCompletion> {
        let mut due = Vec::new();
        while let Ok(completion) = self.completions.try_recv() {
            self.active -= 1;
            let cancelled = self.cancelled.remove(&completion.id);
            if !cancelled {
                due.push(completion);
            }
        }
        due
    }

    pub fn has_pending(&self) -> bool {
        self.active > 0
    }
}

fn fetch_worker(
    host: SharedFetchHost,
    jobs: Receiver<FetchJob>,
    completions: Sender<FetchCompletion>,
) {
    while let Ok(job) = jobs.recv() {
        let result = host.fetch(job.request);
        if completions
            .send(FetchCompletion { id: job.id, result })
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

    use nana_ui_platform::{FetchError, FetchHost, FetchPolicy, shared_fetch_host};

    use super::*;

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
        loop {
            let completions = runtime.drain_completions();
            if let Some(completion) = completions.into_iter().next() {
                assert_eq!(completion.id, id);
                assert_eq!(completion.result.unwrap().body, b"done");
                break;
            }
            assert!(Instant::now() < deadline, "fetch worker did not complete");
            std::thread::yield_now();
        }
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
        released.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(1);
        while runtime.has_pending() {
            assert!(runtime.drain_completions().is_empty());
            assert!(Instant::now() < deadline, "cancelled fetch did not finish");
            std::thread::yield_now();
        }
        assert!(runtime.drain_completions().is_empty());
    }
}
