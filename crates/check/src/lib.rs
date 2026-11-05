//! L3: name resolution, signatures, type and connection checking (`docs/architecture.md`, section 5).
//!
//! So far, pass 1: reading the files and finding the libraries that they use. The other passes come with their steps
//! (section 9), so a program that reads and whose libraries exist is clean for now.
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

use labcraft_graph::{Decl, File, SourceMap};
use labcraft_types::{CheckCounts, Diagnostic, Interner, Limits, Report};

mod load;
pub mod manifest;

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
    /// Its path as reports show it, with its extension.
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
    /// The files that read, of the folder given and its sub-libraries. Dropping one takes the stack that reading it did.
    pub files: Vec<CheckedFile>,
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
}

/// Checks a folder and its sub-libraries (`docs/cli.md`, section 3). `root` includes its sub-libraries, which are
/// checked even if nothing uses them. A dependency is read for what it declares.
///
/// It takes the stack that `format::read` does: see `format::STACK_SIZE`.
pub fn check(root: LibrarySource, libraries: &dyn Libraries, names: &mut Interner, limits: &Limits) -> Checked {
    let mut report = Report::new(limits.max_report);
    let (loaded, dependencies) = load::load(root, libraries, names, limits, &mut report);
    let mut counts = CheckCounts::default();
    let mut files = Vec::new();
    for library in loaded.into_iter().filter(|library| library.checked) {
        counts.libraries += 1;
        for file in library.files {
            let Some(model) = file.model else { continue };
            match model.file.decl {
                Decl::Function(_) | Decl::Extern(_) => counts.functions += 1,
                Decl::Type(_) => counts.types += 1,
            }
            files.push(CheckedFile { shown: file.shown, file: model.file, source_map: model.source_map });
        }
    }
    Checked { report, dependencies, counts, files }
}
