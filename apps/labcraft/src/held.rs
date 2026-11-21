//! The editing commands of the agents that wait for the end of a gesture of the user (architecture 12.5, commands.md
//! 9.1): held by the thread of the session in their order, run when the gesture ends, `gesture.busy` after
//! [`labcraft_engine::gesture::WAIT`] from their arrival, dropped when their client cancels them.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use labcraft_engine::{Author, CommandError};
use serde_json::Value;

/// Where a call of an agent is, which the thread of the session and the connection that waits for it share: sent, held
/// for the end of a gesture, running (or answered), cancelled. Each change is one compare and swap, so that a call is
/// either cancelled before it runs, or runs and is answered, never both.
#[derive(Default)]
pub struct Flags(AtomicU8);

const SENT: u8 = 0;
const HELD: u8 = 1;
const RUNNING: u8 = 2;
const CANCELLED: u8 = 3;

impl Flags {
    fn change(&self, from: u8, to: u8) -> bool {
        self.0.compare_exchange(from, to, Ordering::SeqCst, Ordering::SeqCst).is_ok()
    }

    /// The call waits for the end of a gesture: the connection tells its client so (`waiting for the user`).
    pub fn held(&self) -> bool {
        self.0.load(Ordering::SeqCst) == HELD
    }

    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst) == CANCELLED
    }

    /// The client cancelled the call (mcp.md 4): true when it had not started, and never will.
    pub fn cancel(&self) -> bool {
        self.change(SENT, CANCELLED) || self.change(HELD, CANCELLED)
    }

    /// The session takes the call to run it, or to answer it: false when its client cancelled it first.
    pub fn start(&self) -> bool {
        self.change(SENT, RUNNING) || self.change(HELD, RUNNING)
    }

    fn hold(&self) -> bool {
        self.change(SENT, HELD)
    }
}

/// A call of an agent, as the thread of the session receives it.
pub struct Call {
    pub author: Author,
    pub id: String,
    pub params: Value,
    pub reply: Sender<Result<Value, CommandError>>,
    pub flags: Arc<Flags>,
}

/// The calls held, in the order they came: the oldest first.
pub struct Held {
    calls: VecDeque<Entry>,
    wait: Duration,
    /// A call was `gesture.busy` during the gesture in progress: the next ones are at once, until it ends.
    gave_up: bool,
}

/// A call held, and when it came.
struct Entry {
    call: Call,
    since: Instant,
}

impl Held {
    /// Calls that each wait at most `wait` (`gesture::WAIT`, shorter in the tests).
    pub fn new(wait: Duration) -> Held {
        Held { calls: VecDeque::new(), wait, gave_up: false }
    }

    /// Holds `call`, arrived at `now`; one cancelled already is dropped. Once a call of this gesture was `gesture.busy`,
    /// the next ones are at once, so that an agent never waits more than one wait for one gesture (decision of the
    /// owner, 2026-11-21).
    pub fn hold(&mut self, call: Call, now: Instant) {
        if self.gave_up {
            if call.flags.start() {
                let _ = call.reply.send(Err(labcraft_engine::gesture::still_busy(&call.id)));
            }
        } else if call.flags.hold() {
            self.calls.push_back(Entry { call, since: now });
        }
    }

    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    /// The gesture ended: the calls to run now, in their order, each still to be started.
    pub fn release(&mut self) -> Vec<Call> {
        self.gave_up = false;
        self.calls.drain(..).map(|entry| entry.call).collect()
    }

    /// Answers `gesture.busy` to each call that waited its time by `now`, and drops each one cancelled: whether one
    /// left.
    pub fn expire(&mut self, now: Instant) -> bool {
        let before = self.calls.len();
        let wait = self.wait;
        let mut gave_up = false;
        self.calls.retain(|Entry { call, since }| {
            if call.flags.cancelled() {
                return false;
            }
            if now.saturating_duration_since(*since) < wait {
                return true;
            }
            if call.flags.start() {
                let _ = call.reply.send(Err(labcraft_engine::gesture::busy(&call.id, wait)));
            }
            gave_up = true;
            false
        });
        self.gave_up |= gave_up;
        self.calls.len() != before
    }

    /// How long until the oldest call is due, from `now`: none when none is held.
    pub fn next(&self, now: Instant) -> Option<Duration> {
        self.calls.front().map(|entry| (entry.since + self.wait).saturating_duration_since(now))
    }

    /// The name of the agent whose call waits the longest, which the status bar shows.
    pub fn waiting(&self) -> Option<String> {
        self.calls.iter().find(|entry| !entry.call.flags.cancelled()).map(|entry| entry.call.author.name().to_owned())
    }
}

#[cfg(test)]
#[expect(clippy::disallowed_methods, reason = "the desktop app, never built for WASM, where the clock traps")]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn call(id: &str) -> (Call, mpsc::Receiver<Result<Value, CommandError>>) {
        let (reply, answer) = mpsc::channel();
        let call = Call { author: Author::Agent("claude".to_owned()), id: id.to_owned(), params: Value::Null, reply, flags: Arc::default() };
        (call, answer)
    }

    #[test]
    fn a_call_held_past_its_wait_is_busy_and_one_cancelled_is_dropped_unanswered() {
        let start = Instant::now();
        let mut held = Held::new(Duration::from_secs(30));
        let (first, first_answer) = call("node.add");
        let (second, second_answer) = call("wire.connect");
        let cancelled = second.flags.clone();
        held.hold(first, start);
        held.hold(second, start + Duration::from_secs(10));
        assert_eq!(held.next(start), Some(Duration::from_secs(30)));
        assert!(!held.expire(start + Duration::from_secs(29)));
        assert!(cancelled.cancel());
        assert!(!cancelled.start(), "a call cancelled never runs");
        assert!(held.expire(start + Duration::from_secs(30)));
        assert_eq!(first_answer.try_recv().ok().and_then(Result::err).map(|error| error.code), Some("gesture.busy"));
        assert!(second_answer.try_recv().is_err());
        assert!(held.is_empty());
        // The same gesture: the next call is busy at once; once it ended, a call waits again.
        let (third, third_answer) = call("node.add");
        held.hold(third, start + Duration::from_secs(31));
        assert_eq!(third_answer.try_recv().ok().and_then(Result::err).map(|error| error.code), Some("gesture.busy"));
        assert!(held.release().is_empty());
        let (fourth, _d) = call("node.add");
        held.hold(fourth, start + Duration::from_secs(32));
        assert!(!held.is_empty());
    }

    #[test]
    fn the_end_of_the_gesture_releases_the_calls_in_their_order() {
        let start = Instant::now();
        let mut held = Held::new(Duration::from_secs(30));
        let (first, _a) = call("node.add");
        let (second, _b) = call("wire.connect");
        let flags = first.flags.clone();
        held.hold(first, start);
        held.hold(second, start);
        assert!(flags.held());
        let released = held.release();
        assert_eq!(released.iter().map(|call| call.id.as_str()).collect::<Vec<_>>(), ["node.add", "wire.connect"]);
        assert!(flags.start());
        assert!(!flags.held());
        assert!(!flags.cancel(), "a call that runs is not cancelled");
        assert!(held.waiting().is_none());
    }
}
