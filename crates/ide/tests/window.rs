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
        let session = self.session.borrow();
        let steps = Arc::new(session.steps_done().map(labcraft_ide::LastStep::of).collect());
        Snapshot { view: session.view().clone(), project: Some("/p".to_owned()), notices: Vec::new(), session: 0, waiting: None, steps }
    }

    fn set_zoom(&self, function: &str, zoom: f32) {
        self.session.borrow_mut().set_zoom(function, zoom);
    }

    fn set_user_settings(&self, keys: &serde_json::Map<String, Value>) {
        self.session.borrow_mut().set_user_settings(keys);
    }

    fn set_gesture(&self, gesture: Option<labcraft_engine::Gesture>) {
        self.session.borrow_mut().set_gesture(gesture);
    }

    fn serves_agents(&self) -> bool {
        true
    }
}

fn fake() -> Fake {
    fake_with(&[])
}

/// [`fake`], with more files.
fn fake_with(files: &[(&str, &str)]) -> Fake {
    let memory = Arc::new(Memory::new());
    memory.put("/p/main.lcf", MAIN);
    memory.put("/p/helper.lcf", HELPER);
    memory.put("/p/broken.lcf", BROKEN);
    for (path, text) in files {
        memory.put(path, *text);
    }
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

/// The settings of the window with one key of the user (commands.md 9.7).
fn prefs_with(key: &str, value: Value) -> Prefs {
    let mut prefs = Prefs::default();
    prefs.settings.insert(key.to_owned(), value);
    prefs
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
        let mut harness = harness_with(fake(), prefs_with(labcraft_engine::settings::RECORD_VALUES, json!(record)));
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
    let mut harness = harness_with(fake(), prefs_with(labcraft_engine::settings::RECORD_VALUES, json!(true)));
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

/// A function that no longer draws keeps its last drawing, read only, under a band that names its first error and shows
/// `Problems`; the band goes when it draws again (ide.md 3).
#[test]
fn a_function_that_no_longer_draws_keeps_its_drawing_under_a_band_until_it_draws_again() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    let band = "Not drawn since the last change";
    assert!(harness.query_by_label(band).is_none());
    // `a` selected before: the session keeps it on the drawing kept.
    let (on_a, _) = canvas_element(&harness, "a, core.mul");
    click_at(&mut harness, on_a, egui::Modifiers::NONE);
    assert_eq!(selection(&harness).nodes, ["a"]);
    // `nowhere` is a value that nothing names, `LC2001`, which the drawing cannot get past (cli.md 7.1).
    let broken = MAIN.replace("core.mul(x, 2.0)", "core.mul(nowhere, 2.0)");
    harness.state().host.session.borrow_mut().execute("file.write", &json!({ "path": "main.lcf", "text": broken })).unwrap();
    harness.state().host.session.borrow_mut().files_changed();
    harness.run_steps(4);
    assert!(harness.state().host.session.borrow().view().main.kept);
    assert!(harness.query_by_label(band).is_some(), "the band");
    // Its first error: the code and the place of `nowhere`, line 6, column 18 of the text written above.
    assert!(harness.query_by_label_contains("LC2001  main.lcf 6:18  ·  unknown value `nowhere`").is_some(), "the first error");
    // "Show in Problems" brings `Problems` forward, from behind `Run`, and back once closed.
    click_role(&mut harness, "Run", egui::accesskit::Role::Tab);
    assert!(harness.query_by_label("Errors only").is_none());
    harness.query_by_label("Show in Problems ›").expect("the end of the band").click();
    harness.run_steps(4);
    assert!(harness.query_by_label("Errors only").is_some(), "Problems in front");
    shot(&mut harness, "kept");
    run_in_palette(&mut harness, "view.toggle_panel {\"panel\": \"problems\", \"on\": false}");
    harness.run_steps(3);
    assert!(!harness.state().ide.prefs().layout.unwrap().layout.contains(&labcraft_ide::Pane::Problems));
    harness.query_by_label("Show in Problems ›").expect("the end of the band").click();
    harness.run_steps(4);
    assert!(harness.query_by_label("Errors only").is_some(), "Problems shown again");
    // Read only: a click on a node of the drawing kept changes nothing, and Delete, from its key or the palette, on the
    // selection kept, edits nothing.
    let (on_y, _) = canvas_element(&harness, "y, core.add");
    click_at(&mut harness, on_y, egui::Modifiers::NONE);
    assert_eq!(selection(&harness).nodes, ["a"], "the drawing kept is not the file: no gesture on it");
    harness.key_press(Key::Delete);
    harness.run_steps(3);
    // `ide.delete`: Delete from the palette, on the selection.
    let delete = "ide.delete";
    run_in_palette(&mut harness, delete);
    harness.run_steps(3);
    assert_eq!(text_of(&harness, "/p/main.lcf"), broken, "nothing of the window edits it");
    // Drawn again: the band goes, and the same command edits the file.
    harness.state().host.session.borrow_mut().execute("file.write", &json!({ "path": "main.lcf", "text": MAIN })).unwrap();
    harness.state().host.session.borrow_mut().files_changed();
    harness.run_steps(4);
    assert!(harness.query_by_label(band).is_none(), "the band goes");
    // The palette stays open on the refusal.
    harness.key_press(Key::Escape);
    harness.run_steps(3);
    let (on_y, _) = canvas_element(&harness, "y, core.add");
    click_at(&mut harness, on_y, egui::Modifiers::NONE);
    assert_eq!(selection(&harness).nodes, ["y"], "and the drawing can be edited again");
    run_in_palette(&mut harness, delete);
    harness.run_steps(3);
    assert_ne!(text_of(&harness, "/p/main.lcf"), MAIN, "the command that was refused");
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
    let mut harness = harness_with(fake(), prefs_with(labcraft_engine::settings::AUTOPATCH, json!(false)));
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

/// A function of a library with a mask (spec 11.3), whose block the view `Block` shows (ide.md 3).
const SET: &str = "labcraft 1

use core

fn set(x: f64, n: f64) -> (y: f64) {
    y = core.add(x, n)
}

terminals {
    top 5
    top 3 n
}
";
const PSU: &str = "[mask]
band = \"PSU\"
color = \"#B45309\"
";

fn block_harness() -> Harness<'static, State> {
    let mut harness = harness(fake_with(&[("/p/psu/set.lcf", SET), ("/p/psu/labcraft.toml", PSU)]));
    let host = &harness.state().host;
    host.send("view.show_function", json!({ "function": "psu.set" })).take().unwrap().unwrap();
    host.send("view.set_view", json!({ "function": "psu.set", "view": "block" })).take().unwrap().unwrap();
    harness.run_steps(6);
    harness
}

#[test]
fn the_view_block_shows_the_block_and_what_comes_from_the_mask() {
    let mut harness = block_harness();
    for label in ["Outline", "Band", "Top", "Left", "Colour", "Fill", "Stroke", "Reset the block", "Real size"] {
        assert!(harness.query_by_label_contains(label).is_some(), "no `{label}`");
    }
    // The colour of the mask, said to come from it.
    assert!(harness.query_by_label_contains("#B45309").is_some());
    assert!(harness.query_by_label_contains("from the mask of psu").is_some());
    shot(&mut harness, "block");
}

#[test]
fn the_template_chosen_in_the_view_block_writes_the_outline() {
    let mut harness = block_harness();
    // The first list of the view: the outline.
    harness.query_all_by_role(egui::accesskit::Role::ComboBox).next().expect("the list of the outline").click();
    harness.run_steps(3);
    harness.get_by_label("D").click();
    harness.run_steps(6);
    let text = text_of(&harness, "/p/psu/set.lcf");
    assert!(
        text.contains(
            "outline {
    template d
}"
        ),
        "{text}"
    );
    // The drawing that the session publishes holds the block as the file now says it.
    harness.state().host.session.borrow_mut().files_changed();
    harness.run_steps(4);
    let view = harness.state().host.snapshot().view;
    let own = view.main.drawing.as_ref().and_then(|drawing| drawing.block.as_ref()).expect("the block of the drawing");
    assert_eq!(own.written.outline, Some(labcraft_diagram::OutlineChoice::Template("d".to_owned())));
    shot(&mut harness, "block-d");
}

#[test]
fn reset_in_the_view_block_removes_the_sections() {
    let mut harness = block_harness();
    harness.get_by_label("Reset the block").click();
    harness.run_steps(6);
    let text = text_of(&harness, "/p/psu/set.lcf");
    assert!(!text.contains("terminals {"), "{text}");
}

/// Runs a command of the palette, by its id and its JSON.
fn from_palette(harness: &mut Harness<'_, State>, text: &str) {
    harness.key_press(Key::F1);
    harness.run_steps(3);
    palette_field(harness).type_text(text);
    harness.run_steps(2);
    harness.key_press(Key::Enter);
    harness.run_steps(4);
}

/// The `Settings` dialog (ide.md 2): a choice and a toggle write the store shown, the user's or the project's, by
/// `settings.set`; the window keeps the keys of the user, which its app writes.
#[test]
fn the_settings_dialog_writes_the_store_of_the_user_or_of_the_project() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    // The gear of the top bar.
    harness.get_by_label("Settings").click();
    harness.run_steps(4);
    assert!(harness.query_by_label("Block fill").is_some(), "open");
    shot(&mut harness, "settings");
    harness.get_by_label("Neutral").click();
    harness.run_steps(4);
    let settings = harness.state().host.snapshot().view.settings.clone();
    assert_eq!(settings.user.get("appearance.block_fill"), Some(&json!("neutral")));
    assert_eq!(harness.state().ide.prefs().settings.get("appearance.block_fill"), Some(&json!("neutral")), "kept by the window");
    assert!(harness.state_mut().ide.take_prefs_changed());
    // The switch of the dialog, drawn after the tab `Project` of the dock.
    harness.query_all_by_label("Project").last().unwrap().click();
    harness.run_steps(3);
    // Below the fold of the dialog: clicked through accesskit, as a screen reader would.
    harness.get_by_label("Drag on the empty canvas pans").click_accesskit();
    harness.run_steps(4);
    let written = harness.state().host.memory.get("/p/.labcraft/config.toml").map(|bytes| String::from_utf8(bytes).unwrap());
    assert_eq!(written.as_deref(), Some("[editor]\ndrag_pans = true\n"));
    assert!(!harness.state().ide.prefs().settings.contains_key("editor.drag_pans"), "the user's store is not written");
    shot(&mut harness, "settings-project");
    // `Reset` takes the key out of the store shown.
    harness.get_by_label("Reset").click_accesskit();
    harness.run_steps(4);
    let written = harness.state().host.memory.get("/p/.labcraft/config.toml").map(|bytes| String::from_utf8(bytes).unwrap());
    assert_eq!(written.as_deref(), Some(""));
    harness.get_by_label("Close").click();
    harness.run_steps(3);
    assert!(harness.query_by_label("Block fill").is_none(), "closed");
}

/// A toggle of the menus writes the key of the user (commands.md 9.7), through the session when a project is open.
#[test]
fn a_toggle_of_the_menus_writes_the_key_of_the_user() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    from_palette(&mut harness, "ide.dark");
    let settings = harness.state().host.snapshot().view.settings.clone();
    assert_eq!(settings.user.get("appearance.theme"), Some(&json!("dark")));
    assert_eq!(harness.state().ide.prefs().settings.get("appearance.theme"), Some(&json!("dark")));
    from_palette(&mut harness, "ide.dark");
    assert_eq!(harness.state().host.snapshot().view.settings.user.get("appearance.theme"), Some(&json!("light")));
}

/// Color by type (notation 4.3) is on by default; the status bar switches it, and the drawing follows (ide.md 2).
#[test]
fn the_status_bar_switches_color_by_type_and_the_drawing_follows() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    let drawn = |harness: &Harness<'_, State>| {
        let snapshot = harness.state().host.snapshot();
        snapshot.view.windows().filter_map(|window| window.drawing.as_ref()).map(|drawing| drawing.by_type).collect::<Vec<_>>()
    };
    assert_eq!(drawn(&harness), vec![true], "on by default");
    harness.get_by_label("Types in colour").click();
    harness.run_steps(4);
    assert_eq!(harness.state().host.snapshot().view.settings.user.get("appearance.color_by_type"), Some(&json!(false)));
    assert_eq!(drawn(&harness), vec![false]);
    harness.get_by_label("Types in one ink").click();
    harness.run_steps(4);
    assert_eq!(drawn(&harness), vec![true]);
}

/// A toggle whose key the project holds shows the project's value and is not enabled: the dialog changes it (ide.md 2).
#[test]
fn a_toggle_whose_key_the_project_holds_is_not_enabled() {
    let mut harness = harness(fake_with(&[(
        "/p/.labcraft/config.toml",
        "[editor]
drag_pans = true
",
    )]));
    harness.run_steps(4);
    from_palette(&mut harness, "Drag on the empty canvas pans");
    let settings = harness.state().host.snapshot().view.settings.clone();
    assert_eq!(settings.user.get("editor.drag_pans"), None, "the user's store is not written");
    assert_eq!(settings.project.get("editor.drag_pans"), Some(&json!(true)));
}

/// The keys that the window keeps reach a session that does not hold them (`Stores::given`).
#[test]
fn the_keys_of_the_user_reach_the_session() {
    let mut harness = harness_with(fake(), prefs_with("editor.drag_pans", json!(true)));
    harness.run_steps(4);
    let settings = harness.state().host.snapshot().view.settings.clone();
    assert!(settings.given);
    assert_eq!(settings.user.get("editor.drag_pans"), Some(&json!(true)));
}

/// The view `Text` of `main`, its field focused at the end of the text (commands.md 9.1; step 7a).
fn text_of_main() -> Harness<'static, State> {
    let mut harness = harness(fake());
    harness.state().host.session.borrow_mut().execute("view.set_view", &json!({ "function": "main", "view": "text" })).unwrap();
    harness.run_steps(4);
    harness.get_by_label("Text of main.lcf").click();
    harness.run_steps(1);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::End);
    harness.run_steps(1);
    harness
}

fn file(harness: &Harness<'_, State>, path: &str) -> Option<String> {
    harness.state().host.memory.get(path).map(|bytes| String::from_utf8(bytes).unwrap())
}

/// A text that parses is written after a pause of the keys, one step of history; its errors of `check` do not stop it.
#[test]
fn a_text_typed_is_written_after_a_pause_when_it_parses() {
    let mut harness = text_of_main();
    harness.get_by_label("Text of main.lcf").type_text("\n// a note\n");
    harness.run_steps(1);
    assert_eq!(file(&harness, "/p/main.lcf").as_deref(), Some(MAIN), "not before the pause");
    harness.run_steps(4);
    assert_eq!(file(&harness, "/p/main.lcf"), Some(format!("{MAIN}\n// a note\n")));
    let history = harness.state().host.session.borrow_mut().execute("history.list", &json!({})).unwrap();
    assert_eq!(history["steps"][0]["label"], "Edit Text", "{history}");
    shot(&mut harness, "text-written");
}

/// A text that does not parse stays in the view with its banner, and in the recovery folder; the file keeps its text,
/// and the diagram of the function cannot be edited until the text parses again.
#[test]
fn a_text_that_does_not_parse_is_kept_and_locks_the_diagram() {
    let mut harness = text_of_main();
    harness.get_by_label("Text of main.lcf").type_text("fn (");
    harness.run_steps(5);
    assert_eq!(file(&harness, "/p/main.lcf").as_deref(), Some(MAIN));
    assert_eq!(file(&harness, "/p/.labcraft/recovery/main.lcf"), Some(format!("{MAIN}fn (")));
    assert!(harness.query_by_label_contains("The text does not read").is_some());
    harness.state().host.session.borrow_mut().execute("view.set_view", &json!({ "function": "main", "view": "text", "split": "diagram" })).unwrap();
    harness.run_steps(3);
    assert!(harness.query_by_label_contains("fix it in the view Text").is_some(), "the diagram says why it cannot be edited");
    shot(&mut harness, "text-unparsed");
    // Nor does a command of the palette edit it.
    run_in_palette(&mut harness, "node.add {\"file\": \"main.lcf\", \"call\": \"core.abs\"}");
    harness.run_steps(3);
    assert_eq!(file(&harness, "/p/main.lcf").as_deref(), Some(MAIN), "refused while the text does not read");
    // Fixed: written, the copy gone, the banners gone. The split gave the field another place: it takes the focus again.
    harness.get_by_label("Text of main.lcf").click();
    harness.run_steps(1);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::End);
    for _ in 0..4 {
        harness.key_press(Key::Backspace);
    }
    harness.run_steps(5);
    assert_eq!(file(&harness, "/p/.labcraft/recovery/main.lcf"), None);
    // The banner of the view goes; the notice of the refused command stays until dismissed.
    assert!(harness.query_by_label_contains("The file keeps the last text that read").is_none());
}

/// Ctrl+Z in the view undoes in the field, not in the history of the project (notation 2.6: the focus decides).
#[test]
fn ctrl_z_in_the_text_undoes_the_field() {
    let mut harness = text_of_main();
    harness.get_by_label("Text of main.lcf").type_text("// x");
    harness.run_steps(5);
    assert_eq!(file(&harness, "/p/main.lcf"), Some(format!("{MAIN}// x")));
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run_steps(5);
    // The field went back, and its text was written again: two steps, none undone by the history.
    assert_eq!(file(&harness, "/p/main.lcf").as_deref(), Some(MAIN));
    let history = harness.state().host.session.borrow_mut().execute("history.list", &json!({})).unwrap();
    assert_eq!(history["steps"].as_array().map(Vec::len), Some(2), "{history}");
}

/// An edit from outside while the view holds no text of its own: the view follows the file.
#[test]
fn the_text_follows_a_file_changed_elsewhere() {
    let mut harness = text_of_main();
    let changed = MAIN.replace("2.0", "3.0");
    {
        // As the app does when the files it watches change.
        let mut session = harness.state().host.session.borrow_mut();
        session.execute("file.write", &json!({ "path": "main.lcf", "text": changed })).unwrap();
        session.files_changed();
    }
    harness.run_steps(4);
    harness.get_by_label("Text of main.lcf").type_text("// y");
    harness.run_steps(5);
    assert_eq!(file(&harness, "/p/main.lcf"), Some(format!("{changed}// y")));
}

/// A text of the recovery folder newer than its file is offered back when the project opens (architecture 12.6).
#[test]
fn a_recovered_text_is_offered_back() {
    let fake = fake_with(&[]);
    fake.memory.put("/p/.labcraft/recovery/main.lcf", format!("{MAIN}fn ("));
    let mut harness = harness(fake);
    harness.run_steps(2);
    assert!(harness.query_by_label_contains("A text that did not read was kept for `main.lcf`").is_some());
    harness.state().host.session.borrow_mut().execute("view.set_view", &json!({ "function": "main", "view": "text" })).unwrap();
    harness.run_steps(3);
    harness.get_by_label("Take it back").click();
    harness.run_steps(5);
    assert!(harness.query_by_label_contains("The text does not read").is_some());
    assert_eq!(file(&harness, "/p/main.lcf").as_deref(), Some(MAIN));
}

/// The file changed while a text typed waited for its pause: the text stays, and the banner offers the file or the text
/// (decision of the owner, 2026-11-21).
#[test]
fn a_file_changed_while_typing_offers_the_file_or_the_text() {
    let mut harness = text_of_main();
    harness.get_by_label("Text of main.lcf").type_text("// mine");
    harness.run_steps(1);
    let changed = MAIN.replace("2.0", "3.0");
    {
        let mut session = harness.state().host.session.borrow_mut();
        session.execute("file.write", &json!({ "path": "main.lcf", "text": changed })).unwrap();
        session.files_changed();
    }
    harness.run_steps(5);
    assert!(harness.query_by_label_contains("The file changed while you typed").is_some());
    assert_eq!(file(&harness, "/p/main.lcf"), Some(changed.clone()), "not written over");
    assert_eq!(file(&harness, "/p/.labcraft/recovery/main.lcf"), Some(format!("{MAIN}// mine")), "kept");
    shot(&mut harness, "text-stale");
    harness.get_by_label("Keep my text").click();
    harness.run_steps(4);
    assert_eq!(file(&harness, "/p/main.lcf"), Some(format!("{MAIN}// mine")));
    assert!(harness.query_by_label_contains("The file changed while you typed").is_none());
}

/// "Show the file": the view takes the file, and the text typed is one Ctrl+Z away.
#[test]
fn show_the_file_takes_the_file_and_keeps_the_text_in_the_undo() {
    let mut harness = text_of_main();
    harness.get_by_label("Text of main.lcf").type_text("// mine");
    harness.run_steps(1);
    let changed = MAIN.replace("2.0", "3.0");
    {
        let mut session = harness.state().host.session.borrow_mut();
        session.execute("file.write", &json!({ "path": "main.lcf", "text": changed })).unwrap();
        session.files_changed();
    }
    harness.run_steps(5);
    harness.get_by_label("Show the file").click();
    harness.run_steps(4);
    assert!(harness.query_by_label_contains("The file changed while you typed").is_none());
    assert_eq!(file(&harness, "/p/main.lcf"), Some(changed));
    assert_eq!(file(&harness, "/p/.labcraft/recovery/main.lcf"), Some(format!("{MAIN}// mine")), "kept");
    // One Ctrl+Z in the view gives the text typed back, which is then written.
    harness.get_by_label("Text of main.lcf").click();
    harness.run_steps(1);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run_steps(5);
    assert_eq!(file(&harness, "/p/main.lcf"), Some(format!("{MAIN}// mine")));
}

/// A file with CR LF line endings is written on the first pause, not taken for a file changed elsewhere: the revision
/// is that of its bytes (code review of 7a).
#[test]
fn a_file_with_cr_lf_is_written_at_the_first_pause() {
    let fake = fake();
    let windows = MAIN.replace('\n', "\r\n");
    fake.memory.put("/p/main.lcf", windows.as_str());
    fake.session.borrow_mut().files_changed();
    let mut harness = harness(fake);
    harness.state().host.session.borrow_mut().execute("view.set_view", &json!({ "function": "main", "view": "text" })).unwrap();
    harness.run_steps(4);
    harness.get_by_label("Text of main.lcf").click();
    harness.run_steps(1);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::End);
    harness.get_by_label("Text of main.lcf").type_text("// z");
    harness.run_steps(5);
    assert!(harness.query_by_label_contains("The file changed while you typed").is_none());
    assert_eq!(file(&harness, "/p/main.lcf"), Some(format!("{MAIN}// z")), "written, its line endings LF");
}

/// The cursor of the view `Text` of `main` at a line and a column, from 0, by the keys.
fn cursor_to(harness: &mut Harness<'_, State>, line: usize, column: usize) {
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Home);
    for _ in 0..line {
        harness.key_press(Key::ArrowDown);
    }
    for _ in 0..column {
        harness.key_press(Key::ArrowRight);
    }
    harness.run_steps(1);
}

/// The error of a text typed that does not parse is in `Problems`, before the diagnostics of its file (step 7b).
#[test]
fn the_error_of_a_text_typed_is_in_problems() {
    let mut harness = text_of_main();
    harness.get_by_label("Text of main.lcf").type_text("fn (");
    harness.run_steps(5);
    assert!(harness.query_by_label_contains("The text typed does not read").is_some(), "in Problems");
    shot(&mut harness, "text-unparsed-underlined");
}

/// The field keeps the keyboard while a banner shows above it, or goes: what is typed goes on landing in the text (the
/// owner's test of step 7b).
#[test]
fn the_field_keeps_the_focus_as_a_banner_comes_and_goes() {
    use egui_kittest::kittest::NodeT as _;
    let mut harness = text_of_main();
    let focused = |harness: &Harness<'_, State>| harness.get_by_label("Text of main.lcf").accesskit_node().is_focused();
    harness.input_mut().events.push(egui::Event::Text("\nfn (".to_owned()));
    harness.run_steps(5);
    assert!(harness.query_by_label_contains("The text does not read").is_some(), "the banner shows");
    assert!(focused(&harness), "focused under the banner");
    harness.input_mut().events.push(egui::Event::Text("x".to_owned()));
    harness.run_steps(5);
    assert_eq!(file(&harness, "/p/.labcraft/recovery/main.lcf"), Some(format!("{MAIN}\nfn (x")));
    // Back to the text that reads: the banner goes, and the field still has the keyboard.
    for _ in 0.."\nfn (x".len() {
        harness.key_press(Key::Backspace);
    }
    harness.run_steps(5);
    assert!(harness.query_by_label_contains("The text does not read").is_none(), "the banner went");
    assert!(focused(&harness), "focused once the banner went");
}

/// Completion opens at the first character of a name typed, and Enter takes the entry chosen (ide.md 7): at the top of
/// the file, after `us`, the word `use`.
#[test]
fn completion_opens_as_a_name_is_typed_and_enter_takes_it() {
    let mut harness = text_of_main();
    harness.get_by_label("Text of main.lcf").type_text("\nus");
    harness.run_steps(2);
    assert!(harness.query_by_label("use").is_some(), "the list shows `use`");
    shot(&mut harness, "text-completion");
    harness.key_press(Key::Enter);
    harness.run_steps(5);
    // `use` alone does not parse: the text is kept, and holds the word taken, with no line ending added by Enter.
    assert_eq!(file(&harness, "/p/.labcraft/recovery/main.lcf"), Some(format!("{MAIN}\nuse")));
}

/// Ctrl+Space lists what a body can write, the values of the function first: Enter takes the first, `a`.
#[test]
fn ctrl_space_lists_the_values_of_the_body_first() {
    let mut harness = text_of_main();
    // The end of the line `    y = core.add(a, 1.0)`, then a new line, indented as it is.
    cursor_to(&mut harness, 6, 0);
    harness.key_press(Key::End);
    harness.key_press(Key::Enter);
    harness.key_press_modifiers(Modifiers::CTRL, Key::Space);
    harness.run_steps(2);
    harness.key_press(Key::Enter);
    harness.run_steps(5);
    let expected = MAIN.replace("    y = core.add(a, 1.0)\n", "    y = core.add(a, 1.0)\n    a\n");
    assert_eq!(file(&harness, "/p/.labcraft/recovery/main.lcf"), Some(expected), "a line `a` alone does not parse: kept");
}

/// F12 on a value read takes the cursor to the line that binds it (decision of the owner, 2026-11-21).
#[test]
fn f12_goes_to_the_binding_of_a_value() {
    let mut harness = text_of_main();
    // On `a` in `    y = core.add(a, 1.0)`: line 6, column 17.
    cursor_to(&mut harness, 6, 17);
    harness.key_press(Key::F12);
    harness.run_steps(2);
    // The cursor is at `a` of `    a = core.mul(x, 2.0)`: what is typed goes there.
    harness.get_by_label("Text of main.lcf").type_text("z");
    harness.run_steps(5);
    assert_eq!(file(&harness, "/p/main.lcf"), Some(MAIN.replace("    a = core.mul", "    za = core.mul")));
}

/// Shift+Alt+F in the view writes the text typed, then formats its file (`file.format`).
#[test]
fn shift_alt_f_formats_the_file_once_the_text_is_written() {
    let mut harness = text_of_main();
    harness.get_by_label("Text of main.lcf").type_text("\n\n");
    harness.key_press_modifiers(Modifiers::SHIFT | Modifiers::ALT, Key::F);
    harness.run_steps(6);
    // The canonical form ends the file with one line ending (spec 5.2).
    assert_eq!(file(&harness, "/p/main.lcf").as_deref(), Some(MAIN));
    let history = harness.state().host.session.borrow_mut().execute("history.list", &json!({})).unwrap();
    assert!(history["steps"].as_array().is_some_and(|steps| steps.len() >= 2), "written, then formatted: {history}");
}

/// A diagnostic of the check is underlined in the view, and its message and code show under the pointer at rest (ide.md
/// 7): `nowhere`, which no line binds, in `broken`.
#[test]
fn a_diagnostic_is_underlined_and_its_message_shows_on_hover() {
    let mut harness = harness(fake());
    harness.state().host.session.borrow_mut().execute("view.show_function", &json!({ "function": "broken" })).unwrap();
    harness.state().host.session.borrow_mut().execute("view.set_view", &json!({ "function": "broken", "view": "text" })).unwrap();
    harness.run_steps(6);
    let field = harness.get_by_label("Text of broken.lcf").rect();
    // `nowhere` on line 5 (from 0), at columns 17 to 24: the pointer on its fourth character. The gutter is five
    // characters wide (three digits and two spaces), in the monospace face of 13 points.
    let glyph = harness.ctx.fonts_mut(|fonts| fonts.glyph_width(&egui::FontId::monospace(13.0), '0'));
    let row = harness.ctx.fonts_mut(|fonts| fonts.row_height(&egui::FontId::monospace(13.0)));
    let at = egui::pos2(field.min.x + glyph * (5.0 + 20.5), field.min.y + row * 5.5);
    harness.input_mut().events.push(egui::Event::PointerMoved(at));
    harness.run_steps(40);
    assert!(harness.query_by_label_contains("error[LC").is_some(), "the message of the diagnostic, as the hover writes it");
    shot(&mut harness, "text-hover");
}

/// The first connection of an agent asks (ide.md 8): the three policies, `Connect` once one is chosen, and a policy
/// kept for that client and that project written in `ai.policies` by `settings.set`, under the name of the client.
#[test]
fn the_first_connection_of_an_agent_asks_for_a_policy() {
    use labcraft_ide::agents::{Answer, Asks, Policy, Question};
    let mut harness = harness(fake());
    let question = |id| Question {
        id,
        client: "Claude Code".to_owned(),
        signed: "Claude Code 2".to_owned(),
        project: "/p".to_owned(),
        asks: Asks::Connect,
        seconds_left: 58,
    };
    harness.state_mut().ide.set_agent_questions(vec![question(1)]);
    harness.run_steps(4);
    assert!(harness.query_by_label("Claude Code 2 wants to connect").is_some(), "asked, by the name it signs with");
    shot(&mut harness, "agent-connect");
    harness.get_by_label("Edits").click();
    harness.run_steps(2);
    harness.get_by_label("Remember for this client and this project").click();
    harness.run_steps(2);
    harness.get_by_label("Connect").click();
    harness.run_steps(4);
    assert_eq!(harness.state_mut().ide.take_agent_answers(), vec![(1, Answer::Policy { policy: Policy::Edits, keep: true })]);
    let kept = harness.state().host.snapshot().view.settings.user.get("ai.policies").cloned();
    assert_eq!(kept, Some(json!({ "Claude Code": { "/p": "edits" } })));
    // A command that the policy does not let run alone.
    let command =
        Question { asks: Asks::Command { label: "Start Run".to_owned(), effects: vec![labcraft_engine::registry::Effect::Run] }, ..question(3) };
    harness.state_mut().ide.set_agent_questions(vec![command]);
    harness.run_steps(4);
    assert!(harness.query_by_label("Claude Code 2 asks: Start Run").is_some());
    shot(&mut harness, "agent-command");
    harness.get_by_label("Refuse").click();
    harness.run_steps(3);
    assert_eq!(harness.state_mut().ide.take_agent_answers(), vec![(3, Answer::Refuse)]);
}

fn gesture(harness: &Harness<'_, State>) -> Option<labcraft_engine::Gesture> {
    harness.state().host.session.borrow().gesture()
}

/// The window tells the session when a gesture starts and ends (architecture 12.5): a drag of a node while it goes, a
/// text typed until it is written (decision of the owner, 2026-11-21).
#[test]
fn the_window_says_a_drag_and_a_text_typed_while_they_go() {
    let mut harness = harness(fake());
    harness.run_steps(4);
    let (on_a, _) = canvas_element(&harness, "a, core.mul");
    let button =
        |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let to = on_a + egui::vec2(60.0, 40.0);
    harness.input_mut().events.extend([egui::Event::PointerMoved(on_a), button(on_a, true)]);
    harness.run_steps(1);
    harness.input_mut().events.push(egui::Event::PointerMoved(on_a + egui::vec2(30.0, 20.0)));
    harness.run_steps(1);
    assert_eq!(gesture(&harness), Some(labcraft_engine::Gesture::Move));
    harness.input_mut().events.extend([egui::Event::PointerMoved(to), button(to, false)]);
    harness.run_steps(4);
    assert_eq!(gesture(&harness), None);
    // A text: from the first key not written to its writing.
    let mut harness = text_of_main();
    assert_eq!(gesture(&harness), None, "a field with the focus holds nothing");
    harness.get_by_label("Text of main.lcf").type_text("\n// a note\n");
    harness.run_steps(1);
    assert_eq!(gesture(&harness), Some(labcraft_engine::Gesture::Field));
    harness.run_steps(4);
    assert_eq!(file(&harness, "/p/main.lcf"), Some(format!("{MAIN}\n// a note\n")));
    assert_eq!(gesture(&harness), None);
}

/// A text that does not parse holds the lock while its tab is open (decision of the owner, 2026-11-21); its tab closed,
/// no field shows it, and it holds nothing.
#[test]
fn a_text_that_does_not_parse_holds_the_lock_until_its_tab_closes() {
    let mut harness = text_of_main();
    harness.get_by_label("Text of main.lcf").type_text("fn (");
    harness.run_steps(5);
    assert_eq!(gesture(&harness), Some(labcraft_engine::Gesture::Field));
    harness.state().host.session.borrow_mut().execute("view.close_function", &json!({ "function": "main" })).unwrap();
    harness.run_steps(4);
    assert_eq!(gesture(&harness), None);
}

/// What the window shows of an agent (ide.md 8): the pill, then for its step the notice and the highlight of what it
/// touched; Undo on the notice takes the step back; with AI off, nothing of it.
#[test]
fn an_edit_of_an_agent_shows_in_its_pill_its_notice_and_its_highlight_and_the_notice_undoes_it() {
    use labcraft_ide::agents::{Connected, Policy};
    let mut harness = harness(fake());
    assert!(harness.query_by_label("No agent").is_some());
    let claude = Connected { id: 1, client: "claude".to_owned(), signed: "claude".to_owned(), project: "/p".to_owned(), policy: Some(Policy::Edits) };
    harness.state_mut().ide.set_agents(vec![claude]);
    harness.run_steps(2);
    assert!(harness.query_by_label("● claude · Edits").is_some());
    let before = text_of(&harness, "/p/main.lcf");
    let agent = labcraft_engine::Author::Agent("claude".to_owned());
    {
        // A call of an agent, as the session of the app runs it.
        let mut session = harness.state().host.session.borrow_mut();
        session.execute_as(&agent, "node.add", &json!({ "file": "main.lcf", "call": "core.not" })).unwrap();
    }
    harness.run_steps(2);
    // Its file, written, is drawn again once the watcher of the app sees it: after the step.
    harness.state().host.session.borrow_mut().files_changed();
    harness.run_steps(2);
    assert!(harness.query_by_label_contains("in main").is_some(), "the notice says what it did, and where");
    assert!(harness.state_mut().ide.shown_canvas("main").unwrap().0.highlighted().is_some_and(|touched| !touched.nodes.is_empty()));
    click_role(&mut harness, "Undo", egui::accesskit::Role::Button);
    assert_eq!(text_of(&harness, "/p/main.lcf"), before, "Undo on the notice takes the step back");
    assert!(harness.query_by_label_contains("in main").is_none(), "and the notice goes");
    // AI off: no pill.
    let mut off = harness_with(fake(), prefs_with(labcraft_engine::settings::AI_ENABLED, json!(false)));
    off.run_steps(2);
    assert!(off.query_by_label("No agent").is_none());
    assert!(!off.state().ide.ai_enabled());
}
