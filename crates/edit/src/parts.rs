//! The parts of a structure (commands.md 6.7, "Structures"): the arms of a `match` or a `handle` and their patterns, the
//! carries, the `in` clauses and the `collect` lines of a loop, its options. What a part holds goes with it, and what it
//! fed becomes unwired or gets a hole (9.1), as `node.delete` leaves it.

use std::collections::HashSet;

use labcraft_format::{is_name, read};
use labcraft_graph::{Arm, Attached, Carry, Clause, ClauseKind, Gather, LayoutKind, Operand, Path, Rhs, Statement, StatementKind, Target, TypeExpr};
use labcraft_types::Name;

use crate::delete::{Reader, forget_in_layout, label_readers, readers, take_out};
use crate::model::Edit;
use crate::names::{fresh, given, taken};
use crate::refs::{NodeName, node};
use crate::scope::{self, Bound, Loc, Value};
use crate::wire::unbind_next;
use crate::{EditError, Edited, Judge, StructureOption};

/// The structure that `structure` names, and its statement.
fn structure_at(edit: &Edit, text: &str) -> Result<(Loc, Statement), EditError> {
    let function = edit.body_function()?;
    let name = NodeName::parse(text.trim());
    if name.text.is_empty() {
        return Err(EditError::invalid("structure", "a structure is named as a node: its label (`#for_node`), the name of its line, or `@N`"));
    }
    let loc = node(edit, function, &name, "structure")?;
    let statement = scope::statement(function, &loc).cloned().ok_or_else(|| EditError::internal("a node named went away"))?;
    Ok((loc, statement))
}

fn shown(text: &str) -> String {
    crate::cut(text.trim())
}

/// The arms of the structure, or `params.invalid` for one that has none.
fn branching_of<'s>(statement: &'s mut Statement, text: &str) -> Result<&'s mut labcraft_graph::Branching, EditError> {
    match &mut statement.kind {
        StatementKind::Binding { value: Rhs::Match(branching) | Rhs::Handle(branching), .. } => Ok(branching),
        _ => Err(EditError::invalid("structure", format!("`{}` is not a `match` or a `handle`: it has no arm", shown(text)))),
    }
}

/// The statement back in its place.
fn put_back(edit: &mut Edit, loc: &Loc, statement: Statement) -> Result<(), EditError> {
    let function = edit.body_function_mut()?;
    let place = scope::statement_mut(function, loc).ok_or_else(|| EditError::internal("a structure that went"))?;
    *place = statement;
    Ok(())
}

/// One statement read from its text, in a function of its own, with the names of `edit`: what the grammar reads is what
/// a command writes. The message of the parser when it does not read; none when it reads as something else.
pub(crate) fn read_one(edit: &mut Edit, text: &str) -> Result<Option<Statement>, String> {
    let wrapped = format!("labcraft 1\n\nfn f() -> () {{\n{text}}}\n");
    let limits = edit.limits().clone();
    let reading = read(wrapped.as_bytes(), &mut edit.names, &limits);
    let Some(file) = reading.file else {
        return Err(reading.report.diagnostics().first().map(|diagnostic| diagnostic.message.clone()).unwrap_or_default());
    };
    let labcraft_graph::Decl::Function(function) = file.decl else { return Ok(None) };
    if !function.body.end.is_empty() {
        return Ok(None);
    }
    let mut statements = function.body.statements;
    if statements.len() != 1 {
        return Ok(None);
    }
    Ok(statements.pop())
}

/// The patterns of one arm of a `match` or a `handle`, read from their text.
fn read_arm(edit: &mut Edit, keyword: &str, patterns: &str) -> Result<Arm, EditError> {
    let text = patterns.trim();
    let shown = shown(text);
    if text.is_empty() || text.contains(['\n', '\r']) {
        return Err(EditError::invalid("patterns", format!("`{shown}` is not the patterns of one arm, on one line: `1, 3..=5`, `io.timeout(err)`")));
    }
    let not_one = || EditError::invalid("patterns", format!("`{shown}` is not the patterns of one arm of a `{keyword}`"));
    let statement = match read_one(edit, &format!("    _ = {keyword} x {{\n        {text} => {{\n        }}\n    }}\n")) {
        Ok(Some(statement)) => statement,
        Ok(None) => return Err(not_one()),
        Err(why) => return Err(EditError::invalid("patterns", format!("`{shown}` does not read as patterns: {why}"))),
    };
    let StatementKind::Binding { value: Rhs::Match(branching) | Rhs::Handle(branching), .. } = statement.kind else { return Err(not_one()) };
    let mut arms = branching.arms;
    let (Some(arm), true) = (arms.pop(), arms.is_empty()) else { return Err(not_one()) };
    if arm.comments != Attached::default() || arm.open.is_some() || !arm.body.statements.is_empty() || !arm.yields.values.is_empty() {
        return Err(not_one());
    }
    Ok(arm)
}

/// `structure.add_arm`.
pub(crate) fn add_arm(edit: &mut Edit, structure: &str, patterns: &str, edited: &mut Edited) -> Result<(), EditError> {
    let (loc, mut statement) = structure_at(edit, structure)?;
    let keyword = match &statement.kind {
        StatementKind::Binding { value: Rhs::Handle(_), .. } => "handle",
        _ => "match",
    };
    let mut arm = read_arm(edit, keyword, patterns)?;
    let written = patterns_text_of(&edit.names, &arm);
    let outputs = branching_of(&mut statement, structure)?.arms.first().map_or(0, |first| first.yields.values.len());
    let mut holes = Vec::with_capacity(outputs);
    for _ in 0..outputs {
        holes.push(Operand::Hole(edit.id()));
    }
    arm.yields.values = holes;
    arm.id = edit.id();
    let branching = branching_of(&mut statement, structure)?;
    if branching.arms.iter().any(|known| patterns_text_of(&edit.names, known) == written) {
        return Err(EditError::invalid("patterns", format!("`{}` has an arm `{}` already", shown(structure), crate::cut(&written))));
    }
    // A last `_` takes what the arms before it do not: the new arm goes before it.
    let last_is_wildcard =
        branching.arms.last().is_some_and(|last| last.others.is_empty() && matches!(last.pattern, labcraft_graph::Pattern::Wildcard { .. }));
    if last_is_wildcard {
        let at = branching.arms.len().saturating_sub(1);
        branching.arms.insert(at, arm);
    } else {
        branching.arms.push(arm);
    }
    put_back(edit, &loc, statement)?;
    edited.names.insert("arm", written);
    Ok(())
}

fn patterns_text_of(names: &labcraft_types::Interner, arm: &Arm) -> String {
    arm.patterns().map(|pattern| labcraft_format::write_pattern(pattern, names, usize::MAX)).collect::<Vec<_>>().join(", ")
}

/// The index of the arm that `arm` names, by its patterns.
fn arm_named(edit: &Edit, branching: &labcraft_graph::Branching, structure: &str, arm: &str) -> Result<usize, EditError> {
    crate::frame::arm_index(edit, branching, arm).ok_or_else(|| {
        EditError::new("edit.unknown", format!("`{}` has no arm `{}`", shown(structure), crate::cut(arm)))
            .of("arm")
            .expecting(crate::frame::arm_names(edit, branching))
    })
}

/// `structure.set_patterns`.
pub(crate) fn set_patterns(edit: &mut Edit, structure: &str, arm: &str, patterns: &str, edited: &mut Edited) -> Result<(), EditError> {
    let (loc, mut statement) = structure_at(edit, structure)?;
    let keyword = match &statement.kind {
        StatementKind::Binding { value: Rhs::Handle(_), .. } => "handle",
        _ => "match",
    };
    let index = {
        let branching = branching_of(&mut statement, structure)?;
        arm_named(edit, branching, structure, arm)?
    };
    let new = read_arm(edit, keyword, patterns)?;
    let written = patterns_text_of(&edit.names, &new);
    let branching = branching_of(&mut statement, structure)?;
    if branching.arms.iter().enumerate().any(|(rank, known)| rank != index && patterns_text_of(&edit.names, known) == written) {
        return Err(EditError::invalid("patterns", format!("`{}` has an arm `{}` already", shown(structure), crate::cut(&written))));
    }
    let Some(target) = branching.arms.get_mut(index) else { return Err(EditError::internal("an arm that went")) };
    target.pattern = new.pattern;
    target.others = new.others;
    put_back(edit, &loc, statement)?;
    edited.names.insert("arm", written);
    Ok(())
}

/// The clauses of a loop, and whether it is a `for parallel`; `params.invalid` for what is not a loop.
fn clauses_of<'s>(statement: &'s mut Statement, text: &str) -> Result<(&'s mut Vec<Clause>, bool, bool), EditError> {
    match &mut statement.kind {
        StatementKind::Binding { value: Rhs::For(each), .. } => Ok((&mut each.clauses, true, each.parallel)),
        StatementKind::Binding { value: Rhs::Loop(repeat), .. } => Ok((&mut repeat.clauses, false, false)),
        _ => Err(EditError::invalid("structure", format!("`{}` is not a `for` or a `loop`", shown(text)))),
    }
}

fn loop_body(statement: &Statement) -> Option<&labcraft_graph::Body> {
    match &statement.kind {
        StatementKind::Binding { value: Rhs::For(each), .. } => Some(&each.body),
        StatementKind::Binding { value: Rhs::Loop(repeat), .. } => Some(&repeat.body),
        _ => None,
    }
}

fn loop_body_mut(statement: &mut Statement) -> Option<&mut labcraft_graph::Body> {
    match &mut statement.kind {
        StatementKind::Binding { value: Rhs::For(each), .. } => Some(&mut each.body),
        StatementKind::Binding { value: Rhs::Loop(repeat), .. } => Some(&mut repeat.body),
        _ => None,
    }
}

/// How many outputs a loop gives: its carries, then its `collect` and `last` lines (spec 6.2).
pub(crate) fn loop_outputs(statement: &Statement) -> (usize, usize) {
    let carries = match &statement.kind {
        StatementKind::Binding { value: Rhs::For(each), .. } => &each.clauses,
        StatementKind::Binding { value: Rhs::Loop(repeat), .. } => &repeat.clauses,
        _ => return (0, 0),
    };
    let carries = carries.iter().filter(|clause| matches!(clause.kind, ClauseKind::Carry(_))).count();
    let collects =
        loop_body(statement).map_or(0, |body| body.statements.iter().filter(|line| matches!(line.kind, StatementKind::Gather(_, _))).count());
    (carries, collects)
}

/// Adds a target at `at` among the outputs of the structure; a structure that gave nothing had `_` alone.
pub(crate) fn insert_target(statement: &mut Statement, outputs_before: usize, at: usize, target: Target) -> Result<(), EditError> {
    let StatementKind::Binding { targets, .. } = &mut statement.kind else { return Err(EditError::internal("a structure that is not one")) };
    let mut list = if outputs_before == 0 { Vec::new() } else { std::mem::take(targets).into_vec() };
    let at = at.min(list.len());
    list.insert(at, target);
    *targets = list.into_boxed_slice();
    Ok(())
}

/// Removes the target at `at`; a structure left with no output gets `_` alone.
fn remove_target(edit: &mut Edit, statement: &mut Statement, at: usize) -> Result<(), EditError> {
    let id = edit.id();
    let StatementKind::Binding { targets, .. } = &mut statement.kind else { return Err(EditError::internal("a structure that is not one")) };
    let mut list = std::mem::take(targets).into_vec();
    if at < list.len() {
        list.remove(at);
    }
    if list.is_empty() {
        list.push(Target::Skip(id));
    }
    *targets = list.into_boxed_slice();
    Ok(())
}

/// A type of a library (`visa.session`, `[visa.session]`) needs its `use` line in the file: it is added.
pub(crate) fn use_library_of(edit: &mut Edit, ty: &str) {
    let library = ty.trim().trim_start_matches('[').split_once('.').map(|(first, _)| first.trim()).filter(|first| is_name(first));
    if let Some(library) = library {
        crate::add::add_use(edit, library);
    }
}

/// A type written as the text writes it (`f64[V]`), read by the grammar.
pub(crate) fn read_type(edit: &mut Edit, text: &str) -> Option<TypeExpr> {
    let text = text.trim();
    if text.is_empty() || text.contains(['\n', '\r', '{', '}']) {
        return None;
    }
    let wrapped = format!("labcraft 1\n\nfn f(x: {text}) -> () {{\n}}\n");
    let limits = edit.limits().clone();
    let reading = read(wrapped.as_bytes(), &mut edit.names, &limits);
    let labcraft_graph::Decl::Function(function) = reading.file?.decl else { return None };
    let [param] = function.signature.params.as_slice() else { return None };
    (param.default.is_none() && !param.variadic).then(|| param.ty.clone())
}

/// An operand that a part reads: a value that `frame` sees, by its name, or a constant. A value that the structure at
/// `structure` makes, or that depends on what it makes, would close a cycle (spec 5.2): `wire.cycle`, as `wire.connect`
/// says it.
fn operand_of(edit: &mut Edit, text: &str, frame: &[scope::Step], structure: &Loc, parameter: &'static str) -> Result<Operand, EditError> {
    let text = text.trim();
    if is_name(text) {
        let function = edit.body_function()?.clone();
        let value = crate::surround::visible(edit, &function, text, frame, parameter)?;
        refuse_cycle(edit, &function, &value, structure, parameter)?;
        return Ok(Operand::Path(Path { id: edit.id(), first: value.name, rest: Box::new([]) }));
    }
    crate::add::read_constant(edit, text).map_err(|error| error.of(parameter))
}

/// `wire.cycle` for a value read by the structure at `structure` (its head, or a line of its body) that is made outside
/// its body by it, or by what depends on it.
fn refuse_cycle(edit: &Edit, function: &labcraft_graph::Function, value: &Value, structure: &Loc, parameter: &'static str) -> Result<(), EditError> {
    let Bound::Line(made_by, _) = &value.bound else { return Ok(()) };
    // A value of its own body is read there, after it is made: no cycle.
    if made_by.body.starts_with(&structure.sub(0)) {
        return Ok(());
    }
    if crate::wire::closes_a_cycle(function, made_by, structure) {
        return Err(EditError::new(
            "wire.cycle",
            format!("`{}` is made by the structure or depends on it: it would read its own output (spec 6.2)", edit.text(value.name)),
        )
        .of(parameter));
    }
    Ok(())
}

/// The name of the line of the statement at `loc`, by which `layout` names its node when it has no label (spec 11).
pub(crate) fn line_name_at(edit: &Edit, loc: &Loc) -> Option<(Name, bool)> {
    let statement = scope::statement(edit.body_function().ok()?, loc)?;
    if let Some(label) = statement.label {
        return Some((label.name, true));
    }
    scope::line_name(statement).map(|name| (name, false))
}

/// The outputs of the structure at `loc` changed: the entries of `layout` that named its node by the name of its line
/// follow its new name, or a label when it has none now, so that it keeps its place (spec 11).
pub(crate) fn follow_name(edit: &mut Edit, loc: &Loc, before: Option<(Name, bool)>) -> Result<(), EditError> {
    let Some((old, false)) = before else { return Ok(()) };
    let now = match line_name_at(edit, loc) {
        Some((now, false)) if now == old => return Ok(()),
        Some(now) => now,
        None => {
            let named = edit.file.layout.as_ref().is_some_and(|layout| {
                layout.items.iter().any(|item| match &item.kind {
                    LayoutKind::Frame { node: None, .. } | LayoutKind::Terminal { .. } => false,
                    LayoutKind::Frame { node: Some(node), .. } | LayoutKind::Node { node, .. } => !node.is_label && node.name == old,
                    LayoutKind::Wire { to, .. } => !to.node.is_label && to.node.name == old,
                })
            });
            if !named {
                return Ok(());
            }
            let found = crate::layout::designate(edit, loc, &mut Vec::new())?;
            (found.name, found.is_label)
        }
    };
    let Some(layout) = &mut edit.file.layout else { return Ok(()) };
    // The node: its `node` entry and the ends of the wires into it. A wire from the value `old` names the value, which
    // stays.
    for item in &mut layout.items {
        match &mut item.kind {
            LayoutKind::Node { node, .. } | LayoutKind::Frame { node: Some(node), .. } if !node.is_label && node.name == old => {
                node.name = now.0;
                node.is_label = now.1;
            }
            LayoutKind::Wire { to, .. } if !to.node.is_label && to.node.name == old => {
                to.node.name = now.0;
                to.node.is_label = now.1;
            }
            _ => {}
        }
    }
    Ok(())
}

/// `structure.add_carry`.
pub(crate) fn add_carry(
    edit: &mut Edit,
    structure: &str,
    name: &str,
    initial: Option<&str>,
    ty: Option<&str>,
    edited: &mut Edited,
) -> Result<(), EditError> {
    let (loc, mut statement) = structure_at(edit, structure)?;
    let (_, _, parallel) = clauses_of(&mut statement, structure)?;
    if parallel {
        return Err(EditError::invalid("structure", format!("`{}` is a `for parallel`, which takes no carry (spec 6.2)", shown(structure))));
    }
    let name = given(name.trim(), &taken(edit), "name")?;
    let ty = match ty {
        Some(text) => {
            let found = read_type(edit, text).ok_or_else(|| EditError::invalid("type", format!("`{}` is not a type", shown(text))))?;
            use_library_of(edit, text);
            Some(found)
        }
        None => None,
    };
    let value = match initial {
        Some(text) => operand_of(edit, text, &loc.body, &loc, "initial")?,
        None if ty.is_none() => {
            return Err(EditError::invalid("type", "a carry that starts from a hole needs `type`: its type is on its line alone (spec 6.2)"));
        }
        None => Operand::Hole(edit.id()),
    };
    let carry = edit.intern(&name);
    push_carry(edit, &loc, carry, ty, value)?;
    edited.names.insert("output", name);
    Ok(())
}

/// Adds `carry name = value` to the loop at `loc`, after its other clauses, and its output `name`, after the outputs of
/// the other carries (spec 6.2); the entries of `layout` follow the name of its line (spec 11). What `structure.add_carry`
/// and a wire out of a loop share.
pub(crate) fn push_carry(edit: &mut Edit, loc: &Loc, carry: Name, ty: Option<TypeExpr>, value: Operand) -> Result<(), EditError> {
    let mut statement = scope::statement(edit.body_function()?, loc).cloned().ok_or_else(|| EditError::internal("a loop that went"))?;
    let before = line_name_at(edit, loc);
    let (carries, collects) = loop_outputs(&statement);
    let (carry_id, target_id) = (edit.id(), edit.id());
    let clause = Clause { comments: Attached::default(), kind: ClauseKind::Carry(Carry { id: carry_id, name: carry, ty, value }) };
    match &mut statement.kind {
        StatementKind::Binding { value: Rhs::For(each), .. } => each.clauses.push(clause),
        StatementKind::Binding { value: Rhs::Loop(repeat), .. } => repeat.clauses.push(clause),
        _ => return Err(EditError::internal("a carry for what is not a loop")),
    }
    insert_target(&mut statement, carries + collects, carries, Target::Name(target_id, carry))?;
    put_back(edit, loc, statement)?;
    follow_name(edit, loc, before)
}

/// Adds `collect value` or `last value` at the end of the body of the loop at `loc`, and its output `output`, after the
/// others (spec 6.2); the entries of `layout` follow the name of its line. What `structure.add_collect`,
/// `structure.add_last` and a wire out of a loop share.
pub(crate) fn push_gather(edit: &mut Edit, loc: &Loc, gather: Gather, output: Name, value: Operand) -> Result<(), EditError> {
    let mut statement = scope::statement(edit.body_function()?, loc).cloned().ok_or_else(|| EditError::internal("a loop that went"))?;
    let before = line_name_at(edit, loc);
    let (carries, collects) = loop_outputs(&statement);
    let (line_id, target_id) = (edit.id(), edit.id());
    let body = loop_body_mut(&mut statement).ok_or_else(|| EditError::internal("a `collect` or a `last` for what is not a loop"))?;
    body.statements.push(Statement { id: line_id, comments: Attached::default(), label: None, kind: StatementKind::Gather(gather, value) });
    insert_target(&mut statement, carries + collects, carries + collects, Target::Name(target_id, output))?;
    put_back(edit, loc, statement)?;
    follow_name(edit, loc, before)
}

/// `structure.add_clause`.
pub(crate) fn add_clause(
    edit: &mut Edit,
    judge: &dyn Judge,
    structure: &str,
    over: &str,
    name: Option<&str>,
    edited: &mut Edited,
) -> Result<(), EditError> {
    let (loc, mut statement) = structure_at(edit, structure)?;
    let (_, is_for, _) = clauses_of(&mut statement, structure)?;
    if !is_for {
        return Err(EditError::invalid("structure", format!("`{}` is a `loop`: an `in` clause is for a `for`", shown(structure))));
    }
    let function = edit.body_function()?.clone();
    let over = crate::surround::over_of(edit, &function, over, &loc.body)?;
    let values = scope::values(&function);
    for name in &over.names {
        let value = values.iter().find(|value| value.name == *name && scope::sees(&function, value, &loc.body)).cloned();
        if let Some(value) = value {
            refuse_cycle(edit, &function, &value, &loc, "over")?;
        }
    }
    let found = over.array.as_ref().map(|array| crate::surround::type_of(edit, judge, array));
    let base = crate::surround::element_base(edit, &over, found.as_ref())?;
    let chosen = taken(edit);
    let name = match name {
        Some(text) => given(text.trim(), &chosen, "name")?,
        None => fresh(&base, &chosen),
    };
    let clause = match read_one(edit, &format!("    _ = for {name} in {} {{\n    }}\n", over.text)) {
        Ok(Some(Statement { kind: StatementKind::Binding { value: Rhs::For(each), .. }, .. })) => each.clauses.into_iter().next(),
        _ => None,
    };
    let Some(clause) = clause else { return Err(EditError::internal("an `in` clause that does not read: a bug of `edit`")) };
    let (clauses, _, _) = clauses_of(&mut statement, structure)?;
    // After the other `in` clauses, before the carries (spec 6.2).
    let at = clauses.iter().rposition(|known| matches!(known.kind, ClauseKind::In(_))).map_or(0, |last| last + 1);
    clauses.insert(at, clause);
    put_back(edit, &loc, statement)?;
    edited.names.insert("clause", name);
    Ok(())
}

/// `structure.add_collect` and `structure.add_last`.
pub(crate) fn add_gather(
    edit: &mut Edit,
    gather: Gather,
    structure: &str,
    value: &str,
    name: Option<&str>,
    edited: &mut Edited,
) -> Result<(), EditError> {
    let (loc, mut statement) = structure_at(edit, structure)?;
    clauses_of(&mut statement, structure)?;
    let collected = operand_of(edit, value, &loc.sub(0), &loc, "value")?;
    let chosen = taken(edit);
    let name = match name {
        Some(text) => given(text.trim(), &chosen, "name")?,
        None => {
            let base = if is_name(value.trim()) { value.trim().to_owned() } else { "collected".to_owned() };
            fresh(&base, &chosen)
        }
    };
    let output = edit.intern(&name);
    push_gather(edit, &loc, gather, output, collected)?;
    edited.names.insert("output", name);
    Ok(())
}

/// What `part` names.
enum Part {
    Arm(String),
    Carry(String),
    In(String),
    Gather(Gather, String),
}

fn part_of(text: &str) -> Part {
    let text = text.trim();
    if let Some(name) = text.strip_prefix("carry ") {
        return Part::Carry(name.trim().to_owned());
    }
    if let Some(name) = text.strip_prefix("in ") {
        return Part::In(name.trim().to_owned());
    }
    for gather in [Gather::Collect, Gather::Last] {
        if let Some(name) = text.strip_prefix(gather.word()).and_then(|rest| rest.strip_prefix(' ')) {
            return Part::Gather(gather, name.trim().to_owned());
        }
    }
    Part::Arm(text.to_owned())
}

/// Takes each value out of what reads it outside the places of `skip`, then lets `layout` forget the names that go.
fn take_values_out(edit: &mut Edit, judge: &dyn Judge, values: &[Value], skip: &[Loc]) -> Result<(), EditError> {
    for value in values {
        let now = edit.body_function()?.clone();
        let found = readers(&now, value, skip);
        take_out(edit, judge, value.name, false, &found)?;
    }
    Ok(())
}

/// `structure.remove_part`.
pub(crate) fn remove_part(edit: &mut Edit, judge: &dyn Judge, structure: &str, part: &str) -> Result<(), EditError> {
    let (loc, statement) = structure_at(edit, structure)?;
    let before = line_name_at(edit, &loc);
    let function = edit.body_function()?.clone();
    let values = scope::values(&function);
    let mut gone: HashSet<Name> = HashSet::new();
    match part_of(part) {
        Part::Arm(text) => {
            let mut held = statement.clone();
            let branching = branching_of(&mut held, structure)?;
            let index = arm_named(edit, branching, structure, &text).map_err(|error| EditError { parameter: Some("part"), ..error })?;
            if branching.arms.len() < 2 {
                return Err(EditError::invalid("part", format!("`{}` has one arm: delete the structure (`node.delete`)", shown(structure))));
            }
            let arm_path = loc.sub(index);
            let inside_arm = |at: &Loc| at.body.starts_with(&arm_path);
            // What the arm holds goes: its values, from the readers outside it (an `after:` of a label, none other).
            let held_values: Vec<Value> = values
                .iter()
                .filter(|value| match &value.bound {
                    Bound::Line(at, _) => inside_arm(at),
                    Bound::Head(at, sub) => *at == loc && *sub == index || inside_arm(at),
                    Bound::Input => false,
                })
                .cloned()
                .collect();
            let arm_locs: Vec<Loc> = scope::all(&function).into_iter().filter(|at| inside_arm(at)).collect();
            let labels: Vec<Name> = scope::labels(&function).into_iter().filter(|(_, at)| inside_arm(at)).map(|(label, _)| label).collect();
            take_values_out(edit, judge, &held_values, &arm_locs)?;
            for label in &labels {
                let now = edit.body_function()?.clone();
                let found: Vec<Reader> = label_readers(&now, *label)
                    .into_iter()
                    .filter(|reader| match reader {
                        Reader::After(at) | Reader::Slot(at, _) => !inside_arm(at),
                    })
                    .collect();
                take_out(edit, judge, *label, true, &found)?;
            }
            gone.extend(held_values.iter().map(|value| value.name));
            gone.extend(labels);
            let mut now = scope::statement(edit.body_function()?, &loc).cloned().ok_or_else(|| EditError::internal("a structure that went"))?;
            let branching = branching_of(&mut now, structure)?;
            if index < branching.arms.len() {
                branching.arms.remove(index);
            }
            put_back(edit, &loc, now)?;
        }
        Part::Carry(text) => {
            let (rank, carry) = carry_named(edit, &statement, structure, &text)?;
            let body_path = loc.sub(0);
            // The carry, read in the body, and its output, read outside.
            let inner: Vec<Value> =
                values.iter().filter(|value| value.name == carry && matches!(&value.bound, Bound::Head(at, _) if *at == loc)).cloned().collect();
            let outer: Vec<Value> =
                values.iter().filter(|value| matches!(&value.bound, Bound::Line(at, i) if *at == loc && *i == rank)).cloned().collect();
            take_values_out(edit, judge, &inner, &[])?;
            take_values_out(edit, judge, &outer, &[])?;
            gone.extend(inner.iter().chain(&outer).map(|value| value.name));
            // Its `next`: the line goes; a node that binds it binds `_` there.
            let now_function = edit.body_function()?.clone();
            let lines = scope::body(&now_function, &body_path).map(|body| body.statements.clone()).unwrap_or_default();
            let mut drop_lines = Vec::new();
            for (index, line) in lines.iter().enumerate() {
                let StatementKind::Binding { targets, value } = &line.kind else { continue };
                if !targets.iter().any(|target| matches!(target, Target::Next(_, name) if *name == carry)) {
                    continue;
                }
                if targets.len() == 1 && matches!(value, Rhs::Operand(_)) {
                    drop_lines.push(index);
                } else {
                    unbind_next(edit, &Loc { body: body_path.clone(), index }, carry)?;
                }
            }
            let mut now = scope::statement(edit.body_function()?, &loc).cloned().ok_or_else(|| EditError::internal("a structure that went"))?;
            if let Some(body) = loop_body_mut(&mut now) {
                for index in drop_lines.into_iter().rev() {
                    if index < body.statements.len() {
                        body.statements.remove(index);
                    }
                }
            }
            let (clauses, _, _) = clauses_of(&mut now, structure)?;
            clauses.retain(|clause| !matches!(&clause.kind, ClauseKind::Carry(found) if found.name == carry));
            remove_target(edit, &mut now, rank)?;
            put_back(edit, &loc, now)?;
        }
        Part::In(text) => {
            let mut held = statement.clone();
            let (clauses, is_for, _) = clauses_of(&mut held, structure)?;
            if !is_for {
                return Err(EditError::invalid("part", format!("`{}` is a `loop`: it has no `in` clause", shown(structure))));
            }
            let ins: Vec<Name> = clauses
                .iter()
                .filter_map(|clause| match &clause.kind {
                    ClauseKind::In(each) => Some(each.name),
                    ClauseKind::Carry(_) => None,
                })
                .collect();
            let Some(name) = edit.names.find(&text).filter(|name| ins.contains(name)) else {
                return Err(EditError::new("edit.unknown", format!("`{}` has no clause `in {}`", shown(structure), crate::cut(&text)))
                    .of("part")
                    .expecting(ins.iter().map(|each| format!("in {}", edit.text(*each))).collect()));
            };
            if ins.len() < 2 {
                return Err(EditError::invalid("part", format!("`{}` has one `in` clause: delete the structure (`node.delete`)", shown(structure))));
            }
            let inner: Vec<Value> =
                values.iter().filter(|value| value.name == name && matches!(&value.bound, Bound::Head(at, _) if *at == loc)).cloned().collect();
            take_values_out(edit, judge, &inner, &[])?;
            gone.insert(name);
            let mut now = scope::statement(edit.body_function()?, &loc).cloned().ok_or_else(|| EditError::internal("a structure that went"))?;
            let (clauses, _, _) = clauses_of(&mut now, structure)?;
            clauses.retain(|clause| !matches!(&clause.kind, ClauseKind::In(found) if found.name == name));
            put_back(edit, &loc, now)?;
        }
        Part::Gather(gather, text) => {
            let (index, output) = gather_named(edit, &statement, structure, gather, &text)?;
            let (carries, _) = loop_outputs(&statement);
            let rank = index - carries;
            let outer: Vec<Value> =
                values.iter().filter(|value| matches!(&value.bound, Bound::Line(at, i) if *at == loc && *i == index)).cloned().collect();
            take_values_out(edit, judge, &outer, &[])?;
            gone.insert(output);
            let mut now = scope::statement(edit.body_function()?, &loc).cloned().ok_or_else(|| EditError::internal("a structure that went"))?;
            if let Some(body) = loop_body_mut(&mut now) {
                let at = body
                    .statements
                    .iter()
                    .enumerate()
                    .filter(|(_, line)| matches!(line.kind, StatementKind::Gather(_, _)))
                    .nth(rank)
                    .map(|(at, _)| at);
                if let Some(at) = at {
                    body.statements.remove(at);
                }
            }
            remove_target(edit, &mut now, index)?;
            put_back(edit, &loc, now)?;
        }
    }
    follow_name(edit, &loc, before)?;
    // The name of the node now is not gone, whatever its outputs were.
    if let Some((name, _)) = line_name_at(edit, &loc) {
        gone.remove(&name);
    }
    forget_in_layout(edit, &gone)
}

/// The carry of the loop that `text` names, by its rank among the carries, and its name.
fn carry_named(edit: &Edit, statement: &Statement, structure: &str, text: &str) -> Result<(usize, Name), EditError> {
    let mut held = statement.clone();
    let (clauses, _, _) = clauses_of(&mut held, structure)?;
    let carries: Vec<Name> = clauses
        .iter()
        .filter_map(|clause| match &clause.kind {
            ClauseKind::Carry(carry) => Some(carry.name),
            ClauseKind::In(_) => None,
        })
        .collect();
    let found = edit.names.find(text).and_then(|name| carries.iter().position(|carry| *carry == name));
    let Some((rank, &carry)) = found.and_then(|rank| carries.get(rank).map(|carry| (rank, carry))) else {
        return Err(EditError::new("edit.unknown", format!("`{}` has no carry `{}`", shown(structure), crate::cut(text)))
            .of("part")
            .expecting(carries.iter().map(|carry| format!("carry {}", edit.text(*carry))).collect()));
    };
    Ok((rank, carry))
}

/// The `collect` or `last` of the loop whose output `text` names: the index of its output among those of the loop, and
/// its name.
fn gather_named(edit: &Edit, statement: &Statement, structure: &str, gather: Gather, text: &str) -> Result<(usize, Name), EditError> {
    let mut held = statement.clone();
    clauses_of(&mut held, structure)?;
    let (carries, _) = loop_outputs(statement);
    let StatementKind::Binding { targets, .. } = &statement.kind else { return Err(EditError::internal("a loop that is not one")) };
    let kinds: Vec<Gather> = loop_body(statement)
        .map(|body| {
            body.statements
                .iter()
                .filter_map(|line| match line.kind {
                    StatementKind::Gather(kind, _) => Some(kind),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let gathered: Vec<(usize, Gather, Name)> = targets
        .iter()
        .enumerate()
        .skip(carries)
        .zip(&kinds)
        .filter_map(|((index, target), kind)| match target {
            Target::Name(_, name) => Some((index, *kind, *name)),
            Target::Skip(_) | Target::Next(..) => None,
        })
        .collect();
    let found = edit.names.find(text).and_then(|name| gathered.iter().find(|(_, kind, found)| *kind == gather && *found == name));
    let Some(&(index, _, output)) = found else {
        let word = gather.word();
        return Err(EditError::new("edit.unknown", format!("`{}` has no `{word}` of output `{}`", shown(structure), crate::cut(text)))
            .of("part")
            .expecting(gathered.iter().map(|(_, kind, name)| format!("{} {}", kind.word(), edit.text(*name))).collect()));
    };
    Ok((index, output))
}

/// Moves the output at `from` to `to` among the outputs of the structure, keeping its target.
fn move_target(statement: &mut Statement, from: usize, to: usize) -> Result<(), EditError> {
    let StatementKind::Binding { targets, .. } = &mut statement.kind else { return Err(EditError::internal("a structure that is not one")) };
    let mut list = std::mem::take(targets).into_vec();
    if from < list.len() {
        let target = list.remove(from);
        let to = to.min(list.len());
        list.insert(to, target);
    }
    *targets = list.into_boxed_slice();
    Ok(())
}

/// Forgets the entries of `layout` of the wires that end at `node`: those to its input `terminal`, or, with `from`, those
/// from that value to an input without a name. The routes of an input that goes.
fn forget_routes(edit: &mut Edit, node: Option<(Name, bool)>, terminal: Option<Name>, from: Option<Name>) {
    let (Some((node, is_label)), Some(layout)) = (node, &mut edit.file.layout) else { return };
    layout.items.retain(|item| match &item.kind {
        LayoutKind::Wire { from: start, to, .. } if to.node.name == node && to.node.is_label == is_label => match (terminal, from) {
            (Some(terminal), _) => to.terminal != Some(terminal),
            (None, Some(value)) => !(to.terminal.is_none() && !start.node.is_label && start.node.name == value),
            (None, None) => true,
        },
        _ => true,
    });
}

/// `structure.set_passage`: a carry, a `collect` or a `last` of a loop becomes another of the three, its output kept
/// (decision of the owner, 2026-11-18).
pub(crate) fn set_passage(
    edit: &mut Edit,
    judge: &dyn Judge,
    structure: &str,
    part: &str,
    to: crate::Passage,
    edited: &mut Edited,
) -> Result<(), EditError> {
    let (loc, statement) = structure_at(edit, structure)?;
    let mut held = statement.clone();
    let (_, _, parallel) = clauses_of(&mut held, structure)?;
    let before = line_name_at(edit, &loc);
    let holder = crate::wire::statement_node(&statement);
    let wanted = match to {
        crate::Passage::Carry => None,
        crate::Passage::Collect => Some(Gather::Collect),
        crate::Passage::Last => Some(Gather::Last),
    };
    let output = match (part_of(part), wanted) {
        // The part is found first: one that the loop does not have is `edit.unknown`, whatever `to` says.
        (Part::Carry(text), None) => {
            carry_named(edit, &statement, structure, &text)?;
            return Err(EditError::invalid("to", format!("`carry {}` is a carry already", crate::cut(&text))));
        }
        (Part::Gather(gather, text), Some(kind)) if gather == kind => {
            gather_named(edit, &statement, structure, gather, &text)?;
            return Err(EditError::invalid("to", format!("`{} {}` is a `{}` already", gather.word(), crate::cut(&text), gather.word())));
        }
        (Part::Carry(text), Some(gather)) => carry_into_gather(edit, judge, &loc, structure, &text, gather, holder)?,
        (Part::Gather(gather, text), Some(kind)) => {
            let (index, output) = gather_named(edit, &statement, structure, gather, &text)?;
            let (carries, _) = loop_outputs(&statement);
            let mut now = statement.clone();
            let line = loop_body_mut(&mut now)
                .and_then(|body| body.statements.iter_mut().filter(|line| matches!(line.kind, StatementKind::Gather(_, _))).nth(index - carries));
            let Some(Statement { kind: StatementKind::Gather(found, _), .. }) = line else {
                return Err(EditError::internal("a `collect` that went"));
            };
            *found = kind;
            put_back(edit, &loc, now)?;
            Some(output)
        }
        (Part::Gather(gather, text), None) => {
            gather_named(edit, &statement, structure, gather, &text)?;
            if parallel {
                return Err(EditError::invalid("to", format!("`{}` is a `for parallel`, which takes no carry (spec 6.2)", shown(structure))));
            }
            Some(gather_into_carry(edit, judge, &loc, &statement, structure, gather, &text, holder)?)
        }
        (Part::Arm(_) | Part::In(_), _) => {
            return Err(EditError::invalid(
                "part",
                "only a carry, a `collect` or a `last` changes how the loop takes it out: `carry x`, `collect y`, `last y`",
            ));
        }
    };
    follow_name(edit, &loc, before)?;
    if let Some(output) = output {
        edited.names.insert("output", edit.text(output).to_owned());
    }
    Ok(())
}

/// A carry into a `collect` or a `last`: its `next x = v` becomes `collect v` in its place, a node that binds `next x`
/// binds a value named after the carry, which a new line after it reads. The name of the output, none for `_`: a carry
/// and its output may have two names (`total = for …, carry acc = 0 { … }`, spec 6.2).
fn carry_into_gather(
    edit: &mut Edit,
    judge: &dyn Judge,
    loc: &Loc,
    structure: &str,
    text: &str,
    gather: Gather,
    holder: Option<(Name, bool)>,
) -> Result<Option<Name>, EditError> {
    let statement = scope::statement(edit.body_function()?, loc).cloned().ok_or_else(|| EditError::internal("a loop that went"))?;
    let (rank, carry) = carry_named(edit, &statement, structure, text)?;
    let output = match &statement.kind {
        StatementKind::Binding { targets, .. } => match targets.get(rank) {
            Some(Target::Name(_, name)) => Some(*name),
            _ => None,
        },
        _ => None,
    };
    let function = edit.body_function()?.clone();
    let read_inside = scope::values(&function)
        .into_iter()
        .filter(|value| value.name == carry && matches!(&value.bound, Bound::Head(at, _) if at == loc))
        .any(|value| !readers(&function, &value, &[]).is_empty());
    if read_inside {
        let name = crate::cut(edit.text(carry));
        return Err(EditError::invalid(
            "part",
            format!("the body of `{}` reads the carry `{name}`: a `{}` has no value of the iteration before", shown(structure), gather.word()),
        ));
    }
    let next_lines = |statement: &Statement| -> Vec<(usize, bool, Option<Name>)> {
        loop_body(statement)
            .map(|body| {
                body.statements
                    .iter()
                    .enumerate()
                    .filter_map(|(index, line)| match &line.kind {
                        StatementKind::Binding { targets, value }
                            if targets.iter().any(|target| matches!(target, Target::Next(_, name) if *name == carry)) =>
                        {
                            Some((index, targets.len() == 1 && matches!(value, Rhs::Operand(_)), line.label.map(|label| label.name)))
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    // A line `#n: next x = v` that becomes `collect v` loses its label, which a `collect` takes not (spec 5.1): the
    // `after:` that name it lose it too, and so does `layout`.
    if let Some(&(_, true, Some(label))) = next_lines(&statement).first() {
        let now = edit.body_function()?.clone();
        let found = label_readers(&now, label);
        take_out(edit, judge, label, true, &found)?;
        forget_in_layout(edit, &HashSet::from([label]))?;
    }
    let statement = scope::statement(edit.body_function()?, loc).cloned().ok_or_else(|| EditError::internal("a loop that went"))?;
    let statement = &statement;
    // The places of the `next` of the carry: a line of its own, or a node that binds it among its outputs.
    let lines: Vec<(usize, bool)> = next_lines(statement).into_iter().map(|(index, alone, _)| (index, alone)).collect();
    if lines.len() > 1 {
        let name = crate::cut(edit.text(carry));
        return Err(EditError::invalid(
            "part",
            format!("the carry `{name}` has {} `next` lines, which `check` refuses (R-6): keep one first", lines.len()),
        ));
    }
    let named = match lines.first() {
        Some(&(_, false)) => Some(edit.intern(&fresh(edit.text(carry), &taken(edit)))),
        _ => None,
    };
    let (line_id, path_id, hole_id) = (edit.id(), edit.id(), edit.id());
    let mut now = statement.clone();
    let body = loop_body_mut(&mut now).ok_or_else(|| EditError::internal("a carry of what is not a loop"))?;
    let gathered = |value: Operand| Statement { id: line_id, comments: Attached::default(), label: None, kind: StatementKind::Gather(gather, value) };
    let at = match (lines.first(), named) {
        (Some(&(index, true)), _) => {
            let line = body.statements.get_mut(index).ok_or_else(|| EditError::internal("a `next` that went"))?;
            let StatementKind::Binding { value: Rhs::Operand(value), .. } = &line.kind else {
                return Err(EditError::internal("a `next` that is not one"));
            };
            line.kind = StatementKind::Gather(gather, value.clone());
            line.label = None;
            index
        }
        (Some(&(index, false)), Some(named)) => {
            if let Some(StatementKind::Binding { targets, .. }) = body.statements.get_mut(index).map(|line| &mut line.kind) {
                for target in targets.iter_mut() {
                    if let Target::Next(id, name) = target
                        && *name == carry
                    {
                        *target = Target::Name(*id, named);
                    }
                }
            }
            let at = (index + 1).min(body.statements.len());
            body.statements.insert(at, gathered(Operand::Path(Path { id: path_id, first: named, rest: Box::new([]) })));
            at
        }
        _ => {
            body.statements.push(gathered(Operand::Hole(hole_id)));
            body.statements.len() - 1
        }
    };
    let before_it = body.statements.iter().take(at).filter(|line| matches!(line.kind, StatementKind::Gather(_, _))).count();
    let (clauses, _, _) = clauses_of(&mut now, structure)?;
    clauses.retain(|clause| !matches!(&clause.kind, ClauseKind::Carry(found) if found.name == carry));
    let (carries, _) = loop_outputs(statement);
    move_target(&mut now, rank, carries.saturating_sub(1) + before_it)?;
    put_back(edit, loc, now)?;
    // Its input goes, outside (its initial value) and inside (its `next`): so do their routes.
    forget_routes(edit, holder, Some(carry), None);
    Ok(output)
}

/// A `collect` or a `last` into a carry: `carry y: T = ?` (`carry y = none` for an `error`), after the other carries, as
/// `wire.connect` writes it with `through`, and the line becomes `next y = v` in its place. The name of the output.
#[allow(clippy::too_many_arguments)]
fn gather_into_carry(
    edit: &mut Edit,
    judge: &dyn Judge,
    loc: &Loc,
    statement: &Statement,
    structure: &str,
    gather: Gather,
    text: &str,
    holder: Option<(Name, bool)>,
) -> Result<Name, EditError> {
    let (index, output) = gather_named(edit, statement, structure, gather, text)?;
    let (carries, _) = loop_outputs(statement);
    let function = edit.body_function()?.clone();
    // The carry takes the name of the output (R-3): a value of the body, or a clause, of that name would be defined
    // twice in the body.
    let body_path = loc.sub(0);
    let own = |value: &Value| match &value.bound {
        Bound::Head(at, _) => at == loc,
        Bound::Line(at, _) => at.body == body_path,
        Bound::Input => false,
    };
    if scope::values(&function).iter().any(|value| value.name == output && own(value)) {
        let name = crate::cut(edit.text(output));
        return Err(EditError::invalid(
            "part",
            format!("the body of `{}` has a value `{name}` already: a carry `{name}` would define it twice; rename one first", shown(structure)),
        ));
    }
    // Its type, as the check saw the output: that of the value, in an array for a `collect`.
    let found = scope::values(&function)
        .into_iter()
        .find(|value| matches!(&value.bound, Bound::Line(at, i) if at == loc && *i == index))
        .map(|value| crate::surround::type_of(edit, judge, &value))
        .filter(|found| found.kind != crate::TypeKind::Unknown && !found.shown.is_empty())
        .map(|found| found.shown);
    let shown_type = match (gather, found) {
        (Gather::Collect, Some(text)) => text.strip_prefix('[').and_then(|inner| inner.strip_suffix(']')).map(str::to_owned),
        (_, found) => found,
    };
    let (ty, initial) = if shown_type.as_deref() == Some("error") {
        (None, Operand::None(edit.id()))
    } else {
        let ty = shown_type.as_deref().and_then(|text| read_type(edit, text));
        if let (Some(text), true) = (shown_type.as_deref(), ty.is_some()) {
            use_library_of(edit, text);
        }
        (ty, Operand::Hole(edit.id()))
    };
    let (carry_id, next_id) = (edit.id(), edit.id());
    let mut now = statement.clone();
    let body = loop_body_mut(&mut now).ok_or_else(|| EditError::internal("a `collect` of what is not a loop"))?;
    let line = body
        .statements
        .iter_mut()
        .filter(|line| matches!(line.kind, StatementKind::Gather(_, _)))
        .nth(index - carries)
        .ok_or_else(|| EditError::internal("a `collect` that went"))?;
    let StatementKind::Gather(_, value) = &line.kind else { return Err(EditError::internal("a `collect` that is not one")) };
    let value = value.clone();
    let read = match &value {
        Operand::Path(path) if path.rest.is_empty() => Some(path.first),
        _ => None,
    };
    line.kind = StatementKind::Binding { targets: Box::new([Target::Next(next_id, output)]), value: Rhs::Operand(value) };
    let (clauses, _, _) = clauses_of(&mut now, structure)?;
    clauses.push(Clause { comments: Attached::default(), kind: ClauseKind::Carry(Carry { id: carry_id, name: output, ty, value: initial }) });
    move_target(&mut now, index, carries)?;
    put_back(edit, loc, now)?;
    // The route of the wire to the `collect`, which ends at the carry now: that of a value of the loop, made in its body
    // or given by its head (an `in` clause), whose entry names that wire; a value from outside keeps its route to the
    // edge, which it still crosses.
    let made_inside = read.is_some_and(|name| {
        scope::values(&function).iter().any(|value| {
            value.name == name
                && match &value.bound {
                    Bound::Line(at, _) => at.body.starts_with(&body_path),
                    Bound::Head(at, _) => at == loc,
                    Bound::Input => false,
                }
        })
    });
    // An entry without a terminal names the wire to an input without a name (spec 11): when the value reaches another
    // one (a `last`, an `until`), the entry may be its route, and stays.
    let unnamed = read.map_or(0, |name| {
        loop_body(statement).map_or(0, |body| {
            body.statements
                .iter()
                .filter(|line| match &line.kind {
                    StatementKind::Gather(_, operand) | StatementKind::Until(operand) => crate::places::reads(operand, name),
                    _ => false,
                })
                .count()
        })
    });
    if made_inside && unnamed == 1 {
        forget_routes(edit, holder, None, read);
    }
    Ok(output)
}

/// `structure.set_option`.
pub(crate) fn set_option(edit: &mut Edit, structure: &str, option: StructureOption, on: bool) -> Result<(), EditError> {
    let (loc, mut statement) = structure_at(edit, structure)?;
    match (&mut statement.kind, option) {
        (StatementKind::Binding { value: Rhs::Loop(repeat), .. }, StructureOption::ContinueOnError) => repeat.continue_on_error = on,
        (StatementKind::Binding { value: Rhs::For(each), .. }, StructureOption::Parallel) => {
            if on && each.clauses.iter().any(|clause| matches!(clause.kind, ClauseKind::Carry(_))) {
                return Err(EditError::invalid(
                    "on",
                    format!("`{}` has a carry: a `for parallel` takes none (spec 6.2); remove it first", shown(structure)),
                ));
            }
            each.parallel = on;
        }
        (_, StructureOption::ContinueOnError) => {
            return Err(EditError::invalid("option", format!("`continue_on_error` is an option of a `loop`, and `{}` is not one", shown(structure))));
        }
        (_, StructureOption::Parallel) => {
            return Err(EditError::invalid("option", format!("`parallel` is an option of a `for`, and `{}` is not one", shown(structure))));
        }
    }
    put_back(edit, &loc, statement)
}
