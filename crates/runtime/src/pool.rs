//! The pool of threads that runs the work of native calls that blocks, in real time (`docs/architecture.md`, 10.4 and 10.6;
//! decision of the owner, 2026-11-11): one per session, its threads started when work comes and no thread is idle, at most
//! [`POOL_MAX`] at once *(proposed)*, each ending after [`POOL_IDLE`] without work. Work beyond that waits its turn, never
//! a fault. Work that a run abandons keeps its thread until it returns, and counts.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// The threads of a pool at most *(proposed)*.
pub const POOL_MAX: usize = 64;
/// How long a thread of a pool waits for work before it ends *(proposed)*.
pub const POOL_IDLE: Duration = Duration::from_secs(10);

/// What a thread of the pool runs: the runtime wraps the [`crate::native::Job`] of a call, which never unwinds out of it.
pub(crate) type Work = Box<dyn FnOnce() + Send>;

/// Threads for work that blocks, shared by the runs of a session, one after the other.
#[derive(Clone)]
pub struct Pool {
    shared: Arc<Shared>,
}

struct Shared {
    state: Mutex<State>,
    work: Condvar,
    max: usize,
    idle: Duration,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Work>,
    /// The threads alive, busy or idle.
    threads: usize,
    /// The threads that wait for work.
    idle: usize,
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.shared.lock();
        f.debug_struct("Pool").field("threads", &state.threads).field("idle", &state.idle).field("queued", &state.queue.len()).finish()
    }
}

impl Default for Pool {
    fn default() -> Self {
        Pool::new(POOL_MAX, POOL_IDLE)
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Pool {
    /// A pool of at most `max` threads (at least one), each ending after `idle` without work. No thread starts before work.
    pub fn new(max: usize, idle: Duration) -> Pool {
        Pool { shared: Arc::new(Shared { state: Mutex::default(), work: Condvar::new(), max: max.max(1), idle }) }
    }

    /// The threads alive, busy or idle.
    pub fn threads(&self) -> usize {
        self.shared.lock().threads
    }

    /// Gives work to a thread: an idle one, a new one while there are fewer than the maximum, or the first one free. Gives the
    /// work back when no thread exists and none can start, so that the caller does it itself rather than waiting forever.
    pub(crate) fn submit(&self, work: Work) -> Result<(), Work> {
        let mut state = self.shared.lock();
        state.queue.push_back(work);
        if state.queue.len() <= state.idle {
            drop(state);
            self.shared.work.notify_one();
            return Ok(());
        }
        if state.threads >= self.shared.max {
            return Ok(());
        }
        state.threads += 1;
        drop(state);
        let shared = Arc::clone(&self.shared);
        let started = std::thread::Builder::new().name("labcraft pool".to_owned()).spawn(move || worker(&shared));
        if started.is_ok() {
            return Ok(());
        }
        let mut state = self.shared.lock();
        state.threads -= 1;
        if state.threads == 0
            && let Some(work) = state.queue.pop_back()
        {
            return Err(work);
        }
        Ok(())
    }
}

fn worker(shared: &Shared) {
    let mut state = shared.lock();
    loop {
        if let Some(work) = state.queue.pop_front() {
            drop(state);
            work();
            state = shared.lock();
            continue;
        }
        state.idle += 1;
        let (back, waited) = shared.work.wait_timeout(state, shared.idle).unwrap_or_else(PoisonError::into_inner);
        state = back;
        state.idle -= 1;
        if waited.timed_out() && state.queue.is_empty() {
            state.threads -= 1;
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn work_that_blocks_does_not_wait_behind_other_work_until_the_maximum() {
        let pool = Pool::new(3, Duration::from_secs(5));
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let gate = Arc::new(Mutex::new(gate_rx));
        let (done_tx, done_rx) = mpsc::channel();
        // Two pieces of work that block until the gate opens, and a third that does not: it gets a thread of its own.
        for blocks in [true, true, false] {
            let (gate, done) = (Arc::clone(&gate), done_tx.clone());
            let work: Work = Box::new(move || {
                if blocks {
                    let _ = gate.lock().unwrap().recv();
                }
                done.send(blocks).unwrap();
            });
            assert!(pool.submit(work).is_ok());
        }
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(10)), Ok(false));
        assert_eq!(pool.threads(), 3);
        // A fourth waits for a thread, since there are three at most, and runs once one is free.
        let done = done_tx.clone();
        assert!(pool.submit(Box::new(move || done.send(false).unwrap())).is_ok());
        assert_eq!(pool.threads(), 3);
        gate_tx.send(()).unwrap();
        gate_tx.send(()).unwrap();
        let mut ended: Vec<bool> = (0..3).map(|_| done_rx.recv_timeout(Duration::from_secs(10)).unwrap()).collect();
        ended.sort();
        assert_eq!(ended, [false, true, true]);
    }

    #[test]
    fn an_idle_thread_ends() {
        let pool = Pool::new(2, Duration::from_millis(20));
        let (done_tx, done_rx) = mpsc::channel();
        assert!(pool.submit(Box::new(move || done_tx.send(()).unwrap())).is_ok());
        done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        // Bounded, not timed: the thread ends some time after 20 ms without work.
        for _ in 0..1000 {
            if pool.threads() == 0 {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the idle thread did not end");
    }
}
