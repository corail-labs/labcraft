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
use labcraft_engine::settings::{self as keyed, Stores, Values};
use labcraft_engine::view::{
    Published, Shown as WindowShown, Tab, ViewKind,
    window::{file_of_tab, tab_of},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

mod agent_view;
pub mod agents;
mod block;
pub mod catalog;
mod clipboard;
mod connect;
pub mod host;
mod insert;
pub mod keys;
pub mod pane;
pub mod panels;
mod pill;
mod running;
pub mod service;
mod settings_dialog;
pub mod text;
mod types;
mod values;

pub use host::{Host, LastStep, Reply, Snapshot};
pub use pane::Pane;

/// What the window keeps for the user (ide.md 1: the layout is the user's), which the app stores: the keys of the store
/// of the user (commands.md 9.7), and the state of the window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Where the panels are, without the tabs of functions; none, the layout of ide.md 2.
    pub layout: Option<Workspace<Pane>>,
    /// The keys of the store of the user, `group.name` → value (commands.md 9.7).
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub settings: Map<String, Value>,
    /// Where the last detached window was: the next one opens there (decision of the owner, 2026-11-16).
    pub detached: Option<Place>,
    /// A detached window shows its inspector beside its canvas, and its palette of functions on the other side (decision
    /// of the owner, 2026-11-16: the inspector shown, the palette hidden, until the user changes them).
    pub detached_inspector: bool,
    pub detached_palette: bool,
    /// The insertions of this user, the most recent first, which order quick insertion (notation 2.6): kept on their
    /// machine, with their settings.
    pub recent: Vec<String>,
    /// The settings that an earlier LabCraft kept as fields of its own, read once into `settings` and never written.
    #[serde(flatten, skip_serializing)]
    pub fields: Fields,
    /// `fields` were read into `settings`: they are never read again.
    pub fields_taken: bool,
}

/// The settings as an earlier LabCraft kept them, before the keys of commands.md 9.7.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fields {
    pub dark: Option<bool>,
    pub look: Option<String>,
    pub standard_everywhere: Option<bool>,
    pub reduced_motion: Option<bool>,
    pub preview_tabs: Option<bool>,
    pub values_shown: Option<bool>,
    pub record_values: Option<bool>,
    pub drag_pans: Option<bool>,
    pub fix_every_node: Option<bool>,
    pub mark_fixed: Option<bool>,
    pub replace_with_alt: Option<bool>,
    pub accept_mismatch: Option<bool>,
    pub route_with_shift_alt: Option<bool>,
    pub autopatch: Option<bool>,
    pub into_wire: Option<bool>,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            layout: None,
            settings: Map::new(),
            detached: None,
            detached_inspector: true,
            detached_palette: false,
            recent: Vec::new(),
            fields: Fields::default(),
            fields_taken: false,
        }
    }
}

impl Prefs {
    /// The settings that an earlier LabCraft kept as fields go into `settings`, once; a key already there wins, and a
    /// value that is the default is not written. Whether anything changed.
    pub fn take_fields(&mut self) -> bool {
        if self.fields_taken {
            return false;
        }
        self.fields_taken = true;
        let fields = std::mem::take(&mut self.fields);
        let theme = fields.dark.map(|dark| json!(if dark { "dark" } else { "light" }));
        let given = [
            (keyed::THEME, theme),
            (keyed::LOOK, fields.look.map(Value::String)),
            (keyed::STANDARD_EVERYWHERE, fields.standard_everywhere.map(Value::Bool)),
            (keyed::REDUCED_MOTION, fields.reduced_motion.map(Value::Bool)),
            (keyed::PREVIEW_TABS, fields.preview_tabs.map(Value::Bool)),
            (keyed::VALUES_SHOWN, fields.values_shown.map(Value::Bool)),
            (keyed::RECORD_VALUES, fields.record_values.map(Value::Bool)),
            (keyed::DRAG_PANS, fields.drag_pans.map(Value::Bool)),
            (keyed::FIX_EVERY_NODE, fields.fix_every_node.map(Value::Bool)),
            (keyed::MARK_FIXED, fields.mark_fixed.map(Value::Bool)),
            (keyed::REPLACE_WITH_ALT, fields.replace_with_alt.map(Value::Bool)),
            (keyed::ACCEPT_MISMATCH, fields.accept_mismatch.map(Value::Bool)),
            (keyed::ROUTE_WITH_SHIFT_ALT, fields.route_with_shift_alt.map(Value::Bool)),
            (keyed::AUTOPATCH, fields.autopatch.map(Value::Bool)),
            (keyed::INTO_WIRE, fields.into_wire.map(Value::Bool)),
        ];
        for (id, value) in given {
            let Some(value) = value else { continue };
            let Some(key) = keyed::find(id) else { continue };
            if value != key.default_value() && key.refuses(&value).is_none() && !self.settings.contains_key(id) {
                self.settings.insert(id.to_owned(), value);
            }
        }
        true
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
    /// `Connect an AI agent` (ide.md 8): write the entry of the MCP server in the MCP clients found, then give the lines
    /// of what was done with [`Ide::connected_clients`].
    ConnectAgent,
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
    /// What the settings give now (commands.md 9.7): those of the session with a project, else those the window keeps.
    settings: Values,
    /// The settings of the session last read.
    settings_seen: Option<Arc<Stores>>,
    /// The keys of the user that `settings` was computed from on the start page.
    settings_kept: Option<Map<String, Value>>,
    settings_dialog: settings_dialog::Dialog,
    /// What the app asks the user about the agents attached (ide.md 8).
    agent_questions: agents::Questions,
    /// What the window shows of the agents attached: the pill, the highlights, the dots on the tabs, the notice.
    agent_view: agent_view::AgentView,
    /// Whether the app connects MCP clients (`Connect an AI agent`): the desktop, not the web, which shows nothing of AI.
    connects_agents: bool,
    connect: connect::Connect,
    workspace: Workspace<Pane>,
    platform: Platform,
    keymap: Keymap,
    dispatcher: craft_keymap::egui::Dispatcher,
    palette: Palette,
    /// The detached function whose window the command palette opened in; none for the window of the project.
    palette_in: Option<String>,
    card: Card,
    frame: craft_window::Frame,
    /// The look given to egui last (`appearance.look`).
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
    /// The view `Block` of each function, what it keeps between two frames (ide.md 3).
    blocks: HashMap<String, block::BlockPane>,
    /// The text of each file that a view `Text` showed, edited there (ide.md 7).
    texts: HashMap<String, text::Editing>,
    /// `text.recovered`, asked once a project is open (architecture 12.6).
    recovery: Option<Reply>,
    recovery_asked: bool,
    callables: panels::Callables,
    problems: panels::ProblemsFilter,
    /// The field of "New function" in `Project`, and the answer it waits for (ide.md 4).
    new_function: panels::NewFunction,
    creating: Option<Reply>,
    /// The field of "Rename" in `Project`, and the answer it waits for (ide.md 4).
    renaming: panels::Renaming,
    renamed: Option<Reply>,
    /// A function created, by its file: shown once the session lists it.
    open_created: Option<String>,
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
    /// The gesture of a canvas in this frame, and the gesture last said to the host (architecture 12.5).
    canvas_gesture: Option<labcraft_engine::Gesture>,
    gesture_said: Option<labcraft_engine::Gesture>,
    /// The agent whose editing command waits for the end of the gesture, as the session last published it.
    waiting: Option<String>,
    /// The file whose view `Text` has the keyboard: its keys act on it (F12, Shift+Alt+F).
    text_focus: Option<String>,
    /// Where the pointer was last, on the screen: quick insertion opens under it (notation 2.6).
    pointer: Option<egui::Pos2>,
    /// The list of quick insertion, while it is open (notation 2.6).
    quick: Option<insert::QuickList>,
    /// The value that a structure being made needs, asked for (notation 2.6).
    values: Option<values::ValueList>,
    /// The list of types of a terminal of the function (notation 2.4).
    types: Option<types::TypeList>,
    /// The run the window started, followed to its end (decisions of the owner, 2026-11-16).
    running: Option<running::Running>,
    /// The time of the frame, on the clock of egui.
    now: f64,
    /// The loader of the SVG files of the icons is installed on the context that draws the window.
    loaders: bool,
    /// How the last run the window started of each function ended, for the `Run` panel.
    outcomes: HashMap<String, running::Outcome>,
    /// The copies sent, until their text comes (notation 2.6).
    copying: Vec<clipboard::Copying>,
    /// The pastes sent, by function: the nodes pasted are selected once done.
    pasting: Vec<(String, Reply)>,
    /// A paste asked by a menu or the palette, by function, until egui gives the text of the clipboard: whether egui is
    /// still to be asked for it, at the end of the frame, and when it was asked.
    paste_asked: Option<(String, bool, f64)>,
}

/// The id of the dock area, which names its windows of the system.
fn dock_id() -> egui::Id {
    egui::Id::unique("ide-dock")
}

impl Ide {
    /// The window, with the settings of the user. `frame`: who draws the frame of the window (`craft-window`).
    pub fn new(ctx: &egui::Context, mut prefs: Prefs, frame: craft_window::Frame) -> Ide {
        let platform = craft_keymap::egui::platform(ctx);
        let workspace = pane::restored(prefs.layout.clone());
        let prefs_changed = prefs.take_fields();
        let settings = Stores::of_user(&prefs.settings).values();
        ctx.set_theme(if settings.dark { egui::Theme::Dark } else { egui::Theme::Light });
        kit_shell::theme::install(ctx, kit_shell::theme::Look::named(&settings.look).unwrap_or_default());
        Ide {
            installed_look: settings.look.clone(),
            settings,
            settings_seen: None,
            settings_kept: None,
            settings_dialog: settings_dialog::Dialog::default(),
            agent_questions: agents::Questions::default(),
            agent_view: agent_view::AgentView::default(),
            connects_agents: false,
            connect: connect::Connect::default(),
            prefs,
            prefs_changed,
            workspace,
            platform,
            keymap: keys::keymap(platform),
            // Ctrl+C, Ctrl+X and Ctrl+V reach the keymap, which runs them on the canvas alone (notation 2.6); a text field
            // keeps them.
            dispatcher: craft_keymap::egui::Dispatcher::new().with_clipboard_events(true),
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
            blocks: HashMap::new(),
            texts: HashMap::new(),
            recovery: None,
            recovery_asked: false,
            callables: panels::Callables::default(),
            problems: panels::ProblemsFilter::default(),
            new_function: panels::NewFunction::default(),
            renaming: panels::Renaming::default(),
            renamed: None,
            creating: None,
            open_created: None,
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
            copying: Vec::new(),
            pasting: Vec::new(),
            paste_asked: None,
            templates: Vec::new(),
            failures: Vec::new(),
            canvas_rect: None,
            canvas_focus: None,
            canvas_gesture: None,
            gesture_said: None,
            waiting: None,
            text_focus: None,
            pointer: None,
            quick: None,
            values: None,
            types: None,
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

    /// The questions about the agents attached that wait for the user, as the app holds them (ide.md 8).
    pub fn set_agent_questions(&mut self, questions: Vec<agents::Question>) {
        self.agent_questions.set(questions);
    }

    /// The user's answers to the questions about the agents, since the last call.
    pub fn take_agent_answers(&mut self) -> Vec<(u64, agents::Answer)> {
        self.agent_questions.take_answers()
    }

    /// The agents attached, as the app holds them (ide.md 8): the pill shows them.
    pub fn set_agents(&mut self, connected: Vec<agents::Connected>) {
        self.agent_view.connected = connected;
    }

    /// What the user asked of the agents attached, from the pill, since the last call.
    pub fn take_agent_asks(&mut self) -> Vec<agents::Asked> {
        std::mem::take(&mut self.agent_view.asked)
    }

    /// Whether the window shows anything of the agents (ide.md 8): with a project whose session serves them, and AI on.
    fn agents_shown(&self, host: Option<&dyn Host>) -> bool {
        host.is_some_and(Host::serves_agents) && self.settings.ai
    }

    /// The app connects MCP clients to LabCraft (`Connect an AI agent`, ide.md 8): the desktop says so once.
    pub fn set_connects_agents(&mut self, connects: bool) {
        self.connects_agents = connects;
    }

    /// What `Connect an AI agent` did, a line for each client (mcp.md 9), shown until the user closes it.
    pub fn connected_clients(&mut self, lines: Vec<String>) {
        self.connect = connect::Connect::Done(lines);
    }

    /// Whether the window offers `Connect an AI agent` and the commands about the agents: where it shows something of
    /// AI, with a project or not.
    fn offers_agents(&self) -> bool {
        self.connects_agents && self.settings.ai
    }

    /// The agent attached that a command names (`agents.*`, #298).
    fn agent_named(&self, name: Option<&str>) -> Result<agents::Connected, String> {
        agents::named(&self.agent_view.connected, name).cloned()
    }

    /// The switch of AI (`ai.enabled`, ide.md 8): off, the app refuses the agents on its channel.
    pub fn ai_enabled(&self) -> bool {
        self.settings.ai
    }

    /// Says a problem that the app met, as a failure of a command is said.
    pub fn say(&mut self, problem: impl Into<String>) {
        self.failures.push(problem.into());
        let excess = self.failures.len().saturating_sub(3);
        self.failures.drain(..excess);
    }

    /// What the settings give, from the session with a project, else from those the window keeps: the canvases follow
    /// them, and the keys of the user that the session changed are kept (commands.md 9.7).
    fn follow_settings(&mut self, view: &Published, host: Option<&dyn Host>) {
        let settings = match host {
            Some(host) => {
                if self.settings_seen.as_ref().is_some_and(|seen| Arc::ptr_eq(seen, &view.settings)) {
                    return;
                }
                self.settings_seen = Some(view.settings.clone());
                self.settings_kept = None;
                if view.settings.given {
                    if view.settings.user != self.prefs.settings {
                        self.prefs.settings.clone_from(&view.settings.user);
                        self.prefs_changed = true;
                    }
                    view.settings.values()
                } else {
                    // A session that does not hold the keys of the user yet, a new one or a worker started again: given.
                    host.set_user_settings(&self.prefs.settings);
                    self.window_stores(view).values()
                }
            }
            None => {
                // The start page: the keys change only by `write_setting`.
                if self.settings_kept.as_ref() == Some(&self.prefs.settings) {
                    return;
                }
                self.settings_seen = None;
                self.settings_kept = Some(self.prefs.settings.clone());
                Stores::of_user(&self.prefs.settings).values()
            }
        };
        if settings == self.settings {
            return;
        }
        for drawn in self.drawn.values_mut() {
            drawn.canvas.set_reduced_motion(settings.reduced_motion);
            drawn.canvas.set_values_shown(settings.values_shown);
        }
        self.settings = settings;
    }

    /// The keys the window keeps, with those of the project the session read.
    fn window_stores(&self, view: &Published) -> Stores {
        Stores { project: view.settings.project.clone(), ..Stores::of_user(&self.prefs.settings) }
    }

    /// Writes a setting in the store of the user: by `settings.set` with a project, in the settings the window keeps
    /// without one (the start page).
    fn set_setting(&mut self, key: &str, value: Value, host: Option<&dyn Host>, view: &Published, outward: &mut Vec<Outward>) {
        // A toggle whose key the project holds is not enabled, from the menus, the palette or a shortcut (ide.md 2).
        if host.is_some() && view.settings.project.contains_key(key) {
            self.say(format!("The project holds `{key}`: change it in Settings"));
            return;
        }
        self.write_setting(key, value, false, host, view, outward);
    }

    fn write_setting(&mut self, key: &str, value: Value, project: bool, host: Option<&dyn Host>, view: &Published, outward: &mut Vec<Outward>) {
        if host.is_some() {
            let scope = if project { "project" } else { "user" };
            self.run(CommandRef::with_params("settings.set", json!({ "key": key, "value": value, "scope": scope })), host, view, outward);
            return;
        }
        let value = if value.is_null() { None } else { Some(value) };
        match keyed::set_user(&mut self.prefs.settings, key, value) {
            Ok(_) => self.prefs_changed = true,
            Err(why) => self.say(format!("Settings: {why}")),
        }
    }

    /// The `Settings` dialog, when it is open (ide.md 2).
    fn show_settings(&mut self, ctx: &egui::Context, host: Option<&dyn Host>, view: &Published, outward: &mut Vec<Outward>) {
        if !self.settings_dialog.is_open() {
            return;
        }
        let stores = match host {
            Some(_) if view.settings.given => Stores::clone(&view.settings),
            _ => self.window_stores(view),
        };
        let mut libraries: Vec<String> =
            self.callables.list.iter().map(|callable| callable.library.clone()).filter(|library| !library.is_empty()).collect();
        libraries.sort();
        libraries.dedup();
        for set in self.settings_dialog.show(ctx, &stores, host.is_some(), &libraries) {
            self.write_setting(set.key, set.value, set.project, host, view, outward);
        }
    }

    fn theme(&self) -> Theme {
        if self.settings.dark { Theme::Dark } else { Theme::Light }
    }

    /// The look of the preferences; a name it does not know is the default.
    fn look(&self) -> kit_shell::theme::Look {
        kit_shell::theme::Look::named(&self.settings.look).unwrap_or_default()
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
        self.text_focus = self
            .texts
            .values()
            .find(|editing| editing.field_id.is_some_and(|id| ctx.memory(|memory| memory.has_focus(id))))
            .map(|editing| editing.file.clone());
        let panel = match (&self.canvas_focus, &self.text_focus) {
            (Some(_), _) => Some(keys::DIAGRAM_SCOPE),
            (None, Some(_)) => Some(keys::TEXT_SCOPE),
            (None, None) => None,
        };
        let asked = self
            .asked_paste(ctx)
            .map(|(function, text)| CommandRef::with_params(keys::PASTE, json!({ "function": function, "text": text, "menu": true })));
        let hits = ctx.input_mut(|input| self.dispatcher.dispatch(input, &self.keymap, &Active { panel, contexts: &[], typing }));
        // Ctrl+V brings the text of the clipboard with it.
        let commands = hits.into_iter().map(|hit| match hit.pasted {
            Some(text) if hit.command.id == keys::PASTE => CommandRef::with_params(keys::PASTE, json!({ "text": text })),
            _ => hit.command,
        });
        asked.into_iter().chain(commands).collect()
    }

    /// The permissions of each pane where it is now (`pane::permissions_in`).
    fn permissions(&self) -> impl Fn(&Pane) -> Permissions + use<> {
        let floating = pane::floating(&self.workspace.layout);
        move |pane: &Pane| pane::permissions_in(pane, floating.contains(pane))
    }

    /// Draws the window in `ui`, the whole of the viewport: with no host, the start page alone.
    pub fn show(&mut self, ui: &mut egui::Ui, host: Option<&dyn Host>, start: &StartList) -> Vec<Outward> {
        let mut outward = Vec::new();
        let snapshot = host.map(Host::snapshot).unwrap_or_default();
        self.follow_settings(&snapshot.view, host);
        if self.installed_look != self.settings.look {
            self.installed_look = self.settings.look.clone();
            kit_shell::theme::install(ui.ctx(), self.look());
        }
        let theme = if self.settings.dark { egui::Theme::Dark } else { egui::Theme::Light };
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
        self.new_project(&snapshot);
        let view = snapshot.view.clone();
        self.canvas_gesture = None;
        self.waiting.clone_from(&snapshot.waiting);
        // What the window shows of the agents (ide.md 8): nothing of it with AI off.
        if self.agents_shown(host) {
            self.agent_view.follow(&snapshot.steps, &view, self.now);
        } else {
            self.agent_view.clear(&snapshot.steps);
        }
        self.follow_focus(ui.ctx());
        if let Some(host) = host {
            self.take_replies(host, &view);
            self.follow_texts(host, &view, ui.ctx());
            self.take_clips(ui.ctx(), host);
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
        if let Some((_, ask, _)) = &mut self.paste_asked
            && std::mem::take(ask)
        {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        }
        let menus = self.menus(&view, host.is_some());
        let entries = if self.palette.is_open() { catalog::palette_entries(&self.keymap, host.is_some(), self.offers_agents()) } else { Vec::new() };
        let workspaces = catalog::workspaces();
        let palette_key = keys::key_of(&self.keymap, &CommandRef::new(keys::PALETTE));
        let right: Vec<Segment> =
            if self.agents_shown(host) { vec![pill::segment(&self.agent_view.connected, self.waiting.as_deref())] } else { Vec::new() };
        // The interface of the window (decisions of the owner, 2026-11-16): the theme, then the detached mode to come.
        let tools = [
            Tool { icon: Icon::Settings, hover: "Settings".to_owned(), command: keys::SETTINGS.to_owned(), planned: false },
            if self.settings.dark {
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
                Answer::Opens(command, below) if command == pill::COMMAND => self.agent_view.pill.toggle(below),
                Answer::Opens(command, _) => self.run(CommandRef::new(command), host, &view, &mut outward),
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
        let status_bar = egui::Panel::bottom("ide-status")
            .frame(kit_shell::theme::bar_frame(&tokens).inner_margin(egui::Margin::symmetric(8, 3)))
            .show(ui, |ui| kit_shell::status::show(ui, &status.0, &status.1));
        // The notice of an agent stands over the right end of the status bar.
        let notice_corner = status_bar.response.rect.right_top();
        if let Some(command) = status_bar.inner {
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
        if let Some(host) = host {
            self.show_types(ui.ctx(), host, &view);
        }
        let values_elsewhere = self.values.as_ref().is_some_and(|list| self.native.contains(&list.function));
        if let Some(host) = host
            && !values_elsewhere
        {
            self.show_values(ui.ctx(), host);
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
        self.show_settings(ui.ctx(), host, &view, &mut outward);
        self.connect.show(ui.ctx());
        // Over everything, the settings included: the agent waits for the answer.
        if let Some(policies) = self.agent_questions.show(ui.ctx(), &self.prefs.settings) {
            self.write_setting(keyed::AI_POLICIES, policies, false, host, &view, &mut outward);
        }
        if let Some(host) = host.filter(|&host| self.agents_shown(Some(host))) {
            self.show_agents(ui.ctx(), host, &snapshot, notice_corner, &mut outward);
        }
        // The content reaches under the title bar of macOS: a drag that starts on a control of the top bar must not
        // move the window.
        #[cfg(target_os = "macos")]
        craft_window::title_bar::sync_movable(ui.ctx(), &mut self.movable);
        self.say_gesture(ui.ctx(), host, &view);
        outward
    }

    /// The menu of the pill and the notice of the last step of an agent (ide.md 8), and what they ask.
    fn show_agents(&mut self, ctx: &egui::Context, host: &dyn Host, snapshot: &Snapshot, notice_corner: egui::Pos2, outward: &mut Vec<Outward>) {
        let view = snapshot.view.clone();
        let agent_view::AgentView { pill, connected, .. } = &mut self.agent_view;
        for out in pill.show(ctx, connected, &self.prefs.settings) {
            match out {
                pill::Out::Command(command) => self.run(command, Some(host), &view, outward),
                pill::Out::Forget(policies) => self.write_setting(keyed::AI_POLICIES, policies, false, Some(host), &view, outward),
                pill::Out::Settings => self.settings_dialog.open(),
            }
        }
        let undoable = snapshot.steps.first().map(|step| step.number) == self.agent_view.notice.as_ref().map(|notice| notice.step);
        match self.agent_view.show_notice(ctx, self.now, undoable, notice_corner) {
            Some(agent_view::Clicked::Show(function)) => {
                if view.is_detached(&function) {
                    self.raise = Some(function.clone());
                } else if view.main.function.as_deref() != Some(function.as_str()) {
                    let reply = host.send("view.show_function", json!({ "function": function }));
                    self.asking_show = Some((function.clone(), reply));
                }
                if let Some(touched) = self.agent_view.notice.as_ref().and_then(|notice| notice.touched.clone()) {
                    self.agent_view.again = Some((function, touched));
                }
            }
            // That step, or nothing when another came since (`file.stale`).
            Some(agent_view::Clicked::Undo(step)) => {
                self.run(CommandRef::with_params("history.undo", json!({ "step": step })), Some(host), &view, outward);
            }
            None => {}
        }
    }

    /// Tells the host when a gesture of the user starts and ends, from what the frame drew: a canvas in a gesture, or a
    /// field that holds a text typed and not written (architecture 12.5; decision of the owner, 2026-11-21).
    fn say_gesture(&mut self, ctx: &egui::Context, host: Option<&dyn Host>, view: &Published) {
        // The text of a tab closed is kept, but no field shows it: it holds nothing.
        let open = |file: &str| {
            let detached = view.detached.iter().filter_map(|shown| shown.function.as_deref());
            view.opened.iter().map(String::as_str).chain(detached).any(|function| file_of_tab(view, function) == Some(file))
        };
        // Taken in every frame, so that what a field said in one frame never counts in the next.
        let inspector = kit_shell::inspector::take_typing(ctx);
        let typing = self.drawn.values().any(|drawn| drawn.canvas.typing())
            || self.texts.iter().any(|(file, editing)| editing.typing() && open(file))
            || self.blocks.values().any(block::BlockPane::typing)
            || self.renaming.typing()
            || self.new_function.typing()
            || inspector;
        let gesture = self.canvas_gesture.or(typing.then_some(labcraft_engine::Gesture::Field));
        if let Some(host) = host
            && gesture != self.gesture_said
        {
            host.set_gesture(gesture);
            self.gesture_said = gesture;
        }
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
        // A new session knows of no gesture.
        self.gesture_said = None;
        self.project.clone_from(&snapshot.project);
        self.session = snapshot.session;
        self.drawn.clear();
        self.blocks.clear();
        self.texts.clear();
        self.recovery = None;
        self.recovery_asked = false;
        self.callables = panels::Callables::default();
        self.asking_show = None;
        self.brought = None;
        self.focus = None;
        self.placing.clear();
        self.palette_in = None;
        self.history = History::default();
        // The steps of another session count from 1 again.
        self.agent_view.reset();
        // A run of the project before is not followed into this session, whose runs count from the start again.
        self.running = None;
        self.outcomes.clear();
    }

    /// The texts of the views `Text`, shown or not: their replies, a read after a check, a write after a pause of the keys
    /// (commands.md 9.1); and the texts of the recovery folder, once the project is open.
    fn follow_texts(&mut self, host: &dyn Host, view: &Published, ctx: &egui::Context) {
        if view.folder.is_some() && !std::mem::replace(&mut self.recovery_asked, true) {
            self.recovery = Some(host.send("text.recovered", json!({})));
        }
        if let Some(result) = self.recovery.as_ref().and_then(Reply::take) {
            self.recovery = None;
            let mut kept = Vec::new();
            for found in result.ok().as_ref().and_then(|found| found.get("texts")).and_then(Value::as_array).into_iter().flatten() {
                let (Some(path), Some(text)) = (found.get("path").and_then(Value::as_str), found.get("text").and_then(Value::as_str)) else {
                    continue;
                };
                let editing = self.texts.entry(path.to_owned()).or_insert_with(|| text::Editing::new(path));
                editing.recovered = Some(text.to_owned());
                kept.push(format!("`{path}`"));
            }
            if !kept.is_empty() {
                self.say(format!("A text that did not read was kept for {}: its view Text offers it back", kept.join(", ")));
            }
        }
        let mut waiting = false;
        for editing in self.texts.values_mut() {
            let focused = editing.field_id.is_some_and(|id| ctx.memory(|memory| memory.has_focus(id)));
            waiting |= editing.update(host, view.checks, view.report.as_ref(), self.now, focused);
        }
        if waiting {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// The results that came: the failures to say, the function shown, the history.
    fn take_replies(&mut self, host: &dyn Host, view: &Published) {
        // "New function": the field closes when it is written, and the function shows once the session lists it.
        if let Some(answer) = self.creating.as_ref().and_then(Reply::take) {
            self.creating = None;
            match answer {
                Ok(result) => {
                    // A type opens in no tab before the view `Text` (step 7).
                    if !self.new_function.is_type {
                        self.open_created = result
                            .get("files")
                            .and_then(|files| files.get(0))
                            .and_then(|file| file.get("file"))
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                    }
                    self.new_function.close();
                }
                Err(error) => self.new_function.refused(&error.message),
            }
        }
        // "Rename": the field closes when it is written; the tab and the window of a function follow its name.
        if let Some(answer) = self.renamed.as_ref().and_then(Reply::take) {
            self.renamed = None;
            match answer {
                Ok(_) => self.renaming.close(),
                Err(error) => self.renaming.refused(&error.message),
            }
        }
        if let Some(file) = &self.open_created
            && let Some(listed) = view.functions.iter().find(|listed| listed.file == *file)
        {
            let function = listed.function.clone();
            self.open_created = None;
            let reply = host.send("view.show_function", json!({ "function": function }));
            self.pending.push(("view.show_function".to_owned(), reply));
        }
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
                self.blocks.remove(function);
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
        let reduced = self.settings.reduced_motion;
        let values_shown = self.settings.values_shown;
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
                    let before = drawn.canvas.drawing().cloned();
                    let first = before.is_none();
                    // The glide when the file of the function changed; the first drawing of a tab is fitted.
                    drawn.canvas.set_drawing(drawing.clone(), shown.glide && !first);
                    if first {
                        drawn.canvas.fit();
                        drawn.fit_values = true;
                    }
                    // A step of an agent: what it touched, highlighted for a moment (ide.md 8).
                    if let Some(before) = before
                        && self.agent_view.is_pending(function)
                    {
                        let touched = labcraft_editor::touched(&before, drawing);
                        // A drawing that changes nothing, drawn again for another reason, leaves it waiting.
                        if !touched.is_empty() {
                            self.agent_view.drawn(function, &touched);
                            drawn.canvas.highlight(touched, self.now);
                        }
                    }
                }
            }
            // Show, on the notice: highlighted again once the function is shown.
            if self.agent_view.again.as_ref().is_some_and(|(again, _)| again == function)
                && drawn.canvas.drawing().is_some()
                && let Some((_, touched)) = self.agent_view.again.take()
            {
                drawn.canvas.highlight(touched, self.now);
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
        let mut bar = craft_menu::layout(&keys::menus(self.offers_agents()), Target::current(), &LayoutOptions::new("LabCraft")).bar;
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
                | keys::COLOR_BY_TYPE
                | keys::REPLACE_WITH_ALT
                | keys::ACCEPT_MISMATCH
                | keys::ROUTE_WITH_SHIFT_ALT
                | keys::ABOUT => {
                    let checked = match id {
                        keys::DARK => Some(self.settings.dark),
                        keys::LAB => Some(self.look() == kit_shell::theme::Look::Lab),
                        keys::REDUCED_MOTION => Some(self.settings.reduced_motion),
                        keys::DRAG_PANS => Some(self.settings.drag_pans),
                        keys::FIX_EVERY_NODE => Some(self.settings.fix_every_node),
                        keys::MARK_FIXED => Some(self.settings.mark_fixed),
                        keys::COLOR_BY_TYPE => Some(self.settings.by_type),
                        keys::REPLACE_WITH_ALT => Some(self.settings.replace_with_alt),
                        keys::ACCEPT_MISMATCH => Some(self.settings.accept_mismatch),
                        keys::ROUTE_WITH_SHIFT_ALT => Some(self.settings.route_with_shift_alt),
                        _ => None,
                    };
                    // A key that the project holds is changed in the dialog (ide.md 2).
                    let held = keys::setting_of(id).is_some_and(|key| view.settings.project.contains_key(key));
                    ItemState { enabled: !held, checked }
                }
                keys::CONNECT_AGENT => ItemState { enabled: !self.connect.is_open(), checked: None },
                keys::PLANNED_INSTRUMENTS
                | keys::PLANNED_PAUSE
                | keys::PLANNED_BREAKPOINT
                | keys::PLANNED_WATCH
                | keys::PLANNED_BUILD
                | keys::PLANNED_DETACHED_MODE => ItemState { enabled: true, checked: None },
                keys::RECORD => ItemState { enabled: true, checked: Some(self.settings.record_values) },
                keys::SETTINGS => ItemState { enabled: true, checked: None },
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
                keys::VALUES => ItemState { enabled: project, checked: Some(self.settings.values_shown) },
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
                | keys::COPY
                | keys::CUT
                | keys::PASTE
                | keys::DUPLICATE
                | keys::SURROUND_MATCH
                | keys::SURROUND_HANDLE
                | keys::SURROUND_FOR
                | keys::SURROUND_LOOP
                | keys::ADD_ARM
                | keys::ADD_CARRY
                | keys::ADD_CLAUSE
                | keys::CONTINUE_ON_ERROR
                | keys::PARALLEL
                | keys::RENAME => ItemState { enabled: shown.is_some(), checked: None },
                keys::DEFINITION | keys::FORMAT => ItemState { enabled: self.text_file(view, shown).is_some(), checked: None },
                keys::AUTOPATCH => {
                    ItemState { enabled: !view.settings.project.contains_key(keyed::AUTOPATCH), checked: Some(self.settings.autopatch) }
                }
                keys::INTO_WIRE => {
                    ItemState { enabled: !view.settings.project.contains_key(keyed::INTO_WIRE), checked: Some(self.settings.into_wire) }
                }
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

    /// The status bar: the file shown, the last step; color by type, the zoom. Of the window that has focus.
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
        if shown.is_some() {
            let (text, hover) = if self.settings.by_type {
                ("Types in colour", "Wires in the colour of their type: click for one ink (View › Color wires by type)")
            } else {
                ("Types in one ink", "Wires in one ink: click for the colour of their type (View › Color wires by type)")
            };
            right.push(Segment::new(text).hover(hover).command(keys::COLOR_BY_TYPE));
        }
        if let Some(drawn) = shown.and_then(|function| self.drawn.get(function)) {
            let (zoom, _) = drawn.canvas.viewport();
            right.push(Segment::new(format!("{:.0} %", zoom * 100.0)).hover("Fit the diagram to the view").command(keys::FIT));
        }
        (left, right)
    }

    fn start_page(&self, ui: &mut egui::Ui, start: &StartList) -> Option<start::Chosen> {
        let actions = start_actions(&self.keymap, self.offers_agents());
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
                if let Some(action) = start_actions(&self.keymap, self.offers_agents()).get(at) {
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
            // A place of a file: in its view Text, when its tab shows one.
            panels::Asked::ShowAt { function, file, line, column } => {
                // Only in a view Text shown: a place kept for later would move the cursor unasked.
                let tab = tab_of(view, &function);
                if (tab.view == ViewKind::Text || tab.split == Some(ViewKind::Text))
                    && let Some(editing) = self.texts.get_mut(&file)
                {
                    editing.reveal = Some(editing.place_of(line, column));
                }
                self.panel_asked(panels::Asked::Show { function, preview: false }, host, view, outward);
            }
            panels::Asked::Command(id, params) => self.run(CommandRef::with_params(id, params), host, view, outward),
            panels::Asked::Window(id) => self.run(CommandRef::new(id), host, view, outward),
            panels::Asked::PickType { file, terminal, default, add } => {
                let purpose = if add { types::Purpose::Add { default } } else { types::Purpose::Set };
                self.open_types(view, &file, &terminal, purpose);
            }
            panels::Asked::NewFunction { path } => match host {
                Some(host) => self.creating = Some(host.send("function.create", json!({ "path": path }))),
                None => self.new_function.refused("open a project first"),
            },
            panels::Asked::NewType { path, definition } => match host {
                Some(host) => self.creating = Some(host.send("type.create", json!({ "path": path, "definition": definition }))),
                None => self.new_function.refused("open a project first"),
            },
            panels::Asked::PickDefinition { path } => self.open_definitions(&path),
            panels::Asked::Rename { file, name } => match host {
                Some(host) => {
                    let id = if file.ends_with(".lct") { "type.rename" } else { "function.rename" };
                    self.renamed = Some(host.send(id, json!({ "file": file, "name": name })));
                }
                None => self.renaming.refused("open a project first"),
            },
        }
    }

    /// Ends the runs of Ctrl and the arrows of every canvas: the moves they ask for.
    fn end_runs(&mut self, host: &dyn Host, view: &Published) {
        let ended: Vec<(String, Intent)> =
            self.drawn.iter_mut().filter_map(|(function, drawn)| Some((function.clone(), drawn.canvas.end_run()?))).collect();
        for (function, intent) in ended {
            match intent {
                Intent::Move { nodes, by } => self.ask_move(host, view, &function, &nodes, by),
                Intent::MoveTerminal { terminal, y } => {
                    self.send_edit(host, view, &function, "terminal.move", json!({ "terminal": terminal, "y": y }))
                }
                _ => {}
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
        let fixing: Vec<Value> = match (&drawing, self.settings.fix_every_node) {
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
        let shown = view.window_of(function).map(|window| canvas_shown(window, &self.settings)).unwrap_or_default();
        // The values a structure could look at: only quick insertion on the empty canvas lists the structures.
        let seen = match quick.source {
            labcraft_editor::Source::Nothing => {
                self.drawn.get(function).map(|drawn| drawn.canvas.values_seen(&shown, &Selection::default(), Some(quick.screen))).unwrap_or_default()
            }
            _ => Vec::new(),
        };
        self.quick = Some(insert::QuickList::new(function.to_owned(), file, quick, canvas, seen));
    }

    /// "Surround with ▸" a structure of `kind` the nodes selected on the canvas of `function` (notation 2.6): at once for
    /// a `loop`, after the list of values for the others.
    /// `nodes`: those of the context menu; none, the selection.
    fn surround(&mut self, host: &dyn Host, view: &Published, function: &str, kind: &'static str, nodes: Option<Vec<String>>) {
        let Some(file) = file_of(view, function) else { return };
        let Some(window) = view.window_of(function) else { return };
        let shown = canvas_shown(window, &self.settings);
        let selection = match nodes {
            Some(nodes) => Selection { nodes, ..Selection::default() },
            None => shown.selection.clone(),
        };
        if selection.nodes.is_empty() {
            self.say("Surround with: select the nodes to put in the structure");
            return;
        }
        let around = values::Around::Nodes(selection.nodes.clone());
        if kind == "loop" {
            let reply = host.send("structure.surround", values::command(&file, kind, &around, None));
            self.edits.push((function.to_owned(), reply, None));
            return;
        }
        let Some(drawn) = self.drawn.get(function) else { return };
        let seen = drawn.canvas.values_seen(&shown, &selection, None);
        let screen = drawn.canvas.selection_screen(&selection).unwrap_or_default();
        let canvas = self.canvas_focus.as_ref().filter(|(focused, _)| focused == function).map(|(_, id)| *id);
        self.values = Some(values::ValueList::new(function.to_owned(), file, kind, around, screen, seen, canvas));
    }

    /// An editing command on the file of `function`, one step: `params` with its `file`.
    fn send_edit(&mut self, host: &dyn Host, view: &Published, function: &str, id: &str, mut params: Value) {
        let Some(file) = file_of(view, function) else { return };
        // Its text does not parse: nothing edits the function until it does (commands.md 9.1).
        if self.texts.get(&file).is_some_and(text::Editing::locks_diagram) {
            self.say("The text does not read: fix it in the view Text");
            return;
        }
        if let Some(object) = params.as_object_mut() {
            object.insert("file".to_owned(), json!(file));
        }
        let reply = host.send(id, params);
        self.edits.push((function.to_owned(), reply, None));
    }

    /// "Add clause…" of a `for` (notation 2.6): the list of the values that its frame sees, as surrounding with a `for`
    /// lists them, where the `for` is.
    fn add_clause(&mut self, view: &Published, function: &str, structure: String) {
        let Some(file) = file_of(view, function) else { return };
        let Some(window) = view.window_of(function) else { return };
        let shown = canvas_shown(window, &self.settings);
        let Some(drawn) = self.drawn.get(function) else { return };
        let selection = Selection { nodes: vec![structure.clone()], ..Selection::default() };
        let seen = drawn.canvas.values_seen(&shown, &selection, None);
        let screen = drawn.canvas.selection_screen(&selection).unwrap_or_default();
        let canvas = self.canvas_focus.as_ref().filter(|(focused, _)| focused == function).map(|(_, id)| *id);
        let around = values::Around::Clause { structure };
        self.values = Some(values::ValueList::new(function.to_owned(), file, "for", around, screen, seen, canvas));
    }

    /// The list of values, while it is open: Enter makes the structure, one step (notation 2.6).
    /// The list of types open, and the command of the type chosen (notation 2.4).
    fn show_types(&mut self, ctx: &egui::Context, host: &dyn Host, view: &Published) {
        let Some(mut list) = self.types.take() else { return };
        match list.show(ctx) {
            types::Closed::Open => self.types = Some(list),
            types::Closed::Cancelled => {}
            types::Closed::Picked(id, params) => {
                let reply = host.send(id, params);
                let function = list.function.clone();
                // The definition of a new type: its answer goes to the field of "New type".
                if id == "type.create" {
                    self.new_function.pending = true;
                    self.creating = Some(reply);
                } else if view.functions.iter().any(|listed| listed.function == function) {
                    self.edits.push((function, reply, None));
                } else {
                    self.pending.push((id.to_owned(), reply));
                }
            }
        }
    }

    /// Opens the list of types for the definition of the new type of `path`, where the pointer is: an alias of the one
    /// chosen.
    fn open_definitions(&mut self, path: &str) {
        let project: Vec<String> = self
            .callables
            .list
            .iter()
            .filter(|callable| callable.kind == "type")
            .map(|callable| if callable.library.is_empty() { callable.name.clone() } else { format!("{}.{}", callable.library, callable.name) })
            .collect();
        let at = self.pointer.or_else(|| self.canvas_rect.map(|rect| rect.center())).unwrap_or_default();
        self.types = Some(types::TypeList::new(String::new(), path.to_owned(), String::new(), types::Purpose::Define, project, at));
    }

    /// Opens the list of types for `terminal` of the function of `file`, where the pointer is.
    fn open_types(&mut self, view: &Published, file: &str, terminal: &str, purpose: types::Purpose) {
        let Some(function) = view.functions.iter().find(|listed| listed.file == file).map(|listed| listed.function.clone()) else { return };
        let project: Vec<String> = self
            .callables
            .list
            .iter()
            .filter(|callable| callable.kind == "type")
            .map(|callable| if callable.library.is_empty() { callable.name.clone() } else { format!("{}.{}", callable.library, callable.name) })
            .collect();
        let at = self.pointer.or_else(|| self.canvas_rect.map(|rect| rect.center())).unwrap_or_default();
        self.types = Some(types::TypeList::new(function, file.to_owned(), terminal.to_owned(), purpose, project, at));
    }

    fn show_values(&mut self, ctx: &egui::Context, host: &dyn Host) {
        let Some(mut list) = self.values.take() else { return };
        match list.show(ctx) {
            values::Chosen::Open => {
                self.values = Some(list);
                return;
            }
            values::Chosen::Cancelled => {}
            values::Chosen::Value(value) => {
                let reply = host.send(values::command_id(&list.around), values::command(&list.file, list.kind, &list.around, Some(&value)));
                self.edits.push((list.function.clone(), reply, None));
            }
        }
        if let Some(canvas) = list.canvas {
            ctx.memory_mut(|memory| memory.request_focus(canvas));
        }
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
            // A structure that needs a value asks for it first, where the list was.
            insert::Closed::Picked(insert::Picked::Structure(kind)) if kind != "loop" => {
                let around = values::Around::Empty { at: list.quick.at.unwrap_or_default(), within: list.quick.within.clone() };
                let seen = std::mem::take(&mut list.seen);
                self.values =
                    Some(values::ValueList::new(list.function.clone(), list.file.clone(), kind, around, list.quick.screen, seen, list.canvas));
                return;
            }
            insert::Closed::Picked(picked) => {
                let (id, params) = insert::command(&list.file, &list.quick, &picked, &catalog);
                let reply = host.send(&id, params);
                let call = match &picked {
                    insert::Picked::Entry(entry) => Some(entry.call.clone()),
                    insert::Picked::Constant(_) | insert::Picked::Structure(_) => None,
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
        let selection = canvas_shown(window, &self.settings).selection;
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

    /// The file of the view `Text` that a key or a menu acts on: the one that has the keyboard, else that of the function
    /// shown when its tab shows the view `Text`.
    fn text_file(&self, view: &Published, shown: Option<&str>) -> Option<String> {
        if let Some(file) = &self.text_focus {
            return Some(file.clone());
        }
        let function = shown?;
        let tab = tab_of(view, function);
        let texts = tab.view == ViewKind::Text || tab.split == Some(ViewKind::Text);
        texts.then(|| file_of(view, function)).flatten().filter(|file| self.texts.contains_key(file))
    }

    /// Runs a command of a menu, a key, the palette, a button: an action of the window here, a command of the session
    /// through the host. What takes the function shown takes that of the window that has focus.
    fn run(&mut self, command: CommandRef, host: Option<&dyn Host>, view: &Published, outward: &mut Vec<Outward>) {
        let CommandRef { id, params } = command;
        let mut params = match params {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        // `null` of `signature.set_default` makes the input required, and that of `settings.set` takes the key out: it
        // is a value there.
        if id != "signature.set_default" && id != "settings.set" {
            params.retain(|_, value| !value.is_null());
        }
        let shown = self.target(view).function.clone();
        // A function whose text does not parse is not edited until it does (commands.md 9.1), whatever asks for it.
        let edits = matches!(
            id.as_str(),
            keys::ARRANGE
                | keys::WIRE
                | keys::QUICK_INSERT
                | keys::DELETE
                | keys::RENAME
                | keys::CUT
                | keys::PASTE
                | keys::DUPLICATE
                | keys::AUTOPATCH
                | keys::SURROUND_MATCH
                | keys::SURROUND_HANDLE
                | keys::SURROUND_FOR
                | keys::SURROUND_LOOP
                | keys::ADD_ARM
                | keys::ADD_CARRY
                | keys::ADD_CLAUSE
                | keys::CONTINUE_ON_ERROR
                | keys::PARALLEL
                | keys::INTO_WIRE
                | keys::FIX_EVERY_NODE
                | keys::MARK_FIXED
        ) || labcraft_engine::registry::command(&id)
            .is_some_and(|command| command.undoable && !command.window && !command.id.starts_with("history."));
        let target =
            params.get("file").and_then(Value::as_str).map(str::to_owned).or_else(|| shown.as_deref().and_then(|function| file_of(view, function)));
        if edits && target.and_then(|file| self.texts.get(&file)).is_some_and(text::Editing::locks_diagram) {
            self.say("The text does not read: fix it in the view Text");
            return;
        }
        match id.as_str() {
            // From the palette or a menu, with no path: the field of "New function" or "New type" in `Project`, at the root
            // (ide.md 4); with no file, "Rename" of the function shown, there.
            "function.create" | "type.create" | "function.rename" if !params.contains_key("path") && !params.contains_key("file") => {
                match id.as_str() {
                    "function.create" => self.new_function.open(""),
                    "type.create" => self.new_function.open_type(""),
                    _ => match shown.as_deref().and_then(|function| view.functions.iter().find(|listed| listed.function == function)) {
                        Some(listed) => {
                            let name = labcraft_editor::own_name(&listed.function).to_owned();
                            self.renaming.open(&listed.file, &name);
                        }
                        None => {
                            self.say("Rename: show a function first");
                            return;
                        }
                    },
                }
                if !self.workspace.layout.contains(&Pane::Project)
                    && self.workspace.set_visible(Pane::Project, true, None, self.permissions()).is_ok()
                {
                    self.prefs_changed = true;
                }
            }
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
                            self.running = Some(running::Running::start(host, &function, self.now, self.settings.record_values));
                        }
                    }
                }
            }
            keys::RECORD => self.set_setting(keyed::RECORD_VALUES, json!(!self.settings.record_values), host, view, outward),
            keys::SETTINGS => self.settings_dialog.open(),
            keys::CONNECT_AGENT => {
                if !self.offers_agents() {
                    self.say("Connect an AI agent: AI is off, or this window does not connect MCP clients");
                } else if !self.connect.is_open() {
                    self.connect = connect::Connect::Writing;
                    outward.push(Outward::ConnectAgent);
                }
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
                let theme = if self.settings.dark { "light" } else { "dark" };
                self.set_setting(keyed::THEME, json!(theme), host, view, outward);
            }
            keys::LAB => {
                let look = if self.look() == kit_shell::theme::Look::Lab { kit_shell::theme::Look::Studio } else { kit_shell::theme::Look::Lab };
                self.set_setting(keyed::LOOK, json!(look.name()), host, view, outward);
            }
            keys::REDUCED_MOTION => self.set_setting(keyed::REDUCED_MOTION, json!(!self.settings.reduced_motion), host, view, outward),
            keys::VALUES => self.set_setting(keyed::VALUES_SHOWN, json!(!self.settings.values_shown), host, view, outward),
            keys::FIT => {
                if let Some(drawn) = shown.as_ref().and_then(|function| self.drawn.get_mut(function)) {
                    drawn.canvas.fit();
                }
            }
            keys::DETACHED_INSPECTOR => {
                self.prefs.detached_inspector = !self.prefs.detached_inspector;
                self.prefs_changed = true;
            }
            keys::DRAG_PANS => self.set_setting(keyed::DRAG_PANS, json!(!self.settings.drag_pans), host, view, outward),
            keys::FIX_EVERY_NODE => self.set_setting(keyed::FIX_EVERY_NODE, json!(!self.settings.fix_every_node), host, view, outward),
            keys::MARK_FIXED => self.set_setting(keyed::MARK_FIXED, json!(!self.settings.mark_fixed), host, view, outward),
            keys::COLOR_BY_TYPE => self.set_setting(keyed::COLOR_BY_TYPE, json!(!self.settings.by_type), host, view, outward),
            keys::REPLACE_WITH_ALT => self.set_setting(keyed::REPLACE_WITH_ALT, json!(!self.settings.replace_with_alt), host, view, outward),
            keys::ACCEPT_MISMATCH => self.set_setting(keyed::ACCEPT_MISMATCH, json!(!self.settings.accept_mismatch), host, view, outward),
            keys::ROUTE_WITH_SHIFT_ALT => {
                self.set_setting(keyed::ROUTE_WITH_SHIFT_ALT, json!(!self.settings.route_with_shift_alt), host, view, outward);
            }
            // W (notation 2.6): a wire from the terminal that has the focus, on the canvas that has the keyboard.
            keys::WIRE => {
                let Some(function) = self.canvas_focus.as_ref().map(|(function, _)| function.clone()).or(shown) else { return };
                let canvas_shown = view.window_of(&function).map(|window| canvas_shown(window, &self.settings)).unwrap_or_default();
                let started = self.drawn.get_mut(&function).is_some_and(|drawn| drawn.canvas.start_wire(&canvas_shown));
                if !started {
                    self.say("W starts a wire from the terminal in focus: select a node, then Enter goes to its terminals");
                }
            }
            keys::AUTOPATCH => self.set_setting(keyed::AUTOPATCH, json!(!self.settings.autopatch), host, view, outward),
            keys::INTO_WIRE => self.set_setting(keyed::INTO_WIRE, json!(!self.settings.into_wire), host, view, outward),
            // Space (notation 2.6): quick insertion from what the canvas that has the keyboard holds.
            keys::QUICK_INSERT => {
                let Some(function) = self.canvas_focus.as_ref().map(|(function, _)| function.clone()).or(shown) else { return };
                let Some(window) = view.window_of(&function) else { return };
                let canvas_shown = canvas_shown(window, &self.settings);
                let pointer = self.pointer;
                let autopatch = self.settings.autopatch;
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
            // Copy, cut and duplicate (notation 2.6): the nodes selected on the canvas that has the keyboard.
            keys::SURROUND_MATCH | keys::SURROUND_HANDLE | keys::SURROUND_FOR | keys::SURROUND_LOOP => {
                let function = self.canvas_focus.as_ref().map(|(function, _)| function.clone()).or(shown);
                let (Some(host), Some(function), Some(kind)) = (host, function, keys::surround_kind(&id)) else { return };
                self.surround(host, view, &function, kind, None);
            }
            // The parts of the one structure selected (notation 2.6): as its context menu gives them.
            keys::ADD_ARM | keys::ADD_CARRY | keys::ADD_CLAUSE | keys::CONTINUE_ON_ERROR | keys::PARALLEL => {
                let function = self.canvas_focus.as_ref().map(|(function, _)| function.clone()).or(shown);
                let (Some(function), Some(asked)) = (function, keys::part_asked(&id)) else { return };
                let Some(window) = view.window_of(&function) else { return };
                let canvas_shown = canvas_shown(window, &self.settings);
                let said = self.drawn.get_mut(&function).map(|drawn| drawn.canvas.ask_part(&canvas_shown, asked));
                if let Some(Err(why)) = said {
                    self.say(why);
                }
            }
            keys::COPY | keys::CUT | keys::DUPLICATE => {
                let function = self.canvas_focus.as_ref().map(|(function, _)| function.clone()).or(shown);
                let (Some(host), Some(function)) = (host, function) else { return };
                let clip = match id.as_str() {
                    keys::CUT => clipboard::Clip::Cut,
                    keys::DUPLICATE => clipboard::Clip::Duplicate,
                    _ => clipboard::Clip::Copy,
                };
                self.copy(host, view, &function, clip);
            }
            // Paste (notation 2.6): Ctrl+V brings its text; a menu asks egui for the text of the clipboard first.
            keys::PASTE => {
                let function = params.get("function").and_then(Value::as_str).map(str::to_owned);
                let function = function.or_else(|| self.canvas_focus.as_ref().map(|(function, _)| function.clone())).or(shown);
                let (Some(host), Some(function)) = (host, function) else { return };
                match params.get("text").and_then(Value::as_str) {
                    // From a menu, the pointer is on the menu: the paste falls in the middle of the view.
                    Some(text) => {
                        let pointer = if params.get("menu").and_then(Value::as_bool) == Some(true) { None } else { self.pointer };
                        self.paste(host, view, &function, text, pointer);
                    }
                    None => self.paste_asked = Some((function, true, self.now)),
                }
            }
            // F12 (decision of the owner, 2026-11-21): where the name at the cursor of the view Text is defined.
            keys::DEFINITION => {
                let Some(file) = self.text_file(view, shown.as_deref()) else {
                    self.say("Go to definition: put the cursor on a name in the view Text");
                    return;
                };
                // The view asks its service at its next frame, where it knows the project and the drawing.
                if let Some(editing) = self.texts.get_mut(&file) {
                    editing.go_to_definition = true;
                }
            }
            // Shift+Alt+F in the view Text (decision of the owner, 2026-11-21): `file.format` of its file, once the text
            // typed is written.
            keys::FORMAT => {
                let (Some(host), Some(file)) = (host, self.text_file(view, shown.as_deref())) else {
                    self.say("Format file: show the view Text of a function");
                    return;
                };
                if let Some(why) = self.texts.get_mut(&file).and_then(|editing| editing.format(host)) {
                    self.say(why);
                }
            }
            // F2 (notation 2.6): the name of the node selected, or its label, in a field in place.
            keys::RENAME => {
                let Some(function) = self.canvas_focus.as_ref().map(|(function, _)| function.clone()).or(shown) else { return };
                let opened = self.drawn.get_mut(&function).is_some_and(|drawn| drawn.canvas.edit_name());
                if !opened {
                    self.say("F2 renames the node or the terminal of the frame selected, or the function when nothing is: select one");
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
                    let selection = canvas_shown(window, &self.settings).selection;
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
                let canvas_shown = canvas_shown(window, &self.settings);
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
            // The agents attached are the app's (ide.md 8; #298): the window asks it, as from the pill.
            agents::DISCONNECT => match self.agent_named(params.get("agent").and_then(Value::as_str)) {
                Ok(agent) => self.agent_view.asked.push(agents::Asked::Disconnect(agent.id)),
                Err(why) => self.say(format!("{id}: {why}")),
            },
            agents::SET_POLICY => {
                let agent = match self.agent_named(params.get("agent").and_then(Value::as_str)) {
                    Ok(agent) => agent,
                    Err(why) => {
                        self.say(format!("{id}: {why}"));
                        return;
                    }
                };
                // The first connection is answered in its dialog (ide.md 8), not kept before it.
                if agent.policy.is_none() {
                    self.say(format!("{id}: {} asks to connect: answer in the dialog of the window", agent.signed));
                    return;
                }
                let Some(policy) = params.get("policy").and_then(Value::as_str).and_then(agents::Policy::named) else {
                    self.say("agents.set_policy: `policy` is one of ask, edits, all");
                    return;
                };
                self.agent_view.asked.push(agents::Asked::Policy(agent.id, policy));
                if params.get("keep").and_then(Value::as_bool) == Some(true) {
                    let kept = self.prefs.settings.get(keyed::AI_POLICIES).cloned().unwrap_or(Value::Null);
                    let policies = labcraft_engine::settings::keep_policy(&kept, &agent.client, &agent.project, policy.as_str());
                    self.write_setting(keyed::AI_POLICIES, policies, false, host, view, outward);
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
        // The parts of a structure (notation 2.6).
        FieldKind::AddArm { structure } => ("structure.add_arm", json!({ "file": file, "structure": structure, "patterns": text })),
        FieldKind::Patterns { structure, arm } => {
            ("structure.set_patterns", json!({ "file": file, "structure": structure, "arm": arm, "patterns": text }))
        }
        FieldKind::AddCarry { structure } => ("structure.add_carry", carry_params(file, &structure, text)),
        // The signature (notation 2.4).
        FieldKind::TerminalName { terminal } => ("signature.rename", json!({ "file": file, "terminal": terminal, "name": text })),
        // An empty field makes the input required.
        FieldKind::TerminalDefault { terminal } => {
            let default = if text.is_empty() { Value::Null } else { json!(text) };
            ("signature.set_default", json!({ "file": file, "terminal": terminal, "default": default }))
        }
        FieldKind::AddTerminal { input } => ("signature.add", terminal_params(file, input, text)),
        FieldKind::FunctionName => ("function.rename", json!({ "file": file, "name": text })),
    }
}

/// `name: type` or `name: type = default` as `signature.add` takes them; without `: type`, the command says what is
/// missing.
pub(crate) fn terminal_params(file: &str, input: bool, text: &str) -> Value {
    let (head, default) = match text.split_once('=') {
        Some((head, default)) => (head.trim(), Some(default.trim()).filter(|default| !default.is_empty())),
        None => (text.trim(), None),
    };
    let (name, ty) = match head.split_once(':') {
        Some((name, ty)) => (name.trim(), Some(ty.trim()).filter(|ty| !ty.is_empty())),
        None => (head, None),
    };
    let side = if input { "input" } else { "output" };
    let mut params = json!({ "file": file, "terminal": format!("{side} {name}") });
    if let Some(object) = params.as_object_mut() {
        if let Some(ty) = ty {
            object.insert("type".to_owned(), json!(ty));
        }
        if let Some(default) = default {
            object.insert("default".to_owned(), json!(default));
        }
    }
    params
}

/// `name = initial`, `name: type = initial` or `name: type` as `structure.add_carry` takes them: a hole, or nothing after
/// `=`, starts the carry from a hole.
fn carry_params(file: &str, structure: &str, text: &str) -> Value {
    let (head, initial) = match text.split_once('=') {
        Some((head, initial)) => (head.trim(), Some(initial.trim()).filter(|initial| !initial.is_empty() && *initial != "?")),
        None => (text.trim(), None),
    };
    let (name, ty) = match head.split_once(':') {
        Some((name, ty)) => (name.trim(), Some(ty.trim()).filter(|ty| !ty.is_empty())),
        None => (head, None),
    };
    let mut params = json!({ "file": file, "structure": structure, "name": name });
    if let Some(object) = params.as_object_mut() {
        if let Some(initial) = initial {
            object.insert("initial".to_owned(), json!(initial));
        }
        if let Some(ty) = ty {
            object.insert("type".to_owned(), json!(ty));
        }
    }
    params
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
const EDITS: &[&str] = &[
    "function.rename",
    "node.move",
    "node.place",
    "node.set_view",
    "layout.arrange",
    "frame.resize",
    "terminal.move",
    "function.set_doc",
    "signature.add",
    "signature.remove",
    "signature.rename",
    "signature.set_type",
    "signature.set_default",
    "block.set_terminal",
    "block.set_size",
    "block.set_band",
    "block.set_color",
    "block.set_fill",
    "block.set_stroke",
    "block.reset",
];

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
fn canvas_shown(window: &WindowShown, settings: &Values) -> Shown {
    let selection = &window.selection;
    let selection = if window.function.as_deref() == Some(selection.function.as_str()) {
        Selection { nodes: selection.nodes.clone(), wires: selection.wires.clone(), terminals: selection.terminals.clone() }
    } else {
        Selection::default()
    };
    Shown {
        arms: window.arms.clone(),
        selection,
        drag_pans: settings.drag_pans,
        mark_fixed: settings.mark_fixed,
        replace_with_alt: settings.replace_with_alt,
        accept_mismatch: settings.accept_mismatch,
        route_with_shift_alt: settings.route_with_shift_alt,
        into_wire: settings.into_wire,
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

/// The actions of the start page (ide.md 10); `Connect an AI agent` where the window shows something of AI.
fn start_actions(keymap: &Keymap, agents: bool) -> Vec<palette::Entry> {
    let key = |id: &str| keys::key_of(keymap, &CommandRef::new(id));
    let entry = |id: &str, label: &str| palette::Entry { key: key(id), ..palette::Entry::new(id, label) };
    let mut actions = vec![entry("project.new", "New project…"), entry("project.open", "Open folder…")];
    actions.push(entry(keys::SETTINGS, "Settings…"));
    if agents {
        actions.push(entry(keys::CONNECT_AGENT, "Connect an AI agent…"));
    }
    actions
}

/// Where a definition takes the view (decision of the owner, 2026-11-21).
enum Going {
    /// A place of the text shown.
    Here(kit_text::lsp_types::Position),
    /// Another function of the project: its tab.
    Function(String),
    /// A type, whose `.lct` has no tab yet.
    Type { name: String, file: String },
}

fn going(view: &Published, from: &str, found: kit_text::lsp_types::GotoDefinitionResponse) -> Option<Going> {
    use kit_text::lsp_types::GotoDefinitionResponse;
    let (uri, at) = match found {
        GotoDefinitionResponse::Scalar(location) => (location.uri, location.range.start),
        GotoDefinitionResponse::Array(locations) => locations.into_iter().next().map(|location| (location.uri, location.range.start))?,
        GotoDefinitionResponse::Link(links) => links.into_iter().next().map(|link| (link.target_uri, link.target_selection_range.start))?,
    };
    let file = service::path_of(&uri)?;
    if file == from {
        return Some(Going::Here(at));
    }
    if let Some(listed) = view.functions.iter().find(|listed| listed.file == file) {
        return Some(Going::Function(listed.function.clone()));
    }
    view.types.iter().find(|listed| listed.file == file).map(|listed| Going::Type { name: listed.name.clone(), file: listed.file.clone() })
}

fn no_tab(name: &str, file: &str) -> String {
    format!("The type `{name}` is defined in `{file}`, which has no tab yet")
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
        // A tab in preview has its title in italics (ide.md 3); one that an agent edited while it was not shown, a dot
        // (ide.md 8).
        let Pane::Function(function) = pane else { return logical.into() };
        let text = if self.ide.agent_view.dots.contains(function) { RichText::new(format!("{logical} ●")) } else { RichText::new(logical) };
        if self.view.preview.as_deref() == Some(function.as_str()) { text.italics().into() } else { text.into() }
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
        if self.ide.values.as_ref().is_some_and(|list| list.function == function) {
            self.ide.show_values(&ctx, self.host);
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
            Pane::Project => panels::project(
                ui,
                view,
                focused.function.as_deref(),
                self.ide.settings.preview_tabs,
                &mut self.ide.new_function,
                &mut self.ide.renaming,
            ),
            Pane::Palette => {
                panels::palette(ui, &mut self.ide.callables);
                None
            }
            Pane::Inspector => panels::inspector(ui, view, &view.main, &self.ide.callables),
            Pane::Problems => {
                let unparsed: Vec<panels::Unparsed<'_>> = self
                    .ide
                    .texts
                    .values()
                    .filter_map(|editing| editing.unparsed().map(|(message, line)| panels::Unparsed { file: &editing.file, message, line }))
                    .collect();
                panels::problems(ui, view, focused, &mut self.ide.problems, &unparsed)
            }
            Pane::Run => {
                let said = self.ide.run_said(focused.function.as_deref(), ui.input(|input| input.time));
                panels::run(ui, focused, self.ide.settings.values_shown, said.as_ref())
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
            let record = self.ide.settings.record_values;
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
            ViewKind::Block => self.block(ui, function, file, theme),
        }
    }

    fn diagram(&mut self, ui: &mut egui::Ui, function: &str, theme: Theme) {
        let view = self.view;
        let window = view.window_of(function);
        // Its text does not parse: the diagram shows the file, and cannot be edited (commands.md 9.1).
        let locked = file_of(view, function).and_then(|file| self.ide.texts.get(&file)).is_some_and(text::Editing::locks_diagram);
        let Some(drawn) = self.ide.drawn.get_mut(function) else {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Click to show this function").weak()));
            return;
        };
        if drawn.canvas.drawing().is_none() {
            let text = if window.is_none_or(|window| window.kept) { "This function is not drawn now: see Problems" } else { "Drawing…" };
            ui.centered_and_justified(|ui| ui.label(RichText::new(text).weak()));
            return;
        }
        let shown = window.map(|window| canvas_shown(window, &self.ide.settings)).unwrap_or_default();
        let response = if locked {
            ui.label(RichText::new("The text does not read: fix it in the view Text").color(ui.visuals().warn_fg_color));
            ui.add_enabled_ui(false, |ui| drawn.canvas.show(ui, theme, &shown)).inner
        } else {
            drawn.canvas.show(ui, theme, &shown)
        };
        if let Some(gesture) = response.gesture {
            self.ide.canvas_gesture = Some(engine_gesture(gesture));
        }
        if response.response.has_focus() {
            self.ide.canvas_focus = Some((function.to_owned(), response.response.id));
        } else if self.ide.canvas_focus.as_ref().is_some_and(|(focused, _)| focused == function) {
            self.ide.canvas_focus = None;
        }
        if window.is_none() || locked {
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
                        let params = if !selection.terminals.is_empty() {
                            json!({ "file": file, "terminals": selection.terminals })
                        } else if selection.wires.is_empty() {
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
                Intent::Wire { from, to, arm, replace, accept_mismatch, route, through } => {
                    if let Some(file) = file_of(view, function) {
                        let mut params = json!({ "file": file, "from": from, "to": to, "replace": replace, "accept_mismatch": accept_mismatch });
                        // Out of loops: how each takes the value out (notation 2.6).
                        if let (false, Some(object)) = (through.is_empty(), params.as_object_mut()) {
                            object.insert("through".to_owned(), json!(through));
                        }
                        let params = insert::with_arm(params, arm.as_deref());
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
                            .map(|(wire, arm)| json!({ "id": "wire.disconnect", "params": insert::with_arm(json!({ "file": file, "wire": wire }), arm.as_deref()) }))
                            .chain(connect.into_iter().map(|(from, to, replace, accept_mismatch, arm)| {
                                let params = json!({ "file": file, "from": from, "to": to, "replace": replace, "accept_mismatch": accept_mismatch });
                                json!({ "id": "wire.connect", "params": insert::with_arm(params, arm.as_deref()) })
                            }))
                            .collect();
                        let reply = self.host.send("commands.batch", json!({ "commands": commands, "label": "Move Wire" }));
                        self.ide.edits.push((function.to_owned(), reply, None));
                    }
                }
                // Quick insertion (notation 2.6): the list opens where the wire was dropped.
                Intent::QuickInsert(quick) => self.ide.open_quick(view, function, quick),
                // A function of the panel `Palette` dropped on the canvas (notation 2.5).
                Intent::Drop { call, at, within, into } => {
                    if let Some(file) = file_of(view, function) {
                        let params = match into {
                            Some(wire) => json!({ "file": file, "call": call, "into": wire }),
                            None => insert::within(json!({ "file": file, "call": call, "at": [at.0, at.1] }), &within),
                        };
                        let reply = self.host.send("node.add", params);
                        self.ide.edits.push((function.to_owned(), reply, Some(call)));
                    }
                }
                Intent::Say(text) => self.ide.say(text),
                // The parts of a structure (notation 2.6).
                Intent::RemovePart { structure, part } => {
                    self.ide.send_edit(self.host, view, function, "structure.remove_part", json!({ "structure": structure, "part": part }));
                }
                // An edge of a frame dragged (notation 2.6): its size, from its origin; "Remove the fixed size" (2.4): none.
                Intent::Resize { structure, size } => {
                    let mut params = json!({ "size": size.map(|(width, height)| [width, height]) });
                    if let (Some(structure), Some(object)) = (structure, params.as_object_mut()) {
                        object.insert("inside".to_owned(), json!(structure));
                    }
                    self.ide.send_edit(self.host, view, function, "frame.resize", params);
                }
                Intent::SetPassage { structure, part, to } => {
                    let params = json!({ "structure": structure, "part": part, "to": to });
                    self.ide.send_edit(self.host, view, function, "structure.set_passage", params);
                }
                Intent::SetOption { structure, option, on } => {
                    let params = json!({ "structure": structure, "option": option, "on": on });
                    self.ide.send_edit(self.host, view, function, "structure.set_option", params);
                }
                Intent::AddClause { structure, over: None } => self.ide.add_clause(view, function, structure),
                // A wire dropped on an edge of a loop (notation 2.6): a clause, a `collect` or a `last`, at once.
                Intent::AddClause { structure, over: Some(over) } => {
                    self.ide.send_edit(self.host, view, function, "structure.add_clause", json!({ "structure": structure, "over": over }));
                }
                Intent::AddCollect { structure, value, last } => {
                    let command = if last { "structure.add_last" } else { "structure.add_collect" };
                    self.ide.send_edit(self.host, view, function, command, json!({ "structure": structure, "value": value }));
                }
                // "Surround with ▸" of the context menu of a node (notation 2.6).
                Intent::Surround { kind, nodes } => self.ide.surround(self.host, view, function, kind, Some(nodes)),
                // A terminal of a node made an input or an output of the function, in one step (notation 2.4).
                Intent::MakeTerminal { terminal, ty, from, to } => {
                    if let Some(file) = file_of(view, function) {
                        let label = if terminal.starts_with("input ") { "Make Input" } else { "Make Output" };
                        let commands = json!([
                            { "id": "signature.add", "params": { "file": file, "terminal": terminal, "type": ty } },
                            { "id": "wire.connect", "params": { "file": file, "from": from, "to": to } },
                        ]);
                        let reply = self.host.send("commands.batch", json!({ "commands": commands, "label": label }));
                        self.ide.edits.push((function.to_owned(), reply, None));
                    }
                }
                Intent::RemoveTerminal { terminal } => {
                    self.ide.send_edit(self.host, view, function, "signature.remove", json!({ "terminal": terminal }));
                }
                Intent::MoveTerminal { terminal, y } => {
                    self.ide.send_edit(self.host, view, function, "terminal.move", json!({ "terminal": terminal, "y": y }));
                }
                // The file of the function called, not the one shown: every call to it changes (notation 2.4).
                Intent::BlockTerminal { file, terminal, side, place } => {
                    let reply = self.host.send("block.set_terminal", json!({ "file": file, "terminal": terminal, "side": side, "place": place }));
                    self.ide.edits.push((function.to_owned(), reply, None));
                }
                Intent::ResetBlock { file } => {
                    let reply = self.host.send("block.reset", json!({ "file": file }));
                    self.ide.edits.push((function.to_owned(), reply, None));
                }
                Intent::PickType { terminal } => {
                    if let Some(file) = file_of(view, function) {
                        self.ide.open_types(view, &file, &terminal, types::Purpose::Set);
                    }
                }
                // A new terminal named with no type: the list of types (decision of the owner, 2026-11-19).
                Intent::Field { kind: labcraft_editor::FieldKind::AddTerminal { input }, text } if !types::has_type(&text) => {
                    if let (Some(file), Some(drawn)) = (file_of(view, function), self.ide.drawn.get_mut(function)) {
                        drawn.canvas.close_field();
                        let (name, default) = types::name_and_default(&text);
                        if name.is_empty() {
                            continue;
                        }
                        let side = if input { "input" } else { "output" };
                        self.ide.open_types(view, &file, &format!("{side} {name}"), types::Purpose::Add { default });
                    }
                }
                // A field in place written (notation 2.6).
                Intent::Field { kind, text } => match file_of(view, function) {
                    Some(file) => {
                        let (id, params) = field_command(&file, kind, &text, self.ide.settings.accept_mismatch);
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

    /// The view `Block` (ide.md 3): the block of the drawing of the function, and the commands of the block it sends.
    fn block(&mut self, ui: &mut egui::Ui, function: &str, file: Option<&str>, theme: Theme) {
        let Some(drawing) = self.ide.drawn.get(function).and_then(|drawn| drawn.canvas.drawing().cloned()) else {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Click to show this function").weak()));
            return;
        };
        let Some(own) = drawing.block.as_deref() else {
            ui.centered_and_justified(|ui| ui.label(RichText::new("This function has no block to show now: see Problems").weak()));
            return;
        };
        let diagnostics = match (&self.view.report, file) {
            (Some(report), Some(file)) => block::diagnostics_of(report.diagnostics(), file, &own.manifest),
            _ => Vec::new(),
        };
        let pane = self.ide.blocks.entry(function.to_owned()).or_default();
        for asked in pane.show(ui, &drawing, theme, &diagnostics) {
            match asked {
                block::Asked::Command(id, params) => self.ide.send_edit(self.host, self.view, function, id, params),
                block::Asked::Say(text) => self.ide.say(text),
            }
        }
    }

    fn text(&mut self, ui: &mut egui::Ui, function: &str, file: &str, theme: Theme) {
        let dark = theme == Theme::Dark;
        // The lines of what is selected in the diagram, tinted (notation 2.5): a node, and a wire's line that gives it
        // and the lines that read it.
        let lines = self.view.window_of(function).map(selected_lines).unwrap_or_default();
        let select = theme.color("select.stroke");
        let tint = select.gamma_multiply(0.18);
        let tints: Vec<(usize, egui::Color32)> =
            lines.iter().filter_map(|&line| usize::try_from(line).ok()?.checked_sub(1)).map(|line| (line, tint)).collect();
        let editing = self.ide.texts.entry(file.to_owned()).or_insert_with(|| text::Editing::new(file));
        if editing.loading() {
            editing.update(self.host, self.view.checks, self.view.report.as_ref(), self.ide.now, false);
        }
        let drawing = self.ide.drawn.get(function).and_then(|drawn| drawn.canvas.drawing()).map(|drawing| drawing.as_ref());
        let known = text::Known { callables: &self.ide.callables.list, functions: &self.view.functions, types: &self.view.types, drawing };
        let said = editing.show(ui, dark, select, &tints, &known);
        if let Some(asked) = said.asked {
            editing.answer(self.host, asked);
        }
        match said.definition.and_then(|found| going(self.view, file, found)) {
            Some(Going::Here(at)) => editing.reveal = Some(kit_text::offset_of(&editing.code.field, at)),
            Some(Going::Function(function)) => self.asked.push(panels::Asked::Show { function, preview: false }),
            Some(Going::Type { name, file }) => self.ide.say(no_tab(&name, &file)),
            None if said.nowhere => self.ide.say("Go to definition: no definition of a value, a function or a type of the project there"),
            None => {}
        }
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
        // The parts of a structure (notation 2.6).
        let arm = field_command("f.lcf", FieldKind::AddArm { structure: "m".to_owned() }, "1, 3..=5", false);
        assert_eq!(arm, ("structure.add_arm", json!({ "file": "f.lcf", "structure": "m", "patterns": "1, 3..=5" })));
        let patterns = field_command("f.lcf", FieldKind::Patterns { structure: "m".to_owned(), arm: "_".to_owned() }, "2", false);
        assert_eq!(patterns, ("structure.set_patterns", json!({ "file": "f.lcf", "structure": "m", "arm": "_", "patterns": "2" })));
        let carry = |text: &str| field_command("f.lcf", FieldKind::AddCarry { structure: "#l".to_owned() }, text, false).1;
        assert_eq!(carry("acc = 0 V"), json!({ "file": "f.lcf", "structure": "#l", "name": "acc", "initial": "0 V" }));
        assert_eq!(carry("acc: f64 = ?"), json!({ "file": "f.lcf", "structure": "#l", "name": "acc", "type": "f64" }));
        assert_eq!(carry("n: i32 = x"), json!({ "file": "f.lcf", "structure": "#l", "name": "n", "initial": "x", "type": "i32" }));
        assert_eq!(carry("acc"), json!({ "file": "f.lcf", "structure": "#l", "name": "acc" }));
        // The signature (notation 2.4).
        let terminal = |kind: FieldKind, text: &str| field_command("f.lcf", kind, text, false);
        assert_eq!(
            terminal(FieldKind::TerminalName { terminal: "input x".to_owned() }, "volts"),
            ("signature.rename", json!({ "file": "f.lcf", "terminal": "input x", "name": "volts" }))
        );
        assert_eq!(
            terminal(FieldKind::TerminalDefault { terminal: "input x".to_owned() }, "2 V"),
            ("signature.set_default", json!({ "file": "f.lcf", "terminal": "input x", "default": "2 V" }))
        );
        assert_eq!(
            terminal(FieldKind::TerminalDefault { terminal: "input x".to_owned() }, ""),
            ("signature.set_default", json!({ "file": "f.lcf", "terminal": "input x", "default": null }))
        );
        assert_eq!(terminal(FieldKind::FunctionName, "twice"), ("function.rename", json!({ "file": "f.lcf", "name": "twice" })));
        let add = |input: bool, text: &str| terminal(FieldKind::AddTerminal { input }, text).1;
        assert_eq!(add(true, "gain: f64 = 2"), json!({ "file": "f.lcf", "terminal": "input gain", "type": "f64", "default": "2" }));
        assert_eq!(add(false, "count : i64"), json!({ "file": "f.lcf", "terminal": "output count", "type": "i64" }));
        assert_eq!(add(true, "gain"), json!({ "file": "f.lcf", "terminal": "input gain" }));
    }

    #[test]
    fn a_template_typed_is_quoted_as_a_literal() {
        assert_eq!(quoted("{v} V"), "\"{v} V\"");
        assert_eq!(quoted("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quoted("kept \\\" and \\n"), "\"kept \\\" and \\n\"");
        assert_eq!(quoted("end \\"), "\"end \\\\\"");
    }
}

/// A gesture of the canvas, as the session names it (architecture 12.5).
fn engine_gesture(gesture: labcraft_editor::Gesture) -> labcraft_engine::Gesture {
    match gesture {
        labcraft_editor::Gesture::Rectangle => labcraft_engine::Gesture::Rectangle,
        labcraft_editor::Gesture::Move => labcraft_engine::Gesture::Move,
        labcraft_editor::Gesture::Wire => labcraft_engine::Gesture::Wire,
        labcraft_editor::Gesture::Route => labcraft_engine::Gesture::Route,
        labcraft_editor::Gesture::Resize => labcraft_engine::Gesture::Resize,
    }
}

#[cfg(test)]
mod prefs_tests {
    use super::{Prefs, keyed};
    use serde_json::json;

    /// The settings of an earlier LabCraft, kept as fields, go into the keys once: a value that is the default is not
    /// written, and a key already there wins (commands.md 9.7).
    #[test]
    fn the_fields_of_an_earlier_labcraft_become_keys_once() {
        let earlier = r#"{ "dark": true, "look": "studio", "drag_pans": true, "mark_fixed": false, "preview_tabs": true }"#;
        let mut prefs: Prefs = serde_json::from_str(earlier).unwrap();
        prefs.settings.insert(keyed::DRAG_PANS.to_owned(), json!(false));
        assert!(prefs.take_fields());
        assert_eq!(prefs.settings.get(keyed::THEME), Some(&json!("dark")));
        assert_eq!(prefs.settings.get(keyed::MARK_FIXED), Some(&json!(false)));
        assert_eq!(prefs.settings.get(keyed::DRAG_PANS), Some(&json!(false)), "the key already there");
        assert!(!prefs.settings.contains_key(keyed::LOOK) && !prefs.settings.contains_key(keyed::PREVIEW_TABS), "defaults");
        assert!(!prefs.take_fields(), "once");
        let written = serde_json::to_value(&prefs).unwrap();
        assert!(written.get("dark").is_none() && written.get("fields_taken") == Some(&json!(true)), "{written}");
    }
}
