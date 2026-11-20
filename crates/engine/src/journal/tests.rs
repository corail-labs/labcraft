//! The journal through the session (architecture 12.5). The lines expected are written by hand; a revision is worked out
//! with `sha2`, not with `revision`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use labcraft_types::Limits;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{CLI_FILE, KEPT_FILES, MAX_FILE, MAX_PARAMS};
use crate::history::Author;
use crate::{Config, Memory, ReadOnly, Session, Shipped, Still, Workspace};

const START: &str = "labcraft 1\n\nfn f(x: f64) -> (y: f64) {\n    y = x\n}\n";
const ADDED: &str = "labcraft 1\n\nuse core\n\nfn f(x: f64) -> (y: f64) {\n    sum = core.add()\n    y = x\n}\n";

fn config(process: u32) -> Config {
    Config {
        root: PathBuf::from("/p"),
        lib_paths: Vec::new(),
        limits: Limits::default(),
        shipped: Shipped::default(),
        natives: Vec::new(),
        clock: Arc::new(Still),
        process,
    }
}

fn session(memory: &Arc<Memory>, process: u32) -> Session {
    let ws: Arc<dyn Workspace> = memory.clone();
    Session::new(ws, config(process), false).unwrap_or_else(|error| panic!("{error:?}"))
}

fn project() -> Arc<Memory> {
    let memory = Arc::new(Memory::new());
    memory.put("/p/f.lcf", START);
    memory
}

fn sha(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    format!("sha256:{}", digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>())
}

/// The names in `.labcraft/journal/`, sorted.
fn names(memory: &Memory) -> Vec<String> {
    let mut names = Vec::new();
    let _ = memory.read_dir(Path::new("/p/.labcraft/journal"), &mut |entry| {
        if let Ok(entry) = entry {
            names.push(entry.name.to_string_lossy().into_owned());
        }
    });
    names.sort();
    names
}

/// The lines of the one file of a session of process 7.
fn lines(memory: &Memory) -> Vec<Value> {
    let file = names(memory).into_iter().find(|name| name.starts_with("7-")).unwrap_or_default();
    let bytes = memory.get(format!("/p/.labcraft/journal/{file}")).unwrap_or_default();
    String::from_utf8_lossy(&bytes).lines().map(|line| serde_json::from_str(line).unwrap_or(Value::Null)).collect()
}

#[test]
fn every_command_is_a_line_with_its_author_its_result_and_its_files() {
    let memory = project();
    let mut session = session(&memory, 7);
    let agent = Author::Agent("claude".to_owned());
    session.execute_as(&agent, "node.add", &json!({ "file": "f.lcf", "call": "core.add" })).unwrap_or_else(|error| panic!("{error:?}"));
    let _ = session.execute("node.add", &json!({ "file": "none.lcf", "call": "core.add" }));
    // A read is no line (decision of the owner, 2026-11-16).
    let _ = session.execute("functions.list", &json!({}));
    let lines = lines(&memory);
    assert_eq!(
        lines[0],
        json!({
            "time": "1970-01-01T00:00:00.000000000Z",
            "author": "claude",
            "id": "node.add",
            "params": { "file": "f.lcf", "call": "core.add" },
            "result": "ok",
            "files": [{ "file": "f.lcf", "revision": sha(ADDED) }],
        })
    );
    assert_eq!(
        (lines[1]["author"].clone(), lines[1]["result"].clone(), lines[1]["files"].clone()),
        (json!("user"), json!("path.not_found"), json!([]))
    );
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(
        memory.get("/p/.labcraft/.gitignore").as_deref(),
        Some(crate::runs::LABCRAFT_GITIGNORE),
        "made with `.labcraft`: git sees only it and `config.toml`"
    );
    assert!(names(&memory).contains(&".7.lock".to_owned()), "the lock of the process, which tells another that it runs: {:?}", names(&memory));
}

#[test]
fn an_external_write_is_a_line_of_its_own() {
    let memory = project();
    let mut session = session(&memory, 7);
    let _ = session.execute("file.write", &json!({ "path": "g.lcf", "text": "g", "check": false }));
    memory.put("/p/f.lcf", "outside");
    let _ = session.execute("file.write", &json!({ "path": "g.lcf", "text": "g", "check": false }));
    let lines = lines(&memory);
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(
        lines[1],
        json!({
            "time": "1970-01-01T00:00:00.000000000Z",
            "author": "external",
            "id": null,
            "params": null,
            "result": "ok",
            "files": [{ "file": "f.lcf", "revision": sha("outside") }],
        })
    );
}

#[test]
fn a_link_in_place_of_the_journal_is_never_written_through() {
    let memory = project();
    memory.put("/outside/secret.txt", "s");
    memory.put_folder("/p/.labcraft/journal");
    memory.put_link("/p/.labcraft/journal/cli.jsonl", "/outside/secret.txt");
    let mut session = session(&memory, 7);
    session.for_command_line();
    assert!(session.execute("file.write", &json!({ "path": "g.lcf", "text": "g", "check": false })).is_ok());
    assert_eq!(memory.get("/outside/secret.txt").as_deref(), Some(&b"s"[..]));
}

#[test]
fn a_line_names_at_most_1000_files_and_counts_them_all() {
    let files: Vec<(String, Option<String>)> = (0..1_001).map(|index| (format!("f{index}.lcf"), None)).collect();
    let entry = super::Entry { time_ns: 0, author: &Author::User, id: Some("file.format"), params: &json!({}), outcome: Ok(()), files: &files };
    let line: Value = serde_json::from_str(&super::line_of(&entry)).unwrap_or(Value::Null);
    assert_eq!(line["files"].as_array().map(Vec::len), Some(super::MAX_FILES));
    assert_eq!((line["files"][999]["file"].clone(), line["files_cut"].clone()), (json!("f999.lcf"), json!(1_001)));
}

#[test]
fn commands_execute_writes_the_line_of_the_command_it_runs() {
    let memory = project();
    let mut session = session(&memory, 7);
    let _ = session.execute("commands.execute", &json!({ "id": "file.write", "params": { "path": "g.lcf", "text": "g", "check": false } }));
    // What it runs that reads is no line.
    let _ = session.execute("commands.execute", &json!({ "id": "functions.list", "params": {} }));
    let _ = session.execute("commands.execute", &json!({ "id": "commands.batch" }));
    let lines = lines(&memory);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!((lines[0]["id"].clone(), lines[0]["params"].clone()), (json!("file.write"), json!({ "path": "g.lcf", "text": "g", "check": false })));
    assert_eq!(
        (lines[1]["id"].clone(), lines[1]["result"].clone()),
        (json!("commands.execute"), json!("params.invalid")),
        "refused before it ran one"
    );
}

#[test]
fn parameters_past_64_kib_are_cut_and_the_cut_is_said() {
    let memory = project();
    let mut session = session(&memory, 7);
    let long = "é".repeat(MAX_PARAMS);
    let _ = session.execute("file.write", &json!({ "path": long }));
    let line = &lines(&memory)[0];
    let text = line["params"].as_str().unwrap_or_default();
    // `{"path":"` is 9 bytes, then two bytes for each `é`: the cut falls between the two bytes of one, and goes back a byte.
    assert_eq!(text.len(), MAX_PARAMS - 1);
    assert!(text.starts_with("{\"path\":\"éé"), "{}", text.get(..20).unwrap_or_default());
    assert_eq!(line["params_cut"], json!(9 + 2 * MAX_PARAMS + 2));
}

#[test]
fn the_command_line_writes_cli_jsonl_and_cuts_it_to_its_newest_half() {
    let memory = project();
    // Lines of 1 MiB less one byte, and their ends: 17 of them, over 16 MiB.
    let line = format!("{}\n", "x".repeat((1 << 20) - 1));
    memory.put("/p/.labcraft/journal/cli.jsonl", line.repeat(17));
    let mut session = session(&memory, 7);
    session.for_command_line();
    let _ = session.execute("file.write", &json!({ "path": "g.lcf", "text": "g", "check": false }));
    let bytes = memory.get(format!("/p/.labcraft/journal/{CLI_FILE}")).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let kept: Vec<&str> = text.lines().collect();
    // Half of 17 MiB falls inside the ninth line, which goes: the eight after it stay, then the new line.
    assert_eq!(kept.len(), 9);
    assert!(kept[..8].iter().all(|kept| kept.len() == (1 << 20) - 1));
    assert!(kept[8].contains("\"file.write\""), "{}", kept[8]);
    assert!(bytes.len() as u64 <= MAX_FILE);
}

#[test]
fn the_newest_files_are_kept_and_those_of_a_process_that_runs() {
    let memory = project();
    memory.put_folder("/p/.labcraft/journal");
    // Written one after the other: the clock of `Memory` makes each newer than the one before.
    for process in 100..100 + KEPT_FILES as u32 + 5 {
        memory.put(format!("/p/.labcraft/journal/{process}-1.jsonl"), "{}\n");
    }
    memory.put("/p/.labcraft/journal/.100.lock", "");
    let running = memory.try_lock(Path::new("/p/.labcraft/journal/.100.lock")).ok().flatten();
    memory.put("/p/.labcraft/journal/.101.lock", "");
    let mut session = session(&memory, 7);
    let _ = session.execute("file.write", &json!({ "path": "g.lcf", "text": "g", "check": false }));
    let names = names(&memory);
    let own = names.iter().filter(|name| name.starts_with("7-")).count();
    // The newest 19 and that of this session: 20; the oldest, of process 100, runs still, and its lock stays.
    let kept: Vec<&String> = names.iter().filter(|name| name.ends_with(".jsonl") && !name.starts_with("7-")).collect();
    assert_eq!(own, 1);
    assert_eq!(kept.len(), KEPT_FILES - 1 + 1, "{names:?}");
    assert!(kept.contains(&&"100-1.jsonl".to_owned()));
    assert!(!kept.contains(&&"101-1.jsonl".to_owned()));
    assert!(kept.contains(&&"106-1.jsonl".to_owned()), "the 19 newest are 106 to 124");
    assert!(names.contains(&".100.lock".to_owned()));
    assert!(!names.contains(&".101.lock".to_owned()), "the lock of a process that no longer runs, with no file left: {names:?}");
    drop(running);
}

#[test]
fn a_journal_that_cannot_be_written_never_fails_a_command() {
    let memory = project();
    let ws: Arc<dyn Workspace> = Arc::new(ReadOnly(memory.clone()));
    let mut session = Session::new(ws, config(7), false).unwrap_or_else(|error| panic!("{error:?}"));
    assert!(session.execute("functions.list", &json!({})).is_ok());
    assert!(session.execute("functions.list", &json!({})).is_ok());
    assert_eq!(memory.get("/p/.labcraft/.gitignore"), None);
}

/// What a window reads to draw itself is no line; what changes something is (decision of the owner, 2026-11-16).
#[test]
fn a_read_is_no_line_and_what_changes_is() {
    let memory = project();
    let mut session = session(&memory, 7);
    session.with_window();
    let _ = session.execute("view.open", &json!({ "folder": "." }));
    for _ in 0..3 {
        let _ = session.execute("history.list", &json!({ "limit": 1 }));
        let _ = session.execute("functions.list", &json!({}));
        let _ = session.execute("file.read", &json!({ "path": "f.lcf" }));
        let _ = session.execute("view.get_state", &json!({}));
        let _ = session.execute("commands.execute", &json!({ "id": "file.read", "params": { "path": "f.lcf" } }));
    }
    let _ = session.execute("node.add", &json!({ "file": "f.lcf", "call": "core.add" }));
    let _ = session.execute("history.undo", &json!({}));
    let ids: Vec<Value> = lines(&memory).iter().map(|line| line["id"].clone()).collect();
    assert_eq!(ids, [json!("node.add"), json!("history.undo")]);
}
