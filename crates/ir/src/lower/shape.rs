//! The shape of a function, found in one walk before anything is lowered: its bodies and what holds each, where each value is
//! defined, the lines that make no node (a new name, a field read, a literal bound to an output), the values that each
//! structure passes in (the context of RVSDG), and what each node waits for. Bodies are walked with an explicit stack.

use std::collections::{HashMap, HashSet};

use labcraft_types::Name;

use labcraft_check::{CheckedFile, Meaning};
use labcraft_graph::{Arg, Arm, Body, ClauseKind, For, Function, Id, Loop, Operand, OrderRef, Path, Rhs, Statement, StatementKind, Target};

/// What holds a body.
#[derive(Clone, Copy)]
pub(super) enum Holder<'f> {
    Function,
    Loop(&'f Statement, &'f Loop),
    For(&'f Statement, &'f For),
    Arm { statement: &'f Statement, arm: &'f Arm },
}

impl Holder<'_> {
    /// The statement of the structure, whose node passes values in.
    pub(super) fn statement(&self) -> Option<&Statement> {
        match self {
            Holder::Function => None,
            Holder::Loop(statement, _) | Holder::For(statement, _) | Holder::Arm { statement, .. } => Some(statement),
        }
    }
}

pub(super) struct BodyInfo<'f> {
    pub(super) body: &'f Body,
    pub(super) parent: Option<u32>,
    pub(super) holder: Holder<'f>,
}

/// What a value read stands for, once new names and field reads are followed.
#[derive(Clone, Debug)]
pub(super) enum Base<'f> {
    /// A value defined by an argument of a region or an output of a node, then fields read from it, by name.
    Def { id: Id, fields: Vec<Name> },
    /// A literal: a named constant, whose node is `node`, or a literal bound to an output or a carry, which has none.
    Literal { operand: &'f Operand, node: Option<Id> },
}

/// What a node waits for, in the region where the order goes (`docs/ir.md`, section 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Wait {
    /// A value of the region, by the id of what defines it.
    Value(Id),
    /// A node of the region, by its statement.
    Node(Id),
}

pub(super) struct Shape<'f> {
    pub(super) bodies: Vec<BodyInfo<'f>>,
    /// Inner bodies before the bodies that hold them.
    pub(super) post_order: Vec<u32>,
    /// The bodies of each structure, by its statement: one for a loop, one per arm in written order.
    pub(super) inner: HashMap<Id, Vec<u32>>,
    pub(super) body_of_statement: HashMap<Id, u32>,
    /// By the id of what defines a value: the body it is defined in.
    pub(super) defined_in: HashMap<Id, u32>,
    /// By the id of a target: its statement.
    pub(super) statement_of_target: HashMap<Id, Id>,
    /// By the id of an output: the target that binds it.
    pub(super) binders: HashMap<Id, Id>,
    /// By the id of a carry: the `next` target that binds it.
    pub(super) nexts: HashMap<Id, Id>,
    /// The lines that make no node, by the id of their target: a new name or a field read, and a literal bound to an output or
    /// a carry. A named constant is in `literals` too, and has its node.
    pub(super) aliases: HashMap<Id, &'f Path>,
    pub(super) literals: HashMap<Id, &'f Operand>,
    pub(super) constant_nodes: HashSet<Id>,
    /// By the target of a line `r = [x, y]`: it builds the array on its node.
    pub(super) built_arrays: HashSet<Id>,
    /// What each value read stands for, by the id it is read by.
    bases: HashMap<Id, Base<'f>>,
    /// By the statement of a structure: the values its regions read from outside, in the order first read.
    pub(super) context: HashMap<Id, Vec<Id>>,
    /// By the statement of a node: what it waits for, in its own region.
    pub(super) orders: HashMap<Id, Vec<Wait>>,
    /// By the id of what defines a value: the new names lines give it.
    pub(super) more_names: HashMap<Id, Vec<Name>>,
    pub(super) label_statement: HashMap<Id, Id>,
    /// By the id of a target that names a value, or of what defines an argument of a region: that name.
    pub(super) target_names: HashMap<Id, Name>,
    /// What `orders` holds, to find a repeat at once: an `after:` can name a million values.
    ordered: HashSet<(Id, Wait)>,
}

/// What a line binds.
fn targets_of(statement: &Statement) -> &[Target] {
    match &statement.kind {
        StatementKind::Binding { targets, .. } => targets,
        _ => &[],
    }
}

fn target_id(target: &Target) -> Id {
    match target {
        Target::Name(id, _) | Target::Skip(id) | Target::Next(id, _) => *id,
    }
}

impl<'f> Shape<'f> {
    pub(super) fn of(function: &'f Function, file: &'f CheckedFile) -> Result<Self, String> {
        let mut shape = Shape {
            bodies: vec![BodyInfo { body: &function.body, parent: None, holder: Holder::Function }],
            post_order: Vec::new(),
            inner: HashMap::new(),
            body_of_statement: HashMap::new(),
            defined_in: HashMap::new(),
            statement_of_target: HashMap::new(),
            binders: HashMap::new(),
            nexts: HashMap::new(),
            aliases: HashMap::new(),
            literals: HashMap::new(),
            constant_nodes: HashSet::new(),
            built_arrays: HashSet::new(),
            bases: HashMap::new(),
            context: HashMap::new(),
            orders: HashMap::new(),
            more_names: HashMap::new(),
            label_statement: HashMap::new(),
            target_names: HashMap::new(),
            ordered: HashSet::new(),
        };
        let outputs: HashSet<Id> = function.signature.outputs.iter().map(|output| output.id).collect();
        for param in &function.signature.params {
            shape.defined_in.insert(param.id, 0);
            shape.target_names.insert(param.id, param.name);
        }
        // The lines of one target and an operand, which may make no node.
        let mut operand_lines: HashMap<Id, &'f Operand> = HashMap::new();
        // Pre-order, with an explicit stack; the post-order is the reverse of a pre-order that pushes children last.
        let mut stack = vec![0_u32];
        let mut pre_order = Vec::new();
        while let Some(index) = stack.pop() {
            pre_order.push(index);
            let Some(info) = shape.bodies.get(index as usize) else { continue };
            let body = info.body;
            for statement in &body.statements {
                shape.body_of_statement.insert(statement.id, index);
                if let Some(label) = &statement.label {
                    shape.label_statement.insert(label.id, statement.id);
                }
                for target in targets_of(statement) {
                    let id = target_id(target);
                    shape.defined_in.insert(id, index);
                    shape.statement_of_target.insert(id, statement.id);
                    match target {
                        Target::Name(id, name) => {
                            shape.target_names.insert(*id, *name);
                            if let Meaning::Value(output) = file.resolved.of(*id)
                                && outputs.contains(&output)
                            {
                                shape.binders.insert(output, *id);
                            }
                        }
                        Target::Next(id, _) => {
                            if let Meaning::Value(carry) = file.resolved.of(*id) {
                                shape.nexts.insert(carry, *id);
                            }
                        }
                        Target::Skip(_) => {}
                    }
                }
                let StatementKind::Binding { targets, value } = &statement.kind else { continue };
                let mut children = Vec::new();
                let first = u32::try_from(shape.bodies.len()).unwrap_or(u32::MAX);
                match value {
                    Rhs::Operand(operand) => {
                        if let [target] = &**targets {
                            operand_lines.insert(target_id(target), operand);
                        }
                    }
                    Rhs::Loop(structure) => {
                        children.push(BodyInfo { body: &structure.body, parent: Some(index), holder: Holder::Loop(statement, structure) });
                        for clause in &structure.clauses {
                            if let ClauseKind::Carry(carry) = &clause.kind {
                                shape.defined_in.insert(carry.id, first);
                                shape.target_names.insert(carry.id, carry.name);
                            }
                        }
                    }
                    Rhs::For(structure) => {
                        children.push(BodyInfo { body: &structure.body, parent: Some(index), holder: Holder::For(statement, structure) });
                        for clause in &structure.clauses {
                            let (id, name) = match &clause.kind {
                                ClauseKind::In(clause) => (clause.id, clause.name),
                                ClauseKind::Carry(carry) => (carry.id, carry.name),
                            };
                            shape.defined_in.insert(id, first);
                            shape.target_names.insert(id, name);
                        }
                    }
                    Rhs::Match(branching) | Rhs::Handle(branching) => {
                        for arm in &branching.arms {
                            let arm_index = first.saturating_add(u32::try_from(children.len()).unwrap_or(u32::MAX));
                            for pattern in arm.patterns() {
                                for binding in pattern.bindings() {
                                    shape.defined_in.insert(binding.id, arm_index);
                                    shape.target_names.insert(binding.id, binding.name);
                                }
                            }
                            children.push(BodyInfo { body: &arm.body, parent: Some(index), holder: Holder::Arm { statement, arm } });
                        }
                    }
                    Rhs::Call(_) | Rhs::Struct(_) => {}
                }
                let count = u32::try_from(children.len()).unwrap_or(u32::MAX);
                shape.bodies.extend(children);
                let indices: Vec<u32> = (first..first.saturating_add(count)).collect();
                stack.extend(indices.iter().rev());
                shape.inner.insert(statement.id, indices);
            }
        }
        shape.post_order = pre_order.into_iter().rev().collect();
        shape.sort_lines(&operand_lines, file)?;
        shape.find_bases(file)?;
        shape.find_context_and_orders(function, file)?;
        Ok(shape)
    }

    /// Which lines of one operand make no node, and which are named constants (spec 5.1, R-4).
    fn sort_lines(&mut self, lines: &HashMap<Id, &'f Operand>, file: &CheckedFile) -> Result<(), String> {
        let mut constant = HashMap::new();
        for (target, operand) in lines {
            let binds = matches!(file.resolved.of(*target), Meaning::Value(_));
            match operand {
                Operand::Path(path) => {
                    if let Meaning::Value(_) = file.resolved.of(path.id) {
                        self.aliases.insert(*target, path);
                    }
                }
                Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) | Operand::Array(..) => {
                    if let Operand::Array(_, values) = operand
                        && holds_wire(values, lines, file, &mut constant)
                    {
                        self.built_arrays.insert(*target);
                        continue;
                    }
                    self.literals.insert(*target, operand);
                    if !binds {
                        self.constant_nodes.insert(*target);
                    }
                }
                Operand::Bracket(..) => return Err("a bracket on a line of its own".to_owned()),
            }
        }
        Ok(())
    }

    /// What each value that a path can read stands for: new names and field reads followed, each chain once.
    fn find_bases(&mut self, file: &CheckedFile) -> Result<(), String> {
        let starts: Vec<Id> = self.aliases.keys().chain(self.binders.keys()).copied().collect();
        let limit = self.aliases.len() + self.binders.len() + 1;
        for start in starts {
            let mut path: Vec<(Id, Vec<Name>)> = Vec::new();
            let mut current = start;
            let found = loop {
                if path.len() > limit {
                    return Err("new names that name each other".to_owned());
                }
                if let Some(base) = self.bases.get(&current) {
                    break base.clone();
                }
                if let Some(target) = self.binders.get(&current) {
                    path.push((current, Vec::new()));
                    current = *target;
                    continue;
                }
                if let Some(read) = self.aliases.get(&current) {
                    let Meaning::Value(next) = file.resolved.of(read.id) else { return Err("a new name for what is not a value".to_owned()) };
                    path.push((current, read.rest.to_vec()));
                    current = next;
                    continue;
                }
                if let Some(operand) = self.literals.get(&current) {
                    break Base::Literal { operand, node: self.constant_nodes.contains(&current).then_some(current) };
                }
                break Base::Def { id: current, fields: Vec::new() };
            };
            let mut base = found;
            for (id, fields) in path.into_iter().rev() {
                base = match base {
                    Base::Def { id: def, fields: mut deeper } => {
                        deeper.extend(fields);
                        Base::Def { id: def, fields: deeper }
                    }
                    Base::Literal { .. } if !fields.is_empty() => return Err("a field read from a constant".to_owned()),
                    literal => literal,
                };
                self.bases.insert(id, base.clone());
            }
        }
        // A new name with no field is a name of the value it names.
        let mut more: Vec<(Id, Id)> = Vec::new();
        for (target, read) in &self.aliases {
            if read.rest.is_empty()
                && let Some(Base::Def { id, fields }) = self.bases.get(target)
                && fields.is_empty()
            {
                more.push((*id, *target));
            }
        }
        more.sort();
        for (id, target) in more {
            let Some(name) = self.target_names.get(&target).copied() else { continue };
            self.more_names.entry(id).or_default().push(name);
        }
        Ok(())
    }

    /// What a value read by this id stands for.
    pub(super) fn base(&self, value: Id) -> Base<'f> {
        if let Some(base) = self.bases.get(&value) {
            return base.clone();
        }
        if let Some(operand) = self.literals.get(&value) {
            return Base::Literal { operand, node: self.constant_nodes.contains(&value).then_some(value) };
        }
        Base::Def { id: value, fields: Vec::new() }
    }

    /// What a path reads: its value, then its own fields.
    pub(super) fn base_of_path(&self, path: &Path, file: &CheckedFile) -> Option<Base<'f>> {
        let Meaning::Value(value) = file.resolved.of(path.id) else { return None };
        Some(match self.base(value) {
            Base::Def { id, mut fields } => {
                fields.extend(path.rest.iter().copied());
                Base::Def { id, fields }
            }
            literal => literal,
        })
    }

    fn parent_statement(&self, body: u32) -> Option<(Id, u32)> {
        let info = self.bodies.get(body as usize)?;
        let statement = info.holder.statement()?;
        Some((statement.id, info.parent?))
    }

    /// A value read in `body`, defined in `defined`: each structure between them passes it in.
    fn read_in(&mut self, body: u32, value: Id, seen: &mut HashMap<Id, HashSet<Id>>) -> Result<(), String> {
        let Some(defined) = self.defined_in.get(&value).copied() else { return Err("a value read that nothing defines".to_owned()) };
        let mut current = body;
        while current != defined {
            let Some((statement, parent)) = self.parent_statement(current) else {
                return Err("a value read outside the body that defines it".to_owned());
            };
            if !seen.entry(statement).or_default().insert(value) {
                break;
            }
            self.context.entry(statement).or_default().push(value);
            current = parent;
        }
        Ok(())
    }

    fn read_operand(&mut self, body: u32, operand: &Operand, file: &CheckedFile, seen: &mut HashMap<Id, HashSet<Id>>) -> Result<(), String> {
        let mut pending = vec![operand];
        while let Some(operand) = pending.pop() {
            match operand {
                Operand::Path(path) => {
                    if let Some(Base::Def { id, .. }) = self.base_of_path(path, file) {
                        self.read_in(body, id, seen)?;
                    }
                }
                Operand::Bracket(id, _) => {
                    if let Meaning::ArrayOfValue(value) = file.resolved.of(*id)
                        && let Base::Def { id, .. } = self.base(value)
                    {
                        self.read_in(body, id, seen)?;
                    }
                }
                Operand::Array(_, values) => pending.extend(values.iter().rev()),
                Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) => {}
            }
        }
        Ok(())
    }

    /// What a target that binds a result stands for, read in its body.
    fn read_target(&mut self, body: u32, target: Id, seen: &mut HashMap<Id, HashSet<Id>>) -> Result<(), String> {
        if let Base::Def { id, .. } = self.base(target) {
            self.read_in(body, id, seen)?;
        }
        Ok(())
    }

    /// The bodies from this one up to the function, each with the statement that holds it.
    fn ancestors(&self, body: u32) -> Vec<(u32, Option<Id>)> {
        let mut chain = Vec::new();
        let mut current = Some(body);
        while let Some(index) = current {
            let Some(info) = self.bodies.get(index as usize) else { break };
            chain.push((index, info.holder.statement().map(|statement| statement.id)));
            current = info.parent;
        }
        chain
    }

    /// The node of `body`'s region that holds `statement` of a body inside it, or `statement` itself.
    fn representative(&self, statement: Id, from: u32, region: u32) -> Option<Id> {
        let mut current = statement;
        let mut body = from;
        while body != region {
            let (holder, parent) = self.parent_statement(body)?;
            current = holder;
            body = parent;
        }
        Some(current)
    }

    fn order(&mut self, waiter: Id, body: u32, wait: Wait, target_body: u32) -> Result<(), String> {
        let waiter_chain = self.ancestors(body);
        let target_chain: HashSet<u32> = self.ancestors(target_body).into_iter().map(|(index, _)| index).collect();
        let Some(region) = waiter_chain.iter().map(|(index, _)| *index).find(|index| target_chain.contains(index)) else {
            return Err("an order across functions".to_owned());
        };
        let Some(by) = self.representative(waiter, body, region) else { return Err("an order from nowhere".to_owned()) };
        let wait = match wait {
            Wait::Value(value) if target_body == region => Wait::Value(value),
            Wait::Value(value) => {
                // What defines it is an argument of a region inside, or a node there: the order goes to that structure.
                let statement = self.statement_of_target.get(&value).copied();
                let holder = match statement {
                    Some(statement) => self.representative(statement, target_body, region),
                    None => self.parent_statement(target_body).and_then(|(statement, parent)| self.representative(statement, parent, region)),
                };
                Wait::Node(holder.ok_or("an order on a value of nowhere")?)
            }
            Wait::Node(statement) => Wait::Node(self.representative(statement, target_body, region).ok_or("an order on a node of nowhere")?),
        };
        if wait == Wait::Node(by) {
            return Err("an order between two regions of one structure".to_owned());
        }
        if self.ordered.insert((by, wait)) {
            self.orders.entry(by).or_default().push(wait);
        }
        Ok(())
    }

    fn find_context_and_orders(&mut self, function: &'f Function, file: &'f CheckedFile) -> Result<(), String> {
        let mut seen: HashMap<Id, HashSet<Id>> = HashMap::new();
        // In the order the bodies were found, and their lines in written order: what is passed in comes in the order of a walk
        // of the file, which `fmt` makes canonical.
        let mut order: Vec<u32> = self.post_order.clone();
        order.reverse();
        for index in order {
            let Some(info) = self.bodies.get(index as usize) else { continue };
            let (body, holder) = (info.body, info.holder);
            for statement in &body.statements {
                match &statement.kind {
                    StatementKind::Call(call) => self.read_call(index, statement.id, &call.args, file, &mut seen)?,
                    StatementKind::Collect(value) | StatementKind::Until(value) => self.read_operand(index, value, file, &mut seen)?,
                    StatementKind::Binding { targets, value } => match value {
                        Rhs::Call(call) => self.read_call(index, statement.id, &call.args, file, &mut seen)?,
                        Rhs::Struct(literal) => {
                            for field in &literal.fields {
                                self.read_operand(index, &field.value, file, &mut seen)?;
                            }
                        }
                        Rhs::Operand(operand) => {
                            let target = targets.first().map(target_id);
                            if target.is_some_and(|target| self.built_arrays.contains(&target)) {
                                self.read_operand(index, operand, file, &mut seen)?;
                            }
                        }
                        Rhs::Match(branching) | Rhs::Handle(branching) => self.read_operand(index, &branching.selector, file, &mut seen)?,
                        Rhs::Loop(structure) => {
                            for clause in &structure.clauses {
                                if let ClauseKind::Carry(carry) = &clause.kind {
                                    self.read_operand(index, &carry.value, file, &mut seen)?;
                                }
                            }
                        }
                        Rhs::For(structure) => {
                            for clause in &structure.clauses {
                                match &clause.kind {
                                    ClauseKind::In(clause) => {
                                        self.read_operand(index, &clause.source, file, &mut seen)?;
                                        if let Some(end) = &clause.end {
                                            self.read_operand(index, end, file, &mut seen)?;
                                        }
                                    }
                                    ClauseKind::Carry(carry) => self.read_operand(index, &carry.value, file, &mut seen)?,
                                }
                            }
                        }
                    },
                }
            }
            // What leaves the region.
            match holder {
                Holder::Function => {
                    for output in &function.signature.outputs {
                        if let Some(target) = self.binders.get(&output.id).copied() {
                            self.read_target(index, target, &mut seen)?;
                        }
                    }
                }
                Holder::Loop(_, structure) => self.read_carries(index, structure.clauses.iter().map(|clause| &clause.kind), &mut seen)?,
                Holder::For(_, structure) => self.read_carries(index, structure.clauses.iter().map(|clause| &clause.kind), &mut seen)?,
                Holder::Arm { arm, .. } => {
                    for value in &arm.yields.values {
                        self.read_operand(index, value, file, &mut seen)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn read_carries<'k>(
        &mut self,
        body: u32,
        clauses: impl Iterator<Item = &'k ClauseKind>,
        seen: &mut HashMap<Id, HashSet<Id>>,
    ) -> Result<(), String> {
        for clause in clauses {
            if let ClauseKind::Carry(carry) = clause
                && let Some(next) = self.nexts.get(&carry.id).copied()
            {
                self.read_target(body, next, seen)?;
            }
        }
        Ok(())
    }

    fn read_call(&mut self, body: u32, statement: Id, args: &[Arg], file: &CheckedFile, seen: &mut HashMap<Id, HashSet<Id>>) -> Result<(), String> {
        for arg in args {
            match arg {
                Arg::Value { value, .. } => self.read_operand(body, value, file, seen)?,
                Arg::After { refs, .. } => {
                    for reference in refs {
                        self.after(body, statement, reference, file)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn after(&mut self, body: u32, statement: Id, reference: &OrderRef, file: &CheckedFile) -> Result<(), String> {
        match reference {
            OrderRef::Value(Operand::Path(path)) => match self.base_of_path(path, file) {
                Some(Base::Def { id, .. }) => {
                    let Some(target_body) = self.defined_in.get(&id).copied() else {
                        return Err("an order on a value that nothing defines".to_owned());
                    };
                    self.order(statement, body, Wait::Value(id), target_body)
                }
                Some(Base::Literal { node: Some(target), .. }) => {
                    let Some(line) = self.statement_of_target.get(&target).copied() else { return Ok(()) };
                    let Some(target_body) = self.body_of_statement.get(&line).copied() else { return Ok(()) };
                    self.order(statement, body, Wait::Node(line), target_body)
                }
                // A literal bound to an output is no node, and is there from the start.
                Some(Base::Literal { node: None, .. }) => Ok(()),
                None => Err("an order on what is not a value".to_owned()),
            },
            OrderRef::Value(_) => Ok(()),
            OrderRef::Label(label) => {
                let Meaning::Label(target) = file.resolved.of(label.id) else { return Err("an order on a label that names nothing".to_owned()) };
                let Some(line) = self.label_statement.get(&target).copied() else { return Err("a label of no line".to_owned()) };
                let Some(target_body) = self.body_of_statement.get(&line).copied() else { return Err("a line of no body".to_owned()) };
                self.order(statement, body, Wait::Node(line), target_body)
            }
        }
    }
}

/// Whether an array written on a line holds a wire, at any depth (spec 5.1): a name of a constant is no wire. Arrays nest as deep as
/// `check` let them.
fn holds_wire(values: &[Operand], lines: &HashMap<Id, &Operand>, file: &CheckedFile, known: &mut HashMap<Id, bool>) -> bool {
    values.iter().any(|value| match value {
        Operand::Path(path) => match file.resolved.of(path.id) {
            Meaning::Value(value) => !path.rest.is_empty() || !is_constant(value, lines, file, known),
            _ => false,
        },
        Operand::Bracket(id, _) => match file.resolved.of(*id) {
            Meaning::ArrayOfValue(value) => !is_constant(value, lines, file, known),
            _ => false,
        },
        Operand::Array(_, values) => holds_wire(values, lines, file, known),
        Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) => false,
    })
}

/// Whether a value is a named constant, through new names: the line `k2 = k` names the constant `k`.
fn is_constant(value: Id, lines: &HashMap<Id, &Operand>, file: &CheckedFile, known: &mut HashMap<Id, bool>) -> bool {
    if let Some(known) = known.get(&value) {
        return *known;
    }
    let mut chain = vec![value];
    let mut current = value;
    let answer = loop {
        if chain.len() > lines.len() + 1 {
            break false;
        }
        // A line that binds an output or a carry is a terminal, never a constant.
        if matches!(file.resolved.of(current), Meaning::Value(_)) {
            break false;
        }
        match lines.get(&current) {
            Some(Operand::Path(path)) if path.rest.is_empty() => match file.resolved.of(path.id) {
                Meaning::Value(next) => {
                    if let Some(known) = known.get(&next) {
                        break *known;
                    }
                    current = next;
                    chain.push(next);
                }
                _ => break false,
            },
            Some(Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_)) => break true,
            Some(Operand::Array(_, values)) => {
                // Marked first, so that an array that names itself is a wire and not an endless walk.
                known.insert(current, false);
                break !holds_wire(values, lines, file, known);
            }
            _ => break false,
        }
    };
    for id in chain {
        known.insert(id, answer);
    }
    answer
}
