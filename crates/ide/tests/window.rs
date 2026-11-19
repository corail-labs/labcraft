//! The window without a window (`docs/architecture.md`, 12.10): `ide` driven through its `Host` by a session of
//! `engine` on a project in memory, its frames run by `egui_kittest`, what it shows read from its AccessKit tree.
//!
//! With `LABCRAFT_IDE_SHOT` set to a folder, each test also renders its last frame there, as a BMP file: for a person to
//! look at, never compared. Only then does it need a GPU (wgpu): a runner of the CI has none.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

use egui::{FontData, FontFamily, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use labcraft_engine::{Config, Memory, Session, Shipped, Still};
use labcraft_ide::{Host, Ide, Outward, Prefs, Reply, Snapshot, StartList};
use labcraft_types::Limits;
use serde_json::{Value, json};

const MAIN: &str = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    a = core.mul(x, 2.0)\n    y = core.add(a, 1.0)\n}\n";
const HELPER: &str = "labcraft 1\n\nuse core\n\n/// Adds one.\nfn helper(x: f64) -> (y: f64) {\n    y = core.add(x, 1.0)\n}\n";
const BROKEN: &str = "labcraft 1\n\nuse core\n\nfn broken(x: f64) -> (y: f64) {\n    y = core.add(nowhere, 1.0)\n}\n";

/// A session of `engine`, answered at once on the thread of the test; after each command it reads the trace of each
/// window, as the desktop app does (`apps/labcraft/src/session.rs`).
struct Fake {
    session: RefCell<Session>,
    /// The files of the project, which the tests read.
    memory: Arc<Memory>,
}

impl Host for Fake {
    fn send(&self, id: &str, params: Value) -> Reply {
        let mut session = self.session.borrow_mut();
        let result = session.execute(id, &params);
        session.look_at_trace();
        Reply::ready(result)
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { view: self.session.borrow().view().clone(), project: Some("/p".to_owned()), notices: Vec::new(), session: 0 }
    }

    fn set_zoom(&self, function: &str, zoom: f32) {
        self.session.borrow_mut().set_zoom(function, zoom);
    }
}

fn fake() -> Fake {
    let memory = Arc::new(Memory::new());
    memory.put("/p/main.lcf", MAIN);
    memory.put("/p/helper.lcf", HELPER);
    memory.put("/p/broken.lcf", BROKEN);
    let config = Config {
        root: PathBuf::from("/p"),
        lib_paths: Vec::new(),
        limits: Limits::default(),
        shipped: Shipped::default(),
        natives: Vec::new(),
        clock: Arc::new(Still),
        process: 0,
    };
    let mut session = Session::new(memory.clone(), config, false).unwrap();
    session.with_window();
    session.open_on(Some("main".to_owned()));
    session.execute("view.open", &json!({ "folder": "." })).unwrap();
    Fake { session: RefCell::new(session), memory }
}

/// The fonts of the app: Hack everywhere, which the canvas needs under its own name.
fn install_fonts(ctx: &egui::Context) {
    use craft_ui_fonts::{Family, I18nScripts, SystemFonts, UiFonts};
    let notation = FontFamily::Name(labcraft_editor::FAMILY.into());
    UiFonts::new(&I18nScripts)
        .app_face("Hack", FontData::from_static(labcraft_editor::HACK), &[FontFamily::Proportional, FontFamily::Monospace, notation])
        .family(Family::named(labcraft_editor::FAMILY).monospaced())
        .system_fonts(SystemFonts::Off)
        .install(ctx);
}

/// `craft_ui::fonts`, under a name that says what it is for here.
mod craft_ui_fonts {
    pub use craft_ui::fonts::*;
}

struct State {
    ide: Ide,
    host: Fake,
    /// The fonts are installed by the first frame, and taken from the next one.
    fonts: bool,
    /// What the window asked the app for, every frame together.
    outward: Vec<Outward>,
}

fn harness(host: Fake) -> Harness<'static, State> {
    harness_with(host, Prefs::default())
}

fn harness_with(host: Fake, prefs: Prefs) -> Harness<'static, State> {
    let ide = Ide::new(&egui::Context::default(), prefs, craft_window::Frame::Desktop);
    let mut builder = Harness::builder().with_size(egui::vec2(1280.0, 800.0));
    if std::env::var_os("LABCRAFT_IDE_SHOT").is_some() {
        builder = builder.wgpu();
    }
    let mut harness = builder.build_ui_state(
        |ui, state: &mut State| {
            if !std::mem::replace(&mut state.fonts, true) {
                install_fonts(ui.ctx());
                return;
            }
            let outward = state.ide.show(ui, Some(&state.host), &StartList::default());
            state.outward.extend(outward);
        },
        State { ide, host, fonts: false, outward: Vec::new() },
    );
    harness.run_steps(4);
    harness
}

/// Renders the last frame into `LABCRAFT_IDE_SHOT`, when it is set.
fn shot(harness: &mut Harness<'_, State>, name: &str) {
    let Some(folder) = std::env::var_os("LABCRAFT_IDE_SHOT") else { return };
    let image = harness.render().unwrap();
    let (width, height) = image.dimensions();
    // A BMP of 32 bits, rows bottom up: no encoder needed.
    let mut bytes = Vec::new();
    let size = 54 + width * height * 4;
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&54u32.to_le_bytes());
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 24]);
    for y in (0..height).rev() {
        for x in 0..width {
            let [r, g, b, a] = image.get_pixel(x, y).0;
            bytes.extend_from_slice(&[b, g, r, a]);
        }
    }
    std::fs::write(PathBuf::from(folder).join(format!("{name}.bmp")), bytes).unwrap();
}

#[test]
fn a_project_opens_with_its_panels_and_the_function_shown_in_its_tab() {
    let mut harness = harness(fake());
    for panel in ["Project", "Palette", "Inspector", "Problems", "Run"] {
        assert!(harness.query_all_by_label(panel).count() >= 1, "no `{panel}`");
    }
    // `main` has its tab, which took the place of the start page.
    assert!(harness.query_all_by_label("main").count() >= 1);
    assert!(harness.query_by_label("Start").is_none());
    // The broken function says its error in `Project`, and `Problems` names its code.
    assert!(harness.query_by_label_contains("broken  (1 error)").is_some());
    assert!(harness.query_by_label_contains("LC2001").is_some());
    shot(&mut harness, "project");
}

#[test]
fn a_click_in_project_shows_the_function_in_preview_and_the_next_preview_takes_its_place() {
    let mut harness = harness(fake());
    harness.get_by_label("helper").click();
    harness.run_steps(4);
    let view = harness.state().host.snapshot().view;
    assert_eq!(view.main.function.as_deref(), Some("helper"));
    assert_eq!(view.preview.as_deref(), Some("helper"));
    assert_eq!(view.opened, ["main", "helper"]);
    // The next preview takes its place.
    harness.get_by_label("broken  (1 error)").click();
    harness.run_steps(4);
    let view = harness.state().host.snapshot().view;
    assert_eq!(view.opened, ["main", "broken"]);
    shot(&mut harness, "preview");
}

#[test]
fn the_command_palette_opens_with_its_key_and_runs_a_command_typed_as_json() {
    let mut harness = harness(fake());
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    harness.run_steps(3);
    // The one field shown: `Palette`, which has one too, is behind `Project`.
    palette_field(&harness).type_text("view.set_view {\"function\": \"main\", \"view\": \"text\"}");
    harness.run_steps(3);
    assert!(harness.query_by_label_contains("valid: Enter runs it").is_some());
    harness.key_press(Key::Enter);
    harness.run_steps(4);
    let view = harness.state().host.snapshot().view;
    assert_eq!(view.tabs.get("main").map(|tab| tab.view.as_str()), Some("text"));
    shot(&mut harness, "text");
}

#[test]
fn a_panel_closed_comes_back_from_view() {
    let mut harness = harness(fake());
    let state = harness.state_mut();
    let before = state.ide.prefs().layout.unwrap();
    assert!(before.layout.contains(&labcraft_ide::Pane::Run));
    harness.key_press(Key::F1);
    harness.run_steps(3);
    palette_field(&harness).type_text("view.toggle_panel {\"panel\": \"run\"}");
    harness.run_steps(2);
    harness.key_press(Key::Enter);
    harness.run_steps(3);
    assert!(!harness.state().ide.prefs().layout.unwrap().layout.contains(&labcraft_ide::Pane::Run));
    assert!(harness.state_mut().ide.take_prefs_changed());
    harness.key_press(Key::F1);
    harness.run_steps(3);
    palette_field(&harness).type_text("view.toggle_panel {\"panel\": \"run\"}");
    harness.run_steps(2);
    harness.key_press(Key::Enter);
    harness.run_steps(3);
    assert!(harness.state().ide.prefs().layout.unwrap().layout.contains(&labcraft_ide::Pane::Run));
}

/// The field of the command palette: the inspector has fields of its own (ide.md 4).
fn palette_field<'h>(harness: &'h Harness<'_, State>) -> egui_kittest::Node<'h> {
    use egui_kittest::kittest::NodeT as _;
    let placeholder = Some("Search a command, or type an id and a JSON object");
    harness
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .find(|node| node.accesskit_node().placeholder() == placeholder)
        .unwrap_or_else(|| panic!("the command palette is not open"))
}

/// Types a command in the palette, `id {json}`, and runs it.
fn run_in_palette(harness: &mut Harness<'_, State>, typed: &str) {
    harness.key_press(Key::F1);
    harness.run_steps(3);
    palette_field(harness).type_text(typed);
    harness.run_steps(2);
    harness.key_press(Key::Enter);
    harness.run_steps(4);
}

#[test]
fn the_start_page_stays_in_front_of_the_function_shown() {
    let mut harness = harness(fake());
    assert!(harness.query_by_label_contains("Open folder").is_none());
    run_in_palette(&mut harness, "ide.home {}");
    // Frames after, the session still shows `main`, and the start page is still the one in front.
    harness.run_steps(6);
    assert_eq!(harness.state().host.snapshot().view.main.function.as_deref(), Some("main"));
    assert!(harness.query_by_label_contains("Open folder").is_some(), "the start page went behind the function");
    shot(&mut harness, "home");
}

#[test]
fn the_palette_gives_the_folder_typed_to_the_app() {
    let mut harness = harness(fake());
    run_in_palette(&mut harness, "project.open {\"folder\": \"/q\"}");
    assert_eq!(harness.state().outward, [Outward::OpenFolder(Some("/q".to_owned()))]);
    run_in_palette(&mut harness, "project.new {\"folder\": \"/r\"}");
    assert_eq!(harness.state().outward.last(), Some(&Outward::NewProject(Some("/r".to_owned()))));
}

/// Step 1b (ide.md 9): a tab detaches into a window of its own, which floats in the window here (kittest has one
/// viewport, as the web), with its toolbar and its inspector; the palette and `Fit` then act on it, and it docks back.
#[test]
fn a_tab_detaches_into_a_floating_window_and_docks_back() {
    let mut harness = harness(fake());
    run_in_palette(&mut harness, "view.show_function {\"function\": \"helper\"}");
    run_in_palette(&mut harness, "view.detach {\"function\": \"helper\"}");
    harness.run_steps(4);
    let view = harness.state().host.snapshot().view;
    assert!(view.is_detached("helper") && view.opened == ["main"], "{:?}", view.opened);
    assert_eq!(view.main.function.as_deref(), Some("main"));
    let layout = harness.state().ide.prefs().layout.unwrap();
    assert!(!layout.layout.contains(&labcraft_ide::Pane::Function("helper".to_owned())), "the tab of a function is never saved");
    // The detached window draws its function, and its inspector names it.
    assert!(view.window_of("helper").is_some_and(|window| window.drawing.is_some()));
    assert!(harness.query_all_by_label("helper").count() >= 2, "the title of the window and its inspector");
    shot(&mut harness, "detached");
    // Its place is kept for the next one.
    assert!(harness.state().ide.prefs().detached.is_some());
    // A press in it gives it focus: the palette acts on its function.
    let place = harness.state().ide.prefs().detached.unwrap().rect;
    harness.hover_at(egui::pos2(place[0] + place[2] / 2.0, place[1] + place[3] / 2.0));
    harness.event(egui::Event::PointerButton {
        pos: egui::pos2(place[0] + place[2] / 2.0, place[1] + place[3] / 2.0),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.event(egui::Event::PointerButton {
        pos: egui::pos2(place[0] + place[2] / 2.0, place[1] + place[3] / 2.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(3);
    assert_eq!(harness.state().ide.focused(&harness.state().host.snapshot().view).as_deref(), Some("helper"));
    // Ctrl+W closes the tab of the window that has focus: the detached window closes, with its tab (ide.md 9).
    harness.key_press_modifiers(Modifiers::COMMAND, Key::W);
    harness.run_steps(4);
    let view = harness.state().host.snapshot().view;
    assert!(view.detached.is_empty() && view.opened == ["main"], "{:?}", view.opened);
    // Detached again, then docked back: it is the last tab of the project, shown.
    run_in_palette(&mut harness, "view.show_function {\"function\": \"helper\"}");
    run_in_palette(&mut harness, "view.detach {\"function\": \"helper\"}");
    run_in_palette(&mut harness, "view.dock {\"function\": \"helper\"}");
    harness.run_steps(4);
    let view = harness.state().host.snapshot().view;
    assert!(view.detached.is_empty() && view.opened == ["main", "helper"], "{:?}", view.opened);
    assert_eq!(view.main.function.as_deref(), Some("helper"));
    let layout = &harness.state().ide.prefs().layout.unwrap().layout;
    assert!(layout.floating.is_empty(), "no floating window is left");
}

/// `view.reset_layout` docks every detached tab again (commands.md 6.7), and no floating window is left.
#[test]
fn reset_layout_docks_the_detached_tabs() {
    let mut harness = harness(fake());
    run_in_palette(&mut harness, "view.show_function {\"function\": \"helper\"}");
    run_in_palette(&mut harness, "view.detach {\"function\": \"helper\"}");
    harness.run_steps(4);
    assert!(harness.state().host.snapshot().view.is_detached("helper"));
    run_in_palette(&mut harness, "view.reset_layout {}");
    harness.run_steps(6);
    let view = harness.state().host.snapshot().view;
    assert!(view.detached.is_empty() && view.opened == ["main", "helper"], "{:?}", view.opened);
    let state = harness.state_mut();
    assert!(state.ide.prefs().layout.unwrap().layout.floating.is_empty(), "no floating window is left");
}

/// F5 runs the function shown (decisions of the owner, 2026-11-16): `run.start`, followed to its end; with "Record
/// values" on, the values of the run reach the function by its trace; off, the default, no trace is written.
#[test]
fn f5_runs_the_function_shown_and_its_values_come_when_recorded() {
    for record in [true, false] {
        let mut harness = harness_with(fake(), Prefs { record_values: record, ..Prefs::default() });
        let answer = "labcraft 1

use core

fn answer() -> (y: f64) {
    y = core.add(1.0, 2.0)
}
";
        harness.state().host.session.borrow_mut().execute("file.write", &json!({ "path": "answer.lcf", "text": answer })).unwrap();
        run_in_palette(&mut harness, "view.show_function {\"function\": \"answer\"}");
        harness.run_steps(4);
        assert!(harness.state().host.snapshot().view.main.trace.is_none(), "no run yet");
        harness.key_press(Key::F5);
        let mut ended = None;
        for _ in 0..200 {
            harness.run_steps(2);
            let status = harness.state().host.session.borrow_mut().execute("run.status", &json!({})).unwrap();
            if status["status"] != "running" {
                ended = Some(status);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        harness.run_steps(4);
        assert_eq!(ended.expect("the run ended")["status"], "ok");
        let view = harness.state().host.snapshot().view;
        match (record, view.main.trace.as_ref()) {
            (true, Some(trace)) => assert_eq!(trace.status.as_deref(), Some("ok"), "{trace:?}"),
            (false, None) => {}
            (record, trace) => panic!("record {record}: {trace:?}"),
        }
    }
}

/// The node of `label` with `role`: the panels and the buttons share some names (`Inspector`, `Run`).
fn click_role(harness: &mut Harness<'_, State>, label: &str, role: egui::accesskit::Role) {
    use egui_kittest::kittest::NodeT as _;
    harness
        .query_all_by_label(label)
        .find(|node| node.accesskit_node().role() == role)
        .unwrap_or_else(|| panic!("no {role:?} named {label}"))
        .click();
    harness.run_steps(3);
}

#[test]
fn the_inspector_button_of_a_tab_hides_and_shows_the_inspector_panel() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    assert!(harness.state().ide.prefs().layout.unwrap().layout.contains(&labcraft_ide::Pane::Inspector));
    click_role(&mut harness, "Inspector", egui::accesskit::Role::Button);
    assert!(!harness.state().ide.prefs().layout.unwrap().layout.contains(&labcraft_ide::Pane::Inspector));
    click_role(&mut harness, "Inspector", egui::accesskit::Role::Button);
    assert!(harness.state().ide.prefs().layout.unwrap().layout.contains(&labcraft_ide::Pane::Inspector));
}

#[test]
fn a_planned_workspace_opens_its_card() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    // The line of the workspace is on its button (its hover) and, once clicked, in its card.
    let line = "controls and indicators of a function";
    assert_eq!(harness.query_all_by_label_contains(line).count(), 1);
    harness.query_by_label_contains("Workspace Panel: planned, M3").expect("the button of the workspace Panel").click();
    harness.run_steps(3);
    assert_eq!(harness.query_all_by_label_contains(line).count(), 2);
}

#[test]
fn a_run_without_record_says_that_the_trace_shown_is_of_an_earlier_run() {
    let mut harness = harness_with(fake(), Prefs { record_values: true, ..Prefs::default() });
    let answer = "labcraft 1

use core

fn answer() -> (y: f64) {
    y = core.add(1.0, 2.0)
}
";
    harness.state().host.session.borrow_mut().execute("file.write", &json!({ "path": "answer.lcf", "text": answer })).unwrap();
    run_in_palette(&mut harness, "view.show_function {\"function\": \"answer\"}");
    harness.run_steps(4);
    click_role(&mut harness, "Run", egui::accesskit::Role::Tab);
    let stale = "of an earlier run";
    for record in [true, false] {
        if !record {
            run_in_palette(&mut harness, "ide.record_values");
        }
        harness.key_press(Key::F5);
        for _ in 0..200 {
            harness.run_steps(2);
            if harness.query_by_label_contains("Ended").is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        harness.run_steps(4);
        assert!(harness.query_by_label_contains("Ended").is_some(), "record {record}: the run ended");
        assert_eq!(harness.query_by_label_contains(stale).is_some(), !record, "record {record}");
    }
}

#[test]
fn run_is_greyed_for_a_function_whose_input_has_no_default() {
    let mut harness = harness(fake());
    let twice = "labcraft 1

use core

fn twice(x: f64, k: f64 = 2.0) -> (y: f64) {
    y = core.mul(x, k)
}
";
    harness.state().host.session.borrow_mut().execute("file.write", &json!({ "path": "twice.lcf", "text": twice })).unwrap();
    run_in_palette(&mut harness, "view.show_function {\"function\": \"twice\"}");
    harness.run_steps(4);
    let listed = harness.state().host.snapshot().view.functions.iter().find(|listed| listed.function == "twice").cloned();
    assert_eq!(listed.map(|listed| listed.needs), Some(vec!["x".to_owned()]), "only the input with no default");
    // F5 starts no run, and says why (the hover of the greyed Run says the same).
    harness.key_press(Key::F5);
    harness.run_steps(4);
    let status = harness.state().host.session.borrow_mut().execute("run.status", &json!({}));
    assert!(matches!(&status, Err(error) if error.code == "run.unknown"), "no run started: {status:?}");
    assert!(harness.query_by_label_contains("`x` has no default value").is_some(), "said in the window");
}

/// A press and a release of the primary button at `at`, with `modifiers`.
fn click_at(harness: &mut Harness<'_, State>, at: egui::Pos2, modifiers: egui::Modifiers) {
    let events = &mut harness.input_mut().events;
    events.push(egui::Event::ModifiersChanged(modifiers));
    events.push(egui::Event::PointerMoved(at));
    events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers });
    harness.run_steps(1);
    harness.input_mut().events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers });
    harness.run_steps(1);
    harness.input_mut().events.push(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    harness.run_steps(3);
}

/// The element of the canvas of this name in the tree of AccessKit: its centre, and whether it is selected.
fn canvas_element(harness: &Harness<'_, State>, name: &str) -> (egui::Pos2, bool) {
    use egui_kittest::kittest::NodeT as _;
    let node = harness
        .query_all_by_label(name)
        .find(|node| node.accesskit_node().role() == egui::accesskit::Role::ListBoxOption)
        .unwrap_or_else(|| panic!("no element `{name}` on the canvas"));
    (node.rect().center(), node.accesskit_node().is_selected() == Some(true))
}

fn selection(harness: &Harness<'_, State>) -> labcraft_engine::view::Selection {
    harness.state().host.session.borrow().view().main.selection.clone()
}

#[test]
fn a_click_on_a_node_selects_it_in_the_session_and_the_inspector_and_the_keys_follow() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    // Each node of the canvas is an element of the tree, named by its name and what it calls (notation 2.6).
    let (on_a, selected) = canvas_element(&harness, "a, core.mul");
    assert!(!selected);
    click_at(&mut harness, on_a, egui::Modifiers::NONE);
    // The session holds the selection (`view.select`); the canvas and the inspector show what it holds.
    let held = selection(&harness);
    assert_eq!((held.function.as_str(), held.nodes.as_slice()), ("main", ["a".to_owned()].as_slice()));
    assert!(canvas_element(&harness, "a, core.mul").1, "the element says it is selected");
    assert!(harness.query_by_label("Placed automatically").is_some(), "the inspector of the node says where it is");
    assert!(harness.query_by_label("core.mul").is_some(), "and what it calls");
    // Shift and a click on `y`: both, and the inspector counts them.
    let (on_y, _) = canvas_element(&harness, "y, core.add");
    click_at(&mut harness, on_y, egui::Modifiers::SHIFT);
    assert_eq!(selection(&harness).nodes, ["a", "y"]);
    assert!(harness.query_by_label("2 nodes").is_some());
    // The canvas has the keyboard: Escape selects nothing, Ctrl+A every node of the frame.
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert!(selection(&harness).nodes.is_empty());
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run_steps(3);
    assert_eq!(selection(&harness).nodes, ["a", "y"]);
    shot(&mut harness, "selection");
}

#[test]
fn a_text_field_keeps_its_keys_once_the_canvas_has_lost_the_focus() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    let (on_a, _) = canvas_element(&harness, "a, core.mul");
    click_at(&mut harness, on_a, egui::Modifiers::NONE);
    assert_eq!(selection(&harness).nodes, ["a"]);
    // The field of the command palette takes the focus: Ctrl+A is its own, not the canvas's (notation 2.6: the focus
    // decides).
    harness.key_press(Key::F1);
    harness.run_steps(3);
    // The field of the palette has the focus; the fields of the inspector (X, Y) are text inputs too.
    use egui_kittest::kittest::NodeT as _;
    let focused = harness.query_all_by_role(egui::accesskit::Role::TextInput).any(|node| node.accesskit_node().is_focused());
    assert!(focused, "the field of the palette");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run_steps(3);
    assert_eq!(selection(&harness).nodes, ["a"], "Ctrl+A in the field selects no node");
}

fn text_of(harness: &Harness<'_, State>, path: &str) -> String {
    String::from_utf8(harness.state().host.memory.get(path).unwrap()).unwrap()
}

/// A drag of the primary button from `from` to `to`, over a few frames.
fn drag_from(harness: &mut Harness<'_, State>, from: egui::Pos2, to: egui::Pos2) {
    let button =
        |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    for events in [
        vec![egui::Event::PointerMoved(from), button(from, true)],
        vec![egui::Event::PointerMoved(from + (to - from) / 2.0)],
        vec![egui::Event::PointerMoved(to)],
        vec![egui::Event::PointerMoved(to), button(to, false)],
    ] {
        harness.input_mut().events.extend(events);
        harness.run_steps(1);
    }
    harness.run_steps(4);
}

#[test]
fn a_node_dragged_is_fixed_where_it_goes_and_its_fixed_position_is_removed_from_the_inspector() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    let before = harness.state().host.session.borrow().view().main.drawing.clone().unwrap();
    let a = before.placed.iter().position(|placed| placed.name == "a").unwrap();
    let (x, y) = before.in_frame(a).unwrap();
    let (on_a, _) = canvas_element(&harness, "a, core.mul");
    drag_from(&mut harness, on_a, on_a + egui::vec2(60.0, 40.0));
    // The drag selected `a`, and moved it: its entry of `layout`, the only line that changed (scenario 5), on the grid of
    // 8 from where it was, right and down (notation 2.4, 2.6).
    assert_eq!(selection(&harness).nodes, ["a"]);
    let text = text_of(&harness, "/p/main.lcf");
    let entry = text.strip_prefix(MAIN).unwrap_or_else(|| panic!("only `layout` added: {text}"));
    let at = entry.strip_prefix("\nlayout {\n    node a at (").and_then(|rest| rest.strip_suffix(")\n}\n")).unwrap_or_else(|| panic!("{entry}"));
    let (x1, y1) = at.split_once(", ").map(|(x, y)| (x.parse::<i64>().unwrap(), y.parse::<i64>().unwrap())).unwrap();
    let (dx, dy) = (x1 - (x + 50).div_euclid(100), y1 - (y + 50).div_euclid(100));
    assert!(dx > 0 && dy > 0 && dx % 8 == 0 && dy % 8 == 0, "moved by ({dx}, {dy})");
    // The session drew it again, fixed there: the inspector says so, and removing the fixed position gives the file back.
    harness.state().host.session.borrow_mut().files_changed();
    harness.run_steps(4);
    assert!(harness.query_by_label("Fixed").is_some(), "the inspector says the node is fixed");
    click_role(&mut harness, "Remove the fixed position", egui::accesskit::Role::Button);
    harness.run_steps(4);
    assert_eq!(text_of(&harness, "/p/main.lcf"), MAIN);
    shot(&mut harness, "move");
}

/// A wire dragged from the output of `a` to the constant `1.0` of `y` replaces the constant (notation 2.4, 2.6): one
/// `wire.connect`, the line of `y` alone changes, written by hand from spec 5.1. The terminals are elements of the tree of
/// AccessKit, named as notation 2.6 says.
#[test]
fn a_wire_dragged_between_two_terminals_is_written() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    let (from, _) = canvas_element(&harness, "output result of a, f64");
    let (to, _) = canvas_element(&harness, "input b of y, f64, constant 1.0");
    drag_from(&mut harness, from, to);
    let wired = MAIN.replace("y = core.add(a, 1.0)", "y = core.add(a, a)");
    assert_eq!(text_of(&harness, "/p/main.lcf"), wired);
    shot(&mut harness, "wire");
}

/// Types `text` into the field that has the keyboard, then Enter.
fn type_then_enter(harness: &mut Harness<'_, State>, text: &str) {
    harness.input_mut().events.push(egui::Event::Text(text.to_owned()));
    harness.run_steps(2);
    harness.key_press(Key::Enter);
    harness.run_steps(4);
}

fn undo(harness: &mut Harness<'_, State>) {
    harness.state().host.session.borrow_mut().execute("history.undo", &json!({})).unwrap();
    harness.run_steps(2);
}

/// A wire from the output of `a` dropped on the empty canvas opens quick insertion, filtered; `core.abs` chosen falls
/// there, already wired, in one step (notation 2.6): its line, read from `a`, and its `node … at` entry (spec 11).
#[test]
fn a_wire_dropped_on_the_empty_canvas_inserts_a_node_already_wired() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    let (from, _) = canvas_element(&harness, "output result of a, f64");
    drag_from(&mut harness, from, from + egui::vec2(40.0, 200.0));
    assert!(harness.query_by_label("core.abs").is_some(), "the list holds what takes the wire");
    assert!(harness.query_by_label("core.not").is_none(), "and not what refuses it");
    shot(&mut harness, "quick-insertion");
    type_then_enter(&mut harness, "abs");
    let text = text_of(&harness, "/p/main.lcf");
    let body = text.split("\nlayout {").next().unwrap_or_default();
    assert_eq!(body, MAIN.replace("    y = core.add(a, 1.0)\n", "    abs = core.abs(a)\n    y = core.add(a, 1.0)\n"));
    assert!(text.contains("\nlayout {\n    node abs at ("), "{text}");
    undo(&mut harness);
    assert_eq!(text_of(&harness, "/p/main.lcf"), MAIN, "one step");
}

/// Space with one node selected (autopatch): the node falls to its right, its first input wired from the first output of
/// the node selected (notation 2.6).
#[test]
fn space_on_a_node_selected_inserts_its_next_node_wired_to_it() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    let (on_a, _) = canvas_element(&harness, "a, core.mul");
    click_at(&mut harness, on_a, Modifiers::NONE);
    harness.key_press(Key::Space);
    harness.run_steps(3);
    type_then_enter(&mut harness, "abs");
    let text = text_of(&harness, "/p/main.lcf");
    assert!(text.contains("    a = core.mul(x, 2.0)\n    abs = core.abs(a)\n    y = core.add(a, 1.0)\n"), "{text}");
}

/// Space with a wire selected: the node goes into it (`node.add` with `into`), placed automatically (notation 2.6); the
/// lines in the order of spec 5.2.
#[test]
fn space_on_a_wire_selected_inserts_a_node_into_it() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    let (on_a, _) = canvas_element(&harness, "a, core.mul");
    click_at(&mut harness, on_a, Modifiers::NONE);
    harness.state().host.session.borrow_mut().execute("view.select", &json!({ "function": "main", "wires": ["a -> y.a"] })).unwrap();
    harness.run_steps(3);
    harness.key_press(Key::Space);
    harness.run_steps(3);
    assert!(harness.query_by_label("core.lt").is_none(), "what gives no number back does not go into the wire");
    type_then_enter(&mut harness, "abs");
    let wired = MAIN.replace("    y = core.add(a, 1.0)\n", "    abs = core.abs(a)\n    y = core.add(abs, 1.0)\n");
    assert_eq!(text_of(&harness, "/p/main.lcf"), wired);
}

/// Delete removes the node selected, with its wires, in one step (notation 2.6, commands.md 6.7).
#[test]
fn delete_removes_the_node_selected() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    let (on_a, _) = canvas_element(&harness, "a, core.mul");
    click_at(&mut harness, on_a, Modifiers::NONE);
    harness.key_press(Key::Delete);
    harness.run_steps(4);
    let text = text_of(&harness, "/p/main.lcf");
    assert!(!text.contains("core.mul") && text.contains("    y = core.add("), "the node goes, its reader stays: {text}");
    undo(&mut harness);
    assert_eq!(text_of(&harness, "/p/main.lcf"), MAIN);
}

/// Backspace removes the wire selected: the input it reached gets a hole, which keeps the place of the positional argument
/// after it (spec 5.1, commands.md `wire.disconnect`), one step.
#[test]
fn backspace_removes_the_wire_selected() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    let (on_a, _) = canvas_element(&harness, "a, core.mul");
    click_at(&mut harness, on_a, Modifiers::NONE);
    harness.state().host.session.borrow_mut().execute("view.select", &json!({ "function": "main", "wires": ["a -> y.a"] })).unwrap();
    harness.run_steps(3);
    harness.key_press(Key::Backspace);
    harness.run_steps(4);
    assert_eq!(text_of(&harness, "/p/main.lcf"), MAIN.replace("core.add(a, 1.0)", "core.add(?, 1.0)"));
    undo(&mut harness);
    assert_eq!(text_of(&harness, "/p/main.lcf"), MAIN, "one step");
}

/// Space with nothing selected: the list holds everything, and the node falls where the pointer is, unwired (notation 2.6).
#[test]
fn space_on_the_empty_canvas_inserts_a_node_where_the_pointer_is() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    let (on_a, _) = canvas_element(&harness, "a, core.mul");
    click_at(&mut harness, on_a + egui::vec2(40.0, 200.0), Modifiers::NONE);
    harness.key_press(Key::Space);
    harness.run_steps(3);
    assert!(harness.query_by_label("core.not").is_some(), "no wire filters the list");
    type_then_enter(&mut harness, "abs");
    let text = text_of(&harness, "/p/main.lcf");
    let body = text
        .split(
            "
layout {",
        )
        .next()
        .unwrap_or_default();
    assert_eq!(
        body,
        MAIN.replace(
            "    y = core.add(a, 1.0)
",
            "    abs = core.abs()
    y = core.add(a, 1.0)
"
        )
    );
    assert!(
        text.contains(
            "
layout {
    node abs at ("
        ),
        "{text}"
    );
}

/// With autopatch turned off, Space on a node selected inserts the node unwired (notation 2.6).
#[test]
fn without_autopatch_space_on_a_node_inserts_it_unwired() {
    let mut harness = harness_with(fake(), Prefs { autopatch: false, ..Prefs::default() });
    harness.run_steps(4);
    let (on_a, _) = canvas_element(&harness, "a, core.mul");
    click_at(&mut harness, on_a, Modifiers::NONE);
    harness.key_press(Key::Space);
    harness.run_steps(3);
    type_then_enter(&mut harness, "abs");
    let text = text_of(&harness, "/p/main.lcf");
    assert!(
        text.contains(
            "    abs = core.abs()
"
        ),
        "{text}"
    );
}

/// A wire drawn back from the constant of an input, dropped on the empty canvas: text that reads as a constant is the
/// first row, and Enter adds it as a named constant wired to the input (notation 2.6, `constant.add`): its line between
/// `a` and `y` in the order of spec 5.2.
#[test]
fn a_constant_typed_in_the_list_goes_on_the_input() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    let (from, _) = canvas_element(&harness, "input b of y, f64, constant 1.0");
    drag_from(&mut harness, from, from + egui::vec2(-200.0, 200.0));
    type_then_enter(&mut harness, "2.5");
    assert_eq!(
        text_of(&harness, "/p/main.lcf")
            .split(
                "
layout {"
            )
            .next()
            .unwrap_or_default(),
        MAIN.replace(
            "    y = core.add(a, 1.0)
",
            "    constant = 2.5
    y = core.add(a, constant)
"
        )
    );
}

/// A function of the panel `Palette` dragged onto the canvas: on the empty canvas, a node fixed where it falls; onto a
/// wire that it can take, the node goes into the wire (notation 2.5).
#[test]
fn a_function_dragged_from_the_palette_is_added_where_it_falls_or_into_a_wire() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    use egui_kittest::kittest::NodeT as _;
    let (from, _) = canvas_element(&harness, "output result of a, f64");
    let (to, _) = canvas_element(&harness, "input a of y, wired from a");
    // `Palette` is the tab behind `Project` (ide.md 2).
    let tab = harness.query_all_by_label("Palette").map(|node| node.rect().center()).next().unwrap();
    click_at(&mut harness, tab, Modifiers::NONE);
    let field =
        harness.query_all_by_role(egui::accesskit::Role::TextInput).find(|node| node.accesskit_node().placeholder() == Some("Search a function"));
    let Some(field) = field.map(|node| node.rect().center()) else {
        let found: Vec<String> =
            harness.query_all_by_role(egui::accesskit::Role::TextInput).map(|node| format!("{:?}", node.accesskit_node())).collect();
        panic!("the palette is open: {found:#?}")
    };
    click_at(&mut harness, field, Modifiers::NONE);
    harness.input_mut().events.push(egui::Event::Text("core.abs".to_owned()));
    harness.run_steps(3);
    let row = |harness: &Harness<'_, State>| harness.query_all_by_label("core.abs").map(|node| node.rect().center()).next().unwrap();
    let start = row(&harness);
    drag_from(&mut harness, start, from + (to - from) / 2.0);
    let wired = MAIN.replace("    y = core.add(a, 1.0)\n", "    abs = core.abs(a)\n    y = core.add(abs, 1.0)\n");
    assert_eq!(text_of(&harness, "/p/main.lcf"), wired);
    undo(&mut harness);
    let start = row(&harness);
    drag_from(&mut harness, start, from + egui::vec2(40.0, 200.0));
    let text = text_of(&harness, "/p/main.lcf");
    assert!(text.contains("    abs = core.abs()\n") && text.contains("\nlayout {\n    node abs at ("), "{text}");
}
