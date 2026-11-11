//! Embeds the skills of `skills/` for `guide.read` (`docs/commands.md`, 6.1): a folder with a `SKILL.md` is a skill,
//! and a new file of it, a Markdown file at any depth, joins it with nothing to declare.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let skills = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../skills").canonicalize().unwrap();
    println!("cargo::rerun-if-changed={}", skills.display());
    let mut out = String::from("pub(crate) static SKILLS: &[Skill] = &[\n");
    for folder in sorted_entries(&skills) {
        if !folder.join("SKILL.md").is_file() {
            continue;
        }
        let name = folder.file_name().unwrap().to_str().unwrap();
        write!(out, "Skill {{ name: {name:?}, files: &[").unwrap();
        let mut files = Vec::new();
        markdown(&folder, &folder, &mut files);
        // `SKILL.md` first: it is the file a call without `file` reads, and the first one an agent should see.
        files.sort_by_key(|(path, _)| (path != "SKILL.md", path.clone()));
        for (path, file) in files {
            write!(out, "SkillFile {{ path: {path:?}, text: include_str!({file:?}) }},").unwrap();
        }
        writeln!(out, "] }},").unwrap();
    }
    writeln!(out, "];").unwrap();
    fs::write(PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("skills.rs"), out).unwrap();
}

/// The Markdown files under `folder`, named by their path from `root` with `/`.
fn markdown(root: &Path, folder: &Path, files: &mut Vec<(String, PathBuf)>) {
    println!("cargo::rerun-if-changed={}", folder.display());
    for entry in sorted_entries(folder) {
        if entry.is_dir() {
            markdown(root, &entry, files);
        } else if entry.extension().is_some_and(|extension| extension == "md") {
            let parts: Vec<String> =
                entry.strip_prefix(root).unwrap().components().map(|part| part.as_os_str().to_str().unwrap().to_owned()).collect();
            files.push((parts.join("/"), entry));
        }
    }
}

/// In the same order on every machine.
fn sorted_entries(folder: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(folder).unwrap().map(|entry| entry.unwrap().path()).collect();
    entries.sort();
    entries
}
