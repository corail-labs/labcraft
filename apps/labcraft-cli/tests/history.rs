//! The history of a session on the disk (commands.md 9.1, architecture 12.5): what `Memory`, whose names are compared
//! exactly, cannot show. Expected values are written by hand.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use labcraft_cli::disk::Disk;
use labcraft_engine::{Config, Session};
use labcraft_types::Limits;
use serde_json::{Value, json};

const GAIN: &str = "labcraft 1\n\nfn Gain(x: f64) -> (y: f64) {\n    y = x\n}\n";

/// A folder of its own under the temporary folder of the tests, empty.
fn folder(name: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("history").join(name);
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path.canonicalize().unwrap()
}

fn on<T: Send>(root: &Path, work: impl FnOnce(&mut Session) -> T + Send) -> T {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(labcraft_format::STACK_SIZE)
            .spawn_scoped(scope, || {
                let config = Config {
                    root: root.to_path_buf(),
                    lib_paths: Vec::new(),
                    limits: Limits::default(),
                    shipped: labcraft_cli::shipped(),
                    natives: Vec::new(),
                    clock: Arc::new(labcraft_engine::Still),
                    process: 0,
                };
                let mut session = Session::new(Arc::new(Disk), config, false).unwrap();
                work(&mut session)
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

fn ok(session: &mut Session, id: &str, params: Value) -> Value {
    session.execute(id, &params).unwrap_or_else(|error| panic!("{id}: {}", error.to_json()))
}

/// The names in the folder, as the system writes them, but `.labcraft`, that of the journal.
fn names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != ".labcraft")
        .collect();
    names.sort();
    names
}

/// On a file system that compares names without their case (Windows, macOS), undo of `file.rename` of `Gain.lcf` to
/// `gain.lcf` gives `Gain.lcf` back, and redo `gain.lcf`. Elsewhere there is nothing to show.
#[test]
fn a_rename_of_the_case_alone_is_undone_and_redone() {
    let root = folder("case");
    fs::write(root.join("Gain.lcf"), GAIN).unwrap();
    if !root.join("gain.lcf").exists() {
        return;
    }
    on(&root, |session| {
        ok(session, "file.rename", json!({ "from": "Gain.lcf", "to": "gain.lcf" }));
        assert_eq!(names(&root), ["gain.lcf"]);
        ok(session, "history.undo", json!({}));
        assert_eq!(names(&root), ["Gain.lcf"]);
        ok(session, "history.redo", json!({}));
        assert_eq!(names(&root), ["gain.lcf"]);
        let listed = ok(session, "history.list", json!({}));
        assert!(listed["steps"].as_array().unwrap().iter().all(|step| step["author"] == "user"), "no external write: {listed}");
    });
    assert_eq!(fs::read_to_string(root.join("gain.lcf")).unwrap(), GAIN);
}

/// Headless, with no watch of the files: a write by another program is a step at the next command, and undo gives the
/// bytes before it.
#[test]
fn a_write_by_another_program_is_a_step_on_the_disk() {
    let root = folder("outside");
    fs::write(root.join("Gain.lcf"), GAIN).unwrap();
    on(&root, |session| {
        ok(session, "history.list", json!({}));
        fs::write(root.join("Gain.lcf"), "outside").unwrap();
        let listed = ok(session, "history.list", json!({}));
        assert_eq!((listed["steps"][0]["author"].clone(), listed["steps"][0]["files"].clone()), (json!("external"), json!(["Gain.lcf"])));
        ok(session, "history.undo", json!({}));
    });
    assert_eq!(fs::read_to_string(root.join("Gain.lcf")).unwrap(), GAIN);
}
