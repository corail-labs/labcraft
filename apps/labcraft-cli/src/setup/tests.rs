//! Each expected text is written by hand from the rule of the module: only the entry `labcraft` changes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use super::*;

const EXE: &str = "/opt/labcraft/labcraft-cli";

fn json(client: Client, before: &str) -> (String, Change) {
    let mut text = before.to_owned();
    let change = edit_json(&mut text, client, EXE).unwrap();
    (text, change)
}

fn toml(before: &str) -> (String, Change) {
    let mut text = before.to_owned();
    let change = edit_toml(&mut text, EXE).unwrap();
    (text, change)
}

#[test]
fn an_empty_file_gets_the_list_and_the_entry() {
    let (text, change) = json(Client::Cursor, "");
    assert_eq!(change, Change::Added);
    assert_eq!(
        text,
        "{\n  \"mcpServers\": {\n    \"labcraft\": {\n      \"command\": \"/opt/labcraft/labcraft-cli\",\n      \"args\": [\n        \"mcp\"\n      ]\n    }\n  }\n}\n"
    );
}

#[test]
fn the_entry_goes_last_in_the_list_and_every_other_byte_stays() {
    let before = "{\n    \"theme\": \"dark\", // mine\n    \"mcpServers\": {\n        \"other\": { \"command\": \"o\" } /* keep */\n    },\n    \"zeta\": [1, 2]\n}\n";
    let (text, change) = json(Client::Gemini, before);
    assert_eq!(change, Change::Added);
    let after = "{\n    \"theme\": \"dark\", // mine\n    \"mcpServers\": {\n        \"other\": { \"command\": \"o\" }, /* keep */\n        \"labcraft\": {\n            \"command\": \"/opt/labcraft/labcraft-cli\",\n            \"args\": [\n                \"mcp\"\n            ]\n        }\n    },\n    \"zeta\": [1, 2]\n}\n";
    assert_eq!(text, after);
}

#[test]
fn a_trailing_comma_stays_and_needs_no_other() {
    let before = "{\n  \"servers\": {\n    \"other\": {},\n  },\n}\n";
    let (text, _) = json(Client::VsCode, before);
    assert!(text.starts_with("{\n  \"servers\": {\n    \"other\": {},\n    \"labcraft\": {\n"), "{text}");
    assert!(text.ends_with("    }\n  },\n}\n"), "{text}");
    assert!(!text.contains(",,"), "{text}");
}

#[test]
fn an_object_on_one_line_gets_the_entry_on_that_line() {
    let (text, _) = json(Client::Cursor, "{\"mcpServers\": {\"other\": {}}}");
    assert_eq!(text, "{\"mcpServers\": {\"other\": {}, \"labcraft\": {\"command\":\"/opt/labcraft/labcraft-cli\",\"args\":[\"mcp\"]}}}");
}

#[test]
fn a_file_without_the_list_gets_it_last() {
    let (text, _) = json(Client::Zed, "// Zed settings\n{\n  \"theme\": \"One Dark\"\n}\n");
    assert!(text.starts_with("// Zed settings\n{\n  \"theme\": \"One Dark\",\n  \"context_servers\": {\n    \"labcraft\": {\n"), "{text}");
    assert!(text.ends_with("    }\n  }\n}\n"), "{text}");
}

#[test]
fn an_entry_that_runs_another_labcraft_cli_gets_this_one_and_nothing_else_changes() {
    let before = "{\"mcpServers\": {\"labcraft\": {\"type\": \"stdio\", \"command\": \"C:\\\\Old\\\\labcraft-cli.exe\", \"args\": [\"mcp\"], \"env\": {\"A\": \"1\"}}}}";
    let (text, change) = json(Client::ClaudeCode, before);
    assert_eq!(change, Change::Updated);
    assert_eq!(
        text,
        "{\"mcpServers\": {\"labcraft\": {\"type\": \"stdio\", \"command\": \"/opt/labcraft/labcraft-cli\", \"args\": [\"mcp\"], \"env\": {\"A\": \"1\"}}}}"
    );
}

#[test]
fn an_entry_of_the_user_is_left_as_it_is() {
    for before in [
        "{\"mcpServers\": {\"labcraft\": {\"command\": \"/opt/labcraft/labcraft-cli\", \"args\": [\"mcp\", \"--project\", \"/p\"]}}}",
        "{\"mcpServers\": {\"labcraft\": {\"command\": \"node\", \"args\": [\"mcp\"]}}}",
        "{\"mcpServers\": {\"labcraft\": \"off\"}}",
    ] {
        let (text, change) = json(Client::Cursor, before);
        assert_eq!((text.as_str(), change), (before, Change::Theirs));
    }
    let same = "{\"mcpServers\": {\"labcraft\": {\"command\": \"/opt/labcraft/labcraft-cli\", \"args\": [\"mcp\"]}}}";
    assert_eq!(json(Client::Cursor, same), (same.to_owned(), Change::Unchanged));
}

#[test]
fn opencode_has_the_program_and_its_arguments_in_one_array() {
    let (text, _) = json(Client::OpenCode, "{\n  \"$schema\": \"https://opencode.ai/config.json\"\n}\n");
    let written: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(written["mcp"]["labcraft"], json!({ "type": "local", "command": [EXE, "mcp"], "enabled": true }));
    let old = "{\"mcp\": {\"labcraft\": {\"type\": \"local\", \"command\": [\"/old/labcraft-cli\", \"mcp\"], \"enabled\": true}}}";
    let (text, change) = json(Client::OpenCode, old);
    assert_eq!(change, Change::Updated);
    assert_eq!(text, old.replace("/old/labcraft-cli", EXE));
}

#[test]
fn a_file_that_does_not_read_is_refused() {
    for before in ["{\"mcpServers\": {", "[1]", "{\"mcpServers\": []}", "{} {}", "{/* open"] {
        let mut text = before.to_owned();
        assert!(edit_json(&mut text, Client::Cursor, EXE).is_err(), "{before}");
        assert_eq!(text, before);
    }
}

#[test]
fn a_file_with_crlf_keeps_it() {
    let (text, _) = json(Client::Cursor, "{\r\n  \"mcpServers\": {\r\n    \"other\": {}\r\n  }\r\n}\r\n");
    assert!(!text.replace("\r\n", "").contains('\n'), "{text:?}");
}

#[test]
fn codex_gets_a_table_of_its_own_and_its_file_stays() {
    let before = "# my settings\nmodel = \"o3\"\n\n[mcp_servers.other]\ncommand = \"o\" # keep\n";
    let (text, change) = toml(before);
    assert_eq!(change, Change::Added);
    assert_eq!(text, format!("{before}\n[mcp_servers.labcraft]\ncommand = \"{EXE}\"\nargs = [\"mcp\"]\n"));
    let (text, change) = toml("model = \"o3\"\n");
    assert_eq!(change, Change::Added);
    assert_eq!(text, format!("model = \"o3\"\n\n[mcp_servers.labcraft]\ncommand = \"{EXE}\"\nargs = [\"mcp\"]\n"));
}

#[test]
fn codex_keeps_its_line_endings_its_mark_and_its_comments() {
    let (text, _) = toml("\u{feff}model = \"o3\"\r\n\r\n[mcp_servers.other]\r\ncommand = \"o\"\r\n# the end\r\n");
    assert_eq!(
        text,
        format!(
            "\u{feff}model = \"o3\"\r\n\r\n[mcp_servers.other]\r\ncommand = \"o\"\r\n# the end\r\n\r\n[mcp_servers.labcraft]\r\ncommand = \"{EXE}\"\r\nargs = [\"mcp\"]\r\n"
        )
    );
    let before = "[mcp_servers.labcraft]\ncommand = \"C:/old/labcraft-cli.exe\" # mine\nargs = [\"mcp\"]\n";
    let (text, change) = toml(before);
    assert_eq!(change, Change::Updated);
    assert_eq!(text, before.replace("C:/old/labcraft-cli.exe", EXE));
    // A table given inline cannot take one after it.
    let mut inline = "mcp_servers = { other = { command = \"o\" } }\n".to_owned();
    assert!(edit_toml(&mut inline, EXE).is_err());
    assert_eq!(inline, "mcp_servers = { other = { command = \"o\" } }\n");
}

#[test]
fn codex_an_old_path_is_updated_and_an_entry_of_the_user_stays() {
    let (text, change) = toml("[mcp_servers.labcraft]\ncommand = \"/old/labcraft-cli\"\nargs = [\"mcp\"]\nenv = { A = \"1\" }\n");
    assert_eq!(change, Change::Updated);
    assert_eq!(text, format!("[mcp_servers.labcraft]\ncommand = \"{EXE}\"\nargs = [\"mcp\"]\nenv = {{ A = \"1\" }}\n"));
    let theirs = "[mcp_servers.labcraft]\ncommand = \"/old/labcraft-cli\"\nargs = [\"mcp\", \"--project\", \"/p\"]\n";
    assert_eq!(toml(theirs), (theirs.to_owned(), Change::Theirs));
    let mut broken = "[mcp_servers\n".to_owned();
    assert!(edit_toml(&mut broken, EXE).is_err());
}

#[test]
fn only_a_labcraft_cli_of_another_folder_is_ours_to_update() {
    let mcp = || Some(vec!["mcp".to_owned()]);
    assert_eq!(judge(Some("C:\\LabCraft\\LABCRAFT-CLI.EXE".to_owned()), mcp(), EXE), Change::Updated);
    assert_eq!(judge(Some("/usr/bin/labcraft".to_owned()), mcp(), EXE), Change::Theirs);
    // Found on the `PATH`: the user's choice.
    assert_eq!(judge(Some("labcraft-cli".to_owned()), mcp(), EXE), Change::Theirs);
    assert_eq!(judge(None, mcp(), EXE), Change::Theirs);
}

#[test]
fn the_last_of_two_keys_is_the_one_read() {
    let before = "{\"mcpServers\": {\"labcraft\": {\"command\": \"node\", \"args\": [\"mcp\"]}, \"labcraft\": {\"command\": \"/old/labcraft-cli\", \"args\": [\"mcp\"]}}}";
    let (text, change) = json(Client::Cursor, before);
    assert_eq!(change, Change::Updated);
    assert_eq!(text, before.replace("/old/labcraft-cli", EXE));
}

#[test]
fn what_is_not_json_is_refused_and_not_edited() {
    for before in [
        "{\"servers\": {\"a\": {\"command\": \"x\"} \"b\": 1}}",
        "{\"x\": NaNxyz, \"servers\": {}}",
        "{\"servers\": {\"a\": -}}",
        "{\"servers\": {\"a\": truex}}",
        "{\"servers\": {\"a\": 01}}",
        "{\"servers\": {\"a\": [1 2]}}",
        "{\u{c}\"servers\": {}}",
        "{, \"servers\": {}}",
    ] {
        let mut text = before.to_owned();
        assert!(edit_json(&mut text, Client::VsCode, EXE).is_err(), "{before}");
        assert_eq!(text, before);
    }
    let (_, change) = json(Client::VsCode, "{\"n\": [-0.5e+3, 0, 12, true, false, null], \"servers\": {},}");
    assert_eq!(change, Change::Added);
}

/// A home folder of a test, with the folders of some clients.
fn home_of(name: &str, folders: &[&str]) -> (PathBuf, Env) {
    let home = std::env::temp_dir().join(format!("labcraft-setup-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    for folder in folders {
        std::fs::create_dir_all(home.join(folder)).unwrap();
    }
    let env = Env { home: Some(home.clone()), appdata: Some(home.join("AppData")), xdg_config: None, codex_home: None };
    (home, env)
}

#[test]
fn setup_writes_for_the_clients_whose_folder_is_there_and_keeps_a_copy() {
    let (home, env) = home_of("found", &[".cursor", ".codex"]);
    std::fs::write(home.join(".cursor").join("mcp.json"), "{}\n").unwrap();
    let dry = setup(&env, EXE, &[], true);
    assert_eq!(
        dry.iter().map(|outcome| (outcome.client, outcome.done.clone())).collect::<Vec<_>>(),
        [(Client::Codex, Ok(Change::Added)), (Client::Cursor, Ok(Change::Added))]
    );
    assert_eq!(std::fs::read_to_string(home.join(".cursor").join("mcp.json")).unwrap(), "{}\n");
    assert!(!home.join(".codex").join("config.toml").exists());

    let done = setup(&env, EXE, &[], false);
    assert!(done.iter().all(|outcome| outcome.done == Ok(Change::Added)), "{done:?}");
    assert_eq!(std::fs::read_to_string(home.join(".cursor").join("mcp.json.bak")).unwrap(), "{}\n");
    assert!(!home.join(".codex").join("config.toml.bak").exists());
    let cursor: Value = serde_json::from_str(&std::fs::read_to_string(home.join(".cursor").join("mcp.json")).unwrap()).unwrap();
    assert_eq!(cursor["mcpServers"]["labcraft"]["command"], json!(EXE));
    let again = setup(&env, EXE, &[Client::Cursor], false);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].done, Ok(Change::Unchanged));
    assert!(report(&again, &[Client::Cursor], false).starts_with("Cursor: `labcraft` is set up already, in "));
    let named = setup(&env, EXE, &[Client::Cursor, Client::Zed], true);
    assert!(report(&named, &[Client::Cursor, Client::Zed], true).ends_with("\nZed: not found on this computer\n"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn setup_leaves_a_file_that_does_not_read_and_says_so() {
    let (home, env) = home_of("broken", &[".gemini"]);
    let file = home.join(".gemini").join("settings.json");
    std::fs::write(&file, "{ not json").unwrap();
    let done = setup(&env, EXE, &[], false);
    assert_eq!(done.len(), 1);
    assert!(done[0].done.is_err());
    assert!(done[0].line(false).starts_with("Gemini CLI: nothing written: "), "{}", done[0].line(false));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ not json");
    assert!(report(&setup(&home_of("none", &[]).1, EXE, &[], false), &[], false).starts_with("No MCP client found on this computer"));
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn a_file_that_is_a_link_is_written_where_it_leads() {
    use std::os::unix::fs::PermissionsExt as _;
    let (home, env) = home_of("link", &[".cursor", "dotfiles"]);
    let real = home.join("dotfiles").join("mcp.json");
    std::fs::write(&real, "{}\n").unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::symlink(&real, home.join(".cursor").join("mcp.json")).unwrap();
    let done = setup(&env, EXE, &[Client::Cursor], false);
    assert_eq!(done[0].done, Ok(Change::Added));
    assert!(std::fs::symlink_metadata(home.join(".cursor").join("mcp.json")).unwrap().file_type().is_symlink());
    assert!(std::fs::read_to_string(&real).unwrap().contains("labcraft"));
    // The copy is the user's alone.
    assert_eq!(std::fs::metadata(home.join("dotfiles").join("mcp.json.bak")).unwrap().permissions().mode() & 0o777, 0o600);
    let _ = std::fs::remove_dir_all(&home);
}
