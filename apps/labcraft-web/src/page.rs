//! The page (`docs/architecture.md`, 12.9): the window of `ide`, the same as the desktop's, in a canvas of the browser,
//! eframe on WebGPU, or WebGL2 where the browser lacks it (`?webgl` asks for it); the session in a worker, which the
//! [`Supervisor`] starts, feeds and restarts, behind the [`Shared`] host the window talks to. The project is in memory
//! (ide.md 12): a bundled folder, a new project, or a folder dropped on the page or chosen, read once; a second one
//! replaces it, after asking when it changed. No trace in the browser: a run needs threads (D-22).
//!
//! The address names the bundled folder and the function shown ([`link`](crate::link)), and the page writes it back as
//! the developer moves; without one, the start page. `?smoke`: the page opens the examples, traps its worker once it
//! drew, and says to the server that served it whether the worker came back and drew again (`cargo xtask web-smoke`).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

use craft_ui::fonts::{Family, I18nScripts, UiFonts};
use egui::{FontData, FontFamily};
use js_sys::Uint8Array;
use labcraft_diagram::Drawing;
use labcraft_engine::settings;
use labcraft_ide::{Host, Ide, Outward, Prefs, StartList};
use serde_json::{Value, json};
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
use crate::protocol::{File, Folder};
use crate::supervisor::{Action, Shared, Supervisor};

/// Inter 4.1 Regular, the face of the panels (architecture 11.5; `ATTRIBUTION.md`).
static INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");

/// The worker, as Trunk writes it next to the page: a script that loads its WASM.
const WORKER: &str = "./labcraft-worker_loader.js";

/// Where the page keeps the settings of the user (ide.md 12), as the window keeps them, in JSON.
const PREFS_KEY: &str = "labcraft.ide.prefs";
/// The preferences of the viewer of M0v, read once into them.
const VIEWER_THEME_KEY: &str = "labcraft.viewer.theme";
const VIEWER_STANDARD_KEY: &str = "labcraft.viewer.standard";

/// The first function of a new project (ide.md 10), as the desktop writes it.
const NEW_MAIN: (&str, &str) = ("main.lcf", "labcraft 1\n\nfn main() -> () {}\n");

/// Where the page leads when it cannot start.
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
        let started = runner.start(canvas, options, Box::new(|creation| Ok(Box::new(Page::new(creation))))).await;
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

struct Page {
    ide: Ide,
    shared: Shared,
    inbox: Inbox,
    running: Option<Running>,
    /// The workers started, the first counting 1: what a worker ended says is not for the one that runs.
    started: u64,
    start: StartList,
    /// The bundled folder open, which has an address; `None` for a project of the developer's.
    bundled: Option<&'static str>,
    /// The address and the title last written: the bundled folder, the project, the function.
    written: Option<(Option<&'static str>, Option<String>, Option<String>)>,
    backend: String,
    smoke: Option<Smoke>,
    /// Whether the browser asks before the page is left.
    guarded: bool,
}

thread_local! {
    /// Whether leaving the page loses changes, which the listener of `beforeunload` reads.
    static LEAVING_LOSES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Asks the browser to confirm leaving the page while a project changed.
fn guard_leaving() {
    let Some(window) = web_sys::window() else { return };
    let listener = Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(|event: web_sys::BeforeUnloadEvent| {
        if LEAVING_LOSES.with(std::cell::Cell::get) {
            event.prevent_default();
            event.set_return_value("The changes to this project are lost: the page holds it in memory.");
        }
    });
    let _ = window.add_event_listener_with_callback("beforeunload", listener.as_ref().unchecked_ref());
    // Listens for as long as the page lives.
    listener.forget();
}

impl Page {
    fn new(creation: &eframe::CreationContext<'_>) -> Page {
        let ctx = creation.egui_ctx.clone();
        install_fonts(&ctx);
        let prefs = read_prefs();
        let user_settings = Value::Object(prefs.settings.clone()).to_string();
        let backend = creation.wgpu_render_state.as_ref().map(|state| format!("{:?}", state.adapter.get_info().backend)).unwrap_or_default();
        let inbox = Inbox { queue: Rc::default(), ctx: ctx.clone() };
        listen_to_drops(&inbox);
        guard_leaving();
        let examples = BUNDLED.iter().map(|bundled| labcraft_ide::catalog::example(bundled.name)).collect();
        let mut page = Page {
            // The browser draws the frame of its window.
            ide: Ide::new(&ctx, prefs, craft_window::Frame::Desktop),
            shared: Shared::new(Supervisor::new(user_settings)),
            inbox,
            running: None,
            started: 0,
            start: StartList { recent: Vec::new(), examples, problem: None, in_memory: true },
            bundled: None,
            written: None,
            backend,
            smoke: None,
            guarded: false,
        };
        // The folder and the function of the address; without one, the start page.
        let link = address();
        page.smoke = link.smoke.then_some(Smoke::Drawing { since: 0.0 });
        let names: Vec<&'static str> = BUNDLED.iter().map(|bundled| bundled.name).collect();
        if link.names_a_project()
            && let Some(opening) = link.resolve(&names)
            && let Some(bundled) = BUNDLED.iter().find(|bundled| bundled.name == opening.folder)
        {
            page.open(bundled.folder(), Some(bundled.name), opening.notice, opening.function);
        }
        page
    }

    fn has_project(&self) -> bool {
        self.shared.supervisor.borrow().folder().is_some()
    }

    /// Whether the project open changed: its files, or its history (a trap of the worker loses the history, not the files).
    fn changed(&self) -> bool {
        self.has_project() && (self.shared.supervisor.borrow().edited() || !self.ide.history_is_empty())
    }

    /// Whether another project may replace the one open: always, unless it changed and the user says no (ide.md 12).
    fn may_replace(&self) -> bool {
        if !self.changed() {
            return true;
        }
        let name = self.shared.supervisor.borrow().folder_name().unwrap_or_default().to_owned();
        let question = format!("Open another project? The changes to `{name}` are lost: the page holds one project, in memory.");
        web_sys::window().and_then(|window| window.confirm_with_message(&question).ok()).unwrap_or(false)
    }

    /// A project of the developer's (`bundled`: `None`) or a bundled folder, on `function` when it has it.
    fn open(&mut self, folder: Folder, bundled: Option<&'static str>, notice: Option<String>, function: Option<String>) {
        self.bundled = bundled;
        self.start.problem = None;
        if let Some(notice) = notice {
            self.ide.say(notice);
        }
        self.shared.with(|supervisor| supervisor.open(folder, function));
        self.perform();
    }

    /// Does what the supervisor asks.
    fn perform(&mut self) {
        loop {
            let actions = self.shared.take_actions();
            if actions.is_empty() {
                return;
            }
            for action in actions {
                match action {
                    Action::Start => {
                        self.running = None;
                        self.started += 1;
                        match spawn(self.started, &self.inbox) {
                            Ok(running) => self.running = Some(running),
                            Err(error) => self.shared.with(|supervisor| supervisor.not_started(&error)),
                        }
                    }
                    Action::Send(bytes) => {
                        if let Some(running) = &self.running {
                            let array = Uint8Array::from(bytes.as_slice());
                            if let Err(error) = running.worker.post_message_with_transfer(&array, &js_sys::Array::of1(&array.buffer())) {
                                let why = format!("it could not be reached: {}", js(error));
                                self.shared.with(|supervisor| supervisor.not_started(&why));
                            }
                        }
                    }
                }
            }
        }
    }

    /// What came since the last frame.
    fn take_incoming(&mut self) {
        for incoming in self.inbox.take() {
            match incoming {
                Incoming::Worker(worker, _) if worker != self.started => {}
                Incoming::Worker(_, FromWorker::Message(bytes)) => self.shared.with(|supervisor| supervisor.received(&bytes)),
                Incoming::Worker(_, FromWorker::Trapped) => {
                    self.running = None;
                    self.shared.with(Supervisor::trapped);
                }
                Incoming::Folder(folder, notice) => {
                    if self.may_replace() {
                        self.open(folder, None, notice, None);
                    }
                }
                Incoming::Failed(why) => self.say(why),
            }
            self.perform();
        }
    }

    /// A problem, on the start page when no project is open.
    fn say(&mut self, problem: String) {
        if self.has_project() {
            self.ide.say(problem);
        } else {
            self.start.problem = Some(problem);
        }
    }

    /// What the window asks of the page.
    fn outward(&mut self, ctx: &egui::Context, asked: Outward) {
        match asked {
            Outward::NewProject(name) => {
                if self.may_replace() {
                    // A name given (the form of the palette) is the name of the project; its last part, for a path.
                    let name = name.as_deref().and_then(|name| name.rsplit(['/', '\\']).find(|part| !part.is_empty())).unwrap_or("new project");
                    let (path, text) = NEW_MAIN;
                    let folder = Folder {
                        name: name.to_owned(),
                        files: vec![File { path: path.to_owned(), bytes: text.as_bytes().to_vec() }],
                        folders: Vec::new(),
                    };
                    self.open(folder, None, None, Some("main".to_owned()));
                }
            }
            // Asked once the folder is read, as for a folder dropped.
            Outward::OpenFolder(_) => pick_folder(&self.inbox),
            Outward::OpenExample(at) => {
                if let Some(bundled) = BUNDLED.get(at)
                    && self.may_replace()
                {
                    self.open(bundled.folder(), Some(bundled.name), None, link::default_function(bundled.name));
                }
            }
            // The page keeps no project of its own: what was opened is gone with it.
            Outward::OpenRecent(_) => {}
            Outward::OpenLink(link) => ctx.open_url(egui::OpenUrl::new_tab(link)),
            Outward::OpenTrace(_) => {
                self.say("A trace comes from a run, which the web page cannot make: runs need the desktop app (D-22)".to_owned())
            }
            // The page shows nothing of AI (ide.md 8), and never offers it.
            Outward::ConnectAgent => {}
        }
    }

    /// The address and the title of the page, as what is on screen: a link to it, for a bundled folder.
    fn write_address(&mut self) {
        let snapshot = self.shared.snapshot();
        if self.shared.supervisor.borrow().busy() {
            return;
        }
        let function = snapshot.view.main.function.clone();
        let now = (self.bundled, snapshot.project.clone(), function.clone());
        if self.written.as_ref() == Some(&now) {
            return;
        }
        self.written = Some(now);
        let Some(window) = web_sys::window() else { return };
        let search = Link::search(self.bundled, function.as_deref(), address().webgl);
        if let (Ok(history), Ok(path)) = (window.history(), window.location().pathname()) {
            // `?smoke` keeps its address: its server reads nothing else, but the address is not the page's to change.
            if self.smoke.is_none() {
                let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&format!("{path}{search}")));
            }
        }
        if let Some(document) = window.document() {
            let title = match (&function, &snapshot.project) {
                (Some(function), Some(project)) => format!("{function} · {project} — LabCraft"),
                (None, Some(project)) => format!("{project} — LabCraft"),
                _ => "LabCraft".to_owned(),
            };
            document.set_title(&title);
        }
    }

    /// `?smoke`: one step each frame, until it reports.
    fn smoke(&mut self, now: f64, ctx: &egui::Context) {
        let Some(step) = self.smoke.take() else { return };
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        let snapshot = self.shared.snapshot();
        let drawing = snapshot.view.main.drawing.clone();
        let drawn = !self.shared.supervisor.borrow().busy() && drawing.is_some() && snapshot.notices.is_empty();
        let function = snapshot.view.main.function.clone().unwrap_or_default();
        self.smoke = Some(match step {
            Smoke::Drawing { .. } if drawn && self.started == 1 => {
                let groups = drawing.as_ref().map_or(0, |drawing| drawing.groups.len());
                self.shared.with(Supervisor::trap);
                self.perform();
                Smoke::Trapped { since: now, first: format!("drew `{function}` ({groups} groups)"), before: drawing }
            }
            // The new worker drew anew: not the drawing that the page kept.
            Smoke::Trapped { first, before, .. }
                if drawn && self.started == 2 && !before.as_ref().zip(drawing.as_ref()).is_some_and(|(before, now)| Arc::ptr_eq(before, now)) =>
            {
                report(&format!("ok: {}; {first}; the worker trapped and restarted; drew `{function}` again", self.backend));
                Smoke::Done
            }
            Smoke::Drawing { since } | Smoke::Trapped { since, .. } if now - since > SMOKE_PATIENCE => {
                report(&format!("fail: {} workers started, function {function:?}, notices {:?}", self.started, snapshot.notices));
                Smoke::Done
            }
            other => other,
        });
    }
}

impl eframe::App for Page {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.take_incoming();
        let host = self.has_project().then_some(&self.shared as &dyn Host);
        let outward = self.ide.show(ui, host, &self.start);
        self.perform();
        for asked in outward {
            self.outward(ui.ctx(), asked);
        }
        if self.ide.take_prefs_changed() {
            write_prefs(&self.ide.prefs());
        }
        // Leaving the page, or reloading it, loses a project that changed: the browser asks first.
        let changed = self.changed();
        if self.guarded != changed {
            self.guarded = changed;
            LEAVING_LOSES.with(|loses| loses.set(changed));
        }
        self.write_address();
        let now = ui.input(|input| input.time);
        self.smoke(now, ui.ctx());
    }
}

/// The settings of the user, from the storage of the browser; until the user changes one, those of the viewer of M0v,
/// and the theme and the motion that the system asks for at each start.
fn read_prefs() -> Prefs {
    if let Some(prefs) = stored(PREFS_KEY).and_then(|json| serde_json::from_str::<Prefs>(&json).ok()) {
        return prefs;
    }
    let mut prefs = Prefs::default();
    let mut dark = prefers("(prefers-color-scheme: dark)");
    match stored(VIEWER_THEME_KEY).as_deref() {
        Some("dark") => dark = true,
        Some("light") => dark = false,
        _ => {}
    }
    let keys = &mut prefs.settings;
    if dark {
        keys.insert(settings::THEME.to_owned(), json!("dark"));
    }
    if prefers("(prefers-reduced-motion: reduce)") {
        keys.insert(settings::REDUCED_MOTION.to_owned(), json!(true));
    }
    if stored(VIEWER_STANDARD_KEY).as_deref() == Some("true") {
        keys.insert(settings::STANDARD_EVERYWHERE.to_owned(), json!(true));
    }
    prefs
}

/// Keeps the settings, which then replace those of the viewer of M0v.
fn write_prefs(prefs: &Prefs) {
    if let Ok(json) = serde_json::to_string(prefs) {
        store(PREFS_KEY, &json);
    }
    if let Some(storage) = storage() {
        let _ = storage.remove_item(VIEWER_THEME_KEY);
        let _ = storage.remove_item(VIEWER_STANDARD_KEY);
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
        .app_face("Hack", FontData::from_static(labcraft_editor::HACK), &[FontFamily::Monospace, notation])
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
                    listed.stopped = Some(format!("the folder holds more than {} files and folders: the page reads no further", folder::MAX_LISTED));
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
