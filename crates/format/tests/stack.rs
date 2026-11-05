//! The stack that the work on a file takes at the ceiling of nesting (`docs/architecture.md`, section 4):
//! parsing, `Clone`, `Eq` and `Drop`, canonical order, `canonicalize` and `write`, which all recurse on the model.
//!
//! A stack overflow ends the whole process, so each try runs in a child process: this test binary again,
//! told what to do by an environment variable.

mod deep;

use std::process::{Command, Stdio};

use deep::DEEP;
use labcraft_format::{STACK_SIZE, canonicalize, read, write};
use labcraft_types::{Interner, Limits, MAX_NESTING, Report};

const TRY: &str = "LABCRAFT_STACK_TRY";
const CHILD: &str = "the_child_process_of_a_try";

/// Below this, the work must overflow. Without this, a child that does nothing would look like a low measure.
const TOO_SMALL: usize = 64 * 1024;

const PRECISION: usize = 16 * 1024;

fn work_at_the_ceiling(text: &str) {
    // At the ceiling, the indentation alone can pass the default limit of size, and `write` must really write.
    let mut limits = Limits::default().with_max_nesting(MAX_NESTING).unwrap();
    limits.max_file_size = u64::MAX;
    let mut names = Interner::new();
    let result = read(text.as_bytes(), &mut names, &limits);
    let file = result.file.unwrap_or_else(|| panic!("{:?}", result.report.diagnostics()));
    let copy = file.clone();
    assert!(file == copy);
    let canonical = canonicalize(file, &result.source_map, &mut names, &mut Report::new(1));
    assert!(write(&canonical, &names, &limits).is_ok());
}

/// Does nothing unless a parent process runs it.
#[test]
fn the_child_process_of_a_try() {
    let Ok(request) = std::env::var(TRY) else { return };
    let (kind, bytes) = request.split_once('=').unwrap();
    let (_, program) = DEEP.iter().find(|(name, _)| *name == kind).unwrap();
    let text = program(MAX_NESTING as usize);
    std::thread::Builder::new().stack_size(bytes.parse().unwrap()).spawn(move || work_at_the_ceiling(&text)).unwrap().join().unwrap();
}

fn fits(kind: &str, bytes: usize) -> bool {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD, "--test-threads=1"])
        .env(TRY, format!("{kind}={bytes}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

/// Twice the stack that the worst kind takes in this build, so that the size given to hosts is raised before
/// a host runs out, not after. Run with `--nocapture` for the figures.
#[test]
fn a_host_gives_twice_the_stack_measured_at_the_ceiling() {
    for (kind, _) in DEEP {
        let (mut too_small, mut enough) = (TOO_SMALL, STACK_SIZE / 2);
        assert!(!fits(kind, too_small), "{kind}: nothing overflowed, so this measures nothing");
        assert!(fits(kind, enough), "{kind} needs more than half of STACK_SIZE");
        while enough - too_small > PRECISION {
            let middle = too_small + (enough - too_small) / 2;
            if fits(kind, middle) {
                enough = middle;
            } else {
                too_small = middle;
            }
        }
        eprintln!("{kind}: {} KiB, {} bytes per level", enough / 1024, enough / MAX_NESTING as usize);
    }
}
