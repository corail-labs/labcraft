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
        if folder.join("labcraft.toml").is_file() {
            let name = folder.file_name().unwrap().to_str().unwrap().to_owned();
            library(&mut out, &folder, &name);
        }
    }
    writeln!(out, "];").unwrap();
    fs::write(PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("libraries.rs"), out).unwrap();
}

/// One library, then its sub-libraries (`docs/cli.md`, section 3), each named after its path with `.` (`visa.sim`, from the
/// folder `visa/sim`). A sub-library of a shipped library has its manifest; its folder can also hold the simulation files of
/// the library around it (`visa/sim/`), which are no files of the sub-library.
fn library(out: &mut String, folder: &Path, name: &str) {
    let manifest = folder.join("labcraft.toml");
    println!("cargo::rerun-if-changed={}", folder.display());
    write!(out, "Library {{ name: {name:?}, manifest: include_str!({manifest:?}), files: &[").unwrap();
    let mut subs = Vec::new();
    for file in sorted_entries(folder) {
        if is_sub_library(&file) {
            assert!(file.join("labcraft.toml").is_file(), "{}: a sub-library of a shipped library has its manifest", file.display());
            subs.push(file);
        } else if file.extension().is_some_and(|extension| extension == "lcf" || extension == "lct") {
            let name = file.file_name().unwrap().to_str().unwrap();
            write!(out, "File {{ name: {name:?}, bytes: include_bytes!({file:?}) }},").unwrap();
        }
    }
    // The simulation files it ships, read by `simulation.describe` and `run` as `<labcraft>/visa/sim/bench.toml`; the
    // manifest of a sub-library there is none.
    write!(out, "], simulations: &[").unwrap();
    let sim = folder.join("sim");
    if sim.is_dir() {
        for file in sorted_entries(&sim) {
            let name = file.file_name().unwrap().to_str().unwrap();
            if file.extension().is_some_and(|extension| extension == "toml") && name != "labcraft.toml" {
                let name = format!("sim/{name}");
                write!(out, "File {{ name: {name:?}, bytes: include_bytes!({file:?}) }},").unwrap();
            }
        }
    }
    writeln!(out, "] }},").unwrap();
    for sub in subs {
        let sub_name = format!("{name}.{}", sub.file_name().unwrap().to_str().unwrap());
        library(out, &sub, &sub_name);
    }
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
