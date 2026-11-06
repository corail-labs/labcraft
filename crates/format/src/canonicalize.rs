//! Everything that `fmt` changes in the model (spec 5.2, `docs/cli.md`, section 2). The layout of the text is the
//! job of `write`, and names and comments have their one spelling from the moment they are read.

use std::collections::{HashMap, HashSet};

use labcraft_graph::{
    Attached, Body, Decl, File, LayoutItem, LayoutKind, NodeRef, Numeral, NumeralKind, Operand, Panel, PanelDirection, PanelItem, Path, Point, Rhs,
    Signature, SourceMap, Statement, StatementKind, Use,
};
use labcraft_types::{Code, Diagnostic, Interner, Limits, Name, Report};

use crate::order::{self, BodyOrder, Unorderable, first_name};
use crate::read::read;
use crate::write::{self, reference_text};

/// The largest coordinate in plain decimal (spec 11).
const COORDINATE_RANGE: u32 = 1_000_000;

/// A model in canonical form: `write` prints it as it stands. The ids are renumbered, so `map` no longer applies
/// to what this returns. A hostile file can earn millions of warnings, so they go to a report, which bounds them.
pub fn canonicalize(mut file: File, map: &SourceMap, names: &mut Interner, report: &mut Report) -> File {
    sort_uses(&mut file.uses, names);
    let mut ordered = true;
    if let Decl::Function(function) = &mut file.decl {
        let orders = order::analyse(&function.body, Some(&function.signature), names);
        let mut orders = orders.into_iter();
        ordered = apply_orders(&mut function.body, &mut orders, map, report);
    }
    if let Some(panel) = &mut file.panel {
        sort_panel(panel, signature_of(&file.decl), names);
    }
    canonicalize_layout(&mut file, ordered, map, names, report);
    if file.panel.as_ref().is_some_and(|panel| panel.items.is_empty() && panel.end.is_empty() && nothing_attached(&panel.comments)) {
        file.panel = None;
    }
    file.renumber();
    file
}

/// What [`format`] gives: the canonical text, unless the file has an error, and what there is to say about it.
#[derive(Debug)]
pub struct Formatted {
    pub text: Option<String>,
    pub report: Report,
}

/// A file from its bytes to its canonical text. The caller compares the text with the bytes, and writes it.
///
/// It takes the stack that `read` takes: see [`crate::STACK_SIZE`].
pub fn format(bytes: &[u8], names: &mut Interner, limits: &Limits) -> Formatted {
    let reading = read(bytes, names, limits);
    let mut report = reading.report;
    let Some(file) = reading.file else {
        return Formatted { text: None, report };
    };
    // What `canonicalize` says is about the text, so it counts only if there is one.
    let mut changes = Report::new(limits.max_report);
    let file = canonicalize(file, &reading.source_map, names, &mut changes);
    match write::write(&file, names, limits) {
        Ok(text) => {
            report.merge(changes);
            Formatted { text: Some(text), report }
        }
        Err(write::TooLarge) => {
            let message = format!("the canonical form of this file is over the limit of {} bytes, so the file stays as it is", limits.max_file_size);
            report.push(Diagnostic::new(Code::Lc6003, message, ""));
            Formatted { text: None, report }
        }
    }
}

fn nothing_attached(comments: &Attached) -> bool {
    comments.before.is_empty() && comments.line.is_none()
}

fn path_text(path: &Path, names: &Interner) -> String {
    let mut text = String::new();
    for (index, segment) in path.segments().enumerate() {
        if index > 0 {
            text.push('.');
        }
        text.push_str(names.resolve(segment));
    }
    text
}

/// Sorted, each library once. A line with a comment stays (5.2): a repeated line goes only if nothing is lost.
fn sort_uses(uses: &mut Vec<Use>, names: &Interner) {
    if uses.is_empty() {
        return;
    }
    let mut keyed: Vec<(String, Use)> = std::mem::take(uses).into_iter().map(|line| (path_text(&line.path, names), line)).collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    let commented: HashSet<String> = keyed.iter().filter(|(_, line)| line.line.is_some()).map(|(text, _)| text.clone()).collect();
    let mut seen = HashSet::new();
    for (text, line) in keyed {
        let keep = if commented.contains(&text) { line.line.is_some() } else { seen.insert(text) };
        if keep {
            uses.push(line);
        }
    }
}

/// Puts each body in its order, innermost first, as [`order::analyse`] gave them. False if one cannot have one.
fn apply_orders(body: &mut Body, orders: &mut std::vec::IntoIter<BodyOrder>, map: &SourceMap, report: &mut Report) -> bool {
    let mut ordered = true;
    for statement in &mut body.statements {
        for sub_body in order::sub_bodies(statement) {
            ordered &= apply_orders(sub_body, orders, map, report);
        }
    }
    match orders.next() {
        Some(Ok(Some(order))) => permute(&mut body.statements, &order),
        Some(Err(fault)) => {
            report_unorderable(body, fault, map, report);
            ordered = false;
        }
        Some(Ok(None)) | None => {}
    }
    ordered
}

fn permute(statements: &mut Vec<Statement>, order: &[usize]) {
    let mut slots: Vec<Option<Statement>> = std::mem::take(statements).into_iter().map(Some).collect();
    statements.extend(order.iter().filter_map(|line| slots.get_mut(*line).and_then(Option::take)));
}

fn report_unorderable(body: &Body, fault: Unorderable, map: &SourceMap, report: &mut Report) {
    let (line, why) = match fault {
        Unorderable::DefinedTwice(line) => (line, "a value or a label is defined twice"),
        Unorderable::Cycle(line) => (line, "its lines wait for each other in a cycle"),
    };
    let mut diagnostic = Diagnostic::new(Code::Lc6002, format!("this body keeps its written order: {why}"), "");
    if let Some(span) = body.statements.get(line).and_then(|statement| map.span(statement.id)) {
        diagnostic = diagnostic.with_span(span);
    }
    report.push(diagnostic);
}

fn signature_of(decl: &Decl) -> Option<&Signature> {
    match decl {
        Decl::Function(function) => Some(&function.signature),
        Decl::Extern(function) => Some(&function.signature),
        Decl::Type(_) => None,
    }
}

/// The `control` items, then the `indicator` items; in each group, the terminals in signature order, then the
/// others by name.
fn sort_panel(panel: &mut Panel, signature: Option<&Signature>, names: &Interner) {
    let (mut inputs, mut outputs): (HashMap<Name, usize>, HashMap<Name, usize>) = (HashMap::new(), HashMap::new());
    if let Some(signature) = signature {
        for (index, param) in signature.params.iter().enumerate() {
            inputs.entry(param.name).or_insert(index);
        }
        for (index, output) in signature.outputs.iter().enumerate() {
            outputs.entry(output.name).or_insert(index);
        }
    }
    panel.items.sort_by_cached_key(|item: &PanelItem| {
        let (group, terminals) = match item.direction {
            PanelDirection::Control => (0_u8, &inputs),
            PanelDirection::Indicator => (1_u8, &outputs),
        };
        match terminals.get(&item.name) {
            Some(index) => (group, 0_u8, *index, String::new()),
            None => (group, 1_u8, 0, names.resolve(item.name).to_owned()),
        }
    });
}

fn canonicalize_layout(file: &mut File, ordered: bool, map: &SourceMap, names: &mut Interner, report: &mut Report) {
    let known = match &file.decl {
        _ if !ordered => None,
        Decl::Function(function) => {
            let outputs: HashSet<Name> = function.signature.outputs.iter().map(|output| output.name).collect();
            let mut known = nodes_of(&function.body, &outputs);
            // Terminals of the frame, not nodes: an entry that names one is for `check` to judge (spec 11).
            known.extend(function.signature.params.iter().map(|param| (param.name, false)));
            known.extend(function.signature.outputs.iter().map(|output| (output.name, false)));
            Some(known)
        }
        Decl::Extern(_) => Some(HashSet::new()),
        // A section in a type file is the business of `check` (`LC0104`).
        Decl::Type(_) => None,
    };
    let Some(layout) = &mut file.layout else {
        return;
    };
    if let Some(known) = known {
        layout.items.retain(|item| {
            let keep = names_what_is_known(item, &known) || !nothing_attached(&item.comments);
            if !keep {
                report_stale_entry(item, map, names, report);
            }
            keep
        });
    }
    for item in &mut layout.items {
        match &mut item.kind {
            LayoutKind::Node { at, .. } => {
                if let Some(point) = at {
                    plain_point(point, names);
                }
            }
            LayoutKind::Wire { via, .. } => {
                for point in via {
                    plain_point(point, names);
                }
            }
        }
    }
    let names: &Interner = names;
    layout.items.sort_by_cached_key(|item| match &item.kind {
        LayoutKind::Node { node, .. } => (0_u8, reference_text(*node, None, names), String::new()),
        LayoutKind::Wire { from, to, .. } => (1_u8, reference_text(from.node, from.terminal, names), reference_text(to.node, to.terminal, names)),
    });
    if layout.items.is_empty() && layout.end.is_empty() && nothing_attached(&layout.comments) {
        file.layout = None;
    }
}

fn report_stale_entry(item: &LayoutItem, map: &SourceMap, names: &Interner, report: &mut Report) {
    let what = match &item.kind {
        LayoutKind::Node { node, .. } => format!("node {}", reference_text(*node, None, names)),
        LayoutKind::Wire { from, to, .. } => {
            format!("wire {} -> {}", reference_text(from.node, from.terminal, names), reference_text(to.node, to.terminal, names))
        }
    };
    let mut diagnostic =
        Diagnostic::new(Code::Lc5004, format!("`{what}` names nothing on the diagram of this function, so canonical form leaves it out"), "");
    if let Some(span) = map.span(item.id) {
        diagnostic = diagnostic.with_span(span);
    }
    report.push(diagnostic);
}

fn names_what_is_known(item: &LayoutItem, known: &HashSet<(Name, bool)>) -> bool {
    let is_known = |node: &NodeRef| known.contains(&(node.name, node.is_label));
    match &item.kind {
        LayoutKind::Node { node, .. } => is_known(node),
        LayoutKind::Wire { from, to, .. } => is_known(&from.node) && is_known(&to.node),
    }
}

/// What the `layout` section can designate, in every body of the function (spec 11): a node by its label if it
/// has one, or else by the name of its line. Giving a value a new name creates no node, while a name given to a
/// constant does (5.1). `fmt` reads one file, so it cannot tell `Verdict.Pass` from `step.low`: `check` judges those.
fn nodes_of(body: &Body, outputs: &HashSet<Name>) -> HashSet<(Name, bool)> {
    let mut nodes = HashSet::new();
    add_nodes(body, outputs, &mut nodes);
    nodes
}

fn add_nodes(body: &Body, outputs: &HashSet<Name>, nodes: &mut HashSet<(Name, bool)>) {
    for statement in &body.statements {
        add_node(statement, outputs, nodes);
    }
}

fn add_node(statement: &Statement, outputs: &HashSet<Name>, nodes: &mut HashSet<(Name, bool)>) {
    let (first, value) = match &statement.kind {
        StatementKind::Binding { targets, value } => (first_name(targets), Some(value)),
        StatementKind::Call(_) => (None, None),
        StatementKind::Collect(_) | StatementKind::Until(_) => return,
    };
    // A line that binds an output, a carry or nothing is a terminal, whatever its value (R-4).
    let no_node = match value {
        Some(Rhs::Operand(Operand::Path(path))) if path.rest.is_empty() => true,
        Some(Rhs::Operand(_)) => first.is_none_or(|name| outputs.contains(&name)),
        _ => false,
    };
    if !no_node {
        match (statement.label, first) {
            (Some(label), _) => {
                nodes.insert((label.name, true));
            }
            (None, Some(name)) => {
                nodes.insert((name, false));
            }
            (None, None) => {}
        }
    }
    match value {
        Some(Rhs::Match(branching) | Rhs::Handle(branching)) => {
            for arm in &branching.arms {
                add_nodes(&arm.body, outputs, nodes);
            }
        }
        Some(Rhs::For(structure)) => add_nodes(&structure.body, outputs, nodes),
        Some(Rhs::Loop(structure)) => add_nodes(&structure.body, outputs, nodes),
        Some(Rhs::Call(_) | Rhs::Struct(_) | Rhs::Operand(_)) | None => {}
    }
}

fn plain_point(point: &mut Point, names: &mut Interner) {
    plain_coordinate(&mut point.x, names);
    plain_coordinate(&mut point.y, names);
}

/// An integer literal in the range is written in plain decimal; any other coordinate keeps its spelling, and is
/// an error for `check` (5.2).
fn plain_coordinate(numeral: &mut Numeral, names: &mut Interner) {
    let NumeralKind::Int(text) = numeral.kind else {
        return;
    };
    let Some(value) = coordinate_value(names.resolve(text)) else {
        return;
    };
    let digits = value.to_string();
    if digits != names.resolve(text) {
        numeral.kind = NumeralKind::Int(names.intern(&digits));
    }
    if value == 0 {
        numeral.negative = false;
    }
}

/// Its value, if it is in the range, whatever the zeros and underscores around its digits.
fn coordinate_value(text: &str) -> Option<u32> {
    let mut value: u32 = 0;
    for byte in text.bytes().filter(|byte| *byte != b'_') {
        let digit = u32::from(byte.checked_sub(b'0')?);
        value = value.checked_mul(10)?.checked_add(digit)?;
        if value > COORDINATE_RANGE {
            return None;
        }
    }
    Some(value)
}
