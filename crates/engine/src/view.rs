//! The state of the viewer (`docs/architecture.md`, 10.8 and 11.5; `docs/commands.md`, 6.5): the folder open, its
//! check, the function shown, the view state that the file does not hold, the preference of the standard view. The
//! `view.*` commands change it; the app reads what it publishes ([`Session::view`]), the drawings as `Arc<Drawing>`.
//!
//! It writes no file: the viewer is read-only (scenario 6, point 6). The files are read through the [`Workspace`] of
//! the session; the app watches them and calls [`Session::files_changed`], and a command of the session that writes
//! marks the view stale, so that the next `view.*` command reads the folder again (decisions of the owner, 2026-11-11).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use labcraft_diagram::style::{self, Style};
use labcraft_diagram::{Choice, DiagramLimits, Drawing, Judged, Overlay, ViewState};
use labcraft_trace::Trace;
use labcraft_types::{Code, Report};
use serde_json::{Map, Value, json};

use crate::drawings::{self, Function, Loaded, TracePart};
use crate::error::CommandError;
use crate::params::Params;
use crate::paths::{self, Place, Wanted};
use crate::session::Session;
use crate::walk::slashed;
use crate::workspace::Workspace;

/// The view state of a function that its file does not hold (notation 2.1), kept for the session only, by its file,
/// across changes of the file, by the keys of its nodes (decided at the start of step 6).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewOf {
    /// The arm shown of each structure in tabs, as the developer picked it: over the arm that the run took until an
    /// iteration selector moves or another trace arrives (notation 6.1).
    pub arms: HashMap<String, usize>,
    pub unfolded: BTreeSet<String>,
    pub standard: BTreeSet<String>,
    /// The iteration chosen of each loop, from 0, kept across runs and bounded by the last of each (decisions of the owner,
    /// 2026-11-13); a loop not here shows its last.
    pub iterations: HashMap<String, u64>,
    /// The trace whose values were shown last over this file: another one is another run, and the tabs picked give way.
    pub(crate) trace_seen: Option<Stamp>,
}

/// A function of the folder, as the tree of the window lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub function: String,
    pub file: String,
    pub drawable: bool,
    pub errors: u64,
    pub warnings: u64,
}

/// What the window shows, as the session last published it.
#[derive(Clone, Debug, Default)]
pub struct Published {
    /// Changes with each publication.
    pub generation: u64,
    /// The folder open, as the paths of the session show it (`.` for the root); none when none is open or it could not
    /// be read.
    pub folder: Option<String>,
    pub functions: Vec<Listed>,
    /// The report of the check of the folder, with the diagnostics of the drawing shown (`LC7005`); a new one with each
    /// check and each drawing.
    pub report: Option<Arc<Report>>,
    pub errors: u64,
    pub warnings: u64,
    /// The function shown, and the tabs of the functions opened.
    pub function: Option<String>,
    pub opened: Vec<String>,
    pub drawing: Option<Arc<Drawing>>,
    /// Changes with each drawing.
    pub drawing_generation: u64,
    /// Whether the nodes glide to this drawing: the file of the function shown changed.
    pub glide: bool,
    /// The function is not drawable now: its last drawing stays, with its diagnostics (scenario 6, point 3).
    pub kept: bool,
    /// The arm shown of each structure in tabs: the one the developer picked, else the one the run took (notation 6.1).
    pub arms: HashMap<String, usize>,
    pub standard_everywhere: bool,
    /// The trace whose values show and the iterations chosen, from which the canvas works out the values of a wire over
    /// the iterations of its loop (notation 6.1); published with each overlay.
    pub run: Option<(Arc<Trace>, Choice)>,
    /// The values of the run over the drawing shown (notation 6.1): none until its trace is read, and none without one.
    pub overlay: Option<Arc<Overlay>>,
    /// Changes with each overlay.
    pub overlay_generation: u64,
    /// The trace read for the function shown, as the `trace` part of `labcraft-diagram/1` says it; none when it has none.
    pub trace: Option<TracePart>,
    /// The trace was opened by hand (`view.open_trace` with a `file`): it stays until another function is shown.
    pub trace_opened: bool,
}

/// The trace of the function shown (notation 6.1; decisions of the owner, 2026-11-13).
#[derive(Default)]
struct Traced {
    /// The trace opened by hand, and how it is shown; none for the last run of the function.
    opened: Option<(PathBuf, String)>,
    /// The trace read last, as its file then was; none when none was read.
    read: Option<Stamp>,
    /// What it gave for the drawing shown, and how it stands against the files: the values are worked out again from it
    /// when an iteration is chosen, without reading the file.
    kept: Option<(Arc<Trace>, Judged)>,
    /// The drawing is new: its values must be read again.
    due: bool,
    /// The report of the drawing, without the diagnostics of reading the trace.
    report: Option<Arc<Report>>,
}

/// A trace file as it is: where, its length and its time; none of the two when it does not exist.
type Stamp = (PathBuf, Option<(u64, Option<u128>)>);

/// The state of the viewer that the session holds.
#[derive(Default)]
pub struct View {
    /// The folder open, without links, and how it is shown.
    folder: Option<(PathBuf, String)>,
    loaded: Option<Loaded>,
    views: HashMap<String, ViewOf>,
    standard_everywhere: bool,
    function: Option<String>,
    /// The file of the function shown: two files can give one name.
    file: Option<String>,
    opened: Vec<String>,
    /// The last drawing of each file whose function was drawable: it holds the values it was drawn with.
    last: HashMap<String, Arc<Drawing>>,
    /// The entry of the function shown in `labcraft-diagram/1`.
    entry: Value,
    published: Published,
    /// The values of the notation in force (notation 10): those of `--style`, given by the app.
    style: Arc<Style>,
    /// A command wrote files since the folder was read.
    pub(crate) stale: bool,
    /// The functions never drawn, whose drawing stopped a worker of the web viewer before (architecture 11.5).
    pub(crate) skipped: BTreeSet<String>,
    /// The function that the next `view.open` shows instead of the first one, when the folder has it (architecture
    /// 11.5).
    pub(crate) open_on: Option<String>,
    /// Called with the name of each function just before it is drawn: the worker of the web viewer tells its page, so
    /// that a trap while drawing is put down to that function (architecture 11.5).
    pub(crate) before_drawing: Option<fn(&str)>,
    /// Whether the function shown was drawn now, and not kept from before: only then are values drawn over it.
    drawn: bool,
    traced: Traced,
}

impl View {
    pub fn published(&self) -> &Published {
        &self.published
    }

    fn publish(&mut self) {
        self.published.generation = self.published.generation.wrapping_add(1);
    }
}

/// `view.open`.
pub fn open(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let folder = params.text("folder")?;
    let (path, shown) = match paths::judge(&*session.ws, &session.root, folder, "folder", Wanted::Folder, false)? {
        Place::Project { path, shown, exists: true } => (path, shown),
        Place::Project { shown, .. } => return Err(CommandError::of("path.not_found", "folder", format!("`{shown}` does not exist"))),
        Place::Shipped(shown) => {
            return Err(CommandError::of(
                "path.outside_project",
                "folder",
                format!("`{shown}` is shipped with LabCraft: open a folder of the project"),
            ));
        }
    };
    let view = &mut session.view;
    view.folder = Some((path, shown));
    view.views.clear();
    view.last.clear();
    view.function = None;
    view.file = None;
    view.opened.clear();
    load(session);
    let view = &mut session.view;
    // The function the app asked for, else the first one, so that the window shows something.
    let functions = &view.published.functions;
    let asked = view.open_on.take().filter(|asked| functions.iter().any(|listed| listed.function == *asked));
    if let Some(first) = asked.or_else(|| functions.first().map(|listed| listed.function.clone())) {
        choose(view, &first);
    }
    draw(view, false);
    let functions: Vec<Value> = view
        .published
        .functions
        .iter()
        .map(|listed| json!({ "function": listed.function, "file": listed.file, "drawable": listed.drawable }))
        .collect();
    let check = view.loaded.as_ref().and_then(|loaded| serde_json::to_value(loaded.report.check_document(loaded.counts, false)).ok());
    Ok(json!({ "functions": functions, "check": check }))
}

/// `view.show_function`.
pub fn show_function(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let name = params.text("function")?;
    read_again_if_stale(session);
    let view = &mut session.view;
    known(view, name)?;
    choose(view, name);
    Ok(draw(view, false))
}

/// `view.set_standard_view`: returns the value it had, as `{ "before": … }`.
pub fn set_standard_view(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let on = params.bool("on");
    read_again_if_stale(session);
    let view = &mut session.view;
    let before = std::mem::replace(&mut view.standard_everywhere, on);
    view.published.standard_everywhere = on;
    if before == on {
        view.publish();
    } else {
        draw(view, false);
    }
    Ok(json!({ "before": before }))
}

/// `view.set_state`: all of it, or nothing.
pub fn set_state(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let name = params.text("function")?;
    let Some(state) = params.get("state").and_then(Value::as_object) else {
        return Err(CommandError::of("params.invalid", "state", "`state` is an object"));
    };
    read_again_if_stale(session);
    let view = &mut session.view;
    known(view, name)?;
    let file = file_of(view, name);
    // Read into a copy first.
    let mut of = view.views.get(&file).cloned().unwrap_or_default();
    let (placed, moved) = read_state(state, &mut of)?;
    let shown = view.function.as_deref() == Some(name);
    if !moved.is_empty() {
        let drawing = view.last.get(&file).cloned();
        let overlay = view.published.overlay.clone().filter(|_| shown);
        moved_iterations(&mut of, &moved, drawing.as_deref(), overlay.as_deref());
        // An iteration moved: the tabs picked before give way to the arms that the run took, those of this call stay.
        let given: BTreeSet<&str> =
            state.get("arms").and_then(Value::as_object).map(|arms| arms.keys().map(String::as_str).collect()).unwrap_or_default();
        if let Some(overlay) = &overlay {
            of.arms.retain(|structure, _| given.contains(structure.as_str()) || !overlay.taken.contains_key(structure));
        }
    }
    view.views.insert(file.clone(), of);
    if shown {
        // An arm shown or an iteration places nothing: only a structure unfolded or a node in another view draws again.
        if placed {
            return Ok(draw(view, false));
        }
        if moved.is_empty() {
            publish_arms(view);
            view.publish();
        } else {
            revalue(view, false);
        }
        return Ok(view.entry.clone());
    }
    Ok(render(view, name, &file).map(|(_, _, entry, _)| entry).unwrap_or(Value::Null))
}

/// After the iterations `moved` were chosen: the loops inside each go to their last iteration in its new one, and a loop
/// put on its last iteration follows the last of the runs to come (notation 6.1; decisions of the owner, 2026-11-13). The
/// counts are those of the values shown, unless a loop around it moved too.
fn moved_iterations(of: &mut ViewOf, moved: &[String], drawing: Option<&Drawing>, overlay: Option<&Overlay>) {
    let mut inside: BTreeSet<String> = BTreeSet::new();
    for key in moved {
        inside.extend(drawing.map(|drawing| labcraft_diagram::loops_inside(drawing, key)).unwrap_or_default());
    }
    for key in &inside {
        if !moved.contains(key) {
            of.iterations.remove(key);
        }
    }
    for key in moved {
        let count = overlay.and_then(|overlay| overlay.loops.iter().find(|(looped, _, _)| looped == key)).map(|(_, count, _)| *count);
        if let (Some(count), Some(&chosen), false) = (count, of.iterations.get(key), inside.contains(key))
            && chosen.saturating_add(1) >= count
        {
            of.iterations.remove(key);
        }
    }
}

/// The arm shown of each structure: the one the developer picked, else the one the run took (notation 6.1).
fn publish_arms(view: &mut View) {
    let mut arms = view.file.as_ref().and_then(|file| view.views.get(file)).map(|of| of.arms.clone()).unwrap_or_default();
    if let Some(overlay) = &view.published.overlay {
        for (structure, &arm) in &overlay.taken {
            arms.entry(structure.clone()).or_insert(arm);
        }
    }
    view.published.arms = arms;
}

/// Works out the values of the trace kept over the drawing shown, at the iterations chosen, and publishes them. With
/// `forget_picks`, an iteration selector moved or another trace arrived: the tabs the developer picked give way to the
/// arms the run took (notation 6.1).
fn revalue(view: &mut View, forget_picks: bool) {
    let drawing = view.published.drawing.clone().filter(|_| view.drawn);
    let file = view.file.clone().unwrap_or_default();
    let choice = Choice { iterations: view.views.get(&file).map(|of| of.iterations.clone()).unwrap_or_default() };
    let overlay = match (drawing, &view.traced.kept) {
        (Some(drawing), Some((trace, judged))) => Some(style::scope(&view.style, || labcraft_diagram::values(&drawing, trace, judged, &choice))),
        _ => None,
    };
    if forget_picks && let (Some(overlay), Some(of)) = (&overlay, view.views.get_mut(&file)) {
        of.arms.retain(|structure, _| !overlay.taken.contains_key(structure));
    }
    view.published.run = view.traced.kept.as_ref().filter(|_| overlay.is_some()).map(|(trace, _)| (trace.clone(), choice));
    view.published.overlay = overlay.map(Arc::new);
    view.published.overlay_generation = view.published.overlay_generation.wrapping_add(1);
    publish_arms(view);
    view.publish();
}

/// `view.open_trace`: the values of the run of `file` over the function shown, until another function is shown; without
/// `file`, those of its last run again (decisions of the owner, 2026-11-13). Returns the `trace` part of
/// `labcraft-diagram/1`, or `null` when the function has no trace.
pub fn open_trace(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let opened = match params.str("file") {
        Some(given) => match paths::judge(&*session.ws, &session.root, given, "file", Wanted::Trace, false)? {
            Place::Project { path, shown, .. } => Some((path, shown)),
            Place::Shipped(shown) => {
                return Err(CommandError::of("path.outside_project", "file", format!("`{shown}` is not a trace of the project")));
            }
        },
        None => None,
    };
    show_trace(session, opened)?;
    Ok(session.view.published.trace.as_ref().and_then(|part| serde_json::to_value(part).ok()).unwrap_or(Value::Null))
}

/// The values of the run of the trace at `path` over the function shown, as `view.open_trace` shows them: a trace that
/// the viewer's window chose anywhere on the disk, which a command cannot name (commands.md 2). Not a command.
pub fn open_trace_at(session: &mut Session, path: PathBuf) -> Result<(), CommandError> {
    let shown = match &session.view.folder {
        Some((folder, _)) if path.starts_with(folder) => slashed(path.strip_prefix(folder).unwrap_or(&path)),
        _ => slashed(&path),
    };
    show_trace(session, Some((path, shown)))
}

/// Shows the values of the trace `opened` by hand, or of the last run, over the function shown.
fn show_trace(session: &mut Session, opened: Option<(PathBuf, String)>) -> Result<(), CommandError> {
    read_again_if_stale(session);
    if session.view.function.is_none() {
        return Err(CommandError::of("params.invalid", "file", "no function is shown: `view.show_function` shows one"));
    }
    session.view.traced.opened = opened;
    session.view.traced.due = true;
    look_at_trace(session);
    Ok(())
}

/// How many folders above the folder open the viewer looks for the last run of a function (as many as a path holds).
const MAX_ABOVE: usize = 64;

/// The trace of the last run of `function` of `folder` (notation 6.1; decisions of the owner, 2026-11-13): where a
/// session whose root is the folder, or a folder above it, writes it (commands.md 8), as `labcraft-cli run FOLDER` does
/// for the folder itself; from a folder above, both for a run given that `folder` and for one of the function named
/// from the root (`power.check_power`), as an agent names it; the most recent, and at the same time the nearest. Shown
/// from the folder.
fn last_run(ws: &dyn Workspace, folder: &Path, function: &str) -> Option<(PathBuf, String)> {
    let mut best: Option<(PathBuf, String, Option<u128>)> = None;
    for (level, above) in folder.ancestors().enumerate().take(MAX_ABOVE) {
        let below = slashed(folder.strip_prefix(above).unwrap_or(folder));
        let mut candidates = vec![crate::runs::trace_path(&below, function)];
        if !below.is_empty() && below != "." {
            candidates.push(crate::runs::trace_path("", &format!("{}.{function}", below.replace('/', "."))));
        }
        for from_above in candidates {
            let path = above.join(&from_above);
            let Ok(metadata) = ws.metadata(&path) else { continue };
            if metadata.kind != crate::workspace::Kind::File || best.as_ref().is_some_and(|(_, _, time)| metadata.modified <= *time) {
                continue;
            }
            best = Some((path, format!("{}{from_above}", "../".repeat(level)), metadata.modified));
        }
    }
    best.map(|(path, shown, _)| (path, shown))
}

/// A file as it is now.
fn stamp_of(ws: &dyn Workspace, path: &Path) -> Stamp {
    (path.to_path_buf(), ws.metadata(path).ok().map(|metadata| (metadata.len, metadata.modified)))
}

/// The time of day of a time of a file, in UTC, as a short form gives a timestamp (notation 6.1): `14:02Z`.
fn time_of_day(nanoseconds: u128) -> String {
    let minutes = nanoseconds / 60_000_000_000;
    format!("{:02}:{:02}Z", (minutes / 60) % 24, minutes % 60)
}

/// Reads the trace of the function shown when its drawing is new or the trace changed: the trace opened by hand, or else
/// that of the last run. Whether anything was published. The app calls it after each command and each second (a new run
/// replaces the trace), and the drawing shows meanwhile, without values (`docs/cli.md`, 7.4).
pub fn look_at_trace(session: &mut Session) -> bool {
    let ws = &*session.ws;
    let view = &mut session.view;
    let (Some(name), Some(file)) = (view.function.clone(), view.file.clone()) else { return false };
    let target = match (&view.traced.opened, &view.folder) {
        (Some(opened), _) => Some(opened.clone()),
        (None, Some((folder, _))) => last_run(ws, folder, &name),
        (None, None) => None,
    };
    let stamp = target.as_ref().map(|(path, _)| stamp_of(ws, path));
    if !view.traced.due && stamp == view.traced.read {
        return false;
    }
    view.traced.due = false;
    // The diagnostics of reading the trace, which join those of the drawing.
    let mut read_report = Report::new(usize::MAX);
    let (mut kept, mut part) = (None, None);
    if let (Some((path, shown)), Some(loaded)) = (&target, &view.loaded)
        && let Some(function) = loaded.functions.iter().find(|function| function.shown == file)
    {
        let drawing = view.published.drawing.clone().filter(|_| view.drawn);
        // A trace that a run replaces while it is read is read again from its start (`docs/cli.md`, 7.4), a few times.
        let mut before = stamp.clone();
        let mut settled = false;
        for _ in 0..3 {
            // The trace of the attempt before goes first: never two read at once.
            drop(kept.take());
            read_report = Report::new(usize::MAX);
            let written = before.as_ref().and_then(|(_, now)| now.and_then(|(_, time)| time)).map(time_of_day);
            let (read, said) = style::scope(&view.style, || {
                drawings::read_trace(ws, path, shown, drawing.as_deref(), function, loaded, drawings::DEFAULT_TRACE_SIZE, written, &mut read_report)
            });
            (kept, part) = (read.map(|(trace, judged)| (Arc::new(trace), judged)), Some(said));
            let after = Some(stamp_of(ws, path));
            if after == before {
                settled = true;
                break;
            }
            before = after;
        }
        // A trace that kept changing while it was read is read again at the next look.
        view.traced.read = if settled { before } else { None };
    } else {
        view.traced.read = stamp;
    }
    let mut report = view.traced.report.as_deref().cloned().unwrap_or_else(|| Report::new(usize::MAX));
    report.merge(read_report);
    report.finish();
    // Another trace than the one shown last over this file, not over another function: the tabs picked give way to
    // the arms that the run took (notation 6.1).
    let of = view.views.entry(file.clone()).or_default();
    let arrived = view.traced.read.is_some() && of.trace_seen != view.traced.read;
    of.trace_seen.clone_from(&view.traced.read);
    view.traced.kept = kept;
    view.published.trace = part;
    view.published.trace_opened = view.traced.opened.is_some();
    view.published.errors = report.errors();
    view.published.warnings = report.warnings();
    view.published.report = Some(Arc::new(report));
    revalue(view, arrived);
    true
}

/// Reads `state` into `of`; whether it changes what placement depends on, and the loops whose iteration it chose.
fn read_state(state: &Map<String, Value>, of: &mut ViewOf) -> Result<(bool, Vec<String>), CommandError> {
    let mut placed = false;
    let mut moved = Vec::new();
    for (part, value) in state {
        let Some(entries) = value.as_object() else {
            return Err(CommandError::of("params.invalid", "state", format!("`state.{part}` is an object of keys")));
        };
        for (key, value) in entries {
            match (part.as_str(), value) {
                ("arms", Value::Number(number)) => {
                    let arm = number
                        .as_u64()
                        .and_then(|arm| usize::try_from(arm).ok())
                        .ok_or_else(|| CommandError::of("params.invalid", "state", format!("`state.arms.{key}` is the rank of an arm, from 0")))?;
                    of.arms.insert(key.clone(), arm);
                }
                ("iterations", Value::Number(number)) => {
                    let iteration = number
                        .as_u64()
                        .ok_or_else(|| CommandError::of("params.invalid", "state", format!("`state.iterations.{key}` is an iteration, from 0")))?;
                    if of.iterations.insert(key.clone(), iteration) != Some(iteration) {
                        moved.push(key.clone());
                    }
                }
                ("unfolded" | "standard", Value::Bool(on)) => {
                    let set = if part == "unfolded" { &mut of.unfolded } else { &mut of.standard };
                    if *on {
                        set.insert(key.clone());
                    } else {
                        set.remove(key);
                    }
                    placed = true;
                }
                _ => {
                    let error = CommandError::of("params.invalid", "state", format!("`state.{part}.{key}` is not part of the view state"));
                    return Err(error.expecting(&["arms", "unfolded", "standard", "iterations"]));
                }
            }
        }
    }
    Ok((placed, moved))
}

/// The files changed: reads and checks the folder again and draws the function shown, its nodes gliding when it is the
/// same file.
pub fn files_changed(session: &mut Session) {
    session.view.stale = false;
    if session.view.folder.is_none() {
        return;
    }
    let before = session.view.file.clone();
    load(session);
    let view = &mut session.view;
    // A function whose file went away is no longer shown; what was kept of a file gone goes too.
    let exists = |name: &String, functions: &[Listed]| functions.iter().any(|listed| listed.function == *name);
    let functions = view.published.functions.clone();
    view.opened.retain(|name| exists(name, &functions));
    let files: BTreeSet<&str> = functions.iter().map(|listed| listed.file.as_str()).collect();
    view.views.retain(|file, _| files.contains(file.as_str()));
    view.last.retain(|file, _| files.contains(file.as_str()));
    match view.function.clone() {
        // The same file while it is there: a file that breaks keeps its last drawing, even when another file gives the
        // same name.
        Some(name) if before.as_ref().is_some_and(|file| functions.iter().any(|listed| listed.function == name && listed.file == *file)) => {}
        Some(name) if exists(&name, &functions) => choose(view, &name),
        _ => match view.opened.first().cloned() {
            Some(name) => choose(view, &name),
            None => {
                view.function = None;
                view.file = None;
            }
        },
    }
    let same = before.is_some() && before == view.file;
    draw(view, same);
}

/// Draws with these values of the notation (`--style`, notation 10); the nodes glide to their new places.
pub fn set_style(session: &mut Session, style: Arc<Style>) {
    let view = &mut session.view;
    if *view.style == *style {
        return;
    }
    view.style = style;
    draw(view, true);
}

fn read_again_if_stale(session: &mut Session) {
    if session.view.stale {
        files_changed(session);
    }
}

/// Reads and checks the folder open again.
fn load(session: &mut Session) {
    let Some((path, shown)) = session.view.folder.clone() else { return };
    let config = &session.config;
    let loaded = drawings::load(&*session.ws, &config.shipped, &path, &config.lib_paths, &config.limits, Some(&session.root));
    let view = &mut session.view;
    view.published.folder = loaded.root.as_ref().map(|_| shown);
    view.published.functions = listed(&loaded);
    view.published.report = Some(Arc::new(loaded.report.clone()));
    view.published.errors = loaded.report.errors();
    view.published.warnings = loaded.report.warnings();
    view.loaded = Some(loaded);
    view.publish();
}

/// Shows the function of this name: when two files give it (`a.b.lcf` and `a/b.lcf`), the first that is drawable.
fn choose(view: &mut View, name: &str) {
    let matching = || view.published.functions.iter().filter(|listed| listed.function == name);
    let file = matching().find(|listed| listed.drawable).or_else(|| matching().next()).map(|listed| listed.file.clone());
    // A trace opened by hand is that of one function.
    if view.function.as_deref() != Some(name) {
        view.traced.opened = None;
    }
    view.function = Some(name.to_owned());
    view.file = file;
    if !view.opened.iter().any(|opened| opened == name) {
        view.opened.push(name.to_owned());
    }
}

fn known(view: &View, name: &str) -> Result<(), CommandError> {
    if view.published.functions.iter().any(|listed| listed.function == name) {
        return Ok(());
    }
    if view.folder.is_none() {
        return Err(CommandError::of("params.invalid", "function", format!("unknown function `{name}`: no folder is open, `view.open` opens one")));
    }
    let names: Vec<&str> = view.published.functions.iter().map(|listed| listed.function.as_str()).take(10).collect();
    Err(CommandError::of("params.invalid", "function", format!("unknown function `{name}`: not a function of the folder open")).expecting(&names))
}

/// The file of the function of this name that the viewer shows for it.
fn file_of(view: &View, name: &str) -> String {
    if view.function.as_deref() == Some(name)
        && let Some(file) = &view.file
    {
        return file.clone();
    }
    let matching = || view.published.functions.iter().filter(|listed| listed.function == name);
    matching().find(|listed| listed.drawable).or_else(|| matching().next()).map(|listed| listed.file.clone()).unwrap_or_default()
}

/// The picture of a function in its view state, whether it is drawn, its entry of `diagrams` in `labcraft-diagram/1`,
/// and the report of the check with what drawing it added (`LC7005`, in this view state).
fn render(view: &View, name: &str, file: &str) -> Option<(Arc<Drawing>, bool, Value, Report)> {
    let loaded = view.loaded.as_ref()?;
    let function = loaded.functions.iter().find(|function| function.shown == file)?;
    let of = view.views.get(file).cloned().unwrap_or_default();
    let state = ViewState {
        standard_everywhere: view.standard_everywhere,
        fold_constructions: true,
        tabs: true,
        unfolded: of.unfolded,
        standard: of.standard,
    };
    let mut report = loaded.report.clone();
    let files = drawings::files_of(loaded.checked.as_ref());
    // Drawn with the values in force (notation 10).
    style::scope(&view.style, || {
        let (drawing, nodes, codes) =
            drawings::drawing_of(function, loaded.checked.as_ref(), &files, &loaded.names, &state, &DiagramLimits::default(), &mut report);
        let drawn = drawing.is_some();
        let picture = Arc::new(drawings::picture(function, drawing, &report));
        Some((picture, drawn, entry_of(name, file, drawn, nodes, &codes), report))
    })
}

/// Draws the function shown; returns its entry of `diagrams` in `labcraft-diagram/1`.
fn draw(view: &mut View, glide: bool) -> Value {
    view.publish();
    view.published.function.clone_from(&view.function);
    view.published.opened.clone_from(&view.opened);
    view.published.drawing_generation = view.published.drawing_generation.wrapping_add(1);
    view.published.glide = glide;
    // The values of the drawing before are not those of this one: they come once its trace is read.
    view.traced.due = true;
    view.drawn = false;
    view.traced.kept = None;
    view.published.run = None;
    if view.published.overlay.take().is_some() {
        view.published.overlay_generation = view.published.overlay_generation.wrapping_add(1);
    }
    let rendered = match (view.function.clone(), view.file.clone()) {
        (Some(name), Some(file)) if !view.skipped.contains(&name) => {
            if let Some(before) = view.before_drawing {
                before(&name);
            }
            render(view, &name, &file).map(|rendered| (file, rendered))
        }
        _ => None,
    };
    let Some((file, (picture, drawn, entry, report))) = rendered else {
        view.published.drawing = None;
        view.published.kept = false;
        view.published.trace = None;
        view.traced.report.clone_from(&view.published.report);
        view.entry = Value::Null;
        return Value::Null;
    };
    view.drawn = drawn;
    let shown = if drawn {
        view.last.insert(file.clone(), picture.clone());
        view.published.kept = false;
        picture
    } else if let Some(last) = view.last.get(&file) {
        view.published.kept = true;
        last.clone()
    } else {
        view.published.kept = false;
        picture
    };
    view.published.drawing = Some(shown);
    // The warnings of the drawing (`LC7005`) have no mark: the problems of the viewer list them.
    let counts = |report: &Report| (report.diagnostics().len(), report.errors(), report.warnings());
    if let Some(loaded) = &view.loaded
        && counts(&report) != counts(&loaded.report)
    {
        view.published.errors = report.errors();
        view.published.warnings = report.warnings();
        view.published.report = Some(Arc::new(report));
    } else if let Some(loaded) = &view.loaded
        && view.published.report.as_ref().is_some_and(|published| counts(published) != counts(&loaded.report))
    {
        // Back to the report of the check when the drawing shown adds nothing.
        view.published.errors = loaded.report.errors();
        view.published.warnings = loaded.report.warnings();
        view.published.report = Some(Arc::new(loaded.report.clone()));
    }
    view.published.arms = view.views.get(&file).map(|of| of.arms.clone()).unwrap_or_default();
    view.traced.report.clone_from(&view.published.report);
    view.entry = entry.clone();
    entry
}

/// An entry of `diagrams` in `labcraft-diagram/1` (`docs/cli.md`, 7.5), without the path of an SVG.
fn entry_of(name: &str, file: &str, drawn: bool, nodes: Option<u64>, codes: &[Code]) -> Value {
    let codes: Vec<&str> = codes.iter().map(|code| code.as_str()).collect();
    let mut entry = json!({ "function": name, "file": file, "drawn": drawn });
    if let Some(object) = entry.as_object_mut() {
        if let Some(nodes) = nodes {
            object.insert("nodes".to_owned(), json!(nodes));
        }
        if !codes.is_empty() {
            object.insert("codes".to_owned(), json!(codes));
        }
    }
    entry
}

/// The functions of a folder, with whether each is drawable and the counts of its file.
fn listed(loaded: &Loaded) -> Vec<Listed> {
    let files = drawings::files_of(loaded.checked.as_ref());
    loaded
        .functions
        .iter()
        .map(|function: &Function| {
            let counts = loaded.report.counts_of(&function.shown);
            let drawable = files.contains_key(function.shown.as_str()) && loaded.report.keeping_from_drawing(&function.shown).is_empty();
            Listed { function: function.name.clone(), file: function.shown.clone(), drawable, errors: counts.errors, warnings: counts.warnings }
        })
        .collect()
}
