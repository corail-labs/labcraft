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

/// The notice of an agent undoes the step it names, or nothing (ide.md 8).
#[test]
fn an_undo_of_a_step_named_undoes_it_only_while_it_is_the_last() {
    let (mut session, memory) = session();
    ok(&mut session, "node.add", json!({ "file": "g.lcf", "call": "core.add" }));
    ok(&mut session, "constant.add", json!({ "file": "g.lcf", "value": "240 V" }));
    let after = text(&memory, "/p/g.lcf");
    let error = refused(&mut session, &Author::User, "history.undo", json!({ "step": 1 }));
    assert_eq!(error.code, "file.stale");
    assert!(error.message.contains("not step 1"), "{}", error.message);
    assert_eq!(text(&memory, "/p/g.lcf"), after, "nothing changed");
    assert_eq!(ok(&mut session, "history.undo", json!({ "step": 2 }))["step"], json!(2));
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

/// `frame.resize` through the session: it writes the entry of the structure, a size under what the drawing says the
/// frame holds is refused with that least size in the message, and exactly that size is accepted (commands.md
/// `frame.resize`). The least is read from the message and checked against the rule (one unit less in either direction
/// is refused, the least itself is not), never typed in from the output of a run.
#[test]
fn a_frame_is_resized_and_a_size_under_its_content_is_refused_with_the_least() {
    let (mut session, memory) = session();
    let before = "labcraft 1\n\nfn h(v: [f64]) -> (ys: [f64]) {\n    ys = for x in v {\n        collect x\n    }\n}\n";
    memory.put("/p/h.lcf", before);
    // Large enough for any content of this loop (the limit of a coordinate is 1_000_000).
    ok(&mut session, "frame.resize", json!({ "file": "h.lcf", "inside": "ys", "size": [900, 700] }));
    assert_eq!(text(&memory, "/p/h.lcf"), format!("{before}\nlayout {{\n    frame ys size (900, 700)\n}}\n"));
    // Again: one entry.
    ok(&mut session, "frame.resize", json!({ "file": "h.lcf", "inside": "ys", "size": [901, 701] }));
    assert_eq!(text(&memory, "/p/h.lcf"), format!("{before}\nlayout {{\n    frame ys size (901, 701)\n}}\n"));
    let agent = Author::Agent("test".to_owned());
    let error = refused(&mut session, &agent, "frame.resize", json!({ "file": "h.lcf", "inside": "ys", "size": [1, 1] }));
    assert_eq!((error.code, error.parameter.as_deref()), ("params.invalid", Some("size")));
    let least = error.message.split("least size is [").nth(1).and_then(|rest| rest.split(']').next()).unwrap_or_else(|| panic!("{}", error.message));
    let (w, h) = least.split_once(", ").map(|(w, h)| (w.parse::<i64>().unwrap(), h.parse::<i64>().unwrap())).unwrap();
    assert!(w > 1 && h > 1, "{least}");
    refused(&mut session, &agent, "frame.resize", json!({ "file": "h.lcf", "inside": "ys", "size": [w - 1, h] }));
    refused(&mut session, &agent, "frame.resize", json!({ "file": "h.lcf", "inside": "ys", "size": [w, h - 1] }));
    ok(&mut session, "frame.resize", json!({ "file": "h.lcf", "inside": "ys", "size": [w, h] }));
    assert_eq!(text(&memory, "/p/h.lcf"), format!("{before}\nlayout {{\n    frame ys size ({w}, {h})\n}}\n"));
    // The frame of the function, without `inside`.
    ok(&mut session, "frame.resize", json!({ "file": "h.lcf", "size": [900, 700] }));
    assert_eq!(text(&memory, "/p/h.lcf"), format!("{before}\nlayout {{\n    frame size (900, 700)\n    frame ys size ({w}, {h})\n}}\n"));
    // `size: null` removes an entry ("Remove the fixed size"), the last one the section; a missing `size` is `params.missing` (commands.md, section 2).
    ok(&mut session, "frame.resize", json!({ "file": "h.lcf", "inside": "ys", "size": null }));
    assert_eq!(text(&memory, "/p/h.lcf"), format!("{before}\nlayout {{\n    frame size (900, 700)\n}}\n"));
    ok(&mut session, "frame.resize", json!({ "file": "h.lcf", "size": null }));
    assert_eq!(text(&memory, "/p/h.lcf"), before);
    let error = refused(&mut session, &agent, "frame.resize", json!({ "file": "h.lcf" }));
    assert_eq!((error.code, error.parameter.as_deref()), ("params.missing", Some("size")));
}

// The function itself (commands.md 6.7, "Functions, signatures and types").

#[test]
fn a_new_function_is_one_step_that_undo_removes() {
    let (mut session, memory) = session();
    let result = ok(&mut session, "function.create", json!({ "path": "tools/check.lcf", "doc": "Checks a channel." }));
    let made = "labcraft 1\n\n/// Checks a channel.\nfn check() -> () {}\n";
    assert_eq!(text(&memory, "/p/tools/check.lcf"), made);
    assert_eq!(result["step"], 1);
    assert_eq!(result["files"], json!([{ "file": "tools/check.lcf", "revision": sha(made) }]));
    assert_eq!(result["names"], json!({ "function": "check" }));
    ok(&mut session, "history.undo", json!({}));
    assert_eq!(memory.get("/p/tools/check.lcf"), None);
    let agent = Author::Agent("claude".to_owned());
    assert_eq!(refused(&mut session, &agent, "function.create", json!({ "path": "f.lcf" })).code, "path.exists");
    assert_eq!(refused(&mut session, &agent, "function.create", json!({ "path": "Notes.lct" })).code, "path.invalid");
    assert_eq!(refused(&mut session, &agent, "function.create", json!({ "path": "2nd.lcf" })).code, "name.invalid");
    let batch = json!({ "commands": [{ "id": "function.create", "params": { "path": "h.lcf" } }] });
    assert_eq!(refused(&mut session, &agent, "commands.batch", batch).code, "params.invalid");
    assert_eq!(memory.get("/p/h.lcf"), None);
}

/// The coordinates of the `node NAME at (x, y)` entry of a text.
fn entry(text: &str, name: &str) -> (i64, i64) {
    let rest = text.split(&format!("    node {name} at (")).nth(1).unwrap_or_else(|| panic!("no entry for {name}: {text}"));
    let (x, rest) = rest.split_once(", ").unwrap();
    let y = rest.split_once(')').unwrap().0;
    (x.parse().unwrap(), y.parse().unwrap())
}

/// Three nodes become a function of their own (commands.md 9.8, scenario 9 of acceptance-m1.md): what they read from
/// outside its inputs, what is read outside its output, named as the values; the call where the nodes were, the nodes
/// apart as they were; one step over both files, which undo takes back. The texts are written by hand from 9.8.
#[test]
fn nodes_become_a_function_and_a_call_in_their_place() {
    let (mut session, memory) = session();
    let before = "labcraft 1\n\nuse core\n\nfn h(v: f64[V], low: f64[V], high: f64[V]) -> (ok: bool) {\n    above = core.ge(v, low)\n    below = core.le(v, high)\n    in_range = core.and(above, below)\n    ok = in_range\n}\n\nlayout {\n    node above at (40, 80)\n    node below at (40, 160)\n    node in_range at (120, 120)\n}\n";
    memory.put("/p/h.lcf", before);
    let result = ok(&mut session, "function.extract", json!({ "file": "h.lcf", "nodes": ["above", "below", "in_range"], "name": "in_limits" }));
    assert_eq!(result["names"], json!({ "function": "in_limits", "file": "in_limits.lcf", "node": "in_range" }));
    let caller = "labcraft 1\n\nuse core\n\nfn h(v: f64[V], low: f64[V], high: f64[V]) -> (ok: bool) {\n    in_range = in_limits(v, low, high)\n    ok = in_range\n}\n\nlayout {\n    node in_range at (40, 80)\n}\n";
    assert_eq!(text(&memory, "/p/h.lcf"), caller);
    let made = text(&memory, "/p/in_limits.lcf");
    let (x, y) = entry(&made, "above");
    let expected = format!(
        "labcraft 1\n\nuse core\n\nfn in_limits(v: f64[V], low: f64[V], high: f64[V]) -> (in_range: bool) {{\n    above = core.ge(v, low)\n    below = core.le(v, high)\n    in_range = core.and(above, below)\n}}\n\nlayout {{\n    node above at ({x}, {y})\n    node below at ({x}, {})\n    node in_range at ({}, {})\n}}\n",
        y + 80,
        x + 80,
        y + 40
    );
    assert_eq!(made, expected);
    // The new file first: it is written before the caller.
    assert_eq!(result["files"], json!([{ "file": "in_limits.lcf", "revision": sha(&made) }, { "file": "h.lcf", "revision": sha(caller) }]));
    assert_eq!(result["summary"]["errors"], 0, "{result}");
    ok(&mut session, "history.undo", json!({}));
    assert_eq!(text(&memory, "/p/h.lcf"), before);
    assert_eq!(memory.get("/p/in_limits.lcf"), None);
    ok(&mut session, "history.redo", json!({}));
    assert_eq!((text(&memory, "/p/h.lcf"), text(&memory, "/p/in_limits.lcf")), (caller.to_owned(), made));
}

/// One error that enters and one that leaves become the `err` of the new function, the last on each side; the caller
/// still binds its own names (commands.md 9.8, 9.6). A name that is taken or is no name, and a batch, are refused.
#[test]
fn the_error_chain_becomes_err_and_a_taken_name_is_refused() {
    let (mut session, memory) = session();
    let before = "labcraft 1\n\nuse time\n\nfn w(err: error = none) -> (err: error) {\n    e1 = time.wait(1 s, err: err)\n    e2 = time.wait(2 s, err: e1)\n    err = time.wait(3 s, err: e2)\n}\n";
    memory.put("/p/w.lcf", before);
    let agent = Author::Agent("claude".to_owned());
    let taken = refused(&mut session, &agent, "function.extract", json!({ "file": "w.lcf", "nodes": ["e1"], "name": "f" }));
    assert_eq!((taken.code, taken.parameter.as_deref()), ("name.taken", Some("name")));
    let library = refused(&mut session, &agent, "function.extract", json!({ "file": "w.lcf", "nodes": ["e1"], "name": "time" }));
    assert_eq!(library.code, "name.taken");
    // A value of the caller: the call would read as that value (as for `function.rename`).
    let value = refused(&mut session, &agent, "function.extract", json!({ "file": "w.lcf", "nodes": ["e1"], "name": "e2" }));
    assert_eq!(value.code, "name.taken");
    let invalid = refused(&mut session, &agent, "function.extract", json!({ "file": "w.lcf", "nodes": ["e1"], "name": "2nd" }));
    assert_eq!(invalid.code, "name.invalid");
    let batch = json!({ "commands": [{ "id": "function.extract", "params": { "file": "w.lcf", "nodes": ["e1"], "name": "g2" } }] });
    assert_eq!(refused(&mut session, &agent, "commands.batch", batch).code, "params.invalid");
    assert_eq!(text(&memory, "/p/w.lcf"), before);
    let result = ok(&mut session, "function.extract", json!({ "file": "w.lcf", "nodes": ["e1", "e2"], "name": "wait_twice" }));
    assert_eq!(result["names"]["node"], "e2");
    // The nodes were placed automatically: the call is fixed where the drawing had them, and so are they in the new
    // function, which the drawing gives (9.8); their places are checked by the test above, on entries written by hand.
    let caller = text(&memory, "/p/w.lcf");
    let (x, y) = entry(&caller, "e2");
    assert_eq!(
        caller,
        format!(
            "labcraft 1\n\nuse time\n\nfn w(err: error = none) -> (err: error) {{\n    e2 = wait_twice(err: err)\n    err = time.wait(3 s, err: e2)\n}}\n\nlayout {{\n    node e2 at ({x}, {y})\n}}\n"
        )
    );
    let made = text(&memory, "/p/wait_twice.lcf");
    let body = "labcraft 1\n\nuse time\n\nfn wait_twice(err: error = none) -> (err: error) {\n    e1 = time.wait(1 s, err: err)\n    err = time.wait(2 s, err: e1)\n}\n";
    assert!(made.starts_with(body), "{made}");
    // The entry of the node that gave `e2` follows its new name.
    let (e1, err) = (entry(&made, "e1"), entry(&made, "err"));
    assert!(err.0 > e1.0, "{made}");
    assert_eq!(result["summary"]["errors"], 0, "{result}");
    // An error that leaves and that the nodes read too is given by a line `err = e2`: renamed, its reader would read the
    // input `err` (R-2).
    let read = "labcraft 1\n\nuse time\n\nfn r(err: error = none) -> (err: error) {\n    e1 = time.wait(1 s, err: err)\n    e2 = time.wait(2 s, err: e1)\n    e3 = time.wait(3 s, err: e2)\n    err = e2\n}\n";
    memory.put("/p/r.lcf", read);
    ok(&mut session, "function.extract", json!({ "file": "r.lcf", "nodes": ["e1", "e2", "e3"], "name": "three" }));
    let caller = text(&memory, "/p/r.lcf");
    assert!(
        caller.starts_with("labcraft 1\n\nuse time\n\nfn r(err: error = none) -> (err: error) {\n    e2 = three(err: err)\n    err = e2\n}\n"),
        "{caller}"
    );
    let made = text(&memory, "/p/three.lcf");
    let body = "labcraft 1\n\nuse time\n\nfn three(err: error = none) -> (err: error) {\n    e1 = time.wait(1 s, err: err)\n    e2 = time.wait(2 s, err: e1)\n    e3 = time.wait(3 s, err: e2)\n    err = e2\n}\n";
    assert!(made.starts_with(body), "{made}");
    // A node that reads the input `err` and binds the output of its name (R-2): the input enters, the output leaves.
    let both = "labcraft 1\n\nuse time\n\nfn m(err: error = none) -> (err: error) {\n    err = time.wait(1 s, err: err)\n}\n";
    memory.put("/p/m.lcf", both);
    ok(&mut session, "function.extract", json!({ "file": "m.lcf", "nodes": ["err"], "name": "once" }));
    let caller = text(&memory, "/p/m.lcf");
    assert!(caller.starts_with("labcraft 1\n\nuse time\n\nfn m(err: error = none) -> (err: error) {\n    err = once(err: err)\n}\n"), "{caller}");
    let made = text(&memory, "/p/once.lcf");
    assert!(
        made.starts_with("labcraft 1\n\nuse time\n\nfn once(err: error = none) -> (err: error) {\n    err = time.wait(1 s, err: err)\n}\n"),
        "{made}"
    );
    // A text not in canonical form: the types are those of its canonical form, which the command writes.
    let loose = "labcraft 1\n\nuse core\n\nfn n(x: f64) -> (y: f64) {\n    y = core.add(a, 1.0)\n    a = core.mul(x, 2.0)\n}\n";
    memory.put("/p/n.lcf", loose);
    ok(&mut session, "function.extract", json!({ "file": "n.lcf", "nodes": ["y"], "name": "plus_one" }));
    let caller = text(&memory, "/p/n.lcf");
    assert!(
        caller.starts_with("labcraft 1\n\nuse core\n\nfn n(x: f64) -> (y: f64) {\n    a = core.mul(x, 2.0)\n    y = plus_one(a)\n}\n"),
        "{caller}"
    );
    let made = text(&memory, "/p/plus_one.lcf");
    assert!(made.starts_with("labcraft 1\n\nuse core\n\nfn plus_one(a: f64) -> (y: f64) {\n    y = core.add(a, 1.0)\n}\n"), "{made}");
}

/// A project of `f` and a caller `main`, which names the input `gain` of `f`.
fn called() -> (Session, Arc<Memory>) {
    let memory = Arc::new(Memory::new());
    memory.put("/p/f.lcf", CALLED);
    memory.put("/p/main.lcf", CALLER);
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

const CALLED: &str = "labcraft 1\n\nfn f(x: f64, gain: f64 = 1) -> (y: f64) {\n    y = x\n}\n";
const CALLER: &str = "labcraft 1\n\nfn main(k: f64) -> (r: f64) {\n    r = f(k, gain: 2)\n}\n";

#[test]
fn an_input_renamed_renames_the_calls_of_the_project_in_one_step() {
    let (mut session, memory) = called();
    let result = ok(&mut session, "signature.rename", json!({ "file": "f.lcf", "terminal": "input gain", "name": "scale" }));
    let function = "labcraft 1\n\nfn f(x: f64, scale: f64 = 1) -> (y: f64) {\n    y = x\n}\n";
    let caller = "labcraft 1\n\nfn main(k: f64) -> (r: f64) {\n    r = f(k, scale: 2)\n}\n";
    assert_eq!((text(&memory, "/p/f.lcf"), text(&memory, "/p/main.lcf")), (function.to_owned(), caller.to_owned()));
    assert_eq!(result["step"], 1);
    assert_eq!(result["files"], json!([{ "file": "f.lcf", "revision": sha(function) }, { "file": "main.lcf", "revision": sha(caller) }]));
    assert_eq!(result["names"], json!({ "terminal": "input scale" }));
    assert_eq!(result["summary"]["errors"], 0, "{result}");
    ok(&mut session, "history.undo", json!({}));
    assert_eq!((text(&memory, "/p/f.lcf"), text(&memory, "/p/main.lcf")), (CALLED.to_owned(), CALLER.to_owned()), "one undo takes both back");
}

#[test]
fn a_rename_stops_at_a_caller_that_does_not_parse() {
    let (mut session, memory) = called();
    memory.put("/p/broken.lcf", "labcraft 1\n\nfn broken() -> () {\n    r = f(1, gain: \n}\n");
    let error = refused(&mut session, &Author::User, "signature.rename", json!({ "file": "f.lcf", "terminal": "input gain", "name": "scale" }));
    assert_eq!(error.code, "file.unparsed");
    assert!(error.message.contains("broken.lcf"), "{}", error.message);
    assert_eq!((text(&memory, "/p/f.lcf"), text(&memory, "/p/main.lcf")), (CALLED.to_owned(), CALLER.to_owned()));
}

#[test]
fn a_new_required_input_shows_its_callers_unwired() {
    let (mut session, memory) = called();
    let result = ok(&mut session, "signature.add", json!({ "file": "f.lcf", "terminal": "input offset", "type": "f64" }));
    assert_eq!(text(&memory, "/p/f.lcf"), "labcraft 1\n\nfn f(x: f64, gain: f64 = 1, offset: f64) -> (y: f64) {\n    y = x\n}\n");
    assert_eq!(text(&memory, "/p/main.lcf"), CALLER, "callers are not changed");
    let unwired: Vec<(&str, &str)> = result["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|diagnostic| Some((diagnostic["file"].as_str()?, diagnostic["code"].as_str()?)))
        .collect();
    assert!(unwired.contains(&("main.lcf", "LC2101")), "{result}");
    let error = refused(&mut session, &Author::User, "signature.add", json!({ "file": "f.lcf", "terminal": "x", "type": "f64" }));
    assert_eq!((error.code, error.parameter.as_deref()), ("params.invalid", Some("terminal")));
}

#[test]
fn the_documentation_type_and_default_are_steps() {
    let (mut session, memory) = called();
    ok(&mut session, "function.set_doc", json!({ "file": "f.lcf", "doc": "Scales x." }));
    ok(&mut session, "signature.set_type", json!({ "file": "f.lcf", "terminal": "input x", "type": "f64[V]" }));
    ok(&mut session, "signature.set_type", json!({ "file": "f.lcf", "terminal": "output y", "type": "f64[V]" }));
    ok(&mut session, "signature.set_default", json!({ "file": "f.lcf", "terminal": "input gain", "default": null }));
    assert_eq!(text(&memory, "/p/f.lcf"), "labcraft 1\n\n/// Scales x.\nfn f(x: f64[V], gain: f64) -> (y: f64[V]) {\n    y = x\n}\n");
    ok(&mut session, "signature.remove", json!({ "file": "f.lcf", "terminal": "input gain" }));
    assert_eq!(text(&memory, "/p/f.lcf"), "labcraft 1\n\n/// Scales x.\nfn f(x: f64[V]) -> (y: f64[V]) {\n    y = x\n}\n");
    for _ in 0..5 {
        ok(&mut session, "history.undo", json!({}));
    }
    assert_eq!(text(&memory, "/p/f.lcf"), CALLED);
}

#[test]
fn a_terminal_moved_writes_its_entry_and_arrange_takes_it_away() {
    // commands.md 6.7, `terminal.move`; spec 11: `output y at 40`. "Remove the fixed position" is `layout.arrange`.
    let (mut session, memory) = called();
    let result = ok(&mut session, "terminal.move", json!({ "file": "f.lcf", "terminal": "output y", "y": 40 }));
    let moved = "labcraft 1\n\nfn f(x: f64, gain: f64 = 1) -> (y: f64) {\n    y = x\n}\n\nlayout {\n    output y at 40\n}\n";
    assert_eq!(text(&memory, "/p/f.lcf"), moved);
    assert_eq!(result["names"], json!({ "terminal": "output y" }));
    assert_eq!(result["summary"]["errors"], 0, "{result}");
    ok(&mut session, "layout.arrange", json!({ "file": "f.lcf", "terminals": ["output y"] }));
    assert_eq!(text(&memory, "/p/f.lcf"), CALLED);
    let error = refused(&mut session, &Author::User, "terminal.move", json!({ "file": "f.lcf", "terminal": "input y", "y": 40 }));
    assert_eq!((error.code, error.parameter.as_deref()), ("edit.unknown", Some("terminal")));
    let error = refused(&mut session, &Author::User, "terminal.move", json!({ "file": "f.lcf", "terminal": "output y", "y": 4.5 }));
    assert_eq!(error.code, "params.invalid");
}

#[test]
fn a_function_renamed_moves_its_file_and_renames_its_callers_in_one_step() {
    // commands.md 6.7, `function.rename`: the file of the function is named after it (R-22), and `main` calls it.
    let (mut session, memory) = called();
    let result = ok(&mut session, "function.rename", json!({ "file": "f.lcf", "name": "scale" }));
    let function = "labcraft 1\n\nfn scale(x: f64, gain: f64 = 1) -> (y: f64) {\n    y = x\n}\n";
    let caller = "labcraft 1\n\nfn main(k: f64) -> (r: f64) {\n    r = scale(k, gain: 2)\n}\n";
    assert!(memory.get("/p/f.lcf").is_none(), "the old file goes");
    assert_eq!((text(&memory, "/p/scale.lcf"), text(&memory, "/p/main.lcf")), (function.to_owned(), caller.to_owned()));
    assert_eq!(result["names"], json!({ "function": "scale", "file": "scale.lcf" }));
    assert_eq!(result["summary"]["errors"], 0, "{result}");
    ok(&mut session, "history.undo", json!({}));
    assert!(memory.get("/p/scale.lcf").is_none());
    assert_eq!((text(&memory, "/p/f.lcf"), text(&memory, "/p/main.lcf")), (CALLED.to_owned(), CALLER.to_owned()), "one undo takes all back");
    // A name whose file exists, a name of no function, and a type file.
    let error = refused(&mut session, &Author::User, "function.rename", json!({ "file": "f.lcf", "name": "main" }));
    assert_eq!((error.code, error.parameter.as_deref()), ("path.exists", Some("name")));
    let error = refused(&mut session, &Author::User, "function.rename", json!({ "file": "f.lcf", "name": "two words" }));
    assert_eq!(error.code, "name.invalid");
    let error = refused(
        &mut session,
        &Author::User,
        "commands.batch",
        json!({ "commands": [{ "id": "function.rename", "params": { "file": "f.lcf", "name": "g" } }] }),
    );
    assert_eq!(error.code, "params.invalid");
}

#[test]
fn a_type_made_then_renamed_renames_its_uses() {
    // commands.md 6.7, `type.create` and `type.rename`; spec 3.2.
    let (mut session, memory) = called();
    let result =
        ok(&mut session, "type.create", json!({ "path": "Reading.lct", "definition": "struct { value: f64, ok: bool }", "doc": "A reading." }));
    assert_eq!(text(&memory, "/p/Reading.lct"), "labcraft 1\n\n/// A reading.\ntype Reading = struct { value: f64, ok: bool }\n");
    assert_eq!(result["names"], json!({ "type": "Reading" }));
    let error = refused(&mut session, &Author::User, "type.create", json!({ "path": "Reading.lct", "definition": "bool" }));
    assert_eq!(error.code, "path.exists");
    let error = refused(&mut session, &Author::User, "type.create", json!({ "path": "Other.lct", "definition": "struct {" }));
    assert_eq!((error.code, error.parameter.as_deref()), ("params.invalid", Some("definition")));
    let error = refused(&mut session, &Author::User, "type.create", json!({ "path": "Other.lcf", "definition": "bool" }));
    assert_eq!(error.code, "path.invalid");
    memory.put("/p/user.lcf", "labcraft 1\n\nfn user(r: Reading) -> (v: f64) {\n    v = r.value\n}\n");
    let result = ok(&mut session, "type.rename", json!({ "file": "Reading.lct", "name": "Sample" }));
    assert_eq!(text(&memory, "/p/Sample.lct"), "labcraft 1\n\n/// A reading.\ntype Sample = struct { value: f64, ok: bool }\n");
    assert_eq!(text(&memory, "/p/user.lcf"), "labcraft 1\n\nfn user(r: Sample) -> (v: f64) {\n    v = r.value\n}\n");
    assert!(memory.get("/p/Reading.lct").is_none());
    assert_eq!(result["names"], json!({ "type": "Sample", "file": "Sample.lct" }));
    assert_eq!(result["summary"]["errors"], 0, "{result}");
}

#[test]
fn the_tab_of_a_function_renamed_follows_its_name_through_undo_and_redo() {
    // Decision of the owner, 2026-11-19: the tab follows the rename, its undo and its redo.
    let (mut session, _) = called();
    ok(&mut session, "view.open", json!({ "folder": "." }));
    ok(&mut session, "view.show_function", json!({ "function": "f" }));
    let shown = |session: &Session| session.view.published().main.function.clone();
    ok(&mut session, "function.rename", json!({ "file": "f.lcf", "name": "scale" }));
    session.view.settle();
    assert_eq!(shown(&session).as_deref(), Some("scale"));
    ok(&mut session, "history.undo", json!({}));
    session.view.settle();
    assert_eq!(shown(&session).as_deref(), Some("f"));
    ok(&mut session, "history.redo", json!({}));
    session.view.settle();
    assert_eq!(shown(&session).as_deref(), Some("scale"));
}

/// `bytes`, a primitive added after M0, gives way to a folder `bytes` of the project (`docs/cli.md`, section 3, rule 1):
/// `node.add` takes the function of the folder, and refuses one the folder does not have, as for any library; with no such
/// folder, it adds the primitive. `functions.list` lists only what a call would find.
#[test]
fn a_folder_bytes_replaces_the_primitive_for_node_add_and_functions_list() {
    let (mut session, memory) = session();
    ok(&mut session, "node.add", json!({ "file": "f.lcf", "call": "bytes.from_u16" }));
    assert!(text(&memory, "/p/f.lcf").contains("    data, _ = bytes.from_u16()\n"), "{}", text(&memory, "/p/f.lcf"));
    memory.put("/p/f.lcf", START);
    memory.put("/p/bytes/len.lcf", "labcraft 1\n\nfn len() -> (n: i32) {\n    n = 2\n}\n");
    let refused = session.execute("node.add", &json!({ "file": "f.lcf", "call": "bytes.from_u16" })).unwrap_err();
    assert_eq!(refused.code, "edit.unknown");
    ok(&mut session, "node.add", json!({ "file": "f.lcf", "call": "bytes.len" }));
    assert!(text(&memory, "/p/f.lcf").contains("    n = bytes.len()\n"), "{}", text(&memory, "/p/f.lcf"));
    let listed = ok(&mut session, "functions.list", json!({ "filter": "bytes.len" }));
    let signatures: Vec<&str> = listed["entries"].as_array().unwrap().iter().filter_map(|entry| entry["signature"].as_str()).collect();
    assert_eq!(signatures, ["fn bytes.len() -> (n: i32)"]);
}

/// The same for a library `bytes` of a `--lib-path`, which replaces the primitive once a file uses it (rule 3 before rule
/// 4); and a file that already uses the primitive keeps it.
#[test]
fn a_library_bytes_of_a_lib_path_replaces_the_primitive_too() {
    let memory = Arc::new(Memory::new());
    memory.put("/p/f.lcf", "labcraft 1\n\nuse bytes\n\nfn f(x: f64) -> (y: f64) {\n    y = x\n}\n");
    memory.put("/p/g.lcf", START.replace("fn f(", "fn g(").as_str());
    memory.put("/lib/bytes/len.lcf", "labcraft 1\n\nfn len() -> (n: i32) {\n    n = 2\n}\n");
    let config = |lib_paths: Vec<PathBuf>| Config {
        root: PathBuf::from("/p"),
        lib_paths,
        limits: Limits::default(),
        shipped: Shipped::default(),
        natives: Vec::new(),
        clock: Arc::new(Still),
        process: 0,
    };
    let ws: Arc<dyn Workspace> = memory.clone();
    let mut session = Session::new(ws.clone(), config(vec![PathBuf::from("/lib")]), false).unwrap_or_else(|error| panic!("{error:?}"));
    for file in ["f.lcf", "g.lcf"] {
        let refused = session.execute("node.add", &json!({ "file": file, "call": "bytes.from_u16" })).unwrap_err();
        assert_eq!(refused.code, "edit.unknown", "{file}");
    }
    let listed = ok(&mut session, "functions.list", json!({ "filter": "bytes.len" }));
    let signatures: Vec<&str> = listed["entries"].as_array().unwrap().iter().filter_map(|entry| entry["signature"].as_str()).collect();
    // The primitive is not listed; nor is the library of the `--lib-path`, as for any of them (commands.md 6.1 lists the
    // primitives, the shipped libraries and the project).
    assert_eq!(signatures, [] as [&str; 0]);
    // Without the `--lib-path`, `use bytes` is the primitive.
    let mut session = Session::new(ws, config(Vec::new()), false).unwrap_or_else(|error| panic!("{error:?}"));
    ok(&mut session, "node.add", json!({ "file": "f.lcf", "call": "bytes.from_u16" }));
    assert!(text(&memory, "/p/f.lcf").contains("    data, _ = bytes.from_u16()\n"), "{}", text(&memory, "/p/f.lcf"));
}

#[test]
fn an_outline_is_written_and_null_removes_it() {
    // commands.md, `block.set_outline`: the items in canonical form; `null` removes the section.
    let (mut session, memory) = session();
    ok(&mut session, "block.set_outline", json!({ "file": "f.lcf", "outline": "template d" }));
    assert_eq!(text(&memory, "/p/f.lcf"), format!("{START}\noutline {{\n    template d\n}}\n"));
    ok(&mut session, "block.set_outline", json!({ "file": "f.lcf", "outline": null }));
    assert_eq!(text(&memory, "/p/f.lcf"), START);
    // Items that close the section and open another one are refused.
    let agent = Author::Agent("test".to_owned());
    let error = refused(
        &mut session,
        &agent,
        "block.set_outline",
        json!({ "file": "f.lcf", "outline": "template d\n}\n\nicon 32 {\n    line (2, 2) (30, 30)" }),
    );
    assert_eq!((error.code, error.parameter.as_deref()), ("params.invalid", Some("outline")));
    assert_eq!(text(&memory, "/p/f.lcf"), START);
}

// `library.set_mask` (commands.md 6.7, spec 11.3). The texts expected are written by hand from them.

#[test]
fn set_mask_makes_the_manifest_with_the_table_alone() {
    let (mut session, memory) = session();
    assert_eq!(memory.get("/p/labcraft.toml"), None);
    let result = ok(&mut session, "library.set_mask", json!({ "folder": ".", "band": "DMM", "color": "#2F6F9F" }));
    let manifest = "[mask]\nband = \"DMM\"\ncolor = \"#2F6F9F\"\n";
    assert_eq!(text(&memory, "/p/labcraft.toml"), manifest);
    assert_eq!(result["step"], 1);
    assert_eq!(result["files"], json!([{ "file": "labcraft.toml", "revision": sha(manifest) }]));
    // In a sub-folder, the manifest is that folder's.
    memory.put("/p/sub/h.lcf", START);
    ok(&mut session, "library.set_mask", json!({ "folder": "sub", "band_place": "above" }));
    assert_eq!(text(&memory, "/p/sub/labcraft.toml"), "[mask]\nband_place = \"above\"\n");
}

#[test]
fn set_mask_writes_a_field_into_an_existing_manifest_and_keeps_the_rest() {
    let (mut session, memory) = session();
    memory.put("/p/labcraft.toml", "# the driver\nname = \"p\"\n\n[mask]\nband = \"D\" # short\n");
    ok(&mut session, "library.set_mask", json!({ "folder": ".", "band": "DMM", "stroke": "#1e4f73" }));
    // The field written again keeps its place and its comment, the new one goes at the end; the case of a colour is kept.
    assert_eq!(text(&memory, "/p/labcraft.toml"), "# the driver\nname = \"p\"\n\n[mask]\nband = \"DMM\" # short\nstroke = \"#1e4f73\"\n");
}

#[test]
fn set_mask_writes_an_outline_as_a_literal_multi_line_string() {
    let (mut session, memory) = session();
    ok(&mut session, "library.set_mask", json!({ "folder": ".", "outline": "template d" }));
    assert_eq!(text(&memory, "/p/labcraft.toml"), "[mask]\noutline = '''\ntemplate d\n'''\n");
}

#[test]
fn set_mask_removes_a_field_with_null_and_the_table_when_it_is_empty() {
    let (mut session, memory) = session();
    memory.put("/p/labcraft.toml", "name = \"p\"\n\n[mask]\nband = \"D\"\ncolor = \"#2F6F9F\"\n");
    ok(&mut session, "library.set_mask", json!({ "folder": ".", "band": null }));
    assert_eq!(text(&memory, "/p/labcraft.toml"), "name = \"p\"\n\n[mask]\ncolor = \"#2F6F9F\"\n");
    ok(&mut session, "library.set_mask", json!({ "folder": ".", "color": null }));
    assert_eq!(text(&memory, "/p/labcraft.toml"), "name = \"p\"\n");
}

/// commands.md 6.7: a manifest with nothing left in it goes; one that the command made goes with its last field.
#[test]
fn set_mask_removes_a_manifest_left_empty() {
    let (mut session, memory) = session();
    ok(&mut session, "library.set_mask", json!({ "folder": ".", "band": "A" }));
    assert_eq!(text(&memory, "/p/labcraft.toml"), "[mask]\nband = \"A\"\n");
    ok(&mut session, "library.set_mask", json!({ "folder": ".", "band": null }));
    assert_eq!(memory.get("/p/labcraft.toml"), None);
}

/// commands.md 6.7: the rest of the manifest is kept as written, its byte order mark too; a `band` with a control
/// character is refused (spec 11.3).
#[test]
fn set_mask_keeps_a_byte_order_mark_and_refuses_a_control_character_in_band() {
    let (mut session, memory) = session();
    memory.put("/p/labcraft.toml", "\u{feff}name = \"p\"\n");
    ok(&mut session, "library.set_mask", json!({ "folder": ".", "band": "A" }));
    assert_eq!(text(&memory, "/p/labcraft.toml"), "\u{feff}name = \"p\"\n\n[mask]\nband = \"A\"\n");
    for band in ["a\u{7}", "a\u{85}b", "a\u{2028}b"] {
        let error = refused(&mut session, &Author::User, "library.set_mask", json!({ "folder": ".", "band": band }));
        assert_eq!(error.code, "params.invalid", "{band:?}");
    }
}

/// commands.md 6.7: `library.set_mask` writes a manifest, and does not go in a batch.
#[test]
fn set_mask_does_not_go_in_a_batch() {
    let (mut session, memory) = session();
    let batch = json!({ "commands": [{ "id": "library.set_mask", "params": { "folder": ".", "band": "A" } }] });
    assert_eq!(refused(&mut session, &Author::User, "commands.batch", batch).code, "params.invalid");
    assert_eq!(memory.get("/p/labcraft.toml"), None);
}

/// A control character other than a tab cannot be in a literal TOML string (TOML 1.0, "Literal strings"): refused as
/// a value, not as a manifest that cannot be written.
#[test]
fn set_mask_refuses_an_outline_that_a_literal_string_cannot_hold() {
    let (mut session, _) = session();
    let params = json!({ "folder": ".", "outline": "template d // \u{7}" });
    assert_eq!(refused(&mut session, &Author::User, "library.set_mask", params).code, "params.invalid");
}

#[test]
fn set_mask_refuses_what_the_mask_refuses() {
    let (mut session, memory) = session();
    let long = "x".repeat(65);
    let cases = [
        json!({ "folder": ".", "color": "#12345" }),
        json!({ "folder": ".", "fill": "red" }),
        json!({ "folder": ".", "stroke": 5 }),
        json!({ "folder": ".", "band_place": "left" }),
        json!({ "folder": ".", "band": "" }),
        json!({ "folder": ".", "band": long }),
        json!({ "folder": ".", "band": "a\nb" }),
        // What `block.set_outline` refuses: two templates, a `round` outside 1 to 16, text that does not read.
        json!({ "folder": ".", "outline": "template d\ntemplate block" }),
        json!({ "folder": ".", "outline": "path (0, 0) (w, 0) round 17 (w, h)" }),
        json!({ "folder": ".", "outline": "nonsense here" }),
    ];
    for params in cases {
        let error = refused(&mut session, &Author::User, "library.set_mask", params.clone());
        assert_eq!(error.code, "params.invalid", "{params}: {}", error.message);
        assert_eq!(memory.get("/p/labcraft.toml"), None, "nothing written for {params}");
    }
    // The same outline is refused by `block.set_outline` too.
    let error = refused(&mut session, &Author::User, "block.set_outline", json!({ "file": "f.lcf", "outline": "template d\ntemplate block" }));
    assert_eq!(error.code, "params.invalid");
}

#[test]
fn set_mask_honours_an_expected_revision_naming_the_manifest() {
    let (mut session, memory) = session();
    let manifest = "name = \"p\"\n";
    memory.put("/p/labcraft.toml", manifest);
    let error = refused(
        &mut session,
        &Author::User,
        "library.set_mask",
        json!({ "folder": ".", "band": "A", "expected_revision": { "labcraft.toml": sha("old") } }),
    );
    assert_eq!(error.code, "file.stale");
    assert!(error.message.contains(&sha(manifest)), "{}", error.message);
    assert_eq!(text(&memory, "/p/labcraft.toml"), manifest);
    ok(&mut session, "library.set_mask", json!({ "folder": ".", "band": "A", "expected_revision": { "labcraft.toml": sha(manifest) } }));
    assert_eq!(text(&memory, "/p/labcraft.toml"), "name = \"p\"\n\n[mask]\nband = \"A\"\n");
}

#[test]
fn set_mask_needs_a_folder_that_exists_and_inside_the_project() {
    let (mut session, _) = session();
    let missing = refused(&mut session, &Author::User, "library.set_mask", json!({ "folder": "nowhere", "band": "A" }));
    assert_eq!((missing.code, missing.parameter.as_deref()), ("path.not_found", Some("folder")));
    let outside = refused(&mut session, &Author::User, "library.set_mask", json!({ "folder": "../q", "band": "A" }));
    assert_eq!(outside.code, "path.outside_project");
}

#[test]
fn undo_of_set_mask_restores_the_manifest_and_redo_writes_it_again() {
    let (mut session, memory) = session();
    let before = "name = \"p\"\n";
    memory.put("/p/labcraft.toml", before);
    ok(&mut session, "library.set_mask", json!({ "folder": ".", "band": "A" }));
    let after = text(&memory, "/p/labcraft.toml");
    assert_eq!(after, "name = \"p\"\n\n[mask]\nband = \"A\"\n");
    ok(&mut session, "history.undo", json!({}));
    assert_eq!(text(&memory, "/p/labcraft.toml"), before);
    ok(&mut session, "history.redo", json!({}));
    assert_eq!(text(&memory, "/p/labcraft.toml"), after);
    // Made from nothing, undo removes the manifest again.
    let (mut session, memory) = self::session();
    ok(&mut session, "library.set_mask", json!({ "folder": ".", "band": "A" }));
    ok(&mut session, "history.undo", json!({}));
    assert_eq!(memory.get("/p/labcraft.toml"), None);
}

#[test]
fn another_command_still_refuses_the_manifest_as_a_file() {
    let (mut session, memory) = session();
    memory.put("/p/labcraft.toml", "name = \"p\"\n");
    let error = refused(&mut session, &Author::User, "node.add", json!({ "file": "labcraft.toml", "call": "core.add" }));
    assert_eq!(error.code, "path.outside_project");
    // Nor can `expected_revision` of another command name it.
    let error = refused(
        &mut session,
        &Author::User,
        "node.add",
        json!({ "file": "f.lcf", "call": "core.add", "expected_revision": { "labcraft.toml": sha("name = \"p\"\n") } }),
    );
    assert_eq!(error.code, "path.outside_project");
    assert_eq!(text(&memory, "/p/labcraft.toml"), "name = \"p\"\n");
}
