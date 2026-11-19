//! A simulated device during a run (`docs/io.md`, section 6), seen from one connection or socket, on the clock of the run:
//! the bytes it received and has not matched yet, and the responses it gave, each ready `latency_ms` after the write that
//! asked for it. And what a read needs of the bytes that wait, which a real connection judges the same way, so that a real
//! connection and a simulated one read the same (`libs/net/README.md`, "What `net` guarantees").

use std::collections::VecDeque;
use std::sync::Arc;

use crate::file::{DeviceSpec, Reply};

/// What a read of a connection needs.
#[derive(Clone, Debug)]
pub(crate) enum Need {
    /// `read`: exactly this many bytes.
    Exactly(usize),
    /// `read_until`, `read_line`: up to the terminator, included, within `max` bytes.
    Until(Terminator, usize),
    /// `read_available`: what is there, up to `max` bytes.
    Available(usize),
}

/// What the bytes that wait give a read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Got {
    /// It takes this many bytes.
    Take(usize),
    /// `max` bytes without the terminator: `io.too_long`, and it takes them.
    TooLong(usize),
    /// It needs more than is there.
    More,
}

/// A terminator, with the table of Knuth, Morris and Pratt that lets a search go on where it stopped: a read that waits looks
/// at each byte that arrives once, whatever the terminator (`libs/net/README.md`, `read_until`).
#[derive(Clone, Debug)]
pub(crate) struct Terminator {
    pub(crate) bytes: Vec<u8>,
    /// For each length matched, the longest proper prefix of the terminator that is also a suffix of that much of it.
    fallback: Vec<usize>,
}

impl Terminator {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        let mut fallback = vec![0; bytes.len()];
        let mut matched = 0;
        for (at, byte) in bytes.iter().enumerate().skip(1) {
            while matched > 0 && bytes.get(matched) != Some(byte) {
                matched = fallback.get(matched - 1).copied().unwrap_or(0);
            }
            if bytes.get(matched) == Some(byte) {
                matched += 1;
            }
            if let Some(slot) = fallback.get_mut(at) {
                *slot = matched;
            }
        }
        Terminator { bytes, fallback }
    }
}

/// Where the search of a read stopped: the bytes of what waits it has looked at, and how much of the terminator ends them.
/// What waits only grows while the read holds its connection, so the search goes on from there.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Scan {
    looked: usize,
    matched: usize,
}

impl Need {
    pub(crate) fn judge(&self, waiting: &[u8], scan: &mut Scan) -> Got {
        match self {
            Need::Exactly(count) if waiting.len() >= *count => Got::Take(*count),
            Need::Exactly(_) => Got::More,
            Need::Until(terminator, max) => {
                let within = waiting.get(..waiting.len().min(*max)).unwrap_or_default();
                let bytes = &terminator.bytes;
                for (at, byte) in within.iter().enumerate().skip(scan.looked) {
                    while scan.matched > 0 && bytes.get(scan.matched) != Some(byte) {
                        scan.matched = terminator.fallback.get(scan.matched - 1).copied().unwrap_or(0);
                    }
                    if bytes.get(scan.matched) == Some(byte) {
                        scan.matched += 1;
                    }
                    if scan.matched == bytes.len() && !bytes.is_empty() {
                        return Got::Take(at + 1);
                    }
                }
                scan.looked = scan.looked.max(within.len());
                if waiting.len() >= *max { Got::TooLong(*max) } else { Got::More }
            }
            Need::Available(max) => Got::Take(waiting.len().min(*max)),
        }
    }
}

/// What a read took, simulated or real.
#[derive(Debug)]
pub(crate) enum Taken {
    Bytes(Vec<u8>),
    /// `io.too_long`: the `max` bytes taken without the terminator.
    TooLong(Vec<u8>),
    /// `net.datagram_too_long`: the datagram, dropped.
    DatagramTooLong(Vec<u8>),
}

/// A device seen from one connection (TCP) or one socket (UDP).
#[derive(Debug)]
pub(crate) struct Simulated {
    pub(crate) spec: Arc<DeviceSpec>,
    /// What it received and has not matched yet (TCP).
    pending: Vec<u8>,
    /// What it answered over TCP, ready or not, in order.
    stream: VecDeque<u8>,
    /// When each answer over TCP is ready, with how many bytes of `stream` it ends at, counted from the first byte ever
    /// answered.
    marks: VecDeque<(i64, u64)>,
    /// Bytes of `stream` taken since the first.
    taken: u64,
    /// What it answered over UDP, each datagram with when it is ready.
    datagrams: VecDeque<(i64, Vec<u8>)>,
    /// The bytes of `datagrams`.
    datagram_bytes: usize,
}

/// What a device keeps of its answers not read yet, per connection or socket (decision of the owner, 2026-11-19): a response
/// that would pass it is dropped, as by a device whose buffer overflows.
pub(crate) const MAX_WAITING: usize = 16 * 1024 * 1024;

impl Simulated {
    pub(crate) fn new(spec: Arc<DeviceSpec>) -> Self {
        Simulated {
            spec,
            pending: Vec::new(),
            stream: VecDeque::new(),
            marks: VecDeque::new(),
            taken: 0,
            datagrams: VecDeque::new(),
            datagram_bytes: 0,
        }
    }

    /// Whether the device has closed the connection at `now` (TCP, `close_ms`).
    pub(crate) fn closed(&self, now: i64) -> bool {
        self.spec.close_at().is_some_and(|close| now >= close)
    }

    fn hung(&self, now: i64) -> bool {
        self.spec.hang.is_some_and(|hang| now >= hang)
    }

    /// The response to `reply` asked for at `now`, or to what matches nothing (`unknown`): ready after the latency, and never
    /// given when the device hangs or closes first, or when it would pass [`MAX_WAITING`].
    fn respond(&mut self, now: i64, reply: Option<&Reply>) {
        let spec = Arc::clone(&self.spec);
        let response = match reply {
            Some(reply) => reply.response.as_ref(),
            None => spec.unknown.as_ref(),
        };
        let Some(response) = response else { return };
        let ready = now.saturating_add(spec.latency);
        if spec.hang.is_some_and(|hang| ready >= hang) || spec.close_at().is_some_and(|close| ready >= close) {
            return;
        }
        let waiting = if spec.udp { self.datagram_bytes } else { self.stream.len() };
        if waiting.saturating_add(response.len()) > MAX_WAITING {
            return;
        }
        if spec.udp {
            self.datagram_bytes += response.len();
            self.datagrams.push_back((ready, response.clone()));
        } else {
            self.stream.extend(response);
            let end = self.taken.saturating_add(self.stream.len() as u64);
            self.marks.push_back((ready, end));
        }
    }

    /// A write at `now` (io.md 6): the bytes join those not matched; while they start with a request, it is taken from the
    /// front and its response asked for; when what is left is the start of some request, the device waits for more;
    /// otherwise it forgets it and answers `unknown`, if it has one. A device that hangs takes nothing.
    pub(crate) fn write(&mut self, now: i64, data: &[u8]) {
        if self.hung(now) {
            return;
        }
        self.pending.extend_from_slice(data);
        let spec = Arc::clone(&self.spec);
        let mut start = 0;
        while let Some(rest) = self.pending.get(start..).filter(|rest| !rest.is_empty()) {
            if let Some(reply) = spec.starting(rest) {
                start += reply.request.len();
                self.respond(now, Some(reply));
                continue;
            }
            if !spec.waits_for_more(rest) {
                start = self.pending.len();
                self.respond(now, None);
            }
            break;
        }
        self.pending.drain(..start);
    }

    /// A datagram sent at `now`: a whole request, or one that matches nothing.
    pub(crate) fn send(&mut self, now: i64, datagram: &[u8]) {
        if self.hung(now) {
            return;
        }
        let spec = Arc::clone(&self.spec);
        self.respond(now, spec.equal(datagram));
    }

    /// The bytes ready at `now` over TCP, in one slice.
    pub(crate) fn ready(&mut self, now: i64) -> &[u8] {
        let end = self.marks.iter().take_while(|(ready, _)| *ready <= now).last().map_or(self.taken, |(_, end)| *end);
        let count = usize::try_from(end.saturating_sub(self.taken)).unwrap_or(0);
        self.stream.make_contiguous().get(..count).unwrap_or_default()
    }

    /// Takes `count` bytes from the front of what is ready (TCP).
    pub(crate) fn take(&mut self, count: usize) -> Vec<u8> {
        let count = count.min(self.stream.len());
        let taken: Vec<u8> = self.stream.drain(..count).collect();
        self.taken = self.taken.saturating_add(count as u64);
        while self.marks.front().is_some_and(|(_, end)| *end <= self.taken) {
            self.marks.pop_front();
        }
        taken
    }

    /// The first datagram ready at `now`, taken (UDP).
    pub(crate) fn datagram(&mut self, now: i64) -> Option<Vec<u8>> {
        if !self.datagrams.front().is_some_and(|(ready, _)| *ready <= now) {
            return None;
        }
        let (_, datagram) = self.datagrams.pop_front()?;
        self.datagram_bytes = self.datagram_bytes.saturating_sub(datagram.len());
        Some(datagram)
    }

    /// When more becomes ready after `now`.
    pub(crate) fn next_ready(&self, now: i64) -> Option<i64> {
        let stream = self.marks.iter().map(|(ready, _)| *ready);
        stream.chain(self.datagrams.iter().map(|(ready, _)| *ready)).find(|ready| *ready > now)
    }
}
