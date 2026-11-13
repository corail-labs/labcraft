//! What `labcraft-cli diagram`, the command `diagram.svg` and the viewer share (`docs/cli.md`, 7.1, 7.2 and 7.5;
//! `docs/commands.md`, 6.5): a folder read and checked as `check` does, through the [`Workspace`], its functions named
//! from the folder, the drawing of each, the picture that shows it, with its marks, or its diagnostics when it is not
//! drawable, and the report `labcraft-diagram/1`.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use labcraft_check::{Checked, check};
use labcraft_diagram::{DiagramLimits, Drawing, Drawn, MAX_LINES, Overlay, Theme, ViewState, draw, mark, svg_with, undrawn};
use labcraft_types::{CheckCounts, CheckSummary, Code, Diagnostic, Fix, Interner, Limits, Report, Status, cut_text};
use serde::Serialize;
use serde_json::{Value, json};

use crate::error::CommandError;
use crate::folders::{Host, library_of, readable_lib_paths, roots};
use crate::panics::last_panic;
use crate::params::Params;
use crate::paths::{self, Place, Wanted};
use crate::session::Session;
use crate::shipped::Shipped;
use crate::walk::{self, slashed};
use crate::workspace::Workspace;

/// The schema of the report, which the attributes of the SVG share (`docs/cli.md`, 7.2 and 7.5).
pub const SCHEMA: &str = "labcraft-diagram/1";

/// A function: named from the folder, as `run` names it, by the path of its file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    /// Its file, as reports show it.
    pub shown: String,
    /// The path of its file under the folder, as the SVG shows it.
    pub source: String,
    /// The path of its SVG under `DIR`: that of its file, with `.svg`.
    pub relative: PathBuf,
}

/// A folder read and checked.
pub struct Loaded {
    pub report: Report,
    pub counts: CheckCounts,
    pub checked: Option<Checked>,
    pub names: Interner,
    pub functions: Vec<Function>,
    /// The folder, as reports show it; none when it could not be read.
    pub root: Option<String>,
    /// The SHA-256 of each file of the folder that `check` read, by its name in reports: what a trace is judged
    /// against (notation 6.1).
    pub hashes: Vec<(String, String)>,
}

/// Reads and checks `folder` with the folders of `--lib-path`, its paths shown from `current`, keeping the first
/// diagnostics of each file for its picture, whatever `--max-report` lists (`docs/cli.md`, 7.2).
pub fn load(ws: &dyn Workspace, shipped: &Shipped, folder: &Path, lib_paths: &[PathBuf], limits: &Limits, current: Option<&Path>) -> Loaded {
    let mut limits = limits.clone();
    limits.keep_per_file = MAX_LINES;
    let limits = &limits;
    let folders = [folder.to_path_buf()];
    let mut walked = walk::walk(ws, &folders, current, limits);
    let mut report = std::mem::replace(&mut walked.report, Report::for_limits(limits));
    let lib_paths = readable_lib_paths(ws, &folders, lib_paths, current, &mut report);
    let in_the_walk: HashSet<PathBuf> =
        walked.files.iter().map(|found| found.path.clone()).chain(walked.refused.iter().map(|refused| refused.path.clone())).collect();
    let host = Host { ws, shipped, lib_paths: &lib_paths, in_the_walk: &in_the_walk, current, limits, seen: None };
    let mut names = Interner::new();
    let Some(root) = roots(ws, &folders, current).into_iter().next() else {
        return Loaded { report, counts: CheckCounts::default(), checked: None, names, functions: Vec::new(), root: None, hashes: Vec::new() };
    };
    let functions = functions_of(&root.real, &walked);
    let has_types = walked.files.iter().any(|found| found.path.extension().is_some_and(|extension| extension == "lct"));
    let mut counts = CheckCounts::default();
    let mut hashes = Vec::new();
    let checked = match library_of(ws, &root.real, &root.shown, &mut walked, limits, None) {
        Some(source) => {
            crate::folders::hash_files(&source, &mut hashes);
            let work = || check(source, &host, &mut names, limits);
            match panic::catch_unwind(AssertUnwindSafe(work)) {
                Ok(mut checked) => {
                    report.merge(std::mem::replace(&mut checked.report, Report::for_limits(limits)));
                    for dependency in std::mem::take(&mut checked.dependencies) {
                        report.merge(dependency.report);
                    }
                    counts = checked.counts;
                    Some(checked)
                }
                Err(_) => {
                    let message = format!("internal error, which is a bug of the tool: {}", last_panic());
                    report.push(Diagnostic::new(Code::Lc0099, message, root.shown.as_str()));
                    None
                }
            }
        }
        None => None,
    };
    if functions.is_empty() && has_types {
        report.push(Diagnostic::new(Code::Lc7003, "no function to draw here, only types", root.shown.as_str()));
    }
    Loaded { report, counts, checked, names, functions, root: Some(root.shown), hashes }
}

/// Every `.lcf` file under the folder, read or not, named from the folder by its path: `dmm_driver/measure.lcf` is
/// `dmm_driver.measure`. A file whose name is not that of its function gets `LC2023`, which keeps it from being drawn.
fn functions_of(root: &Path, walked: &walk::Walked) -> Vec<Function> {
    let found =
        walked.files.iter().map(|found| (&found.path, &found.shown)).chain(walked.refused.iter().map(|refused| (&refused.path, &refused.shown)));
    let mut functions: Vec<Function> = found
        .filter(|(path, _)| path.extension().is_some_and(|extension| extension == "lcf"))
        .filter_map(|(path, shown)| {
            let relative = path.strip_prefix(root).ok()?;
            let parts: Vec<String> = relative.with_extension("").components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect();
            Some(Function { name: parts.join("."), shown: shown.clone(), source: slashed(relative), relative: relative.with_extension("svg") })
        })
        .collect();
    functions.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.shown.cmp(&b.shown)));
    functions
}

/// The index of each file in `Checked::files`, by its name in reports.
pub fn files_of(checked: Option<&Checked>) -> HashMap<&str, usize> {
    checked.map(|checked| checked.files.iter().enumerate().map(|(index, file)| (file.shown.as_str(), index)).collect()).unwrap_or_default()
}

/// A drawing or none, the nodes counted, the codes that keep it from being drawn.
pub type Drawn3 = (Option<Drawing>, Option<u64>, Vec<Code>);

/// The drawing of a function, or none when it is not drawable (`docs/cli.md`, 7.1); `LC7002` and a bug of the tool
/// join the report.
pub fn drawing_of(
    function: &Function,
    checked: Option<&Checked>,
    files: &HashMap<&str, usize>,
    names: &Interner,
    view: &ViewState,
    limits: &DiagramLimits,
    report: &mut Report,
) -> Drawn3 {
    let mut codes = report.keeping_from_drawing(&function.shown);
    let (Some(checked), Some(&index)) = (checked, files.get(function.shown.as_str())) else { return (None, None, codes) };
    let nodes = labcraft_diagram::count_nodes(checked, index, names);
    if !codes.is_empty() {
        return (None, nodes, codes);
    }
    let work = || draw(checked, index, names, view, limits);
    match panic::catch_unwind(AssertUnwindSafe(work)) {
        Ok(Drawn::Drawing(mut drawing)) => {
            // Its warnings (`LC7005`) join those of `check` for the file.
            for warning in std::mem::take(&mut drawing.warnings) {
                report.push(warning);
            }
            (Some(drawing), nodes, codes)
        }
        Ok(Drawn::NotDrawn(diagnostics)) => {
            for diagnostic in diagnostics {
                codes.push(diagnostic.code());
                report.push(diagnostic);
            }
            codes.sort();
            codes.dedup();
            (None, nodes, codes)
        }
        Err(_) => {
            let message = format!("internal error, which is a bug of the tool: {}", last_panic());
            report.push(Diagnostic::new(Code::Lc0099, message, function.shown.as_str()));
            (None, nodes, vec![Code::Lc0099])
        }
    }
}

/// What shows a function: its drawing with the marks of the first diagnostics of its file, or, when it is not drawn,
/// its frame and those diagnostics (`docs/cli.md`, 7.2).
pub fn picture(function: &Function, drawing: Option<Drawing>, report: &Report) -> Drawing {
    let of_file = report.first_of(&function.shown);
    match drawing {
        Some(mut drawing) => {
            mark(&mut drawing, &of_file);
            drawing
        }
        None => {
            let shown: Vec<Diagnostic> = of_file
                .into_iter()
                .map(|mut diagnostic| {
                    diagnostic.file.clone_from(&function.source);
                    diagnostic
                })
                .collect();
            let counts = report.counts_of(&function.shown);
            undrawn(&cut_text(&function.name), &shown, counts.errors + counts.warnings)
        }
    }
}

/// The functions asked for, in code point order of their names, or every one when none is; an unknown name gets
/// `LC7001` at the folder, with the nearest names, and its entry. Two files can give one name (`a.b.lcf` and `a/b.lcf`):
/// both are asked for. A name given twice counts once.
pub fn asked<'a>(names: &[String], functions: &'a [Function], folder: &str, report: &mut Report) -> (Vec<&'a Function>, Vec<Entry>) {
    if names.is_empty() {
        return (functions.iter().collect(), Vec::new());
    }
    let mut asked: Vec<&Function> = Vec::new();
    let mut unknown_entries = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for name in names {
        if !seen.insert(name.as_str()) {
            continue;
        }
        let before = asked.len();
        asked.extend(functions.iter().filter(|function| function.name == *name));
        if asked.len() == before {
            report.push(unknown(name, functions, folder));
            unknown_entries.push(Entry { function: name.clone(), drawn: false, codes: vec![Code::Lc7001], ..Entry::default() });
        }
    }
    asked.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.shown.cmp(&b.shown)));
    (asked, unknown_entries)
}

/// `LC7001`, at the folder, with the nearest names as fixes.
fn unknown(name: &str, functions: &[Function], folder: &str) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(Code::Lc7001, format!("unknown function `{name}`: not a function of this folder"), folder);
    let mut near: Vec<(usize, &str)> =
        functions.iter().filter_map(|other| near(name, &other.name).map(|edits| (edits, other.name.as_str()))).collect();
    near.sort();
    near.dedup();
    diagnostic.fixes = near.into_iter().take(3).map(|(_, other)| Fix { title: format!("did you mean `{other}`?"), edits: Vec::new() }).collect();
    diagnostic
}

/// The edits from one name to another, when they are near: at most a third of the longer one.
fn near(a: &str, b: &str) -> Option<usize> {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len() > 200 || b.len() > 200 {
        return None;
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut row = vec![i + 1];
        for (j, y) in b.iter().enumerate() {
            let replace = previous.get(j).copied().unwrap_or(0) + usize::from(x != y);
            let insert = row.get(j).copied().unwrap_or(0) + 1;
            let delete = previous.get(j + 1).copied().unwrap_or(0) + 1;
            row.push(replace.min(insert).min(delete));
        }
        previous = row;
    }
    let edits = previous.last().copied().unwrap_or(0);
    (edits * 3 <= a.len().max(b.len())).then_some(edits)
}

/// The drawing of each function asked for, in the view state of an exported image (`docs/cli.md`, 7.1).
pub fn drawings(asked: &[&Function], loaded: &Loaded, limits: &DiagramLimits, report: &mut Report) -> Vec<Drawn3> {
    let files = files_of(loaded.checked.as_ref());
    asked.iter().map(|function| drawing_of(function, loaded.checked.as_ref(), &files, &loaded.names, &ViewState::default(), limits, report)).collect()
}

/// The `trace` part of `labcraft-diagram/1` (`docs/cli.md`, 7.5).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TracePart {
    pub file: String,
    /// The events read.
    pub events: u64,
    /// That of its `end` event; absent when it has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// The run or a limit of reading cut it.
    pub cut: bool,
    /// The files that have changed or are gone since the run.
    pub stale: Vec<String>,
    /// The events counted by `LC7104`.
    pub ignored: u64,
}

/// `--max-trace-size` by default: twice the 256 MiB after which a run stops tracing, and its most (`docs/cli.md`, 7.4).
pub const DEFAULT_TRACE_SIZE: u64 = 512 << 20;
pub const MAX_TRACE_SIZE: u64 = 4 << 30;

/// What a trace is read in, a part at a time, so that reading never holds the whole file.
const TRACE_PART: usize = 64 * 1024;

/// Reads the trace at `path`, shown as `shown`, for the drawing of `function`, within `max_size` (`--max-trace-size`,
/// `docs/cli.md`, 7.4): the values to draw over it, and the `trace` part of the report; the diagnostics of reading it
/// (`LC71`) join `report`. A function that is not drawn has no values, but its trace is still read and judged. Whether it
/// matches is judged on the files of the folder that the run read, and always on the file of the function (notation
/// 6.1), each named from `FOLDER` (cli.md 7.2): the trace names them from where the run started ([`ours_in_trace`]).
/// `written`, the time of the trace that the viewer gives in its banner; none in an SVG.
#[allow(clippy::too_many_arguments)]
pub fn read_trace(
    ws: &dyn Workspace,
    path: &Path,
    shown: &str,
    drawing: Option<&Drawing>,
    function: &Function,
    loaded: &Loaded,
    max_size: u64,
    written: Option<String>,
    report: &mut Report,
) -> (Option<Overlay>, TracePart) {
    let shown = cut_text(shown).into_owned();
    let mut part = TracePart { file: shown.clone(), ..TracePart::default() };
    let limits = labcraft_trace::ReadLimits { file: max_size, ..labcraft_trace::ReadLimits::default() };
    let expected =
        drawing.map_or_else(|| labcraft_trace::Expected { function: function.name.clone(), ..Default::default() }, labcraft_diagram::expected);
    let mut reader = labcraft_trace::Reader::new(expected, limits);
    let opened = match ws.metadata(path) {
        Ok(metadata) if metadata.kind == crate::workspace::Kind::Folder => Err("it is a folder".to_owned()),
        Ok(_) => ws.open(path, 0).map_err(|error| walk::io_text(&error)),
        Err(error) => Err(walk::io_text(&error)),
    };
    let mut file = match opened {
        Ok(file) => file,
        Err(why) => {
            report.push(labcraft_diagram::values::unreadable(&shown, &why));
            return (None, part);
        }
    };
    let mut buffer = vec![0u8; TRACE_PART];
    loop {
        match std::io::Read::read(&mut file, &mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                if !reader.feed(buffer.get(..read).unwrap_or_default()) {
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => {
                report.push(labcraft_diagram::values::unreadable(&shown, &walk::io_text(&error)));
                return (None, part);
            }
        }
    }
    let trace = reader.finish();
    // Without a drawing, every event fits nothing: that says nothing of the trace.
    for diagnostic in labcraft_diagram::values::diagnostics(&trace, &shown) {
        if drawing.is_some() || diagnostic.code() != labcraft_types::Code::Lc7104 {
            report.push(diagnostic);
        }
    }
    let Ok(header) = &trace.header else { return (None, part) };
    if trace.other_function {
        return (None, part);
    }
    // The files of the folder that the run read, each once, and the file of the function, whatever the trace lists.
    let root = loaded.root.as_deref().unwrap_or(".");
    let from_folder = |file: &str| -> String {
        let prefix = format!("{root}/");
        if root == "." { file.to_owned() } else { file.strip_prefix(prefix.as_str()).unwrap_or(file).to_owned() }
    };
    let ours: HashMap<String, &str> = loaded.hashes.iter().map(|(file, sha)| (from_folder(file), sha.as_str())).collect();
    let listed: Vec<&str> = header.files.iter().map(|(file, _)| file.as_str()).collect();
    // Our deepest name, and that of the function, which is ours whatever the trace lists.
    let depth = ours.keys().chain(std::iter::once(&function.source)).map(|name| name.split('/').count()).max().unwrap_or(1);
    let mut stale: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for ((_, recorded), name) in header.files.iter().zip(ours_in_trace(&listed, &function.source, depth, |name| ours.contains_key(name))) {
        let Some(name) = name else { continue };
        let Some(sha) = ours.get(&name) else { continue };
        if seen.insert(name.clone()) && !recorded.eq_ignore_ascii_case(sha) {
            stale.push(cut_text(&name).into_owned());
        }
    }
    if !seen.contains(&function.source) && !stale.contains(&function.source) {
        stale.insert(0, cut_text(&function.source).into_owned());
    }
    part.events = trace.events;
    part.status = trace.end.as_deref().map(|status| cut_text(status).into_owned());
    part.cut = trace.cut();
    part.ignored = if drawing.is_some() { trace.ignored } else { 0 };
    part.stale.clone_from(&stale);
    let judged = labcraft_diagram::Judged { stale, written };
    (drawing.map(|drawing| labcraft_diagram::values(drawing, &trace, &judged, &labcraft_diagram::Choice::default())), part)
}

/// Which of the files that a trace lists (`listed`) are ours, by our name, from the folder: none for another one. The
/// trace names its files from where the run started, which the reader does not know: the name of the file of the
/// function (`source`) ends one of them at a `/`, which gives where the folder was; every other file of ours is that
/// place followed by its name, so that a file of a library with the name of one of ours is never taken for it. When the
/// name of the file of the function ends several of them, the place that gives the most of ours wins, then the shortest,
/// then the first. When it ends none, a file of ours is the longest end of a name, at a `/`. Only the ends of a name as
/// deep as our deepest name (`depth` names) are looked at, so the work follows the size of the list (scenario 4).
fn ours_in_trace(listed: &[&str], source: &str, depth: usize, is_ours: impl Fn(&str) -> bool) -> Vec<Option<String>> {
    // The ends of each name that are ours, from the shortest, with the place before them.
    let ends: Vec<Vec<(&str, &str)>> = listed.iter().map(|file| ends_of(file, depth).filter(|(_, end)| is_ours(end)).collect()).collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for (place, _) in ends.iter().flatten() {
        *counts.entry(place).or_default() += 1;
    }
    let best = ends
        .iter()
        .flatten()
        .filter(|(_, end)| *end == source)
        .map(|(place, _)| *place)
        .min_by_key(|place| (std::cmp::Reverse(counts.get(place).copied().unwrap_or(0)), place.len()));
    ends.iter()
        .map(|found| match best {
            Some(best) => found.iter().find(|(place, _)| *place == best).map(|(_, end)| (*end).to_owned()),
            None => found.last().map(|(_, end)| (*end).to_owned()),
        })
        .collect()
}

/// The ends of `file` at a `/`, from the shortest, each with what comes before it, at most `depth` of them: `a/b/c`
/// gives `("a/b/", "c")`, `("a/", "b/c")`, `("", "a/b/c")`.
fn ends_of(file: &str, depth: usize) -> impl Iterator<Item = (&str, &str)> {
    let mut at = Some(file.len());
    std::iter::from_fn(move || {
        let end = at?;
        let start = file.get(..end).and_then(|before| before.rfind('/'));
        at = start;
        // The first end is the whole name after the last `/`; the next ones take one more name each.
        let cut = match start {
            Some(slash) => slash + 1,
            None => 0,
        };
        Some((file.get(..cut).unwrap_or(""), file.get(cut..).unwrap_or("")))
    })
    .take(depth)
}

/// The values of a run over the one function asked for (`docs/cli.md`, 7.3), as `labcraft-cli diagram --trace` and
/// `diagram.svg` draw them; for an unknown function, only the `trace` part, which names the file.
#[allow(clippy::too_many_arguments)]
pub fn trace_over(
    ws: &dyn Workspace,
    path: &Path,
    shown: &str,
    asked: &[&Function],
    drawn: &[Drawn3],
    loaded: &Loaded,
    max_size: u64,
    report: &mut Report,
) -> (Option<Overlay>, TracePart) {
    match (asked.first(), drawn.first()) {
        (Some(function), Some((drawing, _, _))) => read_trace(ws, path, shown, drawing.as_ref(), function, loaded, max_size, None, report),
        _ => (None, TracePart { file: cut_text(shown).into_owned(), ..TracePart::default() }),
    }
}

/// What became of one function, as an entry of `diagrams` says it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub function: String,
    /// Absent for an unknown function.
    pub file: Option<String>,
    /// The path of the SVG written, by `labcraft-cli diagram`.
    pub svg: Option<String>,
    /// The SVG itself, by `diagram.svg`, which writes nothing.
    pub image: Option<String>,
    pub drawn: bool,
    pub nodes: Option<u64>,
    pub codes: Vec<Code>,
}

#[derive(Serialize)]
pub struct Document<'a> {
    pub schema: &'static str,
    pub status: Status,
    pub summary: Summary,
    pub diagrams: Vec<JsonEntry<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<&'a TracePart>,
    pub check: CheckPart<'a>,
}

#[derive(Serialize)]
pub struct Summary {
    pub functions: u64,
    pub drawn: u64,
    pub not_drawn: u64,
}

#[derive(Serialize)]
pub struct JsonEntry<'a> {
    pub function: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub svg: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<&'a str>,
    pub drawn: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodes: Option<u64>,
    #[serde(skip_serializing_if = "<[Code]>::is_empty")]
    pub codes: &'a [Code],
}

#[derive(Serialize)]
pub struct CheckPart<'a> {
    pub summary: CheckSummary,
    pub diagnostics: &'a [Diagnostic],
}

/// Whether the report says `error`: a function not drawn, an unknown name, nothing to draw, an SVG not written, or an
/// error that no drawing marks, in a file that is not that of a function drawn or about no file (`docs/cli.md`, 7.1).
/// `written`: whether each entry of a file must have its SVG, as `labcraft-cli diagram` writes them.
pub fn failed(entries: &[Entry], report: &Report, written: bool) -> bool {
    let not_drawn = entries.iter().any(|entry| !entry.drawn);
    let marked: HashSet<&str> = entries.iter().filter(|entry| entry.drawn).filter_map(|entry| entry.file.as_deref()).collect();
    let unmarked = report.file_counts().any(|(file, counts)| counts.errors > 0 && !marked.contains(file));
    let unwritten = written && entries.iter().any(|entry| entry.file.is_some() && entry.svg.is_none());
    entries.is_empty() || not_drawn || unwritten || unmarked
}

/// `labcraft-diagram/1` (`docs/cli.md`, 7.5), the entries in code point order of their names.
pub fn document<'a>(entries: &'a [Entry], report: &'a Report, counts: CheckCounts, failed: bool, trace: Option<&'a TracePart>) -> Document<'a> {
    let drawn = entries.iter().filter(|entry| entry.drawn).count() as u64;
    Document {
        schema: SCHEMA,
        status: if failed { Status::Error } else { Status::Ok },
        summary: Summary { functions: entries.len() as u64, drawn, not_drawn: entries.len() as u64 - drawn },
        diagrams: entries
            .iter()
            .map(|entry| JsonEntry {
                function: cut_text(&entry.function),
                file: entry.file.as_deref(),
                svg: entry.svg.as_deref(),
                image: entry.image.as_deref(),
                drawn: entry.drawn,
                nodes: entry.nodes,
                codes: &entry.codes,
            })
            .collect(),
        trace,
        check: CheckPart { summary: report.check_document(counts, false).summary, diagnostics: report.diagnostics() },
    }
}

/// `diagram.svg` (`docs/commands.md`, 6.5): `labcraft-cli diagram --svg --json` without the files, the SVG of each
/// function in `image`. When the result would pass the bound of a result, the SVGs give way first, from the last
/// function to the first, each as `{ "$cut": true }` (decision of the owner, 2026-11-11), and the result says
/// `truncated`; what still passes is cut by `Session::execute`, as for any command.
pub fn svg_command(session: &Session, params: &Params) -> Result<Value, CommandError> {
    let folder = params.text("folder")?;
    let path = match paths::judge(session.workspace(), session.root(), folder, "folder", Wanted::Folder, false)? {
        Place::Project { path, shown, exists } => {
            if !exists {
                return Err(CommandError::of("path.not_found", "folder", format!("`{shown}` does not exist")));
            }
            path
        }
        Place::Shipped(shown) => {
            return Err(CommandError::of(
                "path.outside_project",
                "folder",
                format!("`{shown}` is shipped with LabCraft: draw a folder of the project"),
            ));
        }
    };
    let names: Vec<String> = match params.get("functions") {
        Some(Value::Array(names)) => names.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        _ => Vec::new(),
    };
    // A name of the language, or that of a folder or a file that `check` reads: only an empty one is no name, as for the
    // `FUNCTION` of `labcraft-cli diagram` (`docs/cli.md`, 7).
    if let Some(empty) = names.iter().find(|name| name.split('.').any(str::is_empty)) {
        return Err(CommandError::of("params.invalid", "functions", format!("`{empty}` is not the name of a function: names joined by `.`")));
    }
    let theme = match params.str("theme") {
        Some("light") => Theme::Light,
        Some("dark") => Theme::Dark,
        _ => Theme::Both,
    };
    let max_nodes = params.u64("max_diagram_nodes").unwrap_or(DiagramLimits::default().max_nodes);
    // The values of a run, from a trace of the project, on one function (`docs/cli.md`, 7.3).
    let trace = match params.str("trace") {
        Some(given) => {
            if names.len() != 1 {
                return Err(CommandError::of(
                    "params.invalid",
                    "trace",
                    "`trace` draws the values of a run on one function: give exactly one in `functions`",
                ));
            }
            match paths::judge(session.workspace(), session.root(), given, "trace", Wanted::Trace, false)? {
                Place::Project { path, shown, .. } => Some((path, shown)),
                Place::Shipped(shown) => {
                    return Err(CommandError::of("path.outside_project", "trace", format!("`{shown}` is not a trace of the project")));
                }
            }
        }
        None => None,
    };
    let max_trace_size = params.u64("max_trace_size").unwrap_or(DEFAULT_TRACE_SIZE);
    let config = session.config();
    let mut loaded = load(session.workspace(), &config.shipped, &path, &config.lib_paths, &config.limits, Some(session.root()));
    let mut report = std::mem::replace(&mut loaded.report, Report::new(0));
    let Some(root) = loaded.root.as_deref() else {
        report.finish();
        return to_json(&document(&[], &report, CheckCounts::default(), true, None));
    };
    let (asked, mut entries) = asked(&names, &loaded.functions, root, &mut report);
    let drawn = drawings(&asked, &loaded, &DiagramLimits { max_nodes }, &mut report);
    let (overlay, part) = match &trace {
        Some((path, shown)) => {
            let (overlay, part) = trace_over(session.workspace(), path, shown, &asked, &drawn, &loaded, max_trace_size, &mut report);
            (overlay, Some(part))
        }
        None => (None, None),
    };
    // The drawings wait, by the index of their entry, until the room left for their SVG is known.
    let mut pending: Vec<Option<(&Function, Option<Drawing>)>> = std::iter::repeat_with(|| None).take(entries.len()).collect();
    for (function, (drawing, nodes, codes)) in asked.iter().zip(drawn) {
        let is_drawn = drawing.is_some();
        let codes = if is_drawn { Vec::new() } else { codes };
        entries.push(Entry {
            function: function.name.clone(),
            file: Some(function.shown.clone()),
            drawn: is_drawn,
            nodes,
            codes,
            ..Entry::default()
        });
        pending.push(Some((function, drawing)));
    }
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by(|&a, &b| entries.get(a).map(|a| (&a.function, &a.file)).cmp(&entries.get(b).map(|b| (&b.function, &b.file))));
    let mut sorted = Vec::with_capacity(entries.len());
    let mut waiting = Vec::with_capacity(entries.len());
    for index in order {
        if let Some(entry) = entries.get_mut(index) {
            sorted.push(std::mem::take(entry));
        }
        waiting.push(pending.get_mut(index).and_then(Option::take));
    }
    report.finish();
    let failed = failed(&sorted, &report, false);
    let mut result = to_json(&document(&sorted, &report, loaded.counts, failed, part.as_ref()))?;
    fill_images(&mut result, crate::bound::MAX_RESULT, |index| {
        waiting
            .get_mut(index)
            .and_then(Option::take)
            .map(|(function, drawing)| svg_with(&picture(function, drawing, &report), overlay.as_ref(), theme))
    });
    Ok(result)
}

fn to_json(document: &Document<'_>) -> Result<Value, CommandError> {
    serde_json::to_value(document).map_err(|error| CommandError::internal(format!("the report does not write as JSON: {error}")))
}

/// What `"truncated":true` costs at the top level, with its comma.
const TRUNCATED: usize = ",\"truncated\":true".len();

/// Gives each entry of a file its SVG in `image`, in order, while the result stays within `max`: the SVGs give way from
/// the last function to the first (decision of the owner, 2026-11-11), and those that give way are not even made.
/// `render` makes the SVG of the entry at this index. The marks of a cut and `truncated` are counted before any SVG, so
/// that they never take the result past `max`, and `bound::fit` has no image left to cut.
fn fill_images(result: &mut Value, max: usize, mut render: impl FnMut(usize) -> Option<String>) {
    let cut = json!({ "$cut": true });
    let Some(diagrams) = result.get_mut("diagrams").and_then(Value::as_array_mut) else { return };
    // Every entry of a file starts with its mark: what an SVG costs is then what it adds to it.
    let mut wanted = Vec::with_capacity(diagrams.len());
    for entry in diagrams.iter_mut() {
        let has_file = entry.get("file").is_some();
        if let (true, Some(object)) = (has_file, entry.as_object_mut()) {
            object.insert("image".to_owned(), cut.clone());
        }
        wanted.push(has_file);
    }
    let mark = crate::bound::size(&cut);
    let mut room = max.saturating_sub(crate::bound::size(result));
    let last = wanted.iter().rposition(|wanted| *wanted);
    let mut truncated = false;
    let Some(diagrams) = result.get_mut("diagrams").and_then(Value::as_array_mut) else { return };
    for (index, (entry, wanted)) in diagrams.iter_mut().zip(wanted).enumerate() {
        if !wanted {
            continue;
        }
        let Some(image) = render(index) else { continue };
        let cost = serde_json::to_string(&image).map_or(usize::MAX, |text| text.len()).saturating_sub(mark);
        // While an SVG may still give way after this one, the room of `truncated` stays.
        let reserve = if Some(index) == last { 0 } else { TRUNCATED };
        if cost.saturating_add(reserve) > room {
            truncated = true;
            break;
        }
        room -= cost;
        if let Some(object) = entry.as_object_mut() {
            object.insert("image".to_owned(), Value::String(image));
        }
    }
    if let (true, Some(object)) = (truncated, result.as_object_mut()) {
        object.insert("truncated".to_owned(), Value::Bool(true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_names_are_a_third_of_their_length_apart_at_most() {
        assert_eq!(near("check_voltag", "check_voltage"), Some(1));
        assert_eq!(near("abc", "xyz"), None);
        assert_eq!(near("measure", "measure"), Some(0));
    }

    #[test]
    fn a_file_of_a_library_with_the_name_of_one_of_ours_is_not_ours() {
        let ours = |name: &str| ["f.lcf", "g.lcf", "sub/h.lcf"].contains(&name);
        let some = |name: &str| Some(name.to_owned());
        // Run from above the folder: the file of the function gives where the folder was, `work/`.
        let listed = ["libs/visa/g.lcf", "work/f.lcf", "work/g.lcf", "work/sub/h.lcf", "<labcraft>/visa/open.lcf"];
        assert_eq!(ours_in_trace(&listed, "f.lcf", 2, ours), [None, some("f.lcf"), some("g.lcf"), some("sub/h.lcf"), None]);
        // Run from the folder itself.
        assert_eq!(ours_in_trace(&["f.lcf", "../libs/g.lcf"], "f.lcf", 2, ours), [some("f.lcf"), None]);
        // Two places end with the file of the function: the one that gives the most of ours, then the shortest.
        assert_eq!(ours_in_trace(&["a/f.lcf", "b/f.lcf", "b/g.lcf"], "f.lcf", 2, ours), [None, some("f.lcf"), some("g.lcf")]);
        assert_eq!(ours_in_trace(&["ab/f.lcf", "b/f.lcf"], "f.lcf", 2, ours), [None, some("f.lcf")]);
        // `xf.lcf` does not end with `f.lcf` at a `/`: without the file of the function, the longest end of each name.
        assert_eq!(ours_in_trace(&["xf.lcf", "w/g.lcf"], "f.lcf", 2, ours), [None, some("g.lcf")]);
    }

    /// As many files as a first line of 16 MiB lists, each a place of the file of the function: one pass over the list,
    /// not one per place (scenario 4).
    #[test]
    fn a_trace_that_lists_many_places_is_matched_in_one_pass() {
        let names: Vec<String> = (0..450_000).map(|at| format!("d{at}/f.lcf")).collect();
        let mut listed: Vec<&str> = names.iter().map(String::as_str).collect();
        listed.push("d7/g.lcf");
        let found = ours_in_trace(&listed, "f.lcf", 1, |name| name == "f.lcf" || name == "g.lcf");
        // `d7/` gives two of ours, every other place one.
        let at: Vec<usize> = found.iter().enumerate().filter_map(|(at, name)| name.as_ref().map(|_| at)).collect();
        assert_eq!(at, [7, 450_000]);
    }

    #[test]
    fn the_images_give_way_from_the_last_and_the_result_fits_whole() {
        // Three entries of a file and one unknown; each SVG is 2,000 characters, over the 1,000 at which `bound::fit`
        // cuts a string, and costs 2,002 bytes as JSON less the 13 of `{"$cut":true}`: 1,989 more than its mark.
        let entries = json!([{ "function": "a", "file": "a.lcf" }, { "function": "b", "file": "b.lcf" }, { "function": "c" }, { "function": "d", "file": "d.lcf" }]);
        let result = json!({ "schema": SCHEMA, "diagrams": entries });
        let image = "x".repeat(2000);
        let fill = |max: usize| {
            let mut made = Vec::new();
            let mut filled = result.clone();
            fill_images(&mut filled, max, |index| {
                made.push(index);
                Some(image.clone())
            });
            (filled, made)
        };
        let mut marked = result.clone();
        for index in [0, 1, 3] {
            marked["diagrams"][index]["image"] = json!({ "$cut": true });
        }
        let base = crate::bound::size(&marked);
        // Room for all three: whole, no `truncated`, exactly at the bound.
        let (whole, made) = fill(base + 3 * 1989);
        assert_eq!((made, whole.get("truncated")), (vec![0, 1, 3], None));
        assert_eq!(crate::bound::size(&whole), base + 3 * 1989);
        assert_eq!(whole["diagrams"][2].get("image"), None);
        // One byte less: the last gives way, its SVG is not made, and `truncated` fits in the room kept for it.
        for max in [base + 3 * 1989 - 1, base + 2 * 1989 + TRUNCATED] {
            let (cut, made) = fill(max);
            assert_eq!(made, vec![0, 1, 3], "{max}");
            assert_eq!((cut["diagrams"][0]["image"].as_str(), cut["diagrams"][1]["image"].as_str()), (Some(image.as_str()), Some(image.as_str())));
            assert_eq!((&cut["diagrams"][3]["image"], &cut["truncated"]), (&json!({ "$cut": true }), &json!(true)));
            assert!(crate::bound::size(&cut) <= max, "{max}");
            // Within the bound, `bound::fit` leaves it as it is: no image is cut to a `$head`.
            assert_eq!(crate::bound::fit(cut.clone(), max), cut);
        }
        // Room for one and the mark of `truncated`: the second gives way, and the third SVG is never made.
        let (one, made) = fill(base + 1989 + TRUNCATED);
        assert_eq!(made, vec![0, 1]);
        assert_eq!((&one["diagrams"][1]["image"], &one["diagrams"][3]["image"]), (&json!({ "$cut": true }), &json!({ "$cut": true })));
        assert!(crate::bound::size(&one) <= base + 1989 + TRUNCATED);
    }
}
