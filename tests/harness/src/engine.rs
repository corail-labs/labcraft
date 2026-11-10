//! The session of `engine` on the files of `examples/` held in memory, natively and in WASM, where the web viewer will run
//! it in a worker (`docs/architecture.md`, 10.8): what it checks and lists is what the files say, with no file system.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use labcraft_engine::{Config, Memory, Session, Shipped, ShippedFile, ShippedLibrary};
use labcraft_types::Limits;
use serde_json::json;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

use crate::PROGRAMS;

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

/// The programs of `examples/`, under `/project`.
fn examples() -> (Memory, usize) {
    let memory = Memory::new();
    let mut functions = 0;
    for file in PROGRAMS {
        if let Some(path) = file.path.strip_prefix("examples/") {
            memory.put(format!("/project/{path}"), file.bytes);
            functions += usize::from(path.ends_with(".lcf"));
        }
    }
    (memory, functions)
}

fn on_the_stack_of_a_host(work: impl FnOnce() + Send + 'static) {
    #[cfg(target_arch = "wasm32")]
    work();
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::Builder::new().stack_size(labcraft_format::STACK_SIZE).spawn(work).unwrap().join().unwrap();
}

#[test]
fn a_session_in_memory_checks_the_examples_and_lists_what_they_declare() {
    on_the_stack_of_a_host(|| {
        let (memory, functions) = examples();
        let config = Config {
            root: "/project".into(),
            lib_paths: Vec::new(),
            limits: Limits::default(),
            shipped: shipped(),
            natives: vec![Arc::new(labcraft_visa::Visa)],
        };
        let mut session = Session::new(Arc::new(memory), config, true).unwrap();
        let report = session.execute("library.check", &json!({})).unwrap();
        assert_eq!(report["status"], "ok", "{report}");
        assert_eq!(report["summary"]["functions"], functions);
        let listed = session.execute("functions.list", &json!({ "filter": "check_voltage" })).unwrap();
        assert_eq!(listed["entries"][0]["name"], "check_voltage");
        let described = session.execute("simulation.describe", &json!({ "path": "<labcraft>/visa/sim/two-dmm.toml" })).unwrap();
        let resources: Vec<&str> = described["parts"][0]["description"]["instruments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|instrument| instrument["resource"].as_str().unwrap())
            .collect();
        assert_eq!(resources, ["SIM::DMM0", "SIM::DMM1"]);
    });
}
