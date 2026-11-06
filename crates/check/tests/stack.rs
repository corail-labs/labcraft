//! The stack that `check` takes at the ceiling of nesting (`docs/architecture.md`, section 4), reading the file and walking
//! its body and its types, then dropping what it made. The same trial as in `format`, whose tests hold the programs.
//!
//! A stack overflow ends the whole process, so each try runs in a child process: this test binary again, told what to do by
//! an environment variable.

#[path = "../../format/tests/deep/mod.rs"]
mod deep;

use std::borrow::Cow;
use std::process::{Command, Stdio};

use deep::DEEP;
use labcraft_check::{Content, Found, Libraries, LibrarySource, Origin, SourceFile, check};
use labcraft_format::STACK_SIZE;
use labcraft_types::{Code, Interner, Limits, MAX_NESTING};

const TRY: &str = "LABCRAFT_CHECK_STACK_TRY";
const CHILD: &str = "the_child_process_of_a_try";

const TOO_SMALL: usize = 64 * 1024;
const PRECISION: usize = 16 * 1024;

struct Nothing;

impl Libraries for Nothing {
    fn find(&self, _: &str, _: Origin) -> Found {
        Found::NotFound
    }
}

fn work_at_the_ceiling(text: String) {
    let mut limits = Limits::default().with_max_nesting(MAX_NESTING).unwrap();
    limits.max_file_size = u64::MAX;
    let mut names = Interner::new();
    let file = SourceFile { shown: "deep.lcf".to_owned(), content: Content::Bytes(Cow::Owned(text.into_bytes())) };
    let root = LibrarySource { files: vec![file], ..LibrarySource::new(String::new(), String::new(), Origin::Project) };
    let checked = check(root, &Nothing, &mut names, &limits);
    assert!(!checked.report.diagnostics().iter().any(|diagnostic| matches!(diagnostic.code(), Code::Lc0007 | Code::Lc0099)));
    assert!(!checked.files.is_empty());
}

/// Does nothing unless a parent process runs it.
#[test]
fn the_child_process_of_a_try() {
    let Ok(request) = std::env::var(TRY) else { return };
    let (kind, bytes) = request.split_once('=').unwrap();
    let (_, program) = DEEP.iter().find(|(name, _)| *name == kind).unwrap();
    let text = program(MAX_NESTING as usize);
    std::thread::Builder::new().stack_size(bytes.parse().unwrap()).spawn(move || work_at_the_ceiling(text)).unwrap().join().unwrap();
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

/// Twice the stack that the worst kind takes in this build. Run with `--nocapture` for the figures.
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
