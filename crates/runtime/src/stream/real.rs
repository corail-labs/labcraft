//! A real stream behind a port or a socket of the system (`docs/io.md`, section 7), for the libraries of raw streams: the
//! bytes received and not taken yet, and the read and the write that wait within the timeout of the open, counted from the
//! start of the call. Each library gives its port or socket as a [`Wire`], and says itself what the end of the stream and
//! an error of the system are (its categories, its messages, whether the handle is closed after). Never built for WASM.

#![expect(clippy::disallowed_methods, reason = "never built for WASM, where the clock traps: real streams need the real clock")]

use std::collections::VecDeque;
use std::io::ErrorKind;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use labcraft_trace::number::duration_text;

use super::{Failure, Got, Need, Scan, Taken};
use crate::io::TIMEOUT;

/// One read of the system takes at most this many bytes; a datagram is received whole into as many, more than the largest
/// one UDP carries.
pub const BUFFER: usize = 65_536;

/// The code of an error of the system, 0 when it has none (io.md 7).
pub fn code(error: &std::io::Error) -> i64 {
    error.raw_os_error().map_or(0, i64::from)
}

/// Whether an error is a timeout, or a read or write that would wait.
pub fn timed_out(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock)
}

/// A timeout, as a message writes it.
pub fn within(timeout: Duration) -> String {
    duration_text(i64::try_from(timeout.as_nanos()).unwrap_or(i64::MAX))
}

/// A mutex, whatever a work that panicked left in it.
pub fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Marks the handle as gone (decision of the owner, 2026-11-19) after an error of a call that is not a timeout.
pub fn broke(broken: &AtomicBool, error: &std::io::Error) {
    if !timed_out(error) && error.kind() != ErrorKind::Interrupted {
        broken.store(true, Ordering::Relaxed);
    }
}

/// A buffer of [`BUFFER`] bytes, made at its first use.
pub fn buffer(buffer: &mut Vec<u8>) -> &mut [u8] {
    if buffer.len() < BUFFER {
        buffer.resize(BUFFER, 0);
    }
    buffer
}

/// A port or a socket of the system, as a stream reads and writes it.
pub trait Wire {
    /// One read, waiting up to `wait`, which is not zero: 0 bytes is the end of the stream.
    fn read_within(&mut self, buffer: &mut [u8], wait: Duration) -> std::io::Result<usize>;
    /// One read of what the system holds already, waiting as little as it can, between [`Wire::start_drain`] and
    /// [`Wire::end_drain`].
    fn read_now(&mut self, buffer: &mut [u8]) -> std::io::Result<usize>;
    /// One write, waiting up to `wait`, which may be zero.
    fn write_within(&mut self, data: &[u8], wait: Duration) -> std::io::Result<usize>;
    /// Before the reads of a drain (a socket that stops blocking); `false`: the drain reads nothing.
    fn start_drain(&mut self) -> bool {
        true
    }
    /// After them.
    fn end_drain(&mut self) {}
    /// Whether a read of `read` bytes, of the `offered` it could take, ends the drain: for a port that returns as soon as
    /// one byte arrives, a short read says the system held no more when it was asked, and a board that prints without end
    /// would otherwise keep the drain going until its `max`. By default only nothing ends it.
    fn ends_drain(&self, read: usize, offered: usize) -> bool {
        let _ = (read, offered);
        false
    }
}

/// Why a read or a write of a real stream stopped without its bytes.
#[derive(Debug)]
pub enum Stop {
    /// `io.timeout`, said.
    TimedOut(Failure),
    /// The end of the stream: a read of 0 bytes, or a write that took none.
    Ended,
    /// An error of the system, after which the handle is marked gone ([`broke`]).
    Error(std::io::Error),
}

/// What one read of the system gave.
enum Polled {
    /// This many bytes.
    Bytes(usize),
    Ended,
    /// Nothing yet: the timeout passed, or it would block.
    Nothing,
    /// A signal came first: try again.
    Again,
}

/// A real stream: its port or socket, and the bytes received and not taken yet, in order.
pub struct Buffered<W> {
    pub wire: W,
    /// What its messages show of it.
    pub endpoint: String,
    /// What each read and write may wait: the timeout of the open.
    pub timeout: Duration,
    waiting: VecDeque<u8>,
    /// The stream has ended.
    ended: bool,
    buffer: Vec<u8>,
    /// Set by an error of a call: the handle is gone.
    pub broken: Arc<AtomicBool>,
}

impl<W: Wire> Buffered<W> {
    pub fn new(wire: W, endpoint: String, timeout: Duration, broken: Arc<AtomicBool>) -> Self {
        Buffered { wire, endpoint, timeout, waiting: VecDeque::new(), ended: false, buffer: Vec::new(), broken }
    }

    /// Forgets the bytes that wait (`serial`, `settle`).
    pub fn forget(&mut self) {
        self.waiting.clear();
    }

    /// One read of the system, waiting up to `wait`, or as little as it can with none, what it gives added to what waits.
    fn poll(&mut self, wait: Option<Duration>) -> std::io::Result<Polled> {
        let Buffered { wire, waiting, ended, buffer: chunk, .. } = self;
        let read = match wait {
            Some(wait) => wire.read_within(buffer(chunk), wait),
            None => wire.read_now(buffer(chunk)),
        };
        match read {
            Ok(0) => {
                *ended = true;
                Ok(Polled::Ended)
            }
            Ok(read) => {
                waiting.extend(chunk.get(..read).unwrap_or_default());
                Ok(Polled::Bytes(read))
            }
            Err(error) if timed_out(&error) => Ok(Polled::Nothing),
            Err(error) if error.kind() == ErrorKind::Interrupted => Ok(Polled::Again),
            Err(error) => Err(error),
        }
    }

    /// What the system holds already, until `limit` bytes wait: the end of the stream, an error, nothing more, or a read
    /// that [`Wire::ends_drain`], ends it. An error marks the handle gone, whatever the read makes of the bytes that wait.
    fn drain(&mut self, limit: usize) -> std::io::Result<()> {
        if self.ended || !self.wire.start_drain() {
            return Ok(());
        }
        let mut drained = Ok(());
        while !self.ended && self.waiting.len() < limit {
            match self.poll(None) {
                Ok(Polled::Bytes(read)) if self.wire.ends_drain(read, BUFFER) => break,
                Ok(Polled::Bytes(_) | Polled::Again) => {}
                Ok(Polled::Ended | Polled::Nothing) => break,
                Err(error) => {
                    broke(&self.broken, &error);
                    drained = Err(error);
                    break;
                }
            }
        }
        self.wire.end_drain();
        drained
    }

    /// A read that started at `since`: what `need` takes of the bytes that wait, waiting for more up to the timeout of the
    /// stream; `io.timeout` then, taking nothing. When the stream has ended, or the system failed, what had arrived is still
    /// read; a read that needs more stops.
    pub fn read(&mut self, need: &Need, since: Instant) -> Result<Taken, Stop> {
        // `count` 0 gives empty bytes at once, whatever the stream (libs/net, `read`).
        if let Need::Exactly(0) = need {
            return Ok(Taken::Bytes(Vec::new()));
        }
        let deadline = since + self.timeout;
        let mut failed = None;
        if matches!(need, Need::Available(_))
            && let Err(error) = self.drain(need.limit())
        {
            failed = Some(error);
        }
        let mut drained = false;
        let mut scan = Scan::default();
        loop {
            let ended = self.ended || failed.is_some();
            match need.judge(self.waiting.make_contiguous(), &mut scan) {
                Got::Take(0) if ended && self.waiting.is_empty() => break,
                Got::Take(count) => return Ok(Taken::Bytes(self.waiting.drain(..count).collect())),
                Got::TooLong(count) => return Ok(Taken::TooLong(self.waiting.drain(..count).collect())),
                Got::More => {}
            }
            if ended {
                break;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                // What the system holds already counts, a timeout of 0 included.
                if !drained {
                    drained = true;
                    if let Err(error) = self.drain(need.limit()) {
                        failed = Some(error);
                    }
                    continue;
                }
                let message = format!("{}: {} within {}", self.endpoint, need.wanted(), within(self.timeout));
                return Err(Stop::TimedOut(Failure::new(TIMEOUT, message)));
            }
            if let Err(error) = self.poll(Some(left)) {
                broke(&self.broken, &error);
                failed = Some(error);
            }
        }
        Err(match failed {
            Some(error) => Stop::Error(error),
            None => Stop::Ended,
        })
    }

    /// Writes every byte within the timeout of the stream, counted from `since`, the start of the call: each write of the
    /// system waits what is left of it, and is tried once even when nothing is left.
    pub fn write_all(&mut self, data: &[u8], since: Instant) -> Result<(), Stop> {
        let deadline = since + self.timeout;
        let mut sent = 0;
        let mut tried = false;
        while let Some(rest) = data.get(sent..).filter(|rest| !rest.is_empty()) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() && tried {
                let message = format!("{}: the bytes could not be sent within {}", self.endpoint, within(self.timeout));
                return Err(Stop::TimedOut(Failure::new(TIMEOUT, message)));
            }
            tried = true;
            match self.wire.write_within(rest, left) {
                Ok(0) => return Err(Stop::Ended),
                Ok(written) => sent += written,
                Err(error) if timed_out(&error) || error.kind() == ErrorKind::Interrupted => {}
                Err(error) => {
                    broke(&self.broken, &error);
                    return Err(Stop::Error(error));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use super::{Buffered, Stop, Wire};
    use crate::stream::{Need, Taken};

    /// A port that gives what `reads` says, read after read (then nothing), and counts its reads.
    struct Board {
        reads: VecDeque<std::io::Result<usize>>,
        count: usize,
        short_ends: bool,
    }

    impl Wire for Board {
        fn read_within(&mut self, buffer: &mut [u8], _: Duration) -> std::io::Result<usize> {
            self.read_now(buffer)
        }

        fn read_now(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.count += 1;
            let read = self.reads.pop_front().unwrap_or_else(|| Err(std::io::ErrorKind::TimedOut.into()))?;
            buffer[..read].fill(b'x');
            Ok(read)
        }

        fn write_within(&mut self, data: &[u8], _: Duration) -> std::io::Result<usize> {
            Ok(data.len())
        }

        fn ends_drain(&self, read: usize, offered: usize) -> bool {
            self.short_ends && read < offered
        }
    }

    fn stream(reads: Vec<std::io::Result<usize>>, short_ends: bool) -> Buffered<Board> {
        let board = Board { reads: reads.into(), count: 0, short_ends };
        Buffered::new(board, "serial:COM3".to_owned(), Duration::from_secs(1), Arc::new(AtomicBool::new(false)))
    }

    #[test]
    fn a_drain_stops_at_a_short_read_when_the_port_says_so() {
        // Review: `read_available(max: 100)` of a board that has 10 bytes at each read, without end. By hand: a port whose
        // short read ends the drain takes the 10 bytes of its first read, in one read; one whose does not reads 10 times, to
        // its `max` of 100.
        let mut port = stream((0..20).map(|_| Ok(10)).collect(), true);
        assert_eq!(port.read(&Need::Available(100), Instant::now()).unwrap(), Taken::Bytes(vec![b'x'; 10]));
        assert_eq!(port.wire.count, 1);
        let mut socket = stream((0..20).map(|_| Ok(10)).collect(), false);
        assert_eq!(socket.read(&Need::Available(100), Instant::now()).unwrap(), Taken::Bytes(vec![b'x'; 100]));
        assert_eq!(socket.wire.count, 10);
    }

    #[test]
    fn an_error_of_a_drain_marks_the_handle_gone_though_the_read_takes_what_waits() {
        // Code review: 5 bytes, then an error of the system. `read_available` takes the 5 bytes, and the handle is marked
        // gone, so that the next call is `io.closed`; a `read` of 6 stops with the error, the handle gone too.
        let mut port = stream(vec![Ok(5), Err(std::io::ErrorKind::BrokenPipe.into())], false);
        assert_eq!(port.read(&Need::Available(100), Instant::now()).unwrap(), Taken::Bytes(vec![b'x'; 5]));
        assert!(port.broken.load(Ordering::Relaxed));
        let mut port = stream(vec![Ok(5), Err(std::io::ErrorKind::BrokenPipe.into())], false);
        assert!(matches!(port.read(&Need::Exactly(6), Instant::now()), Err(Stop::Error(_))));
        assert!(port.broken.load(Ordering::Relaxed));
    }

    #[test]
    fn a_read_of_0_bytes_reads_nothing() {
        // libs/net, `read`: `count` 0 gives empty bytes at once; the system is not asked, so its error is not met.
        let mut port = stream(vec![Err(std::io::ErrorKind::BrokenPipe.into())], false);
        assert_eq!(port.read(&Need::Exactly(0), Instant::now()).unwrap(), Taken::Bytes(Vec::new()));
        assert_eq!((port.wire.count, port.broken.load(Ordering::Relaxed)), (0, false));
    }
}
