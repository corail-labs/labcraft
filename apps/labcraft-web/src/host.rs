//! What the worker does with each message of the page (`docs/architecture.md`, 12.9): a session of `engine`, with a
//! window, on the files the page read, in memory, which its commands write; what it publishes, said as [`Seen`]; and the
//! files that each message changed, of which the page keeps a copy. Plain Rust, tested natively and in WASM; the worker
//! only carries the bytes.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use labcraft_diagram::Drawing;
use labcraft_engine::view::{Published, Shown};
use labcraft_engine::{CommandError, Config, Memory, Session, Still};
use labcraft_types::{Limits, Report};
use serde_json::{Value, json};

use crate::protocol::{Answer, Changed, Failure, File, Folder, Listed, Listing, Problem, Seen, SeenWindow, ToWorker};

/// Where the folder given lies in the memory of the worker: every path of a report is shown from it.
const ROOT: &str = "/folder";

/// The journal of the session, in the project (architecture 12.5).
const JOURNAL: &str = ".labcraft/journal";

/// The session of the worker, its files, and what it sent last: the drawing, the report of the listing, the time of
/// change of each file.
#[derive(Default)]
pub struct Host {
    session: Option<Session>,
    memory: Option<Arc<Memory>>,
    before_drawing: Option<fn(&str)>,
    /// Called with the files a message changed, before the session draws again after them.
    on_files: Option<fn(&Changed)>,
    sent: HashMap<String, Arc<Drawing>>,
    listed: Option<Arc<Report>>,
    files: HashMap<PathBuf, u128>,
}

impl Host {
    /// `before_drawing`: called with the name of each function just before it is drawn (`engine`, `before_drawing`).
    pub fn new(before_drawing: Option<fn(&str)>) -> Host {
        Host { before_drawing, ..Host::default() }
    }

    /// `on_files`: called with the files each message changed, before the session draws again after them.
    pub fn with_files(mut self, on_files: fn(&Changed)) -> Host {
        self.on_files = Some(on_files);
        self
    }

    /// Answers a message; [`ToWorker::Trap`] is the worker's to do, and is answered here as an unknown command.
    pub fn handle(&mut self, message: ToWorker) -> Answer {
        let mut request = None;
        let zoom = matches!(message, ToWorker::Zoom(..));
        let result = match message {
            ToWorker::Open { folder, user_settings, skipped, show, detached } => self.open(folder, &user_settings, skipped, show, detached),
            ToWorker::UserSettings(keys) => {
                if let Some(session) = &mut self.session {
                    session.set_user_settings(&keys_of(&keys));
                }
                Ok(Value::Null)
            }
            ToWorker::Command { request: asked, id, params } => {
                request = Some(asked);
                match serde_json::from_str::<Value>(&params) {
                    Ok(params) => self.execute(&id, &params),
                    Err(error) => Err(CommandError::new("params.invalid", format!("the parameters are not JSON: {error}"))),
                }
            }
            ToWorker::Zoom(function, zoom) => {
                if let Some(session) = &mut self.session {
                    session.set_zoom(&function, zoom);
                }
                Ok(Value::Null)
            }
            ToWorker::Trap => Err(CommandError::new("command.unknown", "`Trap` is the worker's own")),
        };
        self.answer(request, result, !zoom)
    }

    fn open(
        &mut self,
        folder: Arc<Folder>,
        user_settings: &str,
        skipped: Vec<String>,
        show: Option<String>,
        detached: Vec<String>,
    ) -> Result<Value, CommandError> {
        // The session before goes first, and the bytes move into the memory of the new one: a folder is held once.
        self.session = None;
        self.memory = None;
        let folder = Arc::try_unwrap(folder).unwrap_or_else(|shared| Folder::clone(&shared));
        let memory = Arc::new(Memory::new());
        memory.put_folder(ROOT);
        memory.set_current(ROOT);
        for path in folder.folders.iter().filter_map(|path| inside(path)) {
            memory.put_folder(path);
        }
        for file in folder.files {
            if let Some(path) = inside(&file.path) {
                memory.put(path, file.bytes);
            }
        }
        let config = Config {
            root: PathBuf::from(ROOT),
            lib_paths: Vec::new(),
            limits: Limits::default(),
            shipped: labcraft_libs::shipped(),
            natives: Vec::new(),
            clock: Arc::new(Still),
            process: 0,
        };
        let mut session = Session::new(memory.clone(), config, false)?;
        session.with_window();
        session.skip_drawing(skipped.into_iter().collect());
        session.before_drawing(self.before_drawing);
        session.set_user_settings(&keys_of(user_settings));
        // The function shown before a restart, or that the address names, when the folder has it: the only one drawn.
        session.open_on(show.clone());
        let opened = session.execute("view.open", &json!({ "folder": "." }));
        // The detached windows again, each with its function, then the function the window of the project showed: a
        // function that the folder lost, or that trapped, is not drawn, and its window says why (`Supervisor::notices`).
        if opened.is_ok() && !detached.is_empty() {
            for function in &detached {
                let _ = session.execute("view.show_function", &json!({ "function": function }));
                let _ = session.execute("view.detach", &json!({ "function": function }));
            }
            if let Some(show) = &show {
                let _ = session.execute("view.show_function", &json!({ "function": show }));
            }
        }
        self.files = files(&memory);
        self.session = Some(session);
        self.memory = Some(memory);
        self.sent.clear();
        self.listed = None;
        opened
    }

    fn execute(&mut self, id: &str, params: &Value) -> Result<Value, CommandError> {
        match &mut self.session {
            Some(session) => session.execute(id, params),
            None => Err(CommandError::new("command.not_enabled", format!("`{id}` waits for a folder: open one first"))),
        }
    }

    /// The answer to a message; `wrote`: whether it may have changed files (a zoom does not).
    fn answer(&mut self, request: Option<u64>, result: Result<Value, CommandError>, wrote: bool) -> Answer {
        let result = result.map(|value| value.to_string()).map_err(|error| Failure { code: error.code.to_owned(), message: error.message });
        let files = if wrote { self.changed() } else { None };
        if let (Some(files), Some(on_files)) = (&files, self.on_files) {
            on_files(files);
        }
        let Some(session) = &mut self.session else {
            return Answer { request, result, seen: Seen::default(), listing: None, drawings: Vec::new(), files };
        };
        // What the watcher of the files does on the desktop: the session reads, checks and draws again.
        if files.is_some() {
            session.files_changed();
        }
        let view = session.view();
        // The functions and the diagnostics go again only with a new report.
        let listing = match &view.report {
            Some(report) if !self.listed.as_ref().is_some_and(|listed| Arc::ptr_eq(listed, report)) => {
                self.listed = Some(report.clone());
                Some(listing(view, report))
            }
            _ => None,
        };
        let mut drawings = Vec::new();
        for shown in view.windows() {
            let (Some(function), Some(drawing)) = (&shown.function, &shown.drawing) else { continue };
            if !self.sent.get(function).is_some_and(|sent| Arc::ptr_eq(sent, drawing)) {
                self.sent.insert(function.clone(), drawing.clone());
                drawings.push((function.clone(), drawing.clone()));
            }
        }
        Answer { request, result, seen: seen(view), listing, drawings, files }
    }

    /// The files written, renamed or removed since the last answer; not the journal, which is the session's: a new
    /// worker writes its own.
    fn changed(&mut self) -> Option<Changed> {
        let memory = self.memory.as_ref()?;
        let now = files(memory);
        let mut changed = Changed::default();
        for (path, modified) in &now {
            if self.files.get(path) != Some(modified)
                && let (Some(relative), Some(bytes)) = (relative(path), memory.get(path))
            {
                changed.written.push(File { path: relative, bytes });
            }
        }
        changed.removed = self.files.keys().filter(|path| !now.contains_key(*path)).filter_map(|path| relative(path)).collect();
        changed.written.sort_by(|a, b| a.path.cmp(&b.path));
        changed.removed.sort();
        self.files = now;
        (!changed.written.is_empty() || !changed.removed.is_empty()).then_some(changed)
    }
}

/// The files of the project and when each changed, but the journal, which is the session's: a new worker writes its own.
fn files(memory: &Memory) -> HashMap<PathBuf, u128> {
    let journal = Path::new(ROOT).join(JOURNAL);
    memory.files(ROOT).into_iter().filter(|(path, _)| !path.starts_with(&journal)).collect()
}

/// What the window shows, but the drawing and the listing.
fn seen(view: &Published) -> Seen {
    let mut tabs: Vec<(String, String, Option<String>)> = view
        .tabs
        .iter()
        .map(|(function, tab)| (function.clone(), tab.view.as_str().to_owned(), tab.split.map(|split| split.as_str().to_owned())))
        .collect();
    tabs.sort();
    Seen {
        generation: view.generation,
        checks: view.checks,
        folder: view.folder.clone(),
        main: seen_window(&view.main),
        detached: view.detached.iter().map(seen_window).collect(),
        opened: view.opened.clone(),
        standard_everywhere: view.standard_everywhere,
        settings: settings_json(&view.settings).to_string(),
        tabs,
        preview: view.preview.clone(),
        workspace: view.workspace.0.to_owned(),
    }
}

/// The keys of the user as the page sends them, a JSON object; what does not read is no key.
fn keys_of(json: &str) -> serde_json::Map<String, Value> {
    serde_json::from_str::<serde_json::Map<String, Value>>(json).unwrap_or_default()
}

/// The settings of the session, as `Seen` carries them.
pub fn settings_json(stores: &labcraft_engine::settings::Stores) -> Value {
    json!({ "user": stores.user, "project": stores.project, "given": stores.given })
}

fn seen_window(shown: &Shown) -> SeenWindow {
    let mut arms: Vec<(String, usize)> = shown.arms.iter().map(|(key, arm)| (key.clone(), *arm)).collect();
    arms.sort();
    let selection = &shown.selection;
    SeenWindow {
        function: shown.function.clone(),
        glide: shown.glide,
        kept: shown.kept,
        arms,
        selection: (selection.function.clone(), selection.nodes.clone(), selection.wires.clone(), selection.terminals.clone()),
    }
}

fn listing(view: &Published, report: &Report) -> Listing {
    Listing {
        functions: view
            .functions
            .iter()
            .map(|listed| Listed {
                function: listed.function.clone(),
                file: listed.file.clone(),
                drawable: listed.drawable,
                errors: listed.errors,
                warnings: listed.warnings,
            })
            .collect(),
        types: view.types.iter().map(|listed| (listed.name.clone(), listed.file.clone())).collect(),
        problems: report
            .diagnostics()
            .iter()
            .map(|diagnostic| Problem {
                code: diagnostic.code().as_str().to_owned(),
                message: diagnostic.message.clone(),
                file: diagnostic.file.clone(),
                span: diagnostic.span.map(|span| [span.start.line, span.start.column, span.end.line, span.end.column]),
                function: diagnostic.function.clone(),
            })
            .collect(),
        errors: view.errors,
        warnings: view.warnings,
    }
}

/// The path of a file under the root, or none when it would leave it or names nothing: the page walked a folder, and
/// a path it gives is relative to it.
fn inside(path: &str) -> Option<PathBuf> {
    let relative = Path::new(path);
    let normal = !path.is_empty() && relative.components().all(|component| matches!(component, Component::Normal(_)));
    normal.then(|| Path::new(ROOT).join(relative))
}

/// A path of the memory, from the root, with `/`.
fn relative(path: &Path) -> Option<String> {
    let parts: Option<Vec<&str>> = path.strip_prefix(ROOT).ok()?.components().map(|component| component.as_os_str().to_str()).collect();
    parts.map(|parts| parts.join("/")).filter(|path| !path.is_empty())
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;

    const DOUBLE: &str = "labcraft 1\n\nuse core\n\nfn double(x: f64) -> (y: f64) {\n    y = core.add(x, x)\n}\n";

    fn folder() -> Arc<Folder> {
        Arc::new(Folder {
            name: "f".to_owned(),
            files: vec![File { path: "double.lcf".to_owned(), bytes: DOUBLE.as_bytes().to_vec() }],
            folders: Vec::new(),
        })
    }

    fn open(folder: Arc<Folder>) -> ToWorker {
        ToWorker::Open { folder, user_settings: String::new(), skipped: Vec::new(), show: None, detached: Vec::new() }
    }

    fn command(id: &str, params: Value) -> ToWorker {
        ToWorker::Command { request: 1, id: id.to_owned(), params: params.to_string() }
    }

    #[test]
    fn a_path_that_leaves_the_folder_is_dropped() {
        assert_eq!(inside("a/b.lcf"), Some(PathBuf::from("/folder/a/b.lcf")));
        for path in ["", "../x.lcf", "a/../../x.lcf", "/etc/x.lcf", "./x.lcf"] {
            assert_eq!(inside(path), None, "{path}");
        }
        assert_eq!(relative(Path::new("/folder/a/b.lcf")).as_deref(), Some("a/b.lcf"));
        assert_eq!(relative(Path::new("/folder")), None);
    }

    #[test]
    fn a_command_before_a_folder_waits_for_one() {
        let answer = Host::new(None).handle(command("view.show_function", json!({})));
        assert_eq!(answer.request, Some(1));
        assert_eq!(answer.result.map_err(|failure| failure.code), Err("command.not_enabled".to_owned()));
        let answer = Host::new(None).handle(ToWorker::Command { request: 2, id: "view.show_function".to_owned(), params: "{".to_owned() });
        assert_eq!(answer.result.map_err(|failure| failure.code), Err("params.invalid".to_owned()));
    }

    #[test]
    fn a_drawing_is_sent_once() {
        let mut host = Host::new(None);
        let answer = host.handle(open(folder()));
        assert!(answer.result.is_ok(), "{:?}", answer.result);
        assert_eq!((answer.request, answer.seen.main.function.as_deref()), (None, Some("double")));
        assert!(!answer.drawings.is_empty() && answer.listing.is_some() && answer.files.is_none());
        let again = host.handle(command("view.show_function", json!({ "function": "double" })));
        assert!(again.result.is_ok());
        assert!(!again.drawings.is_empty(), "a new drawing of the same function is sent");
        let state = host.handle(command("view.set_standard_view", json!({ "on": false })));
        assert!(state.drawings.is_empty(), "nothing new to draw");
        assert!(state.listing.is_none(), "nothing new to list");
    }

    /// The window's commands work in the worker: it has a window (`view.set_view`), and its files can be written.
    #[test]
    fn the_session_has_a_window_and_its_files_are_written_and_said() {
        let mut host = Host::new(None);
        assert!(host.handle(open(folder())).result.is_ok());
        let view = host.handle(command("view.set_view", json!({ "function": "double", "view": "text", "split": "diagram" })));
        assert!(view.result.is_ok(), "{:?}", view.result);
        assert_eq!(view.seen.tabs, [("double".to_owned(), "text".to_owned(), Some("diagram".to_owned()))]);
        let text = DOUBLE.replace("core.add(x, x)", "core.multiply(x, 2.0)");
        let written = host.handle(command("file.write", json!({ "path": "double.lcf", "text": text })));
        assert!(written.result.is_ok(), "{:?}", written.result);
        let files = written.files.unwrap();
        let lcf: Vec<&File> = files.written.iter().filter(|file| !file.path.starts_with(".labcraft/")).collect();
        assert_eq!(lcf, [&File { path: "double.lcf".to_owned(), bytes: text.into_bytes() }]);
        assert!(files.written.iter().all(|file| !file.path.starts_with(".labcraft/journal")), "not the journal");
        assert!(files.removed.is_empty());
        assert!(written.listing.is_some(), "the write checked the folder again");
        let renamed = host.handle(command("file.rename", json!({ "from": "double.lcf", "to": "twice.lcf" })));
        assert!(renamed.result.is_ok(), "{:?}", renamed.result);
        let files = renamed.files.unwrap();
        assert_eq!((files.written.len(), files.removed.as_slice()), (1, ["double.lcf".to_owned()].as_slice()));
        assert!(host.handle(command("view.set_standard_view", json!({ "on": true }))).files.is_none(), "nothing written");
    }

    #[test]
    fn the_listing_carries_the_diagnostics() {
        let broken = Arc::new(Folder {
            name: "f".to_owned(),
            files: vec![File { path: "broken.lcf".to_owned(), bytes: b"labcraft 1\n\nfn broken( -> () {}\n".to_vec() }],
            folders: Vec::new(),
        });
        let listing = Host::new(None).handle(open(broken)).listing.unwrap();
        assert!(listing.errors > 0);
        let problem = listing.problems.first().unwrap();
        assert!(problem.code.starts_with("LC") && problem.file == "broken.lcf" && problem.span.is_some(), "{problem:?}");
    }

    /// What the walk of `engine` finds in a whole tree, and in what the page gives of it (`folder::plan`): the same.
    #[test]
    fn the_session_judges_what_the_page_gives_as_it_judges_the_whole_folder() {
        use labcraft_engine::walk::BOUNDS;
        let at = |levels: usize, name: &str| format!("{}{name}", "d/".repeat(levels));
        let function = |name: &str| DOUBLE.replace("double", name).into_bytes();
        let tree = [
            ("top.lcf", function("top")),
            ("notes.txt", b"not a program".to_vec()),
            (".git/hidden.lcf", function("hidden")),
            ("sub/.hidden_file.lcf", function("hidden_file")),
            (at(BOUNDS.levels - 1, "deep.lcf").as_str(), function("deep")),
            (at(BOUNDS.levels, "deeper.lcf").as_str(), function("deeper")),
            (at(BOUNDS.levels + 4, "deepest.lcf").as_str(), function("deepest")),
        ]
        .map(|(path, bytes)| File { path: path.to_owned(), bytes });
        let folders = vec!["empty".to_owned(), at(BOUNDS.levels + 6, "")];
        let whole = Folder { name: "f".to_owned(), files: tree.to_vec(), folders: folders.clone() };
        let paths: Vec<String> = tree.iter().map(|file| file.path.clone()).collect();
        let plan = crate::folder::plan(&paths, &folders);
        let given = Folder {
            name: "f".to_owned(),
            files: plan.read.iter().filter_map(|&index| tree.get(index).cloned()).collect(),
            folders: plan.folders.clone(),
        };
        assert!(given.files.len() < whole.files.len(), "{plan:?}");
        let listing = |folder: Folder| Host::new(None).handle(open(Arc::new(folder))).listing.unwrap();
        let (whole, given) = (listing(whole), listing(given));
        assert_eq!(given, whole);
        assert!(whole.problems.iter().any(|problem| problem.code == "LC0010"), "{:?}", whole.problems);
        // A function is named by its folders (cli.md 7): the deepest that is read, the hidden file, the top one.
        let names: Vec<&str> = whole.functions.iter().map(|listed| listed.function.rsplit('.').next().unwrap_or_default()).collect();
        assert_eq!(names, ["deep", "hidden_file", "top"]);
    }

    #[test]
    fn a_function_skipped_is_not_drawn() {
        let mut host = Host::new(None);
        let answer = host.handle(ToWorker::Open {
            folder: folder(),
            user_settings: String::new(),
            skipped: vec!["double".to_owned()],
            show: None,
            detached: Vec::new(),
        });
        assert!(answer.result.is_ok());
        assert_eq!(answer.seen.main.function.as_deref(), Some("double"));
        assert!(answer.drawings.is_empty());
    }
}
