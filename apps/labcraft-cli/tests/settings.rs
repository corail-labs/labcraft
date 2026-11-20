//! `labcraft-cli exec settings.*` (`docs/cli.md`, 9; `docs/commands.md`, 9.7): the store of the user is `settings.toml`
//! in the folder that `LABCRAFT_CONFIG_DIR` names, written key by key, and read again by the next process.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::{Value, json};

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("settings-{name}"));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("project")).unwrap();
        fs::write(path.join("project/main.lcf"), "labcraft 1\n\nfn main() -> () {}\n").unwrap();
        Folder(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn exec(folder: &Folder, id: &str, params: Value) -> (Output, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_labcraft-cli"))
        .env("LABCRAFT_CONFIG_DIR", folder.0.join("config"))
        .args(["exec", "--project"])
        .arg(folder.0.join("project"))
        .args([id, &params.to_string()])
        .output()
        .expect("the program starts");
    let json = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    (output, json)
}

#[test]
fn a_key_of_the_user_is_written_in_its_settings_toml_and_read_by_the_next_process() {
    let folder = Folder::new("user");
    fs::create_dir_all(folder.0.join("config")).unwrap();
    fs::write(folder.0.join("config/settings.toml"), "# mine\n[editor]\ndrag_pans = true\n").unwrap();
    let (output, set) = exec(&folder, "settings.set", json!({ "key": "appearance.theme", "value": "dark" }));
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(set, json!({ "before": null, "scope": "user" }));
    let text = fs::read_to_string(folder.0.join("config/settings.toml")).unwrap();
    assert!(text.starts_with("# mine\n[editor]\ndrag_pans = true\n"), "{text}");
    assert!(text.contains("[appearance]\ntheme = \"dark\"\n"), "{text}");
    let (_, got) = exec(&folder, "settings.get", json!({ "key": "editor.drag_pans" }));
    assert_eq!(got, json!({ "settings": [{ "key": "editor.drag_pans", "value": true, "from": "user", "project": true }] }));
    // `null` takes the key out of the file; its table stays under its comment, and a table without one goes.
    let (_, reset) = exec(&folder, "settings.set", json!({ "key": "editor.drag_pans", "value": null }));
    assert_eq!(reset["before"], true);
    let (_, _) = exec(&folder, "settings.set", json!({ "key": "appearance.theme", "value": null }));
    let text = fs::read_to_string(folder.0.join("config/settings.toml")).unwrap();
    assert_eq!(
        text,
        "# mine
[editor]
"
    );
    assert!(!folder.0.join("project/.labcraft/config.toml").exists(), "the project's store is not written");
}

#[test]
fn a_key_of_the_project_goes_to_its_config_toml() {
    let folder = Folder::new("project");
    let (output, set) = exec(&folder, "settings.set", json!({ "key": "editor.replace_with_alt", "value": true, "scope": "project" }));
    assert!(output.status.success(), "{set}");
    assert_eq!(fs::read_to_string(folder.0.join("project/.labcraft/config.toml")).unwrap(), "[editor]\nreplace_with_alt = true\n");
    assert_eq!(fs::read_to_string(folder.0.join("project/.labcraft/.gitignore")).unwrap(), "*\n!.gitignore\n!config.toml\n");
    assert!(!folder.0.join("config/settings.toml").exists());
    let (output, refused) = exec(&folder, "settings.set", json!({ "key": "appearance.theme", "value": "dark", "scope": "project" }));
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(refused["error"]["code"], "params.invalid");
}
