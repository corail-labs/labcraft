//! The report of a run (`labcraft-run/1`, `docs/commands.md`, 7), written from its [`RunEnd`] as JSON text. What the run
//! does not know is given: the id of the run, the `check` made before it and what became of its trace, each already JSON,
//! and the real time it took. `engine` (step 6) and the corpus of runs call it; neither the report nor the trace knows where
//! it is written (`docs/architecture.md`, 10.7).
//!
//! **A report is bounded** (commands.md 2): the parts that are never cut (the status, the run, the error, the fault, the
//! abort, the clock, the counts) are written first; the values of the outputs and of the panel then take what is left,
//! each within its own budget; the commands of the instruments and the logs take the rest, and give way first. So the text
//! never grows past its limit, however many outputs or commands a run has.

use std::collections::HashMap;
use std::fmt::Write as _;

use labcraft_check::CheckedFile;
use labcraft_graph::Id;
use labcraft_ir::{Body, FunctionId, Program};
use labcraft_trace::number::millis_text;
use labcraft_trace::value::{CUT, TEXT_MAX, VALUE_BUDGET, quoted};
use labcraft_trace::{ValueWriter, WriteValue};
use labcraft_types::{Interner, Name, TyId};

use crate::convert::Env;
use crate::json::{Describe, Json};
use crate::native::{HandleInfo, LogEntry};
use crate::run::{AbortBy, LoopAt, Native, Resource, RunEnd, SetBy, Status};
use crate::trace::status_text;
use crate::value::Handle;

#[cfg(test)]
mod tests;

/// The name and version of the schema.
pub const SCHEMA: &str = "labcraft-run/1";

/// Resources and commands of each shown in a report *(proposed)*: past them, `truncated`.
pub const INSTRUMENTS_MAX: usize = 64;
pub const COMMANDS_MAX: usize = 1000;
/// Resources still open, and leaked, shown in a report *(proposed)*: past them, `truncated`.
pub const RESOURCES_MAX: usize = 1000;
/// The bound of a result (commands.md 2) *(proposed)*.
pub const RESULT_MAX: usize = 4 << 20;
/// The loops of a fault or of an abort shown in a report *(proposed)*: past them, `loops_more`.
pub const LOOPS_MAX: usize = 1000;

/// What a value takes at least in the report: the mark of a cut, and the brackets of a cut array.
const VALUE_MIN: usize = CUT.len() + 2;

/// Room kept for what closes the report and for `"truncated":true`.
const TAIL: usize = 64;

/// What a report needs besides the end of its run.
pub struct ReportOf<'a> {
    /// `r2`
    pub run: &'a str,
    pub program: &'a Program,
    pub names: &'a Interner,
    /// The files of the `Checked` the program was lowered from, which name where a node is.
    pub files: &'a [CheckedFile],
    /// The native libraries of the run, in the order it was given them.
    pub natives: &'a [Native<'a>],
    pub real_ms: u64,
    /// The `check` made before the run: `{"summary":…,"diagnostics":[…]}`, the fields of `labcraft-check/1`, its
    /// diagnostics already within their own bound.
    pub check: &'a str,
    /// What became of the trace: `{"file":…,"events":41,"truncated":false}`, or none without one.
    pub trace: Option<&'a str>,
    /// The bytes the report may take: [`RESULT_MAX`], or less.
    pub limit: usize,
}

/// The report of a run that ended. Its function is `functions[0]` of the program.
pub fn report(end: &RunEnd, of: &ReportOf<'_>) -> String {
    let env = Env { program: of.program, names: of.names };
    let Some(top) = of.program.functions.first() else { return refused(of.run, "", 0, of.check) };
    let handles = Ended(end.resources.iter().filter_map(|resource| Some((resource.handle, resource.info.clone()?))).collect());
    let json = Json { env, viewer: top.library, handles: &handles };

    // What is never cut: written first, apart, so that what is left for the rest is known.
    let mut fixed = String::with_capacity(512);
    if end.status == Status::Error {
        let err = top.outputs.iter().zip(&end.outputs).find(|(port, value)| of.names.resolve(port.name) == "err" && value.as_error().is_some());
        if let Some((_, error)) = err {
            fixed.push_str(",\"error\":");
            value_within(&mut fixed, &json.typed(error, TyId::ERROR), VALUE_BUDGET);
        }
    }
    if let Some(fault) = &end.fault {
        let _ = write!(fixed, ",\"fault\":{{\"code\":{},\"message\":{}", quoted(fault.fault.code.as_str()), quoted(&cut(&fault.fault.message)));
        place(&mut fixed, of, fault.function, fault.node, fault.origin);
        loops(&mut fixed, of, &fault.loops);
        if let Some(iteration) = fault.iteration {
            let _ = write!(fixed, ",\"iteration\":{iteration}");
        }
        fixed.push('}');
    }
    if let Some(abort) = &end.abort {
        let by = match abort.by {
            AbortBy::Request => "request",
            AbortBy::Timeout => "timeout",
            AbortBy::Interrupt => "interrupt",
        };
        let _ = write!(fixed, ",\"abort\":{{\"by\":{},\"at_ms\":{}", quoted(by), ms(abort.at));
        loops(&mut fixed, of, &abort.loops);
        fixed.push_str(",\"warning\":\"the instruments are in an unknown state\"}");
    }
    let _ = write!(fixed, ",\"clock\":{{\"simulated_ms\":{},\"real_ms\":{}}}", ms(end.clock), of.real_ms);
    let mut truncated = false;
    let _ = write!(fixed, ",\"resources\":{{\"opened\":{},\"open\":", end.resources.len());
    truncated |= resource_list(&mut fixed, of, end.resources.iter().filter(|resource| resource.info.as_ref().is_some_and(|info| info.open)));
    fixed.push_str(",\"leaked\":");
    truncated |= resource_list(&mut fixed, of, end.resources.iter().filter(|resource| resource.leaked));
    if truncated {
        fixed.push_str(",\"truncated\":true");
    }
    fixed.push('}');
    let mut closing = String::new();
    if let Some(trace) = of.trace {
        closing.push_str(",\"trace\":");
        closing.push_str(trace);
    }
    closing.push_str(",\"check\":");
    closing.push_str(of.check);

    // Each output and each setting of the panel, without its value: never cut, so counted before any value is written.
    let by_text = |by: SetBy| match by {
        SetBy::Program => "ui.set",
        SetBy::Set => "set",
        SetBy::RunSetElement => "run.set_element",
    };
    let shown_outputs: Vec<(String, TyId)> = top
        .outputs
        .iter()
        .map(|port| (format!("{{\"name\":{},\"type\":{},\"value\":", quoted(of.names.resolve(port.name)), quoted(&json.type_text(port.ty))), port.ty))
        .collect();
    let shown_panel: Vec<(String, String, TyId)> = end
        .panel
        .iter()
        .map(|applied| {
            let setting = &applied.setting;
            let element = top.elements.get(setting.element.0 as usize);
            let name = element.map_or("", |element| of.names.resolve(element.name));
            let ty = element.and_then(|element| element.ty).unwrap_or(TyId::ERROR);
            let after = format!(",\"at_ms\":{},\"by\":{},\"applied\":{}}}", ms(setting.at), quoted(by_text(setting.by)), applied.applied);
            (format!("{{\"element\":{}, \"value\":", quoted(name)).replace(", \"value", ",\"value"), after, ty)
        })
        .collect();
    let mut head_text = String::new();
    head(&mut head_text, status_text(end.status), of.run, &top.shown);
    // The outputs and the settings that fit, each without its value, which takes at least the mark of a cut: those past them
    // are counted in `outputs_more` and `panel_more` (64 bytes kept for the two).
    let mut room = of.limit.saturating_sub(head_text.len() + fixed.len() + closing.len() + TAIL + 64);
    let mut fits = |cost: usize| {
        let fit = cost <= room;
        if fit {
            room -= cost;
        }
        fit
    };
    let outputs_shown = shown_outputs.iter().take_while(|(before, _)| fits(before.len() + VALUE_MIN + 2)).count();
    let panel_shown = shown_panel.iter().take_while(|(before, after, _)| fits(before.len() + after.len() + VALUE_MIN + 1)).count();
    // What the values, the commands and the logs may take, past what is never cut.
    let mut left = room;

    let mut out = head_text;
    out.push_str(",\"outputs\":[");
    for (index, ((before, ty), value)) in shown_outputs.iter().zip(&end.outputs).take(outputs_shown).enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(before);
        let start = out.len();
        truncated |= value_within(&mut out, &json.typed(value, *ty), (left + VALUE_MIN).min(VALUE_BUDGET));
        left = left.saturating_sub((out.len() - start).saturating_sub(VALUE_MIN));
        out.push('}');
    }
    out.push(']');
    if outputs_shown < end.outputs.len() {
        let _ = write!(out, ",\"outputs_more\":{}", end.outputs.len() - outputs_shown);
        truncated = true;
    }
    out.push_str(&fixed);
    if !end.panel.is_empty() {
        out.push_str(",\"panel\":[");
        for (index, ((before, after, ty), applied)) in shown_panel.iter().zip(&end.panel).take(panel_shown).enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(before);
            let start = out.len();
            truncated |= value_within(&mut out, &json.typed(&applied.setting.value, *ty), (left + VALUE_MIN).min(VALUE_BUDGET));
            left = left.saturating_sub((out.len() - start).saturating_sub(VALUE_MIN));
            out.push_str(after);
        }
        out.push(']');
        if panel_shown < end.panel.len() {
            let _ = write!(out, ",\"panel_more\":{}", end.panel.len() - panel_shown);
            truncated = true;
        }
    }
    truncated |= instruments(&mut out, end, &mut left);
    truncated |= logs(&mut out, end, of, json, &mut left);
    out.push_str(&closing);
    if truncated {
        out.push_str(",\"truncated\":true");
    }
    out.push('}');
    out
}

/// The report of a run that nothing ran: the function is unknown, or `check`, an argument, a setting or the simulation file
/// was refused (commands.md 7). Its diagnostics are in `check`.
pub fn refused(run: &str, function: &str, real_ms: u64, check: &str) -> String {
    let mut out = String::new();
    head(&mut out, "refused", run, function);
    let _ = write!(
        out,
        ",\"outputs\":[],\"clock\":{{\"simulated_ms\":0.0,\"real_ms\":{real_ms}}},\"resources\":{{\"opened\":0,\"open\":[],\"leaked\":[]}},\"check\":{check}}}"
    );
    out
}

fn head(out: &mut String, status: &str, run: &str, function: &str) {
    let _ = write!(
        out,
        "{{\"schema\":{},\"status\":{},\"run\":{},\"function\":{},\"mode\":\"deterministic\"",
        quoted(SCHEMA),
        quoted(status),
        quoted(run),
        quoted(function)
    );
}

/// One value within `budget` bytes, at least [`VALUE_MIN`], so that a short value (`0`, `null`) is never replaced by the mark
/// of a cut; whether it was cut by the budget of the report rather than its own (commands.md 2).
fn value_within(out: &mut String, value: &dyn WriteValue, budget: usize) -> bool {
    let budget = budget.max(VALUE_MIN);
    let mut writer = ValueWriter::new(out, budget);
    value.write_value(&mut writer);
    writer.finish().spent && budget < VALUE_BUDGET
}

/// Nanoseconds on the clock of the run, in milliseconds, exact, as the trace writes them: `0.0`, `350.0`, `0.000001`.
fn ms(ns: i64) -> String {
    millis_text(u64::try_from(ns).unwrap_or(0))
}

/// A message, cut as a string is.
fn cut(text: &str) -> String {
    match text.char_indices().nth(TEXT_MAX) {
        Some((end, _)) => text.get(..end).unwrap_or_default().to_owned(),
        None => text.to_owned(),
    }
}

/// A text the library kept cut to its first 1,000 code points, with its whole length: cut as a string is (commands.md 5).
fn text(out: &mut String, kept: &str, length: u64) {
    if length as usize > TEXT_MAX || length > kept.chars().count() as u64 {
        let _ = write!(out, "{{\"$len\":{length},\"$head\":{}}}", quoted(kept));
    } else {
        out.push_str(&quoted(kept));
    }
}

/// `"function":…,"node":…,"file":…,"span":…`: where a node is.
fn place(out: &mut String, of: &ReportOf<'_>, function: FunctionId, node: Option<Name>, origin: Option<Id>) {
    let declared = of.program.functions.get(function.0 as usize);
    let _ = write!(out, ",\"function\":{}", quoted(declared.map_or("", |function| &function.shown)));
    match node {
        Some(node) => {
            let _ = write!(out, ",\"node\":{}", quoted(of.names.resolve(node)));
        }
        None => out.push_str(",\"node\":null"),
    }
    let file = declared.and_then(|function| of.files.get(function.file as usize));
    let _ = write!(out, ",\"file\":{}", quoted(file.map_or("", |file| &file.shown)));
    match file.zip(origin).and_then(|(file, origin)| file.source_map.span(origin)) {
        Some(span) => {
            let _ = write!(
                out,
                ",\"span\":{{\"start\":{{\"line\":{},\"column\":{}}},\"end\":{{\"line\":{},\"column\":{}}}}}",
                span.start.line, span.start.column, span.end.line, span.end.column
            );
        }
        None => out.push_str(",\"span\":null"),
    }
}

/// `,"loops":[…]`, at most [`LOOPS_MAX`], then `,"loops_more":N`: a fault deep in calls of nested loops can have many.
fn loops(out: &mut String, of: &ReportOf<'_>, loops: &[LoopAt]) {
    out.push_str(",\"loops\":[");
    for (index, at) in loops.iter().take(LOOPS_MAX).enumerate() {
        if index > 0 {
            out.push(',');
        }
        let function = of.program.functions.get(at.function.0 as usize).map_or("", |function| &function.shown);
        let _ = write!(out, "{{\"function\":{},\"node\":{},\"iteration\":{}}}", quoted(function), quoted(of.names.resolve(at.node)), at.iteration);
    }
    out.push(']');
    if loops.len() > LOOPS_MAX {
        let _ = write!(out, ",\"loops_more\":{}", loops.len() - LOOPS_MAX);
    }
}

/// At most [`RESOURCES_MAX`] resources; whether some were left out.
fn resource_list<'r>(out: &mut String, of: &ReportOf<'_>, resources: impl Iterator<Item = &'r Resource>) -> bool {
    out.push('[');
    let mut more = false;
    for (shown, resource) in resources.enumerate() {
        if shown == RESOURCES_MAX {
            more = true;
            break;
        }
        if shown > 0 {
            out.push(',');
        }
        resource_entry(out, of, resource.handle, resource.info.as_ref(), resource.function, resource.node, resource.origin);
    }
    out.push(']');
    more
}

/// A resource: its type (`visa.session`), what it designates (`SIM::DMM0`), and where it was opened.
fn resource_entry(
    out: &mut String,
    of: &ReportOf<'_>,
    handle: Handle,
    info: Option<&HandleInfo>,
    function: FunctionId,
    node: Option<Name>,
    origin: Option<Id>,
) {
    let library = of.natives.get(handle.library as usize).map_or("", |native| native.native.name());
    let kind = info.map_or("", |info| info.kind);
    let _ =
        write!(out, "{{\"type\":{},\"resource\":{}", quoted(&format!("{library}.{kind}")), quoted(&cut(info.map_or("", |info| &info.designates))));
    place(out, of, function, node, origin);
    out.push('}');
}

/// The commands each instrument received, within `left` bytes, their brackets and names included; whether some were left
/// out. An instrument that does not fit is counted in `instruments_more`.
fn instruments(out: &mut String, end: &RunEnd, left: &mut usize) -> bool {
    // `,"instruments":[` … `],"instruments_more":N`, and what closes an instrument, `],"truncated":true,"more":N}`.
    const FRAME: usize = 64;
    const CLOSE: usize = 48;
    if end.instruments.is_empty() {
        return false;
    }
    if *left < FRAME {
        return true;
    }
    let room = *left - FRAME;
    let start = out.len();
    let mut truncated = false;
    let mut shown = 0_usize;
    out.push_str(",\"instruments\":[");
    for log in end.instruments.iter().take(INSTRUMENTS_MAX) {
        let mut head = String::new();
        if shown > 0 {
            head.push(',');
        }
        let _ = write!(head, "{{\"resource\":{},\"commands\":[", quoted(&cut(&log.resource)));
        if out.len() - start + head.len() + CLOSE > room {
            break;
        }
        out.push_str(&head);
        let mut kept = 0_usize;
        for command in log.commands.iter().take(COMMANDS_MAX) {
            let mut entry = String::new();
            if kept > 0 {
                entry.push(',');
            }
            let _ = write!(entry, "{{\"t_ms\":{},\"sent\":", ms(command.at));
            text(&mut entry, &command.sent, command.sent_length);
            entry.push_str(",\"answer\":");
            match &command.answer {
                Some(answer) => text(&mut entry, answer, command.answer_length),
                None => entry.push_str("null"),
            }
            entry.push('}');
            if out.len() - start + entry.len() + CLOSE > room {
                break;
            }
            out.push_str(&entry);
            kept += 1;
        }
        out.push(']');
        let more = log.more.saturating_add((log.commands.len() - kept) as u64);
        if more > 0 {
            let _ = write!(out, ",\"truncated\":true,\"more\":{more}");
            truncated = true;
        }
        out.push('}');
        shown += 1;
    }
    out.push(']');
    if end.instruments.len() > shown {
        let _ = write!(out, ",\"instruments_more\":{}", end.instruments.len() - shown);
        truncated = true;
    }
    *left = left.saturating_sub(out.len() - start);
    truncated
}

/// What the libraries kept of the values the run gave them, in the order of `t_ms`, then of the libraries, within `left`
/// bytes, their brackets included; whether some were left out.
fn logs(out: &mut String, end: &RunEnd, of: &ReportOf<'_>, json: Json<'_>, left: &mut usize) -> bool {
    // `,"logs":{"entries":[` … `],"truncated":true,"more":N}`.
    const FRAME: usize = 80;
    if end.logs.iter().all(|(_, logs)| logs.entries.is_empty() && logs.more == 0) {
        return false;
    }
    if *left < FRAME {
        return true;
    }
    let room = *left - FRAME;
    let mut entries: Vec<(usize, &str, &LogEntry)> = Vec::new();
    for (index, (library, logs)) in end.logs.iter().enumerate() {
        entries.extend(logs.entries.iter().map(|entry| (index, library.as_str(), entry)));
    }
    entries.sort_by_key(|(index, _, entry)| (entry.at, *index));
    let start = out.len();
    out.push_str(",\"logs\":{\"entries\":[");
    let mut kept = 0_usize;
    for (_, library, entry) in &entries {
        let mut text = String::new();
        if kept > 0 {
            text.push(',');
        }
        let ty = input_type(of, library, &entry.function, entry.input);
        let type_text = ty.map_or_else(String::new, |ty| json.type_text(ty));
        let _ = write!(
            text,
            "{{\"library\":{},\"function\":{},\"t_ms\":{},\"type\":{},\"value\":",
            quoted(library),
            quoted(&entry.function),
            ms(entry.at),
            quoted(&type_text)
        );
        // What the value may take: what is left, less this entry and the `}` that closes it.
        let value_room = room.saturating_sub(out.len() - start + text.len() + 1);
        if value_room < VALUE_MIN {
            break;
        }
        let mut value = String::new();
        let mut writer = ValueWriter::new(&mut value, value_room.min(VALUE_BUDGET));
        json.typed(&entry.value, ty.unwrap_or(TyId::ERROR)).write_value(&mut writer);
        let written = writer.finish();
        if written.spent && value_room < VALUE_BUDGET {
            break;
        }
        text.push_str(&value);
        text.push('}');
        out.push_str(&text);
        kept += 1;
    }
    let more: u64 = end.logs.iter().map(|(_, logs)| logs.more).fold((entries.len() - kept) as u64, u64::saturating_add);
    let _ = write!(out, "],\"truncated\":{}", more > 0);
    if more > 0 {
        let _ = write!(out, ",\"more\":{more}");
    }
    out.push('}');
    *left = left.saturating_sub(out.len() - start);
    more > 0
}

/// The type of an input of an `extern rust` function of a library, by its place in the signature.
fn input_type(of: &ReportOf<'_>, library: &str, function: &str, input: usize) -> Option<TyId> {
    let native = of.natives.iter().find(|native| native.native.name() == library)?;
    let declared = of.program.functions.iter().find(|declared| match &declared.body {
        Body::Native { library: id, function: name, .. } => *id == native.library && of.names.resolve(*name) == function,
        Body::Region(_) => false,
    })?;
    declared.inputs.get(input).map(|port| port.ty)
}

/// What each resource of a run that ended was, by its handle: what a handle of its outputs shows.
struct Ended(HashMap<Handle, HandleInfo>);

impl Describe for Ended {
    fn describe(&self, handle: Handle) -> Option<HandleInfo> {
        self.0.get(&handle).cloned()
    }
}
