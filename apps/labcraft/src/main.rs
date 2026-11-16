//! `labcraft`, the desktop IDE (`docs/architecture.md`, 12.9; `docs/ide.md`; `docs/cli.md`, section 8): the window of
//! `ide` on eframe, the session of the project on a thread of its own, the disk, the dialogs, the projects opened
//! last. One project per window: a project opened from a window that shows one opens in a window of its own.
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
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use craft_dialogs::{Answer, Dialogs, FileDialog};
use craft_settings::{Location, Settings, SettingsStore};
use craft_ui::fonts::{Family, I18nScripts, SystemFonts, UiFonts};
use egui::{FontData, FontFamily};
use kit_shell::start::Recent;
use labcraft::args::{self, Asked, Options};
use labcraft::examples;
use labcraft::frames::{self, Frames};
use labcraft::session::{self, Handle, Opening};
use labcraft_diagram::style;
use labcraft_editor::Canvas;
use labcraft_ide::{Host, Ide, Outward, Reply, StartList};
use serde::{Deserialize, Serialize};

/// Exit code of wrong usage, as for `labcraft-cli` (`docs/cli.md`, section 1).
const WRONG_USAGE: u8 = 2;

/// At most this many recent projects on the start page *(proposed)* (ide.md 10).
const MAX_RECENT: usize = 12;

/// Inter 4.1 Regular, the face of the panels (architecture 11.5; `ATTRIBUTION.md`).
static INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");

/// The settings of the user, on disk: those the window keeps, and the projects opened last.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Prefs {
    window: labcraft_ide::Prefs,
    /// The folders of the projects opened last, the newest first.
    recent: Vec<String>,
    /// The preferences of the viewer of M0v, read once into `window`.
    #[serde(skip_serializing_if = "Option::is_none")]
    theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    standard_everywhere: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reduced_motion: Option<bool>,
}

impl Settings for Prefs {
    const APP_ID: &'static str = "labcraft";
    const SCHEMA_VERSION: u32 = 1;
}

impl Prefs {
    /// The preferences of the viewer of M0v go into those of the window.
    fn take_those_of_the_viewer(&mut self) -> bool {
        let mut taken = false;
        if let Some(theme) = self.theme.take() {
            self.window.dark = theme == "dark";
            taken = true;
        }
        if let Some(on) = self.standard_everywhere.take() {
            self.window.standard_everywhere = on;
            taken = true;
        }
        if let Some(on) = self.reduced_motion.take() {
            self.window.reduced_motion = on;
            taken = true;
        }
        taken
    }
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
    let frame = frame(craft_window::Choice::probe(craft_window::Prefix::new("LABCRAFT")));
    let viewport = egui::ViewportBuilder::default().with_title("LabCraft").with_inner_size([1400.0, 860.0]).with_min_inner_size([640.0, 400.0]);
    // On macOS the system keeps its buttons and frame, and the content reaches under a see-through title bar, so the
    // top bar is the title bar there too (as craft-anything does).
    let viewport = if cfg!(target_os = "macos") && frame.decorations() {
        viewport.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false)
    } else {
        viewport
    };
    let native = eframe::NativeOptions { viewport: craft_window::decorate(viewport, &frame), ..eframe::NativeOptions::default() };
    let started = eframe::run_native("labcraft", native, Box::new(move |creation| Ok(Box::new(App::new(&creation.egui_ctx, options, frame)))));
    match started {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "labcraft: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Who draws the frame (ide.md 2, "the top bar is the title bar"). On Windows the app draws it, as VS Code does, so
/// the top bar is the only bar; elsewhere `craft-window` decides. `LABCRAFT_TITLEBAR=system` (or `app`) overrides both.
fn frame(choice: craft_window::Choice) -> craft_window::Frame {
    if cfg!(windows) && choice.overridden().is_none() {
        return craft_window::Frame::App(craft_window::Layout::default());
    }
    choice.into_frame()
}

/// What a dialog was asked for.
enum Ask {
    Open,
    New,
    Example(usize),
    Trace,
}

/// A project being opened: its session started, `view.open` not answered yet.
struct Pending {
    folder: PathBuf,
    handle: Handle,
    reply: Reply,
}

struct App {
    ide: Ide,
    options: Options,
    /// The session of the project the window shows.
    project: Option<Handle>,
    opening: Option<Pending>,
    prefs: SettingsStore<Prefs>,
    dialogs: Dialogs<Ask>,
    start: StartList,
    /// Whether each recent project is still there, looked at off the thread of the window.
    found: Option<Receiver<Vec<bool>>>,
    /// `LABCRAFT_FRAMES`: the time of each frame, and the motion it is measured over.
    timing: Option<Timing>,
}

impl App {
    fn new(ctx: &egui::Context, options: Options, frame: craft_window::Frame) -> App {
        install_fonts(ctx);
        let (mut prefs, _) = SettingsStore::<Prefs>::open(Location::Auto);
        if prefs.get().clone().take_those_of_the_viewer() {
            prefs.update(|prefs| {
                prefs.take_those_of_the_viewer();
            });
        }
        let mut window = prefs.get().window.clone();
        // `--theme` is for this window: not kept.
        if let Some(theme) = options.theme {
            window.dark = theme == labcraft_diagram::Theme::Dark;
        }
        let wake = ctx.clone();
        let dialogs = Dialogs::new(Arc::new(move || wake.request_repaint()));
        let examples = examples::BUNDLED.iter().map(|bundled| labcraft_ide::catalog::example(bundled.name)).collect();
        let mut app = App {
            ide: Ide::new(ctx, window, frame),
            options,
            project: None,
            opening: None,
            prefs,
            dialogs,
            start: StartList { recent: Vec::new(), examples, problem: None, in_memory: false },
            found: None,
            timing: std::env::var_os(frames::VARIABLE).is_some().then(Timing::new),
        };
        app.recent_changed(ctx);
        if let Some(folder) = app.options.folder.clone() {
            app.open(ctx, &folder, false);
        }
        app
    }

    /// Opens a project: in this window when it shows the start page, in a window of its own otherwise (ide.md 9).
    fn open(&mut self, ctx: &egui::Context, folder: &Path, new: bool) {
        if self.project.is_some() || self.opening.is_some() {
            if new && let Err(error) = session::write_first_function(folder, self.options.lib_paths.clone()) {
                self.ide.say(error.message);
                return;
            }
            self.another_window(folder);
            return;
        }
        let opening = Opening {
            folder: folder.to_path_buf(),
            lib_paths: self.options.lib_paths.clone(),
            standard_everywhere: self.prefs.get().window.standard_everywhere,
            style: self.options.style.clone(),
            new,
        };
        let wake = ctx.clone();
        match Handle::spawn(opening, Box::new(move || wake.request_repaint())) {
            Ok((handle, reply)) => {
                self.start.problem = None;
                self.opening = Some(Pending { folder: folder.to_path_buf(), handle, reply });
            }
            Err(error) => self.start.problem = Some(format!("{}: {error}", folder.display())),
        }
    }

    /// The same program, on another folder: one project per window.
    fn another_window(&mut self, folder: &Path) {
        let mut command = match std::env::current_exe() {
            Ok(exe) => std::process::Command::new(exe),
            Err(error) => {
                self.ide.say(format!("a window of its own for {}: {error}", folder.display()));
                return;
            }
        };
        for lib_path in &self.options.lib_paths {
            command.arg("--lib-path").arg(lib_path);
        }
        if let Some(style) = &self.options.style {
            command.arg("--style").arg(style);
        }
        command.arg("--").arg(folder);
        if let Err(error) = command.spawn() {
            self.ide.say(format!("a window of its own for {}: {error}", folder.display()));
        }
    }

    /// The project being opened has answered.
    fn opened(&mut self, ctx: &egui::Context) {
        let Some(result) = self.opening.as_ref().and_then(|pending| pending.reply.take()) else { return };
        let Some(pending) = self.opening.take() else { return };
        match result {
            Ok(_) => {
                let folder = pending.folder.display().to_string();
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                    "{} — LabCraft",
                    pending.folder.file_name().map_or(folder.clone(), |name| name.to_string_lossy().into_owned())
                )));
                self.project = Some(pending.handle);
                self.prefs.update(|prefs| {
                    prefs.recent.retain(|recent| *recent != folder);
                    prefs.recent.insert(0, folder);
                    prefs.recent.truncate(MAX_RECENT);
                });
                self.recent_changed(ctx);
            }
            Err(error) => self.start.problem = Some(format!("{}: {}", pending.folder.display(), error.message)),
        }
    }

    /// The recent projects again, and a look, off the thread of the window, at which are still there.
    fn recent_changed(&mut self, ctx: &egui::Context) {
        let recent = self.prefs.get().recent.clone();
        self.start.recent = recent
            .iter()
            .map(|path| Recent {
                name: Path::new(path).file_name().map_or_else(|| path.clone(), |name| name.to_string_lossy().into_owned()),
                path: path.clone(),
                found: None,
            })
            .collect();
        let (sender, receiver) = std::sync::mpsc::channel();
        let wake = ctx.clone();
        let spawned = std::thread::Builder::new().name("labcraft-recent".to_owned()).spawn(move || {
            let _ = sender.send(recent.iter().map(|path| Path::new(path).is_dir()).collect());
            wake.request_repaint();
        });
        self.found = spawned.ok().map(|_| receiver);
    }

    fn outward(&mut self, ctx: &egui::Context, asked: Outward) {
        match asked {
            // A folder given (the form of the palette) is taken as the dialog would give it.
            Outward::NewProject(Some(folder)) => self.answered(ctx, Ask::New, Answer::Paths(vec![PathBuf::from(folder)])),
            Outward::OpenFolder(Some(folder)) => self.answered(ctx, Ask::Open, Answer::Paths(vec![PathBuf::from(folder)])),
            Outward::NewProject(None) => self.dialogs.ask(FileDialog::folder().title("Choose an empty folder for the new project"), Ask::New),
            Outward::OpenFolder(None) => self.dialogs.ask(FileDialog::folder().title("Open a folder of LabCraft files"), Ask::Open),
            Outward::OpenRecent(at) => {
                if let Some(recent) = self.start.recent.get(at).map(|recent| PathBuf::from(&recent.path)) {
                    self.open(ctx, &recent, false);
                }
            }
            Outward::OpenExample(at) => {
                self.dialogs.ask(FileDialog::folder().title("Choose an empty folder for the copy of the example"), Ask::Example(at))
            }
            Outward::OpenLink(link) => ctx.open_url(egui::OpenUrl::new_tab(link)),
            Outward::OpenTrace => self.dialogs.ask(FileDialog::open().title("Open a trace").filter("Trace", &["jsonl"]), Ask::Trace),
        }
    }

    fn answered(&mut self, ctx: &egui::Context, ask: Ask, answer: Answer) {
        let path = match answer {
            Answer::Paths(paths) => match paths.into_iter().next() {
                Some(path) => path,
                None => return,
            },
            Answer::Unavailable(why) => {
                self.ide.say(why);
                return;
            }
            _ => return,
        };
        match ask {
            Ask::Open => self.open(ctx, &path, false),
            Ask::New => match session::make_new_folder(&path) {
                Ok(()) => self.open(ctx, &path, true),
                Err(error) => self.say_on_start(error.message),
            },
            Ask::Example(at) => match examples::copy(at, &path) {
                Ok(()) => self.open(ctx, &path, false),
                Err(error) => self.say_on_start(error.message),
            },
            Ask::Trace => {
                if let Some(project) = &self.project {
                    project.open_trace(&path);
                }
            }
        }
    }

    /// The settings of the window, kept; the theme of `--theme` is for this window only (cli.md 8): the one saved stays,
    /// unless the user changed the theme here.
    fn save_window(&mut self) {
        let mut window = self.ide.prefs();
        let saved = self.prefs.get().window.dark;
        if let Some(theme) = self.options.theme
            && window.dark == (theme == labcraft_diagram::Theme::Dark)
        {
            window.dark = saved;
        }
        self.prefs.update(|prefs| prefs.window = window);
    }

    fn say_on_start(&mut self, problem: String) {
        if self.project.is_some() {
            self.ide.say(problem);
        } else {
            self.start.problem = Some(problem);
        }
    }
}

impl eframe::App for App {
    #[expect(clippy::disallowed_methods, reason = "the desktop app, never built for WASM, where the clock traps")]
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|input| input.time);
        if let Some(timing) = &mut self.timing {
            timing.frames.tick(now, frame.info().cpu_usage);
            ctx.request_repaint();
            // What the time is measured on.
            if !std::mem::replace(&mut timing.adapter_said, true) {
                let adapter = frame.wgpu_render_state().map(|state| state.adapter.get_info());
                let said = adapter.map_or("no wgpu adapter".to_owned(), |info| format!("{} ({:?})", info.name, info.backend));
                let _ = writeln!(std::io::stderr(), "labcraft: drawn by {said}");
            }
        }
        self.opened(&ctx);
        if let Some(found) = self.found.as_ref().and_then(|found| found.try_recv().ok()) {
            self.found = None;
            for (recent, found) in self.start.recent.iter_mut().zip(found) {
                recent.found = Some(found);
            }
        }
        for (ask, answer) in self.dialogs.take_answers() {
            self.answered(&ctx, ask, answer);
        }
        let shown = self.project.as_ref().and_then(|project| project.snapshot().view.function.clone());
        if let (Some(timing), Some(function)) = (&mut self.timing, &shown)
            && let Some((canvas, rect)) = self.ide.shown_canvas(function)
        {
            timing.canvas = rect;
            timing.drive(canvas, now);
        }
        let host = self.project.as_ref().map(|project| project as &dyn Host);
        let outward = self.ide.show(ui, host, &self.start);
        if let (Some(timing), Some(rect)) = (&self.timing, self.timing.as_ref().and_then(|timing| timing.canvas)) {
            timing.corner(&ctx, rect, now);
        }
        for asked in outward {
            self.outward(&ctx, asked);
        }
        if self.ide.take_prefs_changed() {
            self.save_window();
        }
        self.prefs.tick(Instant::now());
        self.dialogs.show_pending(Some(&*frame));
    }

    fn on_exit(&mut self) {
        if let Some(timing) = &self.timing {
            let phase = match timing.motion {
                Motion::Waiting(_) => "before the motion, the first drawing counted",
                Motion::Running(_) => "the motion, cut by the window closed",
                Motion::Done => "by hand",
            };
            let _ = writeln!(std::io::stderr(), "labcraft: {phase}: {}", timing.frames.phase().line());
        }
        self.save_window();
        let _ = self.prefs.flush();
    }
}

/// Where the motion of `LABCRAFT_FRAMES` is.
#[derive(Clone, Copy, PartialEq)]
enum Motion {
    /// No drawing yet, or one too recent: when it is.
    Waiting(Option<f64>),
    /// Since when.
    Running(f64),
    /// The window moves by hand.
    Done,
}

/// The time of the frames, and the motion it is measured over (`labcraft::frames`).
struct Timing {
    frames: Frames,
    motion: Motion,
    /// The canvas, at the frame before.
    canvas: Option<egui::Rect>,
    /// The centres of the nodes of a drawing, which the motion follows.
    centres: Option<(Arc<labcraft_diagram::Drawing>, Vec<egui::Vec2>)>,
    /// The move of the motion under way.
    current: Option<(usize, &'static str)>,
    /// The adapter of wgpu has been said on stderr.
    adapter_said: bool,
}

impl Timing {
    fn new() -> Timing {
        let _ = writeln!(
            std::io::stderr(),
            "labcraft: {} is set: the window paints without pause and times each frame; a motion of {} s starts {} s after \
             the first drawing, then move it by hand",
            frames::VARIABLE,
            frames::motion_seconds(),
            frames::MOTION_DELAY
        );
        Timing { frames: Frames::new(), motion: Motion::Waiting(None), canvas: None, centres: None, current: None, adapter_said: false }
    }

    /// Moves the viewport of `canvas` as the motion says at `now`, and says on stderr how the motion went once it ends.
    fn drive(&mut self, canvas: &mut Canvas, now: f64) {
        let (Some(drawing), Some(rect)) = (canvas.drawing().cloned(), self.canvas) else { return };
        let start = match self.motion {
            Motion::Waiting(None) => {
                self.motion = Motion::Waiting(Some(now));
                return;
            }
            Motion::Waiting(Some(seen)) if now - seen < frames::MOTION_DELAY => return,
            Motion::Waiting(Some(_)) => {
                // What came before, the first drawing and its fit, is not counted.
                self.frames.next_phase();
                self.motion = Motion::Running(now);
                now
            }
            Motion::Running(start) => start,
            Motion::Done => return,
        };
        let size = egui::Vec2::new(drawing.width as f32, drawing.height as f32) / 100.0;
        let coarse = style::scope(&drawing.style, || style::factor(style::coarse_zoom()));
        let centres = match &self.centres {
            Some((of, centres)) if Arc::ptr_eq(of, &drawing) => centres,
            _ => &self.centres.insert((drawing.clone(), frames::centres(&drawing))).1,
        };
        // A line for each move, once it is over.
        let current = frames::move_at(now - start);
        if current.map(|(at, _)| at) != self.current.map(|(at, _)| at) {
            let part = self.frames.next_part();
            if let Some((_, what)) = self.current {
                let _ = writeln!(std::io::stderr(), "labcraft:   {what}: {}", part.line());
            }
            self.current = current;
        }
        match frames::viewport_at(now - start, size, centres, rect.size(), coarse) {
            Some((zoom, origin)) => canvas.move_viewport(zoom, origin),
            None => {
                self.motion = Motion::Done;
                let stats = self.frames.next_phase();
                let _ = writeln!(std::io::stderr(), "labcraft: the motion, {} nodes: {}", drawing.placed.len(), stats.line());
            }
        }
    }

    /// The last 2 seconds, in the top-right corner of the canvas.
    fn corner(&self, ctx: &egui::Context, rect: egui::Rect, now: f64) {
        let stats = self.frames.recent();
        let phase = match self.motion {
            Motion::Waiting(_) => "before the motion".to_owned(),
            Motion::Running(start) => format!("motion, {:.0} of {} s", now - start, frames::motion_seconds()),
            Motion::Done => "by hand".to_owned(),
        };
        let spread = |spread: &frames::Spread| format!("p50 {:5.1}  p95 {:5.1}  max {:5.1} ms", spread.p50, spread.p95, spread.max);
        let text = format!(
            "{:.1} frames/s, {}  ({phase})\nframe {}\ncpu   {}\nlate  {} of {}",
            stats.per_second(),
            if stats.holds_60() { "holds 60" } else { "below 60" },
            spread(&stats.interval),
            stats.cpu.as_ref().map_or("not given".to_owned(), spread),
            stats.late,
            stats.frames
        );
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("labcraft-frames")));
        let visuals = ctx.global_style().visuals.clone();
        let galley = painter.layout_no_wrap(text, egui::FontId::monospace(12.0), visuals.strong_text_color());
        let at = rect.right_top() + egui::Vec2::new(-galley.size().x - 14.0, 10.0);
        painter.rect_filled(egui::Rect::from_min_size(at, galley.size()).expand(6.0), 4.0, visuals.extreme_bg_color);
        painter.galley(at, galley, visuals.strong_text_color());
    }
}

/// The fonts of the window (architecture 11.5): Inter for the panels, Hack for the notation and the text, then the fonts
/// of the system for what they lack. egui is built without its default fonts.
fn install_fonts(ctx: &egui::Context) {
    let notation = FontFamily::Name(labcraft_editor::FAMILY.into());
    UiFonts::new(&I18nScripts)
        .app_face("Inter", FontData::from_static(INTER), &[FontFamily::Proportional])
        .app_face("Hack", FontData::from_static(labcraft_editor::HACK), &[FontFamily::Monospace, notation])
        .family(Family::named(labcraft_editor::FAMILY).monospaced())
        .system_fonts(SystemFonts::AfterCraft)
        .install(ctx);
}
