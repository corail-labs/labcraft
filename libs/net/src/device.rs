//! A simulated device during a run (`docs/io.md`, section 6), seen from one connection or socket, on the clock of the run.
//! Over TCP it is the device of a stream that `runtime::stream` gives every library of raw streams, so that `net` and
//! `serial` answer and read the same; over UDP, what it answered is datagrams, each with when it is ready.

use std::collections::VecDeque;
use std::sync::Arc;

use labcraft_runtime::stream::{Device, MAX_WAITING, Reply};

use crate::file::DeviceSpec;

/// The exchange whose request is `bytes`, a datagram: the least request not below it in the order of the bytes, if it is that.
fn equal<'s>(spec: &'s labcraft_runtime::stream::DeviceSpec, bytes: &[u8]) -> Option<&'s Reply> {
    let at = spec.order.partition_point(|index| spec.replies.get(*index).is_some_and(|reply| reply.request.as_slice() < bytes));
    spec.replies.get(*spec.order.get(at)?).filter(|reply| reply.request == bytes)
}

/// A device seen from one connection (TCP) or one socket (UDP).
#[derive(Debug)]
pub(crate) struct Simulated {
    pub(crate) spec: Arc<DeviceSpec>,
    /// What it received and answered over TCP.
    pub(crate) stream: Device,
    /// What it answered over UDP, each datagram with when it is ready.
    datagrams: VecDeque<(i64, Vec<u8>)>,
    /// The bytes of `datagrams`.
    datagram_bytes: usize,
}

impl Simulated {
    pub(crate) fn new(spec: Arc<DeviceSpec>) -> Self {
        Simulated { spec, stream: Device::default(), datagrams: VecDeque::new(), datagram_bytes: 0 }
    }

    /// A datagram sent at `now`: a whole request, or one that matches nothing. Its response is never given when the device
    /// hangs first, or when it would pass [`MAX_WAITING`].
    pub(crate) fn send(&mut self, now: i64, datagram: &[u8]) {
        let spec = &self.spec.exchanges;
        if spec.hung(now) {
            return;
        }
        let Some((ready, response)) = spec.response(now, equal(spec, datagram)) else { return };
        if self.datagram_bytes.saturating_add(response.len()) > MAX_WAITING {
            return;
        }
        self.datagram_bytes += response.len();
        self.datagrams.push_back((ready, response.to_vec()));
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

    /// When the next datagram becomes ready after `now`.
    pub(crate) fn next_datagram(&self, now: i64) -> Option<i64> {
        self.datagrams.iter().map(|(ready, _)| *ready).find(|ready| *ready > now)
    }
}
