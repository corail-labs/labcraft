//! The editing commands through the session (commands.md 9.1 to 9.5): steps, revisions, batches, undo for the user and
//! for an agent. The texts expected are written by hand; a revision is worked out with `sha2`, not with `revision`.

use std::path::PathBuf;
use std::sync::Arc;

use labcraft_types::Limits;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::history::Author;
use crate::{CommandError, Config, Memory, Session, Shipped, Still, Workspace};

const START: &str = "labcraft 1\n\nfn f(x: f64) -> (y: f64) {\n    y = x\n}\n";
/// Not canonical: undo must give these bytes back.
const LOOSE: &str = "labcraft 1\n\nfn g(x: f64) -> (y: f64) {\n  y   =   x\n}\n";

fn session() -> (Session, Arc<Memory>) {
    let memory = Arc::new(Memory::new());
    memory.put("/p/f.lcf", START);
    memory.put("/p/g.lcf", LOOSE);
    memory.put("/p/notes.toml", "");
    let config = Config {
        root: PathBuf::from("/p"),
        lib_paths: Vec::new(),
        limits: Limits::default(),
        shipped: Shipped::default(),
        natives: Vec::new(),
        clock: Arc::new(Still),
        process: 0,
    };
    let ws: Arc<dyn Workspace> = memory.clone();
    (Session::new(ws, config, false).unwrap_or_else(|error| panic!("{error:?}")), memory)
}

#[track_caller]
fn ok(session: &mut Session, id: &str, params: Value) -> Value {
    session.execute(id, &params).unwrap_or_else(|error| panic!("{id}: {}", error.to_json()))
}

#[track_caller]
fn refused(session: &mut Session, author: &Author, id: &str, params: Value) -> CommandError {
    match session.execute_as(author, id, &params) {
        Ok(value) => panic!("{id}: an error expected, got {value}"),
        Err(error) => error,
    }
}

fn text(memory: &Memory, path: &str) -> String {
    String::from_utf8(memory.get(path).unwrap_or_default()).unwrap_or_default()
}

fn sha(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    format!("sha256:{}", digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>())
}

const ADDED: &str = "labcraft 1\n\nuse core\n\nfn f(x: f64) -> (y: f64) {\n    sum = core.add()\n    y = x\n}\n";

#[test]
fn an_edit_writes_canonical_text_and_returns_its_step_and_revision() {
    let (mut session, memory) = session();
    let result = ok(&mut session, "node.add", json!({ "file": "f.lcf", "call": "core.add" }));
    assert_eq!(text(&memory, "/p/f.lcf"), ADDED);
    assert_eq!(result["step"], 1);
    assert_eq!(result["files"], json!([{ "file": "f.lcf", "revision": sha(ADDED) }]));
    assert_eq!(result["names"], json!({ "node": "sum" }));
    assert_eq!(result["renamed"], json!([]));
    // `sum` misses its two inputs, and `f.lcf` was written: its diagnostics are given.
    let codes: Vec<&str> = result["diagnostics"].as_array().into_iter().flatten().filter_map(|diagnostic| diagnostic["code"].as_str()).collect();
    assert_eq!(codes, ["LC2101"], "{result}");
    assert_eq!(result["summary"]["errors"], 1);
}

#[test]
fn undo_gives_back_every_byte_and_redo_replays() {
    let (mut session, memory) = session();
    ok(&mut session, "node.add", json!({ "file": "g.lcf", "call": "core.add" }));
    ok(&mut session, "constant.add", json!({ "file": "g.lcf", "value": "240 V" }));
    let after = text(&memory, "/p/g.lcf");
    let undone = ok(&mut session, "history.undo", json!({}));
    assert_eq!((undone["step"].clone(), undone["author"].clone(), undone["label"].clone()), (json!(2), json!("user"), json!("Add Constant")));
    ok(&mut session, "history.undo", json!({}));
    assert_eq!(text(&memory, "/p/g.lcf"), LOOSE, "the text that was not canonical comes back byte for byte");
    assert_eq!(session.execute("history.undo", &json!({})).map_err(|error| error.code), Err("history.empty"));
    ok(&mut session, "history.redo", json!({}));
    ok(&mut session, "history.redo", json!({}));
    assert_eq!(text(&memory, "/p/g.lcf"), after);
    assert_eq!(session.execute("history.redo", &json!({})).map_err(|error| error.code), Err("history.empty"));
}

#[test]
fn a_stale_revision_changes_nothing() {
    let (mut session, memory) = session();
    let error = refused(
        &mut session,
        &Author::User,
        "node.add",
        json!({ "file": "f.lcf", "call": "core.add", "expected_revision": { "f.lcf": sha("old") } }),
    );
    assert_eq!(error.code, "file.stale");
    assert!(error.message.contains(&sha(START)), "the current revision: {}", error.message);
    assert_eq!(text(&memory, "/p/f.lcf"), START);
    ok(&mut session, "node.add", json!({ "file": "f.lcf", "call": "core.add", "expected_revision": { "f.lcf": sha(START) } }));
    assert_eq!(text(&memory, "/p/f.lcf"), ADDED);
}

/// Scenario 7: a write from outside is a step like any other.
#[test]
fn an_external_write_is_a_step_that_undo_goes_back_over() {
    let (mut session, memory) = session();
    ok(&mut session, "node.add", json!({ "file": "f.lcf", "call": "core.add" }));
    let outside = "labcraft 1\n\nfn f() -> () {}\n";
    memory.put("/p/f.lcf", outside);
    // An agent that read `f.lcf` before the write is refused, and told the revision now.
    let agent = Author::Agent("claude".to_owned());
    let stale = json!({ "file": "f.lcf", "value": "1", "expected_revision": { "f.lcf": sha(ADDED) } });
    let error = refused(&mut session, &agent, "constant.add", stale);
    assert_eq!(error.code, "file.stale");
    assert!(error.message.contains(&sha(outside)), "{}", error.message);
    let listed = ok(&mut session, "history.list", json!({}));
    assert_eq!((listed["steps"][0]["step"].clone(), listed["steps"][0]["author"].clone()), (json!(2), json!("external")), "{listed}");
    assert_eq!(listed["steps"][0]["files"], json!(["f.lcf"]));
    // An agent never undoes it: it is not its own.
    assert_eq!(refused(&mut session, &agent, "history.undo", json!({})).code, "history.not_yours");
    let undone = ok(&mut session, "history.undo", json!({}));
    assert_eq!((undone["step"].clone(), undone["author"].clone()), (json!(2), json!("external")));
    assert_eq!(text(&memory, "/p/f.lcf"), ADDED, "undo gives back the bytes before the external write");
    ok(&mut session, "history.redo", json!({}));
    assert_eq!(text(&memory, "/p/f.lcf"), outside, "redo gives back those after it");
    ok(&mut session, "history.undo", json!({}));
    ok(&mut session, "history.undo", json!({}));
    assert_eq!(text(&memory, "/p/f.lcf"), START);
}

#[test]
fn external_writes_of_a_burst_are_one_step_and_a_removal_is_one() {
    let (mut session, memory) = session();
    ok(&mut session, "node.add", json!({ "file": "f.lcf", "call": "core.add" }));
    ok(&mut session, "node.add", json!({ "file": "g.lcf", "call": "core.add" }));
    let g = text(&memory, "/p/g.lcf");
    // The clock of the tests never moves: every write is less than 1 s after the one before.
    memory.put("/p/f.lcf", "one");
    session.files_changed();
    memory.put("/p/f.lcf", "two");
    memory.remove_file(std::path::Path::new("/p/g.lcf")).unwrap_or_else(|error| panic!("{error}"));
    session.files_changed();
    let listed = ok(&mut session, "history.list", json!({}));
    assert_eq!(listed["steps"].as_array().map(Vec::len), Some(3), "{listed}");
    assert_eq!((listed["steps"][0]["author"].clone(), listed["steps"][0]["files"].clone()), (json!("external"), json!(["f.lcf", "g.lcf"])));
    ok(&mut session, "history.undo", json!({}));
    assert_eq!((text(&memory, "/p/f.lcf"), text(&memory, "/p/g.lcf")), (ADDED.to_owned(), g));
    // Written back to the bytes of the session: no step.
    memory.put("/p/f.lcf", "three");
    memory.put("/p/f.lcf", ADDED);
    let listed = ok(&mut session, "history.list", json!({}));
    assert_eq!((listed["undo"].clone(), listed["redo"].clone()), (json!(2), json!(3)), "{listed}");
}

#[test]
fn an_external_write_to_a_file_the_session_never_wrote_is_a_step() {
    let (mut session, memory) = session();
    // The first command reads the files of the project, to follow them.
    ok(&mut session, "history.list", json!({}));
    memory.put("/p/g.lcf", "outside");
    memory.put("/p/h.lcf", "labcraft 1\n");
    // Not a file of the project that is followed: a folder whose name starts with `.`, and a file that is no program.
    memory.put("/p/.labcraft/x.lcf", "labcraft 1\n");
    memory.put("/p/notes.txt", "x");
    let listed = ok(&mut session, "history.list", json!({}));
    assert_eq!(listed["steps"].as_array().map(Vec::len), Some(1), "{listed}");
    assert_eq!((listed["steps"][0]["author"].clone(), listed["steps"][0]["files"].clone()), (json!("external"), json!(["g.lcf", "h.lcf"])));
    ok(&mut session, "history.undo", json!({}));
    assert_eq!((text(&memory, "/p/g.lcf"), memory.get("/p/h.lcf")), (LOOSE.to_owned(), None), "a file that appeared goes with undo");
}

#[test]
fn external_writes_more_than_1_s_apart_by_the_time_of_their_files_are_two_steps() {
    let (mut session, memory) = session();
    ok(&mut session, "history.list", json!({}));
    let second: u128 = 1_000_000_000;
    memory.put_modified("/p/f.lcf", "a", 5 * second);
    session.files_changed();
    // Seen together with the next one, 5 s later by the time of its file: a step of its own.
    memory.put_modified("/p/g.lcf", "b", 10 * second);
    memory.put_modified("/p/f.lcf", "c", 10 * second + second / 2);
    let listed = ok(&mut session, "history.list", json!({}));
    let files: Vec<Value> = listed["steps"].as_array().into_iter().flatten().map(|step| step["files"].clone()).collect();
    assert_eq!(files, [json!(["f.lcf", "g.lcf"]), json!(["f.lcf"])], "{listed}");
    // The time of a step is that of the session, whatever the times of its files.
    assert_eq!(listed["steps"][0]["time"], "1970-01-01T00:00:00.000000000Z");
}

#[test]
fn past_64_mib_kept_a_file_is_followed_with_no_step() {
    let memory = Arc::new(Memory::new());
    // 33 MiB each: the first fits in the 64 MiB, the second does not.
    let big = 33 << 20;
    memory.put("/p/a.lcf", vec![b'a'; big]);
    memory.put("/p/b.lcf", vec![b'b'; big]);
    let mut limits = Limits::default();
    limits.max_file_size = 100 << 20;
    let config = Config {
        root: PathBuf::from("/p"),
        lib_paths: Vec::new(),
        limits,
        shipped: Shipped::default(),
        natives: Vec::new(),
        clock: Arc::new(Still),
        process: 0,
    };
    let ws: Arc<dyn Workspace> = memory.clone();
    let mut session = Session::new(ws, config, false).unwrap_or_else(|error| panic!("{error:?}"));
    ok(&mut session, "history.list", json!({}));
    memory.put("/p/b.lcf", "outside");
    assert_eq!(ok(&mut session, "history.list", json!({}))["steps"], json!([]), "`b.lcf` is not kept: no step");
    memory.put("/p/a.lcf", "outside");
    let listed = ok(&mut session, "history.list", json!({}));
    assert_eq!(listed["steps"][0]["files"], json!(["a.lcf"]), "{listed}");
}

#[test]
fn a_write_of_the_same_bytes_makes_no_step_and_keeps_redo() {
    let (mut session, _) = session();
    ok(&mut session, "node.add", json!({ "file": "f.lcf", "call": "core.add" }));
    ok(&mut session, "history.undo", json!({}));
    ok(&mut session, "file.write", json!({ "path": "f.lcf", "text": START }));
    let listed = ok(&mut session, "history.list", json!({}));
    assert_eq!((listed["undo"].clone(), listed["redo"].clone()), (Value::Null, json!(1)), "{listed}");
}

#[test]
fn a_file_over_the_limit_is_still_replaced_or_deleted_with_no_step() {
    let memory = Arc::new(Memory::new());
    memory.put("/p/big.lcf", "x".repeat(200));
    memory.put("/p/other.lcf", "y".repeat(200));
    let config = Config {
        root: PathBuf::from("/p"),
        lib_paths: Vec::new(),
        limits: {
            let mut limits = Limits::default();
            limits.max_file_size = 100;
            limits
        },
        shipped: Shipped::default(),
        natives: Vec::new(),
        clock: Arc::new(Still),
        process: 0,
    };
    let ws: Arc<dyn Workspace> = memory.clone();
    let mut session = Session::new(ws, config, false).unwrap_or_else(|error| panic!("{error:?}"));
    ok(&mut session, "file.write", json!({ "path": "big.lcf", "text": "labcraft 1\n", "check": false }));
    ok(&mut session, "file.delete", json!({ "path": "other.lcf" }));
    assert_eq!((text(&memory, "/p/big.lcf"), memory.get("/p/other.lcf")), ("labcraft 1\n".to_owned(), None));
    assert_eq!(ok(&mut session, "history.list", json!({}))["steps"], json!([]), "the history cannot keep what was there");
}

#[test]
fn the_commands_that_write_files_are_steps_of_their_author() {
    let (mut session, memory) = session();
    let agent = Author::Agent("claude".to_owned());
    let mut run = |id: &str, params: Value| session.execute_as(&agent, id, &params).unwrap_or_else(|error| panic!("{id}: {}", error.to_json()));
    run("file.write", json!({ "path": "f.lcf", "text": LOOSE.replace('g', "f") }));
    run("file.format", json!({ "path": "f.lcf" }));
    run("file.rename", json!({ "from": "f.lcf", "to": "lib/f.lcf" }));
    run("file.delete", json!({ "path": "g.lcf" }));
    run("file.write", json!({ "path": "new.lcf", "text": "labcraft 1\n" }));
    let listed = run("history.list", json!({}));
    let steps: Vec<(Value, Value)> =
        listed["steps"].as_array().into_iter().flatten().map(|step| (step["author"].clone(), step["files"].clone())).collect();
    let claude = json!("claude");
    assert_eq!(
        steps,
        [
            (claude.clone(), json!(["new.lcf"])),
            (claude.clone(), json!(["g.lcf"])),
            (claude.clone(), json!(["f.lcf", "lib/f.lcf"])),
            (claude.clone(), json!(["f.lcf"])),
            (claude, json!(["f.lcf"])),
        ],
        "no step signed `external`: the session wrote each file"
    );
    for _ in 0..5 {
        run("history.undo", json!({}));
    }
    assert_eq!((text(&memory, "/p/f.lcf"), text(&memory, "/p/g.lcf")), (START.to_owned(), LOOSE.to_owned()));
    assert_eq!((memory.get("/p/lib/f.lcf"), memory.get("/p/new.lcf")), (None, None));
}

#[test]
fn a_batch_is_one_step_and_a_failure_writes_nothing() {
    let (mut session, memory) = session();
    let batch = json!({ "commands": [
        { "id": "node.add", "params": { "file": "f.lcf", "call": "core.add" } },
        { "id": "wire.connect", "params": { "file": "f.lcf", "from": "x", "to": "sum.a" } },
    ] });
    let result = ok(&mut session, "commands.batch", batch);
    let wired = "labcraft 1\n\nuse core\n\nfn f(x: f64) -> (y: f64) {\n    sum = core.add(x)\n    y = x\n}\n";
    assert_eq!(text(&memory, "/p/f.lcf"), wired);
    assert_eq!(result["step"], 1);
    assert_eq!(result["commands"], json!([{ "names": { "node": "sum" }, "renamed": [] }, { "names": {}, "renamed": [] }]));
    ok(&mut session, "history.undo", json!({}));
    assert_eq!(text(&memory, "/p/f.lcf"), START, "one undo takes all of it back");

    let failing = json!({ "commands": [
        { "id": "node.add", "params": { "file": "f.lcf", "call": "core.add" } },
        { "id": "wire.connect", "params": { "file": "f.lcf", "from": "nothing", "to": "sum.a" } },
    ] });
    let error = refused(&mut session, &Author::User, "commands.batch", failing);
    assert_eq!((error.code, error.index), ("edit.unknown", Some(1)));
    assert_eq!(error.to_json()["error"]["index"], 1);
    assert_eq!(text(&memory, "/p/f.lcf"), START);
    for (item, code) in [
        (json!({ "id": "history.undo", "params": {} }), "params.invalid"),
        (json!({ "id": "commands.batch", "params": { "commands": [] } }), "params.invalid"),
        (json!({ "id": "node.add", "params": { "file": "f.lcf", "call": "core.add", "expected_revision": {} } }), "params.unknown"),
    ] {
        let error = refused(&mut session, &Author::User, "commands.batch", json!({ "commands": [item] }));
        assert_eq!((error.code, error.index), (code, Some(0)));
    }
}

/// `$i` names the node that an earlier command of the batch named (commands.md 6.7; decision of the owner, 2026-11-17):
/// quick insertion from a wire, one step, whatever name the node gets.
#[test]
fn a_command_of_a_batch_names_the_node_of_an_earlier_one() {
    let (mut session, memory) = session();
    ok(&mut session, "node.add", json!({ "file": "f.lcf", "call": "core.add" }));
    let batch = json!({ "commands": [
        { "id": "node.add", "params": { "file": "f.lcf", "call": "core.add" } },
        { "id": "wire.connect", "params": { "file": "f.lcf", "from": "x", "to": "$0.a" } },
        { "id": "wire.connect", "params": { "file": "f.lcf", "from": "$0", "to": "sum.b" } },
    ] });
    ok(&mut session, "commands.batch", batch);
    let wired = "labcraft 1\n\nuse core\n\nfn f(x: f64) -> (y: f64) {\n    sum2 = core.add(x)\n    sum = core.add(b: sum2)\n    y = x\n}\n";
    assert_eq!(text(&memory, "/p/f.lcf"), wired);
    for (to, index) in [("$1.a", 0), ("$5.a", 1)] {
        let batch = json!({ "commands": [
            { "id": "node.add", "params": { "file": "f.lcf", "call": "core.abs" } },
            { "id": "wire.connect", "params": { "file": "f.lcf", "from": "x", "to": to } },
        ] });
        let failing = if index == 0 { json!({ "commands": [batch["commands"][1].clone(), batch["commands"][0].clone()] }) } else { batch };
        let error = refused(&mut session, &Author::User, "commands.batch", failing);
        assert_eq!((error.code, error.index), ("params.invalid", Some(index)), "{to}");
    }
    // A command that named no node.
    let batch = json!({ "commands": [
        { "id": "wire.connect", "params": { "file": "f.lcf", "from": "x", "to": "sum2.b" } },
        { "id": "wire.connect", "params": { "file": "f.lcf", "from": "x", "to": "$0.a" } },
    ] });
    assert_eq!(refused(&mut session, &Author::User, "commands.batch", batch).code, "params.invalid");
    // A `$i` is a whole name, once in each end: nothing else is replaced, so that a value never grows past two names.
    for to in ["sum2.b $0", "$0 $0", "$0abc.a", "$0_x", "$00.a"] {
        let batch = json!({ "commands": [
            { "id": "node.add", "params": { "file": "f.lcf", "call": "core.abs" } },
            { "id": "wire.connect", "params": { "file": "f.lcf", "from": "x", "to": to } },
        ] });
        assert_eq!(refused(&mut session, &Author::User, "commands.batch", batch).code, "params.invalid", "{to}");
    }
    assert_eq!(text(&memory, "/p/f.lcf"), wired);
}

/// A node named by `$0` and folded into an output (notation 2.4) keeps its `$0` under its new name, though a later node
/// of the batch takes the name it had: worked out by hand from N-14 and spec 5.2.
#[test]
fn a_node_renamed_in_a_batch_keeps_its_earlier_name() {
    let (mut session, memory) = session();
    let batch = json!({ "commands": [
        { "id": "node.add", "params": { "file": "f.lcf", "call": "core.abs" } },
        { "id": "wire.connect", "params": { "file": "f.lcf", "from": "$0", "to": "output y", "replace": true } },
        { "id": "node.add", "params": { "file": "f.lcf", "call": "core.abs" } },
        { "id": "wire.connect", "params": { "file": "f.lcf", "from": "x", "to": "$0" } },
    ] });
    ok(&mut session, "commands.batch", batch);
    assert_eq!(text(&memory, "/p/f.lcf"), "labcraft 1\n\nuse core\n\nfn f(x: f64) -> (y: f64) {\n    abs = core.abs()\n    y = core.abs(x)\n}\n");
}

/// `node.add` with `into`, through the session and the shipped `core` (commands.md 6.7).
#[test]
fn a_node_goes_into_a_wire() {
    let (mut session, memory) = session();
    let result = ok(&mut session, "node.add", json!({ "file": "f.lcf", "call": "core.abs", "into": "x -> y" }));
    assert_eq!((result["names"]["node"].as_str(), result["renamed"][0]["to"].as_str()), (Some("y"), Some("y")), "{result}");
    assert_eq!(text(&memory, "/p/f.lcf"), "labcraft 1\n\nuse core\n\nfn f(x: f64) -> (y: f64) {\n    y = core.abs(x)\n}\n");
    // `core.not` takes no number: nothing is written.
    assert_eq!(
        refused(&mut session, &Author::User, "node.add", json!({ "file": "f.lcf", "call": "core.not", "into": "x -> y" })).code,
        "params.invalid"
    );
}

#[test]
fn an_agent_undoes_only_its_own_steps() {
    let (mut session, memory) = session();
    let agent = Author::Agent("claude".to_owned());
    ok(&mut session, "node.add", json!({ "file": "f.lcf", "call": "core.add" }));
    let error = refused(&mut session, &agent, "history.undo", json!({}));
    assert_eq!(error.code, "history.not_yours");
    assert!(error.message.contains("by user"), "{}", error.message);
    session.execute_as(&agent, "constant.add", &json!({ "file": "f.lcf", "value": "1" })).unwrap_or_else(|error| panic!("{error:?}"));
    session.execute_as(&agent, "history.undo", &json!({})).unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(text(&memory, "/p/f.lcf"), ADDED);
    // The user undoes its own step; the agent cannot redo it.
    ok(&mut session, "history.undo", json!({}));
    assert_eq!(refused(&mut session, &agent, "history.redo", json!({})).code, "history.not_yours");
    let listed = ok(&mut session, "history.list", json!({}));
    // Both steps are undone, the newest first; redo would replay the user's.
    assert_eq!(listed["steps"].as_array().map(Vec::len), Some(2), "{listed}");
    assert_eq!(listed["undo"], Value::Null);
    assert_eq!(listed["redo"], 1);
    assert_eq!((listed["steps"][0]["step"].clone(), listed["steps"][0]["author"].clone()), (json!(2), json!("claude")));
    assert_eq!((listed["steps"][1]["author"].clone(), listed["steps"][1]["files"].clone()), (json!("user"), json!(["f.lcf"])));
    assert_eq!(listed["steps"][1]["time"], "1970-01-01T00:00:00.000000000Z");
}

#[test]
fn commands_execute_runs_a_command_as_its_own_tool() {
    let (mut session, memory) = session();
    let result = ok(&mut session, "commands.execute", json!({ "id": "node.add", "params": { "file": "f.lcf", "call": "core.add" } }));
    assert_eq!(result["names"]["node"], "sum");
    assert_eq!(text(&memory, "/p/f.lcf"), ADDED);
    assert_eq!(refused(&mut session, &Author::User, "commands.execute", json!({ "id": "commands.batch" })).code, "params.invalid");
    assert_eq!(refused(&mut session, &Author::User, "commands.execute", json!({ "id": "nothing" })).code, "command.unknown");
}

#[test]
fn on_the_command_line_there_is_no_step_and_no_undo() {
    let (mut session, _) = session();
    session.for_command_line();
    let result = ok(&mut session, "node.add", json!({ "file": "f.lcf", "call": "core.add" }));
    assert_eq!(result["step"], Value::Null);
    assert_eq!(session.execute("history.undo", &json!({})).map_err(|error| error.code), Err("command.not_enabled"));
}

#[test]
fn an_editing_command_takes_a_function_file_of_the_project() {
    let (mut session, _) = session();
    let user = Author::User;
    assert_eq!(refused(&mut session, &user, "node.add", json!({ "file": "notes.toml", "call": "core.add" })).code, "path.invalid");
    assert_eq!(refused(&mut session, &user, "node.add", json!({ "file": "none.lcf", "call": "core.add" })).code, "path.not_found");
    assert_eq!(refused(&mut session, &user, "node.add", json!({ "file": "f.lcf", "call": "core.add", "into": "x -> z" })).code, "edit.unknown");
    assert_eq!(refused(&mut session, &user, "node.add", json!({ "file": "f.lcf", "call": "core.add", "at": [1] })).code, "params.invalid");
    assert_eq!(refused(&mut session, &user, "node.add", json!({ "file": "f.lcf", "call": "visa.nothing" })).code, "edit.unknown");
}

/// Two nodes named by their lines in one batch: each gets its label before the commands run, so that the label the first
/// one gets, which canonical form may move (spec 5.2), never makes the second line name another node (decision of the
/// owner, 2026-11-17, after the independent review).
#[test]
fn a_batch_names_by_their_labels_the_nodes_it_names_by_their_lines() {
    let (mut session, memory) = session();
    let unnamed = "labcraft 1\n\nuse core\n\nfn f(x: f64) -> (y: f64) {\n    _ = core.add(x, 1.0)\n    _ = core.mul(x, 2.0)\n    y = x\n}\n";
    memory.put("/p/f.lcf", unnamed);
    let commands = json!([
        { "id": "node.place", "params": { "file": "f.lcf", "node": "@6", "at": [40, 40] } },
        { "id": "node.place", "params": { "file": "f.lcf", "node": "@7", "at": [200, 40] } },
    ]);
    let result = ok(&mut session, "commands.batch", json!({ "commands": commands }));
    let after = text(&memory, "/p/f.lcf");
    assert!(after.contains("#add: _ = core.add(x, 1.0)") && after.contains("#mul: _ = core.mul(x, 2.0)"), "{after}");
    assert!(after.contains("    node #add at (40, 40)\n") && after.contains("    node #mul at (200, 40)\n"), "{after}");
    assert_eq!(result["commands"][0]["renamed"], json!([{ "file": "f.lcf", "from": "@6", "to": "#add" }]));
    assert_eq!(result["commands"][1]["renamed"], json!([{ "file": "f.lcf", "from": "@7", "to": "#mul" }]));
    assert_eq!(result["step"], 1);
}

/// `wire.route` writes the `wire … via` entry of a wire the drawing names, as one step; a wire that is not drawn is
/// refused (commands.md 6.7).
#[test]
fn a_wire_is_routed_through_its_points() {
    let (mut session, memory) = session();
    let before = "labcraft 1\n\nuse core\n\nfn h(x: f64) -> (y: f64) {\n    a = core.neg(x)\n    y = core.abs(a)\n}\n";
    memory.put("/p/h.lcf", before);
    let result = ok(&mut session, "wire.route", json!({ "file": "h.lcf", "wire": "a -> y.x", "via": [[120, 16], [160, 16]] }));
    let after = format!("{before}\nlayout {{\n    wire a -> y via (120, 16) (160, 16)\n}}\n");
    assert_eq!(text(&memory, "/p/h.lcf"), after);
    assert_eq!(result["names"], json!({ "wire": "a -> y.x" }));
    ok(&mut session, "history.undo", json!({}));
    assert_eq!(text(&memory, "/p/h.lcf"), before);
    let author = Author::User;
    let unknown = refused(&mut session, &author, "wire.route", json!({ "file": "h.lcf", "wire": "x -> y", "via": [[1, 1]] }));
    assert_eq!((unknown.code, unknown.parameter.as_deref()), ("edit.unknown", Some("wire")));
    let empty = refused(&mut session, &author, "wire.route", json!({ "file": "h.lcf", "wire": "a -> y", "via": [] }));
    assert_eq!((empty.code, empty.parameter.as_deref()), ("params.invalid", Some("via")));
}

/// The wires of `file` with `text` in its place, as the window would draw them: what `edit` asks the session.
fn wires_of(session: &mut Session, file: &str, text: &str) -> Vec<labcraft_edit::WireDrawn> {
    use labcraft_edit::Judge;
    let root = session.root.clone();
    session.checked(&root).unwrap();
    let kept = session.kept_root().unwrap();
    let memory = std::collections::BTreeMap::new();
    let judge = super::SessionJudge { session, kept: &kept.checked, memory: &memory, typed: Default::default() };
    judge.wires(file, text)
}

fn on_path(path: &[(i64, i64)], (x, y): (i64, i64)) -> bool {
    path.windows(2).any(|pair| {
        let (a, b) = (pair[0], pair[1]);
        (a.0 == b.0 && a.0 == x && a.1.min(b.1) <= y && y <= a.1.max(b.1)) || (a.1 == b.1 && a.1 == y && a.0.min(b.0) <= x && x <= a.0.max(b.0))
    })
}

/// After a `node.move` that an agent asks for, a junction still lies on its trunk, as the session draws it (visual
/// notation 2.5; decision of the owner, 2026-11-17): the engine moves it in the same step.
#[test]
fn a_junction_follows_its_trunk_after_the_move_of_an_agent() {
    let (mut session, memory) = session();
    let before = "labcraft 1\n\nuse core\n\nfn h(x: f64) -> (y: f64, z: f64) {\n    a = core.neg(x)\n    b = core.abs(a)\n    y = core.neg(b)\n    z = core.abs(a)\n}\n";
    memory.put("/p/h.lcf", before);
    // A point of the trunk `a -> b.x` on a horizontal, on the grid of units of the frame (whose origin the paths count
    // from), past its start.
    let wires = wires_of(&mut session, "h.lcf", before);
    let trunk = wires.iter().find(|wire| wire.name == "a -> b.x").unwrap().clone();
    let (start, end) = trunk.path.windows(2).map(|pair| (pair[0], pair[1])).find(|(p, q)| p.1 == q.1 && (q.0 - p.0).abs() >= 300).unwrap();
    let junction = ((start.0.min(end.0) / 100 + 1) * 100, start.1);
    assert!(junction.1 % 100 == 0, "{trunk:?}");
    let via = json!([[junction.0 / 100, junction.1 / 100]]);
    ok(&mut session, "wire.route", json!({ "file": "h.lcf", "wire": "a -> z.x", "via": via }));
    // The agent moves `a` and `b` down together: the trunk between them keeps its shape, lower.
    let agent = Author::Agent("test".to_owned());
    session.execute_as(&agent, "node.move", &json!({ "file": "h.lcf", "nodes": ["a", "b"], "by": [0, 40] })).unwrap();
    let after = text(&memory, "/p/h.lcf");
    let first = after
        .lines()
        .find_map(|line| line.trim().strip_prefix("wire a -> z via (")?.split(')').next().map(str::to_owned))
        .unwrap_or_else(|| panic!("the branch keeps its junction: {after}"));
    let (x, y) = first.split_once(", ").map(|(x, y)| (x.parse::<i64>().unwrap() * 100, y.parse::<i64>().unwrap() * 100)).unwrap();
    let moved = wires_of(&mut session, "h.lcf", &after);
    let trunk = moved.iter().find(|wire| wire.name == "a -> b.x").unwrap();
    assert!(on_path(&trunk.path, (x, y)), "{first} off {trunk:?}\n{after}");
    // It moved with the trunk: where it was is no longer on it.
    assert_ne!((x, y), junction, "{after}");
    assert!(!on_path(&trunk.path, junction), "{trunk:?}");
}

/// The fields in place of M1 step 4b through the session: each a step that undo reverses, the template read by the
/// check of the session (commands.md 6.7, 9.1).
#[test]
fn the_fields_in_place_are_steps_of_history() {
    let (mut session, memory) = session();
    let before = "labcraft 1\n\nuse core\nuse string\n\nfn h(x: f64) -> (y: f64, t: string) {\n    a = core.mul(x, 2.0)\n    t = string.format(\"{v}\", v: a)\n    y = core.add(a, 1.0)\n}\n";
    memory.put("/p/h.lcf", before);
    ok(&mut session, "input.set_constant", json!({ "file": "h.lcf", "to": "y.b", "value": "5.0", "replace": true }));
    let renamed = ok(&mut session, "value.rename", json!({ "file": "h.lcf", "value": "a", "name": "doubled" }));
    assert_eq!(renamed["names"], json!({ "value": "doubled" }));
    ok(&mut session, "node.set_template", json!({ "file": "h.lcf", "node": "t", "template": "\"{v} and {u}\"" }));
    let after = "labcraft 1\n\nuse core\nuse string\n\nfn h(x: f64) -> (y: f64, t: string) {\n    doubled = core.mul(x, 2.0)\n    t = string.format(\"{v} and {u}\", v: doubled)\n    y = core.add(doubled, 5.0)\n}\n";
    assert_eq!(text(&memory, "/p/h.lcf"), after);
    let author = Author::User;
    let bad = refused(&mut session, &author, "node.set_template", json!({ "file": "h.lcf", "node": "t", "template": "\"{v\"" }));
    assert_eq!((bad.code, bad.parameter.as_deref()), ("params.invalid", Some("template")));
    let line = refused(&mut session, &author, "node.set_label", json!({ "file": "h.lcf", "line": 0, "label": "#a" }));
    assert_eq!((line.code, line.parameter.as_deref()), ("params.invalid", Some("line")));
    for _ in 0..3 {
        ok(&mut session, "history.undo", json!({}));
    }
    assert_eq!(text(&memory, "/p/h.lcf"), before);
}

/// Copy and paste of M1 step 4c through the session: a copy writes nothing and is no step, a paste is one step that undo
/// reverses (commands.md 6.7). The texts are worked out by hand from commands.md 9.1, notation 2.6 and spec 5.2.
#[test]
fn a_copy_writes_nothing_and_a_paste_is_one_step() {
    let (mut session, memory) = session();
    let before = "labcraft 1\n\nuse core\n\nfn h(x: f64) -> (y: f64) {\n    a = core.mul(x, 2.0)\n    b = core.add(a, 1.0)\n    y = b\n}\n\nlayout {\n    node a at (40, 40)\n    node b at (140, 60)\n}\n";
    memory.put("/p/h.lcf", before);
    let copied = ok(&mut session, "fragment.copy", json!({ "file": "h.lcf", "nodes": ["a", "b"] }));
    let fragment = "labcraft 1\n\nuse core\n\nfn clipboard() -> () {\n    a = core.mul(?, 2.0)\n    b = core.add(a, 1.0)\n}\n\nlayout {\n    node a at (0, 0)\n    node b at (100, 20)\n}\n";
    assert_eq!(copied, json!({ "text": fragment }));
    assert_eq!(text(&memory, "/p/h.lcf"), before);
    assert_eq!(ok(&mut session, "history.list", json!({}))["steps"], json!([]));
    let author = Author::User;
    let batch = json!({ "commands": [{ "id": "fragment.copy", "params": { "file": "h.lcf", "nodes": ["a"] } }] });
    assert_eq!(refused(&mut session, &author, "commands.batch", batch).code, "params.invalid");

    let pasted = ok(&mut session, "fragment.paste", json!({ "file": "h.lcf", "text": fragment, "at": [200, 0] }));
    assert_eq!(pasted["names"], json!({ "nodes": ["a2", "b2"] }));
    assert_eq!(pasted["renamed"], json!([{ "file": "h.lcf", "from": "a", "to": "a2" }, { "file": "h.lcf", "from": "b", "to": "b2" }]));
    let after = "labcraft 1\n\nuse core\n\nfn h(x: f64) -> (y: f64) {\n    a = core.mul(x, 2.0)\n    a2 = core.mul(?, 2.0)\n    b = core.add(a, 1.0)\n    b2 = core.add(a2, 1.0)\n    y = b\n}\n\nlayout {\n    node a at (40, 40)\n    node a2 at (200, 0)\n    node b at (140, 60)\n    node b2 at (300, 20)\n}\n";
    assert_eq!(text(&memory, "/p/h.lcf"), after);
    let inside = refused(&mut session, &author, "fragment.paste", json!({ "file": "h.lcf", "text": fragment, "at": [0, 0], "inside": "m" }));
    // `m` names no structure of the function.
    assert_eq!((inside.code, inside.parameter.as_deref()), ("edit.unknown", Some("inside")));
    ok(&mut session, "history.undo", json!({}));
    assert_eq!(text(&memory, "/p/h.lcf"), before);
}

/// What a paste renames is of its fragment: a `$i` of a batch keeps naming the node that command `i` added
/// (commands.md 6.7, `commands.batch` and `fragment.paste`).
#[test]
fn a_paste_in_a_batch_leaves_the_names_of_the_earlier_commands() {
    let (mut session, memory) = session();
    memory.put("/p/h.lcf", "labcraft 1\n\nuse core\n\nfn h(x: f64) -> (y: f64) {\n    y = x\n}\n");
    let fragment = "labcraft 1\n\nuse core\n\nfn g() -> () {\n    sum = core.add(1.0, 2.0)\n}\n";
    let commands = json!([
        { "id": "node.add", "params": { "file": "h.lcf", "call": "core.add" } },
        { "id": "fragment.paste", "params": { "file": "h.lcf", "text": fragment, "at": [0, 0] } },
        { "id": "wire.connect", "params": { "file": "h.lcf", "from": "x", "to": "$0.a" } },
    ]);
    ok(&mut session, "commands.batch", json!({ "commands": commands }));
    let after = "labcraft 1\n\nuse core\n\nfn h(x: f64) -> (y: f64) {\n    sum = core.add(x)\n    sum2 = core.add(1.0, 2.0)\n    y = x\n}\n";
    assert_eq!(text(&memory, "/p/h.lcf"), after);
}

/// Where each node of `file` with `text` is in the frame of the function, by its name, as the window would draw it; `b` is
/// a node of that frame.
fn absolute_of(session: &mut Session, file: &str, text: &str) -> std::collections::BTreeMap<String, (i64, i64)> {
    use labcraft_edit::Judge;
    let root = session.root.clone();
    session.checked(&root).unwrap();
    let kept = session.kept_root().unwrap();
    let memory = std::collections::BTreeMap::new();
    let judge = super::SessionJudge { session, kept: &kept.checked, memory: &memory, typed: Default::default() };
    let places = judge.places(file, text);
    // In the frame of the function: the place of a node of the body, less where it is on the canvas.
    let origin =
        places.iter().find(|place| place.name == "b").map(|place| (place.absolute.x - place.at.x, place.absolute.y - place.at.y)).unwrap_or_default();
    places.into_iter().map(|place| (place.name, (place.absolute.x - origin.0, place.absolute.y - origin.1))).collect()
}

/// Surrounding moves nothing on the screen: each node keeps where the drawing had it, counted in its new frame, whatever
/// the frame of the arm takes for its header and tabs (commands.md 9.6). A selector of the wrong type is refused, as the
/// type that the check gives it says.
#[test]
fn a_structure_surrounds_its_nodes_where_they_are() {
    let (mut session, memory) = session();
    let before = "labcraft 1\n\nuse core\n\nfn h(x: f64, ok: bool) -> (y: f64) {\n    a = core.neg(x)\n    b = core.abs(a)\n    y = core.abs(b)\n}\n\nlayout {\n    node a at (40, 80)\n    node b at (200, 120)\n}\n";
    memory.put("/p/h.lcf", before);
    let was = std::collections::BTreeMap::from([("a".to_owned(), (40, 80)), ("b".to_owned(), (200, 120))]);
    let result = ok(&mut session, "structure.surround", json!({ "file": "h.lcf", "kind": "match", "selector": "ok", "nodes": ["a", "b"] }));
    assert_eq!(result["names"]["structure"], "b");
    let after = text(&memory, "/p/h.lcf");
    assert!(after.contains("b = match ok {\n        true => {\n            a = core.neg(x)\n            b2 = core.abs(a)\n            yield b2\n        }\n        false => yield ?\n    }"), "{after}");
    assert!(after.contains("    node a at (0, 0)\n") && after.contains("    node b2 at (160, 40)\n"), "{after}");
    let now = absolute_of(&mut session, "h.lcf", &after);
    assert_eq!((now.get("a"), now.get("b2")), (was.get("a"), was.get("b")), "{after}");
    ok(&mut session, "history.undo", json!({}));
    assert_eq!(text(&memory, "/p/h.lcf"), before);
    let author = Author::User;
    let float = refused(&mut session, &author, "structure.surround", json!({ "file": "h.lcf", "kind": "match", "selector": "x", "nodes": ["a"] }));
    assert_eq!((float.code, float.parameter.as_deref()), ("wire.type_mismatch", Some("selector")));
    assert!(float.message.contains("`f64`"), "{}", float.message);
}

/// A wire out of a loop takes the value out as a carry of its type, which the check gives, or as a `last` (commands.md
/// 6.7, `through`); a part of a structure is one step, undone at once.
#[test]
fn a_wire_out_of_a_loop_carries_its_type() {
    let (mut session, memory) = session();
    let before = "labcraft 1\n\nuse core\n\nfn h(xs: [f64]) -> (y: f64) {\n    _ = for v in xs {\n        w = core.abs(v)\n    }\n    y = 0.0\n}\n";
    memory.put("/p/h.lcf", before);
    let author = Author::User;
    let out = refused(&mut session, &author, "wire.connect", json!({ "file": "h.lcf", "from": "w", "to": "output y", "replace": true }));
    assert_eq!(out.code, "wire.out_of_scope");
    ok(&mut session, "wire.connect", json!({ "file": "h.lcf", "from": "w", "to": "output y", "replace": true, "through": "carry" }));
    let after = text(&memory, "/p/h.lcf");
    assert!(after.contains("    y = for v in xs, carry w2: f64 = ? {\n        w = core.abs(v)\n        next w2 = w\n    }\n"), "{after}");
    let bad = refused(&mut session, &author, "wire.connect", json!({ "file": "h.lcf", "from": "w", "to": "output y", "through": "first" }));
    assert_eq!((bad.code, bad.parameter.as_deref()), ("params.invalid", Some("through")));
    ok(&mut session, "history.undo", json!({}));
    assert_eq!(text(&memory, "/p/h.lcf"), before);
    // A carry of an `error` starts from `none`, not from a hole (decision of the owner, 2026-11-18).
    let errors = "labcraft 1\n\nuse time\n\nfn w(err: error = none) -> (out: error) {\n    _ = loop {\n        e = time.wait(1 s, err: err)\n        until true\n    }\n    out = err\n}\n";
    memory.put("/p/w.lcf", errors);
    ok(&mut session, "wire.connect", json!({ "file": "w.lcf", "from": "e", "to": "output out", "replace": true, "through": "carry" }));
    let after = text(&memory, "/p/w.lcf");
    assert!(
        after.contains("    out = loop carry e2 = none {\n        e = time.wait(1 s, err: err)\n        next e2 = e\n        until true\n    }\n"),
        "{after}"
    );
    // A `last` takes no initial value (spec 6.2, #211).
    ok(&mut session, "wire.connect", json!({ "file": "h.lcf", "from": "w", "to": "output y", "replace": true, "through": "last" }));
    let after = text(&memory, "/p/h.lcf");
    assert!(
        after.contains(
            "    y = for v in xs {
        w = core.abs(v)
        last w
    }
"
        ),
        "{after}"
    );
    ok(&mut session, "history.undo", json!({}));
    assert_eq!(text(&memory, "/p/h.lcf"), before);
    let added = ok(&mut session, "structure.add_collect", json!({ "file": "h.lcf", "structure": "@6", "value": "w" }));
    assert_eq!(added["names"]["output"], "w2");
    assert!(text(&memory, "/p/h.lcf").contains("    w2 = for v in xs {\n        w = core.abs(v)\n        collect w\n    }\n"));
}

/// A part of a loop changes kind in one step (commands.md 6.7, `structure.set_passage`, #215): a `last` becomes a carry of
/// the type that the check gives its value, written by hand from that row; undo restores the text.
#[test]
fn a_part_of_a_loop_changes_kind_in_one_step() {
    let (mut session, memory) = session();
    let before = "labcraft 1\n\nuse core\n\nfn h(xs: [f64]) -> (y: f64) {\n    m = for v in xs {\n        w = core.abs(v)\n        last w\n    }\n    y = m\n}\n";
    memory.put("/p/h.lcf", before);
    let changed = ok(&mut session, "structure.set_passage", json!({ "file": "h.lcf", "structure": "m", "part": "last m", "to": "carry" }));
    assert_eq!(changed["names"]["output"], "m");
    let after = text(&memory, "/p/h.lcf");
    assert!(after.contains("    m = for v in xs, carry m: f64 = ? {\n        w = core.abs(v)\n        next m = w\n    }\n    y = m\n"), "{after}");
    ok(&mut session, "history.undo", json!({}));
    assert_eq!(text(&memory, "/p/h.lcf"), before);
    let author = Author::User;
    let bad = refused(&mut session, &author, "structure.set_passage", json!({ "file": "h.lcf", "structure": "m", "part": "last m", "to": "first" }));
    assert_eq!((bad.code, bad.parameter.as_deref()), ("params.invalid", Some("to")));
}
