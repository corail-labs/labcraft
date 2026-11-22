//! `labcraft-cli mcp setup`, and `Connect an AI agent` of the window (ide.md 8; mcp.md 9): the entry `labcraft` of the
//! MCP server written in the configuration of each MCP client found on this computer, so that an agent started in a
//! project serves it with `labcraft-cli mcp`, and attaches to its window when one shows it.
//!
//! A client is found when its folder of configuration is there; no program of a client is started. Only the entry
//! `labcraft` changes: every other byte of the file (other servers, settings, comments, order) stays as it was. A file
//! is replaced at once, its version before kept beside it as `NAME.bak`, and only when it has not changed since it was
//! read; one that does not read is left alone. An entry the user changed (another program, other arguments) is left as
//! it is; one that runs a `labcraft-cli` from elsewhere, as after an update moved it, gets the path of this one.

mod jsonc;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The name of the server in each configuration.
pub const ENTRY: &str = "labcraft";

/// The arguments of the server: `labcraft-cli mcp`, which serves the folder the client starts it in (mcp.md 1).
const ARGS: [&str; 1] = ["mcp"];

/// The largest configuration read *(proposed)*: Claude Code keeps its history in its file.
const MAX_BYTES: u64 = 64 << 20;

/// The MCP clients that `setup` knows (decision of the owner, 2026-11-22): those that start a server in a folder of the
/// user's, a project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Client {
    ClaudeCode,
    Codex,
    Copilot,
    Cursor,
    Gemini,
    OpenCode,
    VsCode,
    Windsurf,
    Zed,
}

impl Client {
    pub const ALL: [Client; 9] = [
        Client::ClaudeCode,
        Client::Codex,
        Client::Copilot,
        Client::Cursor,
        Client::Gemini,
        Client::OpenCode,
        Client::VsCode,
        Client::Windsurf,
        Client::Zed,
    ];

    /// Its name for `--client`.
    pub fn name(self) -> &'static str {
        match self {
            Client::ClaudeCode => "claude-code",
            Client::Codex => "codex",
            Client::Copilot => "copilot",
            Client::Cursor => "cursor",
            Client::Gemini => "gemini",
            Client::OpenCode => "opencode",
            Client::VsCode => "vscode",
            Client::Windsurf => "windsurf",
            Client::Zed => "zed",
        }
    }

    /// Its name for a person.
    pub fn label(self) -> &'static str {
        match self {
            Client::ClaudeCode => "Claude Code",
            Client::Codex => "Codex",
            Client::Copilot => "GitHub Copilot CLI",
            Client::Cursor => "Cursor",
            Client::Gemini => "Gemini CLI",
            Client::OpenCode => "OpenCode",
            Client::VsCode => "VS Code",
            Client::Windsurf => "Windsurf",
            Client::Zed => "Zed",
        }
    }

    pub fn named(name: &str) -> Option<Client> {
        Client::ALL.into_iter().find(|client| client.name() == name)
    }

    /// The key of the top-level object, or table, that holds its servers.
    fn servers_key(self) -> &'static str {
        match self {
            Client::ClaudeCode | Client::Copilot | Client::Cursor | Client::Gemini | Client::Windsurf => "mcpServers",
            Client::Codex => "mcp_servers",
            Client::OpenCode => "mcp",
            Client::VsCode => "servers",
            Client::Zed => "context_servers",
        }
    }

    /// The entry of the server, as the client spells it.
    fn entry(self, exe: &str) -> Value {
        match self {
            Client::ClaudeCode | Client::VsCode => json!({ "type": "stdio", "command": exe, "args": ARGS }),
            Client::Copilot => json!({ "type": "local", "command": exe, "args": ARGS, "tools": ["*"] }),
            Client::OpenCode => json!({ "type": "local", "command": ([exe].into_iter().chain(ARGS).collect::<Vec<_>>()), "enabled": true }),
            Client::Codex | Client::Cursor | Client::Gemini | Client::Windsurf | Client::Zed => json!({ "command": exe, "args": ARGS }),
        }
    }

    /// Its file of configuration, when the client is on this computer.
    pub fn file(self, env: &Env) -> Option<PathBuf> {
        let home = env.home.as_deref();
        let under = |folder: Option<PathBuf>, file: &str| folder.filter(|folder| folder.is_dir()).map(|folder| folder.join(file));
        match self {
            Client::ClaudeCode => {
                let home = home?;
                (home.join(".claude.json").is_file() || home.join(".claude").is_dir()).then(|| home.join(".claude.json"))
            }
            Client::Codex => under(env.codex_home.clone().or_else(|| home.map(|home| home.join(".codex"))), "config.toml"),
            Client::Copilot => under(home.map(|home| home.join(".copilot")), "mcp-config.json"),
            Client::Cursor => under(home.map(|home| home.join(".cursor")), "mcp.json"),
            Client::Gemini => under(home.map(|home| home.join(".gemini")), "settings.json"),
            Client::OpenCode => {
                let folder = env.config_home().map(|config| config.join("opencode")).filter(|folder| folder.is_dir())?;
                // It reads `opencode.jsonc` too: the one the user has.
                let jsonc = folder.join("opencode.jsonc");
                Some(if !folder.join("opencode.json").exists() && jsonc.is_file() { jsonc } else { folder.join("opencode.json") })
            }
            Client::VsCode => under(env.vscode_user(), "mcp.json"),
            Client::Windsurf => under(home.map(|home| home.join(".codeium").join("windsurf")), "mcp_config.json"),
            Client::Zed => {
                let folder = if cfg!(windows) {
                    env.appdata.as_ref().map(|appdata| appdata.join("Zed"))
                } else {
                    env.config_home().map(|config| config.join("zed"))
                };
                under(folder, "settings.json")
            }
        }
    }
}

/// Where the clients keep their configuration: from the environment of the process, or a folder of a test.
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub home: Option<PathBuf>,
    /// `%APPDATA%`, on Windows.
    pub appdata: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME`.
    pub xdg_config: Option<PathBuf>,
    /// `$CODEX_HOME`.
    pub codex_home: Option<PathBuf>,
}

impl Env {
    pub fn of_the_process() -> Env {
        let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from);
        Env { home: dirs::home_dir(), appdata: var("APPDATA"), xdg_config: var("XDG_CONFIG_HOME"), codex_home: var("CODEX_HOME") }
    }

    /// `$XDG_CONFIG_HOME`, else `~/.config`.
    fn config_home(&self) -> Option<PathBuf> {
        self.xdg_config.clone().or_else(|| self.home.as_ref().map(|home| home.join(".config")))
    }

    /// The folder of the settings of the user of VS Code.
    fn vscode_user(&self) -> Option<PathBuf> {
        if cfg!(windows) {
            self.appdata.as_ref().map(|appdata| appdata.join("Code").join("User"))
        } else if cfg!(target_os = "macos") {
            self.home.as_ref().map(|home| home.join("Library").join("Application Support").join("Code").join("User"))
        } else {
            self.config_home().map(|config| config.join("Code").join("User"))
        }
    }
}

/// What setup did, or would do, to the entry of one client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Added,
    /// The entry runs this `labcraft-cli` already.
    Unchanged,
    /// The entry ran a `labcraft-cli` from elsewhere: it runs this one now.
    Updated,
    /// The entry is the user's (another program, other arguments): left as it is.
    Theirs,
}

/// One client: its file, and what was done there, or why nothing was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub client: Client,
    pub file: PathBuf,
    pub done: Result<Change, String>,
}

impl Outcome {
    /// Its line in what `setup` says.
    pub fn line(&self, dry_run: bool) -> String {
        let file = self.file.display();
        let would = if dry_run { "would " } else { "" };
        let did = match &self.done {
            Ok(Change::Added) => format!("{would}add `{ENTRY}` to {file}"),
            Ok(Change::Updated) => format!("{would}make `{ENTRY}` run this labcraft-cli, in {file}"),
            Ok(Change::Unchanged) => format!("`{ENTRY}` is set up already, in {file}"),
            Ok(Change::Theirs) => format!("`{ENTRY}` is left as you wrote it (it is not a plain entry of labcraft-cli), in {file}"),
            Err(why) => format!("nothing written: {why}"),
        };
        format!("{}: {did}", self.client.label())
    }
}

/// Writes the entry `labcraft`, running `exe`, in the configuration of each client of `clients` that is on this
/// computer (every client known when `clients` is empty), or says what it would do with `dry_run`. One outcome for each
/// client found.
pub fn setup(env: &Env, exe: &str, clients: &[Client], dry_run: bool) -> Vec<Outcome> {
    Client::ALL
        .into_iter()
        .filter(|client| clients.is_empty() || clients.contains(client))
        .filter_map(|client| client.file(env).map(|file| (client, file)))
        .map(|(client, file)| {
            let done = configure(client, &file, exe, dry_run).map_err(|why| format!("{} ({why})", file.display()));
            Outcome { client, file, done }
        })
        .collect()
}

/// What `setup` says, a line for each client, or that none was found.
pub fn report(outcomes: &[Outcome], clients: &[Client], dry_run: bool) -> String {
    if outcomes.is_empty() {
        let looked: Vec<&str> = (if clients.is_empty() { Client::ALL.as_slice() } else { clients }).iter().map(|client| client.label()).collect();
        return format!(
            "No MCP client found on this computer (looked for {}): give a client the command `labcraft-cli mcp` by hand (docs/mcp.md, section 9)\n",
            looked.join(", ")
        );
    }
    let mut text: String = outcomes.iter().map(|outcome| outcome.line(dry_run) + "\n").collect();
    for client in clients.iter().filter(|client| !outcomes.iter().any(|outcome| outcome.client == **client)) {
        text.push_str(&format!("{}: not found on this computer\n", client.label()));
    }
    if !dry_run && outcomes.iter().any(|outcome| matches!(outcome.done, Ok(Change::Added | Change::Updated))) {
        text.push_str("Quit and start again those clients (one left open may write its file again without the entry), then start one in the folder of a project: its agent works there, in the window of that project when one shows it.\n");
    }
    text
}

/// The path of this executable, as a client starts it: absolute, without the `\\?\` that Windows gives a canonical
/// path, which some clients do not take.
pub fn this_executable() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|error| format!("this program cannot find its own path: {error}"))?;
    Ok(plain(&exe.canonicalize().unwrap_or(exe)))
}

/// The `labcraft-cli` installed beside the program `app` (the window), when it is there.
pub fn beside(app: &Path) -> Option<String> {
    let cli = app.parent()?.join(if cfg!(windows) { "labcraft-cli.exe" } else { "labcraft-cli" });
    cli.is_file().then(|| plain(&cli.canonicalize().unwrap_or(cli)))
}

/// A path as text, without the `\\?\` of a canonical path of Windows (but with that of a share, `\\?\UNC\`).
pub fn plain(path: &Path) -> String {
    let text = path.to_string_lossy().into_owned();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with(r"UNC\") => rest.to_owned(),
        _ => text,
    }
}

/// What to do with an entry, from its command and its arguments (`None`: not written as strings).
fn judge(command: Option<String>, args: Option<Vec<String>>, exe: &str) -> Change {
    let (Some(command), Some(args)) = (command, args) else { return Change::Theirs };
    if args != ARGS {
        return Change::Theirs;
    }
    if command == exe {
        return Change::Unchanged;
    }
    // A command without a folder is found on the `PATH`: the user's choice, which stays.
    if !command.contains(['/', '\\']) {
        return Change::Theirs;
    }
    let name = command.rsplit(['/', '\\']).next().unwrap_or(&command).to_ascii_lowercase();
    if name.strip_suffix(".exe").unwrap_or(&name) == "labcraft-cli" { Change::Updated } else { Change::Theirs }
}

/// Reads the file, edits it, and writes it when it changes and has not changed meanwhile.
fn configure(client: Client, file: &Path, exe: &str, dry_run: bool) -> Result<Change, String> {
    // A file that is a link, as in a folder of dotfiles, is written where it leads: the link stays.
    let linked = std::fs::symlink_metadata(file).is_ok_and(|metadata| metadata.file_type().is_symlink());
    let target = if linked { std::fs::canonicalize(file).map_err(|error| format!("the link leads nowhere: {error}"))? } else { file.to_owned() };
    let file = target.as_path();
    let before = read(file)?;
    let mut text = before.clone().unwrap_or_default();
    let change = if client == Client::Codex { edit_toml(&mut text, exe) } else { edit_json(&mut text, client, exe) }?;
    if dry_run || before.as_deref() == Some(text.as_str()) {
        return Ok(change);
    }
    // The client does not share a lock with LabCraft: this narrows the race with its own writes, it cannot exclude one.
    if read(file)? != before {
        return Err("it changed while LabCraft was reading it: try again".to_owned());
    }
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder).map_err(|error| format!("its folder cannot be made: {error}"))?;
    }
    if let Some(before) = &before {
        let mut backup = file.as_os_str().to_owned();
        backup.push(".bak");
        // Only the user reads the copy: the file may hold the keys of a server.
        let private = craft_atomic::WriteOptions::default().with_new_file_permissions(craft_atomic::NewFilePermissions::Private);
        private.atomic_write(Path::new(&backup), before.as_bytes()).map_err(|error| format!("its copy `.bak` cannot be written: {error}"))?;
    }
    craft_atomic::atomic_write(file, text.as_bytes()).map_err(|error| format!("it cannot be written: {error}"))?;
    Ok(change)
}

/// The text of the file, none when there is none.
fn read(file: &Path) -> Result<Option<String>, String> {
    use std::io::Read as _;
    let opened = match std::fs::File::open(file) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        opened => opened.map_err(|error| format!("it cannot be read: {error}"))?,
    };
    // Bounded as it is read: a file that grows meanwhile is not read past the bound.
    let mut text = String::new();
    opened.take(MAX_BYTES + 1).read_to_string(&mut text).map_err(|error| format!("it cannot be read: {error}"))?;
    if text.len() as u64 > MAX_BYTES {
        return Err(format!("it holds over {MAX_BYTES} bytes"));
    }
    Ok(Some(text))
}

/// The entry in a JSON file, with comments or not.
fn edit_json(text: &mut String, client: Client, exe: &str) -> Result<Change, String> {
    let key = client.servers_key();
    if text.trim().is_empty() {
        *text = serde_json::to_string_pretty(&json!({ key: { ENTRY: client.entry(exe) } })).unwrap_or_default() + "\n";
        return Ok(Change::Added);
    }
    let root = jsonc::root(text)?;
    let servers = match root.get(key) {
        Some(member) => jsonc::object_of(text, member).ok_or_else(|| format!("its `{key}` is not an object"))??,
        None => {
            jsonc::insert(text, &root, key, &json!({ ENTRY: client.entry(exe) }));
            return Ok(Change::Added);
        }
    };
    let Some(entry) = servers.get(ENTRY) else {
        jsonc::insert(text, &servers, ENTRY, &client.entry(exe));
        return Ok(Change::Added);
    };
    let Some(Ok(fields)) = jsonc::object_of(text, entry) else { return Ok(Change::Theirs) };
    let Some(command) = fields.get("command") else { return Ok(Change::Theirs) };
    // OpenCode writes the program and its arguments in one array, the others a `command` and its `args`.
    let (program, at, args) = if client == Client::OpenCode {
        let mut all = jsonc::strings(text, command.value.clone()).unwrap_or_default().into_iter();
        let (Some(program), Some(first)) = (all.next(), jsonc::items_at(text, command.value.start).ok().and_then(|items| items.into_iter().next()))
        else {
            return Ok(Change::Theirs);
        };
        (program, first, Some(all.collect()))
    } else {
        let Some(program) = jsonc::string(text, command.value.clone()) else { return Ok(Change::Theirs) };
        (program, command.value.clone(), fields.get("args").and_then(|args| jsonc::strings(text, args.value.clone())))
    };
    let change = judge(Some(program), args, exe);
    if change == Change::Updated {
        jsonc::replace(text, at, &json!(exe));
    }
    Ok(change)
}

/// The entry in Codex's `config.toml`, `[mcp_servers.labcraft]`.
fn edit_toml(text: &mut String, exe: &str) -> Result<Change, String> {
    use toml_edit::{Document, Item};
    let key = Client::Codex.servers_key();
    // A byte order mark is kept, and the spans of the parser count after it.
    let bom = if text.starts_with('\u{feff}') { '\u{feff}'.len_utf8() } else { 0 };
    // The entry, if there is one: its command, its arguments, and where its command is written.
    type Found = Option<(Option<String>, Option<Vec<String>>, Option<std::ops::Range<usize>>)>;
    let entry_of = |text: &str| -> Result<Found, String> {
        let document = Document::parse(text.get(bom..).unwrap_or_default())
            .map_err(|error: toml_edit::TomlError| format!("it does not read as TOML: {}", error.message()))?;
        let servers = document.as_table().get(key);
        if servers.is_some_and(|servers| servers.as_table_like().is_none()) {
            return Err(format!("its `{key}` is not a table"));
        }
        let Some(entry) = servers.and_then(Item::as_table_like).and_then(|servers| servers.get(ENTRY)) else { return Ok(None) };
        let Some(entry) = entry.as_table_like() else { return Ok(Some((None, None, None))) };
        let command = entry.get("command");
        let args = entry.get("args").and_then(Item::as_array).and_then(|args| args.iter().map(|arg| arg.as_str().map(str::to_owned)).collect());
        Ok(Some((command.and_then(Item::as_str).map(str::to_owned), args, command.and_then(Item::span))))
    };
    // A TOML basic string: the escapes of JSON are some of those of TOML, and `serde_json` writes no other.
    let quoted = serde_json::to_string(exe).unwrap_or_default();
    match entry_of(text)? {
        None => {
            // The table is written after what the file holds, which stays as it is; what reads it after must hold it.
            let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
            let mut added = text.clone();
            if !added.is_empty() && !added.ends_with('\n') {
                added.push_str(newline);
            }
            if !added.get(bom..).unwrap_or_default().trim().is_empty() {
                added.push_str(newline);
            }
            added.push_str(&format!("[{key}.{ENTRY}]{newline}command = {quoted}{newline}args = [\"mcp\"]{newline}"));
            match entry_of(&added) {
                Ok(Some((Some(command), Some(args), _))) if command == exe && args == ARGS => {
                    *text = added;
                    Ok(Change::Added)
                }
                _ => Err(format!("its `{key}` cannot take a table `[{key}.{ENTRY}]` after it: add the entry by hand")),
            }
        }
        Some((command, args, span)) => {
            let change = judge(command, args, exe);
            if change == Change::Updated {
                let Some(span) = span else { return Ok(Change::Theirs) };
                text.replace_range(span.start + bom..span.end + bom, &quoted);
            }
            Ok(change)
        }
    }
}

#[cfg(test)]
mod tests;
