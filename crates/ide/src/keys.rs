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
/// The `Settings` dialog (ide.md 2).
pub const SETTINGS: &str = "ide.settings";
/// The look Lab (slate, teal) instead of Studio (graphite, indigo), the default.
pub const LAB: &str = "ide.look_lab";
pub const REDUCED_MOTION: &str = "ide.reduced_motion";
pub const VALUES: &str = "ide.values";
pub const OPEN_TRACE: &str = "ide.open_trace";
pub const ABOUT: &str = "ide.about";
/// `Connect an AI agent` (ide.md 8): the entry of the MCP server in the MCP clients found, as `labcraft-cli mcp setup`.
pub const CONNECT_AGENT: &str = "ide.connect_agent";
/// The inspector and the palette of functions beside the canvas of a detached window (ide.md 9).
pub const DETACHED_INSPECTOR: &str = "ide.detached_inspector";
pub const DETACHED_PALETTE: &str = "ide.detached_palette";
/// The entries to come, each with its card (ide.md 6).
pub const PLANNED_INSTRUMENTS: &str = "ide.planned.instruments";
pub const PLANNED_PAUSE: &str = "ide.planned.pause";
pub const PLANNED_BREAKPOINT: &str = "ide.planned.breakpoint";
pub const PLANNED_WATCH: &str = "ide.planned.watch";
pub const PLANNED_BUILD: &str = "ide.planned.build";
/// Run the function shown, and stop its run (decisions of the owner, 2026-11-16): `run.start` and `run.abort`, which
/// the window follows to the end of the run.
pub const RUN: &str = "ide.run";
pub const STOP: &str = "ide.stop";
/// Whether a run records its values (its trace, `trace: file`), off by default (decision of the owner, 2026-11-16).
pub const RECORD: &str = "ide.record_values";
/// Every function in a window of its own, planned until the windows of the system are real ones (craft-libs#1903).
pub const PLANNED_DETACHED_MODE: &str = "ide.planned.detached_mode";
/// The selection on the canvas that has the focus (notation 2.6): every node of its frame, what feeds it, what it feeds.
pub const SELECT_ALL: &str = "ide.select_all";
pub const SELECT_UPSTREAM: &str = "ide.select_upstream";
pub const SELECT_DOWNSTREAM: &str = "ide.select_downstream";
pub const SELECT_ALL_UPSTREAM: &str = "ide.select_all_upstream";
pub const SELECT_ALL_DOWNSTREAM: &str = "ide.select_all_downstream";
/// The setting "drag on the empty canvas pans" (notation 2.6; decision of the owner, 2026-11-17: a setting of the user
/// from step 2, off by default).
pub const DRAG_PANS: &str = "ide.drag_pans";
/// The settings "fix every node at the first move" (off) and "mark the fixed nodes" (on) of notation 2.4: preferences of
/// the user until the card `Settings` of step 6 (decision of the owner, 2026-11-17).
pub const FIX_EVERY_NODE: &str = "ide.fix_every_node";
pub const MARK_FIXED: &str = "ide.mark_fixed";
/// Color by type (notation 4.3): `View`, the status bar and the palette switch `appearance.color_by_type`.
pub const COLOR_BY_TYPE: &str = "ide.color_by_type";
/// Arrange (notation 2.4, 2.6): `layout.arrange` on the selection of the canvas that has the focus, or on the whole
/// function when nothing is selected.
pub const ARRANGE: &str = "ide.arrange";
/// W: a wire from the terminal that has the focus (notation 2.6).
pub const WIRE: &str = "ide.wire";
/// The settings "replace only with Alt" and "accept it" (a wire of the wrong type) of notation 2.4, off by default:
/// preferences of the user until the card `Settings` of step 6, as those of step 2.
pub const REPLACE_WITH_ALT: &str = "ide.replace_with_alt";
pub const ACCEPT_MISMATCH: &str = "ide.accept_mismatch";
/// The modifier of a drag on a wire and on a terminal of a block (notation 2.6): Shift+Alt instead of Alt, for a desktop
/// that keeps Alt and a drag to move windows. Off by default, a preference of the user as the two above.
pub const ROUTE_WITH_SHIFT_ALT: &str = "ide.route_with_shift_alt";
/// Quick insertion (notation 2.6): Space on the canvas, from what has the focus.
pub const QUICK_INSERT: &str = "ide.quick_insert";
/// Delete and Backspace (notation 2.6): the selection, or what the terminal in focus holds, one step.
pub const DELETE: &str = "ide.delete";
/// F2 (notation 2.6): the name of the node selected in a field in place, or its label.
pub const RENAME: &str = "ide.rename";
/// F12 in the view `Text`, and Ctrl+click there: where the name at the cursor is defined (decision of the owner,
/// 2026-11-21).
pub const DEFINITION: &str = "ide.go_to_definition";
/// Shift+Alt+F in the view `Text`: "Format file", `file.format` of its file (decision of the owner, 2026-11-21).
pub const FORMAT: &str = "ide.format_file";
/// Copy, cut, paste and duplicate (notation 2.6): the selection as the text of a function `clipboard` on the clipboard of
/// the system (`fragment.copy`), then `node.delete` for cut; the text of the clipboard into the frame (`fragment.paste`);
/// copy and paste 16 to the right and 16 below, without the clipboard, for duplicate.
pub const COPY: &str = "ide.copy";
pub const CUT: &str = "ide.cut";
pub const PASTE: &str = "ide.paste";
pub const DUPLICATE: &str = "ide.duplicate";
/// The settings "autopatch from the keyboard" and "inserting into a wire" of notation 2.6, on by default: preferences of
/// the user until the card `Settings` of step 6, as those of steps 2 and 3.
pub const AUTOPATCH: &str = "ide.autopatch";
/// "Surround with ▸" (notation 2.6): the nodes selected in a new structure, after the list of values for a `match`, a
/// `handle` or a `for` (`structure.surround`; decision of the owner, 2026-11-18).
pub const SURROUND_MATCH: &str = "ide.surround_match";
pub const SURROUND_HANDLE: &str = "ide.surround_handle";
pub const SURROUND_FOR: &str = "ide.surround_for";
pub const SURROUND_LOOP: &str = "ide.surround_loop";

/// The kind of structure of a command "Surround with ▸".
pub fn surround_kind(id: &str) -> Option<&'static str> {
    match id {
        SURROUND_MATCH => Some("match"),
        SURROUND_HANDLE => Some("handle"),
        SURROUND_FOR => Some("for"),
        SURROUND_LOOP => Some("loop"),
        _ => None,
    }
}
/// The parts of the one structure selected (notation 2.6).
pub const ADD_ARM: &str = "ide.add_arm";
pub const ADD_CARRY: &str = "ide.add_carry";
pub const ADD_CLAUSE: &str = "ide.add_clause";
pub const CONTINUE_ON_ERROR: &str = "ide.continue_on_error";
pub const PARALLEL: &str = "ide.parallel";

/// The part of a structure that a command of the palette asks for.
pub fn part_asked(id: &str) -> Option<labcraft_editor::PartAsked> {
    use labcraft_editor::PartAsked;
    match id {
        ADD_ARM => Some(PartAsked::AddArm),
        ADD_CARRY => Some(PartAsked::AddCarry),
        ADD_CLAUSE => Some(PartAsked::AddClause),
        CONTINUE_ON_ERROR => Some(PartAsked::ContinueOnError),
        PARALLEL => Some(PartAsked::Parallel),
        _ => None,
    }
}
pub const INTO_WIRE: &str = "ide.into_wire";
/// The key of commands.md 9.7 that a toggle of the menus writes.
pub fn setting_of(id: &str) -> Option<&'static str> {
    use labcraft_engine::settings as keyed;
    Some(match id {
        DARK => keyed::THEME,
        LAB => keyed::LOOK,
        REDUCED_MOTION => keyed::REDUCED_MOTION,
        VALUES => keyed::VALUES_SHOWN,
        RECORD => keyed::RECORD_VALUES,
        DRAG_PANS => keyed::DRAG_PANS,
        FIX_EVERY_NODE => keyed::FIX_EVERY_NODE,
        MARK_FIXED => keyed::MARK_FIXED,
        COLOR_BY_TYPE => keyed::COLOR_BY_TYPE,
        REPLACE_WITH_ALT => keyed::REPLACE_WITH_ALT,
        ACCEPT_MISMATCH => keyed::ACCEPT_MISMATCH,
        ROUTE_WITH_SHIFT_ALT => keyed::ROUTE_WITH_SHIFT_ALT,
        AUTOPATCH => keyed::AUTOPATCH,
        INTO_WIRE => keyed::INTO_WIRE,
        _ => return None,
    })
}

/// The scope of the keys that act on the canvas that has the focus.
pub const DIAGRAM_SCOPE: &str = "Diagram";
/// The scope of the keys that act on the view `Text` that has the focus.
pub const TEXT_SCOPE: &str = "Text";

/// The actions of the window, with their labels, for the command palette: each `ide.*` once.
pub const WINDOW_ACTIONS: &[(&str, &str)] = &[
    (PALETTE, "Command palette"),
    (HOME, "Start page"),
    (FIT, "Fit the diagram to the view"),
    (SETTINGS, "Settings…"),
    (DARK, "Dark theme"),
    (LAB, "Lab look (slate, teal)"),
    (REDUCED_MOTION, "Reduced motion"),
    (VALUES, "Values of the run on the wires"),
    (OPEN_TRACE, "Open a trace…"),
    (RUN, "Run the function shown"),
    (STOP, "Stop the run"),
    (RECORD, "Record the values of a run"),
    (DETACHED_INSPECTOR, "Inspector in detached windows"),
    (DETACHED_PALETTE, "Palette of functions in detached windows"),
    (SELECT_ALL, "Select every node of the frame"),
    (SELECT_UPSTREAM, "Select what feeds the selection"),
    (SELECT_DOWNSTREAM, "Select what the selection feeds"),
    (SELECT_ALL_UPSTREAM, "Select everything upstream"),
    (SELECT_ALL_DOWNSTREAM, "Select everything downstream"),
    (DRAG_PANS, "Drag on the empty canvas pans"),
    (FIX_EVERY_NODE, "Fix every node at the first move"),
    (MARK_FIXED, "Mark the fixed nodes"),
    (COLOR_BY_TYPE, "Color wires by type"),
    (ARRANGE, "Arrange the selection, or the whole function"),
    (WIRE, "Start a wire from the terminal in focus"),
    (REPLACE_WITH_ALT, "Replace a wire only with Alt"),
    (ACCEPT_MISMATCH, "Accept a wire of the wrong type"),
    (ROUTE_WITH_SHIFT_ALT, "Shift+Alt in place of Alt"),
    (QUICK_INSERT, "Quick insertion"),
    (DELETE, "Delete the selection"),
    (RENAME, "Rename the node selected"),
    (DEFINITION, "Go to definition"),
    (FORMAT, "Format file"),
    (COPY, "Copy the selection"),
    (CUT, "Cut the selection"),
    (PASTE, "Paste into the frame"),
    (DUPLICATE, "Duplicate the selection"),
    (SURROUND_MATCH, "Surround with match"),
    (SURROUND_HANDLE, "Surround with handle"),
    (SURROUND_FOR, "Surround with for"),
    (SURROUND_LOOP, "Surround with loop"),
    (ADD_ARM, "Add an arm to the structure selected"),
    (ADD_CARRY, "Add a carry to the loop selected"),
    (ADD_CLAUSE, "Add a clause to the for selected"),
    (CONTINUE_ON_ERROR, "Continue on error: the loop selected"),
    (PARALLEL, "Parallel: the for selected"),
    (AUTOPATCH, "Autopatch from the keyboard"),
    (INTO_WIRE, "Insert into a wire on a drop"),
    (ABOUT, "About LabCraft"),
    (CONNECT_AGENT, "Connect an AI agent…"),
];

fn binding(text: &str) -> Option<Binding> {
    Binding::parse(text).ok()
}

fn bindable(command: CommandRef, label: &str, keys: &[&str]) -> Bindable {
    keys.iter().filter_map(|key| binding(key)).fold(Bindable::new(command, label, Scope::app()), Bindable::with_default)
}

/// A key that acts on the canvas that has the focus only (notation 2.6: the focus decides).
fn on_canvas(id: &str, label: &str, keys: &[&str]) -> Option<Bindable> {
    in_scope(DIAGRAM_SCOPE, id, label, keys)
}

/// A key that acts on the view `Text` that has the focus only.
fn on_text(id: &str, label: &str, keys: &[&str]) -> Option<Bindable> {
    in_scope(TEXT_SCOPE, id, label, keys)
}

fn in_scope(scope: &str, id: &str, label: &str, keys: &[&str]) -> Option<Bindable> {
    let scope = Scope::new(scope)?;
    Some(keys.iter().filter_map(|key| binding(key)).fold(Bindable::new(CommandRef::new(id), label, scope), Bindable::with_default))
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
        // Not on the web, where runs cannot go (D-22) and F5 reloads the page.
        bindable(CommandRef::new(RUN), "Run", &(if cfg!(target_arch = "wasm32") { vec![] } else { vec!["F5"] })),
        bindable(CommandRef::new(STOP), "Stop", &(if cfg!(target_arch = "wasm32") { vec![] } else { vec!["Shift+F5"] })),
    ];
    let canvas = [
        on_canvas(SELECT_ALL, "Select all", &["Cmd+A"]),
        on_canvas(SELECT_UPSTREAM, "Select upstream", &["Alt+Left"]),
        on_canvas(SELECT_DOWNSTREAM, "Select downstream", &["Alt+Right"]),
        on_canvas(SELECT_ALL_UPSTREAM, "Select everything upstream", &["Shift+Alt+Left"]),
        on_canvas(SELECT_ALL_DOWNSTREAM, "Select everything downstream", &["Shift+Alt+Right"]),
        on_canvas(ARRANGE, "Arrange", &["Shift+Alt+F"]),
        on_canvas(WIRE, "Start a wire", &["W"]),
        on_canvas(QUICK_INSERT, "Quick insertion", &["Space"]),
        on_canvas(DELETE, "Delete", &["Delete", "Backspace"]),
        on_canvas(RENAME, "Rename", &["F2"]),
        on_canvas(COPY, "Copy", &["Cmd+C"]),
        on_canvas(CUT, "Cut", &["Cmd+X"]),
        on_canvas(PASTE, "Paste", &["Cmd+V"]),
        on_canvas(DUPLICATE, "Duplicate", &["Cmd+D"]),
        on_text(DEFINITION, "Go to definition", &["F12"]),
        on_text(FORMAT, "Format file", &["Shift+Alt+F"]),
    ];
    let catalog = catalog.into_iter().chain(canvas.into_iter().flatten()).collect();
    // A key of the canvas never acts when the canvas does not have the focus, nor one of the view Text out of it.
    Keymap::new(catalog, Scopes::new().with_panel(DIAGRAM_SCOPE).with_panel(TEXT_SCOPE).with_panel_fallthrough(false), platform)
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
/// The menus; `Connect an AI agent` in `Help` only with `ai`, where the window shows something of AI (ide.md 8).
pub fn menus(ai: bool) -> MenuBar {
    let item = |id: &str, label: &str| Node::Item(Item::new(CommandRef::new(id), label));
    let file = Menu::new(
        "File",
        vec![
            item("project.new", "New project…"),
            item("project.open", "Open folder…"),
            Node::Separator,
            item(HOME, "Start page"),
            item(SETTINGS, "Settings…"),
            item("view.close_function", "Close the tab"),
        ],
    )
    .role(MenuRole::File);
    let edit = Menu::new(
        "Edit",
        vec![
            item("history.undo", "Undo"),
            item("history.redo", "Redo"),
            Node::Separator,
            item(SELECT_ALL, "Select all"),
            item(SELECT_UPSTREAM, "Select upstream"),
            item(SELECT_DOWNSTREAM, "Select downstream"),
            item(SELECT_ALL_UPSTREAM, "Select everything upstream"),
            item(SELECT_ALL_DOWNSTREAM, "Select everything downstream"),
            Node::Separator,
            item(ARRANGE, "Arrange"),
            item(WIRE, "Start a wire from the terminal"),
            item(QUICK_INSERT, "Quick insertion…"),
            item(CUT, "Cut"),
            item(COPY, "Copy"),
            item(PASTE, "Paste"),
            item(DUPLICATE, "Duplicate"),
            item(DELETE, "Delete"),
            item(RENAME, "Rename…"),
            Node::Separator,
            item(DEFINITION, "Go to definition"),
            item(FORMAT, "Format file"),
            Node::Separator,
            item(SURROUND_MATCH, "Surround with match…"),
            item(SURROUND_HANDLE, "Surround with handle…"),
            item(SURROUND_FOR, "Surround with for…"),
            item(SURROUND_LOOP, "Surround with loop"),
            item(ADD_ARM, "Add arm…"),
            item(ADD_CARRY, "Add carry…"),
            item(ADD_CLAUSE, "Add clause…"),
            item(CONTINUE_ON_ERROR, "Continue on error"),
            item(PARALLEL, "Parallel"),
            Node::Separator,
            item(PALETTE, "Command palette…"),
        ],
    )
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
    view.push(item(PLANNED_DETACHED_MODE, "Every function in its window  ·  Planned"));
    view.push(Node::Item(Item::new(CommandRef::new("view.set_standard_view"), "Standard view everywhere").check(false)));
    view.push(Node::Item(Item::new(CommandRef::new(VALUES), "Values on the wires").check(true)));
    view.push(Node::Item(Item::new(CommandRef::new(COLOR_BY_TYPE), "Color wires by type").check(true)));
    view.push(Node::Item(Item::new(CommandRef::new(DARK), "Dark theme").check(false)));
    view.push(Node::Item(Item::new(CommandRef::new(LAB), "Lab look (slate, teal)").check(false)));
    view.push(Node::Item(Item::new(CommandRef::new(REDUCED_MOTION), "Reduced motion").check(false)));
    view.push(Node::Item(Item::new(CommandRef::new(DRAG_PANS), "Drag on the empty canvas pans").check(false)));
    view.push(Node::Item(Item::new(CommandRef::new(FIX_EVERY_NODE), "Fix every node at the first move").check(false)));
    view.push(Node::Item(Item::new(CommandRef::new(MARK_FIXED), "Mark the fixed nodes").check(true)));
    view.push(Node::Item(Item::new(CommandRef::new(REPLACE_WITH_ALT), "Replace a wire only with Alt").check(false)));
    view.push(Node::Item(Item::new(CommandRef::new(ACCEPT_MISMATCH), "Accept a wire of the wrong type").check(false)));
    view.push(Node::Item(Item::new(CommandRef::new(ROUTE_WITH_SHIFT_ALT), "Shift+Alt in place of Alt").check(false)));
    view.push(Node::Item(Item::new(CommandRef::new(AUTOPATCH), "Autopatch from the keyboard").check(true)));
    view.push(Node::Item(Item::new(CommandRef::new(INTO_WIRE), "Insert into a wire on a drop").check(true)));
    let view = Menu::new("View", view).role(MenuRole::View);
    let run = Menu::new(
        "Run",
        vec![
            item(RUN, "Run"),
            item(STOP, "Stop"),
            item(PLANNED_PAUSE, "Pause  ·  Planned, M5"),
            Node::Item(Item::new(CommandRef::new(RECORD), "Record values").check(false)),
            Node::Separator,
            Node::Item(Item::new(CommandRef::with_params("view.open_trace", json!({})), "Values of the last run")),
            item(OPEN_TRACE, "Open a trace…"),
            Node::Separator,
            item(PLANNED_BREAKPOINT, "Toggle a breakpoint  ·  Planned, M5"),
            item(PLANNED_WATCH, "Watch a value  ·  Planned, M5"),
            Node::Separator,
            item(PLANNED_INSTRUMENTS, "Real hardware  ·  Planned, M4"),
            item(PLANNED_BUILD, "Build  ·  Planned, M6"),
        ],
    );
    let mut help = Vec::new();
    if ai {
        help.extend([item(CONNECT_AGENT, "Connect an AI agent…"), Node::Separator]);
    }
    help.push(item(ABOUT, "About LabCraft"));
    let help = Menu::new("Help", help).role(MenuRole::Help);
    MenuBar { menus: vec![file, edit, view, run, help], documents: Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_key_reads() {
        for keys in [
            ["Cmd+Shift+P", "F1"].as_slice(),
            &["Cmd+N"],
            &["Cmd+O"],
            &["Cmd+W"],
            &["Cmd+Z"],
            &["Cmd+Y", "Cmd+Shift+Z"],
            &["Cmd+0"],
            &["Cmd+A"],
            &["Alt+Left", "Alt+Right", "Shift+Alt+Left", "Shift+Alt+Right"],
            &["W", "Space", "Delete", "Backspace", "Shift+Alt+F", "F12"],
            &["Cmd+C", "Cmd+X", "Cmd+V", "Cmd+D"],
        ] {
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

    #[test]
    fn the_keys_of_the_selection_act_on_the_canvas_alone() {
        let keymap = keymap(Platform::Windows);
        assert_eq!(key_of(&keymap, &CommandRef::new(SELECT_ALL)).as_deref(), Some("Ctrl+A"));
        assert_eq!(keymap.bindings(&CommandRef::new(SELECT_UPSTREAM)).len(), 1);
        // Ctrl+A selects on the canvas that has the keyboard, and nowhere else (notation 2.6: the focus decides).
        let chord = craft_keymap::Chord::parse("Cmd+A").unwrap();
        let canvas = craft_keymap::Active { panel: Some(DIAGRAM_SCOPE), contexts: &[], typing: false };
        assert_eq!(keymap.lookup(&chord, false, &canvas), craft_keymap::Lookup::Run(CommandRef::new(SELECT_ALL)));
        assert_eq!(keymap.lookup(&chord, false, &craft_keymap::Active::NONE), craft_keymap::Lookup::None);
    }
}
