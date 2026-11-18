//! The places where a statement reads values (its inputs, spec 11 and commands.md 9.2), and what the editor does to a
//! place when the value it reads goes: the place is left out where the text can leave it out and no position moves,
//! and gets a hole `?` everywhere else (commands.md 9.1).

use labcraft_graph::{Arg, ClauseKind, Id, Operand, OrderRef, Rhs, Statement, StatementKind, UnitTerm};
use labcraft_types::Name;

use crate::scope::{BodyPath, Loc};
use crate::{Callable, Kind};

/// An operand of a statement, by where it stands in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    /// An argument of a call, a construction of a variant included, by its index among the arguments.
    Arg(usize),
    /// A field of a struct construction.
    Field(usize),
    /// The operand of `x = operand`: a new name, a named constant, an array.
    Value,
    /// An element of the array of `k = [y, x]`, which a wire names `k.1` (spec 11).
    Element(usize),
    /// The selector of a `match` or a `handle`.
    Selector,
    /// The source of an `in` clause, and the end of its range, by the index of the clause.
    Source(usize),
    End(usize),
    /// The initial value of a carry, by the index of the clause.
    Init(usize),
    /// A value of the `yield` of an arm.
    Yield {
        arm: usize,
        index: usize,
    },
    /// The operand of `collect` and of `until`.
    Collect,
    Until,
}

/// The slots of a statement, each with the body it is read in.
pub(crate) fn slots(statement: &Statement, loc: &Loc) -> Vec<(Slot, BodyPath)> {
    let here = || loc.body.clone();
    let mut found = Vec::new();
    match &statement.kind {
        StatementKind::Call(call) => found.extend((0..call.args.len()).map(|index| (Slot::Arg(index), here()))),
        StatementKind::Gather(_, _) => found.push((Slot::Collect, here())),
        StatementKind::Until(_) => found.push((Slot::Until, here())),
        StatementKind::Binding { value, .. } => match value {
            Rhs::Call(call) => found.extend((0..call.args.len()).map(|index| (Slot::Arg(index), here()))),
            Rhs::Struct(construction) => found.extend((0..construction.fields.len()).map(|index| (Slot::Field(index), here()))),
            // An array that holds wires is a node, its elements its inputs (`k.1`, spec 11).
            Rhs::Operand(Operand::Array(_, values)) => found.extend((0..values.len()).map(|index| (Slot::Element(index), here()))),
            Rhs::Operand(_) => found.push((Slot::Value, here())),
            Rhs::Match(branching) | Rhs::Handle(branching) => {
                found.push((Slot::Selector, here()));
                for (arm, each) in branching.arms.iter().enumerate() {
                    found.extend((0..each.yields.values.len()).map(|index| (Slot::Yield { arm, index }, loc.sub(arm))));
                }
            }
            Rhs::For(each) => {
                for (index, clause) in each.clauses.iter().enumerate() {
                    match &clause.kind {
                        ClauseKind::In(clause) => {
                            found.push((Slot::Source(index), here()));
                            if clause.end.is_some() {
                                found.push((Slot::End(index), here()));
                            }
                        }
                        ClauseKind::Carry(_) => found.push((Slot::Init(index), here())),
                    }
                }
            }
            Rhs::Loop(repeat) => found.extend((0..repeat.clauses.len()).map(|index| (Slot::Init(index), here()))),
        },
    }
    found
}

/// The operand at a slot, for an argument the value of an `Arg::Value` (an `after:` has none).
pub(crate) fn operand(statement: &Statement, slot: Slot) -> Option<&Operand> {
    match (&statement.kind, slot) {
        (StatementKind::Call(call) | StatementKind::Binding { value: Rhs::Call(call), .. }, Slot::Arg(index)) => match call.args.get(index)? {
            Arg::Value { value, .. } => Some(value),
            Arg::After { .. } => None,
        },
        (StatementKind::Binding { value: Rhs::Struct(construction), .. }, Slot::Field(index)) => {
            construction.fields.get(index).map(|field| &field.value)
        }
        (StatementKind::Binding { value: Rhs::Operand(operand), .. }, Slot::Value) => Some(operand),
        (StatementKind::Binding { value: Rhs::Operand(Operand::Array(_, values)), .. }, Slot::Element(index)) => values.get(index),
        (StatementKind::Binding { value: Rhs::Match(branching) | Rhs::Handle(branching), .. }, Slot::Selector) => Some(&branching.selector),
        (StatementKind::Binding { value: Rhs::Match(branching) | Rhs::Handle(branching), .. }, Slot::Yield { arm, index }) => {
            branching.arms.get(arm)?.yields.values.get(index)
        }
        (StatementKind::Binding { value: Rhs::For(each), .. }, Slot::Source(index) | Slot::End(index) | Slot::Init(index)) => {
            match &each.clauses.get(index)?.kind {
                ClauseKind::In(clause) => match slot {
                    Slot::Source(_) => Some(&clause.source),
                    Slot::End(_) => clause.end.as_ref(),
                    _ => None,
                },
                ClauseKind::Carry(carry) => matches!(slot, Slot::Init(_)).then_some(&carry.value),
            }
        }
        (StatementKind::Binding { value: Rhs::Loop(repeat), .. }, Slot::Init(index)) => match &repeat.clauses.get(index)?.kind {
            ClauseKind::Carry(carry) => Some(&carry.value),
            ClauseKind::In(_) => None,
        },
        (StatementKind::Gather(_, operand), Slot::Collect) | (StatementKind::Until(operand), Slot::Until) => Some(operand),
        _ => None,
    }
}

pub(crate) fn operand_mut(statement: &mut Statement, slot: Slot) -> Option<&mut Operand> {
    match (&mut statement.kind, slot) {
        (StatementKind::Call(call) | StatementKind::Binding { value: Rhs::Call(call), .. }, Slot::Arg(index)) => match call.args.get_mut(index)? {
            Arg::Value { value, .. } => Some(value),
            Arg::After { .. } => None,
        },
        (StatementKind::Binding { value: Rhs::Struct(construction), .. }, Slot::Field(index)) => {
            construction.fields.get_mut(index).map(|field| &mut field.value)
        }
        (StatementKind::Binding { value: Rhs::Operand(Operand::Array(_, values)), .. }, Slot::Element(index)) => values.get_mut(index),
        (StatementKind::Binding { value: Rhs::Operand(operand), .. }, Slot::Value) => Some(operand),
        (StatementKind::Binding { value: Rhs::Match(branching) | Rhs::Handle(branching), .. }, Slot::Selector) => Some(&mut branching.selector),
        (StatementKind::Binding { value: Rhs::Match(branching) | Rhs::Handle(branching), .. }, Slot::Yield { arm, index }) => {
            branching.arms.get_mut(arm)?.yields.values.get_mut(index)
        }
        (StatementKind::Binding { value: Rhs::For(each), .. }, Slot::Source(index) | Slot::End(index) | Slot::Init(index)) => {
            match &mut each.clauses.get_mut(index)?.kind {
                ClauseKind::In(clause) => match slot {
                    Slot::Source(_) => Some(&mut clause.source),
                    Slot::End(_) => clause.end.as_mut(),
                    _ => None,
                },
                ClauseKind::Carry(carry) => {
                    if matches!(slot, Slot::Init(_)) {
                        Some(&mut carry.value)
                    } else {
                        None
                    }
                }
            }
        }
        (StatementKind::Binding { value: Rhs::Loop(repeat), .. }, Slot::Init(index)) => match &mut repeat.clauses.get_mut(index)?.kind {
            ClauseKind::Carry(carry) => Some(&mut carry.value),
            ClauseKind::In(_) => None,
        },
        (StatementKind::Gather(_, operand), Slot::Collect) | (StatementKind::Until(operand), Slot::Until) => Some(operand),
        _ => None,
    }
}

/// The arguments of the call of a statement.
pub(crate) fn args(statement: &Statement) -> Option<&[Arg]> {
    match &statement.kind {
        StatementKind::Call(call) | StatementKind::Binding { value: Rhs::Call(call), .. } => Some(&call.args),
        _ => None,
    }
}

pub(crate) fn args_mut(statement: &mut Statement) -> Option<&mut Box<[Arg]>> {
    match &mut statement.kind {
        StatementKind::Call(call) | StatementKind::Binding { value: Rhs::Call(call), .. } => Some(&mut call.args),
        _ => None,
    }
}

/// The name that a `[x]` reads, whether it is a unit or an array (spec 5.2).
pub(crate) fn bracket_name(unit: &labcraft_graph::UnitExpr) -> Option<Name> {
    match (&unit.first, unit.rest.is_empty()) {
        (UnitTerm::Named { name, exponent: None }, true) => Some(*name),
        _ => None,
    }
}

/// Whether an operand reads a name: itself, the first segment of its path, or an element.
pub(crate) fn reads(operand: &Operand, name: Name) -> bool {
    match operand {
        Operand::Path(path) => path.first == name,
        Operand::Bracket(_, unit) => bracket_name(unit) == Some(name),
        Operand::Array(_, values) => values.iter().any(|value| reads(value, name)),
        Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) | Operand::Hole(_) => false,
    }
}

/// Puts a hole wherever the operand reads `name`. Whether it changed.
pub(crate) fn hole_where_read(operand: &mut Operand, name: Name) -> bool {
    match operand {
        Operand::Array(_, values) => {
            let mut changed = false;
            for value in values {
                changed |= hole_where_read(value, name);
            }
            changed
        }
        _ if reads(operand, name) => {
            *operand = Operand::Hole(operand.id());
            true
        }
        _ => false,
    }
}

/// Renames what the operand reads of `from`. Whether it changed.
pub(crate) fn rename_read(operand: &mut Operand, from: Name, to: Name) -> bool {
    match operand {
        Operand::Path(path) if path.first == from => {
            path.first = to;
            true
        }
        Operand::Bracket(_, unit) if bracket_name(unit) == Some(from) => {
            unit.first = UnitTerm::Named { name: to, exponent: None };
            true
        }
        Operand::Array(_, values) => {
            let mut changed = false;
            for value in values {
                changed |= rename_read(value, from, to);
            }
            changed
        }
        _ => false,
    }
}

/// Whether the argument at `index`, a hole now, can be left out with no other argument moving: a named argument, or the
/// last positional one of a parameter that is not variadic. A value of a variant, and the place of an unknown callee, keep
/// their hole.
pub(crate) fn can_leave_out(args: &[Arg], index: usize, callee: Option<&Callable>) -> bool {
    let Some(Arg::Value { name, .. }) = args.get(index) else { return false };
    if name.is_some() {
        return true;
    }
    let later_positional = args.iter().skip(index + 1).any(|arg| matches!(arg, Arg::Value { name: None, .. }));
    if later_positional {
        return false;
    }
    match callee {
        Some(callable) if callable.kind == Kind::Function => {
            // Positional arguments fill the parameters in order; the variadic one takes the rest.
            let position = args.iter().take(index).filter(|arg| matches!(arg, Arg::Value { name: None, .. })).count();
            let variadic_at = callable.inputs.iter().position(|input| input.variadic);
            !matches!(variadic_at, Some(at) if position >= at)
        }
        _ => false,
    }
}

/// Takes `name` out of the `after:` of the arguments (R-15): its value or its label. An empty `after:` goes. Whether it
/// changed.
pub(crate) fn out_of_after(args: &mut Box<[Arg]>, name: Name, label: bool) -> bool {
    let mut changed = false;
    let mut kept: Vec<Arg> = Vec::with_capacity(args.len());
    for arg in std::mem::take(args).into_vec() {
        match arg {
            Arg::After { id, mut refs } => {
                let before = refs.len();
                refs.retain(|reference| match reference {
                    OrderRef::Value(operand) => label || !reads(operand, name),
                    OrderRef::Label(reference) => !label || reference.name != name,
                });
                changed |= refs.len() != before;
                if !refs.is_empty() {
                    kept.push(Arg::After { id, refs });
                }
            }
            other => kept.push(other),
        }
    }
    *args = kept.into_boxed_slice();
    changed
}

/// The id of an argument.
pub(crate) fn arg_id(arg: &Arg) -> Id {
    match arg {
        Arg::Value { id, .. } | Arg::After { id, .. } => *id,
    }
}
