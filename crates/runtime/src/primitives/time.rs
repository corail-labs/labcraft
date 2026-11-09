//! `time` (appendix A.2): the clock of the run, which the host keeps (cli.md 6.2).

use super::{Call, Host, Outcome, input, wrong};
use crate::fault::Fault;
use crate::value::Value;

pub fn now(_: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    Ok(Outcome::Done(vec![Value::Timestamp(host.now())]))
}

/// From `since` to now; beyond what an `i64` of nanoseconds holds, `LC8008`.
pub fn elapsed(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let Value::Timestamp(since) = *input(call, 0)? else { return Err(wrong("time.elapsed")) };
    let now = host.now();
    let elapsed = now
        .checked_sub(since)
        .ok_or_else(|| Fault::out_of_range("time.elapsed: the time since this timestamp is beyond what a duration holds (Q-25)"))?;
    Ok(Outcome::Done(vec![Value::Duration(elapsed)]))
}

/// The node sleeps on the clock of the run; a negative duration waits nothing, as in the proof of concept.
pub fn wait(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let Value::Duration(ns) = *input(call, 0)? else { return Err(wrong("time.wait")) };
    Ok(Outcome::Wait(ns.max(0)))
}
