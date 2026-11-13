//! Embeds the folders that the web viewer opens without a file of the user's (`docs/architecture.md`, 11.5): the
//! program files of `examples/` and of `docs/rfc/`, at every depth, in the same order on every machine. The libraries of
//! `libs/` come from `labcraft-libs`.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

const BUNDLED: [(&str, &str); 2] = [("examples", "examples"), ("docs/rfc", "docs/rfc")];

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..").canonicalize().unwrap();
    let mut out = String::from("pub static BUNDLED: &[Bundled] = &[\n");
    for (name, folder) in BUNDLED {
        let base = root.join(folder);
        println!("cargo::rerun-if-changed={}", base.display());
        write!(out, "Bundled {{ name: {name:?}, files: &[").unwrap();
        let mut files = Vec::new();
        programs(&base, &base, &mut files);
        for (path, file) in files {
            write!(out, "BundledFile {{ path: {path:?}, bytes: include_bytes!({file:?}) }},").unwrap();
        }
        writeln!(out, "] }},").unwrap();
    }
    writeln!(out, "];").unwrap();
    fs::write(PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("bundled.rs"), out).unwrap();
}

/// The `.lcf` and `.lct` files under `folder`, each with its path from `base` with `/`; hidden folders left out, as a walk
/// of `engine` leaves them, and hidden files read, as it reads them.
fn programs(base: &Path, folder: &Path, found: &mut Vec<(String, PathBuf)>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(folder).unwrap().map(|entry| entry.unwrap().path()).collect();
    entries.sort();
    for entry in entries {
        let name = entry.file_name().unwrap().to_str().unwrap();
        if entry.is_dir() && !name.starts_with('.') {
            println!("cargo::rerun-if-changed={}", entry.display());
            programs(base, &entry, found);
        } else if entry.is_file() && entry.extension().is_some_and(|extension| extension == "lcf" || extension == "lct") {
            let path = entry.strip_prefix(base).unwrap().components().map(|part| part.as_os_str().to_str().unwrap()).collect::<Vec<_>>().join("/");
            found.push((path, entry));
        }
    }
}
