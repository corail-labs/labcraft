//! The app's hold on the session of a project (`docs/architecture.md`, 12.9): a session of `engine` on a thread of its
//! own as large as `format::STACK_SIZE`, its root the folder of the project, on the disk (no longer read-only: the
//! window writes `New project`, and the steps of the history write back); the watching of its files, which it tells
//! the session of; the file of `--style`, whose values it hands the session. The window reaches it as an
//! [`ide::Host`](labcraft_ide::Host): a command sent on a channel, and the last snapshot of what it publishes.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use labcraft_cli::disk::Disk;
use labcraft_diagram::style::Style;
use labcraft_engine::Config;
use labcraft_ide::{Host, Reply, Snapshot};
use labcraft_types::Limits;
use serde_json::{Value, json};

use crate::watch::Watching;

pub use labcraft_engine::CommandError;

/// The changes that come within this time are taken together (architecture 11.5).
pub const SETTLE: Duration = Duration::from_millis(50);

/// How often the trace of the function shown is looked at, for a new run (decisions of the owner, 2026-11-13).
pub const TRACE_POLL: Duration = Duration::from_secs(1);

/// The first function of a new project (ide.md 10; commands.md 6.7, `project.new`), the file in canonical form.
pub const NEW_MAIN: (&str, &str) = ("main.lcf", "labcraft 1\n\nfn main() -> () {}\n");

/// What the session of a project holds on its thread.
pub struct Session {
    engine: labcraft_engine::Session,
    folder: PathBuf,
    notices: Vec<String>,
    /// Where the watcher sends the changes it sees: the thread of the session.
    changes: Option<Sender<Message>>,
    watching: Option<Watching>,
    /// The values of the notation in force, and the file of `--style` they come from, watched.
    style: Arc<Style>,
    style_path: Option<PathBuf>,
    style_watching: Option<notify::RecommendedWatcher>,
    style_notice: Option<String>,
}

impl Session {
    /// The session of the project at `folder`, which looks for libraries in `lib_paths` (`--lib-path`), with the
    /// window that shows it. `changes`: where a watcher of the folder sends what it sees; none, no watching.
    /// `user_settings`: the keys of the store of the user (commands.md 9.7), which the window keeps.
    pub fn new(
        folder: &Path,
        lib_paths: Vec<PathBuf>,
        user_settings: &serde_json::Map<String, Value>,
        changes: Option<Sender<Message>>,
    ) -> Result<Session, CommandError> {
        // The home folder and the root of a disk are no project (mcp.md 1): the session would write its journal there and
        // watch all of it.
        if let Some(why) = labcraft_cli::mcp::too_wide_folder(folder) {
            return Err(CommandError::of("path.invalid", "folder", format!("{} {why}: open the folder of a project", folder.display())));
        }
        let config = Config {
            root: folder.to_path_buf(),
            lib_paths: lib_paths.clone(),
            limits: Limits::default(),
            shipped: labcraft_cli::shipped(),
            natives: Vec::new(),
            // The real clock: the journal and the history say when (architecture 12.5).
            clock: Arc::new(labcraft_cli::clock::Real::new()),
            process: std::process::id(),
        };
        let mut engine = labcraft_engine::Session::new(Arc::new(Disk), config, false)
            .map_err(|error| CommandError { parameter: Some("folder".to_owned()), ..error })?;
        engine.with_window();
        engine.set_user_settings(user_settings);
        let mut session = Session {
            engine,
            folder: folder.to_path_buf(),
            notices: Vec::new(),
            changes,
            watching: None,
            style: Arc::new(Style::default()),
            style_path: None,
            style_watching: None,
            style_notice: None,
        };
        // Watched before it is read: a change made while it is read is seen.
        if let Some(changes) = &session.changes {
            let mut paths = vec![folder.to_path_buf()];
            paths.extend(lib_paths);
            let (watching, notice) = crate::watch::watch(&paths, changes.clone());
            session.watching = Some(watching);
            session.notices.extend(notice);
        }
        Ok(session)
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
        notices.extend(self.style_notice.take());
        self.style_notice = (!notices.is_empty()).then(|| notices.join("; "));
    }

    /// The file of `--style` changed: reads it again, and draws again with its values when they are taken and differ.
    pub fn style_changed(&mut self) {
        let Some(path) = &self.style_path else { return };
        let (read, notice) = crate::style_file::read(path);
        self.style_notice = notice;
        if let Some(read) = read
            && read != *self.style
        {
            self.style = Arc::new(read);
            // The nodes glide to their places in the new values.
            self.engine.set_style(self.style.clone());
        }
    }

    /// What the window shows now.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            view: self.engine.view().clone(),
            project: Some(self.folder.display().to_string()),
            session: 0,
            notices: self.notices.iter().chain(self.style_notice.iter()).cloned().collect(),
        }
    }

    /// Opens the folder: what `view.open` returns. `main`: the project is new, and `main.lcf` is written first.
    pub fn open(&mut self, new: bool) -> Result<Value, CommandError> {
        if new {
            self.engine.open_on(Some("main".to_owned()));
        }
        self.engine.execute("view.open", &json!({ "folder": "." }))
    }

    /// A command of the session, for the window.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value, CommandError> {
        self.engine.execute(id, params)
    }

    /// Reads the trace of the function shown when its drawing is new or a run replaced it; whether anything changed.
    pub fn look_at_trace(&mut self) -> bool {
        self.engine.look_at_trace()
    }

    /// Shows the values of the trace at `path`, chosen in the window, over the function of the window that shows
    /// `function`, else of the window of the project.
    pub fn open_trace_at(&mut self, function: Option<&str>, path: PathBuf) -> Result<Value, CommandError> {
        self.engine.open_trace_at(function, path).map(|()| Value::Null)
    }

    /// The files watched changed: the session reads, checks and draws again.
    pub fn files_changed(&mut self) {
        self.engine.files_changed();
    }

    pub fn set_zoom(&mut self, function: &str, zoom: f32) {
        self.engine.set_zoom(function, zoom);
    }
}

/// What the thread of the session receives.
pub enum Message {
    /// A command, and where to send its result.
    Command {
        id: String,
        params: Value,
        reply: Option<Sender<Result<Value, CommandError>>>,
    },
    /// A trace chosen in the window, anywhere on the disk.
    OpenTrace(Option<String>, PathBuf),
    /// The zoom of the canvas shown.
    Zoom(String, f32),
    /// The keys of the store of the user, which the window keeps (commands.md 9.7).
    UserSettings(serde_json::Map<String, Value>),
    /// A call of an agent attached to the window (ide.md 8), signed by it, and where to send its result.
    Agent {
        author: labcraft_engine::Author,
        id: String,
        params: Value,
        reply: Sender<Result<Value, CommandError>>,
    },
    /// What the session serves to an agent: its tools, and its runs.
    Served(Sender<labcraft_mcp::Served>),
    /// A file watched changed.
    Changed,
    /// The file of `--style` changed.
    StyleChanged,
    Stop,
}

/// What a project is opened with.
pub struct Opening {
    pub folder: PathBuf,
    pub lib_paths: Vec<PathBuf>,
    /// The keys of the store of the user (commands.md 9.7).
    pub user_settings: serde_json::Map<String, Value>,
    pub style: Option<PathBuf>,
    /// A new project: `main.lcf` is written before it opens.
    pub new: bool,
}

/// The window's hold on the session of a project, which runs on its thread.
pub struct Handle {
    sender: Sender<Message>,
    snapshot: Arc<Mutex<Snapshot>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Handle {
    /// Starts the session of a project on its own thread, and opens it; what `view.open` returns comes on the reply.
    /// `wake` asks the window for a frame after each publication.
    pub fn spawn(opening: Opening, wake: Box<dyn Fn() + Send>) -> std::io::Result<(Handle, Reply)> {
        let (sender, receiver) = mpsc::channel();
        let (answer, reply) = Reply::channel();
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let shared = snapshot.clone();
        let changes = sender.clone();
        let thread = std::thread::Builder::new().name("labcraft-session".to_owned()).stack_size(labcraft_format::STACK_SIZE).spawn(move || {
            let made = caught(|| {
                // `main.lcf` is written before the session of the project is made, by a session of its own: opening a new
                // project is no step of its history, nor an external write (commands.md 6.7).
                if opening.new {
                    write_first_function(&opening.folder, opening.lib_paths.clone())?;
                }
                let mut session = Session::new(&opening.folder, opening.lib_paths, &opening.user_settings, Some(changes))?;
                if let Some(path) = opening.style {
                    session.watch_style(path);
                }
                Ok(session)
            });
            let mut session = match made {
                Ok(session) => session,
                Err(error) => {
                    let _ = answer.send(Err(error));
                    return;
                }
            };
            let opened = caught(|| session.open(opening.new));
            publish(&session, &shared, &*wake);
            let failed = opened.is_err();
            let _ = answer.send(opened);
            if !failed {
                run(session, &receiver, &shared, &*wake);
            }
        })?;
        Ok((Handle { sender, snapshot, thread: Some(thread) }, reply))
    }

    /// Where the thread of the session receives: the agents send their calls there.
    pub fn sender(&self) -> Sender<Message> {
        self.sender.clone()
    }

    /// What the session serves to an agent, once it has answered; none when it ended.
    pub fn served(&self) -> Option<labcraft_mcp::Served> {
        served(&self.sender)
    }

    /// Shows the values of a trace chosen in the window, over the function of the window that shows `function`; what
    /// it changes shows in what is published.
    pub fn open_trace(&self, function: Option<String>, path: &Path) {
        let _ = self.sender.send(Message::OpenTrace(function, path.to_path_buf()));
    }
}

impl Host for Handle {
    fn send(&self, id: &str, params: Value) -> Reply {
        let (sender, reply) = Reply::channel();
        let _ = self.sender.send(Message::Command { id: id.to_owned(), params, reply: Some(sender) });
        reply
    }

    fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn set_zoom(&self, function: &str, zoom: f32) {
        let _ = self.sender.send(Message::Zoom(function.to_owned(), zoom));
    }

    fn set_user_settings(&self, keys: &serde_json::Map<String, Value>) {
        let _ = self.sender.send(Message::UserSettings(keys.clone()));
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

/// What the session whose thread receives on `sender` serves to an agent, once it has answered; none when it ended.
pub fn served(sender: &Sender<Message>) -> Option<labcraft_mcp::Served> {
    let (reply, served) = mpsc::channel();
    sender.send(Message::Served(reply)).ok()?;
    served.recv_timeout(Duration::from_secs(30)).ok()
}

/// A bug of the app, out of a command (`engine` catches those of its commands), must not end the session.
fn caught<T>(work: impl FnOnce() -> Result<T, CommandError>) -> Result<T, CommandError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
        .unwrap_or_else(|_| Err(CommandError::internal(format!("internal error, which is a bug of the app: {}", labcraft_cli::fmt::last_panic()))))
}

fn publish(session: &Session, shared: &Mutex<Snapshot>, wake: &dyn Fn()) {
    *shared.lock().unwrap_or_else(PoisonError::into_inner) = session.snapshot();
    wake();
}

/// The loop of the thread of the session: each command in turn; changes of files taken together for [`SETTLE`]. After
/// each, what it drew is published, then the trace of the function shown is read and its values published (`docs/cli.md`,
/// 7.4); and every [`TRACE_POLL`], the trace is looked at again.
#[expect(clippy::disallowed_methods, reason = "the desktop app, never built for WASM, where the clock traps")]
fn run(mut session: Session, receiver: &Receiver<Message>, shared: &Mutex<Snapshot>, wake: &dyn Fn()) {
    let mut waiting: Option<Message> = None;
    let look = |session: &mut Session| {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session.look_at_trace())).unwrap_or(false) {
            publish(session, shared, wake);
        }
    };
    loop {
        let message = match waiting.take() {
            Some(message) => message,
            None => match receiver.recv_timeout(TRACE_POLL) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => {
                    look(&mut session);
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            },
        };
        match message {
            Message::Stop => return,
            Message::Command { id, params, reply } => {
                let result = caught(|| session.execute(&id, &params));
                // What it changed is published before it answers.
                publish(&session, shared, wake);
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
            }
            Message::Agent { author, id, params, reply } => {
                let result = caught(|| session.engine.execute_as(&author, &id, &params));
                publish(&session, shared, wake);
                let _ = reply.send(result);
            }
            Message::Served(reply) => {
                let _ = reply.send(labcraft_mcp::Served::of(&session.engine, true));
                continue;
            }
            Message::OpenTrace(function, path) => {
                let _ = caught(|| session.open_trace_at(function.as_deref(), path));
            }
            Message::Zoom(function, zoom) => {
                session.set_zoom(&function, zoom);
                continue;
            }
            Message::UserSettings(keys) => {
                session.engine.set_user_settings(&keys);
                publish(&session, shared, wake);
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
        publish(&session, shared, wake);
        look(&mut session);
    }
}

/// Writes `main.lcf` in the folder of a new project that another window will open, by `file.write` as the window does.
pub fn write_first_function(folder: &Path, lib_paths: Vec<PathBuf>) -> Result<(), CommandError> {
    let mut session = Session::new(folder, lib_paths, &serde_json::Map::new(), None)?;
    let (path, text) = NEW_MAIN;
    session.execute("file.write", &json!({ "path": path, "text": text, "check": false })).map(|_| ())
}

/// Makes the folder of a new project (ide.md 10): a folder that exists and is not empty is `path.exists`.
pub fn make_new_folder(folder: &Path) -> Result<(), CommandError> {
    if let Ok(mut entries) = std::fs::read_dir(folder)
        && entries.next().is_some()
    {
        return Err(CommandError::of("path.exists", "folder", format!("{} is not empty: a new project needs an empty folder", folder.display())));
    }
    std::fs::create_dir_all(folder).map_err(|error| CommandError::of("path.invalid", "folder", format!("{}: {error}", folder.display())))
}
