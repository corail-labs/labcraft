//! Real TCP and UDP, over the standard library of Rust (`docs/io.md`, section 7). Each function here blocks, and runs in the
//! `Blocking` work of its call, away from the thread of the diagram. A connection holds the bytes received and not taken yet,
//! behind a mutex that its work owns while it runs; a clone of its stream stays outside, so that a release shuts it down
//! without waiting for that mutex.

#![expect(clippy::disallowed_methods, reason = "never built for WASM, where the clock traps: real sockets need the real clock")]

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use labcraft_runtime::io::{CLOSED, TIMEOUT};
use labcraft_trace::number::duration_text;

use crate::device::{Got, Need, Scan, Taken};
use crate::endpoint::HOST_NOT_FOUND;
use crate::run::{CONNECTION_REFUSED, DATAGRAM_TOO_LONG, Failure, wanted};

/// A datagram is received whole into this many bytes, more than the largest one UDP carries; and one read of a stream takes
/// at most this many.
const BUFFER: usize = 65_536;

fn code(error: &std::io::Error) -> i64 {
    error.raw_os_error().map_or(0, i64::from)
}

fn fail(category: &'static str, error: &std::io::Error, message: String) -> Failure {
    Failure { category, code: code(error), message }
}

fn timed_out(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock)
}

fn within(timeout: Duration) -> String {
    duration_text(i64::try_from(timeout.as_nanos()).unwrap_or(i64::MAX))
}

/// `io.timeout` of an open.
fn no_connection(endpoint: &str, timeout: Duration, code: i64) -> Failure {
    Failure { category: TIMEOUT, code, message: format!("{endpoint}: no connection within {}", within(timeout)) }
}

/// `io.closed` when the other end has closed.
fn other_end_closed(endpoint: &str) -> Failure {
    Failure { category: CLOSED, code: 0, message: format!("{endpoint}: the other end closed the connection") }
}

/// An error of a write or a send: `io.timeout` when the system could not take the bytes in time, else `io.closed`.
fn sending_error(error: &std::io::Error, endpoint: &str, timeout: Duration) -> Failure {
    if timed_out(error) {
        fail(TIMEOUT, error, format!("{endpoint}: the bytes could not be sent within {}", within(timeout)))
    } else {
        fail(CLOSED, error, format!("{endpoint}: the connection is closed: {error}"))
    }
}

/// A buffer of [`BUFFER`] bytes, made at its first use.
fn buffer(buffer: &mut Vec<u8>) -> &mut [u8] {
    if buffer.len() < BUFFER {
        buffer.resize(BUFFER, 0);
    }
    buffer
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A name that does not resolve is not found (README, `connect`).
fn resolve(host: &str, port: u16, endpoint: &str) -> Result<Vec<SocketAddr>, Failure> {
    match (host, port).to_socket_addrs() {
        Ok(addresses) => {
            let addresses: Vec<SocketAddr> = addresses.collect();
            if addresses.is_empty() {
                Err(Failure { category: HOST_NOT_FOUND, code: 0, message: format!("{endpoint}: the host has no address") })
            } else {
                Ok(addresses)
            }
        }
        Err(error) => Err(fail(HOST_NOT_FOUND, &error, format!("{endpoint}: the host does not resolve: {error}"))),
    }
}

/// An error of an open (decision of the owner, 2026-11-19): a host or a network unreachable is not found, a timeout is
/// `io.timeout`, any other error is refused.
fn open_error(error: &std::io::Error, endpoint: &str, timeout: Duration) -> Failure {
    match error.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => no_connection(endpoint, timeout, code(error)),
        ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable => fail(HOST_NOT_FOUND, error, format!("{endpoint}: unreachable: {error}")),
        _ => fail(CONNECTION_REFUSED, error, format!("{endpoint}: {error}")),
    }
}

/// A real TCP connection.
pub(crate) struct Tcp {
    state: Shared,
    /// The same socket, to shut it down from the thread of the run.
    shutter: Option<TcpStream>,
    broken: Arc<AtomicBool>,
}

/// Whether a call met a reset or an abort, after which the connection or the socket is closed (decision of the owner,
/// 2026-11-19).
fn broke(broken: &AtomicBool, error: &std::io::Error) {
    if !timed_out(error) && error.kind() != ErrorKind::Interrupted {
        broken.store(true, Ordering::Relaxed);
    }
}

/// The connection of a `Blocking` work.
pub(crate) type Shared = Arc<Mutex<Stream>>;

pub(crate) struct Stream {
    stream: TcpStream,
    endpoint: String,
    /// What each read and write may wait: the timeout of the open.
    timeout: Duration,
    /// Received, not taken yet, in order.
    waiting: VecDeque<u8>,
    /// The other end has closed.
    ended: bool,
    buffer: Vec<u8>,
    broken: Arc<AtomicBool>,
}

/// What one read of the system gave.
enum Polled {
    Bytes,
    Ended,
    /// Nothing yet: the timeout passed, or it would block.
    Nothing,
    /// A signal came first: try again.
    Again,
}

impl Tcp {
    /// Opens a connection within `timeout`, trying each address of the host in turn; a timeout of 0 still tries once, for
    /// 1 ms, as a simulated device connects at once. The name is resolved by the system first, which `timeout` does not bound.
    pub(crate) fn connect(host: &str, port: u16, timeout: Duration, endpoint: &str) -> Result<Tcp, Failure> {
        let addresses = resolve(host, port, endpoint)?;
        let deadline = Instant::now() + timeout.max(Duration::from_millis(1));
        let mut last = None;
        for address in addresses {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match TcpStream::connect_timeout(&address, left) {
                Ok(stream) => {
                    let _ = stream.set_nodelay(true);
                    let shutter = stream.try_clone().ok();
                    let broken = Arc::new(AtomicBool::new(false));
                    let state = Stream {
                        stream,
                        endpoint: endpoint.to_owned(),
                        timeout,
                        waiting: VecDeque::new(),
                        ended: false,
                        buffer: Vec::new(),
                        broken: Arc::clone(&broken),
                    };
                    return Ok(Tcp { state: Arc::new(Mutex::new(state)), shutter, broken });
                }
                Err(error) => last = Some(error),
            }
        }
        Err(match last {
            Some(error) => open_error(&error, endpoint, timeout),
            None => no_connection(endpoint, timeout, 0),
        })
    }

    pub(crate) fn handle(&self) -> Shared {
        Arc::clone(&self.state)
    }

    pub(crate) fn broken(&self) -> bool {
        self.broken.load(Ordering::Relaxed)
    }
}

/// Closing it shuts the socket down: the other end sees the end of the stream.
impl Drop for Tcp {
    fn drop(&mut self) {
        if let Some(shutter) = &self.shutter {
            let _ = shutter.shutdown(Shutdown::Both);
        }
    }
}

/// Writes every byte within the timeout of the connection, counted from `since`, the start of the call: each send of the
/// system waits what is left of it.
pub(crate) fn write(shared: &Shared, data: &[u8], since: Instant) -> Result<(), Failure> {
    let mut state = locked(shared);
    let Stream { stream, endpoint, timeout, broken, .. } = &mut *state;
    let deadline = since + *timeout;
    let mut sent = 0;
    let mut tried = false;
    while let Some(rest) = data.get(sent..).filter(|rest| !rest.is_empty()) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() && tried {
            return Err(Failure {
                category: TIMEOUT,
                code: 0,
                message: format!("{endpoint}: the bytes could not be sent within {}", within(*timeout)),
            });
        }
        tried = true;
        let _ = stream.set_write_timeout(Some(left.max(Duration::from_micros(1))));
        match stream.write(rest) {
            Ok(0) => return Err(Failure { category: CLOSED, code: 0, message: format!("{endpoint}: the connection is closed") }),
            Ok(written) => sent += written,
            Err(error) if timed_out(&error) || error.kind() == ErrorKind::Interrupted => {}
            Err(error) => {
                broke(broken, &error);
                return Err(sending_error(&error, endpoint, *timeout));
            }
        }
    }
    Ok(())
}

impl Stream {
    /// One read of the system, what it gives added to what waits.
    fn poll_once(&mut self) -> std::io::Result<Polled> {
        let Stream { stream, waiting, ended, buffer: chunk, .. } = self;
        match stream.read(buffer(chunk)) {
            Ok(0) => {
                *ended = true;
                Ok(Polled::Ended)
            }
            Ok(read) => {
                waiting.extend(chunk.get(..read).unwrap_or_default());
                Ok(Polled::Bytes)
            }
            Err(error) if timed_out(&error) => Ok(Polled::Nothing),
            Err(error) if error.kind() == ErrorKind::Interrupted => Ok(Polled::Again),
            Err(error) => Err(error),
        }
    }

    /// What the system holds already, without waiting, until `limit` bytes wait: the end of the stream, or an error, ends it.
    fn drain(&mut self, limit: usize) {
        if self.ended || self.stream.set_nonblocking(true).is_err() {
            return;
        }
        while self.waiting.len() < limit {
            match self.poll_once() {
                Ok(Polled::Bytes | Polled::Again) => {}
                Ok(Polled::Ended | Polled::Nothing) => break,
                Err(_) => {
                    self.ended = true;
                    break;
                }
            }
        }
        let _ = self.stream.set_nonblocking(false);
    }
}

/// The most bytes a read looks at: what a drain of the system stops at.
fn limit(need: &Need) -> usize {
    match need {
        Need::Exactly(count) => *count,
        Need::Until(_, max) | Need::Available(max) => *max,
    }
}

/// A read that started at `since`: what `need` takes of the bytes that wait, waiting for more up to the timeout of the
/// connection; `io.timeout` then, taking nothing.
pub(crate) fn read(shared: &Shared, need: &Need, since: Instant) -> Result<Taken, Failure> {
    let mut state = locked(shared);
    let deadline = since + state.timeout;
    if matches!(need, Need::Available(_)) {
        state.drain(limit(need));
    }
    let mut drained = false;
    let mut scan = Scan::default();
    loop {
        match need.judge(state.waiting.make_contiguous(), &mut scan) {
            Got::Take(0) if state.ended && state.waiting.is_empty() => return Err(other_end_closed(&state.endpoint)),
            Got::Take(count) => return Ok(Taken::Bytes(state.waiting.drain(..count).collect())),
            Got::TooLong(count) => return Ok(Taken::TooLong(state.waiting.drain(..count).collect())),
            Got::More => {}
        }
        if state.ended {
            return Err(other_end_closed(&state.endpoint));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            // What the system holds already counts, a timeout of 0 included.
            if !drained {
                drained = true;
                state.drain(limit(need));
                continue;
            }
            let message = format!("{}: {} within {}", state.endpoint, wanted(need), within(state.timeout));
            return Err(Failure { category: TIMEOUT, code: 0, message });
        }
        let _ = state.stream.set_read_timeout(Some(left));
        if let Err(error) = state.poll_once() {
            // A reset or an abort (decision of the owner, 2026-11-19).
            broke(&state.broken, &error);
            return Err(fail(CLOSED, &error, format!("{}: the connection is closed: {error}", state.endpoint)));
        }
    }
}

/// A real UDP socket, connected to its endpoint.
pub(crate) struct Udp {
    state: Arc<Mutex<Datagrams>>,
    broken: Arc<AtomicBool>,
}

pub(crate) struct Datagrams {
    socket: UdpSocket,
    endpoint: String,
    timeout: Duration,
    buffer: Vec<u8>,
    broken: Arc<AtomicBool>,
}

impl Udp {
    /// Binds `local_port` on the family of the endpoint, then connects to it.
    pub(crate) fn open(host: &str, port: u16, local_port: u16, timeout: Duration, endpoint: &str) -> Result<Udp, Failure> {
        let addresses = resolve(host, port, endpoint)?;
        let mut last = None;
        for address in addresses {
            let local: SocketAddr =
                if address.is_ipv4() { (Ipv4Addr::UNSPECIFIED, local_port).into() } else { (Ipv6Addr::UNSPECIFIED, local_port).into() };
            match UdpSocket::bind(local).and_then(|socket| socket.connect(address).map(|()| socket)) {
                Ok(socket) => {
                    let broken = Arc::new(AtomicBool::new(false));
                    let state = Datagrams { socket, endpoint: endpoint.to_owned(), timeout, buffer: Vec::new(), broken: Arc::clone(&broken) };
                    return Ok(Udp { state: Arc::new(Mutex::new(state)), broken });
                }
                Err(error) => last = Some(error),
            }
        }
        Err(match last {
            Some(error) => open_error(&error, endpoint, Duration::ZERO),
            None => Failure { category: HOST_NOT_FOUND, code: 0, message: format!("{endpoint}: the host has no address") },
        })
    }

    pub(crate) fn handle(&self) -> Arc<Mutex<Datagrams>> {
        Arc::clone(&self.state)
    }

    pub(crate) fn broken(&self) -> bool {
        self.broken.load(Ordering::Relaxed)
    }
}

/// Sends one datagram, whose size the call has judged, within what is left of the timeout from `since`.
pub(crate) fn send(shared: &Mutex<Datagrams>, data: &[u8], since: Instant) -> Result<(), Failure> {
    let state = locked(shared);
    let left = (since + state.timeout).saturating_duration_since(Instant::now());
    let _ = state.socket.set_write_timeout(Some(left.max(Duration::from_micros(1))));
    match state.socket.send(data) {
        Ok(sent) if sent == data.len() => Ok(()),
        Ok(sent) => {
            Err(Failure { category: DATAGRAM_TOO_LONG, code: 0, message: format!("{}: {sent} of {} bytes sent", state.endpoint, data.len()) })
        }
        Err(error) => {
            broke(&state.broken, &error);
            Err(sending_error(&error, &state.endpoint, state.timeout))
        }
    }
}

/// Takes one datagram, waiting up to the timeout of the socket from `since`; one longer than `max` is dropped.
pub(crate) fn receive(shared: &Mutex<Datagrams>, max: usize, since: Instant) -> Result<Taken, Failure> {
    let mut state = locked(shared);
    let deadline = since + state.timeout;
    let Datagrams { socket, endpoint, timeout, buffer: chunk, broken } = &mut *state;
    let mut polled = false;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let received = if left.is_zero() {
            if polled {
                return Err(Failure { category: TIMEOUT, code: 0, message: format!("{endpoint}: no datagram within {}", within(*timeout)) });
            }
            polled = true;
            let _ = socket.set_nonblocking(true);
            let received = socket.recv(buffer(chunk));
            let _ = socket.set_nonblocking(false);
            received
        } else {
            let _ = socket.set_read_timeout(Some(left));
            socket.recv(buffer(chunk))
        };
        match received {
            Ok(length) => {
                let datagram = chunk.get(..length).unwrap_or_default().to_vec();
                return Ok(if length > max { Taken::DatagramTooLong(datagram) } else { Taken::Bytes(datagram) });
            }
            Err(error) if timed_out(&error) || error.kind() == ErrorKind::Interrupted => {}
            // A reset (the endpoint refused an earlier datagram) or an abort.
            Err(error) => {
                broke(broken, &error);
                return Err(fail(CLOSED, &error, format!("{endpoint}: {error}")));
            }
        }
    }
}
