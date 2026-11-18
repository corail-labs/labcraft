//! Where things are in a function: the bodies, nested in their structures, the statements in them, the values they bind
//! and who can see them (spec 6: a value is seen in its body and in the bodies inside it; a name bound inside hides it).

use labcraft_graph::{Body, ClauseKind, Function, Rhs, Statement, StatementKind, Target};
use labcraft_types::Name;

/// One step down from a body: the statement `stmt` of it, and the body `sub` of that statement (an arm, or the body of a
/// loop).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Step {
    pub stmt: usize,
    pub sub: usize,
}

/// A body, by the steps from the body of the function.
pub(crate) type BodyPath = Vec<Step>;

/// A statement: its body and its index there.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Loc {
    pub body: BodyPath,
    pub index: usize,
}

impl Loc {
    /// The path of a body of this statement.
    pub fn sub(&self, sub: usize) -> BodyPath {
        let mut path = self.body.clone();
        path.push(Step { stmt: self.index, sub });
        path
    }
}

/// What kind of structure holds a body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Frame {
    /// An arm of a `match` or a `handle`.
    Arm,
    /// The body of a `for` or a `loop`.
    Loop,
}

/// The bodies of a statement, in order: the arms of a `match` or a `handle`, the body of a `for` or a `loop`.
pub(crate) fn subs(statement: &Statement) -> Vec<(&Body, Frame)> {
    match &statement.kind {
        StatementKind::Binding { value, .. } => match value {
            Rhs::Match(branching) | Rhs::Handle(branching) => branching.arms.iter().map(|arm| (&arm.body, Frame::Arm)).collect(),
            Rhs::For(each) => vec![(&each.body, Frame::Loop)],
            Rhs::Loop(repeat) => vec![(&repeat.body, Frame::Loop)],
            Rhs::Call(_) | Rhs::Struct(_) | Rhs::Operand(_) => Vec::new(),
        },
        StatementKind::Call(_) | StatementKind::Gather(_, _) | StatementKind::Until(_) => Vec::new(),
    }
}

pub(crate) fn sub_mut(statement: &mut Statement, sub: usize) -> Option<&mut Body> {
    match &mut statement.kind {
        StatementKind::Binding { value, .. } => match value {
            Rhs::Match(branching) | Rhs::Handle(branching) => branching.arms.get_mut(sub).map(|arm| &mut arm.body),
            Rhs::For(each) if sub == 0 => Some(&mut each.body),
            Rhs::Loop(repeat) if sub == 0 => Some(&mut repeat.body),
            _ => None,
        },
        StatementKind::Call(_) | StatementKind::Gather(_, _) | StatementKind::Until(_) => None,
    }
}

pub(crate) fn body<'f>(function: &'f Function, path: &[Step]) -> Option<&'f Body> {
    let mut body = &function.body;
    for step in path {
        let statement = body.statements.get(step.stmt)?;
        body = subs(statement).into_iter().nth(step.sub)?.0;
    }
    Some(body)
}

pub(crate) fn body_mut<'f>(function: &'f mut Function, path: &[Step]) -> Option<&'f mut Body> {
    let mut body = &mut function.body;
    for step in path {
        let statement = body.statements.get_mut(step.stmt)?;
        body = sub_mut(statement, step.sub)?;
    }
    Some(body)
}

pub(crate) fn statement<'f>(function: &'f Function, loc: &Loc) -> Option<&'f Statement> {
    body(function, &loc.body)?.statements.get(loc.index)
}

pub(crate) fn statement_mut<'f>(function: &'f mut Function, loc: &Loc) -> Option<&'f mut Statement> {
    body_mut(function, &loc.body)?.statements.get_mut(loc.index)
}

/// The frame of the body at `path`: what kind of structure holds it, `None` for the body of the function.
pub(crate) fn frame(function: &Function, path: &[Step]) -> Option<Frame> {
    let (last, outer) = path.split_last()?;
    let statement = body(function, outer)?.statements.get(last.stmt)?;
    subs(statement).get(last.sub).map(|(_, frame)| *frame)
}

/// Every statement of the function, outer ones first.
pub(crate) fn all(function: &Function) -> Vec<Loc> {
    let mut found = Vec::new();
    let mut pending: Vec<BodyPath> = vec![Vec::new()];
    while let Some(path) = pending.pop() {
        let Some(body) = body(function, &path) else { continue };
        for (index, statement) in body.statements.iter().enumerate() {
            let loc = Loc { body: path.clone(), index };
            for sub in 0..subs(statement).len() {
                pending.push(loc.sub(sub));
            }
            found.push(loc);
        }
    }
    found.sort();
    found
}

/// The values that a statement binds where it stands: its targets that are names (spec 5.1). `next x` binds no new value.
pub(crate) fn targets(statement: &Statement) -> Vec<Name> {
    match &statement.kind {
        StatementKind::Binding { targets, .. } => targets
            .iter()
            .filter_map(|target| match target {
                Target::Name(_, name) => Some(*name),
                Target::Skip(_) | Target::Next(..) => None,
            })
            .collect(),
        StatementKind::Call(_) | StatementKind::Gather(_, _) | StatementKind::Until(_) => Vec::new(),
    }
}

/// The name of a line (spec 5.2): its first output that is neither `_` nor `next x`.
pub(crate) fn line_name(statement: &Statement) -> Option<Name> {
    targets(statement).first().copied()
}

/// The names that a body of a statement binds for itself, at its head: the bindings of the patterns of an arm, the `in`
/// clauses and the carries of a loop. They hide an outer value of the same name (R-3).
pub(crate) fn head_names(statement: &Statement, sub: usize) -> Vec<Name> {
    let StatementKind::Binding { value, .. } = &statement.kind else { return Vec::new() };
    match value {
        Rhs::Match(branching) | Rhs::Handle(branching) => branching
            .arms
            .get(sub)
            .map(|arm| arm.patterns().flat_map(|pattern| pattern.bindings().iter().map(|binding| binding.name)).collect())
            .unwrap_or_default(),
        Rhs::For(each) => each
            .clauses
            .iter()
            .map(|clause| match &clause.kind {
                ClauseKind::In(clause) => clause.name,
                ClauseKind::Carry(carry) => carry.name,
            })
            .collect(),
        Rhs::Loop(repeat) => repeat
            .clauses
            .iter()
            .filter_map(|clause| match &clause.kind {
                ClauseKind::Carry(carry) => Some(carry.name),
                ClauseKind::In(_) => None,
            })
            .collect(),
        Rhs::Call(_) | Rhs::Struct(_) | Rhs::Operand(_) => Vec::new(),
    }
}

/// Where a value is bound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Bound {
    /// An input of the function.
    Input,
    /// A target of a line, at this place among its targets.
    Line(Loc, usize),
    /// The head of a body of a structure: a pattern, an `in` clause, a carry.
    Head(Loc, usize),
}

/// A value of the function by its name, with the body where it is seen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Value {
    pub name: Name,
    pub bound: Bound,
    /// The body in which it is seen, and the bodies inside it.
    pub seen_in: BodyPath,
}

/// Every value of the function.
pub(crate) fn values(function: &Function) -> Vec<Value> {
    let mut found: Vec<Value> =
        function.signature.params.iter().map(|param| Value { name: param.name, bound: Bound::Input, seen_in: Vec::new() }).collect();
    for loc in all(function) {
        let Some(statement) = statement(function, &loc) else { continue };
        if let StatementKind::Binding { targets, .. } = &statement.kind {
            for (index, target) in targets.iter().enumerate() {
                if let Target::Name(_, name) = target {
                    found.push(Value { name: *name, bound: Bound::Line(loc.clone(), index), seen_in: loc.body.clone() });
                }
            }
        }
        for sub in 0..subs(statement).len() {
            for name in head_names(statement, sub) {
                found.push(Value { name, bound: Bound::Head(loc.clone(), sub), seen_in: loc.sub(sub) });
            }
        }
    }
    found
}

/// Whether a value seen in `seen_in` reaches a reader in `reader`, with nothing of the same name between them.
pub(crate) fn sees(function: &Function, value: &Value, reader: &[Step]) -> bool {
    if !reader.starts_with(&value.seen_in) {
        return false;
    }
    // A body between the two that binds the name for itself, at its head or by a line, hides the value.
    for depth in value.seen_in.len() + 1..=reader.len() {
        let Some(path) = reader.get(..depth) else { return false };
        let Some((last, outer)) = path.split_last() else { return false };
        let Some(statement) = body(function, outer).and_then(|body| body.statements.get(last.stmt)) else { return false };
        if head_names(statement, last.sub).contains(&value.name) {
            return false;
        }
        if body(function, path).is_some_and(|inner| inner.statements.iter().any(|line| targets(line).contains(&value.name))) {
            return false;
        }
    }
    true
}

/// The labels of the function, with their statements.
pub(crate) fn labels(function: &Function) -> Vec<(Name, Loc)> {
    all(function)
        .into_iter()
        .filter_map(|loc| statement(function, &loc).and_then(|statement| statement.label).map(|label| (label.name, loc)))
        .collect()
}
