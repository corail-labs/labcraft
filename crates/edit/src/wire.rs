//! `wire.connect` and `wire.disconnect` (commands.md 6.7). A wire is a name read at a place (spec 11): connecting writes
//! the value at the input, disconnecting takes it out as `node.delete` does. A value wired to an output of the function
//! takes the name of the output, and gives it back when unwired (visual notation 2.4).

use std::collections::{BTreeSet, HashMap, HashSet};

use labcraft_graph::{Arg, Function, Label, LayoutKind, Operand, OrderRef, Path, Rhs, Statement, StatementKind, Target};
use labcraft_types::{Code, Diagnostic, Name};

use crate::delete::{Reader, callee, take_out};
use crate::model::Edit;
use crate::names::{fresh, of_output, rename, taken};
use crate::places::{Slot, args, operand, operand_mut, reads, slots};
use crate::refs::{InputName, NodeName, Part, node, split_wire};
use crate::scope::{self, BodyPath, Bound, Frame, Loc, Value};
use crate::{Callable, EditError, Edited, Judge, Kind, Passage, Renamed, Through};

/// The errors of `check` that a wire of the wrong type makes, where it goes and downstream (cli.md 5).
const TYPE_CODES: &[Code] = &[
    Code::Lc3001,
    Code::Lc3002,
    Code::Lc3003,
    Code::Lc3004,
    Code::Lc3005,
    Code::Lc3006,
    Code::Lc3007,
    Code::Lc3008,
    Code::Lc3013,
    Code::Lc3014,
    Code::Lc3015,
    Code::Lc3020,
];

/// Where a wire ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum End {
    /// A place that the node has already, a hole or a value.
    Slot(Loc, Slot),
    /// A parameter that the call does not give yet, by its index in the signature; for a variadic one, its value at `at`.
    NewArg { loc: Loc, param: usize, variadic: Option<usize> },
    /// The `after:` of a call (R-15).
    After(Loc),
    /// A field of the template of a `string.format` that the call does not give yet, by its name (spec 5.4).
    Field { loc: Loc, field: String },
    /// An output of the function, a terminal of its frame.
    Output(Name),
    /// The `next` of the carry `carry` of the loop at `loc`, which no line of its body sets yet: a line `next x = …` comes
    /// (spec 6.2).
    Next { loc: Loc, carry: Name },
    /// The `next` of the carry `carry`, which a node of the loop body at `loc` binds as one of its outputs.
    NextBound { loc: Loc, carry: Name },
}

impl End {
    fn loc(&self) -> Option<&Loc> {
        match self {
            End::Slot(loc, _) | End::NewArg { loc, .. } | End::After(loc) | End::Field { loc, .. } | End::Next { loc, .. } => Some(loc),
            End::NextBound { .. } | End::Output(_) => None,
        }
    }
}

/// The line `next carry = value`, at the end of the body of the loop at `loc`.
pub(crate) fn add_next(edit: &mut Edit, loc: &Loc, carry: Name, value: Operand) -> Result<(), EditError> {
    let (id, target) = (edit.id(), edit.id());
    let statement = Statement {
        id,
        comments: labcraft_graph::Attached::default(),
        label: None,
        kind: StatementKind::Binding { targets: Box::new([Target::Next(target, carry)]), value: Rhs::Operand(value) },
    };
    crate::add::push(edit, &loc.sub(0), statement)
}

/// Where a wire starts.
#[derive(Clone, Debug)]
enum Start {
    Value(Value),
    /// A label, for an order wire.
    Label(Name),
}

/// How `wire.connect` writes a wire: over what the input holds, of the wrong type, out of the loops it leaves.
pub(crate) struct How<'a> {
    pub replace: bool,
    pub accept_mismatch: bool,
    pub through: Option<&'a Through>,
}

pub(crate) fn connect(
    edit: &mut Edit,
    judge: &dyn Judge,
    from: &str,
    to: &str,
    arm: Option<&str>,
    how: &How,
    edited: &mut Edited,
) -> Result<(), EditError> {
    let (replace, accept_mismatch) = (how.replace, how.accept_mismatch);
    let function = edit.body_function()?.clone();
    let end = resolve_end(edit, judge, &function, &InputName::parse(to, "to")?, Some(from), arm)?;
    let reader = reader_body(&function, &end);
    // `node.output` (`#lbl.out`, `@41.out` too) names one value, where its node is: the structures around it take that
    // value out, not another of its name (9.2). An output with no name gets one there.
    let mut chosen = None;
    let named;
    let from = match from.split_once('.') {
        Some((node_text, _)) if !node_text.is_empty() => {
            let loc = node(edit, &function, &NodeName::parse(node_text), "from")?;
            match resolve_start(edit, judge, &function, from, &loc.body, edited)? {
                Start::Value(value) => {
                    named = edit.text(value.name).to_owned();
                    chosen = Some(value);
                    named.as_str()
                }
                Start::Label(_) => from,
            }
        }
        _ => from,
    };
    // An order wire does not leave a structure (R-17).
    if matches!(end, End::After(_)) && how.through.is_some() {
        return Err(EditError::new("wire.out_of_scope", "an order wire does not leave a structure (R-17): `through` takes no `after`").of("through"));
    }
    let lifted = lift(edit, judge, from, chosen.clone(), &reader, how.through, edited)?;
    let from = lifted.as_deref().unwrap_or(from);
    let function = edit.body_function()?.clone();
    // The value that `node.output` named, when nothing lifted it: that one, not another of its name that the reader sees.
    let start = match (&lifted, chosen) {
        (None, Some(value)) if !scope::sees(&function, &value, &reader) => return Err(unseen(&function, from, &value, &reader)),
        (None, Some(value)) => Start::Value(value),
        _ => resolve_start(edit, judge, &function, from, &reader, edited)?,
    };
    let function = edit.body_function()?.clone();
    if let (Start::Label(_), false) = (&start, matches!(end, End::After(_))) {
        return Err(EditError::invalid("from", format!("`{from}` is a label: a wire from a label is an order wire, to `node.after`")));
    }
    // The statement that makes what the wire carries: the line of a value, or the node of a label.
    let made_by = match &start {
        Start::Value(value) => match &value.bound {
            Bound::Line(loc, _) => Some(loc.clone()),
            Bound::Input | Bound::Head(..) => None,
        },
        Start::Label(label) => scope::labels(&function).into_iter().find(|(found, _)| found == label).map(|(_, loc)| loc),
    };
    // A `next` is read by its loop: a value that the loop makes, or that depends on it, would close a cycle; one of its body
    // comes back through the carry (spec 6.2).
    let reader_loc = match &end {
        End::NextBound { loc, .. } => loop_of(loc).ok(),
        _ => end.loc().cloned(),
    };
    if let (Some(made_by), Some(loc)) = (&made_by, reader_loc.as_ref())
        && closes_a_cycle(&function, made_by, loc)
    {
        return Err(EditError::new(
            "wire.cycle",
            format!("a wire from `{from}` to `{to}` would close a cycle: a value comes back only through a carry (spec 6.2)"),
        )
        .of("to"));
    }
    match &end {
        End::Output(output) => {
            let Start::Value(value) = start else { return Err(EditError::internal("a label to an output")) };
            to_output(edit, judge, &value, *output, replace, edited)?;
        }
        End::After(loc) => {
            let reference = match &start {
                Start::Value(value) => OrderRef::Value(Operand::Path(Path { id: edit.id(), first: value.name, rest: Box::new([]) })),
                Start::Label(label) => OrderRef::Label(Label { id: edit.id(), name: *label }),
            };
            let id = edit.id();
            let args = call_args(edit, loc)?;
            let mut list = std::mem::take(args).into_vec();
            let same = |held: &OrderRef| match (held, &reference) {
                (OrderRef::Label(held), OrderRef::Label(given)) => held.name == given.name,
                (OrderRef::Value(Operand::Path(held)), OrderRef::Value(Operand::Path(given))) => held.first == given.first && held.rest.is_empty(),
                _ => false,
            };
            match list.iter_mut().find_map(|arg| match arg {
                Arg::After { refs, .. } => Some(refs),
                Arg::Value { .. } => None,
            }) {
                // The node waits for it already: the wire is there, and nothing changes.
                Some(refs) if refs.iter().any(same) => {}
                Some(refs) => refs.push(reference),
                None => list.push(Arg::After { id, refs: vec![reference] }),
            }
            *args = list.into_boxed_slice();
        }
        End::Slot(loc, slot) => {
            let Start::Value(value) = &start else { return Err(EditError::internal("a label to a place")) };
            let statement = scope::statement(&function, loc).ok_or_else(|| EditError::internal("a node that went"))?;
            let held = operand(statement, *slot).cloned();
            if let Some(held) = &held
                && !matches!(held, Operand::Hole(_))
            {
                if !replace {
                    return Err(occupied(edit, to, held));
                }
                if let Operand::Path(path) = held {
                    forget_wire(edit, path.first, statement_node(statement), input_name(edit, judge, statement, *slot).as_deref());
                }
            }
            let id = edit.id();
            let function_mut = edit.body_function_mut()?;
            let place = scope::statement_mut(function_mut, loc).and_then(|statement| operand_mut(statement, *slot));
            let Some(place) = place else { return Err(EditError::internal("a place that went")) };
            *place = Operand::Path(Path { id, first: value.name, rest: Box::new([]) });
        }
        End::NewArg { loc, param, variadic } => {
            let Start::Value(value) = &start else { return Err(EditError::internal("a label to a parameter")) };
            let statement = scope::statement(&function, loc).cloned().ok_or_else(|| EditError::internal("a node that went"))?;
            let callable =
                callee(edit, judge, &statement).ok_or_else(|| EditError::internal("a parameter of a call that the check does not know"))?;
            let id = edit.id();
            add_arg(edit, loc, &callable, *param, variadic.is_some(), Operand::Path(Path { id, first: value.name, rest: Box::new([]) }))?;
        }
        End::Field { loc, field } => {
            let Start::Value(value) = &start else { return Err(EditError::internal("a label to a field")) };
            let id = edit.id();
            add_field(edit, loc, field, Operand::Path(Path { id, first: value.name, rest: Box::new([]) }))?;
        }
        End::Next { loc, carry } => {
            let Start::Value(value) = &start else { return Err(EditError::internal("a label to a carry")) };
            let id = edit.id();
            add_next(edit, loc, *carry, Operand::Path(Path { id, first: value.name, rest: Box::new([]) }))?;
        }
        End::NextBound { loc, carry } => {
            let Start::Value(value) = &start else { return Err(EditError::internal("a label to a carry")) };
            if !replace {
                return Err(EditError::new(
                    "wire.occupied",
                    format!("`{to}` is set by the node on line {}: pass `replace` to replace it", edit.lines.get(loc).copied().unwrap_or(0)),
                )
                .of("to"));
            }
            unbind_next(edit, loc, *carry)?;
            let id = edit.id();
            add_next(edit, &loop_of(loc)?, *carry, Operand::Path(Path { id, first: value.name, rest: Box::new([]) }))?;
        }
    }
    judge_types(edit, judge, &format!("a wire from `{from}` to `{to}`"), accept_mismatch)
}

/// A value inside structures that the reader does not see: each structure around it, up to the body that holds the reader
/// or a structure around it, gives it as a new output (commands.md 6.7, `wire.connect`), so that the reader sees it. A
/// `match` or a `handle` yields it from the arm that holds it, and a hole from the other arms; a loop takes it out as
/// `through` says, a carry that holds its last value or a `collect`. The name that the reader then reads, or none when
/// there is nothing to lift (a loop left without `through` is refused later).
#[allow(clippy::too_many_arguments)]
fn lift(
    edit: &mut Edit,
    judge: &dyn Judge,
    from: &str,
    chosen: Option<Value>,
    reader: &[scope::Step],
    through: Option<&Through>,
    edited: &mut Edited,
) -> Result<Option<String>, EditError> {
    let no_loop = || {
        if through.is_some() {
            Err(EditError::invalid(
                "through",
                format!("a wire from `{}` leaves no loop: `through` is for a value that a loop holds", crate::cut(from)),
            ))
        } else {
            Ok(None)
        }
    };
    let function = edit.body_function()?;
    let values: Vec<Value> = match chosen {
        Some(value) => vec![value],
        None if from.starts_with('#') || from.contains('.') => return no_loop(),
        None => {
            let Some(name) = edit.names.find(from) else { return no_loop() };
            scope::values(function).into_iter().filter(|value| value.name == name).collect()
        }
    };
    // A value that the reader sees is wired as it is; with `through`, the one inside a loop is meant, even when an outer
    // value of its name is seen too (R-3).
    let seen = |value: &Value| scope::sees(function, value, reader);
    if values.iter().any(seen) && (through.is_none() || values.len() == 1) {
        return no_loop();
    }
    // Up to the body that both see: the reader's own, or the one around a sibling structure that holds the reader.
    let common = |seen_in: &[scope::Step]| seen_in.iter().zip(reader).take_while(|(a, b)| a == b).count();
    let deeper: Vec<Value> = values.into_iter().filter(|value| !seen(value) && value.seen_in.len() > common(&value.seen_in)).collect();
    // A name of several bodies names none of them (9.2).
    if deeper.len() > 1 {
        return Err(EditError::new(
            "edit.ambiguous",
            format!("`{from}` names {} values in several bodies: name the output by its node (`node.output`) or give it another name", deeper.len()),
        )
        .of("from"));
    }
    let Some(value) = deeper.into_iter().next() else {
        return no_loop();
    };
    let around = common(&value.seen_in);
    let mut frames = Vec::new();
    for depth in around + 1..=value.seen_in.len() {
        match scope::frame(function, value.seen_in.get(..depth).unwrap_or(&[])) {
            Some(frame) => frames.push(frame),
            None => return Ok(None),
        }
    }
    let loops = frames.iter().filter(|frame| **frame == Frame::Loop).count();
    if loops == 0 && through.is_some() {
        return no_loop();
    }
    let passages: Vec<Passage> = match through {
        None if loops > 0 => return Ok(None),
        None => Vec::new(),
        Some(Through::All(passage)) => vec![*passage; loops],
        Some(Through::Each(list)) if list.len() == loops => list.clone(),
        Some(Through::Each(list)) => {
            return Err(EditError::invalid(
                "through",
                format!("a wire from `{from}` leaves {loops} loop(s), and `through` lists {}: one passage per loop, from the inside out", list.len()),
            ));
        }
    };
    // Its type, for a carry, as the check saw it; a `collect` puts it in an array.
    // Only a carry writes a type.
    let mut ty = if passages.contains(&Passage::Carry) {
        let found = type_now(edit, judge, &value);
        (found.kind != crate::TypeKind::Unknown && !found.shown.is_empty()).then_some(found.shown)
    } else {
        None
    };
    let mut passages = passages.into_iter();
    let mut current = value.name;
    let mut path = value.seen_in.clone();
    while path.len() > around {
        let depth = path.len() - around - 1;
        let Some(step) = path.pop() else { break };
        let loc = Loc { body: path.clone(), index: step.stmt };
        current = match frames.get(depth) {
            Some(Frame::Arm) => give(edit, &loc, step.sub, current)?,
            Some(Frame::Loop) => {
                let passage = passages.next().ok_or_else(|| EditError::internal("a loop without its passage"))?;
                let taken_out = pass(edit, &loc, current, passage, ty.as_deref())?;
                if passage == Passage::Collect {
                    ty = ty.map(|inner| format!("[{inner}]"));
                }
                taken_out
            }
            None => return Err(EditError::internal("a frame that went")),
        };
    }
    let given = edit.text(current).to_owned();
    edited.names.insert("output", given.clone());
    Ok(Some(given))
}

/// The type of a value as the check sees it: in the text before the command, or, for a value that the command named
/// (an output that was `_`), in the text as it is now, where the value is found by its name.
fn type_now(edit: &Edit, judge: &dyn Judge, value: &Value) -> crate::ValueType {
    let found = crate::surround::type_of(edit, judge, value);
    if found.kind != crate::TypeKind::Unknown {
        return found;
    }
    let Ok(text) = edit.render() else { return found };
    let Ok(again) = Edit::read(&edit.path, &text, edit.limits()) else { return found };
    let name = edit.text(value.name);
    let (Some(function), Some(wanted)) = (again.function(), again.names.find(name)) else { return found };
    let line = scope::values(function).into_iter().find_map(|value| match &value.bound {
        Bound::Line(loc, _) | Bound::Head(loc, _) if value.name == wanted => again.lines.get(loc).copied(),
        _ => None,
    });
    match line {
        Some(line) => judge.value_type(&edit.path, &text, line, name),
        None => found,
    }
}

/// The loop at `loc` takes `value` out: a carry of its type that holds its last value (`carry v2: T = ?`, `next v2 = v`),
/// the carry and its output named after it (N-14), or a `collect`, its output named after it. The name of the output.
fn pass(edit: &mut Edit, loc: &Loc, value: Name, passage: Passage, ty: Option<&str>) -> Result<Name, EditError> {
    let output = fresh(edit.text(value), &taken(edit));
    let output = edit.intern(&output);
    let statement = scope::statement(edit.body_function()?, loc).ok_or_else(|| EditError::internal("a loop that went"))?;
    let parallel = matches!(&statement.kind, StatementKind::Binding { value: Rhs::For(each), .. } if each.parallel);
    match passage {
        Passage::Carry => {
            if parallel {
                return Err(EditError::invalid("through", "a `for parallel` takes no carry (spec 6.2): it takes a value out as a `collect`"));
            }
            let written = ty;
            let ty = ty.and_then(|text| crate::parts::read_type(edit, text));
            if let (Some(text), true) = (written, ty.is_some()) {
                crate::parts::use_library_of(edit, text);
            }
            let hole = Operand::Hole(edit.id());
            crate::parts::push_carry(edit, loc, output, ty, hole)?;
            let id = edit.id();
            add_next(edit, loc, output, Operand::Path(Path { id, first: value, rest: Box::new([]) }))?;
        }
        Passage::Collect => {
            let id = edit.id();
            crate::parts::push_collect(edit, loc, output, Operand::Path(Path { id, first: value, rest: Box::new([]) }))?;
        }
    }
    Ok(output)
}

/// The `match` or `handle` at `loc` gives `value` from its arm `arm`, as a new output named after it (N-14): the other
/// arms yield a hole for it (9.1). The name of the output.
pub(crate) fn give(edit: &mut Edit, loc: &Loc, arm: usize, value: Name) -> Result<Name, EditError> {
    let output = fresh(edit.text(value), &taken(edit));
    let output = edit.intern(&output);
    let target_id = edit.id();
    let mut holes = Vec::new();
    let function = edit.body_function()?;
    let arms = match scope::statement(function, loc).map(|statement| &statement.kind) {
        Some(StatementKind::Binding { value: Rhs::Match(branching) | Rhs::Handle(branching), .. }) => branching.arms.len(),
        _ => return Err(EditError::internal("a value lifted out of what is not an arm")),
    };
    for _ in 0..arms {
        holes.push(edit.id());
    }
    let value_id = edit.id();
    let function = edit.body_function_mut()?;
    let Some(StatementKind::Binding { targets, value: Rhs::Match(branching) | Rhs::Handle(branching) }) =
        scope::statement_mut(function, loc).map(|statement| &mut statement.kind)
    else {
        return Err(EditError::internal("a structure that went"));
    };
    // A structure that gives nothing has `_` alone as its target, and no `yield` (spec 6.1).
    let gives_nothing = branching.gives_nothing();
    let mut list = if gives_nothing { Vec::new() } else { std::mem::take(targets).into_vec() };
    list.push(Target::Name(target_id, output));
    *targets = list.into_boxed_slice();
    for (rank, (each, hole)) in branching.arms.iter_mut().zip(holes).enumerate() {
        let given = if rank == arm { Operand::Path(Path { id: value_id, first: value, rest: Box::new([]) }) } else { Operand::Hole(hole) };
        each.yields.values.push(given);
    }
    Ok(output)
}

/// The loop whose body holds the statement at `loc`.
pub(crate) fn loop_of(loc: &Loc) -> Result<Loc, EditError> {
    let (last, outer) = loc.body.split_last().ok_or_else(|| EditError::internal("a `next` outside a loop"))?;
    Ok(Loc { body: outer.to_vec(), index: last.stmt })
}

/// The node at `loc` no longer binds `next carry`: that output becomes `_`.
pub(crate) fn unbind_next(edit: &mut Edit, loc: &Loc, carry: Name) -> Result<(), EditError> {
    let id = edit.id();
    let function = edit.body_function_mut()?;
    let Some(StatementKind::Binding { targets, .. }) = scope::statement_mut(function, loc).map(|statement| &mut statement.kind) else {
        return Err(EditError::internal("a node that went"));
    };
    for target in targets.iter_mut() {
        if matches!(target, Target::Next(_, found) if *found == carry) {
            *target = Target::Skip(id);
        }
    }
    Ok(())
}

pub(crate) fn disconnect(edit: &mut Edit, judge: &dyn Judge, wire: &str, arm: Option<&str>, edited: &mut Edited) -> Result<(), EditError> {
    let function = edit.body_function()?.clone();
    // The entry of a wire to a `next`, named after the loop and its carry (spec 11).
    let forget_next = |edit: &mut Edit, loc: &Loc, carry: Name| {
        let from = split_wire(wire).ok().and_then(|(from, _)| edit.names.find(&from));
        let holder = loop_of(loc).ok().and_then(|outer| scope::statement(&function, &outer)).and_then(statement_node);
        if let Some(from) = from {
            let carry = edit.text(carry).to_owned();
            forget_wire(edit, from, holder, Some(&carry));
        }
    };
    match find_in_arm(edit, judge, &function, wire, arm)? {
        // `next x = v`: the line goes, and the carry has no `next` until it is wired (commands.md 6.7).
        Found::NextLine(loc) => {
            let carry = scope::statement(&function, &loc).and_then(|statement| match &statement.kind {
                StatementKind::Binding { targets, .. } => targets.iter().find_map(|target| match target {
                    Target::Next(_, carry) => Some(*carry),
                    _ => None,
                }),
                _ => None,
            });
            let body = scope::body_mut(edit.body_function_mut()?, &loc.body).ok_or_else(|| EditError::internal("a loop that went"))?;
            if loc.index < body.statements.len() {
                body.statements.remove(loc.index);
            }
            if let Some(carry) = carry {
                forget_next(edit, &loc, carry);
            }
            Ok(())
        }
        Found::NextBound { loc, carry } => {
            unbind_next(edit, &loc, carry)?;
            forget_next(edit, &loc, carry);
            Ok(())
        }
        Found::Alias(index) => {
            edit.body_function_mut()?.body.statements.remove(index);
            Ok(())
        }
        Found::Bound { output, value } => {
            let back = node_name_of(edit, judge, &function, &value);
            let back_name = edit.intern(&back);
            rename(edit, &value, back_name)?;
            edited.renamed.push(Renamed { file: edit.path.clone(), from: output, to: back });
            Ok(())
        }
        Found::Reader { name, label, reader, input, node } => {
            take_out(edit, judge, name, label, std::slice::from_ref(&reader))?;
            let input = (!input.is_empty()).then_some(input.as_str());
            forget_wire(edit, name, node, input);
            Ok(())
        }
    }
}

/// A wire that exists in the function, as `wire.disconnect` finds it.
pub(crate) enum Found {
    /// `o = v`, the line at this index of the body.
    Alias(usize),
    /// A value bound by the name of the output `output`.
    Bound { output: String, value: Value },
    /// The place of a node that reads the value (or the label) `name`, by its input `input` (empty when it has no name).
    Reader { name: Name, label: bool, reader: Reader, input: String, node: Option<(Name, bool)> },
    /// The line `next x = v` of a loop, at `loc`.
    NextLine(Loc),
    /// The node at `loc` of a loop body, which binds `next carry` as one of its outputs.
    NextBound { loc: Loc, carry: Name },
}

/// Finds the wire that a name names, changing nothing: what `wire.disconnect` takes out, or why it would refuse it.
pub(crate) fn find(edit: &Edit, judge: &dyn Judge, function: &Function, wire: &str) -> Result<Found, EditError> {
    find_in_arm(edit, judge, function, wire, None)
}

/// [`find`], with the arm of a `yield` (`arm`).
pub(crate) fn find_in_arm(edit: &Edit, judge: &dyn Judge, function: &Function, wire: &str, arm: Option<&str>) -> Result<Found, EditError> {
    let (from, to) = split_wire(wire)?;
    let end_name = InputName::parse(&to, "wire")?;
    // An output: the wire of `o = v`, or of a value that bears the name of the output.
    let output = match &end_name {
        InputName::Output(text) => Some(text.clone()),
        InputName::Node(name, Part::Alone) if !name.is_label && is_output(edit, function, &name.text) => {
            let alias = alias_of(edit, function, &name.text).is_some_and(|(_, value)| edit.text(value) == from);
            (alias || from == name.text || refs_find_none(edit, function, name)).then(|| name.text.clone())
        }
        InputName::Node(..) => None,
    };
    if let Some(output) = output {
        return from_output(edit, function, &from, &output);
    }
    if let Some(found) = structure_wire(edit, judge, function, &from, &to, arm)? {
        return Ok(found);
    }
    let InputName::Node(node_name, part) = end_name else { return Err(EditError::internal("an output")) };
    let loc = node(edit, function, &node_name, "wire")?;
    let statement = scope::statement(function, &loc).cloned().ok_or_else(|| EditError::internal("a node that went"))?;
    let (name, label) = match from.strip_prefix('#') {
        Some(label) => (edit.names.find(label), true),
        None => (edit.names.find(&from), false),
    };
    let no_wire = || EditError::new("edit.unknown", format!("no wire `{wire}` in this function")).of("wire");
    let Some(name) = name else { return Err(no_wire()) };
    let mut found: Vec<(Reader, String)> = Vec::new();
    if label {
        if after_holds(&statement, name, true) {
            found.push((Reader::After(loc.clone()), "after".to_owned()));
        }
    } else {
        for (slot, _) in slots(&statement, &loc) {
            if operand(&statement, slot).is_some_and(|operand| reads(operand, name)) {
                found.push((Reader::Slot(loc.clone(), slot), input_name(edit, judge, &statement, slot).unwrap_or_default()));
            }
        }
        if after_holds(&statement, name, false) {
            found.push((Reader::After(loc.clone()), "after".to_owned()));
        }
    }
    let chosen: Vec<(Reader, String)> = match &part {
        Part::Alone => found,
        Part::Name(input) => found.into_iter().filter(|(_, named)| named == input).collect(),
        // The selector of a `match` or a `handle` is its input without a name, at place 0 (9.2).
        Part::Index(index) => found
            .into_iter()
            .filter(|(reader, named)| *named == index.to_string() || (*index == 0 && matches!(reader, Reader::Slot(_, Slot::Selector))))
            .collect(),
        Part::Variadic(param, index) => found.into_iter().filter(|(_, named)| *named == format!("{param}.{index}")).collect(),
        Part::Next(_) => return Err(step_five("wire", "the `next` of a loop")),
    };
    match chosen.as_slice() {
        [] => Err(no_wire()),
        [(reader, input)] => Ok(Found::Reader { name, label, reader: reader.clone(), input: input.clone(), node: statement_node(&statement) }),
        // The `yield` of several arms: named by `arm` (9.2).
        several if several.iter().all(|(reader, _)| matches!(reader, Reader::Slot(_, Slot::Yield { .. }))) => {
            let arms = match &statement.kind {
                StatementKind::Binding { value: Rhs::Match(branching) | Rhs::Handle(branching), .. } => crate::frame::arm_names(edit, branching),
                _ => Vec::new(),
            };
            let named: Vec<String> = several
                .iter()
                .filter_map(|(reader, _)| match reader {
                    Reader::Slot(_, Slot::Yield { arm, .. }) => arms.get(*arm).cloned(),
                    _ => None,
                })
                .collect();
            Err(EditError::new(
                "edit.ambiguous",
                format!("`{from}` reaches the `yield` of several arms of `{}`: name the arm with `arm`", node_name.shown()),
            )
            .of("arm")
            .expecting(named))
        }
        several => Err(EditError::new("edit.ambiguous", format!("`{from}` reaches `{}` by several inputs: name the input", node_name.shown()))
            .of("wire")
            .expecting(several.iter().map(|(_, input)| format!("{from} -> {}.{input}", node_name.shown())).collect())),
    }
}

/// A wire to a part of a structure: a `yield` (with `arm`), a `collect`, the `until` of a loop or the `next` of a carry,
/// as `wire.connect` names them (commands.md 9.2); none for a wire to anything else.
fn structure_wire(edit: &Edit, judge: &dyn Judge, function: &Function, from: &str, to: &str, arm: Option<&str>) -> Result<Option<Found>, EditError> {
    let name = InputName::parse(to, "wire")?;
    let InputName::Node(node_name, part) = &name else { return Ok(None) };
    // A `yield` named without its arm: the one arm whose `yield` reads the value, when the selector does not (9.2).
    let inferred: Option<String> = match (arm, part) {
        (None, Part::Alone) if !node_name.is_label && node_name.line.is_none() => {
            let value = edit.names.find(from);
            structures_giving(edit, function, &node_name.text).into_iter().find_map(|(loc, index)| {
                let statement = scope::statement(function, &loc)?;
                let StatementKind::Binding { value: Rhs::Match(branching) | Rhs::Handle(branching), .. } = &statement.kind else { return None };
                let value = value?;
                if reads(&branching.selector, value) {
                    return None;
                }
                let reading: Vec<usize> = branching
                    .arms
                    .iter()
                    .enumerate()
                    .filter(|(_, each)| each.yields.values.get(index).is_some_and(|operand| reads(operand, value)))
                    .map(|(rank, _)| rank)
                    .collect();
                match reading.as_slice() {
                    [one] => crate::frame::arm_names(edit, branching).get(*one).cloned(),
                    _ => None,
                }
            })
        }
        _ => None,
    };
    let arm = arm.or(inferred.as_deref());
    let is_loop = || {
        crate::refs::find_nodes(edit, function, node_name).iter().any(|loc| {
            scope::statement(function, loc).is_some_and(|statement| matches!(statement.kind, StatementKind::Binding { value: Rhs::Loop(_), .. }))
        })
    };
    let alone_collect = *part == Part::Alone
        && !node_name.is_label
        && node_name.line.is_none()
        && collect_end(edit, function, &node_name.text).ok().flatten().is_some();
    let until = *part == Part::Alone || *part == Part::Name("until".to_owned());
    let structural = arm.is_some() || matches!(part, Part::Next(_)) || alone_collect || (until && is_loop());
    if !structural {
        return Ok(None);
    }
    let end = resolve_end(edit, judge, function, &name, None, arm).map_err(|error| match error.parameter {
        Some("to") => error.of("wire"),
        _ => error,
    })?;
    let no_wire = || EditError::new("edit.unknown", format!("no wire `{from} -> {to}` in this function")).of("wire");
    let Some(value) = edit.names.find(from) else { return Err(no_wire()) };
    match end {
        End::Slot(loc, slot) => {
            let statement = scope::statement(function, &loc).ok_or_else(|| EditError::internal("a node that went"))?;
            if !operand(statement, slot).is_some_and(|operand| reads(operand, value)) {
                return Err(no_wire());
            }
            if slot == Slot::Value {
                return Ok(Some(Found::NextLine(loc)));
            }
            // The layout names the wire after the structure: the node that holds the `collect` or the `until`.
            let holder = match slot {
                Slot::Collect | Slot::Until => loop_of(&loc).ok().and_then(|outer| scope::statement(function, &outer)).and_then(statement_node),
                _ => statement_node(statement),
            };
            Ok(Some(Found::Reader { name: value, label: false, reader: Reader::Slot(loc, slot), input: String::new(), node: holder }))
        }
        End::NextBound { loc, carry } => {
            // The node binds `next x` itself: the drawing names that wire after the carry (`t -> n.next.t`), or after the
            // node.
            let statement = scope::statement(function, &loc).ok_or_else(|| EditError::internal("a node that went"))?;
            let named = statement_node(statement).map(|(name, _)| name);
            if value != carry && named != Some(value) {
                return Err(no_wire());
            }
            Ok(Some(Found::NextBound { loc, carry }))
        }
        _ => Err(no_wire()),
    }
}

/// Whether nothing is a node of that name, so that a bare name can only be an output.
fn refs_find_none(edit: &Edit, function: &Function, name: &NodeName) -> bool {
    crate::refs::find_nodes(edit, function, name).is_empty()
}

fn after_holds(statement: &Statement, name: Name, label: bool) -> bool {
    args(statement).is_some_and(|args| {
        args.iter().any(|arg| match arg {
            Arg::After { refs, .. } => refs.iter().any(|reference| match reference {
                OrderRef::Value(operand) => !label && reads(operand, name),
                OrderRef::Label(reference) => label && reference.name == name,
            }),
            Arg::Value { .. } => false,
        })
    })
}

pub(crate) fn step_five(parameter: &'static str, what: &str) -> EditError {
    EditError::invalid(parameter, format!("{what} is edited from M1 step 5 (structures)"))
}

pub(crate) fn occupied(edit: &Edit, to: &str, held: &Operand) -> EditError {
    let what = labcraft_format::write_operand(held, &edit.names, 60);
    EditError::new("wire.occupied", format!("`{to}` already holds `{what}`: pass `replace` to replace it")).of("to")
}

/// The name of the node of a statement, as `layout` names it.
pub(crate) fn statement_node(statement: &Statement) -> Option<(Name, bool)> {
    statement.label.map(|label| (label.name, true)).or_else(|| scope::line_name(statement).map(|name| (name, false)))
}

/// The name of an input of a node, as the end of a wire names it after the node (spec 11): `b`, `0`, `parts.2`.
pub(crate) fn input_name(edit: &Edit, judge: &dyn Judge, statement: &Statement, slot: Slot) -> Option<String> {
    match slot {
        Slot::Arg(index) => {
            let args = args(statement)?;
            match args.get(index)? {
                Arg::Value { name: Some(name), .. } => Some(edit.text(*name).to_owned()),
                Arg::Value { name: None, .. } => {
                    let position = args.iter().take(index).filter(|arg| matches!(arg, Arg::Value { name: None, .. })).count();
                    let callable = callee(edit, judge, statement)?;
                    match callable.kind {
                        Kind::Variant | Kind::Struct => Some(position.to_string()),
                        Kind::Function => {
                            let variadic_at = callable.inputs.iter().position(|input| input.variadic);
                            match variadic_at {
                                Some(at) if position >= at => callable.inputs.get(at).map(|input| format!("{}.{}", input.name, position - at)),
                                _ => callable.inputs.get(position).map(|input| input.name.clone()),
                            }
                        }
                    }
                }
                Arg::After { .. } => Some("after".to_owned()),
            }
        }
        Slot::Field(index) => match &statement.kind {
            StatementKind::Binding { value: Rhs::Struct(construction), .. } => {
                construction.fields.get(index).map(|field| edit.text(field.name).to_owned())
            }
            _ => None,
        },
        Slot::Element(index) => Some(index.to_string()),
        Slot::Source(index) | Slot::Init(index) | Slot::End(index) => match &statement.kind {
            StatementKind::Binding { value: Rhs::For(each), .. } => each.clauses.get(index).map(|clause| clause_name(edit, clause)),
            StatementKind::Binding { value: Rhs::Loop(repeat), .. } => repeat.clauses.get(index).map(|clause| clause_name(edit, clause)),
            _ => None,
        },
        Slot::Value | Slot::Selector | Slot::Yield { .. } | Slot::Collect | Slot::Until => None,
    }
}

fn clause_name(edit: &Edit, clause: &labcraft_graph::Clause) -> String {
    match &clause.kind {
        labcraft_graph::ClauseKind::In(clause) => edit.text(clause.name).to_owned(),
        labcraft_graph::ClauseKind::Carry(carry) => edit.text(carry.name).to_owned(),
    }
}

fn is_output(edit: &Edit, function: &Function, name: &str) -> bool {
    function.signature.outputs.iter().any(|output| edit.text(output.name) == name)
}

/// The line `o = v` of an output, and the value it names.
fn alias_of(edit: &Edit, function: &Function, output: &str) -> Option<(usize, Name)> {
    function.body.statements.iter().enumerate().find_map(|(index, statement)| match &statement.kind {
        StatementKind::Binding { targets, value: Rhs::Operand(Operand::Path(path)) }
            if path.rest.is_empty() && matches!(targets.as_ref(), [Target::Name(_, name)] if edit.text(*name) == output) =>
        {
            Some((index, path.first))
        }
        _ => None,
    })
}

/// The line that binds an output by its name, other than `o = v`: `voltage = core.with_unit(…)`.
fn bound_by_name(edit: &Edit, function: &Function, output: &str) -> Option<Value> {
    scope::values(function).into_iter().find(|value| {
        value.seen_in.is_empty()
            && edit.text(value.name) == output
            && match &value.bound {
                Bound::Line(loc, _) => scope::statement(function, loc).is_some_and(|statement| {
                    !matches!(&statement.kind, StatementKind::Binding { value: Rhs::Operand(Operand::Path(path)), .. } if path.rest.is_empty())
                }),
                Bound::Input | Bound::Head(..) => false,
            }
    })
}

/// The name that a node would have had, had it not been bound to an output (N-14).
pub(crate) fn node_name_of(edit: &Edit, judge: &dyn Judge, function: &Function, value: &Value) -> String {
    let taken = taken(edit);
    let Bound::Line(loc, index) = &value.bound else { return fresh("value", &taken) };
    let base = scope::statement(function, loc)
        .and_then(|statement| {
            let callable = callee(edit, judge, statement)?;
            let StatementKind::Binding { value: Rhs::Call(call), .. } = &statement.kind else { return None };
            let text = edit_node_text(edit, call);
            of_output(&callable, &text, *index)
        })
        .or_else(|| {
            scope::statement(function, loc).and_then(|statement| match &statement.kind {
                StatementKind::Binding { value: Rhs::Struct(construction), .. } => {
                    Some(crate::names::of_type(edit.text(construction.path.segments().last().unwrap_or(construction.path.first))))
                }
                StatementKind::Binding { value: Rhs::Operand(_), .. } => Some("constant".to_owned()),
                _ => None,
            })
        })
        .unwrap_or_else(|| "value".to_owned());
    fresh(&base, &taken)
}

/// Wires a value to an output of the function (visual notation 2.4).
fn to_output(edit: &mut Edit, judge: &dyn Judge, value: &Value, output: Name, replace: bool, edited: &mut Edited) -> Result<(), EditError> {
    let function = edit.body_function()?.clone();
    let output_text = edit.text(output).to_owned();
    if value.name == output {
        return Ok(());
    }
    // What the output holds already.
    if let Some((index, held)) = alias_of(edit, &function, &output_text) {
        if !replace {
            let what = edit.text(held).to_owned();
            return Err(
                EditError::new("wire.occupied", format!("the output `{output_text}` already gets `{what}`: pass `replace` to replace it")).of("to")
            );
        }
        edit.body_function_mut()?.body.statements.remove(index);
    } else if let Some(bound) = bound_by_name(edit, &function, &output_text) {
        if !replace {
            return Err(EditError::new(
                "wire.occupied",
                format!("the output `{output_text}` is already given by its node: pass `replace` to replace it"),
            )
            .of("to"));
        }
        // The old node takes back the name that inserting it would have given (2.4).
        let back = node_name_of(edit, judge, &function, &bound);
        let back_name = edit.intern(&back);
        rename(edit, &bound, back_name)?;
        edited.renamed.push(Renamed { file: edit.path.clone(), from: output_text.clone(), to: back });
    }
    let function = edit.body_function()?.clone();
    let value =
        scope::values(&function).into_iter().find(|other| other.name == value.name && other.bound == value.bound).unwrap_or_else(|| value.clone());
    let is_input = |name: Name| function.signature.params.iter().any(|param| param.name == name);
    let other_output = function.signature.outputs.iter().any(|other| other.name == value.name);
    let has_readers = !crate::delete::readers(&function, &value, &[]).is_empty();
    let line_of_body = matches!(&value.bound, Bound::Line(loc, _) if loc.body.is_empty());
    // A body inside that binds the name of the output for itself would read its own value instead (R-3).
    let hidden = scope::values(&function).iter().any(|other| other.name == output);
    let fold = line_of_body && !other_output && !hidden && !(is_input(output) && has_readers);
    if fold {
        rename(edit, &value, output)?;
        edited.renamed.push(Renamed { file: edit.path.clone(), from: edit.text(value.name).to_owned(), to: output_text });
    } else {
        let id = edit.id();
        let path_id = edit.id();
        let line = Statement {
            id,
            comments: labcraft_graph::Attached::default(),
            label: None,
            kind: StatementKind::Binding {
                targets: Box::new([Target::Name(edit.id(), output)]),
                value: Rhs::Operand(Operand::Path(Path { id: path_id, first: value.name, rest: Box::new([]) })),
            },
        };
        edit.body_function_mut()?.body.statements.push(line);
    }
    Ok(())
}

/// The wire of a value to an output of the function: `o = v`, which unwiring takes out, or a node bound by the name of
/// the output, which unwiring gives back the name that inserting it would have given (2.4).
fn from_output(edit: &Edit, function: &Function, from: &str, output: &str) -> Result<Found, EditError> {
    let no_wire = || EditError::new("edit.unknown", format!("no wire `{from} -> {output}` in this function")).of("wire");
    if !is_output(edit, function, output) {
        return Err(EditError::new("edit.unknown", format!("`{output}` is no output of this function")).of("wire"));
    }
    if let Some((index, held)) = alias_of(edit, function, output) {
        if edit.text(held) != from {
            return Err(no_wire());
        }
        return Ok(Found::Alias(index));
    }
    if from != output {
        return Err(no_wire());
    }
    let Some(value) = bound_by_name(edit, function, output) else { return Err(no_wire()) };
    Ok(Found::Bound { output: output.to_owned(), value })
}

/// Removes the entries of `layout` for the wire of `value` to a node: the one that names the input, and the one that
/// names none when the value no longer reaches the node.
pub(crate) fn forget_wire(edit: &mut Edit, value: Name, node: Option<(Name, bool)>, input: Option<&str>) {
    let Some((node, is_label)) = node else { return };
    let input = input.map(|text| edit.names.find(text));
    let still_reaches = edit.function().is_some_and(|function| {
        scope::all(function).into_iter().any(|loc| {
            scope::statement(function, &loc).is_some_and(|statement| {
                statement_node(statement) == Some((node, is_label))
                    && (slots(statement, &loc).into_iter().any(|(slot, _)| operand(statement, slot).is_some_and(|operand| reads(operand, value)))
                        || after_holds(statement, value, false))
            })
        })
    });
    let Some(layout) = &mut edit.file.layout else { return };
    layout.items.retain(|item| match &item.kind {
        LayoutKind::Wire { from, to, .. } => {
            let this = from.node.name == value && to.node.name == node && to.node.is_label == is_label;
            let named_input = match (to.terminal, input) {
                (Some(terminal), Some(Some(input))) => terminal == input,
                _ => false,
            };
            !(this && (named_input || (to.terminal.is_none() && !still_reaches)))
        }
        LayoutKind::Node { .. } => true,
    });
}

/// The arguments of the call at `loc`.
pub(crate) fn call_args<'e>(edit: &'e mut Edit, loc: &Loc) -> Result<&'e mut Box<[Arg]>, EditError> {
    let function = edit.body_function_mut()?;
    scope::statement_mut(function, loc).and_then(crate::places::args_mut).ok_or_else(|| EditError::internal("a call that went"))
}

/// Adds the argument of a parameter that the call does not give. Positional arguments come first (R-11): the value goes
/// in its position when every parameter before it is given by position, by name otherwise. A value of a variadic parameter
/// is positional: the parameters before it are given by position first, a hole where nothing feeds them (9.1).
pub(crate) fn add_arg(edit: &mut Edit, loc: &Loc, callable: &Callable, param: usize, variadic: bool, operand: Operand) -> Result<(), EditError> {
    let names: Vec<Name> = callable.inputs.iter().map(|input| edit.intern(&input.name)).collect();
    let mut ids = Vec::with_capacity(param + 2);
    for _ in 0..=param + 1 {
        ids.push(edit.id());
    }
    let args = call_args(edit, loc)?;
    let mut list = std::mem::take(args).into_vec();
    let positional = list.iter().take_while(|arg| matches!(arg, Arg::Value { name: None, .. })).count();
    let value_id = ids.pop().unwrap_or_else(|| labcraft_graph_id_fallback(&list));
    let named_only = callable.inputs.get(param).is_some_and(|input| input.named_only);
    if variadic || (positional == param && !named_only) {
        let target = if variadic { callable.inputs.iter().position(|input| input.variadic).unwrap_or(param) } else { param };
        // The parameters before it, by position: a named argument moves to its place, a hole fills the others.
        let mut head: Vec<Arg> = list.drain(..positional).collect();
        for (index, name) in names.iter().enumerate().take(target).skip(positional) {
            let named = list.iter().position(|arg| matches!(arg, Arg::Value { name: Some(given), .. } if given == name));
            let moved = match named {
                Some(at) => match list.remove(at) {
                    Arg::Value { id, value, .. } => Arg::Value { id, name: None, value },
                    other => other,
                },
                None => {
                    let id = ids.get(index).copied().unwrap_or(value_id);
                    Arg::Value { id, name: None, value: Operand::Hole(id) }
                }
            };
            head.push(moved);
        }
        head.push(Arg::Value { id: value_id, name: None, value: operand });
        head.extend(list);
        list = head;
    } else {
        let name = names.get(param).copied().ok_or_else(|| EditError::internal("a parameter past the signature"))?;
        list.push(Arg::Value { id: value_id, name: Some(name), value: operand });
    }
    *args = list.into_boxed_slice();
    Ok(())
}

fn labcraft_graph_id_fallback(list: &[Arg]) -> labcraft_graph::Id {
    list.first().map(crate::places::arg_id).unwrap_or_else(|| {
        labcraft_graph::SourceMap::default().add(labcraft_types::Span {
            start: labcraft_types::Position { line: 1, column: 1 },
            end: labcraft_types::Position { line: 1, column: 1 },
        })
    })
}

/// The body that a reader at this end sees from.
fn reader_body(function: &Function, end: &End) -> BodyPath {
    match end {
        End::Slot(loc, slot) => scope::statement(function, loc)
            .and_then(|statement| slots(statement, loc).into_iter().find(|(found, _)| found == slot).map(|(_, seen_in)| seen_in))
            .unwrap_or_else(|| loc.body.clone()),
        End::NewArg { loc, .. } | End::After(loc) | End::Field { loc, .. } | End::NextBound { loc, .. } => loc.body.clone(),
        End::Next { loc, .. } => loc.sub(0),
        End::Output(_) => Vec::new(),
    }
}

/// What the end of a wire names; `arm`, the arm of a `yield`.
pub(crate) fn resolve_end(
    edit: &Edit,
    judge: &dyn Judge,
    function: &Function,
    name: &InputName,
    from: Option<&str>,
    arm: Option<&str>,
) -> Result<End, EditError> {
    if let (Some(arm), InputName::Node(node_name, part)) = (arm, name) {
        return yield_end(edit, function, node_name, part, arm);
    }
    if arm.is_some() {
        return Err(EditError::invalid("arm", "`arm` names the arm of a `yield`: `to` names then the output of its structure"));
    }
    let (node_name, part) = match name {
        InputName::Output(text) => {
            let output = function.signature.outputs.iter().find(|output| edit.text(output.name) == text.as_str());
            return match output {
                Some(output) => Ok(End::Output(output.name)),
                None => Err(EditError::new("edit.unknown", format!("`{text}` is no output of this function")).of("to")),
            };
        }
        InputName::Node(node_name, part) => (node_name, part),
    };
    // A bare name that is an output: the output, unless a node of that name is where the value goes (spec 11).
    if !node_name.is_label && *part == Part::Alone && is_output(edit, function, &node_name.text) {
        let nodes = crate::refs::find_nodes(edit, function, node_name);
        if nodes.is_empty() || from == Some(node_name.text.as_str()) {
            let output = function.signature.outputs.iter().find(|output| edit.text(output.name) == node_name.text);
            if let Some(output) = output {
                return Ok(End::Output(output.name));
            }
        }
    }
    // A `collect`, by the output of the loop that it gives (9.2).
    if !node_name.is_label
        && node_name.line.is_none()
        && *part == Part::Alone
        && let Some(end) = collect_end(edit, function, &node_name.text)?
    {
        return Ok(end);
    }
    let loc = node(edit, function, node_name, "to")?;
    let statement = scope::statement(function, &loc).ok_or_else(|| EditError::internal("a node that went"))?;
    let unknown = |what: &str, known: Vec<String>| {
        EditError::new("edit.unknown", format!("`{}` has no input {what}", node_name.shown())).of("to").expecting(known)
    };
    if let Part::Next(carry) = part {
        return next_end(edit, function, &loc, carry).ok_or_else(|| unknown(&format!("`next.{carry}`: it has no carry `{carry}`"), Vec::new()));
    }
    if let Part::Name(input) = part
        && input == "after"
    {
        return match args(statement) {
            Some(_) => Ok(End::After(loc)),
            None => Err(unknown("`after`: only a call has one", Vec::new())),
        };
    }
    match &statement.kind {
        StatementKind::Call(call) | StatementKind::Binding { value: Rhs::Call(call), .. } => {
            if let Part::Name(input) = part
                && edit_node_text(edit, call) == FORMAT
                && input != "template"
                && let Some(field) = format_field(edit, judge, &loc, call, input)
            {
                return Ok(field);
            }
            let callable = callee(edit, judge, statement);
            call_end(edit, &loc, call, callable.as_ref(), part, &unknown)
        }
        StatementKind::Binding { value: Rhs::Struct(construction), .. } => {
            let fields: Vec<String> = construction.fields.iter().map(|field| edit.text(field.name).to_owned()).collect();
            let index = match part {
                Part::Name(input) => fields.iter().position(|field| field == input),
                Part::Alone if fields.len() == 1 => Some(0),
                _ => None,
            };
            index.map(|index| End::Slot(loc.clone(), Slot::Field(index))).ok_or_else(|| unknown("of that name", fields))
        }
        StatementKind::Binding { value: Rhs::Operand(Operand::Array(_, values)), .. } => match part {
            Part::Index(index) if *index < values.len() => Ok(End::Slot(loc.clone(), Slot::Element(*index))),
            Part::Alone if values.len() == 1 => Ok(End::Slot(loc.clone(), Slot::Element(0))),
            _ => Err(unknown("of that place", (0..values.len()).map(|index| format!("{}.{index}", node_name.shown())).collect())),
        },
        StatementKind::Binding { value: Rhs::Match(_) | Rhs::Handle(_), .. } => match part {
            Part::Alone | Part::Index(0) => Ok(End::Slot(loc.clone(), Slot::Selector)),
            _ => Err(unknown("of that name: a `yield` is named by the output of its structure, with `arm`", Vec::new())),
        },
        StatementKind::Binding { value: Rhs::For(each), .. } => {
            let Part::Name(input) = part else { return Err(unknown("of that name", Vec::new())) };
            each.clauses
                .iter()
                .enumerate()
                .find_map(|(index, clause)| match &clause.kind {
                    labcraft_graph::ClauseKind::In(clause) if edit.text(clause.name) == input => Some(End::Slot(loc.clone(), Slot::Source(index))),
                    labcraft_graph::ClauseKind::Carry(carry) if edit.text(carry.name) == input => Some(End::Slot(loc.clone(), Slot::Init(index))),
                    _ => None,
                })
                .ok_or_else(|| unknown("of that name", Vec::new()))
        }
        StatementKind::Binding { value: Rhs::Loop(repeat), .. } => {
            // `n.until` names it too: `until` is a keyword, so no carry bears that name.
            if *part == Part::Alone || *part == Part::Name("until".to_owned()) {
                // The loop alone names its only input without a name: its `until`.
                let until = repeat.body.statements.iter().position(|line| matches!(line.kind, StatementKind::Until(_)));
                return until
                    .map(|index| End::Slot(Loc { body: loc.sub(0), index }, Slot::Until))
                    .ok_or_else(|| unknown("`until`: its body has none", Vec::new()));
            }
            let Part::Name(input) = part else { return Err(unknown("of that name", Vec::new())) };
            repeat
                .clauses
                .iter()
                .enumerate()
                .find_map(|(index, clause)| match &clause.kind {
                    labcraft_graph::ClauseKind::Carry(carry) if edit.text(carry.name) == input => Some(End::Slot(loc.clone(), Slot::Init(index))),
                    _ => None,
                })
                .ok_or_else(|| unknown("of that name", Vec::new()))
        }
        StatementKind::Binding { value: Rhs::Operand(_), .. } => Err(unknown("a wire can reach: it names a value or a constant", Vec::new())),
        StatementKind::Collect(_) | StatementKind::Until(_) => {
            Err(unknown("of that name: a `collect` is named by the output of its loop, an `until` by the loop", Vec::new()))
        }
    }
}

/// The structures of the function that bind `name` among their outputs, with its place there.
fn structures_giving(edit: &Edit, function: &Function, name: &str) -> Vec<(Loc, usize)> {
    let Some(wanted) = edit.names.find(name) else { return Vec::new() };
    let mut found = Vec::new();
    for loc in scope::all(function) {
        let Some(statement) = scope::statement(function, &loc) else { continue };
        let StatementKind::Binding { targets, value: Rhs::Match(_) | Rhs::Handle(_) | Rhs::For(_) | Rhs::Loop(_) } = &statement.kind else {
            continue;
        };
        if let Some(index) = targets.iter().position(|target| matches!(target, Target::Name(_, found) if *found == wanted)) {
            found.push((loc, index));
        }
    }
    found
}

/// The `yield` of `arm` that gives the output named of a `match` or a `handle`: `verdict` with `arm` `true` (9.2).
fn yield_end(edit: &Edit, function: &Function, node_name: &NodeName, part: &Part, arm: &str) -> Result<End, EditError> {
    if *part != Part::Alone {
        return Err(EditError::invalid("to", "a `yield` is named by the output of its structure (`verdict`), with `arm`"));
    }
    let giving = if node_name.is_label || node_name.line.is_some() {
        Vec::new()
    } else {
        structures_giving(edit, function, &node_name.text)
            .into_iter()
            .filter(|(loc, _)| {
                scope::statement(function, loc)
                    .is_some_and(|statement| matches!(statement.kind, StatementKind::Binding { value: Rhs::Match(_) | Rhs::Handle(_), .. }))
            })
            .collect()
    };
    let (loc, index) = match giving.as_slice() {
        [one] => one.clone(),
        [] => (node(edit, function, node_name, "to")?, 0),
        _ => {
            return Err(EditError::new("edit.ambiguous", format!("`{}` is an output of several structures", node_name.shown())).of("to"));
        }
    };
    let statement = scope::statement(function, &loc).ok_or_else(|| EditError::internal("a node that went"))?;
    let StatementKind::Binding { value: Rhs::Match(branching) | Rhs::Handle(branching), .. } = &statement.kind else {
        return Err(EditError::invalid("arm", format!("`{}` is not a `match` nor a `handle`: it has no arm", node_name.shown())));
    };
    let Some(rank) = crate::frame::arm_index(edit, branching, arm) else {
        return Err(EditError::new("edit.unknown", format!("`{}` has no arm `{}`", node_name.shown(), crate::cut(arm)))
            .of("arm")
            .expecting(crate::frame::arm_names(edit, branching)));
    };
    let gives = branching.arms.get(rank).is_some_and(|found| found.yields.values.len() > index);
    if !gives {
        return Err(EditError::invalid(
            "to",
            format!("`{}` gives nothing: a wire from inside one of its arms to outside adds an output", node_name.shown()),
        ));
    }
    Ok(End::Slot(loc, Slot::Yield { arm: rank, index }))
}

/// The `collect` that gives the output `name` of a loop, when one does.
fn collect_end(edit: &Edit, function: &Function, name: &str) -> Result<Option<End>, EditError> {
    let mut found = Vec::new();
    for (loc, index) in structures_giving(edit, function, name) {
        let Some(statement) = scope::statement(function, &loc) else { continue };
        let (clauses, body) = match &statement.kind {
            StatementKind::Binding { value: Rhs::For(each), .. } => (&each.clauses, &each.body),
            // `n` alone names the `until` of `n = loop …` (9.2), even when `n` is also the output of a `collect`.
            StatementKind::Binding { value: Rhs::Loop(_), .. } if scope::line_name(statement).is_some_and(|line| edit.text(line) == name) => continue,
            StatementKind::Binding { value: Rhs::Loop(repeat), .. } => (&repeat.clauses, &repeat.body),
            _ => continue,
        };
        // The outputs of a loop: its carries, then its `collect`, in order (spec 6.2).
        let carries = clauses.iter().filter(|clause| matches!(clause.kind, labcraft_graph::ClauseKind::Carry(_))).count();
        let Some(rank) = index.checked_sub(carries) else { continue };
        let at = body.statements.iter().enumerate().filter(|(_, line)| matches!(line.kind, StatementKind::Collect(_))).nth(rank);
        if let Some((at, _)) = at {
            found.push(End::Slot(Loc { body: loc.sub(0), index: at }, Slot::Collect));
        }
    }
    match found.len() {
        0 | 1 => Ok(found.pop()),
        _ => Err(EditError::new("edit.ambiguous", format!("`{name}` is an output of several loops")).of("to")),
    }
}

/// The `next` of the carry `carry` of the loop at `loc`: the line `next carry = value` that sets it, or the place for one.
fn next_end(edit: &Edit, function: &Function, loc: &Loc, carry: &str) -> Option<End> {
    let statement = scope::statement(function, loc)?;
    let (clauses, body) = match &statement.kind {
        StatementKind::Binding { value: Rhs::For(each), .. } => (&each.clauses, &each.body),
        StatementKind::Binding { value: Rhs::Loop(repeat), .. } => (&repeat.clauses, &repeat.body),
        _ => return None,
    };
    let name = edit.names.find(carry)?;
    clauses.iter().find(|clause| matches!(&clause.kind, labcraft_graph::ClauseKind::Carry(found) if found.name == name))?;
    let sets = |line: &Statement| match &line.kind {
        StatementKind::Binding { targets, .. } => targets.iter().any(|target| matches!(target, Target::Next(_, found) if *found == name)),
        _ => false,
    };
    match body.statements.iter().position(sets) {
        Some(index) => {
            let at = Loc { body: loc.sub(0), index };
            let line = body.statements.get(index)?;
            match &line.kind {
                // `next x = v`: its operand is the place.
                StatementKind::Binding { targets, value: Rhs::Operand(_) } if targets.len() == 1 => Some(End::Slot(at, Slot::Value)),
                // A node binds it: the wire is there, and `replace` takes it.
                _ => Some(End::NextBound { loc: at, carry: name }),
            }
        }
        None => Some(End::Next { loc: loc.clone(), carry: name }),
    }
}

/// The end of a wire on a call: a parameter, a value of a variadic one, a value of a variant.
fn call_end(
    edit: &Edit,
    loc: &Loc,
    call: &labcraft_graph::Call,
    callable: Option<&Callable>,
    part: &Part,
    unknown: &dyn Fn(&str, Vec<String>) -> EditError,
) -> Result<End, EditError> {
    let positional: Vec<usize> =
        call.args.iter().enumerate().filter(|(_, arg)| matches!(arg, Arg::Value { name: None, .. })).map(|(index, _)| index).collect();
    let Some(callable) = callable else {
        // An unknown callee: only what the text holds already.
        return match part {
            Part::Name(input) => call
                .args
                .iter()
                .position(|arg| matches!(arg, Arg::Value { name: Some(name), .. } if edit.text(*name) == input))
                .map(|index| End::Slot(loc.clone(), Slot::Arg(index)))
                .ok_or_else(|| unknown("of that name", Vec::new())),
            _ => Err(unknown("that the check knows", Vec::new())),
        };
    };
    let inputs: Vec<String> = callable.inputs.iter().map(|input| input.name.clone()).collect();
    if callable.kind == Kind::Variant {
        let index = match part {
            Part::Index(index) => Some(*index),
            Part::Alone if callable.inputs.len() == 1 => Some(0),
            _ => None,
        };
        return match index.and_then(|index| positional.get(index)) {
            Some(arg) => Ok(End::Slot(loc.clone(), Slot::Arg(*arg))),
            None => Err(unknown("of that place: a value of a variant is numbered from 0", Vec::new())),
        };
    }
    let param = match part {
        Part::Name(input) => inputs.iter().position(|name| name == input),
        Part::Alone if callable.inputs.len() == 1 => Some(0),
        Part::Alone => {
            return Err(EditError::new("edit.ambiguous", "the node has several inputs: name one".to_owned())
                .of("to")
                .expecting(inputs.iter().map(|input| format!("{}.{input}", edit_node_text(edit, call))).collect()));
        }
        Part::Variadic(name, _) => inputs.iter().position(|input| input == name),
        Part::Index(_) | Part::Next(_) => None,
    };
    let Some(param) = param else { return Err(unknown("of that name", inputs)) };
    let input = callable.inputs.get(param).ok_or_else(|| EditError::internal("a parameter past the signature"))?;
    if input.variadic {
        let Part::Variadic(_, at) = part else {
            return Err(EditError::invalid("to", format!("`{}` is variadic: name a value of it by its place, `….{}.0`", input.name, input.name)));
        };
        let Some(position) = param.checked_add(*at) else {
            return Err(unknown("of that place: the place just after the last value adds one", Vec::new()));
        };
        return match positional.get(position) {
            Some(arg) => Ok(End::Slot(loc.clone(), Slot::Arg(*arg))),
            None if position == positional.len() || (positional.len() <= param && *at == 0) => {
                Ok(End::NewArg { loc: loc.clone(), param, variadic: Some(*at) })
            }
            None => Err(unknown("of that place: the place just after the last value adds one", Vec::new())),
        };
    }
    // Given by name, by position, or not given.
    let param_name = edit.names.find(&input.name);
    if let Some(index) = call.args.iter().position(|arg| matches!(arg, Arg::Value { name: Some(name), .. } if Some(*name) == param_name)) {
        return Ok(End::Slot(loc.clone(), Slot::Arg(index)));
    }
    if !input.named_only
        && let Some(arg) = positional.get(param)
    {
        return Ok(End::Slot(loc.clone(), Slot::Arg(*arg)));
    }
    Ok(End::NewArg { loc: loc.clone(), param, variadic: None })
}

/// The call whose template names its other inputs (spec 5.4).
pub(crate) const FORMAT: &str = "string.format";

/// The template of a `string.format`, as the text writes it: its first argument given by position, or the one named
/// `template`.
pub(crate) fn template_of<'c>(edit: &Edit, call: &'c labcraft_graph::Call) -> Option<(usize, &'c Operand)> {
    call.args.iter().enumerate().find_map(|(index, arg)| match arg {
        Arg::Value { name: None, value, .. } if index == 0 => Some((index, value)),
        Arg::Value { name: Some(name), value, .. } if edit.text(*name) == "template" => Some((index, value)),
        _ => None,
    })
}

/// A field of the template of a `string.format`: the argument that gives it, or the field the call does not give yet.
fn format_field(edit: &Edit, judge: &dyn Judge, loc: &Loc, call: &labcraft_graph::Call, input: &str) -> Option<End> {
    if let Some(index) = call.args.iter().position(|arg| matches!(arg, Arg::Value { name: Some(name), .. } if edit.text(*name) == input)) {
        return Some(End::Slot(loc.clone(), Slot::Arg(index)));
    }
    let Some((_, Operand::Text(_, literal))) = template_of(edit, call) else { return None };
    let fields = judge.template(literal).ok()?;
    fields.iter().any(|field| field == input).then(|| End::Field { loc: loc.clone(), field: input.to_owned() })
}

/// Gives a field of a `string.format` by name, after the other arguments (R-11).
pub(crate) fn add_field(edit: &mut Edit, loc: &Loc, field: &str, operand: Operand) -> Result<(), EditError> {
    let name = edit.intern(field);
    let id = edit.id();
    let args = call_args(edit, loc)?;
    let mut list = std::mem::take(args).into_vec();
    list.push(Arg::Value { id, name: Some(name), value: operand });
    *args = list.into_boxed_slice();
    Ok(())
}

/// The call of a statement when it calls `string.format`.
pub(crate) fn format_call<'s>(edit: &Edit, statement: &'s Statement) -> Option<&'s labcraft_graph::Call> {
    match &statement.kind {
        StatementKind::Call(call) | StatementKind::Binding { value: Rhs::Call(call), .. } => (edit_node_text(edit, call) == FORMAT).then_some(call),
        _ => None,
    }
}

/// What a call names, as a command writes it: `visa.open`.
pub(crate) fn edit_node_text(edit: &Edit, call: &labcraft_graph::Call) -> String {
    let segments: Vec<&str> = call.callee.segments().map(|segment| edit.text(segment)).collect();
    segments.join(".")
}

/// What the start of a wire names, seen from the body of its reader.
fn resolve_start(
    edit: &mut Edit,
    judge: &dyn Judge,
    function: &Function,
    from: &str,
    reader: &[scope::Step],
    edited: &mut Edited,
) -> Result<Start, EditError> {
    // A label alone is an order wire; `#close.out` is an output of the node of that label.
    if let Some(label) = from.strip_prefix('#')
        && !label.contains('.')
    {
        let name = edit.names.find(label).filter(|name| scope::labels(function).iter().any(|(found, _)| found == name));
        return name.map(Start::Label).ok_or_else(|| EditError::new("edit.unknown", format!("no node is labelled `{from}`")).of("from"));
    }
    if let Some((node_text, output)) = from.split_once('.') {
        // `node.output`: an output of a node by its name in the signature; `_` gets a name (N-14).
        let loc = node(edit, function, &NodeName::parse(node_text), "from")?;
        let statement = scope::statement(function, &loc).cloned().ok_or_else(|| EditError::internal("a node that went"))?;
        // A node that is no call: a constant gives `value`, a structure its outputs by their names.
        if let StatementKind::Binding { targets, value } = &statement.kind
            && !matches!(value, Rhs::Call(_))
        {
            let found = targets.iter().enumerate().find_map(|(index, target)| match target {
                Target::Name(_, name) if edit.text(*name) == output || (output == "value" && index == 0 && matches!(value, Rhs::Operand(_))) => {
                    Some((index, *name))
                }
                _ => None,
            });
            let Some((index, name)) = found else {
                return Err(EditError::new("edit.unknown", format!("`{node_text}` has no output `{output}`")).of("from"));
            };
            let value = scope::values(function)
                .into_iter()
                .find(|value| value.name == name && matches!(&value.bound, Bound::Line(at, i) if *at == loc && *i == index))
                .ok_or_else(|| EditError::internal("an output that went"))?;
            if !scope::sees(function, &value, reader) {
                return Err(unseen(function, from, &value, reader));
            }
            return Ok(Start::Value(value));
        }
        let callable = callee(edit, judge, &statement)
            .ok_or_else(|| EditError::new("edit.unknown", format!("the outputs of `{node_text}` are not known")).of("from"))?;
        let Some(index) = callable.outputs.iter().position(|name| name == output) else {
            return Err(EditError::new("edit.unknown", format!("`{node_text}` has no output `{output}`"))
                .of("from")
                .expecting(callable.outputs.clone()));
        };
        let StatementKind::Binding { targets, value: Rhs::Call(call) } = &statement.kind else {
            return Err(EditError::new("edit.unknown", format!("`{node_text}` binds none of its outputs")).of("from"));
        };
        let target = targets.get(index).copied();
        let name = match target {
            Some(Target::Name(_, name)) => name,
            Some(Target::Skip(_)) => {
                let text = edit_node_text(edit, call);
                let base = of_output(&callable, &text, index).unwrap_or_else(|| output.to_owned());
                let given = fresh(&base, &taken(edit));
                let name = edit.intern(&given);
                let id = edit.id();
                let function_mut = edit.body_function_mut()?;
                if let Some(StatementKind::Binding { targets, .. }) = scope::statement_mut(function_mut, &loc).map(|statement| &mut statement.kind)
                    && let Some(slot) = targets.get_mut(index)
                {
                    *slot = Target::Name(id, name);
                }
                edited.names.insert("output", given);
                name
            }
            Some(Target::Next(..)) => return Err(step_five("from", "the `next` of a loop")),
            None => return Err(EditError::new("edit.unknown", format!("`{node_text}` writes fewer targets than its outputs")).of("from")),
        };
        let function = edit.body_function()?;
        let value = scope::values(function)
            .into_iter()
            .find(|value| value.name == name && matches!(&value.bound, Bound::Line(at, i) if *at == loc && *i == index))
            .ok_or_else(|| EditError::internal("an output that went"))?;
        if !scope::sees(function, &value, reader) {
            return Err(unseen(function, from, &value, reader));
        }
        return Ok(Start::Value(value));
    }
    let Some(name) = edit.names.find(from) else {
        let known: Vec<String> =
            scope::values(function).iter().map(|value| edit.text(value.name).to_owned()).collect::<BTreeSet<_>>().into_iter().collect();
        return Err(EditError::new("edit.unknown", format!("no value is named `{from}` in this function"))
            .of("from")
            .expecting(crate::refs::nearest(from, &known)));
    };
    let mut candidates: Vec<Value> = scope::values(function).into_iter().filter(|value| value.name == name).collect();
    // Reading a name that is an input and an output reads the input (R-2): the line of the function that binds it binds
    // the output.
    if candidates.iter().any(|value| matches!(value.bound, Bound::Input)) {
        candidates.retain(|value| !matches!(&value.bound, Bound::Line(loc, _) if loc.body.is_empty()));
    }
    if candidates.is_empty() {
        let known: Vec<String> =
            scope::values(function).iter().map(|value| edit.text(value.name).to_owned()).collect::<BTreeSet<_>>().into_iter().collect();
        return Err(EditError::new("edit.unknown", format!("no value is named `{from}` in this function"))
            .of("from")
            .expecting(crate::refs::nearest(from, &known)));
    }
    let seen: Vec<&Value> = candidates.iter().filter(|value| scope::sees(function, value, reader)).collect();
    match seen.as_slice() {
        [one] => Ok(Start::Value((*one).clone())),
        [] => {
            // Deeper than the reader: out of an arm, or out of a loop.
            let deeper = candidates.iter().find(|value| value.seen_in.len() > reader.len() && value.seen_in.starts_with(reader));
            Err(match deeper.or(candidates.first()) {
                Some(value) => unseen(function, from, value, reader),
                None => EditError::internal("no value to judge"),
            })
        }
        _ => Err(EditError::new("edit.ambiguous", format!("`{from}` names several values that the input sees")).of("from")),
    }
}

/// Why the reader at `reader` does not see `value`: it is inside an arm or a loop around which the reader is, or in
/// another arm or structure.
fn unseen(function: &Function, from: &str, value: &Value, reader: &[scope::Step]) -> EditError {
    let below = (value.seen_in.len() > reader.len() && value.seen_in.starts_with(reader))
        .then(|| scope::frame(function, value.seen_in.get(..=reader.len()).unwrap_or(&[])))
        .flatten();
    match below {
        Some(Frame::Arm) => step_five("from", "a wire out of an arm, which adds an output to its structure,"),
        Some(Frame::Loop) => EditError::new(
            "wire.out_of_scope",
            format!("`{from}` is inside a loop: pass `through`, `carry` (its last value) or `collect` (an array of its values), to take it out"),
        )
        .of("from"),
        _ => EditError::new("wire.out_of_scope", format!("`{from}` cannot be seen from there: it is a value of another arm or structure")).of("from"),
    }
}

/// Whether wiring what the statement `made_by` makes into the statement at `reader` closes a cycle (spec 5.2): that
/// statement holds the reader, or depends on it, in its body.
pub(crate) fn closes_a_cycle(function: &Function, made_by: &Loc, reader: &Loc) -> bool {
    let body_path = &made_by.body;
    let holder = if &reader.body == body_path {
        reader.index
    } else {
        match reader.body.get(body_path.len()) {
            Some(step) if reader.body.starts_with(body_path) => step.stmt,
            _ => return false,
        }
    };
    if holder == made_by.index {
        return true;
    }
    reaches(function, body_path, vec![made_by.index], holder)
}

/// Whether the statement at `at` reads, deep, what depends on itself in its body (spec 5.2): a structure made around
/// nodes that are not next to each other can read a value that reads one of its outputs.
pub(crate) fn depends_on_itself(function: &Function, at: &Loc) -> bool {
    let Some(body) = scope::body(function, &at.body) else { return false };
    let Some(statement) = body.statements.get(at.index) else { return false };
    let inputs: HashSet<Name> = if at.body.is_empty() { function.signature.params.iter().map(|param| param.name).collect() } else { HashSet::new() };
    let reads: HashSet<Name> = deep_reads(statement).into_iter().filter(|name| !inputs.contains(name)).collect();
    let starts: Vec<usize> = body
        .statements
        .iter()
        .enumerate()
        .filter(|(index, other)| {
            *index != at.index
                && (scope::targets(other).into_iter().any(|name| reads.contains(&name))
                    || other.label.is_some_and(|label| reads.contains(&label.name)))
        })
        .map(|(index, _)| index)
        .collect();
    reaches(function, &at.body, starts, at.index)
}

/// Whether a walk from the lines `starts` of a body, through what each reads of the others, deep, reaches the line
/// `holder`.
fn reaches(function: &Function, body_path: &[scope::Step], starts: Vec<usize>, holder: usize) -> bool {
    let Some(body) = scope::body(function, body_path) else { return false };
    // What each line of the body reads of the others, deep. In the body of the function, reading a name that is an input
    // and an output reads the input: the line `err = …` binds the output (R-2), and no line reads it.
    let inputs: HashSet<Name> =
        if body_path.is_empty() { function.signature.params.iter().map(|param| param.name).collect() } else { HashSet::new() };
    let mut bound_by: HashMap<Name, Vec<usize>> = HashMap::new();
    for (index, statement) in body.statements.iter().enumerate() {
        for name in scope::targets(statement).into_iter().filter(|name| !inputs.contains(name)) {
            bound_by.entry(name).or_default().push(index);
        }
        if let Some(label) = statement.label {
            bound_by.entry(label.name).or_default().push(index);
        }
    }
    let mut seen = HashSet::new();
    let mut pending = starts;
    while let Some(index) = pending.pop() {
        if index == holder {
            return true;
        }
        if !seen.insert(index) {
            continue;
        }
        let Some(statement) = body.statements.get(index) else { continue };
        for name in deep_reads(statement) {
            if let Some(at) = bound_by.get(&name) {
                pending.extend(at.iter().copied().filter(|index| !seen.contains(index)));
            }
        }
    }
    false
}

/// Every name that a statement reads, in its sub-bodies too, and the labels of its `after:`.
fn deep_reads(statement: &Statement) -> Vec<Name> {
    let mut found = Vec::new();
    let loc = Loc { body: Vec::new(), index: 0 };
    for (slot, _) in slots(statement, &loc) {
        if let Some(operand) = operand(statement, slot) {
            names_in(operand, &mut found);
        }
    }
    if let Some(args) = args(statement) {
        for arg in args {
            if let Arg::After { refs, .. } = arg {
                for reference in refs {
                    match reference {
                        OrderRef::Value(operand) => names_in(operand, &mut found),
                        OrderRef::Label(label) => found.push(label.name),
                    }
                }
            }
        }
    }
    for (body, _) in scope::subs(statement) {
        for inner in &body.statements {
            found.extend(deep_reads(inner));
        }
    }
    found
}

pub(crate) fn names_in(operand: &Operand, found: &mut Vec<Name>) {
    match operand {
        Operand::Path(path) => found.push(path.first),
        Operand::Array(_, values) => values.iter().for_each(|value| names_in(value, found)),
        Operand::Bracket(_, unit) => {
            if let labcraft_graph::UnitTerm::Named { name, .. } = &unit.first {
                found.push(*name);
            }
        }
        Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) | Operand::Hole(_) => {}
    }
}

/// A wire of the wrong type is refused (`wire.type_mismatch`), unless it is accepted: it then gives `LC3001` (2.4).
/// `what` says what is written: "a wire from `a` to `b`", "the constant `1 V` on `b`".
pub(crate) fn judge_types(edit: &Edit, judge: &dyn Judge, what: &str, accept_mismatch: bool) -> Result<(), EditError> {
    if accept_mismatch {
        return Ok(());
    }
    let after = edit.render()?;
    let typed = |diagnostics: Vec<Diagnostic>| -> Vec<Diagnostic> {
        diagnostics.into_iter().filter(|diagnostic| TYPE_CODES.contains(&diagnostic.code())).collect()
    };
    let mut before: Vec<(Code, String)> =
        typed(judge.errors(&edit.path, &edit.original)).into_iter().map(|diagnostic| (diagnostic.code(), diagnostic.message)).collect();
    for diagnostic in typed(judge.errors(&edit.path, &after)) {
        match before.iter().position(|(code, message)| *code == diagnostic.code() && *message == diagnostic.message) {
            Some(index) => {
                before.remove(index);
            }
            None => {
                let types = match (&diagnostic.found, &diagnostic.expected) {
                    (Some(found), Some(expected)) => format!(": `{found}` where `{expected}` is expected"),
                    _ => format!(": {}", diagnostic.message),
                };
                return Err(EditError::new(
                    "wire.type_mismatch",
                    format!("{what} has the wrong type{types}; pass `accept_mismatch` to write it anyway"),
                )
                .of("to"));
            }
        }
    }
    Ok(())
}
