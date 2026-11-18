//! `node.add` with `into` (commands.md 6.7, visual notation 2.5): the node goes into a wire. The value enters the first
//! input of the node that takes its type, and the first output of the node that the reader takes goes where the wire went,
//! each judged as `wire.connect` judges a wire. A parameter that takes only a literal, the template of `string.format`,
//! never takes it. When no input takes the value, or no output goes where it went, nothing is written (`params.invalid`;
//! decision of the owner, 2026-11-17).

use labcraft_types::Limits;

use crate::delete::Reader;
use crate::model::Edit;
use crate::places::{Slot, slots};
use crate::refs::split_wire;
use crate::scope;
use crate::wire::FORMAT;
use crate::wire::{self, Found};
use crate::{EditError, Edited, Judge, Kind, Lookup, Point, add};

/// The inputs, and the outputs, that `into` tries at most *(proposed)*: each try judges the file again, so that a call of
/// thousands of parameters would take seconds (architecture 12.1, the bound of 2 s).
pub(crate) const TRIED: usize = 64;

/// Adds the node of `call` into the wire `into` of the text, and gives the new text.
#[allow(clippy::too_many_arguments)]
pub(crate) fn node(
    path: &str,
    text: &str,
    judge: &dyn Judge,
    limits: &Limits,
    call: &str,
    at: Option<Point>,
    name: Option<&str>,
    into: &str,
    edited: &mut Edited,
) -> Result<String, EditError> {
    // The wire, as it is before the node: where its value comes from, and the input that reads it.
    let before = Edit::read(path, text, limits)?;
    let function = before.body_function()?.clone();
    let found = wire::find(&before, judge, &function, into).map_err(|error| error.of("into"))?;
    let (from, to) = split_wire(into).map_err(|error| error.of("into"))?;
    // Both ends named as the text before the node names them: an `@N` would name another line once the node is added.
    // The node goes in the frame where the wire is read: an arm, for a wire to its `yield`.
    let frame = match &found {
        Found::Reader { reader: Reader::Slot(loc, slot), .. } => scope::statement(&function, loc)
            .and_then(|statement| slots(statement, loc).into_iter().find(|(found, _)| found == slot).map(|(_, seen_in)| seen_in))
            .unwrap_or_else(|| loc.body.clone()),
        Found::Reader { reader: Reader::After(loc), .. } => loc.body.clone(),
        Found::Alias(_) | Found::Bound { .. } => Vec::new(),
        Found::NextLine(_) | Found::NextBound { .. } => {
            return Err(EditError::invalid("into", format!("`{into}` sets the `next` of a carry: wire the node to it instead")));
        }
    };
    let (from, reader) = match found {
        Found::Reader { label: true, .. } => {
            return Err(EditError::invalid("into", format!("`{into}` is an order wire: it carries no value for a node to take")));
        }
        // A `collect` or an `until` is named as the wire names it.
        Found::Reader { name, reader: Reader::Slot(_, Slot::Collect | Slot::Until), .. } => (before.text(name).to_owned(), to),
        Found::Reader { name, input, node, .. } => {
            let reader = match node {
                Some((node, is_label)) => {
                    let node = format!("{}{}", if is_label { "#" } else { "" }, before.text(node));
                    if input.is_empty() { node } else { format!("{node}.{input}") }
                }
                None => to,
            };
            (before.text(name).to_owned(), reader)
        }
        Found::Alias(_) | Found::Bound { .. } => (from, format!("output {}", output_of(&to))),
        Found::NextLine(_) | Found::NextBound { .. } => return Err(EditError::internal("a `next` refused above")),
    };
    // `add::node` refuses a call that names nothing, with the nearest names.
    let mut edit = before;
    add::node(&mut edit, judge, call, at, name, &frame, edited)?;
    let Lookup::Found(callable) = judge.callable(path, call) else {
        return Err(EditError::internal(format!("`{call}` was found once and not again")));
    };
    let named = edited.names.get("node").cloned().unwrap_or_default();
    // Only a node with no output gets a label (N-14).
    if callable.outputs.is_empty() {
        return Err(EditError::invalid("into", format!("`{call}` has no output to go where the wire went")));
    }
    let mut now = edit.finish()?;
    // The value enters the first input that takes it.
    let unnamed = callable.inputs.iter().filter(|input| input.name.is_empty()).count();
    let mut entered = false;
    for (rank, input) in callable.inputs.iter().enumerate().take(TRIED) {
        if input.variadic || (call == FORMAT && rank == 0) || (callable.kind == Kind::Function && input.name.is_empty()) {
            continue;
        }
        let end = if input.name.is_empty() {
            if unnamed == 1 { named.clone() } else { format!("{named}.{rank}") }
        } else {
            format!("{named}.{}", input.name)
        };
        if let Some(text) = attempt(path, &now, judge, limits, &from, &end, false, edited) {
            now = text;
            entered = true;
            break;
        }
    }
    if !entered {
        return Err(EditError::invalid("into", format!("no input of `{call}` takes `{from}` (of its first {TRIED})")));
    }
    // The first output that the reader takes goes where the wire went.
    for (rank, output) in callable.outputs.iter().enumerate().take(TRIED) {
        let start = if rank == 0 { named.clone() } else { format!("{named}.{output}") };
        if let Some(text) = attempt(path, &now, judge, limits, &start, &reader, true, edited) {
            // A node folded into an output of the function bears its name now (notation 2.4): `names.node` says so.
            if let Some(renamed) = edited.renamed.iter().rev().find(|renamed| renamed.from == named) {
                edited.names.insert("node", renamed.to.clone());
            }
            return Ok(text);
        }
    }
    Err(EditError::invalid("into", format!("no output of `{call}` goes where `{from}` went (of its first {TRIED})")))
}

/// The text with the wire from `from` to `to`, when `wire.connect` takes it; what it renamed goes into `edited`.
#[allow(clippy::too_many_arguments)]
fn attempt(path: &str, text: &str, judge: &dyn Judge, limits: &Limits, from: &str, to: &str, replace: bool, edited: &mut Edited) -> Option<String> {
    let mut edit = Edit::read(path, text, limits).ok()?;
    let mut said = Edited::default();
    wire::connect(&mut edit, judge, from, to, None, &wire::How { replace, accept_mismatch: false, through: None }, &mut said).ok()?;
    let text = edit.finish().ok()?;
    edited.renamed.extend(said.renamed);
    Some(text)
}

/// The output that a wire into an output of the function names: `y` of `x -> y` or of `x -> output y`.
fn output_of(to: &str) -> &str {
    to.strip_prefix("output ").map_or(to, str::trim)
}
