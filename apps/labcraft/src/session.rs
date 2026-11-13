//! The viewer's hold on the session of `engine` (`docs/architecture.md`, 11.5): the state of the viewer and the
//! `view.*` commands are in `engine::view`; here are what only an app does, on a thread of its own as large as
//! `format::STACK_SIZE`: a session of `engine` for each folder opened, its root that folder (decision of the owner,
//! 2026-11-11), on the disk made read-only; the watching of its files, which it tells the session of; the file of
//! `--style`, whose values it hands the session; and what the window shows, published after each message.
//!
//! It writes no file of a folder: the viewer is read-only (scenario 6, point 6). The one file it may write is that of
//! `--style`, given by name, with the defaults, when it does not exist yet (`docs/cli.md`, 8).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use labcraft_cli::disk::Disk;
use labcraft_diagram::style::Style;
use labcraft_engine::view::Published as ViewPublished;
use labcraft_engine::{Config, ReadOnly, Still};
use labcraft_types::printed::diagnostics_text;
use labcraft_types::{Limits, Report};
use serde_json::{Value, json};

use crate::watch::Watching;

pub use labcraft_engine::CommandError;

/// The changes that come within this time are taken together (architecture 11.5).
pub const SETTLE: Duration = Duration::from_millis(50);

/// What the window shows, as the session last published it.
#[derive(Clone, Debug, Default)]
pub struct Published {
    /// What `engine::view` publishes: the functions, the function shown, its drawing, the view state.
    pub view: ViewPublished,
    /// The folder open, as it was given; none when none is open.
    pub folder: Option<String>,
    /// The diagnostics listed, as `labcraft-cli check` prints them.
    pub problems: String,
    /// What the viewer must say: a folder that cannot be watched.
    pub notice: Option<String>,
    /// What the viewer must say of the file of `--style`.
    pub style_notice: Option<String>,
}

/// What the viewer holds on its thread.
pub struct Session {
    /// The session of the folder open.
    engine: Option<labcraft_engine::Session>,
    lib_paths: Vec<PathBuf>,
    standard_everywhere: bool,
    published: Published,
    /// The report that `problems` was written from.
    written_from: Option<Arc<Report>>,
    /// What the generations of the session open count from: each new session counts from 0, and the window must see a
    /// new drawing all the same.
    base: (u64, u64),
    /// Where the watcher sends the changes it sees: the thread of the session.
    changes: Option<Sender<Message>>,
    watching: Option<Watching>,
    /// The values of the notation in force, and the file of `--style` they come from, watched.
    style: Arc<Style>,
    style_path: Option<PathBuf>,
    style_watching: Option<notify::RecommendedWatcher>,
}

impl Session {
    /// A session with nothing open, which looks for libraries in `lib_paths` (`--lib-path`). `changes`: where a watcher
    /// of the folder sends what it sees; none, no watching.
    pub fn new(standard_everywhere: bool, lib_paths: Vec<PathBuf>, changes: Option<Sender<Message>>) -> Session {
        Session {
            engine: None,
            lib_paths,
            standard_everywhere,
            published: Published { view: ViewPublished { standard_everywhere, ..ViewPublished::default() }, ..Published::default() },
            written_from: None,
            base: (0, 0),
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
        if let Some(read) = read
            && read != *self.style
        {
            self.style = Arc::new(read);
            // The nodes glide to their places in the new values.
            if let Some(engine) = &mut self.engine {
                engine.set_style(self.style.clone());
            }
        }
        self.sync();
    }

    /// What the window shows now.
    pub fn published(&self) -> &Published {
        &self.published
    }

    /// Opens a folder anywhere on the disk: a session of `engine` whose root it is, which the `view.*` commands then
    /// reach; returns what `view.open` returns. The folder open before stays open when this one cannot be.
    pub fn open(&mut self, folder: &Path) -> Result<Value, CommandError> {
        let config = Config {
            root: folder.to_path_buf(),
            lib_paths: self.lib_paths.clone(),
            limits: Limits::default(),
            shipped: labcraft_cli::shipped(),
            natives: Vec::new(),
            clock: Arc::new(Still),
            process: std::process::id(),
        };
        let mut engine = labcraft_engine::Session::new(Arc::new(ReadOnly(Arc::new(Disk))), config, false)
            .map_err(|error| CommandError { parameter: Some("folder".to_owned()), ..error })?;
        engine.set_style(self.style.clone());
        engine.execute("view.set_standard_view", &json!({ "on": self.standard_everywhere }))?;
        // Watched before it is read: a change made while it is read is seen.
        let watched = self.changes.as_ref().map(|changes| {
            let mut paths = vec![folder.to_path_buf()];
            paths.extend(self.lib_paths.iter().cloned());
            crate::watch::watch(&paths, changes.clone())
        });
        let opened = engine.execute("view.open", &json!({ "folder": "." }));
        if opened.is_ok() {
            let view = &self.published.view;
            self.base = (view.generation.wrapping_add(1), view.drawing_generation.wrapping_add(1));
            self.engine = Some(engine);
            self.published.folder = Some(folder.display().to_string());
            if let Some((watching, notice)) = watched {
                self.watching = Some(watching);
                self.published.notice = notice;
            }
        }
        self.sync();
        opened
    }

    /// A command of the session of the folder open: the `view.*` commands of `docs/commands.md`, 6.5.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value, CommandError> {
        // The preference holds before any folder is open, and goes to each session made after.
        let standard = (id == "view.set_standard_view").then(|| params.get("on").and_then(Value::as_bool)).flatten();
        let Some(engine) = &mut self.engine else {
            if let Some(on) = standard {
                let before = std::mem::replace(&mut self.standard_everywhere, on);
                self.published.view.standard_everywhere = on;
                return Ok(json!({ "before": before }));
            }
            return Err(CommandError::new("command.not_enabled", format!("`{id}` waits for a folder: open one first")));
        };
        let result = engine.execute(id, params);
        if let (Some(on), Ok(_)) = (standard, &result) {
            self.standard_everywhere = on;
        }
        self.sync();
        result
    }

    /// The files watched changed: the session reads, checks and draws again.
    pub fn files_changed(&mut self) {
        if let Some(engine) = &mut self.engine {
            engine.files_changed();
        }
        self.sync();
    }

    /// Takes what the session of `engine` publishes; the text of `Problems` is written again when its report is new.
    fn sync(&mut self) {
        let Some(engine) = &self.engine else { return };
        self.published.view = engine.view().clone();
        let view = &mut self.published.view;
        view.generation = view.generation.wrapping_add(self.base.0);
        view.drawing_generation = view.drawing_generation.wrapping_add(self.base.1);
        let report = &self.published.view.report;
        let same = match (report, &self.written_from) {
            (Some(report), Some(before)) => Arc::ptr_eq(report, before),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.published.problems = report.as_deref().map(diagnostics_text).unwrap_or_default();
            self.written_from.clone_from(report);
        }
    }
}

/// What the thread of the session receives.
pub enum Message {
    /// A folder to open, and where to send what `view.open` returns.
    Open {
        folder: PathBuf,
        reply: Option<Sender<Result<Value, CommandError>>>,
    },
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
    pub fn spawn(standard_everywhere: bool, lib_paths: Vec<PathBuf>, style: Option<PathBuf>, wake: Box<dyn Fn() + Send>) -> std::io::Result<Handle> {
        let (sender, receiver) = mpsc::channel();
        let mut session = Session::new(standard_everywhere, lib_paths, Some(sender.clone()));
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

    /// Opens a folder; what `view.open` returns comes back on the receiver.
    pub fn open(&self, folder: &Path) -> Receiver<Result<Value, CommandError>> {
        let (reply, answer) = mpsc::channel();
        let _ = self.sender.send(Message::Open { folder: folder.to_path_buf(), reply: Some(reply) });
        answer
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

/// A bug of the viewer, out of a command (`engine` catches those of its commands), must not end the session.
fn caught(work: impl FnOnce() -> Result<Value, CommandError>) -> Result<Value, CommandError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
        .unwrap_or_else(|_| Err(CommandError::internal(format!("internal error, which is a bug of the viewer: {}", labcraft_cli::fmt::last_panic()))))
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
            Message::Open { folder, reply } => {
                let result = caught(|| session.open(&folder));
                // What it changed is published before it answers.
                *published.lock().unwrap_or_else(PoisonError::into_inner) = session.published().clone();
                wake();
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
            }
            Message::Command { id, params, reply } => {
                let result = caught(|| session.execute(&id, &params));
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
