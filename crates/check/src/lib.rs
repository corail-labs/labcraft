//! L3: name resolution, signatures, type and connection checking (`docs/architecture.md`, section 5).
//!
//! Passes 1 to 6: reading the files, finding the libraries that they use, what each library declares, the names of each body,
//! its types, its dataflow, and the `panel`, `layout` and `icon` sections.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable,
    clippy::indexing_slicing,
    clippy::string_slice
)]
#![forbid(unsafe_code)]

use std::borrow::Cow;
use std::collections::HashMap;

use labcraft_format::{BodyOrders, canonical_orders};
use labcraft_graph::{Decl, File, SourceMap, UnitExpr};
use labcraft_types::{CheckCounts, Diagnostic, FileCounts, Interner, Limits, Report, TypeTable, Unit, UnitError};

mod declare;
mod file_name;
mod load;
pub mod manifest;
mod passed_in;
pub mod primitives;
mod reach;
mod resolved;
mod terminals;

pub use declare::{DeclaredFunction, DeclaredLibrary, DeclaredOutput, DeclaredParam, DeclaredType};
pub use passed_in::{MAX_PASSED_IN, PassedIn, passed_in};
pub use reach::{Reach, Unresolved};
pub use resolved::{Coercion, Meaning, PrimitiveLibrary, Resolved, ResolvedCall, Terminal};
pub use terminals::{MAX_TERMINALS, Terminals, terminals};

/// Which rules of `docs/cli.md`, section 3, a library follows when it uses another: a library shipped with LabCraft
/// sees only the primitives and the others that are shipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    Project,
    Shipped,
}

/// How `use` finds a library beyond the folder given: the caller decides, since only an app touches the file system.
pub trait Libraries {
    /// `name` is normalized (R-20), with its segments separated by `.`. With [`Origin::Shipped`], only a library
    /// shipped with LabCraft can answer. What comes back is read for its signatures: its `sub_libraries` are ignored,
    /// since `use a.b` asks for `a.b` by name.
    fn find(&self, name: &str, by: Origin) -> Found;

    /// What a host knows of the panic that was just caught, such as where it happened, which only a panic hook can see.
    fn last_panic(&self) -> Option<String> {
        None
    }
}

#[derive(Debug)]
pub enum Found {
    Library(LibrarySource),
    NotFound,
    /// The `LC0001` to report.
    Unreadable(Box<Diagnostic>),
}

/// A library as its host found it, with the files of its folder and no others.
#[derive(Debug)]
pub struct LibrarySource {
    /// The name of its folder, as found: empty for the folder given to `check`.
    pub name: String,
    /// The folder as reports show it.
    pub shown: String,
    pub files: Vec<SourceFile>,
    /// The text of `labcraft.toml`, read only for a shipped library (`docs/cli.md`, section 3).
    pub manifest: Option<String>,
    pub sub_libraries: Vec<LibrarySource>,
    pub origin: Origin,
    /// What the host met while gathering these files: a warning on a folder, a file it could not read.
    pub report: Report,
    /// Every file of it is also in a folder that the run checks, and says there what it has to say. Reading it here
    /// can still find a cycle, since a dependency does not resolve its `use` lines as a sub-library does.
    pub checked_elsewhere: bool,
}

impl LibrarySource {
    /// With no files, no sub-libraries and nothing to say yet.
    pub fn new(name: String, shown: String, origin: Origin) -> Self {
        LibrarySource {
            name,
            shown,
            files: Vec::new(),
            manifest: None,
            sub_libraries: Vec::new(),
            origin,
            report: Report::new(0),
            checked_elsewhere: false,
        }
    }
}

#[derive(Debug)]
pub struct SourceFile {
    /// Its path as reports show it, with its extension. Its last segment, after a `/` or a `\`, is the name of the file,
    /// which must be that of what the file declares (R-22): a host that shows something else gets `LC2023`.
    pub shown: String,
    pub content: Content,
}

#[derive(Debug)]
pub enum Content {
    /// A shipped file is embedded in the program, so it costs no copy.
    Bytes(Cow<'static, [u8]>),
    /// Its host could not read it and has said so in `diagnostics`: it exists, and what it declares stays unknown.
    Unread,
}

/// What the layers above read, so that they never resolve again.
#[derive(Debug)]
pub struct Checked {
    /// Unfinished: the caller merges the reports of all its folders, then calls `Report::finish`.
    pub report: Report,
    /// The errors of the dependencies, the only thing of theirs that a run says. A caller with several folders merges
    /// each one once, by `shown`: a dependency says the same whoever reads it.
    pub dependencies: Vec<DependencyReport>,
    pub counts: CheckCounts,
    /// The files that read, of the folder given and its sub-libraries, and for a run those of every library it uses. Dropping
    /// one takes the stack that reading it did.
    pub files: Vec<CheckedFile>,
    pub types: TypeTable,
    /// What each library of the run declares, dependencies included.
    pub libraries: Vec<DeclaredLibrary>,
    /// For a run, what the bodies of the dependencies say, which the report does not: a run says it only of the files it
    /// reaches (`docs/cli.md`, 6.1), and `check` never. One report for all of them, bounded as one; its counts by file are
    /// complete. Empty for a check that is not for a run.
    pub bodies: Report,
    /// Made by [`check_for_run`]: the files of every library are in `files`, and [`Checked::reach`] can follow them.
    pub for_run: bool,
    /// A budget of work or a limit made a pass say less than it would have: past them, what is said can depend on the order of
    /// the lines and entries, which `fmt` changes. Nothing that a user sees; the fuzzing of `check` reads it.
    pub cut_short: bool,
}

#[derive(Debug)]
pub struct DependencyReport {
    /// The folder of the dependency, as reports show it.
    pub shown: String,
    pub report: Report,
}

#[derive(Debug)]
pub struct CheckedFile {
    pub shown: String,
    pub file: File,
    pub source_map: SourceMap,
    /// What each path, call and bracket of it means.
    pub resolved: Resolved,
    /// Its library, by its index in `Checked::libraries`.
    pub library: u32,
    /// For a run, the canonical order of every body of its function, found in one walk (spec 5.2): `None` for a type, an
    /// `extern` declaration, or a check that is not for a run.
    pub orders: Option<BodyOrders>,
    /// Its errors and warnings when `check` ended, complete whatever the reports list: what the report says of it, what its
    /// library says when it is a dependency, and what its body said apart (`Checked::bodies`).
    pub counts: FileCounts,
}

/// Checks a folder and its sub-libraries (`docs/cli.md`, section 3). `root` includes its sub-libraries, which are
/// checked even if nothing uses them. A dependency is read for what it declares.
///
/// It takes the stack that `format::read` does: see `format::STACK_SIZE`.
pub fn check(root: LibrarySource, libraries: &dyn Libraries, names: &mut Interner, limits: &Limits) -> Checked {
    check_with(root, libraries, names, limits, false)
}

/// What a run needs before it is lowered (`docs/architecture.md`, 10.1): [`check`], and also the bodies of every library
/// that the folder uses, resolved and typed as the folder is, with the canonical order of every body. What a body of a
/// dependency says stays apart ([`Checked::bodies`]), for the run that reaches it, so the report says
/// what [`check`] would. [`Checked::reach`] gives what a run of one function reaches.
pub fn check_for_run(root: LibrarySource, libraries: &dyn Libraries, names: &mut Interner, limits: &Limits) -> Checked {
    check_with(root, libraries, names, limits, true)
}

fn check_with(root: LibrarySource, libraries: &dyn Libraries, names: &mut Interner, limits: &Limits, for_run: bool) -> Checked {
    let mut report = Report::new(limits.max_report);
    let (mut loaded, unreadable) = load::load(root, libraries, names, limits, &mut report);
    let mut declared = declare::declare(&mut loaded, libraries, names, limits, &mut report, for_run);
    let dependencies = load::dependency_reports(&mut loaded, unreadable);
    let mut counts = CheckCounts::default();
    let mut files = Vec::new();
    // By library, the index in `files` of each of its files, `None` for one that is not there.
    let mut places: Vec<Vec<Option<u32>>> = Vec::with_capacity(loaded.len());
    for (index, (library, resolved)) in loaded.into_iter().zip(declared.resolved).enumerate() {
        let kept = library.checked || for_run;
        let mut at = Vec::with_capacity(library.files.len());
        if library.checked {
            counts.libraries += 1;
        }
        let library_index = u32::try_from(index).unwrap_or(u32::MAX);
        for (file, resolved) in library.files.into_iter().zip(resolved) {
            let Some(model) = file.model.filter(|_| kept) else {
                at.push(None);
                continue;
            };
            if library.checked {
                match model.file.decl {
                    Decl::Function(_) | Decl::Extern(_) => counts.functions += 1,
                    Decl::Type(_) => counts.types += 1,
                }
            }
            let orders = match &model.file.decl {
                Decl::Function(function) if for_run => Some(canonical_orders(&function.body, &function.signature, names)),
                _ => None,
            };
            at.push(u32::try_from(files.len()).ok());
            files.push(CheckedFile {
                shown: file.shown,
                file: model.file,
                source_map: model.source_map,
                resolved,
                library: library_index,
                orders,
                counts: FileCounts::default(),
            });
        }
        places.push(at);
    }
    // From the index of a file in its library to its index in `files`.
    for (library, at) in declared.libraries.iter_mut().zip(&places) {
        let place = |local: u32| at.get(local as usize).copied().flatten();
        for function in library.functions.values_mut() {
            function.file = function.file.and_then(place);
        }
        library.type_files = library.type_files.iter().filter_map(|(name, local)| Some((*name, place(*local)?))).collect();
    }
    // The counts of each file, from one table of what the dependencies say: looking through each of them for each file
    // would be quadratic in a run that uses many.
    let mut said: HashMap<&str, FileCounts> = HashMap::new();
    for (file, more) in dependencies.iter().flat_map(|dependency| dependency.report.file_counts()).chain(declared.bodies.file_counts()) {
        add(said.entry(file).or_default(), more);
    }
    for file in &mut files {
        let mut counts = report.counts_of(&file.shown);
        if let Some(more) = said.get(file.shown.as_str()) {
            add(&mut counts, *more);
        }
        file.counts = counts;
    }
    drop(said);
    Checked {
        report,
        dependencies,
        counts,
        files,
        types: declared.types,
        libraries: declared.libraries,
        bodies: declared.bodies,
        for_run,
        cut_short: declared.cut_short,
    }
}

/// The unit that a written unit means (spec 3.3), as `check` reads it.
pub fn unit_of(expr: &UnitExpr, names: &Interner) -> Result<Unit, UnitError> {
    declare::unit_of(expr, names)
}

/// The text of a string literal, as it runs: its escapes read (14.1).
pub fn read_escapes(text: &str) -> Cow<'_, str> {
    declare::read_escapes(text)
}

/// The fields of a `string.format` template whose escapes are read, each once, in the order they first appear: the order
/// of the free terminals of its node (5.4).
pub fn template_fields(text: &str) -> Vec<String> {
    declare::template_fields(text)
}

fn add(counts: &mut FileCounts, more: FileCounts) {
    counts.errors = counts.errors.saturating_add(more.errors);
    counts.warnings = counts.warnings.saturating_add(more.warnings);
}

impl Checked {
    /// [`CheckedFile::counts`] of the file at this index of `files`.
    pub fn counts_of(&self, file: usize) -> FileCounts {
        self.files.get(file).map_or_else(FileCounts::default, |file| file.counts)
    }
}
