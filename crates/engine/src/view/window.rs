//! The state of the window of the IDE that the session holds (`docs/ide.md`; `docs/commands.md`, 6.7, "The window";
//! `docs/architecture.md`, 12.5): the view of each tab, the tab in preview, the selection, the zoom, and the commands
//! of the window that change them. The layout of the panels is a setting of the user that the window keeps, not a state
//! of the session (`view.toggle_panel`, `view.reset_layout`); a project is opened by the app, which makes its session
//! (`project.new`, `project.open`).

use serde_json::{Value, json};

use super::{Published, View, choose, draw, known, read_again_if_stale};
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

/// The selection of the window, in the function it shows, named as the editing commands name what they edit
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

/// What the window keeps of the tabs once `opened` lost some: a tab closed has no view, no preview, no selection.
pub(super) fn forget_closed(view: &mut View) {
    let now = &mut view.now;
    let opened = &now.opened;
    now.tabs.retain(|function, _| opened.contains(function));
    if now.preview.as_ref().is_some_and(|preview| !opened.contains(preview)) {
        now.preview = None;
    }
    if !opened.contains(&now.selection.function) {
        now.selection = Selection::default();
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
    let matching = || published.functions.iter().filter(|listed| listed.function == function);
    matching().find(|listed| listed.drawable).or_else(|| matching().next()).map(|listed| listed.file.as_str())
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

/// `view.close_function`: closes the tab of a function; when it was shown, the next tab is, else the one before it.
pub fn close_function(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let name = params.text("function")?;
    read_again_if_stale(session);
    let view = &mut session.view;
    let Some(at) = view.now.opened.iter().position(|opened| opened == name) else {
        let opened: Vec<&str> = view.now.opened.iter().map(String::as_str).take(10).collect();
        return Err(CommandError::of("params.invalid", "function", format!("`{name}` has no tab open")).expecting(&opened));
    };
    view.now.opened.remove(at);
    forget_closed(view);
    if view.now.function.as_deref() == Some(name) {
        let next = view.now.opened.get(at).or_else(|| at.checked_sub(1).and_then(|before| view.now.opened.get(before))).cloned();
        match next {
            Some(next) => choose(view, &next),
            None => {
                view.now.function = None;
                view.file = None;
                // A trace opened by hand is that of the function shown: none is now.
                view.traced.opened = None;
                view.now.trace_opened = false;
            }
        }
        draw(view, false);
    } else {
        view.publish();
    }
    Ok(json!({ "shown": view.now.function }))
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
    if !view.now.opened.iter().any(|opened| opened == name) {
        return Err(CommandError::of("params.invalid", "function", format!("`{name}` has no tab open: `view.show_function` opens it")));
    }
    let tab = Tab { view: first, split: second };
    // `tabs` holds only the tabs whose view is not the diagram alone.
    let before = if tab == Tab::default() { view.now.tabs.remove(name) } else { view.now.tabs.insert(name.to_owned(), tab) }.unwrap_or_default();
    if before != tab {
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

/// `view.select`.
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
    // The selection is that of the tab shown (decision of the owner, 2026-11-16).
    if session.view.now.function.as_deref() != Some(name) {
        let shown = session.view.now.function.as_deref().unwrap_or("none");
        return Err(CommandError::of(
            "params.invalid",
            "function",
            format!("`{name}` is not the function shown (`{shown}`): `view.show_function` shows it"),
        ));
    }
    if let [nodes, wires, terminals] = lists.as_slice() {
        crate::edits::selectable(session, nodes, wires, terminals)?;
    }
    let view = &mut session.view;
    let mut lists = lists.into_iter();
    let selection = Selection {
        function: name.to_owned(),
        nodes: lists.next().unwrap_or_default(),
        wires: lists.next().unwrap_or_default(),
        terminals: lists.next().unwrap_or_default(),
    };
    view.now.selection = if selection.is_empty() { Selection::default() } else { selection };
    view.publish();
    Ok(json!({ "selected": view.now.selection.nodes.len() + view.now.selection.wires.len() + view.now.selection.terminals.len() }))
}

/// `view.get_state`: the one window of the project in 1a; the detached ones come with `view.detach`.
pub fn get_state(session: &Session) -> Value {
    let view = &session.view;
    let now = &view.now;
    let tabs: Vec<Value> = now
        .opened
        .iter()
        .map(|function| {
            let tab = tab_of(now, function);
            let file = file_of_tab(now, function);
            json!({ "function": function, "file": file, "view": tab.view.as_str(), "split": tab.split.map(ViewKind::as_str) })
        })
        .collect();
    let shown = now.function.as_ref().and_then(|function| now.opened.iter().position(|opened| opened == function));
    let selected = |list: &[String]| list.iter().take(MAX_SELECTED).cloned().collect::<Vec<_>>();
    let selection = Some(&now.selection).filter(|selection| now.function.as_deref() == Some(selection.function.as_str()));
    let (nodes, wires, terminals) =
        selection.map(|selection| (selected(&selection.nodes), selected(&selection.wires), selected(&selection.terminals))).unwrap_or_default();
    json!({
        "workspace": now.workspace.0,
        "windows": [{
            "tabs": tabs,
            "shown": shown,
            "selection": { "nodes": nodes, "wires": wires, "terminals": terminals },
            "zoom": view.zoom,
        }],
        "truncated": false,
    })
}
