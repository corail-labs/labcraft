//! `node.delete` (commands.md 6.7), and what it shares with `wire.disconnect`: taking a value out of the places that read
//! it. No other node is deleted (decision of the owner, 2026-11-15): an input that a deleted node fed is left out, or gets
//! a hole where the text cannot leave it out (9.1); an `after:` loses it, and goes when it is empty.

use std::collections::HashSet;

use labcraft_graph::{Arg, Function, LayoutKind, OrderRef, Statement};
use labcraft_types::Name;

use crate::model::Edit;
use crate::places::{Slot, args, args_mut, can_leave_out, hole_where_read, operand, operand_mut, out_of_after, reads, slots};
use crate::refs::{NodeName, node};
use crate::scope::{self, Bound, Loc, Value};
use crate::{Callable, EditError, Judge, Lookup};

/// A place that reads a value: a slot, or the `after:` of a call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reader {
    Slot(Loc, Slot),
    After(Loc),
}

/// The places that read a value, outside the statements of `skip`.
pub(crate) fn readers(function: &Function, value: &Value, skip: &[Loc]) -> Vec<Reader> {
    let mut found = Vec::new();
    for loc in scope::all(function) {
        if skip.iter().any(|skipped| inside(&loc, skipped)) {
            continue;
        }
        let Some(statement) = scope::statement(function, &loc) else { continue };
        for (slot, seen_in) in slots(statement, &loc) {
            if scope::sees(function, value, &seen_in) && operand(statement, slot).is_some_and(|operand| reads(operand, value.name)) {
                found.push(Reader::Slot(loc.clone(), slot));
            }
        }
        if scope::sees(function, value, &loc.body) && after_reads(statement, value.name, false) {
            found.push(Reader::After(loc.clone()));
        }
    }
    found
}

/// The calls whose `after:` names a label.
pub(crate) fn label_readers(function: &Function, label: Name) -> Vec<Reader> {
    scope::all(function)
        .into_iter()
        .filter(|loc| scope::statement(function, loc).is_some_and(|statement| after_reads(statement, label, true)))
        .map(Reader::After)
        .collect()
}

fn after_reads(statement: &Statement, name: Name, label: bool) -> bool {
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

/// Whether the statement at `loc` is `outer` or inside it.
pub(crate) fn inside(loc: &Loc, outer: &Loc) -> bool {
    if loc == outer {
        return true;
    }
    loc.body.len() > outer.body.len()
        && loc.body.starts_with(&outer.body)
        && loc.body.get(outer.body.len()).is_some_and(|step| step.stmt == outer.index)
}

/// What a call names, for the rules of what can be left out.
pub(crate) fn callee(edit: &Edit, judge: &dyn Judge, statement: &Statement) -> Option<Callable> {
    let call = match &statement.kind {
        labcraft_graph::StatementKind::Call(call) | labcraft_graph::StatementKind::Binding { value: labcraft_graph::Rhs::Call(call), .. } => call,
        _ => return None,
    };
    match judge.callable(&edit.path, &crate::wire::edit_node_text(edit, call)) {
        Lookup::Found(callable) => Some(callable),
        Lookup::Unknown(_) => None,
    }
}

/// Takes `name` (a value, or a label) out of each reader: a hole in its place, then the place left out where it can be.
pub(crate) fn take_out(edit: &mut Edit, judge: &dyn Judge, name: Name, label: bool, readers: &[Reader]) -> Result<(), EditError> {
    let mut leave_out: Vec<(Loc, usize)> = Vec::new();
    for reader in readers {
        match reader {
            Reader::Slot(loc, slot) => {
                let function = edit.body_function_mut()?;
                let Some(statement) = scope::statement_mut(function, loc) else { continue };
                let Some(operand) = operand_mut(statement, *slot) else { continue };
                if hole_where_read(operand, name)
                    && matches!(operand, labcraft_graph::Operand::Hole(_))
                    && let Slot::Arg(index) = slot
                {
                    leave_out.push((loc.clone(), *index));
                }
            }
            Reader::After(loc) => {
                let function = edit.body_function_mut()?;
                if let Some(args) = scope::statement_mut(function, loc).and_then(args_mut) {
                    out_of_after(args, name, label);
                }
            }
        }
    }
    leave_out_holes(edit, judge, leave_out)
}

/// Leaves out each argument at these places, a hole now, where the text can leave it out (9.1).
pub(crate) fn leave_out_holes(edit: &mut Edit, judge: &dyn Judge, mut leave_out: Vec<(Loc, usize)>) -> Result<(), EditError> {
    // Last first, so that the indices of the others hold; the rule reads the arguments as the holes left them.
    leave_out.sort();
    leave_out.dedup();
    for (loc, index) in leave_out.into_iter().rev() {
        let Some(statement) = scope::statement(edit.body_function()?, &loc).cloned() else { continue };
        let callable = callee(edit, judge, &statement);
        let function = edit.body_function_mut()?;
        let Some(args) = scope::statement_mut(function, &loc).and_then(args_mut) else { continue };
        if can_leave_out(args, index, callable.as_ref()) {
            let mut kept = std::mem::take(args).into_vec();
            if index < kept.len() {
                kept.remove(index);
            }
            *args = kept.into_boxed_slice();
        }
    }
    Ok(())
}

pub(crate) fn nodes(edit: &mut Edit, judge: &dyn Judge, nodes: &[String]) -> Result<(), EditError> {
    if nodes.is_empty() {
        return Err(EditError::invalid("nodes", "`nodes` names at least one node"));
    }
    let function = edit.body_function()?.clone();
    let mut doomed: Vec<Loc> = Vec::new();
    for text in nodes {
        let loc = node(edit, &function, &NodeName::parse(text), "nodes")?;
        doomed.push(loc);
    }
    doomed.sort();
    doomed.dedup();
    // A node inside a structure that goes goes with it.
    let outer: Vec<Loc> = doomed.iter().filter(|loc| !doomed.iter().any(|other| other != *loc && inside(loc, other))).cloned().collect();

    // What goes: the values and labels of the statements, and of everything they hold.
    let gone_values: Vec<Value> = scope::values(&function)
        .into_iter()
        .filter(|value| match &value.bound {
            Bound::Line(loc, _) | Bound::Head(loc, _) => outer.iter().any(|gone| inside(loc, gone)),
            Bound::Input => false,
        })
        .collect();
    let gone_labels: Vec<Name> =
        scope::labels(&function).into_iter().filter(|(_, loc)| outer.iter().any(|gone| inside(loc, gone))).map(|(label, _)| label).collect();

    // Each value on the function as the ones before left it: taking one out can remove an argument, which moves the
    // places of the others in that call. No statement goes before all are taken out, so the places of statements hold.
    for value in &gone_values {
        let now = edit.body_function()?.clone();
        let found = readers(&now, value, &outer);
        take_out(edit, judge, value.name, false, &found)?;
    }
    for label in &gone_labels {
        let now = edit.body_function()?.clone();
        let found: Vec<Reader> = label_readers(&now, *label)
            .into_iter()
            .filter(|reader| match reader {
                Reader::After(loc) | Reader::Slot(loc, _) => !outer.iter().any(|gone| inside(loc, gone)),
            })
            .collect();
        take_out(edit, judge, *label, true, &found)?;
    }

    // Last first: removing a statement moves the ones after it in its body, and none of those is removed yet.
    let function_mut = edit.body_function_mut()?;
    for loc in outer.iter().rev() {
        if let Some(body) = scope::body_mut(function_mut, &loc.body)
            && loc.index < body.statements.len()
        {
            body.statements.remove(loc.index);
        }
    }

    let mut gone: HashSet<Name> = gone_values.iter().map(|value| value.name).collect();
    gone.extend(gone_labels.iter().copied());
    forget_in_layout(edit, &gone)?;
    Ok(())
}

/// Removes the entries of `layout` that name what is gone, when nothing of the function bears the name any more.
pub(crate) fn forget_in_layout(edit: &mut Edit, gone: &HashSet<Name>) -> Result<(), EditError> {
    let function = edit.body_function()?;
    let mut still: HashSet<Name> = scope::values(function).iter().map(|value| value.name).collect();
    still.extend(scope::labels(function).into_iter().map(|(label, _)| label));
    // A wire can end at an output of the function (`wire e3 -> err`); a `node` entry cannot name one (spec 11).
    let outputs: HashSet<Name> = function.signature.outputs.iter().map(|output| output.name).collect();
    let missing = |name: &Name| gone.contains(name) && !still.contains(name);
    if let Some(layout) = &mut edit.file.layout {
        layout.items.retain(|item| match &item.kind {
            LayoutKind::Frame { node: None, .. } => true,
            LayoutKind::Frame { node: Some(node), .. } | LayoutKind::Node { node, .. } => !missing(&node.name),
            LayoutKind::Wire { from, to, .. } => !missing(&from.node.name) && !(missing(&to.node.name) && !outputs.contains(&to.node.name)),
        });
    }
    Ok(())
}
