//! `labcraft-cli check` as a user runs it (`docs/cli.md`, section 3). Passes 1 to 4 for now: reading the files, finding
//! the libraries they use, what each library declares, the names of each body and its types.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

#[path = "../../../crates/format/tests/deep/mod.rs"]
#[allow(dead_code)]
mod deep;

const FUNCTION: &str = "labcraft 1\n\nfn f() -> () {}\n";

/// The stored cases whose codes come with later steps.
fn later() -> Vec<&'static str> {
    include_str!("../../../tests/acceptance/hostile-later.txt").lines().filter(|line| !line.starts_with('#')).collect()
}

/// A folder of its own for a test, removed at its end.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("check-{name}"));
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
        json_in(&self.0, args)
    }

    fn codes(&self, args: &[&str]) -> (i32, Vec<String>) {
        let (exit, document) = self.json(args);
        (exit, codes_of(&document))
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_in(folder: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_labcraft-cli")).current_dir(folder).arg("check").args(args).output().expect("the program starts")
}

fn json_in(folder: &Path, args: &[&str]) -> (i32, Value) {
    let output = run_in(folder, &[&["--json"], args].concat());
    assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
    (output.status.code().expect("the process ended normally"), serde_json::from_slice(&output.stdout).unwrap())
}

fn codes_of(document: &Value) -> Vec<String> {
    document["diagnostics"].as_array().unwrap().iter().map(|diagnostic| diagnostic["code"].as_str().unwrap().to_owned()).collect()
}

/// The root of the repository, whatever the current folder of the test.
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn uses(lines: &[&str]) -> String {
    let lines: String = lines.iter().map(|name| format!("use {name}\n")).collect();
    format!("labcraft 1\n\n{lines}\nfn f() -> () {{}}\n")
}

/// A mistake in a body, as an agent reads it: its place, its function, its node and its fix.
#[test]
fn a_name_in_a_body_comes_with_its_function_its_node_and_its_fix() {
    let folder = Folder::new("body");
    folder.file("f.lcf", "labcraft 1\n\nfn f(a: i32) -> (x: i32) {\n    x = add(a, 1)\n}\n");
    let (exit, document) = folder.json(&["."]);
    assert_eq!(exit, 1);
    let diagnostic = &document["diagnostics"][0];
    assert_eq!(diagnostic["code"], "LC2002");
    assert_eq!(diagnostic["span"]["start"], serde_json::json!({ "line": 4, "column": 9 }));
    assert_eq!(diagnostic["function"], "f");
    assert_eq!(diagnostic["node"], serde_json::json!({ "name": "x", "text": "x = add(a, 1)" }));
    assert_eq!(diagnostic["fixes"][0]["edits"][0]["text"], "core.add");
    assert_eq!(diagnostic["fixes"][0]["edits"][1]["text"], "\nuse core\n");
}

/// The panel of the spec, with the libraries it uses: what a run of `monitor` reads and writes has its types (section 10).
#[test]
fn the_panel_of_the_spec_has_no_mistake() {
    let spec = fs::read_to_string(repository().join("docs/language-spec-v0.md")).unwrap().replace("\r\n", "\n");
    let block = spec.split("```text\n").find(|block| block.contains("fn monitor(")).unwrap();
    let text = block.split("```").next().unwrap();
    let folder = Folder::new("monitor");
    folder.file("monitor.lcf", text);
    let (exit, document) = folder.json(&["."]);
    assert_eq!(codes_of(&document), Vec::<String>::new(), "{document}");
    assert_eq!(exit, 0);

    let wrong = text.replace("shown = ui.set(live_voltage, volts)", "shown = ui.set(live_voltage, \"text\")");
    folder.file("monitor.lcf", &wrong);
    let (exit, document) = folder.json(&["."]);
    assert_eq!(codes_of(&document), ["LC3001"], "{document}");
    assert_eq!(exit, 1);
}

/// A warning does not change the exit code, unless it is asked to (`docs/cli.md`, section 3).
#[test]
fn a_function_that_calls_itself_is_a_warning() {
    let folder = Folder::new("recursion");
    folder.file("f.lcf", "labcraft 1\n\nfn f(n: i32) -> (x: i32) {\n    x = f(n)\n}\n");
    assert_eq!(folder.codes(&["."]), (0, vec!["LC2016".to_owned()]));
    assert_eq!(folder.codes(&["--deny-warnings", "."]), (1, vec!["LC2016".to_owned()]));
}

#[test]
fn a_clean_folder_gives_the_document_of_the_documentation() {
    let folder = Folder::new("clean");
    folder.file("f.lcf", FUNCTION).file("t.lct", "labcraft 1\n\ntype T = f64\n").file("sub/g.lcf", FUNCTION);
    let (exit, document) = folder.json(&["."]);
    assert_eq!(exit, 0);
    assert_eq!(document["schema"], "labcraft-check/1");
    assert_eq!(document["status"], "ok");
    let summary = serde_json::json!({ "libraries": 2, "functions": 2, "types": 1, "errors": 0, "warnings": 0, "truncated": false });
    assert_eq!(document["summary"], summary);
    assert_eq!(document["diagnostics"], serde_json::json!([]));
}

#[test]
fn the_text_ends_with_the_summary() {
    let folder = Folder::new("text");
    folder.file("f.lcf", uses(&["nowhere"]));
    let output = folder.run(&["."]);
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.starts_with("f.lcf:3:5: error[LC2004]: unknown library `nowhere`"), "{text}");
    assert!(text.ends_with("check: 1 library, 1 function, 0 types: 1 error, 0 warnings\n"), "{text}");
}

#[test]
fn a_signature_can_name_the_types_of_a_shipped_library() {
    let folder = Folder::new("shipped-types");
    folder.file("f.lcf", "labcraft 1\n\nuse visa\n\nfn f(session: visa.session) -> () {}\n");
    assert_eq!(folder.codes(&["."]), (0, vec![]));
    folder.file("f.lcf", "labcraft 1\n\nuse visa\n\nfn f(session: visa.nowhere) -> () {}\n");
    assert_eq!(folder.codes(&["."]), (1, vec!["LC2003".to_owned()]));
}

#[test]
fn a_library_without_use_comes_with_its_fix_in_the_document() {
    let folder = Folder::new("fix-of-a-use");
    folder.file("f.lcf", "labcraft 1\n\nfn f(session: visa.session) -> () {}\n");
    let (exit, document) = folder.json(&["."]);
    assert_eq!(exit, 1);
    let diagnostic = &document["diagnostics"][0];
    assert_eq!(diagnostic["code"], "LC2005");
    assert_eq!(diagnostic["function"], "f");
    assert_eq!(diagnostic["fixes"][0]["edits"][0]["text"], "\nuse visa\n");
    assert_eq!(diagnostic["fixes"][0]["edits"][0]["span"]["start"]["line"], 2);
}

#[test]
fn check_does_not_care_whether_a_file_is_canonical() {
    let folder = Folder::new("not-canonical");
    let path = folder.0.join("f.lcf");
    folder.file("f.lcf", "labcraft 1\nfn f()->(){x=1\n}\n");
    assert_eq!(folder.codes(&["."]), (0, vec![]));
    assert_eq!(fs::read_to_string(path).unwrap(), "labcraft 1\nfn f()->(){x=1\n}\n", "it writes nothing");
}

#[test]
fn a_path_is_data_and_a_bad_one_hides_nothing() {
    let folder = Folder::new("paths");
    folder.file("good/f.lcf", FUNCTION).file("bad/f.lcf", "x");
    fs::create_dir_all(folder.0.join("empty")).unwrap();
    assert_eq!(folder.codes(&["missing"]), (1, vec!["LC0001".to_owned()]));
    assert_eq!(folder.codes(&["empty"]), (1, vec!["LC0002".to_owned()]));
    let (exit, document) = folder.json(&["missing", "empty", "good", "bad"]);
    assert_eq!(exit, 1);
    assert_eq!(codes_of(&document), ["LC0101", "LC0002", "LC0001"]);
    assert_eq!(document["summary"]["libraries"], 2);
}

#[test]
fn a_folder_given_twice_or_inside_another_counts_once() {
    let folder = Folder::new("twice");
    folder.file("a/f.lcf", "x").file("a/b/f.lcf", "x");
    for args in [&["a"][..], &["a", "a"], &["a", "a/./"], &["a", "a/b"], &["a/b", "a"]] {
        let (exit, document) = folder.json(args);
        assert_eq!(exit, 1, "{args:?}");
        assert_eq!(document["summary"]["errors"], 2, "{args:?}");
        assert_eq!(document["summary"]["libraries"], 2, "{args:?}");
    }
}

#[test]
fn a_folder_inside_another_uses_what_the_outer_one_holds() {
    let folder = Folder::new("inside");
    folder.file("a/b/f.lcf", uses(&["c"])).file("a/c/f.lcf", FUNCTION);
    assert_eq!(folder.codes(&["a/b"]), (1, vec!["LC2004".to_owned()]), "alone, `a/c` is not in the folder given");
    assert_eq!(folder.codes(&["a", "a/b"]), (0, vec![]));
}

/// A walk does not reach these from outside, so they would be lost if a folder inside another were simply dropped.
#[test]
fn naming_a_folder_inside_another_loses_nothing_that_naming_it_alone_keeps() {
    let folder = Folder::new("not-reached");
    folder.file("f.lcf", FUNCTION).file(".hidden/bad.lcf", "x");
    fs::create_dir_all(folder.0.join("empty")).unwrap();
    assert_eq!(folder.codes(&[".hidden"]), (1, vec!["LC0101".to_owned()]));
    assert_eq!(folder.codes(&["."]), (0, vec![]));
    assert_eq!(folder.codes(&[".", ".hidden"]), (1, vec!["LC0101".to_owned()]));
    assert_eq!(folder.codes(&[".", "empty"]), (1, vec!["LC0002".to_owned()]));
}

#[test]
fn the_spelling_of_a_folder_does_not_depend_on_the_order() {
    let folder = Folder::new("spelling");
    folder.file("x/f.lcf", "x").file("cwd/keep.txt", "");
    let inner = folder.0.join("cwd");
    assert_eq!(json_in(&inner, &["../x/.", "../x"]), json_in(&inner, &["../x", "../x/."]));
}

#[test]
fn the_order_of_the_folders_changes_nothing() {
    let folder = Folder::new("order");
    folder.file("a/f.lcf", "x").file("b/f.lcf", uses(&["nowhere"]));
    assert_eq!(folder.json(&["a", "b"]), folder.json(&["b", "a"]));
}

#[test]
fn warnings_count_only_on_request() {
    let folder = Folder::new("warnings");
    folder.file("f.lcf", uses(&["time", "time"]));
    assert_eq!(folder.codes(&["."]), (0, vec!["LC2020".to_owned()]));
    let (exit, document) = folder.json(&["--deny-warnings", "."]);
    assert_eq!((exit, &document["status"]), (1, &Value::from("error")));
}

#[test]
fn the_limits_of_a_file_are_those_of_fmt() {
    let folder = Folder::new("limits");
    folder.file("big.lcf", "x".repeat(100)).file("f.lcf", FUNCTION);
    assert_eq!(folder.codes(&["--max-file-size", "50", "."]), (1, vec!["LC0006".to_owned()]));
    let (kind, generate) = deep::DEEP[0];
    let folder = Folder::new("nesting");
    folder.file("deep.lcf", generate(100_000));
    assert_eq!(folder.codes(&["."]), (1, vec!["LC0007".to_owned()]), "{kind}");
    let at_the_ceiling = Folder::new("ceiling");
    at_the_ceiling.file("deep.lcf", generate(1024));
    assert_eq!(at_the_ceiling.codes(&["--max-nesting", "1024", "."]), (0, vec![]), "{kind}");
}

#[test]
fn a_use_finds_the_first_match_of_the_rules() {
    let folder = Folder::new("rules");
    let project = folder.0.join("project");
    folder
        .file("project/f.lcf", uses(&["here", "there", "scpi", "visa", "time"]))
        .file("project/here/g.lcf", FUNCTION)
        .file("first/there/g.lcf", FUNCTION)
        .file("second/there/g.lcf", "x");
    let (exit, document) = json_in(&project, &[".", "--lib-path", "../first", "--lib-path", "../second"]);
    assert_eq!((exit, codes_of(&document)), (0, vec![]));
    assert_eq!(document["summary"]["libraries"], 2, "a dependency is not counted");
    let (exit, document) = json_in(&project, &[".", "--lib-path", "../second", "--lib-path", "../first"]);
    assert_eq!((exit, codes_of(&document)), (1, vec!["LC0101".to_owned()]), "the order of --lib-path decides");
    let (exit, document) = json_in(&project, &["."]);
    assert_eq!((exit, codes_of(&document)), (1, vec!["LC2004".to_owned()]), "without --lib-path, `there` is unknown");
}

/// If the shipped `scpi` saw this folder, `scpi` and it would make a cycle.
#[test]
fn a_folder_of_a_project_replaces_a_shipped_library_for_that_project_alone() {
    let folder = Folder::new("replace");
    folder.file("f.lcf", uses(&["scpi", "visa"])).file("visa/open.lcf", uses(&["scpi"]));
    assert_eq!(folder.codes(&["."]), (0, vec![]));
}

#[test]
fn a_name_under_a_lib_path_is_normalized_and_two_spellings_give_the_same_one_everywhere() {
    let folder = Folder::new("spellings");
    folder.file("p/f.lcf", "labcraft 1\n\nuse caf\u{E9}\n\nfn f() -> () {}\n").file("libs/cafe\u{301}/g.lcf", "x");
    assert_eq!(folder.codes(&["p", "--lib-path", "libs"]), (1, vec!["LC0101".to_owned()]), "the decomposed folder is found");
    let both = Folder::new("spellings-both");
    both.file("p/f.lcf", "labcraft 1\n\nuse caf\u{E9}\n\nfn f() -> () {}\n")
        .file("libs/cafe\u{301}/g.lcf", "x")
        .file("libs/caf\u{E9}/g.lcf", FUNCTION);
    let found = both.codes(&["p", "--lib-path", "libs"]);
    if fs::read_dir(both.0.join("libs")).unwrap().count() == 2 {
        assert_eq!(found, (1, vec!["LC0101".to_owned()]), "`e` sorts before `\u{E9}`");
    }
}

#[test]
fn names_of_libraries_are_compared_exactly_even_where_files_ignore_case() {
    let folder = Folder::new("case");
    folder.file("p/f.lcf", uses(&["VISA", "Lib"])).file("elsewhere/lib/g.lcf", FUNCTION);
    assert_eq!(folder.codes(&["p", "--lib-path", "elsewhere"]), (1, vec!["LC2004".to_owned(), "LC2004".to_owned()]));
}

#[test]
fn a_library_of_a_lib_path_is_its_folder_alone() {
    let folder = Folder::new("alone");
    folder.file("p/f.lcf", uses(&["a", "a.b"])).file("libs/a/g.lcf", FUNCTION).file("libs/a/b/h.lcf", FUNCTION).file("libs/a/bad/not-read.lcf", "x");
    assert_eq!(folder.codes(&["p", "--lib-path", "libs"]), (0, vec![]), "a sub-folder that nothing uses is not read");
    let empty = Folder::new("alone-empty");
    empty.file("p/f.lcf", uses(&["a"])).file("libs/a/b/h.lcf", FUNCTION);
    assert_eq!(empty.codes(&["p", "--lib-path", "libs"]), (1, vec!["LC2004".to_owned()]), "`a` holds no file of its own");
}

#[test]
fn a_dependency_is_read_for_its_errors_not_its_warnings() {
    let folder = Folder::new("dependency");
    folder
        .file("p/f.lcf", uses(&["warned", "broken"]))
        .file("libs/warned/g.lcf", uses(&["time", "time"]))
        .file("libs/broken/g.lcf", "labcraft 1\n\nfn a( {\n");
    let (exit, document) = folder.json(&["p", "--lib-path", "libs"]);
    assert_eq!((exit, codes_of(&document)), (1, vec!["LC1001".to_owned()]));
    assert_eq!(document["diagnostics"][0]["file"], "libs/broken/g.lcf");
}

#[test]
fn a_lib_path_that_does_not_exist_is_reported_once() {
    let folder = Folder::new("missing-lib-path");
    folder.file("p/f.lcf", FUNCTION);
    let one = vec!["LC0001".to_owned()];
    assert_eq!(folder.codes(&["p", "--lib-path", "nowhere"]), (1, one.clone()));
    assert_eq!(folder.codes(&["p", "--lib-path", "nowhere", "--lib-path", "nowhere"]), (1, one.clone()));
    assert_eq!(folder.codes(&["nowhere", "--lib-path", "nowhere"]), (1, one.clone()));
    let (_, document) = folder.json(&["p", "--lib-path", ""]);
    assert_eq!(document["diagnostics"][0]["file"], "\"\"", "a report never has an empty file name");
}

#[test]
fn a_report_cut_to_nothing_does_not_change_the_status() {
    let folder = Folder::new("max-report");
    folder.file("p/f.lcf", uses(&["broken"])).file("libs/broken/g.lcf", "labcraft 1\n\nfn a( {\n");
    let (exit, document) = folder.json(&["--max-report", "0", "p", "--lib-path", "libs"]);
    assert_eq!((exit, &document["status"]), (1, &Value::from("error")));
    assert_eq!(document["summary"]["errors"], 1);
    assert_eq!(document["summary"]["truncated"], true);
}

#[test]
fn a_dependency_says_its_errors_once_whoever_reads_it() {
    let folder = Folder::new("once");
    folder.file("p/f.lcf", uses(&["broken"])).file("q/f.lcf", uses(&["broken"])).file("libs/broken/g.lcf", "labcraft 1\n\nfn a( {\n");
    let (exit, document) = folder.json(&["p", "q", "--lib-path", "libs"]);
    assert_eq!((exit, codes_of(&document)), (1, vec!["LC1001".to_owned()]));
    assert_eq!(document["summary"]["errors"], 1);
    let (_, document) = folder.json(&["p", "q", "--lib-path", "libs", "--lib-path", "./libs"]);
    assert_eq!(document["summary"]["errors"], 1, "a --lib-path given twice");
    let (_, document) = folder.json(&["p", "libs/broken", "--lib-path", "libs"]);
    assert_eq!(document["summary"]["errors"], 1, "also a folder given");
    let (_, document) = folder.json(&["p", "libs", "--lib-path", "libs"]);
    assert_eq!(document["summary"]["errors"], 1, "inside a folder given");
}

#[test]
fn a_dependency_says_the_same_whoever_reads_it() {
    let folder = Folder::new("same");
    folder.file("p/f.lcf", uses(&["d"])).file("p/e/g.lcf", FUNCTION).file("q/f.lcf", uses(&["d"])).file("libs/d/g.lcf", uses(&["e"]));
    let one = (1, vec!["LC2004".to_owned()]);
    assert_eq!(folder.codes(&["p", "--lib-path", "libs"]), one, "a dependency does not see the folder given");
    assert_eq!(folder.codes(&["q", "--lib-path", "libs"]), one);
    assert_eq!(folder.codes(&["p", "q", "--lib-path", "libs"]), one);
}

/// A `--lib-path` gives a folder a name, so `p` can then use itself, which is a cycle, and its bad file is said once.
#[test]
fn a_folder_given_that_uses_itself_through_a_lib_path_is_a_cycle() {
    let folder = Folder::new("root-by-lib-path");
    folder.file("p/f.lcf", uses(&["p"])).file("p/g.lcf", "x");
    let (exit, document) = folder.json(&["p", "--lib-path", "."]);
    assert_eq!(exit, 1);
    assert_eq!(codes_of(&document), ["LC2021", "LC0101"]);
}

#[test]
fn two_folders_given_that_use_each_other_through_a_lib_path_are_one_cycle() {
    let folder = Folder::new("cycle-of-roots");
    folder.file("p/f.lcf", uses(&["q"])).file("q/f.lcf", uses(&["p"]));
    let one = (1, vec!["LC2021".to_owned()]);
    assert_eq!(folder.codes(&["p", "q", "--lib-path", "."]), one);
    assert_eq!(folder.codes(&["q", "p", "--lib-path", "."]), one);
    assert_eq!(folder.codes(&["p", "--lib-path", "."]), one);
}

#[test]
fn a_library_that_uses_itself_through_a_lib_path_under_the_folder_given_is_a_cycle() {
    let folder = Folder::new("self-under-root");
    folder.file("g.lcf", uses(&["lib"])).file("sub/lib/a.lcf", uses(&["lib"]));
    assert_eq!(folder.codes(&[".", "--lib-path", "sub"]), (1, vec!["LC2021".to_owned()]));
}

/// A walk does not reach a hidden folder, so nothing else would report what is wrong in it.
#[test]
fn a_dependency_in_a_hidden_folder_of_the_folder_given_still_gives_its_errors() {
    let folder = Folder::new("hidden-dependency");
    folder.file("f.lcf", uses(&["dep"])).file(".vendor/dep/a.lcf", "labcraft 1\n\nfn a( {\n");
    assert_eq!(folder.codes(&[".", "--lib-path", ".vendor"]), (1, vec!["LC1001".to_owned()]));
}

/// The report does not depend on which other folder is named, nor on where its path sorts.
#[test]
fn naming_another_folder_changes_nothing_about_a_dependency() {
    let folder = Folder::new("named-another");
    folder
        .file("libs/x/f.lcf", uses(&["y"]))
        .file("libs/y/f.lcf", uses(&["x", "nowhere"]))
        .file("aa/f.lcf", uses(&["y"]))
        .file("zz/f.lcf", uses(&["y"]));
    let expected = vec!["LC2004".to_owned(), "LC2021".to_owned()];
    for args in [&["aa"][..], &["libs/x"], &["aa", "libs/x"], &["libs/x", "zz"], &["aa", "zz"]] {
        let (exit, document) = folder.json(&[args, &["--lib-path", "libs"]].concat());
        let mut found = codes_of(&document);
        found.sort();
        assert_eq!((exit, found), (1, expected.clone()), "{args:?}");
        assert_eq!(document["summary"]["errors"], 2, "{args:?}");
    }
}

#[test]
fn a_library_outside_the_current_folder_is_shown_as_it_was_given() {
    let folder = Folder::new("shown-dependency");
    folder.file("p/f.lcf", uses(&["dep"])).file("libs/dep/a.lcf", "x");
    let inside = folder.0.join("p");
    let (_, document) = json_in(&inside, &[".", "--lib-path", "../libs"]);
    assert_eq!(document["diagnostics"][0]["file"], "../libs/dep/a.lcf");
}

#[test]
fn the_limits_of_the_user_are_not_for_the_libraries_shipped() {
    let folder = Folder::new("shipped-limits");
    folder.file("f.lcf", uses(&["visa", "scpi"]));
    assert_eq!(folder.codes(&["--max-file-size", "100", "."]), (0, vec![]));
}

#[test]
fn libraries_in_a_cycle_are_reported_once() {
    let folder = Folder::new("cycle");
    folder.file("a/f.lcf", uses(&["b"])).file("b/f.lcf", uses(&["a"]));
    assert_eq!(folder.codes(&["."]), (1, vec!["LC2021".to_owned()]));
}

#[test]
fn a_type_in_a_function_file_and_the_reverse_are_refused() {
    let folder = Folder::new("kinds");
    folder.file("t.lcf", "labcraft 1\n\ntype T = f64\n").file("f.lct", FUNCTION).file("s.lct", "labcraft 1\n\ntype S = f64\n\npanel {\n}\n");
    let (exit, found) = folder.codes(&["."]);
    assert_eq!((exit, found), (1, vec!["LC0103".to_owned(), "LC0104".to_owned(), "LC0103".to_owned()]));
}

/// The libraries shipped with LabCraft and the programs of the repository must stay clean under `check`.
#[test]
fn the_programs_of_the_repository_are_clean() {
    let root = repository();
    for folder in ["examples", "libs", "libs/visa", "libs/scpi", "docs/rfc"] {
        let (exit, document) = json_in(&root, &[folder]);
        assert_eq!((exit, codes_of(&document)), (0, vec![]), "{folder}");
        assert_eq!(document["summary"]["errors"], 0, "{folder}");
    }
}

/// What the corpus compares of `related`: the file and the span (`tests/acceptance/README.md`).
fn places(related: &Value) -> Value {
    let list =
        related.as_array().map_or_else(Vec::new, |list| list.iter().map(|item| json!({ "file": item["file"], "span": item["span"] })).collect());
    Value::Array(list)
}

/// Each program of this corpus has one error, and the report says what `expected.json` says: everything but the text of
/// `message` and `fixes`. `fix_mentions` is not a field of the report: some fix must mention that text, or each of these texts.
#[test]
fn the_programs_of_scenario_2_give_their_one_diagnostic() {
    let root = repository().join("tests/acceptance/type-errors");
    let cases: Vec<PathBuf> = fs::read_dir(&root).unwrap().map(|entry| entry.unwrap().path()).filter(|path| path.is_dir()).collect();
    assert_eq!(cases.len(), 9);
    for case in cases {
        let name = case.file_name().unwrap().to_str().unwrap().to_owned();
        let expected: Value = serde_json::from_slice(&fs::read(case.join("expected.json")).unwrap()).unwrap();
        let (exit, document) = json_in(&case, &["."]);
        assert_eq!(exit, 1, "{name}");
        assert_eq!(document["schema"], expected["schema"], "{name}");
        assert_eq!(document["status"], expected["status"], "{name}");
        assert_eq!(document["summary"]["errors"], expected["summary"]["errors"], "{name}");
        assert_eq!(document["summary"]["warnings"], expected["summary"]["warnings"], "{name}");
        let (found, wanted) = (document["diagnostics"].as_array().unwrap(), expected["diagnostics"].as_array().unwrap());
        assert_eq!(found.len(), wanted.len(), "{name}: {found:?}");
        for (found, wanted) in found.iter().zip(wanted) {
            for field in ["code", "severity", "file", "span", "function", "node", "wires", "expected", "found"] {
                assert_eq!(found[field], wanted[field], "{name}: {field}");
            }
            assert_eq!(places(&found["related"]), places(&wanted["related"]), "{name}: related");
            for mention in
                wanted["fix_mentions"].as_array().into_iter().flatten().chain(Some(&wanted["fix_mentions"])).filter_map(|text| text.as_str())
            {
                assert!(found["fixes"].to_string().contains(mention), "{name}: no fix mentions {mention}: {}", found["fixes"]);
            }
        }
    }
}

/// "Each code marked `S2` in `docs/cli.md` has one case" (`tests/acceptance/README.md`).
#[test]
fn each_code_marked_s2_has_one_case() {
    let root = repository();
    let cli = fs::read_to_string(root.join("docs/cli.md")).unwrap();
    let marked: BTreeSet<String> =
        cli.lines().filter(|line| line.trim_end().ends_with("| S2 |")).map(|line| line.split('`').nth(1).unwrap().to_owned()).collect();
    let mut cases = BTreeSet::new();
    for entry in fs::read_dir(root.join("tests/acceptance/type-errors")).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            let expected: Value = serde_json::from_slice(&fs::read(path.join("expected.json")).unwrap()).unwrap();
            let code = expected["diagnostics"][0]["code"].as_str().unwrap().to_owned();
            assert!(cases.insert(code), "{}", path.display());
        }
    }
    assert_eq!(marked, cases);
}

fn expected_codes(expected: &Value) -> BTreeSet<String> {
    expected["check"]["codes"].as_array().unwrap().iter().map(|code| code.as_str().unwrap().to_owned()).collect()
}

/// The half of `expected.json` for `check`, run from inside each folder, as the corpus says.
#[test]
fn the_stored_hostile_files_give_what_check_expects_so_far() {
    let root = repository().join("tests/acceptance/hostile");
    let cases: Vec<PathBuf> = fs::read_dir(&root).unwrap().map(|entry| entry.unwrap().path()).collect();
    assert_eq!(cases.len(), 30);
    for case in cases {
        let name = case.file_name().unwrap().to_str().unwrap().to_owned();
        let expected: Value = serde_json::from_slice(&fs::read(case.join("expected.json")).unwrap()).unwrap();
        let codes = expected_codes(&expected);
        let (exit, document) = json_in(&case, &["."]);
        let found: BTreeSet<String> = codes_of(&document).into_iter().collect();
        assert!(found.is_subset(&codes), "{name}: {found:?} is not in {codes:?}");
        if !later().contains(&name.as_str()) {
            assert_eq!(found, codes, "{name}");
            assert_eq!(i64::from(exit), expected["check"]["exit"].as_i64().unwrap(), "{name}");
        }
    }
}
