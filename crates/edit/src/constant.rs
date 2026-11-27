//! The constants of the inputs (commands.md 6.7): `input.set_constant` writes one where a wire would go, and is judged as
//! a wire is; `input.clear_constant` takes it out as `wire.disconnect` takes a wire out (9.1); `input.remove_value`
//! removes a value of a variadic parameter or an element of an array, and the values after it move up one place.

use labcraft_graph::{LayoutKind, Operand, Rhs, StatementKind};

use crate::add::read_constant;
use crate::delete::{callee, leave_out_holes};
use crate::model::Edit;
use crate::places::{Slot, args_mut, operand, operand_mut};
use crate::refs::{InputName, Part};
use crate::scope;
use crate::wire::{End, add_arg, add_field, forget_wire, format_call, input_name, judge_types, occupied, resolve_end, statement_node};
use crate::{EditError, Judge, cut};

/// The values of the function that an operand reads: a wire, `p.volts` included (spec 11). A constant reads none; a
/// variant such as `Verdict.Pass` names a type, not a value.
pub(crate) fn values_read(function: &labcraft_graph::Function, held: &Operand) -> Vec<labcraft_types::Name> {
    let params = function.signature.params.iter().map(|param| param.name);
    let mut found: Vec<labcraft_types::Name> = Vec::new();
    // A name bound in several arms is one value read.
    for name in params.chain(scope::values(function).into_iter().map(|value| value.name)) {
        if crate::places::reads(held, name) && !found.contains(&name) {
            found.push(name);
        }
    }
    found
}

/// Forgets the `wire` entries of the values that the place read, as `wire.disconnect` does.
fn forget_read(edit: &mut Edit, judge: &dyn Judge, statement: &labcraft_graph::Statement, slot: Slot, read: &[labcraft_types::Name]) {
    let input = input_name(edit, judge, statement, slot);
    for name in read {
        forget_wire(edit, *name, statement_node(statement), input.as_deref());
    }
}

/// `input.set_constant`.
pub(crate) fn set(
    edit: &mut Edit,
    judge: &dyn Judge,
    to: &str,
    arm: Option<&str>,
    value: &str,
    replace: bool,
    accept_mismatch: bool,
) -> Result<(), EditError> {
    if value.trim() == "?" {
        return Err(EditError::invalid("value", "a constant is never `?`: `input.clear_constant` is what leaves a hole"));
    }
    let constant = read_constant(edit, value)?;
    let function = edit.body_function()?.clone();
    let end = resolve_end(edit, judge, &function, &InputName::parse(to, "to")?, None, arm)?;
    refuse_template(edit, &function, &end)?;
    match &end {
        End::Output(_) => {
            return Err(EditError::invalid(
                "to",
                format!("`{}` is an output of the function, which a wire feeds: add a named constant (`constant.add`) and wire it", cut(to)),
            ));
        }
        End::After(_) => return Err(EditError::invalid("to", "`after` waits for a node: it takes an order wire, not a constant")),
        // Only a wire names the `yield` of a structure that gives nothing.
        End::NewYield { .. } => return Err(EditError::internal("a `yield` that gives nothing, for a constant")),
        End::Slot(loc, slot) => {
            let statement = scope::statement(&function, loc).ok_or_else(|| EditError::internal("a node that went"))?;
            if let Some(held) = operand(statement, *slot)
                && !matches!(held, Operand::Hole(_))
            {
                if !replace {
                    return Err(occupied(edit, to, held));
                }
                let read = values_read(&function, held);
                forget_read(edit, judge, statement, *slot, &read);
            }
            let function_mut = edit.body_function_mut()?;
            let place = scope::statement_mut(function_mut, loc).and_then(|statement| operand_mut(statement, *slot));
            let Some(place) = place else { return Err(EditError::internal("a place that went")) };
            *place = constant;
        }
        End::NewArg { loc, param, variadic } => {
            let statement = scope::statement(&function, loc).cloned().ok_or_else(|| EditError::internal("a node that went"))?;
            let callable =
                callee(edit, judge, &statement).ok_or_else(|| EditError::internal("a parameter of a call that the check does not know"))?;
            add_arg(edit, loc, &callable, *param, variadic.is_some(), constant)?;
        }
        End::Field { loc, field } => add_field(edit, loc, field, constant)?,
        End::Next { loc, carry } => crate::wire::add_next(edit, loc, *carry, constant)?,
        End::NextBound { loc, carry } => {
            if !replace {
                return Err(EditError::new("wire.occupied", format!("`{}` is set by a node: pass `replace` to replace it", cut(to))).of("to"));
            }
            crate::wire::unbind_next(edit, loc, *carry)?;
            crate::wire::add_next(edit, &crate::wire::loop_of(loc)?, *carry, constant)?;
        }
    }
    judge_types(edit, judge, &format!("the constant `{}` on `{}`", cut(value.trim()), cut(to)), accept_mismatch)
}

/// The template of a `string.format` is written by `node.set_template` alone.
fn refuse_template(edit: &Edit, function: &labcraft_graph::Function, end: &End) -> Result<(), EditError> {
    if is_template(edit, function, end) {
        return Err(EditError::invalid("to", "the template of `string.format` is written by `node.set_template` (spec 5.4)"));
    }
    Ok(())
}

/// Whether the end is the template of a `string.format`, which only `node.set_template` writes: the argument that gives
/// it, or the first parameter of a call that does not give it yet.
pub(crate) fn is_template(edit: &Edit, function: &labcraft_graph::Function, end: &End) -> bool {
    let (loc, at) = match end {
        End::Slot(loc, Slot::Arg(index)) => (loc, Some(*index)),
        End::NewArg { loc, param: 0, variadic: None } => (loc, None),
        _ => return false,
    };
    let Some(call) = scope::statement(function, loc).and_then(|statement| format_call(edit, statement)) else { return false };
    match (at, crate::wire::template_of(edit, call)) {
        (Some(index), Some((given, _))) => given == index,
        (None, None) => true,
        _ => false,
    }
}

/// `input.clear_constant`.
pub(crate) fn clear(edit: &mut Edit, judge: &dyn Judge, to: &str, arm: Option<&str>) -> Result<(), EditError> {
    let function = edit.body_function()?.clone();
    let end = resolve_end(edit, judge, &function, &InputName::parse(to, "to")?, None, arm)?;
    let no_constant = || EditError::invalid("to", format!("`{}` holds no constant", cut(to)));
    refuse_template(edit, &function, &end)?;
    let End::Slot(loc, slot) = end else { return Err(no_constant()) };
    let statement = scope::statement(&function, &loc).ok_or_else(|| EditError::internal("a node that went"))?;
    match operand(statement, slot) {
        // A wire, `p.volts` included, is unwired by `wire.disconnect`.
        Some(held) if !values_read(&function, held).is_empty() => return Err(no_constant()),
        Some(Operand::Hole(_)) | None => return Err(no_constant()),
        Some(_) => {}
    }
    let id = edit.id();
    let function_mut = edit.body_function_mut()?;
    let place = scope::statement_mut(function_mut, &loc).and_then(|statement| operand_mut(statement, slot));
    let Some(place) = place else { return Err(EditError::internal("a place that went")) };
    *place = Operand::Hole(id);
    match slot {
        Slot::Arg(index) => leave_out_holes(edit, judge, vec![(loc, index)]),
        _ => Ok(()),
    }
}

/// `input.remove_value`.
pub(crate) fn remove_value(edit: &mut Edit, judge: &dyn Judge, to: &str) -> Result<(), EditError> {
    let name = InputName::parse(to, "to")?;
    let removed_at = match &name {
        InputName::Node(_, Part::Variadic(_, at) | Part::Index(at)) => *at,
        _ => {
            return Err(EditError::invalid(
                "to",
                format!("`{}` is no value of a variadic parameter nor an element of an array: name it by its place (`msg.parts.2`, `k.1`)", cut(to)),
            ));
        }
    };
    let function = edit.body_function()?.clone();
    let end = resolve_end(edit, judge, &function, &name, None, None)?;
    let End::Slot(loc, slot) = end else {
        return Err(EditError::new("edit.unknown", format!("`{}` holds no value to remove", cut(to))).of("to"));
    };
    let statement = scope::statement(&function, &loc).cloned().ok_or_else(|| EditError::internal("a node that went"))?;
    // What the place names now (`parts.2`), and the parameter whose values move up (`parts`): empty for an array.
    let named = input_name(edit, judge, &statement, slot).unwrap_or_default();
    let prefix = match named.rsplit_once('.') {
        Some((param, _)) => format!("{param}."),
        None => String::new(),
    };
    match slot {
        Slot::Arg(_) if prefix.is_empty() => {
            return Err(EditError::invalid("to", format!("`{}` is no value of a variadic parameter: only those are removed", cut(to))));
        }
        Slot::Arg(_) | Slot::Element(_) => {}
        _ => return Err(EditError::invalid("to", format!("`{}` is no value of a variadic parameter nor an element of an array", cut(to)))),
    }
    let held = operand(&statement, slot).cloned();
    let function_mut = edit.body_function_mut()?;
    let Some(statement_mut) = scope::statement_mut(function_mut, &loc) else { return Err(EditError::internal("a node that went")) };
    match slot {
        Slot::Arg(index) => {
            let args = args_mut(statement_mut).ok_or_else(|| EditError::internal("a call that went"))?;
            let mut list = std::mem::take(args).into_vec();
            if index < list.len() {
                list.remove(index);
            }
            *args = list.into_boxed_slice();
        }
        Slot::Element(index) => {
            if let StatementKind::Binding { value: Rhs::Operand(Operand::Array(_, values)), .. } = &mut statement_mut.kind
                && index < values.len()
            {
                values.remove(index);
            }
        }
        _ => {}
    }
    if let Some(held) = &held {
        for name in values_read(&function, held) {
            forget_wire(edit, name, statement_node(&statement), Some(&named));
        }
    }
    move_up(edit, &statement, &prefix, removed_at);
    Ok(())
}

/// The `wire` entries that name a value after the one removed name it one place up: `parts.3` becomes `parts.2`.
fn move_up(edit: &mut Edit, statement: &labcraft_graph::Statement, prefix: &str, removed_at: usize) {
    let Some((node, is_label)) = statement_node(statement) else { return };
    let mut renames = Vec::new();
    if let Some(layout) = &edit.file.layout {
        for item in &layout.items {
            let LayoutKind::Wire { to, .. } = &item.kind else { continue };
            let Some(terminal) = to.terminal else { continue };
            if to.node.name != node || to.node.is_label != is_label {
                continue;
            }
            let text = edit.text(terminal);
            let Some(place) = text.strip_prefix(prefix).and_then(|rest| rest.parse::<usize>().ok()) else { continue };
            if place > removed_at {
                renames.push((terminal, format!("{prefix}{}", place - 1)));
            }
        }
    }
    let renames: Vec<_> = renames.into_iter().map(|(from, to)| (from, edit.intern(&to))).collect();
    let Some(layout) = &mut edit.file.layout else { return };
    for item in &mut layout.items {
        let LayoutKind::Wire { to, .. } = &mut item.kind else { continue };
        if to.node.name != node || to.node.is_label != is_label {
            continue;
        }
        if let Some(terminal) = to.terminal
            && let Some((_, renamed)) = renames.iter().find(|(from, _)| *from == terminal)
        {
            to.terminal = Some(*renamed);
        }
    }
}
