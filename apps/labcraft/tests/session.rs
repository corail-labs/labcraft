//! The session of the viewer without a window (`docs/architecture.md`, 11.6): the `view.*` commands, the last drawing
//! of a function that is no longer drawable, read-only on every example, and a file change to a new drawing within
//! 200 ms through the watcher (scenario 6, points 3 and 6).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use labcraft::session::{Handle, Session};
use serde_json::{Value, json};

/// A folder of its own under the temporary folder, removed at the end.
struct Folder(PathBuf);

impl Folder {
    fn new(files: &[(&str, &str)]) -> Folder {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("labcraft-viewer-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&path).unwrap();
        for (name, text) in files {
            std::fs::write(path.join(name), text).unwrap();
        }
        Folder(path.canonicalize().unwrap())
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const ONE: &str = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    y = core.add(x, 1.0)\n}\n";
const TWO: &str = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    a = core.mul(x, 2.0)\n    y = core.add(a, 1.0)\n}\n";
const BROKEN: &str = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    y = core.add(nowhere, 1.0)\n}\n";

fn open(session: &mut Session, folder: &Path) -> Value {
    session.open(folder).unwrap()
}

#[test]
fn the_commands_of_the_viewer_answer_as_commands_md_says() {
    let folder = Folder::new(&[("main.lcf", ONE), ("helper.lcf", ONE.replace("main", "helper").as_str())]);
    let mut session = Session::new(false, Vec::new(), None);
    let opened = open(&mut session, &folder.0);
    let names: Vec<&str> = opened["functions"].as_array().unwrap().iter().map(|entry| entry["function"].as_str().unwrap()).collect();
    assert_eq!(names, ["helper", "main"]);
    assert_eq!(opened["functions"][1]["drawable"], json!(true));
    assert_eq!(opened["check"]["schema"], json!("labcraft-check/1"));
    // The first function is shown; another is opened in its tab.
    assert_eq!(session.published().view.function.as_deref(), Some("helper"));
    let entry = session.execute("view.show_function", &json!({ "function": "main" })).unwrap();
    assert_eq!((entry["function"].clone(), entry["file"].clone(), entry["drawn"].clone()), (json!("main"), json!("main.lcf"), json!(true)));
    assert_eq!(session.published().view.opened, ["helper", "main"]);
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
    assert_eq!(code(session.open(&folder.0.join("no-such"))), "path.not_found");
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
    let mut session = Session::new(false, Vec::new(), None);
    open(&mut session, &folder.0);
    let refused = session.execute("view.set_state", &json!({ "function": "main", "state": { "arms": { "k": 1 }, "zzz": { "a": true } } }));
    assert!(refused.is_err());
    assert!(session.published().view.arms.is_empty());
    let entry = session.execute("view.set_state", &json!({ "function": "main", "state": { "arms": { "k": 1 } } })).unwrap();
    assert_eq!(entry["drawn"], json!(false));
    assert_eq!(entry["codes"], json!(["LC2001"]));
    assert_eq!(session.published().view.arms.get("k"), Some(&1));
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
    let mut session = Session::new(false, Vec::new(), None);
    open(&mut session, &folder.0);
    let entry = session.execute("view.show_function", &json!({ "function": "a.b" })).unwrap();
    assert_eq!((entry["file"].clone(), entry["drawn"].clone()), (json!("a/b.lcf"), json!(true)));
    // It breaks: its last drawing stays, and the other file of that name is not shown instead.
    let drawn = session.published().view.drawing.clone().unwrap();
    std::fs::write(folder.0.join("a/b.lcf"), BROKEN.replace("main", "b")).unwrap();
    session.files_changed();
    assert!(session.published().view.kept);
    assert!(Arc::ptr_eq(session.published().view.drawing.as_ref().unwrap(), &drawn));
    let entry = session.execute("view.set_state", &json!({ "function": "a.b", "state": {} })).unwrap();
    assert_eq!((entry["file"].clone(), entry["drawn"].clone()), (json!("a/b.lcf"), json!(false)));
}

/// An arm shown places nothing: no new drawing. A structure unfolded is a new drawing.
#[test]
fn the_view_state_draws_again_only_what_placement_depends_on() {
    let arms = "labcraft 1\n\nfn main(x: f64, n: i64) -> (y: f64) {\n    y = match n {\n        0 => yield x\n        _ => yield 0.0\n    }\n}\n";
    let folder = Folder::new(&[("main.lcf", arms)]);
    let mut session = Session::new(false, Vec::new(), None);
    open(&mut session, &folder.0);
    let drawing = session.published().view.drawing.clone().unwrap();
    let structure = drawing.arms.first().unwrap().structure.clone();
    let generation = session.published().view.drawing_generation;
    session.execute("view.set_state", &json!({ "function": "main", "state": { "arms": { structure.clone(): 1 } } })).unwrap();
    assert_eq!(session.published().view.drawing_generation, generation);
    assert_eq!(session.published().view.arms.get(&structure), Some(&1));
    session.execute("view.set_state", &json!({ "function": "main", "state": { "unfolded": { structure.clone(): true } } })).unwrap();
    assert_ne!(session.published().view.drawing_generation, generation);
    assert!(session.published().view.drawing.as_ref().unwrap().arms.is_empty());
    // Kept across a change of the file, by the key of the structure.
    std::fs::write(folder.0.join("main.lcf"), arms.replace("yield 0.0", "yield 1.0")).unwrap();
    session.files_changed();
    assert!(session.published().view.drawing.as_ref().unwrap().arms.is_empty());
    assert_eq!(session.published().view.arms.get(&structure), Some(&1));
}

/// A function that is no longer drawable keeps its last drawing with its diagnostics (scenario 6, point 3); one that
/// never was shows its diagnostics.
#[test]
fn a_function_no_longer_drawable_keeps_its_last_drawing() {
    let folder = Folder::new(&[("main.lcf", ONE)]);
    let mut session = Session::new(false, Vec::new(), None);
    open(&mut session, &folder.0);
    let drawn = session.published().view.drawing.clone().unwrap();
    assert!(!drawn.undrawn && !session.published().view.kept);
    std::fs::write(folder.0.join("main.lcf"), BROKEN).unwrap();
    session.files_changed();
    let published = session.published();
    assert!(published.view.kept);
    assert!(Arc::ptr_eq(published.view.drawing.as_ref().unwrap(), &drawn));
    assert!(published.problems.contains("LC2001"), "{}", published.problems);
    assert!(!published.view.functions[0].drawable && published.view.functions[0].errors == 1);
    // Never drawable: its diagnostics.
    let other = Folder::new(&[("main.lcf", BROKEN)]);
    let mut fresh = Session::new(false, Vec::new(), None);
    open(&mut fresh, &other.0);
    assert!(fresh.published().view.drawing.as_ref().unwrap().undrawn && !fresh.published().view.kept);
}

/// Every example, library and program of the RFCs opened, each function shown in both views, each structure
/// unfolded: no file changed (scenario 6, point 6).
#[test]
fn every_example_opened_changes_no_file() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let folders = [root.join("examples"), root.join("libs/visa"), root.join("libs/scpi"), root.join("docs/rfc")];
    let before: Vec<_> = folders.iter().map(|folder| contents(folder)).collect();
    let mut shown = 0;
    for folder in &folders {
        let mut session = Session::new(false, Vec::new(), None);
        open(&mut session, folder);
        for listed in session.published().view.functions.clone() {
            for standard in [false, true] {
                session.execute("view.set_standard_view", &json!({ "on": standard })).unwrap();
                session.execute("view.show_function", &json!({ "function": listed.function })).unwrap();
                let structures: Vec<String> =
                    session.published().view.drawing.iter().flat_map(|drawing| drawing.arms.iter().map(|arm| arm.structure.clone())).collect();
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

/// Every file under a folder, and its bytes.
fn contents(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(&path).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    found
}

/// A file change to a new drawing within 200 ms *(proposed)*, through the watcher and the thread of the session, without
/// a window (scenario 6, point 3): the slowest of three changes.
#[test]
#[expect(clippy::disallowed_methods, reason = "a native test, never built for WASM, where the clock traps")]
fn a_change_of_the_file_is_drawn_within_200_ms() {
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
    let mut plain = Session::new(false, Vec::new(), None);
    open(&mut plain, &folder.0);
    let mut session = Session::new(false, Vec::new(), None);
    session.watch_style(path.clone());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), file_of(&Style::default()));
    assert_eq!(session.published().style_notice, None);
    open(&mut session, &folder.0);
    let before = session.published().view.drawing.clone().unwrap();
    assert_eq!(before, plain.published().view.drawing.clone().unwrap());
    // 16 units more of margin on each side, by hand: 32 units wider and higher.
    std::fs::write(&path, "[canvas]\nmargin = 32\n").unwrap();
    let generation = session.published().view.drawing_generation;
    session.style_changed();
    let after = session.published().view.drawing.clone().unwrap();
    assert_ne!(session.published().view.drawing_generation, generation);
    assert!(session.published().view.glide);
    assert_eq!((after.width, after.height), (before.width + 3200, before.height + 3200));
    assert_eq!(session.published().view.drawing.as_ref().unwrap().style.numbers.margin, 3200);
    assert_eq!(session.published().style_notice, None);
    // Refused: the values before stay, and the drawing.
    std::fs::write(&path, "[canvas]\nmargin = 1000\n").unwrap();
    let generation = session.published().view.drawing_generation;
    session.style_changed();
    let notice = session.published().style_notice.clone().unwrap_or_default();
    assert!(notice.contains("`canvas.margin`: 1000 is not between 0 and 64; the values before stay"), "{notice}");
    assert_eq!(session.published().view.drawing_generation, generation);
    assert_eq!(session.published().view.drawing.as_ref().unwrap().style.numbers.margin, 3200);
    // Taken, and said: an error wire as light as the canvas.
    std::fs::write(&path, "[colors.light]\n\"wire.error\" = \"#F7F7F5\"\n").unwrap();
    session.style_changed();
    let notice = session.published().style_notice.clone().unwrap_or_default();
    assert!(notice.contains("colours below the contrasts of notation 8: `wire.error` on `canvas.fill` (light): 1.00:1"), "{notice}");
    assert!(notice.contains("more"), "{notice}");
    assert_eq!(session.published().view.drawing.as_ref().unwrap().style.numbers.margin, 800 * 2);
    // A file gone: said, the values stay.
    std::fs::remove_file(&path).unwrap();
    session.style_changed();
    assert!(session.published().style_notice.clone().unwrap_or_default().ends_with("the values before stay"));
    // A drawing kept while its file is broken is painted with the values it was drawn with.
    std::fs::write(folder.0.join("main.lcf"), BROKEN).unwrap();
    session.files_changed();
    assert!(session.published().view.kept);
    let kept = session.published().view.drawing.clone().unwrap();
    std::fs::write(&path, "[canvas]\nmargin = 48\n").unwrap();
    session.style_changed();
    assert_eq!(session.published().view.drawing.clone().unwrap(), kept);
    assert_eq!(session.published().view.drawing.as_ref().unwrap().style.numbers.margin, 1600);
}

/// A file of values saved to a new drawing within 200 ms *(proposed)*, through its watcher and the thread of the session:
/// the slowest of three saves.
#[test]
#[expect(clippy::disallowed_methods, reason = "a native test, never built for WASM, where the clock traps")]
fn a_file_of_values_saved_is_drawn_within_200_ms() {
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

/// The preference set before any folder is open holds, and the folder opened after is drawn with it.
#[test]
fn the_standard_view_set_before_a_folder_is_open_holds() {
    let folder = Folder::new(&[("main.lcf", ONE)]);
    let mut session = Session::new(false, Vec::new(), None);
    assert_eq!(session.execute("view.set_standard_view", &json!({ "on": true })).unwrap(), json!({ "before": false }));
    assert!(session.published().view.standard_everywhere);
    open(&mut session, &folder.0);
    assert!(session.published().view.standard_everywhere);
    assert_eq!(session.execute("view.set_standard_view", &json!({ "on": false })).unwrap(), json!({ "before": true }));
}
