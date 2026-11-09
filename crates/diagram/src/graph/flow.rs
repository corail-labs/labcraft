//! The frames of the bodies and the wires between terminals: a value from outside a structure crosses each edge on its
//! way, one wire per frame, its readers inside sharing the part outside (spec 11, notation 5).

use std::collections::HashMap;

use labcraft_check::Meaning;
use labcraft_format::{write_operand, write_pattern};
use labcraft_graph::{Body, ClauseKind, Constant, Id, Operand, OrderRef, Pattern, Rhs, Signature, Statement, StatementKind, Target};
use labcraft_types::{Name, TyId, cut_text};

use super::{
    BodyOf, Builder, CONSTANT_BYTES, Counts, Edge, End, Family, FrameTerminal, Graph, Kind, Node, Slot, Structure, StructureKind, Terminal, TooLarge,
    Walked, Wire, path_text,
};

/// What the walk of a function learns, which the wires need.
#[derive(Default)]
pub(super) struct Flow {
    pub statement_node: HashMap<Id, usize>,
    pub label_node: HashMap<Id, usize>,
    /// The nodes that each name of the `layout` section designates (spec 11).
    pub designations: HashMap<(bool, Name), Vec<usize>>,
    /// Where each value is defined: its body, where its wires start there, its name and its type.
    defs: HashMap<Id, (usize, End, String, TyId)>,
    /// The output of the function that a name binds, by the id of that output.
    outputs: HashMap<Id, usize>,
    /// The `next` box of each carry: its body and its index on the right.
    nexts: HashMap<Id, (usize, usize)>,
    /// The notch of an outer value on the left of a body: by body and value.
    notches: HashMap<(usize, Id), usize>,
    /// The input of a structure through which an outer value enters it: by node and value.
    crossings: HashMap<(usize, Id), usize>,
    /// For each input of a structure that an outer value takes, its notch in each body that reads it: the number of
    /// the body among the bodies of the structure, and the index of the notch.
    pub arm_notches: HashMap<(usize, usize), Vec<(usize, usize)>>,
}

impl<'a> Builder<'a> {
    fn frame_terminal(&self, name: String, ty: TyId, edge: Edge) -> FrameTerminal {
        FrameTerminal { terminal: self.terminal(name, ty, None, false), edge }
    }

    /// The frame of the function: its inputs on the left, its outputs on the right, in signature order (notation 2).
    pub(super) fn function_frame(&self, caption: String, signature: &Signature) -> BodyOf {
        let declared = self.checked.libraries.get(self.file.library as usize).and_then(|library| {
            let name = match &self.file.file.decl {
                labcraft_graph::Decl::Function(function) => function.name,
                labcraft_graph::Decl::Extern(external) => external.name,
                labcraft_graph::Decl::Type(declared) => declared.name,
            };
            library.functions.get(&name)
        });
        let left = signature
            .params
            .iter()
            .enumerate()
            .map(|(index, param)| {
                let ty = declared.and_then(|declared| declared.params.get(index)).map_or(TyId::ERROR, |param| param.ty);
                let mut terminal = self.frame_terminal(self.text(param.name), ty, Edge::Input);
                terminal.terminal.constant = param.default.as_ref().map(|default| match default {
                    Constant::Operand(operand) => self.constant_text(operand),
                    Constant::Call(call) => format!("{}(\u{2026})", path_text(&call.callee, self.names)),
                    Constant::Struct(lit) => format!("{} {{\u{2026}}}", path_text(&lit.path, self.names)),
                });
                terminal
            })
            .collect();
        let right = signature
            .outputs
            .iter()
            .enumerate()
            .map(|(index, output)| {
                let ty = declared.and_then(|declared| declared.outputs.get(index)).map_or(TyId::ERROR, |output| output.ty);
                self.frame_terminal(self.text(output.name), ty, Edge::Output)
            })
            .collect();
        BodyOf {
            caption,
            arm: None,
            nodes: Vec::new(),
            owner: None,
            key: String::new(),
            left,
            right,
            slots: Vec::new(),
            pair: (None, None),
            wires: Vec::new(),
        }
    }

    /// A structure as a node: its header, its inputs (the first value of each register, what an `in` walks, the
    /// selector), and one output per name that its line binds. The outer values read inside are added as inputs when
    /// the wires are made.
    pub(super) fn structure(&self, value: &Rhs, targets: &[Target]) -> (Kind<'a>, Vec<Terminal>, Vec<Terminal>) {
        let text = |operand: &Operand| cut_text(&write_operand(operand, self.names, CONSTANT_BYTES)).into_owned();
        let input = |name: String, operand: &Operand| self.input(name, operand, None);
        let mut inputs = Vec::new();
        let (kind, header, badge, parallel) = match value {
            Rhs::Loop(structure) => {
                inputs.extend(structure.clauses.iter().filter_map(|clause| match &clause.kind {
                    ClauseKind::Carry(carry) => Some(input(self.text(carry.name), &carry.value)),
                    ClauseKind::In(_) => None,
                }));
                (StructureKind::Loop, "loop".to_owned(), structure.continue_on_error.then_some("continue_on_error"), false)
            }
            Rhs::For(structure) => {
                let mut walked = Vec::new();
                for clause in &structure.clauses {
                    match &clause.kind {
                        ClauseKind::Carry(carry) => inputs.push(input(self.text(carry.name), &carry.value)),
                        ClauseKind::In(clause) => {
                            let mut terminal = input(self.text(clause.name), &clause.source);
                            let source = match &clause.end {
                                Some(end) => {
                                    terminal.reads.extend(self.reads(end));
                                    format!("{}..{}", text(&clause.source), text(end))
                                }
                                None => text(&clause.source),
                            };
                            walked.push(format!("{} in {source}", self.text(clause.name)));
                            inputs.push(terminal);
                        }
                    }
                }
                let keyword = if structure.parallel { "for parallel" } else { "for" };
                (StructureKind::For, format!("{keyword} {}", walked.join(", ")), None, structure.parallel)
            }
            Rhs::Match(branching) | Rhs::Handle(branching) => {
                let (kind, keyword) =
                    if matches!(value, Rhs::Handle(_)) { (StructureKind::Handle, "handle") } else { (StructureKind::Match, "match") };
                let selector = text(&branching.selector);
                inputs.push(input(selector.clone(), &branching.selector));
                (kind, format!("{keyword} {selector}"), None, false)
            }
            Rhs::Call(_) | Rhs::Struct(_) | Rhs::Operand(_) => (StructureKind::Loop, String::new(), None, false),
        };
        let outputs = (0..targets.len())
            .map(|index| self.output_of(targets, index, &targets.get(index).map(|target| self.target_text(target)).unwrap_or_default()))
            .collect();
        let structure = Structure { kind, header: cut_text(&header).into_owned(), badge, parallel, inputs_at: Vec::new(), pair: None };
        (Kind::Structure(structure), inputs, outputs)
    }

    /// The frame of a body of a structure, before its notches: the registers and the `in` of a loop on the left, its
    /// `next` boxes, its `collect` and its `until` on the right; the values that the patterns of an arm bind on the left,
    /// what it yields on the right.
    pub(super) fn structure_frame(&self, statement: &Statement, arm: Option<usize>, body: &Body, key: String) -> BodyOf {
        let StatementKind::Binding { targets, value } = &statement.kind else { return self.function_frame(String::new(), &Signature::default()) };
        let mut frame = BodyOf {
            caption: String::new(),
            arm,
            nodes: Vec::new(),
            owner: None,
            key,
            left: Vec::new(),
            right: Vec::new(),
            slots: Vec::new(),
            pair: (None, None),
            wires: Vec::new(),
        };
        match value {
            Rhs::Loop(_) | Rhs::For(_) => {
                let clauses = match value {
                    Rhs::Loop(structure) => &structure.clauses,
                    Rhs::For(structure) => &structure.clauses,
                    _ => return frame,
                };
                let mut carries = 0;
                for clause in clauses {
                    match &clause.kind {
                        ClauseKind::Carry(carry) => {
                            let ty = self.resolved().ty(carry.id);
                            frame.left.push(self.frame_terminal(self.text(carry.name), ty, Edge::Register));
                            frame.right.push(self.frame_terminal(format!("next {}", self.text(carry.name)), ty, Edge::Register));
                            carries += 1;
                        }
                        ClauseKind::In(clause) => {
                            frame.left.push(self.frame_terminal(self.text(clause.name), self.resolved().ty(clause.id), Edge::In))
                        }
                    }
                }
                let mut collects = 0;
                for line in &body.statements {
                    match &line.kind {
                        StatementKind::Collect(operand) => {
                            let name = targets.get(carries + collects).map_or_else(|| "[ ]".to_owned(), |target| self.target_text(target));
                            frame.right.push(self.frame_terminal(name, self.resolved().ty(operand.id()), Edge::Collect));
                            collects += 1;
                        }
                        StatementKind::Until(_) => {}
                        StatementKind::Binding { .. } | StatementKind::Call(_) => {}
                    }
                }
                if let Some(operand) = body.statements.iter().find_map(|line| match &line.kind {
                    StatementKind::Until(operand) => Some(operand),
                    _ => None,
                }) {
                    frame.right.push(self.frame_terminal("until".to_owned(), self.resolved().ty(operand.id()), Edge::Until));
                }
            }
            Rhs::Match(branching) | Rhs::Handle(branching) => {
                let Some(arm_of) = arm.and_then(|number| branching.arms.get(number)) else { return frame };
                let titles: Vec<String> = arm_of
                    .patterns()
                    .map(|pattern| match pattern {
                        Pattern::Wildcard { binding: None, .. } => "other".to_owned(),
                        Pattern::Wildcard { binding: Some(binding), .. } => format!("other ({})", self.text(binding.name)),
                        _ => write_pattern(pattern, self.names, CONSTANT_BYTES),
                    })
                    .collect();
                frame.caption = cut_text(&titles.join(", ")).into_owned();
                for pattern in arm_of.patterns() {
                    for binding in pattern.bindings() {
                        frame.left.push(self.frame_terminal(self.text(binding.name), self.resolved().ty(binding.id), Edge::Bound));
                    }
                }
                for (index, target) in targets.iter().enumerate() {
                    let mut terminal = self.output_of(targets, index, &self.target_text(target));
                    terminal.ignored = false;
                    if let Some(operand) = arm_of.yields.values.get(index)
                        && super::is_constant(operand, self.resolved())
                    {
                        terminal.constant = Some(self.constant_text(operand));
                    }
                    frame.right.push(FrameTerminal { terminal, edge: Edge::Yield });
                }
            }
            Rhs::Call(_) | Rhs::Struct(_) | Rhs::Operand(_) => {}
        }
        frame
    }

    /// The wires of every body: first where each value is defined, then, in the order of the lines, what each line reads.
    pub(super) fn wires(
        &self,
        graph: &mut Graph<'a>,
        walked: &[Walked<'a>],
        flow: &mut Flow,
        counts: &mut Counts,
        max_terminals: u64,
    ) -> Result<(), TooLarge> {
        let function = match &self.file.file.decl {
            labcraft_graph::Decl::Function(function) => function,
            _ => return Ok(()),
        };
        for (index, param) in function.signature.params.iter().enumerate() {
            let ty = graph.bodies.first().and_then(|body| body.left.get(index)).map_or(TyId::ERROR, |terminal| terminal.terminal.ty);
            flow.defs.insert(param.id, (0, End::Left(index), self.text(param.name), ty));
        }
        for (index, output) in function.signature.outputs.iter().enumerate() {
            flow.outputs.insert(output.id, index);
        }
        // Where each value is defined.
        for (body_index, (owner, arm, body, _)) in walked.iter().enumerate() {
            if let Some(StatementKind::Binding { value, .. }) = owner.map(|owner| &owner.kind) {
                let clauses = match value {
                    Rhs::Loop(structure) => Some(&structure.clauses),
                    Rhs::For(structure) => Some(&structure.clauses),
                    _ => None,
                };
                let mut carries = 0;
                for (index, clause) in clauses.into_iter().flatten().enumerate() {
                    match &clause.kind {
                        ClauseKind::Carry(carry) => {
                            let ty = self.resolved().ty(carry.id);
                            flow.defs.insert(carry.id, (body_index, End::Left(index), self.text(carry.name), ty));
                            flow.nexts.insert(carry.id, (body_index, carries));
                            carries += 1;
                        }
                        ClauseKind::In(clause) => {
                            flow.defs.insert(clause.id, (body_index, End::Left(index), self.text(clause.name), self.resolved().ty(clause.id)));
                        }
                    }
                }
                if let (Rhs::Match(branching) | Rhs::Handle(branching), Some(number)) = (value, arm)
                    && let Some(arm) = branching.arms.get(*number)
                {
                    let mut index = 0;
                    for pattern in arm.patterns() {
                        for binding in pattern.bindings() {
                            flow.defs.insert(binding.id, (body_index, End::Left(index), self.text(binding.name), self.resolved().ty(binding.id)));
                            index += 1;
                        }
                    }
                }
            }
            for statement in &body.statements {
                let Some(&node) = flow.statement_node.get(&statement.id) else { continue };
                let StatementKind::Binding { targets, .. } = &statement.kind else { continue };
                for (index, target) in targets.iter().enumerate() {
                    let (Target::Name(id, name) | Target::Next(id, name)) = target else { continue };
                    let entry = (body_index, End::Node { node, terminal: index }, self.text(*name), self.resolved().ty(*id));
                    if let (Target::Name(..), Meaning::Value(output)) = (target, self.resolved().of(*id))
                        && flow.outputs.contains_key(&output)
                    {
                        flow.defs.insert(output, entry.clone());
                    }
                    flow.defs.insert(*id, entry);
                }
            }
        }
        // What each line reads, in the order of the lines: a new name for a wire is defined where it is met.
        for (body_index, (owner, arm, body, order)) in walked.iter().enumerate() {
            let mut collects =
                graph.bodies.get(body_index).map_or(0, |frame| frame.right.iter().take_while(|terminal| terminal.edge == Edge::Register).count());
            let until = graph.bodies.get(body_index).and_then(|frame| frame.right.iter().position(|terminal| terminal.edge == Edge::Until));
            let collect_of: HashMap<Id, usize> = body
                .statements
                .iter()
                .filter(|line| matches!(line.kind, StatementKind::Collect(_)))
                .map(|line| {
                    collects += 1;
                    (line.id, collects - 1)
                })
                .collect();
            for &line in order {
                let Some(statement) = body.statements.get(line) else { continue };
                match &statement.kind {
                    StatementKind::Collect(operand) | StatementKind::Until(operand) => {
                        let right = match statement.kind {
                            StatementKind::Until(_) => until,
                            _ => collect_of.get(&statement.id).copied(),
                        };
                        for (read, name) in self.reads(operand) {
                            if let (Some(right), Some((from, family))) = (right, self.source(graph, flow, body_index, read, counts, max_terminals)?) {
                                push(graph, body_index, Wire { from, to: End::Right(right), family, name, read: None });
                            }
                        }
                    }
                    StatementKind::Binding { targets, value: Rhs::Operand(operand) } if !super::is_constant(operand, self.resolved()) => {
                        // A new name for a wire: no node, the wire goes on under that name (R-4).
                        let Operand::Path(path) = operand else { continue };
                        let Some((from, family)) = self.source(graph, flow, body_index, path.id, counts, max_terminals)? else { continue };
                        let name = cut_text(&path_text(path, self.names)).into_owned();
                        for target in targets.iter() {
                            let (Target::Name(id, _) | Target::Next(id, _)) = target else { continue };
                            let ty = self.resolved().ty(*id);
                            flow.defs.insert(*id, (body_index, from, name.clone(), ty));
                            self.bind(graph, flow, body_index, target, from, family, &name);
                        }
                    }
                    _ => {
                        let Some(&node) = flow.statement_node.get(&statement.id) else { continue };
                        self.node_wires(graph, flow, body_index, statement, node, counts, max_terminals)?;
                    }
                }
            }
            // What an arm yields.
            if let (Some(owner), Some(number)) = (owner, arm)
                && let StatementKind::Binding { value: Rhs::Match(branching) | Rhs::Handle(branching), .. } = &owner.kind
                && let Some(arm) = branching.arms.get(*number)
            {
                for (index, value) in arm.yields.values.iter().enumerate() {
                    for (read, name) in self.reads(value) {
                        if let Some((from, family)) = self.source(graph, flow, body_index, read, counts, max_terminals)? {
                            push(graph, body_index, Wire { from, to: End::Right(index), family, name, read: None });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// The wires into a node, and from its outputs to the frame when its line binds an output of the function or a
    /// `next`.
    #[allow(clippy::too_many_arguments)]
    fn node_wires(
        &self,
        graph: &mut Graph<'a>,
        flow: &mut Flow,
        body: usize,
        statement: &Statement,
        node: usize,
        counts: &mut Counts,
        max_terminals: u64,
    ) -> Result<(), TooLarge> {
        let reads: Vec<(usize, Vec<(Id, String)>)> = graph
            .nodes
            .get(node)
            .map(|found| found.inputs.iter().enumerate().map(|(index, terminal)| (index, terminal.reads.clone())).collect())
            .unwrap_or_default();
        for (terminal, ids) in reads {
            for (read, name) in ids {
                if let Some((from, family)) = self.source(graph, flow, body, read, counts, max_terminals)? {
                    push(graph, body, Wire { from, to: End::Node { node, terminal }, family, name, read: Some(read) });
                }
            }
        }
        // `after:` (R-15): from a value, or from the node of a label.
        let call = match &statement.kind {
            StatementKind::Binding { value: Rhs::Call(call), .. } => Some(call),
            StatementKind::Call(call) => Some(call),
            _ => None,
        };
        for arg in call.iter().flat_map(|call| call.args.iter()) {
            let labcraft_graph::Arg::After { refs, .. } = arg else { continue };
            for reference in refs {
                match reference {
                    OrderRef::Value(operand) => {
                        for (read, name) in self.reads(operand) {
                            if let Some((from, _)) = self.source(graph, flow, body, read, counts, max_terminals)? {
                                push(graph, body, Wire { from, to: End::Order(node), family: Family::ORDER, name, read: None });
                            }
                        }
                    }
                    OrderRef::Label(label) => {
                        let Meaning::Label(id) = self.resolved().of(label.id) else { continue };
                        if let Some(&from) = flow.label_node.get(&id)
                            && graph.nodes.get(from).is_some_and(|found| found.body == body)
                        {
                            push(
                                graph,
                                body,
                                Wire {
                                    from: End::Order(from),
                                    to: End::Order(node),
                                    family: Family::ORDER,
                                    name: format!("#{}", self.text(label.name)),
                                    read: None,
                                },
                            );
                        }
                    }
                }
            }
        }
        if let StatementKind::Binding { targets, .. } = &statement.kind {
            for (index, target) in targets.iter().enumerate() {
                let family = graph.nodes.get(node).and_then(|found| found.outputs.get(index)).map_or(Family::ERROR, |terminal| terminal.family);
                let name = self.target_text(target);
                self.bind(graph, flow, body, target, End::Node { node, terminal: index }, family, &name);
            }
        }
        Ok(())
    }

    /// A target that binds an output of the function or a `next`: a wire to the right of the frame.
    #[allow(clippy::too_many_arguments)]
    fn bind(&self, graph: &mut Graph<'a>, flow: &Flow, body: usize, target: &Target, from: End, family: Family, name: &str) {
        let right = match target {
            Target::Name(id, _) => match self.resolved().of(*id) {
                Meaning::Value(output) => flow.outputs.get(&output).filter(|_| body == 0).copied(),
                _ => None,
            },
            Target::Next(id, _) => match self.resolved().of(*id) {
                Meaning::Value(carry) => flow.nexts.get(&carry).filter(|(at, _)| *at == body).map(|(_, right)| *right),
                _ => None,
            },
            Target::Skip(_) => None,
        };
        // What it binds is where `check` puts a conversion into the output (spec 3.3), shown by a pill (notation 4.2).
        let read = match target {
            Target::Name(id, _) | Target::Next(id, _) => Some(*id),
            Target::Skip(_) => None,
        };
        if let Some(right) = right {
            push(graph, body, Wire { from, to: End::Right(right), family, name: name.trim_start_matches("next ").to_owned(), read });
        }
    }

    /// Where the wire of what `read` reads starts in `body`: where the value is defined, or the notch through which it
    /// enters, made with the wires outside on the first read. `None` when the value has no wire (an error said why).
    fn source(
        &self,
        graph: &mut Graph<'a>,
        flow: &mut Flow,
        body: usize,
        read: Id,
        counts: &mut Counts,
        max_terminals: u64,
    ) -> Result<Option<(End, Family)>, TooLarge> {
        let def = match self.resolved().of(read) {
            Meaning::Value(def) | Meaning::ArrayOfValue(def) => def,
            _ => return Ok(None),
        };
        let Some((defined, end, value, ty)) = flow.defs.get(&def).cloned() else { return Ok(None) };
        let family = super::family(self.types(), &self.checked.libraries, ty);
        // The bodies from this one up to the one that defines the value.
        let mut chain = vec![body];
        let mut current = body;
        while current != defined {
            let Some(owner) = graph.bodies.get(current).and_then(|frame| frame.owner) else { return Ok(None) };
            let Some(parent) = graph.nodes.get(owner).map(|node| node.body) else { return Ok(None) };
            chain.push(parent);
            current = parent;
        }
        let mut from = end;
        // From the outside in: each frame crossed has an input on its structure and a notch inside.
        for pair in chain.windows(2).rev() {
            let (Some(&inner), Some(&outer)) = (pair.first(), pair.get(1)) else { continue };
            let Some(structure) = graph.bodies.get(inner).and_then(|frame| frame.owner) else { return Ok(None) };
            // The selector of a `match` or a `handle` serves as the notch of its value (notation 5): no second wire.
            let selector = graph.nodes.get(structure).is_some_and(|node| {
                matches!(&node.kind, Kind::Structure(found) if matches!(found.kind, StructureKind::Match | StructureKind::Handle))
                    && node
                        .inputs
                        .first()
                        .is_some_and(|first| first.reads.iter().any(|(id, _)| matches!(self.resolved().of(*id), Meaning::Value(read) if read == def)))
            });
            if selector {
                flow.crossings.entry((structure, def)).or_insert(0);
            }
            let input = match flow.crossings.get(&(structure, def)) {
                Some(&input) => input,
                None => {
                    counts.terminals = counts.terminals.saturating_add(1);
                    if counts.terminals > max_terminals {
                        return Err(TooLarge(*counts));
                    }
                    let terminal = self.terminal(value.clone(), ty, None, false);
                    let Some(node) = graph.nodes.get_mut(structure) else { return Ok(None) };
                    node.inputs.push(terminal);
                    let input = node.inputs.len() - 1;
                    flow.crossings.insert((structure, def), input);
                    push(graph, outer, Wire { from, to: End::Node { node: structure, terminal: input }, family, name: value.clone(), read: None });
                    input
                }
            };
            let notch = match flow.notches.get(&(inner, def)) {
                Some(&notch) => notch,
                None => {
                    // A notch is a terminal too (`LC7002`): an arm of a `match` has its own.
                    counts.terminals = counts.terminals.saturating_add(1);
                    if counts.terminals > max_terminals {
                        return Err(TooLarge(*counts));
                    }
                    let terminal = self.frame_terminal(value.clone(), ty, Edge::Notch);
                    let Some(frame) = graph.bodies.get_mut(inner) else { return Ok(None) };
                    frame.left.push(terminal);
                    let notch = frame.left.len() - 1;
                    let arm = frame.arm.unwrap_or(0);
                    flow.notches.insert((inner, def), notch);
                    flow.arm_notches.entry((structure, input)).or_default().push((arm, notch));
                    notch
                }
            };
            from = End::Left(notch);
        }
        Ok(Some((from, family)))
    }
}

fn push(graph: &mut Graph<'_>, body: usize, wire: Wire) {
    if let Some(frame) = graph.bodies.get_mut(body) {
        frame.wires.push(wire);
    }
}

/// The places of the terminals of a frame, from the top, and its error pair (notation 2.1, 5): `err` at the bottom of
/// each edge; for a loop, one list for both edges, a register at one height on each.
pub(super) fn finish_frame(body: &mut BodyOf, owner: Option<StructureKind>) {
    let error = |terminal: &FrameTerminal| terminal.terminal.family == Family::ERROR;
    let named_err = |terminals: &[FrameTerminal]| {
        terminals.iter().position(|terminal| terminal.terminal.name == "err" && error(terminal)).or_else(|| terminals.iter().rposition(error))
    };
    match owner {
        None => {
            let left = body.left.iter().position(|terminal| terminal.terminal.name == "err" && error(terminal));
            let right = named_err(&body.right);
            body.pair = (left, right);
            body.slots = vec![edge_list(body.left.len(), left, true), edge_list(body.right.len(), right, false)];
        }
        Some(StructureKind::Loop | StructureKind::For) => {
            let register = body.left.iter().position(|terminal| terminal.edge == Edge::Register && error(terminal));
            // The registers come first on the right, in the order of the carries.
            let mut next = 0;
            let mut slots = Vec::new();
            let mut last = None;
            for (index, terminal) in body.left.iter().enumerate() {
                match terminal.edge {
                    Edge::Register => {
                        let slot = Slot { left: Some(index), right: Some(next) };
                        next += 1;
                        if Some(index) == register { last = Some(slot) } else { slots.push(slot) }
                    }
                    Edge::In => slots.push(Slot { left: Some(index), right: None }),
                    _ => {}
                }
            }
            slots.extend(
                body.right
                    .iter()
                    .enumerate()
                    .filter(|(_, terminal)| terminal.edge == Edge::Collect)
                    .map(|(index, _)| Slot { left: None, right: Some(index) }),
            );
            slots.extend(
                body.left
                    .iter()
                    .enumerate()
                    .filter(|(_, terminal)| terminal.edge == Edge::Notch)
                    .map(|(index, _)| Slot { left: Some(index), right: None }),
            );
            slots.extend(body.right.iter().position(|terminal| terminal.edge == Edge::Until).map(|index| Slot { left: None, right: Some(index) }));
            slots.extend(last);
            body.pair = (last.and_then(|slot| slot.left), last.and_then(|slot| slot.right));
            body.slots = vec![slots];
        }
        Some(kind @ (StructureKind::Match | StructureKind::Handle)) => {
            let source = if kind == StructureKind::Handle { Edge::Bound } else { Edge::Notch };
            let left = body.left.iter().position(|terminal| terminal.edge == source && error(terminal));
            let right = body.right.iter().position(error);
            body.pair = (left, right);
            body.slots = vec![edge_list(body.left.len(), left, true), edge_list(body.right.len(), right, false)];
        }
    }
}

/// One edge, in order, with its error terminal moved to the bottom.
fn edge_list(count: usize, error: Option<usize>, left: bool) -> Vec<Slot> {
    let slot = |index: usize| if left { Slot { left: Some(index), right: None } } else { Slot { left: None, right: Some(index) } };
    let mut slots: Vec<Slot> = (0..count).filter(|&index| Some(index) != error).map(slot).collect();
    slots.extend(error.map(slot));
    slots
}

/// The terminal of each body that each input of a structure is, and its error pair: the register of an `error` carry;
/// the selector of a `handle`; for a `match`, the outer error that its arms take (notation 2.1).
pub(super) fn structure_pairs(graph: &mut Graph<'_>, flow: &Flow) {
    for index in 0..graph.nodes.len() {
        let Some(node) = graph.nodes.get(index) else { continue };
        let Kind::Structure(structure) = &node.kind else { continue };
        let kind = structure.kind;
        let first_error_output = node.outputs.iter().position(|terminal| terminal.family == Family::ERROR);
        let inputs = node.inputs.len();
        let bodies = node.bodies.clone();
        let mut pair = None;
        let inputs_at: Vec<Vec<(usize, usize)>> = match kind {
            // An input of a loop is the terminal of the same index on the left of its body.
            StructureKind::Loop | StructureKind::For => (0..inputs).map(|input| vec![(0, input)]).collect(),
            StructureKind::Match | StructureKind::Handle => {
                (0..inputs).map(|input| flow.arm_notches.get(&(index, input)).cloned().unwrap_or_default()).collect()
            }
        };
        match kind {
            StructureKind::Loop | StructureKind::For => {
                let body = bodies.first().and_then(|body| graph.bodies.get(*body));
                if let Some((Some(left), Some(right))) = body.map(|body| body.pair) {
                    pair = Some((left, right));
                }
            }
            StructureKind::Match | StructureKind::Handle => {
                let entry = if kind == StructureKind::Handle {
                    node.inputs.first().filter(|terminal| terminal.family == Family::ERROR).map(|_| 0)
                } else {
                    (0..inputs).find(|&input| {
                        node.inputs.get(input).is_some_and(|terminal| terminal.family == Family::ERROR)
                            && inputs_at.get(input).is_some_and(|at| {
                                at.iter().any(|&(arm, notch)| {
                                    bodies.get(arm).and_then(|body| graph.bodies.get(*body)).is_some_and(|frame| frame.pair.0 == Some(notch))
                                })
                            })
                    })
                };
                pair = entry.zip(first_error_output);
            }
        }
        if let Some(Node { kind: Kind::Structure(structure), .. }) = graph.nodes.get_mut(index) {
            structure.inputs_at = inputs_at;
            structure.pair = pair;
        }
    }
}
