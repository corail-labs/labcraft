//! `labcraft-cli fmt` as a user runs it (`docs/cli.md`, section 2), and the families of scenario 4 that need files
//! (`tests/acceptance/README.md`): Size and Library, the stored cases, and Nesting at the limits of the options.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

#[path = "../../../crates/format/tests/deep/mod.rs"]
#[allow(dead_code)]
mod deep;

const CANONICAL: &str = "labcraft 1\n\nfn f() -> () {\n    x = 1\n}\n";
const COMPACT: &str = "labcraft 1\nfn f()->(){x=1\n}\n";
const MIB: u64 = 1024 * 1024;

/// A folder of its own for a test, removed at its end.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("fmt-{name}"));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Folder(path)
    }

    fn file(&self, relative: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    /// A file of that size that takes no time to make.
    fn sized(&self, relative: &str, size: u64) -> PathBuf {
        let path = self.file(relative, b"");
        fs::File::options().write(true).open(&path).unwrap().set_len(size).unwrap();
        path
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_labcraft-cli")).current_dir(&self.0).arg("fmt").args(args).output().expect("the program starts")
    }

    fn json(&self, args: &[&str]) -> (i32, Value) {
        let output = self.run(&[&["--json"], args].concat());
        assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
        (output.status.code().expect("the process ended normally"), serde_json::from_slice(&output.stdout).unwrap())
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn codes(document: &Value) -> BTreeSet<String> {
    document["diagnostics"].as_array().unwrap().iter().map(|diagnostic| diagnostic["code"].as_str().unwrap().to_owned()).collect()
}

fn set(codes: &[&str]) -> BTreeSet<String> {
    codes.iter().map(|code| (*code).to_owned()).collect()
}

fn results(document: &Value) -> Vec<(String, String)> {
    let files = document["files"].as_array().unwrap();
    files.iter().map(|file| (file["file"].as_str().unwrap().to_owned(), file["result"].as_str().unwrap().to_owned())).collect()
}

fn pair(file: &str, result: &str) -> (String, String) {
    (file.to_owned(), result.to_owned())
}

/// The root of the repository, whatever the current folder of the test.
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

#[test]
fn a_file_is_rewritten_and_a_second_run_changes_nothing() {
    let folder = Folder::new("rewrite");
    let path = folder.file("a.lcf", COMPACT);
    let (exit, document) = folder.json(&["."]);
    assert_eq!(exit, 0);
    assert_eq!(fs::read_to_string(&path).unwrap(), CANONICAL);
    assert_eq!(document["schema"], "labcraft-fmt/1");
    assert_eq!(document["status"], "ok");
    assert_eq!(document["summary"], serde_json::json!({ "files": 1, "changed": 1, "errors": 0, "warnings": 0, "truncated": false }));
    assert_eq!(results(&document), [pair("a.lcf", "formatted")]);
    assert_eq!(document["diagnostics"], serde_json::json!([]));
    let (exit, document) = folder.json(&["."]);
    assert_eq!(exit, 0);
    assert_eq!(document["summary"]["changed"], 0);
    assert_eq!(results(&document), [pair("a.lcf", "unchanged")]);
}

#[test]
fn a_canonical_file_is_not_touched() {
    let folder = Folder::new("untouched");
    let path = folder.file("a.lcf", CANONICAL);
    let long_ago = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    fs::File::options().write(true).open(&path).unwrap().set_modified(long_ago).unwrap();
    let (exit, _) = folder.json(&["."]);
    assert_eq!(exit, 0);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), long_ago);
}

#[test]
fn line_endings_and_a_byte_order_mark_are_rewritten() {
    let folder = Folder::new("endings");
    let crlf = folder.file("crlf.lcf", CANONICAL.replace('\n', "\r\n"));
    let bom = folder.file("bom.lcf", format!("\u{FEFF}{CANONICAL}"));
    let (exit, document) = folder.json(&["."]);
    assert_eq!(exit, 0);
    assert_eq!(document["summary"]["changed"], 2);
    assert_eq!(fs::read_to_string(crlf).unwrap(), CANONICAL);
    assert_eq!(fs::read_to_string(bom).unwrap(), CANONICAL);
}

#[test]
fn check_writes_nothing_and_locates_the_first_line_that_differs() {
    let folder = Folder::new("check");
    let path = folder.file("a.lcf", "labcraft 1\n\nfn f() -> () {\n  x = 1\n}\n");
    folder.file("b.lcf", CANONICAL);
    let (exit, document) = folder.json(&["--check", "."]);
    assert_eq!(exit, 1);
    assert_eq!(document["status"], "error");
    assert_eq!(fs::read_to_string(path).unwrap(), "labcraft 1\n\nfn f() -> () {\n  x = 1\n}\n");
    assert_eq!(results(&document), [pair("a.lcf", "not_canonical"), pair("b.lcf", "unchanged")]);
    assert_eq!(document["summary"]["changed"], 0);
    let diagnostic = &document["diagnostics"][0];
    assert_eq!(diagnostic["code"], "LC6001");
    assert_eq!(diagnostic["file"], "a.lcf");
    assert_eq!(diagnostic["span"], serde_json::json!({ "start": { "line": 4, "column": 1 }, "end": { "line": 4, "column": 8 } }));
}

#[test]
fn a_clean_check_is_exit_0() {
    let folder = Folder::new("clean");
    folder.file("a.lcf", CANONICAL);
    assert_eq!(folder.json(&["--check", "."]).0, 0);
}

#[test]
fn a_bad_file_stays_as_it_is_and_hides_nothing() {
    let folder = Folder::new("bad");
    let bad = folder.file("a.lcf", "labcraft 1\nfn f(");
    let good = folder.file("b.lcf", COMPACT);
    let (exit, document) = folder.json(&["."]);
    assert_eq!(exit, 1);
    assert_eq!(fs::read_to_string(bad).unwrap(), "labcraft 1\nfn f(");
    assert_eq!(fs::read_to_string(good).unwrap(), CANONICAL);
    assert_eq!(results(&document), [pair("a.lcf", "error"), pair("b.lcf", "formatted")]);
    assert_eq!(codes(&document), set(&["LC1002"]));
    assert_eq!(document["diagnostics"][0]["file"], "a.lcf");
}

#[test]
fn a_warning_does_not_change_the_exit_code() {
    let folder = Folder::new("warning");
    folder.file("a.lcf", "labcraft 1\n\nfn f() -> () {\n    x = \"\u{202E}\"\n}\n");
    let (exit, document) = folder.json(&["."]);
    assert_eq!(exit, 0);
    assert_eq!(document["status"], "ok");
    assert_eq!(codes(&document), set(&["LC0005"]));
    assert_eq!(document["summary"]["warnings"], 1);
}

#[test]
fn the_human_form_starts_each_diagnostic_with_its_place_and_ends_with_the_summary() {
    let folder = Folder::new("human");
    folder.file("sub/a.lcf", "labcraft 1\nfn f(");
    folder.file("b.lcf", COMPACT);
    let output = folder.run(&["."]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "formatted b.lcf");
    assert!(lines[1].starts_with("sub/a.lcf:2:6: error[LC1002]: "), "{text}");
    assert_eq!(*lines.last().unwrap(), "fmt: 2 files, 1 changed: 1 error, 0 warnings");
}

#[test]
fn a_report_can_be_cut_and_its_counts_stay_complete() {
    let folder = Folder::new("cut");
    for name in ["a", "b", "c"] {
        folder.file(&format!("{name}.lcf"), b"x");
    }
    let (exit, document) = folder.json(&["--max-report", "1", "."]);
    assert_eq!(exit, 1);
    assert_eq!(document["summary"]["errors"], 3);
    assert_eq!(document["summary"]["truncated"], true);
    assert_eq!(document["diagnostics"].as_array().unwrap().len(), 1);
    let (exit, document) = folder.json(&["--max-report", "0", "."]);
    assert_eq!(exit, 1);
    assert_eq!(document["status"], "error");
    assert_eq!(document["diagnostics"], serde_json::json!([]));
}

#[test]
fn paths_use_a_slash_and_stay_relative_when_they_are_under_the_current_folder() {
    let folder = Folder::new("paths");
    let path = folder.file("sub/a.lcf", CANONICAL);
    let (_, by_folder) = folder.json(&["sub"]);
    let (_, by_absolute) = folder.json(&[path.to_str().unwrap()]);
    let (_, by_dots) = folder.json(&["./sub/../sub/a.lcf"]);
    assert_eq!(results(&by_folder), [pair("sub/a.lcf", "unchanged")]);
    assert_eq!(results(&by_absolute), [pair("sub/a.lcf", "unchanged")]);
    assert_eq!(by_dots["summary"]["files"], 1);
}

#[test]
fn a_file_named_without_the_extension_is_refused() {
    let folder = Folder::new("extension");
    let path = folder.file("a.txt", COMPACT);
    let (exit, document) = folder.json(&["a.txt"]);
    assert_eq!(exit, 1);
    assert_eq!(codes(&document), set(&["LC0008"]));
    assert_eq!(results(&document), [pair("a.txt", "error")]);
    assert_eq!(fs::read_to_string(path).unwrap(), COMPACT);
}

#[test]
fn a_path_that_starts_with_a_dash_can_be_named() {
    let folder = Folder::new("dash");
    folder.file("-a.lcf", COMPACT);
    let (exit, document) = folder.json(&["--", "-a.lcf"]);
    assert_eq!(exit, 0);
    assert_eq!(results(&document), [pair("-a.lcf", "formatted")]);
}

#[test]
fn a_read_only_file_is_lc6003_at_once_and_stays() {
    let folder = Folder::new("read-only");
    let path = folder.file("a.lcf", COMPACT);
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions.clone()).unwrap();
    let (exit, document) = folder.json(&["."]);
    assert_eq!(fs::read_to_string(&path).unwrap(), COMPACT);
    // Removing a read-only file needs this on Windows.
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(&path, permissions).unwrap();
    assert_eq!(exit, 1);
    assert_eq!(codes(&document), set(&["LC6003"]));
    assert_eq!(results(&document), [pair("a.lcf", "error")]);
    // Not the error of a rename, which on Windows comes after retrying for most of a second.
    assert_eq!(document["diagnostics"][0]["message"], "the file is read-only, so it stays as it is");
}

#[test]
fn files_and_refused_files_are_listed_in_the_order_of_the_paths() {
    let folder = Folder::new("list-order");
    folder.file("a.txt", b"");
    folder.file("b.lcf", CANONICAL);
    folder.file("c.txt", b"");
    folder.file("Z.lcf", CANONICAL);
    let (_, document) = folder.json(&["c.txt", "b.lcf", "a.txt", "Z.lcf"]);
    assert_eq!(results(&document), [pair("Z.lcf", "unchanged"), pair("a.txt", "error"), pair("b.lcf", "unchanged"), pair("c.txt", "error")]);
}

#[test]
fn paths_written_another_way_count_once_and_the_order_of_the_arguments_changes_nothing() {
    let folder = Folder::new("once");
    folder.file("w/a.lcf", CANONICAL);
    folder.file("w/sub/b.lcf", CANONICAL);
    folder.file("w/empty/note.txt", b"");
    let inside = folder.0.join("w");
    let run = |args: &[&str]| {
        let output =
            Command::new(env!("CARGO_BIN_EXE_labcraft-cli")).current_dir(&inside).args(["fmt", "--json", "--check"]).args(args).output().unwrap();
        let document: Value = serde_json::from_slice(&output.stdout).unwrap();
        (output.status.code().unwrap(), document)
    };
    let (exit, document) = run(&["../w", ".", "sub/../sub/b.lcf", "sub/b.lcf"]);
    assert_eq!(exit, 0);
    assert_eq!(results(&document), [pair("a.lcf", "unchanged"), pair("sub/b.lcf", "unchanged")]);
    let (one_exit, one) = run(&[".", "empty"]);
    let (other_exit, other) = run(&["empty", "."]);
    assert_eq!((one_exit, codes(&one)), (1, set(&["LC0002"])));
    assert_eq!((other_exit, &other), (one_exit, &one));
}

#[test]
fn a_file_under_the_current_folder_is_relative_to_it_whatever_path_reached_it() {
    let folder = Folder::new("relative");
    folder.file("a.lcf", CANONICAL);
    folder.file("sub/b.lcf", CANONICAL);
    let inside = folder.0.join("sub");
    let output =
        Command::new(env!("CARGO_BIN_EXE_labcraft-cli")).current_dir(&inside).args(["fmt", "--json", "--check", "..", "b.lcf"]).output().unwrap();
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(results(&document), [pair("../a.lcf", "unchanged"), pair("b.lcf", "unchanged")]);
}

#[test]
fn a_message_says_the_cause_without_a_path() {
    let folder = Folder::new("message");
    let (_, document) = folder.json(&["nowhere"]);
    assert_eq!(document["diagnostics"][0]["message"], "cannot read this path: entity not found");
}

#[test]
fn a_message_about_size_gives_the_size_of_the_file() {
    let folder = Folder::new("size-message");
    folder.sized("a.lcf", 100 * MIB);
    let (_, document) = folder.json(&["."]);
    let message = document["diagnostics"][0]["message"].as_str().unwrap();
    assert!(message.contains(&(100 * MIB).to_string()), "{message}");
}

// Family Library.

#[test]
fn a_missing_path_is_lc0001_and_hides_nothing() {
    let folder = Folder::new("missing");
    folder.file("a.lcf", COMPACT);
    let (exit, document) = folder.json(&["nowhere", "a.lcf", ""]);
    assert_eq!(exit, 1);
    assert_eq!(codes(&document), set(&["LC0001"]));
    assert_eq!(document["summary"]["errors"], 2);
    assert_eq!(results(&document), [pair("a.lcf", "formatted")]);
}

#[test]
fn a_folder_with_nothing_to_check_is_lc0002() {
    let folder = Folder::new("nothing");
    folder.file("note.txt", b"");
    let (exit, document) = folder.json(&["."]);
    assert_eq!(exit, 1);
    assert_eq!(codes(&document), set(&["LC0002"]));
    assert_eq!(document["diagnostics"][0]["file"], ".");
}

/// False where the user may not make links, as on Windows without the privilege: the test then proves nothing, and
/// passes. GitHub's Windows runners have it.
#[cfg(unix)]
fn link(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn link(target: &Path, link: &Path) -> bool {
    if target.is_dir() { std::os::windows::fs::symlink_dir(target, link).is_ok() } else { std::os::windows::fs::symlink_file(target, link).is_ok() }
}

#[test]
fn a_symbolic_link_loop_next_to_a_clean_file_is_lc0009_and_exit_0() {
    let folder = Folder::new("link-loop");
    folder.file("a.lcf", CANONICAL);
    if !link(&folder.0, &folder.0.join("loop")) {
        return;
    }
    let (exit, document) = folder.json(&["."]);
    assert_eq!(exit, 0);
    assert_eq!(codes(&document), set(&["LC0009"]));
    assert_eq!(document["diagnostics"][0]["file"], "loop");
    assert_eq!(results(&document), [pair("a.lcf", "unchanged")]);
}

#[test]
fn a_named_link_is_followed_and_stays_a_link() {
    let folder = Folder::new("named-link");
    let target = folder.file("target.lcf", COMPACT);
    if !link(&target, &folder.0.join("link.lcf")) {
        return;
    }
    let (exit, document) = folder.json(&["link.lcf"]);
    assert_eq!(exit, 0);
    assert_eq!(results(&document), [pair("link.lcf", "formatted")]);
    assert!(fs::symlink_metadata(folder.0.join("link.lcf")).unwrap().file_type().is_symlink());
    assert_eq!(fs::read_to_string(target).unwrap(), CANONICAL);
}

#[test]
fn a_named_link_and_its_target_are_one_file() {
    let folder = Folder::new("link-and-target");
    let target = folder.file("target.lcf", CANONICAL);
    if !link(&target, &folder.0.join("link.lcf")) {
        return;
    }
    let (_, document) = folder.json(&["--check", "link.lcf", "target.lcf"]);
    assert_eq!(document["summary"]["files"], 1);
}

#[test]
fn a_link_to_something_other_than_a_program_is_refused_and_what_it_points_to_stays() {
    let folder = Folder::new("link-to-text");
    let target = folder.file("notes.txt", COMPACT);
    if !link(&target, &folder.0.join("link.lcf")) {
        return;
    }
    let (exit, document) = folder.json(&["link.lcf"]);
    assert_eq!(exit, 1);
    assert_eq!(codes(&document), set(&["LC0008"]));
    assert_eq!(fs::read_to_string(target).unwrap(), COMPACT);
}

#[test]
fn a_link_with_the_wrong_name_does_not_stop_its_target() {
    let folder = Folder::new("link-wrong-name");
    let target = folder.file("target.lcf", COMPACT);
    if !link(&target, &folder.0.join("alink.txt")) {
        return;
    }
    let (exit, document) = folder.json(&["alink.txt", "target.lcf"]);
    assert_eq!((exit, codes(&document)), (1, set(&["LC0008"])));
    assert_eq!(results(&document), [pair("alink.txt", "error"), pair("target.lcf", "formatted")]);
    assert_eq!(fs::read_to_string(target).unwrap(), CANONICAL);
}

#[test]
fn a_file_also_reached_under_its_real_name_is_shown_under_it() {
    let folder = Folder::new("link-real-name");
    let target = folder.file("target.lcf", CANONICAL);
    if !link(&target, &folder.0.join("alink.lcf")) {
        return;
    }
    let (_, document) = folder.json(&[".", "alink.lcf"]);
    assert_eq!(results(&document), [pair("target.lcf", "unchanged")]);
}

#[test]
fn a_file_reached_through_a_link_to_its_folder_keeps_its_real_name() {
    let folder = Folder::new("folder-link");
    folder.file("dir/a.lcf", CANONICAL);
    let real = folder.0.join("dir");
    if !link(&real, &folder.0.join("alink")) || !link(&real, &folder.0.join("zlink")) {
        return;
    }
    for name in ["alink", "zlink"] {
        let (_, document) = folder.json(&["--check", name, "dir"]);
        assert_eq!(results(&document), [pair("dir/a.lcf", "unchanged")], "{name}");
    }
}

#[test]
fn a_named_link_that_leads_nowhere_is_lc0001() {
    let folder = Folder::new("dangling");
    if !link(&folder.0.join("nowhere.lcf"), &folder.0.join("link.lcf")) {
        return;
    }
    let (exit, document) = folder.json(&["link.lcf"]);
    assert_eq!((exit, codes(&document)), (1, set(&["LC0001"])));
    assert_eq!(document["diagnostics"][0]["file"], "link.lcf");
}

/// Root reads everything, so the test proves nothing there, and says so by passing without a check.
#[cfg(unix)]
#[test]
fn an_unreadable_file_is_lc0001() {
    use std::os::unix::fs::PermissionsExt as _;
    let folder = Folder::new("unreadable");
    let path = folder.file("a.lcf", COMPACT);
    folder.file("b.lcf", CANONICAL);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(&path).is_ok() {
        return;
    }
    let (exit, document) = folder.json(&["."]);
    assert_eq!(exit, 1);
    assert_eq!(codes(&document), set(&["LC0001"]));
    assert_eq!(results(&document), [pair("a.lcf", "error"), pair("b.lcf", "unchanged")]);
}

#[cfg(unix)]
#[test]
fn a_file_that_cannot_be_replaced_is_lc6003() {
    use std::os::unix::fs::PermissionsExt as _;
    let folder = Folder::new("locked-folder");
    let path = folder.file("sub/a.lcf", COMPACT);
    let sub = folder.0.join("sub");
    fs::set_permissions(&sub, fs::Permissions::from_mode(0o555)).unwrap();
    let writable = fs::write(sub.join("probe"), b"").is_ok();
    let (exit, document) = folder.json(&["sub"]);
    fs::set_permissions(&sub, fs::Permissions::from_mode(0o755)).unwrap();
    if writable {
        return;
    }
    assert_eq!(exit, 1);
    assert_eq!(codes(&document), set(&["LC6003"]));
    assert_eq!(fs::read_to_string(path).unwrap(), COMPACT);
}

// Family Size.

#[test]
fn a_file_over_the_limit_is_lc0006_and_stays() {
    let folder = Folder::new("over");
    let large = folder.sized("large.lcf", 100 * MIB);
    let just_over = folder.sized("just_over.lcf", 16 * MIB + 1);
    let (exit, document) = folder.json(&["."]);
    assert_eq!(exit, 1);
    assert_eq!(codes(&document), set(&["LC0006"]));
    assert_eq!(document["summary"]["errors"], 2);
    assert_eq!(fs::metadata(large).unwrap().len(), 100 * MIB);
    assert_eq!(fs::metadata(just_over).unwrap().len(), 16 * MIB + 1);
}

#[test]
fn a_file_at_the_limit_is_read() {
    let folder = Folder::new("at-limit");
    let head = "labcraft 1\n\n// ";
    let tail = "\nfn f() -> () {}\n";
    let comment = "x".repeat(16 * MIB as usize - head.len() - tail.len());
    folder.file("a.lcf", format!("{head}{comment}{tail}"));
    let (exit, document) = folder.json(&["--check", "."]);
    assert_eq!(codes(&document), set(&[]));
    assert_eq!(exit, 0);
    folder.file("a.lcf", format!("{head}{comment}x{tail}"));
    assert_eq!(codes(&folder.json(&["--check", "."]).1), set(&["LC0006"]));
}

#[test]
fn the_limit_of_size_is_an_option() {
    let folder = Folder::new("size-option");
    folder.file("a.lcf", CANONICAL);
    let size = CANONICAL.len();
    assert_eq!(folder.json(&["--max-file-size", &size.to_string(), "."]).0, 0);
    let (exit, document) = folder.json(&["--max-file-size", &(size - 1).to_string(), "."]);
    assert_eq!(exit, 1);
    assert_eq!(codes(&document), set(&["LC0006"]));
}

#[test]
fn a_line_of_10_mb_is_read() {
    let folder = Folder::new("long-line");
    folder.file("a.lcf", format!("labcraft 1\n\n// {}\nfn f() -> () {{}}\n", "x".repeat(10_000_000)));
    assert_eq!(folder.json(&["--check", "."]).0, 0);
}

#[test]
fn a_function_of_300000_nodes_ends_normally() {
    let folder = Folder::new("many-nodes");
    let mut body = String::from("labcraft 1\n\nfn f() -> () {\n");
    for index in 0..300_000 {
        body.push_str(&format!("    a{index:06} = 1\n"));
    }
    body.push_str("}\n");
    assert!(body.len() < 16 * MIB as usize);
    folder.file("a.lcf", &body);
    let (exit, document) = folder.json(&["--check", "."]);
    assert_eq!((exit, codes(&document)), (0, set(&[])));
}

/// Short lines deep down, whose canonical form is hundreds of times their size. On a small limit, since the
/// generated file of `crates/format/tests/write.rs` already covers the default one.
#[test]
fn a_canonical_form_over_the_limit_is_lc6003_and_the_file_stays() {
    let folder = Folder::new("too-large");
    let text = deep::deep_loop(200);
    let path = folder.file("a.lcf", &text);
    let limit = (text.len() + 1).to_string();
    let (exit, document) = folder.json(&["--max-file-size", &limit, "."]);
    assert_eq!(exit, 1);
    assert_eq!(codes(&document), set(&["LC6003"]));
    assert_eq!(results(&document), [pair("a.lcf", "error")]);
    assert_eq!(fs::read_to_string(path).unwrap(), text);
}

/// Making the files takes about 40 s on a Windows machine with an antivirus, on every local run of the tests. The
/// job `windows` of the CI runs it.
#[cfg_attr(windows, ignore = "making 100,000 files is slow on Windows; the CI runs it")]
#[test]
fn a_library_of_100001_files_is_lc0010_and_100000_of_them_are_read() {
    let folder = Folder::new("many-files");
    for index in 0..100_000 {
        folder.file(&format!("{:03}/{index:06}.lcf", index % 500), CANONICAL);
    }
    let (exit, document) = folder.json(&["--check", "."]);
    assert_eq!((exit, codes(&document)), (0, set(&[])));
    assert_eq!(document["summary"]["files"], 100_000);
    folder.file("999/last.lcf", CANONICAL);
    let (exit, document) = folder.json(&["--check", "."]);
    assert_eq!((exit, codes(&document)), (1, set(&["LC0010"])));
    assert_eq!(document["summary"]["files"], 100_000);
    assert!(results(&document).iter().all(|(file, _)| file != "999/last.lcf"), "the last path in order is the one left out");
}

// Family Truncation.

fn programs_under(folder: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            programs_under(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "lcf" || extension == "lct") {
            found.push(path);
        }
    }
}

/// Every prefix of every file of `examples/` and `libs/`, in one run.
#[test]
fn a_prefix_never_crashes_and_a_file_that_does_not_parse_stays() {
    let folder = Folder::new("prefixes");
    let mut programs = Vec::new();
    programs_under(&repository().join("examples"), &mut programs);
    programs_under(&repository().join("libs"), &mut programs);
    assert!(programs.len() > 20);
    let mut prefixes = Vec::new();
    for (index, path) in programs.iter().enumerate() {
        let bytes = fs::read(path).unwrap();
        let extension = path.extension().unwrap().to_str().unwrap();
        for end in 0..=bytes.len() {
            let name = format!("{index}/{end}.{extension}");
            folder.file(&name, &bytes[..end]);
            prefixes.push((name, end, bytes.clone()));
        }
    }
    let (exit, document) = folder.json(&["--max-report", "0", "."]);
    assert!(exit == 0 || exit == 1);
    let files = results(&document);
    assert_eq!(files.len(), prefixes.len());
    let by_name: std::collections::HashMap<&str, &str> = files.iter().map(|(file, result)| (file.as_str(), result.as_str())).collect();
    for (name, end, bytes) in &prefixes {
        if by_name[name.as_str()] == "error" {
            assert_eq!(fs::read(folder.0.join(name)).unwrap(), bytes[..*end], "{name}");
        }
    }
}

// Family Nesting.

#[test]
fn nesting_beyond_the_limit_is_lc0007_at_any_depth() {
    let folder = Folder::new("nesting");
    for (kind, generator) in deep::DEEP {
        let name = kind.replace(' ', "-");
        folder.file(&format!("{name}.lcf"), generator(100_000));
    }
    let (exit, document) = folder.json(&["."]);
    assert_eq!(exit, 1);
    assert_eq!(codes(&document), set(&["LC0007"]));
    assert_eq!(document["summary"]["errors"], deep::DEEP.len());
}

/// The real program, on the stack it gives itself: in a debug build, the deepest `match` takes over 12 MiB.
#[test]
fn the_ceiling_of_nesting_holds_on_the_stack_of_the_program() {
    let folder = Folder::new("ceiling");
    for (kind, generator) in deep::DEEP {
        let name = kind.replace(' ', "-");
        folder.file(&format!("{name}.lcf"), generator(1024));
    }
    // At this depth, the indentation alone can pass the default limit of size.
    let huge = labcraft_types::MAX_FILE_SIZE.to_string();
    let (exit, document) = folder.json(&["--max-nesting", "1024", "--max-file-size", &huge, "."]);
    assert_eq!((exit, codes(&document)), (0, set(&[])));
    assert_eq!(document["summary"]["files"], deep::DEEP.len());
    let (exit, document) = folder.json(&["--check", "--max-nesting", "1023", "--max-file-size", &huge, "."]);
    assert_eq!(exit, 1);
    assert_eq!(codes(&document), set(&["LC0007"]));
}

// The stored corpus and the programs of the repository.

/// For `labcraft-cli fmt .` on a copy of the folder (`tests/acceptance/README.md`).
#[test]
fn the_stored_hostile_cases_give_what_is_expected() {
    let corpus = repository().join("tests/acceptance/hostile");
    let mut seen = 0;
    for entry in fs::read_dir(corpus).unwrap() {
        let case = entry.unwrap().path();
        let name = case.file_name().unwrap().to_str().unwrap().to_owned();
        let expected: Value = serde_json::from_slice(&fs::read(case.join("expected.json")).unwrap()).unwrap();
        let expected = &expected["fmt"];
        let folder = Folder::new(&format!("hostile-{name}"));
        let mut originals = Vec::new();
        for file in fs::read_dir(&case).unwrap() {
            let file = file.unwrap();
            let bytes = fs::read(file.path()).unwrap();
            let copy = folder.file(file.file_name().to_str().unwrap(), &bytes);
            originals.push((copy, bytes));
        }
        let (exit, document) = folder.json(&["."]);
        assert_eq!(i64::from(exit), expected["exit"].as_i64().unwrap(), "{name}");
        let wanted: BTreeSet<String> = expected["codes"].as_array().unwrap().iter().map(|code| code.as_str().unwrap().to_owned()).collect();
        assert_eq!(codes(&document), wanted, "{name}");
        let changed = originals.iter().any(|(copy, bytes)| &fs::read(copy).unwrap() != bytes);
        assert_eq!(changed, expected["changes"].as_bool().unwrap(), "{name}");
        seen += 1;
    }
    assert_eq!(seen, 30);
}

#[test]
fn the_programs_of_the_repository_are_canonical() {
    let output = Command::new(env!("CARGO_BIN_EXE_labcraft-cli"))
        .current_dir(repository())
        .args(["fmt", "--check", "--json", "examples", "docs/rfc", "libs"])
        .output()
        .expect("the program starts");
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["diagnostics"], serde_json::json!([]));
    assert_eq!(output.status.code(), Some(0));
    assert!(document["summary"]["files"].as_u64().unwrap() > 20);
    assert!(results(&document).iter().any(|(file, _)| file == "libs/visa/close.lcf"), "paths are relative, with a slash");
}
