//! The times of the viewer through its watchers and the thread of the session, without a window (scenario 6, point 3):
//! a change to a new drawing within 200 ms *(proposed)*. They measure the clock, so they run alone: in a binary of their
//! own, which `cargo test` runs apart since it runs one binary at a time, one test at a time (`ALONE`); under
//! cargo-nextest, each with every thread of the run (`.config/nextest.toml`). Beside the other tests, on a runner of the
//! CI, they took 400 ms.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use labcraft::session::Handle;

mod common;
use common::{Folder, ONE, TWO};

/// Held by each test, so that two of them never run at the same time.
static ALONE: Mutex<()> = Mutex::new(());

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
/// a window (scenario 6, point 3): the slowest of three changes.
#[test]
#[expect(clippy::disallowed_methods, reason = "a native test, never built for WASM, where the clock traps")]
fn a_change_of_the_file_is_drawn_within_200_ms() {
    let _alone = ALONE.lock().unwrap_or_else(PoisonError::into_inner);
    let folder = Folder::new(&[("main.lcf", ONE)]);
    let session = Handle::spawn(false, Vec::new(), None, Box::new(|| {})).unwrap();
    let opened = session.open(&folder.0).recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(opened.is_ok(), "{opened:?}");
    assert!(session.published().notice.is_none(), "{:?}", session.published().notice);
    // The watcher starts with the folder: let it settle.
    std::thread::sleep(Duration::from_millis(300));
    let mut slowest = Duration::ZERO;
    for text in [TWO, ONE, TWO] {
        let before = session.published().view.drawing_generation;
        let start = Instant::now();
        std::fs::write(folder.0.join("main.lcf"), text).unwrap();
        loop {
            if session.published().view.drawing_generation != before {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "no new drawing");
            std::thread::sleep(Duration::from_millis(2));
        }
        slowest = slowest.max(start.elapsed());
        let nodes = session.published().view.drawing.as_ref().unwrap().placed.len();
        assert_eq!(nodes, if text == TWO { 2 } else { 1 });
        assert!(session.published().view.glide);
        // The next change starts once this one is settled.
        std::thread::sleep(Duration::from_millis(200));
    }
    println!("file change to new drawing: {} ms at most", slowest.as_millis());
    assert!(slowest < Duration::from_millis(200), "{slowest:?}");
}

/// Another program file written without end holds no change back: the 50 ms do not move (architecture 11.5); a
/// `--lib-path` that cannot be watched leaves the folder watched.
#[test]
#[expect(clippy::disallowed_methods, reason = "a native test, never built for WASM, where the clock traps")]
fn a_busy_log_and_a_missing_library_path_hold_nothing_back() {
    let _alone = ALONE.lock().unwrap_or_else(PoisonError::into_inner);
    let folder = Folder::new(&[("main.lcf", ONE)]);
    let missing = folder.0.join("no-such-library");
    let session = Handle::spawn(false, vec![missing.clone()], None, Box::new(|| {})).unwrap();
    let opened = session.open(&folder.0).recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(opened.is_ok(), "{opened:?}");
    let notice = session.published().notice.unwrap_or_default();
    // Only the missing path is looked at every second: one path in the notice.
    assert!(notice.contains("no-such-library") && notice.matches('`').count() == 2, "{notice}");
    std::thread::sleep(Duration::from_millis(300));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer = {
        let (stop, other) = (stop.clone(), folder.0.join("other.lcf"));
        std::thread::spawn(move || {
            let mut line = 0u64;
            while !stop.load(Ordering::Relaxed) {
                line += 1;
                let _ = std::fs::write(&other, format!("labcraft 1\n\n/// {line}\nfn other() -> () {{\n}}\n"));
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    };
    std::thread::sleep(Duration::from_millis(200));
    let start = Instant::now();
    std::fs::write(folder.0.join("main.lcf"), TWO).unwrap();
    // Each write of `other.lcf` draws again too, sometimes before `main.lcf` is read: what counts is the first drawing
    // that shows the change of `main.lcf`, its two nodes, not the first new drawing.
    while session.published().view.drawing.as_ref().map_or(0, |drawing| drawing.placed.len()) != 2 {
        assert!(start.elapsed() < Duration::from_secs(5), "`main.lcf` not drawn again while the log is written");
        std::thread::sleep(Duration::from_millis(2));
    }
    let took = start.elapsed();
    stop.store(true, Ordering::Relaxed);
    let _ = writer.join();
    assert!(took < Duration::from_millis(200), "{took:?}");
}

/// A file of values saved to a new drawing within 200 ms *(proposed)*, through its watcher and the thread of the session:
/// the slowest of three saves.
#[test]
#[expect(clippy::disallowed_methods, reason = "a native test, never built for WASM, where the clock traps")]
fn a_file_of_values_saved_is_drawn_within_200_ms() {
    let _alone = ALONE.lock().unwrap_or_else(PoisonError::into_inner);
    let folder = Folder::new(&[("main.lcf", TWO)]);
    let values = Folder::new(&[]);
    let path = values.0.join("notation.toml");
    let session = Handle::spawn(false, Vec::new(), Some(path.clone()), Box::new(|| {})).unwrap();
    assert!(path.exists());
    assert_eq!(session.published().style_notice, None);
    let opened = session.open(&folder.0).recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(opened.is_ok(), "{opened:?}");
    std::thread::sleep(Duration::from_millis(300));
    let mut slowest = Duration::ZERO;
    for margin in [24, 40, 16] {
        let before = session.published().view.drawing_generation;
        let start = Instant::now();
        std::fs::write(&path, format!("[canvas]\nmargin = {margin}\n")).unwrap();
        loop {
            if session.published().view.drawing_generation != before {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "no new drawing");
            std::thread::sleep(Duration::from_millis(2));
        }
        slowest = slowest.max(start.elapsed());
        assert_eq!(session.published().view.drawing.as_ref().unwrap().style.numbers.margin, margin * 100);
        std::thread::sleep(Duration::from_millis(200));
    }
    println!("file of values saved to new drawing: {} ms at most", slowest.as_millis());
    assert!(slowest < Duration::from_millis(200), "{slowest:?}");
}
