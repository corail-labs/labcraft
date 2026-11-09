//! The invariants of `docs/ir.md`, section 7. A violation is a bug of lowering. Regions are walked with an explicit stack.

use labcraft_types::{Ty, TyId};

use crate::{
    Body, Clause, ConstValue, Conversion, Function, Node, NodeKind, Order, Pattern, PortOf, Program, Region, Skip, Source, Static, Use, VERSION,
};

/// One invariant that a program breaks, in one of its functions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// By its index in `functions`, `None` for what is outside them.
    pub function: Option<u32>,
    pub message: String,
}

/// Every violation of the invariants of `docs/ir.md`, section 7, in the order found.
pub fn verify(program: &Program) -> Vec<Violation> {
    let mut checker = Checker { program, violations: Vec::new(), function: None };
    if program.version != VERSION {
        checker.say(format!("version {} where this crate makes {VERSION}", program.version));
    }
    checker.constants();
    for (index, function) in program.functions.iter().enumerate() {
        checker.function = Some(u32::try_from(index).unwrap_or(u32::MAX));
        checker.function_of(function);
    }
    checker.violations
}

struct Checker<'p> {
    program: &'p Program,
    violations: Vec<Violation>,
    function: Option<u32>,
}

/// What a use may name, in the region it is read in.
#[derive(Clone, Copy)]
struct Within {
    args: usize,
    /// The nodes before the one being read for.
    nodes: usize,
}

impl Checker<'_> {
    fn say(&mut self, message: String) {
        self.violations.push(Violation { function: self.function, message });
    }

    /// No type of a program holds `Ty::Error`, at any depth.
    fn known_ty(&mut self, ty: TyId, what: &str) {
        if self.program.types.has_error(ty) {
            self.say(format!("{what} has no type"));
        }
    }

    /// Equal, the names of function arguments aside (spec 4). A type that holds an error matches nothing here.
    fn same(&self, a: TyId, b: TyId) -> bool {
        let types = &self.program.types;
        a == b || (!types.has_error(a) && !types.has_error(b) && types.matches(a, b))
    }

    fn definition(&self, ty: TyId) -> TyId {
        self.program.definitions.get(&ty).copied().flatten().unwrap_or(ty)
    }

    /// What a use delivers is what it reads, its fields read, then converted as its conversion says.
    fn use_types(&mut self, uses: &[Use], region: &Region) {
        let mut pending: Vec<&Use> = uses.iter().collect();
        while let Some(used) = pending.pop() {
            let source = match &used.source {
                Source::Arg(index) => region.args.get(*index as usize).map(|arg| arg.ty),
                Source::Out { node, output } => {
                    region.nodes.get(*node as usize).and_then(|node| node.outputs.get(*output as usize)).map(|value| value.ty)
                }
                Source::Const(constant) => self.program.constants.get(constant.0 as usize).map(|constant| constant.ty),
                Source::Array(items) => {
                    for item in items {
                        if !self.is_element(used.ty, item.ty) {
                            self.say("an array of wires whose type does not hold an item".to_owned());
                        }
                    }
                    pending.extend(items);
                    continue;
                }
            };
            let Some(mut ty) = source else { continue };
            let mut known = true;
            for field in &used.fields {
                match self.program.types.get(self.definition(ty)) {
                    Ty::Struct(fields) => match fields.get(*field as usize) {
                        Some((_, field_ty)) => ty = *field_ty,
                        None => {
                            self.say(format!("a use of field {field}, which its struct does not have"));
                            known = false;
                            break;
                        }
                    },
                    // `t0`, `dt`, `samples`: their types are not in the table unless a program uses them.
                    Ty::Waveform(_) if *field < 3 => {
                        known = false;
                        break;
                    }
                    _ => {
                        self.say(format!("a use of field {field} of a value that has no such field"));
                        known = false;
                        break;
                    }
                }
            }
            if !known {
                continue;
            }
            let (from, to) = (self.program.types.get(ty), self.program.types.get(used.ty));
            let fits = match &used.convert {
                None => self.same(ty, used.ty),
                Some(Conversion::Number { to: kind, .. }) => {
                    matches!((from, to), (Ty::Number { .. }, Ty::Number { kind: to_kind, .. }) if to_kind == kind)
                }
                Some(Conversion::ToDuration { .. }) => matches!((from, to), (Ty::Number { .. }, Ty::Duration)),
                Some(Conversion::FromDuration { to: kind, .. }) => {
                    matches!((from, to), (Ty::Duration, Ty::Number { kind: to_kind, .. }) if to_kind == kind)
                }
            };
            if !fits {
                self.say("a use whose type is not what it reads, through its fields and its conversion".to_owned());
            }
        }
    }

    /// Whether `item` is one level of the array `array`: `f64` of `[f64]`, `[f64]` of `[f64; 2]`.
    fn is_element(&self, array: TyId, item: TyId) -> bool {
        match (self.program.types.get(array), self.program.types.get(item)) {
            (Ty::Array { element, dimensions: 1 }, _) => self.same(*element, item),
            (Ty::Array { element, dimensions }, Ty::Array { element: inner, dimensions: inner_dimensions }) => {
                *inner_dimensions == dimensions - 1 && self.same(*element, *inner)
            }
            _ => false,
        }
    }

    fn is_bool(&self, ty: TyId) -> bool {
        matches!(self.program.types.get(ty), Ty::Bool)
    }

    /// The results of a loop or a `for`: the next carries, the condition of a loop, then the element of each collect.
    fn loop_results(&mut self, node: &Node, region: &Region, carries: &[TyId], until: bool, what: &str) {
        let collects = node.outputs.len().saturating_sub(carries.len());
        let expected = carries.len() + usize::from(until) + collects;
        if region.results.len() != expected {
            self.say(format!("{what} gives {} results where {expected} are expected", region.results.len()));
            return;
        }
        let mut results = region.results.iter();
        // The carries first, so that `zip` takes no result past them.
        for (carry, result) in carries.iter().zip(results.by_ref()) {
            if !self.same(result.ty, *carry) {
                self.say(format!("the next value of a carry of {what} has another type than the carry"));
            }
        }
        if until && !results.next().is_some_and(|condition| self.is_bool(condition.ty)) {
            self.say(format!("the condition of {what} is no `bool`"));
        }
        for (result, output) in results.zip(node.outputs.iter().skip(carries.len())) {
            if !self.is_element(output.ty, result.ty) {
                self.say(format!("a collect of {what} whose array does not hold its type"));
            }
        }
    }

    fn constant(&mut self, id: crate::ConstId, what: &str) {
        if self.program.constants.get(id.0 as usize).is_none() {
            self.say(format!("{what} names constant {}, which does not exist", id.0));
        }
    }

    fn constants(&mut self) {
        for (index, constant) in self.program.constants.iter().enumerate() {
            self.known_ty(constant.ty, &format!("constant {index}"));
            let items: Vec<crate::ConstId> = match &constant.value {
                ConstValue::Array(items) | ConstValue::Struct(items) | ConstValue::Variant { values: items, .. } => items.clone(),
                _ => Vec::new(),
            };
            for item in items {
                // A constant names only constants made before it: the table has no cycle.
                if item.0 as usize >= index {
                    self.say(format!("constant {index} holds constant {}, which comes after it", item.0));
                }
            }
            if let ConstValue::Function(function) = constant.value
                && self.program.functions.get(function.0 as usize).is_none()
            {
                self.say(format!("constant {index} names function {}, which does not exist", function.0));
            }
            if let ConstValue::Primitive { skip, .. } = &constant.value {
                let ty = self.definition(constant.ty);
                let signature = match self.program.types.get(ty) {
                    Ty::Function { params, outputs } => Some((params.iter().map(|param| param.ty).collect::<Vec<TyId>>(), outputs.clone())),
                    _ => None,
                };
                match signature {
                    Some((params, outputs)) => self.skip_against(skip, &params, &outputs, "a primitive as a value"),
                    None => self.say(format!("constant {index}, a primitive, has no function type")),
                }
                if *skip == Skip::Runs {
                    self.say(format!("constant {index}, a primitive, runs as a LabCraft function"));
                }
            }
        }
    }

    fn function_of(&mut self, function: &Function) {
        for port in function.inputs.iter().chain(&function.outputs) {
            self.known_ty(port.ty, "a port");
            if let Some(default) = port.default {
                self.constant(default, "a port");
            }
        }
        for element in &function.elements {
            if let Some(ty) = element.ty {
                self.known_ty(ty, "an element");
            }
            let port_exists = match element.port {
                Some(PortOf::Input(input)) => (input as usize) < function.inputs.len(),
                Some(PortOf::Output(output)) => (output as usize) < function.outputs.len(),
                None => true,
            };
            if !port_exists {
                self.say("an element of a port that its function does not have".to_owned());
            }
            if let Some(default) = element.default {
                self.constant(default, "an element");
            }
        }
        let region = match &function.body {
            Body::Region(region) => region,
            Body::Native { skip, .. } => {
                let inputs: Vec<TyId> = function.inputs.iter().map(|port| port.ty).collect();
                let outputs: Vec<TyId> = function.outputs.iter().map(|port| port.ty).collect();
                self.skip_against(skip, &inputs, &outputs, "a native body");
                if *skip == Skip::Runs {
                    self.say("a native body that runs as a LabCraft function".to_owned());
                }
                return;
            }
        };
        if region.args.len() != function.inputs.len() {
            self.say(format!("the region of the function has {} arguments for {} inputs", region.args.len(), function.inputs.len()));
        }
        for (arg, port) in region.args.iter().zip(&function.inputs) {
            if !self.same(arg.ty, port.ty) {
                self.say("an argument of the function has another type than its input".to_owned());
            }
        }
        self.results_against(region, function.outputs.iter().map(|port| port.ty), "the function");
        let mut pending: Vec<&Region> = vec![region];
        while let Some(region) = pending.pop() {
            self.region(region, function);
            for node in &region.nodes {
                match &node.kind {
                    NodeKind::Loop { region, .. } | NodeKind::For { region, .. } => pending.push(region),
                    NodeKind::Match { arms } | NodeKind::Handle { arms } => pending.extend(arms.iter().map(|arm| &arm.region)),
                    _ => {}
                }
            }
        }
    }

    fn results_against(&mut self, region: &Region, expected: impl ExactSizeIterator<Item = TyId>, what: &str) {
        if region.results.len() != expected.len() {
            self.say(format!("{what} gives {} results where {} are expected", region.results.len(), expected.len()));
        }
        for (result, ty) in region.results.iter().zip(expected) {
            if !self.same(result.ty, ty) {
                self.say(format!("a result of {what} has another type than what is expected there"));
            }
        }
    }

    fn uses(&mut self, uses: &[Use], within: Within) {
        let mut pending: Vec<&Use> = uses.iter().collect();
        while let Some(used) = pending.pop() {
            self.known_ty(used.ty, "a use");
            match &used.source {
                Source::Arg(index) if *index as usize >= within.args => {
                    self.say(format!("a use of argument {index}, which its region does not have"))
                }
                Source::Out { node, .. } if *node as usize >= within.nodes => {
                    self.say(format!("a use of node {node}, which does not come before it in its region"));
                }
                Source::Const(constant) => self.constant(*constant, "a use"),
                Source::Array(items) => pending.extend(items),
                Source::Arg(_) | Source::Out { .. } => {}
            }
        }
    }

    fn outputs_exist(&mut self, uses: &[Use], nodes: &[Node]) {
        let mut pending: Vec<&Use> = uses.iter().collect();
        while let Some(used) = pending.pop() {
            match &used.source {
                Source::Out { node, output } => {
                    if let Some(node) = nodes.get(*node as usize)
                        && *output as usize >= node.outputs.len()
                    {
                        self.say(format!("a use of output {output} of a node that has {}", node.outputs.len()));
                    }
                }
                Source::Array(items) => pending.extend(items),
                Source::Arg(_) | Source::Const(_) => {}
            }
        }
    }

    fn region(&mut self, region: &Region, function: &Function) {
        for arg in &region.args {
            self.known_ty(arg.ty, "an argument");
        }
        let mut last_position = None;
        let mut lines = std::collections::HashSet::new();
        for (index, node) in region.nodes.iter().enumerate() {
            let within = Within { args: region.args.len(), nodes: index };
            self.uses(&node.inputs, within);
            self.use_types(&node.inputs, region);
            self.outputs_exist(&node.inputs, &region.nodes);
            for order in &node.after {
                match order {
                    Order::Value(used) => {
                        self.uses(std::slice::from_ref(used), within);
                        self.use_types(std::slice::from_ref(used), region);
                        self.outputs_exist(std::slice::from_ref(used), &region.nodes);
                    }
                    Order::Node(other) if *other as usize >= index => self.say(format!("an order on node {other}, which does not come before it")),
                    Order::Node(_) => {}
                }
            }
            for output in &node.outputs {
                self.known_ty(output.ty, "an output");
            }
            if last_position.is_some_and(|last| node.position <= last) {
                self.say(format!("positions do not increase at node {index}"));
            }
            last_position = Some(node.position);
            if !lines.insert(node.origin) {
                self.say(format!("two nodes of one line, at node {index}"));
            }
            self.node(node, function);
        }
        let within = Within { args: region.args.len(), nodes: region.nodes.len() };
        self.uses(&region.results, within);
        self.use_types(&region.results, region);
        self.outputs_exist(&region.results, &region.nodes);
    }

    fn node(&mut self, node: &Node, function: &Function) {
        let (inputs, outputs) = (node.inputs.len(), node.outputs.len());
        let variadic = match &node.kind {
            NodeKind::Primitive { variadic, .. } | NodeKind::Native { variadic, .. } | NodeKind::Call { variadic, .. } => *variadic,
            _ => None,
        };
        if let Some(variadic) = variadic
            && (variadic.start as usize).saturating_add(variadic.count as usize) > inputs
        {
            self.say(format!("variadic inputs from {} for {}, past the {inputs} inputs of the node", variadic.start, variadic.count));
        }
        match &node.kind {
            NodeKind::Constant | NodeKind::BuildArray => {
                if outputs != 1 || (matches!(node.kind, NodeKind::Constant) && inputs != 1) {
                    self.say("a constant or an array node with other than one output".to_owned());
                }
            }
            NodeKind::Construct { ty, variant } => {
                self.known_ty(*ty, "a construction");
                let definition = self.program.definitions.get(ty).copied().flatten().unwrap_or(*ty);
                let expected: Option<Vec<TyId>> = match (self.program.types.get(definition), variant) {
                    (Ty::Struct(fields), None) => Some(fields.iter().map(|(_, ty)| *ty).collect()),
                    (Ty::Enum(variants), Some(variant)) => match variants.get(*variant as usize) {
                        Some(variant) => Some(variant.values.clone()),
                        None => {
                            self.say(format!("a construction of variant {variant}, which its enumeration does not have"));
                            None
                        }
                    },
                    // `t0`, `dt` and `samples`, whose types `check` gave the line.
                    (Ty::Waveform(_), None) => {
                        if inputs != 3 {
                            self.say(format!("a waveform built of {inputs} values"));
                        }
                        None
                    }
                    _ => {
                        self.say("a construction of what is neither a struct nor an enumeration".to_owned());
                        None
                    }
                };
                if let Some(expected) = expected {
                    self.inputs_against(node, &expected, "a construction");
                }
                if outputs != 1 {
                    self.say("a construction with other than one output".to_owned());
                }
            }
            NodeKind::Call { function: callee, variadic } | NodeKind::Native { function: callee, variadic } => {
                let Some(callee) = self.program.functions.get(callee.0 as usize) else {
                    self.say(format!("a call of function {}, which does not exist", callee.0));
                    return;
                };
                let mut expected = Vec::new();
                for port in &callee.inputs {
                    match (port.variadic, variadic) {
                        (true, Some(variadic)) => {
                            let element = match self.program.types.get(port.ty) {
                                Ty::Array { element, dimensions: 1 } => *element,
                                _ => TyId::ERROR,
                            };
                            expected.extend(std::iter::repeat_n(element, variadic.count as usize));
                        }
                        (true, None) => self.say("a call of a variadic function that does not say where its arguments are".to_owned()),
                        _ => expected.push(port.ty),
                    }
                }
                self.inputs_against(node, &expected, "a call");
                let callee_outputs: Vec<TyId> = callee.outputs.iter().map(|port| port.ty).collect();
                self.outputs_against(node, &callee_outputs, "a call");
                let native = matches!(callee.body, Body::Native { .. });
                if native != matches!(node.kind, NodeKind::Native { .. }) {
                    self.say("a call whose kind is not that of the body it calls".to_owned());
                }
            }
            NodeKind::Primitive { statics, .. } => {
                for item in statics {
                    if let Static::Element(element) = item
                        && function.elements.get(element.0 as usize).is_none()
                    {
                        self.say(format!("element {}, which its function does not have", element.0));
                    }
                }
            }
            NodeKind::Loop { carries, collects, region, .. } => {
                let (c, collects) = (*carries as usize, *collects as usize);
                let k = inputs.checked_sub(c);
                self.structure_args(node, region, 0, 0, "a loop");
                if k.is_none() || region.args.len() != inputs {
                    self.say(format!("a loop of {c} carries with {inputs} inputs and {} arguments", region.args.len()));
                }
                let carries: Vec<TyId> = region.args.iter().take(c).map(|arg| arg.ty).collect();
                self.loop_results(node, region, &carries, true, "a loop");
                if outputs != c + collects {
                    self.say(format!("a loop of {c} carries and {collects} collects with {outputs} outputs"));
                }
            }
            NodeKind::For { parallel, clauses, carries, collects, region } => {
                let (c, collects) = (*carries as usize, *collects as usize);
                if *parallel && c > 0 {
                    self.say("a parallel `for` with a carry".to_owned());
                }
                let clause_inputs: usize = clauses.iter().map(|clause| if *clause == Clause::Range { 2 } else { 1 }).sum();
                let k = inputs.checked_sub(clause_inputs + c);
                if k.is_none_or(|k| region.args.len() != clauses.len() + c + k) {
                    self.say(format!(
                        "a `for` of {} clauses and {c} carries with {inputs} inputs and {} arguments",
                        clauses.len(),
                        region.args.len()
                    ));
                }
                let mut at = 0;
                for (clause, arg) in clauses.iter().zip(&region.args) {
                    if let (Clause::Each, Some(input)) = (clause, node.inputs.get(at))
                        && !self.is_element(input.ty, arg.ty)
                    {
                        self.say("an `in` clause whose element is not the type of its argument".to_owned());
                    }
                    at += if *clause == Clause::Range { 2 } else { 1 };
                }
                self.structure_args(node, region, clause_inputs, clauses.len(), "a `for`");
                let carries: Vec<TyId> = region.args.iter().skip(clauses.len()).take(c).map(|arg| arg.ty).collect();
                self.loop_results(node, region, &carries, false, "a `for`");
                if outputs != c + collects {
                    self.say(format!("a `for` of {c} carries and {collects} collects with {outputs} outputs"));
                }
            }
            NodeKind::Match { arms } | NodeKind::Handle { arms } => {
                let k = inputs.saturating_sub(1);
                if inputs == 0 {
                    self.say("a structure with no selector".to_owned());
                }
                let variants = node.inputs.first().map_or(0, |selector| match self.program.types.get(self.definition(selector.ty)) {
                    Ty::Enum(variants) => variants.len(),
                    _ => 0,
                });
                for arm in arms {
                    for pattern in &arm.patterns {
                        if let Pattern::Variant { variant, .. } = pattern
                            && *variant as usize >= variants
                        {
                            self.say(format!("a pattern of variant {variant}, which the selector does not have"));
                        }
                    }
                    let binds = match arm.patterns.as_slice() {
                        [Pattern::Variant { binds, .. }] => *binds as usize,
                        [Pattern::Category { binds: true, .. } | Pattern::Code { binds: true, .. } | Pattern::Wildcard { binds: true }] => 1,
                        _ => 0,
                    };
                    for pattern in &arm.patterns {
                        if let Pattern::Literal(constant) = pattern {
                            self.constant(*constant, "a pattern");
                        }
                    }
                    if arm.region.args.len() != binds + k {
                        self.say(format!("an arm that binds {binds} with {} arguments for {k} values passed in", arm.region.args.len()));
                    }
                    for (arg, input) in arm.region.args.iter().skip(binds).zip(node.inputs.iter().skip(1)) {
                        if !self.same(arg.ty, input.ty) {
                            self.say("a value passed into an arm with another type than its argument".to_owned());
                        }
                    }
                    self.results_against(&arm.region, node.outputs.iter().map(|output| output.ty), "an arm");
                }
            }
        }
        self.skip(node);
    }

    /// The values passed in are the last inputs and the last arguments, with the same types; the carries come before them.
    fn structure_args(&mut self, node: &Node, region: &Region, input_start: usize, arg_start: usize, what: &str) {
        for (input, arg) in node.inputs.iter().skip(input_start).zip(region.args.iter().skip(arg_start)) {
            if !self.same(input.ty, arg.ty) {
                self.say(format!("an input of {what} with another type than its argument"));
            }
        }
    }

    fn inputs_against(&mut self, node: &Node, expected: &[TyId], what: &str) {
        if node.inputs.len() != expected.len() {
            self.say(format!("{what} with {} inputs where {} are expected", node.inputs.len(), expected.len()));
            return;
        }
        for (input, ty) in node.inputs.iter().zip(expected) {
            if !self.same(input.ty, *ty) {
                self.say(format!("an input of {what} with another type than its parameter"));
            }
        }
    }

    fn outputs_against(&mut self, node: &Node, expected: &[TyId], what: &str) {
        if node.outputs.len() != expected.len() {
            self.say(format!("{what} with {} outputs where {} are expected", node.outputs.len(), expected.len()));
            return;
        }
        for (output, ty) in node.outputs.iter().zip(expected) {
            if !self.same(output.ty, *ty) {
                self.say(format!("an output of {what} with another type than its signature"));
            }
        }
    }

    fn skip(&mut self, node: &Node) {
        if node.skip == Skip::ByCallee {
            self.by_callee(node);
            return;
        }
        let inputs: Vec<TyId> = node.inputs.iter().map(|input| input.ty).collect();
        let outputs: Vec<TyId> = node.outputs.iter().map(|output| output.ty).collect();
        self.skip_against(&node.skip, &inputs, &outputs, "a node");
    }

    /// `core.call`: a primitive whose first input is a function, whose other inputs and outputs are those of its type (4).
    fn by_callee(&mut self, node: &Node) {
        let callee = node.inputs.first().map(|input| self.definition(input.ty));
        let signature = match callee.map(|ty| self.program.types.get(ty)) {
            Some(Ty::Function { params, outputs }) if matches!(node.kind, NodeKind::Primitive { .. }) => {
                Some((params.iter().map(|param| param.ty).collect::<Vec<TyId>>(), outputs.clone()))
            }
            _ => None,
        };
        let Some((params, outputs)) = signature else {
            self.say("a skip by the callee on a node that calls no reference".to_owned());
            return;
        };
        let inputs: Vec<TyId> = node.inputs.iter().skip(1).map(|input| input.ty).collect();
        let found: Vec<TyId> = node.outputs.iter().map(|output| output.ty).collect();
        let same = |a: &[TyId], b: &[TyId]| a.len() == b.len() && a.iter().zip(b).all(|(a, b)| self.same(*a, *b));
        if !same(&inputs, &params) || !same(&found, &outputs) {
            self.say("a call of a reference whose inputs or outputs are not those of its type".to_owned());
        }
    }

    /// A skip in the places of these inputs and outputs: of a node, of a native body by its ports, of a primitive by its type.
    fn skip_against(&mut self, skip: &Skip, inputs: &[TyId], outputs: &[TyId], what: &str) {
        let Skip::OnError { err, defaults, through, .. } = skip else {
            if *skip == Skip::ByCallee {
                self.say(format!("a skip by the callee on {what}"));
            }
            return;
        };
        let is_error = |ty: TyId| matches!(self.program.types.get(ty), Ty::ErrorValue);
        if !inputs.get(*err as usize).is_some_and(|ty| is_error(*ty)) {
            self.say(format!("a skip of {what} on input {err}, which is no `error`"));
        }
        if !outputs.iter().any(|ty| is_error(*ty)) {
            self.say(format!("{what} that skips with no `err` output"));
        }
        if defaults.len() != outputs.len() || through.len() != outputs.len() {
            self.say(format!("a skip of {what} that does not say what each output takes"));
        }
        for default in defaults {
            self.constant(*default, what);
        }
        for (output, input) in outputs.iter().zip(through) {
            if let Some(input) = input
                && !inputs.get(*input as usize).is_some_and(|input| self.same(*input, *output))
            {
                self.say(format!("an output of {what} that passes through an input of another type"));
            }
        }
    }
}
