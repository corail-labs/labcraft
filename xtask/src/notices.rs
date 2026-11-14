//! `cargo xtask notices TARGET OUT`: the licences of the crates built into the binaries of a release for `TARGET`, in
//! one text file that each archive carries (`docs/architecture.md`, section 8). The crates are those that the apps of
//! the release depend on for that target, through normal dependencies (not build or dev ones), less ours (path
//! packages). For each: its name, version, SPDX expression and repository, then the licence files it ships, or those
//! at the root of its git checkout when the crate is a folder of a larger repository. A text shipped by several crates
//! is printed once and referred to after.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The apps of a release: the desktop ones, or the web viewer for WASM.
pub fn roots(target: &str) -> &'static [&'static str] {
    if target.starts_with("wasm32") { &["labcraft-web"] } else { &["labcraft", "labcraft-cli"] }
}

pub struct Crate {
    pub name: String,
    pub version: String,
    pub license: String,
    pub repository: String,
    /// The `authors` of its manifest, printed for a crate that ships no licence file: the copyright holders that its
    /// licence names.
    pub authors: Vec<String>,
    /// The folder of its `Cargo.toml`.
    pub folder: PathBuf,
    pub git: bool,
}

/// The crates that `roots` reach through normal dependencies in the resolve of `metadata`, ours left out; sorted by
/// name and version.
pub fn reached(metadata: &Value, roots: &[&str]) -> Result<Vec<Crate>, String> {
    let packages = metadata["packages"].as_array().ok_or("metadata: no packages")?;
    let by_id: BTreeMap<&str, &Value> = packages.iter().filter_map(|p| Some((p["id"].as_str()?, p))).collect();
    let nodes = metadata["resolve"]["nodes"].as_array().ok_or("metadata: no resolve")?;
    let deps: BTreeMap<&str, &Value> = nodes.iter().filter_map(|n| Some((n["id"].as_str()?, &n["deps"]))).collect();
    let mut seen = BTreeSet::new();
    let mut stack = Vec::new();
    for root in roots {
        let id = packages
            .iter()
            .find(|p| p["name"].as_str() == Some(root) && p["source"].is_null())
            .and_then(|p| p["id"].as_str())
            .ok_or_else(|| format!("metadata: no package {root}"))?;
        seen.insert(id);
        stack.push(id);
    }
    while let Some(id) = stack.pop() {
        for dep in deps.get(id).and_then(|d| d.as_array()).into_iter().flatten() {
            let normal = dep["dep_kinds"].as_array().is_some_and(|kinds| kinds.iter().any(|k| k["kind"].is_null()));
            if let (true, Some(pkg)) = (normal, dep["pkg"].as_str())
                && seen.insert(pkg)
            {
                stack.push(pkg);
            }
        }
    }
    let mut out = Vec::new();
    for id in seen {
        let package = by_id.get(id).ok_or_else(|| format!("metadata: no package for {id}"))?;
        let Some(source) = package["source"].as_str() else { continue };
        let manifest = package["manifest_path"].as_str().ok_or("metadata: a package without manifest_path")?;
        out.push(Crate {
            name: package["name"].as_str().unwrap_or_default().to_owned(),
            version: package["version"].as_str().unwrap_or_default().to_owned(),
            license: package["license"].as_str().unwrap_or("(none in its manifest)").to_owned(),
            repository: package["repository"].as_str().unwrap_or_default().to_owned(),
            authors: package["authors"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(str::to_owned)).collect(),
            folder: Path::new(manifest).parent().map(Path::to_path_buf).unwrap_or_default(),
            git: source.starts_with("git+"),
        });
    }
    out.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    Ok(out)
}

fn is_licence_file(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    ["LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE", "UNLICENSE"].iter().any(|start| upper.starts_with(start))
}

/// The licence files of a folder, sorted.
fn licence_files(folder: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .filter(|entry| is_licence_file(&entry.file_name().to_string_lossy()))
        .map(|entry| entry.path())
        .collect();
    files.sort();
    files
}

/// The licence files of a crate: in its folder, or, for a crate of a git checkout that ships none, in the nearest
/// folder above that has some, up to the root of the checkout.
pub fn files_of(krate: &Crate) -> Vec<PathBuf> {
    let mut folder = krate.folder.as_path();
    loop {
        let files = licence_files(folder);
        if !files.is_empty() || !krate.git || folder.join(".git").exists() {
            return files;
        }
        match folder.parent() {
            Some(parent) => folder = parent,
            None => return files,
        }
    }
}

/// The text of the notices, from each crate and the files found for it with their texts.
pub fn render(target: &str, crates: &[(Crate, Vec<(String, String)>)]) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Third-party licences of the LabCraft binaries for {target}\n\n\
         The crates below are built into the binaries of this archive. Each is listed with its version, its licence\n\
         (an SPDX expression; where it offers a choice, LabCraft takes it under any of the licences offered) and the\n\
         licence files it ships. A text shipped by several crates is printed once, with the first of them, and the\n\
         others refer to it. For a crate that ships no licence file, the authors of its manifest are given as its\n\
         copyright holders, and the text of its licence is the standard one of its SPDX identifier\n\
         (https://spdx.org/licenses/): for MIT and Apache-2.0, as in LICENSE-MIT and LICENSE-APACHE beside this file,\n\
         with those authors in place of the LabCraft contributors. Written by `cargo xtask notices`.\n"
    );
    let mut first: BTreeMap<&str, String> = BTreeMap::new();
    // For a crate that names no authors, those of another crate of its repository.
    let mut of_repository: BTreeMap<&str, (&str, &[String])> = BTreeMap::new();
    for (krate, _) in crates {
        if !krate.repository.is_empty() && !krate.authors.is_empty() {
            of_repository.entry(krate.repository.as_str()).or_insert((krate.name.as_str(), krate.authors.as_slice()));
        }
    }
    for (krate, files) in crates {
        let who = format!("{} {}", krate.name, krate.version);
        let _ = writeln!(out, "{}\n{who}\nLicence: {}", "=".repeat(78), krate.license);
        if !krate.repository.is_empty() {
            let _ = writeln!(out, "Repository: {}", krate.repository);
        }
        if files.is_empty() {
            let _ = writeln!(out, "\nThis crate ships no licence file; its licence is the one named above.");
            if !krate.authors.is_empty() {
                let _ = writeln!(out, "Copyright: the authors of {}: {}.", krate.name, krate.authors.join(", "));
            } else if let Some((other, authors)) = of_repository.get(krate.repository.as_str()) {
                let _ = writeln!(out, "Copyright: the authors of its repository, as {other} names them: {}.", authors.join(", "));
            }
        }
        for (name, text) in files {
            match first.get(text.as_str()) {
                Some(earlier) => {
                    let _ = writeln!(out, "\n{name}: the same text as for {earlier}.");
                }
                None => {
                    let _ = writeln!(out, "\n--- {name} ---\n{}", text.trim_end());
                    first.insert(text.as_str(), format!("{who} ({name})"));
                }
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata() -> Value {
        serde_json::json!({
            "packages": [
                {"id": "app", "name": "labcraft", "version": "0.3.0", "source": null, "manifest_path": "/w/apps/labcraft/Cargo.toml"},
                {"id": "cli", "name": "labcraft-cli", "version": "0.3.0", "source": null, "manifest_path": "/w/apps/labcraft-cli/Cargo.toml"},
                {"id": "ours", "name": "labcraft-types", "version": "0.3.0", "source": null, "manifest_path": "/w/crates/types/Cargo.toml"},
                {"id": "a", "name": "a", "version": "1.0.0", "license": "MIT", "repository": "https://a", "source": "registry+https://x", "manifest_path": "/r/a/Cargo.toml"},
                {"id": "b", "name": "b", "version": "2.0.0", "license": "Apache-2.0", "source": "git+https://b#c", "manifest_path": "/g/b/sub/Cargo.toml"},
                {"id": "dev", "name": "dev", "version": "1.0.0", "license": "MIT", "source": "registry+https://x", "manifest_path": "/r/dev/Cargo.toml"},
                {"id": "build", "name": "build", "version": "1.0.0", "license": "MIT", "source": "registry+https://x", "manifest_path": "/r/build/Cargo.toml"}
            ],
            "resolve": {"nodes": [
                {"id": "app", "deps": [{"pkg": "ours", "dep_kinds": [{"kind": null}]}, {"pkg": "dev", "dep_kinds": [{"kind": "dev"}]}]},
                {"id": "cli", "deps": [{"pkg": "build", "dep_kinds": [{"kind": "build"}]}]},
                {"id": "ours", "deps": [{"pkg": "a", "dep_kinds": [{"kind": null}]}]},
                {"id": "a", "deps": [{"pkg": "b", "dep_kinds": [{"kind": "build"}, {"kind": null}]}]},
                {"id": "b", "deps": []},
                {"id": "dev", "deps": []},
                {"id": "build", "deps": []}
            ]}
        })
    }

    #[test]
    fn normal_dependencies_only_and_not_ours() {
        let names: Vec<String> = reached(&metadata(), roots("x86_64-pc-windows-msvc")).unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["a", "b"]);
    }

    #[test]
    fn a_missing_root_is_an_error() {
        assert!(reached(&metadata(), roots("wasm32-unknown-unknown")).is_err());
    }

    #[test]
    fn a_text_shipped_twice_is_printed_once() {
        let krate = |name: &str| Crate {
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            license: "MIT".to_owned(),
            repository: String::new(),
            authors: vec![format!("{name} author")],
            folder: PathBuf::new(),
            git: false,
        };
        let text = "Licensed under the Apache License, Version 2.0".to_owned();
        let out = render(
            "t",
            &[
                (krate("a"), vec![("LICENSE-APACHE".to_owned(), text.clone())]),
                (krate("b"), vec![("LICENSE".to_owned(), text.clone())]),
                (krate("c"), vec![]),
            ],
        );
        assert_eq!(out.matches(&text).count(), 1, "{out}");
        assert!(out.contains("LICENSE: the same text as for a 1.0.0 (LICENSE-APACHE)."), "{out}");
        assert!(out.contains("c 1.0.0\nLicence: MIT\n\nThis crate ships no licence file"), "{out}");
        assert!(out.contains("Copyright: the authors of c: c author."), "{out}");
        assert!(!out.contains("the authors of a:"), "a crate with a licence file has its copyright there: {out}");
        let sibling = |name: &str, authors: Vec<String>| Crate {
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            license: "MIT".to_owned(),
            repository: "https://r".to_owned(),
            authors,
            folder: PathBuf::new(),
            git: false,
        };
        let out = render("t", &[(sibling("d", vec!["D".to_owned()]), vec![]), (sibling("e", vec![]), vec![])]);
        assert!(out.contains("e 1.0.0\nLicence: MIT\nRepository: https://r\n\nThis crate ships no licence file; its licence is the one named above.\nCopyright: the authors of its repository, as d names them: D."), "{out}");
    }

    #[test]
    fn licence_files_by_name() {
        for name in ["LICENSE", "LICENSE-MIT", "license.txt", "COPYING", "NOTICE", "UNLICENSE", "LICENCE-APACHE.md"] {
            assert!(is_licence_file(name), "{name}");
        }
        for name in ["README.md", "Cargo.toml", "src"] {
            assert!(!is_licence_file(name), "{name}");
        }
    }
}
