//! On any simulation file, `simulation::read` and the judgement of `visa` end, with the codes they declare and well-formed
//! spans, the same each time; and on a file they accept, every instrument answers each command its section names, the calls
//! end, and two runs of the same file give the same answers and the same log (`libs/visa/simulation.md`).
//!
//! The reader of `toml` recurses, at most 80 levels deep (cli.md 6.2), so the main thread of libFuzzer is enough.
#![no_main]

use std::sync::Arc;

use labcraft_runtime::native::{CallContext, LOG_BUDGET, LibraryDiagnostic, LibraryRun, LogBudget, NativeLibrary, NativeOutcome, Step};
use labcraft_runtime::simulation::{self, read};
use labcraft_runtime::{Handle, Value};
use labcraft_types::Severity;
use labcraft_visa::{Bench, Visa};
use libfuzzer_sys::fuzz_target;

/// The default `--max-nesting` (cli.md 6.2).
const MAX_NESTING: u32 = 256;
/// The calls of one run of a file, so that a file of 64 KiB stays within the timeout of `run.sh`: past them, the run makes no
/// other call.
const MAX_CALLS: usize = 4000;
/// The steps of one call: a query takes the key of its session, then that of its instrument, waits its latency, gives the key
/// back, and ends; a timeout waits once more.
const MAX_STEPS: usize = 16;
const SECOND: i64 = 1_000_000_000;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = std::str::from_utf8(bytes) else { return };
    // The length of each line in code points, as columns are counted (cli.md 4.2).
    let lines: Vec<usize> = text.split('\n').map(|line| line.chars().count()).collect();
    let judgement = judged(text, &lines);
    assert_eq!(judgement, judged(text, &lines), "the judgement of a file depends only on the file");
    if judgement == Judged::Accepted(true) {
        assert_eq!(exercised(text), exercised(text), "two runs of the same file give the same answers and the same log");
    }
});

/// What the reader and `visa` said of a file: its diagnostics as text, since a diagnostic has no equality, or nothing.
#[derive(Debug, PartialEq)]
enum Judged {
    Refused(Vec<String>),
    /// Whether the file has a section for `visa`: a file without one is accepted, and `visa` then simulates nothing.
    Accepted(bool),
}

fn judged(text: &str, lines: &[usize]) -> Judged {
    let file = match read(text, MAX_NESTING) {
        Ok(file) => file,
        Err(diagnostic) => {
            assert!(simulation::CODES.contains(&diagnostic.code), "{diagnostic:?}");
            well_formed(&diagnostic, lines);
            return Judged::Refused(vec![format!("{diagnostic:?}")]);
        }
    };
    match file.judge(&[&Visa]) {
        Ok(simulations) => {
            assert_eq!(simulations.len(), 1);
            Judged::Accepted(simulations.first().is_some_and(Option::is_some))
        }
        Err(diagnostics) => {
            assert!(!diagnostics.is_empty());
            let codes: Vec<&str> = Visa.codes().iter().map(|code| code.code).chain(simulation::CODES.iter().copied()).collect();
            for diagnostic in &diagnostics {
                assert!(codes.contains(&diagnostic.code), "{diagnostic:?}");
                well_formed(diagnostic, lines);
            }
            Judged::Refused(diagnostics.iter().map(|diagnostic| format!("{diagnostic:?}")).collect())
        }
    }
}

/// An error, at a place of the file (cli.md 4.2): lines and columns from 1, in code points, within the file, and an end after
/// its start.
fn well_formed(diagnostic: &LibraryDiagnostic, lines: &[usize]) {
    assert_eq!(diagnostic.severity, Severity::Error, "{diagnostic:?}");
    assert!(!diagnostic.message.is_empty(), "{diagnostic:?}");
    if let Some(span) = diagnostic.span {
        assert!(span.start <= span.end, "{diagnostic:?}");
        for position in [span.start, span.end] {
            let width = usize::try_from(position.line).ok().and_then(|line| line.checked_sub(1)).and_then(|line| lines.get(line));
            let column = usize::try_from(position.column).unwrap_or(usize::MAX);
            assert!(width.is_some_and(|width| column >= 1 && column <= width + 1), "{diagnostic:?} in lines of {lines:?}");
        }
    }
}

/// One run of an accepted file: each instrument opened, every command its section names sent once and its answer compared
/// with the file where the file says it, the errors read, the session closed, then used once closed. On the first
/// instrument, a second session also leaves an answer unread (`-410`), reads, cancels a call and is released. Gives each
/// outcome and the log of the instruments, as text.
fn exercised(text: &str) -> Vec<String> {
    let Ok(file) = read(text, MAX_NESTING) else { panic!("read once, refused now") };
    let Ok(simulations) = file.judge(&[&Visa]) else { panic!("accepted once, refused now") };
    let Some(simulation) = simulations.first().and_then(Option::as_ref) else { panic!("no simulation for visa") };
    let Some(bench) = simulation.downcast_ref::<Arc<Bench>>() else { panic!("visa gives a bench") };
    let mut run = Visa.start_run(0, Some(simulation), LogBudget::new(LOG_BUDGET));
    let mut driver = Driver { run: &mut run, now: 0, calls: 0, said: Vec::new() };
    for (index, instrument) in bench.commands().into_iter().enumerate() {
        let Some(session) = driver.open(&instrument.resource) else { break };
        let hangs = |command: &str| instrument.timeout.iter().any(|timeout| timeout == command);
        for header in &instrument.set {
            for argument in ["2.5", "-1E300", "x"] {
                driver.send(session, &format!("{header} {argument}"));
            }
        }
        for (query, answer) in &instrument.commands {
            let got = driver.send(session, query);
            if got.is_some() && !hangs(query) {
                assert_eq!(answer_of(&got), Some(answer.as_str()), "{}: {query}", instrument.resource);
            }
        }
        for query in &instrument.measure {
            let got = driver.send(session, query);
            if got.is_some() && !hangs(query) {
                let number = answer_of(&got).map(str::parse::<f64>);
                assert!(matches!(number, Some(Ok(x)) if x.is_finite()), "{}: {query} gives {got:?}", instrument.resource);
            }
        }
        for prefix in &instrument.accept {
            driver.send(session, &format!("{prefix} 1"));
        }
        for command in &instrument.timeout {
            let got = driver.send(session, command);
            if got.is_some() && command.ends_with('?') {
                assert!(matches!(got, Some(NativeOutcome::Fail { category: "io.timeout", .. })), "{}: {command} gives {got:?}", instrument.resource);
            }
        }
        for command in instrument.error_on.iter().map(String::as_str).chain(["UNKNOWN:HEADER", "SYST:ERR?", "SYST:ERR?", "*CLS", "SYST:ERR?", "*RST"])
        {
            driver.send(session, command);
        }
        let idn = driver.send(session, "*IDN?");
        if idn.is_some() && !hangs("*IDN?") {
            assert_eq!(answer_of(&idn), Some(instrument.idn.as_str()), "{}", instrument.resource);
        }
        if index == 0 {
            if let Some(second) = driver.open(&instrument.resource) {
                for command in ["*IDN?", "*RST"] {
                    driver.call("write", vec![Value::Handle(second), Value::string(command), Value::NONE]);
                }
                driver.call("read", vec![Value::Handle(second), Value::NONE]);
                driver.call("write", vec![Value::Handle(second), Value::string("*IDN?"), Value::NONE]);
                driver.call("read", vec![Value::Handle(second), Value::NONE]);
                driver.cancelled("query", vec![Value::Handle(second), Value::string("SYST:ERR?"), Value::NONE]);
                driver.send(second, "SYST:ERR?");
                driver.run.release(second);
                let info = driver.run.describe(second);
                assert!(info.as_ref().is_some_and(|info| !info.open), "released: {info:?}");
            }
        }
        if driver.call("close", vec![Value::Handle(session), Value::NONE]).is_none() {
            break;
        }
        let info = driver.run.describe(session);
        assert!(info.as_ref().is_some_and(|info| !info.open), "{info:?}");
        let closed = driver.send(session, "*IDN?");
        assert!(closed.is_none() || matches!(closed, Some(NativeOutcome::Fail { category: "visa.invalid_session", .. })), "{closed:?}");
    }
    let instruments = driver.run.instruments();
    assert!(instruments.windows(2).all(|pair| pair[0].resource < pair[1].resource), "in code point order of their resources");
    for instrument in &instruments {
        assert!(instrument.commands.len() <= 1000, "{}", instrument.resource);
        assert!(instrument.commands.windows(2).all(|pair| pair[0].at <= pair[1].at), "{}: in the order of the clock", instrument.resource);
        for command in &instrument.commands {
            let counted = |text: &str| u64::try_from(text.chars().count()).unwrap_or(u64::MAX);
            assert!(counted(&command.sent) <= 1000 && counted(&command.sent) <= command.sent_length, "{command:?}");
            assert!(
                command.answer.as_deref().is_none_or(|answer| counted(answer) <= 1000 && counted(answer) <= command.answer_length),
                "{command:?}"
            );
        }
    }
    let mut said = driver.said;
    said.push(format!("{instruments:?}"));
    said
}

/// The text of a query that answered.
fn answer_of(outcome: &Option<NativeOutcome>) -> Option<&str> {
    match outcome {
        Some(NativeOutcome::Outputs(outputs)) => match outputs.first() {
            Some(Value::String(text)) => Some(text),
            _ => None,
        },
        _ => None,
    }
}

/// What the scheduler of a run does for calls made one at a time: no other call holds a key, so each is given at once.
struct Driver<'r> {
    run: &'r mut Box<dyn LibraryRun>,
    now: i64,
    calls: usize,
    said: Vec<String>,
}

impl Driver<'_> {
    /// A session on `resource`, which the file has; `None` once the run has made its calls.
    fn open(&mut self, resource: &str) -> Option<Handle> {
        match self.call("open", vec![Value::string(resource), Value::Duration(SECOND), Value::NONE])? {
            NativeOutcome::Outputs(outputs) => match outputs.first() {
                Some(Value::Handle(session)) => {
                    let info = self.run.describe(*session);
                    assert!(info.as_ref().is_some_and(|info| info.designates == resource && info.open), "{info:?}");
                    Some(*session)
                }
                other => panic!("open gives a session, not {other:?}"),
            },
            other => panic!("{resource} does not open: {other:?}"),
        }
    }

    /// A query when the command ends with `?`, else a write.
    fn send(&mut self, session: Handle, command: &str) -> Option<NativeOutcome> {
        let function = if command.ends_with('?') { "query" } else { "write" };
        self.call(function, vec![Value::Handle(session), Value::string(command), Value::NONE])
    }

    /// A call that the run abandons after its first step.
    fn cancelled(&mut self, function: &str, inputs: Vec<Value>) {
        if self.calls == MAX_CALLS {
            return;
        }
        self.calls += 1;
        let Some(call) = self.run.call(function, inputs) else { panic!("visa has {function}") };
        let step = self.run.step(call, CallContext { now: self.now });
        self.said.push(format!("{} {function} cancelled after {step:?}", self.now));
        self.run.cancel(call);
    }

    /// `None` once the run has made its calls.
    fn call(&mut self, function: &str, inputs: Vec<Value>) -> Option<NativeOutcome> {
        if self.calls == MAX_CALLS {
            return None;
        }
        self.calls += 1;
        let Some(call) = self.run.call(function, inputs) else { panic!("visa has {function}") };
        for _ in 0..MAX_STEPS {
            match self.run.step(call, CallContext { now: self.now }) {
                Step::Sleep(ns) => {
                    assert!(ns >= 0, "{function} sleeps {ns} ns");
                    let Some(now) = self.now.checked_add(ns) else { panic!("the clock of the run overflows") };
                    self.now = now;
                }
                Step::Exclusive(_) | Step::Release(_) => {}
                Step::Done(outcome) => {
                    if let NativeOutcome::Fail { category, message, .. } = &outcome {
                        assert!(!category.is_empty() && !message.is_empty(), "{outcome:?}");
                    }
                    self.said.push(format!("{} {function} {outcome:?}", self.now));
                    return Some(outcome);
                }
            }
        }
        panic!("{function} does not end in {MAX_STEPS} steps")
    }
}
