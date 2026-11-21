//! Scenario 4 of M1 (`docs/acceptance-m1.md`): editing at random never breaks. A project in memory from `examples/`, then
//! a sequence of at most 500 steps read from the input: editing commands with parameters drawn from it (names that exist
//! and names that do not, positions, files, revisions), batches, `file.write`, undo and redo, by the user or an agent, and
//! external writes of random bytes to the files of the project, a new one among them, on the clock of the tests
//! (`engine::Still`), so that the merging of external writes is the same at each run.
//!
//! Never a panic, and never `internal`. A command that fails writes nothing. After each editing command, every file that
//! it wrote parses (commands.md 9.1); undo just after it gives back the bytes before it, and redo those after it. After the
//! whole sequence, as many undo give back the bytes of the start, external writes included; and with no external write,
//! no step is signed `external`: a write of the session is never taken for one.
//!
//! The input is read one byte at a time; past its end every byte is 0. It runs on the stack of a host
//! (`format::STACK_SIZE`), as the program does.
#![no_main]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use labcraft_engine::workspace::Kind;
use labcraft_engine::{Author, CommandError, Config, Memory, Session, Shipped, ShippedFile, ShippedLibrary, Still, Workspace};
use labcraft_format::STACK_SIZE;
use labcraft_types::{Interner, Limits};
use libfuzzer_sys::fuzz_target;
use serde_json::{Value as Json, json};

/// At most this many steps *(proposed)*, below the bound of the history.
const MAX_STEPS: usize = 500;

const PROJECT: &[(&str, &str)] = &[
    ("Verdict.lct", include_str!("../../examples/Verdict.lct")),
    ("check_voltage.lcf", include_str!("../../examples/check_voltage.lcf")),
    ("settle.lcf", include_str!("../../examples/settle.lcf")),
    ("projectile.lcf", include_str!("../../examples/projectile.lcf")),
];

/// The files a command names: those of the project, and some that are not.
const FILES: &[&str] = &["check_voltage.lcf", "settle.lcf", "projectile.lcf", "Verdict.lct", "none.lcf", "../out.lcf", ""];
const CALLS: &[&str] =
    &["core.add", "core.mul", "core.ge", "string.format", "visa.open", "settle", "Verdict.Pass", "check_voltage", "nothing.here", "core"];
const CONSTANTS: &[&str] = &["1", "240 V", "\"MEAS?\"", "Verdict.Pass", "[1, 2]", "x", "?", "1 +", ""];
const MADE_UP: &[&str] = &["nothing", "x", "y", "_", "?", "", "a.b.c"];

fn shipped() -> Shipped {
    let files = |files: &'static [labcraft_libs::File]| files.iter().map(|file| ShippedFile { name: file.name, bytes: file.bytes }).collect();
    Shipped::new(
        labcraft_libs::LIBRARIES
            .iter()
            .map(|library| ShippedLibrary {
                name: library.name,
                manifest: library.manifest,
                files: files(library.files),
                simulations: files(library.simulations),
            })
            .collect(),
    )
}

/// The input, a byte at a time; 0 past its end.
struct Input<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Input<'_> {
    fn byte(&mut self) -> u8 {
        let byte = self.bytes.get(self.at).copied().unwrap_or(0);
        self.at += 1;
        byte
    }

    fn done(&self) -> bool {
        self.at >= self.bytes.len()
    }

    fn pick<'s>(&mut self, from: &[&'s str]) -> &'s str {
        from.get(usize::from(self.byte()) % from.len().max(1)).copied().unwrap_or_default()
    }

    fn flag(&mut self) -> bool {
        self.byte() & 1 == 1
    }
}

/// Every file of the project but `.labcraft/`, the journal's, by its path.
fn snapshot(memory: &Memory) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut folders = vec![Path::new("/p").to_path_buf()];
    while let Some(folder) = folders.pop() {
        let _ = memory.read_dir(&folder, &mut |entry| {
            let Ok(entry) = entry else { return };
            let path = folder.join(&entry.name);
            match entry.kind {
                Ok(Kind::Folder) if entry.name != ".labcraft" => folders.push(path),
                Ok(Kind::File) => {
                    files.insert(path.to_string_lossy().replace('\\', "/"), memory.get(&path).unwrap_or_default());
                }
                _ => {}
            }
        });
    }
    files
}

/// The names of a file of the project, which a command may name: its words, as the text writes them.
fn names_of(memory: &Memory, file: &str) -> Vec<String> {
    let text = String::from_utf8_lossy(&memory.get(format!("/p/{file}")).unwrap_or_default()).into_owned();
    let mut names: BTreeSet<String> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
        .filter(|word| !word.is_empty() && word.len() < 40)
        .map(str::to_owned)
        .collect();
    names.extend(MADE_UP.iter().map(|name| (*name).to_owned()));
    names.into_iter().collect()
}

fn name(input: &mut Input<'_>, names: &[String]) -> String {
    names.get(usize::from(input.byte()) % names.len().max(1)).cloned().unwrap_or_default()
}

/// An input of a node, `node.input` or `node`.
fn input_of(input: &mut Input<'_>, names: &[String]) -> String {
    let node = name(input, names);
    if input.flag() { format!("{node}.{}", name(input, names)) } else { node }
}

/// The id and the parameters of an editing command, drawn from the input.
fn edit(input: &mut Input<'_>, memory: &Memory) -> (&'static str, Json) {
    let file = input.pick(FILES);
    let names = names_of(memory, file);
    let (id, mut params) = match input.byte() % 5 {
        0 => {
            let mut params = json!({ "file": file, "call": input.pick(CALLS) });
            if input.flag() {
                params["at"] = json!([i64::from(input.byte()) * 8 - 512, i64::from(input.byte()) * 8 - 512]);
            }
            ("node.add", params)
        }
        1 => ("node.delete", json!({ "file": file, "nodes": [name(input, &names)] })),
        2 => {
            let (from, to) = (name(input, &names), input_of(input, &names));
            ("wire.connect", json!({ "file": file, "from": from, "to": to, "replace": input.flag(), "accept_mismatch": input.flag() }))
        }
        3 => {
            let (from, to) = (name(input, &names), input_of(input, &names));
            ("wire.disconnect", json!({ "file": file, "wire": format!("{from} -> {to}") }))
        }
        _ => ("constant.add", json!({ "file": file, "value": input.pick(CONSTANTS) })),
    };
    if input.byte() % 4 == 0 {
        params["name"] = json!(name(input, &names));
    }
    (id, params)
}

/// A revision for `file`: none, the right one, or one that is stale.
fn revision(input: &mut Input<'_>, memory: &Memory, file: &str) -> Option<Json> {
    match input.byte() % 4 {
        0 => Some(json!({ file: labcraft_engine::revision(&memory.get(format!("/p/{file}")).unwrap_or_default()) })),
        1 => Some(json!({ file: "sha256:0000" })),
        _ => None,
    }
}

/// Never `internal`; a command that fails writes nothing.
fn judge_failure(error: &CommandError, id: &str, params: &Json, before: &BTreeMap<String, Vec<u8>>, memory: &Memory) {
    assert_ne!(error.code, "internal", "{id} {params}: {}", error.message);
    assert!(snapshot(memory) == *before, "{id} {params} failed with `{}` and wrote", error.code);
}

/// Every file that a command wrote parses.
fn judge_written(result: &Json, id: &str, memory: &Memory) {
    for file in result["files"].as_array().into_iter().flatten() {
        if file["revision"].is_null() {
            continue;
        }
        let shown = file["file"].as_str().unwrap_or_default();
        let bytes = memory.get(format!("/p/{shown}")).unwrap_or_default();
        let formatted = labcraft_format::format(&bytes, &mut Interner::new(), &Limits::default());
        assert!(formatted.text.is_some(), "{id} wrote `{shown}`, which does not parse: {}", String::from_utf8_lossy(&bytes));
    }
}

fn files_of(result: &Json) -> Vec<String> {
    result["files"].as_array().into_iter().flatten().filter_map(|file| file["file"].as_str().map(str::to_owned)).collect()
}

fn work(bytes: &[u8]) {
    let memory = Arc::new(Memory::new());
    for (name, text) in PROJECT {
        memory.put(format!("/p/{name}"), *text);
    }
    let start = snapshot(&memory);
    let config = Config {
        root: "/p".into(),
        lib_paths: Vec::new(),
        limits: Limits::default(),
        shipped: shipped(),
        natives: labcraft_libs::natives(),
        clock: Arc::new(Still),
        process: 0,
    };
    let ws: Arc<dyn Workspace> = memory.clone();
    let mut session = Session::new(ws, config, false).expect("a session in memory");
    // The first command reads the files of the project, which the session follows from then on.
    let _ = session.execute("history.list", &json!({}));
    let agent = Author::Agent("fuzz".to_owned());
    let mut input = Input { bytes, at: 0 };
    // The files that an external write may change: those of the project, those the session wrote, and a new one.
    let mut written: BTreeSet<String> = PROJECT.iter().map(|(name, _)| (*name).to_owned()).collect();
    written.insert("new.lcf".to_owned());
    let mut outside = false;
    for _ in 0..MAX_STEPS {
        if input.done() {
            break;
        }
        let author = if input.byte() % 4 == 0 { agent.clone() } else { Author::User };
        let before = snapshot(&memory);
        match input.byte() % 9 {
            0..=3 => {
                let (id, mut params) = edit(&mut input, &memory);
                let file = params["file"].as_str().unwrap_or_default().to_owned();
                if let Some(revision) = revision(&mut input, &memory, &file) {
                    params["expected_revision"] = revision;
                }
                let verify = input.flag();
                match session.execute_as(&author, id, &params) {
                    Err(error) => judge_failure(&error, id, &params, &before, &memory),
                    Ok(result) => {
                        judge_written(&result, id, &memory);
                        written.extend(files_of(&result));
                        if verify && result["step"].is_u64() {
                            let after = snapshot(&memory);
                            session.execute("history.undo", &json!({})).unwrap_or_else(|error| panic!("undo after {id} {params}: {}", error.message));
                            assert!(snapshot(&memory) == before, "undo after {id} {params} does not give back the bytes before it");
                            session.execute("history.redo", &json!({})).unwrap_or_else(|error| panic!("redo after {id} {params}: {}", error.message));
                            assert!(snapshot(&memory) == after, "redo after {id} {params} does not give back the bytes after it");
                        }
                    }
                }
            }
            4 => {
                let commands: Vec<Json> = (0..1 + input.byte() % 3)
                    .map(|_| {
                        let (id, params) = edit(&mut input, &memory);
                        json!({ "id": id, "params": params })
                    })
                    .collect();
                let params = json!({ "commands": commands });
                match session.execute_as(&author, "commands.batch", &params) {
                    Err(error) => judge_failure(&error, "commands.batch", &params, &before, &memory),
                    Ok(result) => {
                        judge_written(&result, "commands.batch", &memory);
                        written.extend(files_of(&result));
                    }
                }
            }
            5 | 6 => {
                let id = if input.flag() { "history.undo" } else { "history.redo" };
                match session.execute_as(&author, id, &json!({})) {
                    Err(error) => assert_ne!(error.code, "internal", "{id}: {}", error.message),
                    Ok(result) => written.extend(files_of(&result)),
                }
            }
            7 => {
                // A file written whole: a text of the project, the shortest program, or random bytes made text.
                let path = input.pick(FILES);
                let text = match input.byte() % 3 {
                    0 => String::from_utf8_lossy(&memory.get(format!("/p/{}", input.pick(FILES))).unwrap_or_default()).into_owned(),
                    1 => "labcraft 1\n".to_owned(),
                    _ => String::from_utf8_lossy(&(0..input.byte() % 64).map(|_| input.byte()).collect::<Vec<u8>>()).into_owned(),
                };
                let params = json!({ "path": path, "text": text, "check": input.flag() });
                match session.execute_as(&author, "file.write", &params) {
                    Err(error) => judge_failure(&error, "file.write", &params, &before, &memory),
                    Ok(_) => {
                        written.insert(path.to_owned());
                    }
                }
            }
            _ => {
                // An external write of random bytes, or a removal, to a file of the project.
                let known: Vec<&String> = written.iter().collect();
                let Some(file) = known.get(usize::from(input.byte()) % known.len().max(1)) else { continue };
                let path = format!("/p/{file}");
                outside = true;
                if input.byte() % 8 == 0 {
                    let _ = memory.remove_file(Path::new(&path));
                } else {
                    let bytes: Vec<u8> = (0..input.byte() % 64).map(|_| input.byte()).collect();
                    memory.put(&path, bytes);
                }
                if input.flag() {
                    session.files_changed();
                }
            }
        }
    }
    if !outside {
        let listed = session.execute("history.list", &json!({ "limit": 2 * MAX_STEPS })).expect("the history");
        let steps = listed["steps"].as_array().cloned().unwrap_or_default();
        assert!(steps.iter().all(|step| step["author"] != "external"), "a step signed `external` with no external write: {listed}");
    }
    // As many undo as there are steps give back the start.
    for _ in 0..2 * MAX_STEPS + 1 {
        match session.execute("history.undo", &json!({})) {
            Ok(_) => {}
            Err(error) if error.code == "history.empty" => break,
            Err(error) => panic!("undo back to the start: `{}` {}", error.code, error.message),
        }
    }
    assert!(snapshot(&memory) == start, "as many undo do not give back the bytes of the start");
}

fuzz_target!(|bytes: &[u8]| {
    let bytes = bytes.to_vec();
    std::thread::Builder::new().stack_size(STACK_SIZE).spawn(move || work(&bytes)).expect("a thread").join().expect("the editing");
});
