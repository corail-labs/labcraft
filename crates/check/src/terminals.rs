//! The terminals of the calls of a function (`docs/ir.md`, section 4): a call has an input for each parameter of what it calls,
//! a default included, an argument for each value of a variadic one, an input for each free terminal (the fields of a
//! `string.format` template, the arguments of `core.call`), and an output for each output of what it calls, written alone
//! too. `LC4008` bounds their number, since lowering builds each one, and a call of four bytes to a function of ten thousand
//! parameters with a default would build ten thousand inputs.
//!
//! What is counted is what `labcraft-ir` builds for its nodes `Primitive`, `Native` and `Call`; the harness compares the two
//! counts on every function of the repository. A variant built with its values and a struct have one input per value
//! written, which the size of the file bounds, and are not counted.

use std::collections::HashMap;

use labcraft_graph::{Arg, Body, Call, Function, Rhs, Statement, StatementKind};
use labcraft_types::Interner;

use crate::declare::DeclaredLibrary;
use crate::primitives::{ParamKind, primitive};
use crate::resolved::{Meaning, PrimitiveLibrary, Resolved, Terminal};

/// The terminals that the calls of a function may have *(proposed, scenario 4)*.
pub const MAX_TERMINALS: u64 = 1_000_000;

/// What [`terminals`] found.
#[derive(Clone, Copy, Debug)]
pub struct Terminals<'f> {
    /// The terminals of the calls; at most one call more than the limit, where it stops.
    pub count: u64,
    /// When the count went over the limit: the line of the call that has the most terminals among those counted, and their
    /// number. The first in the file of those that have the same.
    pub over: Option<(&'f Statement, u64)>,
}

/// Counts the terminals of the calls of `function`, and stops at the first call past `limit`.
pub fn terminals<'f>(function: &'f Function, resolved: &Resolved, declared: &[DeclaredLibrary], names: &Interner, limit: u64) -> Terminals<'f> {
    let mut count = 0_u64;
    let mut heaviest: Option<(&'f Statement, u64)> = None;
    // Each line, then the bodies of its structure before the next line: the order of the file.
    let mut stack: Vec<(&'f Body, usize)> = vec![(&function.body, 0)];
    while let Some((body, at)) = stack.pop() {
        let Some(statement) = body.statements.get(at) else { continue };
        stack.push((body, at.saturating_add(1)));
        let (call, targets) = match &statement.kind {
            StatementKind::Call(call) => (Some(call), 0),
            StatementKind::Binding { targets, value } => match value {
                Rhs::Call(call) => (Some(call), targets.len()),
                Rhs::Loop(structure) => {
                    stack.push((&structure.body, 0));
                    (None, 0)
                }
                Rhs::For(structure) => {
                    stack.push((&structure.body, 0));
                    (None, 0)
                }
                Rhs::Match(branching) | Rhs::Handle(branching) => {
                    stack.extend(branching.arms.iter().rev().map(|arm| (&arm.body, 0)));
                    (None, 0)
                }
                Rhs::Operand(_) | Rhs::Struct(_) => (None, 0),
            },
            StatementKind::Collect(_) | StatementKind::Until(_) => (None, 0),
        };
        let Some(call) = call else { continue };
        let Some(weight) = weight(call, statement, targets, resolved, declared, names) else { continue };
        count = count.saturating_add(weight);
        if heaviest.is_none_or(|(_, most)| weight > most) {
            heaviest = Some((statement, weight));
        }
        if count > limit {
            return Terminals { count, over: heaviest };
        }
    }
    Terminals { count, over: None }
}

/// The terminals of one call, as lowering builds them; `None` for what is no node of a call (a variant, an error).
fn weight(call: &Call, statement: &Statement, targets: usize, resolved: &Resolved, declared: &[DeclaredLibrary], names: &Interner) -> Option<u64> {
    let bound = resolved.call(call.id)?;
    // The arguments that each parameter takes, by its position, and the free terminals.
    let mut by_position: HashMap<u32, u64> = HashMap::new();
    let mut free = 0_u64;
    for (arg, terminal) in call.args.iter().zip(bound.terminals.iter()) {
        match (arg, terminal) {
            (Arg::Value { .. }, Terminal::Param(position)) => *by_position.entry(*position).or_default() += 1,
            (Arg::Value { .. }, Terminal::Free) => free += 1,
            _ => {}
        }
    }
    let wired = |position: usize, variadic: bool| -> u64 {
        if variadic { u32::try_from(position).ok().and_then(|position| by_position.get(&position).copied()).unwrap_or(0) } else { 1 }
    };
    let inputs: u64 = match bound.callee {
        Meaning::Function { library, name } => {
            let function = declared.get(library as usize)?.functions.get(&name)?;
            // A signature with an error of its own has its diagnostic, and a call of it is never lowered: walking its
            // parameters for each call would cost the calls times the parameters, with a variadic parameter as often as written.
            if function.faulty || function.generic {
                return None;
            }
            function.params.iter().enumerate().map(|(position, param)| wired(position, param.variadic)).sum()
        }
        Meaning::Primitive { library, name } => {
            let text = names.resolve(name);
            let primitive = primitive(library, text)?;
            // The template of `string.format` is data of its node, and its inputs are the fields; `core.call` has the function,
            // then the inputs of its type.
            match (library, text) {
                (PrimitiveLibrary::String, "format") => return Some(free.saturating_add(outputs_of_line(statement, targets, resolved))),
                (PrimitiveLibrary::Core, "call") => {
                    return Some(free.saturating_add(1).saturating_add(outputs_of_line(statement, targets, resolved)));
                }
                _ => {}
            }
            let params: u64 = primitive
                .params
                .iter()
                .enumerate()
                .filter(|(_, param)| param.kind == ParamKind::Value)
                .map(|(position, param)| wired(position, param.variadic))
                .sum();
            params.saturating_add(free)
        }
        _ => return None,
    };
    Some(inputs.saturating_add(outputs_of_line(statement, targets, resolved)))
}

/// What the line binds, or what a call written alone gives.
fn outputs_of_line(statement: &Statement, targets: usize, resolved: &Resolved) -> u64 {
    let outputs = if targets > 0 { targets } else { resolved.outputs_of(statement.id).len() };
    u64::try_from(outputs).unwrap_or(u64::MAX)
}
