//! What the page knows of its worker (`docs/architecture.md`, 12.9): the files it gave it, kept with the changes that
//! the worker says, so that a trap of WASM restarts it with a new session and the same files; the commands waiting,
//! one in flight, and where each answer goes; the function the worker said it was drawing; and what the session
//! published, made again as the `Published` that the window reads. [`Shared`] is the window's [`Host`] on it. Plain
//! Rust, tested natively: the page only starts workers and carries the bytes.
//!
//! - A trap after the worker named a function it was drawing: that function is not drawn again, and shows `LC0099` with
//!   its last drawing, whichever command drew it (`view.open` draws the function asked for, else the first one).
//! - Any other trap (reading or checking the folder): the worker restarts once for that set of files; a second trap
//!   shows `LC0099` for the folder until another folder is opened. A file that traps cannot restart the worker forever.
//! - The history goes with the worker that trapped; the files stay (12.9).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::Sender;

use labcraft_diagram::Drawing;
use labcraft_engine::CommandError;
use labcraft_engine::view::{self, Published, Selection, Shown, Tab, ViewKind, WorkspaceId};
use labcraft_ide::{Host, Reply, Snapshot};
use labcraft_types::{Code, Diagnostic, Position, Report, Span};
use serde_json::Value;

use crate::protocol::{Folder, FromWorker, Listing, Seen, SeenWindow, ToWorker, decode, encode};

/// What the page must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// End the worker, if one runs, and start a new one; nothing is sent to it until it says that it listens
    /// ([`FromWorker::Ready`]): a message that comes before is lost.
    Start,
    /// Send it this message.
    Send(Vec<u8>),
}

/// A message the page waits to send.
#[derive(Clone, Debug, PartialEq)]
enum Request {
    Open,
    Command { request: u64, id: String, params: Value },
    Zoom(String, f32),
    UserSettings(String),
    Trap,
}

/// What the page waits for from its worker.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Waiting {
    #[default]
    Nothing,
    /// A worker started: that it listens.
    Ready,
    /// The answer to the message in flight, and the command it is, if it is one.
    Answer(Option<u64>),
}

/// The traps of the folder open that no drawing explains.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Traps {
    #[default]
    None,
    /// The worker restarted once for these files.
    Once,
    /// Twice: `LC0099` for the folder, until another one is opened.
    Twice,
    /// A worker that never said it listens: its WASM could not be loaded or started. The same files are not tried again.
    NoWorker,
}

/// What counts up across the folders opened, so that the window always sees a change as a change.
#[derive(Clone, Copy, Debug, Default)]
struct Counts {
    generation: u64,
    checks: u64,
    drawing_generation: u64,
    request: u64,
    /// The folders opened: each one is a new session for the window, even under the same name.
    opened: u64,
}

#[derive(Default)]
pub struct Supervisor {
    folder: Option<Arc<Folder>>,
    /// The keys of the store of the user, as a JSON object (commands.md 9.7): what a worker opens with, and what the
    /// session holds once it said, so that a worker that starts again takes them.
    user_settings: String,
    pending: VecDeque<Request>,
    waiting: Waiting,
    /// Where the answer of each command goes.
    replies: HashMap<u64, Sender<Result<Value, CommandError>>>,
    /// The function the worker said it was drawing, since the message in flight was sent.
    drawing: Option<String>,
    traps: Traps,
    /// The functions whose drawing trapped a worker.
    trapped: BTreeSet<String>,
    seen: Seen,
    listing: Listing,
    report: Option<Arc<Report>>,
    /// The last drawing of each function, which a function whose drawing trapped keeps.
    drawings: BTreeMap<String, Arc<Drawing>>,
    /// The drawing published last for each function, and its count.
    published_drawings: HashMap<String, (Arc<Drawing>, u64)>,
    /// What went wrong between the page and its worker.
    failure: Option<String>,
    /// The function that [`Supervisor::open`] was asked to show, until the worker answers.
    asked: Option<String>,
    /// The function asked that the folder lacks, and the one shown instead.
    missing: Option<(String, Option<String>)>,
    counts: Counts,
    published: Arc<Published>,
    /// The session wrote, renamed or removed a file since the folder was opened.
    edited: bool,
}

impl Supervisor {
    /// `user_settings`: the keys of the store of the user, as a JSON object.
    pub fn new(user_settings: String) -> Supervisor {
        Supervisor { user_settings, ..Supervisor::default() }
    }

    /// The keys of the store of the user, for a session that does not hold them (`Host::set_user_settings`).
    pub fn user_settings(&mut self, keys: &serde_json::Map<String, Value>) -> Vec<Action> {
        self.user_settings = Value::Object(keys.clone()).to_string();
        if self.refused().is_some() {
            return Vec::new();
        }
        self.pending.retain(|request| !matches!(request, Request::UserSettings(_)));
        self.pending.push_back(Request::UserSettings(self.user_settings.clone()));
        self.next()
    }

    /// A folder read by the page: in a new worker, in place of the folder open; the traps of the files before are
    /// forgotten, and what waited for the folder before fails.
    /// It shows `function` when the folder has it (what the address of the page asks for), else its first function, and
    /// then says that the folder has none of that name, until another function is shown.
    pub fn open(&mut self, folder: Folder, function: Option<String>) -> Vec<Action> {
        self.fail_all("another folder was opened");
        let mut counts = self.counts;
        counts.generation += 1;
        counts.opened += 1;
        *self = Supervisor { folder: Some(Arc::new(folder)), asked: function, counts, ..Supervisor::new(std::mem::take(&mut self.user_settings)) };
        self.publish();
        self.pending.push_back(Request::Open);
        self.start()
    }

    /// A command of the session, sent when the one in flight has its answer; its result goes to `reply`.
    pub fn command(&mut self, id: &str, params: Value, reply: Sender<Result<Value, CommandError>>) -> Vec<Action> {
        if let Some(why) = self.refused() {
            let _ = reply.send(Err(CommandError::new("command.not_enabled", why)));
            return Vec::new();
        }
        self.counts.request += 1;
        let request = self.counts.request;
        self.replies.insert(request, reply);
        self.pending.push_back(Request::Command { request, id: id.to_owned(), params });
        self.next()
    }

    /// The zoom of the canvas of the window that shows `function`: only the last one of each waits.
    pub fn zoom(&mut self, function: &str, zoom: f32) -> Vec<Action> {
        if self.refused().is_some() {
            return Vec::new();
        }
        match self.pending.iter_mut().find(|request| matches!(request, Request::Zoom(of, _) if of == function)) {
            Some(waiting) => *waiting = Request::Zoom(function.to_owned(), zoom),
            None => self.pending.push_back(Request::Zoom(function.to_owned(), zoom)),
        }
        self.next()
    }

    /// Traps the worker, for the smoke test of the page.
    pub fn trap(&mut self) -> Vec<Action> {
        if self.folder.is_none() {
            return Vec::new();
        }
        self.pending.push_back(Request::Trap);
        self.next()
    }

    /// Why a command cannot go to a worker now.
    fn refused(&self) -> Option<String> {
        match self.traps {
            _ if self.folder.is_none() => Some("open a project first".to_owned()),
            Traps::Twice => Some("the session of this project stopped twice, which is a bug of LabCraft (LC0099)".to_owned()),
            Traps::NoWorker => Some("the worker of the session could not start".to_owned()),
            Traps::None | Traps::Once => None,
        }
    }

    /// A message of the worker.
    pub fn received(&mut self, bytes: &[u8]) -> Vec<Action> {
        match decode::<FromWorker>(bytes) {
            Ok(FromWorker::Ready) => self.waiting = Waiting::Nothing,
            Ok(FromWorker::Drawing(function)) => {
                self.drawing = Some(function);
                return Vec::new();
            }
            Ok(FromWorker::Files(changed)) => {
                self.take_files(changed);
                return Vec::new();
            }
            Ok(FromWorker::Answer(answer)) => {
                let answer = *answer;
                // The command in flight has its answer; one that comes without it (a message the worker could not read)
                // still ends it.
                if let Waiting::Answer(Some(request)) = self.waiting
                    && answer.request != Some(request)
                    && let Some(reply) = self.replies.remove(&request)
                {
                    let _ = reply.send(Err(CommandError::internal("the worker of the session did not take this command")));
                }
                self.waiting = Waiting::Nothing;
                self.drawing = None;
                // A message that is no command and failed (a folder that does not open): the page says it, and what the
                // session showed stays.
                if answer.request.is_none()
                    && let Err(failure) = &answer.result
                {
                    self.failure = Some(format!("{}: {}", failure.code, failure.message));
                    if answer.seen.folder.is_none() {
                        return self.next();
                    }
                }
                // A failure of the page is said until a message succeeds.
                if answer.result.is_ok() {
                    self.failure = None;
                }
                if let Some(reply) = answer.request.and_then(|request| self.replies.remove(&request)) {
                    let result = match answer.result {
                        Ok(json) => serde_json::from_str(&json).map_err(|error| CommandError::internal(format!("an answer of the worker: {error}"))),
                        Err(failure) => Err(CommandError::new(code_of(&failure.code), failure.message)),
                    };
                    let _ = reply.send(result);
                }
                if let Some(changed) = answer.files {
                    self.take_files(changed);
                }

                // A change of what the session published, not each answer: the window reads the history again at each
                // one, which a read does not change.
                let changed = answer.seen != self.seen || answer.listing.is_some() || !answer.drawings.is_empty();
                // A check of the session, as `engine` counts them: a drawing gives a new report too (`LC7005`), not a check.
                if answer.seen.checks != self.seen.checks {
                    self.counts.checks += 1;
                }
                if let Some(listing) = answer.listing {
                    self.report = Some(Arc::new(report_of(&listing)));
                    self.listing = listing;
                }
                for (function, drawing) in answer.drawings {
                    self.drawings.insert(function, drawing);
                }
                // The first answer, to the folder opened: the function asked is shown, or the folder lacks it.
                if let Some(asked) = self.asked.take()
                    && self.folder.is_some()
                    && answer.seen.main.function.as_ref() != Some(&asked)
                    && answer.seen.folder.is_some()
                {
                    self.missing = Some((asked, answer.seen.main.function.clone()));
                }
                // Said until another function is shown.
                if self.missing.as_ref().is_some_and(|(_, instead)| *instead != answer.seen.main.function) {
                    self.missing = None;
                }
                // The settings of the user are the session's once it holds them: a worker that starts again takes them.
                if let Ok(settings) = serde_json::from_str::<Value>(&answer.seen.settings)
                    && settings.get("given") == Some(&Value::Bool(true))
                    && let Some(user) = settings.get("user")
                {
                    self.user_settings = user.to_string();
                }
                self.seen = answer.seen;
                if changed {
                    self.counts.generation += 1;
                    self.publish();
                }
            }
            Err(error) => {
                self.waiting = Waiting::Nothing;
                self.failure = Some(error);
            }
        }
        self.next()
    }

    /// Files the session changed, in the copy that a new worker opens.
    fn take_files(&mut self, changed: crate::protocol::Changed) {
        let Some(folder) = &mut self.folder else { return };
        Arc::make_mut(folder).changed(changed);
        self.edited = true;
        // Other files: a trap before them is not counted against them.
        if self.traps == Traps::Once {
            self.traps = Traps::None;
        }
    }

    /// The worker trapped: a function it said it was drawing is not drawn again; otherwise the folder is to blame.
    pub fn trapped(&mut self) -> Vec<Action> {
        if self.waiting == Waiting::Ready {
            return self.not_started("it stopped before it listened");
        }
        if let Waiting::Answer(Some(request)) = self.waiting
            && let Some(reply) = self.replies.remove(&request)
        {
            let _ =
                reply.send(Err(CommandError::internal("the session stopped on this command, which is a bug of LabCraft (LC0099): it starts again")));
        }
        if let Some(function) = self.drawing.take() {
            self.trapped.insert(function.clone());
            // The window that showed it shows it again, not drawn; the window of the project, when none did.
            if !self.seen.detached.iter().any(|window| window.function.as_ref() == Some(&function)) {
                self.seen.main.function = Some(function);
            }
        } else if self.traps == Traps::None {
            self.traps = Traps::Once;
        } else {
            self.traps = Traps::Twice;
            self.fail_all("the session of this project stopped twice, which is a bug of LabCraft (LC0099)");
            self.waiting = Waiting::Nothing;
            self.publish();
            return Vec::new();
        }
        // What was in flight went with the worker; the new one opens the same files first, as they are now.
        self.pending.retain(|request| *request != Request::Open);
        self.pending.push_front(Request::Open);
        self.counts.generation += 1;
        self.publish();
        self.start()
    }

    /// The worker could not be started, or stopped before it listened: said, and not tried again for these files.
    pub fn not_started(&mut self, why: &str) -> Vec<Action> {
        self.traps = Traps::NoWorker;
        self.failure = Some(format!("the worker of the session could not start: {why}"));
        self.fail_all("the worker of the session could not start");
        self.waiting = Waiting::Nothing;
        Vec::new()
    }

    /// Every command waiting fails, with `why`.
    fn fail_all(&mut self, why: &str) {
        self.pending.clear();
        for (_, reply) in self.replies.drain() {
            let _ = reply.send(Err(CommandError::new("command.not_enabled", why)));
        }
    }

    fn start(&mut self) -> Vec<Action> {
        self.waiting = Waiting::Ready;
        self.drawing = None;
        vec![Action::Start]
    }

    /// Sends what waits, if nothing is in flight.
    fn next(&mut self) -> Vec<Action> {
        if self.waiting != Waiting::Nothing || matches!(self.traps, Traps::Twice | Traps::NoWorker) {
            return Vec::new();
        }
        let Some(request) = self.pending.pop_front() else { return Vec::new() };
        let (message, command) = match request {
            Request::Open => match &self.folder {
                Some(folder) => (
                    ToWorker::Open {
                        folder: Arc::clone(folder),
                        user_settings: self.user_settings.clone(),
                        skipped: self.trapped.iter().cloned().collect(),
                        show: self.seen.main.function.clone().or_else(|| self.asked.clone()),
                        detached: self.seen.detached.iter().filter_map(|window| window.function.clone()).collect(),
                    },
                    None,
                ),
                None => return Vec::new(),
            },
            Request::Command { request, id, params } => (ToWorker::Command { request, id, params: params.to_string() }, Some(request)),
            Request::Zoom(function, zoom) => (ToWorker::Zoom(function, zoom), None),
            Request::UserSettings(keys) => (ToWorker::UserSettings(keys), None),
            Request::Trap => (ToWorker::Trap, None),
        };
        match encode(&message) {
            Ok(bytes) => {
                self.waiting = Waiting::Answer(command);
                vec![Action::Send(bytes)]
            }
            Err(error) => {
                if let Some(reply) = command.and_then(|request| self.replies.remove(&request)) {
                    let _ = reply.send(Err(CommandError::internal(error)));
                } else {
                    self.failure = Some(error);
                }
                self.next()
            }
        }
    }

    /// The name of the folder open, as the page read it.
    pub fn folder_name(&self) -> Option<&str> {
        self.folder.as_deref().map(|folder| folder.name.as_str())
    }

    /// The files of the folder open, as they are now.
    pub fn folder(&self) -> Option<&Folder> {
        self.folder.as_deref()
    }

    /// Whether the files differ from those opened: the session wrote, renamed or removed one. A trap loses the history,
    /// not this.
    pub fn edited(&self) -> bool {
        self.edited
    }

    /// A worker starts, or a message is in flight.
    pub fn busy(&self) -> bool {
        self.waiting != Waiting::Nothing
    }

    /// What the session published, made again as the window reads it.
    fn publish(&mut self) {
        // What the session said, aside while each window is made again from it, which counts its drawings.
        let seen = std::mem::take(&mut self.seen);
        let main = self.shown(&seen.main);
        let detached = seen.detached.iter().map(|window| self.shown(window)).collect();
        self.seen = seen;
        let seen = &self.seen;
        let tabs = seen
            .tabs
            .iter()
            .filter_map(|(function, first, split)| {
                let tab = Tab { view: ViewKind::parse(first)?, split: split.as_deref().and_then(ViewKind::parse) };
                Some((function.clone(), tab))
            })
            .collect();
        let workspace = view::window::WORKSPACES.iter().find(|(id, _)| *id == seen.workspace).map(|(id, _)| WorkspaceId(id)).unwrap_or_default();
        let functions = self
            .listing
            .functions
            .iter()
            .map(|listed| view::Listed {
                function: listed.function.clone(),
                file: listed.file.clone(),
                drawable: listed.drawable,
                errors: listed.errors,
                warnings: listed.warnings,
                // The page runs nothing (D-22): what a run needs is never asked there.
                needs: Vec::new(),
            })
            .collect();
        self.published = Arc::new(Published {
            generation: self.counts.generation,
            folder: seen.folder.clone(),
            functions,
            types: self.listing.types.iter().map(|(name, file)| view::ListedType { name: name.clone(), file: file.clone() }).collect(),
            report: self.report.clone(),
            checks: self.counts.checks,
            errors: self.listing.errors,
            warnings: self.listing.warnings,
            main,
            detached,
            opened: seen.opened.clone(),
            standard_everywhere: seen.standard_everywhere,
            tabs,
            settings: Arc::new(stores_of(&seen.settings)),
            preview: seen.preview.clone(),
            workspace,
        });
    }

    /// What a window shows, made again with the drawing of its function: a function whose drawing trapped keeps its
    /// last one. A new drawing of a function counts once.
    fn shown(&mut self, window: &SeenWindow) -> Shown {
        let drawing = window.function.as_ref().and_then(|function| self.drawings.get(function)).cloned();
        let trapped = window.function.as_ref().is_some_and(|function| self.trapped.contains(function));
        let mut drawing_generation = 0;
        if let (Some(function), Some(drawing)) = (&window.function, &drawing) {
            let published = self.published_drawings.get(function).filter(|(before, _)| Arc::ptr_eq(before, drawing)).map(|(_, count)| *count);
            drawing_generation = match published {
                Some(count) => count,
                None => {
                    self.counts.drawing_generation += 1;
                    self.published_drawings.insert(function.clone(), (drawing.clone(), self.counts.drawing_generation));
                    self.counts.drawing_generation
                }
            };
        }
        let (selected, nodes, wires, terminals) = window.selection.clone();
        Shown {
            function: window.function.clone(),
            kept: window.kept || (trapped && drawing.is_some()),
            drawing,
            drawing_generation,
            glide: window.glide,
            arms: window.arms.iter().cloned().collect(),
            selection: Selection { function: selected, nodes, wires, terminals },
            ..Shown::default()
        }
    }

    /// What the window must say: the traps (`LC0099`), a function asked for that the folder lacks, a failure of the page.
    pub fn notices(&self) -> Vec<String> {
        let mut notices = Vec::new();
        if self.traps == Traps::Twice {
            notices.push("LC0099: this project stopped its session twice, which is a bug of LabCraft: open it again".to_owned());
        }
        let functions = self.seen.main.function.iter().chain(self.seen.detached.iter().filter_map(|window| window.function.as_ref()));
        for function in functions.filter(|function| self.trapped.contains(*function)) {
            let kept = if self.drawings.contains_key(function) { "; its last drawing stays" } else { "" };
            notices.push(format!("LC0099: drawing `{function}` stopped the session, which is a bug of LabCraft: it is not drawn again{kept}"));
        }
        if let Some((asked, instead)) = &self.missing {
            let folder = self.folder_name().unwrap_or_default();
            let instead = instead.as_ref().map(|function| format!(": here is `{function}`")).unwrap_or_default();
            notices.push(format!("`{folder}` has no function `{asked}`{instead}"));
        }
        notices.extend(self.failure.clone());
        notices
    }

    /// What the window reads.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            view: self.published.clone(),
            project: self.folder_name().map(str::to_owned),
            notices: self.notices(),
            session: self.counts.opened,
        }
    }
}

/// The code of an error of a command, as `engine` names it: one of those its commands say, else `internal`.
fn code_of(code: &str) -> &'static str {
    labcraft_engine::registry::commands()
        .iter()
        .flat_map(|command| command.errors.iter())
        .chain(["internal", "command.unknown", "command.not_enabled", "agent.not_allowed"].iter())
        .find(|known| **known == code)
        .copied()
        .unwrap_or("internal")
}

/// The report of a listing, as `Problems` reads it.
fn report_of(listing: &Listing) -> Report {
    let mut report = Report::new(listing.problems.len().max(1));
    for problem in &listing.problems {
        let Some(code) = Code::ALL.iter().find(|code| code.as_str() == problem.code) else { continue };
        let mut diagnostic = Diagnostic::new(*code, problem.message.clone(), problem.file.clone());
        if let Some([line, column, end_line, end_column]) = problem.span {
            diagnostic = diagnostic.with_span(Span { start: Position { line, column }, end: Position { line: end_line, column: end_column } });
        }
        diagnostic.function.clone_from(&problem.function);
        report.push(diagnostic);
    }
    report
}

/// The window's hold on the supervisor: a [`Host`] whose messages wait in [`Shared::take_actions`] for the page to
/// carry them.
#[derive(Default)]
pub struct Shared {
    pub supervisor: RefCell<Supervisor>,
    actions: RefCell<Vec<Action>>,
}

impl Shared {
    pub fn new(supervisor: Supervisor) -> Shared {
        Shared { supervisor: RefCell::new(supervisor), actions: RefCell::default() }
    }

    /// Does `work` on the supervisor; what it asks of the page waits in [`Shared::take_actions`].
    pub fn with(&self, work: impl FnOnce(&mut Supervisor) -> Vec<Action>) {
        let actions = work(&mut self.supervisor.borrow_mut());
        self.actions.borrow_mut().extend(actions);
    }

    /// What the page must do now.
    pub fn take_actions(&self) -> Vec<Action> {
        std::mem::take(&mut *self.actions.borrow_mut())
    }
}

impl Host for Shared {
    fn send(&self, id: &str, params: Value) -> Reply {
        let (sender, reply) = Reply::channel();
        self.with(|supervisor| supervisor.command(id, params, sender));
        reply
    }

    fn snapshot(&self) -> Snapshot {
        self.supervisor.borrow().snapshot()
    }

    fn set_zoom(&self, function: &str, zoom: f32) {
        self.with(|supervisor| supervisor.zoom(function, zoom));
    }

    fn set_user_settings(&self, keys: &serde_json::Map<String, Value>) {
        self.with(|supervisor| supervisor.user_settings(keys));
    }
}

/// The settings that `Seen` carries, as JSON; what does not read is none, not given.
fn stores_of(json: &str) -> labcraft_engine::settings::Stores {
    let Ok(Value::Object(mut settings)) = serde_json::from_str::<Value>(json) else { return Default::default() };
    let mut map = |name: &str| match settings.remove(name) {
        Some(Value::Object(keys)) => keys,
        _ => serde_json::Map::new(),
    };
    let (user, project) = (map("user"), map("project"));
    labcraft_engine::settings::Stores { user, project, given: settings.get("given") == Some(&Value::Bool(true)) }
}

#[cfg(test)]
mod tests;
