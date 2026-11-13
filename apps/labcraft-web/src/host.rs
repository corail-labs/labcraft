//! What the worker does with each message of the page (`docs/architecture.md`, 11.5): a session of `engine` on the files
//! the page read, in memory, read-only, and the `view.*` commands. Plain Rust, tested natively and in WASM; the worker
//! only carries the bytes.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use labcraft_diagram::Drawing;
use labcraft_engine::{CommandError, Config, Memory, ReadOnly, Session, Still};
use labcraft_types::{Limits, Report};
use serde_json::{Value, json};

use crate::protocol::{Answer, Failure, Folder, Listed, Listing, State, ToWorker};

/// Where the folder given lies in the memory of the worker: every path of a report is shown from it.
const ROOT: &str = "/folder";

/// The session of the worker, and the drawing and the report whose listing it sent last.
#[derive(Default)]
pub struct Host {
    session: Option<Session>,
    before_drawing: Option<fn(&str)>,
    sent: Option<Arc<Drawing>>,
    listed: Option<Arc<Report>>,
}

impl Host {
    /// `before_drawing`: called with the name of each function just before it is drawn (`engine`, `before_drawing`).
    pub fn new(before_drawing: Option<fn(&str)>) -> Host {
        Host { before_drawing, ..Host::default() }
    }

    /// Answers a message; [`ToWorker::Trap`] is the worker's to do, and is answered here as an unknown command.
    pub fn handle(&mut self, message: ToWorker) -> Answer {
        let result = match message {
            ToWorker::Open { folder, standard_everywhere, skipped, show } => self.open(folder, standard_everywhere, skipped, show),
            ToWorker::Command { id, params } => match serde_json::from_str::<Value>(&params) {
                Ok(params) => self.execute(&id, &params),
                Err(error) => Err(CommandError::new("params.invalid", format!("the parameters are not JSON: {error}"))),
            },
            ToWorker::Trap => Err(CommandError::new("command.unknown", "`Trap` is the worker's own")),
        };
        self.answer(result)
    }

    fn open(&mut self, folder: Arc<Folder>, standard_everywhere: bool, skipped: Vec<String>, show: Option<String>) -> Result<Value, CommandError> {
        // The session before goes first, and the bytes move into the memory of the new one: a folder is held once.
        self.session = None;
        let folder = Arc::try_unwrap(folder).unwrap_or_else(|shared| Folder::clone(&shared));
        let memory = Memory::new();
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
        let mut session = Session::new(Arc::new(ReadOnly(Arc::new(memory))), config, false)?;
        session.skip_drawing(skipped.into_iter().collect());
        session.before_drawing(self.before_drawing);
        session.execute("view.set_standard_view", &json!({ "on": standard_everywhere }))?;
        // The function shown before a restart, or that the address names, when the folder has it: the only one drawn.
        session.open_on(show);
        let opened = session.execute("view.open", &json!({ "folder": "." }));
        self.session = Some(session);
        self.sent = None;
        self.listed = None;
        opened
    }

    fn execute(&mut self, id: &str, params: &Value) -> Result<Value, CommandError> {
        match &mut self.session {
            Some(session) => session.execute(id, params),
            None => Err(CommandError::new("command.not_enabled", format!("`{id}` waits for a folder: open one first"))),
        }
    }

    fn answer(&mut self, result: Result<Value, CommandError>) -> Answer {
        let result = result.map(|value| value.to_string()).map_err(|error| Failure { code: error.code.to_owned(), message: error.message });
        let Some(session) = &self.session else {
            return Answer { result, state: State::default(), listing: None, drawing: None };
        };
        let view = session.view();
        // The functions and the text of `Problems` go again only with a new report.
        let listing = match &view.report {
            Some(report) if !self.listed.as_ref().is_some_and(|listed| Arc::ptr_eq(listed, report)) => {
                self.listed = Some(report.clone());
                Some(Listing {
                    functions: view
                        .functions
                        .iter()
                        .map(|listed| Listed {
                            function: listed.function.clone(),
                            file: listed.file.clone(),
                            drawable: listed.drawable,
                            errors: listed.errors,
                        })
                        .collect(),
                    problems: labcraft_types::printed::diagnostics_text(report),
                    errors: view.errors,
                    warnings: view.warnings,
                })
            }
            _ => None,
        };
        let mut arms: Vec<(String, usize)> = view.arms.iter().map(|(key, arm)| (key.clone(), *arm)).collect();
        arms.sort();
        let state = State {
            folder: view.folder.clone(),
            function: view.function.clone(),
            opened: view.opened.clone(),
            glide: view.glide,
            kept: view.kept,
            arms,
            standard_everywhere: view.standard_everywhere,
        };
        let drawing = view.drawing.clone().filter(|drawing| !self.sent.as_ref().is_some_and(|sent| Arc::ptr_eq(sent, drawing)));
        if let Some(drawing) = &drawing {
            self.sent = Some(drawing.clone());
        }
        Answer { result, state, listing, drawing }
    }
}

/// The path of a file under the root, or none when it would leave it or names nothing: the page walked a folder, and
/// a path it gives is relative to it.
fn inside(path: &str) -> Option<PathBuf> {
    let relative = Path::new(path);
    let normal = !path.is_empty() && relative.components().all(|component| matches!(component, Component::Normal(_)));
    normal.then(|| Path::new(ROOT).join(relative))
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;
    use crate::protocol::File;

    const DOUBLE: &str = "labcraft 1\n\nuse core\n\nfn double(x: f64) -> (y: f64) {\n    y = core.add(x, x)\n}\n";

    fn folder() -> Arc<Folder> {
        Arc::new(Folder {
            name: "f".to_owned(),
            files: vec![File { path: "double.lcf".to_owned(), bytes: DOUBLE.as_bytes().to_vec() }],
            folders: Vec::new(),
        })
    }

    fn command(id: &str, params: Value) -> ToWorker {
        ToWorker::Command { id: id.to_owned(), params: params.to_string() }
    }

    #[test]
    fn a_path_that_leaves_the_folder_is_dropped() {
        assert_eq!(inside("a/b.lcf"), Some(PathBuf::from("/folder/a/b.lcf")));
        for path in ["", "../x.lcf", "a/../../x.lcf", "/etc/x.lcf", "./x.lcf"] {
            assert_eq!(inside(path), None, "{path}");
        }
    }

    #[test]
    fn a_command_before_a_folder_waits_for_one() {
        let answer = Host::new(None).handle(command("view.show_function", json!({})));
        assert_eq!(answer.result.map_err(|failure| failure.code), Err("command.not_enabled".to_owned()));
        let answer = Host::new(None).handle(ToWorker::Command { id: "view.show_function".to_owned(), params: "{".to_owned() });
        assert_eq!(answer.result.map_err(|failure| failure.code), Err("params.invalid".to_owned()));
    }

    #[test]
    fn a_drawing_is_sent_once() {
        let mut host = Host::new(None);
        let answer = host.handle(ToWorker::Open { folder: folder(), standard_everywhere: false, skipped: Vec::new(), show: None });
        assert!(answer.result.is_ok(), "{:?}", answer.result);
        assert_eq!(answer.state.function.as_deref(), Some("double"));
        assert!(answer.drawing.is_some() && answer.listing.is_some());
        let again = host.handle(command("view.show_function", json!({ "function": "double" })));
        assert!(again.result.is_ok());
        assert!(again.drawing.is_some(), "a new drawing of the same function is sent");
        let state = host.handle(command("view.set_standard_view", json!({ "on": false })));
        assert!(state.drawing.is_none(), "nothing new to draw");
        assert!(state.listing.is_none(), "nothing new to list");
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
        let listing = |folder: Folder| {
            let open = ToWorker::Open { folder: Arc::new(folder), standard_everywhere: false, skipped: Vec::new(), show: None };
            Host::new(None).handle(open).listing.unwrap()
        };
        let (whole, given) = (listing(whole), listing(given));
        assert_eq!(given, whole);
        assert!(whole.problems.contains("LC0010"), "{}", whole.problems);
        // A function is named by its folders (cli.md 7): the deepest that is read, the hidden file, the top one.
        let names: Vec<&str> = whole.functions.iter().map(|listed| listed.function.rsplit('.').next().unwrap_or_default()).collect();
        assert_eq!(names, ["deep", "hidden_file", "top"]);
    }

    #[test]
    fn a_function_skipped_is_not_drawn() {
        let mut host = Host::new(None);
        let answer = host.handle(ToWorker::Open { folder: folder(), standard_everywhere: false, skipped: vec!["double".to_owned()], show: None });
        assert!(answer.result.is_ok());
        assert_eq!(answer.state.function.as_deref(), Some("double"));
        assert!(answer.drawing.is_none());
    }
}
