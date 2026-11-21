//! Real TCP and UDP, over the standard library of Rust (`docs/io.md`, section 7). Each function here blocks, and runs in the
//! `Blocking` work of its call, away from the thread of the diagram. A connection is a real stream of `runtime::stream`,
//! behind a mutex that its work owns while it runs; a clone of its socket stays outside, so that a release shuts it down
//! without waiting for that mutex.

#![expect(clippy::disallowed_methods, reason = "never built for WASM, where the clock traps: real sockets need the real clock")]

use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use labcraft_runtime::io::{CLOSED, TIMEOUT};
use labcraft_runtime::stream::real::{Buffered, Stop, Wire, broke, buffer, code, locked, timed_out, within};
use labcraft_runtime::stream::{Failure, Need, Taken};

use crate::endpoint::HOST_NOT_FOUND;
use crate::run::{CONNECTION_REFUSED, DATAGRAM_TOO_LONG};

fn fail(category: &'static str, error: &std::io::Error, message: String) -> Failure {
    Failure { category, code: code(error), message }
}

/// `io.timeout` of an open.
fn no_connection(endpoint: &str, timeout: Duration, code: i64) -> Failure {
    Failure { category: TIMEOUT, code, message: format!("{endpoint}: no connection within {}", within(timeout)) }
}

/// `io.closed` when the other end has closed.
fn other_end_closed(endpoint: &str) -> Failure {
    Failure::new(CLOSED, format!("{endpoint}: the other end closed the connection"))
}

/// An error of a write or a send: `io.timeout` when the system could not take the bytes in time, else `io.closed`.
fn sending_error(error: &std::io::Error, endpoint: &str, timeout: Duration) -> Failure {
    if timed_out(error) {
        fail(TIMEOUT, error, format!("{endpoint}: the bytes could not be sent within {}", within(timeout)))
    } else {
        fail(CLOSED, error, format!("{endpoint}: the connection is closed: {error}"))
    }
}

/// A name that does not resolve is not found (README, `connect`).
fn resolve(host: &str, port: u16, endpoint: &str) -> Result<Vec<SocketAddr>, Failure> {
    match (host, port).to_socket_addrs() {
        Ok(addresses) => {
            let addresses: Vec<SocketAddr> = addresses.collect();
            if addresses.is_empty() { Err(Failure::new(HOST_NOT_FOUND, format!("{endpoint}: the host has no address"))) } else { Ok(addresses) }
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

/// A TCP socket as a stream reads it.
pub(crate) struct Socket(TcpStream);

impl Wire for Socket {
    fn read_within(&mut self, buffer: &mut [u8], wait: Duration) -> std::io::Result<usize> {
        let _ = self.0.set_read_timeout(Some(wait));
        self.0.read(buffer)
    }

    /// Without blocking, within a drain. An error ends the stream, as a read of 0 bytes does: what waits is still read.
    fn read_now(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self.0.read(buffer) {
            Err(error) if !timed_out(&error) && error.kind() != ErrorKind::Interrupted => Ok(0),
            read => read,
        }
    }

    /// The socket stops blocking for the whole drain; when it cannot, the drain reads nothing.
    fn start_drain(&mut self) -> bool {
        self.0.set_nonblocking(true).is_ok()
    }

    fn end_drain(&mut self) {
        let _ = self.0.set_nonblocking(false);
    }

    fn write_within(&mut self, data: &[u8], wait: Duration) -> std::io::Result<usize> {
        let _ = self.0.set_write_timeout(Some(wait.max(Duration::from_micros(1))));
        self.0.write(data)
    }
}

/// The connection of a `Blocking` work.
pub(crate) type Shared = Arc<Mutex<Buffered<Socket>>>;

/// A real TCP connection.
pub(crate) struct Tcp {
    state: Shared,
    /// The same socket, to shut it down from the thread of the run.
    shutter: Option<TcpStream>,
    broken: Arc<AtomicBool>,
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
                    let state = Buffered::new(Socket(stream), endpoint.to_owned(), timeout, Arc::clone(&broken));
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

/// Writes every byte within the timeout of the connection, counted from `since`, the start of the call.
pub(crate) fn write(shared: &Shared, data: &[u8], since: Instant) -> Result<(), Failure> {
    let mut state = locked(shared);
    state.write_all(data, since).map_err(|stop| match stop {
        Stop::TimedOut(failure) => failure,
        Stop::Ended => Failure::new(CLOSED, format!("{}: the connection is closed", state.endpoint)),
        Stop::Error(error) => sending_error(&error, &state.endpoint, state.timeout),
    })
}

/// A read that started at `since`: what `need` takes of the bytes that wait, waiting for more up to the timeout of the
/// connection; `io.timeout` then, taking nothing. When the other end has closed, what has arrived is still read.
pub(crate) fn read(shared: &Shared, need: &Need, since: Instant) -> Result<Taken, Failure> {
    let mut state = locked(shared);
    state.read(need, since).map_err(|stop| match stop {
        Stop::TimedOut(failure) => failure,
        Stop::Ended => other_end_closed(&state.endpoint),
        // A reset or an abort (decision of the owner, 2026-11-19).
        Stop::Error(error) => fail(CLOSED, &error, format!("{}: the connection is closed: {error}", state.endpoint)),
    })
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
            None => Failure::new(HOST_NOT_FOUND, format!("{endpoint}: the host has no address")),
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
        Ok(sent) => Err(Failure::new(DATAGRAM_TOO_LONG, format!("{}: {sent} of {} bytes sent", state.endpoint, data.len()))),
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
                return Err(Failure::new(TIMEOUT, format!("{endpoint}: no datagram within {}", within(*timeout))));
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
