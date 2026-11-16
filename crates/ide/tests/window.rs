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

/// A session of `engine`, answered at once on the thread of the test.
struct Fake {
    session: RefCell<Session>,
}

impl Host for Fake {
    fn send(&self, id: &str, params: Value) -> Reply {
        Reply::ready(self.session.borrow_mut().execute(id, &params))
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { view: self.session.borrow().view().clone(), project: Some("/p".to_owned()), notices: Vec::new() }
    }

    fn set_zoom(&self, zoom: f32) {
        self.session.borrow_mut().set_zoom(zoom);
    }
}

fn fake() -> Fake {
    let memory = Memory::new();
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
    let mut session = Session::new(Arc::new(memory), config, false).unwrap();
    session.with_window();
    session.open_on(Some("main".to_owned()));
    session.execute("view.open", &json!({ "folder": "." })).unwrap();
    Fake { session: RefCell::new(session) }
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
    let ide = Ide::new(&egui::Context::default(), Prefs::default(), craft_window::Frame::Desktop);
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
    assert_eq!(view.function.as_deref(), Some("helper"));
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
    harness.get_by_role(egui::accesskit::Role::TextInput).type_text("view.set_view {\"function\": \"main\", \"view\": \"text\"}");
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
    harness.get_by_role(egui::accesskit::Role::TextInput).type_text("view.toggle_panel {\"panel\": \"run\"}");
    harness.run_steps(2);
    harness.key_press(Key::Enter);
    harness.run_steps(3);
    assert!(!harness.state().ide.prefs().layout.unwrap().layout.contains(&labcraft_ide::Pane::Run));
    assert!(harness.state_mut().ide.take_prefs_changed());
    harness.key_press(Key::F1);
    harness.run_steps(3);
    harness.get_by_role(egui::accesskit::Role::TextInput).type_text("view.toggle_panel {\"panel\": \"run\"}");
    harness.run_steps(2);
    harness.key_press(Key::Enter);
    harness.run_steps(3);
    assert!(harness.state().ide.prefs().layout.unwrap().layout.contains(&labcraft_ide::Pane::Run));
}

/// Types a command in the palette, `id {json}`, and runs it.
fn run_in_palette(harness: &mut Harness<'_, State>, typed: &str) {
    harness.key_press(Key::F1);
    harness.run_steps(3);
    harness.get_by_role(egui::accesskit::Role::TextInput).type_text(typed);
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
    assert_eq!(harness.state().host.snapshot().view.function.as_deref(), Some("main"));
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
