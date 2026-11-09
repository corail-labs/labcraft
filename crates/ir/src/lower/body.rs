//! The regions of one function, innermost first: each body becomes a region whose arguments are what enters it, whose nodes are
//! its lines that make one, in canonical order, and whose results are what leaves it (`docs/ir.md`, sections 3 to 5).

use std::collections::HashMap;

use labcraft_check::primitives::{ParamKind, PrimitiveDefault, PrimitiveTy, primitive};
use labcraft_check::{CheckedFile, Meaning, PrimitiveLibrary, ResolvedCall, Terminal, read_escapes, template_fields, unit_of};
use labcraft_graph::{
    Arg, Arm as WrittenArm, Body as WrittenBody, Branching, Call, ClauseKind, Function as WrittenFunction, Id, Operand, Path,
    Pattern as WrittenPattern, Rhs, Statement, StatementKind, StructLit, Target,
};
use labcraft_types::{Name, NumberKind, Ty, TyId};

use super::shape::{Base, Holder, Shape, Wait};
use super::{Literals, LowerError, Lowering, index_u32};
use crate::constant::conversion;
use crate::{Arm, Clause, ConstValue, Element, ElementId, Node, NodeKind, Order, Pattern, Port, Region, Skip, Source, Static, Use, Value, Variadic};

/// The function's regions, from the shape of its bodies.
pub(super) fn lower_function<'f>(
    lowering: &mut Lowering<'_>,
    file: &'f CheckedFile,
    function: &'f WrittenFunction,
    inputs: &[Port],
    outputs: &[Port],
    elements: &[Element],
) -> Result<Region, LowerError> {
    let shape = Shape::of(function, file).map_err(|message| lowering.bug(file, message))?;
    let mut literals: Literals<'f> = shape.literals.clone();
    for id in shape.aliases.keys().chain(shape.binders.keys()) {
        if let Base::Literal { operand, .. } = shape.base(*id) {
            literals.insert(*id, operand);
        }
    }
    let coercions = file.resolved.coercions().iter().map(|coercion| (coercion.at, (coercion.from, coercion.to))).collect();
    let mut walk = Walk {
        lowering,
        file,
        function,
        shape: &shape,
        literals,
        coercions,
        inputs,
        outputs,
        element_ids: elements.iter().enumerate().map(|(index, element)| (element.origin, ElementId(index_u32(index)))).collect(),
        built: (0..shape.bodies.len()).map(|_| None).collect(),
    };
    for index in shape.post_order.iter().copied() {
        let region = walk.body(index)?;
        if let Some(slot) = walk.built.get_mut(index as usize) {
            *slot = Some(region);
        }
    }
    walk.take(0)
}

struct Walk<'w, 'l, 'f> {
    lowering: &'w mut Lowering<'l>,
    file: &'f CheckedFile,
    function: &'f WrittenFunction,
    shape: &'w Shape<'f>,
    literals: Literals<'f>,
    /// What `check` recorded for each wire, by the id of its operand or of the target that binds it.
    coercions: HashMap<Id, (TyId, TyId)>,
    inputs: &'w [Port],
    outputs: &'w [Port],
    /// The elements of the panel, by the id of their item.
    element_ids: HashMap<Id, ElementId>,
    built: Vec<Option<Region>>,
}

/// The region being lowered: what each value defined in it or passed into it is there.
#[derive(Default)]
struct Scope {
    values: HashMap<Id, (Source, TyId)>,
    nodes: HashMap<Id, u32>,
}

impl<'f> Walk<'_, '_, 'f> {
    fn bug(&self, message: impl Into<String>) -> LowerError {
        self.lowering.bug(self.file, message)
    }

    fn take(&mut self, body: u32) -> Result<Region, LowerError> {
        self.built.get_mut(body as usize).and_then(Option::take).ok_or_else(|| self.bug("a region lowered before what it holds"))
    }

    /// The type of what defines a value: the type of its line's output, before what binding it to an output converts.
    fn value_ty(&self, id: Id) -> TyId {
        self.coercions.get(&id).map_or_else(|| self.file.resolved.ty(id), |(from, _)| *from)
    }

    /// The type an operand delivers where it goes.
    fn use_ty(&self, id: Id) -> TyId {
        self.coercions.get(&id).map_or_else(|| self.file.resolved.ty(id), |(_, to)| *to)
    }

    fn names_of(&self, id: Id, name: Option<Name>) -> Vec<Name> {
        let mut names: Vec<Name> = name.into_iter().collect();
        if let Some(more) = self.shape.more_names.get(&id) {
            names.extend(more.iter().copied());
        }
        names
    }

    fn orders_of(&self, owner: Id, function_body: bool) -> Result<Vec<usize>, LowerError> {
        let orders = self.file.orders.as_ref().ok_or_else(|| self.bug("a file with no canonical order"))?;
        let order = if function_body { orders.of_function() } else { orders.of(owner) };
        match order {
            Some(Ok(order)) => Ok(order.to_vec()),
            _ => Err(self.bug("a body with no canonical order")),
        }
    }

    // Regions.

    fn body(&mut self, index: u32) -> Result<Region, LowerError> {
        let info = self.shape.bodies.get(index as usize).ok_or_else(|| self.bug("a body of nowhere"))?;
        let (body, holder) = (info.body, info.holder);
        let mut scope = Scope::default();
        let mut args = Vec::new();
        let arg = |scope: &mut Scope, id: Id, ty: TyId, names: Vec<Name>, args: &mut Vec<Value>| {
            scope.values.insert(id, (Source::Arg(index_u32(args.len())), ty));
            args.push(Value { ty, names });
        };
        let (origin, order) = match holder {
            Holder::Function => {
                for (param, port) in self.function.signature.params.iter().zip(self.inputs) {
                    arg(&mut scope, param.id, port.ty, self.names_of(param.id, Some(param.name)), &mut args);
                }
                (self.function.id, self.orders_of(self.function.id, true)?)
            }
            Holder::Loop(statement, structure) => {
                for clause in &structure.clauses {
                    if let ClauseKind::Carry(carry) = &clause.kind {
                        arg(&mut scope, carry.id, self.file.resolved.ty(carry.id), self.names_of(carry.id, Some(carry.name)), &mut args);
                    }
                }
                (statement.id, self.orders_of(statement.id, false)?)
            }
            Holder::For(statement, structure) => {
                for clause in &structure.clauses {
                    if let ClauseKind::In(clause) = &clause.kind {
                        arg(&mut scope, clause.id, self.file.resolved.ty(clause.id), self.names_of(clause.id, Some(clause.name)), &mut args);
                    }
                }
                for clause in &structure.clauses {
                    if let ClauseKind::Carry(carry) = &clause.kind {
                        arg(&mut scope, carry.id, self.file.resolved.ty(carry.id), self.names_of(carry.id, Some(carry.name)), &mut args);
                    }
                }
                (statement.id, self.orders_of(statement.id, false)?)
            }
            Holder::Arm { arm, .. } => {
                for binding in arm.pattern.bindings() {
                    arg(&mut scope, binding.id, self.file.resolved.ty(binding.id), self.names_of(binding.id, Some(binding.name)), &mut args);
                }
                (arm.id, self.orders_of(arm.id, false)?)
            }
        };
        if let Some(statement) = holder.statement() {
            for value in self.shape.context.get(&statement.id).into_iter().flatten() {
                let ty = self.value_ty(*value);
                let name = self.shape.target_names.get(value).copied();
                arg(&mut scope, *value, ty, self.names_of(*value, name), &mut args);
            }
        }
        let mut nodes = Vec::new();
        for (position, statement_index) in order.iter().enumerate() {
            let statement = body.statements.get(*statement_index).ok_or_else(|| self.bug("a canonical order past its body"))?;
            if let Some(node) = self.statement(statement, index_u32(position), &mut scope, &nodes)? {
                let node_index = index_u32(nodes.len());
                for (output, target) in targets_of(statement).iter().enumerate() {
                    let ty = node.outputs.get(output).map_or(TyId::ERROR, |value| value.ty);
                    scope.values.insert(target_id(target), (Source::Out { node: node_index, output: index_u32(output) }, ty));
                }
                scope.nodes.insert(statement.id, node_index);
                nodes.push(node);
            }
        }
        let results = self.results(holder, body, &scope)?;
        Ok(Region { origin, args, nodes, results })
    }

    fn results(&mut self, holder: Holder<'f>, body: &'f WrittenBody, scope: &Scope) -> Result<Vec<Use>, LowerError> {
        let mut results = Vec::new();
        match holder {
            Holder::Function => {
                for (output, port) in self.function.signature.outputs.iter().zip(self.outputs) {
                    let target = self.shape.binders.get(&output.id).copied().ok_or_else(|| self.bug("an output that nothing binds"))?;
                    results.push(self.result_of(target, port.ty, scope)?);
                }
            }
            Holder::Loop(_, structure) => {
                self.carry_results(structure.clauses.iter().map(|clause| &clause.kind), 0, scope, &mut results)?;
                let mut until = body.statements.iter().filter_map(|statement| match &statement.kind {
                    StatementKind::Until(value) => Some(value),
                    _ => None,
                });
                let condition = until.next().ok_or_else(|| self.bug("a loop with no `until`"))?;
                if until.next().is_some() {
                    return Err(self.bug("a loop with two `until`"));
                }
                results.push(self.use_of(condition, scope)?);
                self.collect_results(body, scope, &mut results)?;
            }
            Holder::For(_, structure) => {
                let clauses = structure.clauses.iter().filter(|clause| matches!(clause.kind, ClauseKind::In(_))).count();
                self.carry_results(structure.clauses.iter().map(|clause| &clause.kind), clauses, scope, &mut results)?;
                self.collect_results(body, scope, &mut results)?;
            }
            Holder::Arm { arm, .. } => {
                for value in &arm.yields.values {
                    results.push(self.use_of(value, scope)?);
                }
            }
        }
        Ok(results)
    }

    /// The next value of each carry, or the carry itself when no `next` binds it. `first` is the argument of the first carry.
    fn carry_results<'k>(
        &mut self,
        clauses: impl Iterator<Item = &'k ClauseKind>,
        first: usize,
        scope: &Scope,
        results: &mut Vec<Use>,
    ) -> Result<(), LowerError> {
        let mut position = first;
        for clause in clauses {
            let ClauseKind::Carry(carry) = clause else { continue };
            let ty = self.file.resolved.ty(carry.id);
            let result = match self.shape.nexts.get(&carry.id).copied() {
                Some(next) => self.result_of(next, ty, scope)?,
                None => Use { source: Source::Arg(index_u32(position)), fields: Vec::new(), convert: None, ty, origin: carry.id },
            };
            results.push(result);
            position += 1;
        }
        Ok(())
    }

    fn collect_results(&mut self, body: &'f WrittenBody, scope: &Scope, results: &mut Vec<Use>) -> Result<(), LowerError> {
        for statement in &body.statements {
            if let StatementKind::Collect(value) = &statement.kind {
                results.push(self.use_of(value, scope)?);
            }
        }
        Ok(())
    }

    /// What a target that binds a result delivers, in the type the result has.
    fn result_of(&mut self, target: Id, to: TyId, scope: &Scope) -> Result<Use, LowerError> {
        match self.shape.base(target) {
            Base::Def { id, fields } => self.use_of_value(id, &fields, to, target, scope),
            Base::Literal { operand, .. } => {
                let constant = self.lowering.literal(self.file, operand, to, &self.literals)?;
                Ok(Use { source: Source::Const(constant), fields: Vec::new(), convert: None, ty: to, origin: target })
            }
        }
    }

    // Uses.

    /// A value of the region, its fields read, then converted to `to`.
    fn use_of_value(&mut self, id: Id, fields: &[Name], to: TyId, origin: Id, scope: &Scope) -> Result<Use, LowerError> {
        let (source, mut ty) = scope.values.get(&id).cloned().ok_or_else(|| self.bug("a value read before it is defined in its region"))?;
        let mut indices = Vec::with_capacity(fields.len());
        for field in fields {
            let (index, field_ty) = self.field(ty, *field)?;
            indices.push(index);
            ty = field_ty;
        }
        let convert = self.convert(ty, to)?;
        Ok(Use { source, fields: indices, convert, ty: to, origin })
    }

    fn convert(&self, from: TyId, to: TyId) -> Result<Option<crate::Conversion>, LowerError> {
        if from == to || self.lowering.types.matches(from, to) && conversion(&self.lowering.types, from, to).is_none() {
            return Ok(None);
        }
        match conversion(&self.lowering.types, from, to) {
            Some(conversion) => Ok(Some(conversion)),
            None => Err(self.bug("a wire between types that do not convert")),
        }
    }

    /// A field of a struct or a `waveform`, by its index.
    fn field(&mut self, ty: TyId, field: Name) -> Result<(u32, TyId), LowerError> {
        let definition = self.lowering.definition(ty).ok_or_else(|| self.bug("a field of an opaque value"))?;
        match self.lowering.types.get(definition).clone() {
            Ty::Struct(fields) => {
                let index = fields.iter().position(|(name, _)| *name == field).ok_or_else(|| self.bug("a field that its struct does not have"))?;
                let field_ty = fields.get(index).map_or(TyId::ERROR, |(_, ty)| *ty);
                Ok((index_u32(index), field_ty))
            }
            Ty::Waveform(element) => {
                let (index, ty) = match self.lowering.names.resolve(field) {
                    "t0" => (0, Ty::Timestamp),
                    "dt" => (1, Ty::Duration),
                    "samples" => (2, Ty::Array { element, dimensions: 1 }),
                    _ => return Err(self.bug("a field that a waveform does not have")),
                };
                Ok((index, self.lowering.plain(ty, self.file)?))
            }
            _ => Err(self.bug("a field of a value that has none")),
        }
    }

    /// Whether an operand is a constant: a literal, a name of one, a variant, a function, or an array of those (spec 5.1).
    fn is_constant(&self, operand: &Operand) -> bool {
        let mut pending = vec![operand];
        while let Some(operand) = pending.pop() {
            let constant = match operand {
                Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) => true,
                Operand::Array(_, values) => {
                    pending.extend(values);
                    true
                }
                Operand::Path(path) => match self.file.resolved.of(path.id) {
                    Meaning::Value(value) => path.rest.is_empty() && self.literals.contains_key(&value),
                    Meaning::Variant { .. } | Meaning::Function { .. } | Meaning::Primitive { .. } => true,
                    _ => false,
                },
                Operand::Bracket(id, _) => match self.file.resolved.of(*id) {
                    Meaning::ArrayOfNumber | Meaning::ArrayOfFunction { .. } => true,
                    Meaning::ArrayOfValue(value) => self.literals.contains_key(&value),
                    _ => false,
                },
            };
            if !constant {
                return false;
            }
        }
        true
    }

    fn constant_use(&mut self, operand: &Operand, ty: TyId) -> Result<Use, LowerError> {
        let constant = self.lowering.literal(self.file, operand, ty, &self.literals)?;
        Ok(Use { source: Source::Const(constant), fields: Vec::new(), convert: None, ty, origin: operand.id() })
    }

    /// An operand as the input it reaches reads it.
    fn use_of(&mut self, operand: &Operand, scope: &Scope) -> Result<Use, LowerError> {
        let id = operand.id();
        if let Operand::Bracket(..) = operand {
            return self.bracket_use(operand, scope);
        }
        let to = self.use_ty(id);
        if self.is_constant(operand) {
            return self.constant_use(operand, to);
        }
        match operand {
            Operand::Path(path) => match self.shape.base_of_path(path, self.file) {
                Some(Base::Def { id: value, fields }) => self.use_of_value(value, &fields, to, id, scope),
                Some(Base::Literal { operand: literal, .. }) => {
                    let constant = self.lowering.literal(self.file, literal, to, &self.literals)?;
                    Ok(Use { source: Source::Const(constant), fields: Vec::new(), convert: None, ty: to, origin: id })
                }
                None => Err(self.bug("a path that reads no value")),
            },
            Operand::Array(_, values) => {
                let mut items = Vec::with_capacity(values.len());
                for value in values {
                    items.push(self.use_of(value, scope)?);
                }
                Ok(Use { source: Source::Array(items), fields: Vec::new(), convert: None, ty: to, origin: id })
            }
            _ => Err(self.bug("an operand that is neither a wire nor a constant")),
        }
    }

    /// `[x]` for a parameter of kind value: an array of one wire, or of one constant. What `check` recorded converts its element.
    fn bracket_use(&mut self, operand: &Operand, scope: &Scope) -> Result<Use, LowerError> {
        let id = operand.id();
        let ty = self.file.resolved.ty(id);
        if self.is_constant(operand) {
            return self.constant_use(operand, ty);
        }
        let Meaning::ArrayOfValue(value) = self.file.resolved.of(id) else { return Err(self.bug("a bracket that holds no value")) };
        let Base::Def { id: base, fields } = self.shape.base(value) else { return Err(self.bug("a bracket of a constant taken for a wire")) };
        let element = match self.coercions.get(&id) {
            Some((_, to)) => *to,
            None => self.lowering.element_of(ty, self.file)?,
        };
        let item = self.use_of_value(base, &fields, element, id, scope)?;
        Ok(Use { source: Source::Array(vec![item]), fields: Vec::new(), convert: None, ty, origin: id })
    }

    // Nodes.

    fn statement(&mut self, statement: &'f Statement, position: u32, scope: &mut Scope, nodes: &[Node]) -> Result<Option<Node>, LowerError> {
        let targets = targets_of(statement);
        let first_target = targets.first().map(target_id);
        let built = match &statement.kind {
            StatementKind::Collect(_) | StatementKind::Until(_) => return Ok(None),
            StatementKind::Call(call) => self.call(call, statement, scope)?,
            StatementKind::Binding { value, .. } => match value {
                Rhs::Call(call) => self.call(call, statement, scope)?,
                Rhs::Struct(literal) => self.construction(literal, statement, scope)?,
                Rhs::Operand(operand) => {
                    let Some(target) = first_target else { return Err(self.bug("a line of one value with no target")) };
                    if self.shape.aliases.contains_key(&target)
                        || (self.shape.literals.contains_key(&target) && !self.shape.constant_nodes.contains(&target))
                    {
                        return Ok(None);
                    }
                    self.operand_node(operand, target, scope)?
                }
                Rhs::Match(branching) => self.branching(branching, statement, scope, false)?,
                Rhs::Handle(branching) => self.branching(branching, statement, scope, true)?,
                Rhs::Loop(structure) => {
                    let mut inputs = Vec::new();
                    let mut carries = 0;
                    for clause in &structure.clauses {
                        if let ClauseKind::Carry(carry) = &clause.kind {
                            inputs.push(self.use_of(&carry.value, scope)?);
                            carries += 1;
                        }
                    }
                    self.context_inputs(statement, scope, &mut inputs)?;
                    let region = self.inner_region(statement.id, 0)?;
                    let collects = count_collects(&structure.body);
                    let outputs = gives_nothing(carries, collects);
                    let kind = NodeKind::Loop { continue_on_error: structure.continue_on_error, carries, collects, region };
                    Built { kind, inputs, skip: Skip::Never, outputs }
                }
                Rhs::For(structure) => {
                    let mut inputs = Vec::new();
                    let mut clauses = Vec::new();
                    for clause in &structure.clauses {
                        if let ClauseKind::In(clause) = &clause.kind {
                            inputs.push(self.use_of(&clause.source, scope)?);
                            match &clause.end {
                                Some(end) => {
                                    inputs.push(self.use_of(end, scope)?);
                                    clauses.push(Clause::Range);
                                }
                                None => clauses.push(Clause::Each),
                            }
                        }
                    }
                    let mut carries = 0;
                    for clause in &structure.clauses {
                        if let ClauseKind::Carry(carry) = &clause.kind {
                            inputs.push(self.use_of(&carry.value, scope)?);
                            carries += 1;
                        }
                    }
                    self.context_inputs(statement, scope, &mut inputs)?;
                    let region = self.inner_region(statement.id, 0)?;
                    let collects = count_collects(&structure.body);
                    let outputs = gives_nothing(carries, collects);
                    let kind = NodeKind::For { parallel: structure.parallel, clauses, carries, collects, region };
                    Built { kind, inputs, skip: Skip::Never, outputs }
                }
            },
        };
        let outputs = match built.outputs {
            Some(outputs) => outputs,
            None if targets.is_empty() => self.file.resolved.outputs_of(statement.id).iter().map(|ty| Value { ty: *ty, names: Vec::new() }).collect(),
            None => targets.iter().map(|target| self.output_value(target)).collect(),
        };
        let mut after = Vec::new();
        for wait in self.shape.orders.get(&statement.id).into_iter().flatten() {
            after.push(match wait {
                Wait::Value(value) => {
                    let ty = scope.values.get(value).map_or(TyId::ERROR, |(_, ty)| *ty);
                    Order::Value(self.use_of_value(*value, &[], ty, statement.id, scope)?)
                }
                Wait::Node(line) => match scope.nodes.get(line) {
                    Some(node) if (*node as usize) < nodes.len() => Order::Node(*node),
                    _ => return Err(self.bug("an order on a node that comes later or makes no node")),
                },
            });
        }
        let name = self.node_name(statement);
        Ok(Some(Node { kind: built.kind, inputs: built.inputs, outputs, after, position, origin: statement.id, name, skip: built.skip }))
    }

    fn output_value(&mut self, target: &Target) -> Value {
        match target {
            Target::Name(id, name) => Value { ty: self.value_ty(*id), names: self.names_of(*id, Some(*name)) },
            Target::Next(id, name) => {
                let text = format!("next.{}", self.lowering.names.resolve(*name));
                Value { ty: self.value_ty(*id), names: vec![self.lowering.names.intern(&text)] }
            }
            Target::Skip(id) => Value { ty: self.value_ty(*id), names: Vec::new() },
        }
    }

    /// As the trace names a node: its first output, its label, or `@12` (`docs/commands.md`, 8).
    fn node_name(&mut self, statement: &Statement) -> Name {
        if let Some(name) = targets_of(statement).iter().find_map(|target| match target {
            Target::Name(_, name) => Some(*name),
            _ => None,
        }) {
            return name;
        }
        let text = match &statement.label {
            Some(label) => format!("#{}", self.lowering.names.resolve(label.name)),
            None => format!("@{}", self.file.source_map.span(statement.id).map_or(0, |span| span.start.line)),
        };
        self.lowering.names.intern(&text)
    }

    fn context_inputs(&mut self, statement: &Statement, scope: &Scope, inputs: &mut Vec<Use>) -> Result<(), LowerError> {
        for value in self.shape.context.get(&statement.id).into_iter().flatten() {
            let ty = scope.values.get(value).map_or(TyId::ERROR, |(_, ty)| *ty);
            inputs.push(self.use_of_value(*value, &[], ty, statement.id, scope)?);
        }
        Ok(())
    }

    fn inner_region(&mut self, statement: Id, which: usize) -> Result<Region, LowerError> {
        let body =
            self.shape.inner.get(&statement).and_then(|bodies| bodies.get(which)).copied().ok_or_else(|| self.bug("a structure with no body"))?;
        self.take(body)
    }

    fn operand_node(&mut self, operand: &'f Operand, target: Id, scope: &Scope) -> Result<Built, LowerError> {
        let own = self.value_ty(target);
        if self.shape.built_arrays.contains(&target) {
            let Operand::Array(_, values) = operand else { return Err(self.bug("an array node that is no array")) };
            let mut inputs = Vec::with_capacity(values.len());
            for value in values {
                inputs.push(self.use_of(value, scope)?);
            }
            return Ok(Built { kind: NodeKind::BuildArray, inputs, skip: Skip::Never, outputs: None });
        }
        if let Operand::Path(path) = operand
            && let Meaning::Variant { variant, .. } = self.file.resolved.of(path.id)
        {
            let (index, _) = self.lowering.variant_index(own, variant).ok_or_else(|| self.bug("a variant of no enumeration"))?;
            return Ok(Built { kind: NodeKind::Construct { ty: own, variant: Some(index) }, inputs: Vec::new(), skip: Skip::Never, outputs: None });
        }
        // A named constant, or a function as a value: one node, its input the constant in its own type.
        let input = self.constant_use(operand, own)?;
        Ok(Built { kind: NodeKind::Constant, inputs: vec![input], skip: Skip::Never, outputs: None })
    }

    fn construction(&mut self, literal: &'f StructLit, statement: &Statement, scope: &Scope) -> Result<Built, LowerError> {
        let ty = targets_of(statement).first().map_or(TyId::ERROR, |target| self.value_ty(target_id(target)));
        let names: Vec<Name> = match self.file.resolved.of(literal.id) {
            Meaning::Waveform => ["t0", "dt", "samples"].iter().map(|name| self.lowering.names.intern(name)).collect(),
            Meaning::Type(_) => {
                let definition = self.lowering.definition(ty).ok_or_else(|| self.bug("a struct of an opaque type"))?;
                match self.lowering.types.get(definition) {
                    Ty::Struct(fields) => fields.iter().map(|(name, _)| *name).collect(),
                    Ty::Waveform(_) => ["t0", "dt", "samples"].iter().map(|name| self.lowering.names.intern(name)).collect(),
                    _ => return Err(self.bug("a construction of what is not a struct")),
                }
            }
            _ => return Err(self.bug("a construction of nothing")),
        };
        let mut inputs = Vec::with_capacity(names.len());
        for name in names {
            let field = literal.fields.iter().find(|field| field.name == name).ok_or_else(|| self.bug("a construction without one of its fields"))?;
            inputs.push(self.use_of(&field.value, scope)?);
        }
        Ok(Built { kind: NodeKind::Construct { ty, variant: None }, inputs, skip: Skip::Never, outputs: None })
    }

    fn branching(&mut self, branching: &'f Branching, statement: &Statement, scope: &Scope, handle: bool) -> Result<Built, LowerError> {
        let mut inputs = vec![self.use_of(&branching.selector, scope)?];
        self.context_inputs(statement, scope, &mut inputs)?;
        let selector = self.use_ty(branching.selector.id());
        let mut arms = Vec::with_capacity(branching.arms.len());
        let mut last = Vec::new();
        for (which, arm) in branching.arms.iter().enumerate() {
            let patterns = self.patterns(arm, selector)?;
            let region = self.inner_region(statement.id, which)?;
            let wildcard = patterns.iter().any(|pattern| matches!(pattern, Pattern::Wildcard { .. }));
            let lowered = Arm { patterns, origin: arm.id, region };
            if wildcard { last.push(lowered) } else { arms.push(lowered) }
        }
        // `_` is tried last, wherever it is written (spec 6.1). In a `handle` the most specific arm wins (7.3): `ok`, a code,
        // a category, `a.b.*` before `a.*`, then `_`; arms of one rank do not overlap, and keep their written order.
        arms.extend(last);
        if handle {
            arms.sort_by_key(|arm| match arm.patterns.first() {
                Some(Pattern::Ok) => (0, 0),
                Some(Pattern::Code { .. }) => (1, 0),
                Some(Pattern::Category { wildcard: false, .. }) => (2, 0),
                Some(Pattern::Category { category, wildcard: true, .. }) => (3, usize::MAX - category.split('.').count()),
                _ => (4, 0),
            });
        }
        let kind = if handle { NodeKind::Handle { arms } } else { NodeKind::Match { arms } };
        Ok(Built { kind, inputs, skip: Skip::Never, outputs: None })
    }

    fn patterns(&mut self, arm: &'f WrittenArm, selector: TyId) -> Result<Vec<crate::Pattern>, LowerError> {
        let mut patterns = Vec::new();
        for pattern in arm.patterns() {
            patterns.push(match pattern {
                WrittenPattern::Literal(operand) => {
                    let ty = self.file.resolved.ty(operand.id());
                    let ty = if ty == TyId::ERROR { selector } else { ty };
                    Pattern::Literal(self.lowering.literal(self.file, operand, ty, &self.literals)?)
                }
                WrittenPattern::Range { low, high, inclusive, .. } => {
                    Pattern::Range { low: self.bound(*low)?, high: self.bound(*high)?, inclusive: *inclusive }
                }
                WrittenPattern::Variant { path, bindings, .. } => {
                    let Meaning::Variant { ty, variant } = self.file.resolved.of(path.id) else {
                        return Err(self.bug("a variant pattern of no variant"));
                    };
                    let (index, _) = self.lowering.variant_index(ty, variant).ok_or_else(|| self.bug("a variant of no enumeration"))?;
                    Pattern::Variant { variant: index, binds: index_u32(bindings.len()) }
                }
                WrittenPattern::Wildcard { binding, .. } => Pattern::Wildcard { binds: binding.is_some() },
                WrittenPattern::Ok { .. } => Pattern::Ok,
                WrittenPattern::Category { path, wildcard, binding, .. } => {
                    Pattern::Category { category: self.path_text(path).into(), wildcard: *wildcard, binds: binding.is_some() }
                }
                WrittenPattern::Code { code, binding, .. } => Pattern::Code { code: self.bound(*code)?, binds: binding.is_some() },
            });
        }
        Ok(patterns)
    }

    fn bound(&self, bound: labcraft_graph::IntBound) -> Result<i128, LowerError> {
        let digits: String = self.lowering.names.resolve(bound.digits).chars().filter(|c| *c != '_').collect();
        let magnitude: i128 = digits.parse().map_err(|_| self.bug("a bound that is no integer"))?;
        Ok(if bound.negative { -magnitude } else { magnitude })
    }

    fn path_text(&self, path: &Path) -> String {
        path.segments().map(|name| self.lowering.names.resolve(name)).collect::<Vec<_>>().join(".")
    }

    // Calls.

    fn call(&mut self, call: &'f Call, statement: &Statement, scope: &Scope) -> Result<Built, LowerError> {
        let bound = self.file.resolved.call(call.id).cloned().ok_or_else(|| self.bug("a call that `check` did not bind"))?;
        match bound.callee {
            Meaning::Variant { variant, .. } => {
                let ty = targets_of(statement).first().map_or(TyId::ERROR, |target| self.value_ty(target_id(target)));
                let (index, values) = self.lowering.variant_index(ty, variant).ok_or_else(|| self.bug("a variant of no enumeration"))?;
                let mut inputs: Vec<Option<Use>> = vec![None; values.len()];
                for (arg, terminal) in call.args.iter().zip(bound.terminals.iter()) {
                    if let (Arg::Value { value, .. }, Terminal::Param(position)) = (arg, terminal) {
                        let lowered = self.use_of(value, scope)?;
                        if let Some(slot) = inputs.get_mut(*position as usize) {
                            *slot = Some(lowered);
                        }
                    }
                }
                let inputs = inputs.into_iter().collect::<Option<Vec<Use>>>().ok_or_else(|| self.bug("a variant without one of its values"))?;
                Ok(Built { kind: NodeKind::Construct { ty, variant: Some(index) }, inputs, skip: Skip::Never, outputs: None })
            }
            Meaning::Function { library, name } => self.function_call(call, &bound, library, name, scope),
            Meaning::Primitive { library, name } => self.primitive_call(call, statement, &bound, library, name, scope),
            _ => Err(self.bug("a call of what is not a function")),
        }
    }

    fn function_call(&mut self, call: &'f Call, bound: &ResolvedCall, library: u32, name: Name, scope: &Scope) -> Result<Built, LowerError> {
        let function = self.lowering.function_id(library, name, self.file)?;
        let (ports, outputs) = self.lowering.signature(function).cloned().ok_or_else(|| self.bug("a function with no signature"))?;
        let external = self.lowering.declared(library, name).is_some_and(|declared| declared.external);
        let mut inputs = Vec::with_capacity(ports.len());
        let mut variadic = None;
        let by_position = wired(call, bound, ports.len());
        for (position, port) in ports.iter().enumerate() {
            let wired = by_position.get(position).cloned().unwrap_or_default();
            if port.variadic {
                let start = index_u32(inputs.len());
                for value in &wired {
                    inputs.push(self.use_of(value, scope)?);
                }
                variadic = Some(Variadic { start, count: index_u32(wired.len()) });
                continue;
            }
            match (wired.first(), port.default) {
                (Some(value), _) => inputs.push(self.use_of(value, scope)?),
                (None, Some(default)) => {
                    inputs.push(Use { source: Source::Const(default), fields: Vec::new(), convert: None, ty: port.ty, origin: call.id })
                }
                (None, None) => return Err(self.bug("an input with no wire and no default")),
            }
        }
        if !external {
            return Ok(Built { kind: NodeKind::Call { function, variadic }, inputs, skip: Skip::Runs, outputs: None });
        }
        let always = self.lowering.checked.libraries.get(library as usize).is_some_and(|library| library.always.contains(&name));
        let input_names: Vec<Name> = ports.iter().map(|port| port.name).collect();
        let output_names: Vec<Name> = outputs.iter().map(|port| port.name).collect();
        let output_tys: Vec<TyId> = outputs.iter().map(|port| port.ty).collect();
        let skip = self.on_error(&input_names, &inputs, variadic, &output_names, &output_tys, always)?;
        Ok(Built { kind: NodeKind::Native { function, variadic }, inputs, skip, outputs: None })
    }

    /// Spec 7.2: the input named `err`, of type `error`, and what each output takes when the node does not run.
    fn on_error(
        &mut self,
        input_names: &[Name],
        inputs: &[Use],
        variadic: Option<Variadic>,
        output_names: &[Name],
        output_tys: &[TyId],
        always: bool,
    ) -> Result<Skip, LowerError> {
        // The input of a parameter, past the arguments of a variadic one.
        let input_of = |position: usize| -> Option<usize> {
            match variadic {
                Some(Variadic { start, count }) if position > start as usize => Some(position + count as usize - 1),
                Some(Variadic { start, .. }) if position == start as usize => None,
                _ => Some(position),
            }
        };
        let error_names = self.lowering.names.find("err");
        let err = input_names.iter().enumerate().find_map(|(position, name)| {
            let input = input_of(position)?;
            (Some(*name) == error_names && inputs.get(input).is_some_and(|input| matches!(self.lowering.types.get(input.ty), Ty::ErrorValue)))
                .then_some(input)
        });
        let Some(err) = err else { return Ok(Skip::Never) };
        let mut defaults = Vec::with_capacity(output_tys.len());
        for ty in output_tys {
            defaults.push(self.lowering.default_of(*ty, self.file)?);
        }
        let through = output_names
            .iter()
            .zip(output_tys)
            .map(|(name, ty)| {
                let position = input_names.iter().position(|input| input == name)?;
                let input = input_of(position)?;
                inputs.get(input).filter(|input| self.lowering.types.matches(input.ty, *ty)).map(|_| index_u32(input))
            })
            .collect();
        Ok(Skip::OnError { err: index_u32(err), always, defaults, through })
    }

    fn primitive_call(
        &mut self,
        call: &'f Call,
        statement: &Statement,
        bound: &ResolvedCall,
        library: PrimitiveLibrary,
        name: Name,
        scope: &Scope,
    ) -> Result<Built, LowerError> {
        let text = self.lowering.names.resolve(name).to_owned();
        let signature = primitive(library, &text).ok_or_else(|| self.bug("a primitive that the table does not have"))?;
        if library == PrimitiveLibrary::Core && text == "call" {
            return self.call_by_reference(call, bound, name, scope);
        }
        if library == PrimitiveLibrary::String && text == "format" {
            return self.format(call, bound, name, scope);
        }
        let mut inputs = Vec::new();
        let mut statics = Vec::new();
        let mut variadic = None;
        let mut input_names = Vec::new();
        let by_position = wired(call, bound, signature.params.len());
        for (position, param) in signature.params.iter().enumerate() {
            let wired = by_position.get(position).cloned().unwrap_or_default();
            match param.kind {
                ParamKind::Value => {
                    input_names.push(self.lowering.names.intern(param.name));
                    if param.variadic {
                        let start = index_u32(inputs.len());
                        for value in &wired {
                            inputs.push(self.use_of(value, scope)?);
                        }
                        variadic = Some(Variadic { start, count: index_u32(wired.len()) });
                        continue;
                    }
                    match (wired.first(), param.default) {
                        (Some(value), _) => inputs.push(self.use_of(value, scope)?),
                        (None, Some(default)) => {
                            let input = self.primitive_default(default, param.ty, call.id)?;
                            inputs.push(input);
                        }
                        (None, None) => return Err(self.bug("an input of a primitive with no wire and no default")),
                    }
                }
                ParamKind::Unit => {
                    let Some(Operand::Bracket(_, unit)) = wired.first() else { return Err(self.bug("a unit argument that is no bracket")) };
                    statics.push(Static::Unit(unit_of(unit, self.lowering.names).map_err(|_| self.bug("a unit that does not read"))?));
                }
                ParamKind::Category => {
                    let Some(Operand::Path(path)) = wired.first() else { return Err(self.bug("a category argument that is no path")) };
                    statics.push(Static::Category(self.path_text(path).into()));
                }
                ParamKind::Element => {
                    let Some(Operand::Path(path)) = wired.first() else { return Err(self.bug("an element argument that is no path")) };
                    let Meaning::Element(item) = self.file.resolved.of(path.id) else { return Err(self.bug("an element argument of no element")) };
                    let element = self.element_ids.get(&item).copied().ok_or_else(|| self.bug("an element of no panel"))?;
                    statics.push(Static::Element(element));
                }
            }
        }
        let output_names: Vec<Name> = signature.outputs.unwrap_or(&[]).iter().map(|output| self.lowering.names.intern(output)).collect();
        // The types of its outputs are what its line binds, which `check` resolved.
        let output_tys = self.call_output_tys(statement);
        let skip = if output_tys.len() == output_names.len() {
            self.on_error(&input_names, &inputs, variadic, &output_names, &output_tys, false)?
        } else {
            Skip::Never
        };
        Ok(Built { kind: NodeKind::Primitive { library, name, statics, variadic }, inputs, skip, outputs: None })
    }

    /// The types of the outputs of the node of this line.
    fn call_output_tys(&self, statement: &Statement) -> Vec<TyId> {
        let targets = targets_of(statement);
        if targets.is_empty() {
            return self.file.resolved.outputs_of(statement.id).to_vec();
        }
        targets.iter().map(|target| self.value_ty(target_id(target))).collect()
    }

    fn primitive_default(&mut self, default: PrimitiveDefault, form: PrimitiveTy, origin: Id) -> Result<Use, LowerError> {
        let (ty, value) = match default {
            PrimitiveDefault::NoError => (self.lowering.plain(Ty::ErrorValue, self.file)?, ConstValue::NoError),
            PrimitiveDefault::EmptyText => (self.lowering.plain(Ty::String, self.file)?, ConstValue::Text("".into())),
            PrimitiveDefault::Zero => match form {
                PrimitiveTy::I32 => (self.lowering.number(NumberKind::I32, self.file)?, ConstValue::Int(0)),
                PrimitiveTy::F32 => (self.lowering.number(NumberKind::F32, self.file)?, ConstValue::Float(0.0)),
                PrimitiveTy::F64 => (self.lowering.number(NumberKind::F64, self.file)?, ConstValue::Float(0.0)),
                _ => return Err(self.bug("a default of zero for what is not a number")),
            },
            PrimitiveDefault::ToTheEnd => (self.lowering.number(NumberKind::I32, self.file)?, ConstValue::ToTheEnd),
        };
        let constant = self.lowering.intern(ty, value);
        Ok(Use { source: Source::Const(constant), fields: Vec::new(), convert: None, ty, origin })
    }

    /// `string.format`: its template is data of the node, and its inputs are the fields of the template, in order (5.4).
    fn format(&mut self, call: &'f Call, bound: &ResolvedCall, name: Name, scope: &Scope) -> Result<Built, LowerError> {
        let Some(Operand::Text(_, template)) = first_wired(call, bound, 0) else { return Err(self.bug("a template that is no text")) };
        let template = read_escapes(template).into_owned();
        // Each field by name, once: a template can have a million fields.
        let mut by_name: HashMap<&str, &'f Operand> = HashMap::new();
        for (arg, terminal) in call.args.iter().zip(bound.terminals.iter()) {
            if let (Arg::Value { name: Some(name), value, .. }, Terminal::Free) = (arg, terminal) {
                by_name.entry(self.lowering.names.resolve(*name)).or_insert(value);
            }
        }
        let mut wires = Vec::new();
        for field in template_fields(&template) {
            wires.push(by_name.get(field.as_str()).copied().ok_or_else(|| self.bug("a field of a template with no wire"))?);
        }
        let mut inputs = Vec::with_capacity(wires.len());
        for value in wires {
            inputs.push(self.use_of(value, scope)?);
        }
        let statics = vec![Static::Template(template.into())];
        Ok(Built {
            kind: NodeKind::Primitive { library: PrimitiveLibrary::String, name, statics, variadic: None },
            inputs,
            skip: Skip::Never,
            outputs: None,
        })
    }

    /// `core.call`: the function, then its arguments in the order of its type, which has no defaults (4).
    fn call_by_reference(&mut self, call: &'f Call, bound: &ResolvedCall, name: Name, scope: &Scope) -> Result<Built, LowerError> {
        let Some(reference) = first_wired(call, bound, 0) else { return Err(self.bug("`core.call` with no function")) };
        let function = self.use_of(reference, scope)?;
        let definition = self.lowering.definition(function.ty).ok_or_else(|| self.bug("`core.call` of an opaque value"))?;
        let Ty::Function { params, .. } = self.lowering.types.get(definition).clone() else {
            return Err(self.bug("`core.call` of what is no function"));
        };
        let mut wired: Vec<Option<Use>> = vec![None; params.len()];
        let mut next = 0_usize;
        for (arg, terminal) in call.args.iter().zip(bound.terminals.iter()) {
            let (Arg::Value { name, value, .. }, Terminal::Free) = (arg, terminal) else { continue };
            let position = match name {
                None => {
                    next += 1;
                    next - 1
                }
                Some(name) => {
                    params.iter().position(|param| param.name == Some(*name)).ok_or_else(|| self.bug("an argument of `core.call` of no input"))?
                }
            };
            let lowered = self.use_of(value, scope)?;
            let slot = wired.get_mut(position).ok_or_else(|| self.bug("too many arguments for `core.call`"))?;
            *slot = Some(lowered);
        }
        let mut inputs = vec![function];
        inputs.extend(wired.into_iter().collect::<Option<Vec<Use>>>().ok_or_else(|| self.bug("an input of `core.call` with no wire"))?);
        // Whatever names the type gives: the function it runs decides, by its own `err` (7.2).
        let skip = Skip::ByCallee;
        Ok(Built {
            kind: NodeKind::Primitive { library: PrimitiveLibrary::Core, name, statics: Vec::new(), variadic: None },
            inputs,
            skip,
            outputs: None,
        })
    }
}

/// What a line makes, before its outputs and orders.
struct Built {
    kind: NodeKind,
    inputs: Vec<Use>,
    skip: Skip,
    outputs: Option<Vec<Value>>,
}

/// The arguments wired to each parameter, by its position, in written order: one walk of the call.
fn wired<'c>(call: &'c Call, bound: &ResolvedCall, params: usize) -> Vec<Vec<&'c Operand>> {
    let mut wired: Vec<Vec<&Operand>> = vec![Vec::new(); params];
    for (arg, terminal) in call.args.iter().zip(bound.terminals.iter()) {
        if let (Arg::Value { value, .. }, Terminal::Param(at)) = (arg, terminal)
            && let Some(slot) = wired.get_mut(*at as usize)
        {
            slot.push(value);
        }
    }
    wired
}

/// The first argument wired to the parameter at this position.
fn first_wired<'c>(call: &'c Call, bound: &ResolvedCall, position: usize) -> Option<&'c Operand> {
    call.args.iter().zip(bound.terminals.iter()).find_map(|(arg, terminal)| match (arg, terminal) {
        (Arg::Value { value, .. }, Terminal::Param(at)) if *at as usize == position => Some(value),
        _ => None,
    })
}

/// A loop with neither a carry nor a `collect` gives nothing, and its line binds it to `_` (6.2).
fn gives_nothing(carries: u32, collects: u32) -> Option<Vec<Value>> {
    (carries == 0 && collects == 0).then(Vec::new)
}

fn count_collects(body: &WrittenBody) -> u32 {
    index_u32(body.statements.iter().filter(|statement| matches!(statement.kind, StatementKind::Collect(_))).count())
}

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
