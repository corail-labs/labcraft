//! The values that the structures of a function pass in (spec 6, `docs/ir.md`, section 4): a value read inside a structure
//! and defined outside it enters its frame, and every region of it, at every level between the two. `LC4007` bounds their
//! number, since lowering builds each one and a file of a few hundred KiB could ask for billions.
//!
//! What is counted is what `labcraft-ir` builds, found the same way: a new name (`e = d`), a field read (`k = step.low`) and the
//! binding of an output or a carry are followed to the value they name, so that `step` is passed in, not `k`; a literal and a
//! named constant are no value passed in, and neither is an `after:`, which only orders. The harness compares this count with
//! `ir::Region::passed_in` on every function of the repository, and tests of `labcraft-ir` on programs counted by hand, one
//! rule each.

use std::collections::{HashMap, HashSet};

use labcraft_graph::{Arg, Body, Branching, ClauseKind, Function, Id, Operand, Rhs, Statement, StatementKind, Target};

use crate::resolved::{Meaning, Resolved};

/// The values passed in that a function may have, counted once on the frame of each structure they enter and once in each of
/// its regions *(proposed, scenario 4)*. At about 135 bytes each when lowered, the most stays well within the bounds of a run.
pub const MAX_PASSED_IN: u64 = 1_000_000;

/// What [`passed_in`] found.
#[derive(Clone, Copy, Debug)]
pub struct PassedIn<'f> {
    /// The values passed in, each counted on its frame and in each region; at most a few more than the limit, where it stops.
    pub count: u64,
    /// When the count went over the limit: the structure that weighs the most in what was counted, and its weight. The first
    /// in the file of those that weigh the same.
    pub over: Option<(&'f Statement, u64)>,
}

#[derive(Clone, Copy)]
enum Holder<'f> {
    Function,
    Loop { statement: &'f Statement, clauses: &'f [labcraft_graph::Clause] },
    Arm { statement: &'f Statement, arm: &'f labcraft_graph::Arm, arms: u64 },
}

struct Region<'f> {
    body: &'f Body,
    parent: u32,
    holder: Holder<'f>,
}

impl<'f> Holder<'f> {
    /// The structure that passes values into this region, and how many regions it has.
    fn structure(&self) -> Option<(&'f Statement, u64)> {
        match *self {
            Holder::Function => None,
            Holder::Loop { statement, .. } => Some((statement, 1)),
            Holder::Arm { statement, arms, .. } => Some((statement, arms)),
        }
    }
}

struct Count<'f, 'r> {
    resolved: &'r Resolved,
    regions: Vec<Region<'f>>,
    /// By the id of what defines a value: the region it is defined in.
    defined_in: HashMap<Id, u32>,
    /// By the id of an output: the target that binds it, which a read of the output follows.
    binders: HashMap<Id, Id>,
    /// By the id of a carry: its `next` target. A read of the carry reads the argument of its region, not this.
    nexts: HashMap<Id, Id>,
    /// The lines of one target and an operand, by that target.
    lines: HashMap<Id, &'f Operand>,
    /// Lines that only name a value, by their target: the value they name.
    aliases: HashMap<Id, Id>,
    /// Lines that hold a literal, by their target, and lines `r = [x, y]`, which are nodes.
    literals: HashSet<Id>,
    built_arrays: HashSet<Id>,
    /// What each value read stands for, once found: `None` for a literal or what nothing defines.
    bases: HashMap<Id, Option<Id>>,
    seen: HashSet<(Id, Id)>,
    count: u64,
    limit: u64,
    /// By the id of a structure: its weight so far, and its statement.
    weights: HashMap<Id, (u64, &'f Statement)>,
    /// By the id of a structure: its rank in the order of the file. An id is given at the end of its line, after its body.
    ranks: HashMap<Id, u32>,
    over: Option<(&'f Statement, u64)>,
}

/// Counts the values that the structures of `function` pass in, and stops a little after `limit`.
pub fn passed_in<'f>(function: &'f Function, resolved: &Resolved, limit: u64) -> PassedIn<'f> {
    let mut count = Count {
        resolved,
        regions: vec![Region { body: &function.body, parent: u32::MAX, holder: Holder::Function }],
        defined_in: HashMap::new(),
        binders: HashMap::new(),
        nexts: HashMap::new(),
        lines: HashMap::new(),
        aliases: HashMap::new(),
        literals: HashSet::new(),
        built_arrays: HashSet::new(),
        bases: HashMap::new(),
        seen: HashSet::new(),
        count: 0,
        limit,
        weights: HashMap::new(),
        ranks: HashMap::new(),
        over: None,
    };
    for param in &function.signature.params {
        count.defined_in.insert(param.id, 0);
    }
    let outputs: HashSet<Id> = function.signature.outputs.iter().map(|output| output.id).collect();
    count.find_regions(&outputs);
    count.sort_lines();
    count.read_all(function);
    PassedIn { count: count.count, over: count.over }
}

fn target_id(target: &Target) -> Id {
    match target {
        Target::Name(id, _) | Target::Skip(id) | Target::Next(id, _) => *id,
    }
}

impl<'f> Count<'f, '_> {
    /// Every region and where each value is defined, with an explicit stack: bodies nest as deep as `check` lets them. Each
    /// line, then the regions of its structure before the next line, so that structures are ranked in the order of the file.
    fn find_regions(&mut self, outputs: &HashSet<Id>) {
        let mut stack = vec![(0_u32, 0_usize)];
        while let Some((index, at)) = stack.pop() {
            let Some(body) = self.regions.get(index as usize).map(|region| region.body) else { continue };
            let Some(statement) = body.statements.get(at) else { continue };
            stack.push((index, at.saturating_add(1)));
            {
                let StatementKind::Binding { targets, value } = &statement.kind else { continue };
                for target in targets {
                    let id = target_id(target);
                    self.defined_in.insert(id, index);
                    match target {
                        Target::Name(id, _) => {
                            if let Meaning::Value(output) = self.resolved.of(*id)
                                && outputs.contains(&output)
                            {
                                self.binders.insert(output, *id);
                            }
                        }
                        Target::Next(id, _) => {
                            if let Meaning::Value(carry) = self.resolved.of(*id) {
                                self.nexts.insert(carry, *id);
                            }
                        }
                        Target::Skip(_) => {}
                    }
                }
                let first = u32::try_from(self.regions.len()).unwrap_or(u32::MAX);
                match value {
                    Rhs::Operand(operand) => {
                        if let [target] = &**targets {
                            self.lines.insert(target_id(target), operand);
                        }
                    }
                    Rhs::Loop(structure) => {
                        self.regions.push(Region {
                            body: &structure.body,
                            parent: index,
                            holder: Holder::Loop { statement, clauses: &structure.clauses },
                        });
                        self.define_clauses(&structure.clauses, first);
                    }
                    Rhs::For(structure) => {
                        self.regions.push(Region {
                            body: &structure.body,
                            parent: index,
                            holder: Holder::Loop { statement, clauses: &structure.clauses },
                        });
                        self.define_clauses(&structure.clauses, first);
                    }
                    Rhs::Match(branching) | Rhs::Handle(branching) => self.arms(statement, branching, index),
                    Rhs::Call(_) | Rhs::Struct(_) => {}
                }
                let last = u32::try_from(self.regions.len()).unwrap_or(u32::MAX);
                if last > first {
                    let rank = u32::try_from(self.ranks.len()).unwrap_or(u32::MAX);
                    self.ranks.insert(statement.id, rank);
                }
                stack.extend((first..last).rev().map(|region| (region, 0)));
            }
        }
    }

    fn define_clauses(&mut self, clauses: &[labcraft_graph::Clause], region: u32) {
        for clause in clauses {
            let id = match &clause.kind {
                ClauseKind::In(clause) => clause.id,
                ClauseKind::Carry(carry) => carry.id,
            };
            self.defined_in.insert(id, region);
        }
    }

    fn arms(&mut self, statement: &'f Statement, branching: &'f Branching, parent: u32) {
        let arms = u64::try_from(branching.arms.len()).unwrap_or(u64::MAX);
        for arm in &branching.arms {
            let index = u32::try_from(self.regions.len()).unwrap_or(u32::MAX);
            for pattern in arm.patterns() {
                for binding in pattern.bindings() {
                    self.defined_in.insert(binding.id, index);
                }
            }
            self.regions.push(Region { body: &arm.body, parent, holder: Holder::Arm { statement, arm, arms } });
        }
    }

    /// Which lines of one operand only name a value, hold a literal, or build an array of wires (spec 5.1, R-4).
    fn sort_lines(&mut self) {
        let mut constant = HashMap::new();
        let lines: Vec<(Id, &'f Operand)> = self.lines.iter().map(|(id, operand)| (*id, *operand)).collect();
        for (target, operand) in lines {
            match operand {
                Operand::Path(path) => {
                    if let Meaning::Value(value) = self.resolved.of(path.id) {
                        self.aliases.insert(target, value);
                    }
                }
                Operand::Array(_, values) if holds_wire(values, &self.lines, self.resolved, &mut constant) => {
                    self.built_arrays.insert(target);
                }
                Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) | Operand::Array(..) => {
                    self.literals.insert(target);
                }
                Operand::Bracket(..) => {}
            }
        }
    }

    /// What a value read by this id stands for: new names, field reads and bindings followed. Each chain is walked once.
    fn base(&mut self, value: Id) -> Option<Id> {
        let mut chain = Vec::new();
        let mut current = value;
        let limit = self.aliases.len() + self.binders.len() + 1;
        let found = loop {
            if let Some(known) = self.bases.get(&current) {
                break *known;
            }
            // New names that name each other have their `LC4001`.
            if chain.len() > limit {
                break None;
            }
            chain.push(current);
            if let Some(next) = self.binders.get(&current).or_else(|| self.aliases.get(&current)) {
                current = *next;
                continue;
            }
            break (!self.literals.contains(&current)).then_some(current);
        };
        for id in chain {
            self.bases.insert(id, found);
        }
        found
    }

    /// A value read in `region`: each structure between it and where the value is defined passes it in.
    fn read_in(&mut self, region: u32, value: Id) {
        let Some(base) = self.base(value) else { return };
        let Some(defined) = self.defined_in.get(&base).copied() else { return };
        let mut current = region;
        while current != defined && self.over.is_none() {
            let Some(info) = self.regions.get(current as usize) else { return };
            let Some((statement, regions)) = info.holder.structure() else { return };
            let parent = info.parent;
            if !self.seen.insert((statement.id, base)) {
                return;
            }
            let weight = regions.saturating_add(1);
            self.count = self.count.saturating_add(weight);
            let entry = self.weights.entry(statement.id).or_insert((0, statement));
            entry.0 = entry.0.saturating_add(weight);
            if self.count > self.limit {
                // Of those that weigh the same, the first in the file.
                let ranks = &self.ranks;
                self.over = self
                    .weights
                    .iter()
                    .max_by_key(|(id, (weight, _))| (*weight, std::cmp::Reverse(ranks.get(*id).copied().unwrap_or(u32::MAX))))
                    .map(|(_, (weight, statement))| (*statement, *weight));
            }
            current = parent;
        }
    }

    fn read(&mut self, region: u32, operand: &Operand) {
        let mut pending = vec![operand];
        while let Some(operand) = pending.pop() {
            match operand {
                Operand::Path(path) => {
                    if let Meaning::Value(value) = self.resolved.of(path.id) {
                        self.read_in(region, value);
                    }
                }
                Operand::Bracket(id, _) => {
                    if let Meaning::ArrayOfValue(value) = self.resolved.of(*id) {
                        self.read_in(region, value);
                    }
                }
                Operand::Array(_, values) => pending.extend(values.iter().rev()),
                Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) => {}
            }
        }
    }

    fn read_all(&mut self, function: &'f Function) {
        for index in 0..self.regions.len() {
            if self.over.is_some() {
                return;
            }
            let region = u32::try_from(index).unwrap_or(u32::MAX);
            let Some(info) = self.regions.get(index) else { continue };
            let (body, holder) = (info.body, info.holder);
            for statement in &body.statements {
                self.read_statement(region, statement);
            }
            // What leaves the region.
            match holder {
                Holder::Function => {
                    for output in &function.signature.outputs {
                        if let Some(target) = self.binders.get(&output.id).copied() {
                            self.read_in(region, target);
                        }
                    }
                }
                Holder::Loop { clauses, .. } => {
                    for clause in clauses {
                        if let ClauseKind::Carry(carry) = &clause.kind
                            && let Some(next) = self.nexts.get(&carry.id).copied()
                        {
                            self.read_in(region, next);
                        }
                    }
                }
                Holder::Arm { arm, .. } => {
                    for value in &arm.yields.values {
                        self.read(region, value);
                    }
                }
            }
        }
    }

    fn read_statement(&mut self, region: u32, statement: &'f Statement) {
        let args = |args: &'f [Arg]| {
            args.iter().filter_map(|arg| match arg {
                Arg::Value { value, .. } => Some(value),
                // An order is no wire.
                Arg::After { .. } => None,
            })
        };
        match &statement.kind {
            StatementKind::Call(call) => args(&call.args).for_each(|value| self.read(region, value)),
            StatementKind::Collect(value) | StatementKind::Until(value) => self.read(region, value),
            StatementKind::Binding { targets, value } => match value {
                Rhs::Call(call) => args(&call.args).for_each(|value| self.read(region, value)),
                Rhs::Struct(literal) => literal.fields.iter().for_each(|field| self.read(region, &field.value)),
                Rhs::Operand(operand) => {
                    if targets.first().is_some_and(|target| self.built_arrays.contains(&target_id(target))) {
                        self.read(region, operand);
                    }
                }
                Rhs::Match(branching) | Rhs::Handle(branching) => self.read(region, &branching.selector),
                Rhs::Loop(structure) => self.read_initial(region, &structure.clauses),
                Rhs::For(structure) => self.read_initial(region, &structure.clauses),
            },
        }
    }

    /// What the clauses of a loop read before it starts: arrays, bounds and the first value of each carry.
    fn read_initial(&mut self, region: u32, clauses: &'f [labcraft_graph::Clause]) {
        for clause in clauses {
            match &clause.kind {
                ClauseKind::In(clause) => {
                    self.read(region, &clause.source);
                    if let Some(end) = &clause.end {
                        self.read(region, end);
                    }
                }
                ClauseKind::Carry(carry) => self.read(region, &carry.value),
            }
        }
    }
}

/// What an operand on a line asks of the values it names to be a constant: the values that must be constants, or `false`
/// when it holds a wire of its own (a field read is a wire).
fn needs(operand: &Operand, resolved: &Resolved, deps: &mut Vec<Id>) -> bool {
    let mut pending = vec![operand];
    while let Some(operand) = pending.pop() {
        match operand {
            Operand::Path(path) => {
                if let Meaning::Value(value) = resolved.of(path.id) {
                    if !path.rest.is_empty() {
                        return false;
                    }
                    deps.push(value);
                }
            }
            Operand::Bracket(id, _) => {
                if let Meaning::ArrayOfValue(value) = resolved.of(*id) {
                    deps.push(value);
                }
            }
            Operand::Array(_, values) => pending.extend(values),
            Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) => {}
        }
    }
    true
}

/// Whether an array written on a line holds a wire, at any depth (spec 5.1): a name of a constant is no wire.
fn holds_wire(values: &[Operand], lines: &HashMap<Id, &Operand>, resolved: &Resolved, known: &mut HashMap<Id, bool>) -> bool {
    let mut deps = Vec::new();
    let all = values.iter().all(|value| needs(value, resolved, &mut deps));
    !all || deps.into_iter().any(|value| !is_constant(value, lines, resolved, known))
}

/// Whether a value is a named constant, through new names and arrays of constants: the line `k2 = k` names the constant
/// `k`. A walk with an explicit stack, since a file that `check` refuses can chain a hundred thousand of them; a value met
/// again on the way, an array that names itself, is a wire.
fn is_constant(value: Id, lines: &HashMap<Id, &Operand>, resolved: &Resolved, known: &mut HashMap<Id, bool>) -> bool {
    // The values that a value needs, once it is entered.
    let mut needed: HashMap<Id, Vec<Id>> = HashMap::new();
    let mut stack = vec![(value, false)];
    while let Some((current, leaving)) = stack.pop() {
        if known.contains_key(&current) {
            continue;
        }
        if leaving {
            let deps = needed.remove(&current).unwrap_or_default();
            let answer = deps.iter().all(|dep| known.get(dep).copied().unwrap_or(false));
            known.insert(current, answer);
            continue;
        }
        if needed.contains_key(&current) {
            // Met again before its answer: a value that names itself.
            continue;
        }
        // A line that binds an output or a carry is a terminal, never a constant.
        let operand = match lines.get(&current) {
            Some(operand) if !matches!(resolved.of(current), Meaning::Value(_)) => *operand,
            _ => {
                known.insert(current, false);
                continue;
            }
        };
        let mut deps = Vec::new();
        let constant = match operand {
            Operand::Path(path) if path.rest.is_empty() => needs(operand, resolved, &mut deps) && !deps.is_empty(),
            Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) => true,
            Operand::Array(..) => needs(operand, resolved, &mut deps),
            Operand::Path(_) | Operand::Bracket(..) => false,
        };
        if !constant {
            known.insert(current, false);
            continue;
        }
        stack.push((current, true));
        stack.extend(deps.iter().filter(|dep| !known.contains_key(dep)).map(|dep| (*dep, false)));
        needed.insert(current, deps);
    }
    known.get(&value).copied().unwrap_or(false)
}
