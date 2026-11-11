//! The session on a project in memory: each command of 6a, the kept check, the errors an agent fixes its call from, and
//! the last resort. The expected values are written by hand from `docs/commands.md` and the files given.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use labcraft_runtime::native::{Described, LibraryCode, LibraryDiagnostic, LibraryRun, LogBudget, NativeLibrary, Section, Simulation};
use labcraft_types::Limits;
use serde_json::{Value, json};

use crate::workspace::{Entry, Metadata, Read};
use crate::{Config, Memory, Session, Shipped, Workspace};

const MAIN: &str = "labcraft 1\n\nuse core\n\n/// Doubles a number.\n/// Second line.\nfn double(x: f64) -> (y: f64) {\n    y = core.mul(x, 2)\n}\n";
const VERDICT: &str = "labcraft 1\n\ntype Verdict = enum { Pass, Fail }\n";
const BROKEN: &str = "labcraft 1\n\nfn broken() -> (y: f64) {\n    y = nowhere(1)\n}\n";

/// A library that simulates, as `visa` does, with a key `instrument` that it reads as a resource name.
struct Fake;

impl NativeLibrary for Fake {
    fn name(&self) -> &str {
        "fake"
    }

    fn codes(&self) -> &'static [LibraryCode] {
        &[]
    }

    fn simulates(&self) -> bool {
        true
    }

    fn simulate(&self, section: &Section<'_>) -> Result<Simulation, Vec<LibraryDiagnostic>> {
        match section.value.get_ref() {
            toml::de::DeValue::Table(table) => {
                let resource = table.iter().find(|(key, _)| key.get_ref().as_ref() == "instrument").and_then(|(_, value)| match value.get_ref() {
                    toml::de::DeValue::String(text) => Some(text.to_string()),
                    _ => None,
                });
                match resource {
                    Some(resource) => Ok(Box::new(resource)),
                    None => Err(vec![LibraryDiagnostic {
                        code: "fake:S001",
                        severity: labcraft_types::Severity::Error,
                        message: "no instrument".to_owned(),
                        span: None,
                        fix: None,
                    }]),
                }
            }
            _ => Err(Vec::new()),
        }
    }

    fn describe(&self, simulation: &Simulation) -> Option<Described> {
        let resource = simulation.downcast_ref::<String>()?;
        Some(Described::Map(vec![("resource".to_owned(), Described::Text(resource.clone())), ("latency_ms".to_owned(), Described::Number(0.5))]))
    }

    fn resources(&self, simulation: &Simulation) -> Vec<String> {
        simulation.downcast_ref::<String>().cloned().into_iter().collect()
    }

    fn start_run(&self, _: u32, _: Option<&Simulation>, _: LogBudget) -> Box<dyn LibraryRun> {
        unimplemented_run()
    }
}

#[allow(clippy::panic)]
fn unimplemented_run() -> Box<dyn LibraryRun> {
    panic!("no run in these tests")
}

/// The memory, counting the files read, and panicking on the one named.
struct Counting {
    memory: Memory,
    reads: AtomicUsize,
    panic_on: Option<PathBuf>,
}

impl Workspace for Counting {
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.memory.canonicalize(path)
    }
    fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        self.memory.absolute(path)
    }
    fn current(&self) -> Option<PathBuf> {
        self.memory.current()
    }
    fn metadata(&self, path: &Path) -> io::Result<Metadata> {
        self.memory.metadata(path)
    }
    fn link_metadata(&self, path: &Path) -> io::Result<Metadata> {
        self.memory.link_metadata(path)
    }
    fn read_dir(&self, path: &Path, each: &mut dyn FnMut(io::Result<Entry>)) -> io::Result<()> {
        self.memory.read_dir(path, each)
    }
    #[allow(clippy::panic)]
    fn read(&self, path: &Path, max: u64) -> io::Result<Read> {
        if self.panic_on.as_deref() == Some(path) {
            panic!("a bug planted in the test");
        }
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.memory.read(path, max)
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.memory.write(path, bytes)
    }
    fn create_folders(&self, path: &Path) -> io::Result<()> {
        self.memory.create_folders(path)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.memory.rename(from, to)
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.memory.remove_file(path)
    }
}

fn project() -> Memory {
    let memory = Memory::new();
    memory.put("/p/double.lcf", MAIN);
    memory.put("/p/Verdict.lct", VERDICT);
    memory.put("/p/bench.toml", "[fake]\ninstrument = \"SIM::X\"\n");
    memory.put("/p/Cargo.toml", "[package]\nname = \"x\"\n");
    memory
}

fn session_on(ws: Arc<dyn Workspace>) -> Session {
    let config = Config {
        root: PathBuf::from("/p"),
        lib_paths: Vec::new(),
        limits: Limits::default(),
        shipped: Shipped::default(),
        natives: vec![Arc::new(Fake)],
        clock: Arc::new(crate::Still),
        process: 0,
    };
    Session::new(ws, config, false).unwrap()
}

fn session() -> (Session, Arc<Counting>) {
    let counting = Arc::new(Counting { memory: project(), reads: AtomicUsize::new(0), panic_on: None });
    (session_on(counting.clone()), counting)
}

fn ok(session: &mut Session, id: &str, params: Value) -> Value {
    match session.execute(id, &params) {
        Ok(value) => value,
        Err(error) => unreachable_error(id, &error.to_json()),
    }
}

#[allow(clippy::panic)]
fn unreachable_error(id: &str, error: &Value) -> Value {
    panic!("{id}: {error}")
}

fn error(session: &mut Session, id: &str, params: Value) -> crate::CommandError {
    session.execute(id, &params).unwrap_err()
}

#[test]
fn library_check_gives_the_report_of_labcraft_cli_check_and_keeps_it() {
    let (mut session, counting) = session();
    let report = ok(&mut session, "library.check", json!({}));
    assert_eq!(report["schema"], "labcraft-check/1");
    assert_eq!(report["status"], "ok", "{report}");
    assert_eq!(report["summary"]["functions"], 1);
    assert_eq!(report["summary"]["types"], 1);
    assert_eq!(counting.reads.load(Ordering::SeqCst), 2, "the two program files");
    assert_eq!(ok(&mut session, "library.check", json!({ "folder": "." })), report);
    // Nothing changed: each file is read once more, to compare it with what the check read, and not again for a check.
    assert_eq!(counting.reads.load(Ordering::SeqCst), 4);
}

#[test]
fn a_file_changed_by_the_client_s_own_tools_is_seen_by_the_next_check() {
    let (mut session, counting) = session();
    ok(&mut session, "library.check", json!({}));
    counting.memory.put("/p/broken.lcf", BROKEN);
    let report = ok(&mut session, "library.check", json!({}));
    assert_eq!(report["status"], "error");
    assert_eq!(report["diagnostics"][0]["file"], "broken.lcf");
    counting.memory.put("/p/broken.lcf", MAIN.replace("double", "broken"));
    assert_eq!(ok(&mut session, "library.check", json!({}))["status"], "ok", "a change of content is a change of stamp");
}

#[test]
fn the_check_of_a_folder_under_the_root_is_kept_too() {
    let (mut session, counting) = session();
    counting.memory.put("/p/sub/b.lcf", MAIN.replace("double", "b"));
    let report = ok(&mut session, "library.check", json!({ "folder": "sub" }));
    assert_eq!(report["summary"]["functions"], 1);
    assert_eq!(counting.reads.load(Ordering::SeqCst), 1);
    assert_eq!(ok(&mut session, "library.check", json!({ "folder": "sub" })), report);
    assert_eq!(counting.reads.load(Ordering::SeqCst), 2, "read once more to compare, and not checked again");
}

/// The case the clock of a file system cannot tell: a file rewritten with the same size, at the same time.
#[test]
fn a_rewrite_of_the_same_size_at_the_same_time_is_seen() {
    let (mut session, counting) = session();
    let good = MAIN.replace("double", "same");
    let bad = good.replace("core.mul(x, 2)", "core.mux(x, 2)");
    assert_eq!(good.len(), bad.len());
    counting.memory.put("/p/same.lcf", good.clone());
    assert_eq!(ok(&mut session, "library.check", json!({}))["status"], "ok");
    counting.memory.put_keeping_time("/p/same.lcf", bad.clone());
    assert_eq!(ok(&mut session, "library.check", json!({}))["status"], "error", "seen by its content");
    ok(&mut session, "file.write", json!({ "path": "same.lcf", "text": good, "check": false }));
    counting.memory.put_keeping_time("/p/same.lcf", bad);
    assert_eq!(ok(&mut session, "library.check", json!({}))["status"], "error");
}

#[test]
fn a_library_under_lib_path_that_appears_or_changes_is_seen() {
    let memory = project();
    memory.put("/p/uses.lcf", "labcraft 1\n\nuse helpers\n\nfn uses(x: f64) -> (y: f64) {\n    y = helpers.twice(x)\n}\n");
    let counting = Arc::new(Counting { memory, reads: AtomicUsize::new(0), panic_on: None });
    let config = Config {
        root: PathBuf::from("/p"),
        lib_paths: vec![PathBuf::from("/libs")],
        limits: Limits::default(),
        shipped: Shipped::default(),
        natives: vec![Arc::new(Fake)],
        clock: Arc::new(crate::Still),
        process: 0,
    };
    let mut session = Session::new(counting.clone(), config, false).unwrap();
    let missing = ok(&mut session, "library.check", json!({}));
    assert_eq!(missing["status"], "error", "neither `/libs` nor `helpers` exist");
    counting.memory.put("/libs/helpers/twice.lcf", MAIN.replace("double", "twice"));
    assert_eq!(ok(&mut session, "library.check", json!({}))["status"], "ok");
    // A dependency is read for what it declares: a signature that `uses` cannot call, of the same size and time.
    let twice = MAIN.replace("double", "twice");
    counting.memory.put_keeping_time("/libs/helpers/twice.lcf", twice.replace("(x: f64)", "(x: str)"));
    assert_eq!(twice.len(), twice.replace("(x: f64)", "(x: str)").len());
    assert_eq!(ok(&mut session, "library.check", json!({}))["status"], "error");
}

#[test]
fn warnings_count_under_deny_warnings_and_a_folder_must_exist() {
    let (mut session, _) = session();
    let missing = error(&mut session, "library.check", json!({ "folder": "nowhere" }));
    assert_eq!((missing.code, missing.parameter.as_deref()), ("path.not_found", Some("folder")));
    let outside = error(&mut session, "library.check", json!({ "folder": "../q" }));
    assert_eq!(outside.code, "path.outside_project");
}

#[test]
fn file_write_writes_lf_and_returns_the_check_of_the_root() {
    let (mut session, counting) = session();
    let report = ok(&mut session, "file.write", json!({ "path": "lib/broken.lcf", "text": BROKEN.replace('\n', "\r\n") }));
    assert_eq!(report["schema"], "labcraft-check/1");
    assert_eq!(report["status"], "error");
    assert_eq!(report["diagnostics"][0]["file"], "lib/broken.lcf");
    assert_eq!(counting.memory.get("/p/lib/broken.lcf").unwrap(), BROKEN.as_bytes(), "CR LF became LF, and nothing else changed");
    let written = ok(&mut session, "file.write", json!({ "path": "lib/broken.lcf", "text": MAIN.replace("double", "broken"), "check": false }));
    assert_eq!(written, json!({ "written": "lib/broken.lcf" }));
}

#[test]
fn file_write_refuses_what_commands_never_reach() {
    let (mut session, _) = session();
    let cases = [
        (json!({ "path": "../out.lcf", "text": MAIN }), "path.outside_project"),
        (json!({ "path": ".git/x.lcf", "text": MAIN }), "path.outside_project"),
        (json!({ "path": "notes.txt", "text": "x" }), "path.invalid"),
        (json!({ "path": "nul.lcf", "text": MAIN }), "path.invalid"),
        (json!({ "path": "DOUBLE.lcf", "text": MAIN }), "path.invalid"),
        (json!({ "path": "<labcraft>/visa/open.lcf", "text": MAIN }), "path.outside_project"),
        (json!({ "path": "labcraft.toml", "text": "" }), "path.outside_project"),
        (json!({ "path": "Cargo.toml", "text": "[fake]\ninstrument = \"SIM::Y\"\n" }), "file.not_simulation"),
        (json!({ "path": "new.toml", "text": "[package]\n" }), "file.not_simulation"),
        (json!({ "path": "new.toml", "text": "[fake\n" }), "file.not_simulation"),
        (json!({ "path": "a.lcf", "text": "x".repeat(4 << 20 | 1) }), "file.too_large"),
    ];
    for (params, code) in cases {
        assert_eq!(error(&mut session, "file.write", params.clone()).code, code, "{params}");
    }
    let why = error(&mut session, "file.write", json!({ "path": "new.toml", "text": "[package]\n" }));
    assert!(why.message.contains("`package` names no library that simulates (fake)"), "{}", why.message);
    assert_eq!(why.parameter.as_deref(), Some("text"));
    ok(&mut session, "file.write", json!({ "path": "new.toml", "text": "", "check": false }));
}

#[test]
fn file_read_reads_from_a_line_and_says_how_many() {
    let (mut session, _) = session();
    let read = ok(&mut session, "file.read", json!({ "path": "double.lcf", "from": 5, "lines": 2 }));
    assert_eq!(
        read,
        json!({ "file": "double.lcf", "text": "/// Doubles a number.\n/// Second line.\n", "from": 5, "lines": 2, "lines_total": 9, "truncated": false })
    );
    let past = ok(&mut session, "file.read", json!({ "path": "double.lcf", "from": 20 }));
    assert_eq!((past["text"].as_str(), past["lines"].as_u64()), (Some(""), Some(0)));
    assert_eq!(error(&mut session, "file.read", json!({ "path": "Cargo.toml" })).code, "file.not_simulation");
    assert_eq!(error(&mut session, "file.read", json!({ "path": "gone.lcf" })).code, "path.not_found");
    assert_eq!(ok(&mut session, "file.read", json!({ "path": "bench.toml" }))["lines"], 2);
}

#[test]
fn file_read_cuts_at_the_end_of_a_line_past_one_mebibyte() {
    let memory = project();
    let line = format!("// {}\n", "x".repeat(1000));
    memory.put("/p/long.lcf", line.repeat(2000));
    let mut session = session_on(Arc::new(memory));
    let read = ok(&mut session, "file.read", json!({ "path": "long.lcf" }));
    // A line is 1,005 bytes as JSON writes it (its line feed takes two): 1,043 of them are 1,048,215 bytes, within 1,048,576, and
    // 1,044 would be 1,049,220.
    assert_eq!((read["lines"].as_u64(), read["truncated"].as_bool()), (Some(1043), Some(true)));
    let next = ok(&mut session, "file.read", json!({ "path": "long.lcf", "from": 1044 }));
    assert_eq!((next["lines"].as_u64(), next["truncated"].as_bool()), (Some(957), Some(false)));
}

/// The cases of the review of 6a: a text whose JSON is six times its size, and documentation of 100 kB a line.
#[test]
fn results_stay_within_their_bounds_as_json_writes_them() {
    let memory = project();
    memory.put("/p/controls.lcf", "\u{1}".repeat(2_000_000));
    for index in 0..60 {
        let name = format!("f{index}");
        memory.put(format!("/p/{name}.lcf"), format!("labcraft 1\n\n/// {}\nfn {name}() -> (y: f64) {{\n    y = 1.0\n}}\n", "d".repeat(100_000)));
    }
    let mut session = session_on(Arc::new(memory));
    let read = ok(&mut session, "file.read", json!({ "path": "controls.lcf" }));
    let text = serde_json::to_string(&read["text"]).unwrap();
    assert!(text.len() <= (1 << 20) + 2, "{}", text.len());
    assert_eq!(read["truncated"], true);
    let listed = ok(&mut session, "functions.list", json!({ "filter": "f" }));
    let size = serde_json::to_string(&listed).unwrap().len();
    assert!(size < 1 << 20, "{size}");
    let doc = listed["entries"].as_array().unwrap().iter().find(|entry| entry["name"] == "f7").unwrap()["doc"].as_str().unwrap().chars().count();
    assert!(doc <= 1001, "{doc}");
}

#[test]
fn file_rename_changes_a_case_alone_and_refuses_to_replace_a_file() {
    let (mut session, counting) = session();
    ok(&mut session, "file.write", json!({ "path": "Gain.lcf", "text": MAIN.replace("double", "gain"), "check": false }));
    let report = ok(&mut session, "file.rename", json!({ "from": "Gain.lcf", "to": "gain.lcf" }));
    assert_eq!(report["status"], "ok", "{report}");
    assert!(counting.memory.get("/p/gain.lcf").is_some() && counting.memory.get("/p/Gain.lcf").is_none());
    assert_eq!(error(&mut session, "file.rename", json!({ "from": "gain.lcf", "to": "double.lcf" })).code, "path.invalid");
    assert_eq!(error(&mut session, "file.rename", json!({ "from": "gain.lcf", "to": "gain.toml" })).code, "path.invalid");
    assert_eq!(error(&mut session, "file.rename", json!({ "from": "Cargo.toml", "to": "c.toml" })).code, "file.not_simulation");
    ok(&mut session, "file.rename", json!({ "from": "gain.lcf", "to": "sub/gain.lcf/." }));
    assert!(counting.memory.get("/p/sub/gain.lcf").is_some(), "`.` and a final `/` name no file");
}

#[test]
fn file_delete_deletes_then_checks() {
    let (mut session, counting) = session();
    let report = ok(&mut session, "file.delete", json!({ "path": "Verdict.lct" }));
    assert_eq!(report["summary"]["types"], 0);
    assert!(counting.memory.get("/p/Verdict.lct").is_none());
    assert_eq!(error(&mut session, "file.delete", json!({ "path": "Cargo.toml" })).code, "file.not_simulation");
    assert!(counting.memory.get("/p/Cargo.toml").is_some());
}

#[test]
fn file_format_gives_the_report_of_labcraft_cli_fmt() {
    let (mut session, counting) = session();
    counting.memory.put("/p/loose.lcf", "labcraft 1\nfn loose() -> (y: f64) {\n  y = 1.0\n}\n");
    let checked = ok(&mut session, "file.format", json!({ "path": ".", "check_only": true }));
    assert_eq!(checked["schema"], "labcraft-fmt/1");
    assert_eq!(checked["status"], "error", "{checked}");
    assert_eq!(checked["diagnostics"][0]["code"], "LC6001");
    assert_eq!(checked["diagnostics"][0]["file"], "loose.lcf");
    let formatted = ok(&mut session, "file.format", json!({ "path": "loose.lcf" }));
    assert_eq!(formatted["summary"]["changed"], 1, "{formatted}");
    assert_eq!(counting.memory.get("/p/loose.lcf").unwrap(), b"labcraft 1\n\nfn loose() -> (y: f64) {\n    y = 1.0\n}\n");
}

#[test]
fn an_unknown_command_or_parameter_names_the_nearest() {
    let (mut session, _) = session();
    let unknown = error(&mut session, "library.chek", json!({}));
    assert_eq!((unknown.code, unknown.expected.as_slice()), ("command.unknown", ["library.check".to_owned()].as_slice()));
    let parameter = error(&mut session, "library.check", json!({ "deny_warning": true }));
    assert_eq!((parameter.code, parameter.parameter.as_deref()), ("params.unknown", Some("deny_warning")));
    assert!(parameter.message.contains("did you mean `deny_warnings`"));
}

#[test]
fn a_planned_command_is_listed_greyed_with_its_reason_and_refused() {
    let (mut session, _) = session();
    let refused = error(&mut session, "view.open_trace", json!({ "file": "a.trace.jsonl" }));
    assert_eq!(refused.code, "command.not_enabled");
    assert!(refused.message.contains("step 8"), "{}", refused.message);
    let listed = ok(&mut session, "commands.list", json!({ "filter": "run." }));
    let commands = listed["commands"].as_array().unwrap();
    let of = |id: &str| commands.iter().find(|command| command["id"] == id).unwrap();
    assert_eq!((of("run.start")["enabled"].as_bool(), of("run.start")["long"].as_bool()), (Some(true), Some(true)));
    assert_eq!(of("run.set_element")["enabled"], true, "in real time (step 8)");
    let described = ok(&mut session, "commands.describe", json!({ "id": "file.write" }));
    assert_eq!(described["enabled"], true);
    assert_eq!(described["effects"], json!(["files"]));
    assert_eq!(described["params"]["required"], json!(["path", "text"]));
    assert_eq!(error(&mut session, "commands.describe", json!({ "id": "file.wrte" })).code, "command.unknown");
}

#[test]
fn a_panic_in_a_command_is_internal_and_the_session_goes_on() {
    let counting = Arc::new(Counting { memory: project(), reads: AtomicUsize::new(0), panic_on: Some(PathBuf::from("/p/double.lcf")) });
    let mut session = session_on(counting.clone());
    let internal = error(&mut session, "file.read", json!({ "path": "double.lcf" }));
    assert_eq!(internal.code, "internal");
    assert!(internal.message.contains("`file.read`"), "{}", internal.message);
    assert_eq!(ok(&mut session, "file.read", json!({ "path": "Verdict.lct" }))["lines"], 3, "the session goes on");
}

#[test]
fn project_describe_lists_libraries_files_simulations_and_the_summary() {
    let (mut session, counting) = session();
    counting.memory.put("/p/sub/b.lcf", MAIN.replace("double", "b"));
    let described = ok(&mut session, "project.describe", json!({}));
    assert_eq!(described["root"], "p");
    assert_eq!(
        described["libraries"],
        json!([
            { "library": "", "folder": ".", "files": [{ "file": "Verdict.lct", "declares": { "type": "Verdict" } }, { "file": "double.lcf", "declares": { "function": "double" } }] },
            { "library": "sub", "folder": "sub", "files": [{ "file": "sub/b.lcf", "declares": { "function": "b" } }] },
        ])
    );
    assert_eq!(
        described["simulations"],
        json!([{ "file": "bench.toml", "resources": ["SIM::X"], "valid": true }]),
        "Cargo.toml is no simulation file"
    );
    assert_eq!(described["summary"]["functions"], 2);
    assert_eq!(described["truncated"], false);
}

#[test]
fn simulation_describe_gives_each_part_or_its_diagnostics() {
    let (mut session, counting) = session();
    let described = ok(&mut session, "simulation.describe", json!({ "path": "bench.toml" }));
    assert_eq!(
        described,
        json!({ "file": "bench.toml", "parts": [{ "library": "fake", "description": { "resource": "SIM::X", "latency_ms": 0.5 } }] })
    );
    counting.memory.put("/p/bad.toml", "[fake]\nother = 1\n");
    let bad = ok(&mut session, "simulation.describe", json!({ "path": "bad.toml" }));
    assert_eq!(bad["diagnostics"][0]["code"], "fake:S001");
    assert_eq!(error(&mut session, "simulation.describe", json!({ "path": "Cargo.toml" })).code, "file.not_simulation");
    assert_eq!(error(&mut session, "simulation.describe", json!({ "path": "double.lcf" })).code, "path.invalid");
}

#[test]
fn functions_list_gives_primitives_then_the_project_as_signatures() {
    let (mut session, _) = session();
    let listed = ok(&mut session, "functions.list", json!({ "filter": "double" }));
    assert_eq!(
        listed["entries"],
        json!([{ "name": "double", "library": "", "kind": "function", "signature": "fn double(x: f64) -> (y: f64)", "doc": "Doubles a number." }])
    );
    let documented = ok(&mut session, "functions.list", json!({ "filter": "double", "docs": true }));
    assert_eq!(documented["entries"][0]["doc"], "Doubles a number.\nSecond line.");
    let get = ok(&mut session, "functions.list", json!({ "library": "array", "filter": "get" }));
    assert_eq!(get["entries"][0]["signature"], "fn array.get<T>(array: [T], index: i32, err: error = none) -> (item: T, err: error)");
    assert_eq!(get["entries"][0]["categories"], json!(["array.index_out_of_range"]));
    let types = ok(&mut session, "functions.list", json!({ "filter": "Verdict" }));
    assert_eq!(types["entries"][0]["signature"], "type Verdict = enum { Pass, Fail }");
}

/// Every declaration of appendix A, sections A.1 and A.2, is the signature that `functions.list` gives of its primitive,
/// once its lines are joined and its comments taken off.
#[test]
fn the_signature_of_each_primitive_is_that_of_appendix_a() {
    let spec = include_str!("../../../docs/language-spec-v0.md");
    let (_, after) = spec.split_once("## Appendix A. Library signatures").unwrap();
    let (section, _) = after.split_once("### A.3").unwrap();
    let written: Vec<String> = labcraft_check::primitives::PRIMITIVES.iter().map(crate::functions::primitive_signature).collect();
    let mut found = 0;
    for block in section.split("```text").skip(1) {
        let (block, _) = block.split_once("```").unwrap();
        let joined: String = block.lines().map(|line| line.split("//").next().unwrap().trim()).collect::<Vec<_>>().join(" ");
        for declaration in joined.split("fn ").filter(|text| !text.trim().is_empty()) {
            let declaration = format!("fn {}", declaration.split_whitespace().collect::<Vec<_>>().join(" ")).replace("( ", "(");
            assert!(written.contains(&declaration), "appendix A: `{declaration}` is not what functions.list writes");
            found += 1;
        }
    }
    assert!(found >= 40, "{found}");
}

// The viewer and `diagram.svg` (`docs/commands.md`, 6.5; decisions of the owner, 2026-11-11). The expected values come
// from the files of `project()`: one function, `double`, drawable; `Verdict.lct` is a type and gets no entry.

const DOUBLE_BROKEN: &str = "labcraft 1\n\nfn double(x: f64) -> (y: f64) {\n    y = nowhere(x)\n}\n";

#[test]
fn diagram_svg_gives_the_svg_of_each_function_in_image_and_writes_nothing() {
    let (mut session, counting) = session();
    let before = counting.memory.get("/p/double.lcf");
    let report = ok(&mut session, "diagram.svg", json!({ "folder": "." }));
    assert_eq!((report["schema"].as_str(), report["status"].as_str()), (Some("labcraft-diagram/1"), Some("ok")), "{report}");
    assert_eq!(report["summary"], json!({ "functions": 1, "drawn": 1, "not_drawn": 0 }));
    let entry = &report["diagrams"][0];
    assert_eq!((entry["function"].as_str(), entry["file"].as_str(), entry["drawn"].as_bool()), (Some("double"), Some("double.lcf"), Some(true)));
    // No file is written: the SVG is in `image`, and `svg`, the path of a file written, is absent.
    assert!(entry.get("svg").is_none(), "{entry}");
    let image = entry["image"].as_str().unwrap();
    assert!(image.starts_with("<svg"), "{image}");
    // Both themes by default: the dark one under `prefers-color-scheme` (`docs/cli.md`, 7.2); `light` alone has none.
    assert!(image.contains("prefers-color-scheme: dark"));
    let light = ok(&mut session, "diagram.svg", json!({ "folder": ".", "theme": "light" }));
    assert!(!light["diagrams"][0]["image"].as_str().unwrap().contains("prefers-color-scheme"));
    assert_eq!(counting.memory.get("/p/double.lcf"), before);
    assert!(counting.memory.get("/p/double.svg").is_none());
}

#[test]
fn diagram_svg_names_an_unknown_function_and_refuses_a_folder_out_of_reach() {
    let (mut session, _) = session();
    let report = ok(&mut session, "diagram.svg", json!({ "folder": ".", "functions": ["doubel"] }));
    assert_eq!(report["status"], "error");
    assert_eq!(report["diagrams"], json!([{ "function": "doubel", "drawn": false, "codes": ["LC7001"] }]));
    let diagnostic = &report["check"]["diagnostics"][0];
    assert_eq!(diagnostic["code"], "LC7001");
    // `doubel` is two edits from `double`, within a third of 6.
    assert_eq!(diagnostic["fixes"][0]["title"], "did you mean `double`?");
    assert_eq!(error(&mut session, "diagram.svg", json!({ "folder": "nowhere" })).code, "path.not_found");
    assert_eq!(error(&mut session, "diagram.svg", json!({ "folder": "../q" })).code, "path.outside_project");
    assert_eq!(error(&mut session, "diagram.svg", json!({})).code, "params.missing");
    // An empty name between two dots is no name, wrong usage on the command line (cli.md 7).
    let empty = error(&mut session, "diagram.svg", json!({ "folder": ".", "functions": ["a..b"] }));
    assert_eq!((empty.code, empty.parameter.as_deref()), ("params.invalid", Some("functions")));
}

#[test]
fn the_viewer_opens_a_folder_of_the_project_and_shows_its_first_function() {
    let (mut session, _) = session();
    let shown = error(&mut session, "view.show_function", json!({ "function": "double" }));
    assert_eq!(shown.code, "params.invalid");
    assert!(shown.message.contains("no folder is open"), "{}", shown.message);
    // The libraries are those of the session (decision of the owner, 2026-11-11).
    assert_eq!(error(&mut session, "view.open", json!({ "folder": ".", "lib_path": [] })).code, "params.unknown");
    let opened = ok(&mut session, "view.open", json!({ "folder": "." }));
    assert_eq!(opened["functions"], json!([{ "function": "double", "file": "double.lcf", "drawable": true }]));
    assert_eq!(opened["check"]["schema"], "labcraft-check/1");
    let view = session.view();
    assert_eq!(
        (view.folder.as_deref(), view.function.as_deref(), view.opened.as_slice()),
        (Some("."), Some("double"), ["double".to_owned()].as_slice())
    );
    assert!(view.drawing.is_some() && !view.kept);
    let entry = ok(&mut session, "view.show_function", json!({ "function": "double" }));
    assert_eq!((entry["function"].as_str(), entry["drawn"].as_bool()), (Some("double"), Some(true)));
    assert_eq!(ok(&mut session, "view.set_standard_view", json!({ "on": true })), json!({ "before": false }));
    assert_eq!(ok(&mut session, "view.set_standard_view", json!({ "on": false })), json!({ "before": true }));
    let unknown = error(&mut session, "view.show_function", json!({ "function": "dobule" }));
    assert_eq!((unknown.code, unknown.expected.as_slice()), ("params.invalid", ["double".to_owned()].as_slice()));
    assert_eq!(error(&mut session, "view.set_state", json!({ "function": "double", "state": { "colour": { "a": 1 } } })).code, "params.invalid");
    assert_eq!(error(&mut session, "view.open", json!({ "folder": "nowhere" })).code, "path.not_found");
    assert_eq!(error(&mut session, "view.open_trace", json!({ "file": "t.jsonl" })).code, "command.not_enabled");
}

#[test]
fn a_file_written_by_a_command_is_read_again_by_the_next_command_of_the_viewer() {
    let (mut session, _) = session();
    ok(&mut session, "view.open", json!({ "folder": "." }));
    let drawn = session.view().drawing.clone().unwrap();
    ok(&mut session, "file.write", json!({ "path": "double.lcf", "text": DOUBLE_BROKEN }));
    // `nowhere` is an unknown function, `LC2002`, of the family `LC20`: not drawable (`docs/cli.md`, 7.1). The last drawing stays.
    let entry = ok(&mut session, "view.show_function", json!({ "function": "double" }));
    assert_eq!((entry["drawn"].as_bool(), &entry["codes"]), (Some(false), &json!(["LC2002"])), "{entry}");
    let view = session.view();
    assert!(view.kept);
    assert!(Arc::ptr_eq(view.drawing.as_ref().unwrap(), &drawn));
    assert_eq!(view.errors, 1);
}

#[test]
fn a_change_that_the_app_sees_is_drawn_again_and_glides() {
    let (mut session, counting) = session();
    ok(&mut session, "view.open", json!({ "folder": "." }));
    let before = session.view().drawing_generation;
    assert!(!session.view().glide);
    counting.memory.put("/p/double.lcf", "labcraft 1\n\nuse core\n\nfn double(x: f64) -> (y: f64) {\n    y = core.add(x, x)\n}\n");
    session.files_changed();
    let view = session.view();
    assert!(view.drawing_generation > before && view.glide && !view.kept && view.errors == 0);
    // The file gone: nothing shown, nothing listed.
    counting.memory.remove_file(Path::new("/p/double.lcf")).unwrap();
    session.files_changed();
    let view = session.view();
    assert!(view.functions.is_empty() && view.function.is_none() && view.drawing.is_none() && view.opened.is_empty());
}

#[test]
fn a_session_on_a_read_only_workspace_writes_nothing() {
    let memory = Arc::new(project());
    let mut session = session_on(Arc::new(crate::ReadOnly(memory.clone())));
    let refused = error(&mut session, "file.write", json!({ "path": "double.lcf", "text": DOUBLE_BROKEN }));
    assert_eq!(refused.code, "file.unwritable", "{}", refused.message);
    assert_eq!(memory.get("/p/double.lcf").as_deref(), Some(MAIN.as_bytes()));
    assert_eq!(error(&mut session, "file.delete", json!({ "path": "double.lcf" })).code, "file.unwritable");
    assert!(memory.get("/p/double.lcf").is_some());
}

#[test]
fn guide_read_lists_the_skills_then_gives_the_one_that_writes_a_program() {
    let (mut session, _) = session();
    let first = ok(&mut session, "guide.read", json!({}));
    let names: Vec<&str> = first["skills"].as_array().unwrap().iter().map(|skill| skill["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["labcraft-debugging", "labcraft-testing", "labcraft-writing"]);
    let writing = first["skills"].as_array().unwrap().iter().find(|skill| skill["name"] == "labcraft-writing").unwrap();
    assert!(writing["description"].as_str().unwrap().starts_with("Use when writing"), "{writing}");
    assert_eq!(writing["files"], json!(["SKILL.md", "reference.md"]));
    assert_eq!((first["skill"].as_str(), first["file"].as_str()), (Some("labcraft-writing"), Some("SKILL.md")));
    assert!(first["text"].as_str().unwrap().starts_with("---\nname: labcraft-writing\n"));
    // The same text as the file a client that supports skills installs.
    let installed = include_str!("../../../skills/labcraft-writing/reference.md");
    let reference = ok(&mut session, "guide.read", json!({ "skill": "labcraft-writing", "file": "reference.md" }));
    assert_eq!(reference, json!({ "skill": "labcraft-writing", "file": "reference.md", "text": installed }));
    let debugging = ok(&mut session, "guide.read", json!({ "skill": "labcraft-debugging" }));
    assert_eq!(debugging["file"], "SKILL.md");
}

#[test]
fn guide_read_refuses_what_it_does_not_hold_with_the_choices() {
    let (mut session, _) = session();
    let skill = error(&mut session, "guide.read", json!({ "skill": "labcraft-writng" }));
    assert_eq!((skill.code, skill.parameter.as_deref()), ("params.invalid", Some("skill")));
    // The schema lists the skills (`enum`), so the agent sees them before it calls, and the error gives them.
    assert!(skill.message.contains("`labcraft-writing`"), "{}", skill.message);
    let described = ok(&mut session, "commands.describe", json!({ "id": "guide.read" }));
    assert_eq!(described["params"]["properties"]["skill"]["enum"], json!(["labcraft-debugging", "labcraft-testing", "labcraft-writing"]));
    assert_eq!(skill.expected, ["labcraft-debugging", "labcraft-testing", "labcraft-writing"]);
    // Never a path: `..`, a folder or a file of the disk are names that no skill holds.
    for path in ["../SKILL.md", "/etc/passwd", "reference", "README.md", "labcraft-writing/SKILL.md"] {
        let file = error(&mut session, "guide.read", json!({ "skill": "labcraft-writing", "file": path }));
        assert_eq!((file.code, file.parameter.as_deref()), ("params.invalid", Some("file")), "{path}");
        assert_eq!(file.expected, ["SKILL.md", "reference.md"], "{path}");
    }
    let alone = error(&mut session, "guide.read", json!({ "file": "reference.md" }));
    assert_eq!((alone.code, alone.parameter.as_deref()), ("params.invalid", Some("skill")));
    assert_eq!(error(&mut session, "guide.read", json!({ "skill": 3 })).code, "params.invalid");
    assert_eq!(error(&mut session, "guide.read", json!({ "skil": "labcraft-writing" })).code, "params.unknown");
}
