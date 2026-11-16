//! The session of a project without a window (`docs/architecture.md`, 11.6 and 12.9): the `view.*` commands, the last
//! drawing of a function that is no longer drawable, no program file written by opening every example (scenario 6,
//! points 3 and 6), a new project. The times of 200 ms through the watchers are in `timing.rs`, which runs alone; the
//! values of a new run shown within a second stay here, with a bound of 2.5 s.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use labcraft::session::{Handle, Opening, Session};
use labcraft_ide::Host;
use serde_json::{Value, json};

mod common;
use common::{Folder, ONE, TWO};

const BROKEN: &str = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    y = core.add(nowhere, 1.0)\n}\n";

/// The session of the project at `folder`, opened: what `view.open` returned.
fn open(folder: &Path) -> (Session, Value) {
    let mut session = Session::new(folder, Vec::new(), false, None).unwrap();
    let opened = session.open(false).unwrap();
    (session, opened)
}

#[test]
fn the_commands_of_the_viewer_answer_as_commands_md_says() {
    let folder = Folder::new(&[("main.lcf", ONE), ("helper.lcf", ONE.replace("main", "helper").as_str())]);
    let (mut session, opened) = open(&folder.0);
    let names: Vec<&str> = opened["functions"].as_array().unwrap().iter().map(|entry| entry["function"].as_str().unwrap()).collect();
    assert_eq!(names, ["helper", "main"]);
    assert_eq!(opened["functions"][1]["drawable"], json!(true));
    assert_eq!(opened["check"]["schema"], json!("labcraft-check/1"));
    // The first function is shown; another is opened in its tab.
    assert_eq!(session.snapshot().view.main.function.as_deref(), Some("helper"));
    let entry = session.execute("view.show_function", &json!({ "function": "main" })).unwrap();
    assert_eq!((entry["function"].clone(), entry["file"].clone(), entry["drawn"].clone()), (json!("main"), json!("main.lcf"), json!(true)));
    assert_eq!(session.snapshot().view.opened, ["helper", "main"]);
    // The preference returns the value it had.
    assert_eq!(session.execute("view.set_standard_view", &json!({ "on": true })).unwrap(), json!({ "before": false }));
    assert_eq!(session.execute("view.set_standard_view", &json!({ "on": false })).unwrap(), json!({ "before": true }));
    // Errors of a command (section 3).
    let code = |result: Result<Value, labcraft::session::CommandError>| result.unwrap_err().code;
    assert_eq!(code(session.execute("view.nothing", &json!({}))), "command.unknown");
    assert_eq!(code(session.execute("view.show_function", &json!({ "function": "main", "extra": 1 }))), "params.unknown");
    assert_eq!(code(session.execute("view.show_function", &json!({}))), "params.missing");
    assert_eq!(code(session.execute("view.show_function", &json!({ "function": 3 }))), "params.invalid");
    assert_eq!(code(session.execute("view.show_function", &json!({ "function": "nowhere" }))), "params.invalid");
    // A folder of the session's root, named from it (commands.md 2); the viewer opens one anywhere through `open`.
    assert_eq!(code(session.execute("view.open", &json!({ "folder": "no-such" }))), "path.not_found");
    assert_eq!(code(Session::new(&folder.0.join("no-such"), Vec::new(), false, None).map(|_| Value::Null)), "path.not_found");
    // The libraries are those of `--lib-path`, given to the session (decision of the owner, 2026-11-11).
    assert_eq!(code(session.execute("view.open", &json!({ "folder": ".", "lib_path": [] }))), "params.unknown");
    assert_eq!(code(session.execute("view.set_state", &json!({ "function": "main", "state": { "colour": { "a": 1 } } }))), "params.invalid");
    assert_eq!(code(session.execute("view.set_standard_view", &json!({}))), "params.missing");
    // The entry of another function than the one shown is its own.
    let other = session.execute("view.set_state", &json!({ "function": "helper", "state": { "arms": {} } })).unwrap();
    assert_eq!((other["function"].clone(), other["file"].clone(), other["drawn"].clone()), (json!("helper"), json!("helper.lcf"), json!(true)));
}

/// A state refused is not applied in part; the entry of a function that is not drawable says so.
#[test]
fn a_state_refused_changes_nothing_and_an_entry_says_what_is_drawn() {
    let folder = Folder::new(&[("main.lcf", BROKEN)]);
    let (mut session, _) = open(&folder.0);
    let refused = session.execute("view.set_state", &json!({ "function": "main", "state": { "arms": { "k": 1 }, "zzz": { "a": true } } }));
    assert!(refused.is_err());
    assert!(session.snapshot().view.main.arms.is_empty());
    let entry = session.execute("view.set_state", &json!({ "function": "main", "state": { "arms": { "k": 1 } } })).unwrap();
    assert_eq!(entry["drawn"], json!(false));
    assert_eq!(entry["codes"], json!(["LC2001"]));
    assert_eq!(session.snapshot().view.main.arms.get("k"), Some(&1));
}

/// Two files give one name (`a.b.lcf`, always `LC2023`, and `a/b.lcf`): the viewer shows the one that draws.
#[test]
fn of_two_files_of_one_name_the_drawable_one_is_shown() {
    let b = "labcraft 1

fn b(x: f64) -> (y: f64) {
    y = x
}
";
    let folder = Folder::new(&[("a.b.lcf", b)]);
    std::fs::create_dir_all(folder.0.join("a")).unwrap();
    std::fs::write(folder.0.join("a/b.lcf"), b).unwrap();
    let (mut session, _) = open(&folder.0);
    let entry = session.execute("view.show_function", &json!({ "function": "a.b" })).unwrap();
    assert_eq!((entry["file"].clone(), entry["drawn"].clone()), (json!("a/b.lcf"), json!(true)));
    // It breaks: its last drawing stays, and the other file of that name is not shown instead.
    let drawn = session.snapshot().view.main.drawing.clone().unwrap();
    std::fs::write(folder.0.join("a/b.lcf"), BROKEN.replace("main", "b")).unwrap();
    session.files_changed();
    assert!(session.snapshot().view.main.kept);
    assert!(Arc::ptr_eq(session.snapshot().view.main.drawing.as_ref().unwrap(), &drawn));
    let entry = session.execute("view.set_state", &json!({ "function": "a.b", "state": {} })).unwrap();
    assert_eq!((entry["file"].clone(), entry["drawn"].clone()), (json!("a/b.lcf"), json!(false)));
}

/// An arm shown places nothing: no new drawing. A structure unfolded is a new drawing.
#[test]
fn the_view_state_draws_again_only_what_placement_depends_on() {
    let arms = "labcraft 1\n\nfn main(x: f64, n: i64) -> (y: f64) {\n    y = match n {\n        0 => yield x\n        _ => yield 0.0\n    }\n}\n";
    let folder = Folder::new(&[("main.lcf", arms)]);
    let (mut session, _) = open(&folder.0);
    let drawing = session.snapshot().view.main.drawing.clone().unwrap();
    let structure = drawing.arms.first().unwrap().structure.clone();
    let generation = session.snapshot().view.main.drawing_generation;
    session.execute("view.set_state", &json!({ "function": "main", "state": { "arms": { structure.clone(): 1 } } })).unwrap();
    assert_eq!(session.snapshot().view.main.drawing_generation, generation);
    assert_eq!(session.snapshot().view.main.arms.get(&structure), Some(&1));
    session.execute("view.set_state", &json!({ "function": "main", "state": { "unfolded": { structure.clone(): true } } })).unwrap();
    assert_ne!(session.snapshot().view.main.drawing_generation, generation);
    assert!(session.snapshot().view.main.drawing.as_ref().unwrap().arms.is_empty());
    // Kept across a change of the file, by the key of the structure.
    std::fs::write(folder.0.join("main.lcf"), arms.replace("yield 0.0", "yield 1.0")).unwrap();
    session.files_changed();
    assert!(session.snapshot().view.main.drawing.as_ref().unwrap().arms.is_empty());
    assert_eq!(session.snapshot().view.main.arms.get(&structure), Some(&1));
}

/// A function that is no longer drawable keeps its last drawing with its diagnostics (scenario 6, point 3); one that
/// never was shows its diagnostics.
#[test]
fn a_function_no_longer_drawable_keeps_its_last_drawing() {
    let folder = Folder::new(&[("main.lcf", ONE)]);
    let (mut session, _) = open(&folder.0);
    let drawn = session.snapshot().view.main.drawing.clone().unwrap();
    assert!(!drawn.undrawn && !session.snapshot().view.main.kept);
    std::fs::write(folder.0.join("main.lcf"), BROKEN).unwrap();
    session.files_changed();
    let published = session.snapshot();
    assert!(published.view.main.kept);
    assert!(Arc::ptr_eq(published.view.main.drawing.as_ref().unwrap(), &drawn));
    let codes: Vec<&str> = published.view.report.as_ref().unwrap().diagnostics().iter().map(|diagnostic| diagnostic.code().as_str()).collect();
    assert!(codes.contains(&"LC2001"), "{codes:?}");
    assert!(!published.view.functions[0].drawable && published.view.functions[0].errors == 1);
    // Never drawable: its diagnostics.
    let other = Folder::new(&[("main.lcf", BROKEN)]);
    let (fresh, _) = open(&other.0);
    assert!(fresh.snapshot().view.main.drawing.as_ref().unwrap().undrawn && !fresh.snapshot().view.main.kept);
}

/// Every example, library and program of the RFCs opened, each function shown in both views, each structure
/// unfolded: no program file changed (scenario 6, point 6). On copies: the session keeps its journal in the folder it
/// opens (architecture 12.5), which is no program file.
#[test]
fn every_example_opened_changes_no_program_file() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let copies: Vec<Folder> = ["examples", "libs/visa", "libs/scpi", "docs/rfc"].iter().map(|folder| copy_of(&root.join(folder))).collect();
    let folders: Vec<PathBuf> = copies.iter().map(|copy| copy.0.clone()).collect();
    let before: Vec<_> = folders.iter().map(|folder| contents(folder)).collect();
    let mut shown = 0;
    for folder in &folders {
        let (mut session, _) = open(folder);
        for listed in session.snapshot().view.functions.clone() {
            for standard in [false, true] {
                session.execute("view.set_standard_view", &json!({ "on": standard })).unwrap();
                session.execute("view.show_function", &json!({ "function": listed.function })).unwrap();
                let structures: Vec<String> =
                    session.snapshot().view.main.drawing.iter().flat_map(|drawing| drawing.arms.iter().map(|arm| arm.structure.clone())).collect();
                for structure in structures {
                    session.execute("view.set_state", &json!({ "function": listed.function, "state": { "unfolded": { structure: true } } })).unwrap();
                }
                shown += 1;
            }
        }
    }
    assert!(shown > 20, "{shown}");
    let after: Vec<_> = folders.iter().map(|folder| contents(folder)).collect();
    assert!(before == after, "a file changed");
}

/// A copy of a folder, in a folder of its own.
fn copy_of(folder: &Path) -> Folder {
    let copy = Folder::new(&[]);
    let mut pending = vec![folder.to_path_buf()];
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(&path).unwrap().flatten() {
            let path = entry.path();
            let to = copy.0.join(path.strip_prefix(folder).unwrap());
            if path.is_dir() {
                std::fs::create_dir_all(&to).unwrap();
                pending.push(path);
            } else {
                std::fs::write(&to, std::fs::read(&path).unwrap()).unwrap();
            }
        }
    }
    copy
}

/// Every file under a folder, and its bytes, but what the session keeps in `.labcraft/`.
fn contents(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(&path).unwrap().flatten() {
            let path = entry.path();
            if path.file_name().is_some_and(|name| name == ".labcraft") {
                continue;
            }
            if path.is_dir() {
                pending.push(path);
            } else {
                found.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    found
}

/// Inter as `ATTRIBUTION.md` names it (architecture 11.6).
#[test]
fn the_face_of_the_panels_is_inter_4_1() {
    use sha2::Digest;
    let face = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/Inter-Regular.ttf")).unwrap();
    let digest: String = sha2::Sha256::digest(&face).iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(digest, "40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82");
}

/// `--style` (cli.md 8, notation 10): a file that does not exist is written with the defaults, which draw as without it;
/// its values draw again when it changes; values refused leave those before, and the notice says why; colours below
/// notation 8 are taken, and said.
#[test]
fn a_file_of_values_starts_as_the_defaults_and_draws_again_with_what_it_says() {
    use labcraft_diagram::style::{Style, file_of};
    let folder = Folder::new(&[("main.lcf", TWO)]);
    let values = Folder::new(&[]);
    let path = values.0.join("notation.toml");
    let (plain, _) = open(&folder.0);
    let mut session = Session::new(&folder.0, Vec::new(), false, None).unwrap();
    session.watch_style(path.clone());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), file_of(&Style::default()));
    assert!(session.snapshot().notices.is_empty());
    session.open(false).unwrap();
    let before = session.snapshot().view.main.drawing.clone().unwrap();
    assert_eq!(before, plain.snapshot().view.main.drawing.clone().unwrap());
    // 16 units more of margin on each side, by hand: 32 units wider and higher.
    std::fs::write(&path, "[canvas]\nmargin = 32\n").unwrap();
    let generation = session.snapshot().view.main.drawing_generation;
    session.style_changed();
    let after = session.snapshot().view.main.drawing.clone().unwrap();
    assert_ne!(session.snapshot().view.main.drawing_generation, generation);
    assert!(session.snapshot().view.main.glide);
    assert_eq!((after.width, after.height), (before.width + 3200, before.height + 3200));
    assert_eq!(session.snapshot().view.main.drawing.as_ref().unwrap().style.numbers.margin, 3200);
    assert!(session.snapshot().notices.is_empty());
    // Refused: the values before stay, and the drawing.
    std::fs::write(&path, "[canvas]\nmargin = 1000\n").unwrap();
    let generation = session.snapshot().view.main.drawing_generation;
    session.style_changed();
    let notice = session.snapshot().notices.join("; ");
    assert!(notice.contains("`canvas.margin`: 1000 is not between 0 and 64; the values before stay"), "{notice}");
    assert_eq!(session.snapshot().view.main.drawing_generation, generation);
    assert_eq!(session.snapshot().view.main.drawing.as_ref().unwrap().style.numbers.margin, 3200);
    // Taken, and said: an error wire as light as the canvas.
    std::fs::write(&path, "[colors.light]\n\"wire.error\" = \"#F7F7F5\"\n").unwrap();
    session.style_changed();
    let notice = session.snapshot().notices.join("; ");
    assert!(notice.contains("colours below the contrasts of notation 8: `wire.error` on `canvas.fill` (light): 1.00:1"), "{notice}");
    assert!(notice.contains("more"), "{notice}");
    assert_eq!(session.snapshot().view.main.drawing.as_ref().unwrap().style.numbers.margin, 800 * 2);
    // A file gone: said, the values stay.
    std::fs::remove_file(&path).unwrap();
    session.style_changed();
    assert!(session.snapshot().notices.join("; ").ends_with("the values before stay"));
    // A drawing kept while its file is broken is painted with the values it was drawn with.
    std::fs::write(folder.0.join("main.lcf"), BROKEN).unwrap();
    session.files_changed();
    assert!(session.snapshot().view.main.kept);
    let kept = session.snapshot().view.main.drawing.clone().unwrap();
    std::fs::write(&path, "[canvas]\nmargin = 48\n").unwrap();
    session.style_changed();
    assert_eq!(session.snapshot().view.main.drawing.clone().unwrap(), kept);
    assert_eq!(session.snapshot().view.main.drawing.as_ref().unwrap().style.numbers.margin, 1600);
}

/// A trace of `main` with `x` in and `y` out, its file named from the folder.
fn trace_of(x: f64, y: f64) -> String {
    let header = json!({ "schema": "labcraft-trace/1", "run": "r1", "function": "main", "mode": "deterministic", "files": [] });
    let call = json!({ "seq": 1, "t_ms": 0.0, "event": "call", "frame": 0, "function": "main",
        "in": [{ "name": "x", "type": "f64", "value": x }], "types": { "y": "f64" } });
    let ran = json!({ "seq": 2, "t_ms": 0.0, "event": "ran", "frame": 0, "node": "y", "out": { "y": y } });
    let end = json!({ "seq": 3, "t_ms": 0.0, "event": "end", "status": "ok" });
    format!("{header}\n{call}\n{ran}\n{end}\n")
}

fn value_of_y(published: &labcraft_ide::Snapshot) -> Option<String> {
    let overlay = published.view.main.overlay.as_ref()?;
    overlay.bubbles.iter().find(|bubble| bubble.wire == "y").map(|bubble| bubble.title.clone())
}

/// The last run of the function shown is read after its drawing, and a new run is seen within the second that the
/// viewer looks at it (decisions of the owner, 2026-11-13); writing a trace draws nothing again.
#[test]
#[expect(clippy::disallowed_methods, reason = "a native test, never built for WASM, where the clock traps")]
fn a_new_run_shows_its_values_within_a_second_and_a_trace_draws_nothing_again() {
    let folder = Folder::new(&[("main.lcf", ONE)]);
    std::fs::create_dir_all(folder.0.join(".labcraft/runs")).unwrap();
    std::fs::write(folder.0.join(".labcraft/runs/main.trace.jsonl"), trace_of(1.0, 2.0)).unwrap();
    let session = spawn(&folder.0, Vec::new(), None, false);
    let start = Instant::now();
    while value_of_y(&session.snapshot()).is_none() {
        assert!(start.elapsed() < Duration::from_secs(10), "no values");
        std::thread::sleep(Duration::from_millis(5));
    }
    let published = session.snapshot();
    assert_eq!(value_of_y(&published).as_deref(), Some("2.0"));
    assert_eq!(published.view.main.trace.as_ref().map(|trace| trace.file.as_str()), Some(".labcraft/runs/main.trace.jsonl"));
    let drawn = published.view.main.drawing_generation;
    // A run replaces it, as a run does: written beside, then renamed over it.
    std::thread::sleep(Duration::from_millis(50));
    let partial = folder.0.join(".labcraft/runs/main.trace.jsonl.1-1-r2.partial");
    std::fs::write(&partial, trace_of(2.0, 3.0)).unwrap();
    std::fs::rename(&partial, folder.0.join(".labcraft/runs/main.trace.jsonl")).unwrap();
    let start = Instant::now();
    while value_of_y(&session.snapshot()).as_deref() != Some("3.0") {
        assert!(start.elapsed() < Duration::from_secs(5), "the new run is not shown");
        std::thread::sleep(Duration::from_millis(5));
    }
    // Looked at every second; a loaded runner may take longer.
    assert!(start.elapsed() < Duration::from_millis(2500), "{:?}", start.elapsed());
    assert_eq!(session.snapshot().view.main.drawing_generation, drawn, "a trace is no program file");
}

/// The session of a project on its thread, opened.
pub fn spawn(folder: &Path, lib_paths: Vec<PathBuf>, style: Option<PathBuf>, new: bool) -> Handle {
    let opening = Opening { folder: folder.to_path_buf(), lib_paths, standard_everywhere: false, style, new };
    let (handle, reply) = Handle::spawn(opening, Box::new(|| {})).unwrap();
    reply.wait(Duration::from_secs(30)).unwrap().unwrap();
    handle
}

/// `New project` (ide.md 10): an empty folder gets `main.lcf`, and opens with `main` shown; a folder that is not empty
/// is `path.exists`, and nothing is written.
#[test]
fn a_new_project_is_main_in_an_empty_folder() {
    let parent = Folder::new(&[]);
    let folder = parent.0.join("new project");
    labcraft::session::make_new_folder(&folder).unwrap();
    let session = spawn(&folder, Vec::new(), None, true);
    // Canonical (spec 5.2): written by hand, then `labcraft-cli fmt` changes nothing in it and `check` finds nothing.
    assert_eq!(std::fs::read_to_string(folder.join("main.lcf")).unwrap(), "labcraft 1\n\nfn main() -> () {}\n");
    let snapshot = session.snapshot();
    assert_eq!(snapshot.view.main.function.as_deref(), Some("main"));
    assert_eq!(snapshot.view.errors, 0, "{:?}", snapshot.view.report);
    let full = Folder::new(&[("main.lcf", ONE)]);
    assert_eq!(labcraft::session::make_new_folder(&full.0).unwrap_err().code, "path.exists");
    assert_eq!(std::fs::read_to_string(full.0.join("main.lcf")).unwrap(), ONE);
}
