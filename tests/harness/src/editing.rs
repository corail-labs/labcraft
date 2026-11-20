//! Scenario 3 of `docs/acceptance-m1.md`, natively and in WASM: on every function of `examples/`, `docs/rfc/` and `libs/`
//! held in memory, each editing command that applies, then `history.undo`, gives back the bytes of every file it touched;
//! a sequence of n commands then n undo gives back the bytes of the start, and n redo those after the sequence; a batch
//! is one step. The texts after each command are those of the unit tests of `edit`, written by hand; here, undo is checked
//! against the bytes read before.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::sync::Arc;

use labcraft_engine::{Config, Memory, Session};
use labcraft_graph::{Arg, Body, Decl, Operand, Rhs, Statement, StatementKind, Target};
use labcraft_types::{Interner, Limits};
use serde_json::{Value, json};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

use crate::PROGRAMS;

/// The folder that is the root of the library of a program: `examples`, `docs/rfc`, `libs/visa`.
fn root_of(path: &str) -> Option<String> {
    let mut parts = path.split('/');
    match (parts.next(), parts.next()) {
        (Some("examples"), _) => Some("examples".to_owned()),
        (Some("docs"), Some("rfc")) => Some("docs/rfc".to_owned()),
        (Some("libs"), Some(library)) => Some(format!("libs/{library}")),
        _ => None,
    }
}

/// The programs of each root, by their path under it.
fn roots() -> BTreeMap<String, Vec<(String, Vec<u8>)>> {
    let mut found: BTreeMap<String, Vec<(String, Vec<u8>)>> = BTreeMap::new();
    for file in PROGRAMS {
        if let Some(root) = root_of(file.path) {
            let under = file.path.strip_prefix(&format!("{root}/")).unwrap().to_owned();
            found.entry(root).or_default().push((under, file.bytes.to_vec()));
        }
    }
    found
}

fn session_on(files: &[(String, Vec<u8>)]) -> (Session, Arc<Memory>) {
    let memory = Arc::new(Memory::new());
    for (path, bytes) in files {
        memory.put(format!("/project/{path}"), bytes.clone());
    }
    let config = Config {
        root: "/project".into(),
        lib_paths: Vec::new(),
        limits: Limits::default(),
        shipped: crate::engine::shipped(),
        natives: labcraft_libs::natives(),
        clock: Arc::new(labcraft_engine::Still),
        process: 0,
    };
    let ws: Arc<dyn labcraft_engine::Workspace> = memory.clone();
    (Session::new(ws, config, false).unwrap(), memory)
}

/// The nodes of a function as a command names them, and the wires that reach them from a value (spec 11).
fn nodes_and_wires(text: &[u8]) -> (Vec<String>, Vec<String>) {
    let (nodes, wires, _) = parts(text);
    (nodes, wires)
}

/// Its nodes, its wires, and its inputs.
fn parts(text: &[u8]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut names = Interner::new();
    let reading = labcraft_format::read(text, &mut names, &Limits::default());
    let Some(file) = reading.file else { return (Vec::new(), Vec::new(), Vec::new()) };
    let Decl::Function(function) = &file.decl else { return (Vec::new(), Vec::new(), Vec::new()) };
    let (mut nodes, mut wires) = (Vec::new(), Vec::new());
    walk(&function.body, &names, &mut nodes, &mut wires);
    let inputs = function.signature.params.iter().map(|param| names.resolve(param.name).to_owned()).collect();
    (nodes, wires, inputs)
}

/// The same program, not in canonical form: each indentation halved, spaces doubled around `=`. Undo must give back
/// these bytes, not those of `fmt` (commands.md 9.1).
fn loosened(bytes: &[u8]) -> Vec<u8> {
    String::from_utf8_lossy(bytes).replace("\n    ", "\n  ").replace(" = ", "  =  ").into_bytes()
}

fn walk(body: &Body, names: &Interner, nodes: &mut Vec<String>, wires: &mut Vec<String>) {
    for statement in &body.statements {
        let Some(node) = node_name(statement, names) else { continue };
        nodes.push(node.clone());
        let mut reads = Vec::new();
        match &statement.kind {
            StatementKind::Call(call) | StatementKind::Binding { value: Rhs::Call(call), .. } => {
                for arg in call.args.iter() {
                    if let Arg::Value { value, .. } = arg {
                        reads.push(value);
                    }
                }
            }
            StatementKind::Binding { value: Rhs::Struct(construction), .. } => reads.extend(construction.fields.iter().map(|field| &field.value)),
            StatementKind::Binding { value: Rhs::Match(branching) | Rhs::Handle(branching), .. } => {
                reads.push(&branching.selector);
                for arm in &branching.arms {
                    walk(&arm.body, names, nodes, wires);
                }
            }
            StatementKind::Binding { value: Rhs::For(each), .. } => walk(&each.body, names, nodes, wires),
            StatementKind::Binding { value: Rhs::Loop(repeat), .. } => walk(&repeat.body, names, nodes, wires),
            _ => {}
        }
        for operand in reads {
            if let Operand::Path(path) = operand {
                wires.push(format!("{} -> {node}", names.resolve(path.first)));
            }
        }
    }
}

fn node_name(statement: &Statement, names: &Interner) -> Option<String> {
    if let Some(label) = statement.label {
        return Some(format!("#{}", names.resolve(label.name)));
    }
    match &statement.kind {
        StatementKind::Binding { targets, .. } => targets.iter().find_map(|target| match target {
            Target::Name(_, name) => Some(names.resolve(*name).to_owned()),
            Target::Skip(_) | Target::Next(..) => None,
        }),
        _ => None,
    }
}

fn snapshot(memory: &Memory, files: &[(String, Vec<u8>)]) -> Vec<Option<Vec<u8>>> {
    files.iter().map(|(path, _)| memory.get(format!("/project/{path}"))).collect()
}

/// Each command that applies, then undo: the bytes of before. Whether it applied.
fn applies_then_undoes(session: &mut Session, memory: &Memory, files: &[(String, Vec<u8>)], id: &str, params: &Value) -> bool {
    let before = snapshot(memory, files);
    match session.execute(id, params) {
        Ok(result) => {
            assert_ne!(result["step"], Value::Null, "{id} {params}: no step, yet it applied: {result}");
            let undone = session.execute("history.undo", &json!({})).unwrap_or_else(|error| panic!("undo of {id} {params}: {}", error.to_json()));
            assert_eq!(undone["step"], result["step"]);
            assert!(snapshot(memory, files) == before, "{id} {params}: undo does not give back the bytes");
            true
        }
        Err(error) => {
            assert_ne!(error.code, "internal", "{id} {params}: {}", error.to_json());
            assert!(snapshot(memory, files) == before, "{id} {params}: an error wrote something");
            false
        }
    }
}

fn on_the_stack_of_a_host(work: impl FnOnce() + Send + 'static) {
    #[cfg(target_arch = "wasm32")]
    work();
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::Builder::new().stack_size(labcraft_format::STACK_SIZE).spawn(work).unwrap().join().unwrap();
}

/// Every command of step 0 that applies to each function, each followed by undo; how many applied.
fn every_edit(files: &[(String, Vec<u8>)]) -> usize {
    let (mut session, memory) = session_on(files);
    let mut applied = 0;
    for (path, bytes) in files.iter().filter(|(path, _)| path.ends_with(".lcf")) {
        let (nodes, wires, inputs) = parts(bytes);
        if nodes.is_empty() {
            continue;
        }
        let mut each = |id: &str, params: Value| applied += usize::from(applies_then_undoes(&mut session, &memory, files, id, &params));
        for node in &nodes {
            each("node.delete", json!({ "file": path, "nodes": [node] }));
        }
        for wire in &wires {
            each("wire.disconnect", json!({ "file": path, "wire": wire }));
        }
        each("node.add", json!({ "file": path, "call": "core.add" }));
        each("node.add", json!({ "file": path, "call": "time.now", "at": [40, -80] }));
        each("constant.add", json!({ "file": path, "value": "240 V" }));
        // Each input of the function, wired to a node added for it: each input that `wire.connect` can take.
        if session.execute("node.add", &json!({ "file": path, "call": "core.abs", "name": "probe_node" })).is_ok() {
            let added = snapshot(&memory, files);
            for input in &inputs {
                let params = json!({ "file": path, "from": input, "to": "probe_node", "accept_mismatch": true });
                applied += usize::from(applies_then_undoes(&mut session, &memory, files, "wire.connect", &params));
                assert!(snapshot(&memory, files) == added);
            }
            session.execute("history.undo", &json!({})).unwrap();
        }
    }
    applied
}

#[test]
fn each_edit_then_undo_gives_back_every_byte() {
    on_the_stack_of_a_host(|| {
        let mut applied = 0;
        for files in roots().values() {
            applied += every_edit(files);
        }
        // 470 on 2026-11-15, give or take; far fewer means the functions are no longer found.
        assert!(applied > 300, "{applied} commands applied");
    });
}

#[test]
fn undo_gives_back_a_text_that_was_not_canonical() {
    on_the_stack_of_a_host(|| {
        let files: Vec<(String, Vec<u8>)> = roots()
            .remove("examples")
            .unwrap()
            .into_iter()
            .map(|(path, bytes)| (path.clone(), if path.ends_with(".lcf") { loosened(&bytes) } else { bytes }))
            .collect();
        let loose = files.iter().find(|(path, _)| path == "check_voltage.lcf").unwrap();
        assert!(String::from_utf8_lossy(&loose.1).contains("  =  "), "the copy is not canonical");
        let applied = every_edit(&files);
        assert!(applied > 100, "{applied} commands applied on texts that are not canonical");
    });
}

#[test]
fn a_node_of_a_library_that_no_file_uses_yet_adds_its_use() {
    on_the_stack_of_a_host(|| {
        let files = vec![("f.lcf".to_owned(), b"labcraft 1\n\nfn f(r: string) -> (y: f64) {\n    y = 1.0\n}\n".to_vec())];
        let (mut session, memory) = session_on(&files);
        let result =
            session.execute("node.add", &json!({ "file": "f.lcf", "call": "visa.open" })).unwrap_or_else(|error| panic!("{}", error.to_json()));
        assert_eq!(result["names"]["node"], "session");
        let written = String::from_utf8(memory.get("/project/f.lcf").unwrap()).unwrap();
        assert_eq!(written, "labcraft 1\n\nuse visa\n\nfn f(r: string) -> (y: f64) {\n    session, _ = visa.open()\n    y = 1.0\n}\n");
    });
}

#[test]
fn n_edits_then_n_undo_give_back_the_start_and_n_redo_the_end() {
    on_the_stack_of_a_host(|| {
        let files = roots().remove("examples").unwrap();
        let (mut session, memory) = session_on(&files);
        let path = "check_voltage.lcf";
        let start = snapshot(&memory, &files);
        let (nodes, _) = nodes_and_wires(memory.get(format!("/project/{path}")).unwrap().as_slice());
        let mut made = 0;
        for (id, params) in [
            ("node.add", json!({ "file": path, "call": "core.abs" })),
            ("constant.add", json!({ "file": path, "value": "1 V" })),
            ("wire.connect", json!({ "file": path, "from": "constant", "to": "abs" })),
            ("node.delete", json!({ "file": path, "nodes": [nodes[0]] })),
        ] {
            session.execute(id, &params).unwrap_or_else(|error| panic!("{id}: {}", error.to_json()));
            made += 1;
        }
        let end = snapshot(&memory, &files);
        for _ in 0..made {
            session.execute("history.undo", &json!({})).unwrap();
        }
        assert!(snapshot(&memory, &files) == start);
        for _ in 0..made {
            session.execute("history.redo", &json!({})).unwrap();
        }
        assert!(snapshot(&memory, &files) == end);

        // A batch is one step; one whose command fails writes nothing.
        let batch = json!({ "commands": [
            { "id": "node.add", "params": { "file": path, "call": "core.abs", "name": "magnitude" } },
            { "id": "constant.add", "params": { "file": path, "value": "2 V", "name": "two" } },
            { "id": "wire.connect", "params": { "file": path, "from": "two", "to": "magnitude" } },
        ] });
        let result = session.execute("commands.batch", &batch).unwrap();
        session.execute("history.undo", &json!({})).unwrap();
        assert!(snapshot(&memory, &files) == end, "one undo takes the batch back: {result}");
        let failing = json!({ "commands": [
            { "id": "node.add", "params": { "file": path, "call": "core.abs" } },
            { "id": "node.delete", "params": { "file": path, "nodes": ["nothing"] } },
        ] });
        assert_eq!(session.execute("commands.batch", &failing).unwrap_err().index, Some(1));
        assert!(snapshot(&memory, &files) == end);
    });
}
