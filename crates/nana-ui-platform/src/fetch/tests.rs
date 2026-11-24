#[cfg(feature = "fetch")]
use std::io::{self, Read, Write};
#[cfg(feature = "fetch")]
use std::net::{TcpListener, TcpStream};
#[cfg(feature = "fetch")]
use std::sync::mpsc;
#[cfg(feature = "fetch")]
use std::thread;
#[cfg(feature = "fetch")]
use std::time::Duration;

#[cfg(feature = "fetch")]
use ureq::unversioned::transport::{LazyBuffers, NextTimeout, Transport};
#[cfg(feature = "fetch")]
use url::Url;

use super::transport::CancellableTcpTransport;
use super::*;

#[cfg(feature = "fetch")]
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

#[cfg(feature = "fetch")]
fn accept_with_timeout(listener: &TcpListener) -> std::net::TcpStream {
    listener.set_nonblocking(true).expect("nonblocking");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "timed out waiting for loopback test client"
                );
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("accept loopback request failed: {error}"),
        }
    };
    stream.set_nonblocking(false).expect("blocking stream");
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    stream
}

#[cfg(feature = "fetch")]
fn one_response_server(
    response: Vec<u8>,
) -> (String, mpsc::Receiver<Vec<u8>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback server");
    let address = listener.local_addr().unwrap();
    let (request_tx, request_rx) = mpsc::channel();
    let join = thread::spawn(move || {
        let mut stream = accept_with_timeout(&listener);
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
    let error = policy.authorize_url("https://example.com/a").unwrap_err();
    assert_eq!(error.kind, FetchErrorKind::Policy);
    #[cfg(feature = "fetch")]
    {
        let error = policy
            .authorize(&Url::parse("https://example.com/a").unwrap())
            .unwrap_err();
        assert_eq!(error.kind, FetchErrorKind::Policy);
    }
}

#[test]
fn policy_matches_normalized_origin_not_path() {
    let policy = FetchPolicy::default()
        .with_allowed_origin("https://example.com:443")
        .unwrap();
    policy
        .authorize_url("https://example.com/path?q=1")
        .unwrap();
    assert!(
        policy
            .authorize_url("https://api.example.com/path")
            .is_err()
    );
    #[cfg(feature = "fetch")]
    {
        policy
            .authorize(&Url::parse("https://example.com/path?q=1").unwrap())
            .unwrap();
        assert!(
            policy
                .authorize(&Url::parse("https://api.example.com/path").unwrap())
                .is_err()
        );
    }
}

#[test]
fn the_default_streaming_fallback_still_applies_the_hosts_own_cap() {
    /// Implements only `fetch`, so it takes the trait's buffered fallback.
    #[derive(Debug)]
    struct BufferedOnlyHost {
        policy: FetchPolicy,
        body: Vec<u8>,
    }
    impl FetchHost for BufferedOnlyHost {
        fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
            Ok(FetchResponse {
                url: request.url,
                status: 200,
                status_text: "OK".into(),
                headers: Vec::new(),
                body: self.body.clone(),
                redirected: false,
            })
        }
        fn policy(&self) -> &FetchPolicy {
            &self.policy
        }
    }
    struct NullSink(usize);
    impl FetchSink for NullSink {
        fn head(&mut self, _head: FetchHead) -> Result<(), FetchError> {
            Ok(())
        }
        fn chunk(&mut self, bytes: &[u8]) -> Result<(), FetchError> {
            self.0 += bytes.len();
            Ok(())
        }
    }

    let mut policy = FetchPolicy::default();
    policy.max_response_bytes = 1_000;
    let host = BufferedOnlyHost {
        policy: policy.clone(),
        body: vec![b'z'; 4_000],
    };
    let mut sink = NullSink(0);
    let error = host
        .fetch_streaming(
            FetchRequest::get("https://example.test/big"),
            FetchCancellation::new(),
            &mut sink,
        )
        .unwrap_err();
    assert_eq!(error.kind, FetchErrorKind::ResponseTooLarge);
    assert_eq!(sink.0, 0, "an over-cap body must not reach the sink at all");

    // Under the cap the same host streams normally, as one chunk.
    let host = BufferedOnlyHost {
        policy,
        body: vec![b'z'; 900],
    };
    let mut sink = NullSink(0);
    host.fetch_streaming(
        FetchRequest::get("https://example.test/ok"),
        FetchCancellation::new(),
        &mut sink,
    )
    .unwrap();
    assert_eq!(sink.0, 900);
}

#[cfg(feature = "fetch")]
#[test]
fn streaming_delivers_head_before_body_and_keeps_the_cap_cumulative() {
    #[derive(Default)]
    struct RecordingSink {
        head: Option<FetchHead>,
        chunks: Vec<Vec<u8>>,
        head_seen_before_first_chunk: bool,
    }
    impl FetchSink for RecordingSink {
        fn head(&mut self, head: FetchHead) -> Result<(), FetchError> {
            assert!(self.chunks.is_empty(), "head must precede every chunk");
            self.head_seen_before_first_chunk = true;
            self.head = Some(head);
            Ok(())
        }
        fn chunk(&mut self, bytes: &[u8]) -> Result<(), FetchError> {
            self.chunks.push(bytes.to_vec());
            Ok(())
        }
    }

    let body = vec![b'x'; 200_000];
    let mut raw = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    raw.extend_from_slice(&body);
    let (origin, _request_rx, join) = one_response_server(raw);
    let policy = FetchPolicy::default().with_allowed_origin(&origin).unwrap();
    let host = NativeFetchHost::new(policy);

    let mut sink = RecordingSink::default();
    host.fetch_streaming(
        FetchRequest::get(format!("{origin}/big")),
        FetchCancellation::new(),
        &mut sink,
    )
    .unwrap();
    join.join().unwrap();

    assert!(sink.head_seen_before_first_chunk);
    assert_eq!(sink.head.as_ref().map(|head| head.status), Some(200));
    assert!(
        sink.chunks.len() > 1,
        "a 200 KB body must arrive in several 64 KiB reads, got {} chunk(s)",
        sink.chunks.len()
    );
    assert_eq!(
        sink.chunks.iter().map(Vec::len).sum::<usize>(),
        body.len(),
        "streamed chunks must reassemble into the whole body"
    );
}

#[cfg(feature = "fetch")]
#[test]
fn streaming_enforces_the_same_response_cap_as_the_buffered_path() {
    struct CountingSink(usize);
    impl FetchSink for CountingSink {
        fn head(&mut self, _head: FetchHead) -> Result<(), FetchError> {
            Ok(())
        }
        fn chunk(&mut self, bytes: &[u8]) -> Result<(), FetchError> {
            self.0 += bytes.len();
            Ok(())
        }
    }

    // No Content-Length, so the cap can only be caught while reading: the
    // limit has to be cumulative across chunks, not per chunk.
    let body = vec![b'y'; 300_000];
    let mut raw = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
    raw.extend_from_slice(&body);
    let (origin, _request_rx, join) = one_response_server(raw);
    let mut policy = FetchPolicy::default().with_allowed_origin(&origin).unwrap();
    policy.max_response_bytes = 100_000;
    let host = NativeFetchHost::new(policy);

    let mut sink = CountingSink(0);
    let error = host
        .fetch_streaming(
            FetchRequest::get(format!("{origin}/big")),
            FetchCancellation::new(),
            &mut sink,
        )
        .unwrap_err();
    let _ = join.join();
    assert_eq!(error.kind, FetchErrorKind::ResponseTooLarge);
    assert!(
        sink.0 <= 100_000 + 64 * 1024,
        "the read must stop at the cap, not drain the whole body: {} bytes",
        sink.0
    );
}

#[cfg(feature = "fetch")]
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
    let request = String::from_utf8(
        request_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("receive loopback HTTP request"),
    )
    .unwrap();
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

#[cfg(feature = "fetch")]
#[test]
fn native_fetch_cancellation_interrupts_a_blocked_response_wait() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (waiting_tx, waiting_rx) = mpsc::sync_channel(1);
    let server = thread::spawn(move || {
        let mut stream = accept_with_timeout(&listener);
        let _ = read_request(&mut stream);
        waiting_tx.send(()).unwrap();
        let mut byte = [0_u8; 1];
        let _ = stream.read(&mut byte);
    });
    let policy = FetchPolicy::default().with_allowed_origin(&origin).unwrap();
    let host = NativeFetchHost::new(policy);
    let cancellation = FetchCancellation::new();
    let worker_cancellation = cancellation.clone();
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let result = host.fetch_cancellable(
            FetchRequest::get(format!("{origin}/waiting")),
            worker_cancellation,
        );
        let _ = result_tx.send(result);
    });

    waiting_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("fetch did not reach response wait");
    let started = std::time::Instant::now();
    cancellation.cancel();
    let error = result_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("cancelled socket did not release fetch worker")
        .unwrap_err();
    assert_eq!(error.kind, FetchErrorKind::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(1));
    server.join().unwrap();
}

#[cfg(feature = "fetch")]
#[test]
fn native_fetch_follows_authorized_redirect_and_buffers_json() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let origin = format!("http://{address}");
    let join = thread::spawn(move || {
        let mut first = accept_with_timeout(&listener);
        let _ = read_request(&mut first);
        first
            .write_all(
                b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        let mut second = accept_with_timeout(&listener);
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

#[cfg(feature = "fetch")]
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

#[cfg(feature = "fetch")]
#[test]
fn native_fetch_enforces_timeout_and_body_limits() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let origin = format!("http://{address}");
    // Hold the reply until the client has already given up. A fixed sleep
    // raced the timeout: the read deadline is per-read, so a scheduling
    // stall long enough to buffer the whole response made the read succeed
    // instantly and the timeout never fired.
    let (release, wait_for_release) = mpsc::channel::<()>();
    let join = thread::spawn(move || {
        let mut stream = accept_with_timeout(&listener);
        let _ = read_request(&mut stream);
        let _ = wait_for_release.recv_timeout(Duration::from_secs(5));
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    });
    let mut timeout_policy = FetchPolicy::default().with_allowed_origin(&origin).unwrap();
    timeout_policy.timeout = Duration::from_millis(25);
    let error = NativeFetchHost::new(timeout_policy)
        .fetch(FetchRequest::get(format!("{origin}/slow")))
        .unwrap_err();
    let _ = release.send(());
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

/// The socket is never registered with the token, standing in for a
/// platform where `shutdown` does not wake the read.
#[cfg(feature = "fetch")]
#[test]
fn a_stalled_read_observes_cancellation_without_relying_on_socket_shutdown() {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).expect("bind loopback");
    let address = listener.local_addr().expect("loopback address");
    let accepted = thread::spawn(move || listener.accept().map(|(stream, _)| stream));
    let stream = TcpStream::connect(address).expect("connect loopback");
    let server = accepted.join().expect("accept thread").expect("accept");

    let cancellation = FetchCancellation::new();
    let mut transport = CancellableTcpTransport {
        stream,
        buffers: LazyBuffers::new(4096, 4096),
        cancellation: cancellation.clone(),
        read_timeout: None,
        write_timeout: None,
    };
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        cancellation.cancel();
    });

    let started = std::time::Instant::now();
    let error = transport
        .await_input(NextTimeout {
            after: ureq::unversioned::transport::time::Duration::Exact(Duration::from_secs(30)),
            reason: ureq::Timeout::Global,
        })
        .expect_err("a cancelled read must not succeed");
    let waited = started.elapsed();
    canceller.join().expect("canceller thread");
    drop(server);

    assert!(
        matches!(&error, ureq::Error::Io(error) if error.kind() == io::ErrorKind::Interrupted),
        "expected an interrupted read, got {error:?}"
    );
    assert!(
        waited < Duration::from_secs(5),
        "the read must come back on its own; waited {waited:?}"
    );
}
