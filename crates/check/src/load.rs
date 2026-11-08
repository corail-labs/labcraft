//! Pass 1 (`docs/architecture.md`, section 5): `format::read` on each file, then the set of libraries through `use`,
//! with their cycles. A file that does not parse is marked, and the other files go on.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::panic::{self, AssertUnwindSafe};

use labcraft_format::{Read, read};
use labcraft_graph::{Decl, File, Id, SourceMap, TypeExpr};
use labcraft_types::{Code, Diagnostic, Interner, Limits, Name, Related, Report, Severity, cut_text, normalize};

use crate::file_name::{case_key, kept_by_windows};
use crate::manifest::Manifest;
use crate::primitives::PrimitiveLibrary;
use crate::{Content, DependencyReport, Found, Libraries, LibrarySource, Origin, SourceFile};

/// `type session = opaque` (3.2).
fn is_opaque(ty: &TypeExpr, names: &Interner) -> bool {
    matches!(ty, TypeExpr::Named(named) if named.path.rest.is_empty() && named.args.is_empty() && names.resolve(named.path.first) == "opaque")
}

pub(crate) struct Library {
    /// With its segments separated by `.`: empty for the folder given.
    pub(crate) name: String,
    /// In the folder given: everything in it is checked. A dependency is read for what it declares.
    pub(crate) checked: bool,
    pub(crate) files: Vec<LoadedFile>,
    shown: String,
    pub(crate) origin: Origin,
    uses: Vec<LibraryUse>,
    /// The errors of a dependency, kept apart: several folders given can read one dependency.
    pub(crate) report: Report,
    pub(crate) checked_elsewhere: bool,
    /// What its manifest declares as resources (8.1). Only a shipped library has one before M2.
    pub(crate) resources: Vec<String>,
}

pub(crate) struct LoadedFile {
    pub(crate) shown: String,
    /// `None` when the file does not read: what it declares stays unknown.
    pub(crate) model: Option<Model>,
    /// What each of its `use` lines found, by the name of the library with `.` between segments.
    pub(crate) imports: HashMap<String, Target>,
}

pub(crate) struct Model {
    pub(crate) file: File,
    pub(crate) source_map: SourceMap,
}

/// A `use` that found a library, and where it is written.
struct LibraryUse {
    target: usize,
    file: usize,
    id: Id,
}

#[derive(Clone, Copy)]
pub(crate) enum Target {
    Primitive,
    Library(usize),
    Missing,
    /// Reported once, by the first `use` that met it.
    Unreadable,
}

/// The libraries of a run: the folder given and its sub-libraries first, then what they use. What a dependency that
/// could not be read has to say comes back apart from `report`.
pub(crate) fn load(
    root: LibrarySource,
    libraries: &dyn Libraries,
    names: &mut Interner,
    limits: &Limits,
    report: &mut Report,
) -> (Vec<Library>, Vec<DependencyReport>) {
    let mut loader =
        Loader { libraries, names, limits, report, loaded: Vec::new(), project: HashMap::new(), found: HashMap::new(), unreadable: Vec::new() };
    loader.add_root(root);
    // What a `use` finds joins the end of the list: a chain of libraries costs no stack.
    let mut next = 0;
    while next < loader.loaded.len() {
        loader.resolve_uses(next);
        next += 1;
    }
    loader.report_cycles();
    let Loader { loaded, unreadable, .. } = loader;
    (loaded, unreadable)
}

/// What the dependencies have to say, once the passes that read them are done.
pub(crate) fn dependency_reports(loaded: &mut [Library], unreadable: Vec<DependencyReport>) -> Vec<DependencyReport> {
    let mut dependencies = unreadable;
    for library in loaded.iter_mut().filter(|library| !library.checked && library.report.errors() > 0) {
        let report = std::mem::replace(&mut library.report, Report::new(0));
        dependencies.push(DependencyReport { shown: library.shown.clone(), report });
    }
    dependencies
}

struct Loader<'a> {
    libraries: &'a dyn Libraries,
    names: &'a mut Interner,
    limits: &'a Limits,
    report: &'a mut Report,
    loaded: Vec<Library>,
    /// The sub-libraries of the folder given, by their path from it, with `/` between segments: a folder can hold a `.`
    /// in its name, and is then no library that a `use` can name.
    project: HashMap<String, usize>,
    found: HashMap<(String, Origin), Target>,
    unreadable: Vec<DependencyReport>,
}

/// What the files of one library, or of the folder given and its sub-libraries, may still take.
struct Budget {
    left: u64,
    over: bool,
}

/// A warning on a dependency is not for this run: the dependency gets it when it is itself checked. An error is, since
/// it is the cause of what follows.
fn say(sink: &mut Report, checked: bool, diagnostic: Diagnostic) {
    if checked || diagnostic.severity() == Severity::Error {
        sink.push(diagnostic);
    }
}

fn over_the_limit(limits: &Limits) -> String {
    format!("files left out: over the limit of {} bytes in a folder", limits.max_library_size)
}

impl Loader<'_> {
    /// Sub-libraries are flattened with a stack of our own: `check` does not know how deep its host walked.
    fn add_root(&mut self, root: LibrarySource) {
        let shown = root.shown.clone();
        let mut budget = Budget { left: self.limits.max_library_size, over: false };
        let mut stack = vec![(String::new(), root)];
        while let Some((path, mut source)) = stack.pop() {
            self.judge_the_folders(&source.sub_libraries);
            // In reverse, so that the first of two folders with one normalized name is the one that a `use` finds.
            for sub in std::mem::take(&mut source.sub_libraries).into_iter().rev() {
                let segment = normalize(&sub.name).into_owned();
                let sub_path = if path.is_empty() { segment } else { format!("{path}/{segment}") };
                stack.push((sub_path, sub));
            }
            let name = path.replace('/', ".");
            if let Some(index) = self.add(source, name, true, &mut budget)
                && !path.is_empty()
            {
                self.project.entry(path).or_insert(index);
            }
        }
        if budget.over {
            self.report.push(Diagnostic::new(Code::Lc0010, over_the_limit(self.limits), shown));
        }
    }

    /// The folders side by side follow R-22 as files do: none that Windows keeps for a device, and no two that only
    /// their case or their spelling tell apart. Once for each pair, on the last one, as for files.
    fn judge_the_folders(&mut self, folders: &[LibrarySource]) {
        let mut by_case: HashMap<String, Vec<&LibrarySource>> = HashMap::new();
        for folder in folders {
            let name = normalize(&folder.name);
            if kept_by_windows(&name) {
                let message = format!("Windows keeps `{}` for a device: Git for Windows refuses a folder of that name (R-22)", cut_text(&name));
                self.report.push(Diagnostic::new(Code::Lc2025, message, folder.shown.clone()));
            }
            by_case.entry(case_key(&name)).or_default().push(folder);
        }
        let mut groups: Vec<Vec<&LibrarySource>> = by_case.into_values().filter(|group| group.len() > 1).collect();
        for group in &mut groups {
            group.sort_by(|a, b| a.shown.cmp(&b.shown));
        }
        groups.sort_by(|a, b| a.first().map(|f| &f.shown).cmp(&b.first().map(|f| &f.shown)));
        for group in groups {
            let Some((last, others)) = group.split_last() else { continue };
            let message = format!(
                "the folder `{}` differs from another one beside it only by its case or its spelling: they cannot be side by side on \
                 Windows or macOS (R-22)",
                cut_text(&last.name)
            );
            let mut diagnostic = Diagnostic::new(Code::Lc2024, message, last.shown.clone());
            diagnostic.related = others
                .iter()
                .map(|other| Related { message: format!("`{}` is here", cut_text(&other.name)), file: other.shown.clone(), span: None })
                .collect();
            self.report.push(diagnostic);
        }
    }

    /// `None` for a folder without files of its own, which is no library, though its sub-folders can be.
    fn add(&mut self, source: LibrarySource, name: String, checked: bool, budget: &mut Budget) -> Option<usize> {
        let LibrarySource { shown, files, manifest, origin, report: met, checked_elsewhere, .. } = source;
        let mut sink = Report::new(self.limits.max_report);
        if checked {
            sink.merge(met);
        } else {
            sink.merge_errors(met);
        }
        let library = if files.is_empty() {
            None
        } else {
            let files: Vec<LoadedFile> = files.into_iter().map(|file| self.read_file(file, checked, origin, budget, &mut sink)).collect();
            let mut resources = Vec::new();
            if origin == Origin::Shipped
                && let Some(text) = manifest
            {
                resources = self.read_manifest(&text, &shown, &name, &files, &mut sink);
            }
            Some(Library {
                name,
                checked,
                files,
                shown,
                origin,
                uses: Vec::new(),
                report: Report::new(self.limits.max_report),
                checked_elsewhere,
                resources,
            })
        };
        match (checked, library) {
            (true, library) => {
                self.report.merge(sink);
                library.map(|library| self.push(library))
            }
            (false, Some(mut library)) => {
                library.report = sink;
                Some(self.push(library))
            }
            (false, None) => None,
        }
    }

    fn push(&mut self, library: Library) -> usize {
        self.loaded.push(library);
        self.loaded.len() - 1
    }

    fn read_file(&mut self, file: SourceFile, checked: bool, origin: Origin, budget: &mut Budget, sink: &mut Report) -> LoadedFile {
        let SourceFile { shown, content } = file;
        let Content::Bytes(bytes) = content else {
            return LoadedFile { shown, model: None, imports: HashMap::new() };
        };
        let size = bytes.len() as u64;
        if budget.over || size > budget.left {
            budget.over = true;
            return LoadedFile { shown, model: None, imports: HashMap::new() };
        }
        budget.left -= size;
        // What is embedded in the tool is not the user's to limit, nor to fix.
        let shipped_limits = Limits::default();
        let limits = if origin == Origin::Shipped { &shipped_limits } else { self.limits };
        let names = &mut *self.names;
        let Read { file, source_map, mut report } = match last_resort(|| read(&bytes, names, limits)) {
            Ok(read) => read,
            Err(payload) => {
                let bug = self.libraries.last_panic().unwrap_or(payload);
                say(sink, checked, Diagnostic::new(Code::Lc0099, format!("internal error, which is a bug of the tool: {bug}"), shown.as_str()));
                return LoadedFile { shown, model: None, imports: HashMap::new() };
            }
        };
        report.set_file(&shown);
        if checked {
            sink.merge(report);
        } else {
            sink.merge_errors(report);
        }
        let model = file.map(|file| Model { file, source_map });
        if let Some(model) = &model {
            judge_the_kind_of_file(&shown, model, checked, sink);
        }
        LoadedFile { shown, model, imports: HashMap::new() }
    }

    /// What it declares as resources.
    fn read_manifest(&mut self, text: &str, shown: &str, name: &str, files: &[LoadedFile], sink: &mut Report) -> Vec<String> {
        let path = format!("{shown}/labcraft.toml");
        let manifest = match Manifest::parse(text) {
            Ok(manifest) => manifest,
            Err(why) => {
                sink.push(Diagnostic::new(Code::Lc0105, format!("cannot read this manifest: {why}"), path));
                return Vec::new();
            }
        };
        if manifest.name != name {
            let message = format!("this manifest names the library `{}`, whose name is `{name}`", manifest.name);
            sink.push(Diagnostic::new(Code::Lc0105, message, path.as_str()));
        }
        // A file that did not read has its own diagnostic, and may hold the native function named here.
        if files.iter().any(|file| file.model.is_none()) {
            return manifest.resources;
        }
        let native: HashSet<&str> = files
            .iter()
            .filter_map(|file| match &file.model.as_ref()?.file.decl {
                Decl::Extern(function) => Some(self.names.resolve(function.name)),
                _ => None,
            })
            .collect();
        for always in manifest.always.iter().filter(|always| !native.contains(normalize(always).as_ref())) {
            let message = format!("`always` can only mark a native function of this library, and `{always}` is none (spec 7.2)");
            sink.push(Diagnostic::new(Code::Lc0105, message, path.as_str()));
        }
        let opaque: HashSet<&str> = files
            .iter()
            .filter_map(|file| match &file.model.as_ref()?.file.decl {
                Decl::Type(declared) if is_opaque(&declared.ty, self.names) => Some(self.names.resolve(declared.name)),
                _ => None,
            })
            .collect();
        for resource in manifest.resources.iter().filter(|resource| !opaque.contains(normalize(resource).as_ref())) {
            let message = format!("`resources` can only name an opaque type of this library, and `{resource}` is none (spec 8.1)");
            sink.push(Diagnostic::new(Code::Lc0105, message, path.as_str()));
        }
        manifest.resources
    }

    /// The files of a library are set aside meanwhile, since what their `use` lines find joins the list of libraries.
    fn resolve_uses(&mut self, index: usize) {
        let Some(library) = self.loaded.get_mut(index) else { return };
        let (checked, origin, checked_elsewhere) = (library.checked, library.origin, library.checked_elsewhere);
        let mut files = std::mem::take(&mut library.files);
        let mut sink = std::mem::replace(&mut library.report, Report::new(self.limits.max_report));
        let mut uses = Vec::new();
        for (file_index, file) in files.iter_mut().enumerate() {
            let Some(model) = &file.model else { continue };
            file.imports = self.resolve_uses_of_file(file_index, &file.shown, model, (checked, origin), &mut uses, &mut sink);
        }
        if checked {
            self.report.merge(sink);
            sink = Report::new(self.limits.max_report);
        } else if checked_elsewhere {
            sink = Report::new(self.limits.max_report);
        }
        if let Some(library) = self.loaded.get_mut(index) {
            library.files = files;
            library.uses = uses;
            library.report = sink;
        }
    }

    fn resolve_uses_of_file(
        &mut self,
        file_index: usize,
        shown: &str,
        model: &Model,
        from: (bool, Origin),
        uses: &mut Vec<LibraryUse>,
        sink: &mut Report,
    ) -> HashMap<String, Target> {
        let (checked, origin) = from;
        let map = &model.source_map;
        let mut imports = HashMap::new();
        let mut seen: HashMap<String, Id> = HashMap::new();
        for line in &model.file.uses {
            let segments: Vec<Name> = line.path.segments().collect();
            let name = self.joined(&segments, ".");
            if let Some(first) = seen.get(&name) {
                let mut diagnostic = at(
                    Code::Lc2020,
                    format!("`{name}` is imported twice: `labcraft-cli fmt` removes this line, unless it has a comment"),
                    shown,
                    map,
                    line.path.id,
                );
                diagnostic.related =
                    vec![Related { message: format!("`{name}` is first imported here"), file: shown.to_owned(), span: map.span(*first) }];
                say(sink, checked, diagnostic);
                continue;
            }
            seen.insert(name.clone(), line.path.id);
            let target = self.target(&segments, &name, checked, origin);
            match target {
                Target::Library(target) => uses.push(LibraryUse { target, file: file_index, id: line.path.id }),
                Target::Missing => {
                    let message = format!(
                        "unknown library `{name}`: no primitive, sub-folder of the folder given, folder of a `--lib-path` or library shipped with LabCraft has this name"
                    );
                    say(sink, checked, at(Code::Lc2004, message, shown, map, line.path.id));
                }
                Target::Primitive | Target::Unreadable => {}
            }
            imports.insert(name, target);
        }
        imports
    }

    fn joined(&self, segments: &[Name], separator: &str) -> String {
        let texts: Vec<&str> = segments.iter().map(|segment| self.names.resolve(*segment)).collect();
        texts.join(separator)
    }

    /// The rules of `docs/cli.md`, section 3, in order: the first match wins. Only what is in the folder given sees the
    /// other folders in it, so a dependency says the same whoever reads it.
    fn target(&mut self, segments: &[Name], name: &str, checked: bool, origin: Origin) -> Target {
        if PrimitiveLibrary::from_name(name).is_some() {
            return Target::Primitive;
        }
        if checked && let Some(index) = self.project.get(&self.joined(segments, "/")) {
            return Target::Library(*index);
        }
        let key = (name.to_owned(), origin);
        if let Some(target) = self.found.get(&key) {
            return *target;
        }
        let target = self.find(name, origin);
        self.found.insert(key, target);
        target
    }

    fn find(&mut self, name: &str, by: Origin) -> Target {
        match self.libraries.find(name, by) {
            Found::NotFound => Target::Missing,
            Found::Unreadable(diagnostic) => {
                let mut report = Report::new(self.limits.max_report);
                let shown = diagnostic.file.clone();
                report.push(*diagnostic);
                self.unreadable.push(DependencyReport { shown, report });
                Target::Unreadable
            }
            Found::Library(source) => {
                // One shipped library, whoever asks: a project and `scpi` see the same `visa`, and so the same types.
                let shipped = (name.to_owned(), Origin::Shipped);
                let is_shipped = source.origin == Origin::Shipped;
                if is_shipped && let Some(target) = self.found.get(&shipped) {
                    return *target;
                }
                let shown = source.shown.clone();
                let mut budget = Budget { left: self.limits.max_library_size, over: false };
                let added = self.add(source, name.to_owned(), false, &mut budget);
                if budget.over
                    && let Some(library) = added.and_then(|index| self.loaded.get_mut(index))
                {
                    library.report.push(Diagnostic::new(Code::Lc0010, over_the_limit(self.limits), shown));
                }
                let target = added.map_or(Target::Missing, Target::Library);
                if is_shipped {
                    self.found.insert(shipped, target);
                }
                target
            }
        }
    }

    /// One diagnostic per cycle, on the first `use` of the first file that takes part in it (`docs/cli.md`, section 4.2).
    fn report_cycles(&mut self) {
        let edges: Vec<Vec<usize>> = self.loaded.iter().map(|library| library.uses.iter().map(|edge| edge.target).collect()).collect();
        for component in cycles(&edges) {
            self.report_cycle(&component);
        }
    }

    fn report_cycle(&mut self, component: &[usize]) {
        let members: HashSet<usize> = component.iter().copied().collect();
        let mut lines = Vec::new();
        let mut library_names = Vec::new();
        for index in component {
            let Some(library) = self.loaded.get(*index) else { continue };
            library_names.push(format!("`{}`", library.name));
            let Some(first) = library.uses.iter().find(|edge| members.contains(&edge.target)) else { continue };
            let Some(file) = library.files.get(first.file) else { continue };
            let span = file.model.as_ref().and_then(|model| model.source_map.span(first.id));
            lines.push((file.shown.clone(), span, *index));
        }
        lines.sort();
        library_names.sort();
        let names = library_names.join(", ");
        let mut lines = lines.into_iter();
        let Some((file, span, index)) = lines.next() else { return };
        let mut diagnostic = Diagnostic::new(Code::Lc2021, format!("libraries that use each other in a cycle: {names}"), file);
        diagnostic.span = span;
        diagnostic.related = lines.map(|(file, span, _)| Related { message: "this `use` is part of the cycle".to_owned(), file, span }).collect();
        match self.loaded.get_mut(index) {
            Some(library) if !library.checked => library.report.push(diagnostic),
            _ => self.report.push(diagnostic),
        }
    }
}

/// `LC0103` is about the extension, and `LC0104` about the declaration: a section belongs to a function (R-19).
fn judge_the_kind_of_file(shown: &str, model: &Model, checked: bool, sink: &mut Report) {
    let (file, map) = (&model.file, &model.source_map);
    let (is_type, id) = match &file.decl {
        Decl::Function(function) => (false, function.id),
        Decl::Extern(function) => (false, function.id),
        Decl::Type(ty) => (true, ty.id),
    };
    if is_type && shown.ends_with(".lcf") {
        let message = "a type goes in a `.lct` file, and a `.lcf` file holds a function";
        say(sink, checked, at(Code::Lc0103, message.to_owned(), shown, map, id));
    } else if !is_type && shown.ends_with(".lct") {
        let message = "a function goes in a `.lcf` file, and a `.lct` file holds a type";
        say(sink, checked, at(Code::Lc0103, message.to_owned(), shown, map, id));
    }
    if is_type {
        let sections = [
            file.panel.as_ref().map(|panel| ("panel", panel.id)),
            file.layout.as_ref().map(|layout| ("layout", layout.id)),
            file.icon.as_ref().map(|icon| ("icon", icon.id)),
        ];
        for (section, id) in sections.into_iter().flatten() {
            let message = format!("a `{section}` section belongs to a function, not to a type");
            say(sink, checked, at(Code::Lc0104, message, shown, map, id));
        }
    }
}

pub(crate) fn at(code: Code, message: String, shown: &str, map: &SourceMap, id: Id) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(code, message, shown);
    diagnostic.span = map.span(id);
    diagnostic
}

/// A panic is a bug, reported on what it stopped while the rest goes on (`docs/architecture.md`, section 5). WASM traps
/// instead, and its host runs this work in a worker that it can restart.
pub(crate) fn last_resort<T>(work: impl FnOnce() -> T) -> Result<T, String> {
    panic::catch_unwind(AssertUnwindSafe(work)).map_err(|payload| panic_text(payload.as_ref()))
}

fn panic_text(payload: &(dyn Any + Send)) -> String {
    match (payload.downcast_ref::<&str>(), payload.downcast_ref::<String>()) {
        (Some(text), _) => (*text).to_owned(),
        (_, Some(text)) => text.clone(),
        _ => "a panic".to_owned(),
    }
}

/// The groups of nodes that reach each other, by Tarjan's algorithm, with a stack of its own: how long a chain of
/// libraries is depends on the program. A node alone counts only if it has an edge to itself.
pub(crate) fn cycles(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    #[derive(Clone, Copy, Default)]
    struct State {
        order: Option<usize>,
        low: usize,
        waiting: bool,
    }
    let mut states = vec![State::default(); edges.len()];
    let mut waiting = Vec::new();
    let mut found = Vec::new();
    let mut count = 0;
    for start in 0..edges.len() {
        if states.get(start).is_none_or(|state| state.order.is_some()) {
            continue;
        }
        let mut frames = vec![(start, 0)];
        while let Some(&(node, next)) = frames.last() {
            if next == 0
                && let Some(state) = states.get_mut(node)
            {
                *state = State { order: Some(count), low: count, waiting: true };
                count += 1;
                waiting.push(node);
            }
            let child = edges.get(node).and_then(|children| children.get(next)).copied();
            if let Some(frame) = frames.last_mut() {
                frame.1 += 1;
            }
            if let Some(child) = child {
                let seen = states.get(child).copied().unwrap_or_default();
                match seen.order {
                    None => frames.push((child, 0)),
                    Some(order) if seen.waiting => {
                        if let Some(state) = states.get_mut(node) {
                            state.low = state.low.min(order);
                        }
                    }
                    Some(_) => {}
                }
                continue;
            }
            frames.pop();
            let state = states.get(node).copied().unwrap_or_default();
            if let Some(&(parent, _)) = frames.last()
                && let Some(parent) = states.get_mut(parent)
            {
                parent.low = parent.low.min(state.low);
            }
            if state.order == Some(state.low) {
                let mut component = Vec::new();
                while let Some(member) = waiting.pop() {
                    if let Some(state) = states.get_mut(member) {
                        state.waiting = false;
                    }
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                let on_itself = edges.get(node).is_some_and(|children| children.contains(&node));
                if component.len() > 1 || on_itself {
                    found.push(component);
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;
    use crate::check;

    const FUNCTION: &[u8] = b"labcraft 1\n\nfn f() -> () {}\n";

    /// A file of another name than `f` declares a function of its name (R-22), as `fn f` would in `f.lcf`.
    fn file(shown: &str, bytes: &[u8]) -> SourceFile {
        let stem = crate::file_name::stem(shown);
        let bytes = match std::str::from_utf8(bytes) {
            Ok(text) if stem != "f" && labcraft_format::is_name(&stem) => text.replace("fn f(", &format!("fn {stem}(")).into_bytes(),
            _ => bytes.to_vec(),
        };
        SourceFile { shown: shown.to_owned(), content: Content::Bytes(Cow::Owned(bytes)) }
    }

    fn library(name: &str, files: Vec<SourceFile>) -> LibrarySource {
        LibrarySource { files, ..LibrarySource::new(name.to_owned(), name.to_owned(), Origin::Project) }
    }

    fn uses(lines: &[&str]) -> Vec<u8> {
        let lines: String = lines.iter().map(|name| format!("use {name}\n")).collect();
        format!("labcraft 1\n\n{lines}\nfn f() -> () {{}}\n").into_bytes()
    }

    struct Nothing;

    impl Libraries for Nothing {
        fn find(&self, _: &str, _: Origin) -> Found {
            Found::NotFound
        }
    }

    /// A host that answers as a test says.
    struct Answering<F: Fn(&str, Origin) -> Found>(F);

    impl<F: Fn(&str, Origin) -> Found> Libraries for Answering<F> {
        fn find(&self, name: &str, by: Origin) -> Found {
            (self.0)(name, by)
        }
    }

    fn codes(root: LibrarySource, libraries: &dyn Libraries, limits: &Limits) -> Vec<&'static str> {
        let mut checked = check(root, libraries, &mut Interner::new(), limits);
        for dependency in checked.dependencies {
            checked.report.merge(dependency.report);
        }
        checked.report.finish();
        checked.report.diagnostics().iter().map(|diagnostic| diagnostic.code().as_str()).collect()
    }

    fn codes_of_files(files: Vec<SourceFile>) -> Vec<&'static str> {
        codes(library("", files), &Nothing, &Limits::default())
    }

    #[test]
    fn a_clean_folder_is_counted() {
        let mut root = library("", vec![file("f.lcf", FUNCTION), file("T.lct", b"labcraft 1\n\ntype T = f64\n")]);
        root.sub_libraries.push(library("sub", vec![file("sub/g.lcf", FUNCTION)]));
        let checked = check(root, &Nothing, &mut Interner::new(), &Limits::default());
        assert_eq!(checked.report.errors() + checked.report.warnings(), 0);
        assert_eq!((checked.counts.libraries, checked.counts.functions, checked.counts.types), (2, 2, 1));
        assert_eq!(checked.files.len(), 3);
    }

    #[test]
    fn a_folder_without_files_of_its_own_is_no_library() {
        let mut middle = library("a", Vec::new());
        middle.sub_libraries.push(library("b", vec![file("a/b/g.lcf", FUNCTION)]));
        let mut root = library("", vec![file("f.lcf", &uses(&["a.b"]))]);
        root.sub_libraries.push(middle);
        assert_eq!(codes(root, &Nothing, &Limits::default()), Vec::<&str>::new());
        let mut middle = library("a", Vec::new());
        middle.sub_libraries.push(library("b", vec![file("a/b/g.lcf", FUNCTION)]));
        let mut root = library("", vec![file("f.lcf", &uses(&["a"]))]);
        root.sub_libraries.push(middle);
        assert_eq!(codes(root, &Nothing, &Limits::default()), ["LC2004"]);
    }

    #[test]
    fn a_bad_file_hides_nothing_of_the_others() {
        let found = codes_of_files(vec![file("a.lcf", b"labcraft 1\n\nfn a( {\n"), file("b.lcf", &uses(&["nowhere"])), file("c.lcf", b"x")]);
        assert_eq!(found, ["LC1001", "LC2004", "LC0101"]);
    }

    #[test]
    fn a_file_in_the_wrong_kind_of_file_is_refused_and_still_known() {
        let ty = b"labcraft 1\n\ntype T = f64\n";
        let extern_ = b"labcraft 1\n\nextern rust(crate: \"a\", function: \"b\")\nfn g() -> (err: error)\n";
        let other = b"labcraft 1\n\ntype U = f64\n";
        let found = codes_of_files(vec![file("T.lcf", ty), file("f.lct", FUNCTION), file("g.lct", extern_), file("U.lct", other)]);
        assert_eq!(found, ["LC0103", "LC0103", "LC0103"]);
        let checked = check(library("", vec![file("T.lcf", ty), file("f.lct", FUNCTION)]), &Nothing, &mut Interner::new(), &Limits::default());
        assert_eq!((checked.counts.functions, checked.counts.types), (1, 1));
    }

    #[test]
    fn each_section_of_a_type_is_refused() {
        let text = b"labcraft 1\n\ntype T = f64\n\npanel {\n}\n\nlayout {\n}\n\nicon 32 {\n}\n";
        assert_eq!(codes_of_files(vec![file("T.lct", text)]), ["LC0104", "LC0104", "LC0104"]);
    }

    #[test]
    fn a_library_imported_twice_is_a_warning_and_a_related_place() {
        let root = library("", vec![file("f.lcf", &uses(&["time", "time"]))]);
        let mut checked = check(root, &Nothing, &mut Interner::new(), &Limits::default());
        checked.report.finish();
        let diagnostics = checked.report.diagnostics();
        assert_eq!(diagnostics.iter().map(|d| d.code().as_str()).collect::<Vec<_>>(), ["LC2020"]);
        assert_eq!(diagnostics[0].span.unwrap().start.line, 4);
        assert_eq!(diagnostics[0].related[0].span.unwrap().start.line, 3);
        assert_eq!(checked.report.status(false), labcraft_types::Status::Ok);
        assert_eq!(checked.report.status(true), labcraft_types::Status::Error);
    }

    #[test]
    fn the_primitives_are_found_and_cannot_be_replaced() {
        for name in ["core", "string", "array", "errors", "time", "ui"] {
            assert_eq!(codes_of_files(vec![file("f.lcf", &uses(&[name]))]), Vec::<&str>::new(), "{name}");
        }
        let asked = std::cell::Cell::new(false);
        let libraries = Answering(|_: &str, _| {
            asked.set(true);
            Found::NotFound
        });
        let mut root = library("", vec![file("f.lcf", &uses(&["string"]))]);
        root.sub_libraries.push(library("string", vec![file("string/g.lcf", b"not a program")]));
        assert_eq!(codes(root, &libraries, &Limits::default()), ["LC0101"]);
        assert!(!asked.get());
    }

    #[test]
    fn a_sub_folder_comes_before_the_host_and_names_are_exact_and_normalized() {
        let refuse = Answering(|name: &str, _| panic!("asked for {name}"));
        let mut root = library("", vec![file("f.lcf", "labcraft 1\n\nuse caf\u{E9}\n\nfn f() -> () {}\n".as_bytes())]);
        root.sub_libraries.push(library("cafe\u{301}", vec![file("g.lcf", FUNCTION)]));
        assert_eq!(codes(root, &refuse, &Limits::default()), Vec::<&str>::new());
        let mut root = library("", vec![file("f.lcf", &uses(&["VISA"]))]);
        root.sub_libraries.push(library("visa", vec![file("visa/g.lcf", FUNCTION)]));
        assert_eq!(codes(root, &Nothing, &Limits::default()), ["LC2004"]);
    }

    #[test]
    fn folders_side_by_side_follow_the_rules_of_files() {
        let mut root = library("", vec![file("f.lcf", FUNCTION)]);
        root.sub_libraries.push(library("Visa", vec![file("Visa/g.lcf", FUNCTION)]));
        root.sub_libraries.push(library("visa", vec![file("visa/g.lcf", FUNCTION)]));
        root.sub_libraries.push(library("nul", vec![file("nul/g.lcf", FUNCTION)]));
        root.sub_libraries.push(library("caf\u{E9}", vec![file("caf\u{E9}/g.lcf", FUNCTION)]));
        root.sub_libraries.push(library("cafe\u{301}", vec![file("cafe\u{301}/g.lcf", FUNCTION)]));
        let mut checked = check(root, &Nothing, &mut Interner::new(), &Limits::default());
        checked.report.finish();
        let found: Vec<(&str, &str)> =
            checked.report.diagnostics().iter().map(|diagnostic| (diagnostic.code().as_str(), diagnostic.file.as_str())).collect();
        assert_eq!(found, [("LC2024", "caf\u{E9}"), ("LC2025", "nul"), ("LC2024", "visa")], "once for each pair, on the last one");
        assert!(checked.report.diagnostics().iter().all(|diagnostic| !diagnostic.message.contains("  ")));
        let mut inner = library("a", vec![file("a/f.lcf", FUNCTION)]);
        inner.sub_libraries.push(library("Aux", vec![file("a/Aux/g.lcf", FUNCTION)]));
        let mut root = library("", vec![file("f.lcf", FUNCTION)]);
        root.sub_libraries.push(inner);
        assert_eq!(codes(root, &Nothing, &Limits::default()), ["LC2025"], "at any depth");
    }

    #[test]
    fn a_folder_with_a_dot_in_its_name_is_not_a_path() {
        let mut root = library("", vec![file("f.lcf", &uses(&["a.b"]))]);
        root.sub_libraries.push(library("a.b", vec![file("a.b/g.lcf", FUNCTION)]));
        assert_eq!(codes(root, &Nothing, &Limits::default()), ["LC2004"]);
    }

    #[test]
    fn nothing_can_use_the_folder_given() {
        let root = library("project", vec![file("f.lcf", &uses(&["project"]))]);
        assert_eq!(codes(root, &Nothing, &Limits::default()), ["LC2004"]);
    }

    fn shipped(name: &str, files: Vec<SourceFile>) -> LibrarySource {
        LibrarySource { origin: Origin::Shipped, shown: format!("<labcraft>/{name}"), ..library(name, files) }
    }

    /// `visa` is in a project and shipped, and the shipped `scpi` uses `visa`: the rule of `docs/cli.md`, section 3.
    #[test]
    fn a_shipped_library_sees_only_what_is_shipped() {
        let asked = std::cell::RefCell::new(Vec::new());
        let libraries = Answering(|name: &str, by| {
            asked.borrow_mut().push((name.to_owned(), by));
            match name {
                "scpi" => Found::Library(shipped("scpi", vec![file("<labcraft>/scpi/q.lcf", &uses(&["visa"]))])),
                "visa" => Found::Library(shipped("visa", vec![file("<labcraft>/visa/o.lcf", FUNCTION)])),
                _ => Found::NotFound,
            }
        });
        let mut root = library("", vec![file("f.lcf", &uses(&["scpi", "visa"]))]);
        root.sub_libraries.push(library("visa", vec![file("visa/g.lcf", FUNCTION)]));
        let checked = check(root, &libraries, &mut Interner::new(), &Limits::default());
        assert_eq!(checked.report.errors() + checked.report.warnings(), 0);
        assert_eq!(*asked.borrow(), [("scpi".to_owned(), Origin::Project), ("visa".to_owned(), Origin::Shipped)]);
        assert_eq!(checked.counts.libraries, 2, "dependencies are not counted");
        assert_eq!(checked.files.len(), 2, "dependencies are not among the files checked");
    }

    #[test]
    fn a_project_and_a_shipped_library_share_the_shipped_library_they_both_use() {
        let calls = std::cell::Cell::new(0);
        let libraries = Answering(|name: &str, _| {
            calls.set(calls.get() + 1);
            match name {
                "scpi" => Found::Library(shipped("scpi", vec![file("<labcraft>/scpi/q.lcf", &uses(&["visa"]))])),
                "visa" => Found::Library(shipped("visa", vec![file("<labcraft>/visa/o.lcf", FUNCTION)])),
                _ => Found::NotFound,
            }
        });
        let loaded = |lines: &[&str]| {
            let root = library("", vec![file("f.lcf", &uses(lines))]);
            load(root, &libraries, &mut Interner::new(), &Limits::default(), &mut Report::new(10)).0.len()
        };
        assert_eq!(loaded(&["scpi", "visa"]), 3);
        assert_eq!(loaded(&["visa", "scpi"]), 3);
        assert!(calls.get() >= 4);
    }

    #[test]
    fn a_dependency_gives_its_errors_and_keeps_its_warnings() {
        let libraries = Answering(|name: &str, _| match name {
            "warned" => Found::Library(library("warned", vec![file("warned/a.lcf", &uses(&["time", "time"]))])),
            "broken" => Found::Library(library("broken", vec![file("broken/a.lcf", b"labcraft 1\n\nfn a( {\n")])),
            "lost" => Found::Library(library("lost", vec![file("lost/a.lcf", &uses(&["nowhere"]))])),
            "locked" => Found::Unreadable(Box::new(Diagnostic::new(Code::Lc0001, "cannot read", "locked"))),
            _ => Found::NotFound,
        });
        let root = library("", vec![file("f.lcf", &uses(&["broken", "locked", "lost", "warned"]))]);
        assert_eq!(codes(root, &libraries, &Limits::default()), ["LC1001", "LC0001", "LC2004"]);
    }

    #[test]
    fn a_dependency_with_a_body_that_does_not_parse_is_the_cause_and_is_located() {
        let libraries = Answering(|_: &str, _| Found::Library(library("dep", vec![file("dep/a.lcf", b"labcraft 1\n\nfn a( {\n")])));
        let checked = check(library("", vec![file("f.lcf", &uses(&["dep"]))]), &libraries, &mut Interner::new(), &Limits::default());
        assert_eq!(checked.report.errors(), 0, "it is said apart, since several folders can read it");
        assert_eq!(checked.dependencies.len(), 1);
        assert_eq!(checked.dependencies[0].shown, "dep");
        assert_eq!(checked.dependencies[0].report.errors(), 1);
        assert_eq!(checked.dependencies[0].report.diagnostics()[0].file, "dep/a.lcf");
    }

    #[test]
    fn libraries_that_use_each_other_give_one_diagnostic_with_the_others_related() {
        let mut root = library("", vec![file("f.lcf", &uses(&["a"]))]);
        root.sub_libraries.push(library("a", vec![file("a/x.lcf", &uses(&["b"]))]));
        root.sub_libraries.push(library("b", vec![file("b/x.lcf", &uses(&["c"]))]));
        root.sub_libraries.push(library("c", vec![file("c/x.lcf", &uses(&["a"]))]));
        let mut checked = check(root, &Nothing, &mut Interner::new(), &Limits::default());
        checked.report.finish();
        let diagnostics = checked.report.diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!((diagnostics[0].code(), diagnostics[0].file.as_str()), (Code::Lc2021, "a/x.lcf"));
        assert_eq!(diagnostics[0].span.unwrap().start.line, 3);
        let related: Vec<&str> = diagnostics[0].related.iter().map(|related| related.file.as_str()).collect();
        assert_eq!(related, ["b/x.lcf", "c/x.lcf"]);
        assert!(diagnostics[0].message.contains("`a`, `b`, `c`"), "{}", diagnostics[0].message);
    }

    #[test]
    fn a_library_that_uses_itself_is_a_cycle_and_so_is_one_through_a_dependency() {
        let mut root = library("", Vec::new());
        root.sub_libraries.push(library("a", vec![file("a/x.lcf", &uses(&["a"]))]));
        assert_eq!(codes(root, &Nothing, &Limits::default()), ["LC2021"]);
        let libraries = Answering(|name: &str, _| match name {
            "d" => Found::Library(library("d", vec![file("d/x.lcf", &uses(&["e"]))])),
            "e" => Found::Library(library("e", vec![file("e/x.lcf", &uses(&["d"]))])),
            _ => Found::NotFound,
        });
        assert_eq!(codes(library("", vec![file("f.lcf", &uses(&["d"]))]), &libraries, &Limits::default()), ["LC2021"]);
    }

    #[test]
    fn no_chain_of_libraries_takes_the_stack() {
        let libraries = Answering(|name: &str, _| {
            let number: u32 = name.strip_prefix('l').unwrap().parse().unwrap();
            if number == 100_000 {
                return Found::NotFound;
            }
            let next = format!("l{}", number + 1);
            Found::Library(library(name, vec![file("x.lcf", &uses(&[&next]))]))
        });
        // On the default stack of a test thread, far below what a recursion over 100,000 libraries takes.
        assert_eq!(codes(library("", vec![file("f.lcf", &uses(&["l0"]))]), &libraries, &Limits::default()), ["LC2004"]);
    }

    #[test]
    fn what_has_no_cycle_has_no_report() {
        assert_eq!(cycles(&[vec![1, 2], vec![2], vec![]]), Vec::<Vec<usize>>::new());
        assert_eq!(cycles(&[]), Vec::<Vec<usize>>::new());
        let mut two = cycles(&[vec![1], vec![0], vec![3], vec![2], vec![0, 5], vec![5], vec![5]]);
        two.iter_mut().for_each(|component| component.sort_unstable());
        two.sort();
        assert_eq!(two, [vec![0, 1], vec![2, 3], vec![5]]);
    }

    #[test]
    fn a_host_error_stays_with_the_file_it_could_not_read() {
        let mut root = library("", vec![SourceFile { shown: "locked.lcf".to_owned(), content: Content::Unread }, file("f.lcf", FUNCTION)]);
        root.report.push(Diagnostic::new(Code::Lc0001, "cannot read", "locked.lcf"));
        let checked = check(root, &Nothing, &mut Interner::new(), &Limits::default());
        assert_eq!(checked.report.errors(), 1);
        assert_eq!((checked.counts.libraries, checked.counts.functions), (1, 1));
    }

    #[test]
    fn a_library_over_its_limit_keeps_what_comes_first() {
        let mut limits = Limits::default();
        limits.max_library_size = FUNCTION.len() as u64 + 1;
        let mut root = library("", vec![file("a.lcf", FUNCTION), file("b.lcf", b"not a program"), file("c.lcf", b"")]);
        root.sub_libraries.push(library("sub", vec![file("sub/g.lcf", FUNCTION)]));
        let checked = check(root, &Nothing, &mut Interner::new(), &limits);
        assert_eq!(checked.report.errors(), 1);
        assert_eq!(checked.report.diagnostics()[0].code(), Code::Lc0010);
        assert_eq!(checked.files.iter().map(|file| file.shown.as_str()).collect::<Vec<_>>(), ["a.lcf"]);
    }

    fn shipped_with_manifest(text: &str, files: Vec<SourceFile>) -> LibrarySource {
        LibrarySource { manifest: Some(text.to_owned()), ..shipped("dep", files) }
    }

    fn codes_of_manifest(text: &str) -> Vec<&'static str> {
        let extern_ = b"labcraft 1\n\nextern rust(crate: \"a\", function: \"b\")\nfn close() -> (err: error)\n";
        let text = text.to_owned();
        let libraries = Answering(move |_: &str, _| {
            Found::Library(shipped_with_manifest(&text, vec![file("<labcraft>/dep/close.lcf", extern_), file("<labcraft>/dep/open.lcf", FUNCTION)]))
        });
        codes(library("", vec![file("f.lcf", &uses(&["dep"]))]), &libraries, &Limits::default())
    }

    #[test]
    fn resources_names_an_opaque_type_of_the_library() {
        let codes_with = |resources: &str| {
            let text = format!("name = \"dep\"\nversion = \"1\"\nresources = [{resources}]\n");
            let libraries = Answering(move |_: &str, _| {
                let files = vec![
                    file("<labcraft>/dep/open.lcf", FUNCTION),
                    file("<labcraft>/dep/link.lct", b"labcraft 1\n\ntype link = opaque\n"),
                    file("<labcraft>/dep/other.lct", b"labcraft 1\n\ntype other = struct { x: f64 }\n"),
                ];
                Found::Library(shipped_with_manifest(&text, files))
            });
            codes(library("", vec![file("f.lcf", &uses(&["dep"]))]), &libraries, &Limits::default())
        };
        assert_eq!(codes_with("\"link\""), Vec::<&str>::new());
        assert_eq!(codes_with("\"lnik\""), ["LC0105"], "a typo would silence the warnings of 8.1");
        assert_eq!(codes_with("\"other\""), ["LC0105"], "a struct is not opaque");
    }

    #[test]
    fn a_manifest_is_read_for_a_shipped_library_and_ignored_for_any_other() {
        assert_eq!(codes_of_manifest("name = \"dep\"\nversion = \"1\"\nalways = [\"close\"]\n"), Vec::<&str>::new());
        assert_eq!(codes_of_manifest("name = \"dep\""), ["LC0105"]);
        assert_eq!(codes_of_manifest("name = \"other\"\nversion = \"1\"\n"), ["LC0105"]);
        assert_eq!(codes_of_manifest("name = \"dep\"\nversion = \"1\"\nalways = [\"open\"]\n"), ["LC0105"]);
        assert_eq!(codes_of_manifest("name = \"dep\"\nversion = \"1\"\nalways = [\"nowhere\"]\n"), ["LC0105"]);
        let mut root = library("", vec![file("f.lcf", FUNCTION)]);
        root.manifest = Some("nonsense".to_owned());
        assert_eq!(codes(root, &Nothing, &Limits::default()), Vec::<&str>::new());
    }

    /// A list cut to nothing still counts: the status of a run comes from the counts.
    #[test]
    fn a_report_cut_to_nothing_still_counts_the_errors_of_a_dependency() {
        let libraries = Answering(|_: &str, _| Found::Library(library("dep", vec![file("dep/a.lcf", b"labcraft 1\n\nfn a( {\n")])));
        let mut limits = Limits::default();
        limits.max_report = 0;
        let checked = check(library("", vec![file("f.lcf", &uses(&["dep"]))]), &libraries, &mut Interner::new(), &limits);
        assert_eq!(checked.dependencies[0].report.errors(), 1);
    }

    #[test]
    fn a_dependency_sees_the_primitives_the_host_and_nothing_of_the_folder_given() {
        let libraries = Answering(|name: &str, _| match name {
            "dep" => Found::Library(library("dep", vec![file("dep/a.lcf", &uses(&["here"]))])),
            _ => Found::NotFound,
        });
        let mut root = library("", vec![file("f.lcf", &uses(&["dep"]))]);
        root.sub_libraries.push(library("here", vec![file("here/a.lcf", FUNCTION)]));
        assert_eq!(codes(root, &libraries, &Limits::default()), ["LC2004"]);
    }

    #[test]
    fn what_is_checked_elsewhere_is_read_and_says_only_a_cycle_here() {
        let libraries = Answering(|_: &str, _| {
            let mut source = library("dep", vec![file("dep/a.lcf", b"labcraft 1\n\nfn a( {\n")]);
            source.checked_elsewhere = true;
            Found::Library(source)
        });
        let checked = check(library("", vec![file("f.lcf", &uses(&["dep"]))]), &libraries, &mut Interner::new(), &Limits::default());
        assert_eq!(checked.report.errors(), 0);
        assert!(checked.dependencies.is_empty());
        let libraries = Answering(|_: &str, _| {
            let mut source = library("dep", vec![file("dep/a.lcf", &uses(&["dep", "nowhere"]))]);
            source.checked_elsewhere = true;
            Found::Library(source)
        });
        assert_eq!(codes(library("", vec![file("f.lcf", &uses(&["dep"]))]), &libraries, &Limits::default()), ["LC2021"]);
    }

    #[test]
    fn the_limits_of_the_user_do_not_apply_to_what_is_embedded() {
        let text = "name = \"dep\"\nversion = \"1\"\nalways = [\"close\"]\n".to_owned();
        let extern_ = b"labcraft 1\n\nextern rust(crate: \"a\", function: \"b\")\nfn close() -> (err: error)\n";
        let libraries = Answering(move |_: &str, _| Found::Library(shipped_with_manifest(&text, vec![file("<labcraft>/dep/close.lcf", extern_)])));
        let mut limits = Limits::default();
        limits.max_file_size = 60;
        let text = uses(&["dep"]);
        assert!(text.len() < 60);
        assert_eq!(codes(library("", vec![file("f.lcf", &text)]), &libraries, &limits), Vec::<&str>::new());
    }

    #[test]
    fn a_function_that_did_not_read_may_be_the_one_marked_always() {
        let text = "name = \"dep\"\nversion = \"1\"\nalways = [\"close\"]\n".to_owned();
        let libraries = Answering(move |_: &str, _| {
            Found::Library(shipped_with_manifest(&text, vec![file("<labcraft>/dep/close.lcf", b"labcraft 1\n\nfn a( {\n")]))
        });
        assert_eq!(codes(library("", vec![file("f.lcf", &uses(&["dep"]))]), &libraries, &Limits::default()), ["LC1001"]);
    }

    #[test]
    fn a_panic_is_an_internal_error_with_its_cause() {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(|_| {}));
        let text: Result<(), String> = last_resort(|| panic!("a bug"));
        let formatted: Result<(), String> = last_resort(|| panic!("a bug of {}", "f"));
        let other: Result<(), String> = last_resort(|| panic::panic_any(1));
        panic::set_hook(previous);
        assert_eq!(text.unwrap_err(), "a bug");
        assert_eq!(formatted.unwrap_err(), "a bug of f");
        assert_eq!(other.unwrap_err(), "a panic");
    }
}
