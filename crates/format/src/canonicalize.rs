//! Everything that `fmt` changes in the model (spec 5.2, `docs/cli.md`, section 2). The layout of the text is the
//! job of `write`, and names and comments have their one spelling from the moment they are read.

use std::collections::{HashMap, HashSet};

use labcraft_graph::{
    Anchor, Attached, BlockPlace, BlockSide, Body, Clause, ClauseKind, Decl, File, LayoutItem, LayoutKind, NodeRef, Numeral, NumeralKind, Operand,
    OutlineKind, Panel, PanelDirection, PanelItem, Path, Point, Rhs, Signature, SourceMap, Statement, StatementKind, Target, TerminalsKind, Use,
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
    canonicalize_terminals(&mut file, names);
    canonicalize_outline(&mut file, names);
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
            let mut nodes = nodes_of(&function.body, &outputs);
            // Terminals of the frame, not nodes: an entry that names one is for `check` to judge (spec 11).
            let frame = function.signature.params.iter().map(|param| param.name).chain(function.signature.outputs.iter().map(|output| output.name));
            nodes.extend(frame.map(|name| (name, false)));
            // A wire starts at the value it carries, or at a label (spec 11): any name that the function binds.
            let mut starts = nodes.clone();
            add_values(&function.body, &mut starts);
            let inputs = function.signature.params.iter().map(|param| param.name).collect();
            Some(Known { nodes, starts, inputs, outputs })
        }
        Decl::Extern(_) => Some(Known::default()),
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
            LayoutKind::Frame { size, .. } => plain_point(size, names),
            LayoutKind::Terminal { y, .. } => plain_coordinate(y, names),
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
    // The place of each terminal in the signature, for the order of their entries.
    let (mut inputs, mut outputs): (HashMap<Name, usize>, HashMap<Name, usize>) = (HashMap::new(), HashMap::new());
    if let Decl::Function(function) = &file.decl {
        for (index, param) in function.signature.params.iter().enumerate() {
            inputs.entry(param.name).or_insert(index);
        }
        for (index, output) in function.signature.outputs.iter().enumerate() {
            outputs.entry(output.name).or_insert(index);
        }
    }
    // The frame of the function, those of the structures by name, the inputs then the outputs in signature order (an
    // entry that names no terminal after those of its kind), the nodes, the wires (spec 5.2).
    layout.items.sort_by_cached_key(|item| match &item.kind {
        LayoutKind::Frame { node: None, .. } => (0_u8, 0, String::new(), String::new()),
        LayoutKind::Frame { node: Some(node), .. } => (1_u8, 0, reference_text(*node, None, names), String::new()),
        LayoutKind::Terminal { output, name, .. } => {
            let (group, places) = if *output { (3_u8, &outputs) } else { (2_u8, &inputs) };
            (group, places.get(name).copied().unwrap_or(usize::MAX), String::new(), String::new())
        }
        LayoutKind::Node { node, .. } => (4_u8, 0, reference_text(*node, None, names), String::new()),
        LayoutKind::Wire { from, to, .. } => (5_u8, 0, reference_text(from.node, from.terminal, names), reference_text(to.node, to.terminal, names)),
    });
    if layout.items.is_empty() && layout.end.is_empty() && nothing_attached(&layout.comments) {
        file.layout = None;
    }
}

/// The `terminals` section (5.2): `top`, `left`, the lines of the terminals in signature order, the inputs then the
/// outputs, those that name no terminal after them in written order, then `band`; numbers in plain decimal; `input` or
/// `output` kept only where the name alone would mean the other terminal (11.2).
fn canonicalize_terminals(file: &mut File, names: &mut Interner) {
    let Some(section) = &mut file.terminals else {
        return;
    };
    let (mut inputs, mut outputs): (HashMap<Name, usize>, HashMap<Name, usize>) = (HashMap::new(), HashMap::new());
    if let Some(signature) = signature_of(&file.decl) {
        for (index, param) in signature.params.iter().enumerate() {
            inputs.entry(param.name).or_insert(index);
        }
        for (index, output) in signature.outputs.iter().enumerate() {
            outputs.entry(output.name).or_insert(index);
        }
    }
    for item in &mut section.items {
        match &mut item.kind {
            TerminalsKind::Size { places, .. } => plain_digits(places, names),
            TerminalsKind::Place { side, place, output, name } => {
                if let BlockPlace::Number(digits) = place {
                    plain_digits(digits, names);
                }
                // The name alone means the input on the left or the top, the output on the right, when it is both.
                let both = inputs.contains_key(name) && outputs.contains_key(name);
                let alone_means_output = both && *side == BlockSide::Right;
                let kept = match *output {
                    Some(true) => !outputs.contains_key(name) || (both && !alone_means_output),
                    Some(false) => !inputs.contains_key(name) || (both && alone_means_output),
                    None => false,
                };
                if !kept {
                    *output = None;
                }
            }
            TerminalsKind::Band(_) | TerminalsKind::Colour { .. } => {}
        }
    }
    section.items.sort_by_key(|item| match item.kind {
        TerminalsKind::Size { side, .. } => (0_u8, usize::from(side != BlockSide::Top)),
        TerminalsKind::Place { side, output, name, .. } => {
            let means_output = match output {
                Some(output) => output,
                None => !inputs.contains_key(&name) || (side == BlockSide::Right && outputs.contains_key(&name)),
            };
            let found = if means_output { outputs.get(&name) } else { inputs.get(&name) };
            match found {
                Some(index) => (if means_output { 2 } else { 1 }, *index),
                None => (3, 0),
            }
        }
        TerminalsKind::Band(_) => (4, 0),
        TerminalsKind::Colour { role, .. } => (5, role as usize),
    });
    if section.items.is_empty() && section.end.is_empty() && nothing_attached(&section.comments) {
        file.terminals = None;
    }
}

/// The `outline` section (5.2): `template` or `path` first, then the pieces of the `top`, `left` and `right` sides, each
/// kind in written order; integers in plain decimal, `x + 0` written `x`, `-0` written `0` (11.4).
fn canonicalize_outline(file: &mut File, names: &mut Interner) {
    let Some(section) = &mut file.outline else {
        return;
    };
    for item in &mut section.items {
        match &mut item.kind {
            OutlineKind::Template(_) => {}
            OutlineKind::Path(corners) => {
                for corner in corners {
                    for coordinate in [&mut corner.x, &mut corner.y] {
                        if let Anchor::Number(digits) = &mut coordinate.base {
                            plain_digits(digits, names);
                        }
                        if let Some((_, digits)) = &mut coordinate.shift {
                            plain_digits(digits, names);
                            if names.resolve(*digits) == "0" {
                                coordinate.shift = None;
                            }
                        }
                    }
                    if let Some(round) = &mut corner.round {
                        plain_digits(round, names);
                    }
                }
            }
            OutlineKind::Piece { points, .. } => {
                for point in points {
                    for signed in [&mut point.out, &mut point.along] {
                        plain_digits(&mut signed.digits, names);
                        if names.resolve(signed.digits) == "0" {
                            signed.negative = false;
                        }
                    }
                }
            }
        }
    }
    section.items.sort_by_key(|item| match item.kind {
        OutlineKind::Template(_) | OutlineKind::Path(_) => 0_u8,
        OutlineKind::Piece { side: BlockSide::Top, .. } => 1,
        OutlineKind::Piece { side: BlockSide::Left, .. } => 2,
        OutlineKind::Piece { side: BlockSide::Right, .. } => 3,
    });
    if section.items.is_empty() && section.end.is_empty() && nothing_attached(&section.comments) {
        file.outline = None;
    }
}

/// The digits of a number of `terminals` in plain decimal (`1_6`, `016`: `16`), when they read as one.
fn plain_digits(digits: &mut Name, names: &mut Interner) {
    let Some(value) = coordinate_value(names.resolve(*digits)) else {
        return;
    };
    let text = value.to_string();
    if text != names.resolve(*digits) {
        *digits = names.intern(&text);
    }
}

fn report_stale_entry(item: &LayoutItem, map: &SourceMap, names: &Interner, report: &mut Report) {
    let what = match &item.kind {
        LayoutKind::Frame { node: None, .. } => "frame size".to_owned(),
        LayoutKind::Frame { node: Some(node), .. } => format!("frame {} size", reference_text(*node, None, names)),
        LayoutKind::Terminal { output, name, .. } => format!("{} {}", if *output { "output" } else { "input" }, names.resolve(*name)),
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

/// What a layout entry can name, by name and whether it is a label.
#[derive(Default)]
struct Known {
    /// The nodes, the inputs and the outputs: what a `node` entry names, and where a wire ends.
    nodes: HashSet<(Name, bool)>,
    /// Those, and every value and label of the function: where a wire starts.
    starts: HashSet<(Name, bool)>,
    /// The inputs and the outputs of the function: what an `input` or an `output` entry names.
    inputs: HashSet<Name>,
    outputs: HashSet<Name>,
}

fn names_what_is_known(item: &LayoutItem, known: &Known) -> bool {
    let is = |set: &HashSet<(Name, bool)>, node: &NodeRef| set.contains(&(node.name, node.is_label));
    match &item.kind {
        LayoutKind::Frame { node: None, .. } => true,
        LayoutKind::Terminal { output: false, name, .. } => known.inputs.contains(name),
        LayoutKind::Terminal { output: true, name, .. } => known.outputs.contains(name),
        LayoutKind::Frame { node: Some(node), .. } | LayoutKind::Node { node, .. } => is(&known.nodes, node),
        // A wire starts at the value it carries, never at a terminal (`raw.session`): one file is enough to see it.
        LayoutKind::Wire { from, to, .. } => from.terminal.is_none() && is(&known.starts, &from.node) && is(&known.nodes, &to.node),
    }
}

/// Every name that a body binds, in every body of the function: lines, labels, carries, `in` clauses, patterns.
fn add_values(body: &Body, values: &mut HashSet<(Name, bool)>) {
    for statement in &body.statements {
        if let Some(label) = &statement.label {
            values.insert((label.name, true));
        }
        let StatementKind::Binding { targets, value } = &statement.kind else { continue };
        for target in targets.iter() {
            if let Target::Name(_, name) = target {
                values.insert((*name, false));
            }
        }
        match value {
            Rhs::Match(branching) | Rhs::Handle(branching) => {
                for arm in &branching.arms {
                    for pattern in arm.patterns() {
                        values.extend(pattern.bindings().iter().map(|binding| (binding.name, false)));
                    }
                    add_values(&arm.body, values);
                }
            }
            Rhs::For(structure) => {
                add_clauses(&structure.clauses, values);
                add_values(&structure.body, values);
            }
            Rhs::Loop(structure) => {
                add_clauses(&structure.clauses, values);
                add_values(&structure.body, values);
            }
            Rhs::Call(_) | Rhs::Struct(_) | Rhs::Operand(_) => {}
        }
    }
}

fn add_clauses(clauses: &[Clause], values: &mut HashSet<(Name, bool)>) {
    for clause in clauses {
        let name = match &clause.kind {
            ClauseKind::In(clause) => clause.name,
            ClauseKind::Carry(carry) => carry.name,
        };
        values.insert((name, false));
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
        StatementKind::Gather(_, _) | StatementKind::Until(_) => return,
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

/// The value of the digits of a coordinate (`INT`, spec 14.1), if it is in the range of spec 11, whatever the zeros and
/// underscores around them. `check` refuses any other coordinate (`LC5005`).
pub fn coordinate_value(text: &str) -> Option<u32> {
    let mut value: u32 = 0;
    for byte in text.bytes().filter(|byte| *byte != b'_') {
        // A `BASED` integer has letters, and is no coordinate.
        if !byte.is_ascii_digit() {
            return None;
        }
        let digit = u32::from(byte - b'0');
        value = value.checked_mul(10)?.checked_add(digit)?;
        if value > COORDINATE_RANGE {
            return None;
        }
    }
    Some(value)
}
