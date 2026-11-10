//! `cargo xtask layers`: a crate depends only on lower layers (`docs/roadmap.md`, layers). The rules
//! work on a small model so that they can be tested without a workspace that breaks them.

use serde_json::Value;

#[derive(Clone, Copy)]
enum Class {
    Layer(u8),
    /// Apps and tooling sit above the layers and may depend on anything; no layer depends on them.
    Above,
}

/// The table of `docs/roadmap.md`. A package that is not here is an error, so that a new crate must
/// be given its place.
const TABLE: &[(&str, Class)] = &[
    ("labcraft-types", Class::Layer(0)),
    ("labcraft-graph", Class::Layer(1)),
    ("labcraft-trace", Class::Layer(1)),
    ("labcraft-format", Class::Layer(2)),
    ("labcraft-check", Class::Layer(3)),
    ("labcraft-ir", Class::Layer(4)),
    ("labcraft-runtime", Class::Layer(5)),
    ("labcraft-diagram", Class::Layer(5)),
    ("labcraft-interop", Class::Layer(5)),
    ("labcraft-engine", Class::Layer(6)),
    ("labcraft-visa", Class::Layer(6)),
    ("labcraft-results-log", Class::Layer(6)),
    ("labcraft-editor", Class::Layer(7)),
    ("labcraft-ui-egui", Class::Layer(7)),
    ("labcraft-mcp", Class::Layer(7)),
    ("labcraft-codegen", Class::Layer(7)),
    ("labcraft", Class::Above),
    ("labcraft-cli", Class::Above),
    ("labcraft-web", Class::Above),
    ("labcraft-libs", Class::Above),
    ("labcraft-harness", Class::Above),
    ("xtask", Class::Above),
];

/// Nothing below the interface depends on a UI toolkit (`docs/craft-ecosystem.md`). A name ending in
/// `*` is a prefix.
const UI_CRATES: &[&str] = &["egui*", "eframe", "winit", "wgpu*", "rfd", "craft-ui", "craft-window", "craft-keymap"];
const FIRST_UI_LAYER: u8 = 7;

fn class(package: &str) -> Option<Class> {
    TABLE.iter().find(|(name, _)| *name == package).map(|(_, class)| *class)
}

fn is_ui_crate(name: &str) -> bool {
    UI_CRATES.iter().any(|pattern| match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == *pattern,
    })
}

pub struct Dependency {
    name: String,
    /// In the workspace, as opposed to from a registry or a git repository.
    local: bool,
}

pub struct Crate {
    name: String,
    /// Every kind: a dev-dependency that goes up is an error too.
    dependencies: Vec<Dependency>,
}

pub fn from_metadata(metadata: &Value) -> Result<Vec<Crate>, String> {
    let packages = metadata["packages"].as_array().ok_or("metadata: no packages")?;
    let mut crates = Vec::new();
    for package in packages {
        let name = package["name"].as_str().ok_or("metadata: a package without a name")?.to_owned();
        let mut dependencies = Vec::new();
        for dependency in package["dependencies"].as_array().into_iter().flatten() {
            let dependency_name = dependency["name"].as_str().ok_or("metadata: a dependency without a name")?.to_owned();
            dependencies.push(Dependency { name: dependency_name, local: dependency["path"].is_string() });
        }
        crates.push(Crate { name, dependencies });
    }
    Ok(crates)
}

pub fn check(crates: &[Crate]) -> Vec<String> {
    let mut violations = Vec::new();
    for krate in crates {
        let Some(class) = class(&krate.name) else {
            violations.push(format!("{}: unknown crate, give it a place in the table of xtask/src/layers.rs", krate.name));
            continue;
        };
        let Class::Layer(layer) = class else { continue };
        for dependency in krate.dependencies.iter().filter(|dependency| dependency.name != krate.name) {
            if dependency.local {
                match self::class(&dependency.name) {
                    Some(Class::Layer(target)) if target < layer => {}
                    // A local dependency that the table does not know is reported on its own entry.
                    None => {}
                    Some(Class::Layer(target)) => {
                        violations.push(format!("{} (L{layer}) -> {} (L{target}): only lower layers", krate.name, dependency.name));
                    }
                    Some(Class::Above) => {
                        violations.push(format!("{} (L{layer}) -> {}: an app or a tool, above the layers", krate.name, dependency.name));
                    }
                }
            } else if layer < FIRST_UI_LAYER && is_ui_crate(&dependency.name) {
                violations.push(format!("{} (L{layer}) -> {}: a UI toolkit, only from L{FIRST_UI_LAYER}", krate.name, dependency.name));
            }
        }
    }
    violations
}

/// egui and eframe are built without their default fonts (architecture 11.5): the canvas and the app install theirs,
/// each with its line in `ATTRIBUTION.md`. A crate that enables them, through any feature of the graph, would embed
/// faces that no line names.
pub fn default_fonts(metadata: &Value) -> Result<Vec<String>, String> {
    let packages = metadata["packages"].as_array().ok_or("metadata: no packages")?;
    let mut violations = Vec::new();
    for package in packages {
        if package["name"] == "epaint_default_fonts" {
            violations.push("epaint_default_fonts is in the graph: a crate builds egui, epaint or eframe with `default_fonts`".to_owned());
        }
    }
    for node in metadata["resolve"]["nodes"].as_array().into_iter().flatten() {
        let id = node["id"].as_str().unwrap_or_default();
        let features: Vec<&str> = node["features"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        if features.contains(&"default_fonts") {
            violations.push(format!("{id}: built with `default_fonts`"));
        }
    }
    Ok(violations)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn krate(name: &str, dependencies: &[(&str, bool)]) -> Crate {
        let dependencies = dependencies.iter().map(|(name, local)| Dependency { name: (*name).to_owned(), local: *local }).collect();
        Crate { name: name.to_owned(), dependencies }
    }

    fn violations(name: &str, dependencies: &[(&str, bool)]) -> Vec<String> {
        check(&[krate(name, dependencies)])
    }

    #[test]
    fn the_default_fonts_of_egui_are_refused() {
        let clean = serde_json::json!({ "packages": [{ "name": "egui" }], "resolve": { "nodes": [{ "id": "egui 0.37", "features": ["serde"] }] } });
        assert_eq!(default_fonts(&clean), Ok(Vec::new()));
        let fonts = serde_json::json!({
            "packages": [{ "name": "egui" }, { "name": "epaint_default_fonts" }],
            "resolve": { "nodes": [{ "id": "egui 0.37", "features": ["default_fonts"] }] }
        });
        assert_eq!(default_fonts(&fonts).map(|found| found.len()), Ok(2));
    }

    #[test]
    fn a_lower_layer_is_allowed() {
        assert!(violations("labcraft-check", &[("labcraft-format", true), ("labcraft-types", true)]).is_empty());
    }

    #[test]
    fn a_higher_layer_is_refused() {
        let found = violations("labcraft-types", &[("labcraft-check", true)]);
        assert!(found[0].contains("(L0) -> labcraft-check (L3)"), "{found:?}");
    }

    #[test]
    fn the_same_layer_is_refused() {
        assert_eq!(violations("labcraft-diagram", &[("labcraft-runtime", true)]).len(), 1);
    }

    #[test]
    fn apps_and_tools_are_out_of_reach_of_the_layers() {
        for target in ["labcraft-cli", "labcraft-libs", "labcraft-harness", "xtask"] {
            assert_eq!(violations("labcraft-engine", &[(target, true)]).len(), 1, "{target}");
        }
    }

    #[test]
    fn apps_and_tools_may_depend_on_anything() {
        assert!(violations("labcraft-cli", &[("labcraft-mcp", true), ("egui", false)]).is_empty());
    }

    #[test]
    fn an_unknown_crate_is_refused() {
        let found = violations("labcraft-mystery", &[]);
        assert!(found[0].contains("unknown crate"), "{found:?}");
    }

    #[test]
    fn a_ui_toolkit_waits_for_the_ui_layer() {
        for toolkit in ["egui", "egui_extras", "eframe", "winit", "wgpu", "wgpu-core", "rfd", "craft-ui", "craft-window", "craft-keymap"] {
            assert_eq!(violations("labcraft-diagram", &[(toolkit, false)]).len(), 1, "{toolkit}");
        }
        assert!(violations("labcraft-editor", &[("egui", false)]).is_empty());
        assert!(violations("labcraft-diagram", &[("serde", false)]).is_empty());
    }

    #[test]
    fn a_crate_may_name_itself_for_its_tests() {
        assert!(violations("labcraft-types", &[("labcraft-types", true)]).is_empty());
    }

    #[test]
    fn metadata_gives_the_model() {
        let metadata = serde_json::json!({"packages": [{"name": "labcraft-types", "dependencies": [
            {"name": "labcraft-graph", "path": "/repo/crates/graph", "kind": "dev"},
            {"name": "serde", "kind": null},
        ]}]});
        let crates = from_metadata(&metadata).unwrap();
        assert!(crates[0].dependencies[0].local);
        assert!(!crates[0].dependencies[1].local);
        assert_eq!(check(&crates).len(), 1);
    }
}
