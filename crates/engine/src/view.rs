//! The state of the viewer (`docs/architecture.md`, 10.8 and 11.5; `docs/commands.md`, 6.5): the folder open, its
//! check, what each window shows, the view state that the file does not hold, the preference of the standard view. The
//! `view.*` commands change it; the app reads what it publishes ([`Session::view`]), the drawings as `Arc<Drawing>`.
//!
//! Each window of the project shows one function (ide.md 9): the window of the project the function of its tab shown,
//! and each detached window its own. Each has its drawing, its values, its trace, its selection and its zoom
//! ([`Shown`]); the view state of a file and the last drawing of each file are shared.
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
use crate::paths::{self, Wanted};
use crate::session::Session;
use crate::walk::slashed;
use crate::workspace::Workspace;

pub mod window;

pub use window::{Selection, Tab, ViewKind, WorkspaceId};

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
    /// Its inputs with no default, in their order: a run started with no argument cannot give them (`LC8103`).
    pub needs: Vec<String>,
}

/// A type of the folder, as the tree of the window lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedType {
    pub name: String,
    pub file: String,
}

/// What one window shows (ide.md 9): the window of the project, the function of its tab shown; a detached window, its
/// function.
#[derive(Clone, Debug, Default)]
pub struct Shown {
    /// None in the window of the project when no tab is open; always one in a detached window.
    pub function: Option<String>,
    pub drawing: Option<Arc<Drawing>>,
    /// Changes with each drawing of this window. One count for all the windows: a function that moves to another
    /// window is never given a count that its canvas has seen.
    pub drawing_generation: u64,
    /// Whether the nodes glide to this drawing: the file of the function changed.
    pub glide: bool,
    /// The function is not drawable now: its last drawing stays, with its diagnostics (scenario 6, point 3).
    pub kept: bool,
    /// The arm shown of each structure in tabs: the one the developer picked, else the one the run took (notation 6.1).
    pub arms: HashMap<String, usize>,
    /// The trace whose values show and the iterations chosen, from which the canvas works out the values of a wire over
    /// the iterations of its loop (notation 6.1); published with each overlay.
    pub run: Option<(Arc<Trace>, Choice)>,
    /// The values of the run over the drawing (notation 6.1): none until its trace is read, and none without one.
    pub overlay: Option<Arc<Overlay>>,
    /// Changes with each overlay, counted as `drawing_generation` is.
    pub overlay_generation: u64,
    /// The trace read for the function, as the `trace` part of `labcraft-diagram/1` says it; none when it has none.
    pub trace: Option<TracePart>,
    /// The trace was opened by hand (`view.open_trace` with a `file`): it stays until the window shows another function.
    pub trace_opened: bool,
    /// The selection of the window (`view.select`), in the function it names.
    pub selection: Selection,
}

/// What the windows show, as the session last published it.
#[derive(Clone, Debug, Default)]
pub struct Published {
    /// Changes with each publication.
    pub generation: u64,
    /// The folder open, as the paths of the session show it (`.` for the root); none when none is open or it could not
    /// be read.
    pub folder: Option<String>,
    pub functions: Vec<Listed>,
    /// The types of the folder, each with its file (`Step.lct`), as the tree of the window lists them (ide.md 4).
    pub types: Vec<ListedType>,
    /// The report of the check of the folder, with the diagnostics of the drawings shown (`LC7005`) and of reading their
    /// traces; a new one with each check and each drawing.
    pub report: Option<Arc<Report>>,
    /// Changes with each check of the folder, and only then: what the window read of the files is read again.
    pub checks: u64,
    pub errors: u64,
    pub warnings: u64,
    /// What the window of the project shows. Set in the snapshot only: the session holds the windows apart.
    pub main: Shown,
    /// The tabs of the window of the project.
    pub opened: Vec<String>,
    /// The detached windows, in the order they were detached, each with its function (ide.md 9).
    pub detached: Vec<Shown>,
    pub standard_everywhere: bool,
    /// The view of each tab open, in any window, whose view is not the diagram alone (ide.md 3).
    pub tabs: HashMap<String, Tab>,
    /// The function whose tab is a preview, replaced by the next one (ide.md 3).
    pub preview: Option<String>,
    /// The workspace of the window (`view.set_workspace`).
    pub workspace: window::WorkspaceId,
    /// The settings of the user and of the project (commands.md 9.7): the window reads what they give, and the host
    /// keeps those of the user.
    pub settings: Arc<crate::settings::Stores>,
}

impl Published {
    /// The window of the project, then the detached ones.
    pub fn windows(&self) -> impl Iterator<Item = &Shown> {
        std::iter::once(&self.main).chain(&self.detached)
    }

    /// The window that shows this function: the window of the project when it is its tab shown, or its detached window.
    pub fn window_of(&self, function: &str) -> Option<&Shown> {
        self.windows().find(|shown| shown.function.as_deref() == Some(function))
    }

    /// Whether the function is in a window of its own.
    pub fn is_detached(&self, function: &str) -> bool {
        self.detached.iter().any(|shown| shown.function.as_deref() == Some(function))
    }
}

/// The trace of the function of a window (notation 6.1; decisions of the owner, 2026-11-13).
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
}

/// A trace file as it is: where, its length and its time; none of the two when it does not exist.
type Stamp = (PathBuf, Option<(u64, Option<u128>)>);

/// A window as the session holds it: what it publishes, and what it keeps to work it out.
#[derive(Default)]
struct Window {
    shown: Shown,
    /// The file of its function: two files can give one name.
    file: Option<String>,
    /// The entry of its function in `labcraft-diagram/1`.
    entry: Value,
    /// Whether its function was drawn now, and not kept from before: only then are values drawn over it.
    drawn: bool,
    traced: Traced,
    /// What drawing its function added to the report of the check (`LC7005`), and what reading its trace added.
    drawing_report: Option<Report>,
    trace_report: Option<Report>,
}

/// Which window: the one of the project, or the detached one of a function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Which {
    Main,
    Detached(String),
}

/// The state of the viewer that the session holds.
#[derive(Default)]
pub struct View {
    /// The folder open, without links, and how it is shown.
    folder: Option<(PathBuf, String)>,
    loaded: Option<Loaded>,
    /// The report of the check of the folder, as `load` published it: what the window shows when no window adds to it.
    checked: Option<Arc<Report>>,
    views: HashMap<String, ViewOf>,
    /// The last drawing of each file whose function was drawable: it holds the values it was drawn with.
    last: HashMap<String, Arc<Drawing>>,
    /// The zoom of the canvas of each function with a tab, as the window last gave it (`Session::set_zoom`): each tab
    /// has its canvas, which keeps its zoom across tabs and windows. `view.get_state` returns that of the function each
    /// window shows, `null` until the window gives it.
    pub(crate) zooms: HashMap<String, f32>,
    /// The window of the project, and the detached ones in their order.
    main: Window,
    detached: Vec<Window>,
    /// What is shown now, the one state of the view (architecture 12.5), but for the windows: the snapshot takes them
    /// from `main` and `detached`.
    now: Published,
    /// What the window reads: a snapshot of `now` and of the windows, made by each publication and kept nowhere else.
    snapshot: Arc<Published>,
    /// `now` was published since the snapshot was made.
    due: bool,
    /// The count of the drawings and the overlays of all the windows.
    counted: u64,
    /// The values of the notation in force (notation 10): those of `--style`, given by the app.
    style: Arc<Style>,
    /// What the settings of `now` change of the colours of the blocks, computed when they change.
    appearance: labcraft_diagram::Appearance,
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
}

impl View {
    /// The view state that the function of `file` is drawn in, and the values of the notation in force: what a move of a
    /// node placed automatically starts from is the drawing that the window shows (architecture 12.3).
    pub(crate) fn state_of(&self, file: &str) -> (ViewState, Arc<Style>) {
        let of = self.views.get(file).cloned().unwrap_or_default();
        let state = ViewState {
            standard_everywhere: self.now.standard_everywhere,
            fold_constructions: true,
            tabs: true,
            unfolded: of.unfolded,
            standard: of.standard,
            appearance: self.appearance.clone(),
        };
        (state, self.style.clone())
    }

    pub fn published(&self) -> &Arc<Published> {
        &self.snapshot
    }

    /// The file of the function that this window shows: two files can give one name.
    pub(crate) fn file_shown(&self, which: &Which) -> Option<&str> {
        self.window(which).and_then(|window| window.file.as_deref())
    }

    /// Where the detached window of this function is among the detached ones.
    fn detached_at(&self, function: &str) -> Option<usize> {
        self.detached.iter().position(|window| window.shown.function.as_deref() == Some(function))
    }

    /// Whether the function is in a window of its own.
    pub(crate) fn is_detached(&self, function: &str) -> bool {
        self.detached_at(function).is_some()
    }

    /// The functions in a window of their own, in their order.
    fn detached_functions(&self) -> impl Iterator<Item = &str> {
        self.detached.iter().filter_map(|window| window.shown.function.as_deref())
    }

    /// The window that shows this function.
    /// Whether the tab of `function` shows the view `Block`, alone or beside `Text` (ide.md 3).
    pub(crate) fn shows_block(&self, function: &str) -> bool {
        self.now.tabs.get(function).is_some_and(|tab| tab.view == window::ViewKind::Block || tab.split == Some(window::ViewKind::Block))
    }

    pub(crate) fn which_shows(&self, function: &str) -> Option<Which> {
        if self.main.shown.function.as_deref() == Some(function) {
            return Some(Which::Main);
        }
        self.is_detached(function).then(|| Which::Detached(function.to_owned()))
    }

    fn window(&self, which: &Which) -> Option<&Window> {
        match which {
            Which::Main => Some(&self.main),
            Which::Detached(function) => self.detached.get(self.detached_at(function)?),
        }
    }

    fn window_mut(&mut self, which: &Which) -> Option<&mut Window> {
        match which {
            Which::Main => Some(&mut self.main),
            Which::Detached(function) => {
                let at = self.detached_at(function)?;
                self.detached.get_mut(at)
            }
        }
    }

    /// Removes a detached window.
    fn remove_detached(&mut self, function: &str) -> Option<Window> {
        let at = self.detached_at(function)?;
        Some(self.detached.remove(at))
    }

    /// Each window: the one of the project, then the detached ones.
    fn all(&self) -> Vec<Which> {
        std::iter::once(Which::Main).chain(self.detached_functions().map(|function| Which::Detached(function.to_owned()))).collect()
    }

    /// The next count of a drawing or an overlay.
    fn count(&mut self) -> u64 {
        self.counted = self.counted.wrapping_add(1);
        self.counted
    }

    fn publish(&mut self) {
        self.now.generation = self.now.generation.wrapping_add(1);
        self.due = true;
    }

    /// Makes the snapshot that the window reads, once what a command or a change from the system published is all
    /// done: the session calls it at the end of each.
    pub(crate) fn settle(&mut self) {
        if std::mem::take(&mut self.due) {
            self.snapshot = Arc::new(Published {
                main: self.main.shown.clone(),
                detached: self.detached.iter().map(|window| window.shown.clone()).collect(),
                ..self.now.clone()
            });
        }
    }

    /// The function of `file` is renamed `name` in `new_file` (`function.rename`): its tab, its window, its view and its
    /// zoom follow it, in their places (ide.md 4), before the folder is read again.
    pub(crate) fn function_renamed(&mut self, file: &str, name: &str, new_file: &str) {
        let Some(old) = self.now.functions.iter().find(|listed| listed.file == file).map(|listed| listed.function.clone()) else { return };
        let new = match old.rsplit_once('.') {
            Some((library, _)) => format!("{library}.{name}"),
            None => name.to_owned(),
        };
        let follow = |function: &mut String| {
            if *function == old {
                function.clone_from(&new);
            }
        };
        self.now.opened.iter_mut().for_each(follow);
        if let Some(preview) = &mut self.now.preview {
            follow(preview);
        }
        if let Some(tab) = self.now.tabs.remove(&old) {
            self.now.tabs.insert(new.clone(), tab);
        }
        if let Some(zoom) = self.zooms.remove(&old) {
            self.zooms.insert(new.clone(), zoom);
        }
        for window in std::iter::once(&mut self.main).chain(self.detached.iter_mut()) {
            if window.file.as_deref() == Some(file) {
                window.file = Some(new_file.to_owned());
            }
            if let Some(function) = &mut window.shown.function {
                follow(function);
            }
        }
        if let Some(of) = self.views.remove(file) {
            self.views.insert(new_file.to_owned(), of);
        }
        if let Some(last) = self.last.remove(file) {
            self.last.insert(new_file.to_owned(), last);
        }
        if let Some(listed) = self.now.functions.iter_mut().find(|listed| listed.file == file) {
            listed.function.clone_from(&new);
            listed.file = new_file.to_owned();
        }
        self.publish();
    }

    /// The zoom of the canvas of this function, which has a tab.
    pub(crate) fn set_zoom(&mut self, function: &str, zoom: f32) {
        if self.now.opened.iter().any(|opened| opened == function) || self.is_detached(function) {
            self.zooms.insert(function.to_owned(), zoom);
        }
    }
}

/// Runs `work` on a window swapped out of the view, with the rest of the view, and swaps it back: meanwhile its slot
/// holds an empty window, which `work` never looks at.
fn on_window<R>(view: &mut View, which: &Which, work: impl FnOnce(&mut View, &mut Window) -> R) -> Option<R> {
    let at = match which {
        Which::Main => None,
        Which::Detached(function) => Some(view.detached_at(function)?),
    };
    let mut window = std::mem::take(slot(view, at)?);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(view, &mut window)));
    if let Some(slot) = slot(view, at) {
        *slot = window;
    }
    match result {
        Ok(result) => Some(result),
        // The panic goes on to the caller that catches it (`panics::guarded`, the thread of the app).
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// The window of the project, or the detached window at `at`.
fn slot(view: &mut View, at: Option<usize>) -> Option<&mut Window> {
    match at {
        None => Some(&mut view.main),
        Some(at) => view.detached.get_mut(at),
    }
}

/// `view.open`.
pub fn open(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let folder = params.text("folder")?;
    let (path, shown) = paths::existing_in_project(&*session.ws, &session.root, folder, "folder", Wanted::Folder)?;
    let view = &mut session.view;
    view.folder = Some((path, shown));
    view.views.clear();
    view.last.clear();
    view.main = Window::default();
    view.detached.clear();
    view.now.opened.clear();
    window::forget_closed(view);
    load(session);
    let view = &mut session.view;
    // The function the app asked for, else the first one, so that the window shows something.
    let functions = &view.now.functions;
    let asked = view.open_on.take().filter(|asked| functions.iter().any(|listed| listed.function == *asked));
    if let Some(first) = asked.or_else(|| functions.first().map(|listed| listed.function.clone())) {
        choose(view, &first);
    }
    draw(view, &Which::Main, false);
    let functions: Vec<Value> =
        view.now.functions.iter().map(|listed| json!({ "function": listed.function, "file": listed.file, "drawable": listed.drawable })).collect();
    let check = view.loaded.as_ref().and_then(|loaded| serde_json::to_value(loaded.report.check_document(loaded.counts, false)).ok());
    Ok(json!({ "functions": functions, "check": check }))
}

/// `view.show_function`: a function in a window of its own is shown there, and opens no tab.
pub fn show_function(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let name = params.text("function")?;
    read_again_if_stale(session);
    let view = &mut session.view;
    known(view, name)?;
    let detached = Which::Detached(name.to_owned());
    if let Some(window) = view.window(&detached) {
        return Ok(window.entry.clone());
    }
    window::place_tab(view, name, params.bool("preview"));
    choose(view, name);
    Ok(draw(view, &Which::Main, false))
}

/// `view.set_standard_view`: the setting `view.standard_everywhere` of the user (commands.md 9.7). Returns the value it
/// had, as `{ "before": … }`.
pub fn set_standard_view(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let on = params.bool("on");
    let before = session.view.now.standard_everywhere;
    crate::settings::write_user(session, crate::settings::STANDARD_EVERYWHERE, Some(Value::Bool(on)))?;
    settings_changed(session);
    Ok(json!({ "before": before }))
}

/// The settings changed, or were read: published, and the windows drawn again when what they draw changed.
pub(crate) fn settings_changed(session: &mut Session) {
    let values = session.settings.drawn().values();
    let standard = values.standard_everywhere;
    let appearance = labcraft_diagram::Appearance { neutral_blocks: values.neutral_blocks, library_colors: values.library_colors };
    let drawn_again = standard != session.view.now.standard_everywhere || appearance != session.view.appearance;
    session.view.now.settings = Arc::new(session.settings.stores.clone());
    session.view.now.standard_everywhere = standard;
    session.view.appearance = appearance;
    if drawn_again {
        read_again_if_stale(session);
        draw_all(&mut session.view, false);
    } else {
        session.view.publish();
    }
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
    let which = view.which_shows(name);
    if !moved.is_empty() {
        let drawing = view.last.get(&file).cloned();
        let overlay = which.as_ref().and_then(|which| view.window(which)).and_then(|window| window.shown.overlay.clone());
        moved_iterations(&mut of, &moved, drawing.as_deref(), overlay.as_deref());
        // An iteration moved: the tabs picked before give way to the arms that the run took, those of this call stay.
        let given: BTreeSet<&str> =
            state.get("arms").and_then(Value::as_object).map(|arms| arms.keys().map(String::as_str).collect()).unwrap_or_default();
        if let Some(overlay) = &overlay {
            of.arms.retain(|structure, _| given.contains(structure.as_str()) || !overlay.taken.contains_key(structure));
        }
    }
    view.views.insert(file.clone(), of);
    if let Some(which) = which {
        // An arm shown or an iteration places nothing: only a structure unfolded or a node in another view draws again.
        if placed {
            return Ok(draw(view, &which, false));
        }
        on_window(view, &which, |view, window| {
            if moved.is_empty() {
                publish_arms(view, window);
                view.publish();
            } else {
                revalue(view, window, false);
            }
        });
        return Ok(view.window(&which).map(|window| window.entry.clone()).unwrap_or(Value::Null));
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
fn publish_arms(view: &View, window: &mut Window) {
    let mut arms = window.file.as_ref().and_then(|file| view.views.get(file)).map(|of| of.arms.clone()).unwrap_or_default();
    if let Some(overlay) = &window.shown.overlay {
        for (structure, &arm) in &overlay.taken {
            arms.entry(structure.clone()).or_insert(arm);
        }
    }
    window.shown.arms = arms;
}

/// Works out the values of the trace kept over the drawing of a window, at the iterations chosen, and publishes them.
/// With `forget_picks`, an iteration selector moved or another trace arrived: the tabs the developer picked give way to
/// the arms the run took (notation 6.1).
fn revalue(view: &mut View, window: &mut Window, forget_picks: bool) {
    let drawing = window.shown.drawing.clone().filter(|_| window.drawn);
    let file = window.file.clone().unwrap_or_default();
    let choice = Choice { iterations: view.views.get(&file).map(|of| of.iterations.clone()).unwrap_or_default() };
    let overlay = match (drawing, &window.traced.kept) {
        (Some(drawing), Some((trace, judged))) => Some(style::scope(&view.style, || labcraft_diagram::values(&drawing, trace, judged, &choice))),
        _ => None,
    };
    if forget_picks && let (Some(overlay), Some(of)) = (&overlay, view.views.get_mut(&file)) {
        of.arms.retain(|structure, _| !overlay.taken.contains_key(structure));
    }
    window.shown.run = window.traced.kept.as_ref().filter(|_| overlay.is_some()).map(|(trace, _)| (trace.clone(), choice));
    window.shown.overlay = overlay.map(Arc::new);
    window.shown.overlay_generation = view.count();
    publish_arms(view, window);
    view.publish();
}

/// `view.open_trace`: the values of the run of `file` over the function of a window, until that window shows another
/// function; without `file`, those of its last run again (decisions of the owner, 2026-11-13). The window is the one that
/// shows `function`, else the window of the project (decision of the owner, 2026-11-16: the window that has focus,
/// which the IDE names). Returns the `trace` part of `labcraft-diagram/1`, or `null` when the function has no trace.
pub fn open_trace(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let opened = match params.str("file") {
        Some(given) => {
            let (path, shown, _) = paths::in_project(&*session.ws, &session.root, given, "file", Wanted::Trace)?;
            Some((path, shown))
        }
        None => None,
    };
    let which = show_trace(session, params.str("function"), opened)?;
    Ok(session
        .view
        .window(&which)
        .and_then(|window| window.shown.trace.as_ref())
        .and_then(|part| serde_json::to_value(part).ok())
        .unwrap_or(Value::Null))
}

/// The values of the run of the trace at `path` over the function of a window (the one that shows `function`, else the
/// window of the project), as `view.open_trace` shows them: a trace that the viewer's window chose anywhere on the
/// disk, which a command cannot name (commands.md 2). Not a command.
pub fn open_trace_at(session: &mut Session, function: Option<&str>, path: PathBuf) -> Result<(), CommandError> {
    let shown = match &session.view.folder {
        Some((folder, _)) if path.starts_with(folder) => slashed(path.strip_prefix(folder).unwrap_or(&path)),
        _ => slashed(&path),
    };
    show_trace(session, function, Some((path, shown))).map(|_| ())
}

/// Shows the values of the trace `opened` by hand, or of the last run, over the function of a window; the window.
fn show_trace(session: &mut Session, function: Option<&str>, opened: Option<(PathBuf, String)>) -> Result<Which, CommandError> {
    read_again_if_stale(session);
    let view = &mut session.view;
    let which = match function {
        Some(function) => view.which_shows(function).ok_or_else(|| {
            CommandError::of("params.invalid", "function", format!("`{function}` is not shown in a window: `view.show_function` shows it"))
        })?,
        None => Which::Main,
    };
    let Some(window) = view.window_mut(&which).filter(|window| window.shown.function.is_some()) else {
        return Err(CommandError::of("params.invalid", "file", "no function is shown: `view.show_function` shows one"));
    };
    window.traced.opened = opened;
    window.traced.due = true;
    look_at_trace_of(session, &which);
    compose_report(&mut session.view);
    Ok(which)
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

/// Reads the trace of the function of each window when its drawing is new or the trace changed: the trace opened by
/// hand, or else that of the last run. Whether anything was published. The app calls it after each command and each
/// second (a new run replaces the trace), and the drawing shows meanwhile, without values (`docs/cli.md`, 7.4).
pub fn look_at_trace(session: &mut Session) -> bool {
    let mut published = false;
    for which in session.view.all() {
        published |= look_at_trace_of(session, &which);
    }
    if published {
        compose_report(&mut session.view);
    }
    published
}

/// [`look_at_trace`] for one window, without the report.
fn look_at_trace_of(session: &mut Session, which: &Which) -> bool {
    let ws = &*session.ws;
    on_window(&mut session.view, which, |view, window| look_at_trace_in(ws, view, window)).unwrap_or(false)
}

fn look_at_trace_in(ws: &dyn Workspace, view: &mut View, window: &mut Window) -> bool {
    let (Some(name), Some(file)) = (window.shown.function.clone(), window.file.clone()) else { return false };
    let target = match (&window.traced.opened, &view.folder) {
        (Some(opened), _) => Some(opened.clone()),
        (None, Some((folder, _))) => last_run(ws, folder, &name),
        (None, None) => None,
    };
    let stamp = target.as_ref().map(|(path, _)| stamp_of(ws, path));
    if !window.traced.due && stamp == window.traced.read {
        return false;
    }
    window.traced.due = false;
    // The diagnostics of reading the trace, which join those of the drawing.
    let mut read_report = Report::new(usize::MAX);
    let (mut kept, mut part) = (None, None);
    if let (Some((path, shown)), Some(loaded)) = (&target, &view.loaded)
        && let Some(function) = loaded.functions.iter().find(|function| function.shown == file)
    {
        let drawing = window.shown.drawing.clone().filter(|_| window.drawn);
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
        window.traced.read = if settled { before } else { None };
    } else {
        window.traced.read = stamp;
    }
    window.trace_report = Some(read_report);
    // Another trace than the one shown last over this file, not over another function: the tabs picked give way to
    // the arms that the run took (notation 6.1).
    let of = view.views.entry(file.clone()).or_default();
    let arrived = window.traced.read.is_some() && of.trace_seen != window.traced.read;
    of.trace_seen.clone_from(&window.traced.read);
    window.traced.kept = kept;
    window.shown.trace = part;
    window.shown.trace_opened = window.traced.opened.is_some();
    revalue(view, window, arrived);
    true
}

/// The report that the window shows: that of the check of the folder, with what drawing the function of each window
/// added (`LC7005`) and what reading its trace added.
fn compose_report(view: &mut View) {
    let Some(checked) = view.checked.clone() else { return };
    let added: Vec<&Report> = std::iter::once(&view.main)
        .chain(&view.detached)
        .flat_map(|window| [&window.drawing_report, &window.trace_report])
        .flatten()
        .filter(|added| !is_empty(added))
        .collect();
    let report = if added.is_empty() {
        checked
    } else {
        let mut report = (*checked).clone();
        for added in added {
            report.merge(added.clone());
        }
        report.finish();
        Arc::new(report)
    };
    view.now.errors = report.errors();
    view.now.warnings = report.warnings();
    view.now.report = Some(report);
    view.publish();
}

/// A report that says nothing.
fn is_empty(report: &Report) -> bool {
    report.diagnostics().is_empty() && report.errors() == 0 && report.warnings() == 0
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

/// The files changed: reads and checks the folder again and draws the function of each window, its nodes gliding when
/// it is the same file.
pub fn files_changed(session: &mut Session) {
    session.view.stale = false;
    if session.view.folder.is_none() {
        return;
    }
    let before = session.view.main.file.clone();
    load(session);
    let view = &mut session.view;
    // A function whose file went away is no longer shown; what was kept of a file gone goes too.
    let exists = |name: &String, functions: &[Listed]| functions.iter().any(|listed| listed.function == *name);
    let functions = view.now.functions.clone();
    view.now.opened.retain(|name| exists(name, &functions));
    // A detached window whose function went away closes.
    view.detached.retain(|window| window.shown.function.as_ref().is_some_and(|name| exists(name, &functions)));
    window::forget_closed(view);
    let files: BTreeSet<&str> = functions.iter().map(|listed| listed.file.as_str()).collect();
    view.views.retain(|file, _| files.contains(file.as_str()));
    view.last.retain(|file, _| files.contains(file.as_str()));
    match view.main.shown.function.clone() {
        // The same file while it is there: a file that breaks keeps its last drawing, even when another file gives the
        // same name.
        Some(name) if before.as_ref().is_some_and(|file| functions.iter().any(|listed| listed.function == name && listed.file == *file)) => {}
        Some(name) if exists(&name, &functions) => choose(view, &name),
        _ => match view.now.opened.first().cloned() {
            Some(name) => choose(view, &name),
            None => {
                view.main.shown.function = None;
                view.main.file = None;
            }
        },
    }
    let same = before.is_some() && before == view.main.file;
    draw_one(view, &Which::Main, same);
    // Each detached window, the same way: its file while it gives the function, else the file that gives it now.
    let detached: Vec<(String, Option<String>)> =
        view.detached.iter().filter_map(|window| Some((window.shown.function.clone()?, window.file.clone()))).collect();
    for (name, before) in detached {
        let still = before.as_ref().is_some_and(|file| functions.iter().any(|listed| listed.function == name && listed.file == *file));
        let which = Which::Detached(name.clone());
        if !still && let Some(window) = view.window_mut(&which) {
            window.file = file_for(&functions, &name).map(str::to_owned);
        }
        draw_one(view, &which, still);
    }
    compose_report(view);
}

/// Draws with these values of the notation (`--style`, notation 10); the nodes glide to their new places.
pub fn set_style(session: &mut Session, style: Arc<Style>) {
    let view = &mut session.view;
    if *view.style == *style {
        return;
    }
    view.style = style;
    draw_all(view, true);
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
    view.now.folder = loaded.root.as_ref().map(|_| shown);
    view.now.functions = listed(&loaded);
    view.now.types = listed_types(&loaded);
    let checked = Arc::new(loaded.report.clone());
    view.checked = Some(checked.clone());
    view.now.report = Some(checked);
    view.now.checks = view.now.checks.wrapping_add(1);
    view.now.errors = loaded.report.errors();
    view.now.warnings = loaded.report.warnings();
    view.loaded = Some(loaded);
    view.publish();
}

/// The file that shows the function of this name: when two files give it (`a.b.lcf` and `a/b.lcf`), the first that is
/// drawable.
pub(crate) fn file_for<'a>(functions: &'a [Listed], name: &str) -> Option<&'a str> {
    let matching = || functions.iter().filter(|listed| listed.function == name);
    matching().find(|listed| listed.drawable).or_else(|| matching().next()).map(|listed| listed.file.as_str())
}

/// Shows the function of this name in the window of the project.
fn choose(view: &mut View, name: &str) {
    let file = file_for(&view.now.functions, name).map(str::to_owned);
    let main = &mut view.main;
    // A trace opened by hand is that of one function.
    if main.shown.function.as_deref() != Some(name) {
        main.traced.opened = None;
    }
    main.shown.function = Some(name.to_owned());
    main.file = file;
    if !view.now.opened.iter().any(|opened| opened == name) {
        view.now.opened.push(name.to_owned());
    }
}

fn known(view: &View, name: &str) -> Result<(), CommandError> {
    if view.now.functions.iter().any(|listed| listed.function == name) {
        return Ok(());
    }
    if view.folder.is_none() {
        return Err(CommandError::of("params.invalid", "function", format!("unknown function `{name}`: no folder is open, `view.open` opens one")));
    }
    let names: Vec<&str> = view.now.functions.iter().map(|listed| listed.function.as_str()).take(10).collect();
    Err(CommandError::of("params.invalid", "function", format!("unknown function `{name}`: not a function of the folder open")).expecting(&names))
}

/// The file of the function of this name that the viewer shows for it: the one of its window, when one shows it.
fn file_of(view: &View, name: &str) -> String {
    if let Some(which) = view.which_shows(name)
        && let Some(file) = view.file_shown(&which)
    {
        return file.to_owned();
    }
    file_for(&view.now.functions, name).unwrap_or_default().to_owned()
}

/// The picture of a function in its view state, whether it is drawn, its entry of `diagrams` in `labcraft-diagram/1`,
/// and what drawing it added to the report of the check (`LC7005`, in this view state).
fn render(view: &View, name: &str, file: &str) -> Option<(Arc<Drawing>, bool, Value, Report)> {
    let loaded = view.loaded.as_ref()?;
    let function = loaded.functions.iter().find(|function| function.shown == file)?;
    let (state, _) = view.state_of(file);
    let files = drawings::files_of(loaded.checked.as_ref());
    // Drawn with the values in force (notation 10).
    style::scope(&view.style, || {
        let mut added = Report::new(usize::MAX);
        let keeping = loaded.report.keeping_from_drawing(&function.shown);
        let (mut drawing, nodes, codes) =
            drawings::drawing_apart(function, loaded.checked.as_ref(), &files, &loaded.names, &state, &DiagramLimits::default(), keeping, &mut added);
        // The block of the function alone, for a tab that shows the view `Block` (ide.md 3); an export never has it.
        if let (true, Some(drawing), Some(checked), Some(&index)) =
            (view.shows_block(name), drawing.as_mut(), loaded.checked.as_ref(), files.get(file))
        {
            // A bug of the tool there leaves the view without its block, and says so; never the session without its drawing.
            let work = || labcraft_diagram::own_block(checked, index, &loaded.names, &DiagramLimits::default());
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
                Ok(block) => drawing.block = block.map(Box::new),
                Err(_) => {
                    let message = format!("internal error, which is a bug of the tool: {}", crate::panics::last_panic());
                    added.push(labcraft_types::Diagnostic::new(Code::Lc0099, message, function.shown.as_str()));
                }
            }
        }
        let drawn = drawing.is_some();
        let picture = if is_empty(&added) {
            drawings::picture(function, drawing, &loaded.report)
        } else {
            let mut report = loaded.report.clone();
            report.merge(added.clone());
            drawings::picture(function, drawing, &report)
        };
        let picture = Arc::new(picture);
        Some((picture, drawn, entry_of(name, file, drawn, nodes, &codes), added))
    })
}

/// Draws the function of each window.
fn draw_all(view: &mut View, glide: bool) {
    for which in view.all() {
        draw_one(view, &which, glide);
    }
    compose_report(view);
}

/// Draws the function of a window; returns its entry of `diagrams` in `labcraft-diagram/1`.
fn draw(view: &mut View, which: &Which, glide: bool) -> Value {
    let entry = draw_one(view, which, glide);
    compose_report(view);
    entry
}

/// [`draw`], without the report: the caller composes it once the windows are drawn.
fn draw_one(view: &mut View, which: &Which, glide: bool) -> Value {
    on_window(view, which, |view, window| draw_in(view, window, glide)).unwrap_or(Value::Null)
}

fn draw_in(view: &mut View, window: &mut Window, glide: bool) -> Value {
    view.publish();
    window.shown.drawing_generation = view.count();
    window.shown.glide = glide;
    let before = window.shown.drawing.clone();
    // The values of the drawing before are not those of this one: they come once its trace is read.
    window.traced.due = true;
    window.drawn = false;
    window.traced.kept = None;
    window.shown.run = None;
    window.trace_report = None;
    if window.shown.overlay.take().is_some() {
        window.shown.overlay_generation = view.count();
    }
    let rendered = match (window.shown.function.clone(), window.file.clone()) {
        (Some(name), Some(file)) if !view.skipped.contains(&name) => {
            if let Some(before) = view.before_drawing {
                before(&name);
            }
            render(view, &name, &file).map(|rendered| (file, rendered))
        }
        _ => None,
    };
    let Some((file, (picture, drawn, entry, added))) = rendered else {
        keep_lines(&mut window.shown.selection, before.as_deref(), None);
        window.shown.drawing = None;
        window.shown.kept = false;
        window.shown.trace = None;
        window.drawing_report = None;
        window.entry = Value::Null;
        return Value::Null;
    };
    window.drawn = drawn;
    let shown = if drawn {
        view.last.insert(file.clone(), picture.clone());
        window.shown.kept = false;
        picture
    } else if let Some(last) = view.last.get(&file) {
        window.shown.kept = true;
        last.clone()
    } else {
        window.shown.kept = false;
        picture
    };
    keep_lines(&mut window.shown.selection, before.as_deref(), Some(&shown));
    window.shown.drawing = Some(shown);
    // The warnings of the drawing (`LC7005`) have no mark: the problems of the viewer list them.
    window.drawing_report = Some(added);
    window.shown.arms = view.views.get(&file).map(|of| of.arms.clone()).unwrap_or_default();
    window.entry = entry.clone();
    entry
}

/// A node named by its line (`@N`, commands.md 9.2) stays in the selection while the same node, known by its key, starts
/// on that line in the new drawing; otherwise its line names another node now, or none, and it leaves the selection with
/// the wires that reach it (decision of the owner, 2026-11-17).
fn keep_lines(selection: &mut window::Selection, before: Option<&Drawing>, after: Option<&Drawing>) {
    // `@N` is the node that starts on line N, whatever its name in the drawing.
    let key_of = |drawing: Option<&Drawing>, name: &str| {
        let line = name.strip_prefix('@')?.parse::<u32>().ok()?;
        drawing?.placed.iter().find(|placed| placed.line == line).map(|placed| placed.key.clone())
    };
    let stays = |name: &str| {
        let key = key_of(before, name);
        key.is_some() && key == key_of(after, name)
    };
    selection.nodes.retain(|name| !name.starts_with('@') || stays(name));
    selection.wires.retain(|wire| {
        wire.split(|letter: char| !(letter.is_alphanumeric() || letter == '@' || letter == '_')).filter(|piece| piece.starts_with('@')).all(stays)
    });
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

/// The inputs of `function` with no default, as `check` declared them; none when it did not declare it.
fn needs(loaded: &Loaded, files: &HashMap<&str, usize>, function: &Function) -> Vec<String> {
    let Some(checked) = loaded.checked.as_ref() else { return Vec::new() };
    let library =
        files.get(function.shown.as_str()).and_then(|index| checked.files.get(*index)).and_then(|file| checked.libraries.get(file.library as usize));
    let declared = function.name.rsplit('.').next().and_then(|name| loaded.names.find(name)).and_then(|name| library?.functions.get(&name));
    declared
        .map(|declared| declared.params.iter().filter(|param| !param.has_default).map(|param| loaded.names.resolve(param.name).to_owned()).collect())
        .unwrap_or_default()
}

/// The types of the files of the folder that the check read, by their file.
fn listed_types(loaded: &Loaded) -> Vec<ListedType> {
    let Some(checked) = &loaded.checked else { return Vec::new() };
    let mut found: Vec<ListedType> = checked
        .files
        .iter()
        .filter(|file| checked.libraries.get(file.library as usize).is_some_and(|library| library.checked))
        .filter_map(|file| match &file.file.decl {
            labcraft_graph::Decl::Type(declared) => {
                Some(ListedType { name: loaded.names.resolve(declared.name).to_owned(), file: file.shown.clone() })
            }
            _ => None,
        })
        .collect();
    found.sort_by(|a, b| a.file.cmp(&b.file));
    found
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
            Listed {
                function: function.name.clone(),
                file: function.shown.clone(),
                drawable,
                errors: counts.errors,
                warnings: counts.warnings,
                needs: needs(loaded, &files, function),
            }
        })
        .collect()
}
