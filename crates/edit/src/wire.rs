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
use crate::{Callable, EditError, Edited, Judge, Kind, Renamed};

/// The errors of `check` that a wire of the wrong type makes, where it goes and downstream (cli.md 5).
const TYPE_CODES: &[Code] = &[
    Code::Lc3001,
    Code::Lc3002,
    Code::Lc3003,
    Code::Lc3004,
    Code::Lc3005,
    Code::Lc3007,
    Code::Lc3008,
    Code::Lc3013,
    Code::Lc3014,
    Code::Lc3015,
    Code::Lc3020,
];

/// Where a wire ends.
#[derive(Clone, Debug, PartialEq, Eq)]
enum End {
    /// A place that the node has already, a hole or a value.
    Slot(Loc, Slot),
    /// A parameter that the call does not give yet, by its index in the signature; for a variadic one, its value at `at`.
    NewArg { loc: Loc, param: usize, variadic: Option<usize> },
    /// The `after:` of a call (R-15).
    After(Loc),
    /// An output of the function, a terminal of its frame.
    Output(Name),
}

impl End {
    fn loc(&self) -> Option<&Loc> {
        match self {
            End::Slot(loc, _) | End::NewArg { loc, .. } | End::After(loc) => Some(loc),
            End::Output(_) => None,
        }
    }
}

/// Where a wire starts.
#[derive(Clone, Debug)]
enum Start {
    Value(Value),
    /// A label, for an order wire.
    Label(Name),
}

pub(crate) fn connect(
    edit: &mut Edit,
    judge: &dyn Judge,
    from: &str,
    to: &str,
    replace: bool,
    accept_mismatch: bool,
    edited: &mut Edited,
) -> Result<(), EditError> {
    let function = edit.body_function()?.clone();
    let end = resolve_end(edit, judge, &function, &InputName::parse(to, "to")?, Some(from))?;
    let reader = reader_body(&function, &end);
    let start = resolve_start(edit, judge, &function, from, &reader, edited)?;
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
    if let (Some(made_by), Some(loc)) = (&made_by, end.loc())
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
            add_arg(edit, loc, &callable, *param, variadic.is_some(), value.name)?;
        }
    }
    judge_types(edit, judge, from, to, accept_mismatch)
}

pub(crate) fn disconnect(edit: &mut Edit, judge: &dyn Judge, wire: &str, edited: &mut Edited) -> Result<(), EditError> {
    let function = edit.body_function()?.clone();
    match find(edit, judge, &function, wire)? {
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
}

/// Finds the wire that a name names, changing nothing: what `wire.disconnect` takes out, or why it would refuse it.
pub(crate) fn find(edit: &Edit, judge: &dyn Judge, function: &Function, wire: &str) -> Result<Found, EditError> {
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
        Part::Index(index) => found.into_iter().filter(|(_, named)| *named == index.to_string()).collect(),
        Part::Variadic(param, index) => found.into_iter().filter(|(_, named)| *named == format!("{param}.{index}")).collect(),
        Part::Next(_) => return Err(step_five("wire", "the `next` of a loop")),
    };
    match chosen.as_slice() {
        [] => Err(no_wire()),
        [(reader, input)] => Ok(Found::Reader { name, label, reader: reader.clone(), input: input.clone(), node: statement_node(&statement) }),
        several => Err(EditError::new("edit.ambiguous", format!("`{from}` reaches `{}` by several inputs: name the input", node_name.shown()))
            .of("wire")
            .expecting(several.iter().map(|(_, input)| format!("{from} -> {}.{input}", node_name.shown())).collect())),
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

fn step_five(parameter: &'static str, what: &str) -> EditError {
    EditError::invalid(parameter, format!("{what} is edited from M1 step 5 (structures)"))
}

fn occupied(edit: &Edit, to: &str, held: &Operand) -> EditError {
    let what = labcraft_format::write_operand(held, &edit.names, 60);
    EditError::new("wire.occupied", format!("`{to}` already holds `{what}`: pass `replace` to replace it")).of("to")
}

/// The name of the node of a statement, as `layout` names it.
fn statement_node(statement: &Statement) -> Option<(Name, bool)> {
    statement.label.map(|label| (label.name, true)).or_else(|| scope::line_name(statement).map(|name| (name, false)))
}

/// The name of an input of a node, as the end of a wire names it after the node (spec 11): `b`, `0`, `parts.2`.
fn input_name(edit: &Edit, judge: &dyn Judge, statement: &Statement, slot: Slot) -> Option<String> {
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
fn node_name_of(edit: &Edit, judge: &dyn Judge, function: &Function, value: &Value) -> String {
    let taken = taken(edit);
    let Bound::Line(loc, index) = &value.bound else { return fresh("value", &taken) };
    let base = scope::statement(function, loc)
        .and_then(|statement| {
            let callable = callee(edit, judge, statement)?;
            let StatementKind::Binding { value: Rhs::Call(call), .. } = &statement.kind else { return None };
            let text: Vec<&str> = call.callee.segments().map(|segment| edit.text(segment)).collect();
            of_output(&callable, &text.join("."), *index)
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
fn forget_wire(edit: &mut Edit, value: Name, node: Option<(Name, bool)>, input: Option<&str>) {
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
fn call_args<'e>(edit: &'e mut Edit, loc: &Loc) -> Result<&'e mut Box<[Arg]>, EditError> {
    let function = edit.body_function_mut()?;
    scope::statement_mut(function, loc).and_then(crate::places::args_mut).ok_or_else(|| EditError::internal("a call that went"))
}

/// Adds the argument of a parameter that the call does not give. Positional arguments come first (R-11): the value goes
/// in its position when every parameter before it is given by position, by name otherwise. A value of a variadic parameter
/// is positional: the parameters before it are given by position first, a hole where nothing feeds them (9.1).
fn add_arg(edit: &mut Edit, loc: &Loc, callable: &Callable, param: usize, variadic: bool, value: Name) -> Result<(), EditError> {
    let names: Vec<Name> = callable.inputs.iter().map(|input| edit.intern(&input.name)).collect();
    let mut ids = Vec::with_capacity(param + 2);
    for _ in 0..=param + 1 {
        ids.push(edit.id());
    }
    let args = call_args(edit, loc)?;
    let mut list = std::mem::take(args).into_vec();
    let positional = list.iter().take_while(|arg| matches!(arg, Arg::Value { name: None, .. })).count();
    let value_id = ids.pop().unwrap_or_else(|| labcraft_graph_id_fallback(&list));
    let operand = Operand::Path(Path { id: value_id, first: value, rest: Box::new([]) });
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
        End::NewArg { loc, .. } | End::After(loc) => loc.body.clone(),
        End::Output(_) => Vec::new(),
    }
}

/// What the end of a wire names.
fn resolve_end(edit: &Edit, judge: &dyn Judge, function: &Function, name: &InputName, from: Option<&str>) -> Result<End, EditError> {
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
    let loc = node(edit, function, node_name, "to")?;
    let statement = scope::statement(function, &loc).ok_or_else(|| EditError::internal("a node that went"))?;
    let unknown = |what: &str, known: Vec<String>| {
        EditError::new("edit.unknown", format!("`{}` has no input {what}", node_name.shown())).of("to").expecting(known)
    };
    if let Part::Next(_) = part {
        return Err(step_five("to", "the `next` of a loop"));
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
            _ => Err(step_five("to", "a `yield` of an arm")),
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
        StatementKind::Collect(_) | StatementKind::Until(_) => Err(step_five("to", "`collect` and `until`")),
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

fn edit_node_text(edit: &Edit, call: &labcraft_graph::Call) -> String {
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
    if let Some(label) = from.strip_prefix('#') {
        let name = edit.names.find(label).filter(|name| scope::labels(function).iter().any(|(found, _)| found == name));
        return name.map(Start::Label).ok_or_else(|| EditError::new("edit.unknown", format!("no node is labelled `{from}`")).of("from"));
    }
    if let Some((node_text, output)) = from.split_once('.') {
        // `node.output`: an output of a node by its name in the signature; `_` gets a name (N-14).
        let loc = node(edit, function, &NodeName::parse(node_text), "from")?;
        let statement = scope::statement(function, &loc).cloned().ok_or_else(|| EditError::internal("a node that went"))?;
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
                let text: Vec<&str> = call.callee.segments().map(|segment| edit.text(segment)).collect();
                let base = of_output(&callable, &text.join("."), index).unwrap_or_else(|| output.to_owned());
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
        return scope::values(function)
            .into_iter()
            .find(|value| value.name == name && matches!(&value.bound, Bound::Line(at, i) if *at == loc && *i == index))
            .map(Start::Value)
            .ok_or_else(|| EditError::internal("an output that went"));
    }
    let Some(name) = edit.names.find(from) else {
        let known: Vec<String> =
            scope::values(function).iter().map(|value| edit.text(value.name).to_owned()).collect::<BTreeSet<_>>().into_iter().collect();
        return Err(EditError::new("edit.unknown", format!("no value is named `{from}` in this function"))
            .of("from")
            .expecting(crate::refs::nearest(from, &known)));
    };
    let candidates: Vec<Value> = scope::values(function).into_iter().filter(|value| value.name == name).collect();
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
            let below = candidates.iter().find_map(|value| {
                (value.seen_in.len() > reader.len() && value.seen_in.starts_with(reader))
                    .then(|| scope::frame(function, value.seen_in.get(..=reader.len()).unwrap_or(&[])))
            });
            match below {
                Some(Some(Frame::Arm)) => Err(step_five("from", "a wire out of an arm, which adds an output to its structure,")),
                Some(Some(Frame::Loop)) => {
                    Err(EditError::new("wire.out_of_scope", format!("`{from}` is inside a loop: a `collect` or a carry takes a value out of it"))
                        .of("from"))
                }
                _ => {
                    Err(EditError::new("wire.out_of_scope", format!("`{from}` cannot be seen from there: it is a value of another arm or structure"))
                        .of("from"))
                }
            }
        }
        _ => Err(EditError::new("edit.ambiguous", format!("`{from}` names several values that the input sees")).of("from")),
    }
}

/// Whether wiring what the statement `made_by` makes into the statement at `reader` closes a cycle (spec 5.2): that
/// statement holds the reader, or depends on it, in its body.
fn closes_a_cycle(function: &Function, made_by: &Loc, reader: &Loc) -> bool {
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
    let Some(body) = scope::body(function, body_path) else { return false };
    // What each line of the body reads of the others, deep.
    let mut bound_by: HashMap<Name, Vec<usize>> = HashMap::new();
    for (index, statement) in body.statements.iter().enumerate() {
        for name in scope::targets(statement) {
            bound_by.entry(name).or_default().push(index);
        }
        if let Some(label) = statement.label {
            bound_by.entry(label.name).or_default().push(index);
        }
    }
    let mut seen = HashSet::new();
    let mut pending = vec![made_by.index];
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

fn names_in(operand: &Operand, found: &mut Vec<Name>) {
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
fn judge_types(edit: &Edit, judge: &dyn Judge, from: &str, to: &str, accept_mismatch: bool) -> Result<(), EditError> {
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
                    format!("a wire from `{from}` to `{to}` has the wrong type{types}; pass `accept_mismatch` to write it anyway"),
                )
                .of("to"));
            }
        }
    }
    Ok(())
}
