//! What the page knows of its worker (`docs/architecture.md`, 11.5): the files it gave it, kept so that a trap of WASM
//! restarts it with a new session and the same files, the one message in flight, the function the worker said it was
//! drawing, and what a trap leaves out. Plain Rust, tested natively: the page only starts workers and carries the bytes.
//!
//! - A trap after the worker named a function it was drawing: that function is not drawn again, and shows `LC0099` with
//!   its last drawing, whichever command drew it (`view.open` draws the first function).
//! - Any other trap (reading or checking the folder): the worker restarts once for that set of files; a second trap
//!   shows `LC0099` for the folder until the files change. A file that traps cannot restart the worker forever.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use labcraft_diagram::Drawing;
use serde_json::Value;

use crate::protocol::{Folder, FromWorker, State, ToWorker, decode, encode};

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
    Command { id: String, params: Value },
    Trap,
}

/// What the page waits for from its worker.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Waiting {
    #[default]
    Nothing,
    /// A worker started: that it listens.
    Ready,
    /// The answer to the message in flight.
    Answer,
}

/// The traps of the folder open that no drawing explains.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Traps {
    #[default]
    None,
    /// The worker restarted once for these files.
    Once,
    /// Twice: `LC0099` for the folder, until the files change.
    Twice,
}

#[derive(Default)]
pub struct Supervisor {
    folder: Option<Arc<Folder>>,
    pending: VecDeque<Request>,
    waiting: Waiting,
    /// The function the worker said it was drawing, since the message in flight was sent.
    drawing: Option<String>,
    traps: Traps,
    /// The functions whose drawing trapped a worker.
    trapped: BTreeSet<String>,
    state: Arc<State>,
    /// The last drawing of each function, which a function whose drawing trapped keeps.
    drawings: BTreeMap<String, Arc<Drawing>>,
    failure: Option<String>,
}

/// What the window shows.
#[derive(Clone, Debug, Default)]
pub struct Shown {
    pub state: Arc<State>,
    pub drawing: Option<Arc<Drawing>>,
    /// The function shown is not drawable now, or its drawing trapped: its last drawing stays.
    pub kept: bool,
    /// What the window must say: the traps (`LC0099`), a command that failed.
    pub notices: Vec<String>,
    /// A worker starts, or a message is in flight: the window says it works.
    pub busy: bool,
}

impl Supervisor {
    pub fn new(standard_everywhere: bool) -> Supervisor {
        Supervisor { state: Arc::new(State { standard_everywhere, ..State::default() }), ..Supervisor::default() }
    }

    /// A folder read by the page: in a new worker, in place of the folder open; the traps of the files before are
    /// forgotten.
    pub fn open(&mut self, folder: Folder) -> Vec<Action> {
        let standard_everywhere = self.state.standard_everywhere;
        *self = Supervisor { folder: Some(Arc::new(folder)), ..Supervisor::new(standard_everywhere) };
        self.pending.push_back(Request::Open);
        self.start()
    }

    /// A command of the viewer (`docs/commands.md`, 6.5), sent when the one in flight has its answer.
    pub fn command(&mut self, id: &str, params: Value) -> Vec<Action> {
        if self.traps == Traps::Twice || self.folder.is_none() {
            return Vec::new();
        }
        self.pending.push_back(Request::Command { id: id.to_owned(), params });
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

    /// A message of the worker.
    pub fn received(&mut self, bytes: &[u8]) -> Vec<Action> {
        match decode::<FromWorker>(bytes) {
            Ok(FromWorker::Ready) => self.waiting = Waiting::Nothing,
            Ok(FromWorker::Drawing(function)) => {
                self.drawing = Some(function);
                return Vec::new();
            }
            Ok(FromWorker::Answer(answer)) => {
                self.waiting = Waiting::Nothing;
                self.drawing = None;
                if let Err(failure) = &answer.result {
                    self.failure = Some(format!("{}: {}", failure.code, failure.message));
                }
                if let (Some(drawing), Some(function)) = (answer.drawing, &answer.state.function) {
                    self.drawings.insert(function.clone(), drawing);
                }
                self.state = Arc::new(answer.state);
            }
            Err(error) => {
                self.waiting = Waiting::Nothing;
                self.failure = Some(error);
            }
        }
        self.next()
    }

    /// The worker trapped: a function it said it was drawing is not drawn again; otherwise the folder is to blame.
    pub fn trapped(&mut self) -> Vec<Action> {
        if let Some(function) = self.drawing.take() {
            self.trapped.insert(function.clone());
            Arc::make_mut(&mut self.state).function = Some(function);
        } else if self.traps == Traps::None {
            self.traps = Traps::Once;
        } else {
            self.traps = Traps::Twice;
            self.pending.clear();
            self.waiting = Waiting::Nothing;
            return Vec::new();
        }
        // What was in flight went with the worker; the new one opens the same files first.
        self.pending.retain(|request| *request != Request::Open);
        self.pending.push_front(Request::Open);
        self.start()
    }

    fn start(&mut self) -> Vec<Action> {
        self.waiting = Waiting::Ready;
        self.drawing = None;
        vec![Action::Start]
    }

    /// Sends what waits, if nothing is in flight.
    fn next(&mut self) -> Vec<Action> {
        if self.waiting != Waiting::Nothing || self.traps == Traps::Twice {
            return Vec::new();
        }
        let Some(request) = self.pending.pop_front() else { return Vec::new() };
        let message = match request {
            Request::Open => match &self.folder {
                Some(folder) => ToWorker::Open {
                    folder: Arc::clone(folder),
                    standard_everywhere: self.state.standard_everywhere,
                    skipped: self.trapped.iter().cloned().collect(),
                    show: self.state.function.clone(),
                },
                None => return Vec::new(),
            },
            Request::Command { id, params } => ToWorker::Command { id, params: params.to_string() },
            Request::Trap => ToWorker::Trap,
        };
        match encode(&message) {
            Ok(bytes) => {
                self.waiting = Waiting::Answer;
                vec![Action::Send(bytes)]
            }
            Err(error) => {
                self.failure = Some(error);
                self.next()
            }
        }
    }

    /// The name of the folder open, as the page read it.
    pub fn folder_name(&self) -> Option<&str> {
        self.folder.as_deref().map(|folder| folder.name.as_str())
    }

    /// What the window shows now.
    pub fn shown(&self) -> Shown {
        let mut notices = Vec::new();
        if self.traps == Traps::Twice {
            notices.push(
                "LC0099: reading this folder stopped the viewer twice, which is a bug of LabCraft: open the folder again once its files changed"
                    .to_owned(),
            );
        }
        let function = self.state.function.as_ref();
        let drawing = function.and_then(|function| self.drawings.get(function)).cloned();
        let trapped = function.filter(|function| self.trapped.contains(*function));
        if let Some(function) = trapped {
            let kept = if drawing.is_some() { "; its last drawing stays" } else { "" };
            notices.push(format!(
                "LC0099: drawing `{function}` stopped the viewer, which is a bug of LabCraft: it is not drawn again until the files change{kept}"
            ));
        }
        if let Some(failure) = &self.failure {
            notices.push(failure.clone());
        }
        Shown {
            state: self.state.clone(),
            kept: self.state.kept || (trapped.is_some() && drawing.is_some()),
            drawing,
            notices,
            busy: self.waiting != Waiting::Nothing,
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use std::cell::RefCell;

    use super::*;
    use crate::host::Host;
    use crate::protocol::File;
    use serde_json::json;

    thread_local! {
        /// The functions whose drawing traps the simulated worker, and the one it was drawing when it did.
        static POISON: RefCell<(BTreeSet<String>, Option<String>)> = RefCell::default();
    }

    /// Says what it draws, as the worker does, and remembers a drawing that traps.
    fn announce(function: &str) {
        POISON.with_borrow_mut(|(poison, trapped)| {
            if trapped.is_none() && poison.contains(function) {
                *trapped = Some(function.to_owned());
            }
        });
    }

    fn poison(functions: &[&str]) {
        POISON.with_borrow_mut(|(poison, _)| *poison = functions.iter().map(|function| (*function).to_owned()).collect());
    }

    /// A worker that traps on `Trap`, and on drawing a poisoned function, once it said that it draws it.
    #[derive(Default)]
    struct Worker {
        host: Option<Host>,
        started: usize,
    }

    enum Outcome {
        Message(Box<FromWorker>),
        Trap,
    }

    impl Worker {
        fn run(&mut self, actions: Vec<Action>) -> Vec<Outcome> {
            let mut outcomes = Vec::new();
            for action in actions {
                match action {
                    Action::Start => {
                        self.host = Some(Host::new(Some(announce)));
                        self.started += 1;
                        outcomes.push(Outcome::Message(Box::new(FromWorker::Ready)));
                    }
                    Action::Send(bytes) => {
                        let message: ToWorker = decode(&bytes).unwrap();
                        if message == ToWorker::Trap {
                            self.host = None;
                            outcomes.push(Outcome::Trap);
                            continue;
                        }
                        let answer = self.host.as_mut().unwrap().handle(message);
                        if let Some(function) = POISON.with_borrow_mut(|(_, trapped)| trapped.take()) {
                            self.host = None;
                            outcomes.push(Outcome::Message(Box::new(FromWorker::Drawing(function))));
                            outcomes.push(Outcome::Trap);
                        } else {
                            outcomes.push(Outcome::Message(Box::new(FromWorker::Answer(answer))));
                        }
                    }
                }
            }
            outcomes
        }
    }

    fn drive(supervisor: &mut Supervisor, worker: &mut Worker, actions: Vec<Action>) {
        let mut outcomes: VecDeque<Outcome> = worker.run(actions).into();
        while let Some(outcome) = outcomes.pop_front() {
            let actions = match outcome {
                Outcome::Message(message) => supervisor.received(&encode(&*message).unwrap()),
                Outcome::Trap => supervisor.trapped(),
            };
            outcomes.extend(worker.run(actions));
        }
    }

    fn function(name: &str) -> File {
        let text = format!("labcraft 1\n\nuse core\n\nfn {name}(x: f64) -> (y: f64) {{\n    y = core.add(x, x)\n}}\n");
        File { path: format!("{name}.lcf"), bytes: text.into_bytes() }
    }

    fn folder() -> Folder {
        Folder { name: "bench".to_owned(), files: vec![function("alpha"), function("beta")] }
    }

    fn shown_function(supervisor: &Supervisor) -> Option<String> {
        supervisor.shown().state.function.clone()
    }

    fn drawn_function(supervisor: &Supervisor) -> Option<String> {
        supervisor.shown().drawing.map(|drawing| drawing.function.clone())
    }

    #[test]
    fn a_folder_opens_and_draws_its_first_function() {
        poison(&[]);
        let (mut supervisor, mut worker) = (Supervisor::new(false), Worker::default());
        let actions = supervisor.open(folder());
        drive(&mut supervisor, &mut worker, actions);
        let shown = supervisor.shown();
        assert_eq!(shown.state.function.as_deref(), Some("alpha"));
        assert!(shown.drawing.is_some() && shown.notices.is_empty() && !shown.busy);
        let actions = supervisor.command("view.show_function", json!({ "function": "beta" }));
        drive(&mut supervisor, &mut worker, actions);
        assert_eq!(drawn_function(&supervisor).as_deref(), Some("beta"));
    }

    #[test]
    fn a_drawing_that_traps_keeps_its_last_drawing_and_is_not_drawn_again() {
        poison(&[]);
        let (mut supervisor, mut worker) = (Supervisor::new(false), Worker::default());
        let actions = supervisor.open(folder());
        drive(&mut supervisor, &mut worker, actions);
        let actions = supervisor.command("view.show_function", json!({ "function": "beta" }));
        drive(&mut supervisor, &mut worker, actions);
        // Its file changed, say, and its next drawing traps.
        poison(&["beta"]);
        let actions = supervisor.command("view.set_standard_view", json!({ "on": true }));
        drive(&mut supervisor, &mut worker, actions);
        assert_eq!(worker.started, 2, "restarted once");
        let shown = supervisor.shown();
        assert_eq!(shown.state.function.as_deref(), Some("beta"));
        assert_eq!(drawn_function(&supervisor).as_deref(), Some("beta"), "its last drawing");
        assert!(shown.kept);
        assert!(shown.notices.iter().any(|notice| notice.starts_with("LC0099: drawing `beta`")), "{:?}", shown.notices);
        // The new worker never draws it: shown again, it does not trap.
        let actions = supervisor.command("view.show_function", json!({ "function": "alpha" }));
        drive(&mut supervisor, &mut worker, actions);
        assert_eq!(shown_function(&supervisor).as_deref(), Some("alpha"));
        assert!(supervisor.shown().notices.is_empty());
        let actions = supervisor.command("view.show_function", json!({ "function": "beta" }));
        drive(&mut supervisor, &mut worker, actions);
        assert_eq!((worker.started, drawn_function(&supervisor).as_deref()), (2, Some("beta")));
        // New files: forgotten.
        poison(&[]);
        let actions = supervisor.open(folder());
        drive(&mut supervisor, &mut worker, actions);
        let actions = supervisor.command("view.show_function", json!({ "function": "beta" }));
        drive(&mut supervisor, &mut worker, actions);
        assert!(supervisor.shown().notices.is_empty());
    }

    #[test]
    fn a_first_function_that_traps_is_its_own_fault_not_the_folders() {
        poison(&["alpha"]);
        let (mut supervisor, mut worker) = (Supervisor::new(false), Worker::default());
        // `view.open` draws `alpha`, the first function, and traps: the new worker shows it without drawing it.
        let actions = supervisor.open(folder());
        drive(&mut supervisor, &mut worker, actions);
        assert_eq!(worker.started, 2);
        let shown = supervisor.shown();
        assert_eq!(shown.state.function.as_deref(), Some("alpha"));
        assert!(shown.drawing.is_none() && !shown.busy);
        assert!(shown.notices.iter().any(|notice| notice.starts_with("LC0099: drawing `alpha`")), "{:?}", shown.notices);
        let actions = supervisor.command("view.show_function", json!({ "function": "beta" }));
        drive(&mut supervisor, &mut worker, actions);
        assert_eq!(drawn_function(&supervisor).as_deref(), Some("beta"));
    }

    #[test]
    fn a_folder_that_traps_twice_shows_lc0099_until_its_files_change() {
        poison(&[]);
        let (mut supervisor, mut worker) = (Supervisor::new(false), Worker::default());
        let actions = supervisor.open(folder());
        drive(&mut supervisor, &mut worker, actions);
        // Two traps that no drawing explains.
        let actions = supervisor.trap();
        drive(&mut supervisor, &mut worker, actions);
        assert_eq!(worker.started, 2);
        assert!(supervisor.shown().notices.is_empty());
        let actions = supervisor.trap();
        drive(&mut supervisor, &mut worker, actions);
        assert_eq!(worker.started, 2, "no third worker");
        let shown = supervisor.shown();
        assert!(shown.notices.iter().any(|notice| notice.starts_with("LC0099: reading this folder")), "{:?}", shown.notices);
        assert!(supervisor.command("view.show_function", json!({ "function": "beta" })).is_empty());
        assert!(!shown.busy);
        let actions = supervisor.open(folder());
        drive(&mut supervisor, &mut worker, actions);
        assert_eq!(worker.started, 3);
        assert!(supervisor.shown().notices.is_empty() && supervisor.shown().drawing.is_some());
    }

    #[test]
    fn a_trap_restarts_the_worker_on_the_same_files_and_the_function_shown() {
        poison(&[]);
        let (mut supervisor, mut worker) = (Supervisor::new(true), Worker::default());
        let actions = supervisor.open(folder());
        drive(&mut supervisor, &mut worker, actions);
        let actions = supervisor.command("view.show_function", json!({ "function": "beta" }));
        drive(&mut supervisor, &mut worker, actions);
        let actions = supervisor.trap();
        drive(&mut supervisor, &mut worker, actions);
        assert_eq!(worker.started, 2);
        let shown = supervisor.shown();
        assert_eq!(shown.state.function.as_deref(), Some("beta"), "the function shown before");
        assert!(shown.state.standard_everywhere, "the preference");
        assert!(shown.notices.is_empty() && shown.drawing.is_some());
    }

    #[test]
    fn nothing_goes_to_a_worker_before_it_listens_and_one_message_at_a_time() {
        let mut supervisor = Supervisor::new(false);
        assert_eq!(supervisor.open(folder()), [Action::Start]);
        assert!(supervisor.shown().busy);
        assert!(supervisor.command("view.show_function", json!({ "function": "beta" })).is_empty());
        let actions = supervisor.received(&encode(&FromWorker::Ready).unwrap());
        assert!(matches!(actions.as_slice(), [Action::Send(_)]));
        assert!(supervisor.command("view.show_function", json!({ "function": "beta" })).is_empty());
        assert!(supervisor.shown().busy);
    }
}
