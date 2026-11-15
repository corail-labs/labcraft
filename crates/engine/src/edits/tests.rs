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

#[test]
fn a_file_changed_since_its_step_is_not_undone() {
    let (mut session, memory) = session();
    ok(&mut session, "node.add", json!({ "file": "f.lcf", "call": "core.add" }));
    memory.put("/p/f.lcf", "labcraft 1\n\nfn f() -> () {}\n");
    assert_eq!(session.execute("history.undo", &json!({})).map_err(|error| error.code), Err("file.stale"));
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
fn without_history_there_is_no_step_and_no_undo() {
    let (mut session, _) = session();
    session.without_history();
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
    assert_eq!(refused(&mut session, &user, "node.add", json!({ "file": "f.lcf", "call": "core.add", "into": "x -> y" })).code, "params.invalid");
    assert_eq!(refused(&mut session, &user, "node.add", json!({ "file": "f.lcf", "call": "core.add", "at": [1] })).code, "params.invalid");
    assert_eq!(refused(&mut session, &user, "node.add", json!({ "file": "f.lcf", "call": "visa.nothing" })).code, "edit.unknown");
}
