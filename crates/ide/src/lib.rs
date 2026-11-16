//! L8: the window of LabCraft (`docs/ide.md`; `docs/architecture.md`, 12.7), written once for the desktop and the web.
//! The pieces of `kit-shell` (top bar, command palette, start page, inspector, status bar), the docking of `craft-ui`,
//! the menus of `craft-menu` and the keys of `craft-keymap`, the canvas of `editor` and the views of a tab. It talks
//! to the session of its project through [`Host`], and asks the app, through [`Outward`], for what only an app does:
//! opening a project, a dialog, a link.
//!
//! Step 1a: read-only. The gestures that edit come with the steps after (architecture 12.11).
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

use std::collections::HashMap;
use std::sync::Arc;

use craft_keymap::{Active, CommandRef, Keymap, Platform};
use craft_menu::{ItemState, LayoutOptions, MenuBar, Target};
use craft_ui::docking::{Action, DockArea, DockContent, DockStyle, PanelState, Workspace};
use egui::{FontId, RichText};
use kit_shell::palette::{self, Chosen, Palette};
use kit_shell::start::{self, Example, Recent};
use kit_shell::status::Segment;
use kit_shell::top_bar::{self, Answer, Bar};
use kit_shell::workspaces::Asked as WorkspaceAsked;
use kit_shell::{Card, Planned};
use labcraft_editor::{Canvas, Shown, Theme};
use labcraft_engine::view::{
    Published, Tab, ViewKind,
    window::{file_of_tab, tab_of},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub mod catalog;
pub mod host;
pub mod keys;
pub mod pane;
pub mod panels;
pub mod text;

pub use host::{Host, Reply, Snapshot};
pub use pane::Pane;

/// The settings of the user that the window keeps (ide.md 1: the layout is the user's), which the app stores.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Where the panels are, without the tabs of functions; none, the layout of ide.md 2.
    pub layout: Option<Workspace<Pane>>,
    pub dark: bool,
    /// Every node in standard view (notation 3).
    pub standard_everywhere: bool,
    /// No glide (notation 2.1).
    pub reduced_motion: bool,
    /// A single click on a file opens it as a preview (ide.md 3), on by default.
    pub preview_tabs: bool,
    /// The values of the last run on the wires (notation 6.1).
    pub values_shown: bool,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs { layout: None, dark: false, standard_everywhere: false, reduced_motion: false, preview_tabs: true, values_shown: true }
    }
}

/// What the window asks the app for.
#[derive(Clone, Debug, PartialEq)]
pub enum Outward {
    /// `project.new`: make the project in the folder given, or one asked for, and open it.
    NewProject(Option<String>),
    /// `project.open`: open the folder given, or one asked for.
    OpenFolder(Option<String>),
    OpenRecent(usize),
    /// Copy this example into a folder the user chooses, and open the copy (ide.md 10).
    OpenExample(usize),
    /// Open this address in the browser.
    OpenLink(String),
    /// Ask for a trace file, anywhere on the disk, and show its values over the function shown.
    OpenTrace,
}

/// What the start page lists, given by the app.
#[derive(Clone, Debug, Default)]
pub struct StartList {
    pub recent: Vec<Recent>,
    pub examples: Vec<Example>,
    /// A project that did not open, said on the page.
    pub problem: Option<String>,
}

/// What the history says, for the top bar and the status bar.
#[derive(Default)]
struct History {
    undo: Option<String>,
    redo: Option<String>,
    last: Option<String>,
    generation: Option<u64>,
    asked: Option<Reply>,
}

/// The canvas of each tab of a function, and what it last took from the session.
struct Drawn {
    canvas: Canvas,
    /// The canvas was fitted when its first drawing came, before its values: fit it again with their banner.
    fit_values: bool,
}

/// The window.
pub struct Ide {
    prefs: Prefs,
    prefs_changed: bool,
    workspace: Workspace<Pane>,
    platform: Platform,
    keymap: Keymap,
    dispatcher: craft_keymap::egui::Dispatcher,
    palette: Palette,
    card: Card,
    frame: craft_window::Frame,
    /// The `movable` flag last given to AppKit (`craft_window::title_bar::sync_movable`).
    #[cfg(target_os = "macos")]
    movable: Option<bool>,
    /// The project the per-project state below belongs to.
    project: Option<String>,
    drawn: HashMap<String, Drawn>,
    texts: HashMap<String, text::Kept>,
    callables: panels::Callables,
    problems: panels::ProblemsFilter,
    drawing_generation: u64,
    overlay_generation: u64,
    /// The function asked for by a click on its tab, until the session shows it.
    asking_show: Option<(String, Reply)>,
    /// The function shown that the dock last brought to the front.
    brought: Option<String>,
    history: History,
    /// Commands whose failure the window says.
    pending: Vec<(String, Reply)>,
    failures: Vec<String>,
    zoom_given: Option<f32>,
    /// Where the canvas of the function shown was painted last.
    canvas_rect: Option<egui::Rect>,
}

/// The height of the top bar, the title bar of the window (ide.md 2), in points.
const TOP_BAR: f32 = 34.0;

impl Ide {
    /// The window, with the settings of the user. `frame`: who draws the frame of the window (`craft-window`).
    pub fn new(ctx: &egui::Context, prefs: Prefs, frame: craft_window::Frame) -> Ide {
        let platform = craft_keymap::egui::platform(ctx);
        let workspace = pane::restored(prefs.layout.clone());
        ctx.set_theme(if prefs.dark { egui::Theme::Dark } else { egui::Theme::Light });
        Ide {
            prefs,
            prefs_changed: false,
            workspace,
            platform,
            keymap: keys::keymap(platform),
            dispatcher: craft_keymap::egui::Dispatcher::new(),
            palette: Palette::default(),
            card: Card::default(),
            frame,
            #[cfg(target_os = "macos")]
            movable: None,
            project: None,
            drawn: HashMap::new(),
            texts: HashMap::new(),
            callables: panels::Callables::default(),
            problems: panels::ProblemsFilter::default(),
            drawing_generation: 0,
            overlay_generation: 0,
            asking_show: None,
            brought: None,
            history: History::default(),
            pending: Vec::new(),
            failures: Vec::new(),
            zoom_given: None,
            canvas_rect: None,
        }
    }

    /// The canvas of the function shown, and where it was painted last: for the measure of the frames of the app.
    pub fn shown_canvas(&mut self, function: &str) -> Option<(&mut Canvas, Option<egui::Rect>)> {
        let rect = self.canvas_rect;
        self.drawn.get_mut(function).map(|drawn| (&mut drawn.canvas, rect))
    }

    /// The settings of the user, as the window holds them now.
    pub fn prefs(&self) -> Prefs {
        Prefs { layout: Some(pane::to_save(&self.workspace)), ..self.prefs.clone() }
    }

    /// Whether the settings changed since the last call: the app writes them.
    pub fn take_prefs_changed(&mut self) -> bool {
        std::mem::take(&mut self.prefs_changed)
    }

    /// Says a problem that the app met, as a failure of a command is said.
    pub fn say(&mut self, problem: impl Into<String>) {
        self.failures.push(problem.into());
        let excess = self.failures.len().saturating_sub(3);
        self.failures.drain(..excess);
    }

    fn theme(&self) -> Theme {
        if self.prefs.dark { Theme::Dark } else { Theme::Light }
    }

    /// Draws the window in `ui`, the whole of the viewport: with no host, the start page alone.
    pub fn show(&mut self, ui: &mut egui::Ui, host: Option<&dyn Host>, start: &StartList) -> Vec<Outward> {
        let mut outward = Vec::new();
        let theme = if self.prefs.dark { egui::Theme::Dark } else { egui::Theme::Light };
        if ui.ctx().theme() != theme {
            ui.ctx().set_theme(theme);
        }
        // The platform of the keys can change after the first frame, on the web.
        let platform = craft_keymap::egui::platform(ui.ctx());
        if platform != self.platform {
            self.platform = platform;
            self.keymap = keys::keymap(platform);
        }
        let snapshot = host.map(Host::snapshot).unwrap_or_default();
        self.new_project(&snapshot);
        let view = snapshot.view.clone();
        if let Some(host) = host {
            self.take_replies(host, &view);
            self.sync_tabs(&view);
            self.take_drawing(&view);
            self.callables.refresh(host, view.checks);
        }
        // The keys first, so that a key the keymap takes reaches no widget.
        let typing = ui.ctx().egui_wants_keyboard_input();
        let hits = ui.ctx().input_mut(|input| self.dispatcher.dispatch(input, &self.keymap, &Active { panel: None, contexts: &[], typing }));
        for hit in hits {
            self.run(hit.command, host, &view, &mut outward);
        }
        let menus = self.menus(&view, host.is_some());
        let entries = if self.palette.is_open() { catalog::palette_entries(&self.keymap, host.is_some()) } else { Vec::new() };
        let workspaces = catalog::workspaces();
        let palette_key = keys::key_of(&self.keymap, &CommandRef::new(keys::PALETTE));
        let right: Vec<Segment> = host.iter().map(|_| Segment::new("No agent").hover("Connecting an AI agent comes with step 8 of M1")).collect();
        // When the app draws the frame (Windows): the edges resize the window, the top bar is its title bar.
        kit_shell::frame::edges(ui.ctx(), &self.frame);
        let answers = egui::Panel::top("ide-top")
            .exact_size(TOP_BAR)
            .show(ui, |ui| {
                top_bar::show(
                    ui,
                    &Bar {
                        name: "LabCraft",
                        menus: &menus,
                        undo: (self.history.undo.is_some(), self.history.undo.clone()),
                        redo: (self.history.redo.is_some(), self.history.redo.clone()),
                        workspaces: &workspaces,
                        workspace: "edit",
                        project: snapshot.project.as_deref(),
                        palette_key: palette_key.as_deref(),
                        right: &right,
                        frame: &self.frame,
                    },
                )
            })
            .inner;
        for answer in answers {
            match answer {
                Answer::Home => self.run(CommandRef::new(keys::HOME), host, &view, &mut outward),
                Answer::Menu(command) => self.run(command, host, &view, &mut outward),
                Answer::Undo => self.run(CommandRef::new("history.undo"), host, &view, &mut outward),
                Answer::Redo => self.run(CommandRef::new("history.redo"), host, &view, &mut outward),
                Answer::Workspace(WorkspaceAsked::Choose(id)) => {
                    self.run(CommandRef::with_params("view.set_workspace", json!({ "workspace": id })), host, &view, &mut outward);
                }
                Answer::Workspace(WorkspaceAsked::Planned(planned)) => self.card.open(planned),
                Answer::OpenPalette => self.palette.open(),
                Answer::Segment(command) => self.run(CommandRef::new(command), host, &view, &mut outward),
            }
        }
        let notices: Vec<String> = snapshot.notices.iter().chain(self.failures.iter()).cloned().collect();
        if !notices.is_empty() {
            let mut dismissed = false;
            egui::Panel::top("ide-notices").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        for notice in &notices {
                            ui.colored_label(ui.visuals().warn_fg_color, notice);
                        }
                    });
                    if !self.failures.is_empty() && kit_shell::icon::button(ui, kit_shell::icon::Icon::Close, true, "Dismiss").clicked() {
                        dismissed = true;
                    }
                });
            });
            if dismissed {
                self.failures.clear();
            }
        }
        let status = self.status(&view);
        if let Some(command) = egui::Panel::bottom("ide-status").show(ui, |ui| kit_shell::status::show(ui, &status.0, &status.1)).inner {
            self.run(CommandRef::new(command), host, &view, &mut outward);
        }
        match host {
            None => {
                egui::CentralPanel::default().show(ui, |ui| {
                    if let Some(chosen) = self.start_page(ui, start) {
                        self.chose_on_start(chosen, None, &view, &mut outward);
                    }
                });
            }
            Some(host) => self.dock(ui, host, &view, start, &mut outward),
        }
        if let Some(chosen) = self.palette.show(ui.ctx(), &entries) {
            match chosen {
                Chosen::Run { id, params } => self.run(CommandRef::with_params(id, params), Some(host).flatten(), &view, &mut outward),
                Chosen::Planned(planned) => self.card.open(planned),
            }
        }
        if let Some(link) = self.card.show(ui.ctx()) {
            outward.push(Outward::OpenLink(link));
        }
        // The content reaches under the title bar of macOS: a drag that starts on a control of the top bar must not
        // move the window.
        #[cfg(target_os = "macos")]
        craft_window::title_bar::sync_movable(ui.ctx(), &mut self.movable);
        outward
    }

    /// A new project: what the window kept of the one before goes.
    fn new_project(&mut self, snapshot: &Snapshot) {
        // A new session counts from 0 again, even for the same folder.
        let again = snapshot.view.drawing_generation < self.drawing_generation || snapshot.view.overlay_generation < self.overlay_generation;
        if self.project == snapshot.project && !again {
            return;
        }
        self.project.clone_from(&snapshot.project);
        self.drawn.clear();
        self.texts.clear();
        self.callables = panels::Callables::default();
        self.drawing_generation = 0;
        self.overlay_generation = 0;
        self.asking_show = None;
        self.brought = None;
        self.history = History::default();
        self.zoom_given = None;
    }

    /// The results that came: the failures to say, the function shown, the history.
    fn take_replies(&mut self, host: &dyn Host, view: &Published) {
        let mut failures = Vec::new();
        self.pending.retain(|(id, reply)| match reply.take() {
            None => true,
            Some(Ok(_)) => false,
            Some(Err(error)) => {
                failures.push(format!("{id}: {}", error.message));
                false
            }
        });
        for failure in failures {
            self.say(failure);
        }
        if let Some((function, reply)) = &self.asking_show
            && (reply.take().is_some() || view.function.as_deref() == Some(function.as_str()))
        {
            self.asking_show = None;
        }
        // The history, after each change of what is published.
        if let Some(result) = self.history.asked.as_ref().and_then(Reply::take) {
            self.history.asked = None;
            if let Ok(value) = result {
                let step = value.get("steps").and_then(Value::as_array).and_then(|steps| steps.first());
                let text = |value: Option<&Value>, key: &str| value.and_then(|value| value.get(key)).and_then(Value::as_str).map(str::to_owned);
                let label = text(step, "label");
                let author = text(step, "author");
                self.history.last = label.clone().map(|label| match &author {
                    Some(author) => format!("{label} · {author}"),
                    None => label,
                });
                self.history.undo =
                    value.get("undo").filter(|undo| !undo.is_null()).map(|_| label.clone().unwrap_or_else(|| "the last step".to_owned()));
                self.history.redo = value.get("redo").filter(|redo| !redo.is_null()).map(|_| "the step undone".to_owned());
            }
        }
        if self.history.asked.is_none() && self.history.generation != Some(view.generation) {
            self.history.generation = Some(view.generation);
            self.history.asked = Some(host.send("history.list", json!({ "limit": 1 })));
        }
    }

    /// The tabs of the dock follow the tabs of the session: a function opened has its tab, a tab closed goes, and the
    /// function shown is in front.
    fn sync_tabs(&mut self, view: &Published) {
        for function in &view.opened {
            let pane = Pane::Function(function.clone());
            if self.workspace.layout.contains(&pane) {
                continue;
            }
            let beside = self.workspace.layout.panels().into_iter().find(|other| other.function().is_some()).cloned();
            let start = self.workspace.layout.contains(&Pane::Start).then_some(Pane::Start);
            let anchor = beside.clone().or(start);
            let _ = self.workspace.apply(Action::Open { panel: pane, anchor: anchor.clone() }, pane::permissions);
            // The start page gives way to the first function opened in its place.
            if beside.is_none() && anchor == Some(Pane::Start) {
                let _ = self.workspace.apply(Action::Close { panel: Pane::Start }, pane::permissions);
            }
        }
        let gone: Vec<Pane> = self
            .workspace
            .layout
            .panels()
            .into_iter()
            .filter(|pane| pane.function().is_some_and(|function| !view.opened.iter().any(|opened| opened == function)))
            .cloned()
            .collect();
        for pane in gone {
            if let Some(function) = pane.function() {
                self.drawn.remove(function);
            }
            let _ = self.workspace.apply(Action::Close { panel: pane }, pane::permissions);
        }
        self.workspace.hidden.retain(|(pane, _)| pane.function().is_none());
        // The function shown comes to the front once, when the session shows it: after that, the start page or a panel
        // the user brings in front of it stays there.
        if self.asking_show.is_none() && self.brought != view.function {
            self.brought.clone_from(&view.function);
            if let Some(function) = &view.function {
                let pane = Pane::Function(function.clone());
                if self.workspace.layout.panel_state(&pane) == PanelState::Open {
                    let _ = self.workspace.apply(Action::Activate { panel: pane }, pane::permissions);
                }
            }
        }
    }

    /// The drawing and the values of the function shown, to its canvas.
    fn take_drawing(&mut self, view: &Published) {
        let Some(function) = &view.function else { return };
        let reduced = self.prefs.reduced_motion;
        let values_shown = self.prefs.values_shown;
        let drawn = self.drawn.entry(function.clone()).or_insert_with(|| {
            let mut canvas = Canvas::new();
            canvas.set_reduced_motion(reduced);
            canvas.set_values_shown(values_shown);
            Drawn { canvas, fit_values: false }
        });
        if view.drawing_generation != self.drawing_generation {
            self.drawing_generation = view.drawing_generation;
            if let Some(drawing) = &view.drawing
                && !drawn.canvas.drawing().is_some_and(|before| Arc::ptr_eq(before, drawing))
            {
                let first = drawn.canvas.drawing().is_none();
                // The glide when the file of the function changed; the first drawing of a tab is fitted.
                drawn.canvas.set_drawing(drawing.clone(), view.glide && !first);
                if first {
                    drawn.canvas.fit();
                    drawn.fit_values = true;
                }
            }
        }
        if view.overlay_generation != self.overlay_generation {
            self.overlay_generation = view.overlay_generation;
            drawn.canvas.set_values(view.overlay.clone(), view.run.clone());
            if std::mem::take(&mut drawn.fit_values) && view.overlay.as_ref().is_some_and(|overlay| overlay.top > 0) {
                drawn.canvas.fit();
            }
        }
    }

    /// The menus, laid out for the platform, with their keys and their states.
    fn menus(&self, view: &Published, project: bool) -> MenuBar {
        let mut bar = craft_menu::layout(&keys::menus(), Target::current(), &LayoutOptions::new("LabCraft")).bar;
        craft_menu::apply_keymap(&mut bar, &self.keymap);
        let shown = view.function.as_deref();
        let tab = shown.map(|function| tab_of(view, function)).unwrap_or_default();
        craft_menu::apply_state(&mut bar, |command| {
            let id = command.id.as_str();
            let param = |name: &str| command.params.get(name).and_then(Value::as_str).map(str::to_owned);
            match id {
                "project.new" | "project.open" | keys::HOME | keys::PALETTE | keys::DARK | keys::REDUCED_MOTION | keys::ABOUT => {
                    let checked = match id {
                        keys::DARK => Some(self.prefs.dark),
                        keys::REDUCED_MOTION => Some(self.prefs.reduced_motion),
                        _ => None,
                    };
                    ItemState { enabled: true, checked }
                }
                keys::PLANNED_AGENT | keys::PLANNED_INSTRUMENTS => ItemState { enabled: true, checked: None },
                "view.toggle_panel" => {
                    let visible = param("panel").and_then(|name| Pane::panel(&name)).is_some_and(|pane| self.workspace.layout.contains(&pane));
                    ItemState { enabled: project, checked: Some(visible) }
                }
                "view.set_view" => {
                    let asked = (param("view"), param("split"));
                    let now = (Some(tab.view.as_str().to_owned()), tab.split.map(|split| split.as_str().to_owned()));
                    ItemState { enabled: shown.is_some(), checked: Some(asked == now) }
                }
                "view.set_standard_view" => ItemState { enabled: project, checked: Some(view.standard_everywhere) },
                keys::VALUES => ItemState { enabled: project, checked: Some(self.prefs.values_shown) },
                "history.undo" => ItemState { enabled: self.history.undo.is_some(), checked: None },
                "history.redo" => ItemState { enabled: self.history.redo.is_some(), checked: None },
                "view.close_function" | keys::FIT | "view.open_trace" | keys::OPEN_TRACE => ItemState { enabled: shown.is_some(), checked: None },
                _ => ItemState { enabled: project, checked: None },
            }
        });
        bar
    }

    /// The status bar: the file shown, the last step; the agent, the zoom.
    fn status(&self, view: &Published) -> (Vec<Segment>, Vec<Segment>) {
        let mut left = Vec::new();
        if let Some(file) = view.function.as_deref().and_then(|function| file_of_tab(view, function)) {
            left.push(Segment::new(format!("{file} · saved")).hover("The window writes every change to the file at once"));
        }
        if let Some(last) = &self.history.last {
            left.push(Segment::new(last.clone()).hover("The last step of the history"));
        }
        let mut right = Vec::new();
        if let Some(drawn) = view.function.as_ref().and_then(|function| self.drawn.get(function)) {
            let (zoom, _) = drawn.canvas.viewport();
            right.push(Segment::new(format!("{:.0} %", zoom * 100.0)).hover("Fit the diagram to the view").command(keys::FIT));
        }
        (left, right)
    }

    fn start_page(&self, ui: &mut egui::Ui, start: &StartList) -> Option<start::Chosen> {
        let actions = start_actions(&self.keymap);
        start::show(
            ui,
            &start::Page {
                title: "LabCraft",
                line: "A graphical dataflow language, open and cross-platform",
                actions: &actions,
                recent: &start.recent,
                examples: &start.examples,
                problem: start.problem.as_deref(),
            },
        )
    }

    fn chose_on_start(&mut self, chosen: start::Chosen, host: Option<&dyn Host>, view: &Published, outward: &mut Vec<Outward>) {
        match chosen {
            start::Chosen::Action(at) => {
                if let Some(action) = start_actions(&self.keymap).get(at) {
                    match &action.state {
                        palette::State::Planned(planned) => self.card.open(planned.clone()),
                        _ => self.run(CommandRef::new(action.id.clone()), host, view, outward),
                    }
                }
            }
            start::Chosen::Recent(at) => outward.push(Outward::OpenRecent(at)),
            start::Chosen::Example(at) => outward.push(Outward::OpenExample(at)),
        }
    }

    /// The dock: the panels and the tabs of the functions.
    fn dock(&mut self, ui: &mut egui::Ui, host: &dyn Host, view: &Arc<Published>, start: &StartList, outward: &mut Vec<Outward>) {
        let style = DockStyle::from_ui(ui);
        let mut content = Content { ide: self, host, view, start, asked: Vec::new(), chosen: None };
        let output = egui::CentralPanel::default()
            .show(ui, |ui| DockArea::new(egui::Id::unique("ide-dock")).show_content(ui, &content.ide.workspace.layout.clone(), &style, &mut content))
            .inner;
        let Content { asked, chosen, .. } = content;
        for action in output.actions {
            match &action {
                Action::Close { panel: Pane::Function(function) } => {
                    let reply = host.send("view.close_function", json!({ "function": function }));
                    self.pending.push(("view.close_function".to_owned(), reply));
                }
                Action::Activate { panel: Pane::Function(function) } if view.function.as_deref() != Some(function.as_str()) => {
                    // A click on the title of a tab in preview leaves it in preview (ide.md 3): an edit or a double click keeps it.
                    let params = if view.preview.as_deref() == Some(function.as_str()) {
                        json!({ "function": function, "preview": true })
                    } else {
                        json!({ "function": function })
                    };
                    let reply = host.send("view.show_function", params);
                    self.asking_show = Some((function.clone(), reply));
                }
                _ => {}
            }
            if self.workspace.apply(action, pane::permissions).is_ok() {
                self.prefs_changed = true;
            }
        }
        for asked in asked {
            self.panel_asked(asked, Some(host), view, outward);
        }
        if let Some(chosen) = chosen {
            self.chose_on_start(chosen, Some(host), view, outward);
        }
        // The zoom of the canvas shown, which `view.get_state` returns.
        if let Some(drawn) = view.function.as_ref().and_then(|function| self.drawn.get(function)) {
            let (zoom, _) = drawn.canvas.viewport();
            if self.zoom_given != Some(zoom) {
                self.zoom_given = Some(zoom);
                host.set_zoom(zoom);
            }
        }
    }

    fn panel_asked(&mut self, asked: panels::Asked, host: Option<&dyn Host>, view: &Published, outward: &mut Vec<Outward>) {
        match asked {
            panels::Asked::Show { function, preview } => {
                // Asked again, it comes to the front again, even when it is the function shown.
                self.brought = None;
                let params = if preview { json!({ "function": function, "preview": true }) } else { json!({ "function": function }) };
                self.run(CommandRef::with_params("view.show_function", params), host, view, outward);
            }
            panels::Asked::Command(id, params) => self.run(CommandRef::with_params(id, params), host, view, outward),
            panels::Asked::Window(id) => self.run(CommandRef::new(id), host, view, outward),
        }
    }

    /// Runs a command of a menu, a key, the palette, a button: an action of the window here, a command of the session
    /// through the host.
    fn run(&mut self, command: CommandRef, host: Option<&dyn Host>, view: &Published, outward: &mut Vec<Outward>) {
        let CommandRef { id, params } = command;
        let mut params = match params {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        params.retain(|_, value| !value.is_null());
        let shown = view.function.clone();
        match id.as_str() {
            "project.new" | "project.open" => {
                let folder = params.get("folder").and_then(Value::as_str).filter(|folder| !folder.is_empty()).map(str::to_owned);
                outward.push(if id == "project.new" { Outward::NewProject(folder) } else { Outward::OpenFolder(folder) });
            }
            keys::PALETTE => self.palette.open(),
            keys::HOME => {
                if host.is_some() {
                    let anchor = self.workspace.layout.panels().into_iter().find(|pane| pane.function().is_some()).cloned();
                    if !self.workspace.layout.contains(&Pane::Start) {
                        let _ = self.workspace.apply(Action::Open { panel: Pane::Start, anchor }, pane::permissions);
                    }
                    let _ = self.workspace.apply(Action::Activate { panel: Pane::Start }, pane::permissions);
                }
            }
            keys::OPEN_TRACE => outward.push(Outward::OpenTrace),
            keys::ABOUT => self.card.open(Planned {
                label: format!("LabCraft {}", env!("CARGO_PKG_VERSION")),
                line: "A graphical dataflow language and IDE, in Rust. MIT OR Apache-2.0.".to_owned(),
                milestone: "M1".to_owned(),
                link: Some(catalog::ROADMAP.to_owned()),
            }),
            keys::DARK => {
                self.prefs.dark = !self.prefs.dark;
                self.prefs_changed = true;
            }
            keys::REDUCED_MOTION => {
                self.prefs.reduced_motion = !self.prefs.reduced_motion;
                self.prefs_changed = true;
                for drawn in self.drawn.values_mut() {
                    drawn.canvas.set_reduced_motion(self.prefs.reduced_motion);
                }
            }
            keys::VALUES => {
                self.prefs.values_shown = !self.prefs.values_shown;
                self.prefs_changed = true;
                for drawn in self.drawn.values_mut() {
                    drawn.canvas.set_values_shown(self.prefs.values_shown);
                }
            }
            keys::FIT => {
                if let Some(drawn) = shown.as_ref().and_then(|function| self.drawn.get_mut(function)) {
                    drawn.canvas.fit();
                }
            }
            id if id.starts_with("ide.planned.") => {
                if let Some(planned) = catalog::planned(id) {
                    self.card.open(planned);
                }
            }
            // The layout is the user's: the window keeps it, the session never sees it.
            "view.toggle_panel" => {
                let Some(pane) = params.get("panel").and_then(Value::as_str).and_then(Pane::panel) else {
                    self.say("view.toggle_panel: `panel` is one of project, palette, inspector, problems, run");
                    return;
                };
                let visible = self.workspace.layout.contains(&pane);
                let on = params.get("on").and_then(Value::as_bool).unwrap_or(!visible);
                if on != visible && self.workspace.set_visible(pane, on, None, pane::permissions).is_ok() {
                    self.prefs_changed = true;
                }
            }
            "view.reset_layout" => {
                self.workspace.reset(&pane::default_layout());
                self.workspace.saved = None;
                self.prefs_changed = true;
            }
            _ => {
                let Some(host) = host else {
                    self.say(format!("{id}: open a project first"));
                    return;
                };
                // What the window fills in: the function shown, and the other state of a toggle.
                if matches!(id.as_str(), "view.close_function" | "view.set_view")
                    && !params.contains_key("function")
                    && let Some(function) = &shown
                {
                    params.insert("function".to_owned(), json!(function));
                }
                if id == "view.set_standard_view" {
                    let on = params.get("on").and_then(Value::as_bool).unwrap_or(!view.standard_everywhere);
                    params.insert("on".to_owned(), json!(on));
                    self.prefs.standard_everywhere = on;
                    self.prefs_changed = true;
                }
                let reply = host.send(&id, Value::Object(params));
                self.pending.push((id, reply));
            }
        }
    }
}

/// The actions of the start page (ide.md 10).
fn start_actions(keymap: &Keymap) -> Vec<palette::Entry> {
    let key = |id: &str| keys::key_of(keymap, &CommandRef::new(id));
    let entry = |id: &str, label: &str| palette::Entry { key: key(id), ..palette::Entry::new(id, label) };
    let planned = |id: &str| {
        catalog::planned(id)
            .map(|planned| palette::Entry { state: palette::State::Planned(planned.clone()), ..palette::Entry::new(id, planned.label) })
    };
    let mut actions = vec![entry("project.new", "New project…"), entry("project.open", "Open folder…")];
    actions.extend(planned("ide.planned.settings"));
    actions.extend(planned(keys::PLANNED_AGENT));
    actions
}

/// What the dock draws in each pane.
struct Content<'a> {
    ide: &'a mut Ide,
    host: &'a dyn Host,
    view: &'a Arc<Published>,
    start: &'a StartList,
    asked: Vec<panels::Asked>,
    chosen: Option<start::Chosen>,
}

impl DockContent<Pane> for Content<'_> {
    fn body(&mut self, ui: &mut egui::Ui, pane: &Pane) {
        let view = self.view;
        let asked = match pane {
            Pane::Project => panels::project(ui, view, self.ide.prefs.preview_tabs),
            Pane::Palette => {
                panels::palette(ui, &mut self.ide.callables);
                None
            }
            Pane::Inspector => {
                panels::inspector(ui, view, &self.ide.callables);
                None
            }
            Pane::Problems => panels::problems(ui, view, &mut self.ide.problems),
            Pane::Run => panels::run(ui, view, self.ide.prefs.values_shown),
            Pane::Start => {
                if let Some(chosen) = self.ide.start_page(ui, self.start) {
                    self.chosen = Some(chosen);
                }
                None
            }
            Pane::Function(function) => self.function(ui, function),
        };
        self.asked.extend(asked);
    }

    fn logical_label(&self, pane: &Pane) -> String {
        pane.title()
    }

    fn label(&mut self, _ui: &egui::Ui, pane: &Pane, logical: &str, _font: &FontId, _color: egui::Color32) -> egui::WidgetText {
        // A tab in preview has its title in italics (ide.md 3).
        match pane {
            Pane::Function(function) if self.view.preview.as_deref() == Some(function.as_str()) => RichText::new(logical).italics().into(),
            _ => logical.into(),
        }
    }

    fn permissions(&self, pane: &Pane) -> craft_ui::docking::Permissions {
        pane::permissions(pane)
    }
}

impl Content<'_> {
    /// The tab of a function: its views, and the view or the two of a split.
    fn function(&mut self, ui: &mut egui::Ui, function: &str) -> Option<panels::Asked> {
        let view = self.view;
        let shown = view.function.as_deref() == Some(function);
        let tab = tab_of(view, function);
        let mut asked = None;
        ui.horizontal(|ui| {
            for (kind, label) in [(ViewKind::Diagram, "Diagram"), (ViewKind::Text, "Text"), (ViewKind::Block, "Block")] {
                if ui.selectable_label(tab.view == kind && tab.split.is_none(), label).clicked() {
                    asked = Some(panels::Asked::Command("view.set_view".to_owned(), json!({ "function": function, "view": kind.as_str() })));
                }
            }
            let split = tab.split.is_some();
            if ui.selectable_label(split, "Text | Diagram").on_hover_text("The text beside the diagram").clicked() {
                let params = if split {
                    json!({ "function": function, "view": "diagram" })
                } else {
                    json!({ "function": function, "view": "text", "split": "diagram" })
                };
                asked = Some(panels::Asked::Command("view.set_view".to_owned(), params));
            }
            if ui.selectable_label(false, RichText::new("Panel").weak()).on_hover_text("Planned, M3").clicked() {
                asked = Some(panels::Asked::Window("ide.planned.panel_view"));
            }
        });
        ui.separator();
        let file = file_of_tab(view, function).map(str::to_owned);
        match tab {
            Tab { view: first, split: Some(second) } => {
                let theme = self.ide.theme();
                ui.columns(2, |columns| {
                    if let [left, right] = columns {
                        self.one_view(left, function, first, shown, file.as_deref(), theme);
                        self.one_view(right, function, second, shown, file.as_deref(), theme);
                    }
                });
            }
            Tab { view: kind, split: None } => {
                let theme = self.ide.theme();
                self.one_view(ui, function, kind, shown, file.as_deref(), theme);
            }
        }
        if !shown && ui.ui_contains_pointer() && ui.input(|input| input.pointer.primary_pressed()) {
            asked = asked.or(Some(panels::Asked::Show { function: function.to_owned(), preview: false }));
        }
        asked
    }

    fn one_view(&mut self, ui: &mut egui::Ui, function: &str, kind: ViewKind, shown: bool, file: Option<&str>, theme: Theme) {
        match kind {
            ViewKind::Diagram => self.diagram(ui, function, shown, theme),
            ViewKind::Text => match file {
                Some(file) => self.text(ui, file, theme == Theme::Dark),
                None => {
                    ui.label(RichText::new("This function has no file").weak());
                }
            },
            ViewKind::Block => {
                ui.label(RichText::new("The view Block, the terminals and the icon of the function, comes with step 6 of M1.").weak());
            }
        }
    }

    fn diagram(&mut self, ui: &mut egui::Ui, function: &str, shown: bool, theme: Theme) {
        let view = self.view;
        let Some(drawn) = self.ide.drawn.get_mut(function) else {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Click to show this function").weak()));
            return;
        };
        if drawn.canvas.drawing().is_none() {
            let text = if view.kept || !shown { "This function is not drawn now: see Problems" } else { "Drawing…" };
            ui.centered_and_justified(|ui| ui.label(RichText::new(text).weak()));
            return;
        }
        let arms = if shown { view.arms.clone() } else { HashMap::new() };
        let response = drawn.canvas.show(ui, theme, &Shown { arms });
        if !shown {
            return;
        }
        self.ide.canvas_rect = Some(response.response.rect);
        if let Some((structure, arm)) = response.tab {
            self.asked
                .push(panels::Asked::Command("view.set_state".to_owned(), json!({ "function": function, "state": { "arms": { structure: arm } } })));
        }
        // An iteration chosen by a selector or on the values pinned of a wire (notation 6.1).
        if let Some((looped, iteration)) = response.iteration {
            self.asked.push(panels::Asked::Command(
                "view.set_state".to_owned(),
                json!({ "function": function, "state": { "iterations": { looped: iteration } } }),
            ));
        }
    }

    fn text(&mut self, ui: &mut egui::Ui, file: &str, dark: bool) {
        let kept = self.ide.texts.entry(file.to_owned()).or_insert_with(|| text::Kept::new(file));
        kept.refresh(self.host, self.view.checks);
        if let Some(failure) = &kept.failure {
            ui.label(RichText::new(failure).color(ui.visuals().warn_fg_color));
            return;
        }
        if kept.loading() && kept.text.is_empty() {
            ui.label(RichText::new("Reading…").weak());
            return;
        }
        let font = FontId::new(13.0, egui::FontFamily::Monospace);
        let galley = kept.galley(ui, dark, &font);
        let cut = kept.cut;
        egui::ScrollArea::both().id_salt(("ide-text", file)).auto_shrink([false, false]).show(ui, |ui| {
            // Laid out once: a label of the galley, which the user can select and copy, read only.
            ui.add(egui::Label::new(galley).selectable(true).extend());
            if cut {
                ui.label(RichText::new(format!("The rest of the file is not shown: past {} MiB", text::MAX_TEXT / 1024 / 1024)).weak());
            }
        });
    }
}
