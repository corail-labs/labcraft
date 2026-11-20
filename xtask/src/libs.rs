//! `cargo xtask libs`: no crate of `crates/` depends on `libs/`, nor on a native crate of `examples/`; only the apps and the
//! harness link the libraries shipped with LabCraft and those of the examples (`docs/roadmap.md`, architecture 7). The
//! boundary that packages will use is tried from M0. One exception, read here: `crates/check/src/core_blocks.rs` embeds the
//! files of `libs/core`, the blocks of the primitives of `core` (spec A.1), which no library holds; no other source file
//! of `crates/` embeds a file of `libs/` or `examples/`.

use std::path::{Path, PathBuf};

use serde_json::Value;

pub struct Package {
    name: String,
    manifest: PathBuf,
    /// The local dependencies: name and folder.
    dependencies: Vec<(String, PathBuf)>,
}

pub fn from_metadata(metadata: &Value) -> Result<Vec<Package>, String> {
    let packages = metadata["packages"].as_array().ok_or("metadata: no packages")?;
    let mut out = Vec::new();
    for package in packages {
        let name = package["name"].as_str().ok_or("metadata: a package without a name")?.to_owned();
        let manifest = package["manifest_path"].as_str().ok_or("metadata: a package without a manifest")?.into();
        let mut dependencies = Vec::new();
        for dependency in package["dependencies"].as_array().into_iter().flatten() {
            if let (Some(name), Some(path)) = (dependency["name"].as_str(), dependency["path"].as_str()) {
                dependencies.push((name.to_owned(), path.into()));
            }
        }
        out.push(Package { name, manifest, dependencies });
    }
    Ok(out)
}

pub fn check(root: &Path, packages: &[Package]) -> Vec<String> {
    let crates = root.join("crates");
    let mut violations = Vec::new();
    for package in packages.iter().filter(|package| package.manifest.starts_with(&crates)) {
        for (name, path) in &package.dependencies {
            for folder in ["libs", "examples"] {
                if path.starts_with(root.join(folder)) {
                    violations.push(format!("{} -> {name}: no crate of crates/ depends on {folder}/", package.name));
                }
            }
        }
    }
    violations
}

/// The one source file of `crates/` that may embed a file of `libs/`, and only of `libs/core`.
const CORE_BLOCKS: &str = "crates/check/src/core_blocks.rs";

/// The embeddings (`include!`, `include_str!`, `include_bytes!`) of a file of `libs/` or `examples/` by a source file of
/// `crates/`: `files` gives each file by its path from the root, with its text. The path is the text from the macro to
/// its `)`, over several lines if need be; `CORE_BLOCKS` may name `libs/core/` only, with no `..` after it.
pub fn embeddings(files: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    for (path, text) in files {
        for (start, _) in text.match_indices("include") {
            let Some(rest) = text.get(start..) else { continue };
            if !(rest.starts_with("include!(") || rest.starts_with("include_str!(") || rest.starts_with("include_bytes!(")) {
                continue;
            }
            let argument = rest.split(')').next().unwrap_or(rest);
            for folder in ["libs/", "examples/"] {
                if !argument.contains(&format!("../{folder}")) {
                    continue;
                }
                let core = argument.split("../libs/core/").nth(1);
                let allowed = path == CORE_BLOCKS && folder == "libs/" && core.is_some_and(|after| !after.contains(".."));
                if !allowed {
                    violations.push(format!("{path}: embeds a file of {folder}: {}", argument.split_whitespace().collect::<Vec<_>>().join(" ")));
                }
            }
        }
    }
    violations
}

/// The source files of `crates/`, by their path from `root` with `/`, and their text.
pub fn sources(root: &Path) -> Result<Vec<(String, String)>, String> {
    let files = crate::clean_room::repository_files(root, "libs")?;
    files
        .into_iter()
        // A file deleted but not yet committed is still listed.
        .filter(|path| path.starts_with("crates/") && path.ends_with(".rs") && root.join(path).is_file())
        .map(|path| std::fs::read_to_string(root.join(&path)).map(|text| (path.clone(), text)).map_err(|error| format!("{path}: {error}")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(manifest: &str, dependency: &str) -> Package {
        Package { name: "tested".to_owned(), manifest: manifest.into(), dependencies: vec![("dependency".to_owned(), dependency.into())] }
    }

    #[test]
    fn a_crate_cannot_depend_on_libs() {
        let found = check(Path::new("/repo"), &[package("/repo/crates/check/Cargo.toml", "/repo/libs/embedded")]);
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn a_crate_cannot_depend_on_a_crate_of_the_examples() {
        let found = check(Path::new("/repo"), &[package("/repo/crates/runtime/Cargo.toml", "/repo/examples/results_log")]);
        assert_eq!(found, vec!["tested -> dependency: no crate of crates/ depends on examples/".to_owned()]);
    }

    #[test]
    fn an_app_can_depend_on_libs() {
        assert!(check(Path::new("/repo"), &[package("/repo/apps/labcraft-cli/Cargo.toml", "/repo/libs/embedded")]).is_empty());
    }

    #[test]
    fn a_crate_can_depend_on_a_crate() {
        assert!(check(Path::new("/repo"), &[package("/repo/crates/check/Cargo.toml", "/repo/crates/format")]).is_empty());
    }

    #[test]
    fn a_folder_that_only_starts_with_libs_is_not_libs() {
        assert!(check(Path::new("/repo"), &[package("/repo/crates/check/Cargo.toml", "/repo/libsomething/x")]).is_empty());
    }

    #[test]
    fn metadata_gives_the_model() {
        let metadata = serde_json::json!({"packages": [{"name": "labcraft-check", "manifest_path": "/repo/crates/check/Cargo.toml",
            "dependencies": [{"name": "labcraft-libs", "path": "/repo/libs/embedded"}, {"name": "serde"}]}]});
        let packages = from_metadata(&metadata).unwrap();
        assert_eq!(packages[0].dependencies.len(), 1);
        assert_eq!(check(Path::new("/repo"), &packages).len(), 1);
    }
    #[test]
    fn only_core_blocks_embeds_libs_core() {
        let line = |path: &str, line: &str| vec![(path.to_owned(), format!("const A: &str = {line};\n"))];
        assert!(embeddings(&line(CORE_BLOCKS, r#"include_str!("../../../libs/core/add.lcf")"#)).is_empty());
        assert_eq!(embeddings(&line(CORE_BLOCKS, r#"include_str!("../../../libs/visa/close.lcf")"#)).len(), 1);
        assert_eq!(embeddings(&line("crates/diagram/src/a.rs", r#"include_str!("../../../libs/core/add.lcf")"#)).len(), 1);
        assert_eq!(embeddings(&line("crates/engine/src/a.rs", r#"include_bytes!("../../../examples/x.lcf")"#)).len(), 1);
        assert!(embeddings(&line("crates/engine/src/a.rs", r#"include_str!("../../../docs/cli.md")"#)).is_empty());
        // Over two lines, through `include!`, or out of `libs/core` by `..`.
        assert_eq!(embeddings(&line("crates/diagram/src/a.rs", "include_str!(\n    \"../../../libs/visa/close.lcf\"\n)")).len(), 1);
        assert_eq!(embeddings(&line("crates/diagram/src/a.rs", r#"include!("../../../libs/visa/x.rs")"#)).len(), 1);
        assert_eq!(embeddings(&line(CORE_BLOCKS, r#"include_str!("../../../libs/core/../scpi/a.lcf")"#)).len(), 1);
    }
}
