//! Pass 5 (`docs/architecture.md`, section 5): the dataflow of a function, once its types are known. Cycles (`LC4001`), outputs and
//! carries bound once (`LC4002`, `LC4003`), `for parallel` (`LC4004`, `LC4104`), the `err` outputs wired nowhere (`LC4101`), and who
//! reaches a resource or a panel element with nothing to order them (`LC4102`, `LC4103`, `LC4105`).
//!
//! It has its own graph, since an order wire counts here and not for types: one graph for the whole function, as pass 4 has, and a
//! function is judged against the signatures of the others, never by following a call. So it assumes nothing about a node: any node
//! that receives a resource may change it, and an output of the type of a resource that a node received continues that resource (8.1).

use std::collections::{HashMap, HashSet, VecDeque};

use labcraft_graph::{
    Arg, Body, Branching, Call, Clause, ClauseKind, Function, Id, Operand, OrderRef, PanelItem, Pattern, Rhs, Signature, Statement, StatementKind,
    Target as Line,
};
use labcraft_types::text::grouped;
use labcraft_types::{Code, Diagnostic, Name, Related, Ty, TyId, cut_text};

use super::super::{Pass, Scope, path_text};
use super::types::panel::ui_node;
use super::{Context, definition_of, node_of, targets_of};
use crate::load::{at, cycles};
use crate::passed_in::MAX_PASSED_IN;
use crate::primitives::PrimitiveLibrary;
use crate::resolved::{Meaning, Resolved};
use crate::terminals::MAX_TERMINALS;

#[cfg(test)]
mod tests;

const NONE: u32 = u32::MAX;

/// The names that the message of a cycle gives.
const NAMES_SHOWN: usize = 5;

/// The resources that one value can continue: a chain of nodes that each take two sessions would otherwise double them at each line.
const MAX_REFS: usize = 16;

/// The work allowed on questions of order, by item of the graph and at least `MIN_WORK`, so that many resources cannot make it quadratic
/// (`docs/architecture.md`, section 5). Beyond it nothing more is said, and that is a warning less on a file built for it.
const WORK_PER_ITEM: u64 = 64;
const MIN_WORK: u64 = 1 << 24;

/// What only this pass says: the programs that test the other passes are written for what those judge.
#[cfg(test)]
pub(in crate::declare) fn only_this_pass(code: Code) -> bool {
    matches!(
        code,
        Code::Lc4001
            | Code::Lc4002
            | Code::Lc4003
            | Code::Lc4004
            | Code::Lc4007
            | Code::Lc4008
            | Code::Lc4101
            | Code::Lc4102
            | Code::Lc4103
            | Code::Lc4104
            | Code::Lc4105
    )
}

/// A line, or one of the two moments of a structure.
#[derive(Clone, Copy)]
struct Step<'a> {
    statement: &'a Statement,
    /// What a structure gives, after its body. Its selector or its clauses are the step before.
    closing: bool,
    arm: u32,
    parallel: u32,
}

/// An arm of a `match` or a `handle`, by the step that opens it.
#[derive(Clone, Copy)]
struct ArmPlace {
    parent: u32,
    structure: u32,
    depth: u32,
}

/// The body of a `for parallel`.
#[derive(Clone, Copy)]
struct ParallelPlace<'a> {
    parent: u32,
    depth: u32,
    statement: &'a Statement,
}

/// A loop, for what its carries have to have.
#[derive(Clone, Copy)]
struct LoopLines<'a> {
    clauses: &'a [Clause],
    body: &'a Body,
    parallel: bool,
}

struct Graph<'a, 'r> {
    resolved: &'r Resolved,
    steps: Vec<Step<'a>>,
    edges: Vec<(u32, u32)>,
    /// Who waits for what: the id of a value, and the step. An order wire counts.
    reads: Vec<(Id, u32)>,
    /// What wires a value on: `x = y` only names it, and what counts there is who reads `x`.
    wired: Vec<Id>,
    aliases: HashMap<Id, Id>,
    /// By id, the step that gives a value. A line that binds an output counts for that output (R-2), which a program can read back.
    definers: Vec<u32>,
    /// What binds each output, in written order.
    bound: Vec<(Id, Id)>,
    labels: HashMap<Id, u32>,
    label_reads: Vec<(Id, u32)>,
    arms: Vec<ArmPlace>,
    parallels: Vec<ParallelPlace<'a>>,
    arm: u32,
    parallel: u32,
    loops: Vec<LoopLines<'a>>,
    /// By the id of a statement, the body of its `for parallel`.
    parallel_of: HashMap<Id, u32>,
}

impl<'a, 'r> Graph<'a, 'r> {
    fn of(function: &'a Function, resolved: &'r Resolved, places: usize) -> Self {
        let mut graph = Graph {
            resolved,
            steps: Vec::new(),
            edges: Vec::new(),
            reads: Vec::new(),
            wired: Vec::new(),
            aliases: HashMap::new(),
            definers: vec![NONE; places],
            bound: Vec::new(),
            labels: HashMap::new(),
            label_reads: Vec::new(),
            arms: Vec::new(),
            parallels: Vec::new(),
            arm: NONE,
            parallel: NONE,
            loops: Vec::new(),
            parallel_of: HashMap::new(),
        };
        graph.body(&function.body, None);
        graph.join();
        graph
    }

    fn join(&mut self) {
        for (value, reader) in std::mem::take(&mut self.reads) {
            if let Some(definer) = self.definers.get(value.index()).copied().filter(|definer| *definer != NONE) {
                self.edges.push((definer, reader));
            }
        }
        for (label, reader) in std::mem::take(&mut self.label_reads) {
            if let Meaning::Label(target) = self.resolved.of(label)
                && let Some(definer) = self.labels.get(&target)
            {
                self.edges.push((*definer, reader));
            }
        }
    }

    fn add(&mut self, statement: &'a Statement, closing: bool) -> u32 {
        let index = u32::try_from(self.steps.len()).unwrap_or(NONE);
        self.steps.push(Step { statement, closing, arm: self.arm, parallel: self.parallel });
        index
    }

    fn body(&mut self, body: &'a Body, around: Option<(u32, u32)>) {
        for statement in &body.statements {
            self.statement(statement, around);
        }
    }

    fn statement(&mut self, statement: &'a Statement, around: Option<(u32, u32)>) {
        let is_structure =
            matches!(&statement.kind, StatementKind::Binding { value: Rhs::Match(_) | Rhs::Handle(_) | Rhs::For(_) | Rhs::Loop(_), .. });
        let first = self.add(statement, false);
        let last = if is_structure { self.add(statement, true) } else { first };
        if is_structure {
            self.edges.push((first, last));
        }
        if let Some((open, close)) = around {
            self.edges.push((open, first));
            self.edges.push((last, close));
        }
        if let Some(label) = &statement.label {
            self.labels.entry(label.id).or_insert(last);
        }
        for target in targets_of(statement) {
            let Line::Name(id, _) = target else { continue };
            let output = match self.resolved.of(*id) {
                Meaning::Value(output) => {
                    self.bound.push((output, *id));
                    Some(output)
                }
                _ => None,
            };
            let value = output.unwrap_or(*id);
            // A second binding of an output has its `LC4002`, and what it might still hide comes back once that is fixed.
            if let Some(slot) = self.definers.get_mut(value.index())
                && *slot == NONE
            {
                *slot = last;
            }
        }
        match &statement.kind {
            StatementKind::Call(call) => self.call(call, first),
            StatementKind::Collect(value) | StatementKind::Until(value) => self.read(value, first),
            StatementKind::Binding { targets, value } => match value {
                Rhs::Call(call) => self.call(call, first),
                Rhs::Struct(literal) => {
                    for field in &literal.fields {
                        self.read(&field.value, first);
                    }
                }
                Rhs::Operand(operand) => self.naming(targets, operand, first),
                Rhs::Match(branching) | Rhs::Handle(branching) => self.branching(branching, first, last),
                Rhs::For(structure) => {
                    self.loops.push(LoopLines { clauses: &structure.clauses, body: &structure.body, parallel: structure.parallel });
                    self.clauses(&structure.clauses, first);
                    let outer = self.parallel;
                    if structure.parallel {
                        let depth = self.parallels.get(outer as usize).map_or(0, |place| place.depth) + 1;
                        self.parallel = u32::try_from(self.parallels.len()).unwrap_or(NONE);
                        self.parallels.push(ParallelPlace { parent: outer, depth, statement });
                        self.parallel_of.insert(statement.id, self.parallel);
                    }
                    self.body(&structure.body, Some((first, last)));
                    self.parallel = outer;
                }
                Rhs::Loop(structure) => {
                    self.loops.push(LoopLines { clauses: &structure.clauses, body: &structure.body, parallel: false });
                    self.clauses(&structure.clauses, first);
                    self.body(&structure.body, Some((first, last)));
                }
            },
        }
    }

    /// `x = y` makes no node (5.1), so the wire of `y` goes on when `x` is read. `_ = y` wires it nowhere.
    fn naming(&mut self, targets: &'a [Line], operand: &'a Operand, step: u32) {
        if let (Operand::Path(path), [target]) = (operand, targets)
            && path.rest.is_empty()
            && let Meaning::Value(value) = self.resolved.of(path.id)
        {
            self.reads.push((value, step));
            match target {
                Line::Skip(_) => return,
                Line::Name(id, _) if !matches!(self.resolved.of(*id), Meaning::Value(_)) => {
                    self.aliases.insert(*id, value);
                    return;
                }
                _ => {
                    self.wired.push(value);
                    return;
                }
            }
        }
        self.read(operand, step);
    }

    fn branching(&mut self, branching: &'a Branching, first: u32, last: u32) {
        self.read(&branching.selector, first);
        let outer = self.arm;
        let depth = self.arms.get(outer as usize).map_or(0, |place| place.depth) + 1;
        for arm in &branching.arms {
            for pattern in arm.patterns() {
                if let Pattern::Literal(operand) = pattern {
                    self.read(operand, first);
                }
            }
            self.arm = u32::try_from(self.arms.len()).unwrap_or(NONE);
            self.arms.push(ArmPlace { parent: outer, structure: first, depth });
            self.body(&arm.body, Some((first, last)));
            self.arm = outer;
            for value in &arm.yields.values {
                self.read(value, last);
            }
        }
    }

    fn clauses(&mut self, clauses: &'a [Clause], first: u32) {
        for clause in clauses {
            match &clause.kind {
                ClauseKind::In(clause) => {
                    self.read(&clause.source, first);
                    if let Some(end) = &clause.end {
                        self.read(end, first);
                    }
                }
                ClauseKind::Carry(carry) => self.read(&carry.value, first),
            }
        }
    }

    fn call(&mut self, call: &'a Call, step: u32) {
        for arg in &call.args {
            match arg {
                Arg::Value { value, .. } => self.read(value, step),
                Arg::After { refs, .. } => {
                    for order in refs {
                        match order {
                            OrderRef::Value(value) => self.read(value, step),
                            OrderRef::Label(label) => self.label_reads.push((label.id, step)),
                        }
                    }
                }
            }
        }
    }

    fn read(&mut self, operand: &Operand, step: u32) {
        match operand {
            Operand::Path(path) => {
                if let Meaning::Value(value) = self.resolved.of(path.id) {
                    self.reads.push((value, step));
                    self.wired.push(value);
                }
            }
            Operand::Array(_, values) => {
                for value in values {
                    self.read(value, step);
                }
            }
            Operand::Bracket(id, _) => {
                if let Meaning::ArrayOfValue(value) = self.resolved.of(*id) {
                    self.reads.push((value, step));
                    self.wired.push(value);
                }
            }
            Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) => {}
        }
    }

    /// Whether two steps are in different arms of one structure: only one of them runs (8.1).
    fn exclusive(&self, a: u32, b: u32) -> bool {
        let place = |index: u32| self.arms.get(index as usize).copied();
        let (mut a, mut b) = (self.steps.get(a as usize).map_or(NONE, |step| step.arm), self.steps.get(b as usize).map_or(NONE, |step| step.arm));
        let depth = |index: u32| place(index).map_or(0, |place| place.depth);
        while depth(a) > depth(b) {
            a = place(a).map_or(NONE, |place| place.parent);
        }
        while depth(b) > depth(a) {
            b = place(b).map_or(NONE, |place| place.parent);
        }
        while a != b {
            let (Some(first), Some(second)) = (place(a), place(b)) else { return false };
            if first.structure == second.structure {
                return true;
            }
            (a, b) = (first.parent, second.parent);
        }
        false
    }

    fn arm_of(&self, step: u32) -> u32 {
        self.steps.get(step as usize).map_or(NONE, |step| step.arm)
    }

    fn arm_depth(&self, step: u32) -> u32 {
        self.arms.get(self.arm_of(step) as usize).map_or(0, |place| place.depth)
    }

    /// Whether every arm that holds `outer` also holds `inner`: what is exclusive with `outer` then is with `inner` too.
    fn arms_cover(&self, outer: u32, inner: u32) -> bool {
        let target = self.arm_of(outer);
        let depth = self.arm_depth(outer);
        let mut current = self.arm_of(inner);
        while self.arms.get(current as usize).map_or(0, |place| place.depth) > depth {
            current = self.arms.get(current as usize).map_or(NONE, |place| place.parent);
        }
        current == target
    }

    /// The innermost `for parallel` that holds both, or `NONE`.
    fn common_parallel(&self, a: u32, b: u32) -> u32 {
        let place = |index: u32| self.parallels.get(index as usize).copied();
        let depth = |index: u32| place(index).map_or(0, |place| place.depth);
        let (mut a, mut b) = (a, b);
        while depth(a) > depth(b) {
            a = place(a).map_or(NONE, |place| place.parent);
        }
        while depth(b) > depth(a) {
            b = place(b).map_or(NONE, |place| place.parent);
        }
        while a != b {
            a = place(a).map_or(NONE, |place| place.parent);
            b = place(b).map_or(NONE, |place| place.parent);
        }
        a
    }
}

/// The steps in an order that puts each one after what it waits for, and what follows each one. What is in a cycle does not come out.
struct Order {
    steps: Vec<u32>,
    /// By step, its place in `steps`.
    ranks: Vec<u32>,
    starts: Vec<usize>,
    followers: Vec<u32>,
}

impl Order {
    fn of(count: usize, edges: &[(u32, u32)]) -> Self {
        let mut waits_for = vec![0_u32; count];
        let mut starts = vec![0_usize; count + 1];
        for (from, to) in edges {
            if let Some(slot) = waits_for.get_mut(*to as usize) {
                *slot = slot.saturating_add(1);
            }
            if let Some(slot) = starts.get_mut(*from as usize + 1) {
                *slot += 1;
            }
        }
        for index in 0..count {
            let before = starts.get(index).copied().unwrap_or(0);
            if let Some(slot) = starts.get_mut(index + 1) {
                *slot += before;
            }
        }
        let mut next_free = starts.clone();
        let mut followers = vec![0_u32; edges.len()];
        for (from, to) in edges {
            if let Some(place) = next_free.get_mut(*from as usize) {
                if let Some(slot) = followers.get_mut(*place) {
                    *slot = *to;
                }
                *place += 1;
            }
        }
        let mut queue: VecDeque<u32> = waits_for.iter().enumerate().filter(|(_, waits)| **waits == 0).map(|(index, _)| index as u32).collect();
        let mut steps = Vec::with_capacity(count);
        let mut ranks = vec![NONE; count];
        while let Some(index) = queue.pop_front() {
            if let Some(rank) = ranks.get_mut(index as usize) {
                *rank = u32::try_from(steps.len()).unwrap_or(NONE);
            }
            steps.push(index);
            let start = starts.get(index as usize).copied().unwrap_or(0);
            let end = starts.get(index as usize + 1).copied().unwrap_or(start);
            for follower in followers.get(start..end).unwrap_or(&[]) {
                if let Some(slot) = waits_for.get_mut(*follower as usize) {
                    *slot -= 1;
                    if *slot == 0 {
                        queue.push_back(*follower);
                    }
                }
            }
        }
        Order { steps, ranks, starts, followers }
    }

    fn followers(&self, step: u32) -> &[u32] {
        let start = self.starts.get(step as usize).copied().unwrap_or(0);
        let end = self.starts.get(step as usize + 1).copied().unwrap_or(start);
        self.followers.get(start..end).unwrap_or(&[])
    }
}

/// What a value designates: a resource, or a part of one (`bench.dmm`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Ref {
    root: u32,
    path: u32,
    /// The outermost `for parallel` whose `in` clause gives this element, through the `in` clauses of the loops inside it: each iteration of
    /// these has its own, and those around them share it (8.1).
    own: u32,
}

/// `found` stays short and without repeats: an array of millions of wires can name one session. True when `reference` is left
/// out for want of room, which says less (`Checked::cut_short`).
#[must_use]
fn push_ref(found: &mut Vec<Ref>, reference: Ref) -> bool {
    if found.contains(&reference) {
        return false;
    }
    if found.len() < MAX_REFS {
        found.push(reference);
        return false;
    }
    true
}

#[derive(Clone, Copy)]
struct Access {
    path: u32,
    step: u32,
}

/// What stands before a node, with the last node it was found to reach: what reaches that one reaches this one too, with no new search.
#[derive(Clone, Copy)]
struct Standing {
    access: Access,
    reached: u32,
}

struct Root {
    name: Name,
    /// The body of the `for parallel` that it was born in.
    parallel: u32,
    accesses: Vec<Access>,
}

/// Which resources each value continues.
#[derive(Default)]
struct Resources {
    /// Some value had more than `MAX_REFS` resources.
    cut_short: bool,
    roots: Vec<Root>,
    /// The parts of a resource: `paths[n - 1]` is part `n`, and 0 is the whole of it.
    paths: Vec<(u32, Name, u32)>,
    path_ids: HashMap<(u32, Name), u32>,
    values: HashMap<Id, Vec<Ref>>,
    /// What a `next` gives each carry: it counts for what the loop gives, and what reads the carry in the body does not depend on it, since
    /// whether that line was walked before the `next` would then decide.
    next_refs: HashMap<Id, Vec<Ref>>,
    /// What a body of a `for parallel` uses, in written order once sorted.
    used_in_parallel: Vec<(u32, Ref)>,
    /// What a `ui.set` writes and what a `ui.get` reads, by element, in the order they were met.
    writes: HashMap<Id, Vec<u32>>,
    write_order: Vec<Id>,
    reads: HashMap<Id, Vec<u32>>,
    read_order: Vec<Id>,
}

impl Resources {
    fn part(&mut self, parent: u32, field: Name) -> u32 {
        if let Some(known) = self.path_ids.get(&(parent, field)) {
            return *known;
        }
        let depth = self.depth(parent) + 1;
        self.paths.push((parent, field, depth));
        let id = u32::try_from(self.paths.len()).unwrap_or(NONE);
        self.path_ids.insert((parent, field), id);
        id
    }

    fn depth(&self, path: u32) -> u32 {
        path.checked_sub(1).and_then(|index| self.paths.get(index as usize)).map_or(0, |part| part.2)
    }

    fn parent(&self, path: u32) -> u32 {
        path.checked_sub(1).and_then(|index| self.paths.get(index as usize)).map_or(0, |part| part.0)
    }

    /// `bench` overlaps `bench.dmm`, and `bench.dmm` does not overlap `bench.source` (8.1).
    fn overlap(&self, a: u32, b: u32) -> bool {
        if self.depth(a) <= self.depth(b) { self.covers(a, b) } else { self.covers(b, a) }
    }

    /// Whether `outer` holds `inner`, or is it.
    fn covers(&self, outer: u32, inner: u32) -> bool {
        let mut inner = inner;
        while self.depth(inner) > self.depth(outer) {
            inner = self.parent(inner);
        }
        inner == outer
    }

    fn add_refs(&mut self, value: Id, refs: &[Ref]) {
        let known = self.values.entry(value).or_default();
        for found in refs {
            self.cut_short |= push_ref(known, *found);
        }
    }

    fn born(&mut self, value: Id, name: Name, parallel: u32) {
        let root = u32::try_from(self.roots.len()).unwrap_or(NONE);
        self.roots.push(Root { name, parallel, accesses: Vec::new() });
        self.add_refs(value, &[Ref { root, path: 0, own: NONE }]);
    }

    fn text_of(&self, reference: Ref, names: &labcraft_types::Interner) -> String {
        let mut parts = Vec::new();
        let mut path = reference.path;
        while let Some(part) = path.checked_sub(1).and_then(|index| self.paths.get(index as usize)) {
            parts.push(names.resolve(part.1));
            path = part.0;
        }
        let first = self.roots.get(reference.root as usize).map_or("", |root| names.resolve(root.name));
        parts.push(first);
        parts.reverse();
        cut_text(&parts.join(".")).into_owned()
    }
}

/// Whether a value of this type holds a resource: a type that a manifest declares, or a struct, a variant or an array with one in it.
fn holds_resource(pass: &Pass<'_>, context: &mut Context<'_, '_>, ty: TyId) -> bool {
    if context.facts.resources.is_empty() || ty == TyId::ERROR {
        return false;
    }
    if context.facts.resources.contains(&ty) {
        return true;
    }
    if let Some(known) = context.facts.holds.get(&ty) {
        return *known;
    }
    context.facts.holds.insert(ty, false);
    let definition = definition_of(pass, context, ty);
    let parts: Vec<TyId> = match pass.table.get(definition) {
        Ty::Array { element, .. } => vec![*element],
        Ty::Struct(fields) => fields.iter().map(|(_, field)| *field).collect(),
        Ty::Enum(variants) => variants.iter().flat_map(|variant| variant.values.iter().copied()).collect(),
        Ty::Waveform(element) | Ty::Queue(element) | Ty::Ref(element) => vec![*element],
        _ => Vec::new(),
    };
    let holds = context.facts.resources.contains(&definition) || parts.into_iter().any(|part| holds_resource(pass, context, part));
    context.facts.holds.insert(ty, holds);
    holds
}

/// What judges one function.
struct Flow<'p, 'c, 'w, 'a, 'g> {
    pass: &'p mut Pass<'a>,
    context: &'c mut Context<'w, 'a>,
    scope: Scope<'a>,
    resolved: &'g Resolved,
    graph: Graph<'a, 'g>,
    /// The elements of the panel, by the id of the item.
    items: HashMap<Id, &'a PanelItem>,
    /// What is left of the work allowed on questions of order.
    work: u64,
}

/// Judges the dataflow of one function.
pub(super) fn judge_function<'a>(
    pass: &mut Pass<'a>,
    context: &mut Context<'_, 'a>,
    scope: Scope<'a>,
    resolved: &Resolved,
    name: Name,
    signature: &'a Signature,
    function: &'a Function,
) {
    let graph = Graph::of(function, resolved, scope.model.source_map.len());
    let declared = context.declared.get(scope.library).and_then(|library| library.functions.get(&name));
    let faulty = declared.is_none_or(|declared| declared.faulty);
    let generic = declared.is_some_and(|declared| declared.generic);
    let items = scope.model.file.panel.iter().flat_map(|panel| &panel.items).map(|item| (item.id, item)).collect();
    let items_of_graph = u64::try_from(graph.steps.len() + graph.edges.len()).unwrap_or(u64::MAX);
    let work = items_of_graph.saturating_mul(WORK_PER_ITEM).max(MIN_WORK);
    let mut flow = Flow { pass, context, scope, resolved, graph, items, work };
    if !faulty {
        flow.judge_outputs(signature);
    }
    flow.judge_carries();
    flow.judge_passed_in(function);
    flow.judge_terminals(function);
    let order = Order::of(flow.graph.steps.len(), &flow.graph.edges);
    if order.steps.len() < flow.graph.steps.len() {
        flow.judge_cycles();
    }
    // An error of this function can be the cause of a warning: a wire with a mistake in its name reads nothing. They come once it has none.
    let errors = flow.pass.reports.get(flow.scope.library).map_or(0, labcraft_types::Report::errors);
    if errors != flow.pass.function_errors || faulty || generic {
        return;
    }
    flow.judge_err_outputs();
    let resources = flow.follow_resources(signature, &order);
    flow.judge_buttons(&resources);
    flow.judge_order(&resources, &order);
    flow.judge_parallel(&resources);
}

fn call_of(statement: &Statement) -> Option<&Call> {
    match &statement.kind {
        StatementKind::Call(call) | StatementKind::Binding { value: Rhs::Call(call), .. } => Some(call),
        _ => None,
    }
}

impl<'a> Flow<'_, '_, '_, 'a, '_> {
    fn names(&self) -> &'a labcraft_types::Interner {
        self.pass.names
    }

    fn say(&mut self, code: Code, message: String, id: Id, statement: Option<&Statement>, related: Vec<Related>) {
        let mut diagnostic: Diagnostic = at(code, message, &self.scope.file.shown, &self.scope.model.source_map, id);
        diagnostic.function = self.scope.function.map(|name| cut_text(name).into_owned());
        diagnostic.node = statement.map(|statement| node_of(statement, self.pass.names));
        diagnostic.related = related;
        self.pass.push(self.scope.library, diagnostic);
    }

    fn here(&self, message: String, id: Id) -> Related {
        Related { message, file: self.scope.file.shown.clone(), span: self.scope.model.source_map.span(id) }
    }

    /// Spec 6: the values that the structures pass in are bounded, since each one is built when the function is lowered.
    fn judge_passed_in(&mut self, function: &'a Function) {
        let found = crate::passed_in::passed_in(function, self.resolved, MAX_PASSED_IN);
        let Some((statement, weight)) = found.over else { return };
        let message = format!(
            "the structures of this function pass in more than {} values, the limit, {} of them in this one: a value counts on each \
             frame and in each region it enters, each arm (spec 6)",
            grouped(MAX_PASSED_IN),
            grouped(weight)
        );
        self.say(Code::Lc4007, message, statement.id, Some(statement), Vec::new());
    }

    /// Spec 4: the terminals of the calls are bounded, since each one is built when the function is lowered.
    fn judge_terminals(&mut self, function: &'a Function) {
        let found = crate::terminals::terminals(function, self.resolved, self.context.declared, self.pass.names, MAX_TERMINALS);
        let Some((statement, weight)) = found.over else { return };
        let message = format!(
            "the calls of this function have more than {} terminals, the limit, {} on this one: an input per parameter of what it \
             calls, defaults included, and an output per output (spec 4)",
            grouped(MAX_TERMINALS),
            grouped(weight)
        );
        self.say(Code::Lc4008, message, statement.id, Some(statement), Vec::new());
    }

    /// R-2: an output is bound exactly once.
    fn judge_outputs(&mut self, signature: &'a Signature) {
        let mut first: HashMap<Id, Id> = HashMap::with_capacity(self.graph.bound.len());
        let mut twice = HashSet::new();
        let names = self.names();
        let by_id: HashMap<Id, Name> = signature.outputs.iter().map(|output| (output.id, output.name)).collect();
        for (output, line) in std::mem::take(&mut self.graph.bound) {
            let Some(known) = first.get(&output).copied() else {
                first.insert(output, line);
                continue;
            };
            if !twice.insert(output) {
                continue;
            }
            let Some(name) = by_id.get(&output) else { continue };
            let text = cut_text(names.resolve(*name)).into_owned();
            let related = vec![self.here(format!("`{text}` is first bound here"), known)];
            self.say(Code::Lc4002, format!("the output `{text}` is bound twice: a value is defined once (R-2)"), line, None, related);
        }
        let mut seen = HashSet::with_capacity(signature.outputs.len());
        for output in &signature.outputs {
            if seen.insert(output.name) && !first.contains_key(&output.id) {
                let text = cut_text(names.resolve(output.name)).into_owned();
                self.say(Code::Lc4002, format!("the output `{text}` is never bound (R-2)"), output.id, None, Vec::new());
            }
        }
    }

    /// R-6: a carry has exactly one `next`. 6.2: `for parallel` has none, and that says enough about its carries.
    fn judge_carries(&mut self) {
        let names = self.names();
        for structure in std::mem::take(&mut self.graph.loops) {
            let carries = structure.clauses.iter().filter_map(|clause| match &clause.kind {
                ClauseKind::Carry(carry) => Some(carry),
                ClauseKind::In(_) => None,
            });
            if structure.parallel {
                if let Some(carry) = carries.clone().next() {
                    let why = "its iterations have no order, so nothing can pass from one to the next";
                    self.say(Code::Lc4004, format!("a `for parallel` takes no `carry`: {why} (6.2)"), carry.id, None, Vec::new());
                }
                continue;
            }
            let mut first: HashMap<Id, Id> = HashMap::new();
            let mut twice = HashSet::new();
            let resolved = self.resolved;
            for statement in &structure.body.statements {
                for target in targets_of(statement) {
                    let Line::Next(id, name) = target else { continue };
                    let Meaning::Value(carry) = resolved.of(*id) else { continue };
                    let Some(known) = first.get(&carry).copied() else {
                        first.insert(carry, *id);
                        continue;
                    };
                    if twice.insert(carry) {
                        let text = cut_text(names.resolve(*name)).into_owned();
                        let related = vec![self.here(format!("`next {text}` is first written here"), known)];
                        self.say(Code::Lc4003, format!("the carry `{text}` has two `next` (R-6)"), *id, Some(statement), related);
                    }
                }
            }
            // A `next` that names no carry has its `LC2018`, and may be the one that is missing here.
            let mistaken = structure
                .body
                .statements
                .iter()
                .flat_map(targets_of)
                .any(|target| matches!(target, Line::Next(id, _) if !matches!(resolved.of(*id), Meaning::Value(_))));
            if mistaken {
                continue;
            }
            let mut seen = HashSet::new();
            for carry in carries {
                // Two carries of one name have their `LC2012`, and the second one has no `next` to find.
                if seen.insert(carry.name) && !first.contains_key(&carry.id) {
                    let text = cut_text(names.resolve(carry.name)).into_owned();
                    self.say(Code::Lc4003, format!("the carry `{text}` has no `next` (R-6)"), carry.id, None, Vec::new());
                }
            }
        }
    }

    /// 5.2: `LC4001`, once per cycle, on its first line. What only waits for a cycle has nothing to say.
    fn judge_cycles(&mut self) {
        let mut edges: Vec<Vec<usize>> = vec![Vec::new(); self.graph.steps.len()];
        for (from, to) in &self.graph.edges {
            if let Some(list) = edges.get_mut(*from as usize) {
                list.push(*to as usize);
            }
        }
        let names = self.names();
        for component in cycles(&edges) {
            // Steps come in written order. The ids of a structure come after those of its body.
            let mut sorted = component;
            sorted.sort_unstable();
            let mut seen = HashSet::new();
            let lines: Vec<&'a Statement> =
                sorted.iter().filter_map(|step| self.graph.steps.get(*step)).map(|step| step.statement).filter(|line| seen.insert(line.id)).collect();
            let Some(first) = lines.first().copied() else { continue };
            let mut texts: Vec<String> = lines
                .iter()
                .filter_map(|line| {
                    let target = targets_of(line).iter().find_map(|target| match target {
                        Line::Name(_, name) => Some(*name),
                        Line::Skip(_) | Line::Next(..) => None,
                    });
                    match (target, line.label) {
                        (Some(name), _) => Some(format!("`{}`", cut_text(names.resolve(name)))),
                        (None, Some(label)) => Some(format!("`#{}`", cut_text(names.resolve(label.name)))),
                        (None, None) => None,
                    }
                })
                .collect();
            texts.sort_unstable();
            texts.dedup();
            let more = texts.len().saturating_sub(NAMES_SHOWN);
            texts.truncate(NAMES_SHOWN);
            let list = if more > 0 { format!("{} and {more} more", texts.join(", ")) } else { texts.join(", ") };
            let waits = if lines.len() == 1 { "waits for itself" } else { "wait for each other" };
            let why = "a value can be fed back only through a carry";
            let related = lines.iter().skip(1).take(NAMES_SHOWN).map(|line| self.here("part of this cycle".to_owned(), line.id)).collect();
            self.say(Code::Lc4001, format!("cycle: {list} {waits}: {why} (5.2)"), first.id, Some(first), related);
        }
    }

    /// Where the `err` output of a function or a primitive is, with how many outputs it has. `core.call` runs what only a wire says.
    fn err_position(&self, callee: Meaning, known: &mut HashMap<(u32, Name), Option<(usize, usize)>>) -> Option<(usize, usize)> {
        let names = self.names();
        match callee {
            Meaning::Function { library, name } => *known.entry((library, name)).or_insert_with(|| {
                let outputs = &self.context.declared.get(library as usize)?.functions.get(&name)?.outputs;
                Some((outputs.iter().position(|output| names.resolve(output.name) == "err")?, outputs.len()))
            }),
            Meaning::Primitive { library, name } => {
                let outputs = self.context.primitives.signature(library, name)?.outputs?;
                Some((outputs.iter().position(|output| *output == "err")?, outputs.len()))
            }
            _ => None,
        }
    }

    /// 7.5: `err` wired nowhere, which `_` writes, or a name that nothing reads. A name given to it counts for what reads that name.
    fn judge_err_outputs(&mut self) {
        let mut used = vec![false; self.scope.model.source_map.len()];
        for value in std::mem::take(&mut self.graph.wired) {
            let mut current = value;
            loop {
                match used.get_mut(current.index()) {
                    Some(slot) if !*slot => *slot = true,
                    _ => break,
                }
                match self.graph.aliases.get(&current) {
                    Some(source) => current = *source,
                    None => break,
                }
            }
        }
        let names = self.names();
        let mut known = HashMap::new();
        let lines: Vec<&'a Statement> = self.graph.steps.iter().filter(|step| !step.closing).map(|step| step.statement).collect();
        for statement in lines {
            let Some(call) = call_of(statement) else { continue };
            let Some(bound) = self.resolved.call(call.id) else { continue };
            let Some((position, count)) = self.err_position(bound.callee, &mut known) else { continue };
            let lost = match &statement.kind {
                StatementKind::Call(_) => Some(call.id),
                StatementKind::Binding { targets, .. } if targets.len() == count => match targets.get(position) {
                    Some(Line::Skip(id)) => Some(*id),
                    Some(Line::Name(id, _))
                        if !matches!(self.resolved.of(*id), Meaning::Value(_)) && !used.get(id.index()).copied().unwrap_or(true) =>
                    {
                        Some(*id)
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some(id) = lost {
                let callee = cut_text(&path_text(&call.callee, names)).into_owned();
                let message = format!("the `err` output of `{callee}` is wired nowhere: an error would be lost (7.5)");
                self.say(Code::Lc4101, message, id, Some(statement), Vec::new());
            }
        }
    }

    /// What this operand designates. A field that holds no resource designates none, whatever the struct around it holds.
    fn refs_of(&mut self, resources: &mut Resources, operand: &Operand, found: &mut Vec<Ref>) {
        match operand {
            Operand::Path(path) => {
                let Meaning::Value(value) = self.resolved.of(path.id) else { return };
                let Some(refs) = resources.values.get(&value).cloned() else { return };
                if !path.rest.is_empty() && !holds_resource(self.pass, self.context, self.resolved.ty(path.id)) {
                    return;
                }
                for reference in refs {
                    let mut part = reference.path;
                    for field in &path.rest {
                        part = resources.part(part, *field);
                    }
                    resources.cut_short |= push_ref(found, Ref { path: part, ..reference });
                }
            }
            Operand::Array(_, values) => {
                for value in values {
                    self.refs_of(resources, value, found);
                }
            }
            Operand::Bracket(id, _) => {
                if let Meaning::ArrayOfValue(value) = self.resolved.of(*id)
                    && let Some(refs) = resources.values.get(&value)
                {
                    for reference in refs {
                        resources.cut_short |= push_ref(found, *reference);
                    }
                }
            }
            Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) => {}
        }
    }

    fn value_of_target(&self, id: Id) -> Id {
        match self.resolved.of(id) {
            Meaning::Value(output) => output,
            _ => id,
        }
    }

    /// A target that holds a resource: it continues `refs`, or else a new resource starts here.
    fn give(&mut self, resources: &mut Resources, target: &Line, refs: &[Ref], parallel: u32) {
        let (id, name) = match target {
            Line::Name(id, name) => (id, name),
            Line::Next(id, _) => {
                if let Meaning::Value(carry) = self.resolved.of(*id)
                    && holds_resource(self.pass, self.context, self.resolved.ty(*id))
                {
                    let known = resources.next_refs.entry(carry).or_default();
                    for reference in refs {
                        resources.cut_short |= push_ref(known, *reference);
                    }
                }
                return;
            }
            Line::Skip(_) => return,
        };
        let ty = self.resolved.ty(*id);
        if !holds_resource(self.pass, self.context, ty) {
            return;
        }
        let value = self.value_of_target(*id);
        if refs.is_empty() {
            resources.born(value, *name, parallel);
        } else {
            resources.add_refs(value, refs);
        }
    }

    /// A node: a function that receives a resource reaches it, and what it gives of that type continues it. `core.call` runs a function. Any
    /// other primitive only moves what it holds, as `array.get` or `core.select` do.
    fn node(&mut self, resources: &mut Resources, call: &'a Call, targets: &'a [Line], index: u32, parallel: u32) {
        let names = self.names();
        let callee = self.resolved.call(call.id).map_or(Meaning::None, |bound| bound.callee);
        let runs_a_function = matches!(callee, Meaning::Primitive { library: PrimitiveLibrary::Core, name } if names.resolve(name) == "call");
        let is_function = matches!(callee, Meaning::Function { .. }) || runs_a_function;
        let is_primitive = matches!(callee, Meaning::Primitive { .. }) && !runs_a_function;
        if matches!(callee, Meaning::Primitive { library: PrimitiveLibrary::Ui, .. })
            && let Some(node) = ui_node(call, self.resolved, names)
        {
            let (list, order) =
                if node.is_set { (&mut resources.writes, &mut resources.write_order) } else { (&mut resources.reads, &mut resources.read_order) };
            let steps = list.entry(node.element).or_default();
            if steps.is_empty() {
                order.push(node.element);
            }
            steps.push(index);
        }
        let mut by_type: HashMap<TyId, Vec<Ref>> = HashMap::new();
        let mut all: Vec<Ref> = Vec::new();
        for arg in &call.args {
            let Arg::Value { value, .. } = arg else { continue };
            let mut refs = Vec::new();
            self.refs_of(resources, value, &mut refs);
            if refs.is_empty() {
                continue;
            }
            let of_type = by_type.entry(self.resolved.ty(value.id())).or_default();
            for reference in refs {
                resources.cut_short |= push_ref(of_type, reference);
                resources.cut_short |= push_ref(&mut all, reference);
                if is_function {
                    if let Some(root) = resources.roots.get_mut(reference.root as usize) {
                        let access = Access { path: reference.path, step: index };
                        if !root.accesses.iter().rev().take(MAX_REFS).any(|known| known.path == access.path && known.step == access.step) {
                            root.accesses.push(access);
                        }
                    }
                    if parallel != NONE {
                        resources.used_in_parallel.push((index, reference));
                    }
                }
            }
        }
        for target in targets {
            let (Line::Name(id, _) | Line::Next(id, _)) = target else { continue };
            let refs: &[Ref] = if is_primitive {
                &all
            } else if is_function {
                by_type.get(&self.resolved.ty(*id)).map_or(&[], Vec::as_slice)
            } else {
                &[]
            };
            self.give(resources, target, refs, parallel);
        }
    }

    fn opening(&mut self, resources: &mut Resources, statement: &'a Statement, value: &'a Rhs, around: u32) {
        let parallel = self.graph.parallel_of.get(&statement.id).copied();
        let clauses: &'a [Clause] = match value {
            Rhs::For(structure) => &structure.clauses,
            Rhs::Loop(structure) => &structure.clauses,
            Rhs::Match(branching) | Rhs::Handle(branching) => {
                let mut refs = Vec::new();
                self.refs_of(resources, &branching.selector, &mut refs);
                for arm in &branching.arms {
                    let Pattern::Variant { bindings, .. } = &arm.pattern else { continue };
                    for binding in bindings {
                        let ty = self.resolved.ty(binding.id);
                        if holds_resource(self.pass, self.context, ty) {
                            if refs.is_empty() {
                                resources.born(binding.id, binding.name, around);
                            } else {
                                resources.add_refs(binding.id, &refs);
                            }
                        }
                    }
                }
                return;
            }
            _ => return,
        };
        for clause in clauses {
            match &clause.kind {
                ClauseKind::In(clause) => {
                    let ty = self.resolved.ty(clause.id);
                    if !holds_resource(self.pass, self.context, ty) {
                        continue;
                    }
                    // In a `for parallel`, each iteration has its element, which is no resource from outside, and still what the array holds (8.1).
                    let mut refs = Vec::new();
                    self.refs_of(resources, &clause.source, &mut refs);
                    if let Some(place) = parallel {
                        for reference in &mut refs {
                            // Still the element of the loop around this one, or else an element that left its loop.
                            if self.graph.common_parallel(reference.own, place) != reference.own || reference.own == NONE {
                                reference.own = place;
                            }
                        }
                    }
                    if refs.is_empty() {
                        resources.born(clause.id, clause.name, parallel.unwrap_or(around));
                    } else {
                        resources.add_refs(clause.id, &refs);
                    }
                }
                ClauseKind::Carry(carry) => {
                    let ty = self.resolved.ty(carry.id);
                    if !holds_resource(self.pass, self.context, ty) {
                        continue;
                    }
                    let mut refs = Vec::new();
                    self.refs_of(resources, &carry.value, &mut refs);
                    if refs.is_empty() {
                        resources.born(carry.id, carry.name, around);
                    } else {
                        resources.add_refs(carry.id, &refs);
                    }
                }
            }
        }
    }

    /// What a structure gives: what its arms yield, its carries, then what it collects (6.2, 6.1).
    fn closing(&mut self, resources: &mut Resources, targets: &'a [Line], value: &'a Rhs, parallel: u32) {
        match value {
            Rhs::Match(branching) | Rhs::Handle(branching) => {
                let mut refs: Vec<Vec<Ref>> = vec![Vec::new(); targets.len()];
                for arm in &branching.arms {
                    for (value, found) in arm.yields.values.iter().zip(&mut refs) {
                        self.refs_of(resources, value, found);
                    }
                }
                for (target, found) in targets.iter().zip(&refs) {
                    self.give(resources, target, found, parallel);
                }
            }
            Rhs::For(structure) => self.closing_loop(resources, targets, &structure.clauses, &structure.body, parallel),
            Rhs::Loop(structure) => self.closing_loop(resources, targets, &structure.clauses, &structure.body, parallel),
            _ => {}
        }
    }

    fn closing_loop(&mut self, resources: &mut Resources, targets: &'a [Line], clauses: &'a [Clause], body: &'a Body, parallel: u32) {
        let carries = clauses.iter().filter_map(|clause| match &clause.kind {
            ClauseKind::Carry(carry) => Some(carry),
            ClauseKind::In(_) => None,
        });
        let mut cut_short = false;
        let carry_refs: Vec<Vec<Ref>> = carries
            .map(|carry| {
                let mut refs = resources.values.get(&carry.id).cloned().unwrap_or_default();
                for reference in resources.next_refs.get(&carry.id).into_iter().flatten() {
                    cut_short |= push_ref(&mut refs, *reference);
                }
                refs
            })
            .collect();
        resources.cut_short |= cut_short;
        let collects = body.statements.iter().filter_map(|line| match &line.kind {
            StatementKind::Collect(value) => Some(value),
            _ => None,
        });
        let mut outputs = carry_refs;
        for value in collects {
            let mut refs = Vec::new();
            self.refs_of(resources, value, &mut refs);
            outputs.push(refs);
        }
        if outputs.len() != targets.len() {
            return;
        }
        for (target, refs) in targets.iter().zip(&outputs) {
            self.give(resources, target, refs, parallel);
        }
    }

    /// Who reaches what, walked in dependency order. What is in a cycle stays out.
    fn follow_resources(&mut self, signature: &'a Signature, order: &Order) -> Resources {
        let mut resources = Resources::default();
        for param in &signature.params {
            if holds_resource(self.pass, self.context, self.resolved.ty(param.id)) {
                resources.born(param.id, param.name, NONE);
            }
        }
        for index in &order.steps {
            let Some(step) = self.graph.steps.get(*index as usize).copied() else { continue };
            match &step.statement.kind {
                StatementKind::Call(call) if !step.closing => self.node(&mut resources, call, &[], *index, step.parallel),
                StatementKind::Binding { targets, value } => match value {
                    Rhs::Call(call) => self.node(&mut resources, call, targets, *index, step.parallel),
                    Rhs::Struct(_) => {
                        for target in targets {
                            self.give(&mut resources, target, &[], step.parallel);
                        }
                    }
                    Rhs::Operand(operand) => {
                        let mut refs = Vec::new();
                        self.refs_of(&mut resources, operand, &mut refs);
                        for target in targets {
                            self.give(&mut resources, target, &refs, step.parallel);
                        }
                    }
                    Rhs::Match(_) | Rhs::Handle(_) | Rhs::For(_) | Rhs::Loop(_) => {
                        if step.closing {
                            self.closing(&mut resources, targets, value, step.parallel);
                        } else {
                            self.opening(&mut resources, step.statement, value, step.parallel);
                        }
                    }
                },
                StatementKind::Call(_) | StatementKind::Collect(_) | StatementKind::Until(_) => {}
            }
        }
        self.pass.cut_short |= resources.cut_short;
        resources
    }

    /// Whether `to` waits for `from`, looking only at what comes before it, so that the searches of one resource cover windows that do
    /// not overlap (`docs/architecture.md`, section 5). When the work allowed runs out, it answers yes, and nothing more is said.
    fn reaches(order: &Order, from: u32, to: u32, seen: &mut [u32], epoch: &mut u32, work: &mut u64) -> bool {
        let limit = order.ranks.get(to as usize).copied().unwrap_or(NONE);
        *epoch = epoch.wrapping_add(1);
        let mut waiting = vec![from];
        while let Some(step) = waiting.pop() {
            if step == to {
                return true;
            }
            for follower in order.followers(step) {
                if *work == 0 {
                    return true;
                }
                *work -= 1;
                if order.ranks.get(*follower as usize).copied().unwrap_or(NONE) > limit {
                    continue;
                }
                if let Some(slot) = seen.get_mut(*follower as usize)
                    && *slot != *epoch
                {
                    *slot = *epoch;
                    waiting.push(*follower);
                }
            }
        }
        false
    }

    /// The pairs of nodes with no order between them, in the order of `accesses`, at most one for each part that a warning would name. Each
    /// node is compared with what stands before it, the latest first, and takes the place of what it reaches when that is the same part and
    /// every arm that holds the node also holds the other: an arm of a structure or a field of a struct can be exclusive with this node or apart
    /// from it, and not with what it reached (8.1). So what does not take a place stays, and what it reached saves it a new search.
    fn unordered(&mut self, resources: &Resources, order: &Order, accesses: &[Access], seen: &mut [u32], epoch: &mut u32) -> Vec<(u32, u32, u32)> {
        let has_parts = accesses.iter().any(|access| access.path != 0);
        let mut found = Vec::new();
        let mut warned: HashSet<u32> = HashSet::new();
        let mut frontier: Vec<Standing> = Vec::new();
        let mut reaching: HashSet<u32> = HashSet::new();
        for access in accesses {
            reaching.clear();
            let mut kept = Vec::with_capacity(frontier.len() + 1);
            for standing in frontier.into_iter().rev() {
                let known = standing.access;
                let walks =
                    self.graph.arm_depth(known.step) + self.graph.arm_depth(access.step) + resources.depth(known.path) + resources.depth(access.path);
                let cost = u64::from(walks) + 1;
                if self.work < cost {
                    self.work = 0;
                    self.pass.cut_short = true;
                    return found;
                }
                self.work -= cost;
                if known.step == access.step || !resources.overlap(known.path, access.path) || self.graph.exclusive(known.step, access.step) {
                    kept.push(standing);
                    continue;
                }
                let reached = reaching.contains(&standing.reached) || Self::reaches(order, known.step, access.step, seen, epoch, &mut self.work);
                // Out of work, `reaches` says yes, which hides a warning.
                self.pass.cut_short |= self.work == 0;
                if !reached {
                    let shown = if resources.depth(known.path) >= resources.depth(access.path) { known.path } else { access.path };
                    if warned.insert(shown) {
                        found.push((shown, known.step, access.step));
                    }
                    // Only one thing can be said of a resource without parts.
                    if !has_parts {
                        return found;
                    }
                    kept.push(standing);
                    continue;
                }
                reaching.insert(known.step);
                if !(access.path == known.path && self.graph.arms_cover(access.step, known.step)) {
                    kept.push(Standing { access: known, reached: access.step });
                }
            }
            kept.reverse();
            kept.push(Standing { access: *access, reached: NONE });
            frontier = kept;
        }
        found
    }

    /// 8.1: `LC4102`, once per resource, and `LC4103`, once per element.
    fn judge_order(&mut self, resources: &Resources, order: &Order) {
        let mut seen = vec![0_u32; self.graph.steps.len()];
        let mut epoch = 0;
        let names = self.names();
        for (index, root) in resources.roots.iter().enumerate() {
            let root_index = u32::try_from(index).unwrap_or(NONE);
            for (path, first, second) in self.unordered(resources, order, &root.accesses, &mut seen, &mut epoch) {
                let text = resources.text_of(Ref { root: root_index, path, own: NONE }, names);
                self.warn_order(Code::Lc4102, format!("concurrent access to `{text}`: order by the error wire or `after:` (8.1)"), first, second);
            }
        }
        for element in &resources.write_order {
            let Some(writers) = resources.writes.get(element) else { continue };
            let mut sorted = writers.clone();
            sorted.sort_by_key(|step| order.ranks.get(*step as usize).copied().unwrap_or(NONE));
            let accesses: Vec<Access> = sorted.into_iter().map(|step| Access { path: 0, step }).collect();
            if let Some((_, first, second)) = self.unordered(resources, order, &accesses, &mut seen, &mut epoch).first().copied() {
                let text = self.element_text(*element);
                let why = "order them by a wire or `after:`";
                self.warn_order(
                    Code::Lc4103,
                    format!("the panel element `{text}` is written by two nodes with no order between them: {why} (8.1)"),
                    first,
                    second,
                );
            }
        }
    }

    fn element_text(&self, element: Id) -> String {
        let names = self.names();
        cut_text(self.items.get(&element).map_or("", |item| names.resolve(item.name))).into_owned()
    }

    fn warn_order(&mut self, code: Code, message: String, first: u32, second: u32) {
        let (Some(first), Some(second)) = (self.graph.steps.get(first as usize).copied(), self.graph.steps.get(second as usize).copied()) else {
            return;
        };
        let at_second = call_of(second.statement).map_or(second.statement.id, |call| call.id);
        let at_first = call_of(first.statement).map_or(first.statement.id, |call| call.id);
        let related = vec![self.here("the other node is here".to_owned(), at_first)];
        self.say(code, message, at_second, Some(second.statement), related);
    }

    /// 8.1: `LC4104`, once per loop, on what comes first in it. What was born in its body is its own.
    fn judge_parallel(&mut self, resources: &Resources) {
        let names = self.names();
        let mut uses: Vec<(u32, Result<Ref, Id>)> = resources.used_in_parallel.iter().map(|(step, reference)| (*step, Ok(*reference))).collect();
        for (element, writers) in &resources.writes {
            for writer in writers {
                let inside = self.graph.steps.get(*writer as usize).is_some_and(|step| step.parallel != NONE);
                if inside {
                    uses.push((*writer, Err(*element)));
                }
            }
        }
        uses.sort_by_key(|(step, _)| *step);
        // A million calls in one body ask one question. Of what a loop uses, the first in the order of the texts: the lines have no order.
        let mut asked: HashSet<(u32, Result<Ref, Id>)> = HashSet::new();
        let mut chosen: HashMap<u32, (String, u32)> = HashMap::new();
        for (step, what) in uses {
            let Some(line) = self.graph.steps.get(step as usize).copied() else { continue };
            if !asked.insert((line.parallel, what)) {
                continue;
            }
            let cost = u64::from(self.graph.parallels.get(line.parallel as usize).map_or(0, |place| place.depth)) + 1;
            if self.work < cost {
                self.pass.cut_short = true;
                break;
            }
            self.work -= cost;
            let (born, own) = match what {
                Ok(reference) => (resources.roots.get(reference.root as usize).map_or(NONE, |root| root.parallel), reference.own),
                Err(_) => (NONE, NONE),
            };
            // What holds the node and the place where the resource was born has it as its own, and so does what is around that.
            let stop = self.graph.common_parallel(line.parallel, born);
            let mut exempt = own != NONE && self.graph.common_parallel(line.parallel, own) == own;
            let why = "its iterations have no order (8.1)";
            let message = match what {
                Ok(reference) => format!("this `for parallel` uses `{}`, a resource from outside: {why}", resources.text_of(reference, names)),
                Err(element) => format!("this `for parallel` writes the panel element `{}`: {why}", self.element_text(element)),
            };
            let mut place = line.parallel;
            while place != stop {
                let Some(body) = self.graph.parallels.get(place as usize).copied() else { break };
                if exempt {
                    exempt = place != own;
                } else {
                    let best = chosen.entry(place).or_insert_with(|| (message.clone(), step));
                    if message < best.0 {
                        *best = (message.clone(), step);
                    }
                }
                place = body.parent;
            }
        }
        let mut chosen: Vec<(u32, (String, u32))> = chosen.into_iter().collect();
        chosen.sort_by_key(|(place, _)| *place);
        for (place, (message, step)) in chosen {
            let (Some(body), Some(line)) = (self.graph.parallels.get(place as usize).copied(), self.graph.steps.get(step as usize).copied()) else {
                continue;
            };
            let related = vec![self.here("the `for parallel` is here".to_owned(), body.statement.id)];
            let id = call_of(line.statement).map_or(line.statement.id, |call| call.id);
            self.say(Code::Lc4104, message, id, Some(line.statement), related);
        }
    }

    /// 10: `LC4105`, once per element. Two reads in different arms never run together.
    fn judge_buttons(&mut self, resources: &Resources) {
        let names = self.names();
        for element in &resources.read_order {
            let Some(item) = self.items.get(element) else { continue };
            if names.resolve(item.kind) != "button" {
                continue;
            }
            let Some(reads) = resources.reads.get(element) else { continue };
            let mut sorted = reads.clone();
            sorted.sort_unstable();
            let mut pair = None;
            'reads: for (position, second) in sorted.iter().enumerate() {
                for first in sorted.iter().take(position) {
                    let cost = u64::from(self.graph.arm_depth(*first) + self.graph.arm_depth(*second)) + 1;
                    if self.work < cost {
                        self.pass.cut_short = true;
                        return;
                    }
                    self.work -= cost;
                    if !self.graph.exclusive(*first, *second) {
                        pair = Some((*first, *second));
                        break 'reads;
                    }
                }
            }
            if let Some((first, second)) = pair {
                self.warn_order(Code::Lc4105, "only one of these reads sees a press: use a switch (10)".to_owned(), first, second);
            }
        }
    }
}
