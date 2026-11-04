//! `cargo xtask libs`: no crate of `crates/` depends on `libs/`; only the apps link the libraries
//! shipped with LabCraft (`docs/roadmap.md`). The boundary that packages will use is tried from M0.

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
    let (crates, libs) = (root.join("crates"), root.join("libs"));
    let mut violations = Vec::new();
    for package in packages.iter().filter(|package| package.manifest.starts_with(&crates)) {
        for (name, _) in package.dependencies.iter().filter(|(_, path)| path.starts_with(&libs)) {
            violations.push(format!("{} -> {name}: no crate of crates/ depends on libs/", package.name));
        }
    }
    violations
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
}
