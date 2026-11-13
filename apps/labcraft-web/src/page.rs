//! The page (`docs/architecture.md`, 11.5): the window of the desktop viewer in a canvas of the browser, eframe on
//! WebGPU, or WebGL2 where the browser lacks it (`?webgl` asks for it); the session in a worker, which the
//! [`Supervisor`] starts, feeds and restarts; the folders it bundles, or one dropped on the page or chosen, read once.
//! There is no trace in the browser in M0 (scenario 6, point 7).
//!
//! The address names the bundled folder and the function shown ([`link`](crate::link)), and the page writes it back as
//! the developer moves. `?smoke`: the page opens the examples, traps its worker once it drew, and says to the server
//! that served it whether the worker came back and drew again (`cargo xtask web-smoke`).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

use craft_ui::fonts::{Family, I18nScripts, UiFonts};
use egui::{FontData, FontFamily};
use js_sys::Uint8Array;
use labcraft_diagram::Drawing;
use labcraft_editor::{Canvas, Shown as Tabs, Theme};
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::{Closure, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    DragEvent, ErrorEvent, FileList, FileSystemDirectoryEntry, FileSystemDirectoryReader, FileSystemEntry, FileSystemFileEntry, HtmlCanvasElement,
    HtmlInputElement, MessageEvent, RequestInit, Worker,
};

use crate::BUNDLED;
use crate::folder::{self, Budget};
use crate::link::{self, Link};
use crate::protocol::{File, Folder, Listing};
use crate::supervisor::{Action, Shown, Supervisor};

/// Inter 4.1 Regular, the face of the panels (architecture 11.5; `ATTRIBUTION.md`).
static INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
/// Hack 3.003 Regular, the face of the notation and of `Problems`.
static HACK: &[u8] = include_bytes!("../../../assets/fonts/Hack-Regular.ttf");

/// The worker, as Trunk writes it next to the page: a script that loads its WASM.
const WORKER: &str = "./labcraft-worker_loader.js";

/// Where the page keeps the preferences of the viewer, in the storage of the browser.
const THEME_KEY: &str = "labcraft.viewer.theme";
const STANDARD_KEY: &str = "labcraft.viewer.standard";

/// Where the welcome line of the page leads.
const REPOSITORY: &str = "https://github.com/corail-labs/labcraft";

/// How long the smoke test waits for each step, in seconds.
const SMOKE_PATIENCE: f64 = 90.0;

/// Starts the page in its canvas `#labcraft`; what fails to start is said in `#loading`.
pub fn start() {
    spawn_local(async {
        let Some(document) = web_sys::window().and_then(|window| window.document()) else { return };
        let Some(canvas) = document.get_element_by_id("labcraft").and_then(|element| element.dyn_into::<HtmlCanvasElement>().ok()) else { return };
        let mut options = eframe::WebOptions::default();
        if address().webgl {
            let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
            setup.instance_descriptor.backends = eframe::wgpu::Backends::GL;
            options.wgpu_options.wgpu_setup = eframe::egui_wgpu::WgpuSetup::CreateNew(setup);
        }
        let runner = eframe::WebRunner::new();
        let started = runner.start(canvas, options, Box::new(|creation| Ok(Box::new(Viewer::new(creation))))).await;
        let loading = document.get_element_by_id("loading");
        match (started, loading) {
            (Ok(()), Some(loading)) => loading.remove(),
            // `index.html` says it in `#loading`, which stops waiting.
            (Err(error), Some(loading)) => {
                let _ = loading.set_attribute("data-failed", "");
                loading.set_text_content(Some(&format!(
                    "LabCraft could not start: {}. It needs a browser with WebGPU or WebGL2. LabCraft on GitHub: {REPOSITORY}",
                    js(error)
                )));
            }
            _ => {}
        }
    });
}

/// What the address of the page asks for.
fn address() -> Link {
    Link::parse(&web_sys::window().and_then(|window| window.location().search().ok()).unwrap_or_default())
}

/// An error of the browser, as text.
fn js(error: JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
}

/// What comes to the page between two frames.
enum Incoming {
    /// From the worker of this number.
    Worker(u64, FromWorker),
    /// A folder read, and why its walk stopped, if it did.
    Folder(Folder, Option<String>),
    Failed(String),
}

enum FromWorker {
    Message(Vec<u8>),
    Trapped,
}

#[derive(Clone)]
struct Inbox {
    queue: Rc<RefCell<VecDeque<Incoming>>>,
    ctx: egui::Context,
}

impl Inbox {
    fn push(&self, incoming: Incoming) {
        self.queue.borrow_mut().push_back(incoming);
        self.ctx.request_repaint();
    }

    fn take(&self) -> VecDeque<Incoming> {
        std::mem::take(&mut *self.queue.borrow_mut())
    }

    fn unreadable(&self, path: &str, why: String) {
        self.push(Incoming::Failed(format!("`{path}` could not be read: {why}")));
    }
}

/// A worker that runs, with what listens to it; dropping it ends it.
struct Running {
    worker: Worker,
    _message: Closure<dyn FnMut(MessageEvent)>,
    _error: Closure<dyn FnMut(ErrorEvent)>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.worker.terminate();
    }
}

enum Smoke {
    /// Waits for the first drawing, from the start of the page (the time of egui counts from it).
    Drawing {
        since: f64,
    },
    /// Trapped the worker after the first drawing, `before`; waits for a new worker to draw anew.
    Trapped {
        since: f64,
        first: String,
        before: Option<Arc<Drawing>>,
    },
    Done,
}

struct Viewer {
    supervisor: Supervisor,
    inbox: Inbox,
    running: Option<Running>,
    /// The workers started, the first counting 1: what a worker ended says is not for the one that runs.
    started: u64,
    canvas: Canvas,
    /// The drawing the canvas paints.
    painted: Option<Arc<Drawing>>,
    theme: Theme,
    shown: Shown,
    /// The lines of `Problems`, from the listing they were written from.
    problems: (Arc<Listing>, Vec<String>),
    /// The preference last kept in the storage of the browser: the one that the session has.
    standard_stored: Option<bool>,
    /// A folder was opened: fit its first drawing.
    fit: bool,
    /// The dialog that chooses a folder is open, or the folder is being read.
    reading: bool,
    /// Why the walk of the last folder read stopped, or why it could not be read.
    folder_notice: Option<String>,
    /// The bundled folder open, which has an address; `None` for a folder of the developer's.
    bundled: Option<&'static str>,
    backend: String,
    smoke: Option<Smoke>,
}

impl Viewer {
    fn new(creation: &eframe::CreationContext<'_>) -> Viewer {
        let ctx = creation.egui_ctx.clone();
        install_fonts(&ctx);
        let theme = match stored(THEME_KEY).as_deref() {
            Some("dark") => Theme::Dark,
            Some("light") => Theme::Light,
            _ if prefers("(prefers-color-scheme: dark)") => Theme::Dark,
            _ => Theme::Light,
        };
        ctx.set_theme(if theme == Theme::Dark { egui::Theme::Dark } else { egui::Theme::Light });
        let mut canvas = Canvas::new();
        // The glide follows what the system asks for (architecture 11.4).
        canvas.set_reduced_motion(prefers("(prefers-reduced-motion: reduce)"));
        let backend = creation.wgpu_render_state.as_ref().map(|state| format!("{:?}", state.adapter.get_info().backend)).unwrap_or_default();
        let inbox = Inbox { queue: Rc::default(), ctx: ctx.clone() };
        listen_to_drops(&inbox);
        let mut viewer = Viewer {
            supervisor: Supervisor::new(stored(STANDARD_KEY).as_deref() == Some("true")),
            inbox,
            running: None,
            started: 0,
            canvas,
            painted: None,
            theme,
            shown: Shown::default(),
            problems: Default::default(),
            standard_stored: None,
            fit: false,
            reading: false,
            folder_notice: None,
            bundled: None,
            backend,
            smoke: None,
        };
        // The folder and the function of the address, else the examples on `check_voltage`.
        let link = address();
        viewer.smoke = link.smoke.then_some(Smoke::Drawing { since: 0.0 });
        let names: Vec<&'static str> = BUNDLED.iter().map(|bundled| bundled.name).collect();
        if let Some(opening) = link.resolve(&names)
            && let Some(bundled) = BUNDLED.iter().find(|bundled| bundled.name == opening.folder)
        {
            viewer.open(bundled.folder(), Some(bundled.name), opening.notice, opening.function);
        }
        viewer
    }

    /// A folder of the developer's (`bundled`: `None`) or a bundled one, on `function` when it has it.
    fn open(&mut self, folder: Folder, bundled: Option<&'static str>, notice: Option<String>, function: Option<String>) {
        self.bundled = bundled;
        self.folder_notice = notice;
        self.reading = false;
        self.fit = true;
        let actions = self.supervisor.open(folder, function);
        self.perform(actions);
    }

    fn command(&mut self, id: &str, params: serde_json::Value) {
        let actions = self.supervisor.command(id, params);
        self.perform(actions);
    }

    /// Does what the supervisor asks, then shows what it now shows.
    fn perform(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Start => {
                    self.running = None;
                    self.started += 1;
                    match spawn(self.started, &self.inbox) {
                        Ok(running) => self.running = Some(running),
                        Err(error) => {
                            let _ = self.supervisor.not_started(&error);
                        }
                    }
                }
                Action::Send(bytes) => {
                    if let Some(running) = &self.running {
                        let array = Uint8Array::from(bytes.as_slice());
                        if let Err(error) = running.worker.post_message_with_transfer(&array, &js_sys::Array::of1(&array.buffer())) {
                            self.folder_notice = Some(format!("the worker of the viewer could not be reached: {}", js(error)));
                        }
                    }
                }
            }
        }
        self.show();
    }

    /// What the supervisor shows, taken once after each change, not at each frame.
    fn show(&mut self) {
        self.shown = self.supervisor.shown();
        if let Some(drawing) = &self.shown.drawing
            && !self.painted.as_ref().is_some_and(|painted| Arc::ptr_eq(painted, drawing))
        {
            self.canvas.set_drawing(drawing.clone(), self.shown.state.glide);
            self.painted = Some(drawing.clone());
            if self.fit {
                self.canvas.fit();
                self.fit = false;
            }
        }
        if !Arc::ptr_eq(&self.problems.0, &self.shown.listing) {
            self.problems = (self.shown.listing.clone(), self.shown.listing.problems.lines().map(str::to_owned).collect());
        }
        // The preference is the session's: a command that trapped did not change it.
        let standard = self.shown.state.standard_everywhere;
        if self.standard_stored != Some(standard) {
            store(STANDARD_KEY, if standard { "true" } else { "false" });
            self.standard_stored = Some(standard);
        }
        if !self.shown.busy {
            self.write_address();
        }
    }

    /// The address and the title of the page, as what is on screen: a link to it, for a bundled folder.
    fn write_address(&self) {
        let Some(window) = web_sys::window() else { return };
        let function = self.shown.state.function.as_deref();
        let search = Link::search(self.bundled, function, address().webgl);
        if let (Ok(history), Ok(path)) = (window.history(), window.location().pathname()) {
            // `?smoke` keeps its address: its server reads nothing else, but the address is not the page's to change.
            if self.smoke.is_none() {
                let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&format!("{path}{search}")));
            }
        }
        if let Some(document) = window.document() {
            document.set_title(&function.map_or_else(|| "LabCraft viewer".to_owned(), |function| format!("{function} · LabCraft viewer")));
        }
    }

    /// What came since the last frame.
    fn take_incoming(&mut self) {
        for incoming in self.inbox.take() {
            match incoming {
                Incoming::Worker(worker, _) if worker != self.started => {}
                Incoming::Worker(_, FromWorker::Message(bytes)) => {
                    let actions = self.supervisor.received(&bytes);
                    self.perform(actions);
                }
                Incoming::Worker(_, FromWorker::Trapped) => {
                    self.running = None;
                    let actions = self.supervisor.trapped();
                    self.perform(actions);
                }
                Incoming::Folder(folder, notice) => self.open(folder, None, notice, None),
                Incoming::Failed(why) => {
                    self.reading = false;
                    self.folder_notice = Some(why);
                }
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        // What the page is, for whoever comes to it from a link.
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("LabCraft").strong());
            ui.label("A graphical dataflow language, in pure Rust. This viewer draws programs; the files you open stay in your browser.");
            ui.add(egui::Hyperlink::from_label_and_url("Source on GitHub", REPOSITORY).open_in_new_tab(true));
        });
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui.button("Open a folder…").on_hover_text("Or drop a folder on the page").clicked() {
                self.reading = true;
                pick_folder(&self.inbox);
            }
            for bundled in BUNDLED {
                let open = self.bundled == Some(bundled.name);
                if ui.selectable_label(open, bundled.name).on_hover_text("A folder bundled with the viewer").clicked() && !open {
                    self.open(bundled.folder(), Some(bundled.name), None, link::default_function(bundled.name));
                }
            }
            if let (None, Some(name)) = (self.bundled, self.supervisor.folder_name()) {
                ui.selectable_label(true, name).on_hover_text("The folder you opened");
            }
            ui.separator();
            let mut dark = self.theme == Theme::Dark;
            if ui.checkbox(&mut dark, "Dark").changed() {
                self.theme = if dark { Theme::Dark } else { Theme::Light };
                ui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                store(THEME_KEY, if dark { "dark" } else { "light" });
            }
            let mut standard = self.shown.state.standard_everywhere;
            if ui.checkbox(&mut standard, "Standard view everywhere").changed() {
                self.command("view.set_standard_view", json!({ "on": standard }));
            }
            if ui.button("Fit").clicked() {
                self.canvas.fit();
            }
            if self.shown.busy || self.reading {
                ui.spinner();
            }
        });
        // A tab per function opened.
        let state = self.shown.state.clone();
        let mut clicked = None;
        ui.horizontal_wrapped(|ui| {
            for name in &state.opened {
                let shown = state.function.as_ref() == Some(name);
                if ui.selectable_label(shown, name).clicked() && !shown {
                    clicked = Some(name.clone());
                }
            }
        });
        if let Some(function) = clicked {
            self.command("view.show_function", json!({ "function": function }));
        }
        for notice in self.folder_notice.iter().chain(&self.shown.notices) {
            ui.colored_label(ui.visuals().warn_fg_color, notice);
        }
    }

    /// The functions of the folder: only the rows in sight are laid out, however many there are.
    fn functions(&mut self, ui: &mut egui::Ui) {
        ui.heading("Functions");
        let (state, listing) = (self.shown.state.clone(), self.shown.listing.clone());
        let row = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y;
        let mut clicked = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row, listing.functions.len(), |ui, rows| {
            for listed in listing.functions.get(rows).unwrap_or_default() {
                let shown = state.function.as_ref() == Some(&listed.function);
                // A function that is not drawn gives the number of errors of its file (architecture 11.5).
                let text = if listed.drawable || listed.errors == 0 {
                    listed.function.clone()
                } else {
                    format!("{}  ({} error{})", listed.function, listed.errors, if listed.errors == 1 { "" } else { "s" })
                };
                if ui.selectable_label(shown, text).on_hover_text(&listed.file).clicked() && !shown {
                    clicked = Some(listed.function.clone());
                }
            }
        });
        if let Some(function) = clicked {
            self.command("view.show_function", json!({ "function": function }));
        }
    }

    /// The diagnostics: only the lines in sight are laid out.
    fn problems(&self, ui: &mut egui::Ui) {
        let listing = &self.shown.listing;
        ui.label(egui::RichText::new(format!("Problems: {} errors, {} warnings", listing.errors, listing.warnings)).strong());
        if self.shown.kept {
            ui.label("This function is not drawn now: its last drawing stays, with the diagnostics of its file.");
        }
        let lines = &self.problems.1;
        let row = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row, lines.len(), |ui, rows| {
            for line in lines.get(rows).unwrap_or_default() {
                ui.label(egui::RichText::new(line).monospace());
            }
        });
    }

    /// `?smoke`: one step each frame, until it reports.
    fn smoke(&mut self, now: f64, ctx: &egui::Context) {
        let Some(step) = self.smoke.take() else { return };
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        let drawn = !self.shown.busy && self.shown.drawing.is_some() && self.shown.notices.is_empty();
        let function = self.shown.state.function.clone().unwrap_or_default();
        self.smoke = Some(match step {
            Smoke::Drawing { .. } if drawn && self.started == 1 => {
                let groups = self.shown.drawing.as_ref().map_or(0, |drawing| drawing.groups.len());
                let before = self.shown.drawing.clone();
                let actions = self.supervisor.trap();
                self.perform(actions);
                Smoke::Trapped { since: now, first: format!("drew `{function}` ({groups} groups)"), before }
            }
            // The new worker drew anew: not the drawing that the page kept.
            Smoke::Trapped { first, before, .. }
                if drawn
                    && self.started == 2
                    && !before.as_ref().zip(self.shown.drawing.as_ref()).is_some_and(|(before, now)| Arc::ptr_eq(before, now)) =>
            {
                report(&format!("ok: {}; {first}; the worker trapped and restarted; drew `{function}` again", self.backend));
                Smoke::Done
            }
            Smoke::Drawing { since } | Smoke::Trapped { since, .. } if now - since > SMOKE_PATIENCE => {
                report(&format!("fail: {} workers started, function {function:?}, notices {:?}", self.started, self.shown.notices));
                Smoke::Done
            }
            other => other,
        });
    }
}

impl eframe::App for Viewer {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.take_incoming();
        egui::Panel::top("labcraft-top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("labcraft-problems").resizable(true).default_size(120.0).show(ui, |ui| self.problems(ui));
        egui::Panel::left("labcraft-functions").resizable(true).default_size(240.0).show(ui, |ui| self.functions(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            if self.shown.drawing.is_none() {
                let text = match (self.supervisor.folder_name(), &self.shown.state.function) {
                    _ if self.reading => "Reading the folder…".to_owned(),
                    (Some(name), _) if self.shown.busy => format!("Opening `{name}`…"),
                    // Not drawn: the diagnostics of its file, or the notice of a trap, say why.
                    (Some(_), Some(function)) => format!("`{function}` is not drawn: the notices and Problems say why."),
                    (Some(_), None) if !self.shown.notices.is_empty() => "The notices above say why nothing is drawn.".to_owned(),
                    (Some(name), None) if self.shown.listing.functions.is_empty() => {
                        format!("`{name}` has no function to draw. Open another folder, or drop one on the page.")
                    }
                    (Some(_), None) => "Choose a function on the left.".to_owned(),
                    (None, _) => "Open a folder, or drop one on the page, to see its functions.".to_owned(),
                };
                let working = self.reading || self.shown.busy;
                ui.vertical_centered(|ui| {
                    ui.add_space((ui.available_height() / 2.0 - 24.0).max(0.0));
                    if working {
                        ui.spinner();
                    }
                    ui.label(text);
                });
                return;
            }
            let tabs = Tabs { arms: self.shown.state.arms.iter().cloned().collect() };
            let response = self.canvas.show(ui, self.theme, &tabs);
            if let (Some((structure, arm)), Some(function)) = (response.tab, self.shown.state.function.clone()) {
                self.command("view.set_state", json!({ "function": function, "state": { "arms": { structure: arm } } }));
            }
        });
        let now = ui.input(|input| input.time);
        self.smoke(now, ui.ctx());
    }
}

/// Starts a worker whose messages and trap go to `inbox`, under the number `number`.
fn spawn(number: u64, inbox: &Inbox) -> Result<Running, String> {
    let worker = Worker::new(WORKER).map_err(js)?;
    let to = inbox.clone();
    let message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        to.push(Incoming::Worker(number, FromWorker::Message(Uint8Array::new(&event.data()).to_vec())));
    });
    let to = inbox.clone();
    let error = Closure::<dyn FnMut(ErrorEvent)>::new(move |event: ErrorEvent| {
        // The page answers for it: the browser need not print it again.
        event.prevent_default();
        to.push(Incoming::Worker(number, FromWorker::Trapped));
    });
    worker.set_onmessage(Some(message.as_ref().unchecked_ref()));
    worker.set_onerror(Some(error.as_ref().unchecked_ref()));
    Ok(Running { worker, _message: message, _error: error })
}

/// The fonts of the window (architecture 11.5): Inter for the panels, Hack for the notation and `Problems`. A character
/// that they lack shows in its cells as a missing glyph.
fn install_fonts(ctx: &egui::Context) {
    let notation = FontFamily::Name(labcraft_editor::FAMILY.into());
    UiFonts::new(&I18nScripts)
        .app_face("Inter", FontData::from_static(INTER), &[FontFamily::Proportional])
        .app_face("Hack", FontData::from_static(HACK), &[FontFamily::Monospace, notation])
        .family(Family::named(labcraft_editor::FAMILY).monospaced())
        .install(ctx);
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

fn stored(key: &str) -> Option<String> {
    storage()?.get_item(key).ok()?
}

fn store(key: &str, value: &str) {
    if let Some(storage) = storage() {
        let _ = storage.set_item(key, value);
    }
}

fn prefers(query: &str) -> bool {
    web_sys::window().and_then(|window| window.match_media(query).ok().flatten()).is_some_and(|list| list.matches())
}

/// Tells the server that served the page how the smoke test went.
fn report(text: &str) {
    let Some(window) = web_sys::window() else { return };
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_body(&JsValue::from_str(text));
    let _ = window.fetch_with_str_and_init("/smoke", &init);
}

/// A file found by a walk of the browser: from the dialog, already a file; from a drop, an entry to open.
enum Found {
    File(web_sys::File),
    Entry(FileSystemFileEntry),
}

impl Found {
    async fn file(self) -> Result<web_sys::File, String> {
        match self {
            Found::File(file) => Ok(file),
            Found::Entry(entry) => {
                let promise = js_sys::Promise::new(&mut |resolve: js_sys::Function, reject: js_sys::Function| {
                    entry.file_with_callback_and_callback(&resolve, &reject);
                });
                JsFuture::from(promise).await.map_err(js)?.dyn_into::<web_sys::File>().map_err(|_| "not a file".to_owned())
            }
        }
    }
}

/// A folder as the browser listed it: its name, its files, each with its path from it, its folders, and why the listing
/// stopped, if it did.
struct Listed {
    name: String,
    files: Vec<(String, Found)>,
    folders: Vec<String>,
    stopped: Option<String>,
}

/// Reads the files of a folder that [`folder::plan`] keeps, and opens it.
async fn read_folder(listed: Listed, inbox: Inbox) {
    let paths: Vec<String> = listed.files.iter().map(|(path, _)| path.clone()).collect();
    let plan = folder::plan(&paths, &listed.folders);
    let mut found: Vec<Option<(String, Found)>> = listed.files.into_iter().map(Some).collect();
    let mut budget = Budget::default();
    let mut files = Vec::new();
    for index in plan.read {
        let Some((path, found)) = found.get_mut(index).and_then(Option::take) else { continue };
        let file = match found.file().await {
            Ok(file) => file,
            Err(why) => return inbox.unreadable(&path, why),
        };
        let Some(most) = budget.take(file.size() as u64) else { break };
        match bytes_of(&file, most).await {
            Ok(bytes) => files.push(File { path, bytes }),
            Err(why) => return inbox.unreadable(&path, why),
        }
    }
    let notice = [listed.stopped, budget.notice()].into_iter().flatten().collect::<Vec<_>>().join("; ");
    let folder = Folder { name: listed.name, files, folders: plan.folders };
    inbox.push(Incoming::Folder(folder, (!notice.is_empty()).then_some(notice)));
}

/// The dialog of the browser that chooses a folder: its files come with their paths, the name of the folder first.
fn pick_folder(inbox: &Inbox) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else { return };
    let Some(input) = document.create_element("input").ok().and_then(|element| element.dyn_into::<HtmlInputElement>().ok()) else { return };
    input.set_type("file");
    input.set_multiple(true);
    let _ = input.set_attribute("webkitdirectory", "");
    let chosen = input.clone();
    let to = inbox.clone();
    let on_change = Closure::once_into_js(move || match chosen.files() {
        Some(files) => spawn_local(read_folder(chosen_files(&files), to)),
        None => to.push(Incoming::Failed("no folder was chosen".to_owned())),
    });
    input.set_onchange(Some(on_change.unchecked_ref()));
    input.click();
}

/// The files of a folder chosen, each with its path from the folder, and the name of the folder: the dialog lists
/// them all, and no empty folder.
fn chosen_files(files: &FileList) -> Listed {
    let mut name = None;
    let mut found = Vec::new();
    for file in (0..files.length()).filter_map(|index| files.item(index)) {
        // `webkitRelativePath`, which web-sys does not bind: the path from the folder chosen, its name first.
        let path = js_sys::Reflect::get(&file, &JsValue::from_str("webkitRelativePath"))
            .ok()
            .and_then(|path| path.as_string())
            .unwrap_or_else(|| file.name());
        let (root, under) = path.split_once('/').unwrap_or(("", path.as_str()));
        name.get_or_insert_with(|| root.to_owned());
        found.push((under.to_owned(), Found::File(file)));
    }
    Listed { name: name.unwrap_or_default(), files: found, folders: Vec::new(), stopped: None }
}

/// What is dropped on the page: a folder, read as a folder chosen is; files and folders together, as the files of a
/// folder named `dropped`.
fn listen_to_drops(inbox: &Inbox) {
    let Some(window) = web_sys::window() else { return };
    let over = Closure::<dyn FnMut(DragEvent)>::new(|event: DragEvent| event.prevent_default());
    let _ = window.add_event_listener_with_callback_and_bool("dragover", over.as_ref().unchecked_ref(), true);
    over.forget();
    let to = inbox.clone();
    let on_drop = Closure::<dyn FnMut(DragEvent)>::new(move |event: DragEvent| {
        event.prevent_default();
        let Some(transfer) = event.data_transfer() else { return };
        // The entries are there only while the event is.
        let items = transfer.items();
        let roots: Vec<FileSystemEntry> = (0..items.length()).filter_map(|index| items.get(index)?.webkit_get_as_entry().ok().flatten()).collect();
        let to = to.clone();
        spawn_local(async move {
            match dropped_files(roots).await {
                Ok(listed) => read_folder(listed, to).await,
                Err((path, why)) => to.unreadable(&path, why),
            }
        });
    });
    let _ = window.add_event_listener_with_callback_and_bool("drop", on_drop.as_ref().unchecked_ref(), true);
    // Listens for as long as the page lives.
    on_drop.forget();
}

/// What was dropped, as a walk of the browser lists it: hidden folders left out, a folder deeper than a walk of `engine`
/// goes listed without what it holds, and [`folder::MAX_LISTED`] files and folders at most.
async fn dropped_files(roots: Vec<FileSystemEntry>) -> Result<Listed, (String, String)> {
    let mut listed = Listed { name: "dropped".to_owned(), files: Vec::new(), folders: Vec::new(), stopped: None };
    let mut pending: Vec<(FileSystemDirectoryEntry, String)> = Vec::new();
    match roots.as_slice() {
        [only] if only.is_directory() => {
            pending.push((only.clone().unchecked_into(), String::new()));
            listed.name = only.name();
        }
        _ => {
            for root in roots {
                take(root, "", &mut pending, &mut listed);
            }
        }
    }
    while let Some((folder, prefix)) = pending.pop() {
        let reader = folder.create_reader();
        // A reader gives the entries of a folder in batches, until an empty one.
        loop {
            let batch = read_entries(&reader).await.map_err(|why| (prefix.clone(), why))?;
            if batch.length() == 0 {
                break;
            }
            for value in batch.iter() {
                if listed.files.len() + listed.folders.len() >= folder::MAX_LISTED {
                    listed.stopped =
                        Some(format!("the folder holds more than {} files and folders: the viewer reads no further", folder::MAX_LISTED));
                    return Ok(listed);
                }
                take(value.unchecked_into(), &prefix, &mut pending, &mut listed);
            }
        }
    }
    Ok(listed)
}

/// An entry of a dropped folder: a file, or a folder, walked when the walk enters it.
fn take(entry: FileSystemEntry, prefix: &str, pending: &mut Vec<(FileSystemDirectoryEntry, String)>, listed: &mut Listed) {
    let path = format!("{prefix}{}", entry.name());
    if entry.is_directory() {
        if folder::enters(&path) {
            pending.push((entry.unchecked_into(), format!("{path}/")));
        }
        listed.folders.push(path);
    } else if entry.is_file() {
        listed.files.push((path, Found::Entry(entry.unchecked_into())));
    }
}

async fn read_entries(reader: &FileSystemDirectoryReader) -> Result<js_sys::Array, String> {
    let promise = js_sys::Promise::new(&mut |resolve: js_sys::Function, reject: js_sys::Function| {
        if let Err(error) = reader.read_entries_with_callback_and_callback(&resolve, &reject) {
            let _ = reject.call1(&JsValue::NULL, &error);
        }
    });
    JsFuture::from(promise).await.map_err(js)?.dyn_into::<js_sys::Array>().map_err(|_| "not a list of entries".to_owned())
}

/// The first `most` bytes of a file.
async fn bytes_of(file: &web_sys::File, most: u64) -> Result<Vec<u8>, String> {
    let blob = file.slice_with_f64_and_f64(0.0, most as f64).map_err(js)?;
    let buffer = JsFuture::from(blob.array_buffer()).await.map_err(js)?;
    Ok(Uint8Array::new(&buffer).to_vec())
}
