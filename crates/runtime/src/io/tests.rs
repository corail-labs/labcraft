//! The table of io.md 3 and the rights of io.md 4, read from the documents; runs through the scheduler are in `run::tests`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::{Allowed, CATEGORIES, Consent, Context, Judged, Open, Question, Reach, Rights, StartedBy, open_real};
use crate::native::{Mode, NativeOutcome};

const IO: &str = include_str!("../../../../docs/io.md");

fn endpoint(name: &str) -> Reach {
    Reach::Endpoint { endpoint: name.to_owned(), given: name.to_owned() }
}

fn agent(allowed: Allowed) -> Context {
    Context {
        started_by: StartedBy::Agent { client: "client".to_owned(), rights: Rights::of_run(allowed, "r1", "client", None) },
        ..Context::default()
    }
}

fn category(outcome: &NativeOutcome) -> (&'static str, i64) {
    match outcome {
        NativeOutcome::Fail { category, code, .. } => (category, *code),
        NativeOutcome::Outputs(_) => ("", -1),
    }
}

#[test]
fn the_categories_are_those_of_the_table_of_io_md_8() {
    let section = IO.split("## 8. Categories").nth(1).unwrap().split("\n## ").next().unwrap();
    let rows: Vec<&str> = section
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|rest| rest.split('`').next())
        .filter(|name| name.starts_with("io."))
        .collect();
    assert_eq!(rows, CATEGORIES);
}

#[test]
fn open_real_follows_the_table_of_io_md_3() {
    let tcp = endpoint("tcp:10.0.0.5:5025");
    let file = Context { simulation_file: true, ..Context::default() };
    // With `--sim FILE`: the device of the file, or not found; never the hardware; in both modes.
    for mode in [Mode::Deterministic, Mode::RealTime] {
        assert!(matches!(open_real(mode, &file, true, &tcp), Open::Simulated));
        assert!(matches!(open_real(mode, &file, false, &tcp), Open::NotFound));
    }
    // Without it: the hardware in real time; `io.needs_real_time`, code 0, in deterministic mode.
    assert!(matches!(open_real(Mode::RealTime, &Context::default(), false, &tcp), Open::Real));
    let Open::Fail(outcome) = open_real(Mode::Deterministic, &Context::default(), false, &tcp) else { panic!() };
    assert_eq!(category(&outcome), ("io.needs_real_time", 0));
}

#[test]
fn an_agent_reaches_what_it_was_allowed_and_nothing_else() {
    let allowed = Allowed { endpoints: ["tcp:10.0.0.5:502".to_owned()].into(), paths: vec![PathBuf::from("/data/bench")], consent: None };
    let context = agent(allowed);
    assert!(matches!(context.judge(&endpoint("tcp:10.0.0.5:502")), Judged::Allowed));
    // Names are exact (io.md 4): another port, or the same address by another name, is another endpoint.
    for other in ["tcp:10.0.0.5:503", "tcp:localhost:502"] {
        let Judged::NotAllowed(outcome) = context.judge(&endpoint(other)) else { panic!("{other}") };
        assert_eq!(category(&outcome), ("io.not_allowed", 0));
        let NativeOutcome::Fail { message, .. } = outcome else { panic!() };
        assert!(message.contains(&format!("--allow-hardware {other}")), "{message}");
    }
    // A folder of `--allow-path` and what is under it; not a folder beside it whose name starts the same.
    assert!(matches!(context.judge(&Reach::Path(PathBuf::from("/data/bench/run1.csv"))), Judged::Allowed));
    let Judged::NotAllowed(outcome) = context.judge(&Reach::Path(PathBuf::from("/data/bench2/run1.csv"))) else { panic!() };
    let NativeOutcome::Fail { message, .. } = outcome else { panic!() };
    assert!(message.contains("--allow-path"), "{message}");
    // The human has every right.
    assert!(matches!(Context::default().judge(&endpoint("tcp:10.0.0.6:1")), Judged::Allowed));
}

/// Answers the question as it is told, counting the questions; `panics` makes the host panic instead.
struct Window {
    yes: bool,
    panics: bool,
    asked: AtomicUsize,
}

impl Consent for Window {
    fn ask(&self, question: &Question<'_>) -> bool {
        // The question names the run and the client (io.md 4), which the host shows, and by which it takes it away.
        assert_eq!((question.run, question.client), ("r1", "client"));
        self.asked.fetch_add(1, Ordering::SeqCst);
        assert!(!self.panics, "the host panics, as the test asks");
        self.yes
    }
}

fn asking(yes: bool, panics: bool) -> (Context, Arc<Window>) {
    let window = Arc::new(Window { yes, panics, asked: AtomicUsize::new(0) });
    let context = agent(Allowed { consent: Some(Arc::clone(&window) as Arc<dyn Consent>), ..Allowed::default() });
    (context, window)
}

/// Runs the work of a question, as the runtime would, and reads its answer.
fn answer(context: &Context, reach: &Reach) -> Result<(), NativeOutcome> {
    match context.judge(reach) {
        Judged::Allowed => Ok(()),
        Judged::NotAllowed(outcome) => Err(outcome),
        Judged::Ask(job) => context.answered(reach, job()),
    }
}

#[test]
fn an_answer_holds_for_the_endpoint_for_the_rest_of_the_run() {
    for yes in [true, false] {
        let (context, window) = asking(yes, false);
        let tcp = endpoint("tcp:10.0.0.5:5025");
        // Through another name of one endpoint (a VISA name), the answer is the same: it holds for the endpoint (io.md 2, 4).
        let visa = Reach::Endpoint { endpoint: "tcp:10.0.0.5:5025".to_owned(), given: "TCPIP0::10.0.0.5::5025::SOCKET".to_owned() };
        assert_eq!(answer(&context, &tcp).is_ok(), yes);
        assert_eq!(answer(&context, &visa).is_ok(), yes);
        assert_eq!(window.asked.load(Ordering::SeqCst), 1);
        // A refusal is `io.not_allowed`, code 0, and says it was refused in the window.
        if !yes {
            let outcome = answer(&context, &tcp).unwrap_err();
            assert_eq!(category(&outcome), ("io.not_allowed", 0));
            let NativeOutcome::Fail { message, .. } = outcome else { panic!() };
            assert!(message.contains("window"), "{message}");
        }
        // Another run starts with no answer.
        let again = agent(Allowed { consent: Some(Arc::clone(&window) as Arc<dyn Consent>), ..Allowed::default() });
        let _ = answer(&again, &tcp);
        assert_eq!(window.asked.load(Ordering::SeqCst), 2);
    }
}

#[test]
fn a_question_asked_while_another_waits_waits_for_its_answer() {
    // A host whose window answers once the test says so: the second question, asked while the first waits, is not asked.
    struct Slow {
        asked: AtomicUsize,
        answer: Mutex<()>,
    }
    impl Consent for Slow {
        fn ask(&self, _: &Question<'_>) -> bool {
            self.asked.fetch_add(1, Ordering::SeqCst);
            let _held = self.answer.lock().unwrap();
            true
        }
    }
    let slow = Arc::new(Slow { asked: AtomicUsize::new(0), answer: Mutex::new(()) });
    let context = agent(Allowed { consent: Some(Arc::clone(&slow) as Arc<dyn Consent>), ..Allowed::default() });
    let tcp = endpoint("tcp:10.0.0.5:5025");
    let (Judged::Ask(first), Judged::Ask(second)) = (context.judge(&tcp), context.judge(&tcp)) else { panic!() };
    let held = slow.answer.lock().unwrap();
    let one = std::thread::spawn(first);
    while slow.asked.load(Ordering::SeqCst) == 0 {
        std::thread::yield_now();
    }
    let two = std::thread::spawn(second);
    // The second waits for the answer to the first before the window answers.
    let StartedBy::Agent { rights, .. } = &context.started_by else { panic!() };
    while rights.waits() == 0 {
        std::thread::yield_now();
    }
    drop(held);
    assert!(context.answered(&tcp, one.join().unwrap()).is_ok());
    assert!(context.answered(&tcp, two.join().unwrap()).is_ok());
    assert_eq!(slow.asked.load(Ordering::SeqCst), 1);
}

#[test]
fn a_host_that_panics_while_it_asks_answers_no() {
    let (context, window) = asking(true, true);
    let tcp = endpoint("tcp:10.0.0.5:5025");
    let Judged::Ask(job) = context.judge(&tcp) else { panic!() };
    // The runtime catches the panic of a job (10.4); the question is no then, and is not asked again.
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).is_err());
    assert!(matches!(context.judge(&tcp), Judged::NotAllowed(_)));
    assert_eq!(window.asked.load(Ordering::SeqCst), 1);
}

#[test]
fn what_is_not_an_answer_is_no() {
    let (context, _) = asking(true, false);
    let tcp = endpoint("tcp:10.0.0.5:5025");
    let outcome = context.answered(&tcp, Box::new(42_u64)).unwrap_err();
    assert_eq!(category(&outcome), ("io.not_allowed", 0));
}

#[test]
fn an_answer_for_a_folder_holds_for_what_is_under_it() {
    let (context, window) = asking(true, false);
    // io.md 4: the human allows folder by folder; a file under a folder allowed is allowed, a folder beside it is asked.
    assert!(answer(&context, &Reach::Path(PathBuf::from("/data"))).is_ok());
    assert!(answer(&context, &Reach::Path(PathBuf::from("/data/a.csv"))).is_ok());
    assert!(answer(&context, &Reach::Path(PathBuf::from("/data/runs/b.csv"))).is_ok());
    assert_eq!(window.asked.load(Ordering::SeqCst), 1);
    assert!(answer(&context, &Reach::Path(PathBuf::from("/database/c.csv"))).is_ok());
    assert_eq!(window.asked.load(Ordering::SeqCst), 2);
}

#[test]
fn a_file_under_a_folder_whose_question_waits_waits_for_its_answer() {
    // A window that answers once the test lets it.
    struct Slow {
        asked: AtomicUsize,
        answer: Mutex<()>,
    }
    impl Consent for Slow {
        fn ask(&self, _: &Question<'_>) -> bool {
            self.asked.fetch_add(1, Ordering::SeqCst);
            let _held = self.answer.lock().unwrap();
            true
        }
    }
    let slow = Arc::new(Slow { asked: AtomicUsize::new(0), answer: Mutex::new(()) });
    let context = agent(Allowed { consent: Some(Arc::clone(&slow) as Arc<dyn Consent>), ..Allowed::default() });
    let StartedBy::Agent { rights, .. } = &context.started_by else { panic!() };
    let (folder, file) = (Reach::Path(PathBuf::from("/data")), Reach::Path(PathBuf::from("/data/a.csv")));
    let (Judged::Ask(first), Judged::Ask(second)) = (context.judge(&folder), context.judge(&file)) else { panic!() };
    // io.md 4, folder by folder: while the question of `/data` waits, one for a file under it waits for that answer.
    let held = slow.answer.lock().unwrap();
    let one = std::thread::spawn(first);
    while slow.asked.load(Ordering::SeqCst) == 0 {
        std::thread::yield_now();
    }
    let two = std::thread::spawn(second);
    while rights.waits() == 0 {
        std::thread::yield_now();
    }
    drop(held);
    assert!(context.answered(&folder, one.join().unwrap()).is_ok());
    assert!(context.answered(&file, two.join().unwrap()).is_ok());
    assert_eq!(slow.asked.load(Ordering::SeqCst), 1);
}
