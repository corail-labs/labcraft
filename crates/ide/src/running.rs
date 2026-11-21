//! The run the window started (decisions of the owner, 2026-11-16): `run.start` with no wait, then `run.status` every
//! quarter of a second until the run ends, `run.abort` when the user stops it. The window never waits on an answer: it
//! reads it at a later frame. `run.start` and `run.status` answer at once; `run.abort` answers when the run has stopped,
//! and the commands sent meanwhile wait behind it on the session. The values of the run reach the canvas as those of any
//! run, by its trace.

use serde_json::{Value, json};

use crate::host::{Host, Reply};

/// How often the window asks how the run goes.
pub const EVERY: f64 = 0.25;

/// A run that the window follows.
pub struct Running {
    pub function: String,
    /// The id of the run, once the session gave it.
    run: Option<String>,
    /// The command asked and not answered yet: `run.start`, `run.status` or `run.abort`.
    asked: Option<Reply>,
    /// When to ask again.
    next: f64,
    /// Stop was asked; `run.abort` goes as soon as nothing else is asked.
    stop: bool,
    /// When the run started, on the clock of egui.
    pub started: f64,
    /// Whether it writes its trace (`Record values`).
    record: bool,
}

/// What the window learns from the session at a frame.
#[derive(Debug, PartialEq)]
pub enum Heard {
    /// Nothing new: the run goes on.
    Going,
    /// The run ended: how it went.
    Ended(Outcome),
    /// A command of the run failed: its message.
    Failed(String),
}

/// How a run the window started ended, for the `Run` panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// In a few words, then its outputs, one a line.
    pub said: String,
    /// It ran (it was not refused) and wrote no trace, since `Record values` was off: a trace on the disk is of an
    /// earlier run.
    pub unrecorded: bool,
}

impl Running {
    /// Starts a run of `function`; `record`, whether it writes its trace (its values on the wires).
    pub fn start(host: &dyn Host, function: &str, now: f64, record: bool) -> Running {
        let trace = if record { "file" } else { "none" };
        let asked = host.send("run.start", json!({ "function": function, "wait": "0 s", "trace": trace }));
        Running { function: function.to_owned(), run: None, asked: Some(asked), next: now, stop: false, started: now, record }
    }

    /// Asks to stop the run.
    pub fn stop(&mut self) {
        self.stop = true;
    }

    /// Whether stop was asked.
    pub fn stopping(&self) -> bool {
        self.stop
    }

    /// Takes the answer of the session, and asks the next thing.
    pub fn follow(&mut self, host: &dyn Host, now: f64) -> Heard {
        if let Some(reply) = &self.asked {
            let Some(result) = reply.take() else { return Heard::Going };
            self.asked = None;
            let report = match result {
                Ok(report) => report,
                Err(error) => return Heard::Failed(error.message),
            };
            let status = report.get("status").and_then(Value::as_str);
            if status != Some("running") {
                let ran = status.is_some_and(|status| status != "refused");
                return Heard::Ended(Outcome { said: outcome(&report, &self.function), unrecorded: ran && !self.record });
            }
            match report.get("run").and_then(Value::as_str) {
                Some(run) => self.run = Some(run.to_owned()),
                // Without its id, the run could be neither followed nor stopped.
                None if self.run.is_none() => return Heard::Failed("the session gave no id for the run".to_owned()),
                None => {}
            }
            self.next = now + EVERY;
        }
        let Some(run) = &self.run else { return Heard::Going };
        if self.stop {
            self.stop = false;
            self.asked = Some(host.send("run.abort", json!({ "run": run })));
        } else if now >= self.next {
            self.asked = Some(host.send("run.status", json!({ "run": run })));
        }
        Heard::Going
    }
}

/// How a run of `function` ended, in a few words, from its report (commands.md 7).
pub fn outcome(report: &Value, function: &str) -> String {
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
    let took = report
        .get("clock")
        .and_then(|clock| clock.get("real_ms"))
        .and_then(Value::as_f64)
        .map(|ms| if ms < 1_000.0 { format!(" in {ms:.0} ms") } else { format!(" in {:.1} s", ms / 1_000.0) })
        .unwrap_or_default();
    let said = match report.get("status").and_then(Value::as_str).unwrap_or_default() {
        "ok" => format!("Ended{took}"),
        "aborted" => format!("Stopped{took}"),
        "error" => {
            let error = report.get("error").map(|error| text(error, "message")).filter(|message| !message.is_empty());
            format!("Ended with an error{took}{}", error.map(|message| format!(": {message}")).unwrap_or_default())
        }
        "fault" => {
            let fault = report.get("fault").cloned().unwrap_or_default();
            format!("Fault{took}: {} {}", text(&fault, "code"), text(&fault, "message")).trim_end().to_owned()
        }
        "refused" => {
            // The diagnostics also hold the warnings, and the errors elsewhere in the folder, which refuse nothing
            // (commands.md 7): an error of the function run first, then any error, then whatever there is.
            let diagnostics = report.pointer("/check/diagnostics").and_then(Value::as_array).cloned().unwrap_or_default();
            let error = |diagnostic: &&Value| diagnostic.get("severity").and_then(Value::as_str) == Some("error");
            let first = diagnostics
                .iter()
                .find(|diagnostic| error(diagnostic) && diagnostic.get("function").and_then(Value::as_str) == Some(function))
                .or_else(|| diagnostics.iter().find(error))
                .or(diagnostics.first())
                .cloned()
                .unwrap_or_default();
            let said = format!("{} {}", text(&first, "code"), text(&first, "message"));
            if said.trim().is_empty() { "Not run".to_owned() } else { format!("Not run: {}", said.trim()) }
        }
        other => format!("Ended: {other}"),
    };
    // Its outputs, one a line, each cut to a line that the panel holds.
    let outputs = report.get("outputs").and_then(Value::as_array).into_iter().flatten().map(|output| {
        let value = output.get("value").map(|value| match value {
            Value::String(text) => format!("{text:?}"),
            other => other.to_string(),
        });
        let mut line = format!("{} = {}", text(output, "name"), value.unwrap_or_default());
        if line.chars().count() > 120 {
            line = line.chars().take(119).collect::<String>() + "…";
        }
        line
    });
    std::iter::once(said).chain(outputs).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use labcraft_engine::CommandError;
    use labcraft_engine::view::Published;

    use super::*;
    use crate::host::Snapshot;

    /// A session that answers each command with the next of its answers, and keeps what it was asked.
    struct Scripted {
        answers: RefCell<Vec<Result<Value, CommandError>>>,
        asked: RefCell<Vec<(String, Value)>>,
    }

    impl Scripted {
        fn new(answers: Vec<Result<Value, CommandError>>) -> Scripted {
            Scripted { answers: RefCell::new(answers), asked: RefCell::new(Vec::new()) }
        }

        fn ids(&self) -> Vec<String> {
            self.asked.borrow().iter().map(|(id, _)| id.clone()).collect()
        }
    }

    impl Host for Scripted {
        fn send(&self, id: &str, params: Value) -> Reply {
            self.asked.borrow_mut().push((id.to_owned(), params));
            let mut answers = self.answers.borrow_mut();
            let answer = if answers.is_empty() { Err(CommandError::internal("no answer left")) } else { answers.remove(0) };
            Reply::ready(answer)
        }

        fn snapshot(&self) -> Snapshot {
            Snapshot { view: std::sync::Arc::new(Published::default()), ..Snapshot::default() }
        }

        fn set_zoom(&self, _function: &str, _zoom: f32) {}

        fn set_user_settings(&self, _keys: &serde_json::Map<String, Value>) {}

        fn set_gesture(&self, _gesture: Option<labcraft_engine::Gesture>) {}
    }

    fn going(run: &str) -> Result<Value, CommandError> {
        Ok(json!({ "status": "running", "run": run }))
    }

    #[test]
    fn a_run_is_started_with_no_wait_then_followed_to_its_end() {
        let host = Scripted::new(vec![going("r1"), going("r1"), Ok(json!({ "status": "ok", "clock": { "real_ms": 12.0 } }))]);
        let mut running = Running::start(&host, "main", 0.0, false);
        assert_eq!(host.asked.borrow()[0].1, json!({ "function": "main", "wait": "0 s", "trace": "none" }));
        // The answer of `run.start`: the run goes on, asked again after a quarter of a second, not before.
        assert_eq!(running.follow(&host, 0.0), Heard::Going);
        assert_eq!(host.ids(), ["run.start"]);
        assert_eq!(running.follow(&host, 0.3), Heard::Going);
        assert_eq!(host.ids(), ["run.start", "run.status"]);
        assert_eq!(host.asked.borrow()[1].1, json!({ "run": "r1" }));
        assert_eq!(running.follow(&host, 0.4), Heard::Going);
        assert_eq!(running.follow(&host, 0.6), Heard::Going);
        assert_eq!(host.ids().len(), 2);
        // Asked at 0.7, its answer read at the frame after.
        assert_eq!(running.follow(&host, 0.7), Heard::Going);
        assert_eq!(running.follow(&host, 0.8), Heard::Ended(Outcome { said: "Ended in 12 ms".to_owned(), unrecorded: true }));
    }

    #[test]
    fn stop_aborts_the_run_once_its_id_is_known() {
        let host = Scripted::new(vec![going("r2"), Ok(json!({ "status": "aborted", "clock": { "real_ms": 2500.0 } }))]);
        let mut running = Running::start(&host, "main", 0.0, true);
        assert_eq!(host.asked.borrow()[0].1["trace"], "file");
        running.stop();
        // `run.start` answers first; then `run.abort`, at once, not after the quarter of a second.
        assert_eq!(running.follow(&host, 0.0), Heard::Going);
        assert_eq!(host.ids(), ["run.start", "run.abort"]);
        assert!(!running.stopping());
        assert_eq!(running.follow(&host, 0.0), Heard::Ended(Outcome { said: "Stopped in 2.5 s".to_owned(), unrecorded: false }));
    }

    #[test]
    fn a_run_that_ends_within_the_start_or_fails_says_so() {
        let host = Scripted::new(vec![Ok(json!({ "status": "ok" }))]);
        assert_eq!(
            Running::start(&host, "main", 0.0, false).follow(&host, 0.0),
            Heard::Ended(Outcome { said: "Ended".to_owned(), unrecorded: true })
        );
        // Refused, it did not run: no trace is not for want of `Record values`.
        let host = Scripted::new(vec![Ok(json!({ "status": "refused" }))]);
        let refused = Outcome { said: "Not run".to_owned(), unrecorded: false };
        assert_eq!(Running::start(&host, "main", 0.0, false).follow(&host, 0.0), Heard::Ended(refused));
        // Going, but with no id to follow it or stop it by.
        let host = Scripted::new(vec![Ok(json!({ "status": "running" }))]);
        assert!(matches!(Running::start(&host, "main", 0.0, false).follow(&host, 0.0), Heard::Failed(message) if message.contains("no id")));
        let host = Scripted::new(vec![Err(CommandError::internal("boom"))]);
        assert!(matches!(Running::start(&host, "main", 0.0, false).follow(&host, 0.0), Heard::Failed(message) if message.contains("boom")));
    }

    #[test]
    fn each_status_has_its_words() {
        let refused = json!({ "status": "refused", "check": { "diagnostics": [{ "code": "LC2001", "message": "unknown name" }] } });
        assert_eq!(outcome(&refused, "main"), "Not run: LC2001 unknown name");
        assert_eq!(outcome(&json!({ "status": "refused" }), "main"), "Not run");
        // The error of the function run, not a warning nor an error of another function, though they come first.
        let mixed = json!({ "status": "refused", "check": { "diagnostics": [
            { "severity": "warning", "code": "LC5001", "message": "unused", "function": "main" },
            { "severity": "error", "code": "LC2001", "message": "elsewhere", "function": "other" },
            { "severity": "error", "code": "LC8103", "message": "x has no value", "function": "main" },
        ] } });
        assert_eq!(outcome(&mixed, "main"), "Not run: LC8103 x has no value");
        assert_eq!(outcome(&mixed, "third"), "Not run: LC2001 elsewhere");
        let fault = json!({ "status": "fault", "fault": { "code": "LC8001", "message": "too many iterations" } });
        assert_eq!(outcome(&fault, "main"), "Fault: LC8001 too many iterations");
        assert_eq!(outcome(&json!({ "status": "error", "error": { "message": "no reply" } }), "main"), "Ended with an error: no reply");
        let outputs = json!({ "status": "ok", "outputs": [{ "name": "y", "value": 3.0 }, { "name": "s", "value": "hi" }] });
        assert_eq!(outcome(&outputs, "main"), "Ended\ny = 3.0\ns = \"hi\"");
    }
}
