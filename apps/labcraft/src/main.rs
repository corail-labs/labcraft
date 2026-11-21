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
use labcraft::agents::{Agents, Channel};
use labcraft::args::{self, Asked, Options};
use labcraft::examples;
use labcraft::frames::{self, Frames};
use labcraft::session::{self, Handle, Opening};
use labcraft_diagram::style;
use labcraft_editor::Canvas;
use labcraft_engine::settings as keyed;
use labcraft_ide::{Host, Ide, Outward, Reply, StartList};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Exit code of wrong usage, as for `labcraft-cli` (`docs/cli.md`, section 1).
const WRONG_USAGE: u8 = 2;

/// At most this many recent projects on the start page *(proposed)* (ide.md 10).
const MAX_RECENT: usize = 12;

/// Inter 4.1 Regular, the face of the panels (architecture 11.5; `ATTRIBUTION.md`).
static INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");

/// The settings of the user, on disk: what the window keeps, and the projects opened last. The keys of commands.md 9.7
/// are in `settings.toml` beside it (`labcraft_cli::user_settings`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Prefs {
    window: labcraft_ide::Prefs,
    /// The folders of the projects opened last, the newest first.
    recent: Vec<String>,
    /// The preferences of the viewer of M0v, read once into the keys. craft-settings keeps in the file the keys that the
    /// app no longer writes, so they stay there: `viewer_taken` says they were read, and they are never read again.
    #[serde(skip_serializing_if = "Option::is_none")]
    theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    standard_everywhere: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reduced_motion: Option<bool>,
    viewer_taken: bool,
}

impl Settings for Prefs {
    const APP_ID: &'static str = "labcraft";
    const SCHEMA_VERSION: u32 = 1;
}

impl Prefs {
    /// The preferences of the viewer of M0v go into `keys`, once; whether there is anything to write. When the window
    /// kept settings already, as keys or as the fields of an earlier LabCraft, they are the newer ones, and win.
    fn take_those_of_the_viewer(&mut self, keys: &mut Map<String, Value>) -> bool {
        if self.viewer_taken {
            return false;
        }
        self.viewer_taken = true;
        if !keys.is_empty() || self.window.fields != labcraft_ide::Fields::default() {
            return true;
        }
        if let Some(theme) = self.theme.take() {
            keys.insert(keyed::THEME.to_owned(), json!(if theme == "dark" { "dark" } else { "light" }));
        }
        if let Some(on) = self.standard_everywhere.take() {
            keys.insert(keyed::STANDARD_EVERYWHERE.to_owned(), json!(on));
        }
        if let Some(on) = self.reduced_motion.take() {
            keys.insert(keyed::REDUCED_MOTION.to_owned(), json!(on));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{Prefs, keyed};
    use serde_json::{Map, json};

    /// craft-settings keeps the keys of the viewer of M0v in the file: read once, then never again, and never over the
    /// settings that the window kept.
    #[test]
    fn the_settings_of_the_viewer_are_read_once_and_never_over_those_of_the_window() {
        let mut first = Prefs { theme: Some("dark".to_owned()), ..Prefs::default() };
        let mut keys = Map::new();
        assert!(first.take_those_of_the_viewer(&mut keys));
        assert_eq!(keys.get(keyed::THEME), Some(&json!("dark")));
        assert!(first.viewer_taken);
        // The user turns the dark theme off; the file still has `theme = "dark"`.
        keys.remove(keyed::THEME);
        first.theme = Some("dark".to_owned());
        assert!(!first.take_those_of_the_viewer(&mut keys));
        assert!(keys.is_empty());
        // A file of an earlier LabCraft: the window kept its settings as fields, and the key of the viewer stayed.
        let mut kept = Prefs { theme: Some("dark".to_owned()), ..Prefs::default() };
        kept.window.fields.preview_tabs = Some(false);
        assert!(kept.take_those_of_the_viewer(&mut keys));
        assert!(keys.is_empty() && kept.viewer_taken);
    }
}

/// The value of `appearance.theme` for a theme.
fn theme_name(theme: labcraft_diagram::Theme) -> &'static str {
    if theme == labcraft_diagram::Theme::Dark { "dark" } else { "light" }
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
    /// A trace for the window that shows this function (the window that had focus), or for the window of the project.
    Trace(Option<String>),
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
    /// The agents attached to the project, and the channel they come by (ide.md 8): before `project`, so that it is
    /// dropped first.
    channel: Option<Channel>,
    /// The channel being opened, off the thread of the window: what it gives once open.
    channel_opening: Option<Receiver<Result<Option<Channel>, String>>>,
    /// The window shows a question about an agent, whose seconds go down.
    agent_asking: bool,
    /// The session of the project the window shows.
    project: Option<Handle>,
    opening: Option<Pending>,
    prefs: SettingsStore<Prefs>,
    /// The keys of the user as `settings.toml` held them when read, then as this window last wrote them: a write
    /// changes only those that differ, and keeps those another process wrote.
    kept: Map<String, Value>,
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
        let (mut keys, problem) = labcraft_cli::user_settings::read();
        let mut taken = prefs.get().clone();
        let viewer = taken.take_those_of_the_viewer(&mut keys);
        if viewer {
            prefs.set(taken);
        }
        let mut window = labcraft_ide::Prefs { settings: keys.clone(), ..prefs.get().window.clone() };
        // The fields of an earlier LabCraft, once: written with the keys when the window is saved.
        let fields = window.take_fields();
        let kept = keys;
        // `--theme` is for this window: not kept.
        if let Some(theme) = options.theme {
            window.settings.insert(keyed::THEME.to_owned(), json!(theme_name(theme)));
        }
        let wake = ctx.clone();
        let dialogs = Dialogs::new(Arc::new(move || wake.request_repaint()));
        let examples = examples::BUNDLED.iter().map(|bundled| labcraft_ide::catalog::example(bundled.name)).collect();
        let mut app = App {
            ide: Ide::new(ctx, window, frame),
            options,
            channel: None,
            channel_opening: None,
            agent_asking: false,
            project: None,
            opening: None,
            prefs,
            kept,
            dialogs,
            start: StartList { recent: Vec::new(), examples, problem: None, in_memory: false },
            found: None,
            timing: std::env::var_os(frames::VARIABLE).is_some().then(Timing::new),
        };
        if let Some(problem) = problem {
            app.say_on_start(problem);
        }
        if fields || viewer {
            app.save_window();
        }
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
            user_settings: self.ide.prefs().settings,
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
                self.open_channel(ctx, &pending.folder, &pending.handle);
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

    /// Opens the channel of the agents of the project at `folder` (ide.md 8), off the thread of the window: it waits for
    /// the session, and on Windows for `icacls`. What fails is said, and the window works without it.
    fn open_channel(&mut self, ctx: &egui::Context, folder: &Path, handle: &Handle) {
        let root = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
        let sender = handle.sender();
        let wake = ctx.clone();
        let (opened, receiver) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("labcraft-channel".to_owned()).spawn(move || {
            let repaint = wake.clone();
            let agents = Arc::new(Agents::new(&root, Box::new(move || repaint.request_repaint())));
            let channel = match session::served(&sender) {
                Some(served) => Channel::open(&root, sender, served, agents),
                None => Err("the session did not answer".to_owned()),
            };
            let _ = opened.send(channel);
            wake.request_repaint();
        });
        match spawned {
            Ok(_) => self.channel_opening = Some(receiver),
            Err(error) => self.ide.say(format!("No AI agent can attach to this window: {error}")),
        }
    }

    /// The channel once open; the questions about the agents to the window, when they changed or one is shown, and its
    /// answers to them.
    fn ask_about_agents(&mut self) {
        if let Some(opened) = self.channel_opening.as_ref().and_then(|receiver| receiver.try_recv().ok()) {
            self.channel_opening = None;
            match opened {
                Ok(channel) => {
                    self.channel = channel;
                    self.keep_policies();
                }
                Err(why) => self.ide.say(format!("No AI agent can attach to this window: {why}")),
            }
        }
        let Some(channel) = &self.channel else { return };
        let agents = channel.agents();
        for (id, answer) in self.ide.take_agent_answers() {
            agents.answer(id, answer);
        }
        if let Some(questions) = agents.questions(self.agent_asking) {
            self.agent_asking = !questions.is_empty();
            self.ide.set_agent_questions(questions);
        }
    }

    /// The policies the user kept (`ai.policies`), to the agents.
    fn keep_policies(&self) {
        if let Some(channel) = &self.channel {
            let policies = self.ide.prefs().settings.get(keyed::AI_POLICIES).cloned().unwrap_or_else(|| json!({}));
            channel.agents().keep(policies);
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
            Outward::OpenTrace(function) => {
                self.dialogs.ask(FileDialog::open().title("Open a trace").filter("Trace", &["jsonl"]), Ask::Trace(function))
            }
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
            Ask::Trace(function) => {
                if let Some(project) = &self.project {
                    project.open_trace(function, &path);
                }
            }
        }
    }

    /// The settings of the window, kept; the theme of `--theme` is for this window only (cli.md 8): the one saved stays,
    /// unless the user changed the theme here.
    fn save_window(&mut self) {
        let mut window = self.ide.prefs();
        let mut keys = std::mem::take(&mut window.settings);
        if let Some(theme) = self.options.theme
            && keys.get(keyed::THEME) == Some(&json!(theme_name(theme)))
        {
            match self.kept.get(keyed::THEME) {
                Some(saved) => keys.insert(keyed::THEME.to_owned(), saved.clone()),
                None => keys.remove(keyed::THEME),
            };
        }
        if self.kept != keys {
            match labcraft_cli::user_settings::write(&self.kept, &keys) {
                Ok(()) => self.kept = keys,
                Err(error) => {
                    self.say_on_start(error.message);
                    // The fields of an earlier LabCraft stay in its file until they are in `settings.toml`.
                    if !self.prefs.get().window.fields_taken {
                        return;
                    }
                }
            }
        }
        self.prefs.update(|prefs| {
            prefs.window = window;
            prefs.window.fields_taken = true;
        });
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
        let shown = self.project.as_ref().and_then(|project| project.snapshot().view.main.function.clone());
        if let (Some(timing), Some(function)) = (&mut self.timing, &shown)
            && let Some((canvas, rect)) = self.ide.shown_canvas(function)
        {
            timing.canvas = rect;
            // The drop of the scripted drag, a move asked as the canvas asks it.
            if let (Some(labcraft_editor::Intent::Move { nodes, by }), Some(project)) = (timing.drive(canvas, now), &self.project) {
                self.ide.ask_canvas_move(project, function, &nodes, by);
            }
        }
        self.ask_about_agents();
        let host = self.project.as_ref().map(|project| project as &dyn Host);
        let outward = self.ide.show(ui, host, &self.start);
        // An answer given in this frame reaches the agent now.
        self.ask_about_agents();
        if let (Some(timing), Some(rect)) = (&self.timing, self.timing.as_ref().and_then(|timing| timing.canvas)) {
            timing.corner(&ctx, rect, now);
        }
        for asked in outward {
            self.outward(&ctx, asked);
        }
        if self.ide.take_prefs_changed() {
            self.save_window();
            self.keep_policies();
        }
        self.prefs.tick(Instant::now());
        self.dialogs.show_pending(Some(&*frame));
    }

    fn on_exit(&mut self) {
        if let Some(timing) = &self.timing {
            let phase = match timing.motion {
                Motion::Waiting(_) => "before the motion, the first drawing counted",
                Motion::Running(_) => "the motion, cut by the window closed",
                Motion::Dragging(_) | Motion::Dropped(..) => "the drag, cut by the window closed",
                Motion::Done => "by hand",
            };
            let _ = writeln!(std::io::stderr(), "labcraft: {phase}: {}", timing.frames.phase().line());
        }
        self.save_window();
        let _ = self.prefs.flush();
        // The agents go first: their file goes, and nothing more reaches the session.
        self.channel = None;
    }
}

/// Where the motion of `LABCRAFT_FRAMES` is.
#[derive(Clone, Copy, PartialEq)]
enum Motion {
    /// No drawing yet, or one too recent: when it is.
    Waiting(Option<f64>),
    /// Since when.
    Running(f64),
    /// The scripted drag of 100 nodes, since when (scenario 6 of M1).
    Dragging(f64),
    /// The drop of the drag, when, and the drawing before it, by its address: the new one is another.
    Dropped(f64, usize),
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
    /// The nodes of the scripted drag, and the time of the preview of each of its frames, in milliseconds.
    dragged: Vec<usize>,
    previews: Vec<f64>,
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
        Timing {
            frames: Frames::new(),
            motion: Motion::Waiting(None),
            canvas: None,
            centres: None,
            current: None,
            adapter_said: false,
            dragged: Vec::new(),
            previews: Vec::new(),
        }
    }

    /// Moves the viewport of `canvas` as the motion says at `now`, and says on stderr how the motion went once it ends;
    /// then drags 100 nodes, and gives the move to ask for at the drop (scenario 6 of M1).
    fn drive(&mut self, canvas: &mut Canvas, now: f64) -> Option<labcraft_editor::Intent> {
        let (Some(drawing), Some(rect)) = (canvas.drawing().cloned(), self.canvas) else { return None };
        let start = match self.motion {
            Motion::Waiting(None) => {
                self.motion = Motion::Waiting(Some(now));
                return None;
            }
            Motion::Waiting(Some(seen)) if now - seen < frames::MOTION_DELAY => return None,
            Motion::Waiting(Some(_)) => {
                // What came before, the first drawing and its fit, is not counted.
                self.frames.next_phase();
                self.motion = Motion::Running(now);
                now
            }
            Motion::Running(start) => start,
            Motion::Dragging(start) => return self.drag(canvas, &drawing, now, start),
            Motion::Dropped(at, before) => {
                // The new drawing, off the thread of the window: no frame waits for it (architecture 12.4).
                let came = Arc::as_ptr(&drawing) as usize != before;
                if came || now - at > 10.0 {
                    let said = if came { format!("came in {:.0} ms", (now - at) * 1000.0) } else { "did not come in 10 s".to_owned() };
                    let stats = self.frames.next_phase();
                    let _ = writeln!(
                        std::io::stderr(),
                        "labcraft: after the drop, the new drawing {said} (budget {} ms): {}",
                        frames::REDRAW_BUDGET_MS,
                        stats.line()
                    );
                    self.motion = Motion::Done;
                }
                return None;
            }
            Motion::Done => return None,
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
                let stats = self.frames.next_phase();
                let _ = writeln!(std::io::stderr(), "labcraft: the motion, {} nodes: {}", drawing.placed.len(), stats.line());
                // Then the drag, at the fit, where the nodes dragged show.
                canvas.fit();
                self.dragged = frames::dragged(&drawing);
                self.motion = Motion::Dragging(now);
            }
        }
        None
    }

    /// A frame of the scripted drag: the nodes held where the drag goes, the time of the preview of their wires; at its
    /// end, the drop, and the move it asks for.
    #[expect(clippy::disallowed_methods, reason = "the desktop app, never built for WASM, where the clock traps")]
    fn drag(&mut self, canvas: &mut Canvas, drawing: &labcraft_diagram::Drawing, now: f64, start: f64) -> Option<labcraft_editor::Intent> {
        match frames::drag_at(now - start) {
            Some(by) => {
                let began = Instant::now();
                let preview = labcraft_diagram::preview(drawing, &self.dragged, (by.0 * 100, by.1 * 100));
                self.previews.push(began.elapsed().as_secs_f64() * 1000.0);
                drop(preview);
                canvas.hold_move(&self.dragged, by);
                None
            }
            None => {
                let stats = self.frames.next_phase();
                let previews = frames::spread_of(std::mem::take(&mut self.previews));
                let _ = writeln!(
                    std::io::stderr(),
                    "labcraft: the drag of {} nodes: {}; the preview p50 {:.2} p95 {:.2} max {:.2} ms (budget {} ms)",
                    self.dragged.len(),
                    stats.line(),
                    previews.p50,
                    previews.p95,
                    previews.max,
                    frames::PREVIEW_BUDGET_MS
                );
                self.motion = Motion::Dropped(now, std::ptr::from_ref(drawing) as usize);
                canvas.drop_held()
            }
        }
    }

    /// The last 2 seconds, in the top-right corner of the canvas.
    fn corner(&self, ctx: &egui::Context, rect: egui::Rect, now: f64) {
        let stats = self.frames.recent();
        let phase = match self.motion {
            Motion::Waiting(_) => "before the motion".to_owned(),
            Motion::Running(start) => format!("motion, {:.0} of {} s", now - start, frames::motion_seconds()),
            Motion::Dragging(start) => format!("drag of {} nodes, {:.0} of {} s", self.dragged.len(), now - start, frames::DRAG_SECONDS),
            Motion::Dropped(..) => "the drop, the new drawing to come".to_owned(),
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
