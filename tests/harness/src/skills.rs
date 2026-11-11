//! The code blocks of the skills of `skills/`, as `guide.read` serves them (`skills/README.md`): the whole files of a
//! skill form one library, which `check` passes with no error and no warning, each `toml` block is a simulation file that
//! `visa` accepts, and no skill gives the programs of the acceptance scenarios. The Python checks of `cargo xtask spec`
//! parse every block and judge its canonical form, as for the spec.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use labcraft_engine::guide::{self, Skill};
use labcraft_engine::{Config, Memory, Session};
use labcraft_types::Limits;
use serde_json::json;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

/// The fenced blocks of a Markdown text: the word after the opening fence, and the lines between the fences.
fn blocks(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut open: Option<(String, Vec<&str>)> = None;
    for line in text.lines() {
        match (&mut open, line.strip_prefix("```")) {
            (None, Some(info)) => open = Some((info.trim().to_owned(), Vec::new())),
            (Some(_), Some("")) => {
                let (info, lines) = open.take().unwrap();
                found.push((info, lines.iter().map(|line| format!("{line}\n")).collect()));
            }
            (Some((_, lines)), _) => lines.push(line),
            (None, None) => {}
        }
    }
    assert!(open.is_none(), "a fence is not closed");
    found
}

/// The whole files of the `text` blocks of a skill (they start with `labcraft 1`), each named after what it declares.
fn whole_files(skill: &Skill) -> Vec<(String, String)> {
    let mut files = Vec::new();
    for file in skill.files {
        for (info, text) in blocks(file.text) {
            if info != "text" || !text.starts_with("labcraft 1\n") {
                continue;
            }
            let declaration = text.lines().find_map(|line| {
                let (kind, rest) = line.split_once(' ')?;
                let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                match kind {
                    "fn" => Some(format!("{name}.lcf")),
                    "type" => Some(format!("{name}.lct")),
                    _ => None,
                }
            });
            files.push((declaration.unwrap_or_else(|| panic!("{}/{}: a whole file declares nothing", skill.name, file.path)), text));
        }
    }
    files
}

fn session_on(memory: Memory) -> Session {
    let config = Config {
        root: "/skill".into(),
        lib_paths: Vec::new(),
        limits: Limits::default(),
        shipped: crate::engine::shipped(),
        natives: vec![Arc::new(labcraft_visa::Visa)],
        clock: Arc::new(labcraft_engine::Still),
        process: 0,
    };
    Session::new(Arc::new(memory), config, true).unwrap()
}

fn on_the_stack_of_a_host(work: impl FnOnce() + Send + 'static) {
    #[cfg(target_arch = "wasm32")]
    work();
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::Builder::new().stack_size(labcraft_format::STACK_SIZE).spawn(work).unwrap().join().unwrap();
}

#[test]
fn the_whole_files_of_each_skill_check_with_no_error_and_no_warning() {
    on_the_stack_of_a_host(|| {
        let mut checked = 0;
        for skill in guide::skills() {
            let files = whole_files(skill);
            if files.is_empty() {
                continue;
            }
            let memory = Memory::new();
            for (name, text) in &files {
                assert!(memory.get(format!("/skill/{name}")).is_none(), "{}: two blocks declare {name}", skill.name);
                memory.put(format!("/skill/{name}"), text.as_bytes());
            }
            let mut session = session_on(memory);
            let report = session.execute("library.check", &json!({ "deny_warnings": true })).unwrap();
            assert_eq!(report["status"], "ok", "{}: {report}", skill.name);
            assert_eq!((report["summary"]["errors"].as_u64(), report["summary"]["warnings"].as_u64()), (Some(0), Some(0)), "{report}");
            checked += files.len();
        }
        // `Reading`, `read_power`, `to_volts`, `sweep_current`, `wait_ready`, `watch_current`: the test is not empty.
        assert!(checked >= 6, "{checked} whole files");
    });
}

#[test]
fn each_toml_block_is_a_simulation_file_that_visa_accepts() {
    on_the_stack_of_a_host(|| {
        let memory = Memory::new();
        let mut paths = Vec::new();
        for skill in guide::skills() {
            for file in skill.files {
                for (info, text) in blocks(file.text) {
                    if info == "toml" {
                        let path = format!("{}-{}.toml", skill.name, paths.len());
                        memory.put(format!("/skill/{path}"), text.as_bytes());
                        paths.push(path);
                    }
                }
            }
        }
        assert!(paths.len() >= 3, "{paths:?}");
        let mut session = session_on(memory);
        for path in paths {
            let described = session.execute("simulation.describe", &json!({ "path": path })).unwrap();
            assert!(described.get("diagnostics").is_none(), "{path}: {described}");
            assert!(!described["parts"][0]["description"]["instruments"].as_array().unwrap().is_empty(), "{path}: {described}");
        }
    });
}

#[test]
fn no_skill_gives_the_programs_of_the_acceptance_scenarios_or_a_path_of_the_repository() {
    // Scenario 1: the agent has the skills and not the repository, where `examples/` holds the answer.
    for skill in guide::skills() {
        for file in skill.files {
            for word in ["check_voltage", "Verdict", "two_benches", "examples/", "docs/", "tools/"] {
                assert!(!file.text.contains(word), "{}/{}: `{word}`", skill.name, file.path);
            }
        }
    }
}

#[test]
fn no_whole_file_of_a_skill_declares_a_name_of_the_examples() {
    // The programs of the acceptance scenarios are in `examples/`; a file is named after what it declares (R-22).
    let declared: Vec<&str> =
        crate::PROGRAMS.iter().filter(|file| file.path.starts_with("examples/")).filter_map(|file| file.path.rsplit('/').next()).collect();
    assert!(declared.contains(&"check_voltage.lcf") && declared.contains(&"Verdict.lct"), "{declared:?}");
    for skill in guide::skills() {
        for (name, _) in whole_files(skill) {
            assert!(!declared.contains(&name.as_str()), "{}: `{name}` is declared in `examples/`", skill.name);
        }
    }
}

#[test]
fn a_fence_opens_with_its_word_and_closes_bare() {
    let text = "a\n```text\nx = 1\n```\n```toml\n[visa]\n```\n";
    assert_eq!(blocks(text), [("text".to_owned(), "x = 1\n".to_owned()), ("toml".to_owned(), "[visa]\n".to_owned())]);
}
