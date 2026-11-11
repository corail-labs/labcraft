//! On any library, a run of its first function ends with a report, under small limits: never a panic, never a bug of the
//! runtime caught (`LC8004`), always one of the statuses of spec 8.2 or `refused`; and two runs of the same library give the
//! same report, the real time aside (D-12). Scenario 4: runs of fuzzed programs, every outcome a report.
//!
//! The input is a library of up to 32 files separated by two NUL bytes, as for the target `check`; each file is named after
//! what it declares, or `f<N>`. The run goes through `engine::run_to_end`, as in WASM, against the simulated instruments of
//! `bench.toml`, with its trace written inline, so that the writer of the trace is fuzzed too. It runs on the stack of a host
//! (`format::STACK_SIZE`), as the program does.
#![no_main]

use std::sync::Arc;

use labcraft_engine::{Config, Memory, Session, Shipped, ShippedFile, ShippedLibrary, Still, run_to_end};
use labcraft_format::{STACK_SIZE, read};
use labcraft_graph::Decl;
use labcraft_types::{Interner, Limits};
use libfuzzer_sys::fuzz_target;
use serde_json::{Value as Json, json};

const FILES: usize = 32;

/// The libraries shipped with LabCraft, as `engine` is given them.
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

/// The files of the library, each named after what it declares, and the first function declared.
fn library(bytes: &[u8]) -> (Vec<(String, Vec<u8>)>, Option<String>) {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut first = None;
    // Two NUL bytes separate files, as for `check`; a file that reads holds none (`LC0005`), so each NUL splits the same.
    for (index, file) in bytes.split(|byte| *byte == 0).filter(|part| !part.is_empty()).take(FILES).enumerate() {
        let mut names = Interner::new();
        let (stem, kind) = match read(file, &mut names, &Limits::default()).file.map(|file| file.decl) {
            Some(Decl::Type(ty)) => (names.resolve(ty.name).to_owned(), "lct"),
            Some(Decl::Function(function)) => (names.resolve(function.name).to_owned(), "lcf"),
            Some(Decl::Extern(function)) => (names.resolve(function.name).to_owned(), "lcf"),
            None => (String::new(), "lcf"),
        };
        let taken = |name: &str| files.iter().any(|(held, _)| held == name);
        let name = if !stem.is_empty() && !taken(&format!("{stem}.{kind}")) { format!("{stem}.{kind}") } else { format!("f{index}.{kind}") };
        if taken(&name) {
            continue;
        }
        if kind == "lcf" && !stem.is_empty() && first.is_none() {
            first = Some(stem);
        }
        files.push((name, file.to_vec()));
    }
    (files, first)
}

/// The report, without what depends on the speed of the machine.
fn settled(mut report: Json) -> Json {
    if let Some(clock) = report.get_mut("clock").and_then(Json::as_object_mut) {
        clock.remove("real_ms");
    }
    report
}

fuzz_target!(|bytes: &[u8]| {
    let (files, function) = library(bytes);
    let Some(function) = function else { return };
    let work = move || {
        let memory = Memory::new();
        for (name, bytes) in &files {
            memory.put(format!("/fuzz/{name}"), bytes.clone());
        }
        let config = Config {
            root: "/".into(),
            lib_paths: Vec::new(),
            limits: Limits::default(),
            shipped: shipped(),
            natives: vec![Arc::new(labcraft_visa::Visa)],
            clock: Arc::new(Still),
            process: 0,
        };
        let session = Session::new(Arc::new(memory) as Arc<dyn labcraft_engine::Workspace>, config, false).expect("a session in memory");
        let params = json!({
            "function": function,
            "folder": "fuzz",
            "sim": "<labcraft>/visa/sim/bench.toml",
            "max_iterations": 200,
            "max_nodes": 20000,
            "max_call_depth": 64,
            "max_memory": 1_048_576,
            "max_made": 4_194_304,
            "timeout": "60 s",
            "trace": "inline",
        });
        let first = run_to_end(&session, &params).unwrap_or_else(|error| panic!("a command error: {}", error.to_json()));
        let status = first["status"].as_str().unwrap_or_default();
        assert!(["ok", "error", "aborted", "fault", "refused"].contains(&status), "status {status}: {first}");
        assert_ne!(first["fault"]["code"], "LC8004", "a bug of the runtime: {}", first["fault"]);
        let second = run_to_end(&session, &params).unwrap_or_else(|error| panic!("a command error: {}", error.to_json()));
        assert_eq!(settled(first), settled(second), "two runs of the same library differ");
    };
    std::thread::Builder::new().stack_size(STACK_SIZE).spawn(work).expect("a thread").join().expect("the run of the input");
});
