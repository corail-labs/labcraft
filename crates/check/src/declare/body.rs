//! Pass 3 (`docs/architecture.md`, section 5): names resolved in scopes, calls bound to signatures, labels and `after:`.
//! A function is checked against the signatures of the others, never by following a call. Pass 4, in `types`, reads what this
//! one found: a field of a value, the values of a variant and the fields of a `string.format` template wait for it.
//!
//! It lives under `declare` since it reads a type or a function as that pass does, with its reports and its `use` lines.
//!
//! Walking a body recurses, as far as the nesting that `format::read` let through. What a body binds costs the same at any
//! depth: a name read looks at what it means now, and leaving a body puts back what it hid.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use labcraft_format::write_line;
use labcraft_graph::{
    Arg, Binding, Body, Branching, Call, Clause, ClauseKind, Constant, Decl, For, ForIn, Function, Id, IntBound, Loop, Numeral, NumeralKind, Operand,
    OrderRef, PanelDirection, Path, Pattern, Rhs, Signature, Statement, StatementKind, StructLit, Target as Line, TypeExpr, UnitExpr, UnitTerm,
};
use labcraft_types::{Code, Diagnostic, Edit, Fix, Interner, LibraryId, Name, Node, Related, Report, Ty, TyId, UnitError, cut_text, normalize};

use super::{DeclaredFunction, DeclaredLibrary, DeclaredType, Pass, Scope, add_use, path_text, unit_of};
use crate::load::{Target, at, cycles, last_resort};

use crate::primitives::{PRIMITIVES, ParamKind, Primitive, PrimitiveLibrary};
use crate::resolved::{Meaning, Resolved, ResolvedCall, Terminal};
pub use sections::{MAX_ICON_SHAPES, PathSegment, icon_refused, path_segments};

pub(super) mod flow;
pub(super) mod sections;
#[cfg(test)]
mod tests;

/// What only passes 5 and 6 say: the programs that test the passes before them are written for what those judge.
#[cfg(test)]
pub(in crate::declare) fn later_pass(code: Code) -> bool {
    flow::only_this_pass(code) || sections::only_this_pass(code)
}
mod types;

pub use types::{Align, Format, Piece};
pub(crate) use types::{read_escapes, template_fields, template_pieces};

/// What a call binds to, made once for each function: a hostile file can hold a million parameters and a million calls.
struct Callable {
    params: Vec<(Name, ParamKind)>,
    by_name: HashMap<Name, u32>,
    /// The parameters that a call must wire, in order.
    required: Vec<u32>,
    variadic: Option<u32>,
    /// `None` when only the function it runs can say.
    outputs: Option<usize>,
    /// It takes more than its parameters (5.4).
    open: bool,
    /// How many type arguments a call can write. `None` for a generic function of the user, which has its `LC4006`.
    generics: Option<usize>,
    /// Its signature has an error of its own, and a call says nothing more about it.
    faulty: bool,
}

impl Callable {
    fn new(params: Vec<(Name, ParamKind, bool, bool)>, outputs: Option<usize>, open: bool, generics: Option<usize>, faulty: bool) -> Self {
        let mut by_name = HashMap::with_capacity(params.len());
        let mut required = Vec::new();
        let mut variadic = None;
        for (position, (name, _, has_default, is_variadic)) in params.iter().enumerate() {
            let position = u32::try_from(position).unwrap_or(u32::MAX);
            by_name.entry(*name).or_insert(position);
            if *is_variadic {
                variadic = variadic.or(Some(position));
            } else if !has_default {
                required.push(position);
            }
        }
        let params = params.into_iter().map(|(name, kind, _, _)| (name, kind)).collect();
        Callable { params, by_name, required, variadic, outputs, open, generics, faulty }
    }

    fn of_function(function: &DeclaredFunction) -> Self {
        let params = function.params.iter().map(|param| (param.name, ParamKind::Value, param.has_default, param.variadic)).collect();
        Callable::new(params, Some(function.outputs.len()), false, if function.generic { None } else { Some(0) }, function.faulty)
    }
}

/// The signatures of the primitives, with their names interned. Made once for a run.
pub(super) struct Primitives {
    table: HashMap<(PrimitiveLibrary, Name), Rc<Callable>>,
    signatures: HashMap<(PrimitiveLibrary, Name), &'static Primitive>,
    core: HashSet<Name>,
}

impl Primitives {
    pub(super) fn new(names: &mut Interner) -> Self {
        let mut table = HashMap::with_capacity(PRIMITIVES.len());
        let mut signatures = HashMap::with_capacity(PRIMITIVES.len());
        let mut core = HashSet::new();
        for primitive in PRIMITIVES {
            let name = names.intern(primitive.name);
            let params =
                primitive.params.iter().map(|param| (names.intern(param.name), param.kind, param.default.is_some(), param.variadic)).collect();
            let callable = Callable::new(params, primitive.outputs.map(<[&str]>::len), primitive.open, Some(primitive.variables.names.len()), false);
            table.insert((primitive.library, name), Rc::new(callable));
            signatures.insert((primitive.library, name), primitive);
            if primitive.library == PrimitiveLibrary::Core {
                core.insert(name);
            }
        }
        Primitives { table, signatures, core }
    }

    fn get(&self, library: PrimitiveLibrary, name: Name) -> Option<Rc<Callable>> {
        self.table.get(&(library, name)).cloned()
    }

    fn signature(&self, library: PrimitiveLibrary, name: Name) -> Option<&'static Primitive> {
        self.signatures.get(&(library, name)).copied()
    }

    fn is_in_core(&self, name: Name) -> bool {
        self.core.contains(&name)
    }
}

/// What a named type stands for, made once for each: an enumeration can hold a million variants and be named a million times.
#[derive(Default)]
struct TypeFacts {
    /// From `Checked::libraries` by `LibraryId`.
    libraries: HashMap<LibraryId, usize>,
    /// Through the aliases: `type A = Verdict` stands for the `enum` of `Verdict`.
    definitions: HashMap<TyId, TyId>,
    /// The variants of an enumeration, or the fields of a struct, by the id of its type.
    members: HashMap<TyId, HashSet<Name>>,
    /// The fields of a struct, with their types, by the id of its type.
    fields: HashMap<TyId, HashMap<Name, TyId>>,
    /// The values of each variant of an enumeration, by the id of its type.
    variants: HashMap<TyId, HashMap<Name, Vec<TyId>>>,
    /// The type of a reference to a function of a library, by that library and its name.
    references: HashMap<(u32, Name), TyId>,
    /// The names of a function type, by the id of that type: a call must not walk them each time, in any file of a run.
    reference_names: HashMap<TyId, Rc<HashMap<Name, u32>>>,
    /// What a field of a template can make of a type, by the id of that type: a struct can have a million fields, and be shown a million times.
    shown: HashMap<TyId, types::Shown>,
    /// The types that a manifest declares as resources (8.1).
    resources: HashSet<TyId>,
    /// Whether a type holds one, once for each: a struct can have a million fields, and be read a million times.
    holds: HashMap<TyId, bool>,
}

/// What a value is, and in which body.
#[derive(Clone, Copy)]
struct Value {
    id: Id,
    body: u32,
    carry: bool,
}

/// What a path names, apart from a value.
enum Named {
    Function(Meaning, Rc<Callable>),
    /// `Meaning::Variant`
    Variant(Meaning),
    /// `State.Nowhere`, with the name of the type.
    NoVariant(String),
    /// A library that exists, with no such function.
    NoFunction(String),
    /// A type where a function or a value is expected.
    IsType,
    /// A library that the file does not import, by the name that it would take. `known` when something says that it is
    /// one: otherwise, the first name may be a typo in the name of a value or of a type.
    NotImported {
        library: String,
        known: bool,
    },
    Unknown,
    /// An error that has its own diagnostic may be the cause.
    AlreadySaid,
}

/// A function of a checked library, for `LC2016`: the first call to each function of its library.
struct FunctionCalls<'a> {
    name: Name,
    file: &'a str,
    calls: HashMap<Name, Id>,
    source_map: &'a labcraft_graph::SourceMap,
}

/// Where the body before this one stood.
#[derive(Clone, Copy)]
struct Mark {
    body: u32,
    values: usize,
    labels: usize,
}

/// What pass 3 reads throughout a run.
struct Context<'w, 'a> {
    declared: &'w [DeclaredLibrary],
    primitives: &'w Primitives,
    callables: HashMap<(usize, Name), Rc<Callable>>,
    facts: TypeFacts,
    /// The libraries being checked, by name: what a path can be known to name, whatever the other files of the run import.
    known_libraries: HashSet<&'a str>,
}

struct Walker<'p, 'w, 'a> {
    pass: &'p mut Pass<'a>,
    context: &'p mut Context<'w, 'a>,
    scope: Scope<'a>,
    /// What each name means now.
    values: HashMap<Name, Value>,
    /// What each name of the bodies open now has hidden.
    hidden_values: Vec<(Name, Option<Value>)>,
    labels: HashMap<Name, Id>,
    hidden_labels: Vec<(Name, Option<Id>)>,
    /// Every label of the function, the first one of each name: a label is unique in its function (R-17).
    all_labels: HashMap<Name, Id>,
    /// The output of each name: a line of the body of the function binds it (R-2).
    outputs: HashMap<Name, Id>,
    /// The panel elements of the function by name, and those that are neither an input nor an output.
    elements: HashMap<Name, Id>,
    live: HashMap<Name, Id>,
    body: u32,
    last_body: u32,
    statement: Option<&'a Statement>,
    /// What `statement` gives a diagnostic, made once, since a line of 16 MiB can have a million mistakes.
    node: Option<Node>,
    resolved: Resolved,
    calls: HashMap<Name, Id>,
    /// The first segments of the `use` lines that found nothing, which have their diagnostic: what starts with one says nothing.
    missing: HashSet<Name>,
}

/// What a call has wired so far.
#[derive(Default)]
struct Wiring {
    named: HashSet<u32>,
    /// The names that are not parameters, for `string.format` and `core.call`.
    free: HashSet<Name>,
    /// It has a mistake of its own, and what seems to be missing may be its cause.
    mistaken: bool,
}

/// What a named argument wires.
enum Wired {
    Param(u32),
    Free,
    Mistake,
}

/// What a `waveform` has (3.2).
const WAVEFORM_FIELDS: [&str; 3] = ["t0", "dt", "samples"];

fn unknown_waveform_field(field: &str) -> String {
    format!("unknown field `{}`: a `waveform` has `t0`, `dt` and `samples`", cut_text(field))
}

const SIGNATURE_BODY: u32 = 0;
const FUNCTION_BODY: u32 = 1;

/// Whether an integer fits `i64` or `u64` (`LC1006`). Beyond them no type could hold it.
fn integer_fits(negative: bool, digits: &str) -> bool {
    let digits: String = digits.chars().filter(|c| *c != '_').collect();
    let digits = digits.trim_start_matches('0');
    match digits.parse::<u64>() {
        Ok(value) => !negative || value <= 1 << 63,
        Err(_) => digits.is_empty(),
    }
}

/// Whether a decimal literal fits `f64`. One too small to tell from zero reads as zero.
fn float_fits(digits: &str) -> bool {
    let digits: String = digits.chars().filter(|c| *c != '_').collect();
    digits.parse::<f64>().is_ok_and(f64::is_finite)
}

/// What a diagnostic says of this line: the name of its first target, or else its label, and its text in canonical form.
/// Cut here, not by the report: each diagnostic of a line copies it.
fn node_of(statement: &Statement, names: &Interner) -> Node {
    let line = targets_of(statement).iter().find_map(|target| match target {
        Line::Name(_, name) => Some(*name),
        Line::Skip(_) | Line::Next(..) => None,
    });
    let name = line.or(statement.label.map(|label| label.name)).map(|name| cut_text(names.resolve(name)).into_owned());
    Node { name, text: cut_text(&write_line(statement, names)).into_owned() }
}

fn targets_of(statement: &Statement) -> &[Line] {
    match &statement.kind {
        StatementKind::Binding { targets, .. } => targets,
        _ => &[],
    }
}

/// What a type stands for, through the aliases, each link once: a chain of `.lct` files is as long as the program wants.
fn definition_of(pass: &Pass<'_>, context: &mut Context<'_, '_>, start: TyId) -> TyId {
    let declared = context.declared;
    let mut chain = Vec::new();
    let mut current = start;
    let end = loop {
        if let Some(known) = context.facts.definitions.get(&current) {
            break *known;
        }
        let Ty::Named { library, name } = pass.table.get(current) else { break current };
        let library = context.facts.libraries.get(library).and_then(|index| declared.get(*index));
        let next = library.and_then(|library| library.types.get(name)).map(|ty| ty.definition);
        match next {
            Some(next) if next == current => break current,
            Some(next) if chain.len() <= pass.types.len() => {
                chain.push(current);
                current = next;
            }
            _ => break TyId::ERROR,
        }
    };
    for member in chain {
        context.facts.definitions.insert(member, end);
    }
    end
}

impl<'a> Pass<'a> {
    /// What each file of each library resolves to, parallel to the files of each library: something only for the libraries that
    /// are checked, and for a run for every library. What the bodies of the dependencies say goes to `bodies`, apart: a run
    /// says it only of what it reaches (`docs/cli.md`, 6.1).
    pub(super) fn check_bodies(&mut self, declared: &[DeclaredLibrary], primitives: &Primitives) -> Vec<Vec<Resolved>> {
        let loaded = self.loaded;
        let mut resources = HashSet::new();
        for (library, source) in loaded.iter().enumerate() {
            let Some(types) = declared.get(library).map(|library| &library.types) else { continue };
            for (name, ty) in types {
                let text = self.names.resolve(*name);
                if ty.opaque && source.manifest.resources.iter().any(|resource| normalize(resource).as_ref() == text) {
                    resources.insert(ty.ty);
                }
            }
        }
        let facts = TypeFacts { libraries: self.ids.iter().enumerate().map(|(index, id)| (*id, index)).collect(), resources, ..TypeFacts::default() };
        let known_libraries =
            loaded.iter().filter(|library| library.checked && !library.name.is_empty()).map(|library| library.name.as_str()).collect();
        let mut context = Context { declared, primitives, callables: HashMap::new(), facts, known_libraries };
        let mut all = Vec::with_capacity(loaded.len());
        for (library, source) in loaded.iter().enumerate() {
            let mut resolved = Vec::with_capacity(source.files.len());
            let mut functions = Vec::new();
            let apart = !source.checked;
            for file in &source.files {
                let Some(model) = file.model.as_ref().filter(|_| source.checked || self.for_run) else {
                    resolved.push(Resolved::default());
                    continue;
                };
                let (name, signature, function) = match &model.file.decl {
                    Decl::Function(function) => (function.name, &function.signature, Some(function)),
                    Decl::Extern(function) => (function.name, &function.signature, None),
                    Decl::Type(_) => {
                        resolved.push(Resolved::default());
                        continue;
                    }
                };
                // The body of a dependency says what it has to say apart, to the run that reaches it; and what a budget of
                // it leaves unsaid is nothing that `check` would say.
                let library_report = apart.then(|| self.set_aside(library));
                let cut_short = self.cut_short;
                self.leaned = false;
                self.begin_declaration(model);
                self.current.not_imported = self.said.remove(file.shown.as_str()).unwrap_or_default();
                self.function_errors = self.reports.get(library).map_or(0, labcraft_types::Report::errors);
                let scope = Scope { library, file, model, generic: !signature.generics.is_empty(), function: Some(self.names.resolve(name)) };
                let mut walker = Walker::new(self, &mut context, scope);
                let outcome = last_resort(|| walker.function(signature, function));
                let (mut done, calls) = (std::mem::take(&mut walker.resolved), std::mem::take(&mut walker.calls));
                match (outcome, function) {
                    (Err(payload), _) => self.report_a_panic(library, &file.shown, payload),
                    (Ok(()), function) => {
                        let typed = last_resort(|| types::type_function(self, &mut context, scope, &mut done, name, signature, function));
                        if let Err(payload) = typed {
                            self.report_a_panic(library, &file.shown, payload);
                        } else {
                            if let Some(function) = function {
                                let judged = last_resort(|| flow::judge_function(self, &mut context, scope, &done, name, signature, function));
                                if let Err(payload) = judged {
                                    self.report_a_panic(library, &file.shown, payload);
                                }
                            }
                            let judged = last_resort(|| sections::judge_sections(self, &context, scope, &done, name, signature, function));
                            if let Err(payload) = judged {
                                self.report_a_panic(library, &file.shown, payload);
                            }
                        }
                    }
                }
                if let Some(report) = library_report {
                    self.set_aside_back(library, report);
                    self.cut_short = cut_short;
                }
                if self.leaned {
                    done.lean_elsewhere();
                }
                // An `extern` has no calls, and a call can reach it.
                functions.push(FunctionCalls { name, file: &file.shown, calls, source_map: &model.source_map });
                resolved.push(done);
            }
            if source.checked {
                self.report_recursion(library, functions);
            }
            all.push(resolved);
        }
        all
    }

    /// Gives the library the report of the bodies of the dependencies for one body, and returns its own: one report for all of
    /// them, so that it is bounded as one, whatever the number of files.
    fn set_aside(&mut self, library: usize) -> Report {
        let bodies = std::mem::replace(&mut self.bodies, Report::new(0));
        self.reports.get_mut(library).map_or_else(|| Report::new(0), |report| std::mem::replace(report, bodies))
    }

    /// Puts the report of the library back, and the report of the bodies where it was.
    fn set_aside_back(&mut self, library: usize, report: Report) {
        if let Some(slot) = self.reports.get_mut(library) {
            self.bodies = std::mem::replace(slot, report);
        }
    }

    /// `LC2016`, once per cycle: on the first file in the order of the paths, with the others in `related`. Only calls within
    /// a library count, since a cycle through a `use` has its `LC2021`.
    fn report_recursion(&mut self, library: usize, mut functions: Vec<FunctionCalls<'a>>) {
        functions.sort_by(|a, b| a.file.cmp(b.file));
        // Of two functions of one name, which have their `LC2013`, a call reaches the first one.
        let mut index: HashMap<Name, usize> = HashMap::new();
        for (position, function) in functions.iter().enumerate() {
            index.entry(function.name).or_insert(position);
        }
        let edges: Vec<Vec<usize>> =
            functions.iter().map(|function| function.calls.keys().filter_map(|name| index.get(name).copied()).collect()).collect();
        for component in cycles(&edges) {
            let members: HashSet<usize> = component.iter().copied().collect();
            let mut sorted = component;
            sorted.sort_unstable();
            let call_in_cycle = |function: &FunctionCalls<'_>| {
                let mut calls: Vec<Id> = function
                    .calls
                    .iter()
                    .filter(|(name, _)| index.get(*name).is_some_and(|callee| members.contains(callee)))
                    .map(|(_, id)| *id)
                    .collect();
                calls.sort_unstable();
                calls.first().copied()
            };
            let mut places =
                sorted.iter().filter_map(|member| functions.get(*member)).filter_map(|function| Some((function, call_in_cycle(function)?)));
            let Some((first, call)) = places.next() else { continue };
            let name = self.names.resolve(first.name);
            let message =
                format!("the function `{name}` calls itself, directly or through other functions: a run stops at the limit of call depth (8.2)");
            let mut diagnostic = at(Code::Lc2016, message, first.file, first.source_map, call);
            diagnostic.function = Some(cut_text(name).into_owned());
            diagnostic.related = places
                .map(|(function, call)| Related {
                    message: format!("`{}` is part of this cycle", self.names.resolve(function.name)),
                    file: function.file.to_owned(),
                    span: function.source_map.span(call),
                })
                .collect();
            self.push(library, diagnostic);
        }
    }
}

impl<'p, 'w, 'a> Walker<'p, 'w, 'a> {
    fn new(pass: &'p mut Pass<'a>, context: &'p mut Context<'w, 'a>, scope: Scope<'a>) -> Self {
        Walker {
            pass,
            context,
            scope,
            values: HashMap::new(),
            hidden_values: Vec::new(),
            labels: HashMap::new(),
            hidden_labels: Vec::new(),
            all_labels: HashMap::new(),
            outputs: HashMap::new(),
            elements: HashMap::new(),
            live: HashMap::new(),
            body: SIGNATURE_BODY,
            last_body: SIGNATURE_BODY,
            statement: None,
            node: None,
            resolved: Resolved::new(scope.model.source_map.len()),
            calls: HashMap::new(),
            missing: HashSet::new(),
        }
    }

    fn names(&self) -> &'a Interner {
        self.pass.names
    }

    fn text(&self, name: Name) -> &'a str {
        self.pass.names.resolve(name)
    }

    fn node(&mut self) -> Option<Node> {
        let statement = self.statement?;
        if self.node.is_none() {
            self.node = Some(node_of(statement, self.names()));
        }
        self.node.clone()
    }

    fn say(&mut self, code: Code, message: String, id: Id) {
        self.say_with(code, message, id, Vec::new(), Vec::new());
    }

    fn say_with(&mut self, code: Code, message: String, id: Id, related: Vec<Related>, fixes: Vec<Fix>) {
        let mut diagnostic: Diagnostic = at(code, message, &self.scope.file.shown, &self.scope.model.source_map, id);
        // Cut here, not by the report: a name of 8 MiB and millions of diagnostics would copy it each time.
        diagnostic.function = self.scope.function.map(|name| cut_text(name).into_owned());
        diagnostic.node = self.node();
        diagnostic.related = related;
        diagnostic.fixes = fixes;
        self.pass.push(self.scope.library, diagnostic);
    }

    fn here(&self, message: String, id: Id) -> Related {
        Related { message, file: self.scope.file.shown.clone(), span: self.scope.model.source_map.span(id) }
    }

    /// Nothing to say here: the cause, a file that did not read or a declaration with an error, has its diagnostic
    /// elsewhere. A run cannot lower what this leaves unknown, so the file is marked ([`Resolved::leans_elsewhere`]).
    fn said_elsewhere(&mut self) -> Named {
        self.pass.leaned = true;
        Named::AlreadySaid
    }

    fn own_library_is_broken(&self) -> bool {
        self.pass.broken.get(self.scope.library).copied().unwrap_or(true)
    }

    fn read(&self, name: Name) -> Option<Value> {
        self.values.get(&name).copied()
    }

    // Scopes.

    fn enter(&mut self) -> Mark {
        let mark = Mark { body: self.body, values: self.hidden_values.len(), labels: self.hidden_labels.len() };
        self.last_body = self.last_body.saturating_add(1);
        self.body = self.last_body;
        mark
    }

    fn leave(&mut self, mark: Mark) {
        self.body = mark.body;
        // In reverse: a name that a body bound twice has its `LC2012`, and is bound once, but a mistake must not leave it.
        for (name, hidden) in self.hidden_values.drain(mark.values..).rev() {
            match hidden {
                Some(value) => self.values.insert(name, value),
                None => self.values.remove(&name),
            };
        }
        for (name, hidden) in self.hidden_labels.drain(mark.labels..).rev() {
            match hidden {
                Some(id) => self.labels.insert(name, id),
                None => self.labels.remove(&name),
            };
        }
    }

    /// `LC2012` when this body already has the name, and R-1 otherwise. A sub-body can hide a name from outside it.
    fn bind(&mut self, name: Name, id: Id, carry: bool) {
        let body = self.body;
        if let Some(first) = self.read(name).filter(|value| value.body == body || (body == FUNCTION_BODY && value.body == SIGNATURE_BODY)) {
            let text = self.text(name);
            let related = vec![self.here(format!("`{text}` is first defined here"), first.id)];
            self.say_with(Code::Lc2012, format!("`{text}` is defined twice in this body"), id, related, Vec::new());
            return;
        }
        let hidden = self.values.insert(name, Value { id, body, carry });
        self.hidden_values.push((name, hidden));
        self.judge_the_name(name, id);
    }

    /// R-1: what a value in a body cannot be named.
    fn judge_the_name(&mut self, name: Name, id: Id) {
        if let Some((message, related)) = self.pass.value_name_conflict(name, &self.scope) {
            self.say_with(Code::Lc2014, message, id, vec![related], Vec::new());
        } else if let Some(element) = self.live.get(&name).copied() {
            let text = self.text(name);
            let related = vec![self.here(format!("the panel element `{text}` is declared here"), element)];
            let message = format!(
                "a value cannot bear the name `{text}`: this function has a panel element of that name, neither an input nor an output (R-1)"
            );
            self.say_with(Code::Lc2014, message, id, related, Vec::new());
        }
    }

    /// The names of a body, before anything in it reads them: the order of lines has no meaning (5.2).
    fn declare_lines(&mut self, body: &'a Body) {
        let saved = (self.statement, self.node.take());
        for statement in &body.statements {
            self.statement = Some(statement);
            self.node = None;
            if let Some(label) = &statement.label {
                self.declare_label(label.name, label.id);
            }
            for target in targets_of(statement) {
                if let Line::Name(id, name) = target {
                    self.declare_line(*name, *id);
                }
            }
        }
        (self.statement, self.node) = saved;
    }

    fn declare_label(&mut self, name: Name, id: Id) {
        if let Some(first) = self.all_labels.get(&name).copied() {
            let text = self.text(name);
            let related = vec![self.here(format!("`#{text}` is first used here"), first)];
            self.say_with(Code::Lc2015, format!("the label `#{text}` is used twice in this function"), id, related, Vec::new());
        } else {
            self.all_labels.insert(name, id);
        }
        // Even the second one: an `after:` beside it reads it, and that is no second mistake.
        let hidden = self.labels.insert(name, id);
        self.hidden_labels.push((name, hidden));
    }

    /// In the body of the function, the name of an output binds that output (R-2), which pass 5 counts (`LC4002`).
    fn declare_line(&mut self, name: Name, id: Id) {
        if self.body == FUNCTION_BODY
            && let Some(output) = self.outputs.get(&name).copied()
        {
            self.resolved.set(id, Meaning::Value(output));
            return;
        }
        self.bind(name, id, false);
    }

    // The function.

    fn function(&mut self, signature: &'a Signature, function: Option<&'a Function>) {
        for line in &self.scope.model.file.uses {
            let target = self.scope.file.imports.get(&path_text(&line.path, self.names()));
            if matches!(target, Some(Target::Missing | Target::Unreadable)) {
                self.missing.insert(line.path.first);
            }
        }
        // Before the values of the signature are known, since a default reads none of them.
        for param in &signature.params {
            if let Some(default) = &param.default {
                self.judge_constant(default);
            }
        }
        self.panel_defaults(signature);
        let Some(function) = function else { return };
        self.declare_signature(signature);
        let mark = self.enter();
        self.declare_lines(&function.body);
        self.walk_statements(&function.body);
        self.leave(mark);
    }

    /// A `default:` is for an element that exists only live: an input has its default in the signature, with the one field that the
    /// IDE shows for both, and an output shows nothing before it is produced (10). Like the default of a parameter, it is a constant.
    fn panel_defaults(&mut self, signature: &'a Signature) {
        let Some(panel) = &self.scope.model.file.panel else { return };
        let inputs: HashMap<Name, bool> = signature.params.iter().map(|param| (param.name, param.default.is_some())).collect();
        let outputs: HashSet<Name> = signature.outputs.iter().map(|output| output.name).collect();
        // A name given twice has its `LC5001`, and which item counts would depend on their order, which `fmt` changes.
        let twice = types::panel::names_given_twice(&panel.items);
        for item in panel.items.iter().filter(|item| !twice.contains(&item.name)) {
            let mut given = false;
            for arg in item.args.iter().flatten() {
                let Arg::Value { id, name: Some(name), value } = arg else { continue };
                if self.text(*name) != "default" {
                    continue;
                }
                let element = cut_text(self.text(item.name)).into_owned();
                // Of an input and an output of one name, an indicator shows the output (10).
                let shows_output = outputs.contains(&item.name) && (item.direction == PanelDirection::Indicator || !inputs.contains_key(&item.name));
                if given {
                    self.say(Code::Lc5012, format!("`{element}` is given two `default:`"), *id);
                } else if shows_output {
                    let why = "a value shown before the output is produced would read as a result";
                    self.say(Code::Lc5012, format!("`{element}` is an output, so it has no `default:`: {why} (10)"), *id);
                } else if let Some(has_one) = inputs.get(&item.name) {
                    let why = if *has_one {
                        "it already has one in the signature of the function"
                    } else {
                        "its default belongs to the signature, or an input would be optional here and required for who calls the function"
                    };
                    self.say(Code::Lc5012, format!("`{element}` is an input, so a `default:` cannot go here: {why} (10)"), *id);
                } else {
                    self.panel_constant(value);
                }
                given = true;
            }
        }
    }

    /// What a `default:` holds (10).
    fn panel_constant(&mut self, value: &'a Operand) {
        self.constant_operand(value);
        if types::panel::refused_constant(value, &self.resolved) {
            self.say(
                Code::Lc3017,
                "a `default:` holds a number, a text, a boolean, a variant without values, or an array of those (10)".to_owned(),
                value.id(),
            );
        }
    }

    fn declare_signature(&mut self, signature: &'a Signature) {
        let mut own: HashSet<Name> = HashSet::with_capacity(signature.params.len() + signature.outputs.len());
        for param in &signature.params {
            if own.insert(param.name) {
                self.values.insert(param.name, Value { id: param.id, body: SIGNATURE_BODY, carry: false });
            }
        }
        for output in &signature.outputs {
            if self.outputs.insert(output.name, output.id).is_none() && own.insert(output.name) {
                self.values.insert(output.name, Value { id: output.id, body: SIGNATURE_BODY, carry: false });
            }
        }
        if let Some(panel) = &self.scope.model.file.panel {
            for item in &panel.items {
                self.elements.entry(item.name).or_insert(item.id);
                if !own.contains(&item.name) {
                    self.live.entry(item.name).or_insert(item.id);
                }
            }
        }
    }

    fn walk_statements(&mut self, body: &'a Body) {
        for statement in &body.statements {
            self.walk_statement(statement);
        }
    }

    fn walk_statement(&mut self, statement: &'a Statement) {
        let saved = (self.statement.replace(statement), self.node.take());
        for target in targets_of(statement) {
            if let Line::Next(id, name) = target {
                self.judge_next(*id, *name);
            }
        }
        match &statement.kind {
            StatementKind::Binding { targets, value } => self.walk_rhs(targets.len(), value, statement.id),
            StatementKind::Call(call) => self.walk_call(call, None),
            StatementKind::Collect(value) | StatementKind::Until(value) => self.value_operand(value),
        }
        (self.statement, self.node) = saved;
    }

    /// `LC2018`: a carry of this very loop. `next` appears only directly in a loop body (R-6).
    fn judge_next(&mut self, id: Id, name: Name) {
        match self.read(name) {
            Some(value) if value.carry && value.body == self.body => self.resolved.set(id, Meaning::Value(value.id)),
            _ => {
                let text = self.text(name);
                self.say(Code::Lc2018, format!("`next {text}`: `{text}` is not a carry of this loop (R-6)"), id);
            }
        }
    }

    fn judge_targets(&mut self, id: Id, targets: usize, outputs: usize, what: &str) {
        if targets != outputs {
            let plural = if outputs == 1 { "" } else { "s" };
            self.say(Code::Lc2103, format!("{targets} targets for {what}, which gives {outputs} value{plural}"), id);
        }
    }

    fn walk_rhs(&mut self, targets: usize, rhs: &'a Rhs, id: Id) {
        match rhs {
            Rhs::Call(call) => self.walk_call(call, Some((targets, id))),
            Rhs::Match(branching) | Rhs::Handle(branching) => self.walk_branching(branching, targets, id),
            Rhs::For(structure) => self.walk_for(structure, targets, id),
            Rhs::Loop(structure) => self.walk_loop(structure, targets, id),
            Rhs::Struct(literal) => {
                self.walk_struct(literal);
                self.judge_targets(id, targets, 1, "a struct construction");
            }
            Rhs::Operand(operand) => {
                self.value_operand(operand);
                self.judge_targets(id, targets, 1, "one value");
            }
        }
    }

    fn walk_branching(&mut self, branching: &'a Branching, targets: usize, id: Id) {
        self.value_operand(&branching.selector);
        for arm in &branching.arms {
            let mark = self.enter();
            for pattern in arm.patterns() {
                self.walk_pattern(pattern);
            }
            self.declare_lines(&arm.body);
            self.walk_statements(&arm.body);
            for value in &arm.yields.values {
                self.value_operand(value);
            }
            self.leave(mark);
        }
        // Arms that disagree have their `LC3006` (pass 4).
        let mut counts = branching.arms.iter().map(|arm| arm.yields.values.len());
        if let Some(first) = counts.next()
            && counts.all(|count| count == first)
        {
            self.judge_targets(id, targets, first, "this structure");
        }
    }

    fn walk_pattern(&mut self, pattern: &'a Pattern) {
        match pattern {
            Pattern::Literal(operand) => self.value_operand(operand),
            Pattern::Range { id, low, high, .. } => {
                self.judge_bound(*low, *id);
                self.judge_bound(*high, *id);
            }
            Pattern::Variant { path, bindings, .. } => {
                self.walk_variant_pattern(path);
                for binding in bindings {
                    self.bind(binding.name, binding.id, false);
                }
            }
            Pattern::Wildcard { binding, .. } => self.bind_optional(binding.as_ref()),
            Pattern::Ok { .. } => {}
            Pattern::Category { path, binding, .. } => {
                self.resolved.set(path.id, Meaning::Category);
                self.bind_optional(binding.as_ref());
            }
            Pattern::Code { id, code, binding } => {
                self.judge_bound(*code, *id);
                self.bind_optional(binding.as_ref());
            }
        }
    }

    fn bind_optional(&mut self, binding: Option<&Binding>) {
        if let Some(binding) = binding {
            self.bind(binding.name, binding.id, false);
        }
    }

    fn walk_variant_pattern(&mut self, path: &'a Path) {
        match self.find_named(path) {
            Named::Variant(meaning) => self.resolved.set(path.id, meaning),
            Named::NoVariant(ty) => {
                let text = path_text(path, self.names());
                self.say(Code::Lc2007, format!("unknown variant `{text}`: `{ty}` has no such variant"), path.id);
            }
            Named::NotImported { library, known: true } => self.report_not_imported(library, path.id),
            Named::AlreadySaid => {}
            Named::Function(..) | Named::NoFunction(_) | Named::IsType | Named::Unknown | Named::NotImported { .. } => {
                let text = path_text(path, self.names());
                self.say(Code::Lc2007, format!("unknown variant `{text}`: a variant is written with its type, as in `State.Init`"), path.id);
            }
        }
    }

    fn walk_for(&mut self, structure: &'a For, targets: usize, id: Id) {
        self.walk_structure(&structure.clauses, &structure.body, targets, id);
    }

    fn walk_loop(&mut self, structure: &'a Loop, targets: usize, id: Id) {
        self.walk_structure(&structure.clauses, &structure.body, targets, id);
    }

    /// Its clauses read from outside the structure, even a name that a carry then hides (`carry err = err`, R-3).
    fn walk_structure(&mut self, clauses: &'a [Clause], body: &'a Body, targets: usize, id: Id) {
        for clause in clauses {
            match &clause.kind {
                ClauseKind::In(ForIn { source, end, .. }) => {
                    self.value_operand(source);
                    if let Some(end) = end {
                        self.value_operand(end);
                    }
                }
                ClauseKind::Carry(carry) => {
                    self.value_operand(&carry.value);
                    if let Some(ty) = &carry.ty {
                        let ty = self.written_type(ty, carry.id);
                        self.resolved.set(carry.id, Meaning::Type(ty));
                    }
                }
            }
        }
        let mark = self.enter();
        let mut carries = 0;
        for clause in clauses {
            match &clause.kind {
                ClauseKind::In(clause) => self.bind(clause.name, clause.id, false),
                ClauseKind::Carry(carry) => {
                    carries += 1;
                    self.bind(carry.name, carry.id, true);
                }
            }
        }
        self.declare_lines(body);
        self.walk_statements(body);
        self.leave(mark);
        let collects = body.statements.iter().filter(|statement| matches!(statement.kind, StatementKind::Collect(_))).count();
        // A loop that gives nothing still needs a target, and `_` binds nothing (6.2).
        let gives_nothing = carries + collects == 0 && matches!(self.statement.map(targets_of), Some([Line::Skip(_)]));
        if !gives_nothing {
            self.judge_targets(id, targets, carries + collects, "this loop");
        }
    }

    fn walk_struct(&mut self, literal: &'a StructLit) {
        for field in &literal.fields {
            self.value_operand(&field.value);
        }
        self.struct_type(literal);
    }

    /// What a struct construction builds, and the names of its fields.
    fn struct_type(&mut self, literal: &'a StructLit) {
        let scope = self.scope;
        let lookup = self.pass.lookup(&scope, &literal.path);
        let declared = match lookup {
            super::Lookup::Declared(index) => self.pass.resolved.get(index).copied().flatten(),
            super::Lookup::Unknown => {
                let text = path_text(&literal.path, self.names());
                self.say(Code::Lc2003, format!("unknown type `{text}`"), literal.path.id);
                return;
            }
            super::Lookup::NotImported(library) => {
                self.report_not_imported(library, literal.path.id);
                return;
            }
            super::Lookup::Waveform => {
                self.resolved.set(literal.id, Meaning::Waveform);
                self.judge_waveform_fields(literal);
                return;
            }
            super::Lookup::Base(_) | super::Lookup::Later | super::Lookup::Opaque => {
                self.resolved.set(literal.id, Meaning::NotAStruct);
                return;
            }
            // What has its diagnostic, elsewhere.
            super::Lookup::AlreadySaid => {
                self.pass.leaned = true;
                return;
            }
        };
        let Some(declared) = declared else { return };
        self.resolved.set(literal.id, Meaning::Type(declared.ty));
        let definition = self.definition_of(declared.definition);
        let names = self.names();
        match self.pass.table.get(definition) {
            Ty::Struct(_) => {}
            Ty::Waveform(_) => {
                self.judge_waveform_fields(literal);
                return;
            }
            _ => return,
        }
        for field in &literal.fields {
            if !self.has_member(definition, field.name) {
                let (text, ty) = (names.resolve(field.name), path_text(&literal.path, names));
                self.say(Code::Lc2006, format!("unknown field `{text}`: `{ty}` has no such field"), field.id);
            }
        }
    }

    fn judge_waveform_fields(&mut self, literal: &StructLit) {
        for field in &literal.fields {
            let text = self.text(field.name);
            if !WAVEFORM_FIELDS.contains(&text) {
                self.say(Code::Lc2006, unknown_waveform_field(text), field.id);
            }
        }
    }

    /// A type written in a body says what it has to say with the line it is on.
    fn written_type(&mut self, ty: &TypeExpr, id: Id) -> TyId {
        self.pass.node = self.node();
        let scope = self.scope;
        let ty = self.pass.signature_type(ty, &scope, id);
        self.pass.node = None;
        ty
    }

    // What types stand for.

    fn definition_of(&mut self, start: TyId) -> TyId {
        definition_of(self.pass, self.context, start)
    }

    fn has_member(&mut self, definition: TyId, name: Name) -> bool {
        let table = &self.pass.table;
        let members = self.context.facts.members.entry(definition).or_insert_with(|| match table.get(definition) {
            Ty::Enum(variants) => variants.iter().map(|variant| variant.name).collect(),
            Ty::Struct(fields) => fields.iter().map(|(name, _)| *name).collect(),
            _ => HashSet::new(),
        });
        members.contains(&name)
    }

    // What a path names.

    /// `lib.function`, `lib.Type.Variant`, or `Type.Variant` and `function` of this library.
    fn find_named(&mut self, path: &Path) -> Named {
        let own = self.scope.library;
        let Some((last, head)) = path.rest.split_last() else {
            return self.find_in(own, path.first, Named::Unknown);
        };
        let mut library_name = self.text(path.first).to_owned();
        for segment in head {
            library_name.push('.');
            library_name.push_str(self.text(*segment));
        }
        let file = self.scope.file;
        let imports = &file.imports;
        if let Some(target) = imports.get(&library_name).copied() {
            return self.find_in_target(target, *last, library_name);
        }
        if self.missing.contains(&path.first) {
            return self.said_elsewhere();
        }
        let Some((ty, library_head)) = head.split_last() else {
            // `Reading.Value`
            return self.find_own_variant(path.first, *last, library_name);
        };
        // `lib.Reading.Value`
        let mut owner = self.text(path.first).to_owned();
        for segment in library_head {
            owner.push('.');
            owner.push_str(self.text(*segment));
        }
        match imports.get(&owner).copied() {
            Some(Target::Library(target)) => self.find_variant_in(target, *ty, *last, library_name),
            Some(Target::Primitive) => Named::NoFunction(library_name),
            Some(Target::Missing | Target::Unreadable) => self.said_elsewhere(),
            None if self.is_known(&library_name) => Named::NotImported { library: library_name, known: true },
            None if self.is_known(&owner) => Named::NotImported { library: owner, known: true },
            None => Named::NotImported { library: library_name, known: false },
        }
    }

    fn is_known(&self, library: &str) -> bool {
        PrimitiveLibrary::from_name(library).is_some() || self.context.known_libraries.contains(library)
    }

    fn find_in_target(&mut self, target: Target, name: Name, library_name: String) -> Named {
        match target {
            Target::Primitive => {
                let primitive =
                    PrimitiveLibrary::from_name(&library_name).and_then(|library| Some((library, self.context.primitives.get(library, name)?)));
                match primitive {
                    Some((library, callable)) => Named::Function(Meaning::Primitive { library, name }, callable),
                    None => Named::NoFunction(library_name),
                }
            }
            Target::Library(target) => self.find_in(target, name, Named::NoFunction(library_name)),
            Target::Missing | Target::Unreadable => self.said_elsewhere(),
        }
    }

    fn find_in(&mut self, library: usize, name: Name, otherwise: Named) -> Named {
        let declared = self.context.declared;
        let Some(source) = declared.get(library) else { return self.said_elsewhere() };
        if let Some(function) = source.functions.get(&name) {
            let key = (library, name);
            let callable = self.context.callables.entry(key).or_insert_with(|| Rc::new(Callable::of_function(function)));
            let index = u32::try_from(library).unwrap_or(u32::MAX);
            return Named::Function(Meaning::Function { library: index, name }, Rc::clone(callable));
        }
        if source.types.contains_key(&name) {
            return Named::IsType;
        }
        if source.broken || self.pass.is_refused(library, name) { self.said_elsewhere() } else { otherwise }
    }

    fn find_own_variant(&mut self, ty: Name, variant: Name, library_name: String) -> Named {
        let own = self.scope.library;
        let declared = self.context.declared;
        match declared.get(own).and_then(|source| source.types.get(&ty)).copied() {
            Some(declared) => self.variant_of(declared, ty, variant),
            None if self.own_library_is_broken() || self.pass.is_refused(own, ty) => self.said_elsewhere(),
            // `g.h`, with a function `g`: the first name is known, and it is no library.
            None if declared.get(own).is_some_and(|source| source.functions.contains_key(&ty)) => Named::Unknown,
            None => {
                let known = self.is_known(&library_name);
                Named::NotImported { library: library_name, known }
            }
        }
    }

    fn find_variant_in(&mut self, library: usize, ty: Name, variant: Name, library_name: String) -> Named {
        let declared = self.context.declared;
        let Some(source) = declared.get(library) else { return self.said_elsewhere() };
        match source.types.get(&ty).copied() {
            Some(declared) => self.variant_of(declared, ty, variant),
            None if source.broken || self.pass.is_refused(library, ty) => self.said_elsewhere(),
            // `a.b.g`, with `use a`: the file does not import the sub-library that has `g`.
            None if self.is_known(&library_name) => Named::NotImported { library: library_name, known: true },
            None => Named::NoFunction(library_name),
        }
    }

    fn variant_of(&mut self, declared: DeclaredType, ty: Name, variant: Name) -> Named {
        if declared.ty == TyId::ERROR {
            return self.said_elsewhere();
        }
        let definition = self.definition_of(declared.definition);
        if matches!(self.pass.table.get(definition), Ty::Error) {
            return self.said_elsewhere();
        }
        let is_enum = matches!(self.pass.table.get(definition), Ty::Enum(_));
        if is_enum && self.has_member(definition, variant) {
            return Named::Variant(Meaning::Variant { ty: declared.ty, variant });
        }
        Named::NoVariant(self.text(ty).to_owned())
    }

    fn report_not_imported(&mut self, library: String, id: Id) {
        if self.pass.current.not_imported.contains(&library) {
            return;
        }
        let fix = add_use(&self.pass.current.sorted_uses, &library);
        let message = format!("the library `{library}` is used without a `use` line in this file");
        self.pass.current.not_imported.insert(library);
        self.say_with(Code::Lc2005, message, id, Vec::new(), vec![fix]);
    }

    // Operands.

    fn value_operand(&mut self, operand: &'a Operand) {
        match operand {
            Operand::Path(path) => self.value_path(path),
            Operand::Number(id, numeral, unit) => self.judge_number(*numeral, unit.as_deref(), *id),
            Operand::Array(_, values) => {
                for value in values {
                    self.value_operand(value);
                }
            }
            Operand::Bracket(id, unit) => self.bracket_as_value(*id, unit),
            Operand::Text(..) | Operand::Bool(..) | Operand::None(_) => {}
        }
    }

    /// A value first, so a value can bear the name of a function; then a function reference, or a variant.
    fn value_path(&mut self, path: &'a Path) {
        if let Some(value) = self.read(path.first) {
            self.resolved.set(path.id, Meaning::Value(value.id));
            return;
        }
        let names = self.names();
        let unknown =
            |path: &Path| format!("unknown value `{}`: nothing in this body, its signature or its library has this name", path_text(path, names));
        match self.find_named(path) {
            Named::Function(meaning, _) | Named::Variant(meaning) => self.resolved.set(path.id, meaning),
            Named::NoVariant(ty) => {
                let text = path_text(path, names);
                self.say(Code::Lc2007, format!("unknown variant `{text}`: `{ty}` has no such variant"), path.id);
            }
            Named::NoFunction(library) => {
                let text = path_text(path, names);
                self.say(Code::Lc2002, format!("unknown function `{text}`: the library `{library}` has no such function"), path.id);
            }
            Named::IsType => {
                let text = path_text(path, names);
                self.say(Code::Lc2001, format!("`{text}` is a type, and a value is expected here"), path.id);
            }
            Named::NotImported { library, known: true } => self.report_not_imported(library, path.id),
            Named::NotImported { library, known: false } => {
                let message = format!("{}, unless `{library}` is a library, which this file does not import", unknown(path));
                self.say(Code::Lc2001, message, path.id);
            }
            Named::Unknown => self.say(Code::Lc2001, unknown(path), path.id),
            Named::AlreadySaid => {}
        }
    }

    /// `[x]` is an array of one value and `[1]` of one number, for a parameter that takes a value (R-5).
    fn bracket_as_value(&mut self, id: Id, unit: &'a UnitExpr) {
        if unit.rest.is_empty() {
            match &unit.first {
                UnitTerm::Named { name, exponent: None } => {
                    if let Some(value) = self.read(*name) {
                        self.resolved.set(id, Meaning::ArrayOfValue(value.id));
                        return;
                    }
                    match self.find_in(self.scope.library, *name, Named::Unknown) {
                        Named::Function(Meaning::Function { library, name }, _) => self.resolved.set(id, Meaning::ArrayOfFunction { library, name }),
                        Named::AlreadySaid => {}
                        _ => {
                            let text = self.text(*name);
                            self.say(
                                Code::Lc2001,
                                format!("unknown value `{text}`: this parameter takes a value, so `[{text}]` is an array of one (R-5)"),
                                id,
                            );
                        }
                    }
                    return;
                }
                UnitTerm::One { exponent: None } => {
                    self.resolved.set(id, Meaning::ArrayOfNumber);
                    return;
                }
                _ => {}
            }
        }
        self.say(
            Code::Lc3001,
            "this parameter takes a value, not a unit: a unit is written in brackets only for a parameter of kind unit (R-5)".to_owned(),
            id,
        );
    }

    // Default values: their form and their names (R-21). Their types are for pass 4.

    fn judge_constant(&mut self, constant: &'a Constant) {
        match constant {
            Constant::Operand(operand) => self.constant_operand(operand),
            Constant::Call(call) => self.constant_call(call),
            Constant::Struct(literal) => {
                for field in &literal.fields {
                    self.constant_operand(&field.value);
                }
                self.struct_type(literal);
            }
        }
    }

    fn not_a_constant(&mut self, id: Id, what: &str) {
        self.say(Code::Lc3017, format!("{what}: a default value is a constant, which reads nothing of the diagram (R-21)"), id);
    }

    fn constant_operand(&mut self, operand: &'a Operand) {
        match operand {
            Operand::Path(path) => self.constant_path(path),
            Operand::Number(id, numeral, unit) => self.judge_number(*numeral, unit.as_deref(), *id),
            Operand::Array(_, values) => {
                for value in values {
                    self.constant_operand(value);
                }
            }
            Operand::Bracket(id, unit) => self.constant_bracket(*id, unit),
            Operand::Text(..) | Operand::Bool(..) | Operand::None(_) => {}
        }
    }

    /// `[1]` and `[f]` are arrays of one, for a value of a variant (R-5). Nothing else in brackets is a constant.
    fn constant_bracket(&mut self, id: Id, unit: &UnitExpr) {
        if unit.rest.is_empty() {
            match &unit.first {
                UnitTerm::One { exponent: None } => {
                    self.resolved.set(id, Meaning::ArrayOfNumber);
                    return;
                }
                UnitTerm::Named { name, exponent: None } => match self.find_in(self.scope.library, *name, Named::Unknown) {
                    Named::Function(Meaning::Function { library, name }, _) => {
                        self.resolved.set(id, Meaning::ArrayOfFunction { library, name });
                        return;
                    }
                    Named::AlreadySaid => return,
                    _ => {}
                },
                _ => {}
            }
        }
        self.not_a_constant(id, "this names neither a number nor a function of this library");
    }

    fn constant_path(&mut self, path: &'a Path) {
        let names = self.names();
        match self.find_named(path) {
            Named::Function(meaning, _) | Named::Variant(meaning) => self.resolved.set(path.id, meaning),
            Named::NoVariant(ty) => {
                let text = path_text(path, names);
                self.say(Code::Lc2007, format!("unknown variant `{text}`: `{ty}` has no such variant"), path.id);
            }
            Named::NoFunction(library) => {
                let text = path_text(path, names);
                self.say(Code::Lc2002, format!("unknown function `{text}`: the library `{library}` has no such function"), path.id);
            }
            Named::NotImported { library, known: true } => self.report_not_imported(library, path.id),
            Named::AlreadySaid => {}
            Named::IsType => {
                let text = path_text(path, names);
                self.not_a_constant(path.id, &format!("`{text}` is a type"));
            }
            Named::Unknown | Named::NotImported { .. } => {
                let text = path_text(path, names);
                self.not_a_constant(path.id, &format!("`{text}` is neither a variant nor a function"));
            }
        }
    }

    /// A variant with its values. What it holds says nothing if the call is the mistake.
    fn constant_call(&mut self, call: &'a Call) {
        let names = self.names();
        let meaning = match self.find_named(&call.callee) {
            Named::Variant(meaning) => meaning,
            Named::NoVariant(ty) => {
                let text = path_text(&call.callee, names);
                self.say(Code::Lc2007, format!("unknown variant `{text}`: `{ty}` has no such variant"), call.callee.id);
                return;
            }
            Named::NotImported { library, known: true } => {
                self.report_not_imported(library, call.callee.id);
                return;
            }
            Named::AlreadySaid => return,
            Named::Function(..) | Named::NoFunction(_) | Named::IsType | Named::Unknown | Named::NotImported { .. } => {
                let text = path_text(&call.callee, names);
                self.not_a_constant(call.id, &format!("`{text}(…)` is a call"));
                return;
            }
        };
        self.resolved.set(call.callee.id, meaning);
        if !call.type_args.is_empty() {
            self.not_a_constant(call.id, "a variant takes no type argument");
        }
        let mut position = 0_u32;
        let mut terminals = Vec::with_capacity(call.args.len());
        for arg in &call.args {
            match arg {
                Arg::After { id, .. } => {
                    self.not_a_constant(*id, "`after:` orders the nodes of a diagram");
                    terminals.push(Terminal::Order);
                }
                Arg::Value { id, name, value } => {
                    if let Some(name) = name {
                        let text = self.text(*name);
                        self.say(Code::Lc2009, format!("unknown named argument `{text}`: the values of a variant have no name"), *id);
                    }
                    self.constant_operand(value);
                    terminals.push(Terminal::Param(position));
                    position = position.saturating_add(1);
                }
            }
        }
        self.resolved.add_call(call.id, ResolvedCall { callee: meaning, terminals: terminals.into_boxed_slice(), type_args: Box::default() });
    }

    fn judge_number(&mut self, numeral: Numeral, unit: Option<&'a UnitExpr>, id: Id) {
        let names = self.names();
        match numeral.kind {
            NumeralKind::Int(digits) => {
                let text = names.resolve(digits);
                if !integer_fits(numeral.negative, text) {
                    self.say(Code::Lc1006, "this integer is out of bounds: no type holds it, beyond `i64` and `u64`".to_owned(), id);
                }
            }
            NumeralKind::Float(digits) => {
                if !float_fits(names.resolve(digits)) {
                    self.say(Code::Lc1006, "this number is out of bounds: it is too large for `f64`".to_owned(), id);
                }
            }
            NumeralKind::Inf | NumeralKind::Nan => {}
        }
        if let Some(unit) = unit {
            self.judge_unit(unit);
        }
    }

    fn judge_bound(&mut self, bound: IntBound, id: Id) {
        if !integer_fits(bound.negative, self.text(bound.digits)) {
            self.say(Code::Lc1006, "this integer is out of bounds: no type holds it, beyond `i64` and `u64`".to_owned(), id);
        }
    }

    fn judge_unit(&mut self, unit: &UnitExpr) {
        if let Err(error) = unit_of(unit, self.names()) {
            let message = match &error {
                UnitError::Unknown(name) => format!("unknown unit `{name}`"),
                _ => "this unit is out of bounds: an exponent goes beyond 127, or the scale beyond 1e300".to_owned(),
            };
            self.say(error.code(), message, unit.id);
        }
    }

    // Calls.

    fn walk_call(&mut self, call: &'a Call, targets: Option<(usize, Id)>) {
        let type_args: Box<[TyId]> = call.type_args.iter().map(|ty| self.written_type(ty, call.id)).collect();
        match self.find_named(&call.callee) {
            Named::Function(meaning, callable) => self.bind_function(call, meaning, &callable, type_args, targets),
            Named::Variant(meaning) => self.bind_variant(call, meaning, type_args, targets),
            Named::NoVariant(ty) => {
                let text = path_text(&call.callee, self.names());
                self.say(Code::Lc2007, format!("unknown variant `{text}`: `{ty}` has no such variant"), call.callee.id);
                self.bind_nothing(call, type_args);
            }
            Named::NoFunction(library) => {
                let text = path_text(&call.callee, self.names());
                self.say(Code::Lc2002, format!("unknown function `{text}`: the library `{library}` has no such function"), call.callee.id);
                self.bind_nothing(call, type_args);
            }
            Named::IsType => {
                let text = path_text(&call.callee, self.names());
                self.say(
                    Code::Lc2002,
                    format!("`{text}` is a type, not a function: a struct is built with `{{ }}`, and an enumeration by a variant"),
                    call.callee.id,
                );
                self.bind_nothing(call, type_args);
            }
            Named::NotImported { library, .. } => {
                if self.read(call.callee.first).is_some() {
                    self.report_a_value_called(&call.callee);
                } else {
                    self.report_not_imported(library, call.callee.id);
                }
                self.bind_nothing(call, type_args);
            }
            Named::Unknown => {
                if self.read(call.callee.first).is_some() {
                    self.report_a_value_called(&call.callee);
                } else {
                    self.report_unknown_function(&call.callee);
                }
                self.bind_nothing(call, type_args);
            }
            Named::AlreadySaid => self.bind_nothing(call, type_args),
        }
    }

    fn report_a_value_called(&mut self, path: &Path) {
        let text = path_text(path, self.names());
        self.say(Code::Lc2002, format!("unknown function `{text}`: this is a value, and a function reference runs with `core.call` (4)"), path.id);
    }

    /// For a name of `core` written without its prefix, the fix qualifies it, with the `use` that it needs (`docs/cli.md`, 4.2).
    fn report_unknown_function(&mut self, path: &Path) {
        let text = path_text(path, self.names());
        let mut fixes = Vec::new();
        if self.context.primitives.is_in_core(path.first)
            && path.rest.is_empty()
            && let Some(span) = self.scope.model.source_map.span(path.id)
        {
            let qualified = format!("core.{text}");
            let mut edits = vec![Edit { span, text: qualified.clone() }];
            if !self.scope.file.imports.contains_key("core") {
                edits.extend(add_use(&self.pass.current.sorted_uses, "core").edits);
            }
            fixes.push(Fix { title: format!("Call `{qualified}`"), edits });
        }
        self.say_with(Code::Lc2002, format!("unknown function `{text}`"), path.id, Vec::new(), fixes);
    }

    /// The arguments of a call that an error left unbound still read their names. A unit in brackets says nothing, as it
    /// may be a unit.
    fn bind_nothing(&mut self, call: &'a Call, type_args: Box<[TyId]>) {
        let mut after = false;
        let mut terminals = Vec::with_capacity(call.args.len());
        for arg in &call.args {
            match arg {
                Arg::After { id, refs } => {
                    self.walk_after(*id, refs, &mut after);
                    terminals.push(Terminal::Order);
                }
                Arg::Value { value, .. } => {
                    if !matches!(value, Operand::Bracket(..)) {
                        self.value_operand(value);
                    }
                    terminals.push(Terminal::Free);
                }
            }
        }
        self.resolved.add_call(call.id, ResolvedCall { callee: Meaning::None, terminals: terminals.into_boxed_slice(), type_args });
    }

    fn bind_variant(&mut self, call: &'a Call, meaning: Meaning, type_args: Box<[TyId]>, targets: Option<(usize, Id)>) {
        self.resolved.set(call.callee.id, meaning);
        if !call.type_args.is_empty() {
            self.say(Code::Lc3019, "a variant takes no type argument".to_owned(), call.id);
        }
        let mut after = false;
        let mut position = 0_u32;
        let mut terminals = Vec::with_capacity(call.args.len());
        for arg in &call.args {
            match arg {
                Arg::After { id, refs } => {
                    self.walk_after(*id, refs, &mut after);
                    terminals.push(Terminal::Order);
                }
                Arg::Value { id, name, value } => {
                    if let Some(name) = name {
                        let text = self.text(*name);
                        self.say(Code::Lc2009, format!("unknown named argument `{text}`: the values of a variant have no name"), *id);
                    } else if after {
                        self.say(Code::Lc2104, "a positional argument comes before the named ones (4)".to_owned(), *id);
                    }
                    self.value_operand(value);
                    terminals.push(Terminal::Param(position));
                    position = position.saturating_add(1);
                }
            }
        }
        self.resolved.add_call(call.id, ResolvedCall { callee: meaning, terminals: terminals.into_boxed_slice(), type_args });
        if let Some((count, id)) = targets {
            self.judge_targets(id, count, 1, "a variant construction");
        }
    }

    fn bind_function(&mut self, call: &'a Call, meaning: Meaning, callable: &Callable, type_args: Box<[TyId]>, targets: Option<(usize, Id)>) {
        self.resolved.set(call.callee.id, meaning);
        if let Meaning::Function { library, name } = meaning
            && library as usize == self.scope.library
        {
            self.calls.entry(name).or_insert(call.callee.id);
        }
        if let Some(generics) = callable.generics
            && !type_args.is_empty()
            && type_args.len() != generics
        {
            self.say(Code::Lc3019, format!("this function takes {generics} type arguments, and has {} here", type_args.len()), call.id);
        }
        let terminals = self.bind_arguments(call, callable);
        self.resolved.add_call(call.id, ResolvedCall { callee: meaning, terminals, type_args });
        if let (Some((count, id)), Some(outputs), false) = (targets, callable.outputs, callable.faulty) {
            self.judge_targets(id, count, outputs, "this function");
        }
    }

    /// Positional, then named (spec 4). Parameters after a variadic one are wired by name only (R-11). One diagnostic for
    /// what is missing, and the work stays in the arguments that the call has.
    fn bind_arguments(&mut self, call: &'a Call, callable: &Callable) -> Box<[Terminal]> {
        let mut terminals = Vec::with_capacity(call.args.len());
        let mut wiring = Wiring::default();
        let (mut next_position, mut named_seen, mut after, mut too_many) = (0_u32, false, false, false);
        for arg in &call.args {
            match arg {
                Arg::After { id, refs } => {
                    self.walk_after(*id, refs, &mut after);
                    named_seen = true;
                    terminals.push(Terminal::Order);
                }
                Arg::Value { id, name: None, value } => {
                    if named_seen {
                        self.say(Code::Lc2104, "a positional argument comes before the named ones (4)".to_owned(), *id);
                        wiring.mistaken = true;
                        self.argument_of_unknown_kind(value);
                        terminals.push(Terminal::Free);
                        continue;
                    }
                    let position = match callable.variadic {
                        Some(variadic) if next_position >= variadic => Some(variadic),
                        _ if (next_position as usize) < callable.params.len() => Some(next_position),
                        _ => None,
                    };
                    match position {
                        Some(position) => {
                            next_position = next_position.saturating_add(1);
                            self.argument(position, callable, value);
                            terminals.push(Terminal::Param(position));
                        }
                        None if callable.open => {
                            self.value_operand(value);
                            terminals.push(Terminal::Free);
                        }
                        None => {
                            if !too_many && !callable.faulty {
                                let count = callable.params.len();
                                self.say(Code::Lc2102, format!("too many arguments: this function has {count} inputs"), *id);
                            }
                            too_many = true;
                            wiring.mistaken = true;
                            self.argument_of_unknown_kind(value);
                            terminals.push(Terminal::Free);
                        }
                    }
                }
                Arg::Value { id, name: Some(name), value } => {
                    named_seen = true;
                    match self.named_argument(*id, *name, callable, next_position, &mut wiring) {
                        Wired::Param(position) => {
                            self.argument(position, callable, value);
                            terminals.push(Terminal::Param(position));
                        }
                        Wired::Free => {
                            self.value_operand(value);
                            terminals.push(Terminal::Free);
                        }
                        Wired::Mistake => {
                            wiring.mistaken = true;
                            self.argument_of_unknown_kind(value);
                            terminals.push(Terminal::Free);
                        }
                    }
                }
            }
        }
        self.judge_missing(call, callable, next_position, &wiring);
        terminals.into_boxed_slice()
    }

    /// What this name wires. An open function takes any name that its signature does not have, once each.
    fn named_argument(&mut self, id: Id, name: Name, callable: &Callable, positional: u32, wiring: &mut Wiring) -> Wired {
        let text = self.text(name);
        let Some(position) = callable.by_name.get(&name).copied() else {
            if callable.open {
                if wiring.free.insert(name) {
                    return Wired::Free;
                }
                self.say(Code::Lc2102, format!("the input `{text}` is wired twice"), id);
                return Wired::Mistake;
            }
            if !callable.faulty {
                self.say(Code::Lc2009, format!("unknown named argument `{text}`: this function has no input of that name"), id);
            }
            return Wired::Mistake;
        };
        if callable.variadic == Some(position) {
            self.say(Code::Lc2009, format!("`{text}` is variadic, and takes positional arguments only (5.4)"), id);
            return Wired::Mistake;
        }
        let by_position = callable.variadic.is_none_or(|variadic| position < variadic) && position < positional;
        if by_position || !wiring.named.insert(position) {
            if !callable.faulty {
                self.say(Code::Lc2102, format!("the input `{text}` is wired twice"), id);
            }
            return Wired::Mistake;
        }
        Wired::Param(position)
    }

    /// `required` is in order, so this looks at no more than the arguments of the call, and one more.
    fn judge_missing(&mut self, call: &Call, callable: &Callable, positional: u32, wiring: &Wiring) {
        if callable.faulty || wiring.mistaken {
            return;
        }
        let is_wired = |position: u32| {
            wiring.named.contains(&position) || (position < positional && callable.variadic.is_none_or(|variadic| position < variadic))
        };
        let Some(first) = callable.required.iter().copied().find(|position| !is_wired(*position)) else { return };
        let name = callable.params.get(first as usize).map_or("", |(name, _)| self.text(*name));
        // Cut here: the name is not in the call, so a long one and many calls would copy it each time.
        self.say(Code::Lc2101, format!("the required input `{}` is not wired", cut_text(name)), call.id);
    }

    /// A unit in brackets says nothing here: what it would have been stays unknown.
    fn argument_of_unknown_kind(&mut self, value: &'a Operand) {
        if !matches!(value, Operand::Bracket(..)) {
            self.value_operand(value);
        }
    }

    fn argument(&mut self, position: u32, callable: &Callable, value: &'a Operand) {
        let kind = callable.params.get(position as usize).map_or(ParamKind::Value, |(_, kind)| *kind);
        match kind {
            ParamKind::Value => self.value_operand(value),
            ParamKind::Unit => match value {
                Operand::Bracket(id, unit) => {
                    self.resolved.set(*id, Meaning::Unit);
                    self.judge_unit(unit);
                }
                other => self.say(Code::Lc3001, "this parameter takes a unit, written in brackets: `[V]` (R-5)".to_owned(), other.id()),
            },
            ParamKind::Category => match value {
                Operand::Path(path) => self.resolved.set(path.id, Meaning::Category),
                other => self.say(Code::Lc3001, "this parameter takes an error category, such as `io.timeout`".to_owned(), other.id()),
            },
            ParamKind::Element => match value {
                Operand::Path(path) => self.panel_element(path),
                other => self.say(Code::Lc3001, "this parameter takes the name of a panel element".to_owned(), other.id()),
            },
        }
    }

    fn panel_element(&mut self, path: &Path) {
        match self.elements.get(&path.first).copied().filter(|_| path.rest.is_empty()) {
            Some(element) => self.resolved.set(path.id, Meaning::Element(element)),
            None => {
                let text = path_text(path, self.names());
                self.say(Code::Lc2010, format!("unknown panel element `{text}`: the `panel` section of this function does not declare it"), path.id);
            }
        }
    }

    /// A value, a label, or a list of them (R-15).
    fn walk_after(&mut self, id: Id, refs: &'a [OrderRef], after: &mut bool) {
        if *after {
            self.say(Code::Lc2105, "a call has one `after:`, which can take a list: `after: (x, #label)` (R-15)".to_owned(), id);
        }
        *after = true;
        for order in refs {
            match order {
                OrderRef::Label(label) => match self.labels.get(&label.name).copied() {
                    Some(target) => self.resolved.set(label.id, Meaning::Label(target)),
                    None => {
                        let text = self.text(label.name);
                        let where_it_is =
                            if self.all_labels.contains_key(&label.name) { ": it is in a body that this one is not inside" } else { "" };
                        self.say(Code::Lc2011, format!("unknown label `#{text}` in `after:`{where_it_is}"), label.id);
                    }
                },
                // A value comes from this file, whatever the others hold.
                OrderRef::Value(Operand::Path(path)) => match self.read(path.first) {
                    Some(value) => self.resolved.set(path.id, Meaning::Value(value.id)),
                    None => {
                        let text = path_text(path, self.names());
                        self.say(Code::Lc2001, format!("unknown value `{text}` in `after:`"), path.id);
                    }
                },
                OrderRef::Value(other) => {
                    self.say(Code::Lc2105, "`after:` waits for a value or a label, not a literal (R-15)".to_owned(), other.id());
                }
            }
        }
    }
}
