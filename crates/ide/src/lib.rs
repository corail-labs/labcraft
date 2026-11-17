//! L8: the window of LabCraft (`docs/ide.md`; `docs/architecture.md`, 12.7), written once for the desktop and the web.
//! The pieces of `kit-shell` (top bar, command palette, start page, inspector, status bar), the docking of `craft-ui`,
//! the menus of `craft-menu` and the keys of `craft-keymap`, the canvas of `editor` and the views of a tab. It talks
//! to the session of its project through [`Host`], and asks the app, through [`Outward`], for what only an app does:
//! opening a project, a dialog, a link.
//!
//! The tab of a function can be detached into a window of its own (ide.md 9): a floating window of the dock, which is a
//! window of the system with the feature `native-windows` (the desktop) and a panel floating in the page without it (the
//! web). The session holds which functions are detached (`view.detach`, `view.dock`); the dock follows it, and a tab
//! dragged out or back asks it.
//!
//! Step 1: read-only. The gestures that edit come with the steps after (architecture 12.11).
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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use craft_keymap::{Active, CommandRef, Keymap, Platform};
use craft_menu::{ItemState, LayoutOptions, MenuBar, Target};
use craft_ui::docking::{
    Action, DockArea, DockContent, DockStyle, Floating, Monitor, PanelState, Permissions, Target as DockTarget, WindowKind, Workspace,
};
use egui::{FontId, RichText};
use kit_shell::icon::{self, Icon};
use kit_shell::palette::{self, Chosen, Palette};
use kit_shell::start::{self, Example, Recent};
use kit_shell::status::Segment;
use kit_shell::top_bar::{self, Answer, Bar, Tool};
use kit_shell::workspaces::Asked as WorkspaceAsked;
use kit_shell::{Card, Planned};
use labcraft_editor::{Canvas, Intent, Selection, Shown, Step, Theme};
use labcraft_engine::view::{
    Published, Shown as WindowShown, Tab, ViewKind,
    window::{file_of_tab, tab_of},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub mod catalog;
pub mod host;
mod insert;
pub mod keys;
pub mod pane;
pub mod panels;
mod running;
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
    /// The look of the window: `studio` (graphite, indigo), the default, or `lab` (slate, teal).
    pub look: String,
    /// Every node in standard view (notation 3).
    pub standard_everywhere: bool,
    /// No glide (notation 2.1).
    pub reduced_motion: bool,
    /// A single click on a file opens it as a preview (ide.md 3), on by default.
    pub preview_tabs: bool,
    /// The values of the last run on the wires (notation 6.1).
    pub values_shown: bool,
    /// Where the last detached window was: the next one opens there (decision of the owner, 2026-11-16).
    pub detached: Option<Place>,
    /// A detached window shows its inspector beside its canvas, and its palette of functions on the other side (decision
    /// of the owner, 2026-11-16: the inspector shown, the palette hidden, until the user changes them).
    pub detached_inspector: bool,
    pub detached_palette: bool,
    /// A run records its values, its trace, for the wires (decision of the owner, 2026-11-16: off by default).
    pub record_values: bool,
    /// The setting "drag on the empty canvas pans" (notation 2.6): off, a drag on the empty canvas selects with a
    /// rectangle (decision of the owner, 2026-11-17).
    pub drag_pans: bool,
    /// The setting "fix every node at the first move" (notation 2.4): a move fixes every free node of the function where
    /// it is, in the same step. Off by default.
    pub fix_every_node: bool,
    /// The setting "mark the fixed nodes" (notation 2.4): a pin on each fixed node. On by default.
    pub mark_fixed: bool,
    /// The setting "replace only with Alt" (notation 2.4): a wire dropped on an input that holds something replaces it
    /// only with Alt down. Off by default.
    #[serde(default)]
    pub replace_with_alt: bool,
    /// The setting "accept it" (notation 2.4): a wire of the wrong type is written, and `check` gives `LC3001`. Off by
    /// default.
    #[serde(default)]
    pub accept_mismatch: bool,
    /// The modifier of a wire moved by hand (notation 2.6): Shift+Alt and a drag on a segment instead of Alt. Off by
    /// default.
    #[serde(default)]
    pub route_with_shift_alt: bool,
    /// The settings "autopatch from the keyboard" and "inserting into a wire" (notation 2.6). On by default.
    pub autopatch: bool,
    pub into_wire: bool,
    /// The insertions of this user, the most recent first, which order quick insertion (notation 2.6): kept on their
    /// machine, with their settings.
    pub recent: Vec<String>,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            layout: None,
            dark: false,
            look: "studio".to_owned(),
            standard_everywhere: false,
            reduced_motion: false,
            preview_tabs: true,
            values_shown: true,
            detached: None,
            detached_inspector: true,
            detached_palette: false,
            record_values: false,
            drag_pans: false,
            fix_every_node: false,
            mark_fixed: true,
            replace_with_alt: false,
            accept_mismatch: false,
            route_with_shift_alt: false,
            autopatch: true,
            into_wire: true,
            recent: Vec::new(),
        }
    }
}

/// The place and the size of a detached window, as the dock keeps those of a floating window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Place {
    /// `[x, y, width, height]` in the window of the project.
    pub rect: [f32; 4],
    /// Its rect on the desktop while it is a window of the system.
    pub native: Option<[f32; 4]>,
    pub monitor: Option<Monitor>,
    pub monitor_size: Option<[f32; 2]>,
}

/// Where a detached window opens when no window was detached before.
const FIRST_PLACE: [f32; 4] = [120.0, 90.0, 960.0, 640.0];

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
    /// Ask for a trace file, anywhere on the disk, and show its values over the function of the window that has focus,
    /// named here; none for the window of the project.
    OpenTrace(Option<String>),
}

/// What the start page lists, given by the app.
#[derive(Clone, Debug, Default)]
pub struct StartList {
    pub recent: Vec<Recent>,
    pub examples: Vec<Example>,
    /// A project that did not open, said on the page.
    pub problem: Option<String>,
    /// The projects are held in memory, as on the web (ide.md 12): an example opens as it is, and none is recent.
    pub in_memory: bool,
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

/// A tab moved or closed by a gesture on the dock, asked of the session: the dock leaves it where the gesture put it
/// until the view the window reads shows it done, not only until the session answers, which can come before.
struct Placing {
    function: String,
    /// `view.detach`, `view.dock` or `view.close_function`.
    id: &'static str,
    reply: Reply,
    answered: bool,
}

impl Placing {
    /// Whether the view shows the request done.
    fn done(&self, view: &Published) -> bool {
        let opened = view.opened.contains(&self.function);
        let detached = view.is_detached(&self.function);
        match self.id {
            "view.detach" => detached,
            "view.dock" => opened && !detached,
            _ => !opened && !detached,
        }
    }
}

/// The canvas of each tab of a function, and what it last took from the session.
struct Drawn {
    canvas: Canvas,
    /// The canvas was fitted when its first drawing came, before its values: fit it again with their banner.
    fit_values: bool,
    drawing_generation: Option<u64>,
    overlay_generation: Option<u64>,
    /// The zoom last given to the session for its window.
    zoom_given: Option<f32>,
    /// Whether its tab showed two views side by side, and in a detached window, at the frame before: the canvas is fitted
    /// again when either changes, since another width or another window is another view of the diagram.
    split_seen: Option<(bool, bool)>,
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
    /// The detached function whose window the command palette opened in; none for the window of the project.
    palette_in: Option<String>,
    card: Card,
    frame: craft_window::Frame,
    /// The look given to egui last (`Prefs::look`).
    installed_look: String,
    /// The `movable` flag last given to AppKit (`craft_window::title_bar::sync_movable`).
    #[cfg(target_os = "macos")]
    movable: Option<bool>,
    /// The project the per-project state below belongs to, and the session of the app it came from.
    project: Option<String>,
    session: u64,
    /// The generation of the publication last read: a new session counts from 0 again.
    generation: u64,
    /// The project is in memory (`StartList::in_memory`): the status bar says so, not "saved".
    in_memory: bool,
    drawn: HashMap<String, Drawn>,
    texts: HashMap<String, text::Kept>,
    callables: panels::Callables,
    problems: panels::ProblemsFilter,
    /// The function asked for by a click on its tab, until the session shows it.
    asking_show: Option<(String, Reply)>,
    /// The function shown that the dock last brought to the front.
    brought: Option<String>,
    /// The detached function whose window has focus (decision of the owner, 2026-11-16): the keys, the command palette
    /// and the commands that take the function shown act on it. None for the window of the project.
    focus: Option<String>,
    /// Whether the window of the project had focus at the frame before: it gives it back to the window of the project.
    root_focused: bool,
    /// The detached functions asked of the session by a gesture on the dock, until it answers: the dock leaves them
    /// where the gesture put them.
    placing: Vec<Placing>,
    /// The detached window to bring to the front.
    raise: Option<String>,
    /// The detached windows drawn as windows of the system at the last frame.
    native: HashSet<String>,
    /// The place of each detached window at the frame before, by the id of its floating window: the one that moves is
    /// the place kept for the next one.
    places_seen: HashMap<u64, Place>,
    history: History,
    /// Commands whose failure the window says.
    pending: Vec<(String, Reply)>,
    /// The editing commands asked for, by the function they edit: a failure puts the nodes of a move back on its canvas,
    /// and a node that got a label stays selected by it (commands.md 9.3, `renamed`).
    /// The editing commands sent, by function, and the call to remember among the recent insertions once it is done.
    edits: Vec<(String, Reply, Option<String>)>,
    /// The command of a field in place, by function: its answer closes the field, or leaves it open with the refusal
    /// (notation 2.6).
    field_edits: Vec<(String, Reply)>,
    /// A `string.format` inserted, by function and name: its template field opens once the drawing holds it.
    templates: Vec<(String, String)>,
    failures: Vec<String>,
    /// Where the canvas of the function shown in the window of the project was painted last.
    canvas_rect: Option<egui::Rect>,
    /// The selections asked of the session (`view.select`), with what they asked: one refused for a name by its line is
    /// asked again without them (`@N`, commands.md 9.2, names nothing in a file that canonical form would reorder).
    selecting: Vec<(Selection, String, Reply)>,
    /// The function whose canvas had the keyboard, and the widget of that canvas: the keys of the canvas act on it while
    /// egui still gives it the focus (notation 2.6).
    canvas_focus: Option<(String, egui::Id)>,
    /// Where the pointer was last, on the screen: quick insertion opens under it (notation 2.6).
    pointer: Option<egui::Pos2>,
    /// The list of quick insertion, while it is open (notation 2.6).
    quick: Option<insert::QuickList>,
    /// The run the window started, followed to its end (decisions of the owner, 2026-11-16).
    running: Option<running::Running>,
    /// The time of the frame, on the clock of egui.
    now: f64,
    /// The loader of the SVG files of the icons is installed on the context that draws the window.
    loaders: bool,
    /// How the last run the window started of each function ended, for the `Run` panel.
    outcomes: HashMap<String, running::Outcome>,
}

/// The id of the dock area, which names its windows of the system.
fn dock_id() -> egui::Id {
    egui::Id::unique("ide-dock")
}

impl Ide {
    /// The window, with the settings of the user. `frame`: who draws the frame of the window (`craft-window`).
    pub fn new(ctx: &egui::Context, prefs: Prefs, frame: craft_window::Frame) -> Ide {
        let platform = craft_keymap::egui::platform(ctx);
        let workspace = pane::restored(prefs.layout.clone());
        ctx.set_theme(if prefs.dark { egui::Theme::Dark } else { egui::Theme::Light });
        kit_shell::theme::install(ctx, kit_shell::theme::Look::named(&prefs.look).unwrap_or_default());
        Ide {
            installed_look: prefs.look.clone(),
            prefs,
            prefs_changed: false,
            workspace,
            platform,
            keymap: keys::keymap(platform),
            dispatcher: craft_keymap::egui::Dispatcher::new(),
            palette: Palette::default(),
            palette_in: None,
            card: Card::default(),
            frame,
            #[cfg(target_os = "macos")]
            movable: None,
            project: None,
            session: 0,
            generation: 0,
            in_memory: false,
            drawn: HashMap::new(),
            texts: HashMap::new(),
            callables: panels::Callables::default(),
            problems: panels::ProblemsFilter::default(),
            asking_show: None,
            brought: None,
            focus: None,
            root_focused: true,
            placing: Vec::new(),
            raise: None,
            native: HashSet::new(),
            places_seen: HashMap::new(),
            history: History::default(),
            pending: Vec::new(),
            edits: Vec::new(),
            field_edits: Vec::new(),
            templates: Vec::new(),
            failures: Vec::new(),
            canvas_rect: None,
            canvas_focus: None,
            pointer: None,
            quick: None,
            selecting: Vec::new(),
            running: None,
            now: 0.0,
            loaders: false,
            outcomes: HashMap::new(),
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

    /// Whether the history of the project shown has nothing to undo or redo, as the window last read it: the web page
    /// asks before another project replaces one that changed (ide.md 12).
    pub fn history_is_empty(&self) -> bool {
        self.history.undo.is_none() && self.history.redo.is_none()
    }

    /// The detached function whose window has focus, if it still is detached: the window of the project otherwise.
    pub fn focused(&self, view: &Published) -> Option<String> {
        self.focus.clone().filter(|function| view.is_detached(function))
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

    /// The look of the preferences; a name it does not know is the default.
    fn look(&self) -> kit_shell::theme::Look {
        kit_shell::theme::Look::named(&self.prefs.look).unwrap_or_default()
    }

    /// What the window that has focus shows: its detached window, else the window of the project.
    fn target<'a>(&self, view: &'a Published) -> &'a WindowShown {
        self.focus
            .as_deref()
            .and_then(|function| view.detached.iter().find(|shown| shown.function.as_deref() == Some(function)))
            .unwrap_or(&view.main)
    }

    /// The commands of the keys pressed in the viewport of `ctx`: the window of the project, or a detached window of the
    /// system.
    fn key_hits(&mut self, ctx: &egui::Context) -> Vec<CommandRef> {
        // The canvas takes the keyboard, but is no text field: its keys are the window's (notation 2.6). Asked of egui each
        // frame, since another widget, a tab or a window can take the focus without the canvas being drawn again.
        if self.canvas_focus.as_ref().is_some_and(|(_, id)| !ctx.memory(|memory| memory.has_focus(*id))) {
            self.canvas_focus = None;
        }
        let typing = ctx.egui_wants_keyboard_input() && self.canvas_focus.is_none();
        let panel = self.canvas_focus.as_ref().map(|_| keys::DIAGRAM_SCOPE);
        let hits = ctx.input_mut(|input| self.dispatcher.dispatch(input, &self.keymap, &Active { panel, contexts: &[], typing }));
        hits.into_iter().map(|hit| hit.command).collect()
    }

    /// The permissions of each pane where it is now (`pane::permissions_in`).
    fn permissions(&self) -> impl Fn(&Pane) -> Permissions + use<> {
        let floating = pane::floating(&self.workspace.layout);
        move |pane: &Pane| pane::permissions_in(pane, floating.contains(pane))
    }

    /// Draws the window in `ui`, the whole of the viewport: with no host, the start page alone.
    pub fn show(&mut self, ui: &mut egui::Ui, host: Option<&dyn Host>, start: &StartList) -> Vec<Outward> {
        let mut outward = Vec::new();
        if self.installed_look != self.prefs.look {
            self.installed_look = self.prefs.look.clone();
            kit_shell::theme::install(ui.ctx(), self.look());
        }
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
        self.now = ui.input(|input| input.time);
        self.pointer = ui.ctx().pointer_latest_pos();
        if !std::mem::replace(&mut self.loaders, true) {
            kit_shell::icon::install(ui.ctx());
        }
        let snapshot = host.map(Host::snapshot).unwrap_or_default();
        self.new_project(&snapshot);
        let view = snapshot.view.clone();
        self.follow_focus(ui.ctx());
        if let Some(host) = host {
            self.take_replies(host, &view);
            self.follow_run(host, ui.ctx());
            self.sync_tabs(ui.ctx(), &view);
            self.take_drawing(&view);
            self.callables.refresh(host, view.checks);
        }
        // The keys first, so that a key the keymap takes reaches no widget. A run of Ctrl and the arrows ends before what
        // a key does (notation 2.6).
        // A press of the pointer ends it too: on the top bar, Undo would come before the move.
        let hits = self.key_hits(ui.ctx());
        if (!hits.is_empty() || ui.input(|input| input.pointer.any_pressed()))
            && let Some(host) = host
        {
            self.end_runs(host, &view);
        }
        for command in hits {
            self.run(command, host, &view, &mut outward);
        }
        let menus = self.menus(&view, host.is_some());
        let entries = if self.palette.is_open() { catalog::palette_entries(&self.keymap, host.is_some()) } else { Vec::new() };
        let workspaces = catalog::workspaces();
        let palette_key = keys::key_of(&self.keymap, &CommandRef::new(keys::PALETTE));
        let right: Vec<Segment> = host.iter().map(|_| Segment::new("No agent").hover("Connecting an AI agent comes with step 8 of M1")).collect();
        // The interface of the window (decisions of the owner, 2026-11-16): the theme, then the detached mode to come.
        let tools = [
            if self.prefs.dark {
                Tool { icon: Icon::Sun, hover: "Light theme".to_owned(), command: keys::DARK.to_owned(), planned: false }
            } else {
                Tool { icon: Icon::Moon, hover: "Dark theme".to_owned(), command: keys::DARK.to_owned(), planned: false }
            },
            Tool {
                icon: Icon::DetachedMode,
                hover: "Every function in its window: planned, after craft-libs#1903".to_owned(),
                command: keys::PLANNED_DETACHED_MODE.to_owned(),
                planned: true,
            },
        ];
        // When the app draws the frame (Windows): the edges resize the window, the top bar is its title bar.
        kit_shell::frame::edges(ui.ctx(), &self.frame);
        let tokens = kit_shell::theme::Tokens::of(ui.ctx());
        let answers = egui::Panel::top("ide-top")
            .exact_size(kit_shell::theme::TOP_BAR)
            .frame(kit_shell::theme::bar_frame(&tokens))
            .show(ui, |ui| {
                top_bar::show(
                    ui,
                    &Bar {
                        name: "/labcraft",
                        menus: &menus,
                        undo: (self.history.undo.is_some(), self.history.undo.clone()),
                        redo: (self.history.redo.is_some(), self.history.redo.clone()),
                        workspaces: &workspaces,
                        workspace: "edit",
                        project: snapshot.project.as_deref(),
                        palette_key: palette_key.as_deref(),
                        right: &right,
                        tools: &tools,
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
                Answer::OpenPalette => self.open_palette(&view),
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
        self.in_memory = start.in_memory;
        let status = self.status(&view);
        if let Some(command) = egui::Panel::bottom("ide-status")
            .frame(kit_shell::theme::bar_frame(&tokens).inner_margin(egui::Margin::symmetric(8, 3)))
            .show(ui, |ui| kit_shell::status::show(ui, &status.0, &status.1))
            .inner
        {
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
            Some(host) => self.dock(ui, host, &view, start, &entries, &mut outward),
        }
        // The list of quick insertion opened in a window of the system is drawn there (`Content::window_drawn`): its
        // place is in the frame of that window.
        let quick_elsewhere = self.quick.as_ref().is_some_and(|list| self.native.contains(&list.function));
        if let Some(host) = host
            && !quick_elsewhere
        {
            self.show_quick(ui.ctx(), host);
        }
        // The palette opened in a window of the system is drawn there (`Content::window_drawn`).
        let elsewhere = self.palette_in.as_ref().is_some_and(|function| self.native.contains(function));
        if !elsewhere && let Some(chosen) = self.palette.show(ui.ctx(), &entries) {
            // A command chosen with the mouse acts on the window the palette opened in, though the click moved the focus.
            self.focus.clone_from(&self.palette_in);
            self.chosen(chosen, host, &view, &mut outward);
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

    /// The window of the project takes focus back from a detached one: a press in it, out of a detached window that
    /// floats in it (the web), or the window of the system coming to the front (the desktop).
    fn follow_focus(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|input| input.viewport().focused).unwrap_or(true);
        let pressed = ctx.input(|input| input.pointer.primary_pressed().then(|| input.pointer.interact_pos()).flatten());
        if let Some(at) = pressed {
            // The detached window under the press, its bar and its tab included, the one in front first: one that
            // floats in this window (the web); a window of the system takes its own focus (`Content::window_drawn`).
            self.focus = self.workspace.layout.floating.iter().rev().filter(|window| !window.minimized).find_map(|window| {
                let function = window.panels().find_map(Pane::function).filter(|function| !self.native.contains(*function))?;
                let [x, y, width, height] = window.rect;
                egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, height)).contains(at).then(|| function.to_owned())
            });
        } else if focused && !self.root_focused {
            self.focus = None;
        }
        self.root_focused = focused;
    }

    fn open_palette(&mut self, view: &Published) {
        self.palette_in = self.focused(view);
        self.palette.open();
    }

    fn chosen(&mut self, chosen: Chosen, host: Option<&dyn Host>, view: &Published, outward: &mut Vec<Outward>) {
        match chosen {
            Chosen::Run { id, params } => self.run(CommandRef::with_params(id, params), host, view, outward),
            Chosen::Planned(planned) => self.card.open(planned),
        }
    }

    /// A new project: what the window kept of the one before goes.
    fn new_project(&mut self, snapshot: &Snapshot) {
        // A new session counts from 0 again, even for the same folder.
        let again = snapshot.view.generation < self.generation;
        self.generation = snapshot.view.generation;
        if self.project == snapshot.project && self.session == snapshot.session && !again {
            return;
        }
        self.project.clone_from(&snapshot.project);
        self.session = snapshot.session;
        self.drawn.clear();
        self.texts.clear();
        self.callables = panels::Callables::default();
        self.asking_show = None;
        self.brought = None;
        self.focus = None;
        self.placing.clear();
        self.palette_in = None;
        self.history = History::default();
        // A run of the project before is not followed into this session, whose runs count from the start again.
        self.running = None;
        self.outcomes.clear();
    }

    /// The results that came: the failures to say, the function shown, the history.
    fn take_replies(&mut self, host: &dyn Host, view: &Published) {
        let mut failures = Vec::new();
        let mut take = |id: &str, reply: &Reply| match reply.take() {
            None => true,
            Some(Ok(_)) => false,
            Some(Err(error)) => {
                failures.push(format!("{id}: {}", error.message));
                false
            }
        };
        self.pending.retain(|(id, reply)| take(id, reply));
        let mut moved_back = Vec::new();
        let mut relabelled = Vec::new();
        let mut inserted = Vec::new();
        let mut templates = Vec::new();
        self.edits.retain(|(function, reply, call)| match reply.take() {
            None => true,
            Some(Ok(result)) => {
                // Only an insertion done goes among the recent ones (notation 2.6).
                inserted.extend(call.clone());
                // `string.format` arrives with its template field open (notation 2.6).
                if call.as_deref() == Some("string.format")
                    && let Some(node) = result.get("names").and_then(|names| names.get("node")).and_then(Value::as_str)
                {
                    templates.push((function.clone(), node.to_owned()));
                }
                // A move that changed nothing brings no new drawing: its nodes go back now.
                if result.get("step").is_some_and(Value::is_null) {
                    moved_back.push(function.clone());
                }
                relabelled.push((function.clone(), renamed_nodes(&result)));
                false
            }
            Some(Err(error)) => {
                failures.push(format!("{}: {}", function, error.message));
                moved_back.push(function.clone());
                false
            }
        });
        for call in inserted {
            labcraft_editor::insertion::remember(&mut self.prefs.recent, &call);
            self.prefs_changed = true;
        }
        self.templates.extend(templates);
        self.templates.retain(|(function, node)| match self.drawn.get_mut(function) {
            Some(drawn) => !drawn.canvas.edit_template(node),
            None => false,
        });
        // A field in place: closed when its command is done, open with the message when it refuses (notation 2.6).
        let drawn = &mut self.drawn;
        self.field_edits.retain(|(function, reply)| match reply.take() {
            None => true,
            Some(answer) => {
                if let Some(drawn) = drawn.get_mut(function) {
                    match answer {
                        Ok(_) => drawn.canvas.close_field(),
                        Err(error) => drawn.canvas.field_refused(&error.message),
                    }
                }
                false
            }
        });
        for function in moved_back {
            if let Some(drawn) = self.drawn.get_mut(&function) {
                drawn.canvas.cancel_move();
                drawn.canvas.cancel_wire();
            }
        }
        // A node named by its line that a command gave a label stays selected by its label (decision of the owner,
        // 2026-11-17).
        for (function, renamed) in relabelled {
            let Some(window) = view.window_of(&function) else { continue };
            let selection = &window.selection;
            if renamed.is_empty()
                || selection.function != function
                || !selection.nodes.iter().any(|name| renamed.iter().any(|(from, _)| from == name))
            {
                continue;
            }
            let nodes =
                selection.nodes.iter().map(|name| renamed.iter().find(|(from, _)| from == name).map_or_else(|| name.clone(), |(_, to)| to.clone()));
            let wires = selection.wires.iter().filter(|wire| !wire.contains('@')).cloned().collect();
            let asked = Selection { nodes: nodes.collect(), wires, terminals: selection.terminals.clone() };
            self.ask_select(host, &function, asked);
        }
        let mut again = Vec::new();
        self.selecting.retain(|(selection, function, reply)| match reply.take() {
            None => true,
            Some(Ok(_)) => false,
            Some(Err(error)) => {
                let without = without_lines(selection);
                if without != *selection {
                    again.push((function.clone(), without));
                } else {
                    failures.push(format!("view.select: {}", error.message));
                }
                false
            }
        });
        for (function, selection) in again {
            self.ask_select(host, &function, selection);
        }
        self.placing.retain_mut(|placing| {
            if !placing.answered {
                match placing.reply.take() {
                    None => {}
                    Some(Ok(_)) => placing.answered = true,
                    // A request refused is over: the dock follows the session again.
                    Some(Err(error)) => {
                        failures.push(format!("{}: {}", placing.id, error.message));
                        return false;
                    }
                }
            }
            !(placing.answered && placing.done(view))
        });
        for failure in failures {
            self.say(failure);
        }
        if let Some((function, reply)) = &self.asking_show
            && (reply.take().is_some() || view.main.function.as_deref() == Some(function.as_str()))
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

    /// Follows the run the window started: what the session answered, then how the run goes, to its end.
    fn follow_run(&mut self, host: &dyn Host, ctx: &egui::Context) {
        let Some(running) = &mut self.running else { return };
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(running::EVERY));
        let function = running.function.clone();
        match running.follow(host, ctx.input(|input| input.time)) {
            running::Heard::Going => {}
            running::Heard::Ended(outcome) => {
                self.running = None;
                self.outcomes.insert(function, outcome);
            }
            running::Heard::Failed(message) => {
                self.running = None;
                self.outcomes.insert(function.clone(), running::Outcome { said: format!("Failed: {message}"), unrecorded: false });
                self.say(format!("Run {function}: {message}"));
            }
        }
    }

    /// What the `Run` panel says of the run of `function` that the window started: going, or how it ended.
    fn run_said(&self, function: Option<&str>, now: f64) -> Option<panels::RunSaid> {
        let function = function?;
        match &self.running {
            Some(running) if running.function == function => {
                let stopping = if running.stopping() { ", stopping" } else { "" };
                let said = format!("Running{stopping}… {:.0} s", (now - running.started).max(0.0));
                Some(panels::RunSaid { said, unrecorded: false })
            }
            _ => self.outcomes.get(function).map(|outcome| panels::RunSaid { said: outcome.said.clone(), unrecorded: outcome.unrecorded }),
        }
    }

    /// Asks the session to detach, dock or close the tab of a function, as a gesture on the dock did.
    fn place(&mut self, host: &dyn Host, id: &'static str, function: String) {
        let reply = host.send(id, json!({ "function": function }));
        self.placing.push(Placing { function, id, reply, answered: false });
    }

    fn is_placing(&self, function: &str) -> bool {
        self.placing.iter().any(|placing| placing.function == function)
    }

    /// The tabs of the dock follow the tabs of the session: a function opened has its tab among the tabs of the
    /// project, a detached one its floating window, a tab closed goes, and the function shown is in front.
    fn sync_tabs(&mut self, ctx: &egui::Context, view: &Published) {
        let detached: Vec<String> = view.detached.iter().filter_map(|shown| shown.function.clone()).collect();
        // A tab docked again by a command: out of its floating window, then among the tabs below.
        for function in &view.opened {
            let pane = Pane::Function(function.clone());
            if !self.is_placing(function) && pane::is_floating(&self.workspace.layout, &pane) {
                let _ = self.workspace.apply(Action::Close { panel: pane }, self.permissions());
            }
        }
        for function in view.opened.iter().chain(&detached) {
            let pane = Pane::Function(function.clone());
            // A tab the user just closed stays closed while the session closes it.
            if self.workspace.layout.contains(&pane) || self.is_placing(function) {
                continue;
            }
            let beside = self.docked_function();
            let start = self.workspace.layout.contains(&Pane::Start).then_some(Pane::Start);
            let anchor = beside.clone().or(start);
            let _ = self.workspace.apply(Action::Open { panel: pane, anchor: anchor.clone() }, self.permissions());
            // The start page gives way to the first function opened in its place.
            if beside.is_none() && anchor == Some(Pane::Start) {
                let _ = self.workspace.apply(Action::Close { panel: Pane::Start }, self.permissions());
            }
        }
        // A tab detached by a command: into a floating window, where the last one was (decision of the owner, 2026-11-16).
        for function in &detached {
            let pane = Pane::Function(function.clone());
            if !self.is_placing(function) && !pane::is_floating(&self.workspace.layout, &pane) {
                self.float(ctx, pane);
                self.raise = Some(function.clone());
            }
        }
        let gone: Vec<Pane> = self
            .workspace
            .layout
            .panels()
            .into_iter()
            .filter(|pane| pane.function().is_some_and(|function| !view.opened.iter().chain(&detached).any(|open| open == function)))
            .cloned()
            .collect();
        for pane in gone {
            if let Some(function) = pane.function() {
                self.drawn.remove(function);
            }
            let _ = self.workspace.apply(Action::Close { panel: pane }, self.permissions());
        }
        self.workspace.hidden.retain(|(pane, _)| pane.function().is_none());
        // The function shown comes to the front once, when the session shows it: after that, the start page or a panel
        // the user brings in front of it stays there.
        if self.asking_show.is_none() && self.brought != view.main.function {
            self.brought.clone_from(&view.main.function);
            if let Some(function) = &view.main.function {
                let pane = Pane::Function(function.clone());
                if self.workspace.layout.panel_state(&pane) == PanelState::Open {
                    let _ = self.workspace.apply(Action::Activate { panel: pane }, self.permissions());
                }
            }
        }
        // A detached window asked for: to the front, with focus.
        if let Some(function) = self.raise.take() {
            let pane = Pane::Function(function.clone());
            if let Some(_id) = pane::floating_window(&self.workspace.layout, &pane).map(|window| window.id) {
                #[cfg(feature = "native-windows")]
                ctx.send_viewport_cmd_to(DockArea::<Pane>::window_viewport(dock_id(), _id), egui::ViewportCommand::Focus);
                let _ = self.workspace.apply(Action::Activate { panel: pane }, self.permissions());
                self.focus = Some(function);
            }
        }
    }

    /// Floats the tab of a function into a window of its own, at the place of the last detached window.
    fn float(&mut self, ctx: &egui::Context, pane: Pane) {
        let place = self.prefs.detached.clone();
        // A window of the system opens where the last one was on the desktop, a second screen included: its rect is
        // given from the window of the project, which may have moved since.
        let origin = ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.min)).filter(|origin| origin.is_finite());
        let native = place.as_ref().and_then(|place| place.native).zip(origin.filter(|_| cfg!(feature = "native-windows")));
        let rect = match native {
            Some(([x, y, width, height], origin)) => [x - origin.x, y - origin.y, width, height],
            None => {
                // In the window of the project, kept within it: it may have shrunk since.
                let screen = ctx.content_rect();
                let mut rect = place.as_ref().map_or(FIRST_PLACE, |place| place.rect);
                let [x, y, width, height] = &mut rect;
                *width = width.clamp(240.0, screen.width().max(240.0));
                *height = height.clamp(160.0, screen.height().max(160.0));
                *x = x.clamp(screen.left(), (screen.right() - *width).max(screen.left()));
                *y = y.clamp(screen.top(), (screen.bottom() - *height).max(screen.top()));
                rect
            }
        };
        self.keep_centre(&pane);
        if self.workspace.apply(Action::Float { panel: pane.clone(), rect }, self.permissions()).is_err() {
            return;
        }
        if let Some(place) = place
            && let Some(window) = self.workspace.layout.floating.iter_mut().find(|window| window.contains(&pane))
        {
            window.native = place.native;
            window.monitor = place.monitor;
            window.monitor_size = place.monitor_size;
        }
    }

    /// The last tab of a function among the tabs of the project is leaving them: the start page takes its place, so
    /// that the centre of the window stays where the tabs open (ide.md 2).
    fn keep_centre(&mut self, pane: &Pane) {
        let layout = &self.workspace.layout;
        let docked = layout.panels().into_iter().filter(|other| other.function().is_some() && !pane::is_floating(layout, other)).count();
        if docked == 1 && !pane::is_floating(layout, pane) && !layout.contains(&Pane::Start) {
            let _ = self.workspace.apply(Action::Open { panel: Pane::Start, anchor: Some(pane.clone()) }, self.permissions());
        }
    }

    /// The place of the detached windows, kept for the next one: that of the last one the user moved.
    fn keep_place(&mut self) {
        let place_of = |window: &Floating<Pane>| Place {
            rect: window.rect,
            native: window.native,
            monitor: window.monitor.clone(),
            monitor_size: window.monitor_size,
        };
        let same = |place: &Place, window: &Floating<Pane>| {
            place.rect == window.rect && place.native == window.native && place.monitor == window.monitor && place.monitor_size == window.monitor_size
        };
        let detached =
            || self.workspace.layout.floating.iter().filter(|window| !window.minimized && window.panels().any(|pane| pane.function().is_some()));
        // The window that moved, or that is new, since the frame before.
        let moved = detached().find(|window| !self.places_seen.get(&window.id).is_some_and(|seen| same(seen, window))).map(place_of);
        if moved.is_none() && self.places_seen.len() == detached().count() {
            return;
        }
        self.places_seen = detached().map(|window| (window.id, place_of(window))).collect();
        if let Some(moved) = moved
            && self.prefs.detached.as_ref() != Some(&moved)
        {
            self.prefs.detached = Some(moved);
            self.prefs_changed = true;
        }
    }

    /// The drawing and the values of the function of each window, to its canvas.
    fn take_drawing(&mut self, view: &Published) {
        let reduced = self.prefs.reduced_motion;
        let values_shown = self.prefs.values_shown;
        for shown in view.windows() {
            let Some(function) = &shown.function else { continue };
            if !self.drawn.contains_key(function) {
                let mut canvas = Canvas::new();
                canvas.set_reduced_motion(reduced);
                canvas.set_values_shown(values_shown);
                let drawn =
                    Drawn { canvas, fit_values: false, drawing_generation: None, overlay_generation: None, zoom_given: None, split_seen: None };
                self.drawn.insert(function.clone(), drawn);
            }
            let Some(drawn) = self.drawn.get_mut(function) else { continue };
            if drawn.drawing_generation != Some(shown.drawing_generation) {
                drawn.drawing_generation = Some(shown.drawing_generation);
                if let Some(drawing) = &shown.drawing
                    && !drawn.canvas.drawing().is_some_and(|before| Arc::ptr_eq(before, drawing))
                {
                    let first = drawn.canvas.drawing().is_none();
                    // The glide when the file of the function changed; the first drawing of a tab is fitted.
                    drawn.canvas.set_drawing(drawing.clone(), shown.glide && !first);
                    if first {
                        drawn.canvas.fit();
                        drawn.fit_values = true;
                    }
                }
            }
            if drawn.overlay_generation != Some(shown.overlay_generation) {
                drawn.overlay_generation = Some(shown.overlay_generation);
                drawn.canvas.set_values(shown.overlay.clone(), shown.run.clone());
                if std::mem::take(&mut drawn.fit_values) && shown.overlay.as_ref().is_some_and(|overlay| overlay.top > 0) {
                    drawn.canvas.fit();
                }
            }
        }
    }

    /// The menus, laid out for the platform, with their keys and their states: those of the function shown act on the
    /// window that has focus.
    fn menus(&self, view: &Published, project: bool) -> MenuBar {
        let mut bar = craft_menu::layout(&keys::menus(), Target::current(), &LayoutOptions::new("LabCraft")).bar;
        craft_menu::apply_keymap(&mut bar, &self.keymap);
        let shown = self.target(view).function.as_deref();
        let tab = shown.map(|function| tab_of(view, function)).unwrap_or_default();
        craft_menu::apply_state(&mut bar, |command| {
            let id = command.id.as_str();
            let param = |name: &str| command.params.get(name).and_then(Value::as_str).map(str::to_owned);
            match id {
                "project.new"
                | "project.open"
                | keys::HOME
                | keys::PALETTE
                | keys::DARK
                | keys::LAB
                | keys::REDUCED_MOTION
                | keys::DRAG_PANS
                | keys::FIX_EVERY_NODE
                | keys::MARK_FIXED
                | keys::REPLACE_WITH_ALT
                | keys::ACCEPT_MISMATCH
                | keys::ROUTE_WITH_SHIFT_ALT
                | keys::ABOUT => {
                    let checked = match id {
                        keys::DARK => Some(self.prefs.dark),
                        keys::LAB => Some(self.look() == kit_shell::theme::Look::Lab),
                        keys::REDUCED_MOTION => Some(self.prefs.reduced_motion),
                        keys::DRAG_PANS => Some(self.prefs.drag_pans),
                        keys::FIX_EVERY_NODE => Some(self.prefs.fix_every_node),
                        keys::MARK_FIXED => Some(self.prefs.mark_fixed),
                        keys::REPLACE_WITH_ALT => Some(self.prefs.replace_with_alt),
                        keys::ACCEPT_MISMATCH => Some(self.prefs.accept_mismatch),
                        keys::ROUTE_WITH_SHIFT_ALT => Some(self.prefs.route_with_shift_alt),
                        _ => None,
                    };
                    ItemState { enabled: true, checked }
                }
                keys::PLANNED_AGENT
                | keys::PLANNED_INSTRUMENTS
                | keys::PLANNED_PAUSE
                | keys::PLANNED_BREAKPOINT
                | keys::PLANNED_WATCH
                | keys::PLANNED_BUILD
                | keys::PLANNED_DETACHED_MODE => ItemState { enabled: true, checked: None },
                keys::RECORD => ItemState { enabled: true, checked: Some(self.prefs.record_values) },
                keys::RUN => ItemState { enabled: self.can_run(view, shown).is_ok(), checked: None },
                keys::STOP => ItemState { enabled: self.running.as_ref().is_some_and(|running| !running.stopping()), checked: None },
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
                keys::SELECT_ALL
                | keys::SELECT_UPSTREAM
                | keys::SELECT_DOWNSTREAM
                | keys::SELECT_ALL_UPSTREAM
                | keys::SELECT_ALL_DOWNSTREAM
                | keys::ARRANGE
                | keys::WIRE
                | keys::QUICK_INSERT
                | keys::DELETE
                | keys::RENAME => ItemState { enabled: shown.is_some(), checked: None },
                keys::AUTOPATCH => ItemState { enabled: true, checked: Some(self.prefs.autopatch) },
                keys::INTO_WIRE => ItemState { enabled: true, checked: Some(self.prefs.into_wire) },
                _ => ItemState { enabled: project, checked: None },
            }
        });
        bar
    }

    /// Whether the window can run `function` now, or why not.
    fn can_run(&self, view: &Published, function: Option<&str>) -> Result<(), String> {
        if let Some(why) = labcraft_engine::runs::not_enabled_here() {
            return Err(format!("Run: here, {why}"));
        }
        let Some(function) = function else { return Err("Run: no function shown".to_owned()) };
        // Arguments cannot be given from the window yet: a function that needs one would be refused (`LC8103`).
        if let Some(listed) = view.functions.iter().find(|listed| listed.function == function)
            && !listed.needs.is_empty()
        {
            let inputs = listed.needs.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ");
            let have = if listed.needs.len() == 1 { "has" } else { "have" };
            return Err(format!("Run: {inputs} {have} no default value; running with arguments comes later"));
        }
        if let Some(running) = &self.running {
            return Err(if running.function == function {
                format!("{function} is running")
            } else {
                format!("Run: {} is running; one run at a time", running.function)
            });
        }
        Ok(())
    }

    /// The status bar: the file shown, the last step; the agent, the zoom. Of the window that has focus.
    fn status(&self, view: &Published) -> (Vec<Segment>, Vec<Segment>) {
        let shown = self.target(view).function.as_deref();
        let mut left = Vec::new();
        if let Some(file) = shown.and_then(|function| file_of_tab(view, function)) {
            left.push(if self.in_memory {
                Segment::new(format!("{file} · in memory")).hover("The page holds the project in memory: nothing leaves your browser")
            } else {
                Segment::new(format!("{file} · saved")).hover("The window writes every change to the file at once")
            });
        }
        if let Some(last) = &self.history.last {
            left.push(Segment::new(last.clone()).hover("The last step of the history"));
        }
        let mut right = Vec::new();
        if let Some(drawn) = shown.and_then(|function| self.drawn.get(function)) {
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
                recent: (!start.in_memory).then_some(start.recent.as_slice()),
                examples: &start.examples,
                examples_note: if start.in_memory { "Each opens in this page, in memory" } else { "Each opens as a copy, in a folder you choose" },
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

    /// The dock: the panels, the tabs of the functions, and the detached windows.
    fn dock(
        &mut self,
        ui: &mut egui::Ui,
        host: &dyn Host,
        view: &Arc<Published>,
        start: &StartList,
        entries: &[palette::Entry],
        outward: &mut Vec<Outward>,
    ) {
        let mut style = DockStyle::from_ui(ui);
        let tokens = kit_shell::theme::Tokens::of(ui.ctx());
        style.background = tokens.chrome;
        style.tab_background = tokens.chrome;
        style.active_background = tokens.base;
        style.text = tokens.text;
        style.inactive_text = tokens.text_weak;
        style.border = egui::Stroke::new(1.0, tokens.border);
        style.accent = tokens.accent;
        // Room between the panels, so that each reads as a surface of its own (owner, 2026-11-16).
        style.gap = 4.0;
        // Rounded panels, as every surface of `kit_shell::theme`; the room between them is the
        // `CentralPanel` below, `base`.
        style.corner_radius = f32::from(kit_shell::theme::RADIUS);
        style.corner_fill = tokens.base;
        self.native.clear();
        let mut content = Content {
            ide: self,
            host,
            view,
            start,
            entries,
            asked: Vec::new(),
            chosen: None,
            drawing: None,
            commands: Vec::new(),
            palette_chosen: None,
        };
        let layout = content.ide.workspace.layout.clone();
        let output = egui::CentralPanel::default()
            .show(ui, |ui| DockArea::new(dock_id()).native_windows(cfg!(feature = "native-windows")).show_content(ui, &layout, &style, &mut content))
            .inner;
        let Content { asked, chosen, commands, palette_chosen, .. } = content;
        // What the gestures on the dock changed: only when there were some.
        let moved = !output.actions.is_empty();
        let before = if moved { self.functions_in_dock() } else { Vec::new() };
        for action in output.actions {
            // A tab dragged out of the tabs of the project, into a window of its own.
            if let Action::Float { panel, .. }
            | Action::MoveGroup { panel, to: DockTarget::Float { .. } }
            | Action::MoveColumn { panel, to: DockTarget::Float { .. } } = &action
                && panel.function().is_some()
            {
                self.keep_centre(panel);
            }
            match &action {
                Action::Activate { panel: Pane::Function(function) }
                    if view.main.function.as_deref() != Some(function.as_str()) && !view.is_detached(function) =>
                {
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
            if self.workspace.apply(action, self.permissions()).is_ok() {
                self.prefs_changed = true;
            }
        }
        // A tab of a function that the dock closed, by its ×, its menu, or the × of its window of the system (decision of
        // the owner, 2026-11-16: closing a detached window closes the tab).
        if moved {
            let after = self.functions_in_dock();
            for function in before.into_iter().filter(|function| !after.contains(function)) {
                if view.opened.contains(&function) || view.is_detached(&function) {
                    self.place(host, "view.close_function", function);
                }
            }
            self.ask_placing(host, view);
        }
        self.keep_place();
        // What a detached window of the system asked for, with its focus.
        for command in commands {
            self.run(command, Some(host), view, outward);
        }
        if let Some(chosen) = palette_chosen {
            self.chosen(chosen, Some(host), view, outward);
        }
        for asked in asked {
            self.panel_asked(asked, Some(host), view, outward);
        }
        if let Some(chosen) = chosen {
            self.chose_on_start(chosen, Some(host), view, outward);
        }
        // The zoom of the canvas of each window, which `view.get_state` returns.
        for shown in view.windows() {
            let Some(function) = &shown.function else { continue };
            if let Some(drawn) = self.drawn.get_mut(function) {
                let (zoom, _) = drawn.canvas.viewport();
                if drawn.zoom_given != Some(zoom) {
                    drawn.zoom_given = Some(zoom);
                    host.set_zoom(function, zoom);
                }
            }
        }
    }

    /// The functions whose tab is in the dock, docked or floating.
    fn functions_in_dock(&self) -> Vec<String> {
        self.workspace.layout.panels().into_iter().filter_map(|pane| pane.function().map(str::to_owned)).collect()
    }

    /// The tab of a function among the tabs of the project, not in a window of its own: where a new tab goes.
    fn docked_function(&self) -> Option<Pane> {
        let layout = &self.workspace.layout;
        layout.panels().into_iter().find(|pane| pane.function().is_some() && !pane::is_floating(layout, pane)).cloned()
    }

    /// A tab that the user dragged out of the dock, or back into it, asks the session to detach or to dock it.
    fn ask_placing(&mut self, host: &dyn Host, view: &Published) {
        let functions: Vec<(String, bool)> = self
            .workspace
            .layout
            .panels()
            .into_iter()
            .filter_map(|pane| pane.function().map(|function| (function.to_owned(), pane::is_floating(&self.workspace.layout, pane))))
            .collect();
        for (function, floating) in functions {
            if self.is_placing(&function) {
                continue;
            }
            let id = match (floating, view.is_detached(&function)) {
                (true, false) if view.opened.contains(&function) => "view.detach",
                (false, true) => "view.dock",
                _ => continue,
            };
            if id == "view.detach" {
                self.focus = Some(function.clone());
            }
            self.place(host, id, function);
        }
    }

    fn panel_asked(&mut self, asked: panels::Asked, host: Option<&dyn Host>, view: &Published, outward: &mut Vec<Outward>) {
        match asked {
            panels::Asked::Show { function, preview } => {
                // A detached function comes to the front in its window.
                if view.is_detached(&function) {
                    self.raise = Some(function);
                    return;
                }
                // Asked again, it comes to the front again, even when it is the function shown.
                self.brought = None;
                let params = if preview { json!({ "function": function, "preview": true }) } else { json!({ "function": function }) };
                self.run(CommandRef::with_params("view.show_function", params), host, view, outward);
            }
            panels::Asked::Command(id, params) => self.run(CommandRef::with_params(id, params), host, view, outward),
            panels::Asked::Window(id) => self.run(CommandRef::new(id), host, view, outward),
        }
    }

    /// Ends the runs of Ctrl and the arrows of every canvas: the moves they ask for.
    fn end_runs(&mut self, host: &dyn Host, view: &Published) {
        let ended: Vec<(String, Intent)> =
            self.drawn.iter_mut().filter_map(|(function, drawn)| Some((function.clone(), drawn.canvas.end_run()?))).collect();
        for (function, intent) in ended {
            if let Intent::Move { nodes, by } = intent {
                self.ask_move(host, view, &function, &nodes, by);
            }
        }
    }

    /// `node.move` for a drag or a run of keys on the canvas of `function`; with the setting "fix every node at the first
    /// move", in one batch with a `node.place` of every other free node where it is (notation 2.4; decision of the owner,
    /// 2026-11-17).
    fn ask_move(&mut self, host: &dyn Host, view: &Published, function: &str, nodes: &[String], by: (i64, i64)) {
        let Some(file) = file_of(view, function) else { return };
        let moving = json!({ "file": file, "nodes": nodes, "by": [by.0, by.1] });
        let drawing = view.window_of(function).and_then(|window| window.drawing.clone());
        let fixing: Vec<Value> = match (&drawing, self.prefs.fix_every_node) {
            (Some(drawing), true) => drawing
                .placed
                .iter()
                .enumerate()
                .filter(|(_, placed)| placed.at.is_none() && !placed.name.is_empty())
                .filter_map(|(at, placed)| {
                    let (x, y) = drawing.in_frame(at)?;
                    let unit = |hundredths: i64| (hundredths + 50).div_euclid(100);
                    Some(json!({ "id": "node.place", "params": { "file": file, "node": placed.name, "at": [unit(x), unit(y)] } }))
                })
                .collect(),
            _ => Vec::new(),
        };
        // A batch holds at most 1,000 commands (commands.md 6.7): past that, the move alone.
        let reply = if fixing.is_empty() || fixing.len() >= 1_000 {
            host.send("node.move", moving)
        } else {
            let mut commands = fixing;
            commands.push(json!({ "id": "node.move", "params": moving }));
            host.send("commands.batch", json!({ "commands": commands, "label": "Move Nodes" }))
        };
        self.edits.push((function.to_owned(), reply, None));
    }

    /// Asks for a move as the canvas of `function` would: for an app that drives the canvas itself (the scripted drag of
    /// `LABCRAFT_FRAMES`, scenario 6 of M1).
    pub fn ask_canvas_move(&mut self, host: &dyn Host, function: &str, nodes: &[String], by: (i64, i64)) {
        let view = host.snapshot().view;
        self.ask_move(host, &view, function, nodes, by);
    }

    /// Opens quick insertion on the canvas of `function` (notation 2.6); the canvas that has the keyboard gets it back
    /// when the list closes.
    fn open_quick(&mut self, view: &Published, function: &str, quick: labcraft_editor::Quick) {
        let Some(file) = file_of(view, function) else { return };
        let canvas = self.canvas_focus.as_ref().filter(|(focused, _)| focused == function).map(|(_, id)| *id);
        self.quick = Some(insert::QuickList::new(function.to_owned(), file, quick, canvas));
    }

    /// The list of quick insertion, while it is open: a choice is one step, the node and its wire (notation 2.6).
    fn show_quick(&mut self, ctx: &egui::Context, host: &dyn Host) {
        let Some(mut list) = self.quick.take() else { return };
        let catalog = self.callables.catalog.clone();
        match list.show(ctx, &catalog, &self.prefs.recent) {
            insert::Closed::Open => {
                self.quick = Some(list);
                return;
            }
            insert::Closed::Cancelled => {}
            insert::Closed::Picked(picked) => {
                let (id, params) = insert::command(&list.file, &list.quick, &picked, &catalog);
                let reply = host.send(&id, params);
                let call = match &picked {
                    insert::Picked::Entry(entry) => Some(entry.call.clone()),
                    insert::Picked::Constant(_) => None,
                };
                self.edits.push((list.function.clone(), reply, call));
            }
        }
        if let Some(canvas) = list.canvas {
            ctx.memory_mut(|memory| memory.request_focus(canvas));
        }
    }

    /// Delete on the canvas of `function` (notation 2.6): `node.delete` for the nodes selected, `wire.disconnect` for the
    /// wires selected and for those of the terminals selected or in focus, one step. A wire of a node deleted goes with
    /// it, and is not named again.
    fn delete(&mut self, host: &dyn Host, view: &Published, function: &str) {
        let Some(file) = file_of(view, function) else { return };
        let Some(window) = view.window_of(function) else { return };
        let selection = canvas_shown(window, &self.prefs).selection;
        let Some(drawn) = self.drawn.get(function) else { return };
        let mut wires: Vec<String> = selection.wires.clone();
        for terminal in &selection.terminals {
            wires.extend(drawn.canvas.wires_of_terminal(Some(terminal)));
        }
        if selection.is_empty() {
            wires.extend(drawn.canvas.wires_of_terminal(None));
        }
        // A wire that a deleted node gives or reads goes with the node.
        let deleted = |name: &str| selection.nodes.iter().any(|node| node == name);
        wires.retain(|wire| {
            let Some((from, to)) = wire.split_once("->") else { return false };
            let source = from.trim().split('.').next().unwrap_or_default();
            let reader = to.trim().split('.').next().unwrap_or_default();
            !deleted(source) && !deleted(reader)
        });
        wires.sort();
        wires.dedup();
        let mut commands: Vec<Value> =
            wires.iter().map(|wire| json!({ "id": "wire.disconnect", "params": { "file": file, "wire": wire } })).collect();
        // The constant of the terminal in focus goes too (notation 2.6).
        if selection.is_empty()
            && let Some(to) = drawn.canvas.constant_in_focus()
        {
            commands.push(json!({ "id": "input.clear_constant", "params": { "file": file, "to": to } }));
        }
        if !selection.nodes.is_empty() {
            commands.push(json!({ "id": "node.delete", "params": { "file": file, "nodes": selection.nodes } }));
        }
        if commands.is_empty() {
            self.say("Delete removes the nodes and the wires selected, or the wires of the terminal in focus");
            return;
        }
        let reply = host.send("commands.batch", json!({ "commands": commands, "label": "Delete" }));
        self.edits.push((function.to_owned(), reply, None));
    }

    /// Asks the session for a selection of the canvas of `function` (`view.select`).
    fn ask_select(&mut self, host: &dyn Host, function: &str, selection: Selection) {
        let reply = host.send("view.select", select_params(function, &selection));
        self.selecting.push((selection, function.to_owned(), reply));
    }

    /// Runs a command of a menu, a key, the palette, a button: an action of the window here, a command of the session
    /// through the host. What takes the function shown takes that of the window that has focus.
    fn run(&mut self, command: CommandRef, host: Option<&dyn Host>, view: &Published, outward: &mut Vec<Outward>) {
        let CommandRef { id, params } = command;
        let mut params = match params {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        params.retain(|_, value| !value.is_null());
        let shown = self.target(view).function.clone();
        match id.as_str() {
            "project.new" | "project.open" => {
                let folder = params.get("folder").and_then(Value::as_str).filter(|folder| !folder.is_empty()).map(str::to_owned);
                outward.push(if id == "project.new" { Outward::NewProject(folder) } else { Outward::OpenFolder(folder) });
            }
            keys::PALETTE => self.open_palette(view),
            keys::HOME => {
                if host.is_some() {
                    let anchor = self.docked_function();
                    if !self.workspace.layout.contains(&Pane::Start) {
                        let _ = self.workspace.apply(Action::Open { panel: Pane::Start, anchor }, self.permissions());
                    }
                    let _ = self.workspace.apply(Action::Activate { panel: Pane::Start }, self.permissions());
                }
            }
            keys::OPEN_TRACE => outward.push(Outward::OpenTrace(self.focused(view))),
            keys::RUN => {
                let Some(host) = host else {
                    self.say("Run: open a project first");
                    return;
                };
                let function = params.get("function").and_then(Value::as_str).map(str::to_owned).or(shown);
                match self.can_run(view, function.as_deref()) {
                    Err(why) => self.say(why),
                    Ok(()) => {
                        if let Some(function) = function {
                            self.outcomes.remove(&function);
                            self.running = Some(running::Running::start(host, &function, self.now, self.prefs.record_values));
                        }
                    }
                }
            }
            keys::RECORD => {
                self.prefs.record_values = !self.prefs.record_values;
                self.prefs_changed = true;
            }
            keys::STOP => match &mut self.running {
                Some(running) => running.stop(),
                None => self.say("Stop: no run going"),
            },
            keys::ABOUT => self.card.open(Planned {
                label: format!("LabCraft {}", env!("CARGO_PKG_VERSION")),
                line: "A graphical dataflow language and IDE, in Rust. MIT OR Apache-2.0.".to_owned(),
                milestone: String::new(),
                link: Some(catalog::REPOSITORY.to_owned()),
            }),
            keys::DARK => {
                self.prefs.dark = !self.prefs.dark;
                self.prefs_changed = true;
            }
            keys::LAB => {
                let look = if self.look() == kit_shell::theme::Look::Lab { kit_shell::theme::Look::Studio } else { kit_shell::theme::Look::Lab };
                self.prefs.look = look.name().to_owned();
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
            keys::DETACHED_INSPECTOR => {
                self.prefs.detached_inspector = !self.prefs.detached_inspector;
                self.prefs_changed = true;
            }
            keys::DRAG_PANS => {
                self.prefs.drag_pans = !self.prefs.drag_pans;
                self.prefs_changed = true;
            }
            keys::FIX_EVERY_NODE => {
                self.prefs.fix_every_node = !self.prefs.fix_every_node;
                self.prefs_changed = true;
            }
            keys::MARK_FIXED => {
                self.prefs.mark_fixed = !self.prefs.mark_fixed;
                self.prefs_changed = true;
            }
            keys::REPLACE_WITH_ALT => {
                self.prefs.replace_with_alt = !self.prefs.replace_with_alt;
                self.prefs_changed = true;
            }
            keys::ACCEPT_MISMATCH => {
                self.prefs.accept_mismatch = !self.prefs.accept_mismatch;
                self.prefs_changed = true;
            }
            keys::ROUTE_WITH_SHIFT_ALT => {
                self.prefs.route_with_shift_alt = !self.prefs.route_with_shift_alt;
                self.prefs_changed = true;
            }
            // W (notation 2.6): a wire from the terminal that has the focus, on the canvas that has the keyboard.
            keys::WIRE => {
                let Some(function) = self.canvas_focus.as_ref().map(|(function, _)| function.clone()).or(shown) else { return };
                let canvas_shown = view.window_of(&function).map(|window| canvas_shown(window, &self.prefs)).unwrap_or_default();
                let started = self.drawn.get_mut(&function).is_some_and(|drawn| drawn.canvas.start_wire(&canvas_shown));
                if !started {
                    self.say("W starts a wire from the terminal in focus: select a node, then Enter goes to its terminals");
                }
            }
            keys::AUTOPATCH => {
                self.prefs.autopatch = !self.prefs.autopatch;
                self.prefs_changed = true;
            }
            keys::INTO_WIRE => {
                self.prefs.into_wire = !self.prefs.into_wire;
                self.prefs_changed = true;
            }
            // Space (notation 2.6): quick insertion from what the canvas that has the keyboard holds.
            keys::QUICK_INSERT => {
                let Some(function) = self.canvas_focus.as_ref().map(|(function, _)| function.clone()).or(shown) else { return };
                let Some(window) = view.window_of(&function) else { return };
                let canvas_shown = canvas_shown(window, &self.prefs);
                let pointer = self.pointer;
                let autopatch = self.prefs.autopatch;
                let quick = self.drawn.get_mut(&function).and_then(|drawn| drawn.canvas.quick_insert(&canvas_shown, autopatch, pointer));
                if let Some(quick) = quick {
                    self.open_quick(view, &function, quick);
                }
            }
            // Delete (notation 2.6): the nodes and the wires selected, or what the terminals selected or in focus hold, one
            // step.
            keys::DELETE => {
                let function = self.canvas_focus.as_ref().map(|(function, _)| function.clone()).or(shown);
                let (Some(host), Some(function)) = (host, function) else { return };
                self.delete(host, view, &function);
            }
            // F2 (notation 2.6): the name of the node selected, or its label, in a field in place.
            keys::RENAME => {
                let Some(function) = self.canvas_focus.as_ref().map(|(function, _)| function.clone()).or(shown) else { return };
                let opened = self.drawn.get_mut(&function).is_some_and(|drawn| drawn.canvas.edit_name());
                if !opened {
                    self.say("F2 renames the node selected: select one node");
                }
            }
            // Arrange (notation 2.4, 2.6): the selection of the canvas that has the keyboard, else the whole function.
            keys::ARRANGE => {
                let function = self.canvas_focus.as_ref().map(|(function, _)| function.clone()).or(shown);
                let (Some(host), Some(function)) = (host, function) else { return };
                let Some(file) = file_of(view, &function) else { return };
                let mut params = Map::new();
                params.insert("file".to_owned(), json!(file));
                if let Some(window) = view.window_of(&function) {
                    let selection = canvas_shown(window, &self.prefs).selection;
                    for (name, list) in [("nodes", selection.nodes), ("wires", selection.wires), ("terminals", selection.terminals)] {
                        if !list.is_empty() {
                            params.insert(name.to_owned(), json!(list));
                        }
                    }
                }
                let reply = host.send("layout.arrange", Value::Object(params));
                self.edits.push((function, reply, None));
            }
            keys::SELECT_ALL | keys::SELECT_UPSTREAM | keys::SELECT_DOWNSTREAM | keys::SELECT_ALL_UPSTREAM | keys::SELECT_ALL_DOWNSTREAM => {
                let step = match id.as_str() {
                    keys::SELECT_ALL => Step::All,
                    keys::SELECT_UPSTREAM => Step::Upstream { every: false },
                    keys::SELECT_DOWNSTREAM => Step::Downstream { every: false },
                    keys::SELECT_ALL_UPSTREAM => Step::Upstream { every: true },
                    _ => Step::Downstream { every: true },
                };
                // The canvas that has the keyboard, else that of the window that has focus.
                let function = self.canvas_focus.as_ref().map(|(function, _)| function.clone()).or(shown);
                let (Some(host), Some(function)) = (host, function) else { return };
                let Some(window) = view.window_of(&function) else { return };
                let canvas_shown = canvas_shown(window, &self.prefs);
                let Some(selection) = self.drawn.get(&function).and_then(|drawn| drawn.canvas.step_selection(step, &canvas_shown)) else { return };
                self.ask_select(host, &function, selection);
            }
            keys::DETACHED_PALETTE => {
                self.prefs.detached_palette = !self.prefs.detached_palette;
                self.prefs_changed = true;
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
                if on != visible && self.workspace.set_visible(pane, on, None, self.permissions()).is_ok() {
                    self.prefs_changed = true;
                }
            }
            // The layout of ide.md 2, every detached tab docked again (commands.md 6.7).
            "view.reset_layout" => {
                self.workspace.reset(&pane::default_layout());
                self.workspace.saved = None;
                self.prefs_changed = true;
                if let Some(host) = host {
                    // Asked by the dock: it leaves them where it put them until the session answers.
                    for function in view.detached.iter().filter_map(|shown| shown.function.clone()) {
                        self.place(host, "view.dock", function);
                    }
                }
            }
            _ => {
                let Some(host) = host else {
                    self.say(format!("{id}: open a project first"));
                    return;
                };
                // What the window fills in: the function shown, and the other state of a toggle.
                if matches!(id.as_str(), "view.close_function" | "view.set_view" | "view.detach" | "view.dock" | "view.open_trace")
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
                // An editing command: its function, from its file, for what follows its result.
                let edited = EDITS
                    .contains(&id.as_str())
                    .then(|| params.get("file").and_then(Value::as_str))
                    .flatten()
                    .and_then(|file| view.functions.iter().find(|listed| listed.file == file).map(|listed| listed.function.clone()));
                let reply = host.send(&id, Value::Object(params));
                match edited {
                    Some(function) => self.edits.push((function, reply, None)),
                    None => self.pending.push((id, reply)),
                }
            }
        }
    }
}

/// The command of a field in place (notation 2.6, commands.md 6.7).
fn field_command(file: &str, kind: labcraft_editor::FieldKind, text: &str, accept_mismatch: bool) -> (&'static str, Value) {
    use labcraft_editor::FieldKind;
    match kind {
        FieldKind::Rename { value } => ("value.rename", json!({ "file": file, "value": value, "name": text })),
        FieldKind::Label { line } => ("node.set_label", json!({ "file": file, "line": line, "label": text })),
        // An empty field clears the constant (notation 2.6).
        FieldKind::Constant { to, .. } if text.is_empty() => ("input.clear_constant", json!({ "file": file, "to": to })),
        FieldKind::Constant { to, .. } => {
            ("input.set_constant", json!({ "file": file, "to": to, "value": text, "replace": true, "accept_mismatch": accept_mismatch }))
        }
        // The field shows the template between its quotes; a quote typed is one of the text (spec 14.1).
        FieldKind::Template { node } => ("node.set_template", json!({ "file": file, "node": node, "template": quoted(text) })),
    }
}

/// The field of a function whose file the window no longer knows: refused at once, so that it does not wait.
fn drawn_canvas_refused(ide: &mut Ide, function: &str) {
    if let Some(drawn) = ide.drawn.get_mut(function) {
        drawn.canvas.field_refused("this function has no file now: write it again once the project is read");
    }
}

/// A template typed in its field, as a string literal: a `"` that no `\\` escapes gets one.
fn quoted(text: &str) -> String {
    let mut literal = String::with_capacity(text.len() + 2);
    literal.push('"');
    let mut escaped = false;
    for c in text.chars() {
        if c == '"' && !escaped {
            literal.push('\\');
        }
        escaped = c == '\\' && !escaped;
        literal.push(c);
    }
    // A `\` at the end would escape the closing quote.
    if escaped {
        literal.push('\\');
    }
    literal.push('"');
    literal
}

/// The editing commands whose results the window follows (`Ide::edits`).
const EDITS: &[&str] = &["node.move", "node.place", "node.set_view", "layout.arrange"];

/// The file of a function, as the session lists it.
fn file_of(view: &Published, function: &str) -> Option<String> {
    view.functions.iter().find(|listed| listed.function == function).map(|listed| listed.file.clone())
}

/// The nodes named by their line that a command gave a label, from its result: each `@N` and its label (commands.md 9.3,
/// `renamed`), in the result itself or in each command of a batch.
fn renamed_nodes(result: &Value) -> Vec<(String, String)> {
    let lists = std::iter::once(result.get("renamed"))
        .chain(result.get("commands").and_then(Value::as_array).into_iter().flatten().map(|command| command.get("renamed")));
    lists
        .flatten()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|renamed| Some((renamed.get("from")?.as_str()?.to_owned(), renamed.get("to")?.as_str()?.to_owned())))
        .filter(|(from, _)| from.starts_with('@'))
        .collect()
}

/// What the canvas of a window shows without drawing again: its arms, its selection when it is of its function, and the
/// setting of the drag (notation 2.6).
fn canvas_shown(window: &WindowShown, prefs: &Prefs) -> Shown {
    let selection = &window.selection;
    let selection = if window.function.as_deref() == Some(selection.function.as_str()) {
        Selection { nodes: selection.nodes.clone(), wires: selection.wires.clone(), terminals: selection.terminals.clone() }
    } else {
        Selection::default()
    };
    Shown {
        arms: window.arms.clone(),
        selection,
        drag_pans: prefs.drag_pans,
        mark_fixed: prefs.mark_fixed,
        replace_with_alt: prefs.replace_with_alt,
        accept_mismatch: prefs.accept_mismatch,
        route_with_shift_alt: prefs.route_with_shift_alt,
        into_wire: prefs.into_wire,
    }
}

/// The lines of the file that the selection of a window names (notation 2.5): those of its nodes, and for a wire the
/// line that gives its value and those that read it.
fn selected_lines(window: &WindowShown) -> std::collections::BTreeSet<u32> {
    let mut lines = std::collections::BTreeSet::new();
    let (Some(drawing), Some(function)) = (&window.drawing, &window.function) else { return lines };
    let selection = &window.selection;
    if selection.function != *function {
        return lines;
    }
    let lines_of: std::collections::HashMap<&str, u32> = drawing.placed.iter().map(|placed| (placed.key.as_str(), placed.line)).collect();
    let line_of_key = |key: &str| lines_of.get(key).copied();
    for name in &selection.nodes {
        lines.extend(drawing.placed.iter().filter(|placed| placed.name == *name).map(|placed| placed.line));
    }
    for wire in drawing.groups.iter().filter_map(|group| group.wire.as_ref()) {
        if selection.wires.iter().any(|name| wire.named.contains(name)) {
            lines.extend(wire.from_node.as_deref().and_then(line_of_key));
            lines.extend(wire.to_nodes.iter().filter_map(|key| line_of_key(key)));
        }
    }
    lines.remove(&0);
    lines
}

/// A selection without the names by their line (`@N`, commands.md 9.2) and the wires into them.
fn without_lines(selection: &Selection) -> Selection {
    Selection {
        nodes: selection.nodes.iter().filter(|name| !name.starts_with('@')).cloned().collect(),
        wires: selection.wires.iter().filter(|name| !name.contains('@')).cloned().collect(),
        terminals: selection.terminals.clone(),
    }
}

/// The parameters of `view.select` for a selection of the canvas of `function`.
fn select_params(function: &str, selection: &Selection) -> Value {
    json!({ "function": function, "nodes": selection.nodes, "wires": selection.wires, "terminals": selection.terminals })
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

/// What the dock draws in each pane, and what the windows of the system asked for while it drew them.
struct Content<'a> {
    ide: &'a mut Ide,
    host: &'a dyn Host,
    view: &'a Arc<Published>,
    start: &'a StartList,
    /// The entries of the command palette, when it is open.
    entries: &'a [palette::Entry],
    asked: Vec<panels::Asked>,
    chosen: Option<start::Chosen>,
    /// The detached function whose window of the system is being drawn: `window_drawn` closes its pass.
    drawing: Option<String>,
    /// The keys of a window of the system, run once the dock is drawn, and what its palette chose.
    commands: Vec<CommandRef>,
    palette_chosen: Option<Chosen>,
}

impl DockContent<Pane> for Content<'_> {
    fn body(&mut self, ui: &mut egui::Ui, pane: &Pane) {
        // The panels have a margin; the tab of a function gives its canvas the whole of it.
        match pane {
            Pane::Function(function) if pane::is_floating(&self.ide.workspace.layout, pane) => self.detached(ui, function),
            Pane::Function(_) => self.body_inside(ui, pane),
            _ => {
                egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| self.body_inside(ui, pane));
            }
        }
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

    fn permissions(&self, pane: &Pane) -> Permissions {
        pane::permissions_in(pane, pane::is_floating(&self.ide.workspace.layout, pane))
    }

    /// Only the tab of a function becomes a window of the system; the panels float in the window of the project (ide.md
    /// 9: it keeps them).
    fn allow_native(&self, pane: &Pane) -> bool {
        pane.function().is_some()
    }

    /// A window of its own, which stays shown while another app has focus: not a palette of tools.
    fn window_kind(&self, pane: &Pane) -> WindowKind {
        if pane.function().is_some() { WindowKind::Normal } else { WindowKind::Palette }
    }

    /// A detached window of the system was drawn: its keys and its command palette, with its focus.
    fn window_drawn(&mut self, ui: &mut egui::Ui) {
        let Some(function) = self.drawing.take() else { return };
        let ctx = ui.ctx().clone();
        // Its keys while it has focus; its palette, open in it, stays drawn in it when it loses it.
        if ctx.input(|input| input.viewport().focused.unwrap_or(false)) {
            self.ide.focus = Some(function.clone());
            // Where quick insertion falls is read in the frame of this window.
            self.ide.pointer = ctx.pointer_latest_pos();
            for command in self.ide.key_hits(&ctx) {
                if command.id == keys::PALETTE {
                    self.ide.open_palette(self.view);
                } else {
                    self.commands.push(command);
                }
            }
        }
        if self.ide.quick.as_ref().is_some_and(|list| list.function == function) {
            self.ide.show_quick(&ctx, self.host);
        }
        if self.ide.palette_in.as_deref() == Some(function.as_str())
            && let Some(chosen) = self.ide.palette.show(&ctx, self.entries)
        {
            self.palette_chosen = Some(chosen);
        }
    }

    /// The menu of a tab of a function: detach or dock it, close it (decision of the owner, 2026-11-16).
    fn panel_menu(&mut self, ui: &mut egui::Ui, pane: &Pane) {
        let Pane::Function(function) = pane else { return };
        let detached = self.view.is_detached(function);
        let (id, label) = if detached { ("view.dock", "Dock") } else { ("view.detach", "Detach") };
        if ui.button(label).clicked() {
            self.asked.push(panels::Asked::Command(id.to_owned(), json!({ "function": function })));
            ui.close();
        }
        if ui.button("Close").clicked() {
            self.asked.push(panels::Asked::Command("view.close_function".to_owned(), json!({ "function": function })));
            ui.close();
        }
    }
}

impl Content<'_> {
    /// What a pane shows.
    fn body_inside(&mut self, ui: &mut egui::Ui, pane: &Pane) {
        let view = self.view;
        let focused = self.ide.target(view);
        let asked = match pane {
            Pane::Project => panels::project(ui, view, focused.function.as_deref(), self.ide.prefs.preview_tabs),
            Pane::Palette => {
                panels::palette(ui, &mut self.ide.callables);
                None
            }
            Pane::Inspector => panels::inspector(ui, view, &view.main, &self.ide.callables),
            Pane::Problems => panels::problems(ui, view, focused, &mut self.ide.problems),
            Pane::Run => {
                let said = self.ide.run_said(focused.function.as_deref(), ui.input(|input| input.time));
                panels::run(ui, focused, self.ide.prefs.values_shown, said.as_ref())
            }
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

    /// The toolbar of the tab of a function (ide.md 3): its views as icons, then what acts on the diagram; in a detached
    /// window, `Dock` and its side panels too (decisions of the owner, 2026-11-16).
    fn toolbar(&mut self, ui: &mut egui::Ui, function: &str, detached: bool) -> Option<panels::Asked> {
        let view = self.view;
        let live = view.window_of(function).is_some();
        let tab = tab_of(view, function);
        let split = tab.split.is_some();
        let zoom = self.ide.drawn.get(function).map(|drawn| drawn.canvas.viewport().0);
        let tokens = kit_shell::theme::Tokens::of(ui.ctx());
        let mut asked = None;
        ui.horizontal(|ui| {
            // The views are a mode, one at a time: a group of their own, apart from the actions that follow.
            egui::Frame::new()
                .fill(tokens.sunken)
                .stroke(egui::Stroke::new(1.0, tokens.border))
                .corner_radius(egui::CornerRadius::same(kit_shell::theme::RADIUS))
                .inner_margin(egui::Margin::same(2))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (kind, icon, hover) in
                        [(ViewKind::Diagram, Icon::Diagram, "Diagram"), (ViewKind::Text, Icon::Text, "Text"), (ViewKind::Block, Icon::Block, "Block")]
                    {
                        if icon::toggle(ui, icon, tab.view == kind && !split, true, hover).clicked() {
                            asked = Some(panels::Asked::Command("view.set_view".to_owned(), json!({ "function": function, "view": kind.as_str() })));
                        }
                    }
                    if icon::toggle(ui, Icon::Split, split, true, "Text | Diagram: the text beside the diagram").clicked() {
                        let params = if split {
                            json!({ "function": function, "view": "diagram" })
                        } else {
                            json!({ "function": function, "view": "text", "split": "diagram" })
                        };
                        asked = Some(panels::Asked::Command("view.set_view".to_owned(), params));
                    }
                    if icon::toggle(ui, Icon::Panel, false, true, "Panel: planned, M3").clicked() {
                        asked = Some(panels::Asked::Window("ide.planned.panel_view"));
                    }
                });
            ui.separator();
            // The run of the function (decisions of the owner, 2026-11-16): Run and Stop; pause, breakpoint and watch to
            // come, each opening its card.
            let key = |id: &str| keys::key_of(&self.ide.keymap, &CommandRef::new(id)).map(|key| format!(" ({key})")).unwrap_or_default();
            let can_run = self.ide.can_run(view, Some(function));
            let run_hover = match &can_run {
                Ok(()) => format!("Run {function}{}", key(keys::RUN)),
                Err(why) => why.clone(),
            };
            if icon::button(ui, Icon::Run, live && can_run.is_ok(), &run_hover).clicked() {
                asked = Some(panels::Asked::Command(keys::RUN.to_owned(), json!({ "function": function })));
            }
            let stoppable = self.ide.running.as_ref().is_some_and(|running| running.function == function && !running.stopping());
            if icon::button(ui, Icon::Stop, stoppable, &format!("Stop the run{}", key(keys::STOP))).clicked() {
                asked = Some(panels::Asked::Window(keys::STOP));
            }
            let record = self.ide.prefs.record_values;
            let record_hover = if record {
                "Record values: on. The next run writes its trace, and its values show on the wires"
            } else {
                "Record values: off. The next run writes no trace; the Run panel still gives its outputs"
            };
            if icon::toggle(ui, Icon::Record, record, true, record_hover).clicked() {
                asked = Some(panels::Asked::Window(keys::RECORD));
            }
            for (icon, id, hover) in [
                (Icon::Pause, keys::PLANNED_PAUSE, "Pause: planned, M5"),
                (Icon::Breakpoint, keys::PLANNED_BREAKPOINT, "Breakpoint: planned, M5"),
                (Icon::Watch, keys::PLANNED_WATCH, "Watch: planned, M5"),
            ] {
                if icon::planned(ui, icon, hover).clicked() {
                    asked = Some(panels::Asked::Window(id));
                }
            }
            ui.separator();
            // The actions on the diagram (the zoom is in the status bar): Fit, then the window; arrange and the tools
            // of editing come with their steps.
            if icon::button(ui, Icon::Fit, live && zoom.is_some(), "Fit the diagram to the view").clicked() {
                self.ide.focus = detached.then(|| function.to_owned());
                asked = Some(panels::Asked::Window(keys::FIT));
            }
            if detached {
                if icon::button(ui, Icon::Dock, true, "Dock: back among the tabs of the project").clicked() {
                    asked = Some(panels::Asked::Command("view.dock".to_owned(), json!({ "function": function })));
                }
                ui.separator();
                if icon::toggle(ui, Icon::Inspector, self.ide.prefs.detached_inspector, true, "Inspector").clicked() {
                    asked = Some(panels::Asked::Window(keys::DETACHED_INSPECTOR));
                }
                if icon::toggle(ui, Icon::Palette, self.ide.prefs.detached_palette, true, "Palette of functions").clicked() {
                    asked = Some(panels::Asked::Window(keys::DETACHED_PALETTE));
                }
            } else {
                if icon::button(ui, Icon::Detach, true, "Detach: into a window of its own").clicked() {
                    asked = Some(panels::Asked::Command("view.detach".to_owned(), json!({ "function": function })));
                }
                ui.separator();
                // The same button as in a detached window: here it shows or hides the panel of the window of the project.
                let visible = self.ide.workspace.layout.contains(&Pane::Inspector);
                if icon::toggle(ui, Icon::Inspector, visible, true, "Inspector").clicked() {
                    asked = Some(panels::Asked::Command("view.toggle_panel".to_owned(), json!({ "panel": "inspector" })));
                }
            }
        });
        ui.separator();
        asked
    }

    /// The tab of a function among the tabs of the project: its toolbar, and the view or the two of a split.
    fn function(&mut self, ui: &mut egui::Ui, function: &str) -> Option<panels::Asked> {
        let shown = self.view.main.function.as_deref() == Some(function);
        let asked = self.toolbar(ui, function, false);
        self.views(ui, function, false);
        if !shown && ui.ui_contains_pointer() && ui.input(|input| input.pointer.primary_pressed()) {
            return asked.or(Some(panels::Asked::Show { function: function.to_owned(), preview: false }));
        }
        asked
    }

    /// A detached window (ide.md 9): the toolbar of its function, its canvas, its inspector beside it and the palette of
    /// functions on the other side when the user shows them.
    fn detached(&mut self, ui: &mut egui::Ui, function: &str) {
        let viewport = ui.ctx().viewport_id();
        if viewport != egui::ViewportId::ROOT {
            self.drawing = Some(function.to_owned());
            self.ide.native.insert(function.to_owned());
        }
        let asked = self.toolbar(ui, function, true);
        self.asked.extend(asked);
        let view = self.view;
        if self.ide.prefs.detached_inspector
            && let Some(shown) = view.window_of(function)
        {
            egui::Panel::right(("ide-detached-inspector", function)).resizable(true).default_size(260.0).show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| self.asked.extend(panels::inspector(ui, view, shown, &self.ide.callables)));
            });
        }
        if self.ide.prefs.detached_palette {
            egui::Panel::left(("ide-detached-palette", function)).resizable(true).default_size(220.0).show(ui, |ui| {
                egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| panels::palette(ui, &mut self.ide.callables));
            });
        }
        egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| self.views(ui, function, true));
    }

    /// The view of a tab, or the two of a split.
    fn views(&mut self, ui: &mut egui::Ui, function: &str, detached: bool) {
        let view = self.view;
        let file = file_of_tab(view, function).map(str::to_owned);
        let theme = self.ide.theme();
        let tab = tab_of(view, function);
        if let Some(drawn) = self.ide.drawn.get_mut(function) {
            let placed = (tab.split.is_some(), detached);
            if drawn.split_seen.is_some_and(|seen| seen != placed) {
                drawn.canvas.fit();
            }
            drawn.split_seen = Some(placed);
        }
        match tab {
            Tab { view: first, split: Some(second) } => {
                ui.columns(2, |columns| {
                    if let [left, right] = columns {
                        self.one_view(left, function, first, file.as_deref(), theme);
                        self.one_view(right, function, second, file.as_deref(), theme);
                    }
                });
            }
            Tab { view: kind, split: None } => self.one_view(ui, function, kind, file.as_deref(), theme),
        }
    }

    fn one_view(&mut self, ui: &mut egui::Ui, function: &str, kind: ViewKind, file: Option<&str>, theme: Theme) {
        match kind {
            ViewKind::Diagram => self.diagram(ui, function, theme),
            ViewKind::Text => match file {
                Some(file) => self.text(ui, function, file, theme),
                None => {
                    ui.label(RichText::new("This function has no file").weak());
                }
            },
            ViewKind::Block => {
                ui.label(RichText::new("The view Block, the terminals and the icon of the function, comes with step 6 of M1.").weak());
            }
        }
    }

    fn diagram(&mut self, ui: &mut egui::Ui, function: &str, theme: Theme) {
        let view = self.view;
        let window = view.window_of(function);
        let Some(drawn) = self.ide.drawn.get_mut(function) else {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Click to show this function").weak()));
            return;
        };
        if drawn.canvas.drawing().is_none() {
            let text = if window.is_none_or(|window| window.kept) { "This function is not drawn now: see Problems" } else { "Drawing…" };
            ui.centered_and_justified(|ui| ui.label(RichText::new(text).weak()));
            return;
        }
        let shown = window.map(|window| canvas_shown(window, &self.ide.prefs)).unwrap_or_default();
        let response = drawn.canvas.show(ui, theme, &shown);
        if response.response.has_focus() {
            self.ide.canvas_focus = Some((function.to_owned(), response.response.id));
        } else if self.ide.canvas_focus.as_ref().is_some_and(|(focused, _)| focused == function) {
            self.ide.canvas_focus = None;
        }
        if window.is_none() {
            return;
        }
        // A gesture asks for a selection: the session holds it (`view.select`), and the canvas shows what it holds.
        for intent in response.intents {
            match intent {
                Intent::Select(selection) => self.ide.ask_select(self.host, function, selection),
                Intent::Move { nodes, by } => self.ide.ask_move(self.host, view, function, &nodes, by),
                // "Remove the fixed position", "Arrange this wire" (notation 2.4).
                Intent::Arrange(selection) => {
                    if let Some(file) = file_of(view, function) {
                        let params = if selection.wires.is_empty() {
                            json!({ "file": file, "nodes": selection.nodes })
                        } else {
                            json!({ "file": file, "wires": selection.wires })
                        };
                        let reply = self.host.send("layout.arrange", params);
                        self.ide.edits.push((function.to_owned(), reply, None));
                    }
                }
                // A wire dropped, or wired with Enter (notation 2.6): one step of undo; a branch with its junction, in one
                // step too (notation 2.4).
                Intent::Wire { from, to, replace, accept_mismatch, route } => {
                    if let Some(file) = file_of(view, function) {
                        let params = json!({ "file": file, "from": from, "to": to, "replace": replace, "accept_mismatch": accept_mismatch });
                        let reply = match route {
                            None => self.host.send("wire.connect", params),
                            Some((wire, via)) => {
                                let commands = json!([
                                    { "id": "wire.connect", "params": params },
                                    { "id": "wire.route", "params": { "file": file, "wire": wire, "via": via } },
                                ]);
                                self.host.send("commands.batch", json!({ "commands": commands, "label": "Branch Wire" }))
                            }
                        };
                        self.ide.edits.push((function.to_owned(), reply, None));
                    }
                }
                // A segment moved by hand (notation 2.4): every wire it moves, in one step.
                Intent::Route(routes) => {
                    if let Some(file) = file_of(view, function) {
                        let commands: Vec<Value> = routes
                            .into_iter()
                            .map(|(wire, via)| json!({ "id": "wire.route", "params": { "file": file, "wire": wire, "via": via } }))
                            .collect();
                        let reply = self.host.send("commands.batch", json!({ "commands": commands, "label": "Route Wire" }));
                        self.ide.edits.push((function.to_owned(), reply, None));
                    }
                }
                // Ctrl and a drag on a wired terminal: its wires to another terminal, in one step (notation 2.6).
                Intent::Rewire { disconnect, connect } => {
                    if let Some(file) = file_of(view, function) {
                        let commands: Vec<Value> = disconnect
                            .into_iter()
                            .map(|wire| json!({ "id": "wire.disconnect", "params": { "file": file, "wire": wire } }))
                            .chain(connect.into_iter().map(|(from, to, replace, accept_mismatch)| {
                                json!({ "id": "wire.connect", "params": { "file": file, "from": from, "to": to, "replace": replace, "accept_mismatch": accept_mismatch } })
                            }))
                            .collect();
                        let reply = self.host.send("commands.batch", json!({ "commands": commands, "label": "Move Wire" }));
                        self.ide.edits.push((function.to_owned(), reply, None));
                    }
                }
                // Quick insertion (notation 2.6): the list opens where the wire was dropped.
                Intent::QuickInsert(quick) => self.ide.open_quick(view, function, quick),
                // A function of the panel `Palette` dropped on the canvas (notation 2.5).
                Intent::Drop { call, at, into } => {
                    if let Some(file) = file_of(view, function) {
                        let params = match into {
                            Some(wire) => json!({ "file": file, "call": call, "into": wire }),
                            None => json!({ "file": file, "call": call, "at": [at.0, at.1] }),
                        };
                        let reply = self.host.send("node.add", params);
                        self.ide.edits.push((function.to_owned(), reply, Some(call)));
                    }
                }
                Intent::Say(text) => self.ide.say(text),
                // A field in place written (notation 2.6).
                Intent::Field { kind, text } => match file_of(view, function) {
                    Some(file) => {
                        let (id, params) = field_command(&file, kind, &text, self.ide.prefs.accept_mismatch);
                        let reply = self.host.send(id, params);
                        self.ide.field_edits.push((function.to_owned(), reply));
                    }
                    // Nothing sent: the field must not wait for an answer.
                    None => drawn_canvas_refused(self.ide, function),
                },
            }
        }
        if view.main.function.as_deref() == Some(function) {
            self.ide.canvas_rect = Some(response.response.rect);
        }
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

    fn text(&mut self, ui: &mut egui::Ui, function: &str, file: &str, theme: Theme) {
        let dark = theme == Theme::Dark;
        // The lines of what is selected in the diagram, tinted (notation 2.5): a node, and a wire's line that gives it
        // and the lines that read it.
        let lines = self.view.window_of(function).map(selected_lines).unwrap_or_default();
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
            // Laid out once: a label of the galley, which the user can select and copy, read only; the tint of the lines
            // selected under it, a row of the galley per line, since it never wraps.
            let tint = ui.painter().add(egui::Shape::Noop);
            let response = ui.add(egui::Label::new(galley.clone()).selectable(true).extend());
            let color = theme.color("select.stroke").gamma_multiply(0.18);
            let width = response.rect.x_range().max.max(ui.clip_rect().max.x);
            let rows: Vec<egui::Shape> = lines
                .iter()
                .filter_map(|&line| galley.rows.get(usize::try_from(line).ok()?.checked_sub(1)?))
                .map(|row| {
                    let area = row.rect().translate(response.rect.min.to_vec2());
                    egui::Shape::rect_filled(egui::Rect::from_x_y_ranges(response.rect.min.x..=width, area.y_range()), 0.0, color)
                })
                .collect();
            ui.painter().set(tint, egui::Shape::Vec(rows));
            if cut {
                ui.label(RichText::new(format!("The rest of the file is not shown: past {} MiB", text::MAX_TEXT / 1024 / 1024)).weak());
            }
        });
    }
}

#[cfg(test)]
mod field_tests {
    use super::*;
    use labcraft_editor::FieldKind;

    /// The command of each field in place (notation 2.6, commands.md 6.7), written by hand.
    #[test]
    fn each_field_sends_its_command() {
        let rename = field_command("f.lcf", FieldKind::Rename { value: "a".to_owned() }, "c", false);
        assert_eq!(rename, ("value.rename", json!({ "file": "f.lcf", "value": "a", "name": "c" })));
        let label = field_command("f.lcf", FieldKind::Label { line: 7 }, "#scale", false);
        assert_eq!(label, ("node.set_label", json!({ "file": "f.lcf", "line": 7, "label": "#scale" })));
        let set = field_command("f.lcf", FieldKind::Constant { to: "a.b".to_owned(), held: false }, "5 V", true);
        assert_eq!(set, ("input.set_constant", json!({ "file": "f.lcf", "to": "a.b", "value": "5 V", "replace": true, "accept_mismatch": true })));
        let clear = field_command("f.lcf", FieldKind::Constant { to: "a.b".to_owned(), held: true }, "", false);
        assert_eq!(clear, ("input.clear_constant", json!({ "file": "f.lcf", "to": "a.b" })));
        let template = field_command("f.lcf", FieldKind::Template { node: "text".to_owned() }, "{v} V", false);
        assert_eq!(template, ("node.set_template", json!({ "file": "f.lcf", "node": "text", "template": "\"{v} V\"" })));
    }

    #[test]
    fn a_template_typed_is_quoted_as_a_literal() {
        assert_eq!(quoted("{v} V"), "\"{v} V\"");
        assert_eq!(quoted("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quoted("kept \\\" and \\n"), "\"kept \\\" and \\n\"");
        assert_eq!(quoted("end \\"), "\"end \\\\\"");
    }
}
