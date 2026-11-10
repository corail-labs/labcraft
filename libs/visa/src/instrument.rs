//! A simulated instrument during a run (`libs/visa/simulation.md`, sections 3 and 4): its quantities, its error queue, and how
//! it handles one command. Only integer operations, conversions, and rounded additions, products and divisions take part, so
//! every platform gives the same answer.

use std::collections::VecDeque;

use labcraft_runtime::native::{CommandLog, LogBudget};

use crate::file::{Bench, InstrumentSpec, QUEUE_SIZE};

/// The generator of the noise of a quantity: SplitMix64 (simulation.md 3).
#[derive(Clone, Debug)]
pub(crate) struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub(crate) fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }

    pub(crate) fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Twelve draws, the top 53 bits of each added as integers, then `(S − 6 × 2^53) × 2^−53`: one conversion, rounded to
    /// nearest, then an exact product.
    pub(crate) fn factor(&mut self) -> f64 {
        let sum: i128 = (0..12).map(|_| i128::from(self.next() >> 11)).sum();
        let centred = sum - 6 * (1_i128 << 53);
        (centred as f64) * (1.0 / 9_007_199_254_740_992.0)
    }
}

/// The state of a quantity in one run: the move it is in, and its noise.
#[derive(Clone, Debug)]
pub(crate) struct QuantityState {
    from: f64,
    to: f64,
    start: i64,
    duration: i64,
    noise: f64,
    rng: SplitMix64,
    root: usize,
}

impl QuantityState {
    pub(crate) fn of(bench: &Bench) -> Vec<QuantityState> {
        bench
            .quantities
            .iter()
            .map(|spec| QuantityState {
                from: spec.initial,
                to: spec.value,
                start: spec.start,
                duration: spec.settle,
                noise: spec.noise,
                rng: SplitMix64::new(spec.seed),
                root: spec.root,
            })
            .collect()
    }

    /// Its own true value at `t` (simulation.md 3): `from` before its start, then the ramp, then `to`. The two differences of
    /// times are exact integers, each converted to an `f64`.
    fn own(&self, t: i64) -> f64 {
        if t < self.start {
            return self.from;
        }
        if self.duration <= 0 || t.saturating_sub(self.start) >= self.duration {
            return self.to;
        }
        let elapsed = (t - self.start) as f64;
        let duration = self.duration as f64;
        self.from + (self.to - self.from) * (elapsed / duration)
    }
}

/// The true value of a quantity at `t`: that of the end of its chain of sources.
fn true_value(quantities: &[QuantityState], id: usize, t: i64) -> f64 {
    let root = quantities.get(id).map_or(id, |quantity| quantity.root);
    quantities.get(root).map_or(0.0, |quantity| quantity.own(t))
}

/// A read: the true value plus the noise times a factor; every read draws, a noise of 0 included.
fn read(quantities: &mut [QuantityState], id: usize, t: i64) -> f64 {
    let base = true_value(quantities, id, t);
    let Some(quantity) = quantities.get_mut(id) else { return base };
    let factor = quantity.rng.factor();
    base + quantity.noise * factor
}

/// A write of `set` at `t` starts a new move, from the true value at `t` to the value written.
fn set(quantities: &mut [QuantityState], id: usize, target: f64, t: i64, settle: i64) {
    let now = true_value(quantities, id, t);
    if let Some(quantity) = quantities.get_mut(id) {
        *quantity = QuantityState { from: now, to: target, start: t, duration: settle, ..quantity.clone() };
    }
}

/// How many commands an instrument keeps for the report (commands.md 7) *(proposed)*.
pub(crate) const KEPT_COMMANDS: usize = 1000;

/// An instrument in one run.
#[derive(Debug)]
pub(crate) struct Instrument {
    queue: VecDeque<(i64, String)>,
    pub(crate) log: Vec<CommandLog>,
    pub(crate) more: u64,
}

impl Instrument {
    pub(crate) fn new(spec: &InstrumentSpec) -> Self {
        Instrument { queue: spec.error_queue.iter().cloned().collect(), log: Vec::new(), more: 0 }
    }

    /// One more error; past the size of the queue, the last becomes `-350,"Queue overflow"` (simulation.md 4).
    pub(crate) fn push_error(&mut self, code: i64, message: &str) {
        if self.queue.len() < QUEUE_SIZE {
            self.queue.push_back((code, message.to_owned()));
        } else if let Some(last) = self.queue.back_mut() {
            *last = (-350, "Queue overflow".to_owned());
        }
    }

    /// Each command kept costs its two texts, as kept, and 64 bytes, from the budget of the run (`LogBudget`): from the first
    /// command that does not fit, commands are counted, not kept, whatever the number of instruments, so that the log of an
    /// instrument is the start of what it received.
    fn log(&mut self, budget: &LogBudget, at: i64, sent: &str, answer: Option<&str>) {
        let cost = (sent.len().min(4 * KEPT_TEXT) + answer.map_or(0, |answer| answer.len().min(4 * KEPT_TEXT)) + 64) as u64;
        if self.more == 0 && self.log.len() < KEPT_COMMANDS && budget.take(cost) {
            let (sent, sent_length) = head(sent);
            let (answer, answer_length) = answer.map_or((None, 0), |answer| {
                let (text, length) = head(answer);
                (Some(text), length)
            });
            self.log.push(CommandLog { at, sent, sent_length, answer, answer_length });
        } else {
            self.more = self.more.saturating_add(1);
        }
    }
}

/// How much of a text the report shows (commands.md 5): what is kept of each command and answer.
pub(crate) const KEPT_TEXT: usize = 1000;

/// The first 1,000 code points of a text, and its length in code points.
fn head(text: &str) -> (String, u64) {
    let length = text.chars().count();
    (text.chars().take(KEPT_TEXT).collect(), u64::try_from(length).unwrap_or(u64::MAX))
}

/// The command without one `\n` or `\r\n` that ends it, then without spaces (U+0020) at its ends.
pub(crate) fn command_text(command: &str) -> &str {
    let command = command.strip_suffix("\r\n").or_else(|| command.strip_suffix('\n')).unwrap_or(command);
    command.trim_matches(' ')
}

/// A decimal number as SCPI writes it, in ASCII: a sign, digits with a fraction, an exponent (`5`, `-0.5`, `.5`, `1.2E3`).
fn is_scpi_number(text: &str) -> bool {
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(at) => (body.get(..at).unwrap_or_default(), Some(body.get(at + 1..).unwrap_or_default())),
        None => (body, None),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    let mantissa_ok = digits(whole) && digits(fraction) && !(whole.is_empty() && fraction.is_empty());
    let exponent_ok = exponent.is_none_or(|exponent| {
        let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        !exponent.is_empty() && digits(exponent)
    });
    mantissa_ok && exponent_ok
}

/// Handles a command at `t`, once its latency has passed (simulation.md 4), and gives its answer: `None` for a write, and
/// for a query never answered.
pub(crate) fn execute(
    spec: &InstrumentSpec,
    state: &mut Instrument,
    quantities: &mut [QuantityState],
    bench: &Bench,
    budget: &LogBudget,
    command: &str,
    t: i64,
) -> Option<String> {
    let c = command_text(command);
    if spec.timeouts.contains(c) {
        state.log(budget, t, c, None);
        return None;
    }
    if let Some((code, message)) = spec.error_on.get(c) {
        state.push_error(*code, message);
    }
    let answer = match c {
        "SYST:ERR?" => {
            let (code, message) = state.queue.pop_front().unwrap_or((0, "No error".to_owned()));
            Some(format!("{code},\"{}\"", message.replace('"', "\"\"")))
        }
        "*IDN?" => Some(spec.idn.clone()),
        "*CLS" => {
            state.queue.clear();
            None
        }
        "*RST" => None,
        _ => {
            if let Some(answer) = spec.commands.get(c) {
                Some(answer.clone())
            } else if let Some(id) = spec.measure.get(c) {
                Some(format!("{:.6}", read(quantities, *id, t)))
            } else {
                let (header, argument) = c.split_once(' ').unwrap_or((c, ""));
                if let Some(id) = spec.set.get(header).filter(|_| !c.ends_with('?')) {
                    let argument = argument.trim_start_matches(' ');
                    if !is_scpi_number(argument) {
                        state.push_error(-224, "Illegal parameter value");
                    } else {
                        match argument.parse::<f64>() {
                            Ok(x) if x.is_finite() && x.abs() <= 1e100 => {
                                let settle = bench.quantities.get(*id).map_or(0, |quantity| quantity.settle);
                                set(quantities, *id, x, t, settle);
                            }
                            _ => state.push_error(-222, "Data out of range"),
                        }
                    }
                } else if !c.ends_with('?') && spec.accept.iter().any(|prefix| c.starts_with(prefix.as_str())) {
                    // Accepted, and does nothing.
                } else {
                    state.push_error(-113, "Undefined header");
                }
                None
            }
        }
    };
    state.log(budget, t, c, answer.as_deref());
    answer
}
