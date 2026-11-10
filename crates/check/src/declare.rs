//! Pass 2 (`docs/architecture.md`, section 5): what each library declares, its functions and its types, with what can
//! be wrong in a declaration alone. Passes 3 and 4, in `body`, read the bodies with the same means.
//!
//! Walking a `TypeExpr` recurses, as far as the nesting that `format::read` let through, on the stack that it took. Going
//! from a type to another never does, since a chain of `.lct` files is as long as the program wants.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use labcraft_format::is_name;
use labcraft_graph::{Decl, EnumType, FnType, Id, NamedType, Path, Signature, StructType, TypeDecl, TypeExpr, UnitExpr, UnitOp, UnitTerm};
use labcraft_types::{
    Code, Diagnostic, Edit, Fix, FnParam, Interner, LibraryId, Limits, MAX_ARRAY_DIMENSIONS, MAX_NESTING, Name, Node, NumberKind, Position, Related,
    Report, Span, Ty, TyId, TypeTable, Unit, UnitError, Variant, cut_text, normalize,
};

use self::body::Primitives;
use crate::file_name::{case_key, file_name, kept_by_windows, stem};
use crate::load::{Library, LoadedFile, Model, Target, at, cycles, last_resort};
use crate::resolved::Resolved;
use crate::{Libraries, Origin};

mod body;

pub use self::body::{Align, Format, Piece};
pub(crate) use self::body::{read_escapes, template_fields, template_pieces};
#[cfg(test)]
mod tests;

/// What a library declares, as the passes above read it.
#[derive(Debug)]
pub struct DeclaredLibrary {
    /// With its segments separated by `.`: empty for the folder given.
    pub name: String,
    pub id: LibraryId,
    pub types: HashMap<Name, DeclaredType>,
    pub functions: HashMap<Name, DeclaredFunction>,
    /// The file of each type of `types`, by its index in `Checked::files`: for a run, those of every library; otherwise those
    /// of the folder given only.
    pub type_files: HashMap<Name, u32>,
    /// In the folder given or one of its sub-libraries, rather than a dependency.
    pub checked: bool,
    /// What its manifest says, normalized (R-20), in its order: the native functions that run even when `err` holds an error (7.2), the
    /// opaque types that are resources (8.1), and the categories of its errors (7.1). Only a shipped library has one in M0.
    pub always: Vec<Name>,
    pub resources: Vec<Name>,
    pub categories: Vec<String>,
    /// A file of it did not read, or a bug stopped one: a name that it does not seem to have may still be its own, and
    /// that has its diagnostic.
    pub broken: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct DeclaredType {
    /// What a program means by its name: an alias is the type it names (3.2), and only a declared `struct` or `enum`, or an
    /// opaque type, is a type of its own.
    pub ty: TyId,
    /// What it stands for: `ty` for an opaque type or an alias, which have nothing else to say.
    pub definition: TyId,
    /// An opaque type or an alias of one, whose name a value can bear (R-1).
    pub opaque: bool,
}

#[derive(Debug)]
pub struct DeclaredFunction {
    pub params: Vec<DeclaredParam>,
    pub outputs: Vec<DeclaredOutput>,
    /// With an `extern` line, even one that runs from M0.
    pub external: bool,
    /// With type variables, which come with a later milestone (`LC4006`).
    pub generic: bool,
    /// Its signature has an error of its own (`LC2019`, `LC2105`, `LC2107`) or its name is defined twice (`LC2013`), so a
    /// call says nothing more about what it wires.
    pub faulty: bool,
    /// Its file, by its index in `Checked::files`: for a run, in every library; otherwise in the folder given only.
    pub file: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
pub struct DeclaredParam {
    pub name: Name,
    pub ty: TyId,
    pub has_default: bool,
    pub variadic: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct DeclaredOutput {
    pub name: Name,
    pub ty: TyId,
}

pub(crate) struct Declared {
    pub(crate) types: TypeTable,
    pub(crate) libraries: Vec<DeclaredLibrary>,
    /// Pass 3, parallel to the files of each library.
    pub(crate) resolved: Vec<Vec<Resolved>>,
    /// For a run, what the bodies of the dependencies say.
    pub(crate) bodies: Report,
    pub(crate) cut_short: bool,
}

/// What a path in a type stands for.
enum Lookup {
    Base(Ty),
    Waveform,
    /// `queue`, `ref`, `event`, `notifier`, `map` and `set`, for a later milestone (`LC4006`).
    Later,
    /// A contextual word, which means something only as the whole of a `type` declaration.
    Opaque,
    Declared(usize),
    Unknown,
    /// A library that the file does not import, by the name that it would take.
    NotImported(String),
    /// Unknown because of an error that has its own diagnostic.
    AlreadySaid,
}

/// A name that the language gives to a type (`LC2022`). `Number` is not one: it appears only as a bound.
fn language_type(text: &str) -> Option<Lookup> {
    let number = |kind| Some(Lookup::Base(Ty::Number { kind, unit: Unit::ONE }));
    match text {
        "bool" => Some(Lookup::Base(Ty::Bool)),
        "i8" => number(NumberKind::I8),
        "i16" => number(NumberKind::I16),
        "i32" => number(NumberKind::I32),
        "i64" => number(NumberKind::I64),
        "u8" => number(NumberKind::U8),
        "u16" => number(NumberKind::U16),
        "u32" => number(NumberKind::U32),
        "u64" => number(NumberKind::U64),
        "f32" => number(NumberKind::F32),
        "f64" => number(NumberKind::F64),
        "c64" => number(NumberKind::C64),
        "c128" => number(NumberKind::C128),
        "string" => Some(Lookup::Base(Ty::String)),
        "bytes" => Some(Lookup::Base(Ty::Bytes)),
        "timestamp" => Some(Lookup::Base(Ty::Timestamp)),
        "duration" => Some(Lookup::Base(Ty::Duration)),
        "error" => Some(Lookup::Base(Ty::ErrorValue)),
        "waveform" => Some(Lookup::Waveform),
        "queue" | "ref" | "event" | "notifier" | "map" | "set" => Some(Lookup::Later),
        "opaque" => Some(Lookup::Opaque),
        _ => None,
    }
}

/// Whether a type is opaque, as far as its declaration can tell.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `type session = opaque`, or an alias of it: the index of the type at the end of the chain.
    Opaque(usize),
    Plain,
    /// An error in the chain has its own diagnostic, and what it would have been stays unknown.
    Unknown,
}

struct TypeInfo<'a> {
    library: usize,
    file: &'a LoadedFile,
    model: &'a Model,
    decl: &'a TypeDecl,
}

/// Where a type is written: what it can name, and who is told about it.
#[derive(Clone, Copy)]
struct Scope<'a> {
    library: usize,
    file: &'a LoadedFile,
    model: &'a Model,
    /// In a generic declaration, refused for now (`LC4006`): a name that means nothing may be one of its variables.
    generic: bool,
    function: Option<&'a str>,
}

/// What a library imports, in any of its files: the first `use` of each first segment, in the order of the paths.
type Imports<'a> = HashMap<&'a str, (&'a LoadedFile, Id)>;

struct Pass<'a> {
    loaded: &'a [Library],
    libraries: &'a dyn Libraries,
    names: &'a Interner,
    limits: &'a Limits,
    table: TypeTable,
    reports: Vec<Report>,
    ids: Vec<LibraryId>,
    types: Vec<TypeInfo<'a>>,
    /// The types of each library, by name: the first one when a name is defined twice.
    index: Vec<HashMap<Name, usize>>,
    /// The names that a type of each library bears against `LC2022`: that mistake has its diagnostic.
    refused: Vec<HashSet<Name>>,
    /// The names that a file of each library declares under its own name (R-22): another file that declares one of
    /// them is a copy that has its `LC2023`, not a second definition.
    named: Vec<HashSet<(Name, bool)>>,
    /// A file that does not read may declare what a name stands for.
    broken: Vec<bool>,
    imports: Vec<Imports<'a>>,
    /// The types that each type names.
    mentions: Vec<Vec<usize>>,
    /// `None` until it is found.
    kinds: Vec<Option<Kind>>,
    /// Levels of nesting, through the other types: `None` until it is counted, `Some(None)` when an error stopped the count.
    depths: Vec<Option<Option<u32>>>,
    /// Each type after those it names, and the members of a cycle last: what names a broken type is told so.
    order: Vec<usize>,
    /// The members of a cycle, until they join `order`.
    in_cycles: Vec<usize>,
    /// `None` until it is resolved, or if a bug stopped it.
    resolved: Vec<Option<DeclaredType>>,
    current: CurrentFile,
    /// What each function file has said of `LC2005` in its signature, for its body, by the file as shown.
    said: HashMap<&'a str, HashSet<String>>,
    /// The line of a body that a type is written on, for what that type has to say.
    node: Option<Node>,
    /// How many errors its library had when the function being read began: a mistake in it can be a `ui.set` that nothing could bind.
    function_errors: u64,
    /// A budget of work or a limit made a pass say less (`Checked::cut_short`).
    cut_short: bool,
    /// The bodies of the dependencies are checked too, for a run (`docs/architecture.md`, 10.1).
    for_run: bool,
    /// The body being read leaves something unknown because of a mistake that has its diagnostic elsewhere.
    leaned: bool,
    /// For a run, what the bodies of the dependencies say, kept apart (`Checked::bodies`).
    bodies: Report,
    /// The output types of a function, by its library and its name, and of a function type, made once for the calls written
    /// alone (`Resolved::outputs_of`): a call of four bytes to a function of a million outputs costs nothing more.
    function_outputs: HashMap<(u32, Name), Arc<[TyId]>>,
    type_outputs: HashMap<TyId, Arc<[TyId]>>,
}

/// What the declaration of one file needs of it, made once: a file can hold millions of `use` lines and of types.
#[derive(Default)]
struct CurrentFile {
    /// The first `use` of each first segment.
    imports: HashMap<Name, Id>,
    /// Each `use` line by its text, with its line.
    sorted_uses: Vec<(String, u32)>,
    /// The libraries that have their `LC2005`: one mistake, however many times the file names them.
    not_imported: HashSet<String>,
}

pub(crate) fn declare(
    loaded: &mut [Library],
    libraries: &dyn Libraries,
    names: &mut Interner,
    limits: &Limits,
    report: &mut Report,
    for_run: bool,
) -> Declared {
    // Each library has its own limit, so the table only keeps the ceiling that a stack is sized from.
    let mut table = TypeTable::new(MAX_NESTING);
    let ids: Vec<LibraryId> = loaded.iter().map(|library| table.add_library(names.intern(&library.name))).collect();
    let primitives = Primitives::new(names);
    let mut pass = Pass {
        loaded: &*loaded,
        libraries,
        names: &*names,
        limits,
        table,
        reports: loaded.iter().map(|_| Report::for_limits(limits)).collect(),
        ids,
        types: Vec::new(),
        index: Vec::new(),
        refused: Vec::new(),
        named: Vec::new(),
        broken: Vec::new(),
        imports: Vec::new(),
        mentions: Vec::new(),
        kinds: Vec::new(),
        depths: Vec::new(),
        order: Vec::new(),
        in_cycles: Vec::new(),
        resolved: Vec::new(),
        current: CurrentFile::default(),
        said: HashMap::new(),
        node: None,
        function_errors: 0,
        cut_short: false,
        for_run,
        leaned: false,
        bodies: Report::for_limits(limits),
        function_outputs: HashMap::new(),
        type_outputs: HashMap::new(),
    };
    pass.gather();
    pass.find_cycles();
    pass.find_kinds();
    pass.measure();
    pass.resolve_types();
    let declared = pass.declare_all();
    let resolved = pass.check_bodies(&declared, &primitives);
    let Pass { table, reports, cut_short, bodies, .. } = pass;
    for (library, own) in loaded.iter_mut().zip(reports) {
        if library.checked {
            report.merge(own);
        } else if !library.checked_elsewhere {
            library.report.merge(own);
        }
    }
    Declared { types: table, libraries: declared, resolved, bodies, cut_short }
}

/// Of two declarations of one name, which have their `LC2013`, a call binds to the first one and says nothing about it.
/// A copy in a file of another name (`copy`) has its `LC2023` instead, and leaves the first one as it is.
fn keep_the_first(functions: &mut HashMap<Name, DeclaredFunction>, name: Name, declared: DeclaredFunction, copy: bool) {
    match functions.entry(name) {
        Entry::Occupied(_) if copy => {}
        Entry::Occupied(mut first) => first.get_mut().faulty = true,
        Entry::Vacant(slot) => {
            slot.insert(declared);
        }
    }
}

/// What the files of a library already take: by `case_key`, their names, extension included, and the names they declare,
/// by kind; and the names they declare as they are, which a declaration of the other kind cannot bear (`LC2013`).
#[derive(Default)]
struct Taken {
    files: HashMap<String, usize>,
    declared: HashMap<(bool, String), usize>,
    exact: HashMap<String, usize>,
}

/// Whether the file bears the name of what it declares (R-22).
fn well_named(file: &LoadedFile, model: &Model, names: &Interner) -> bool {
    let (name, _, _) = name_of_decl(&model.file.decl);
    stem(&file.shown) == names.resolve(name)
}

/// In the order of the paths, but the files that bear the name of what they declare first: a call binds to one of them
/// rather than to a copy under another name.
fn sorted_files<'a>(library: &'a Library, names: &Interner) -> Vec<&'a LoadedFile> {
    sorted_files_at(library, names).into_iter().map(|(_, file)| file).collect()
}

/// As [`sorted_files`], with the index of each file in its library.
fn sorted_files_at<'a>(library: &'a Library, names: &Interner) -> Vec<(usize, &'a LoadedFile)> {
    let mut files: Vec<(bool, usize, &LoadedFile)> = library
        .files
        .iter()
        .enumerate()
        .map(|(index, file)| (file.model.as_ref().is_some_and(|model| !well_named(file, model, names)), index, file))
        .collect();
    files.sort_by(|(a_copy, _, a), (b_copy, _, b)| a_copy.cmp(b_copy).then_with(|| a.shown.cmp(&b.shown)));
    files.into_iter().map(|(_, index, file)| (index, file)).collect()
}

/// Its name, where it is, and whether it is a type.
fn name_of_decl(decl: &Decl) -> (Name, Id, bool) {
    match decl {
        Decl::Function(function) => (function.name, function.id, false),
        Decl::Extern(function) => (function.name, function.id, false),
        Decl::Type(ty) => (ty.name, ty.id, true),
    }
}

fn path_text(path: &Path, names: &Interner) -> String {
    let texts: Vec<&str> = path.segments().map(|segment| names.resolve(segment)).collect();
    texts.join(".")
}

fn span_of(file: &LoadedFile, id: Id) -> Option<Span> {
    file.model.as_ref().and_then(|model| model.source_map.span(id))
}

/// The `use` line to add, in its sorted place. A file without any gets a section of its own after its header.
fn add_use(sorted_uses: &[(String, u32)], library: &str) -> Fix {
    let next = sorted_uses.partition_point(|(text, _)| text.as_str() <= library);
    let (line, text) = match (sorted_uses.get(next), sorted_uses.last()) {
        (Some((_, line)), _) => (*line, format!("use {library}\n")),
        (None, Some((_, line))) => (line.saturating_add(1), format!("use {library}\n")),
        (None, None) => (2, format!("\nuse {library}\n")),
    };
    let place = Position { line, column: 1 };
    Fix { title: format!("Add `use {library}`"), edits: vec![Edit { span: Span { start: place, end: place }, text }] }
}

/// The number of dimensions, saturating: whatever is too long to read is out of bounds anyway.
fn dimensions_of(text: &str) -> Option<u8> {
    let value = text.chars().filter_map(|c| c.to_digit(10)).fold(0_u32, |value, digit| value.saturating_mul(10).saturating_add(digit));
    if value == 0 || value > MAX_ARRAY_DIMENSIONS {
        return None;
    }
    u8::try_from(value).ok()
}

/// A `UnitExpr` has the shape that `parse_unit` reads from a text: equal precedence, from left to right.
pub(crate) fn unit_of(expr: &UnitExpr, names: &Interner) -> Result<Unit, UnitError> {
    let mut unit = term_of(&expr.first, names)?;
    for (op, term) in &expr.rest {
        let term = term_of(term, names)?;
        let product = match op {
            UnitOp::Star | UnitOp::MiddleDot => unit.mul(&term),
            UnitOp::Slash => unit.div(&term),
        };
        unit = product.ok_or(UnitError::OutOfBounds)?;
    }
    Ok(unit)
}

fn term_of(term: &UnitTerm, names: &Interner) -> Result<Unit, UnitError> {
    let (unit, exponent) = match term {
        UnitTerm::Named { name, exponent } => {
            let text = names.resolve(*name);
            (Unit::from_name(text).ok_or_else(|| UnitError::Unknown(text.to_owned()))?, exponent)
        }
        UnitTerm::One { exponent } => (Unit::ONE, exponent),
        UnitTerm::Group(inner) => return unit_of(inner, names),
    };
    let Some(exponent) = exponent else { return Ok(unit) };
    let digits = names.resolve(exponent.digits);
    let value = digits.chars().filter_map(|c| c.to_digit(10)).fold(0_i32, |value, digit| value.saturating_mul(10).saturating_add(digit as i32));
    if value > i32::from(i8::MAX) {
        return Err(UnitError::OutOfBounds);
    }
    unit.pow(if exponent.negative { -value } else { value }).ok_or(UnitError::OutOfBounds)
}

impl<'a> Pass<'a> {
    fn push(&mut self, library: usize, diagnostic: Diagnostic) {
        if let Some(report) = self.reports.get_mut(library) {
            report.push(diagnostic);
        }
    }

    fn error(&mut self, scope: &Scope<'_>, code: Code, message: String, id: Id) {
        self.error_with(scope, code, message, id, Vec::new(), Vec::new());
    }

    fn error_with(&mut self, scope: &Scope<'_>, code: Code, message: String, id: Id, related: Vec<Related>, fixes: Vec<Fix>) {
        let mut diagnostic = at(code, message, &scope.file.shown, &scope.model.source_map, id);
        // Cut here, not by the report: a name of 8 MiB and millions of diagnostics would copy it each time.
        diagnostic.function = scope.function.map(|name| cut_text(name).into_owned());
        diagnostic.node = self.node.clone();
        diagnostic.related = related;
        diagnostic.fixes = fixes;
        self.push(scope.library, diagnostic);
    }

    /// `LC2013`, the names that a declaration cannot bear, and the list of the types.
    fn gather(&mut self) {
        let loaded = self.loaded;
        let names = self.names;
        for (library, source) in loaded.iter().enumerate() {
            let mut imports: Imports<'a> = HashMap::new();
            let mut broken = false;
            let mut places: HashMap<Name, Vec<(&'a LoadedFile, &'a Model)>> = HashMap::new();
            let mut order = Vec::new();
            let mut named = HashSet::new();
            let mut taken = Taken::default();
            for file in sorted_files(source, names) {
                *taken.files.entry(case_key(file_name(&file.shown))).or_default() += 1;
                let Some(model) = &file.model else {
                    broken = true;
                    continue;
                };
                let (declared, _, is_type) = name_of_decl(&model.file.decl);
                *taken.declared.entry((is_type, case_key(names.resolve(declared)))).or_default() += 1;
                *taken.exact.entry(names.resolve(declared).to_owned()).or_default() += 1;
                if well_named(file, model, names) {
                    named.insert((declared, is_type));
                }
                for line in &model.file.uses {
                    imports.entry(names.resolve(line.path.first)).or_insert((file, line.path.id));
                }
                let (name, _, _) = name_of_decl(&model.file.decl);
                let entry = places.entry(name).or_default();
                if entry.is_empty() {
                    order.push(name);
                }
                entry.push((file, model));
            }
            self.broken.push(broken);
            self.imports.push(imports);
            let mut index = HashMap::new();
            let mut refused = HashSet::new();
            let mut by_case: HashMap<(bool, String), Vec<(&'a LoadedFile, &'a Model)>> = HashMap::new();
            let mut case_order = Vec::new();
            for name in order {
                let list = places.remove(&name).unwrap_or_default();
                // A copy of a declaration that has its own file, of the same kind, has its `LC2023`, which says it all.
                let twice: Vec<_> = list
                    .iter()
                    .copied()
                    .filter(|(file, model)| well_named(file, model, names) || !named.contains(&(name, name_of_decl(&model.file.decl).2)))
                    .collect();
                self.report_defined_twice(library, name, &twice);
                for is_type in [false, true] {
                    let Some(&(file, model)) = list.iter().find(|(_, model)| name_of_decl(&model.file.decl).2 == is_type) else { continue };
                    let key = (is_type, case_key(names.resolve(name)));
                    let entry = by_case.entry(key.clone()).or_default();
                    if entry.is_empty() {
                        case_order.push(key);
                    }
                    entry.push((file, model));
                }
                for (file, model) in list {
                    self.judge_the_name(library, file, model, &taken);
                    let Decl::Type(decl) = &model.file.decl else { continue };
                    if language_type(names.resolve(name)).is_some() {
                        refused.insert(name);
                    } else if let Entry::Vacant(slot) = index.entry(name) {
                        slot.insert(self.types.len());
                        self.types.push(TypeInfo { library, file, model, decl });
                    }
                }
            }
            for key in case_order {
                let list = by_case.remove(&key).unwrap_or_default();
                self.report_same_but_case(library, &list);
            }
            self.index.push(index);
            self.refused.push(refused);
            self.named.push(named);
        }
    }

    /// Two names of one kind that only their case tells apart (R-22): once, on the last file in the order of the
    /// paths, with the others in `related`, as for `LC2013`.
    fn report_same_but_case(&mut self, library: usize, list: &[(&LoadedFile, &Model)]) {
        let mut list = list.to_vec();
        list.sort_by(|(a, _), (b, _)| a.shown.cmp(&b.shown));
        let Some(((last, model), others)) = list.split_last() else { return };
        if others.is_empty() {
            return;
        }
        let (name, id, is_type) = name_of_decl(&model.file.decl);
        let text = self.names.resolve(name);
        let kind = if is_type { "type" } else { "function" };
        let message = format!(
            "`{text}` differs from another {kind} of this library only by its case: their files cannot be side by side on \
             Windows or macOS (R-22)"
        );
        let mut diagnostic = at(Code::Lc2024, message, &last.shown, &model.source_map, id);
        diagnostic.function = self.function_at_fault(&model.file.decl);
        diagnostic.related = others
            .iter()
            .map(|(file, model)| {
                let (name, id, _) = name_of_decl(&model.file.decl);
                let other = self.names.resolve(name);
                Related { message: format!("`{other}` is declared here"), file: file.shown.clone(), span: model.source_map.span(id) }
            })
            .collect();
        self.push(library, diagnostic);
    }

    fn function_at_fault(&self, decl: &Decl) -> Option<String> {
        let (name, _, is_type) = name_of_decl(decl);
        (!is_type).then(|| cut_text(self.names.resolve(name)).into_owned())
    }

    /// Once, on the last file in the order of the paths, with the others in `related` (`docs/cli.md`, section 4.2).
    fn report_defined_twice(&mut self, library: usize, name: Name, list: &[(&LoadedFile, &Model)]) {
        let Some(((last, model), others)) = list.split_last() else { return };
        if others.is_empty() {
            return;
        }
        let text = self.names.resolve(name);
        let (_, id, _) = name_of_decl(&model.file.decl);
        let mut diagnostic = at(Code::Lc2013, format!("`{text}` is defined twice in this library"), &last.shown, &model.source_map, id);
        diagnostic.function = self.function_at_fault(&model.file.decl);
        diagnostic.related = others
            .iter()
            .map(|(file, model)| {
                let (_, id, _) = name_of_decl(&model.file.decl);
                Related { message: format!("`{text}` is also defined here"), file: file.shown.clone(), span: model.source_map.span(id) }
            })
            .collect();
        self.push(library, diagnostic);
    }

    /// `LC2022`, or `LC2014` against what any file of the library imports: a type is visible in all of them (R-1).
    fn judge_the_name(&mut self, library: usize, file: &'a LoadedFile, model: &'a Model, taken: &Taken) {
        let (name, id, is_type) = name_of_decl(&model.file.decl);
        let text = self.names.resolve(name);
        let kind = if is_type { "type" } else { "function" };
        if kept_by_windows(text) {
            let message =
                format!("Windows keeps `{text}` for a device: Git for Windows refuses a file of that name, so a {kind} cannot bear it (R-22)");
            let mut diagnostic = at(Code::Lc2025, message, &file.shown, &model.source_map, id);
            diagnostic.function = self.function_at_fault(&model.file.decl);
            self.push(library, diagnostic);
        } else if !well_named(file, model, self.names) {
            self.report_misnamed(library, file, model, taken);
        }
        if is_type && language_type(text).is_some() {
            let message = format!("`{text}` is a name that the language gives to a type, so a type cannot bear it");
            self.push(library, at(Code::Lc2022, message, &file.shown, &model.source_map, id));
            return;
        }
        let Some((imported_by, use_id)) = self.imports.get(library).and_then(|imports| imports.get(text)).copied() else { return };
        let message = format!("a {kind} cannot bear the name `{text}`: this library imports a library of that name (R-1)");
        let mut diagnostic = at(Code::Lc2014, message, &file.shown, &model.source_map, id);
        diagnostic.function = self.function_at_fault(&model.file.decl);
        diagnostic.related =
            vec![Related { message: format!("`{text}` is imported here"), file: imported_by.shown.clone(), span: span_of(imported_by, use_id) }];
        self.push(library, diagnostic);
    }

    /// A file that declares something of another name: copied or renamed outside the editor, which keeps the two together.
    /// Which of the two names was meant, only the user knows, so both fixes are offered, each when its name is free: a fix
    /// never leads to another error of R-22, nor to `LC2013`.
    fn report_misnamed(&mut self, library: usize, file: &LoadedFile, model: &Model, taken: &Taken) {
        let (name, id, is_type) = name_of_decl(&model.file.decl);
        let text = self.names.resolve(name);
        let (kind, extension) = if is_type { ("type", "lct") } else { ("function", "lcf") };
        let stem = stem(&file.shown);
        let shown_name = file_name(&file.shown);
        let message = format!("the file `{}` declares the {kind} `{text}`: a file bears the name of what it declares (R-22)", cut_text(shown_name));
        let mut diagnostic = at(Code::Lc2023, message, &file.shown, &model.source_map, id);
        diagnostic.function = self.function_at_fault(&model.file.decl);
        // This declaration counts once in what is taken, under its own name, and so does this file.
        let own = case_key(text);
        let same_kind = |key: &str| taken.declared.get(&(is_type, key.to_owned())).copied().unwrap_or(0) > usize::from(key == own);
        let exact = |name: &str| taken.exact.get(name).copied().unwrap_or(0);
        let imported = self.imports.get(library).is_some_and(|imports| imports.contains_key(stem.as_ref()));
        let refused = (is_type && language_type(&stem).is_some()) || imported || kept_by_windows(&stem);
        if is_name(&stem) && !refused && !same_kind(&case_key(&stem)) && exact(&stem) == 0 {
            diagnostic.fixes.push(Fix { title: format!("rename the {kind} to `{stem}`"), edits: Vec::new() });
        }
        let target = format!("{text}.{extension}");
        let target_key = case_key(&target);
        let file_free = taken.files.get(&target_key).copied().unwrap_or(0) <= usize::from(target_key == case_key(shown_name));
        // A declaration of the other kind with this very name would keep its `LC2013` after the rename.
        // Nor would a name that the language gives to a type lose its `LC2022`.
        if file_free && exact(text) <= 1 && !(is_type && language_type(text).is_some()) {
            diagnostic.fixes.push(Fix { title: format!("rename the file to `{target}`"), edits: Vec::new() });
        }
        if diagnostic.fixes.is_empty() {
            let title = format!("give this {kind} and its file a name of their own, or delete the file if it is a copy");
            diagnostic.fixes.push(Fix { title, edits: Vec::new() });
        }
        self.push(library, diagnostic);
    }

    fn lookup(&self, scope: &Scope<'_>, path: &Path) -> Lookup {
        let Some((last, head)) = path.rest.split_last() else {
            return self.lookup_simple(scope, path.first);
        };
        let mut library_name = self.names.resolve(path.first).to_owned();
        for segment in head {
            library_name.push('.');
            library_name.push_str(self.names.resolve(*segment));
        }
        match scope.file.imports.get(&library_name) {
            // `Verdict.Pass` where a type is expected: the first name is known, and it is no library.
            None if head.is_empty() && matches!(self.declared_in(scope.library, path.first), Lookup::Declared(_)) => Lookup::Unknown,
            None if head.is_empty() && self.is_refused(scope.library, path.first) => Lookup::AlreadySaid,
            None => Lookup::NotImported(library_name),
            // The primitives declare no type.
            Some(Target::Primitive) => Lookup::Unknown,
            Some(Target::Missing | Target::Unreadable) => Lookup::AlreadySaid,
            Some(Target::Library(target)) => self.declared_in(*target, *last),
        }
    }

    fn lookup_simple(&self, scope: &Scope<'_>, name: Name) -> Lookup {
        if self.is_refused(scope.library, name) {
            return Lookup::AlreadySaid;
        }
        if let Some(language) = language_type(self.names.resolve(name)) {
            return language;
        }
        match self.declared_in(scope.library, name) {
            Lookup::Unknown if scope.generic => Lookup::AlreadySaid,
            found => found,
        }
    }

    fn is_refused(&self, library: usize, name: Name) -> bool {
        self.refused.get(library).is_some_and(|refused| refused.contains(&name))
    }

    fn declared_in(&self, library: usize, name: Name) -> Lookup {
        match self.index.get(library).and_then(|index| index.get(&name)) {
            Some(index) => Lookup::Declared(*index),
            None if self.broken.get(library).copied().unwrap_or(true) || self.is_refused(library, name) => Lookup::AlreadySaid,
            None => Lookup::Unknown,
        }
    }

    fn scope_of(info: &TypeInfo<'a>) -> Scope<'a> {
        Scope { library: info.library, file: info.file, model: info.model, generic: !info.decl.generics.is_empty(), function: None }
    }

    /// The types that a type names, and whether it names each one through a `use`.
    fn collect_mentions(&self, expr: &TypeExpr, scope: &Scope<'_>, found: &mut Vec<(usize, bool)>) {
        match expr {
            TypeExpr::Named(named) => {
                if let Lookup::Declared(index) = self.lookup(scope, &named.path) {
                    found.push((index, !named.path.rest.is_empty()));
                }
                for arg in &named.args {
                    self.collect_mentions(arg, scope, found);
                }
            }
            TypeExpr::Array(array) => self.collect_mentions(&array.element, scope, found),
            TypeExpr::Struct(fields) => {
                for field in &fields.fields {
                    self.collect_mentions(&field.ty, scope, found);
                }
            }
            TypeExpr::Enum(variants) => {
                for ty in variants.variants.iter().flat_map(|variant| &variant.payload) {
                    self.collect_mentions(ty, scope, found);
                }
            }
            TypeExpr::Fn(function) => {
                for ty in function.params.iter().map(|param| &param.ty).chain(&function.outputs) {
                    self.collect_mentions(ty, scope, found);
                }
            }
        }
    }

    /// `LC2017`, once per cycle: on the first file in the order of the paths, with the others in `related`. Any mention
    /// counts, in an array or a function type too. A cycle that passes through a `use` cannot exist unless the libraries
    /// use each other in a cycle, which `LC2021` has already said. A generic type mentions nothing yet: what it names may
    /// be one of its variables, and it has its `LC4006`.
    fn find_cycles(&mut self) {
        let found: Vec<Vec<(usize, bool)>> = self
            .types
            .iter()
            .map(|info| {
                let mut found = Vec::new();
                if info.decl.generics.is_empty() {
                    self.collect_mentions(&info.decl.ty, &Self::scope_of(info), &mut found);
                }
                found
            })
            .collect();
        let mentions: Vec<Vec<usize>> = found.iter().map(|list| list.iter().map(|(index, _)| *index).collect()).collect();
        self.kinds = vec![None; self.types.len()];
        self.depths = vec![None; self.types.len()];
        let components = cycles(&mentions);
        self.mentions = mentions;
        for component in components {
            for member in &component {
                if let Some(kind) = self.kinds.get_mut(*member) {
                    *kind = Some(Kind::Unknown);
                }
                if let Some(depth) = self.depths.get_mut(*member) {
                    *depth = Some(None);
                }
            }
            self.in_cycles.extend(&component);
            let members: HashSet<usize> = component.iter().copied().collect();
            let said = component
                .iter()
                .filter_map(|member| found.get(*member))
                .flatten()
                .any(|(mention, through_a_use)| *through_a_use && members.contains(mention));
            if !said {
                self.report_cycle(&component);
            }
        }
    }

    fn report_cycle(&mut self, component: &[usize]) {
        let mut members: Vec<&TypeInfo<'a>> = component.iter().filter_map(|index| self.types.get(*index)).collect();
        members.sort_by(|a, b| a.file.shown.cmp(&b.file.shown));
        let Some((first, others)) = members.split_first() else { return };
        let name = self.names.resolve(first.decl.name);
        let message = format!("the type `{name}` contains itself, directly or through other types (3.2)");
        let mut diagnostic = at(Code::Lc2017, message, &first.file.shown, &first.model.source_map, first.decl.id);
        diagnostic.related = others
            .iter()
            .map(|info| Related {
                message: format!("`{}` is part of this cycle", self.names.resolve(info.decl.name)),
                file: info.file.shown.clone(),
                span: info.model.source_map.span(info.decl.id),
            })
            .collect();
        let library = first.library;
        self.push(library, diagnostic);
    }

    /// A cycle is known by now, so a chain of aliases always ends. Each link is walked once.
    fn find_kinds(&mut self) {
        for start in 0..self.types.len() {
            let mut chain = Vec::new();
            let mut current = start;
            let kind = loop {
                if let Some(known) = self.kinds.get(current).copied().flatten() {
                    break known;
                }
                chain.push(current);
                match self.alias_of(current) {
                    Ok(next) => current = next,
                    Err(kind) => break kind,
                }
            };
            for member in chain {
                if let Some(slot) = self.kinds.get_mut(member) {
                    *slot = Some(kind);
                }
            }
        }
    }

    /// The type that this one is another name of, or what it is when it is no alias.
    fn alias_of(&self, index: usize) -> Result<usize, Kind> {
        let Some(info) = self.types.get(index) else { return Err(Kind::Unknown) };
        if !info.decl.generics.is_empty() {
            return Err(Kind::Unknown);
        }
        let TypeExpr::Named(named) = &info.decl.ty else { return Err(Kind::Plain) };
        let bare = named.args.is_empty() && named.unit.is_none();
        match self.lookup(&Self::scope_of(info), &named.path) {
            Lookup::Opaque if bare => Err(Kind::Opaque(index)),
            Lookup::Declared(next) if bare => Ok(next),
            // A declared type takes neither a unit nor an argument for now, so this one has a diagnostic.
            Lookup::Declared(_) | Lookup::Opaque | Lookup::Unknown | Lookup::NotImported(_) | Lookup::AlreadySaid => Err(Kind::Unknown),
            Lookup::Base(_) | Lookup::Waveform | Lookup::Later => Err(Kind::Plain),
        }
    }

    /// What is embedded in the tool is not the user's to limit, as for reading it.
    fn nesting_limit(&self, library: usize) -> u32 {
        match self.loaded.get(library) {
            Some(source) if source.origin == Origin::Shipped => Limits::default().max_nesting(),
            _ => self.limits.max_nesting(),
        }
    }

    /// `LC1006` for a type nested too deep through its lines or its files, on the first one over the limit: what names
    /// it says nothing more. Types are measured after those they name, with a stack of our own.
    fn measure(&mut self) {
        let mut next_mention = vec![0_usize; self.types.len()];
        for start in 0..self.types.len() {
            let mut stack = vec![start];
            while let Some(&current) = stack.last() {
                if self.depths.get(current).copied().flatten().is_some() {
                    stack.pop();
                    continue;
                }
                let position = next_mention.get(current).copied().unwrap_or(usize::MAX);
                if let Some(slot) = next_mention.get_mut(current) {
                    *slot = position.saturating_add(1);
                }
                match self.mentions.get(current).and_then(|mentions| mentions.get(position)).copied() {
                    Some(mention) => {
                        if self.depths.get(mention).copied().flatten().is_none() {
                            stack.push(mention);
                        }
                    }
                    None => {
                        let depth = self.measure_one(current);
                        if let Some(slot) = self.depths.get_mut(current) {
                            *slot = Some(depth);
                        }
                        self.order.push(current);
                        stack.pop();
                    }
                }
            }
        }
    }

    fn measure_one(&mut self, index: usize) -> Option<u32> {
        let info = self.types.get(index)?;
        let (scope, decl) = (Self::scope_of(info), info.decl);
        self.within_the_limit(&decl.ty, &scope, decl.id)
    }

    /// The depth of a type, or `None` if it is over the limit, which is reported, or if an error stopped the count.
    fn within_the_limit(&mut self, expr: &TypeExpr, scope: &Scope<'_>, id: Id) -> Option<u32> {
        let depth = self.expr_depth(expr, scope)?;
        let limit = self.nesting_limit(scope.library);
        if depth > limit {
            self.error(scope, Code::Lc1006, format!("this type is nested {depth} levels deep through other types, over the limit of {limit}"), id);
            return None;
        }
        Some(depth)
    }

    /// One level for an array, a struct, an enum, a function type or the `<` of a named type (spec 14.1).
    fn expr_depth(&self, expr: &TypeExpr, scope: &Scope<'_>) -> Option<u32> {
        let deepest = |types: &mut dyn Iterator<Item = &TypeExpr>| -> Option<u32> {
            let mut most = 0;
            for ty in types {
                most = most.max(self.expr_depth(ty, scope)?);
            }
            Some(most.saturating_add(1))
        };
        match expr {
            TypeExpr::Named(named) => {
                let own = match self.lookup(scope, &named.path) {
                    Lookup::Declared(index) => self.depths.get(index).copied().flatten().flatten()?,
                    _ => 0,
                };
                if named.args.is_empty() { Some(own) } else { Some(own.max(deepest(&mut named.args.iter())?)) }
            }
            TypeExpr::Array(array) => deepest(&mut std::iter::once(&*array.element)),
            TypeExpr::Struct(fields) => deepest(&mut fields.fields.iter().map(|field| &field.ty)),
            TypeExpr::Enum(variants) => deepest(&mut variants.variants.iter().flat_map(|variant| &variant.payload)),
            TypeExpr::Fn(function) => deepest(&mut function.params.iter().map(|param| &param.ty).chain(&function.outputs)),
        }
    }

    /// What one file needs for its declaration, made once.
    fn begin_declaration(&mut self, model: &Model) {
        let uses = &model.file.uses;
        let line_of = |id| model.source_map.span(id).map_or(1, |span| span.start.line);
        let mut sorted_uses: Vec<(String, u32)> = uses.iter().map(|line| (path_text(&line.path, self.names), line_of(line.id))).collect();
        sorted_uses.sort();
        // In reverse, so that the first `use` of a segment is the one that stays.
        let imports = uses.iter().rev().map(|line| (line.path.first, line.path.id)).collect();
        self.current = CurrentFile { imports, sorted_uses, not_imported: HashSet::new() };
    }

    /// A panic is a bug, reported on that file while the others go on (`docs/architecture.md`, section 5).
    fn report_a_panic(&mut self, library: usize, shown: &str, payload: String) {
        let bug = self.libraries.last_panic().unwrap_or(payload);
        let message = format!("internal error, which is a bug of the tool: {bug}");
        self.push(library, Diagnostic::new(Code::Lc0099, message, shown));
        if let Some(broken) = self.broken.get_mut(library) {
            *broken = true;
        }
    }

    fn resolve_types(&mut self) {
        self.resolved = vec![None; self.types.len()];
        let mut order = std::mem::take(&mut self.order);
        order.append(&mut self.in_cycles);
        for index in order {
            let Some(info) = self.types.get(index) else { continue };
            let (scope, decl) = (Self::scope_of(info), info.decl);
            self.begin_declaration(scope.model);
            match last_resort(|| self.declare_type(decl, scope, Some(index))) {
                Ok(declared) => {
                    if let Some(slot) = self.resolved.get_mut(index) {
                        *slot = declared;
                    }
                }
                Err(payload) => {
                    self.report_a_panic(scope.library, &scope.file.shown, payload);
                    if let Some(slot) = self.resolved.get_mut(index) {
                        *slot = Some(DeclaredType { ty: TyId::ERROR, definition: TyId::ERROR, opaque: false });
                    }
                }
            }
        }
    }

    fn declare_all(&mut self) -> Vec<DeclaredLibrary> {
        let loaded = self.loaded;
        let ids = self.ids.clone();
        loaded.iter().zip(ids).enumerate().map(|(library, (source, id))| self.declare_library(library, source, id)).collect()
    }

    /// Of two declarations of one name, the first one in the order of `sorted_files` is the one that stays.
    fn declare_library(&mut self, library: usize, source: &'a Library, id: LibraryId) -> DeclaredLibrary {
        let mut types = HashMap::new();
        let mut type_files = HashMap::new();
        let mut functions = HashMap::new();
        // The index of each file in its library, until `check` gives the one in `Checked::files`.
        for (at, file) in sorted_files_at(source, self.names) {
            let at = u32::try_from(at).ok();
            let Some(model) = &file.model else { continue };
            let scope = Scope { library, file, model, generic: false, function: None };
            let declared_name = name_of_decl(&model.file.decl).0;
            let copy = !well_named(file, model, self.names) && self.named.get(library).is_some_and(|named| named.contains(&(declared_name, false)));
            if let Decl::Type(decl) = &model.file.decl
                && let Some(index) = self.index_of(library, decl)
            {
                if let Some(declared) = self.resolved.get(index).copied().flatten() {
                    types.insert(decl.name, declared);
                    if let Some(at) = at {
                        type_files.insert(decl.name, at);
                    }
                }
                continue;
            }
            self.begin_declaration(model);
            let outcome = last_resort(|| match &model.file.decl {
                Decl::Function(function) => {
                    let declared = DeclaredFunction { file: at, ..self.declare_function(function.name, &function.signature, scope, None) };
                    keep_the_first(&mut functions, function.name, declared, copy);
                }
                Decl::Extern(function) => {
                    let declared =
                        DeclaredFunction { file: at, ..self.declare_function(function.name, &function.signature, scope, Some(function.id)) };
                    keep_the_first(&mut functions, function.name, declared, copy);
                }
                // No program can name it, and its mistakes are still mistakes.
                Decl::Type(decl) => {
                    self.declare_type(decl, Scope { generic: !decl.generics.is_empty(), ..scope }, None);
                }
            });
            if let Err(payload) = outcome {
                self.report_a_panic(library, &file.shown, payload);
            }
            if !self.current.not_imported.is_empty() {
                let said = std::mem::take(&mut self.current.not_imported);
                self.said.insert(&file.shown, said);
            }
        }
        let broken = self.broken.get(library).copied().unwrap_or(true);
        let normalized = |list: &[String]| -> Vec<Name> { list.iter().filter_map(|text| self.names.find(text)).collect() };
        let always = normalized(&source.manifest.always);
        let resources = normalized(&source.manifest.resources);
        DeclaredLibrary {
            name: source.name.clone(),
            id,
            types,
            functions,
            type_files,
            checked: source.checked,
            always,
            resources,
            categories: source.manifest.categories.iter().map(|category| normalize(category).into_owned()).collect(),
            broken,
        }
    }

    /// The index of this very declaration: `None` when its name is refused, or when another one has it.
    fn index_of(&self, library: usize, decl: &TypeDecl) -> Option<usize> {
        let index = self.index.get(library)?.get(&decl.name).copied()?;
        self.types.get(index).is_some_and(|info| std::ptr::eq(info.decl, decl)).then_some(index)
    }

    /// `None` for a type that no program can name. A type with an error anywhere inside is the error type by its name
    /// too, so that what names it says nothing more.
    fn declare_type(&mut self, decl: &'a TypeDecl, scope: Scope<'a>, index: Option<usize>) -> Option<DeclaredType> {
        if let Some(generic) = decl.generics.first() {
            let message = "a generic type written by the user comes with a later milestone (spec, section 9)".to_owned();
            self.error(&scope, Code::Lc4006, message, generic.id);
        }
        let whole_opaque = matches!(&decl.ty, TypeExpr::Named(whole) if self.is_the_word_opaque(whole, &scope));
        let definition = if whole_opaque { index.map_or(TyId::ERROR, |index| self.named_ty(index)) } else { self.resolve(&decl.ty, &scope) };
        let index = index?;
        let kind = self.kinds.get(index).copied().flatten();
        let stopped = self.depths.get(index).copied().flatten().is_none_or(|depth| depth.is_none());
        let broken = kind == Some(Kind::Unknown) || stopped || self.table.has_error(definition);
        let ty = match &decl.ty {
            _ if broken => TyId::ERROR,
            TypeExpr::Struct(_) | TypeExpr::Enum(_) => self.named_ty(index),
            // `type session = opaque` has its name as its definition, and an alias has the type that it names.
            TypeExpr::Named(_) | TypeExpr::Array(_) | TypeExpr::Fn(_) => definition,
        };
        Some(DeclaredType { ty, definition, opaque: matches!(kind, Some(Kind::Opaque(_))) })
    }

    fn is_the_word_opaque(&self, named: &NamedType, scope: &Scope<'_>) -> bool {
        named.args.is_empty() && named.unit.is_none() && matches!(self.lookup(scope, &named.path), Lookup::Opaque)
    }

    /// The type of this name: of the opaque type at the end of a chain of aliases (3.2).
    fn named_ty(&mut self, index: usize) -> TyId {
        let root = match self.kinds.get(index).copied().flatten() {
            Some(Kind::Opaque(root)) => root,
            _ => index,
        };
        let Some(info) = self.types.get(root) else { return TyId::ERROR };
        let (name, library) = (info.decl.name, info.library);
        let Some(library) = self.ids.get(library).copied() else { return TyId::ERROR };
        self.table.intern(Ty::Named { library, name }).unwrap_or(TyId::ERROR)
    }

    /// What a mention of a declared type means: its `order` puts it after what it names, and a bug or a cycle leaves it unknown.
    fn mention_of(&self, index: usize) -> TyId {
        self.resolved.get(index).copied().flatten().map_or(TyId::ERROR, |declared| declared.ty)
    }

    /// `external` holds where the `extern` declaration is.
    fn declare_function(&mut self, name: Name, signature: &'a Signature, scope: Scope<'a>, external: Option<Id>) -> DeclaredFunction {
        let scope = Scope { generic: !signature.generics.is_empty(), function: Some(self.names.resolve(name)), ..scope };
        if let Some(generic) = signature.generics.first() {
            self.error(
                &scope,
                Code::Lc4006,
                "a generic function written by the user comes with a later milestone (spec, section 9)".to_owned(),
                generic.id,
            );
        }
        let faulty = self.judge_the_parameters(signature, &scope);
        self.judge_the_value_names(signature, &scope);
        let params = signature
            .params
            .iter()
            .map(|param| DeclaredParam {
                name: param.name,
                ty: self.signature_type(&param.ty, &scope, param.id),
                has_default: param.default.is_some(),
                variadic: param.variadic,
            })
            .collect();
        let outputs: Vec<DeclaredOutput> = signature
            .outputs
            .iter()
            .map(|output| DeclaredOutput { name: output.name, ty: self.signature_type(&output.ty, &scope, output.id) })
            .collect();
        if let Some(id) = external {
            // A type with an error inside has its diagnostic, and what it would have been stays unknown. The type is written
            // as such (R-13): an alias of `error` does not do.
            let has_err = signature.outputs.iter().zip(&outputs).any(|(written, output)| {
                let as_such =
                    matches!(&written.ty, TypeExpr::Named(named) if named.path.rest.is_empty() && self.names.resolve(named.path.first) == "error");
                self.names.resolve(output.name) == "err" && (as_such || self.table.has_error(output.ty))
            });
            if !has_err {
                self.error(&scope, Code::Lc4005, "an external function has an output `err: error` (R-13)".to_owned(), id);
            }
        }
        DeclaredFunction { params, outputs, external: external.is_some(), generic: !signature.generics.is_empty(), faulty, file: None }
    }

    fn signature_type(&mut self, expr: &TypeExpr, scope: &Scope<'_>, id: Id) -> TyId {
        let measured = self.within_the_limit(expr, scope, id);
        let ty = self.resolve(expr, scope);
        if measured.is_some() { ty } else { TyId::ERROR }
    }

    /// `LC2019` within the parameters and within the outputs, since `err` can be both (R-2); `LC2105`; `LC2107`. Whether any
    /// of these has something to say.
    fn judge_the_parameters(&mut self, signature: &Signature, scope: &Scope<'_>) -> bool {
        let params: Vec<(Name, Id)> = signature.params.iter().map(|param| (param.name, param.id)).collect();
        let outputs: Vec<(Name, Id)> = signature.outputs.iter().map(|output| (output.name, output.id)).collect();
        let mut faulty = self.judge_twice(&params, scope, "parameter");
        faulty |= self.judge_twice(&outputs, scope, "output");
        for (name, id) in &params {
            if self.names.resolve(*name) == "after" {
                faulty = true;
                self.error(scope, Code::Lc2105, "no parameter is named `after`, the order input of every call (R-15)".to_owned(), *id);
            }
        }
        let (mut variadic, mut default) = (false, false);
        for param in &signature.params {
            let misplaced = if param.variadic { variadic || default } else { variadic && param.default.is_none() };
            if misplaced {
                let message = "a function has one variadic parameter, after all required parameters and before those with a default value (R-11)";
                self.error(scope, Code::Lc2107, message.to_owned(), param.id);
                faulty = true;
                break;
            }
            variadic |= param.variadic;
            default |= param.default.is_some();
        }
        faulty
    }

    /// Whether two items have one name.
    fn judge_twice(&mut self, items: &[(Name, Id)], scope: &Scope<'_>, what: &str) -> bool {
        let mut seen: HashMap<Name, Id> = HashMap::new();
        let mut twice = false;
        for (name, id) in items {
            let Some(first) = seen.get(name).copied() else {
                seen.insert(*name, *id);
                continue;
            };
            twice = true;
            let text = self.names.resolve(*name);
            let related = vec![Related {
                message: format!("`{text}` is first declared here"),
                file: scope.file.shown.clone(),
                span: scope.model.source_map.span(first),
            }];
            self.error_with(scope, Code::Lc2019, format!("two of these have the name `{text}`: each {what} needs its own"), *id, related, Vec::new());
        }
        twice
    }

    /// R-1, for what is a value in the body. Against the `use` lines of this file, the only ones that its body can read.
    /// A name that is both an input and an output is one mistake.
    fn judge_the_value_names(&mut self, signature: &Signature, scope: &Scope<'_>) {
        let mut seen = HashSet::new();
        let params = signature.params.iter().map(|param| (param.name, param.id));
        let outputs = signature.outputs.iter().map(|output| (output.name, output.id));
        for (name, id) in params.chain(outputs) {
            if !seen.insert(name) {
                continue;
            }
            if let Some((message, related)) = self.value_name_conflict(name, scope) {
                self.error_with(scope, Code::Lc2014, message, id, vec![related], Vec::new());
            }
        }
    }

    /// What a value of this name is in conflict with, apart from a panel element: a `use` of this file, or a type of its
    /// library that has something a path can name (R-1).
    fn value_name_conflict(&self, name: Name, scope: &Scope<'_>) -> Option<(String, Related)> {
        let text = self.names.resolve(name);
        if let Some(use_id) = self.current.imports.get(&name).copied() {
            let related =
                Related { message: format!("`{text}` is imported here"), file: scope.file.shown.clone(), span: scope.model.source_map.span(use_id) };
            return Some((format!("a value cannot bear the name `{text}`: this file imports a library of that name (R-1)"), related));
        }
        let index = self.index.get(scope.library)?.get(&name).copied()?;
        if self.kinds.get(index).copied().flatten() != Some(Kind::Plain) {
            return None;
        }
        let info = self.types.get(index)?;
        let related = Related {
            message: format!("the type `{text}` is declared here"),
            file: info.file.shown.clone(),
            span: info.model.source_map.span(info.decl.id),
        };
        Some((format!("a value cannot bear the name `{text}`: this library has a type of that name (R-1)"), related))
    }

    fn intern(&mut self, ty: Ty, scope: &Scope<'_>, id: Id) -> TyId {
        match self.table.intern(ty) {
            Ok(ty) => ty,
            Err(_) => {
                self.error(scope, Code::Lc1006, "this type is out of bounds".to_owned(), id);
                TyId::ERROR
            }
        }
    }

    fn resolve(&mut self, expr: &TypeExpr, scope: &Scope<'_>) -> TyId {
        match expr {
            TypeExpr::Named(named) => self.resolve_named(named, scope),
            TypeExpr::Array(array) => {
                let element = self.resolve(&array.element, scope);
                let dimensions = match array.dimensions {
                    None => Some(1),
                    Some(text) => dimensions_of(self.names.resolve(text)),
                };
                let Some(dimensions) = dimensions else {
                    self.error(scope, Code::Lc1006, format!("an array has from 1 to {MAX_ARRAY_DIMENSIONS} dimensions"), array.id);
                    return TyId::ERROR;
                };
                self.intern(Ty::Array { element, dimensions }, scope, array.id)
            }
            TypeExpr::Struct(fields) => self.resolve_struct(fields, scope),
            TypeExpr::Enum(variants) => self.resolve_enum(variants, scope),
            TypeExpr::Fn(function) => self.resolve_function_type(function, scope),
        }
    }

    fn resolve_struct(&mut self, fields: &StructType, scope: &Scope<'_>) -> TyId {
        let items: Vec<(Name, Id)> = fields.fields.iter().map(|field| (field.name, field.id)).collect();
        let twice = self.judge_twice(&items, scope, "field");
        let resolved = fields.fields.iter().map(|field| (field.name, self.resolve(&field.ty, scope))).collect();
        if twice {
            return TyId::ERROR;
        }
        self.intern(Ty::Struct(resolved), scope, fields.id)
    }

    fn resolve_enum(&mut self, variants: &EnumType, scope: &Scope<'_>) -> TyId {
        let items: Vec<(Name, Id)> = variants.variants.iter().map(|variant| (variant.name, variant.id)).collect();
        let twice = self.judge_twice(&items, scope, "variant");
        let resolved = variants
            .variants
            .iter()
            .map(|variant| Variant { name: variant.name, values: variant.payload.iter().map(|ty| self.resolve(ty, scope)).collect() })
            .collect();
        if twice {
            return TyId::ERROR;
        }
        self.intern(Ty::Enum(resolved), scope, variants.id)
    }

    /// The name of an argument says how `core.call` wires it (spec 4), so two of one name cannot both be wired.
    fn resolve_function_type(&mut self, function: &FnType, scope: &Scope<'_>) -> TyId {
        let named: Vec<(Name, Id)> = function.params.iter().filter_map(|param| Some((param.name?, param.id))).collect();
        let mut refused = self.judge_twice(&named, scope, "argument");
        let mut params = Vec::with_capacity(function.params.len());
        for param in &function.params {
            if param.name.is_some_and(|name| self.names.resolve(name) == "after") {
                self.error(scope, Code::Lc2105, "no parameter is named `after`, the order input of every call (R-15)".to_owned(), param.id);
                refused = true;
            }
            params.push(FnParam { name: param.name, ty: self.resolve(&param.ty, scope) });
        }
        let outputs = function.outputs.iter().map(|ty| self.resolve(ty, scope)).collect();
        if refused {
            return TyId::ERROR;
        }
        self.intern(Ty::Function { params, outputs }, scope, function.id)
    }

    fn resolve_named(&mut self, named: &NamedType, scope: &Scope<'_>) -> TyId {
        let names = self.names;
        let text = || path_text(&named.path, names);
        let id = named.id;
        // Each argument can hold a mistake of its own, whatever comes of the type around it.
        let args: Vec<TyId> = named.args.iter().map(|arg| self.resolve(arg, scope)).collect();
        match self.lookup(scope, &named.path) {
            Lookup::Unknown => {
                self.error(scope, Code::Lc2003, format!("unknown type `{}`", text()), named.path.id);
                TyId::ERROR
            }
            Lookup::NotImported(library) => {
                let fix = add_use(&self.current.sorted_uses, &library);
                let message = format!("the library `{library}` is used without a `use` line in this file");
                if self.current.not_imported.insert(library) {
                    self.error_with(scope, Code::Lc2005, message, named.path.id, Vec::new(), vec![fix]);
                }
                TyId::ERROR
            }
            // What has its diagnostic, elsewhere.
            Lookup::AlreadySaid => {
                self.leaned = true;
                TyId::ERROR
            }
            Lookup::Opaque => {
                let message = "`opaque` is only the whole type of a `type` declaration (3.2)".to_owned();
                self.error(scope, Code::Lc2003, message, named.path.id);
                TyId::ERROR
            }
            Lookup::Later => {
                self.error(scope, Code::Lc4006, format!("`{}` comes with a later milestone (spec, section 9)", text()), id);
                TyId::ERROR
            }
            Lookup::Waveform => {
                let [element] = args.as_slice() else {
                    self.error(scope, Code::Lc3019, format!("`waveform` takes one type argument, and has {} here", args.len()), id);
                    return TyId::ERROR;
                };
                let element = *element;
                if self.refuse_a_unit(named, scope) {
                    return TyId::ERROR;
                }
                // An alias already is what it names, and a type with an error has its diagnostic.
                if !self.table.has_error(element) && !matches!(self.table.get(element), Ty::Number { .. }) {
                    self.error(scope, Code::Lc3019, "a `waveform` holds numbers, as in `waveform<f64[V]>` (3.2)".to_owned(), id);
                    return TyId::ERROR;
                }
                self.intern(Ty::Waveform(element), scope, id)
            }
            Lookup::Base(base) => {
                if !args.is_empty() {
                    self.error(scope, Code::Lc3019, format!("`{}` takes no type argument", text()), id);
                    return TyId::ERROR;
                }
                self.resolve_base(base, named, scope)
            }
            Lookup::Declared(index) => {
                let generic = self.types.get(index).is_some_and(|info| !info.decl.generics.is_empty());
                if !args.is_empty() && !generic {
                    self.error(scope, Code::Lc3019, format!("`{}` takes no type argument", text()), id);
                    return TyId::ERROR;
                }
                if self.refuse_a_unit(named, scope) {
                    return TyId::ERROR;
                }
                self.mention_of(index)
            }
        }
    }

    fn resolve_base(&mut self, base: Ty, named: &NamedType, scope: &Scope<'_>) -> TyId {
        let Ty::Number { kind, .. } = base else {
            if self.refuse_a_unit(named, scope) {
                return TyId::ERROR;
            }
            return self.intern(base, scope, named.id);
        };
        let Some(expr) = &named.unit else { return self.intern(Ty::Number { kind, unit: Unit::ONE }, scope, named.id) };
        match unit_of(expr, self.names) {
            Ok(unit) => self.intern(Ty::Number { kind, unit }, scope, named.id),
            Err(error) => {
                let message = match &error {
                    UnitError::Unknown(name) => format!("unknown unit `{name}`"),
                    _ => "this unit is out of bounds: an exponent goes beyond 127, or the scale beyond 1e300".to_owned(),
                };
                self.error(scope, error.code(), message, expr.id);
                TyId::ERROR
            }
        }
    }

    /// `LC3021`: only a number has a unit.
    fn refuse_a_unit(&mut self, named: &NamedType, scope: &Scope<'_>) -> bool {
        let Some(unit) = &named.unit else { return false };
        let text = path_text(&named.path, self.names);
        self.error(scope, Code::Lc3021, format!("`{text}` is not a number, so it cannot have a unit"), unit.id);
        true
    }
}
