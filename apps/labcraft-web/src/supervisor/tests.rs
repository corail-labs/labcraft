#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

use std::cell::RefCell;

use serde_json::json;

use super::*;
use crate::host::Host as WorkerHost;
use crate::protocol::File;

thread_local! {
    /// The functions whose drawing traps the simulated worker, and the one it was drawing when it did.
    static POISON: RefCell<(BTreeSet<String>, Option<String>)> = RefCell::default();
    /// The files the simulated worker said, before it drew.
    static SAID: RefCell<Vec<crate::protocol::Changed>> = RefCell::default();
}

/// Says the files changed, as the worker does.
fn say_files(changed: &crate::protocol::Changed) {
    SAID.with_borrow_mut(|said| said.push(changed.clone()));
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
    host: Option<WorkerHost>,
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
                    self.host = Some(WorkerHost::new(Some(announce)).with_files(say_files));
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
                    for said in SAID.with_borrow_mut(std::mem::take) {
                        outcomes.push(Outcome::Message(Box::new(FromWorker::Files(said))));
                    }
                    if let Some(function) = POISON.with_borrow_mut(|(_, trapped)| trapped.take()) {
                        self.host = None;
                        outcomes.push(Outcome::Message(Box::new(FromWorker::Drawing(function))));
                        outcomes.push(Outcome::Trap);
                    } else {
                        outcomes.push(Outcome::Message(Box::new(FromWorker::Answer(Box::new(answer)))));
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

/// A command sent and driven to its answer, which comes back.
fn command(supervisor: &mut Supervisor, worker: &mut Worker, id: &str, params: Value) -> Result<Value, CommandError> {
    let (sender, reply) = Reply::channel();
    let actions = supervisor.command(id, params, sender);
    drive(supervisor, worker, actions);
    reply.take().unwrap_or_else(|| Err(CommandError::internal("no answer")))
}

fn function(name: &str) -> File {
    let text = format!("labcraft 1\n\nuse core\n\nfn {name}(x: f64) -> (y: f64) {{\n    y = core.add(x, x)\n}}\n");
    File { path: format!("{name}.lcf"), bytes: text.into_bytes() }
}

fn folder() -> Folder {
    Folder { name: "bench".to_owned(), files: vec![function("alpha"), function("beta")], folders: Vec::new() }
}

fn shown(supervisor: &Supervisor) -> Arc<Published> {
    supervisor.snapshot().view
}

fn shown_function(supervisor: &Supervisor) -> Option<String> {
    shown(supervisor).main.function.clone()
}

fn drawn_function(supervisor: &Supervisor) -> Option<String> {
    shown(supervisor).main.drawing.as_ref().map(|drawing| drawing.function.clone())
}

fn opened(folder: Folder, function: Option<&str>) -> (Supervisor, Worker) {
    let (mut supervisor, mut worker) = (Supervisor::new(String::new()), Worker::default());
    let actions = supervisor.open(folder, function.map(str::to_owned));
    drive(&mut supervisor, &mut worker, actions);
    (supervisor, worker)
}

#[test]
fn a_folder_opens_and_draws_its_first_function() {
    poison(&[]);
    let (mut supervisor, mut worker) = opened(folder(), None);
    let snapshot = supervisor.snapshot();
    assert_eq!((snapshot.view.main.function.as_deref(), snapshot.project.as_deref()), (Some("alpha"), Some("bench")));
    assert!(snapshot.view.main.drawing.is_some() && snapshot.notices.is_empty() && !supervisor.busy());
    assert_eq!(snapshot.view.functions.len(), 2);
    assert!(snapshot.view.report.is_some());
    let result = command(&mut supervisor, &mut worker, "view.show_function", json!({ "function": "beta" }));
    assert_eq!(result.unwrap().get("function").and_then(Value::as_str), Some("beta"), "the answer comes back to its reply");
    assert_eq!(drawn_function(&supervisor).as_deref(), Some("beta"));
    assert_eq!(shown(&supervisor).opened, ["alpha", "beta"]);
}

/// The window sees each change as one: the counts go up, across the folders opened too.
#[test]
fn what_is_published_counts_up_across_folders() {
    poison(&[]);
    let (mut supervisor, mut worker) = opened(folder(), None);
    let before = shown(&supervisor);
    command(&mut supervisor, &mut worker, "view.show_function", json!({ "function": "beta" })).unwrap();
    let after = shown(&supervisor);
    assert!(after.generation > before.generation && after.main.drawing_generation > before.main.drawing_generation);
    let actions = supervisor.open(folder(), None);
    drive(&mut supervisor, &mut worker, actions);
    let again = shown(&supervisor);
    assert!(again.generation > after.generation && again.main.drawing_generation > after.main.drawing_generation && again.checks > after.checks);
}

/// The diagnostics of the worker reach `Problems` as a report, with their code, place and function.
#[test]
fn the_diagnostics_come_as_a_report() {
    poison(&[]);
    let broken = Folder {
        name: "broken".to_owned(),
        files: vec![function("alpha"), File { path: "bad.lcf".to_owned(), bytes: b"labcraft 1\n\nfn bad( -> () {}\n".to_vec() }],
        folders: Vec::new(),
    };
    let (supervisor, _) = opened(broken, None);
    let view = shown(&supervisor);
    let report = view.report.as_ref().unwrap();
    assert!(view.errors > 0);
    let diagnostic = report.diagnostics().first().unwrap();
    assert_eq!(diagnostic.file, "bad.lcf");
    assert!(diagnostic.span.is_some());
}

/// The address of the page names a function: the folder opens on it, or on its first function when it has none
/// of that name, which the window says until another function is shown.
#[test]
fn a_folder_opens_on_the_function_asked_for_when_it_has_it() {
    poison(&[]);
    for (asked, expected) in [(Some("beta"), "beta"), (Some("gamma"), "alpha"), (None, "alpha")] {
        let (mut supervisor, mut worker) = (Supervisor::new(String::new()), Worker::default());
        let actions = supervisor.open(folder(), asked.map(str::to_owned));
        // Nothing is drawn before the worker answers, whatever was asked, and nothing is said yet.
        assert!(shown(&supervisor).main.drawing.is_none() && supervisor.busy() && supervisor.notices().is_empty());
        drive(&mut supervisor, &mut worker, actions);
        assert_eq!((shown_function(&supervisor).as_deref(), drawn_function(&supervisor).as_deref()), (Some(expected), Some(expected)), "{asked:?}");
        let said: &[&str] = if asked == Some("gamma") { &["`bench` has no function `gamma`: here is `alpha`"] } else { &[] };
        assert_eq!(supervisor.notices(), said, "{asked:?}");
        command(&mut supervisor, &mut worker, "view.show_function", json!({ "function": "beta" })).unwrap();
        assert!(supervisor.notices().is_empty(), "{asked:?}: said until another function is shown");
        command(&mut supervisor, &mut worker, "view.show_function", json!({ "function": "alpha" })).unwrap();
        assert!(supervisor.notices().is_empty(), "{asked:?}: and not again");
    }
}

/// The function asked for traps as it is first drawn: it is shown with `LC0099` and no drawing, the folder is not
/// blamed, and nothing says that the folder lacks it.
#[test]
fn a_function_asked_for_that_traps_is_its_own_fault() {
    poison(&["beta"]);
    let (supervisor, worker) = opened(folder(), Some("beta"));
    assert_eq!((shown_function(&supervisor).as_deref(), shown(&supervisor).main.drawing.is_none(), worker.started), (Some("beta"), true, 2));
    let notices = supervisor.notices();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices.iter().all(|notice| notice.starts_with("LC0099: drawing `beta`")), "{notices:?}");
}

/// The function asked for is missing, and the first function, shown instead, traps: both are said.
#[test]
fn a_function_missing_and_the_first_that_traps_are_both_said() {
    poison(&["alpha"]);
    let (supervisor, _) = opened(folder(), Some("gamma"));
    assert_eq!(shown_function(&supervisor).as_deref(), Some("alpha"));
    let notices = supervisor.notices();
    assert_eq!(notices.len(), 2, "{notices:?}");
    assert!(notices.iter().any(|notice| notice.starts_with("LC0099: drawing `alpha`")), "{notices:?}");
    assert!(notices.iter().any(|notice| notice == "`bench` has no function `gamma`: here is `alpha`"), "{notices:?}");
}

#[test]
fn a_drawing_that_traps_keeps_its_last_drawing_and_is_not_drawn_again() {
    poison(&[]);
    let (mut supervisor, mut worker) = opened(folder(), None);
    command(&mut supervisor, &mut worker, "view.show_function", json!({ "function": "beta" })).unwrap();
    // Its file changed, say, and its next drawing traps: the command that drew it fails, and says why.
    poison(&["beta"]);
    let failed = command(&mut supervisor, &mut worker, "view.set_standard_view", json!({ "on": true })).unwrap_err();
    assert!(failed.message.contains("LC0099"), "{failed:?}");
    assert_eq!(worker.started, 2, "restarted once");
    let view = shown(&supervisor);
    assert_eq!(view.main.function.as_deref(), Some("beta"));
    assert_eq!(drawn_function(&supervisor).as_deref(), Some("beta"), "its last drawing");
    assert!(view.main.kept);
    assert!(supervisor.notices().iter().any(|notice| notice.starts_with("LC0099: drawing `beta`")), "{:?}", supervisor.notices());
    // The new worker never draws it: shown again, it does not trap.
    command(&mut supervisor, &mut worker, "view.show_function", json!({ "function": "alpha" })).unwrap();
    assert_eq!(shown_function(&supervisor).as_deref(), Some("alpha"));
    assert!(supervisor.notices().is_empty());
    command(&mut supervisor, &mut worker, "view.show_function", json!({ "function": "beta" })).unwrap();
    assert_eq!((worker.started, drawn_function(&supervisor).as_deref()), (2, Some("beta")));
    // New files: forgotten.
    poison(&[]);
    let actions = supervisor.open(folder(), None);
    drive(&mut supervisor, &mut worker, actions);
    command(&mut supervisor, &mut worker, "view.show_function", json!({ "function": "beta" })).unwrap();
    assert!(supervisor.notices().is_empty());
}

#[test]
fn a_first_function_that_traps_is_its_own_fault_not_the_folders() {
    poison(&["alpha"]);
    // `view.open` draws `alpha`, the first function, and traps: the new worker shows it without drawing it.
    let (mut supervisor, mut worker) = opened(folder(), None);
    assert_eq!(worker.started, 2);
    assert_eq!(shown_function(&supervisor).as_deref(), Some("alpha"));
    assert!(shown(&supervisor).main.drawing.is_none() && !supervisor.busy());
    assert!(supervisor.notices().iter().any(|notice| notice.starts_with("LC0099: drawing `alpha`")), "{:?}", supervisor.notices());
    command(&mut supervisor, &mut worker, "view.show_function", json!({ "function": "beta" })).unwrap();
    assert_eq!(drawn_function(&supervisor).as_deref(), Some("beta"));
}

#[test]
fn a_folder_that_traps_twice_shows_lc0099_until_another_is_opened() {
    poison(&[]);
    let (mut supervisor, mut worker) = opened(folder(), None);
    // Two traps that no drawing explains.
    let actions = supervisor.trap();
    drive(&mut supervisor, &mut worker, actions);
    assert_eq!(worker.started, 2);
    assert!(supervisor.notices().is_empty());
    let actions = supervisor.trap();
    drive(&mut supervisor, &mut worker, actions);
    assert_eq!(worker.started, 2, "no third worker");
    assert!(supervisor.notices().iter().any(|notice| notice.starts_with("LC0099: this project")), "{:?}", supervisor.notices());
    let refused = command(&mut supervisor, &mut worker, "view.show_function", json!({ "function": "beta" })).unwrap_err();
    assert_eq!(refused.code, "command.not_enabled");
    assert!(!supervisor.busy());
    let actions = supervisor.open(folder(), None);
    drive(&mut supervisor, &mut worker, actions);
    assert_eq!(worker.started, 3);
    assert!(supervisor.notices().is_empty() && shown(&supervisor).main.drawing.is_some());
}

#[test]
fn a_trap_restarts_the_worker_on_the_same_files_and_the_function_shown() {
    poison(&[]);
    let (mut supervisor, mut worker) = (Supervisor::new(r#"{ "view.standard_everywhere": true }"#.to_owned()), Worker::default());
    let actions = supervisor.open(folder(), None);
    drive(&mut supervisor, &mut worker, actions);
    command(&mut supervisor, &mut worker, "view.show_function", json!({ "function": "beta" })).unwrap();
    // A key that the worker set: the next one is given it.
    command(&mut supervisor, &mut worker, "settings.set", json!({ "key": "appearance.block_fill", "value": "neutral" })).unwrap();
    let actions = supervisor.trap();
    drive(&mut supervisor, &mut worker, actions);
    assert_eq!(worker.started, 2);
    let view = shown(&supervisor);
    assert_eq!(view.main.function.as_deref(), Some("beta"), "the function shown before");
    assert!(view.standard_everywhere, "the preference");
    assert_eq!(view.settings.user.get("appearance.block_fill"), Some(&json!("neutral")), "set before the trap");
    assert!(supervisor.notices().is_empty() && view.main.drawing.is_some());
}

/// A file written in the session is in the copy of the page: a worker that traps after it opens the file as it was
/// written, not as it was read (architecture 12.9). The history goes with the worker.
#[test]
fn a_file_written_survives_a_trap() {
    poison(&[]);
    let (mut supervisor, mut worker) = opened(folder(), None);
    let text = "labcraft 1\n\nuse core\n\nfn gamma(x: f64) -> (y: f64) {\n    y = core.add(x, x)\n}\n";
    command(&mut supervisor, &mut worker, "file.write", json!({ "path": "gamma.lcf", "text": text })).unwrap();
    command(&mut supervisor, &mut worker, "file.delete", json!({ "path": "beta.lcf" })).unwrap();
    // The programs: what `.labcraft` holds depends on the target (the journal stays in memory in WASM), and never the journal.
    let paths = |supervisor: &Supervisor| {
        let files = &supervisor.folder().unwrap().files;
        assert!(files.iter().all(|file| !file.path.starts_with(".labcraft/journal")), "not the journal");
        files.iter().map(|file| file.path.clone()).filter(|path| path.ends_with(".lcf")).collect::<Vec<_>>()
    };
    assert_eq!(paths(&supervisor), ["alpha.lcf", "gamma.lcf"]);
    let actions = supervisor.trap();
    drive(&mut supervisor, &mut worker, actions);
    assert_eq!(worker.started, 2);
    let functions: Vec<String> = shown(&supervisor).functions.iter().map(|listed| listed.function.clone()).collect();
    assert_eq!(functions, ["alpha", "gamma"]);
    let history = command(&mut supervisor, &mut worker, "history.list", json!({})).unwrap();
    assert!(history.get("undo").is_none_or(Value::is_null), "{history}");
}

#[test]
fn a_command_before_a_folder_fails_at_once() {
    let mut supervisor = Supervisor::new(String::new());
    let (sender, reply) = Reply::channel();
    assert!(supervisor.command("view.show_function", json!({}), sender).is_empty());
    assert_eq!(reply.take().unwrap().unwrap_err().code, "command.not_enabled");
}

#[test]
fn a_worker_that_does_not_load_is_said_and_not_blamed_on_the_folder() {
    let mut supervisor = Supervisor::new(String::new());
    assert_eq!(supervisor.open(folder(), None), [Action::Start]);
    let (sender, reply) = Reply::channel();
    assert!(supervisor.command("view.show_function", json!({ "function": "beta" }), sender).is_empty(), "it waits for the worker");
    assert!(supervisor.trapped().is_empty(), "not started again");
    assert!(!supervisor.busy());
    assert_eq!(supervisor.notices(), ["the worker of the session could not start: it stopped before it listened"]);
    assert_eq!(reply.take().unwrap().unwrap_err().code, "command.not_enabled", "what waited fails");
    assert_eq!(supervisor.open(folder(), None), [Action::Start], "another folder tries again");
    assert!(supervisor.notices().is_empty());
}

#[test]
fn a_failure_goes_to_its_reply_with_its_code() {
    poison(&[]);
    let (mut supervisor, mut worker) = opened(folder(), None);
    let failed = command(&mut supervisor, &mut worker, "view.show_function", json!({ "function": "nowhere" })).unwrap_err();
    assert_eq!(failed.code, "params.invalid", "{failed:?}");
    assert!(supervisor.notices().is_empty(), "the window says it, not the page");
    assert_eq!(shown(&supervisor).functions.len(), 2, "the listing stays when an answer has none");
}

#[test]
fn nothing_goes_to_a_worker_before_it_listens_and_one_message_at_a_time() {
    let mut supervisor = Supervisor::new(String::new());
    assert_eq!(supervisor.open(folder(), None), [Action::Start]);
    assert!(supervisor.busy());
    let (sender, _reply) = Reply::channel();
    assert!(supervisor.command("view.show_function", json!({ "function": "beta" }), sender).is_empty());
    let actions = supervisor.received(&encode(&FromWorker::Ready).unwrap());
    assert!(matches!(actions.as_slice(), [Action::Send(_)]));
    let (sender, _reply) = Reply::channel();
    assert!(supervisor.command("view.show_function", json!({ "function": "beta" }), sender).is_empty());
    assert!(supervisor.busy());
}

/// The zoom changes at each frame of a pinch: only the last one waits to be sent.
#[test]
fn only_the_last_zoom_waits() {
    poison(&[]);
    let (mut supervisor, mut worker) = opened(folder(), None);
    let (sender, _reply) = Reply::channel();
    let actions = supervisor.command("view.show_function", json!({ "function": "beta" }), sender);
    assert!(supervisor.zoom("beta", 1.5).is_empty() && supervisor.zoom("beta", 2.0).is_empty(), "the command is in flight");
    assert_eq!(supervisor.pending.iter().filter(|request| matches!(request, Request::Zoom(..))).count(), 1);
    assert!(supervisor.pending.contains(&Request::Zoom("beta".to_owned(), 2.0)));
    drive(&mut supervisor, &mut worker, actions);
    let state = command(&mut supervisor, &mut worker, "view.get_state", json!({})).unwrap();
    assert_eq!(state.pointer("/windows/0/zoom").and_then(Value::as_f64), Some(2.0), "{state}");
}

/// The window reaches the supervisor as a [`Host`]: what it sends waits for the page to carry it.
#[test]
fn the_window_sends_through_the_shared_host() {
    poison(&[]);
    let (supervisor, mut worker) = opened(folder(), None);
    let shared = Shared::new(supervisor);
    let reply = shared.send("view.show_function", json!({ "function": "beta" }));
    assert!(reply.take().is_none(), "not answered yet");
    let actions = shared.take_actions();
    assert!(matches!(actions.as_slice(), [Action::Send(_)]));
    drive(&mut shared.supervisor.borrow_mut(), &mut worker, actions);
    assert!(reply.take().unwrap().is_ok());
    assert_eq!(shared.snapshot().view.main.function.as_deref(), Some("beta"));
}

/// The window reads the history at each publication: a read publishes nothing, or the two would ask each other forever.
#[test]
fn a_read_publishes_nothing() {
    poison(&[]);
    let (mut supervisor, mut worker) = opened(folder(), None);
    let before = shown(&supervisor).generation;
    command(&mut supervisor, &mut worker, "history.list", json!({ "limit": 1 })).unwrap();
    command(&mut supervisor, &mut worker, "functions.list", json!({})).unwrap();
    assert_eq!(shown(&supervisor).generation, before);
    command(&mut supervisor, &mut worker, "view.show_function", json!({ "function": "beta" })).unwrap();
    assert!(shown(&supervisor).generation > before);
}

/// A trap, edits, then a trap again: other files, so the worker starts again, and the edits stay reachable.
#[test]
fn a_trap_after_edits_is_not_the_second_of_the_files_opened() {
    poison(&[]);
    let (mut supervisor, mut worker) = opened(folder(), None);
    let actions = supervisor.trap();
    drive(&mut supervisor, &mut worker, actions);
    let text = "labcraft 1\n\nfn gamma() -> () {}\n";
    command(&mut supervisor, &mut worker, "file.write", json!({ "path": "gamma.lcf", "text": text })).unwrap();
    let actions = supervisor.trap();
    drive(&mut supervisor, &mut worker, actions);
    assert_eq!(worker.started, 3, "started again");
    assert!(supervisor.notices().is_empty(), "{:?}", supervisor.notices());
    assert!(command(&mut supervisor, &mut worker, "view.show_function", json!({ "function": "gamma" })).is_ok());
}

/// Undoing a write gives the page back the file as it was, byte for byte (scenario 8, point 2), and the project still
/// counts as changed: its files were written.
#[test]
fn an_undo_gives_the_copy_back_its_bytes() {
    poison(&[]);
    let (mut supervisor, mut worker) = opened(folder(), None);
    assert!(!supervisor.edited());
    let before = supervisor.folder().unwrap().files.iter().find(|file| file.path == "alpha.lcf").cloned().unwrap();
    let text = String::from_utf8(before.bytes.clone()).unwrap().replace("core.add(x, x)", "core.multiply(x, 2.0)");
    command(&mut supervisor, &mut worker, "file.write", json!({ "path": "alpha.lcf", "text": text })).unwrap();
    assert!(supervisor.edited());
    command(&mut supervisor, &mut worker, "history.undo", json!({})).unwrap();
    let after = supervisor.folder().unwrap().files.iter().find(|file| file.path == "alpha.lcf").cloned().unwrap();
    assert_eq!(after, before);
}

/// An answer that is not the one of the command in flight (a message the worker could not read) still ends it.
#[test]
fn a_command_is_answered_even_by_a_message_the_worker_could_not_read() {
    poison(&[]);
    let (mut supervisor, _) = opened(folder(), None);
    let (sender, reply) = Reply::channel();
    assert!(matches!(supervisor.command("view.show_function", json!({ "function": "beta" }), sender).as_slice(), [Action::Send(_)]));
    let unread = FromWorker::Answer(Box::new(crate::protocol::Answer::failed("params.invalid", "unreadable".to_owned())));
    supervisor.received(&encode(&unread).unwrap());
    assert_eq!(reply.take().unwrap().unwrap_err().code, "internal");
    assert!(!supervisor.busy());
    assert_eq!(shown_function(&supervisor).as_deref(), Some("alpha"), "what was shown stays");
}

/// A folder opened again under the same name is another session for the window, which starts afresh.
#[test]
fn each_folder_opened_is_another_session() {
    poison(&[]);
    let (mut supervisor, mut worker) = opened(folder(), None);
    let first = supervisor.snapshot().session;
    let actions = supervisor.open(folder(), None);
    drive(&mut supervisor, &mut worker, actions);
    let second = supervisor.snapshot();
    assert_eq!(second.project.as_deref(), Some("bench"));
    assert_ne!(second.session, first);
}

/// A write whose drawing traps: the page has the file as written, from before the drawing, and the function is not
/// drawn again.
#[test]
fn a_write_whose_drawing_traps_is_kept() {
    poison(&[]);
    let (mut supervisor, mut worker) = opened(folder(), None);
    command(&mut supervisor, &mut worker, "view.show_function", json!({ "function": "beta" })).unwrap();
    poison(&["beta"]);
    let text = "labcraft 1\n\nuse core\n\nfn beta(x: f64) -> (y: f64) {\n    y = core.multiply(x, 3.0)\n}\n";
    let failed = command(&mut supervisor, &mut worker, "file.write", json!({ "path": "beta.lcf", "text": text })).unwrap_err();
    assert!(failed.message.contains("LC0099"), "{failed:?}");
    assert_eq!(worker.started, 2);
    let beta = supervisor.folder().unwrap().files.iter().find(|file| file.path == "beta.lcf").cloned().unwrap();
    assert_eq!(beta.bytes, text.as_bytes());
    assert!(supervisor.edited());
    assert!(supervisor.notices().iter().any(|notice| notice.starts_with("LC0099: drawing `beta`")), "{:?}", supervisor.notices());
}

/// A failure of the page is said until a message succeeds.
#[test]
fn a_failure_of_the_page_is_said_until_a_message_succeeds() {
    poison(&[]);
    let (mut supervisor, mut worker) = opened(folder(), None);
    let unread = FromWorker::Answer(Box::new(crate::protocol::Answer::failed("params.invalid", "unreadable".to_owned())));
    supervisor.received(&encode(&unread).unwrap());
    assert_eq!(supervisor.notices(), ["params.invalid: unreadable"]);
    command(&mut supervisor, &mut worker, "view.show_function", json!({ "function": "beta" })).unwrap();
    assert!(supervisor.notices().is_empty());
}

/// A trap restarts the worker with the detached windows (ide.md 9): each comes back with its function, and one whose
/// drawing trapped comes back not drawn, with `LC0099`.
#[test]
fn a_trap_restarts_the_worker_with_the_detached_windows() {
    poison(&[]);
    let (mut supervisor, mut worker) = opened(folder(), None);
    command(&mut supervisor, &mut worker, "view.show_function", json!({ "function": "beta" })).unwrap();
    command(&mut supervisor, &mut worker, "view.detach", json!({ "function": "beta" })).unwrap();
    let view = shown(&supervisor);
    assert!(view.is_detached("beta") && view.window_of("beta").is_some_and(|window| window.drawing.is_some()));
    let actions = supervisor.trap();
    drive(&mut supervisor, &mut worker, actions);
    assert_eq!(worker.started, 2);
    let view = shown(&supervisor);
    assert!(view.is_detached("beta") && view.window_of("beta").is_some_and(|window| window.drawing.is_some()), "{:?}", view.opened);
    assert_eq!((view.main.function.as_deref(), view.opened.as_slice()), (Some("alpha"), ["alpha".to_owned()].as_slice()));
    // Its drawing traps: it comes back not drawn, and its window says so.
    poison(&["beta"]);
    command(&mut supervisor, &mut worker, "view.set_standard_view", json!({ "on": true })).ok();
    let view = shown(&supervisor);
    assert!(view.is_detached("beta"), "{:?}", view.opened);
    let notices = supervisor.notices();
    assert!(notices.iter().any(|notice| notice.starts_with("LC0099: drawing `beta`")), "{notices:?}");
}

/// `Download` gives the zip of the worker once; a trap while it zips fails that download, not the files, which the new
/// worker opens as they were written, and a download after it works.
#[test]
fn a_download_gives_its_zip_and_a_trap_while_zipping_keeps_the_files() {
    poison(&[]);
    let mut nothing = Supervisor::default();
    assert!(nothing.download(Stamp::default()).is_empty());
    assert!(nothing.take_download().is_some_and(|zip| zip.is_err()), "no folder");
    let (mut supervisor, mut worker) = opened(folder(), None);
    let text = String::from_utf8(function("alpha").bytes).unwrap().replace("core.add(x, x)", "core.add(x, 1.0)");
    command(&mut supervisor, &mut worker, "file.write", json!({ "path": "alpha.lcf", "text": text })).unwrap();
    let actions = supervisor.download(Stamp::default());
    drive(&mut supervisor, &mut worker, actions);
    let zip = supervisor.take_download().unwrap().unwrap();
    assert!(zip.starts_with(b"PK\x03\x04"));
    assert_eq!(supervisor.take_download(), None, "taken once");
    // The worker traps while it writes the zip.
    let actions = supervisor.download(Stamp::default());
    assert!(matches!(actions.as_slice(), [Action::Send(_)]));
    let actions = supervisor.trapped();
    let why = supervisor.take_download().unwrap().unwrap_err();
    assert!(why.contains("LC0099"), "{why}");
    drive(&mut supervisor, &mut worker, actions);
    assert_eq!(worker.started, 2);
    let alpha = supervisor.folder().and_then(|folder| folder.files.iter().find(|file| file.path == "alpha.lcf")).unwrap();
    assert_eq!(alpha.bytes, text.as_bytes(), "the files stay");
    assert!(supervisor.notices().is_empty(), "{:?}", supervisor.notices());
    let actions = supervisor.download(Stamp::default());
    drive(&mut supervisor, &mut worker, actions);
    assert!(supervisor.take_download().is_some_and(|zip| zip.is_ok()));
}

/// A zip saved counts the files as saved, until they change again; a download that fails changes nothing.
#[test]
fn a_zip_saved_counts_the_project_as_saved_until_it_changes() {
    poison(&[]);
    let (mut supervisor, mut worker) = opened(folder(), None);
    assert!(!supervisor.saved(), "nothing saved yet");
    let text = String::from_utf8(function("alpha").bytes).unwrap().replace("core.add(x, x)", "core.add(x, 1.0)");
    command(&mut supervisor, &mut worker, "file.write", json!({ "path": "alpha.lcf", "text": text })).unwrap();
    let actions = supervisor.download(Stamp::default());
    drive(&mut supervisor, &mut worker, actions);
    assert!(supervisor.take_download().is_some_and(|zip| zip.is_ok()));
    assert!(!supervisor.saved(), "a zip not saved yet");
    supervisor.mark_saved();
    assert!(supervisor.saved() && supervisor.edited());
    let again = text.replace("1.0", "2.0");
    command(&mut supervisor, &mut worker, "file.write", json!({ "path": "alpha.lcf", "text": again })).unwrap();
    assert!(!supervisor.saved(), "a new change");
    // A download that fails: still not saved.
    let actions = supervisor.download(Stamp::default());
    let actions = match actions.as_slice() {
        [Action::Send(_)] => supervisor.trapped(),
        _ => panic!("{actions:?}"),
    };
    assert!(supervisor.take_download().is_some_and(|zip| zip.is_err()));
    supervisor.mark_saved();
    assert!(!supervisor.saved(), "the zip saved before is not the files now");
    drive(&mut supervisor, &mut worker, actions);
}

/// A download asked while another is written is refused and said; the first one still comes.
#[test]
fn a_second_download_meanwhile_is_said() {
    poison(&[]);
    let (mut supervisor, mut worker) = opened(folder(), None);
    let first = supervisor.download(Stamp::default());
    assert!(matches!(first.as_slice(), [Action::Send(_)]));
    assert!(supervisor.download(Stamp::default()).is_empty(), "nothing more is sent");
    let why = supervisor.take_download().unwrap().unwrap_err();
    assert!(why.starts_with("Download: ") && why.contains("being written"), "{why}");
    drive(&mut supervisor, &mut worker, first);
    assert!(supervisor.take_download().is_some_and(|zip| zip.is_ok()));
    assert_eq!(supervisor.take_download(), None);
}

/// Another folder opened, or an answer that does not read, while the zip is written: the download fails, and the
/// page still has it to say after the new folder.
#[test]
fn a_download_lost_to_another_folder_or_to_an_unread_answer_is_said() {
    poison(&[]);
    let (mut supervisor, _worker) = opened(folder(), None);
    assert!(matches!(supervisor.download(Stamp::default()).as_slice(), [Action::Send(_)]));
    let actions = supervisor.open(folder(), None);
    assert_eq!(actions, [Action::Start]);
    let why = supervisor.take_download().unwrap().unwrap_err();
    assert!(why.starts_with("Download: ") && why.contains("another folder"), "{why}");
    let (mut supervisor, _worker) = opened(folder(), None);
    assert!(matches!(supervisor.download(Stamp::default()).as_slice(), [Action::Send(_)]));
    supervisor.received(b"\xff not postcard");
    let why = supervisor.take_download().unwrap().unwrap_err();
    assert!(why.starts_with("Download: "), "{why}");
    assert!(!supervisor.busy());
}
