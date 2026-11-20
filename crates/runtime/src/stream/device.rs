//! A simulated device of a stream during a run (`docs/io.md`, section 6), seen from one connection or one port, on the clock
//! of the run: the bytes it received and has not matched yet, and the responses it gave, each ready `latency_ms` after the
//! write that asked for it. And one step of a write or a read on it, which the call of the library repeats until it is done.

use std::collections::VecDeque;

use super::{Failure, Got, Need, Scan, Taken};
use crate::native::{Described, bytes_text};

/// What a file says of a device, judged: its endpoint, its exchanges, its answer to what matches nothing, and the times of
/// its faults that every stream has. A library keeps what is its own beside it.
#[derive(Debug, Default)]
pub struct DeviceSpec {
    /// As a program reaches it (io.md 2): `tcp:10.0.0.7:502`, `serial:COM3`.
    pub endpoint: String,
    /// Nanoseconds on the clock of the run.
    pub latency: i64,
    /// In the order of the file.
    pub replies: Vec<Reply>,
    /// The indices of `replies` in the order of their requests, for a binary search.
    pub order: Vec<usize>,
    pub unknown: Option<Vec<u8>>,
    /// From that time on, nanoseconds on the clock of the run, the device takes no request and gives no response.
    pub hang: Option<i64>,
    /// When the device ends the stream (`close_ms`): what was ready is still read, and nothing more.
    pub close: Option<i64>,
}

#[derive(Debug)]
pub struct Reply {
    pub request: Vec<u8>,
    /// None: the request is taken and nothing answers.
    pub response: Option<Vec<u8>>,
}

impl DeviceSpec {
    /// The request at a place of the order of the bytes.
    fn sorted(&self, at: usize) -> Option<&Reply> {
        self.replies.get(*self.order.get(at)?)
    }

    /// The exchange whose request starts `bytes`. No request is the start of another (S007), so the only one that can is the
    /// greatest request not above `bytes` in the order of the bytes: one between it and `bytes` would start with it.
    fn starting(&self, bytes: &[u8]) -> Option<&Reply> {
        let after = self.order.partition_point(|index| self.replies.get(*index).is_some_and(|reply| reply.request.as_slice() <= bytes));
        self.sorted(after.checked_sub(1)?).filter(|reply| bytes.starts_with(&reply.request))
    }

    /// Whether `bytes` is the start of some request: then of the least request not below it, as above.
    fn waits_for_more(&self, bytes: &[u8]) -> bool {
        let at = self.order.partition_point(|index| self.replies.get(*index).is_some_and(|reply| reply.request.as_slice() < bytes));
        self.sorted(at).is_some_and(|reply| reply.request.starts_with(bytes))
    }

    /// Whether the device has ended the stream at `now` (`close_ms`).
    fn closed(&self, now: i64) -> bool {
        self.close.is_some_and(|close| now >= close)
    }

    /// Whether the device hangs at `now` (`hang_ms`).
    pub fn hung(&self, now: i64) -> bool {
        self.hang.is_some_and(|hang| now >= hang)
    }

    /// The response to `reply` asked for at `now`, or to what matches nothing (`unknown`), with when it is ready, after the
    /// latency; none when the device hangs or ends the stream first.
    pub fn response<'s>(&'s self, now: i64, reply: Option<&'s Reply>) -> Option<(i64, &'s [u8])> {
        let response = match reply {
            Some(reply) => reply.response.as_deref(),
            None => self.unknown.as_deref(),
        }?;
        let ready = now.saturating_add(self.latency);
        if self.hang.is_some_and(|hang| ready >= hang) || self.close.is_some_and(|close| ready >= close) {
            return None;
        }
        Some((ready, response))
    }

    /// What a description of the file says of the device, for `simulation.describe`, in this order: its endpoint, its
    /// latency, its exchanges in their order, and its answer to what matches nothing; bytes as text when they are text, else
    /// in hexadecimal (io.md 6). The library adds what is its own after them.
    pub fn described(&self) -> Vec<(String, Described)> {
        let bytes = |key: &str, bytes: &[u8]| {
            let (text, hex) = bytes_text(bytes);
            (if hex { format!("{key}_hex") } else { key.to_owned() }, Described::Text(text))
        };
        let replies = self
            .replies
            .iter()
            .map(|reply| {
                let mut fields = vec![bytes("request", &reply.request)];
                if let Some(response) = &reply.response {
                    fields.push(bytes("response", response));
                }
                Described::Map(fields)
            })
            .collect();
        let unknown = match &self.unknown {
            Some(unknown) => Described::Map(vec![bytes("response", unknown)]),
            None => Described::Null,
        };
        vec![
            ("endpoint".to_owned(), Described::Text(self.endpoint.clone())),
            ("latency_ms".to_owned(), Described::Number(self.latency as f64 / 1e6)),
            ("replies".to_owned(), Described::List(replies)),
            ("unknown".to_owned(), unknown),
        ]
    }
}

/// A time of a description, in milliseconds, or null.
pub fn described_time(ns: Option<i64>) -> Described {
    ns.map_or(Described::Null, |ns| Described::Number(ns as f64 / 1e6))
}

/// What a device keeps of its answers not read yet, per connection or port (decision of the owner, 2026-11-19): a response
/// that would pass it is dropped, as by a device whose buffer overflows.
pub const MAX_WAITING: usize = 16 * 1024 * 1024;

/// A device of a stream, seen from one connection or one port.
#[derive(Debug, Default)]
pub struct Device {
    /// What it received and has not matched yet.
    pending: Vec<u8>,
    /// What it answered, ready or not, in order.
    stream: VecDeque<u8>,
    /// When each answer is ready, with how many bytes of `stream` it ends at, counted from the first byte ever answered. Both
    /// only grow: the latency of a device is one, and its clock only goes forward.
    marks: VecDeque<(i64, u64)>,
    /// Bytes of `stream` taken since the first.
    taken: u64,
}

impl Device {
    /// The response asked for at `now`, unless it would pass [`MAX_WAITING`].
    fn respond(&mut self, spec: &DeviceSpec, now: i64, reply: Option<&Reply>) {
        let Some((ready, response)) = spec.response(now, reply) else { return };
        if self.stream.len().saturating_add(response.len()) > MAX_WAITING {
            return;
        }
        self.stream.extend(response);
        let end = self.taken.saturating_add(self.stream.len() as u64);
        self.marks.push_back((ready, end));
    }

    /// A write at `now` (io.md 6): the bytes join those not matched; while they start with a request, it is taken from the
    /// front and its response asked for; when what is left is the start of some request, the device waits for more;
    /// otherwise it forgets it and answers `unknown`, if it has one. A device that hangs takes nothing.
    fn write(&mut self, spec: &DeviceSpec, now: i64, data: &[u8]) {
        if spec.hung(now) {
            return;
        }
        self.pending.extend_from_slice(data);
        let mut start = 0;
        while let Some(rest) = self.pending.get(start..).filter(|rest| !rest.is_empty()) {
            if let Some(reply) = spec.starting(rest) {
                start += reply.request.len();
                self.respond(spec, now, Some(reply));
                continue;
            }
            if !spec.waits_for_more(rest) {
                start = self.pending.len();
                self.respond(spec, now, None);
            }
            break;
        }
        self.pending.drain(..start);
    }

    /// The marks ready at `now`: those before this place.
    fn ready_marks(&self, now: i64) -> usize {
        self.marks.partition_point(|(ready, _)| *ready <= now)
    }

    /// How many bytes are ready at `now`.
    pub fn ready_len(&self, now: i64) -> usize {
        let end = self.ready_marks(now).checked_sub(1).and_then(|last| self.marks.get(last)).map_or(self.taken, |(_, end)| *end);
        usize::try_from(end.saturating_sub(self.taken)).unwrap_or(0)
    }

    /// The bytes ready at `now`, in one slice.
    fn ready(&mut self, now: i64) -> &[u8] {
        let count = self.ready_len(now);
        self.stream.make_contiguous().get(..count).unwrap_or_default()
    }

    /// Takes `count` bytes from the front of what is ready.
    pub fn take(&mut self, count: usize) -> Vec<u8> {
        let count = count.min(self.stream.len());
        let taken: Vec<u8> = self.stream.drain(..count).collect();
        self.taken = self.taken.saturating_add(count as u64);
        while self.marks.front().is_some_and(|(_, end)| *end <= self.taken) {
            self.marks.pop_front();
        }
        taken
    }

    /// When more becomes ready after `now`.
    fn next_ready(&self, now: i64) -> Option<i64> {
        self.marks.get(self.ready_marks(now)).map(|(ready, _)| *ready)
    }
}

/// What a call on a simulated device does next.
#[derive(Debug)]
pub enum Next {
    Wrote,
    Read(Taken),
    Failed(Failure),
    Sleep(i64),
}

/// A write on a simulated device at `now`: `io.closed` once it has ended the stream (`close_ms`).
pub fn write_step(device: &mut Device, spec: &DeviceSpec, label: &str, now: i64, data: &[u8]) -> Next {
    if spec.closed(now) {
        return Next::Failed(Failure::closed(label));
    }
    device.write(spec, now, data);
    Next::Wrote
}

/// When a step of a read is, on the clock of the run, in nanoseconds.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub now: i64,
    /// When its timeout ends, counted from the start of the call.
    pub deadline: i64,
    /// The timeout of the connection or the port, for the message of `io.timeout`.
    pub timeout: i64,
}

/// One step of a read on a simulated device at `now`, which started so that its timeout ends at `deadline`: what it takes,
/// or `io.timeout` at the deadline, taking nothing, or `io.closed` once the device has ended the stream, or when to look
/// again.
pub fn read_step(device: &mut Device, spec: &DeviceSpec, need: &Need, scan: &mut Scan, label: &str, clock: Clock) -> Next {
    let Clock { now, deadline, timeout } = clock;
    let closed = spec.closed(now);
    let ready = device.ready(now);
    let nothing = ready.is_empty();
    match need.judge(ready, scan) {
        // Decision of the owner (2026-11-19): nothing waits and the other end has closed.
        Got::Take(0) if matches!(need, Need::Available(_)) && closed && nothing => Next::Failed(Failure::closed(label)),
        Got::Take(count) => Next::Read(Taken::Bytes(device.take(count))),
        Got::TooLong(count) => Next::Read(Taken::TooLong(device.take(count))),
        Got::More if closed => Next::Failed(Failure::closed(label)),
        Got::More if now >= deadline => Next::Failed(Failure::timed_out(label, &need.wanted(), timeout)),
        Got::More => {
            let mut wake = deadline;
            if let Some(ready) = device.next_ready(now) {
                wake = wake.min(ready);
            }
            if let Some(close) = spec.close.filter(|close| *close > now) {
                wake = wake.min(close);
            }
            Next::Sleep(wake.saturating_sub(now))
        }
    }
}
