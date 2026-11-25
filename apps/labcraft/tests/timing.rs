//! The times of the viewer through its watchers and the thread of the session, without a window (scenario 6, point 3):
//! a change to a new drawing within 200 ms *(proposed)*. They measure the clock, so they run alone: in a binary of their
//! own, which `cargo test` runs apart since it runs one binary at a time, one test at a time (`ALONE`); under
//! cargo-nextest, each with every thread of the run (`.config/nextest.toml`). Beside the other tests, on a runner of the
//! CI, they took 400 ms. A shared runner is still slow now and then, with no change of the code measured: each test
//! meets the 200 ms in one of three attempts (`within_budget`), and its checks in every attempt.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use labcraft::session::{Handle, Opening};
use labcraft_ide::Host;

mod common;
use common::{Folder, ONE, TWO};

/// Held by each test, so that two of them never run at the same time.
static ALONE: Mutex<()> = Mutex::new(());

/// A change to a new drawing *(proposed)*.
const BUDGET: Duration = Duration::from_millis(200);

/// The attempts of a test before it fails.
const ATTEMPTS: usize = 3;

/// The wait before an attempt after the first, so that it does not start in the same slow moment of the runner.
const BETWEEN: Duration = Duration::from_secs(1);

/// The filter of `.config/nextest.toml` names this binary: renamed or moved, the tests would run beside the others again
/// without a word from nextest.
#[test]
fn the_runner_still_runs_these_tests_alone() {
    let config = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.config/nextest.toml")).unwrap();
    let filter = format!("filter = 'package({}) & binary({})'", env!("CARGO_PKG_NAME"), env!("CARGO_CRATE_NAME"));
    assert!(config.contains(&filter), "`.config/nextest.toml` has no `{filter}`");
    assert!(config.contains("threads-required = 'num-test-threads'"));
}

/// A file change to a new drawing within 200 ms *(proposed)*, through the watcher and the thread of the session, without
/// a window (scenario 6, point 3): the slowest of three changes, in one of three attempts.
#[test]
fn a_change_of_the_file_is_drawn_within_200_ms() {
    within_budget("file change to new drawing, the slowest of three", three_changes_of_the_file);
}

/// One attempt: a folder, a session, three writes of `main.lcf`; the slowest change.
#[expect(clippy::disallowed_methods, reason = "a native test, never built for WASM, where the clock traps")]
fn three_changes_of_the_file() -> Duration {
    let folder = Folder::new(&[("main.lcf", ONE)]);
    let session = spawn(&folder.0, Vec::new(), None);
    assert!(session.snapshot().notices.is_empty(), "{:?}", session.snapshot().notices);
    // The watcher starts with the folder: let it settle.
    std::thread::sleep(Duration::from_millis(300));
    let mut slowest = Duration::ZERO;
    for text in [TWO, ONE, TWO] {
        let before = session.snapshot().view.main.drawing_generation;
        let start = Instant::now();
        std::fs::write(folder.0.join("main.lcf"), text).unwrap();
        loop {
            if session.snapshot().view.main.drawing_generation != before {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "no new drawing");
            std::thread::sleep(Duration::from_millis(2));
        }
        slowest = slowest.max(start.elapsed());
        let nodes = session.snapshot().view.main.drawing.as_ref().unwrap().placed.len();
        assert_eq!(nodes, if text == TWO { 2 } else { 1 });
        assert!(session.snapshot().view.main.glide);
        // The next change starts once this one is settled.
        std::thread::sleep(Duration::from_millis(200));
    }
    slowest
}

/// Another program file written without end holds no change back: the 50 ms do not move (architecture 11.5); a
/// `--lib-path` that cannot be watched leaves the folder watched. In one of three attempts.
#[test]
fn a_busy_log_and_a_missing_library_path_hold_nothing_back() {
    within_budget("change drawn while another file is written", a_change_beside_a_busy_log);
}

/// One attempt: a folder, a session with a missing library path, a writer of `other.lcf`; the time until `main.lcf` is
/// drawn again.
#[expect(clippy::disallowed_methods, reason = "a native test, never built for WASM, where the clock traps")]
fn a_change_beside_a_busy_log() -> Duration {
    let folder = Folder::new(&[("main.lcf", ONE)]);
    let missing = folder.0.join("no-such-library");
    let session = spawn(&folder.0, vec![missing.clone()], None);
    let notice = session.snapshot().notices.join("; ");
    // Only the missing path is looked at every second: one path in the notice.
    assert!(notice.contains("no-such-library") && notice.matches('`').count() == 2, "{notice}");
    std::thread::sleep(Duration::from_millis(300));
    let _writer = Writer::start(folder.0.join("other.lcf"));
    std::thread::sleep(Duration::from_millis(200));
    let start = Instant::now();
    std::fs::write(folder.0.join("main.lcf"), TWO).unwrap();
    // Each write of `other.lcf` draws again too, sometimes before `main.lcf` is read: what counts is the first drawing
    // that shows the change of `main.lcf`, its two nodes, not the first new drawing.
    while session.snapshot().view.main.drawing.as_ref().map_or(0, |drawing| drawing.placed.len()) != 2 {
        assert!(start.elapsed() < Duration::from_secs(5), "`main.lcf` not drawn again while the log is written");
        std::thread::sleep(Duration::from_millis(2));
    }
    start.elapsed()
}

/// A file of values saved to a new drawing within 200 ms *(proposed)*, through its watcher and the thread of the session:
/// the slowest of three saves, in one of three attempts.
#[test]
fn a_file_of_values_saved_is_drawn_within_200_ms() {
    within_budget("file of values saved to new drawing, the slowest of three", three_saves_of_values);
}

/// One attempt: a folder, a session, three saves of `notation.toml`; the slowest change.
#[expect(clippy::disallowed_methods, reason = "a native test, never built for WASM, where the clock traps")]
fn three_saves_of_values() -> Duration {
    let folder = Folder::new(&[("main.lcf", TWO)]);
    let values = Folder::new(&[]);
    let path = values.0.join("notation.toml");
    let session = spawn(&folder.0, Vec::new(), Some(path.clone()));
    assert!(path.exists());
    assert!(session.snapshot().notices.is_empty());
    std::thread::sleep(Duration::from_millis(300));
    let mut slowest = Duration::ZERO;
    for margin in [24, 40, 16] {
        let before = session.snapshot().view.main.drawing_generation;
        let start = Instant::now();
        std::fs::write(&path, format!("[canvas]\nmargin = {margin}\n")).unwrap();
        loop {
            if session.snapshot().view.main.drawing_generation != before {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "no new drawing");
            std::thread::sleep(Duration::from_millis(2));
        }
        slowest = slowest.max(start.elapsed());
        assert_eq!(session.snapshot().view.main.drawing.as_ref().unwrap().style.numbers.margin, margin * 100);
        std::thread::sleep(Duration::from_millis(200));
    }
    slowest
}

/// Up to [`ATTEMPTS`] of `attempt`, [`BETWEEN`] apart, each printed, holding [`ALONE`]: passes as soon as one is under
/// [`BUDGET`], and fails with the time of each when none is. Only the time has another try: a failed check of an attempt
/// fails the test.
fn within_budget(what: &str, attempt: fn() -> Duration) {
    let _alone = ALONE.lock().unwrap_or_else(PoisonError::into_inner);
    let mut times = Vec::new();
    for count in 1..=ATTEMPTS {
        if count > 1 {
            std::thread::sleep(BETWEEN);
        }
        let took = attempt();
        println!("{what}: {} ms, attempt {count} of {ATTEMPTS}", took.as_millis());
        times.push(took);
        if took < BUDGET {
            return;
        }
    }
    panic!("{what}: {BUDGET:?} or more in each of {ATTEMPTS} attempts: {times:?}");
}

/// A thread that writes `other.lcf` every 5 ms, a line more each time, until it is dropped: then it stops and is waited
/// for, whether the attempt returned or failed a check.
struct Writer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Writer {
    fn start(other: std::path::PathBuf) -> Writer {
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut line = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    line += 1;
                    let _ = std::fs::write(&other, format!("labcraft 1\n\n/// {line}\nfn other() -> () {{\n}}\n"));
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
        };
        Writer { stop, thread: Some(thread) }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The session of a project on its thread, opened.
fn spawn(folder: &std::path::Path, lib_paths: Vec<std::path::PathBuf>, style: Option<std::path::PathBuf>) -> Handle {
    let opening = Opening {
        folder: folder.to_path_buf(),
        lib_paths,
        user_settings: serde_json::Map::new(),
        style,
        new: false,
        function: None,
        gesture_wait: labcraft_engine::gesture::WAIT,
    };
    let (handle, reply) = Handle::spawn(opening, Box::new(|| {})).unwrap();
    let opened = reply.wait(Duration::from_secs(30)).unwrap();
    assert!(opened.is_ok(), "{opened:?}");
    handle
}
