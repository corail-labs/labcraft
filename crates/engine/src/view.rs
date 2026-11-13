//! The state of the viewer (`docs/architecture.md`, 10.8 and 11.5; `docs/commands.md`, 6.5): the folder open, its
//! check, the function shown, the view state that the file does not hold, the preference of the standard view. The
//! `view.*` commands change it; the app reads what it publishes ([`Session::view`]), the drawings as `Arc<Drawing>`.
//!
//! It writes no file: the viewer is read-only (scenario 6, point 6). The files are read through the [`Workspace`] of
//! the session; the app watches them and calls [`Session::files_changed`], and a command of the session that writes
//! marks the view stale, so that the next `view.*` command reads the folder again (decisions of the owner, 2026-11-11).

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use labcraft_diagram::style::{self, Style};
use labcraft_diagram::{DiagramLimits, Drawing, ViewState};
use labcraft_types::{Code, Report};
use serde_json::{Map, Value, json};

use crate::drawings::{self, Function, Loaded};
use crate::error::CommandError;
use crate::params::Params;
use crate::paths::{self, Place, Wanted};
use crate::session::Session;

/// The view state of a function that its file does not hold (notation 2.1), kept for the session only, by its file,
/// across changes of the file, by the keys of its nodes (decided at the start of step 6).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewOf {
    /// The arm shown of each structure in tabs.
    pub arms: HashMap<String, usize>,
    pub unfolded: BTreeSet<String>,
    pub standard: BTreeSet<String>,
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
    pub arms: HashMap<String, usize>,
    pub standard_everywhere: bool,
}

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
    /// Called with the name of each function just before it is drawn: the worker of the web viewer tells its page, so
    /// that a trap while drawing is put down to that function (architecture 11.5).
    pub(crate) before_drawing: Option<fn(&str)>,
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
    // The first function, so that the window shows something.
    if let Some(first) = view.published.functions.first().map(|listed| listed.function.clone()) {
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
    let placed = read_state(state, &mut of)?;
    view.views.insert(file.clone(), of.clone());
    if view.function.as_deref() == Some(name) {
        // An arm shown places nothing: only a structure unfolded or a node in another view draws again.
        if placed {
            return Ok(draw(view, false));
        }
        view.published.arms = of.arms;
        view.publish();
        return Ok(view.entry.clone());
    }
    Ok(render(view, name, &file).map(|(_, _, entry, _)| entry).unwrap_or(Value::Null))
}

/// Reads `state` into `of`; whether it changes what placement depends on.
fn read_state(state: &Map<String, Value>, of: &mut ViewOf) -> Result<bool, CommandError> {
    let mut placed = false;
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
                    return Err(error.expecting(&["arms", "unfolded", "standard"]));
                }
            }
        }
    }
    Ok(placed)
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
        view.entry = Value::Null;
        return Value::Null;
    };
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
