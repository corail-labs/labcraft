//! Embeds the examples that the start page copies into a folder of the user (`docs/ide.md`, 10): the
//! program files of `examples/` and of `docs/rfc/`, at every depth, in the same order on every machine. The libraries of
//! `libs/` come from `labcraft-libs`.
//!
//! For a Windows target it also gives `labcraft.exe` its icon, `assets/brand/labcraft.ico`, and its version. Without a
//! resource compiler it is a warning, so that a build still links, unless `LABCRAFT_REQUIRE_ICON` is set, as the
//! workflow `release` sets it.

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
    windows_resource(&root);
}

/// The icon and the version of `labcraft.exe`, for a Windows target only.
fn windows_resource(root: &Path) {
    let file = root.join("assets/brand/labcraft.ico");
    println!("cargo::rerun-if-changed={}", file.display());
    println!("cargo::rerun-if-env-changed=LABCRAFT_REQUIRE_ICON");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon(&file.to_string_lossy())
        .set("ProductName", "LabCraft")
        .set("FileDescription", "LabCraft")
        .set("CompanyName", "LabCraft contributors")
        .set("LegalCopyright", "Copyright (c) 2026 LabCraft contributors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "labcraft.exe")
        .set("InternalName", "labcraft");
    if let Err(error) = resource.compile() {
        if std::env::var_os("LABCRAFT_REQUIRE_ICON").is_some() {
            panic!("labcraft.exe: its icon could not be embedded: {error}");
        }
        println!("cargo::warning=labcraft.exe built without its icon: {error}");
    }
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
