//! `labcraft-cli diagram --svg` as a user runs it (`docs/cli.md`, section 7): the SVG of each function at the path of its
//! file, the SVG of the diagnostics of a function that is not drawn, the marks of the errors of one that is, the codes
//! `LC7001` to `LC7004`, and the report.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

/// A folder of its own for a test, removed at its end.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("diagram-{name}"));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Folder(path)
    }

    fn file(&self, relative: &str, bytes: impl AsRef<[u8]>) -> &Self {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        self
    }

    fn run(&self, args: &[&str]) -> Output {
        run_in(&self.0, args)
    }

    fn json(&self, args: &[&str]) -> (i32, Value) {
        let output = self.run(&[&["--json"], args].concat());
        assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
        (output.status.code().unwrap(), serde_json::from_slice(&output.stdout).unwrap())
    }

    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.0.join(relative)).unwrap_or_else(|error| panic!("{relative}: {error}"))
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_in(folder: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_labcraft-cli")).current_dir(folder).arg("diagram").args(args).output().expect("the program starts")
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn codes(document: &Value) -> Vec<String> {
    document["check"]["diagnostics"].as_array().unwrap().iter().map(|diagnostic| diagnostic["code"].as_str().unwrap().to_owned()).collect()
}

/// The `.lcf` files under a folder, as a walk finds them: not in a folder whose name starts with `.`.
fn walk_lcf(folder: &Path) -> usize {
    let mut count = 0;
    let mut stack = vec![folder.to_path_buf()];
    while let Some(folder) = stack.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            let hidden = path.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.'));
            if path.is_dir() && !hidden {
                stack.push(path);
            } else if path.extension().is_some_and(|extension| extension == "lcf") {
                count += 1;
            }
        }
    }
    count
}

fn svgs_under(folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(folder) = stack.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|extension| extension == "svg") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

const BROKEN: &str = "labcraft 1\n\nuse core\n\nfn broken(a: f64) -> (y: f64) {\n    y = core.add(x, a)\n}\n";
const POWER: &str = "labcraft 1\n\nuse core\n\nfn power(voltage: f64[V], current: f64[A]) -> (p: f64[V]) {\n    p = core.add(voltage, current)\n}\n";

/// Scenario 6, point 2: the SVG of each function of the repository, as `diagram --svg` writes it, is its snapshot,
/// which the harness also draws from the `diagram` crate, natively and in WASM.
#[test]
fn every_function_of_the_repository_is_drawn_as_its_snapshot() {
    let repository = repository();
    let out = Folder::new("snapshots");
    for root in ["examples", "libs", "docs/rfc"] {
        let target = out.0.join(root);
        let output = run_in(&repository, &["--svg", "--out", target.to_str().unwrap(), root]);
        assert_eq!(output.status.code(), Some(0), "{root}: {}", String::from_utf8_lossy(&output.stdout));
        let written = svgs_under(&target);
        let functions = walk_lcf(&repository.join(root));
        assert_eq!(written.len(), functions, "{root}: one SVG per function");
        for svg in written {
            let relative = svg.strip_prefix(&target).unwrap();
            let snapshot = repository.join("tests/harness/snapshots/diagram").join(root).join(relative);
            assert!(fs::read(&svg).unwrap() == fs::read(&snapshot).unwrap(), "{root}/{} differs from its snapshot", relative.display());
        }
    }
}

/// The example of `docs/cli.md`, 7.5: a function that is not drawn, one drawn with its errors marked, and the report.
#[test]
fn a_function_that_is_not_drawable_shows_its_diagnostics_and_the_others_their_marks() {
    let folder = Folder::new("report");
    folder.file("lib/broken.lcf", BROKEN).file("lib/power.lcf", POWER);
    let output = folder.run(&["--svg", "--out", "diagrams", "lib"]);
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("lib/broken.lcf:6:18: error[LC2001]: unknown value `x`"), "{text}");
    assert!(text.contains("\nnot drawn broken: diagrams/broken.svg, its diagnostics\n"), "{text}");
    assert!(text.contains("\ndrawn power: diagrams/power.svg, 1 node\n"), "{text}");
    assert!(text.ends_with("\ndiagram: 2 functions, 1 drawn, 1 not drawn: 2 errors, 0 warnings\n"), "{text}");
    // Not drawn: its frame, its name, its diagnostics placed from the folder, each with its code.
    let broken = folder.read("diagrams/broken.svg");
    assert!(broken.contains(" data-function=\"broken\" data-drawn=\"false\">"), "{broken}");
    assert!(broken.contains("<g data-code=\"LC2001\">") && broken.contains(">broken.lcf:6:18: </tspan>"), "{broken}");
    assert!(!broken.contains("data-node"), "{broken}");
    // Drawn, with the wires of its type error in `mark.invalid`.
    let power = folder.read("diagrams/power.svg");
    assert!(!power.contains("data-drawn") && power.contains("data-node=\"p\""), "{power}");
    assert!(power.contains("<g data-code=\"LC3002\">\n<path") && power.contains("stroke: var(--mark-invalid)"), "{power}");
    // The same in JSON, and the same bytes again.
    let (exit, document) = folder.json(&["--svg", "--out", "diagrams", "lib"]);
    assert_eq!(exit, 1);
    assert_eq!((document["schema"].as_str(), document["status"].as_str()), (Some("labcraft-diagram/1"), Some("error")));
    assert_eq!(document["summary"], serde_json::json!({ "functions": 2, "drawn": 1, "not_drawn": 1 }));
    assert_eq!(
        document["diagrams"],
        serde_json::json!([
            { "function": "broken", "file": "lib/broken.lcf", "svg": "diagrams/broken.svg", "drawn": false, "nodes": 1, "codes": ["LC2001"] },
            { "function": "power", "file": "lib/power.lcf", "svg": "diagrams/power.svg", "drawn": true, "nodes": 1 }
        ])
    );
    assert_eq!(codes(&document), ["LC2001", "LC3002"]);
    assert_eq!(document["check"]["summary"]["errors"], 2);
    assert_eq!(folder.read("diagrams/broken.svg"), broken);
}

/// Exit 0 with an error marked, the path of a sub-library, `--theme`, a folder that holds `DIR`, and from another
/// current folder the same bytes (`docs/cli.md`, 7.2).
#[test]
fn an_svg_is_at_the_path_of_its_file_and_the_same_from_anywhere() {
    let folder = Folder::new("paths");
    folder.file("bench/power.lcf", POWER).file("bench/dmm/measure.lcf", "labcraft 1\n\nfn measure() -> () {}\n");
    let output = folder.run(&["--svg", "--out", "bench/diagrams", "--theme", "light", "bench"]);
    assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stdout));
    let measure = folder.read("bench/diagrams/dmm/measure.svg");
    assert!(measure.contains("data-function=\"dmm.measure\"") && !measure.contains("@media"), "{measure}");
    // Again, from inside the folder: the SVG names its files from the folder, not from the current one.
    let again = run_in(&folder.0.join("bench"), &["--svg", "--out", "../again", "--theme", "light", "."]);
    assert_eq!(again.status.code(), Some(0), "{}", String::from_utf8_lossy(&again.stdout));
    for svg in ["power.svg", "dmm/measure.svg"] {
        assert_eq!(folder.read(&format!("again/{svg}")), folder.read(&format!("bench/diagrams/{svg}")), "{svg}");
    }
    // Both themes by default.
    let both = folder.run(&["--svg", "--out", "both", "bench"]);
    assert_eq!(both.status.code(), Some(0));
    assert!(folder.read("both/power.svg").contains("@media (prefers-color-scheme: dark)"));
}

/// `LC7001` with the nearest names, `LC7003`, `LC7004`, and what `diagram` never touches in `DIR`.
#[test]
fn what_cannot_be_drawn_or_written_has_its_code() {
    let folder = Folder::new("codes");
    folder.file("lib/power.lcf", POWER).file("types/t.lct", "labcraft 1\n\ntype t = f64\n");
    let (exit, document) = folder.json(&["--svg", "--out", "d", "lib", "powr", "power"]);
    assert_eq!(exit, 1);
    assert_eq!(codes(&document), ["LC7001", "LC3002"]);
    let unknown = &document["check"]["diagnostics"][0];
    assert_eq!((unknown["file"].as_str(), unknown["fixes"][0]["title"].as_str()), (Some("lib"), Some("did you mean `power`?")));
    assert_eq!(document["diagrams"][1], serde_json::json!({ "function": "powr", "drawn": false, "codes": ["LC7001"] }));
    // A folder of types only.
    let (exit, document) = folder.json(&["--svg", "--out", "d", "types"]);
    assert_eq!((exit, codes(&document)), (1, vec!["LC7003".to_owned()]));
    // `DIR` is a file: each function gets `LC7004`.
    folder.file("plain", "x");
    let (exit, document) = folder.json(&["--svg", "--out", "plain", "lib"]);
    assert_eq!(exit, 1);
    assert!(codes(&document).contains(&"LC7004".to_owned()), "{document}");
    assert!(document["diagrams"][0].get("svg").is_none(), "{document}");
    // What else is in `DIR` stays: another file, and the SVG of a function that is gone.
    folder.file("d/notes.txt", "mine").file("d/gone.svg", "<svg/>");
    let (exit, _) = folder.json(&["--svg", "--out", "d", "lib"]);
    assert_eq!(exit, 0);
    assert_eq!((folder.read("d/notes.txt"), folder.read("d/gone.svg")), ("mine".to_owned(), "<svg/>".to_owned()));
    assert_eq!(fs::read_dir(folder.0.join("d")).unwrap().count(), 3, "no temporary file is left");
}

/// Wrong usage: exit 2, a message on stderr, nothing written (`docs/cli.md`, 7.1).
#[test]
fn wrong_usage_writes_nothing() {
    let folder = Folder::new("usage");
    folder.file("lib/power.lcf", POWER);
    for args in [
        &["--out", "d", "lib"][..],
        &["--svg", "lib"],
        &["--svg", "--out", "d", "--trace", "t.jsonl", "lib", "power"],
        &["--svg", "--out", "d", "--theme", "sepia", "lib"],
        &["--svg", "--out", "d", "lib", "a..b"],
        &["--svg", "--out", "d", "lib/power.lcf"],
    ] {
        let output = folder.run(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty() && !output.stderr.is_empty(), "{args:?}");
        assert!(!folder.0.join("d").exists(), "{args:?}");
    }
}

/// What an SVG shows and what decides the exit code never depend on `--max-report`, which only shortens the report
/// (`docs/cli.md`, section 1 and 7.2).
#[test]
fn the_svgs_and_the_exit_code_do_not_depend_on_the_report() {
    let folder = Folder::new("max-report");
    folder.file("lib/broken.lcf", BROKEN).file("lib/power.lcf", POWER);
    let (exit, full) = folder.json(&["--svg", "--out", "full", "lib"]);
    let (exit_cut, cut) = folder.json(&["--svg", "--out", "cut", "lib", "--max-report", "0"]);
    assert_eq!((exit, exit_cut), (1, 1));
    assert_eq!(cut["check"]["diagnostics"], serde_json::json!([]));
    assert_eq!(cut["check"]["summary"]["truncated"], true);
    assert_eq!(full["diagrams"][0]["codes"], cut["diagrams"][0]["codes"]);
    for svg in ["broken.svg", "power.svg"] {
        assert_eq!(folder.read(&format!("full/{svg}")), folder.read(&format!("cut/{svg}")), "{svg}");
    }
    // An SVG that cannot be written fails the run, however short the list.
    folder
        .file(
            "ok/good.lcf",
            "labcraft 1

fn good() -> () {}
",
        )
        .file(
            "ok/other.lcf",
            "labcraft 1

fn other() -> () {}
",
        );
    fs::create_dir_all(folder.0.join("o/good.svg")).unwrap();
    let (exit, document) = folder.json(&["--svg", "--out", "o", "ok", "--max-report", "0"]);
    assert_eq!((exit, document["status"].as_str()), (1, Some("error")));
    assert!(document["diagrams"][0].get("svg").is_none() && document["diagrams"][1]["svg"] == "o/other.svg", "{document}");
}

/// A function that is not drawn lists at most 32 diagnostics, then how many are left out; its places are those of the
/// folder, whatever the current folder.
#[test]
fn a_function_that_is_not_drawn_lists_its_first_diagnostics_from_its_folder() {
    let folder = Folder::new("lines");
    let lines: String = (0..40)
        .map(|index| {
            format!(
                "    v{index} = core.neg(x{index})
"
            )
        })
        .collect();
    folder.file(
        "lib/sub/many.lcf",
        format!(
            "labcraft 1

use core

fn many() -> () {{
{lines}}}
"
        ),
    );
    let output = folder.run(&["--svg", "--out", "d", "lib"]);
    assert_eq!(output.status.code(), Some(1));
    let many = folder.read("d/sub/many.svg");
    assert_eq!(many.matches("<g data-code=\"LC2001\">").count(), 32, "{many}");
    assert!(many.contains(">8 more</text>") && many.contains(">sub/many.lcf:6:") && many.contains(" data-function=\"sub.many\""), "{many}");
    let again = run_in(&folder.0.join("lib/sub"), &["--svg", "--out", "../../again", ".."]);
    assert_eq!(again.status.code(), Some(1));
    assert_eq!(folder.read("again/sub/many.svg"), many);
}

/// A mark draws the path of the reader it names, not every path of its value: an SVG grows with its errors and its
/// readers, not with their product.
#[test]
fn a_mark_draws_one_path_and_the_svg_stays_small() {
    let folder = Folder::new("fan");
    let lines: String = (0..400)
        .map(|index| {
            format!(
                "    n{index} = core.add(a, c)
"
            )
        })
        .collect();
    folder.file(
        "fan/fan.lcf",
        format!(
            "labcraft 1

use core

fn fan(a: f64[V], c: f64[A]) -> () {{
{lines}}}
"
        ),
    );
    let output = folder.run(&["--svg", "--out", "d", "fan"]);
    assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stdout));
    let fan = folder.read("d/fan.svg");
    // The first 32 errors are marked, each with its two wires.
    assert_eq!(fan.matches("<g data-code=\"LC3002\">").count(), 32);
    let marks = &fan[fan.find("<g data-code").unwrap()..];
    assert_eq!(marks.matches("<path").count(), 64, "{}", &marks[..2000.min(marks.len())]);
}

/// Two files of one name, a name that is not one of the language, and an error that no drawing marks.
#[test]
fn every_function_found_is_drawn_and_every_error_counts() {
    let folder = Folder::new("names");
    folder
        .file(
            "lib/a/b.lcf",
            "labcraft 1

fn b() -> () {}
",
        )
        .file(
            "lib/a.b.lcf",
            "labcraft 1

fn b() -> () {}
",
        );
    folder.file(
        "lib/my-lib/f.lcf",
        "labcraft 1

fn f() -> () {}
",
    );
    let (_, document) = folder.json(&["--svg", "--out", "d", "lib"]);
    let drawn: Vec<(&str, &str)> =
        document["diagrams"].as_array().unwrap().iter().map(|entry| (entry["function"].as_str().unwrap(), entry["svg"].as_str().unwrap())).collect();
    assert_eq!(drawn, [("a.b", "d/a.b.svg"), ("a.b", "d/a/b.svg"), ("my-lib.f", "d/my-lib/f.svg")]);
    let (_, document) = folder.json(&["--svg", "--out", "e", "lib", "my-lib.f", "a.b"]);
    assert_eq!(document["summary"]["functions"], 3, "{document}");
    // Every function drawn, and an error in a file of types that no drawing marks: exit 1.
    let good = Folder::new("unmarked");
    good.file(
        "lib/f.lcf",
        "labcraft 1

fn f() -> () {}
",
    )
    .file(
        "lib/t.lct",
        "labcraft 1

type t = 
",
    );
    let (exit, document) = good.json(&["--svg", "--out", "d", "lib"]);
    assert_eq!((exit, document["summary"]["not_drawn"].as_u64(), document["status"].as_str()), (1, Some(0), Some("error")), "{document}");
}

/// `LC7005` (notation 2.1, cli.md 5): `y` placed over `ab` and `cd`, which do not touch each other. `core.add` is a
/// circle of 28 (notation 10): `y` spans 100 to 128 across and 40 to 68 down, `ab` 110 to 138 and 20 to 48, `cd` 110 to
/// 138 and 60 to 88. From the left, `y` comes first; `ab` overlaps it, and so does `cd` (60 is above 68), and neither
/// overlaps the other (48 is above 60): two warnings, at the entries of `ab` and `cd`, lines 12 and 13 (1 `labcraft`,
/// 3 `use`, 5 `fn`, 6 to 8 the body, 9 `}`, 11 `layout {`, 12 `ab`, 13 `cd`, 14 `y`). Warnings: the function is drawn,
/// exit 0.
#[test]
fn nodes_that_the_file_places_one_over_the_other_are_drawn_and_warned() {
    let folder = Folder::new("overlap");
    folder.file(
        "lib/sum4.lcf",
        "labcraft 1\n\nuse core\n\nfn sum4(a: f64, b: f64, c: f64, d: f64) -> (y: f64) {\n    ab = core.add(a, b)\n    cd = core.add(c, d)\n    \
         y = core.add(ab, cd)\n}\n\nlayout {\n    node ab at (110, 20)\n    node cd at (110, 60)\n    node y at (100, 40)\n}\n",
    );
    let output = folder.run(&["--svg", "--out", "diagrams", "lib"]);
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("lib/sum4.lcf:12:5: warning[LC7005]: node `ab` overlaps node `y`"), "{text}");
    assert!(text.contains("lib/sum4.lcf:13:5: warning[LC7005]: node `cd` overlaps node `y`"), "{text}");
    assert_eq!(text.matches("LC7005").count(), 2, "{text}");
    assert!(text.ends_with("\ndiagram: 1 function, 1 drawn, 0 not drawn: 0 errors, 2 warnings\n"), "{text}");
}
