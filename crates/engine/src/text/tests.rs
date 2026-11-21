//! The commands of the `Text` panel (commands.md 9.1; decisions of the owner, 2026-11-21). The texts expected are written
//! by hand; a revision is worked out with `sha2`, not with `revision`.

use std::path::PathBuf;
use std::sync::Arc;

use labcraft_types::Limits;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::history::Author;
use crate::{Config, Memory, Session, Shipped, Still, Workspace};

const START: &str = "labcraft 1\n\nfn f(x: f64) -> (y: f64) {\n    y = x\n}\n";
/// Parses, with an error of `check` (`z` is bound by nothing): it is written all the same.
const WRONG: &str = "labcraft 1\n\nfn f(x: f64) -> (y: f64) {\n    y = z\n}\n";
/// Does not parse: the body is not closed.
const BROKEN: &str = "labcraft 1\n\nfn f(x: f64) -> (y: f64) {\n    y = x\n";

fn session() -> (Session, Arc<Memory>) {
    let memory = Arc::new(Memory::new());
    memory.put("/p/f.lcf", START);
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
    let mut session = Session::new(ws, config, false).unwrap_or_else(|error| panic!("{error:?}"));
    session.with_window();
    (session, memory)
}

#[track_caller]
fn ok(session: &mut Session, id: &str, params: Value) -> Value {
    session.execute(id, &params).unwrap_or_else(|error| panic!("{id}: {}", error.to_json()))
}

#[track_caller]
fn code(session: &mut Session, id: &str, params: Value) -> String {
    match session.execute(id, &params) {
        Ok(value) => panic!("{id}: an error expected, got {value}"),
        Err(error) => error.code.to_string(),
    }
}

fn text(memory: &Memory, path: &str) -> Option<String> {
    memory.get(path).map(|bytes| String::from_utf8(bytes).unwrap_or_default())
}

fn sha(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    format!("sha256:{}", digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>())
}

fn steps(session: &mut Session) -> usize {
    ok(session, "history.list", json!({}))["steps"].as_array().map_or(0, Vec::len)
}

#[test]
fn a_text_that_parses_is_written_as_one_step_whatever_its_errors() {
    let (mut session, memory) = session();
    let saved = ok(&mut session, "text.save", json!({ "path": "f.lcf", "text": WRONG.replace('\n', "\r\n") }));
    assert_eq!(saved, json!({ "written": true, "revision": sha(WRONG) }));
    assert_eq!(text(&memory, "/p/f.lcf").as_deref(), Some(WRONG), "CR LF became LF");
    assert_eq!(steps(&mut session), 1);
    assert_eq!(ok(&mut session, "history.list", json!({}))["steps"][0]["label"], "Edit Text");
    ok(&mut session, "history.undo", json!({}));
    assert_eq!(text(&memory, "/p/f.lcf").as_deref(), Some(START));
}

#[test]
fn a_text_that_does_not_parse_is_kept_in_the_recovery_folder_and_the_file_keeps_its_text() {
    let (mut session, memory) = session();
    let saved = ok(&mut session, "text.save", json!({ "path": "f.lcf", "text": BROKEN }));
    assert_eq!(saved["written"], false);
    assert_eq!(saved["kept"], true);
    assert_eq!(saved["error"]["file"], "f.lcf");
    assert_eq!(saved["error"]["severity"], "error");
    assert!(saved["error"]["code"].as_str().is_some_and(|code| code.starts_with("LC1")), "{saved}");
    assert_eq!(text(&memory, "/p/f.lcf").as_deref(), Some(START));
    assert_eq!(text(&memory, "/p/.labcraft/recovery/f.lcf").as_deref(), Some(BROKEN));
    // git sees only the settings of `.labcraft/`.
    assert_eq!(text(&memory, "/p/.labcraft/.gitignore").as_deref(), Some("*\n!.gitignore\n!config.toml\n"));
    assert_eq!(steps(&mut session), 0, "nothing of the project changed");
    // Once it parses again, the copy goes.
    ok(&mut session, "text.save", json!({ "path": "f.lcf", "text": WRONG }));
    assert_eq!(text(&memory, "/p/.labcraft/recovery/f.lcf"), None);
}

#[test]
fn a_file_changed_since_its_revision_is_not_written_and_the_text_is_kept() {
    let (mut session, memory) = session();
    memory.put("/p/f.lcf", WRONG);
    let saved = ok(&mut session, "text.save", json!({ "path": "f.lcf", "text": START.replace('x', "w"), "expected_revision": sha(START) }));
    assert_eq!(saved, json!({ "written": false, "kept": true, "stale": true, "revision": sha(WRONG) }));
    assert_eq!(text(&memory, "/p/f.lcf").as_deref(), Some(WRONG));
    assert_eq!(text(&memory, "/p/.labcraft/recovery/f.lcf"), Some(START.replace('x', "w")));
    // "Keep my text": written over, with the revision it has now or with none.
    let kept = ok(&mut session, "text.save", json!({ "path": "f.lcf", "text": START, "expected_revision": sha(WRONG) }));
    assert_eq!(kept["written"], true);
    assert_eq!(text(&memory, "/p/f.lcf").as_deref(), Some(START));
}

/// `keep` only keeps the text, even one that parses on the revision of the file: "Show the file" writes nothing.
#[test]
fn keep_writes_nothing_in_the_project() {
    let (mut session, memory) = session();
    let saved = ok(&mut session, "text.save", json!({ "path": "f.lcf", "text": WRONG, "expected_revision": sha(START), "keep": true }));
    assert_eq!(saved, json!({ "written": false, "kept": true }));
    assert_eq!(text(&memory, "/p/f.lcf").as_deref(), Some(START));
    assert_eq!(text(&memory, "/p/.labcraft/recovery/f.lcf").as_deref(), Some(WRONG));
    assert_eq!(steps(&mut session), 0);
}

#[test]
fn the_recovered_texts_are_those_newer_than_their_file() {
    let (mut session, memory) = session();
    memory.put("/p/lib/g.lct", "labcraft 1\n\ntype G = f64\n");
    memory.put_modified("/p/f.lcf", START, 10);
    memory.put_modified("/p/.labcraft/recovery/f.lcf", BROKEN, 20);
    memory.put_modified("/p/lib/g.lct", "labcraft 1\n\ntype G = f64\n", 30);
    memory.put_modified("/p/.labcraft/recovery/lib/g.lct", "labcraft 1\n\ntype G =\n", 20);
    // A copy whose file is gone, and a file that is no program, are not offered.
    memory.put_modified("/p/.labcraft/recovery/gone.lcf", BROKEN, 40);
    memory.put_modified("/p/.labcraft/recovery/notes.toml", "x", 40);
    let found = ok(&mut session, "text.recovered", json!({}));
    assert_eq!(found, json!({ "texts": [{ "path": "f.lcf", "text": BROKEN, "modified": 20 }], "truncated": false }));
    assert_eq!(ok(&mut session, "text.discard", json!({ "path": "f.lcf" })), json!({ "discarded": true }));
    assert_eq!(ok(&mut session, "text.discard", json!({ "path": "f.lcf" })), json!({ "discarded": false }));
    assert_eq!(ok(&mut session, "text.recovered", json!({})), json!({ "texts": [], "truncated": false }));
}

#[test]
fn the_panel_edits_programs_and_types_that_exist() {
    let (mut session, _) = session();
    assert_eq!(code(&mut session, "text.save", json!({ "path": "notes.toml", "text": "" })), "path.invalid");
    assert_eq!(code(&mut session, "text.save", json!({ "path": "new.lcf", "text": START })), "path.not_found");
    assert_eq!(code(&mut session, "text.save", json!({ "path": "../f.lcf", "text": START })), "path.outside_project");
    assert_eq!(code(&mut session, "text.discard", json!({ "path": "notes.toml" })), "path.invalid");
    let (mut session, memory) = self::session();
    memory.put("/p/README.md", "x");
    assert_eq!(code(&mut session, "text.save", json!({ "path": "README.md", "text": START })), "path.invalid");
    assert_eq!(text(&memory, "/p/README.md").as_deref(), Some("x"));
}

#[test]
fn a_recovery_folder_behind_a_link_is_neither_written_nor_read() {
    let (mut session, memory) = session();
    memory.put_folder("/elsewhere");
    memory.put_link("/p/.labcraft/recovery", "/elsewhere");
    let saved = ok(&mut session, "text.save", json!({ "path": "f.lcf", "text": BROKEN }));
    assert_eq!(saved["kept"], false);
    assert_eq!(text(&memory, "/elsewhere/f.lcf"), None);
    // Nor are the folders of a file deeper down made through it.
    memory.put("/p/lib/g.lcf", START);
    let saved = ok(&mut session, "text.save", json!({ "path": "lib/g.lcf", "text": BROKEN }));
    assert_eq!(saved["kept"], false);
    assert!(memory.get("/elsewhere/lib/g.lcf").is_none());
    assert!(memory.metadata(std::path::Path::new("/elsewhere/lib")).is_err(), "no folder made outside the project");
    memory.put("/elsewhere/f.lcf", BROKEN);
    assert_eq!(ok(&mut session, "text.recovered", json!({})), json!({ "texts": [], "truncated": false }));
    assert_eq!(ok(&mut session, "text.discard", json!({ "path": "f.lcf" })), json!({ "discarded": false }));
    assert_eq!(text(&memory, "/elsewhere/f.lcf").as_deref(), Some(BROKEN));
}

#[test]
fn only_the_window_of_the_user_calls_them() {
    let (mut headless, _) = session();
    headless.has_window = false;
    assert_eq!(code(&mut headless, "text.recovered", json!({})), "command.not_enabled");
    let (mut session, _) = session();
    let agent = Author::Agent("claude-code".to_owned());
    for (id, params) in
        [("text.save", json!({ "path": "f.lcf", "text": START })), ("text.recovered", json!({})), ("text.discard", json!({ "path": "f.lcf" }))]
    {
        let refused = session.execute_as(&agent, id, &params).map(|_| ()).unwrap_err();
        assert_eq!(refused.code, "agent.not_allowed", "{id}");
    }
}

/// `text.recovered` stops at 1,000 entries of the folder, and says so (code review of 7a).
#[test]
fn the_recovered_texts_stop_at_their_bound() {
    let (mut session, memory) = session();
    for at in 0..1_001 {
        memory.put(format!("/p/.labcraft/recovery/n{at:04}.lcf"), "x");
    }
    let found = ok(&mut session, "text.recovered", json!({}));
    assert_eq!(found["truncated"], true);
    assert_eq!(found["texts"], json!([]), "no file of the project has these names");
}
