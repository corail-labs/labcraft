//! The canonical order of the lines of a body (spec 5.2).
//!
//! The "repeatedly emit" of the spec is quadratic if written naively, and so is asking each body for what its
//! lines read, on a file nested 256 levels deep. So one walk serves every body: a name read anywhere goes to the
//! innermost body or structure that binds it, as a dependency of the line being walked there. Then each body
//! orders its lines with a binary heap of the lines that are ready.
//!
//! Dependencies come from syntax alone: `fmt` reads no other file (`docs/cli.md`, section 2).

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use labcraft_graph::{
    Arg, Arm, Body, Branching, Call, Clause, ClauseKind, For, Loop, Operand, OrderRef, Pattern, Rhs, Signature, Statement, StatementKind, Target,
    UnitTerm,
};
use labcraft_types::{Interner, Name};

/// Why a body keeps its written order, with the line at fault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unorderable {
    /// A value or a label defined twice in this body.
    DefinedTwice(usize),
    /// A line of a cycle: it waits for itself, through the others. The first of that cycle, in written order.
    Cycle(usize),
}

/// The order of the lines of `body`, as indices of its statements. `signature` says that this is the body of a
/// function: its output lines come last, and a name that is both an input and an output reads the input (R-2).
/// It walks everything inside `body`, so asking it for every body of a deep function is not linear.
pub fn canonical_order(body: &Body, signature: Option<&Signature>, names: &Interner) -> Result<Vec<usize>, Unorderable> {
    let mut orders = analyse(body, signature, names);
    match orders.pop() {
        Some(Ok(Some(order))) => Ok(order),
        Some(Err(fault)) => Err(fault),
        _ => Ok((0..body.statements.len()).collect()),
    }
}

/// What one body gets: nothing to change, a new order, or why it cannot have one.
pub(crate) type BodyOrder = Result<Option<Vec<usize>>, Unorderable>;

/// The order of `body` and of every body inside it, innermost first: [`sub_bodies`] walks them the same way.
pub(crate) fn analyse(body: &Body, signature: Option<&Signature>, names: &Interner) -> Vec<BodyOrder> {
    let mut analysis = Analysis { names, tops: HashMap::new(), scopes: Vec::new(), frames: Vec::new(), orders: Vec::new() };
    let terminals = signature.map(Terminals::of);
    let mark = analysis.scopes.len();
    analysis.open(body, terminals.as_ref());
    analysis.close(body, mark, terminals.as_ref());
    analysis.orders
}

/// The bodies directly inside a statement, in the order of the walk of [`analyse`].
pub(crate) fn sub_bodies(statement: &mut Statement) -> Vec<&mut Body> {
    let StatementKind::Binding { value, .. } = &mut statement.kind else {
        return Vec::new();
    };
    match value {
        Rhs::Match(branching) | Rhs::Handle(branching) => branching.arms.iter_mut().map(|arm| &mut arm.body).collect(),
        Rhs::For(structure) => vec![&mut structure.body],
        Rhs::Loop(structure) => vec![&mut structure.body],
        Rhs::Call(_) | Rhs::Struct(_) | Rhs::Operand(_) => Vec::new(),
    }
}

/// The inputs and outputs of the function, for its body only.
struct Terminals {
    outputs: HashMap<Name, usize>,
    inputs: HashSet<Name>,
}

impl Terminals {
    fn of(signature: &Signature) -> Self {
        let mut outputs = HashMap::with_capacity(signature.outputs.len());
        for (index, output) in signature.outputs.iter().enumerate() {
            outputs.entry(output.name).or_insert(index);
        }
        Terminals { outputs, inputs: signature.params.iter().map(|param| param.name).collect() }
    }
}

/// A name, or a label: `x` and `#x` are different things (`after:` and `layout` tell them apart).
type Key = (Name, bool);

#[derive(Clone, Copy)]
enum Binder {
    /// A carry, a `for` value or a pattern: no line to wait for.
    Structure,
    /// A line of the body of a frame. `readable` is false for an output that is also an input (R-2).
    Line { frame: usize, line: usize, readable: bool },
}

/// One entry of the stack of what is in scope, with what it hides.
struct Scoped {
    key: Key,
    binder: Binder,
    hides: Option<usize>,
}

/// A body being walked.
struct Frame {
    /// The line being walked; `None` once they are all walked, when a `yield` reads.
    current: Option<usize>,
    /// (the line that reads, the line that defines)
    edges: Vec<(usize, usize)>,
    twice: Option<usize>,
}

struct Analysis<'a> {
    names: &'a Interner,
    tops: HashMap<Key, usize>,
    scopes: Vec<Scoped>,
    frames: Vec<Frame>,
    orders: Vec<BodyOrder>,
}

impl Analysis<'_> {
    fn bind(&mut self, key: Key, binder: Binder) {
        let hides = self.tops.insert(key, self.scopes.len());
        self.scopes.push(Scoped { key, binder, hides });
    }

    fn unbind_to(&mut self, mark: usize) {
        while self.scopes.len() > mark {
            let Some(scoped) = self.scopes.pop() else { return };
            match scoped.hides {
                Some(index) => self.tops.insert(scoped.key, index),
                None => self.tops.remove(&scoped.key),
            };
        }
    }

    fn read(&mut self, key: Key) {
        let Some(scoped) = self.tops.get(&key).and_then(|index| self.scopes.get(*index)) else {
            return;
        };
        if let Binder::Line { frame, line, readable: true } = scoped.binder
            && let Some(frame) = self.frames.get_mut(frame)
            && let Some(current) = frame.current
            && current != line
        {
            frame.edges.push((current, line));
        }
    }

    /// Starts a body: all its lines are in scope, wherever they are written, and then each one is walked.
    fn open(&mut self, body: &Body, terminals: Option<&Terminals>) {
        let frame = self.frames.len();
        self.frames.push(Frame { current: None, edges: Vec::new(), twice: None });
        for (line, statement) in body.statements.iter().enumerate() {
            self.bind_line(frame, line, statement, terminals);
        }
        for (line, statement) in body.statements.iter().enumerate() {
            if let Some(frame) = self.frames.get_mut(frame) {
                frame.current = Some(line);
            }
            self.statement(statement);
        }
        if let Some(frame) = self.frames.get_mut(frame) {
            frame.current = None;
        }
    }

    /// Ends the body opened last: orders its lines, and forgets what was bound since `mark`.
    fn close(&mut self, body: &Body, mark: usize, terminals: Option<&Terminals>) {
        let frame = self.frames.pop();
        self.unbind_to(mark);
        if let Some(frame) = frame {
            self.orders.push(order(body, frame, terminals, self.names));
        }
    }

    fn bind_line(&mut self, frame: usize, line: usize, statement: &Statement, terminals: Option<&Terminals>) {
        if let Some(label) = statement.label {
            self.define((label.name, true), frame, line, true);
        }
        if let StatementKind::Binding { targets, .. } = &statement.kind {
            for target in targets {
                if let Target::Name(_, name) = target {
                    let both = terminals.is_some_and(|terminals| terminals.inputs.contains(name) && terminals.outputs.contains_key(name));
                    self.define((*name, false), frame, line, !both);
                }
            }
        }
    }

    fn define(&mut self, key: Key, frame: usize, line: usize, readable: bool) {
        let top = self.tops.get(&key).and_then(|index| self.scopes.get(*index));
        if let Some(Scoped { binder: Binder::Line { frame: bound_in, .. }, .. }) = top
            && *bound_in == frame
        {
            if let Some(frame) = self.frames.get_mut(frame) {
                frame.twice = frame.twice.or(Some(line));
            }
            return;
        }
        self.bind(key, Binder::Line { frame, line, readable });
    }

    fn statement(&mut self, statement: &Statement) {
        match &statement.kind {
            StatementKind::Binding { value, .. } => self.rhs(value),
            StatementKind::Call(call) => self.call(call),
            StatementKind::Collect(operand) | StatementKind::Until(operand) => self.operand(operand),
        }
    }

    fn rhs(&mut self, rhs: &Rhs) {
        match rhs {
            Rhs::Call(call) => self.call(call),
            Rhs::Match(branching) | Rhs::Handle(branching) => self.branching(branching),
            Rhs::For(structure) => {
                let For { clauses, body, .. } = &**structure;
                self.structure(clauses, body);
            }
            Rhs::Loop(structure) => {
                let Loop { clauses, body, .. } = &**structure;
                self.structure(clauses, body);
            }
            Rhs::Struct(literal) => {
                for field in &literal.fields {
                    self.operand(&field.value);
                }
            }
            Rhs::Operand(operand) => self.operand(operand),
        }
    }

    fn call(&mut self, call: &Call) {
        self.args(&call.args);
    }

    fn args(&mut self, args: &[Arg]) {
        for arg in args {
            match arg {
                Arg::Value { value, .. } => self.operand(value),
                Arg::After { refs, .. } => {
                    for order in refs {
                        match order {
                            OrderRef::Value(operand) => self.operand(operand),
                            OrderRef::Label(label) => self.read((label.name, true)),
                        }
                    }
                }
            }
        }
    }

    /// Every operand reads its name, or the first segment of its path. `[x]` reads `x`, whether it turns out to
    /// be a unit or an array; any other unit reads nothing.
    fn operand(&mut self, operand: &Operand) {
        match operand {
            Operand::Path(path) => self.read((path.first, false)),
            Operand::Array(_, values) => {
                for value in values {
                    self.operand(value);
                }
            }
            Operand::Bracket(_, unit) => {
                if let UnitTerm::Named { name, exponent: None } = unit.first
                    && unit.rest.is_empty()
                {
                    self.read((name, false));
                }
            }
            Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) => {}
        }
    }

    fn branching(&mut self, branching: &Branching) {
        self.operand(&branching.selector);
        for arm in &branching.arms {
            self.arm(arm);
        }
    }

    fn arm(&mut self, arm: &Arm) {
        let mark = self.scopes.len();
        for pattern in arm.patterns() {
            self.pattern(pattern);
        }
        self.open(&arm.body, None);
        for value in &arm.yields.values {
            self.operand(value);
        }
        self.close(&arm.body, mark, None);
    }

    fn pattern(&mut self, pattern: &Pattern) {
        match pattern {
            // An array in a pattern holds operands.
            Pattern::Literal(operand) => self.operand(operand),
            Pattern::Variant { bindings, .. } => {
                for binding in bindings {
                    self.bind((binding.name, false), Binder::Structure);
                }
            }
            Pattern::Wildcard { binding, .. } | Pattern::Category { binding, .. } | Pattern::Code { binding, .. } => {
                if let Some(binding) = binding {
                    self.bind((binding.name, false), Binder::Structure);
                }
            }
            Pattern::Range { .. } | Pattern::Ok { .. } => {}
        }
    }

    /// A clause reads from outside the structure, even a name that a carry then hides (`carry err = err`, R-3).
    fn structure(&mut self, clauses: &[Clause], body: &Body) {
        for clause in clauses {
            match &clause.kind {
                ClauseKind::In(clause) => {
                    self.operand(&clause.source);
                    if let Some(end) = &clause.end {
                        self.operand(end);
                    }
                }
                ClauseKind::Carry(carry) => self.operand(&carry.value),
            }
        }
        let mark = self.scopes.len();
        for clause in clauses {
            let name = match &clause.kind {
                ClauseKind::In(clause) => clause.name,
                ClauseKind::Carry(carry) => carry.name,
            };
            self.bind((name, false), Binder::Structure);
        }
        self.open(body, None);
        self.close(body, mark, None);
    }
}

/// What decides which ready line comes first. The derived order compares in this order, and the written order
/// breaks ties.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Rank<'a> {
    category: u8,
    name: &'a str,
    output: usize,
    written: usize,
}

/// The name of a line, without its label (5.2): its first output that is neither `_` nor `next x`.
pub(crate) fn first_name(targets: &[Target]) -> Option<Name> {
    targets.iter().find_map(|target| match target {
        Target::Name(_, name) => Some(*name),
        Target::Skip(_) | Target::Next(..) => None,
    })
}

fn rank<'a>(statement: &Statement, written: usize, terminals: Option<&Terminals>, names: &'a Interner) -> Rank<'a> {
    let ordinary = |name: Option<Name>| Rank { category: 0, name: name.map_or("", |name| names.resolve(name)), output: 0, written };
    let label = statement.label.map(|label| label.name);
    match &statement.kind {
        StatementKind::Call(_) => ordinary(label),
        StatementKind::Collect(_) => Rank { category: 2, name: "", output: 0, written },
        StatementKind::Until(_) => Rank { category: 3, name: "", output: 0, written },
        StatementKind::Binding { targets, value } => {
            if let (Some(terminals), Rhs::Operand(_), [Target::Name(_, name)]) = (terminals, value, &**targets)
                && let Some(output) = terminals.outputs.get(name)
            {
                return Rank { category: 4, name: "", output: *output, written };
            }
            match targets.iter().find(|target| !matches!(target, Target::Skip(_))) {
                Some(Target::Next(_, carry)) => Rank { category: 1, name: names.resolve(*carry), output: 0, written },
                _ => ordinary(first_name(targets).or(label)),
            }
        }
    }
}

/// The first line, in written order, of a cycle among the lines still waiting. Each one waits for another of them,
/// so following them from any one of them ends in a cycle.
fn line_in_a_cycle(waiting: &[u32], edges: &[(usize, usize)]) -> usize {
    let is_waiting = |line: usize| waiting.get(line).is_some_and(|waits| *waits > 0);
    let mut waits_for = vec![usize::MAX; waiting.len()];
    for (reader, definer) in edges {
        if is_waiting(*definer)
            && let Some(slot) = waits_for.get_mut(*reader)
        {
            *slot = *definer;
        }
    }
    let next = |line: usize| waits_for.get(line).copied().unwrap_or(usize::MAX);
    let mut line = waiting.iter().position(|waits| *waits > 0).unwrap_or(0);
    for _ in 0..waiting.len() {
        line = next(line);
    }
    let start = line;
    let mut first = line;
    line = next(line);
    while line != start && line != usize::MAX {
        first = first.min(line);
        line = next(line);
    }
    first
}

fn order(body: &Body, frame: Frame, terminals: Option<&Terminals>, names: &Interner) -> BodyOrder {
    if let Some(line) = frame.twice {
        return Err(Unorderable::DefinedTwice(line));
    }
    let count = body.statements.len();
    if count < 2 {
        return Ok(None);
    }
    let mut waiting = vec![0_u32; count];
    // The readers of each line, as linked lists through `edges`.
    let mut first_reader = vec![usize::MAX; count];
    let mut next_reader = vec![usize::MAX; frame.edges.len()];
    for (index, (reader, definer)) in frame.edges.iter().enumerate() {
        if let (Some(waits), Some(first), Some(next)) = (waiting.get_mut(*reader), first_reader.get_mut(*definer), next_reader.get_mut(index)) {
            *waits += 1;
            *next = *first;
            *first = index;
        }
    }
    let ranks: Vec<Rank> = body.statements.iter().enumerate().map(|(written, statement)| rank(statement, written, terminals, names)).collect();
    let mut ready = BinaryHeap::new();
    for (line_rank, waits) in ranks.iter().zip(&waiting) {
        if *waits == 0 {
            ready.push(Reverse(line_rank));
        }
    }
    let mut result = Vec::with_capacity(count);
    while let Some(Reverse(Rank { written: line, .. })) = ready.pop() {
        let line = *line;
        result.push(line);
        let mut reader = first_reader.get(line).copied().unwrap_or(usize::MAX);
        while let Some((reads, next)) = frame.edges.get(reader).zip(next_reader.get(reader)) {
            if let Some(waits) = waiting.get_mut(reads.0) {
                *waits -= 1;
                if *waits == 0
                    && let Some(line_rank) = ranks.get(reads.0)
                {
                    ready.push(Reverse(line_rank));
                }
            }
            reader = *next;
        }
    }
    if result.len() < count {
        return Err(Unorderable::Cycle(line_in_a_cycle(&waiting, &frame.edges)));
    }
    if result.iter().enumerate().all(|(index, line)| index == *line) { Ok(None) } else { Ok(Some(result)) }
}
