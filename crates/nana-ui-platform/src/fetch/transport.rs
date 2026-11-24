use std::fmt;
use std::io::{self, Read as _, Write as _};
use std::net::TcpStream;
use std::time::Duration;

use ureq::unversioned::resolver::{DefaultResolver, Resolver};
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, Either, LazyBuffers, NextTimeout, Transport,
};

use super::FetchCancellation;

const FETCH_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Longest one address may spend in the TCP handshake.
///
/// `TcpStream::connect_timeout` cannot be resumed, so it cannot be sliced for
/// cancellation polling the way a read can: every slice abandons the half-open
/// socket and sends a fresh SYN, and a peer whose handshake takes longer than
/// one slice never connects at all. The handshake therefore runs in one piece;
/// cancellation is checked between addresses, and this cap bounds how long a
/// black-holed address can hold the request before the next one gets its turn.
const FETCH_CONNECT_ATTEMPT: Duration = Duration::from_secs(10);

pub(super) fn cancelled_transport_error() -> ureq::Error {
    ureq::Error::Io(io::Error::new(
        io::ErrorKind::Interrupted,
        "fetch was cancelled",
    ))
}

#[derive(Debug, Clone)]
pub(super) struct CancellableResolver {
    cancellation: FetchCancellation,
}

impl CancellableResolver {
    pub(super) fn new(cancellation: FetchCancellation) -> Self {
        Self { cancellation }
    }
}

impl Resolver for CancellableResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        if self.cancellation.is_cancelled() {
            return Err(cancelled_transport_error());
        }
        let uri = uri.clone();
        let config = config.clone();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("nana-fetch-dns".into())
            .spawn(move || {
                let result = DefaultResolver::default().resolve(&uri, &config, timeout);
                let _ = sender.send(result);
            })
            .map_err(ureq::Error::Io)?;
        let deadline = timeout
            .not_zero()
            .and_then(|duration| std::time::Instant::now().checked_add(*duration));
        loop {
            if self.cancellation.is_cancelled() {
                return Err(cancelled_transport_error());
            }
            let wait = deadline
                .map(|deadline| {
                    deadline
                        .saturating_duration_since(std::time::Instant::now())
                        .min(FETCH_CANCEL_POLL_INTERVAL)
                })
                .unwrap_or(FETCH_CANCEL_POLL_INTERVAL);
            if wait.is_zero() {
                return Err(ureq::Error::Timeout(timeout.reason));
            }
            match receiver.recv_timeout(wait) {
                Ok(result) => return result,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(ureq::Error::Io(io::Error::other(
                        "fetch resolver stopped without a result",
                    )));
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct CancellableTcpConnector {
    cancellation: FetchCancellation,
}

impl CancellableTcpConnector {
    pub(super) fn new(cancellation: FetchCancellation) -> Self {
        Self { cancellation }
    }
}

impl<In: Transport> Connector<In> for CancellableTcpConnector {
    type Out = Either<In, CancellableTcpTransport>;

    fn connect(
        &self,
        details: &ConnectionDetails<'_>,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        if let Some(chained) = chained {
            return Ok(Some(Either::A(chained)));
        }
        if self.cancellation.is_cancelled() {
            return Err(cancelled_transport_error());
        }
        let deadline = details
            .timeout
            .not_zero()
            .and_then(|duration| std::time::Instant::now().checked_add(*duration));
        let mut last_error = None;
        for address in &details.addrs {
            if self.cancellation.is_cancelled() {
                return Err(cancelled_transport_error());
            }
            let remaining = deadline
                .map(|deadline| deadline.saturating_duration_since(std::time::Instant::now()));
            if remaining.is_some_and(|remaining| remaining.is_zero()) {
                return Err(ureq::Error::Timeout(details.timeout.reason));
            }
            let attempt = remaining.map_or(FETCH_CONNECT_ATTEMPT, |remaining| {
                remaining.min(FETCH_CONNECT_ATTEMPT)
            });
            match TcpStream::connect_timeout(address, attempt) {
                Ok(stream) => {
                    if details.config.no_delay() {
                        stream.set_nodelay(true).map_err(ureq::Error::Io)?;
                    }
                    self.cancellation
                        .register_socket(&stream)
                        .map_err(ureq::Error::Io)?;
                    let buffers = LazyBuffers::new(
                        details.config.input_buffer_size(),
                        details.config.output_buffer_size(),
                    );
                    return Ok(Some(Either::B(CancellableTcpTransport {
                        stream,
                        buffers,
                        cancellation: self.cancellation.clone(),
                        read_timeout: None,
                        write_timeout: None,
                    })));
                }
                // Every failure moves on to the next resolved address, not just
                // a refusal: a dual-stack machine with no IPv6 route gets
                // `NetworkUnreachable` on the AAAA record that DNS put first,
                // and the A record behind it would have connected.
                Err(error) => last_error = Some(error),
            }
        }
        if self.cancellation.is_cancelled() {
            return Err(cancelled_transport_error());
        }
        if deadline.is_some_and(|deadline| deadline <= std::time::Instant::now()) {
            return Err(ureq::Error::Timeout(details.timeout.reason));
        }
        Err(ureq::Error::Io(last_error.unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "no fetch address connected",
            )
        })))
    }
}

pub(super) struct CancellableTcpTransport {
    pub(super) stream: TcpStream,
    pub(super) buffers: LazyBuffers,
    pub(super) cancellation: FetchCancellation,
    pub(super) read_timeout: Option<Duration>,
    pub(super) write_timeout: Option<Duration>,
}

impl fmt::Debug for CancellableTcpTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellableTcpTransport")
            .field("cancelled", &self.cancellation.is_cancelled())
            .finish_non_exhaustive()
    }
}

fn update_socket_timeout(
    socket: &TcpStream,
    previous: &mut Option<Duration>,
    timeout: Option<Duration>,
    set: impl FnOnce(&TcpStream, Option<Duration>) -> io::Result<()>,
) -> Result<(), ureq::Error> {
    if *previous != timeout {
        set(socket, timeout).map_err(ureq::Error::Io)?;
        *previous = timeout;
    }
    Ok(())
}

impl Transport for CancellableTcpTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        if self.cancellation.is_cancelled() {
            return Err(cancelled_transport_error());
        }
        update_socket_timeout(
            &self.stream,
            &mut self.write_timeout,
            timeout.not_zero().map(|duration| *duration),
            TcpStream::set_write_timeout,
        )?;
        let result = self.stream.write_all(&self.buffers.output()[..amount]);
        if self.cancellation.is_cancelled() {
            return Err(cancelled_transport_error());
        }
        result.map_err(|error| match error.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => {
                ureq::Error::Timeout(timeout.reason)
            }
            _ => ureq::Error::Io(error),
        })
    }

    /// Reads in `FETCH_CANCEL_POLL_INTERVAL` slices: `cancel()` shuts the
    /// socket down, which Winsock does not guarantee wakes a blocked `recv`.
    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        let deadline = timeout
            .not_zero()
            .and_then(|duration| std::time::Instant::now().checked_add(*duration));
        loop {
            if self.cancellation.is_cancelled() {
                return Err(cancelled_transport_error());
            }
            let remaining = deadline
                .map(|deadline| deadline.saturating_duration_since(std::time::Instant::now()));
            if remaining.is_some_and(|remaining| remaining.is_zero()) {
                return Err(ureq::Error::Timeout(timeout.reason));
            }
            let attempt = remaining
                .map(|remaining| remaining.min(FETCH_CANCEL_POLL_INTERVAL))
                .unwrap_or(FETCH_CANCEL_POLL_INTERVAL);
            update_socket_timeout(
                &self.stream,
                &mut self.read_timeout,
                Some(attempt),
                TcpStream::set_read_timeout,
            )?;
            let result = self.stream.read(self.buffers.input_append_buf());
            if self.cancellation.is_cancelled() {
                return Err(cancelled_transport_error());
            }
            match result {
                Ok(amount) => {
                    self.buffers.input_appended(amount);
                    return Ok(amount > 0);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) => {}
                Err(error) => return Err(ureq::Error::Io(error)),
            }
        }
    }

    fn is_open(&mut self) -> bool {
        !self.cancellation.is_cancelled() && self.stream.peer_addr().is_ok()
    }
}
