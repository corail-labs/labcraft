//! Embeds the libraries of `libs/`: a folder with a `labcraft.toml` is a library, and a new file joins it with
//! nothing to declare (`docs/architecture.md`, section 6).

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let libs = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("..").canonicalize().unwrap();
    println!("cargo::rerun-if-changed={}", libs.display());
    let mut out = String::from("pub static LIBRARIES: &[Library] = &[\n");
    for folder in sorted_entries(&libs) {
        let manifest = folder.join("labcraft.toml");
        if !manifest.is_file() {
            continue;
        }
        println!("cargo::rerun-if-changed={}", folder.display());
        let name = folder.file_name().unwrap().to_str().unwrap();
        write!(out, "Library {{ name: {name:?}, manifest: include_str!({manifest:?}), files: &[").unwrap();
        for file in sorted_entries(&folder) {
            // `docs/cli.md`, section 3, has sub-libraries; no shipped library has one yet, and this does not embed them.
            assert!(!is_sub_library(&file), "{}: a sub-library of a shipped library is not embedded yet", file.display());
            if file.extension().is_some_and(|extension| extension == "lcf" || extension == "lct") {
                let name = file.file_name().unwrap().to_str().unwrap();
                write!(out, "File {{ name: {name:?}, bytes: include_bytes!({file:?}) }},").unwrap();
            }
        }
        writeln!(out, "] }},").unwrap();
    }
    writeln!(out, "];").unwrap();
    fs::write(PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("libraries.rs"), out).unwrap();
}

/// A folder that the walk of `labcraft-cli` would read programs from. Other folders are no concern of this crate.
fn is_sub_library(path: &Path) -> bool {
    let hidden = path.file_name().unwrap().to_string_lossy().starts_with('.');
    path.is_dir()
        && !hidden
        && sorted_entries(path)
            .iter()
            .any(|entry| entry.extension().is_some_and(|extension| extension == "lcf" || extension == "lct") || is_sub_library(entry))
}

/// In the same order on every machine.
fn sorted_entries(folder: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(folder).unwrap().map(|entry| entry.unwrap().path()).collect();
    entries.sort();
    entries
}
