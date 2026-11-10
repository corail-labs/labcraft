//! The session of the viewer (`docs/architecture.md`, 11.5): the folder open, its check, the function shown, the view
//! state that the file does not hold, the preference of the standard view. The window sends it the `view.*` commands
//! of `docs/commands.md`, 6.5, as an id and JSON parameters, and reads what it publishes. It runs on a thread of its
//! own, as large as `format::STACK_SIZE`, and checks again when the files it watches change.
//!
//! It writes no file of a folder: the viewer is read-only (scenario 6, point 6). The one file it may write is that of
//! `--style`, given by name, with the defaults, when it does not exist yet (`docs/cli.md`, 8).

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use labcraft_cli::args::CheckOptions;
use labcraft_cli::folder::{self, Function, Loaded};
use labcraft_cli::output::diagnostics_text;
use labcraft_diagram::style::{self, Style};
use labcraft_diagram::{DiagramLimits, Drawing, ViewState};
use labcraft_types::{Code, Limits};
use serde_json::{Value, json};

use crate::watch::Watching;

/// The changes that come within this time are taken together (architecture 11.5).
pub const SETTLE: Duration = Duration::from_millis(50);

/// An error of a command (`docs/commands.md`, section 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandError {
    pub code: &'static str,
    pub message: String,
    pub parameter: Option<String>,
    pub expected: Vec<String>,
}

impl CommandError {
    fn new(code: &'static str, message: impl Into<String>) -> CommandError {
        CommandError { code, message: message.into(), parameter: None, expected: Vec::new() }
    }

    fn of(code: &'static str, parameter: &str, message: impl Into<String>) -> CommandError {
        CommandError { parameter: Some(parameter.to_owned()), ..CommandError::new(code, message) }
    }

    /// `{ "error": { … } }`.
    pub fn to_json(&self) -> Value {
        let mut error = json!({ "code": self.code, "message": self.message });
        if let (Some(parameter), Some(object)) = (&self.parameter, error.as_object_mut()) {
            object.insert("parameter".to_owned(), json!(parameter));
        }
        if let (false, Some(object)) = (self.expected.is_empty(), error.as_object_mut()) {
            object.insert("expected".to_owned(), json!(self.expected));
        }
        json!({ "error": error })
    }
}

/// The view state of a function that its file does not hold (notation 2.1), kept for the session only, by its file,
/// across changes of the file, by the keys of its nodes (decided at the start of step 6).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewOf {
    /// The arm shown of each structure in tabs.
    pub arms: HashMap<String, usize>,
    pub unfolded: BTreeSet<String>,
    pub standard: BTreeSet<String>,
}

/// A function of the folder, as the tree of the window lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub function: String,
    pub file: String,
    pub drawable: bool,
    pub errors: u64,
    pub warnings: u64,
}

/// What the window shows, as the session last published it.
#[derive(Clone, Debug, Default)]
pub struct Published {
    /// Changes with each publication.
    pub generation: u64,
    pub folder: Option<String>,
    pub functions: Vec<Listed>,
    /// The diagnostics listed, as `labcraft-cli check` prints them.
    pub problems: String,
    pub errors: u64,
    pub warnings: u64,
    /// The function shown, and the tabs of the functions opened.
    pub function: Option<String>,
    pub opened: Vec<String>,
    pub drawing: Option<Arc<Drawing>>,
    /// Changes with each drawing.
    pub drawing_generation: u64,
    /// Whether the nodes glide to this drawing: the file of the function shown changed.
    pub glide: bool,
    /// The function is not drawable now: its last drawing stays, with its diagnostics (scenario 6, point 3).
    pub kept: bool,
    pub arms: HashMap<String, usize>,
    pub standard_everywhere: bool,
    /// What the viewer must say: a folder that cannot be watched.
    pub notice: Option<String>,
    /// What the viewer must say of the file of `--style`.
    pub style_notice: Option<String>,
}

/// The state of the viewer, and its commands.
pub struct Session {
    options: CheckOptions,
    loaded: Option<Loaded>,
    views: HashMap<String, ViewOf>,
    standard_everywhere: bool,
    function: Option<String>,
    /// The file of the function shown: two files can give one name.
    file: Option<String>,
    opened: Vec<String>,
    /// The last drawing of each file whose function was drawable: it holds the values it was drawn with.
    last: HashMap<String, Arc<Drawing>>,
    /// The entry of the function shown in `labcraft-diagram/1`.
    entry: Value,
    published: Published,
    /// Where the watcher sends the changes it sees: the thread of the session.
    changes: Option<Sender<Message>>,
    watching: Option<Watching>,
    /// The values of the notation in force, and the file of `--style` they come from, watched.
    style: Arc<Style>,
    style_path: Option<PathBuf>,
    style_watching: Option<notify::RecommendedWatcher>,
}

impl Session {
    /// A session with nothing open. `changes`: where a watcher of the folder sends what it sees; none, no watching.
    pub fn new(standard_everywhere: bool, changes: Option<Sender<Message>>) -> Session {
        let options = CheckOptions { deny_warnings: false, json: false, lib_paths: Vec::new(), limits: Limits::default(), folders: Vec::new() };
        Session {
            options,
            loaded: None,
            views: HashMap::new(),
            standard_everywhere,
            function: None,
            file: None,
            opened: Vec::new(),
            last: HashMap::new(),
            entry: Value::Null,
            published: Published { standard_everywhere, ..Published::default() },
            changes,
            watching: None,
            style: Arc::new(Style::default()),
            style_path: None,
            style_watching: None,
        }
    }

    /// Draws with the values of the file at `path` (`--style`): written with the defaults when it does not exist, then
    /// watched, and read again each time it changes.
    pub fn watch_style(&mut self, path: PathBuf) {
        let mut notices = Vec::new();
        if let Err(problem) = crate::style_file::prepare(&path) {
            notices.push(problem);
        }
        if let Some(changes) = &self.changes {
            match crate::style_file::watch(&path, changes.clone()) {
                Ok(watcher) => self.style_watching = Some(watcher),
                Err(problem) => notices.push(format!("{problem}: the values are read only once")),
            }
        }
        self.style_path = Some(path);
        self.style_changed();
        if let Some(notice) = self.published.style_notice.take() {
            notices.push(notice);
        }
        self.published.style_notice = (!notices.is_empty()).then(|| notices.join("; "));
    }

    /// The file of `--style` changed: reads it again, and draws again with its values when they are taken and differ.
    pub fn style_changed(&mut self) {
        let Some(path) = &self.style_path else { return };
        let (read, notice) = crate::style_file::read(path);
        self.published.style_notice = notice;
        self.published.generation = self.published.generation.wrapping_add(1);
        if let Some(read) = read
            && read != *self.style
        {
            self.style = Arc::new(read);
            // The nodes glide to their places in the new values.
            self.draw(true);
        }
    }

    pub fn published(&self) -> &Published {
        &self.published
    }

    /// A command of `docs/commands.md`, 6.5, by its id, with its parameters.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value, CommandError> {
        let Some(object) = params.as_object() else { return Err(CommandError::new("params.invalid", "the parameters are a JSON object")) };
        let known: &[&str] = match id {
            "view.open" => &["folder", "lib_path"],
            "view.show_function" => &["function"],
            "view.set_standard_view" => &["on"],
            "view.set_state" => &["function", "state"],
            _ => {
                let mut error = CommandError::new("command.unknown", format!("no command `{id}` in the viewer"));
                error.expected = ["view.open", "view.show_function", "view.set_standard_view", "view.set_state"].map(str::to_owned).to_vec();
                return Err(error);
            }
        };
        if let Some(unknown) = object.keys().find(|key| !known.contains(&key.as_str())) {
            let mut error = CommandError::of("params.unknown", unknown, format!("unknown parameter `{unknown}`"));
            error.expected = known.iter().map(|key| (*key).to_owned()).collect();
            return Err(error);
        }
        let text = |name: &str| -> Result<String, CommandError> {
            match object.get(name) {
                Some(Value::String(text)) => Ok(text.clone()),
                Some(_) => Err(CommandError::of("params.invalid", name, format!("`{name}` is a string"))),
                None => Err(CommandError::of("params.missing", name, format!("`{name}` is required"))),
            }
        };
        match id {
            "view.open" => {
                let folder = PathBuf::from(text("folder")?);
                let lib_paths = match object.get("lib_path") {
                    None => Vec::new(),
                    Some(Value::Array(paths)) => paths
                        .iter()
                        .map(|path| {
                            path.as_str()
                                .map(PathBuf::from)
                                .ok_or_else(|| CommandError::of("params.invalid", "lib_path", "`lib_path` is a list of strings"))
                        })
                        .collect::<Result<_, _>>()?,
                    Some(_) => return Err(CommandError::of("params.invalid", "lib_path", "`lib_path` is a list of strings")),
                };
                self.open(folder, lib_paths)
            }
            "view.show_function" => self.show_function(&text("function")?),
            "view.set_standard_view" => {
                let on = match object.get("on") {
                    Some(Value::Bool(on)) => *on,
                    Some(_) => return Err(CommandError::of("params.invalid", "on", "`on` is true or false")),
                    None => return Err(CommandError::of("params.missing", "on", "`on` is required")),
                };
                let before = std::mem::replace(&mut self.standard_everywhere, on);
                self.published.standard_everywhere = on;
                if before != on {
                    self.draw(false);
                }
                Ok(json!(before))
            }
            _ => {
                let function = text("function")?;
                let state = object.get("state").ok_or_else(|| CommandError::of("params.missing", "state", "`state` is required"))?;
                self.set_state(&function, state)
            }
        }
    }

    fn open(&mut self, folder: PathBuf, lib_paths: Vec<PathBuf>) -> Result<Value, CommandError> {
        if !folder.is_dir() {
            return Err(CommandError::of("path.not_found", "folder", format!("`{}` is not a folder", folder.display())));
        }
        self.options.folders = vec![folder.clone()];
        self.options.lib_paths = lib_paths.clone();
        self.views.clear();
        self.last.clear();
        self.function = None;
        self.file = None;
        self.opened.clear();
        // Watched before it is read: a change made while it is read is seen.
        if let Some(changes) = &self.changes {
            let mut paths = vec![folder];
            paths.extend(lib_paths);
            let (watching, notice) = crate::watch::watch(&paths, changes.clone());
            self.watching = Some(watching);
            self.published.notice = notice;
        }
        self.load();
        // The first function, so that the window shows something.
        if let Some(first) = self.published.functions.first().map(|listed| listed.function.clone()) {
            self.choose(&first);
        }
        self.draw(false);
        let functions: Vec<Value> = self
            .published
            .functions
            .iter()
            .map(|listed| json!({ "function": listed.function, "file": listed.file, "drawable": listed.drawable }))
            .collect();
        let check = self.loaded.as_ref().and_then(|loaded| serde_json::to_value(loaded.report.check_document(loaded.counts, false)).ok());
        Ok(json!({ "functions": functions, "check": check }))
    }

    /// Shows the function of this name: when two files give it (`a.b.lcf` and `a/b.lcf`), the first that is drawable.
    fn choose(&mut self, name: &str) {
        let matching = || self.published.functions.iter().filter(|listed| listed.function == name);
        let file = matching().find(|listed| listed.drawable).or_else(|| matching().next()).map(|listed| listed.file.clone());
        self.function = Some(name.to_owned());
        self.file = file;
        if !self.opened.iter().any(|opened| opened == name) {
            self.opened.push(name.to_owned());
        }
    }

    fn known(&self, name: &str) -> Result<(), CommandError> {
        if self.published.functions.iter().any(|listed| listed.function == name) {
            return Ok(());
        }
        let mut error = CommandError::of("params.invalid", "function", format!("unknown function `{name}`: not a function of the folder open"));
        error.expected = self.published.functions.iter().map(|listed| listed.function.clone()).take(10).collect();
        Err(error)
    }

    fn show_function(&mut self, name: &str) -> Result<Value, CommandError> {
        self.known(name)?;
        self.choose(name);
        Ok(self.draw(false))
    }

    fn set_state(&mut self, name: &str, state: &Value) -> Result<Value, CommandError> {
        self.known(name)?;
        let Some(state) = state.as_object() else { return Err(CommandError::of("params.invalid", "state", "`state` is an object")) };
        let file = self.file_of(name);
        // All of it, or nothing: read into a copy first.
        let mut view = self.views.get(&file).cloned().unwrap_or_default();
        let mut placed = false;
        for (part, value) in state {
            let Some(entries) = value.as_object() else {
                return Err(CommandError::of("params.invalid", "state", format!("`state.{part}` is an object of keys")));
            };
            for (key, value) in entries {
                match (part.as_str(), value) {
                    ("arms", Value::Number(number)) => {
                        let arm = number.as_u64().and_then(|arm| usize::try_from(arm).ok()).ok_or_else(|| {
                            CommandError::of("params.invalid", "state", format!("`state.arms.{key}` is the rank of an arm, from 0"))
                        })?;
                        view.arms.insert(key.clone(), arm);
                    }
                    ("unfolded" | "standard", Value::Bool(on)) => {
                        let set = if part == "unfolded" { &mut view.unfolded } else { &mut view.standard };
                        if *on {
                            set.insert(key.clone())
                        } else {
                            set.remove(key)
                        };
                        placed = true;
                    }
                    _ => {
                        let mut error = CommandError::of("params.invalid", "state", format!("`state.{part}.{key}` is not part of the view state"));
                        error.expected = ["arms", "unfolded", "standard"].map(str::to_owned).to_vec();
                        return Err(error);
                    }
                }
            }
        }
        self.views.insert(file.clone(), view.clone());
        if self.function.as_deref() == Some(name) {
            // An arm shown places nothing: only a structure unfolded or a node in another view draws again.
            if placed {
                return Ok(self.draw(false));
            }
            self.published.arms = view.arms;
            self.published.generation = self.published.generation.wrapping_add(1);
            return Ok(self.entry.clone());
        }
        Ok(self.render(name, &file).map(|(_, _, entry)| entry).unwrap_or(Value::Null))
    }

    /// The files changed: checks the folder again and draws the function shown, its nodes gliding when it is the same.
    pub fn files_changed(&mut self) {
        if self.options.folders.is_empty() {
            return;
        }
        let before = self.file.clone();
        self.load();
        // A function whose file went away is no longer shown; what was kept of a file gone goes too.
        let exists = |name: &String, functions: &[Listed]| functions.iter().any(|listed| listed.function == *name);
        let functions = self.published.functions.clone();
        self.opened.retain(|name| exists(name, &functions));
        let files: BTreeSet<&str> = functions.iter().map(|listed| listed.file.as_str()).collect();
        self.views.retain(|file, _| files.contains(file.as_str()));
        self.last.retain(|file, _| files.contains(file.as_str()));
        match self.function.clone() {
            // The same file while it is there: a file that breaks keeps its last drawing, even when another file gives
            // the same name.
            Some(name) if before.as_ref().is_some_and(|file| functions.iter().any(|listed| listed.function == name && listed.file == *file)) => {}
            Some(name) if exists(&name, &functions) => self.choose(&name),
            _ => match self.opened.first().cloned() {
                Some(name) => self.choose(&name),
                None => {
                    self.function = None;
                    self.file = None;
                }
            },
        }
        let same = before.is_some() && before == self.file;
        self.draw(same);
    }

    /// Reads and checks the folder again.
    fn load(&mut self) {
        // Paths as the folder open sees them: `main.lcf`, `drivers/open.lcf`.
        let current = self.options.folders.first().cloned();
        let loaded = folder::load(&self.options, current.as_deref());
        self.published.folder = loaded.root.as_ref().and(self.options.folders.first()).map(|folder| folder.display().to_string());
        self.published.functions = listed(&loaded);
        self.published.problems = diagnostics_text(&loaded.report);
        self.published.errors = loaded.report.errors();
        self.published.warnings = loaded.report.warnings();
        self.loaded = Some(loaded);
        self.published.generation = self.published.generation.wrapping_add(1);
    }

    /// The file of the function of this name that the viewer shows for it.
    fn file_of(&self, name: &str) -> String {
        if self.function.as_deref() == Some(name)
            && let Some(file) = &self.file
        {
            return file.clone();
        }
        let matching = || self.published.functions.iter().filter(|listed| listed.function == name);
        matching().find(|listed| listed.drawable).or_else(|| matching().next()).map(|listed| listed.file.clone()).unwrap_or_default()
    }

    /// The picture of a function in its view state, whether it is drawn, and its entry of `diagrams` in
    /// `labcraft-diagram/1`.
    fn render(&self, name: &str, file: &str) -> Option<(Arc<Drawing>, bool, Value)> {
        let loaded = self.loaded.as_ref()?;
        let function = loaded.functions.iter().find(|function| function.shown == file)?;
        let view = self.views.get(file).cloned().unwrap_or_default();
        let state = ViewState {
            standard_everywhere: self.standard_everywhere,
            fold_constructions: true,
            tabs: true,
            unfolded: view.unfolded,
            standard: view.standard,
        };
        let mut report = loaded.report.clone();
        let files = folder::files_of(loaded.checked.as_ref());
        // Drawn with the values in force (notation 10).
        style::scope(&self.style, || {
            let (drawing, nodes, codes) =
                folder::drawing_of(function, loaded.checked.as_ref(), &files, &loaded.names, &state, &DiagramLimits::default(), &mut report);
            let drawn = drawing.is_some();
            let picture = Arc::new(folder::picture(function, drawing, &report));
            Some((picture, drawn, entry_of(name, file, drawn, nodes, &codes)))
        })
    }

    /// Draws the function shown; returns its entry of `diagrams` in `labcraft-diagram/1`.
    fn draw(&mut self, glide: bool) -> Value {
        self.published.generation = self.published.generation.wrapping_add(1);
        self.published.function.clone_from(&self.function);
        self.published.opened.clone_from(&self.opened);
        self.published.drawing_generation = self.published.drawing_generation.wrapping_add(1);
        self.published.glide = glide;
        let (Some(name), Some(file)) = (self.function.clone(), self.file.clone()) else {
            self.published.drawing = None;
            self.entry = Value::Null;
            return Value::Null;
        };
        let Some((picture, drawn, entry)) = self.render(&name, &file) else {
            self.published.drawing = None;
            self.entry = Value::Null;
            return Value::Null;
        };
        let shown = if drawn {
            self.last.insert(file.clone(), picture.clone());
            self.published.kept = false;
            picture
        } else if let Some(last) = self.last.get(&file) {
            self.published.kept = true;
            last.clone()
        } else {
            self.published.kept = false;
            picture
        };
        self.published.drawing = Some(shown);
        self.published.arms = self.views.get(&file).map(|view| view.arms.clone()).unwrap_or_default();
        self.entry = entry.clone();
        entry
    }
}

/// An entry of `diagrams` in `labcraft-diagram/1` (`docs/cli.md`, 7.5), without the path of an SVG.
fn entry_of(name: &str, file: &str, drawn: bool, nodes: Option<u64>, codes: &[Code]) -> Value {
    let codes: Vec<&str> = codes.iter().map(|code| code.as_str()).collect();
    let mut entry = json!({ "function": name, "file": file, "drawn": drawn });
    if let Some(object) = entry.as_object_mut() {
        if let Some(nodes) = nodes {
            object.insert("nodes".to_owned(), json!(nodes));
        }
        if !codes.is_empty() {
            object.insert("codes".to_owned(), json!(codes));
        }
    }
    entry
}

/// The functions of a folder, with whether each is drawable and the counts of its file.
fn listed(loaded: &Loaded) -> Vec<Listed> {
    let files = folder::files_of(loaded.checked.as_ref());
    loaded
        .functions
        .iter()
        .map(|function: &Function| {
            let counts = loaded.report.counts_of(&function.shown);
            let drawable = files.contains_key(function.shown.as_str()) && loaded.report.keeping_from_drawing(&function.shown).is_empty();
            Listed { function: function.name.clone(), file: function.shown.clone(), drawable, errors: counts.errors, warnings: counts.warnings }
        })
        .collect()
}

/// What the thread of the session receives.
pub enum Message {
    /// A command, and where to send its result.
    Command {
        id: String,
        params: Value,
        reply: Option<Sender<Result<Value, CommandError>>>,
    },
    /// A file watched changed.
    Changed,
    /// The file of `--style` changed.
    StyleChanged,
    Stop,
}

/// The window's hold on the session that runs on its thread.
pub struct Handle {
    sender: Sender<Message>,
    published: Arc<Mutex<Published>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Handle {
    /// Starts a session on its own thread. `wake` asks the window for a frame after each publication.
    /// With `style`, the file of `--style`.
    pub fn spawn(standard_everywhere: bool, style: Option<PathBuf>, wake: Box<dyn Fn() + Send>) -> std::io::Result<Handle> {
        let (sender, receiver) = mpsc::channel();
        let mut session = Session::new(standard_everywhere, Some(sender.clone()));
        if let Some(path) = style {
            session.watch_style(path);
        }
        let published = Arc::new(Mutex::new(session.published().clone()));
        let shared = published.clone();
        let thread = std::thread::Builder::new()
            .name("labcraft-session".to_owned())
            .stack_size(labcraft_format::STACK_SIZE)
            .spawn(move || run(session, &receiver, &shared, &*wake))?;
        Ok(Handle { sender, published, thread: Some(thread) })
    }

    /// Sends a command; its result comes back on the receiver.
    pub fn send(&self, id: &str, params: Value) -> Receiver<Result<Value, CommandError>> {
        let (reply, answer) = mpsc::channel();
        let _ = self.sender.send(Message::Command { id: id.to_owned(), params, reply: Some(reply) });
        answer
    }

    /// Sends a command whose result the window does not wait for: what it changes shows in what is published.
    pub fn post(&self, id: &str, params: Value) {
        let _ = self.sender.send(Message::Command { id: id.to_owned(), params, reply: None });
    }

    /// What the session last published.
    pub fn published(&self) -> Published {
        self.published.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = self.sender.send(Message::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The loop of the thread of the session: each command in turn; changes of files taken together for [`SETTLE`].
#[expect(clippy::disallowed_methods, reason = "the desktop app, never built for WASM, where the clock traps")]
fn run(mut session: Session, receiver: &Receiver<Message>, published: &Mutex<Published>, wake: &dyn Fn()) {
    let mut waiting: Option<Message> = None;
    loop {
        let message = match waiting.take() {
            Some(message) => message,
            None => match receiver.recv() {
                Ok(message) => message,
                Err(_) => return,
            },
        };
        match message {
            Message::Stop => return,
            Message::Command { id, params, reply } => {
                // A bug of a command must not end the session.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session.execute(&id, &params))).unwrap_or_else(|_| {
                    Err(CommandError::new("internal", format!("internal error, which is a bug of the viewer: {}", labcraft_cli::fmt::last_panic())))
                });
                // What it changed is published before it answers.
                *published.lock().unwrap_or_else(PoisonError::into_inner) = session.published().clone();
                wake();
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
            }
            first @ (Message::Changed | Message::StyleChanged) => {
                // The changes of the 50 ms after the first one, with it: a window that does not move, so that a file
                // written without end still lets the folder be checked. A command that comes meanwhile waits for the
                // check.
                let (mut files, mut values) = (matches!(first, Message::Changed), matches!(first, Message::StyleChanged));
                let deadline = Instant::now() + SETTLE;
                loop {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        break;
                    }
                    match receiver.recv_timeout(left) {
                        Ok(Message::Changed) => files = true,
                        Ok(Message::StyleChanged) => values = true,
                        Ok(other) => {
                            waiting = Some(other);
                            break;
                        }
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                if values {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session.style_changed()));
                }
                if files {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session.files_changed()));
                }
            }
        }
        *published.lock().unwrap_or_else(PoisonError::into_inner) = session.published().clone();
        wake();
    }
}
