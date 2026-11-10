//! The parts of a report that `engine` writes and the report cuts (`docs/commands.md`, section 2): the `check`, whose
//! diagnostics give way after the values of the outputs, warnings before errors, and an inline trace, whose events give way
//! first. Each is cut at the end of a whole entry, so that what stays reads as JSON.

use labcraft_runtime::report::Part;
use labcraft_types::{CheckCounts, Report};
use serde_json::{Value as Json, json};

/// The `check` of a report: the summary of `labcraft-check/1` and its diagnostics, those of the run after those of `check`.
#[derive(Clone, Debug)]
pub(crate) struct CheckPart {
    summary: Json,
    /// Each diagnostic as JSON text, with whether it is an error.
    diagnostics: Vec<(String, bool)>,
}

impl CheckPart {
    /// From the report of `check`, the diagnostics of the run added, each an error.
    pub fn of_report(report: &Report, counts: CheckCounts, deny_warnings: bool, problems: Vec<Json>) -> CheckPart {
        let document = serde_json::to_value(report.check_document(counts, deny_warnings)).unwrap_or(Json::Null);
        let mut summary = document.get("summary").cloned().unwrap_or_else(|| json!({}));
        let errors = summary.get("errors").and_then(Json::as_u64).unwrap_or(0) + problems.len() as u64;
        if let Some(object) = summary.as_object_mut() {
            object.insert("errors".to_owned(), json!(errors));
        }
        let mut diagnostics: Vec<(String, bool)> = document
            .get("diagnostics")
            .and_then(Json::as_array)
            .map(|found| found.iter().map(|diagnostic| (diagnostic.to_string(), diagnostic["severity"] == "error")).collect())
            .unwrap_or_default();
        diagnostics.extend(problems.iter().map(|problem| (problem.to_string(), true)));
        CheckPart { summary, diagnostics }
    }

    /// The diagnostics of a run that could not even be checked: its folder does not exist.
    pub fn of_problems(problems: Vec<Json>) -> CheckPart {
        let summary = json!({ "libraries": 0, "functions": 0, "types": 0, "errors": problems.len(), "warnings": 0, "truncated": false });
        CheckPart { summary, diagnostics: problems.iter().map(|problem| (problem.to_string(), true)).collect() }
    }

    fn write(&self, kept: &[bool], cut: bool) -> String {
        let mut summary = self.summary.clone();
        if cut && let Some(object) = summary.as_object_mut() {
            object.insert("truncated".to_owned(), json!(true));
        }
        let mut out = format!("{{\"summary\":{summary},\"diagnostics\":[");
        let mut first = true;
        for ((text, _), keep) in self.diagnostics.iter().zip(kept) {
            if *keep {
                if !first {
                    out.push(',');
                }
                first = false;
                out.push_str(text);
            }
        }
        out.push_str("]}");
        out
    }
}

impl Part for CheckPart {
    fn least(&self) -> String {
        self.write(&vec![false; self.diagnostics.len()], !self.diagnostics.is_empty())
    }

    fn within(&self, bytes: usize) -> (String, bool) {
        let mut kept = vec![true; self.diagnostics.len()];
        let whole = self.write(&kept, false);
        if whole.len() <= bytes {
            return (whole, false);
        }
        // Warnings give way before errors, the last first, until the rest fits (`,` and the room of `"truncated":true`).
        let mut size = self.least().len();
        let mut order: Vec<usize> =
            (0..self.diagnostics.len()).filter(|index| self.diagnostics.get(*index).is_some_and(|(_, error)| *error)).collect();
        order.extend((0..self.diagnostics.len()).filter(|index| self.diagnostics.get(*index).is_some_and(|(_, error)| !*error)));
        kept.iter_mut().for_each(|keep| *keep = false);
        for index in order {
            let Some((text, _)) = self.diagnostics.get(index) else { continue };
            if size + text.len() + 1 > bytes {
                break;
            }
            size += text.len() + 1;
            if let Some(keep) = kept.get_mut(index) {
                *keep = true;
            }
        }
        (self.write(&kept, true), true)
    }
}

/// An inline trace (commands.md 7, `trace: inline`): its first line and its events, the last giving way first.
pub(crate) struct InlineTrace {
    pub header: String,
    pub events: Vec<String>,
    /// The writer reached its own bound of events.
    pub truncated: bool,
}

impl InlineTrace {
    fn write(&self, count: usize, truncated: bool) -> String {
        let mut out = format!("{{\"header\":{},\"events\":[", self.header);
        for (index, event) in self.events.iter().take(count).enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(event);
        }
        out.push_str(&format!("],\"truncated\":{truncated}}}"));
        out
    }

    /// From the lines a trace writer wrote.
    pub fn of_lines(text: &str, truncated: bool) -> InlineTrace {
        let mut lines = text.lines();
        let header = lines.next().unwrap_or("{}").to_owned();
        InlineTrace { header, events: lines.map(str::to_owned).collect(), truncated }
    }
}

impl Part for InlineTrace {
    fn least(&self) -> String {
        self.write(0, self.truncated || !self.events.is_empty())
    }

    fn within(&self, bytes: usize) -> (String, bool) {
        let whole = self.write(self.events.len(), self.truncated);
        if whole.len() <= bytes {
            return (whole, false);
        }
        let mut size = self.least().len();
        let mut count = 0;
        for event in &self.events {
            if size + event.len() + 1 > bytes {
                break;
            }
            size += event.len() + 1;
            count += 1;
        }
        (self.write(count, true), true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part() -> CheckPart {
        CheckPart {
            summary: json!({ "errors": 1, "warnings": 2, "truncated": false }),
            diagnostics: vec![
                (json!({ "code": "LC1", "severity": "warning", "message": "w".repeat(50) }).to_string(), false),
                (json!({ "code": "LC2", "severity": "error", "message": "e".repeat(50) }).to_string(), true),
                (json!({ "code": "LC3", "severity": "warning", "message": "w".repeat(50) }).to_string(), false),
            ],
        }
    }

    #[test]
    fn the_check_gives_up_its_warnings_before_its_errors() {
        let part = part();
        let (whole, cut) = part.within(10_000);
        assert!(!cut && whole.contains("LC1") && whole.contains("LC2") && whole.contains("LC3"));
        let least = part.least().len();
        let (some, cut) = part.within(least + 120);
        let read: Json = serde_json::from_str(&some).unwrap();
        assert!(cut);
        assert_eq!(read["diagnostics"].as_array().map(Vec::len), Some(1));
        assert_eq!(read["diagnostics"][0]["code"], "LC2", "the error stays");
        assert_eq!(read["summary"]["truncated"], true);
        let (none, _) = part.within(0);
        assert_eq!(none, part.least());
    }

    #[test]
    fn an_inline_trace_keeps_its_first_events() {
        let trace = InlineTrace::of_lines("{\"schema\":\"labcraft-trace/1\"}\n{\"seq\":1}\n{\"seq\":2}\n", false);
        let (whole, cut) = trace.within(10_000);
        assert!(!cut);
        assert_eq!(serde_json::from_str::<Json>(&whole).unwrap()["events"].as_array().map(Vec::len), Some(2));
        let (one, cut) = trace.within(trace.least().len() + 11);
        let read: Json = serde_json::from_str(&one).unwrap();
        assert!(cut && read["truncated"] == true);
        assert_eq!(read["events"], json!([{ "seq": 1 }]));
    }
}
