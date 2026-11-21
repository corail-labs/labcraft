//! The state of the windows of the IDE that the session holds (`docs/ide.md`; `docs/commands.md`, 6.7, "The window";
//! `docs/architecture.md`, 12.5): the view of each tab, the tab in preview, the detached windows, and for each window
//! its selection and its zoom, and the commands of the window that change them. The layout of the panels is a setting
//! of the user that the window keeps, not a state of the session (`view.toggle_panel`, `view.reset_layout`); a project
//! is opened by the app, which makes its session (`project.new`, `project.open`).

use serde_json::{Value, json};

use super::{Published, View, Which, Window, choose, compose_report, draw, file_for, known, read_again_if_stale};
use crate::error::CommandError;
use crate::params::Params;
use crate::session::Session;

/// The commands of the window that a session runs only when a window shows it (commands.md 6.7, "The window").
pub const WINDOW_ONLY: &[&str] = &[
    "project.new",
    "project.open",
    "view.get_state",
    "view.select",
    "view.set_view",
    "view.close_function",
    "view.detach",
    "view.dock",
    "view.toggle_panel",
    "view.reset_layout",
    "view.set_workspace",
    "text.save",
    "text.recovered",
    "text.discard",
];

/// The commands of the window that the window runs itself, never the session: a project is opened by the app, which
/// makes its session, and the layout of the panels is a setting of the user.
pub const RUN_BY_THE_WINDOW: &[&str] = &["project.new", "project.open", "view.toggle_panel", "view.reset_layout"];

/// At most this many items of a selection, in `view.select` and in what `view.get_state` returns *(proposed)*.
pub const MAX_SELECTED: usize = 1_000;

/// A view of a tab (ide.md 3). `Panel` is planned (M3), and is never one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewKind {
    #[default]
    Diagram,
    Text,
    Block,
}

impl ViewKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ViewKind::Diagram => "diagram",
            ViewKind::Text => "text",
            ViewKind::Block => "block",
        }
    }

    pub fn parse(name: &str) -> Option<ViewKind> {
        match name {
            "diagram" => Some(ViewKind::Diagram),
            "text" => Some(ViewKind::Text),
            "block" => Some(ViewKind::Block),
            _ => None,
        }
    }
}

/// How a tab shows its function: a view, and a second one beside it (ide.md 3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tab {
    pub view: ViewKind,
    pub split: Option<ViewKind>,
}

/// The selection of a window, in the function it shows, named as the editing commands name what they edit
/// (commands.md 9.2).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub function: String,
    pub nodes: Vec<String>,
    pub wires: Vec<String>,
    pub terminals: Vec<String>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.wires.is_empty() && self.terminals.is_empty()
    }
}

/// The view of the tab of a function: the diagram until it is set.
pub fn tab_of(published: &Published, function: &str) -> Tab {
    published.tabs.get(function).copied().unwrap_or_default()
}

/// Whether the function has a tab, among the tabs of the window of the project or in a window of its own.
fn has_tab(view: &View, name: &str) -> bool {
    view.now.opened.iter().any(|opened| opened == name) || view.is_detached(name)
}

/// What the window keeps of the tabs once some closed: a tab closed has no view, no preview, no selection.
pub(super) fn forget_closed(view: &mut View) {
    let detached: Vec<String> = view.detached_functions().map(str::to_owned).collect();
    let now = &mut view.now;
    let opened = &now.opened;
    now.tabs.retain(|function, _| opened.contains(function) || detached.contains(function));
    view.zooms.retain(|function, _| opened.contains(function) || detached.contains(function));
    if now.preview.as_ref().is_some_and(|preview| !opened.contains(preview)) {
        now.preview = None;
    }
    if !opened.contains(&view.main.shown.selection.function) {
        view.main.shown.selection = Selection::default();
    }
}

/// `view.show_function` with `preview`: the tab of a function shown in preview takes the place of the tab in preview
/// before it (ide.md 3); a function shown without `preview` keeps its tab.
pub(super) fn place_tab(view: &mut View, name: &str, preview: bool) {
    let now = &mut view.now;
    let open = now.opened.iter().any(|opened| opened == name);
    if !preview {
        if now.preview.as_deref() == Some(name) {
            now.preview = None;
        }
        return;
    }
    if open {
        return;
    }
    if let Some(before) = now.preview.take()
        && let Some(at) = now.opened.iter().position(|opened| *opened == before)
        && let Some(slot) = now.opened.get_mut(at)
    {
        *slot = name.to_owned();
        // The tab replaced is closed: its view and its selection go with it.
        forget_closed(view);
    }
    view.now.preview = Some(name.to_owned());
}

/// The file of a function that a tab shows: when two files give its name (`a.b.lcf` and `a/b.lcf`), the first that is
/// drawable, as `view.show_function` chooses it.
pub fn file_of_tab<'a>(published: &'a Published, function: &str) -> Option<&'a str> {
    file_for(&published.functions, function)
}

/// The workspace of the window (ide.md 6), one of [`WORKSPACES`]: `edit` until one is chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkspaceId(pub &'static str);

impl Default for WorkspaceId {
    fn default() -> WorkspaceId {
        WorkspaceId("edit")
    }
}

/// The workspaces of ide.md 6: `edit` is the one of M1, the others are planned.
pub const WORKSPACES: &[(&str, Option<&str>)] = &[("edit", None), ("panel", Some("M3")), ("debug", Some("M5")), ("build", Some("M6"))];

/// The tabs of the window of the project lost the one at `at`, which it showed: the next tab is shown, else the one
/// before it, else none.
fn show_next(view: &mut View, at: usize) {
    let next = view.now.opened.get(at).or_else(|| at.checked_sub(1).and_then(|before| view.now.opened.get(before))).cloned();
    match next {
        Some(next) => choose(view, &next),
        None => view.main = Window::default(),
    }
    draw(view, &Which::Main, false);
}

/// `view.close_function`: closes the tab of a function; when it was shown, the next tab is, else the one before it. The
/// tab of a function in a window of its own closes with its window (decision of the owner, 2026-11-16).
pub fn close_function(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let name = params.text("function")?;
    read_again_if_stale(session);
    let view = &mut session.view;
    if view.remove_detached(name).is_some() {
        forget_closed(view);
        compose_report(view);
        return Ok(json!({ "shown": view.main.shown.function }));
    }
    let Some(at) = view.now.opened.iter().position(|opened| opened == name) else {
        let opened: Vec<&str> = view.now.opened.iter().map(String::as_str).take(10).collect();
        return Err(CommandError::of("params.invalid", "function", format!("`{name}` has no tab open")).expecting(&opened));
    };
    view.now.opened.remove(at);
    forget_closed(view);
    if view.main.shown.function.as_deref() == Some(name) {
        show_next(view, at);
    } else {
        view.publish();
    }
    Ok(json!({ "shown": view.main.shown.function }))
}

/// `view.detach`: moves the tab of a function into a window of its own (ide.md 9). The tab shown takes its window with
/// it: its drawing, its values, its selection and its zoom; the window of the project shows the next tab. A function
/// already in a window of its own stays there. Returns `{ "shown" }`, the function that the window of the project shows.
pub fn detach(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let name = params.text("function")?;
    read_again_if_stale(session);
    let view = &mut session.view;
    known(view, name)?;
    if view.is_detached(name) {
        return Ok(json!({ "shown": view.main.shown.function }));
    }
    let Some(at) = view.now.opened.iter().position(|opened| opened == name) else {
        return Err(CommandError::of("params.invalid", "function", format!("`{name}` has no tab open: `view.show_function` opens it")));
    };
    view.now.opened.remove(at);
    // A tab in its own window is kept: it is no longer a preview.
    if view.now.preview.as_deref() == Some(name) {
        view.now.preview = None;
    }
    let shown = view.main.shown.function.as_deref() == Some(name);
    let window = if shown {
        std::mem::take(&mut view.main)
    } else {
        Window {
            shown: super::Shown { function: Some(name.to_owned()), ..super::Shown::default() },
            file: file_for(&view.now.functions, name).map(str::to_owned),
            ..Window::default()
        }
    };
    view.detached.push(window);
    forget_closed(view);
    if shown {
        show_next(view, at);
    } else {
        draw(view, &Which::Detached(name.to_owned()), false);
    }
    Ok(json!({ "shown": view.main.shown.function }))
}

/// `view.dock`: moves the tab of a detached function back among the tabs of the window of the project, as its last tab,
/// shown there with what its window showed. A function whose tab is there already stays. Returns `{ "shown" }`.
pub fn dock(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let name = params.text("function")?;
    read_again_if_stale(session);
    let view = &mut session.view;
    known(view, name)?;
    if view.now.opened.iter().any(|opened| opened == name) {
        return Ok(json!({ "shown": view.main.shown.function }));
    }
    let Some(window) = view.remove_detached(name) else {
        return Err(CommandError::of("params.invalid", "function", format!("`{name}` is not in a window of its own: `view.detach` moves it there")));
    };
    view.now.opened.push(name.to_owned());
    view.main = window;
    forget_closed(view);
    compose_report(view);
    Ok(json!({ "shown": view.main.shown.function }))
}

/// `view.set_view`.
pub fn set_view(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let name = params.text("function")?;
    let asked = params.text("view")?;
    let split = params.str("split");
    if asked == "panel" || split == Some("panel") {
        return Err(CommandError::new("command.not_enabled", "the view `panel` is planned, and comes with M3"));
    }
    let Some(first) = ViewKind::parse(asked) else {
        return Err(CommandError::of("params.invalid", "view", format!("unknown view `{asked}`")).expecting(&["diagram", "text", "block"]));
    };
    let second = match split {
        None => None,
        Some(other) => match ViewKind::parse(other) {
            Some(other) => Some(other),
            None => {
                return Err(CommandError::of("params.invalid", "split", format!("unknown view `{other}`")).expecting(&["diagram", "text", "block"]));
            }
        },
    };
    // `text` beside `diagram` or `block`, either way round (ide.md 3).
    if let Some(second) = second {
        let pair = [first, second];
        if !pair.contains(&ViewKind::Text) || first == second {
            return Err(CommandError::of(
                "params.invalid",
                "split",
                format!("`{}` cannot be beside `{}`: a split shows `text` with `diagram` or with `block`", second.as_str(), first.as_str()),
            ));
        }
    }
    read_again_if_stale(session);
    let view = &mut session.view;
    known(view, name)?;
    if !has_tab(view, name) {
        return Err(CommandError::of("params.invalid", "function", format!("`{name}` has no tab open: `view.show_function` opens it")));
    }
    let tab = Tab { view: first, split: second };
    // `tabs` holds only the tabs whose view is not the diagram alone.
    let block_before = view.shows_block(name);
    let before = if tab == Tab::default() { view.now.tabs.remove(name) } else { view.now.tabs.insert(name.to_owned(), tab) }.unwrap_or_default();
    // The drawing holds the block alone only for a tab that shows it: drawn again when the tab comes to it.
    if !block_before
        && view.shows_block(name)
        && let Some(which) = view.which_shows(name)
    {
        draw(view, &which, false);
    } else if before != tab {
        view.publish();
    }
    Ok(json!({ "before": { "view": before.view.as_str(), "split": before.split.map(ViewKind::as_str) } }))
}

/// `view.set_workspace`: `edit` is the one workspace of M1 (ide.md 6).
pub fn set_workspace(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let asked = params.text("workspace")?;
    match WORKSPACES.iter().find(|(id, _)| *id == asked) {
        Some((id, None)) => {
            let view = &mut session.view;
            let before = std::mem::replace(&mut view.now.workspace, WorkspaceId(id));
            if before.0 != *id {
                view.publish();
            }
            Ok(json!({ "before": before.0 }))
        }
        Some((id, Some(milestone))) => {
            Err(CommandError::new("command.not_enabled", format!("the workspace `{id}` is planned, and comes with {milestone}")))
        }
        None => {
            let ids: Vec<&str> = WORKSPACES.iter().map(|(id, _)| *id).collect();
            Err(CommandError::of("params.invalid", "workspace", format!("unknown workspace `{asked}`")).expecting(&ids))
        }
    }
}

/// `view.select`: the selection of the window that shows the function.
pub fn select(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let name = params.text("function")?;
    let mut lists = Vec::new();
    let mut count = 0usize;
    for part in ["nodes", "wires", "terminals"] {
        let given = params.get(part).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        count = count.saturating_add(given.len());
        if count > MAX_SELECTED {
            return Err(CommandError::of("params.invalid", part, format!("at most {MAX_SELECTED} items are selected at once")));
        }
        let mut names = Vec::with_capacity(given.len());
        for value in given {
            let Some(text) = value.as_str() else {
                return Err(CommandError::of("params.invalid", part, format!("`{part}` is a list of names")));
            };
            names.push(text.to_owned());
        }
        lists.push(names);
    }
    read_again_if_stale(session);
    known(&session.view, name)?;
    // The selection is that of a window, in the function it shows (decisions of the owner, 2026-11-16).
    let Some(which) = session.view.which_shows(name) else {
        let shown = session.view.main.shown.function.as_deref().unwrap_or("none");
        return Err(CommandError::of(
            "params.invalid",
            "function",
            format!("`{name}` is not shown in a window (the window of the project shows `{shown}`): `view.show_function` shows it"),
        ));
    };
    if let [nodes, wires, terminals] = lists.as_slice() {
        let file = session.view.file_shown(&which).map(str::to_owned);
        crate::edits::selectable(session, file.as_deref(), nodes, wires, terminals)?;
    }
    let view = &mut session.view;
    let mut lists = lists.into_iter();
    let selection = Selection {
        function: name.to_owned(),
        nodes: lists.next().unwrap_or_default(),
        wires: lists.next().unwrap_or_default(),
        terminals: lists.next().unwrap_or_default(),
    };
    let Some(window) = view.window_mut(&which) else { return Err(CommandError::internal("the window of the function went away")) };
    window.shown.selection = if selection.is_empty() { Selection::default() } else { selection };
    let selected = &window.shown.selection;
    let count = selected.nodes.len() + selected.wires.len() + selected.terminals.len();
    view.publish();
    Ok(json!({ "selected": count }))
}

/// `view.get_state`: the window of the project, then each detached window, which has the one tab of its function.
pub fn get_state(session: &Session) -> Value {
    let view = &session.view;
    let now = &view.now;
    let tab_entry = |function: &String| {
        let tab = tab_of(now, function);
        let file = file_of_tab(now, function);
        json!({ "function": function, "file": file, "view": tab.view.as_str(), "split": tab.split.map(ViewKind::as_str) })
    };
    let selected = |list: &[String]| list.iter().take(MAX_SELECTED).cloned().collect::<Vec<_>>();
    let selection_of = |window: &Window| {
        let selection = Some(&window.shown.selection).filter(|selection| window.shown.function.as_deref() == Some(selection.function.as_str()));
        let (nodes, wires, terminals) =
            selection.map(|selection| (selected(&selection.nodes), selected(&selection.wires), selected(&selection.terminals))).unwrap_or_default();
        json!({ "nodes": nodes, "wires": wires, "terminals": terminals })
    };
    let main = &view.main;
    let shown = main.shown.function.as_ref().and_then(|function| now.opened.iter().position(|opened| opened == function));
    let mut windows = vec![json!({
        "tabs": now.opened.iter().map(tab_entry).collect::<Vec<_>>(),
        "shown": shown,
        "selection": selection_of(main),
        "zoom": main.shown.function.as_ref().and_then(|function| view.zooms.get(function)),
    })];
    for window in &view.detached {
        let Some(function) = &window.shown.function else { continue };
        let zoom = view.zooms.get(function);
        windows.push(json!({ "tabs": [tab_entry(function)], "shown": 0, "selection": selection_of(window), "zoom": zoom }));
    }
    json!({ "workspace": now.workspace.0, "windows": windows, "truncated": false })
}
