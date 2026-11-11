//! One run, from what `prepare` judged to its report (`docs/commands.md`, sections 7 and 8): the trace written as it goes,
//! to `.labcraft/runs/<function>.trace.jsonl.<process>-<session>-<run>.partial` (or beside the file of `--trace FILE`),
//! then renamed over the last trace of the function; inline, in the report; or nowhere. A trace that cannot be written
//! never stops the run: the report says why.
//!
//! **Whether a process still runs** is told by a lock (decision of the owner, 2026-11-11): a process that writes a
//! `.partial` holds the lock of `.<process>.lock` in its folder until it ends, and the system lets it go however it ends.
//! A run removes the `.partial` files of its function that a process left whose lock nobody holds, or that has none.

use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use labcraft_runtime::Env;
use labcraft_runtime::report::{Part, RESULT_MAX, ReportOf, Whole, report, running};
use labcraft_runtime::run::{AbortBy, Control, Native, Progress, Run, RunEnd, Unobserved, run};
use labcraft_runtime::trace::TraceObserver;
use labcraft_trace::{Header, WriteLimits};
use serde_json::{Value as Json, json};

use super::Tag;
use super::parts::InlineTrace;
use super::prepare::{Prepared, TraceMode};
use crate::walk::io_text;
use crate::workspace::{Held, Kind, Workspace};

/// The events of an inline trace *(proposed)*: past them, `truncated`.
pub(crate) const INLINE_EVENTS: u64 = 2000;

/// The text of an output, at most, in the human form of the command line, and of all of them.
const TEXT_MAX: usize = 16 << 10;
const TEXTS_MAX: usize = 1 << 20;

/// The folder of the traces, under the root (commands.md 8).
pub(crate) const RUNS_FOLDER: &str = ".labcraft/runs";

/// What the app gives a run to read real time: monotonic, in nanoseconds from any start. The runtime never reads a clock
/// itself (`docs/architecture.md`, 10.4), nor does `engine`, which builds for WASM.
pub trait Clock: Send + Sync {
    fn real_ns(&self) -> u64;
}

/// A clock that never moves: tests and the corpus of runs, whose reports then say `real_ms: 0`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Still;

impl Clock for Still {
    fn real_ns(&self) -> u64 {
        0
    }
}

/// What a run tells the session while it goes on, and what reaches it from outside: `run.abort`, and `run.status` asking
/// where it is.
pub(crate) trait Steer: Sync {
    /// An abort asked since the last time it was asked.
    fn abort(&self) -> Option<AbortBy>;
    /// Whether someone waits for where the run is.
    fn wants_progress(&self) -> bool;
    /// The `running` report, written by the run.
    fn progress(&self, report: Json);
    /// Whether the grace period after an abort was ended from outside (cli.md 6.5).
    fn grace_over(&self) -> bool {
        false
    }
}

/// Nobody steers: a run to its end on the caller's thread.
pub(crate) struct Alone;

impl Steer for Alone {
    fn abort(&self) -> Option<AbortBy> {
        None
    }
    fn wants_progress(&self) -> bool {
        false
    }
    fn progress(&self, _: Json) {}
}

/// What a run gives back: its report, and where its trace is, when it was written to a file.
pub(crate) struct Finished {
    pub report: Json,
    /// The text of each output as the language writes a constant, cut as a report cuts it, for the human form of the
    /// command line (cli.md 6.7); `None` for one that has no text.
    pub texts: Vec<Option<String>>,
    /// The trace file under the root, from the root, once it has been renamed in place.
    pub trace_file: Option<String>,
}

struct Steering<'a> {
    clock: &'a dyn Clock,
    start: u64,
    steer: &'a dyn Steer,
    of: ReportOf<'a>,
}

impl Control for Steering<'_> {
    fn real_ns(&mut self) -> u64 {
        self.clock.real_ns().saturating_sub(self.start)
    }

    fn abort(&mut self) -> Option<AbortBy> {
        self.steer.abort()
    }

    fn wants_progress(&mut self) -> bool {
        self.steer.wants_progress()
    }

    fn grace_over(&mut self) -> bool {
        self.steer.grace_over()
    }

    fn progress(&mut self, progress: Progress) {
        let real_ms = self.real_ns() / 1_000_000;
        let of = ReportOf { real_ms, ..copy_of(&self.of) };
        let text = running(&progress, &of);
        self.steer.progress(serde_json::from_str(&text).unwrap_or(Json::Null));
    }
}

fn copy_of<'a>(of: &ReportOf<'a>) -> ReportOf<'a> {
    ReportOf {
        run: of.run,
        program: of.program,
        names: of.names,
        files: of.files,
        natives: of.natives,
        real_ms: of.real_ms,
        check: of.check,
        trace: of.trace,
        limit: of.limit,
    }
}

/// Where a trace file is written, and what went wrong with it.
struct TraceFile {
    /// `.labcraft/runs/check_voltage.trace.jsonl`, from the root.
    shown: String,
    last: PathBuf,
    partial: PathBuf,
    error: Option<String>,
}

/// The file of the last trace of a function run from a folder (commands.md 8; decision of the owner, 2026-11-10):
/// `.labcraft/runs/<function>.trace.jsonl` for the root, `.labcraft/runs/<folder>/<function>.trace.jsonl` otherwise, so
/// that two folders with a function of one name never share a trace.
pub(crate) fn trace_path(folder: &str, function: &str) -> String {
    if folder == "." || folder.is_empty() {
        format!("{RUNS_FOLDER}/{function}.trace.jsonl")
    } else {
        format!("{RUNS_FOLDER}/{folder}/{function}.trace.jsonl")
    }
}

/// The `.partial` of a run, from the root: unique to the process, the session and the run (commands.md 8).
pub(crate) fn partial_of(trace_file: &str, tag: Tag, id: &str) -> String {
    format!("{trace_file}.{}-{}-{id}.partial", tag.process, tag.session)
}

/// The locks this process holds, one in each folder where it writes a `.partial`, with the workspace they were taken in:
/// held until [`let_go`], or until the process ends.
static HELD: Mutex<Vec<Lock>> = Mutex::new(Vec::new());

/// A lock held: the workspace it was taken in, kept as long as the lock, its file, and the guard that holds it.
type Lock = (Arc<dyn Workspace>, PathBuf, Held);

/// The lock of a process, in a folder of `.partial` files.
fn lock_of(folder: &Path, process: u32) -> PathBuf {
    folder.join(format!(".{process}.lock"))
}

/// Holds the lock of this process in a folder, once for the process; whether the workspace has locks.
fn hold(ws: &Arc<dyn Workspace>, folder: &Path, process: u32) -> bool {
    let path = lock_of(folder, process);
    let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    if held.iter().any(|(owner, at, _)| *at == path && std::ptr::addr_eq(Arc::as_ptr(owner), Arc::as_ptr(ws))) {
        return true;
    }
    match ws.try_lock(&path) {
        Ok(Some(guard)) => {
            held.push((Arc::clone(ws), path, guard));
            true
        }
        // Held by this process through another workspace, or by another process: then this one has no lock there.
        Ok(None) => held.iter().any(|(_, at, _)| *at == path),
        Err(_) => false,
    }
}

/// Lets go of the locks of this process, and removes their files, which no `.partial` of it needs any more: the command
/// line calls it as it ends. A process that ends otherwise leaves its lock files, which the next run removes.
pub fn let_go() {
    let held = std::mem::take(&mut *HELD.lock().unwrap_or_else(PoisonError::into_inner));
    for (owner, path, guard) in held {
        // Let go first: Windows does not remove a file that a lock holds.
        drop(guard);
        let _ = owner.remove_file(&path);
    }
}

/// The process and the session of a `.partial` of `name`: `<name>.<process>-<session>-<run>.partial`.
fn partial_owner(file: &str) -> Option<(&str, u32, u64)> {
    let (base, id) = file.strip_suffix(".partial")?.rsplit_once('.')?;
    let mut parts = id.splitn(3, '-');
    let process = parts.next()?.parse().ok()?;
    let session = parts.next()?.parse().ok()?;
    parts.next()?;
    Some((base, process, session))
}

/// Removes the `.partial` files of `name` in `folder` that this session left, or that a process that no longer runs
/// left; with them, the lock of such a process once no `.partial` of it is left there.
fn sweep(ws: &dyn Workspace, folder: &Path, name: &str, tag: Tag, locks: bool) {
    let mut found: Vec<(String, u32, u64)> = Vec::new();
    let mut elsewhere: HashSet<u32> = HashSet::new();
    let mut locks_found: Vec<u32> = Vec::new();
    let _ = ws.read_dir(folder, &mut |entry| {
        if let Ok(entry) = entry
            && matches!(entry.kind, Ok(Kind::File))
            && let Some(file) = entry.name.to_str()
        {
            if let Some((base, process, session)) = partial_owner(file) {
                if base == name {
                    found.push((file.to_owned(), process, session));
                } else {
                    elsewhere.insert(process);
                }
            } else if let Some(process) = file.strip_prefix('.').and_then(|rest| rest.strip_suffix(".lock")).and_then(|pid| pid.parse().ok()) {
                locks_found.push(process);
            }
        }
    });
    let with_partials: HashSet<u32> = found.iter().map(|(_, process, _)| *process).chain(elsewhere.iter().copied()).collect();
    // A process is gone when its lock is missing or free; the lock taken to know it is held until its files are removed.
    let mut gone: HashMap<u32, Option<Option<Held>>> = HashMap::new();
    for (file, process, session) in found {
        let remove = if process == tag.process {
            session == tag.session
        } else {
            locks
                && gone
                    .entry(process)
                    .or_insert_with(|| {
                        let lock = lock_of(folder, process);
                        if ws.link_metadata(&lock).is_err() {
                            return Some(None);
                        }
                        match ws.try_lock(&lock) {
                            Ok(Some(guard)) => Some(Some(guard)),
                            _ => None,
                        }
                    })
                    .is_some()
        };
        if remove {
            let _ = ws.remove_file(&folder.join(file));
        }
    }
    // A lock with no `.partial` left beside it, of a process that no longer runs: one that ended without letting go.
    if locks {
        for process in locks_found {
            if process != tag.process
                && !with_partials.contains(&process)
                && !gone.contains_key(&process)
                && let Ok(Some(guard)) = ws.try_lock(&lock_of(folder, process))
            {
                drop(guard);
                let _ = ws.remove_file(&lock_of(folder, process));
            }
        }
    }
    for (process, verdict) in gone {
        if let Some(Some(guard)) = verdict {
            // Let go first: Windows does not remove a file that a lock holds.
            drop(guard);
            if !elsewhere.contains(&process) {
                let _ = ws.remove_file(&lock_of(folder, process));
            }
        }
    }
}

/// Whether a path from the root, once its links are followed, is still where its names say: a link or a junction never
/// leads a trace out of the project (commands.md 2).
pub(crate) fn in_place(ws: &dyn Workspace, root: &Path, shown: &str) -> bool {
    let mut expected = root.to_path_buf();
    expected.extend(shown.split('/'));
    ws.canonicalize(&expected).is_ok_and(|real| real == expected)
}

/// Makes the folder of the trace (and `.labcraft/.gitignore` holding `*` when it makes `.labcraft`), refuses one that a link
/// leads elsewhere, holds the lock of the process there, and removes the `.partial` files of the function that this
/// session left or that a process that no longer runs left (decisions of the owner, 2026-11-10 and 2026-11-11).
fn trace_file(ws: &Arc<dyn Workspace>, root: &Path, shown: &str, display: &str, tag: Tag, id: &str) -> TraceFile {
    let mut last = root.to_path_buf();
    last.extend(shown.split('/'));
    let partial_shown = partial_of(shown, tag, id);
    let mut partial = root.to_path_buf();
    partial.extend(partial_shown.split('/'));
    let mut file = TraceFile { shown: display.to_owned(), last: last.clone(), partial, error: None };
    let ws_arc = ws;
    let ws: &dyn Workspace = &**ws;
    let Some(folder) = last.parent().map(Path::to_path_buf) else {
        file.error = Some(format!("`{shown}` has no folder"));
        return file;
    };
    let folder_shown = shown.rsplit_once('/').map_or("", |(folder, _)| folder);
    let labcraft = root.join(".labcraft");
    let new = ws.link_metadata(&labcraft).is_err();
    // One level at a time, each that exists checked before anything is made in it: a link never leads a folder out.
    let mut level = root.to_path_buf();
    let mut level_shown = String::new();
    for name in folder_shown.split('/') {
        level.push(name);
        level_shown = if level_shown.is_empty() { name.to_owned() } else { format!("{level_shown}/{name}") };
        if ws.link_metadata(&level).is_ok() {
            if !in_place(ws, root, &level_shown) {
                file.error = Some(format!("`{level_shown}` leads out of its place through a link, and a trace is written only in the project"));
                return file;
            }
        } else if let Err(error) = ws.create_folders(&level) {
            file.error = Some(format!("`{level_shown}` cannot be made: {}", io_text(&error)));
            return file;
        }
    }
    if !in_place(ws, root, folder_shown) {
        file.error = Some(format!("`{folder_shown}` leads out of its place through a link, and a trace is written only in the project"));
        return file;
    }
    if new && let Err(error) = ws.write(&labcraft.join(".gitignore"), b"*\n") {
        file.error = Some(format!("`.labcraft/.gitignore` cannot be written: {}", io_text(&error)));
    }
    let name = shown.rsplit('/').next().unwrap_or(shown);
    let locks = hold(ws_arc, &folder, tag.process);
    sweep(ws, &folder, name, tag, locks);
    file
}

/// The trace of `--trace FILE`: its `.partial` beside it, the lock of the process there too. Its folder is not made.
fn trace_file_at(ws: &Arc<dyn Workspace>, at: &super::prepare::Outside, tag: Tag, id: &str) -> TraceFile {
    let last = at.path.clone();
    let name = last.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let folder = last.parent().map(Path::to_path_buf).unwrap_or_default();
    let partial = folder.join(format!("{name}.{}-{}-{id}.partial", tag.process, tag.session));
    let mut file = TraceFile { shown: at.shown.clone(), last, partial, error: None };
    if ws.metadata(&folder).map(|metadata| metadata.kind).ok() != Some(Kind::Folder) {
        file.error = Some(format!("`{}` cannot be written: its folder does not exist", at.shown));
        return file;
    }
    let locks = hold(ws, &folder, tag.process);
    sweep(&**ws, &folder, &name, tag, locks);
    file
}

/// Runs what `prepare` judged, to its end.
pub(crate) fn execute(prepared: Prepared, ws: &Arc<dyn Workspace>, root: &Path, clock: &dyn Clock, steer: &dyn Steer, tag: Tag) -> Finished {
    let start = clock.real_ns();
    let Prepared {
        id,
        function,
        checked,
        names,
        program,
        inputs,
        settings,
        natives,
        check,
        files,
        trace,
        trace_file: under,
        trace_shown,
        trace_to,
        texts: want_texts,
    } = prepared;
    let env = Env { program: &program, names: &names };
    let natives: Vec<Native<'_>> =
        natives.iter().map(|native| Native { library: native.library, native: &*native.native, simulation: native.simulation.as_ref() }).collect();
    let header_files: Vec<(&str, &str)> = files.iter().map(|(file, sha)| (file.as_str(), sha.as_str())).collect();
    let shown = program.functions.first().map_or(function.as_str(), |top| &*top.shown);
    let header = Header { run: &id, function: shown, mode: "deterministic", files: &header_files };
    let of = ReportOf {
        run: &id,
        program: &program,
        names: &names,
        files: &checked.files,
        natives: &natives,
        real_ms: 0,
        check: &check,
        trace: None,
        limit: RESULT_MAX,
    };
    // What the run says before its first node, so that `run.status` always has an answer.
    steer.progress(serde_json::from_str(&running(&Progress::default(), &of)).unwrap_or(Json::Null));
    let mut control = Steering { clock, start, steer, of: copy_of(&of) };
    let given = Run { program: &program, names: &names, files: &checked.files, natives: &natives, inputs, settings };

    let (end, trace_json, trace_file): (RunEnd, Option<Box<dyn Part + '_>>, Option<String>) = match trace {
        TraceMode::None => (run(given, &mut control, &mut Unobserved), None, None),
        TraceMode::Inline => {
            let limits = WriteLimits { events: INLINE_EVENTS, file: (RESULT_MAX as u64) * 2, ..WriteLimits::default() };
            let mut observer = TraceObserver::new(Vec::new(), &header, env, &checked.files, limits);
            let end = run(given, &mut control, &mut observer);
            let traced = observer.finish(&end);
            let text = String::from_utf8_lossy(&traced.sink).into_owned();
            (end, Some(Box::new(InlineTrace::of_lines(&text, traced.truncated))), None)
        }
        TraceMode::File => {
            let mut file = match &trace_to {
                Some(at) => trace_file_at(ws, at, tag, &id),
                None => trace_file(ws, root, &under, &trace_shown, tag, &id),
            };
            let sink: Box<dyn Write + Send> = if file.error.is_none() {
                match ws.create(&file.partial) {
                    Ok(sink) => sink,
                    Err(error) => {
                        file.error = Some(format!("`{}` cannot be written: {}", file.shown, io_text(&error)));
                        Box::new(io::sink())
                    }
                }
            } else {
                Box::new(io::sink())
            };
            let mut observer = TraceObserver::new(io::BufWriter::with_capacity(64 << 10, sink), &header, env, &checked.files, WriteLimits::default());
            let end = run(given, &mut control, &mut observer);
            let traced = observer.finish(&end);
            if file.error.is_none()
                && let Some(error) = traced.error
            {
                file.error = Some(format!("the trace stopped: {}", io_text(&error)));
            }
            // The sink is closed before the rename, which Windows asks.
            match traced.sink.into_inner() {
                Ok(mut sink) => {
                    let _ = sink.flush();
                }
                Err(error) if file.error.is_none() => file.error = Some(format!("the trace stopped: {}", io_text(error.error()))),
                Err(_) => {}
            }
            let mut renamed = None;
            if file.error.is_none() {
                match ws.rename(&file.partial, &file.last) {
                    // The traces under the root are the last of their function, which `run.trace` reads.
                    Ok(()) => renamed = trace_to.is_none().then(|| under.clone()),
                    Err(error) => {
                        file.error = Some(format!("the trace stays as `.partial`, since it could not replace the last one: {}", io_text(&error)))
                    }
                }
            }
            let mut part = json!({ "file": file.shown, "events": traced.events, "truncated": traced.truncated });
            if let (Some(error), Some(object)) = (&file.error, part.as_object_mut()) {
                object.insert("error".to_owned(), json!({ "code": "file.unwritable", "message": labcraft_types::cut_text(error) }));
            }
            (end, Some(Box::new(Owned(part.to_string()))), renamed)
        }
    };
    let real_ms = clock.real_ns().saturating_sub(start) / 1_000_000;
    // Only the human form asks for them, within a budget for all: past it, an output shows its JSON form.
    let mut texts = Vec::new();
    if want_texts && let Some(top) = program.functions.first() {
        let mut left = TEXTS_MAX;
        for (value, port) in end.outputs.iter().zip(&top.outputs) {
            let text = (left > 0).then(|| labcraft_runtime::constant_text(env, top.library, value, port.ty, TEXT_MAX.min(left))).flatten();
            left = left.saturating_sub(text.as_ref().map_or(0, String::len));
            texts.push(text);
        }
    }
    let of = ReportOf { real_ms, trace: trace_json.as_deref(), ..of };
    let text = report(&end, &of);
    let report = serde_json::from_str(&text).unwrap_or_else(|error| {
        json!({ "schema": "labcraft-run/1", "status": "fault", "run": id, "function": function, "fault": { "code": "LC8004", "message": format!("a bug of the runtime: the report does not read as JSON: {error}") } })
    });
    Finished { report, texts, trace_file }
}

/// A part held as text, never cut.
struct Owned(String);

impl Part for Owned {
    fn least(&self) -> String {
        self.0.clone()
    }

    fn within(&self, bytes: usize) -> (String, bool) {
        Whole(&self.0).within(bytes)
    }
}
