//! Embeds the corpora in the test binary, since WASM has no file system (`docs/architecture.md`, section 8).
//! Walking the folders here means that a new file joins the tests with nothing to declare.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..").canonicalize().unwrap();
    let mut out = String::new();

    let mut programs = Vec::new();
    for folder in ["examples", "libs"] {
        println!("cargo::rerun-if-changed={}", root.join(folder).display());
        walk(&root.join(folder), &mut programs);
    }
    writeln!(out, "pub static PROGRAMS: &[Embedded] = &[{}];", embedded(&root, &programs)).unwrap();

    let hostile = root.join("tests/acceptance/hostile");
    println!("cargo::rerun-if-changed={}", hostile.display());
    let mut cases: Vec<PathBuf> = fs::read_dir(&hostile).unwrap().map(|entry| entry.unwrap().path()).collect();
    cases.sort();
    writeln!(out, "pub static HOSTILE: &[Case] = &[").unwrap();
    for case in cases {
        let mut files = Vec::new();
        walk(&case, &mut files);
        let name = case.file_name().unwrap().to_str().unwrap();
        let expected = case.join("expected.json");
        writeln!(out, "Case {{ name: {name:?}, files: &[{}], expected: include_str!({expected:?}) }},", embedded(&root, &files)).unwrap();
    }
    writeln!(out, "];").unwrap();

    fs::write(PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("corpus.rs"), out).unwrap();
}

/// The programs below a folder, in the same order on every machine.
fn walk(folder: &Path, files: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(folder).unwrap().map(|entry| entry.unwrap().path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            walk(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "lcf" || extension == "lct") {
            files.push(path);
        }
    }
}

fn embedded(root: &Path, files: &[PathBuf]) -> String {
    let mut out = String::new();
    for file in files {
        let path = file.strip_prefix(root).unwrap().to_str().unwrap().replace('\\', "/");
        write!(out, "Embedded {{ path: {path:?}, bytes: include_bytes!({file:?}) }},").unwrap();
    }
    out
}
