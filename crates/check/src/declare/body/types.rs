//! Pass 4 (`docs/architecture.md`, section 5): types and units, wire by wire, in dependency order. A value whose type an error
//! left unknown is [`TyId::ERROR`], which matches everything (`labcraft_types::TypeTable::matches`), so that one error gives
//! one diagnostic. It reads what pass 3 found, and a function is checked against the signatures of the others.
//!
//! The order of lines has no meaning (spec 5.2), so a body is typed in the order of what each line reads. That order is one
//! graph for the whole function, walked with a queue: a chain of `a1 = a0`, `a2 = a1`… is as long as the file. A structure is
//! two steps, since what its body reads of its clauses comes before what it gives. What is left in a cycle stays unknown, and
//! pass 5 says why (`LC4001`).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use labcraft_graph::{
    Arg, Body, Branching, Call, Clause, ClauseKind, Function, Id, Numeral, NumeralKind, Operand, Path, Pattern, Rhs, Statement, StatementKind,
    Target as Line, UnitExpr, UnitTerm,
};
use labcraft_types::{
    Code, Diagnostic, Edit, Fix, FnParam, LibraryId, MAX_TEXT, Name, Node, NumberKind, Related, Ty, TyId, Unit, Wire, cut_text, parse_unit,
};

use super::super::{DeclaredFunction, Pass, Scope, add_use, path_text, unit_of};
use super::{Context, definition_of, float_fits, integer_fits, node_of, targets_of};
use crate::load::at;
use crate::primitives::{ParamKind, Primitive, PrimitiveLibrary, PrimitiveOutput, PrimitiveParam, PrimitiveTy};
use crate::resolved::{Meaning, Resolved, ResolvedCall, Terminal};

#[cfg(test)]
mod tests;

const NO_STEP: u32 = u32::MAX;
const NO_RANK: u8 = u8::MAX;

/// What a unit in a fix is checked against: it must read back as a unit, at any nesting that it can have here.
const UNIT_NESTING: u32 = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// A line with no body.
    Whole,
    /// The selector or the clauses of a structure, before its body.
    Open,
    /// What a structure gives, after its body.
    Close,
}

#[derive(Clone, Copy)]
struct Step<'a> {
    statement: &'a Statement,
    phase: Phase,
}

/// The steps of a function, with what each one waits for.
struct Plan<'a> {
    steps: Vec<Step<'a>>,
    edges: Vec<(u32, u32)>,
    /// Who reads what: the id of a value, and the step.
    reads: Vec<(Id, u32)>,
    /// By id, the step that gives a line its type.
    definers: Vec<u32>,
    /// The first name of a statement whose first target has none: `_, ..., _, y = g()` can have a million targets, and a million mistakes.
    late_names: HashMap<Id, Option<Name>>,
}

impl<'a> Plan<'a> {
    fn of(function: &'a Function, resolved: &Resolved, places: usize) -> Self {
        let mut plan = Plan { steps: Vec::new(), edges: Vec::new(), reads: Vec::new(), definers: vec![NO_STEP; places], late_names: HashMap::new() };
        plan.body(&function.body, None, resolved);
        let reads = std::mem::take(&mut plan.reads);
        for (value, reader) in reads {
            let definer = plan.definers.get(value.index()).copied().unwrap_or(NO_STEP);
            if definer != NO_STEP {
                plan.edges.push((definer, reader));
            }
        }
        plan
    }

    fn add(&mut self, statement: &'a Statement, phase: Phase) -> u32 {
        let index = u32::try_from(self.steps.len()).unwrap_or(u32::MAX);
        self.steps.push(Step { statement, phase });
        index
    }

    /// `around` is the steps of the structure that holds this body.
    fn body(&mut self, body: &'a Body, around: Option<(u32, u32)>, resolved: &Resolved) {
        for statement in &body.statements {
            self.statement(statement, around, resolved);
        }
    }

    fn statement(&mut self, statement: &'a Statement, around: Option<(u32, u32)>, resolved: &Resolved) {
        let value = match &statement.kind {
            StatementKind::Binding { value, .. } => Some(value),
            _ => None,
        };
        let is_structure = matches!(value, Some(Rhs::Match(_) | Rhs::Handle(_) | Rhs::For(_) | Rhs::Loop(_)));
        let first = self.add(statement, if is_structure { Phase::Open } else { Phase::Whole });
        let last = if is_structure { self.add(statement, Phase::Close) } else { first };
        if is_structure {
            self.edges.push((first, last));
        }
        if let Some((open, close)) = around {
            self.edges.push((open, first));
            self.edges.push((last, close));
        }
        let targets = targets_of(statement);
        if targets.first().is_some_and(|target| !matches!(target, Line::Name(..))) {
            self.late_names.insert(statement.id, targets.iter().find_map(line_name));
        }
        for target in targets {
            // A target that binds an output has the type of its signature, which nothing waits for (R-2).
            if let Line::Name(id, _) = target
                && !matches!(resolved.of(*id), Meaning::Value(_))
                && let Some(slot) = self.definers.get_mut(id.index())
            {
                *slot = last;
            }
        }
        match &statement.kind {
            StatementKind::Call(call) => self.call(call, first, resolved),
            StatementKind::Collect(value) | StatementKind::Until(value) => self.read(value, first, resolved),
            StatementKind::Binding { value, .. } => match value {
                Rhs::Call(call) => self.call(call, first, resolved),
                Rhs::Struct(literal) => {
                    for field in &literal.fields {
                        self.read(&field.value, first, resolved);
                    }
                }
                Rhs::Operand(operand) => self.read(operand, first, resolved),
                Rhs::Match(branching) | Rhs::Handle(branching) => {
                    self.read(&branching.selector, first, resolved);
                    for arm in &branching.arms {
                        // An array in a pattern holds operands, which the structure judges when it opens.
                        for pattern in arm.patterns() {
                            if let Pattern::Literal(operand) = pattern {
                                self.read(operand, first, resolved);
                            }
                        }
                        self.body(&arm.body, Some((first, last)), resolved);
                        for value in &arm.yields.values {
                            self.read(value, last, resolved);
                        }
                    }
                }
                Rhs::For(structure) => self.structure(&structure.clauses, &structure.body, first, last, resolved),
                Rhs::Loop(structure) => self.structure(&structure.clauses, &structure.body, first, last, resolved),
            },
        }
    }

    fn structure(&mut self, clauses: &'a [Clause], body: &'a Body, first: u32, last: u32, resolved: &Resolved) {
        for clause in clauses {
            match &clause.kind {
                ClauseKind::In(clause) => {
                    self.read(&clause.source, first, resolved);
                    if let Some(end) = &clause.end {
                        self.read(end, first, resolved);
                    }
                }
                ClauseKind::Carry(carry) => self.read(&carry.value, first, resolved),
            }
        }
        self.body(body, Some((first, last)), resolved);
    }

    /// `after:` carries no type, so it does not count here: pass 5 orders the nodes.
    fn call(&mut self, call: &'a Call, step: u32, resolved: &Resolved) {
        for arg in &call.args {
            if let Arg::Value { value, .. } = arg {
                self.read(value, step, resolved);
            }
        }
    }

    fn read(&mut self, operand: &Operand, step: u32, resolved: &Resolved) {
        match operand {
            Operand::Path(path) => {
                if let Meaning::Value(value) = resolved.of(path.id) {
                    self.reads.push((value, step));
                }
            }
            Operand::Array(_, values) => {
                for value in values {
                    self.read(value, step, resolved);
                }
            }
            Operand::Bracket(id, _) => {
                if let Meaning::ArrayOfValue(value) = resolved.of(*id) {
                    self.reads.push((value, step));
                }
            }
            Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) => {}
        }
    }

    /// The steps in an order that puts each one after what it waits for. What is in a cycle does not come out.
    fn in_order(&self) -> Vec<Step<'a>> {
        let count = self.steps.len();
        let mut waits_for = vec![0_u32; count];
        let mut starts = vec![0_usize; count + 1];
        for (from, to) in &self.edges {
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
        let mut followers = vec![0_u32; self.edges.len()];
        for (from, to) in &self.edges {
            if let Some(place) = next_free.get_mut(*from as usize) {
                if let Some(slot) = followers.get_mut(*place) {
                    *slot = *to;
                }
                *place += 1;
            }
        }
        let mut queue: VecDeque<u32> = waits_for.iter().enumerate().filter(|(_, waits)| **waits == 0).map(|(index, _)| index as u32).collect();
        let mut order = Vec::with_capacity(count);
        while let Some(index) = queue.pop_front() {
            let Some(step) = self.steps.get(index as usize) else { continue };
            order.push(*step);
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
        order
    }
}

/// How a value comes to what expects a type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fit {
    Same,
    /// A conversion that the wire shows (3.3, 3.4).
    Coerce,
    /// It would lose something, and needs a node (3.4).
    Narrow,
    No,
}

/// Which output of a node a value comes from.
#[derive(Clone, Copy)]
enum Label {
    Named(Name),
    Text(&'static str),
    /// A structure, or a function reference: their outputs have no name.
    Position(u32),
}

/// Where a wire starts (`docs/cli.md`, section 4.3).
#[derive(Clone, Copy)]
enum Origin<'a> {
    Function(Name),
    Carry(Name),
    Node { statement: &'a Statement, label: Label },
}

/// A value that a node gives.
struct Out<'a> {
    ty: TyId,
    label: Label,
    /// `x = y` makes no node: it gives what `y` is.
    operand: Option<&'a Operand>,
}

/// What a target expects.
#[derive(Clone, Copy)]
enum Bound {
    Output(Name),
    Carry(Name),
}

/// What a diagnostic says beyond its code, its message and its place.
#[derive(Default)]
struct Details {
    wires: Vec<Wire>,
    expected: Option<String>,
    found: Option<String>,
    related: Vec<Related>,
    fixes: Vec<Fix>,
}

/// A type variable of a primitive, as a call binds it.
#[derive(Clone, Copy, Default)]
struct Variable<'a> {
    ty: Option<TyId>,
    /// A type argument or an array decides it, and the others convert to it.
    fixed: bool,
    /// What decided it first, for the wires of a diagnostic.
    first: Option<(&'a Operand, &'static str)>,
    /// Something about it has a diagnostic.
    failed: bool,
}

/// An argument that a call wires to a parameter of a primitive, of kind `Value`.
type PrimitiveArgument<'a> = (&'a Operand, &'static PrimitiveParam);

struct Typer<'p, 'c, 'w, 'a> {
    pass: &'p mut Pass<'a>,
    context: &'c mut Context<'w, 'a>,
    scope: Scope<'a>,
    resolved: &'p mut Resolved,
    viewer: LibraryId,
    function: &'a Function,
    origins: HashMap<Id, Origin<'a>>,
    statement: Option<&'a Statement>,
    /// What `statement` gives a diagnostic, made once, since a line of 16 MiB can have a million mistakes.
    node: Option<Node>,
    /// Every name of the function, once a fix needs a new one.
    used_names: Option<HashSet<&'a str>>,
    /// Where the search for a new name stopped, by what it starts with: a function can have `x2` to `x100000`, and a million fixes.
    next_suffix: HashMap<String, u32>,
    late_names: HashMap<Id, Option<Name>>,
    /// By id, the rank of an array, so that the rank of a deep array costs its size and not its size by its depth.
    ranks: Vec<u8>,
}

/// Types one function, unless its signature has an error or is generic, which have their diagnostics.
pub(super) fn type_function<'a>(
    pass: &mut Pass<'a>,
    context: &mut Context<'_, 'a>,
    scope: Scope<'a>,
    resolved: &mut Resolved,
    name: Name,
    function: &'a Function,
) {
    let declared = context.declared;
    let Some(signature) = declared.get(scope.library).and_then(|library| library.functions.get(&name)) else { return };
    if signature.faulty || signature.generic {
        return;
    }
    let Some(viewer) = pass.ids.get(scope.library).copied() else { return };
    let places = scope.model.source_map.len();
    let mut plan = Plan::of(function, resolved, places);
    let late_names = std::mem::take(&mut plan.late_names);
    let mut typer = Typer {
        pass,
        context,
        scope,
        resolved,
        viewer,
        function,
        origins: HashMap::new(),
        statement: None,
        node: None,
        used_names: None,
        next_suffix: HashMap::new(),
        late_names,
        ranks: vec![NO_RANK; places],
    };
    typer.signature(signature);
    for step in plan.in_order() {
        typer.step(step);
    }
}

fn line_name(target: &Line) -> Option<Name> {
    match target {
        Line::Name(_, name) => Some(*name),
        Line::Skip(_) | Line::Next(..) => None,
    }
}

/// An integer literal, when `integer_fits` said that it is one of `i64` or `u64`.
fn integer_value(numeral: Numeral, digits: &str) -> Option<i128> {
    let digits: String = digits.chars().filter(|c| *c != '_').collect();
    let value: i128 = digits.parse().ok()?;
    Some(if numeral.negative { -value } else { value })
}

/// Whether the type holds this integer exactly: a floating type holds what its digits can write (3.4).
fn fits_kind(value: i128, kind: NumberKind) -> bool {
    let magnitude = value.unsigned_abs();
    let digits = u128::BITS.saturating_sub(magnitude.leading_zeros()).saturating_sub(magnitude.trailing_zeros());
    let (low, high) = match kind {
        NumberKind::I8 => (i128::from(i8::MIN), i128::from(i8::MAX)),
        NumberKind::I16 => (i128::from(i16::MIN), i128::from(i16::MAX)),
        NumberKind::I32 => (i128::from(i32::MIN), i128::from(i32::MAX)),
        NumberKind::I64 => (i128::from(i64::MIN), i128::from(i64::MAX)),
        NumberKind::U8 => (0, i128::from(u8::MAX)),
        NumberKind::U16 => (0, i128::from(u16::MAX)),
        NumberKind::U32 => (0, i128::from(u32::MAX)),
        NumberKind::U64 => (0, i128::from(u64::MAX)),
        NumberKind::F32 | NumberKind::C64 => return digits <= f32::MANTISSA_DIGITS,
        NumberKind::F64 | NumberKind::C128 => return digits <= f64::MANTISSA_DIGITS,
    };
    value >= low && value <= high
}

/// Complex numbers have no order (3.5).
fn is_ordered(primitive: &Primitive) -> bool {
    primitive.library == PrimitiveLibrary::Core && matches!(primitive.name, "lt" | "le" | "gt" | "ge" | "min" | "max")
}

/// What `core.div`, `core.sqrt` and `core.pow` give: integers give an `f64` (3.4), and an `i64` or a `u64` does not widen to it.
fn floated(kind: NumberKind) -> Option<NumberKind> {
    if !kind.is_integer() {
        return Some(kind);
    }
    kind.widens_to(NumberKind::F64).then_some(NumberKind::F64)
}

fn unit_text(unit: &Unit) -> String {
    if *unit == Unit::ONE { "[1]".to_owned() } else { format!("[{unit}]") }
}

impl<'p, 'c, 'w, 'a> Typer<'p, 'c, 'w, 'a> {
    fn signature(&mut self, declared: &DeclaredFunction) {
        let signature = &self.function.signature;
        for (param, declared) in signature.params.iter().zip(&declared.params) {
            let ty = if declared.variadic { self.array_of(declared.ty, param.id) } else { declared.ty };
            self.resolved.set_type(param.id, ty);
            self.origins.insert(param.id, Origin::Function(param.name));
        }
        for (output, declared) in signature.outputs.iter().zip(&declared.outputs) {
            self.resolved.set_type(output.id, declared.ty);
            self.origins.insert(output.id, Origin::Function(output.name));
        }
    }

    fn step(&mut self, step: Step<'a>) {
        let Step { statement, phase } = step;
        if self.statement.is_none_or(|current| !std::ptr::eq(current, statement)) {
            self.statement = Some(statement);
            self.node = None;
        }
        match (phase, &statement.kind) {
            (Phase::Whole, StatementKind::Call(call)) => {
                self.type_call(call, None);
            }
            (Phase::Whole, StatementKind::Collect(value)) => {
                let ty = self.value_of(value, None);
                self.resolved.set_type(statement.id, ty);
            }
            (Phase::Whole, StatementKind::Until(value)) => self.judge_until(value),
            (Phase::Whole, StatementKind::Binding { targets, value }) => self.binding(statement, targets, value),
            (Phase::Open, StatementKind::Binding { value, .. }) => match value {
                Rhs::Match(branching) => self.open_branching(branching, false),
                Rhs::Handle(branching) => self.open_branching(branching, true),
                Rhs::For(structure) => self.open_loop(&structure.clauses),
                Rhs::Loop(structure) => self.open_loop(&structure.clauses),
                Rhs::Call(_) | Rhs::Struct(_) | Rhs::Operand(_) => {}
            },
            (Phase::Close, StatementKind::Binding { targets, value }) => match value {
                Rhs::Match(branching) | Rhs::Handle(branching) => self.close_branching(statement, targets, branching),
                Rhs::For(structure) => self.close_loop(statement, targets, &structure.clauses, &structure.body),
                Rhs::Loop(structure) => self.close_loop(statement, targets, &structure.clauses, &structure.body),
                Rhs::Call(_) | Rhs::Struct(_) | Rhs::Operand(_) => {}
            },
            _ => {}
        }
    }

    // Reporting.

    fn show(&self, ty: TyId) -> String {
        self.pass.table.display(ty, self.viewer, self.pass.names)
    }

    fn text(&self, name: Name) -> &'a str {
        self.pass.names.resolve(name)
    }

    fn say(&mut self, code: Code, message: String, id: Id, details: Details) {
        let mut diagnostic: Diagnostic = at(code, message, &self.scope.file.shown, &self.scope.model.source_map, id);
        diagnostic.function = self.scope.function.map(|name| cut_text(name).into_owned());
        if self.node.is_none()
            && let Some(statement) = self.statement
        {
            self.node = Some(node_of(statement, self.pass.names));
        }
        diagnostic.node = self.node.clone();
        diagnostic.wires = details.wires;
        diagnostic.expected = details.expected;
        diagnostic.found = details.found;
        diagnostic.related = details.related;
        diagnostic.fixes = details.fixes;
        self.pass.push(self.scope.library, diagnostic);
    }

    fn here(&self, message: String, id: Id) -> Related {
        Related { message, file: self.scope.file.shown.clone(), span: self.scope.model.source_map.span(id) }
    }

    /// What decides a type first: a value, then an integer without a unit, which takes the type around it, then `[]`, which needs
    /// one (R-7). An array goes by the element that decides most, whatever its place.
    fn rank(&mut self, operand: &Operand) -> u8 {
        match operand {
            Operand::Number(_, Numeral { kind: NumeralKind::Int(_), .. }, None) => 1,
            Operand::Bracket(_, unit) if unit.rest.is_empty() && matches!(unit.first, UnitTerm::One { exponent: None }) => 1,
            Operand::Array(id, values) => {
                if let Some(known) = self.ranks.get(id.index()).copied().filter(|known| *known != NO_RANK) {
                    return known;
                }
                let mut lowest = 2;
                for value in values {
                    lowest = lowest.min(self.rank(value));
                    if lowest == 0 {
                        break;
                    }
                }
                if let Some(slot) = self.ranks.get_mut(id.index()) {
                    *slot = lowest;
                }
                lowest
            }
            _ => 0,
        }
    }

    fn is_flexible(&mut self, operand: &Operand) -> bool {
        self.rank(operand) > 0
    }

    /// A literal of rows becomes the array of two dimensions that is expected (3.1). Any other literal that decides keeps its type.
    fn rows_hint(&self, operand: &Operand, hint: Option<TyId>) -> Option<TyId> {
        hint.filter(|hint| {
            matches!(operand, Operand::Array(..) | Operand::Bracket(..))
                && matches!(self.pass.table.get(*hint), Ty::Array { dimensions, .. } if *dimensions >= 2)
        })
    }

    fn line_of(&self, id: Id) -> u32 {
        self.scope.model.source_map.span(id).map_or(0, |span| span.start.line)
    }

    /// What a terminal of this node starts with: the name of its line, its label, or `@12` (`docs/cli.md`, section 4.3).
    fn node_prefix(&self, statement: &Statement) -> String {
        let line = match targets_of(statement).first() {
            Some(Line::Name(_, name)) => Some(*name),
            _ => self.late_names.get(&statement.id).copied().flatten(),
        };
        match (line, statement.label) {
            (Some(name), _) => cut_text(self.text(name)).into_owned(),
            (None, Some(label)) => format!("#{}", cut_text(self.text(label.name))),
            (None, None) => format!("@{}", self.line_of(statement.id)),
        }
    }

    fn input_terminal(&self, param: &str) -> String {
        let prefix = self.statement.map_or_else(String::new, |statement| self.node_prefix(statement));
        format!("{prefix}.{}", cut_text(param))
    }

    fn origin_text(&self, origin: Origin<'a>) -> String {
        match origin {
            Origin::Function(name) => format!("fn.{}", cut_text(self.text(name))),
            Origin::Carry(name) => format!("next.{}", cut_text(self.text(name))),
            Origin::Node { statement, label } => {
                let prefix = self.node_prefix(statement);
                match label {
                    Label::Named(name) => format!("{prefix}.{}", cut_text(self.text(name))),
                    Label::Text(name) => format!("{prefix}.{name}"),
                    Label::Position(position) => format!("{prefix}.{position}"),
                }
            }
        }
    }

    /// The wire from an operand to a terminal: a literal or a function has no wire behind it, only where it goes.
    fn wire_of(&self, operand: &Operand, to: String) -> Wire {
        let mut wire = Wire { value: None, from: None, to: Some(to) };
        if let Operand::Path(path) = operand
            && let Meaning::Value(value) = self.resolved.of(path.id)
        {
            wire.value = Some(cut_text(&path_text(path, self.pass.names)).into_owned());
            wire.from = self.origins.get(&value).map(|origin| self.origin_text(*origin));
        }
        wire
    }

    // Types.

    fn plain(&mut self, ty: Ty) -> TyId {
        self.pass.table.intern(ty).unwrap_or(TyId::ERROR)
    }

    /// `LC1006`: a program can nest a type line by line, through `a2 = [a1]`, past what a type can hold.
    fn intern_at(&mut self, ty: Ty, id: Id) -> TyId {
        match self.pass.table.intern(ty) {
            Ok(ty) => ty,
            Err(_) => {
                self.say(Code::Lc1006, "this type is nested too deep, or is out of bounds".to_owned(), id, Details::default());
                TyId::ERROR
            }
        }
    }

    fn array_of(&mut self, element: TyId, id: Id) -> TyId {
        self.intern_at(Ty::Array { element, dimensions: 1 }, id)
    }

    fn number(&mut self, kind: NumberKind, unit: Unit) -> TyId {
        self.plain(Ty::Number { kind, unit })
    }

    fn number_of(&self, ty: TyId) -> Option<(NumberKind, Unit)> {
        match self.pass.table.get(ty) {
            Ty::Number { kind, unit } => Some((*kind, *unit)),
            _ => None,
        }
    }

    fn fit(&self, found: TyId, expected: TyId) -> Fit {
        if self.pass.table.matches(found, expected) {
            return Fit::Same;
        }
        let number_to_number = |from_kind: NumberKind, from_unit: &Unit, to_kind: NumberKind, to_unit: &Unit| {
            if !from_unit.same_dimension(to_unit) {
                return Fit::No;
            }
            let widens = from_kind == to_kind || from_kind.widens_to(to_kind);
            // A change of scale goes through a floating-point number (3.4).
            let keeps = from_unit == to_unit || to_kind.is_inexact();
            if widens && keeps { Fit::Coerce } else { Fit::Narrow }
        };
        match (self.pass.table.get(found), self.pass.table.get(expected)) {
            (Ty::Number { kind: from_kind, unit: from_unit }, Ty::Number { kind: to_kind, unit: to_unit }) => {
                number_to_number(*from_kind, from_unit, *to_kind, to_unit)
            }
            // `duration` converts to and from `f64[s]` (3.1).
            (Ty::Duration, Ty::Number { kind, unit }) => number_to_number(NumberKind::F64, &Unit::SECOND, *kind, unit),
            (Ty::Number { kind, unit }, Ty::Duration) => number_to_number(*kind, unit, NumberKind::F64, &Unit::SECOND),
            _ => Fit::No,
        }
    }

    fn is_scaled(&self, ty: TyId) -> bool {
        self.number_of(ty).is_some_and(|(_, unit)| unit != Unit::ONE && unit.same_dimension(&Unit::ONE))
    }

    /// A number without a unit does not become a scale: `5` for an `f64[mV/V]` would read 5000 mV/V (3.4).
    fn fit_of(&self, operand: &Operand, found: TyId, expected: TyId) -> Fit {
        if found != TyId::ERROR && matches!(operand, Operand::Number(_, _, None)) && self.is_scaled(expected) {
            return Fit::No;
        }
        self.fit(found, expected)
    }

    /// What it is as a number, for a variable bound by `Number` (3.5): a `duration` counts as an `f64[s]`.
    fn as_number(&mut self, ty: TyId) -> Option<TyId> {
        match self.pass.table.get(ty) {
            Ty::Number { .. } => Some(ty),
            Ty::Duration => Some(self.number(NumberKind::F64, Unit::SECOND)),
            _ => None,
        }
    }

    fn is_function(&self, ty: TyId) -> bool {
        matches!(self.pass.table.get(ty), Ty::Function { .. })
    }

    /// The type of one level of an array: `f64` in `[f64]`, `[f64]` in `[f64; 2]`.
    fn element_of(&mut self, ty: TyId) -> Option<TyId> {
        let Ty::Array { element, dimensions } = self.pass.table.get(ty) else { return None };
        let (element, dimensions) = (*element, *dimensions);
        if dimensions <= 1 {
            return Some(element);
        }
        Some(self.plain(Ty::Array { element, dimensions: dimensions - 1 }))
    }

    // Values.

    fn value_of(&mut self, operand: &'a Operand, hint: Option<TyId>) -> TyId {
        let ty = match operand {
            Operand::Path(path) => self.path_type(path),
            Operand::Number(id, numeral, unit) => self.number_type(*id, *numeral, unit.as_deref(), hint),
            Operand::Text(..) => self.plain(Ty::String),
            Operand::Bool(..) => self.plain(Ty::Bool),
            Operand::None(_) => self.plain(Ty::ErrorValue),
            Operand::Array(id, values) => self.array_type(*id, values, hint),
            Operand::Bracket(id, _) => self.bracket_type(*id, hint),
        };
        self.resolved.set_type(operand.id(), ty);
        ty
    }

    fn path_type(&mut self, path: &'a Path) -> TyId {
        match self.resolved.of(path.id) {
            Meaning::Value(value) => {
                let mut ty = self.resolved.ty(value);
                for field in &path.rest {
                    ty = self.field_of(ty, *field, path);
                }
                ty
            }
            Meaning::Variant { ty, .. } => ty,
            Meaning::Function { library, name } => self.reference_to(library, name),
            // A primitive as a reference comes with what has no type yet, and anything else has its diagnostic.
            _ => TyId::ERROR,
        }
    }

    fn field_of(&mut self, ty: TyId, field: Name, path: &Path) -> TyId {
        if ty == TyId::ERROR {
            return ty;
        }
        let definition = definition_of(self.pass, self.context, ty);
        let table = &self.pass.table;
        match table.get(definition) {
            Ty::Error => TyId::ERROR,
            Ty::Struct(fields) => {
                let map = self.context.facts.fields.entry(definition).or_insert_with(|| fields.iter().rev().copied().collect());
                if let Some(found) = map.get(&field).copied() {
                    return found;
                }
                let message = format!("unknown field `{}`: `{}` has no such field", self.text(field), self.show(ty));
                self.say(Code::Lc2006, message, path.id, Details::default());
                TyId::ERROR
            }
            _ => {
                let message = format!("unknown field `{}`: a value of type `{}` has no fields", self.text(field), self.show(ty));
                self.say(Code::Lc2006, message, path.id, Details::default());
                TyId::ERROR
            }
        }
    }

    /// The type of a reference to a function of a library: names of arguments do not count, and a variadic parameter is an
    /// array (section 4). Made once for each, since a function can have a million parameters and be named a million times.
    fn reference_to(&mut self, library: u32, name: Name) -> TyId {
        if let Some(known) = self.context.facts.references.get(&(library, name)) {
            return *known;
        }
        let declared = self.context.declared;
        let function = declared.get(library as usize).and_then(|source| source.functions.get(&name));
        let ty = match function {
            Some(function) if !function.faulty && !function.generic => {
                let params = function
                    .params
                    .iter()
                    .map(|param| {
                        let ty = if param.variadic { self.plain(Ty::Array { element: param.ty, dimensions: 1 }) } else { param.ty };
                        FnParam { name: None, ty }
                    })
                    .collect();
                let outputs = function.outputs.iter().map(|output| output.ty).collect();
                self.plain(Ty::Function { params, outputs })
            }
            _ => TyId::ERROR,
        };
        self.context.facts.references.insert((library, name), ty);
        ty
    }

    /// An integer literal keeps its value in its type, or has its `LC3018` (a number beyond every type has its `LC1006`).
    fn integer_literal_fits(&mut self, id: Id, numeral: Numeral, digits: Name, kind: NumberKind) -> bool {
        let text = self.text(digits);
        if !integer_fits(numeral.negative, text) {
            return false;
        }
        let Some(value) = integer_value(numeral, text) else { return false };
        if fits_kind(value, kind) {
            return true;
        }
        let sign = if numeral.negative { "-" } else { "" };
        let kind_name = kind.as_str();
        let written = cut_text(text);
        let message = if kind.is_inexact() {
            format!("`{sign}{written}` would change value as a `{kind_name}`: write a decimal to accept the rounding")
        } else {
            format!("`{sign}{written}` does not fit a value of type `{kind_name}`")
        };
        self.say(Code::Lc3018, message, id, Details { expected: Some(kind_name.to_owned()), ..Details::default() });
        false
    }

    fn kind_that_holds(&self, numeral: Numeral, digits: Name) -> NumberKind {
        let value = integer_value(numeral, self.text(digits));
        [NumberKind::I32, NumberKind::I64].into_iter().find(|kind| value.is_some_and(|value| fits_kind(value, *kind))).unwrap_or(NumberKind::U64)
    }

    fn number_type(&mut self, id: Id, numeral: Numeral, unit: Option<&UnitExpr>, hint: Option<TyId>) -> TyId {
        if let Some(unit) = unit {
            // A unit that is not one has its diagnostic.
            let Ok(unit) = unit_of(unit, self.pass.names) else { return TyId::ERROR };
            if let NumeralKind::Int(digits) = numeral.kind
                && !self.integer_literal_fits(id, numeral, digits, NumberKind::F64)
            {
                return TyId::ERROR;
            }
            return self.number(NumberKind::F64, unit);
        }
        match numeral.kind {
            NumeralKind::Int(digits) => {
                // A literal without a unit has no dimension, whatever its context (3.4).
                let wanted = hint.and_then(|hint| self.number_of(hint).filter(|(_, unit)| *unit == Unit::ONE).map(|(kind, _)| (kind, hint)));
                let (kind, ty) = match wanted {
                    Some(wanted) => wanted,
                    None => {
                        // Against a type that it cannot take, the cause is that type, and not the size of the literal.
                        let kind = if hint.is_some() { self.kind_that_holds(numeral, digits) } else { NumberKind::I32 };
                        (kind, self.number(kind, Unit::ONE))
                    }
                };
                if self.integer_literal_fits(id, numeral, digits, kind) { ty } else { TyId::ERROR }
            }
            NumeralKind::Float(digits) => {
                if !float_fits(self.text(digits)) {
                    return TyId::ERROR;
                }
                self.number(NumberKind::F64, Unit::ONE)
            }
            NumeralKind::Inf | NumeralKind::Nan => self.number(NumberKind::F64, Unit::ONE),
        }
    }

    /// The first element that decides gives the type of the others, as for the arms of a `match` (R-7).
    fn array_type(&mut self, id: Id, values: &'a [Operand], hint: Option<TyId>) -> TyId {
        let hinted = hint.filter(|hint| matches!(self.pass.table.get(*hint), Ty::Array { .. }));
        let mut decided = hinted.and_then(|hint| self.element_of(hint));
        if values.is_empty() {
            // What is unknown here has its diagnostic.
            if hint == Some(TyId::ERROR) {
                return TyId::ERROR;
            }
            if let Some(hinted) = hinted {
                return hinted;
            }
            // Where something else is expected, `[]` is judged as an array there.
            if let Some(hint) = hint {
                return self.array_of(hint, id);
            }
            let message = "the type of this `[]` is not known here: write it, as in `carry results: [f64] = []`".to_owned();
            self.say(Code::Lc3019, message, id, Details::default());
            return TyId::ERROR;
        }
        let ranks: Vec<u8> = values.iter().map(|value| self.rank(value)).collect();
        for wanted in 0..=2 {
            for (value, _) in values.iter().zip(&ranks).filter(|(_, rank)| **rank == wanted) {
                let found = self.value_of(value, decided);
                match decided {
                    None => decided = Some(found),
                    Some(expected) => self.judge_element(value, found, expected),
                }
            }
        }
        match (hinted, decided) {
            (Some(hint), _) => hint,
            (None, Some(element)) if element != TyId::ERROR => self.array_of(element, id),
            (None, _) => TyId::ERROR,
        }
    }

    fn judge_element(&mut self, value: &Operand, found: TyId, expected: TyId) {
        match self.fit_of(value, found, expected) {
            Fit::Same => {}
            Fit::Coerce => self.resolved.add_coercion(value.id(), found, expected),
            Fit::Narrow | Fit::No => {
                let (expected_text, found_text) = (self.show(expected), self.show(found));
                let message = format!("the elements of an array have one type, `{expected_text}`, and this one is `{found_text}`");
                let details = Details { expected: Some(expected_text), found: Some(found_text), ..Details::default() };
                self.say(Code::Lc3001, message, value.id(), details);
            }
        }
    }

    fn bracket_type(&mut self, id: Id, hint: Option<TyId>) -> TyId {
        let array = hint.filter(|hint| matches!(self.pass.table.get(*hint), Ty::Array { .. }));
        let wanted = array.and_then(|array| self.element_of(array));
        let element = match self.resolved.of(id) {
            Meaning::ArrayOfValue(value) => self.resolved.ty(value),
            Meaning::ArrayOfFunction { library, name } => self.reference_to(library, name),
            Meaning::ArrayOfNumber => match wanted.filter(|wanted| self.number_of(*wanted).is_some_and(|(_, unit)| unit == Unit::ONE)) {
                Some(wanted) => wanted,
                None => self.number(NumberKind::I32, Unit::ONE),
            },
            _ => return TyId::ERROR,
        };
        // An array of one is an array like another: it takes the type expected as `[x, x]` does, each element converting.
        let number = matches!(self.resolved.of(id), Meaning::ArrayOfNumber);
        if let Some((array, wanted)) = array.zip(wanted).filter(|(_, wanted)| !number || !self.is_scaled(*wanted)) {
            match self.fit(element, wanted) {
                Fit::Same => return array,
                Fit::Coerce => {
                    self.resolved.add_coercion(id, element, wanted);
                    return array;
                }
                Fit::Narrow | Fit::No => {}
            }
        }
        self.array_of(element, id)
    }

    // Wires.

    /// An argument against what its parameter expects.
    fn check_input(&mut self, operand: &'a Operand, expected: TyId, param: &str) {
        let found = self.value_of(operand, Some(expected));
        self.judge_input(operand, found, expected, param);
    }

    fn judge_input(&mut self, operand: &'a Operand, found: TyId, expected: TyId, param: &str) {
        let fit = self.fit_of(operand, found, expected);
        match fit {
            Fit::Same => {}
            Fit::Coerce => self.resolved.add_coercion(operand.id(), found, expected),
            Fit::Narrow | Fit::No => {
                let references = self.is_function(found) && self.is_function(expected);
                let code = match fit {
                    Fit::Narrow => Code::Lc3004,
                    _ if references => Code::Lc3008,
                    _ => Code::Lc3001,
                };
                let (expected_text, found_text) = (self.show(expected), self.show(found));
                let message = match code {
                    Code::Lc3004 => format!("`{found_text}` does not convert to `{expected_text}` without a node: nothing is lost silently (3.4)"),
                    Code::Lc3008 => format!("this function does not match the type expected, `{expected_text}`: a reference matches exactly (4)"),
                    _ => format!("the input `{}` expects `{expected_text}`, and this is `{found_text}`", cut_text(param)),
                };
                let fixes = self.conversion_fixes(operand, found, expected);
                let wires = vec![self.wire_of(operand, self.input_terminal(param))];
                let details = Details { wires, expected: Some(expected_text), found: Some(found_text), fixes, ..Details::default() };
                self.say(code, message, operand.id(), details);
            }
        }
    }

    /// What a value bound to an output or a carry has to fit.
    fn judge_bound(&mut self, id: Id, bound: Bound, statement: &'a Statement, out: &Out<'a>, expected: TyId) {
        let fit = match out.operand {
            Some(operand) => self.fit_of(operand, out.ty, expected),
            None => self.fit(out.ty, expected),
        };
        match fit {
            Fit::Same => {}
            Fit::Coerce => self.resolved.add_coercion(id, out.ty, expected),
            Fit::Narrow | Fit::No => {
                let (expected_text, found_text) = (self.show(expected), self.show(out.ty));
                let (code, to, value, message) = match bound {
                    Bound::Output(name) => {
                        let text = self.text(name);
                        let message = format!("the output `{}` has the type `{expected_text}`, and this is `{found_text}`", cut_text(text));
                        (Code::Lc3003, format!("fn.{}", cut_text(text)), Some(cut_text(text).into_owned()), message)
                    }
                    Bound::Carry(name) => {
                        let text = self.text(name);
                        let message =
                            format!("`{}` has the type `{expected_text}`, and `next` gives `{found_text}`: a carry keeps its type", cut_text(text));
                        (Code::Lc3007, format!("next.{}", cut_text(text)), None, message)
                    }
                };
                let wire = match out.operand {
                    Some(operand) => self.wire_of(operand, to),
                    None => Wire { value, from: Some(self.origin_text(Origin::Node { statement, label: out.label })), to: Some(to) },
                };
                let fixes = self.bound_fixes(bound, out, expected);
                self.say(
                    code,
                    message,
                    id,
                    Details { wires: vec![wire], expected: Some(expected_text), found: Some(found_text), fixes, ..Details::default() },
                );
            }
        }
    }

    fn bound_fixes(&mut self, bound: Bound, out: &Out<'a>, expected: TyId) -> Vec<Fix> {
        if let Bound::Carry(name) = bound {
            let found = self.show(out.ty);
            return vec![Fix {
                title: format!("Write the type on the line of the carry: `carry {}: {found} = …`", cut_text(self.text(name))),
                edits: Vec::new(),
            }];
        }
        match out.operand {
            Some(operand) => self.conversion_fixes(operand, out.ty, expected),
            None => self
                .conversion_name(out.ty, expected)
                .map_or_else(Vec::new, |(name, _)| vec![Fix { title: format!("Convert with `{name}`"), edits: Vec::new() }]),
        }
    }

    /// The function that makes a number of this type into that one, with the unit that it takes.
    fn conversion_name(&self, found: TyId, expected: TyId) -> Option<(&'static str, Option<Unit>)> {
        let ((from_kind, from_unit), (to_kind, to_unit)) = (self.number_of(found)?, self.number_of(expected)?);
        let reaches_f64 = from_kind == NumberKind::F64 || from_kind.widens_to(NumberKind::F64);
        if !from_unit.same_dimension(&to_unit) {
            // Both give an `f64`.
            return match (from_unit == Unit::ONE, to_unit == Unit::ONE, to_kind == NumberKind::F64) {
                (true, false, true) if reaches_f64 => Some(("core.with_unit", Some(to_unit))),
                (false, true, true) => Some(("core.unitless", Some(from_unit))),
                _ => None,
            };
        }
        if from_unit != Unit::ONE || to_unit != Unit::ONE || !reaches_f64 {
            return None;
        }
        match to_kind {
            NumberKind::I32 => Some(("core.round_to_i32", None)),
            NumberKind::F32 => Some(("core.to_f32", None)),
            _ => None,
        }
    }

    /// `0` where a voltage is expected becomes `0 V`, and a wire gets a conversion on a line of its own (`docs/cli.md`, 4.2).
    fn conversion_fixes(&mut self, operand: &'a Operand, found: TyId, expected: TyId) -> Vec<Fix> {
        let bare_literal = match operand {
            Operand::Number(_, numeral, None) => self.literal_text(*numeral),
            _ => None,
        };
        // A literal with a unit is an `f64` (3.4), so it fixes nothing else.
        if let (Some(text), Some((NumberKind::F64, unit)), Some((_, found_unit))) = (&bare_literal, self.number_of(expected), self.number_of(found))
            && found_unit == Unit::ONE
            && unit != Unit::ONE
        {
            if parse_unit(&unit.to_string(), UNIT_NESTING).is_err() {
                return vec![Fix { title: "Write the unit of this number (3.4)".to_owned(), edits: Vec::new() }];
            }
            let written = format!("{text} {unit}");
            let title = format!("Write `{written}`: a number without a unit has no dimension (3.4)");
            let edits = self.scope.model.source_map.span(operand.id()).map_or_else(Vec::new, |span| vec![Edit { span, text: written }]);
            return vec![Fix { title, edits }];
        }
        let Some((name, unit)) = self.conversion_name(found, expected) else { return Vec::new() };
        let title = format!("Convert with `{name}`");
        let argument = match operand {
            Operand::Path(path) => Some(path_text(path, self.pass.names)),
            _ => bare_literal,
        };
        let unit_argument = unit.map(|unit| unit.to_string());
        let readable = unit_argument.as_deref().is_none_or(|unit| parse_unit(unit, UNIT_NESTING).is_ok());
        // An edit is never cut, so a text too long to copy for each diagnostic leaves the title alone.
        let edits = match (argument.filter(|argument| argument.len() <= MAX_TEXT), self.statement, readable) {
            (Some(argument), Some(statement), true) => self.conversion_edits(operand, statement, name, &argument, unit_argument.as_deref()),
            _ => Vec::new(),
        };
        vec![Fix { title, edits }]
    }

    fn conversion_edits(&mut self, operand: &Operand, statement: &Statement, name: &str, argument: &str, unit: Option<&str>) -> Vec<Edit> {
        let map = &self.scope.model.source_map;
        let (Some(line), Some(place)) = (map.span(statement.id), map.span(operand.id())) else { return Vec::new() };
        if line.start.column as usize > MAX_TEXT {
            return Vec::new();
        }
        let base = argument.split('.').next().filter(|base| base.chars().next().is_some_and(char::is_alphabetic)).unwrap_or("value");
        let new = self.fresh_name(base);
        let indent = " ".repeat(line.start.column.saturating_sub(1) as usize);
        let unit = unit.map_or_else(String::new, |unit| format!(", [{unit}]"));
        let start = labcraft_types::Span { start: line.start, end: line.start };
        let mut edits = vec![Edit { span: start, text: format!("{new} = {name}({argument}{unit})\n{indent}") }, Edit { span: place, text: new }];
        if !self.scope.file.imports.contains_key("core") {
            edits.extend(add_use(&self.pass.current.sorted_uses, "core").edits);
        }
        edits
    }

    /// A name that no value of the function has.
    fn fresh_name(&mut self, base: &str) -> String {
        if self.used_names.is_none() {
            let mut used = HashSet::new();
            let names = self.pass.names;
            let signature = &self.function.signature;
            used.extend(signature.params.iter().map(|param| names.resolve(param.name)));
            used.extend(signature.outputs.iter().map(|output| names.resolve(output.name)));
            collect_names(&self.function.body, names, &mut used);
            self.used_names = Some(used);
        }
        let used = self.used_names.get_or_insert_with(HashSet::new);
        let first = self.next_suffix.get(base).copied().unwrap_or(2);
        let found = (first..=u32::MAX).find(|suffix| !used.contains(format!("{base}{suffix}").as_str())).unwrap_or(first);
        self.next_suffix.insert(base.to_owned(), found.saturating_add(1));
        format!("{base}{found}")
    }

    /// `None` for a number too long to copy for each diagnostic.
    fn literal_text(&self, numeral: Numeral) -> Option<String> {
        let sign = if numeral.negative { "-" } else { "" };
        match numeral.kind {
            NumeralKind::Int(digits) | NumeralKind::Float(digits) => {
                let text = self.text(digits);
                (text.len() <= MAX_TEXT).then(|| format!("{sign}{text}"))
            }
            NumeralKind::Inf => Some(format!("{sign}inf")),
            NumeralKind::Nan => Some(format!("{sign}nan")),
        }
    }

    // Lines.

    /// What the first target expects: a literal takes it, since it decides nothing itself.
    fn target_hint(&self, target: Option<&Line>) -> Option<TyId> {
        let (Line::Next(id, _) | Line::Name(id, _)) = target? else { return None };
        match self.resolved.of(*id) {
            Meaning::Value(value) => Some(self.resolved.ty(value)),
            _ => None,
        }
    }

    fn binding(&mut self, statement: &'a Statement, targets: &'a [Line], value: &'a Rhs) {
        match value {
            Rhs::Call(call) => {
                let outs = self.type_call(call, Some(targets.len()));
                self.assign(statement, targets, outs);
            }
            Rhs::Struct(literal) => {
                let ty = match self.resolved.of(literal.id) {
                    Meaning::Type(ty) => ty,
                    _ => TyId::ERROR,
                };
                self.assign(statement, targets, vec![Out { ty, label: Label::Position(0), operand: None }]);
            }
            Rhs::Operand(operand) => {
                let hint = self.target_hint(targets.first());
                let ty = self.value_of(operand, hint);
                self.assign(statement, targets, vec![Out { ty, label: Label::Position(0), operand: Some(operand) }]);
            }
            Rhs::Match(_) | Rhs::Handle(_) | Rhs::For(_) | Rhs::Loop(_) => {}
        }
    }

    /// A number of targets that differs has its `LC2103`, and they stay unknown.
    fn assign(&mut self, statement: &'a Statement, targets: &'a [Line], outs: Vec<Out<'a>>) {
        if targets.len() != outs.len() {
            return;
        }
        for (target, out) in targets.iter().zip(&outs) {
            match target {
                Line::Skip(_) => {}
                Line::Next(id, name) => {
                    if let Meaning::Value(carry) = self.resolved.of(*id) {
                        let expected = self.resolved.ty(carry);
                        self.resolved.set_type(*id, expected);
                        self.judge_bound(*id, Bound::Carry(*name), statement, out, expected);
                    }
                }
                Line::Name(id, name) => {
                    if let Meaning::Value(output) = self.resolved.of(*id) {
                        let expected = self.resolved.ty(output);
                        self.resolved.set_type(*id, expected);
                        self.judge_bound(*id, Bound::Output(*name), statement, out, expected);
                    } else {
                        self.resolved.set_type(*id, out.ty);
                        if let Some(origin) = self.origin_of(statement, out) {
                            self.origins.insert(*id, origin);
                        }
                    }
                }
            }
        }
    }

    /// `x = y` makes no node, so a wire from `x` comes from where `y` comes from.
    fn origin_of(&self, statement: &'a Statement, out: &Out<'a>) -> Option<Origin<'a>> {
        match out.operand {
            Some(Operand::Path(path)) => match self.resolved.of(path.id) {
                Meaning::Value(value) => self.origins.get(&value).copied(),
                _ => None,
            },
            Some(_) => None,
            None => Some(Origin::Node { statement, label: out.label }),
        }
    }

    fn judge_until(&mut self, value: &'a Operand) {
        let found = self.value_of(value, None);
        let expected = self.plain(Ty::Bool);
        if self.fit(found, expected) != Fit::Same {
            let found_text = self.show(found);
            let message = format!("`until` takes a `bool`, and this is `{found_text}`");
            let wires = vec![self.wire_of(value, "until".to_owned())];
            let details = Details { wires, expected: Some("bool".to_owned()), found: Some(found_text), ..Details::default() };
            self.say(Code::Lc3014, message, value.id(), details);
        }
    }

    // Structures.

    fn open_branching(&mut self, branching: &'a Branching, is_handle: bool) {
        let found = self.value_of(&branching.selector, None);
        let selector = self.judge_selector(&branching.selector, found, is_handle);
        let error = self.plain(Ty::ErrorValue);
        for arm in &branching.arms {
            for pattern in arm.patterns() {
                self.judge_pattern(pattern, selector, is_handle, error);
            }
        }
    }

    /// The selector, or what is unknown when it cannot be one, so that its arms say nothing more.
    fn judge_selector(&mut self, operand: &Operand, found: TyId, is_handle: bool) -> TyId {
        let definition = if is_handle { found } else { definition_of(self.pass, self.context, found) };
        let accepted = match self.pass.table.get(definition) {
            Ty::Error => true,
            Ty::ErrorValue => is_handle,
            Ty::Bool | Ty::String | Ty::Enum(_) => !is_handle,
            Ty::Number { kind, unit } => !is_handle && kind.is_integer() && *unit == Unit::ONE,
            _ => false,
        };
        if accepted {
            return found;
        }
        let found_text = self.show(found);
        let expected = if is_handle { "error" } else { "bool, an integer, string or an enumeration" };
        let message = format!("a {} takes {expected}, and this is `{found_text}`", if is_handle { "`handle`" } else { "`match`" });
        self.say(Code::Lc3013, message, operand.id(), Details { expected: Some(expected.to_owned()), found: Some(found_text), ..Details::default() });
        TyId::ERROR
    }

    fn wrong_pattern(&mut self, id: Id, selector: TyId, found: Option<String>) {
        if selector == TyId::ERROR {
            return;
        }
        let expected = self.show(selector);
        let message = match &found {
            Some(found) => format!("this pattern is `{found}`, and the selector is `{expected}`"),
            None => format!("this pattern does not apply to a selector of type `{expected}`"),
        };
        self.say(Code::Lc3009, message, id, Details { expected: Some(expected), found, ..Details::default() });
    }

    fn judge_pattern(&mut self, pattern: &'a Pattern, selector: TyId, is_handle: bool, error: TyId) {
        match pattern {
            Pattern::Literal(operand) => {
                let found = self.value_of(operand, Some(selector));
                if found != TyId::ERROR && !self.pass.table.matches(found, selector) {
                    let text = self.show(found);
                    self.wrong_pattern(operand.id(), selector, Some(text));
                }
            }
            Pattern::Range { id, low, high, .. } => {
                let kind = self.number_of(selector).map(|(kind, _)| kind).filter(|kind| kind.is_integer());
                let Some(kind) = kind else {
                    self.wrong_pattern(*id, selector, None);
                    return;
                };
                for bound in [low, high] {
                    let text = self.text(bound.digits);
                    // A number beyond every type has its `LC1006`.
                    if !integer_fits(bound.negative, text) {
                        continue;
                    }
                    let digits: String = text.chars().filter(|c| *c != '_').collect();
                    let value = digits.parse::<i128>().ok().map(|value| if bound.negative { -value } else { value });
                    if value.is_some_and(|value| !fits_kind(value, kind)) {
                        let sign = if bound.negative { "-" } else { "" };
                        let message = format!("`{sign}{}` does not fit a value of type `{}`", cut_text(text), kind.as_str());
                        self.say(Code::Lc3018, message, *id, Details { expected: Some(kind.as_str().to_owned()), ..Details::default() });
                    }
                }
            }
            Pattern::Variant { path, bindings, .. } => {
                let Meaning::Variant { ty, variant } = self.resolved.of(path.id) else { return };
                if !self.pass.table.matches(ty, selector) {
                    let found = self.show(ty);
                    self.wrong_pattern(path.id, selector, Some(found));
                    return;
                }
                let definition = definition_of(self.pass, self.context, ty);
                let count = self.variant_values(definition, variant).map_or(0, <[TyId]>::len);
                if count != bindings.len() {
                    let message = format!("this variant holds {count} values, and this pattern binds {}", bindings.len());
                    self.say(Code::Lc3009, message, path.id, Details::default());
                    return;
                }
                for (position, binding) in bindings.iter().enumerate() {
                    let ty = self.variant_values(definition, variant).and_then(|values| values.get(position)).copied().unwrap_or(TyId::ERROR);
                    self.resolved.set_type(binding.id, ty);
                }
            }
            Pattern::Wildcard { binding, .. } => {
                if let Some(binding) = binding {
                    self.resolved.set_type(binding.id, if is_handle { error } else { selector });
                }
            }
            Pattern::Ok { id } => {
                if !is_handle {
                    self.wrong_pattern(*id, selector, Some("error".to_owned()));
                }
            }
            Pattern::Category { id, binding, .. } | Pattern::Code { id, binding, .. } => {
                if !is_handle {
                    self.wrong_pattern(*id, selector, Some("error".to_owned()));
                }
                if let Some(binding) = binding {
                    self.resolved.set_type(binding.id, error);
                }
            }
        }
    }

    fn variant_values(&mut self, definition: TyId, variant: Name) -> Option<&[TyId]> {
        let table = &self.pass.table;
        let Ty::Enum(variants) = table.get(definition) else { return None };
        let map = self
            .context
            .facts
            .variants
            .entry(definition)
            .or_insert_with(|| variants.iter().map(|variant| (variant.name, variant.values.clone())).collect());
        map.get(&variant).map(Vec::as_slice)
    }

    /// The first value that decides gives each position its type, and the others convert to it (R-7).
    fn close_branching(&mut self, statement: &'a Statement, targets: &'a [Line], branching: &'a Branching) {
        let Some(first) = branching.arms.first() else { return };
        let count = first.yields.values.len();
        if let Some(other) = branching.arms.iter().find(|arm| arm.yields.values.len() != count) {
            let found = other.yields.values.len();
            let related = vec![self.here("the first arm gives this many".to_owned(), first.yields.id)];
            self.say(
                Code::Lc3006,
                format!("this arm gives {found} values, and the first one gives {count} (R-7)"),
                other.yields.id,
                Details { related, ..Details::default() },
            );
            return;
        }
        let mut outs = Vec::with_capacity(count);
        for position in 0..count {
            // Targets that do not match have their `LC2103`, and say nothing about the values.
            let hint = if targets.len() == count { self.target_hint(targets.get(position)) } else { None };
            let (ty, decider) = self.decide_position(branching, position, hint);
            // A number that decides goes to the target as it would on a line of its own.
            let operand = decider.filter(|value| matches!(value, Operand::Number(_, _, None)) && hint.is_some_and(|hint| self.is_scaled(hint)));
            outs.push(Out { ty, label: Label::Position(u32::try_from(position).unwrap_or(u32::MAX)), operand });
        }
        self.assign(statement, targets, outs);
    }

    /// Arms that disagree have their diagnostic, and what they give stays unknown.
    fn decide_position(&mut self, branching: &'a Branching, position: usize, hint: Option<TyId>) -> (TyId, Option<&'a Operand>) {
        let mut decided: Option<(TyId, Id, &'a Operand)> = None;
        let mut agree = true;
        for wanted in 0..=2 {
            for arm in &branching.arms {
                let Some(value) = arm.yields.values.get(position) else { continue };
                if self.rank(value) != wanted {
                    continue;
                }
                // A value that decides keeps its type against the target (R-7).
                let around = match decided {
                    Some((ty, ..)) => Some(ty),
                    None if wanted == 0 => self.rows_hint(value, hint),
                    None => hint,
                };
                let found = self.value_of(value, around);
                match decided {
                    None => decided = Some((found, arm.yields.id, value)),
                    Some((expected, by, _)) => agree &= self.judge_arm(value, arm.yields.id, position, found, expected, by),
                }
            }
        }
        decided.filter(|_| agree).map_or((TyId::ERROR, None), |(ty, _, value)| (ty, Some(value)))
    }

    /// Whether this arm gives what the arm that decides gives.
    fn judge_arm(&mut self, value: &Operand, yields: Id, position: usize, found: TyId, expected: TyId, by: Id) -> bool {
        match self.fit_of(value, found, expected) {
            Fit::Same => true,
            Fit::Coerce => {
                self.resolved.add_coercion(value.id(), found, expected);
                true
            }
            Fit::Narrow | Fit::No => {
                let (expected_text, found_text) = (self.show(expected), self.show(found));
                let message = format!("this arm gives `{found_text}`, and the arm that decides gives `{expected_text}` (R-7)");
                let wires = vec![self.wire_of(value, format!("yield.{position}"))];
                let related = vec![self.here(format!("this arm decides the type: `{expected_text}`"), by)];
                let details = Details { wires, expected: Some(expected_text), found: Some(found_text), related, ..Details::default() };
                self.say(Code::Lc3006, message, yields, details);
                false
            }
        }
    }

    fn open_loop(&mut self, clauses: &'a [Clause]) {
        for clause in clauses {
            match &clause.kind {
                ClauseKind::In(clause) => {
                    let ty = match &clause.end {
                        None => self.element_of_source(&clause.source),
                        Some(end) => self.range_type(&clause.source, end),
                    };
                    self.resolved.set_type(clause.id, ty);
                }
                ClauseKind::Carry(carry) => {
                    let ty = match self.resolved.of(carry.id) {
                        Meaning::Type(written) => {
                            let found = self.value_of(&carry.value, Some(written));
                            self.judge_initial(&carry.value, carry.name, found, written);
                            written
                        }
                        _ => self.value_of(&carry.value, None),
                    };
                    self.resolved.set_type(carry.id, ty);
                    self.origins.insert(carry.id, Origin::Carry(carry.name));
                }
            }
        }
    }

    fn judge_initial(&mut self, value: &'a Operand, name: Name, found: TyId, written: TyId) {
        match self.fit_of(value, found, written) {
            Fit::Same => {}
            Fit::Coerce => self.resolved.add_coercion(value.id(), found, written),
            Fit::Narrow | Fit::No => {
                let (expected, found_text) = (self.show(written), self.show(found));
                let message = format!("the carry `{}` has the type `{expected}`, and its initial value is `{found_text}`", cut_text(self.text(name)));
                let fixes = self.conversion_fixes(value, found, written);
                let wires = vec![self.wire_of(value, format!("next.{}", cut_text(self.text(name))))];
                self.say(
                    Code::Lc3007,
                    message,
                    value.id(),
                    Details { wires, expected: Some(expected), found: Some(found_text), fixes, ..Details::default() },
                );
            }
        }
    }

    fn element_of_source(&mut self, source: &'a Operand) -> TyId {
        let found = self.value_of(source, None);
        if found == TyId::ERROR {
            return found;
        }
        match self.element_of(found) {
            Some(element) => element,
            None => {
                let found_text = self.show(found);
                let message = format!("this clause goes over an array or a range, and this is `{found_text}`");
                self.say(Code::Lc3020, message, source.id(), Details { found: Some(found_text), ..Details::default() });
                TyId::ERROR
            }
        }
    }

    fn range_type(&mut self, start: &'a Operand, end: &'a Operand) -> TyId {
        let (first, second) = if self.is_flexible(start) { (end, start) } else { (start, end) };
        let decided = self.value_of(first, None);
        let other = self.value_of(second, Some(decided));
        let mut kinds = Vec::with_capacity(2);
        for (operand, ty) in [(first, decided), (second, other)] {
            if ty == TyId::ERROR {
                return TyId::ERROR;
            }
            match self.number_of(ty).filter(|(kind, unit)| kind.is_integer() && *unit == Unit::ONE) {
                Some((kind, _)) => kinds.push(kind),
                None => {
                    let found = self.show(ty);
                    let message = format!("the bounds of a range are integers without a unit, and this is `{found}`");
                    self.say(Code::Lc3020, message, operand.id(), Details { found: Some(found), ..Details::default() });
                    return TyId::ERROR;
                }
            }
        }
        let kind = match kinds.as_slice() {
            [a, b] => a.meeting(*b),
            _ => None,
        };
        let Some(kind) = kind else {
            let found = self.show(other);
            let message = format!("the bounds of this range have no type in common: `{}` and `{found}`", self.show(decided));
            self.say(Code::Lc3020, message, second.id(), Details { found: Some(found), ..Details::default() });
            return TyId::ERROR;
        };
        self.number(kind, Unit::ONE)
    }

    /// Outputs, in order: the carries, then the `collect` values (6.2).
    fn close_loop(&mut self, statement: &'a Statement, targets: &'a [Line], clauses: &'a [Clause], body: &'a Body) {
        let mut outs = Vec::new();
        for clause in clauses {
            if let ClauseKind::Carry(carry) = &clause.kind {
                outs.push(Out {
                    ty: self.resolved.ty(carry.id),
                    label: Label::Position(u32::try_from(outs.len()).unwrap_or(u32::MAX)),
                    operand: None,
                });
            }
        }
        for line in &body.statements {
            if matches!(line.kind, StatementKind::Collect(_)) {
                let element = self.resolved.ty(line.id);
                let ty = self.array_of(element, line.id);
                outs.push(Out { ty, label: Label::Position(u32::try_from(outs.len()).unwrap_or(u32::MAX)), operand: None });
            }
        }
        self.assign(statement, targets, outs);
    }

    // Calls.

    /// What a call gives, when `wanted` targets can take it. Pass 3 said what it has to say of a number that differs, so
    /// a function of a million outputs costs nothing per call.
    fn type_call(&mut self, call: &'a Call, wanted: Option<usize>) -> Vec<Out<'a>> {
        let Some(bound) = self.resolved.call(call.id).cloned() else { return Vec::new() };
        match bound.callee {
            Meaning::Function { library, name } => self.call_function(call, &bound, library, name, wanted),
            Meaning::Primitive { library, name } => {
                let primitive = self.context.primitives.signature(library, name);
                primitive.map_or_else(Vec::new, |primitive| self.call_primitive(call, &bound, primitive, wanted))
            }
            // The values of a variant come with what has no type yet.
            Meaning::Variant { ty, .. } => vec![Out { ty, label: Label::Position(0), operand: None }],
            _ => Vec::new(),
        }
    }

    fn call_function(&mut self, call: &'a Call, bound: &ResolvedCall, library: u32, name: Name, wanted: Option<usize>) -> Vec<Out<'a>> {
        let declared = self.context.declared;
        let Some(function) = declared.get(library as usize).and_then(|source| source.functions.get(&name)) else { return Vec::new() };
        if function.faulty || function.generic {
            return Vec::new();
        }
        for (arg, terminal) in call.args.iter().zip(&bound.terminals) {
            if let (Arg::Value { value, .. }, Terminal::Param(position)) = (arg, terminal)
                && let Some(param) = function.params.get(*position as usize)
            {
                self.check_input(value, param.ty, self.pass.names.resolve(param.name));
            }
        }
        if wanted != Some(function.outputs.len()) {
            return Vec::new();
        }
        function.outputs.iter().map(|output| Out { ty: output.ty, label: Label::Named(output.name), operand: None }).collect()
    }

    fn call_primitive(&mut self, call: &'a Call, bound: &ResolvedCall, primitive: &'static Primitive, wanted: Option<usize>) -> Vec<Out<'a>> {
        if primitive.library == PrimitiveLibrary::Core && primitive.name == "call" {
            return self.call_reference(call, bound, wanted);
        }
        // The panel and the fields of a template come with what has no type yet.
        if primitive.library == PrimitiveLibrary::Ui {
            return Vec::new();
        }
        if primitive.library == PrimitiveLibrary::String && primitive.name == "format" {
            let ty = self.plain(Ty::String);
            return vec![Out { ty, label: Label::Text("text"), operand: None }];
        }
        let mut wired: Vec<(u32, PrimitiveArgument<'a>)> = call
            .args
            .iter()
            .zip(&bound.terminals)
            .filter_map(|(arg, terminal)| match (arg, terminal) {
                (Arg::Value { value, .. }, Terminal::Param(position)) => {
                    primitive.params.get(*position as usize).filter(|param| param.kind == ParamKind::Value).map(|param| (*position, (value, param)))
                }
                _ => None,
            })
            .collect();
        // In the order of the signature, whatever the call wrote: `core.add(b: v, a: mv)` has the unit of `a`.
        wired.sort_by_key(|(position, _)| *position);
        let arguments: Vec<PrimitiveArgument<'a>> = wired.into_iter().map(|(_, argument)| argument).collect();
        let mut variables = [Variable::default(); 2];
        if bound.type_args.len() == primitive.variables.names.len() {
            for (variable, written) in variables.iter_mut().zip(&bound.type_args) {
                let refused = self.refuses_as_argument(primitive, *written, call.id);
                // A `duration` counts as an `f64[s]`, wherever it comes from (3.5).
                let as_written = if primitive.variables.numbers { self.as_number(*written).unwrap_or(*written) } else { *written };
                *variable = Variable {
                    ty: Some(if refused { TyId::ERROR } else { as_written }),
                    fixed: true,
                    first: None,
                    failed: refused || *written == TyId::ERROR,
                };
            }
        }
        self.bind_arrays(primitive, &arguments, &mut variables);
        let (kelvin, conversions) = self.bind_scalars(primitive, &arguments, &mut variables);
        self.check_concrete(&arguments);
        self.bind_flexible(primitive, &arguments, &mut variables);
        for (id, found, variable) in conversions {
            if let Some(final_ty) = variables.get(variable).and_then(|variable| variable.ty)
                && !self.pass.table.matches(found, final_ty)
            {
                self.resolved.add_coercion(id, found, final_ty);
            }
        }
        self.judge_unit_argument(call, bound, primitive, &variables);
        let results: Vec<TyId> =
            primitive.results.iter().map(|result| self.result_of(*result, call, bound, primitive, &arguments, &variables, kelvin)).collect();
        let labels = primitive.outputs.unwrap_or(&[]);
        results.into_iter().zip(labels).map(|(ty, label)| Out { ty, label: Label::Text(label), operand: None }).collect()
    }

    /// `LC3019`: a type argument keeps to the bound of its variable (3.5), and to what an order needs.
    fn refuses_as_argument(&mut self, primitive: &'static Primitive, written: TyId, id: Id) -> bool {
        if written == TyId::ERROR {
            return false;
        }
        let not_a_number = primitive.variables.numbers && self.as_number(written).is_none();
        let no_order = is_ordered(primitive) && self.number_of(written).is_some_and(|(kind, _)| kind.is_complex());
        if !not_a_number && !no_order {
            return false;
        }
        let what = if no_order { "a real number" } else { "a number" };
        let found = self.show(written);
        let message = format!("this type argument is `{found}`, and this function takes {what} (3.5)");
        self.say(Code::Lc3019, message, id, Details { found: Some(found), ..Details::default() });
        true
    }

    fn concrete(&mut self, ty: PrimitiveTy) -> Option<TyId> {
        let one = Unit::ONE;
        Some(match ty {
            PrimitiveTy::Bool => self.plain(Ty::Bool),
            PrimitiveTy::I32 => self.number(NumberKind::I32, one),
            PrimitiveTy::F32 => self.number(NumberKind::F32, one),
            PrimitiveTy::F64 => self.number(NumberKind::F64, one),
            PrimitiveTy::Text => self.plain(Ty::String),
            PrimitiveTy::Texts => {
                let element = self.plain(Ty::String);
                self.plain(Ty::Array { element, dimensions: 1 })
            }
            PrimitiveTy::Timestamp => self.plain(Ty::Timestamp),
            PrimitiveTy::Duration => self.plain(Ty::Duration),
            PrimitiveTy::Error => self.plain(Ty::ErrorValue),
            PrimitiveTy::Var(_) | PrimitiveTy::ArrayOf(_) | PrimitiveTy::Function | PrimitiveTy::None => return None,
        })
    }

    fn not_a_number(&mut self, operand: &'a Operand, param: &str, found: TyId, what: &str) {
        let found_text = self.show(found);
        let message = format!("the input `{}` takes {what}, and this is `{found_text}`", cut_text(param));
        let wires = vec![self.wire_of(operand, self.input_terminal(param))];
        let details = Details { wires, expected: Some(what.to_owned()), found: Some(found_text), ..Details::default() };
        self.say(Code::Lc3001, message, operand.id(), details);
    }

    /// An array decides its variable, and what is in an array has no conversion.
    fn bind_arrays(&mut self, primitive: &'static Primitive, arguments: &[PrimitiveArgument<'a>], variables: &mut [Variable<'a>; 2]) {
        for (operand, param) in arguments {
            let PrimitiveTy::ArrayOf(index) = param.ty else { continue };
            if self.is_flexible(operand) {
                continue;
            }
            let Some(variable) = variables.get(index).copied() else { continue };
            // A type argument that was refused has its diagnostic.
            if variable.ty == Some(TyId::ERROR) {
                self.value_of(operand, Some(TyId::ERROR));
                continue;
            }
            let hint = variable.ty.map(|element| self.plain(Ty::Array { element, dimensions: 1 }));
            let found = self.value_of(operand, hint);
            let mut slot = variable;
            if found == TyId::ERROR {
                slot.failed = true;
            } else if let Some(element) = self.element_of(found) {
                if primitive.variables.numbers && self.as_number(element).is_none() && element != TyId::ERROR {
                    self.not_a_number(operand, param.name, found, "an array of numbers");
                    slot.failed = true;
                } else if let Some(known) = variable.ty {
                    let value = if primitive.variables.numbers { self.as_number(element).unwrap_or(element) } else { element };
                    if !self.pass.table.matches(known, value) {
                        let expected = self.plain(Ty::Array { element: known, dimensions: 1 });
                        self.judge_input(operand, found, expected, param.name);
                        slot.failed = true;
                    }
                } else {
                    slot.ty = Some(if primitive.variables.numbers { self.as_number(element).unwrap_or(element) } else { element });
                    slot.fixed = true;
                    slot.first = Some((*operand, param.name));
                }
            } else {
                self.not_a_number(operand, param.name, found, "an array");
                slot.failed = true;
            }
            if let Some(place) = variables.get_mut(index) {
                *place = slot;
            }
        }
    }

    /// What is not a literal decides a variable, in the order of the arguments. Two number types meet (3.4). The conversions to
    /// show are known once the variable is final.
    fn bind_scalars(
        &mut self,
        primitive: &'static Primitive,
        arguments: &[PrimitiveArgument<'a>],
        variables: &mut [Variable<'a>; 2],
    ) -> (bool, Vec<(Id, TyId, usize)>) {
        let numbers = primitive.variables.numbers;
        let ordered = is_ordered(primitive);
        let is_sub = primitive.library == PrimitiveLibrary::Core && primitive.name == "sub";
        let mut kelvin = false;
        let mut conversions = Vec::new();
        for (operand, param) in arguments {
            let PrimitiveTy::Var(index) = param.ty else { continue };
            if self.is_flexible(operand) {
                continue;
            }
            let Some(mut variable) = variables.get(index).copied() else { continue };
            // A type argument that was refused has its diagnostic.
            if variable.ty == Some(TyId::ERROR) {
                self.value_of(operand, Some(TyId::ERROR));
                continue;
            }
            let hint = if variable.fixed { variable.ty } else { self.rows_hint(operand, variable.ty) };
            let found = self.value_of(operand, hint);
            if found == TyId::ERROR {
                variable.failed = true;
            } else if numbers && self.as_number(found).is_none() {
                self.not_a_number(operand, param.name, found, "a number");
                variable.failed = true;
            } else if ordered && self.number_of(found).is_some_and(|(kind, _)| kind.is_complex()) {
                self.not_a_number(operand, param.name, found, "a real number: complex numbers have no order");
                variable.failed = true;
            } else {
                let value = if numbers { self.as_number(found).unwrap_or(found) } else { found };
                match variable.ty {
                    None => {
                        variable.ty = Some(value);
                        variable.first = Some((*operand, param.name));
                        conversions.push((operand.id(), found, index));
                    }
                    // What is fixed says its conversion at once.
                    Some(known) if variable.fixed => {
                        self.judge_input(operand, value, known, param.name);
                        let is_scale = |ty| self.number_of(ty).is_some_and(|(_, unit)| unit.is_temperature_scale());
                        kelvin |= is_sub && is_scale(value) && is_scale(known);
                    }
                    Some(known) => {
                        self.meet(operand, param.name, value, known, &mut variable, is_sub, &mut kelvin);
                        conversions.push((operand.id(), found, index));
                    }
                }
            }
            if let Some(place) = variables.get_mut(index) {
                *place = variable;
            }
        }
        (kelvin, conversions)
    }

    #[allow(clippy::too_many_arguments)]
    fn meet(&mut self, operand: &'a Operand, param: &str, value: TyId, known: TyId, variable: &mut Variable<'a>, is_sub: bool, kelvin: &mut bool) {
        let (Some((first_kind, first_unit)), Some((kind, unit))) = (self.number_of(known), self.number_of(value)) else {
            match self.fit(value, known) {
                Fit::Same | Fit::Coerce => {}
                Fit::Narrow | Fit::No => {
                    self.judge_input(operand, value, known, param);
                    variable.failed = true;
                }
            }
            return;
        };
        if self.fit_of(operand, value, known) == Fit::No && first_unit.same_dimension(&unit) {
            self.judge_input(operand, value, known, param);
            variable.failed = true;
            return;
        }
        if !first_unit.same_dimension(&unit) {
            self.report_dimensions(operand, param, value, known, variable);
            variable.failed = true;
            return;
        }
        let met = first_kind.meeting(kind).and_then(|met| {
            // A change of scale goes through a floating-point number (3.4).
            if first_unit == unit || met.is_inexact() {
                return Some(met);
            }
            (met.widens_to(NumberKind::F64)).then_some(NumberKind::F64)
        });
        let Some(met) = met else {
            self.judge_input(operand, value, known, param);
            variable.failed = true;
            return;
        };
        *kelvin |= is_sub && first_unit.is_temperature_scale() && unit.is_temperature_scale();
        variable.ty = Some(self.number(met, first_unit));
    }

    /// `LC3002`, on the second of the two wires, with both.
    fn report_dimensions(&mut self, operand: &'a Operand, param: &str, value: TyId, known: TyId, variable: &Variable<'a>) {
        let (expected, found) = (self.show(known), self.show(value));
        let message = format!("`{expected}` and `{found}` have different dimensions");
        let mut wires = Vec::with_capacity(2);
        if let Some((first, first_param)) = variable.first {
            wires.push(self.wire_of(first, self.input_terminal(first_param)));
        }
        wires.push(self.wire_of(operand, self.input_terminal(param)));
        let fixes = self.conversion_fixes(operand, value, known);
        self.say(Code::Lc3002, message, operand.id(), Details { wires, expected: Some(expected), found: Some(found), fixes, ..Details::default() });
    }

    fn check_concrete(&mut self, arguments: &[PrimitiveArgument<'a>]) {
        for (operand, param) in arguments {
            if let Some(expected) = self.concrete(param.ty) {
                self.check_input(operand, expected, param.name);
            }
        }
    }

    /// Literals decide last: an integer takes the type around it, and `[]` needs one (3.4).
    fn bind_flexible(&mut self, primitive: &'static Primitive, arguments: &[PrimitiveArgument<'a>], variables: &mut [Variable<'a>; 2]) {
        let numbers = primitive.variables.numbers;
        let ranks: Vec<u8> = arguments.iter().map(|(operand, _)| self.rank(operand)).collect();
        let mut in_order: Vec<_> = arguments.iter().zip(ranks).filter(|(_, rank)| *rank > 0).collect();
        in_order.sort_by_key(|(_, rank)| *rank);
        for ((operand, param), _) in in_order {
            let (index, in_array) = match param.ty {
                PrimitiveTy::Var(index) => (index, false),
                PrimitiveTy::ArrayOf(index) => (index, true),
                _ => continue,
            };
            let Some(mut variable) = variables.get(index).copied() else { continue };
            // Of two variables, `core.mul(x, 2)`: the literal takes the number type of the other, with no unit.
            let other = variables.get(1 - index.min(1)).and_then(|other| other.ty).filter(|_| numbers && !in_array);
            let other_kind = other.and_then(|other| self.number_of(other)).map(|(kind, _)| kind);
            let hint = match (variable.ty, other_kind) {
                (Some(known), _) if in_array => Some(self.plain(Ty::Array { element: known, dimensions: 1 })),
                (Some(known), _) => Some(known),
                (None, Some(kind)) => Some(self.number(kind, Unit::ONE)),
                // What failed has its diagnostic, and a second `[]` would only repeat it.
                (None, None) if variable.failed => Some(TyId::ERROR),
                (None, None) => None,
            };
            let found = self.value_of(operand, hint);
            if found == TyId::ERROR {
                variable.failed = true;
            } else if in_array {
                match variable.ty {
                    None => match self.element_of(found) {
                        None => {
                            self.not_a_number(operand, param.name, found, "an array");
                            variable.failed = true;
                        }
                        Some(element) if numbers && element != TyId::ERROR && self.as_number(element).is_none() => {
                            self.not_a_number(operand, param.name, found, "an array of numbers");
                            variable.failed = true;
                        }
                        Some(element) => {
                            variable.ty = Some(element);
                            variable.first = Some((*operand, param.name));
                        }
                    },
                    Some(known) => {
                        let expected = self.plain(Ty::Array { element: known, dimensions: 1 });
                        if !self.pass.table.matches(found, expected) {
                            self.judge_input(operand, found, expected, param.name);
                            variable.failed = true;
                        }
                    }
                }
            } else if numbers && self.as_number(found).is_none() {
                self.not_a_number(operand, param.name, found, "a number");
                variable.failed = true;
            } else {
                match variable.ty {
                    None => {
                        variable.ty = Some(found);
                        variable.first = Some((*operand, param.name));
                    }
                    Some(known) => match self.fit_of(operand, found, known) {
                        Fit::Same => {}
                        Fit::Coerce => self.resolved.add_coercion(operand.id(), found, known),
                        Fit::Narrow | Fit::No => {
                            match (self.number_of(found), self.number_of(known)) {
                                // What is fixed is an input like another (`docs/cli.md`, 4.2).
                                (Some((_, a)), Some((_, b))) if !variable.fixed && !a.same_dimension(&b) => {
                                    self.report_dimensions(operand, param.name, found, known, &variable);
                                }
                                _ => self.judge_input(operand, found, known, param.name),
                            }
                            variable.failed = true;
                        }
                    },
                }
            }
            if let Some(place) = variables.get_mut(index) {
                *place = variable;
            }
        }
    }

    /// `LC3005`: the unit of `core.unitless` has the dimension of `x` (3.3).
    fn judge_unit_argument(&mut self, call: &'a Call, bound: &ResolvedCall, primitive: &'static Primitive, variables: &[Variable<'a>; 2]) {
        if primitive.library != PrimitiveLibrary::Core || primitive.name != "unitless" {
            return;
        }
        let Some(variable) = variables.first().filter(|variable| !variable.failed) else { return };
        let Some((_, expected)) = variable.ty.and_then(|ty| self.number_of(ty)) else { return };
        let Some((id, unit)) = self.unit_argument(call, bound, primitive) else { return };
        if unit.same_dimension(&expected) {
            return;
        }
        let (expected_text, found_text) = (unit_text(&expected), unit_text(&unit));
        let message =
            format!("`core.unitless` needs a unit of the dimension of its value, such as `{expected_text}`, and this is `{found_text}` (3.3)");
        let wires = vec![Wire { value: None, from: None, to: Some(self.input_terminal("unit")) }];
        let edits = self.scope.model.source_map.span(id).map_or_else(Vec::new, |span| vec![Edit { span, text: expected_text.clone() }]);
        let fixes = vec![Fix { title: format!("Use `{expected_text}`"), edits }];
        self.say(Code::Lc3005, message, id, Details { wires, expected: Some(expected_text), found: Some(found_text), fixes, ..Details::default() });
    }

    fn unit_argument(&self, call: &Call, bound: &ResolvedCall, primitive: &Primitive) -> Option<(Id, Unit)> {
        call.args.iter().zip(&bound.terminals).find_map(|(arg, terminal)| match (arg, terminal) {
            (Arg::Value { value: Operand::Bracket(id, unit), .. }, Terminal::Param(position)) => {
                let param = primitive.params.get(*position as usize)?;
                if param.kind != ParamKind::Unit {
                    return None;
                }
                // A unit that is not one has its diagnostic.
                Some((*id, unit_of(unit, self.pass.names).ok()?))
            }
            _ => None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn result_of(
        &mut self,
        result: PrimitiveOutput,
        call: &'a Call,
        bound: &ResolvedCall,
        primitive: &'static Primitive,
        arguments: &[PrimitiveArgument<'a>],
        variables: &[Variable<'a>; 2],
        kelvin: bool,
    ) -> TyId {
        let variable = |index: usize| variables.get(index).filter(|variable| !variable.failed).and_then(|variable| variable.ty);
        match result {
            PrimitiveOutput::Is(PrimitiveTy::Var(index)) => {
                let Some(ty) = variable(index) else { return TyId::ERROR };
                if kelvin && let Some((kind, _)) = self.number_of(ty) {
                    // A change of scale goes through a floating-point number (3.4).
                    let Some(kind) = self.floated_or_say(kind, call.id) else { return TyId::ERROR };
                    return self.number(kind, Unit::KELVIN);
                }
                ty
            }
            PrimitiveOutput::Is(PrimitiveTy::ArrayOf(index)) => {
                let Some(element) = variable(index) else { return TyId::ERROR };
                self.array_of(element, call.id)
            }
            PrimitiveOutput::Is(ty) => self.concrete(ty).unwrap_or(TyId::ERROR),
            PrimitiveOutput::WithUnit => match self.unit_argument(call, bound, primitive) {
                Some((_, unit)) => self.number(NumberKind::F64, unit),
                None => TyId::ERROR,
            },
            PrimitiveOutput::Product | PrimitiveOutput::Quotient => {
                let Some(((a_kind, a_unit), (b_kind, b_unit))) =
                    variable(0).zip(variable(1)).and_then(|(a, b)| self.number_of(a).zip(self.number_of(b)))
                else {
                    return TyId::ERROR;
                };
                let is_product = result == PrimitiveOutput::Product;
                let second = arguments.get(1).map_or(call.id, |(operand, _)| operand.id());
                let met = a_kind.meeting(b_kind);
                let kind = if is_product { met } else { met.and_then(floated) };
                let Some(kind) = kind else {
                    let message = "these two numbers have no type that both convert to without losing anything (3.4)".to_owned();
                    let details = Details { found: Some(format!("{} and {}", a_kind.as_str(), b_kind.as_str())), ..Details::default() };
                    self.say(Code::Lc3004, message, second, details);
                    return TyId::ERROR;
                };
                let unit = if is_product { a_unit.mul(&b_unit) } else { a_unit.div(&b_unit) };
                self.computed_unit(kind, unit, call.id)
            }
            PrimitiveOutput::Root => {
                let Some((kind, unit)) = variable(0).and_then(|ty| self.number_of(ty)) else { return TyId::ERROR };
                let Some(kind) = self.floated_or_say(kind, call.id) else { return TyId::ERROR };
                self.computed_unit(kind, unit.sqrt(), call.id)
            }
            PrimitiveOutput::Power => {
                let Some((kind, unit)) = variable(0).and_then(|ty| self.number_of(ty)) else { return TyId::ERROR };
                let Some(kind) = self.floated_or_say(kind, call.id) else { return TyId::ERROR };
                // An exponent of the wrong type, or none, has its diagnostic.
                let written = arguments.iter().find(|(_, param)| param.name == "exponent").map(|(operand, _)| operand.id());
                let i32_type = self.number(NumberKind::I32, Unit::ONE);
                if written.is_none_or(|id| {
                    let ty = self.resolved.ty(id);
                    ty == TyId::ERROR || matches!(self.fit(ty, i32_type), Fit::Narrow | Fit::No)
                }) {
                    return TyId::ERROR;
                }
                let exponent = arguments.iter().find(|(_, param)| param.name == "exponent").and_then(|(operand, _)| match operand {
                    Operand::Number(_, numeral, None) => match numeral.kind {
                        NumeralKind::Int(digits) => integer_value(*numeral, self.text(digits)).and_then(|value| i32::try_from(value).ok()),
                        _ => None,
                    },
                    _ => None,
                });
                let Some(exponent) = exponent else {
                    self.say(
                        Code::Lc3016,
                        "the exponent of `core.pow` is a constant integer: the unit of its result depends on it (3.3)".to_owned(),
                        call.id,
                        Details::default(),
                    );
                    return TyId::ERROR;
                };
                self.computed_unit(kind, unit.pow(exponent), call.id)
            }
        }
    }

    fn floated_or_say(&mut self, kind: NumberKind, id: Id) -> Option<NumberKind> {
        let floated = floated(kind);
        if floated.is_none() {
            let message = format!("this gives an `f64`, and `{}` does not convert to it without losing precision (3.4)", kind.as_str());
            self.say(
                Code::Lc3004,
                message,
                id,
                Details { expected: Some("f64".to_owned()), found: Some(kind.as_str().to_owned()), ..Details::default() },
            );
        }
        floated
    }

    /// `LC3016`: `None` is a unit that cannot be computed.
    fn computed_unit(&mut self, kind: NumberKind, unit: Option<Unit>, id: Id) -> TyId {
        match unit {
            Some(unit) => self.number(kind, unit),
            None => {
                let message = "the unit of this result cannot be computed: an exponent is odd under a root, a scale is no exact square, or an exponent or a scale goes out of bounds (3.3)".to_owned();
                self.say(Code::Lc3016, message, id, Details::default());
                TyId::ERROR
            }
        }
    }

    /// `core.call`: the terminals are those of the function type, in order or by the names it gives (section 4).
    fn call_reference(&mut self, call: &'a Call, bound: &ResolvedCall, wanted: Option<usize>) -> Vec<Out<'a>> {
        let mut args = call.args.iter().zip(&bound.terminals);
        let reference = args.by_ref().find_map(|(arg, terminal)| match (arg, terminal) {
            (Arg::Value { value, .. }, Terminal::Param(0)) => Some(value),
            _ => None,
        });
        let Some(reference) = reference else { return Vec::new() };
        let ty = self.value_of(reference, None);
        if ty == TyId::ERROR {
            return Vec::new();
        }
        if !self.is_function(ty) {
            self.not_a_number(reference, "function", ty, "a function");
            return Vec::new();
        }
        let names = self.names_of_reference(ty);
        let count = self.reference_params(ty);
        let mut wired: HashSet<u32> = HashSet::new();
        let mut named: HashSet<Name> = HashSet::new();
        let (mut next, mut named_seen, mut mistaken) = (0_u32, false, false);
        for (arg, terminal) in call.args.iter().zip(&bound.terminals) {
            let (Arg::Value { id, name, value }, Terminal::Free) = (arg, terminal) else {
                named_seen |= matches!(arg, Arg::After { .. } | Arg::Value { name: Some(_), .. });
                continue;
            };
            let position = match name {
                None => {
                    if named_seen {
                        // It has its `LC2104`.
                        mistaken = true;
                        continue;
                    }
                    if next as usize >= count {
                        if !mistaken {
                            self.say(Code::Lc2102, format!("too many arguments: this function type has {count} inputs"), *id, Details::default());
                        }
                        mistaken = true;
                        continue;
                    }
                    next += 1;
                    next - 1
                }
                Some(name) => {
                    named_seen = true;
                    // The same name twice has its `LC2102`.
                    if !named.insert(*name) {
                        mistaken = true;
                        continue;
                    }
                    let Some(position) = names.get(name).copied() else {
                        if !mistaken {
                            let message =
                                format!("unknown named argument `{}`: this function type has no input of that name", cut_text(self.text(*name)));
                            self.say(Code::Lc2009, message, *id, Details::default());
                        }
                        mistaken = true;
                        continue;
                    };
                    position
                }
            };
            if !wired.insert(position) {
                self.say(Code::Lc2102, "this input of the function type is wired twice".to_owned(), *id, Details::default());
                mistaken = true;
                continue;
            }
            let param = self.reference_param(ty, position);
            let Some((param_name, expected)) = param else { continue };
            let label: Cow<'_, str> =
                param_name.map_or_else(|| Cow::Owned(position.to_string()), |name| Cow::Borrowed(self.pass.names.resolve(name)));
            self.check_input(value, expected, &label);
        }
        if !mistaken && let Some(missing) = (0..u32::try_from(count).unwrap_or(u32::MAX)).find(|position| !wired.contains(position)) {
            let name = self.reference_param(ty, missing).and_then(|(name, _)| name);
            let label: Cow<'_, str> = name.map_or_else(|| Cow::Owned(missing.to_string()), |name| Cow::Borrowed(self.pass.names.resolve(name)));
            let message = format!("the input `{}` of this function type is not wired: a function type has no default values (4)", cut_text(&label));
            self.say(Code::Lc2101, message, call.id, Details::default());
        }
        let outputs = match self.pass.table.get(ty) {
            Ty::Function { outputs, .. } => outputs.len(),
            _ => 0,
        };
        let Some(targets) = wanted else { return Vec::new() };
        if targets != outputs {
            if let Some(statement) = self.statement {
                let plural = if outputs == 1 { "" } else { "s" };
                self.say(
                    Code::Lc2103,
                    format!("{targets} targets for this function, which gives {outputs} value{plural}"),
                    statement.id,
                    Details::default(),
                );
            }
            return Vec::new();
        }
        (0..outputs)
            .map(|position| {
                let ty = match self.pass.table.get(ty) {
                    Ty::Function { outputs, .. } => outputs.get(position).copied().unwrap_or(TyId::ERROR),
                    _ => TyId::ERROR,
                };
                Out { ty, label: Label::Position(u32::try_from(position).unwrap_or(u32::MAX)), operand: None }
            })
            .collect()
    }

    fn reference_params(&self, ty: TyId) -> usize {
        match self.pass.table.get(ty) {
            Ty::Function { params, .. } => params.len(),
            _ => 0,
        }
    }

    fn reference_param(&self, ty: TyId, position: u32) -> Option<(Option<Name>, TyId)> {
        match self.pass.table.get(ty) {
            Ty::Function { params, .. } => params.get(position as usize).map(|param| (param.name, param.ty)),
            _ => None,
        }
    }

    fn names_of_reference(&mut self, ty: TyId) -> Rc<HashMap<Name, u32>> {
        if let Some(known) = self.context.facts.reference_names.get(&ty) {
            return Rc::clone(known);
        }
        let names: HashMap<Name, u32> = match self.pass.table.get(ty) {
            Ty::Function { params, .. } => {
                params.iter().enumerate().filter_map(|(position, param)| Some((param.name?, u32::try_from(position).unwrap_or(u32::MAX)))).collect()
            }
            _ => HashMap::new(),
        };
        let names = Rc::new(names);
        self.context.facts.reference_names.insert(ty, Rc::clone(&names));
        names
    }
}

fn collect_names<'a>(body: &'a Body, names: &'a labcraft_types::Interner, used: &mut HashSet<&'a str>) {
    for statement in &body.statements {
        for target in targets_of(statement) {
            if let Line::Name(_, name) = target {
                used.insert(names.resolve(*name));
            }
        }
        let StatementKind::Binding { value, .. } = &statement.kind else { continue };
        match value {
            Rhs::Match(branching) | Rhs::Handle(branching) => {
                for arm in &branching.arms {
                    match &arm.pattern {
                        Pattern::Variant { bindings, .. } => used.extend(bindings.iter().map(|binding| names.resolve(binding.name))),
                        Pattern::Wildcard { binding, .. } | Pattern::Category { binding, .. } | Pattern::Code { binding, .. } => {
                            used.extend(binding.iter().map(|binding| names.resolve(binding.name)));
                        }
                        Pattern::Literal(_) | Pattern::Range { .. } | Pattern::Ok { .. } => {}
                    }
                    collect_names(&arm.body, names, used);
                }
            }
            Rhs::For(structure) => {
                collect_clause_names(&structure.clauses, names, used);
                collect_names(&structure.body, names, used);
            }
            Rhs::Loop(structure) => {
                collect_clause_names(&structure.clauses, names, used);
                collect_names(&structure.body, names, used);
            }
            Rhs::Call(_) | Rhs::Struct(_) | Rhs::Operand(_) => {}
        }
    }
}

fn collect_clause_names<'a>(clauses: &'a [Clause], names: &'a labcraft_types::Interner, used: &mut HashSet<&'a str>) {
    for clause in clauses {
        match &clause.kind {
            ClauseKind::In(clause) => {
                used.insert(names.resolve(clause.name));
            }
            ClauseKind::Carry(carry) => {
                used.insert(names.resolve(carry.name));
            }
        }
    }
}
