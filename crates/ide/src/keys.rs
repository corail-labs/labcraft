//! The menus and the keys of the window (ide.md 1 and 2; notation 2.6): one description of the menus, as data
//! (`craft-menu`), and one keymap (`craft-keymap`), so that a menu entry, a key, a button and the command palette call
//! the same command. Each entry is a command of the session (commands.md 6.7), or an action of the window itself,
//! named `ide.*`, which the window runs and the session never sees.

use craft_keymap::{Bindable, Binding, CommandRef, Keymap, Platform, Scope, Scopes};
use craft_menu::{Item, Menu, MenuBar, MenuRole, Node};
use serde_json::json;

use crate::pane::PANELS;

/// The actions of the window that are not commands of the session.
pub const PALETTE: &str = "ide.palette";
pub const HOME: &str = "ide.home";
pub const FIT: &str = "ide.fit";
pub const DARK: &str = "ide.dark";
/// The look Lab (slate, teal) instead of Studio (graphite, indigo), the default.
pub const LAB: &str = "ide.look_lab";
pub const REDUCED_MOTION: &str = "ide.reduced_motion";
pub const VALUES: &str = "ide.values";
pub const OPEN_TRACE: &str = "ide.open_trace";
pub const ABOUT: &str = "ide.about";
/// The inspector and the palette of functions beside the canvas of a detached window (ide.md 9).
pub const DETACHED_INSPECTOR: &str = "ide.detached_inspector";
pub const DETACHED_PALETTE: &str = "ide.detached_palette";
/// The entries to come, each with its card (ide.md 6).
pub const PLANNED_INSTRUMENTS: &str = "ide.planned.instruments";
pub const PLANNED_AGENT: &str = "ide.planned.agent";

/// The actions of the window, with their labels, for the command palette: each `ide.*` once.
pub const WINDOW_ACTIONS: &[(&str, &str)] = &[
    (PALETTE, "Command palette"),
    (HOME, "Start page"),
    (FIT, "Fit the diagram to the view"),
    (DARK, "Dark theme"),
    (LAB, "Lab look (slate, teal)"),
    (REDUCED_MOTION, "Reduced motion"),
    (VALUES, "Values of the run on the wires"),
    (OPEN_TRACE, "Open a trace…"),
    (DETACHED_INSPECTOR, "Inspector in detached windows"),
    (DETACHED_PALETTE, "Palette of functions in detached windows"),
    (ABOUT, "About LabCraft"),
];

fn binding(text: &str) -> Option<Binding> {
    Binding::parse(text).ok()
}

fn bindable(command: CommandRef, label: &str, keys: &[&str]) -> Bindable {
    keys.iter().filter_map(|key| binding(key)).fold(Bindable::new(command, label, Scope::app()), Bindable::with_default)
}

/// The keymap of the window, with the keys of notation 2.6 by default.
pub fn keymap(platform: Platform) -> Keymap {
    let catalog = vec![
        bindable(CommandRef::new(PALETTE), "Command palette", &["Cmd+Shift+P", "F1"]),
        bindable(CommandRef::new("project.new"), "New project…", &["Cmd+N"]),
        bindable(CommandRef::new("project.open"), "Open folder…", &["Cmd+O"]),
        bindable(CommandRef::new("view.close_function"), "Close the tab", &["Cmd+W"]),
        bindable(CommandRef::new("history.undo"), "Undo", &["Cmd+Z"]),
        bindable(CommandRef::new("history.redo"), "Redo", &["Cmd+Y", "Cmd+Shift+Z"]),
        bindable(CommandRef::new(FIT), "Fit", &["Cmd+0"]),
    ];
    Keymap::new(catalog, Scopes::new(), platform)
}

/// The key of a command, as the platform shows it.
pub fn key_of(keymap: &Keymap, command: &CommandRef) -> Option<String> {
    keymap.bindings(command).first().map(|binding| binding.trigger.display(keymap.platform()))
}

/// The command of `view.toggle_panel` for a panel.
pub fn toggle_panel(name: &str) -> CommandRef {
    CommandRef::with_params("view.toggle_panel", json!({ "panel": name }))
}

/// The command of `view.set_view` for the tab shown, as the window fills it.
pub fn set_view(view: &str, split: Option<&str>) -> CommandRef {
    CommandRef::with_params("view.set_view", json!({ "view": view, "split": split }))
}

/// The menus `File`, `Edit`, `View`, `Run`, `Help`, before the layout of the platform.
pub fn menus() -> MenuBar {
    let item = |id: &str, label: &str| Node::Item(Item::new(CommandRef::new(id), label));
    let file = Menu::new(
        "File",
        vec![
            item("project.new", "New project…"),
            item("project.open", "Open folder…"),
            Node::Separator,
            item(HOME, "Start page"),
            item("view.close_function", "Close the tab"),
        ],
    )
    .role(MenuRole::File);
    let edit =
        Menu::new("Edit", vec![item("history.undo", "Undo"), item("history.redo", "Redo"), Node::Separator, item(PALETTE, "Command palette…")])
            .role(MenuRole::Edit);
    let mut view = Vec::new();
    for (_, name, title) in PANELS {
        view.push(Node::Item(Item::new(toggle_panel(name), *title).check(true)));
    }
    view.push(Node::Item(Item::new(CommandRef::new("view.reset_layout"), "Reset layout")));
    view.push(Node::Separator);
    view.push(Node::Item(Item::new(set_view("diagram", None), "Diagram")));
    view.push(Node::Item(Item::new(set_view("text", None), "Text")));
    view.push(Node::Item(Item::new(set_view("text", Some("diagram")), "Text beside the diagram")));
    view.push(Node::Item(Item::new(set_view("block", None), "Block")));
    view.push(Node::Separator);
    view.push(item(FIT, "Fit"));
    view.push(Node::Item(Item::new(CommandRef::new("view.set_standard_view"), "Standard view everywhere").check(false)));
    view.push(Node::Item(Item::new(CommandRef::new(VALUES), "Values on the wires").check(true)));
    view.push(Node::Item(Item::new(CommandRef::new(DARK), "Dark theme").check(false)));
    view.push(Node::Item(Item::new(CommandRef::new(LAB), "Lab look (slate, teal)").check(false)));
    view.push(Node::Item(Item::new(CommandRef::new(REDUCED_MOTION), "Reduced motion").check(false)));
    let view = Menu::new("View", view).role(MenuRole::View);
    let run = Menu::new(
        "Run",
        vec![
            Node::Item(Item::new(CommandRef::with_params("view.open_trace", json!({})), "Values of the last run")),
            item(OPEN_TRACE, "Open a trace…"),
            Node::Separator,
            item(PLANNED_INSTRUMENTS, "Real instruments  ·  Planned, M4"),
        ],
    );
    let help = Menu::new("Help", vec![item(PLANNED_AGENT, "Connect an AI agent  ·  Planned"), Node::Separator, item(ABOUT, "About LabCraft")])
        .role(MenuRole::Help);
    MenuBar { menus: vec![file, edit, view, run, help], documents: Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_key_reads() {
        for keys in [["Cmd+Shift+P", "F1"].as_slice(), &["Cmd+N"], &["Cmd+O"], &["Cmd+W"], &["Cmd+Z"], &["Cmd+Y", "Cmd+Shift+Z"], &["Cmd+0"]] {
            for key in keys {
                assert!(binding(key).is_some(), "{key}");
            }
        }
    }

    #[test]
    fn the_palette_has_its_two_keys() {
        let keymap = keymap(Platform::Windows);
        assert_eq!(keymap.bindings(&CommandRef::new(PALETTE)).len(), 2);
        assert_eq!(key_of(&keymap, &CommandRef::new(PALETTE)).as_deref(), Some("Ctrl+Shift+P"));
    }
}
