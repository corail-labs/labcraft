//! The session (`docs/architecture.md`, 10.8): the root of the project, its libraries, the native libraries the app
//! gave it, and its kept checks. [`Session::execute`] is the only way in: an id and JSON parameters, a JSON result or a
//! [`CommandError`]. A panic in a command is caught there and returned as `internal`, and the session goes on.
//!
//! It takes the stack that `format::read` does: the app runs it on a thread of `format::STACK_SIZE`.

use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use labcraft_runtime::native::NativeLibrary;
use labcraft_types::Limits;
use serde_json::{Value, json};

use crate::error::{CommandError, did_you_mean};
use crate::folders::{Folders, check_folders_keeping};
use crate::history::Author;
use crate::kept::Kept;
use crate::panics::guarded;
use crate::params::{self, Params};
use crate::registry::{self, Command};
use crate::runs::{Clock, Patient, Runs, Waiter};
use crate::shipped::Shipped;
use crate::workspace::Workspace;
use crate::{catalog, files, functions, project, simulations};

/// At most this many folders keep their check: the root, and those that `library.check` was last asked for.
const KEPT_FOLDERS: usize = 8;

/// What an app gives a session.
pub struct Config {
    /// The root of the project: every path of a command is relative to it.
    pub root: PathBuf,
    /// The folders of `--lib-path`, where libraries are looked for before those shipped (`docs/cli.md`, section 3).
    pub lib_paths: Vec<PathBuf>,
    pub limits: Limits,
    pub shipped: Shipped,
    /// The native libraries the app links (`labcraft-visa`, `labcraft-results-log`).
    pub natives: Vec<Arc<dyn NativeLibrary>>,
    /// The real clock, which a run reads through `Control` (`docs/architecture.md`, 10.10): `runs::Still` in tests.
    pub clock: Arc<dyn Clock>,
    /// The id of the process, which names the `.partial` of a trace (`docs/commands.md`, section 8): 0 where there is none.
    pub process: u32,
}

pub struct Session {
    pub(crate) ws: Arc<dyn Workspace>,
    /// The root, without links.
    pub(crate) root: PathBuf,
    pub(crate) config: Config,
    /// The kept checks, by folder without links, the last used at the end.
    kept: Vec<(PathBuf, Kept)>,
    /// Its runs, shared with whoever waits for them.
    runs: Runs,
    /// The state of the viewer, which the `view.*` commands change (`view`).
    pub(crate) view: crate::view::View,
    /// The first check of the root, made in the background when the session starts.
    #[cfg(not(target_arch = "wasm32"))]
    first: Option<std::thread::JoinHandle<Option<Kept>>>,
    /// The history of the project (commands.md 9.1).
    pub(crate) history: crate::history::History,
    /// Whether it keeps one: `labcraft-cli` runs one command per process, with none (9.5).
    pub(crate) keeps_history: bool,
    /// Whether a window of the IDE shows it (`with_window`).
    pub(crate) has_window: bool,
    /// The bytes of its last write to each file, which tell an external write (architecture 12.5).
    pub(crate) known: crate::external::Known,
    /// The journal of its commands (architecture 12.5).
    journal: crate::journal::Journal,
    /// The files that the command going wrote, with their revisions, for its line of the journal.
    pub(crate) wrote_now: Vec<(String, Option<String>)>,
}

impl Session {
    /// A session on the root of `config`, which must be a folder. Unless `background` is false, the root is checked on
    /// a thread of its own as the session starts, so that the first command that reports a check finds it done.
    pub fn new(ws: Arc<dyn Workspace>, config: Config, background: bool) -> Result<Session, CommandError> {
        let root = ws.canonicalize(&config.root).map_err(|error| {
            CommandError::new("path.not_found", format!("the root of the project cannot be read: {}", crate::walk::io_text(&error)))
        })?;
        if ws.metadata(&root).map(|metadata| metadata.kind).ok() != Some(crate::workspace::Kind::Folder) {
            return Err(CommandError::new("path.invalid", "the root of the project is not a folder"));
        }
        let runs = Runs::default();
        let journal = crate::journal::Journal::new(config.process, runs.session_number());
        let mut session = Session {
            ws,
            root,
            config,
            kept: Vec::new(),
            runs,
            view: crate::view::View::default(),
            #[cfg(not(target_arch = "wasm32"))]
            first: None,
            history: crate::history::History::default(),
            keeps_history: true,
            has_window: false,
            known: crate::external::Known::default(),
            journal,
            wrote_now: Vec::new(),
        };
        if background {
            session.start_first_check();
        }
        Ok(session)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start_first_check(&mut self) {
        let ws = Arc::clone(&self.ws);
        let root = self.root.clone();
        let lib_paths = self.config.lib_paths.clone();
        let limits = self.config.limits.clone();
        let shipped = self.config.shipped.clone();
        let work = move || {
            let checking = || Kept::take(check_of(&*ws, &shipped, &root, &root, &lib_paths, &limits));
            panic::catch_unwind(AssertUnwindSafe(checking)).ok()
        };
        self.first = std::thread::Builder::new().name("first check".to_owned()).stack_size(labcraft_format::STACK_SIZE).spawn(work).ok();
    }

    /// Without threads, the first command that needs the check makes it.
    #[cfg(target_arch = "wasm32")]
    fn start_first_check(&mut self) {}

    /// Whether the first check of the root is still going.
    pub(crate) fn first_check_pending(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.first.as_ref().is_some_and(|first| !first.is_finished())
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }

    /// Waits for the first check, and keeps it.
    fn join_first_check(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(first) = self.first.take()
            && let Ok(Some(kept)) = first.join()
        {
            self.keep(self.root.clone(), kept);
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn workspace(&self) -> &dyn Workspace {
        &*self.ws
    }

    /// The workspace, shared: for [`Runs::execute`] on a thread that does not hold the session.
    pub fn shared_workspace(&self) -> Arc<dyn Workspace> {
        Arc::clone(&self.ws)
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The runs of the session, which `run.wait`, `run.status`, `run.abort` and `run.trace` reach without it
    /// ([`Runs::execute`]), so that a caller that waits for a run never holds the session.
    pub fn runs(&self) -> Runs {
        self.runs.clone()
    }

    /// What the viewer shows, as the `view.*` commands last left it (`docs/architecture.md`, 11.5).
    pub fn view(&self) -> &std::sync::Arc<crate::view::Published> {
        self.view.published()
    }

    /// The files of the folder open in the viewer, or of its libraries, changed: the app that watches them says so,
    /// and the folder is read, checked and drawn again (decisions of the owner, 2026-11-11). Not a command: the change
    /// comes from the system, not from a caller.
    /// The files that the session wrote are looked at too: one that another program changed becomes a step signed
    /// `external` (commands.md 9.1). The session looks before each command as well, so that a write made with no app to
    /// watch the files, under `labcraft-cli mcp`, is a step before the next command reads it.
    pub fn files_changed(&mut self) {
        self.notice_external();
        crate::view::files_changed(self);
        self.view.settle();
    }

    /// The external writes since the last look, as a step and a line of the journal (architecture 12.5).
    fn notice_external(&mut self) {
        // A bug there never stops the command: what was seen is kept only once the look is done.
        let looked = panic::catch_unwind(AssertUnwindSafe(|| crate::external::notice(self)));
        if let Ok(Some((_, files))) = looked {
            let entry = crate::journal::Entry {
                time_ns: self.config.clock.utc_ns(),
                author: &Author::External,
                id: None,
                params: &Value::Null,
                outcome: Ok(()),
                files: &files,
            };
            self.journal.write(&self.ws, &self.root, &entry);
        }
    }

    /// The lines of the journal kept in memory, oldest first: in WASM, where the journal has no file (architecture 12.5).
    pub fn journal_in_memory(&self) -> Vec<String> {
        self.journal.in_memory()
    }

    /// The viewer never draws these functions: their drawing trapped a worker of the web viewer before, which a new
    /// session must not do again (`docs/architecture.md`, 11.5). Shown, such a function has no drawing. Not a command:
    /// what traps is the page's to know, not a caller's.
    pub fn skip_drawing(&mut self, functions: std::collections::BTreeSet<String>) {
        self.view.skipped = functions;
    }

    /// The next `view.open` shows this function, when the folder has it, instead of its first one: the function that
    /// the address of the web viewer names, or the one shown before a worker trapped (`docs/architecture.md`, 11.5).
    /// The first function is then neither drawn nor opened; the `view.open` after it shows the first one again. Not a
    /// command, as [`Session::skip_drawing`]: what the page shows is its own.
    pub fn open_on(&mut self, function: Option<String>) {
        self.view.open_on = function;
    }

    /// Calls `hook` with the name of each function just before the viewer draws it: the worker of the web viewer tells
    /// its page, so that a trap while drawing is put down to that function, whichever command drew it (`view.open` draws
    /// the first one, or that of [`Session::open_on`]). Not a command, as [`Session::skip_drawing`].
    pub fn before_drawing(&mut self, hook: Option<fn(&str)>) {
        self.view.before_drawing = hook;
    }

    /// Draws the viewer with these values of the notation (`labcraft --style`, notation 10): a tool to tune the
    /// notation, not a preference, so not a command.
    pub fn set_style(&mut self, style: std::sync::Arc<labcraft_diagram::style::Style>) {
        crate::view::set_style(self, style);
        self.view.settle();
    }

    /// Reads the trace of the function shown in the viewer when its drawing is new or the trace changed: the app calls it
    /// after each command and each second, so that a new run shows (decisions of the owner, 2026-11-13). Whether the
    /// viewer published anything. Not a command: the change comes from the system, as for [`Session::files_changed`].
    pub fn look_at_trace(&mut self) -> bool {
        let published = crate::view::look_at_trace(self);
        self.view.settle();
        published
    }

    /// Shows the values of the trace at `path` over the function shown, as `view.open_trace` does, for a trace that the
    /// window chose anywhere on the disk, which a command cannot name (`docs/commands.md`, 2).
    pub fn open_trace_at(&mut self, path: std::path::PathBuf) -> Result<(), CommandError> {
        let opened = crate::view::open_trace_at(self, path);
        self.view.settle();
        opened
    }

    /// A session that a window of the IDE shows: the commands of the window are enabled (commands.md 6.7, "The window").
    pub fn with_window(&mut self) {
        self.has_window = true;
    }

    /// The zoom of the canvas of the window, which `view.get_state` returns. Not a command: the window says what it
    /// shows, as the app says what changed on the disk.
    pub fn set_zoom(&mut self, zoom: f32) {
        self.view.zoom = Some(zoom);
    }

    /// A session of the command line, which runs one command per process: it keeps no history, so `history.*` is not
    /// enabled and a command makes no step (commands.md 9.5), and its journal is `cli.jsonl` (architecture 12.5).
    pub fn for_command_line(&mut self) {
        self.keeps_history = false;
        self.journal.for_command_line();
    }

    /// Runs a command for the user: the only way in.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value, CommandError> {
        self.execute_waiting(id, params, &Patient)
    }

    /// Runs a command for an author (commands.md 9.5): the history signs its step with it.
    pub fn execute_as(&mut self, author: &Author, id: &str, params: &Value) -> Result<Value, CommandError> {
        self.execute_waiting_as(author, id, params, &Patient)
    }

    /// As [`Session::execute`], with whoever waits for the run in `run.start` and `run.wait`: told where it is each
    /// second, and able to stop waiting (`docs/mcp.md`, 4).
    pub fn execute_waiting(&mut self, id: &str, params: &Value, waiter: &dyn Waiter) -> Result<Value, CommandError> {
        self.execute_waiting_as(&Author::User, id, params, waiter)
    }

    /// As [`Session::execute_waiting`], for an author. The files that the session wrote are looked at first, and an
    /// external write is a step before the command (architecture 12.5); the command is a line of the journal.
    pub fn execute_waiting_as(&mut self, author: &Author, id: &str, params: &Value, waiter: &dyn Waiter) -> Result<Value, CommandError> {
        self.notice_external();
        let lines = self.journal.lines;
        self.wrote_now.clear();
        let outcome = self.run_as(author, id, params, waiter);
        self.view.settle();
        // The command that `commands.execute` runs writes its own line; `commands.execute` writes one only when it failed
        // before running it.
        if id != "commands.execute" || self.journal.lines == lines {
            self.journal_line(author, id, params, outcome.as_ref().map(|_| ()));
        }
        outcome
    }

    fn journal_line(&mut self, author: &Author, id: &str, params: &Value, outcome: Result<(), &CommandError>) {
        let files = std::mem::take(&mut self.wrote_now);
        let entry = crate::journal::Entry { time_ns: self.config.clock.utc_ns(), author, id: Some(id), params, outcome, files: &files };
        self.journal.write(&self.ws, &self.root, &entry);
    }

    fn run_as(&mut self, author: &Author, id: &str, params: &Value, waiter: &dyn Waiter) -> Result<Value, CommandError> {
        let Some(command) = registry::command(id) else {
            let ids: Vec<&str> = registry::commands().iter().map(|command| command.id).collect();
            let near: Vec<&str> = labcraft_runtime::simulation::nearest(id, &ids).into_iter().collect();
            return Err(CommandError::new("command.unknown", format!("no command has the id `{id}`{}", did_you_mean(id, &ids))).expecting(&near));
        };
        if let Some(reason) = self.not_enabled(command) {
            return Err(CommandError::new("command.not_enabled", format!("`{id}` is not enabled: {reason}")));
        }
        let params = params::judge(command, params)?;
        guarded(id, || self.dispatch(author, command, &params, waiter)).unwrap_or_else(|internal| {
            // What the command was changing may be half done: nothing kept is trusted after it.
            self.kept.clear();
            Err(internal)
        })
    }

    /// `run.start` from the command line (`docs/cli.md`, section 6; decisions of the owner, 2026-11-11): the parameters of
    /// the command, judged as `execute` judges them, and what `labcraft-cli run` gives beyond them, its values as text and
    /// its files outside the project.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn start_run(&mut self, params: &Value, line: crate::runs::CommandLine) -> Result<Value, CommandError> {
        let Some(command) = registry::command("run.start") else { return Err(CommandError::internal("`run.start` is not registered")) };
        // Not a line of the journal: a run whose trace goes to `--trace FILE` writes nothing under the root (commands.md 8).
        let params = params::judge(command, params)?;
        let runs = self.runs.clone();
        let session = &*self;
        guarded("run.start", || runs.start_with(session, &params, line, &Patient)).unwrap_or_else(Err)
    }

    /// Why a command cannot run now, if it cannot.
    pub fn not_enabled(&self, command: &Command) -> Option<String> {
        if let Some(planned) = command.planned {
            return Some(format!("it is planned, and comes with {planned}"));
        }
        if command.id.starts_with("run.") {
            return crate::runs::not_enabled_here().map(str::to_owned);
        }
        if crate::view::window::WINDOW_ONLY.contains(&command.id) && !self.has_window {
            return Some("it is a command of the window of the IDE, and the command line and the MCP server alone have none".to_owned());
        }
        if crate::view::window::RUN_BY_THE_WINDOW.contains(&command.id) {
            return Some("the window of the IDE runs it itself".to_owned());
        }
        if command.id.starts_with("history.") && !self.keeps_history {
            return Some("the command line runs one command per process, with no history to undo".to_owned());
        }
        None
    }

    fn dispatch(&mut self, author: &Author, command: &Command, params: &Params, waiter: &dyn Waiter) -> Result<Value, CommandError> {
        match command.id {
            id if crate::edits::EDITING.contains(&id) => crate::edits::edit(self, author, command, params),
            "commands.batch" => crate::edits::batch(self, author, params),
            "commands.execute" => crate::edits::execute(self, author, params),
            "history.undo" => crate::edits::undo(self, author),
            "history.redo" => crate::edits::redo(self, author),
            "history.list" => Ok(crate::edits::list(self, params)),
            "guide.read" => crate::guide::read(params),
            "commands.list" => Ok(catalog::list(self, params)),
            "commands.describe" => catalog::describe(self, params),
            "library.check" => files::library_check(self, params),
            "file.format" => files::format(self, author, command, params),
            "file.read" => files::read(self, params),
            "file.write" => files::write(self, author, command, params),
            "file.rename" => files::rename(self, author, command, params),
            "file.delete" => files::delete(self, author, command, params),
            "project.describe" => project::describe(self),
            "functions.list" => functions::list(self, params),
            "simulation.describe" => simulations::describe(self, params),
            "diagram.svg" => crate::drawings::svg_command(self, params),
            "view.open" => crate::view::open(self, params),
            "view.show_function" => crate::view::show_function(self, params),
            "view.set_standard_view" => crate::view::set_standard_view(self, params),
            "view.set_state" => crate::view::set_state(self, params),
            "view.open_trace" => crate::view::open_trace(self, params),
            "view.close_function" => crate::view::window::close_function(self, params),
            "view.set_view" => crate::view::window::set_view(self, params),
            "view.set_workspace" => crate::view::window::set_workspace(params),
            "view.select" => crate::view::window::select(self, params),
            "view.get_state" => Ok(crate::view::window::get_state(self)),
            #[cfg(not(target_arch = "wasm32"))]
            "run.start" => self.runs.start(self, params, waiter),
            "run.wait" | "run.status" | "run.abort" | "run.set_element" | "run.trace" => {
                let max_nesting = self.config.limits.max_nesting();
                self.runs.dispatch(&*self.ws, &self.root, command.id, params, max_nesting, waiter)
            }
            other => Err(CommandError::internal(format!("`{other}` is registered and enabled, with no code behind it"))),
        }
    }

    /// A command changed files: nothing kept is given again before it is checked. What it saw would tell the change too;
    /// this costs nothing and does not lean on it. The viewer reads its folder again at its next command.
    pub(crate) fn forget(&mut self) {
        self.kept.clear();
        self.view.stale = true;
    }

    fn keep(&mut self, folder: PathBuf, kept: Kept) {
        self.kept.retain(|(other, _)| *other != folder);
        if self.kept.len() >= KEPT_FOLDERS {
            self.kept.remove(0);
        }
        self.kept.push((folder, kept));
    }

    /// The check of the root as last kept, without looking whether it still holds: right after [`Session::checked`].
    pub(crate) fn kept_root(&self) -> Option<&Kept> {
        self.kept.iter().rev().find(|(folder, _)| *folder == self.root).map(|(_, kept)| kept)
    }

    /// The check of a folder without links, kept if it still holds, made again otherwise.
    pub(crate) fn checked(&mut self, folder: &Path) -> Result<&Kept, CommandError> {
        if folder == self.root {
            self.join_first_check();
        }
        let (ws, lib_paths, limits) = (&*self.ws, &self.config.lib_paths, &self.config.limits);
        let root = &self.root;
        let held = self.kept.iter().position(|(kept_folder, kept)| kept_folder == folder && kept.holds(ws, folder, lib_paths, Some(root), limits));
        let kept = match held {
            Some(index) => self.kept.remove(index).1,
            None => Kept::take(check_of(ws, &self.config.shipped, root, folder, lib_paths, limits)),
        };
        self.keep(folder.to_path_buf(), kept);
        // `keep` leaves it last.
        self.kept.last().map(|(_, kept)| kept).ok_or_else(|| CommandError::internal("a check kept is not there"))
    }

    /// The report of a check of the folder, as `labcraft-cli check --json` gives it.
    pub(crate) fn check_document(&mut self, folder: &Path, deny_warnings: bool) -> Result<Value, CommandError> {
        let kept = self.checked(folder)?;
        let document = kept.checked.report.check_document(kept.checked.counts, deny_warnings);
        serde_json::to_value(document).map_err(|error| CommandError::internal(format!("the report does not write as JSON: {error}")))
    }

    /// The report of a check of the root.
    pub(crate) fn root_check_document(&mut self) -> Result<Value, CommandError> {
        let root = self.root.clone();
        self.check_document(&root, false)
    }
}

/// A run never outlives its session (commands.md 4): the run going is aborted when the session goes away.
impl Drop for Session {
    fn drop(&mut self) {
        self.runs.abort_going();
    }
}

/// A check of one folder of the project, its paths shown from the root, what `check` found kept for the root.
pub(crate) fn check_of(
    ws: &dyn Workspace,
    shipped: &Shipped,
    root: &Path,
    folder: &Path,
    lib_paths: &[PathBuf],
    limits: &Limits,
) -> crate::folders::FoldersChecked {
    let folders = [folder.to_path_buf()];
    let given = Folders { folders: &folders, lib_paths, current: Some(root.to_path_buf()), limits };
    let mut checked = check_folders_keeping(ws, shipped, &given, folder == root, true);
    checked.report.finish();
    checked
}

/// `{ "written": … }`, when a command that writes does not check.
pub(crate) fn written(shown: &str) -> Value {
    json!({ "written": shown })
}
