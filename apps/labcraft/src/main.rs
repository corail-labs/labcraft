//! `labcraft`, the desktop viewer of M0v (`docs/architecture.md`, 11.5; `docs/cli.md`, section 8): a list of the
//! functions of the folder, a tab per function opened, the canvas, `Problems`. Read-only. The window is throwaway:
//! `ui-egui` starts with the IDE (M1); what lasts is `diagram`, `editor` and the commands.
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

use std::io::Write;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use craft_dialogs::{Answer, Dialogs, FileDialog};
use craft_settings::{Location, Settings, SettingsStore};
use craft_ui::fonts::{Family, I18nScripts, SystemFonts, UiFonts};
use egui::{FontData, FontFamily};
use labcraft::args::{self, Asked, Options};
use std::sync::mpsc::Receiver;

use labcraft::session::{CommandError, Handle, Published};
use labcraft_editor::{Canvas, Shown, Theme};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Exit code of wrong usage, as for `labcraft-cli` (`docs/cli.md`, section 1).
const WRONG_USAGE: u8 = 2;

/// Inter 4.1 Regular, the face of the panels (architecture 11.5; `ATTRIBUTION.md`).
static INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
/// Hack 3.003 Regular, the face of the notation and of `Problems`.
static HACK: &[u8] = include_bytes!("../../../assets/fonts/Hack-Regular.ttf");

/// The preferences of the viewer, on disk (decided at the start of step 6: the view state is not).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Prefs {
    /// `light`, `dark`, or nothing for light.
    theme: String,
    /// Every node in standard view (notation 3).
    standard_everywhere: bool,
    /// No glide (notation 2.1): egui does not say what the system asks for on the desktop.
    reduced_motion: bool,
}

impl Settings for Prefs {
    const APP_ID: &'static str = "labcraft";
    const SCHEMA_VERSION: u32 = 1;
}

fn main() -> ExitCode {
    // A bug caught in the session says where it happened (`LC0099`, commands.md 3); a bug of the window is printed.
    labcraft_cli::fmt::keep_panics_and_print_them();
    let asked = match args::parse(std::env::args_os().skip(1)) {
        Ok(asked) => asked,
        Err(message) => {
            let _ = write!(std::io::stderr(), "labcraft: {message}\n{}", args::USAGE);
            return ExitCode::from(WRONG_USAGE);
        }
    };
    let options = match asked {
        Asked::Help => {
            let _ = write!(std::io::stdout(), "{}", args::USAGE);
            return ExitCode::SUCCESS;
        }
        Asked::Version => {
            let _ = writeln!(std::io::stdout(), "labcraft {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Asked::Open(options) => options,
    };
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("LabCraft").with_inner_size([1400.0, 860.0]),
        ..eframe::NativeOptions::default()
    };
    let started = eframe::run_native("labcraft", native, Box::new(move |creation| Ok(Box::new(Viewer::new(&creation.egui_ctx, options)?))));
    match started {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "labcraft: {error}");
            ExitCode::FAILURE
        }
    }
}

/// What a dialog was asked for.
enum Ask {
    Folder,
}

struct Viewer {
    session: Handle,
    published: Published,
    canvas: Canvas,
    shown_generation: u64,
    prefs: SettingsStore<Prefs>,
    theme: Theme,
    dialogs: Dialogs<Ask>,
    /// A command that failed, said in the window.
    failure: Option<String>,
    /// The answer of `view.open`, which the window does not wait for: checking a folder takes time.
    opening: Option<Receiver<Result<serde_json::Value, CommandError>>>,
    /// A folder was just opened: fit its first drawing, that of this generation or a later one.
    fit_at: Option<u64>,
}

impl Viewer {
    fn new(ctx: &egui::Context, options: Options) -> Result<Viewer, Box<dyn std::error::Error + Send + Sync>> {
        install_fonts(ctx);
        let (prefs, _) = SettingsStore::<Prefs>::open(Location::Auto);
        let theme = match (options.theme, prefs.get().theme.as_str()) {
            (Some(args::Theme::Dark), _) | (None, "dark") => Theme::Dark,
            _ => Theme::Light,
        };
        ctx.set_theme(if theme == Theme::Dark { egui::Theme::Dark } else { egui::Theme::Light });
        let wake_ctx = ctx.clone();
        let session = Handle::spawn(
            prefs.get().standard_everywhere,
            options.lib_paths.clone(),
            options.style.clone(),
            Box::new(move || wake_ctx.request_repaint()),
        )?;
        let mut canvas = Canvas::new();
        canvas.set_reduced_motion(prefs.get().reduced_motion);
        let wake_ctx = ctx.clone();
        let dialogs = Dialogs::new(Arc::new(move || wake_ctx.request_repaint()));
        let mut viewer = Viewer {
            session,
            published: Published::default(),
            canvas,
            shown_generation: 0,
            prefs,
            theme,
            dialogs,
            failure: None,
            opening: None,
            fit_at: None,
        };
        if let Some(folder) = options.folder {
            viewer.open(&folder);
        }
        Ok(viewer)
    }

    fn open(&mut self, folder: &std::path::Path) {
        self.opening = Some(self.session.open(folder));
        self.failure = None;
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Open a folder…").clicked() {
                self.dialogs.ask(FileDialog::folder().title("Open a library folder"), Ask::Folder);
            }
            ui.label(self.published.folder.as_deref().unwrap_or("no folder open"));
            ui.separator();
            let mut dark = self.theme == Theme::Dark;
            if ui.checkbox(&mut dark, "Dark").changed() {
                self.theme = if dark { Theme::Dark } else { Theme::Light };
                ui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                self.prefs.update(|prefs| prefs.theme = if dark { "dark" } else { "light" }.to_owned());
            }
            let mut standard = self.published.view.standard_everywhere;
            if ui.checkbox(&mut standard, "Standard view everywhere").changed() {
                self.session.post("view.set_standard_view", json!({ "on": standard }));
                self.prefs.update(|prefs| prefs.standard_everywhere = standard);
            }
            let mut reduced = self.prefs.get().reduced_motion;
            if ui.checkbox(&mut reduced, "Reduced motion").changed() {
                self.canvas.set_reduced_motion(reduced);
                self.prefs.update(|prefs| prefs.reduced_motion = reduced);
            }
            if ui.button("Fit").clicked() {
                self.canvas.fit();
            }
        });
        // A tab per function opened.
        ui.horizontal_wrapped(|ui| {
            for name in self.published.view.opened.clone() {
                let shown = self.published.view.function.as_deref() == Some(name.as_str());
                if ui.selectable_label(shown, &name).clicked() && !shown {
                    self.session.post("view.show_function", json!({ "function": name }));
                }
            }
        });
        for notice in [&self.published.notice, &self.failure, &self.published.style_notice].into_iter().flatten() {
            ui.colored_label(ui.visuals().warn_fg_color, notice);
        }
    }

    fn functions(&mut self, ui: &mut egui::Ui) {
        ui.heading("Functions");
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for listed in self.published.view.functions.clone() {
                let shown = self.published.view.function.as_deref() == Some(listed.function.as_str());
                // A function that is not drawn gives the number of errors of its file (architecture 11.5).
                let text = if listed.drawable || listed.errors == 0 {
                    listed.function.clone()
                } else {
                    format!("{}  ({} error{})", listed.function, listed.errors, if listed.errors == 1 { "" } else { "s" })
                };
                if ui.selectable_label(shown, text).on_hover_text(&listed.file).clicked() && !shown {
                    self.session.post("view.show_function", json!({ "function": listed.function }));
                }
            }
        });
    }

    fn problems(&self, ui: &mut egui::Ui) {
        let title = format!("Problems: {} errors, {} warnings", self.published.view.errors, self.published.view.warnings);
        ui.label(egui::RichText::new(title).strong());
        if self.published.view.kept {
            ui.label("This function is not drawable now: its last drawing stays, with the diagnostics of its file.");
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.label(egui::RichText::new(&self.published.problems).monospace());
        });
    }
}

impl eframe::App for Viewer {
    #[expect(clippy::disallowed_methods, reason = "the desktop app, never built for WASM, where the clock traps")]
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let published = self.session.published();
        if let Some(answer) = self.opening.as_ref().and_then(|opening| opening.try_recv().ok()) {
            self.opening = None;
            match answer {
                // The session publishes before it answers: its drawing is this one or a later one.
                Ok(_) => self.fit_at = Some(self.session.published().view.drawing_generation),
                Err(error) => self.failure = Some(error.message),
            }
        }
        if published.view.drawing_generation != self.shown_generation {
            self.shown_generation = published.view.drawing_generation;
            if let Some(drawing) = &published.view.drawing {
                // The glide when the file of the function shown changed; the viewport stays (scenario 6, point 3), but
                // for a folder just opened.
                self.canvas.set_drawing(drawing.clone(), published.view.glide);
                if self.fit_at.is_some_and(|at| published.view.drawing_generation >= at) {
                    self.canvas.fit();
                    self.fit_at = None;
                }
            }
        }
        self.published = published;
        for (ask, answer) in self.dialogs.take_answers() {
            match (ask, answer) {
                (Ask::Folder, Answer::Paths(paths)) => {
                    if let Some(folder) = paths.first() {
                        self.open(folder);
                    }
                }
                (Ask::Folder, Answer::Unavailable(why)) => self.failure = Some(why),
                _ => {}
            }
        }
        egui::Panel::top("labcraft-top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("labcraft-problems").resizable(true).default_size(160.0).show(ui, |ui| self.problems(ui));
        egui::Panel::left("labcraft-functions").resizable(true).default_size(240.0).show(ui, |ui| self.functions(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            if self.published.view.drawing.is_none() {
                ui.centered_and_justified(|ui| ui.label("Open a folder to see its functions."));
                return;
            }
            let shown = Shown { arms: self.published.view.arms.clone() };
            let response = self.canvas.show(ui, self.theme, &shown);
            if let (Some((structure, arm)), Some(function)) = (response.tab, self.published.view.function.clone()) {
                self.session.post("view.set_state", json!({ "function": function, "state": { "arms": { structure: arm } } }));
            }
        });
        self.prefs.tick(Instant::now());
        self.dialogs.show_pending(Some(&*frame));
    }

    fn on_exit(&mut self) {
        let _ = self.prefs.flush();
    }
}

/// The fonts of the window (architecture 11.5): Inter for the panels, Hack for the notation and `Problems`, then the
/// fonts of the system for what they lack. egui is built without its default fonts.
fn install_fonts(ctx: &egui::Context) {
    let notation = FontFamily::Name(labcraft_editor::FAMILY.into());
    UiFonts::new(&I18nScripts)
        .app_face("Inter", FontData::from_static(INTER), &[FontFamily::Proportional])
        .app_face("Hack", FontData::from_static(HACK), &[FontFamily::Monospace, notation])
        .family(Family::named(labcraft_editor::FAMILY).monospaced())
        .system_fonts(SystemFonts::AfterCraft)
        .install(ctx);
}
