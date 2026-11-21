//! The settings (`docs/commands.md`, 9.7): the keys, their values and defaults, the two stores, the user's and the
//! project's, and `settings.get` and `settings.set`. The store of the user is the host's to keep (its `settings.toml`,
//! [`read_keys`] and [`write_keys`], or the storage of a web page): the session holds its keys, and says when they change
//! ([`crate::view::Published::settings`], [`Session::on_user_settings`]). The store of the project is
//! `.labcraft/config.toml`, which the session reads and writes itself.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::error::CommandError;
use crate::params::Params;
use crate::session::Session;
use crate::workspace::{Read, Workspace};

/// What a key takes, and its default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bool(bool),
    /// One of the names, the default the second.
    Choice(&'static [&'static str], &'static str),
    /// A table: the name of a library → `"#RRGGBB"` (spec 11.3).
    LibraryColors,
    /// A table: the name of an MCP client → the canonical folder of a project → `ask`, `edits` or `all`, the policy the
    /// user kept for it (ide.md 8).
    Policies,
}

/// A key of the table of commands.md 9.7.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    /// `group.name`.
    pub id: &'static str,
    /// What the `Settings` dialog calls it (ide.md 2).
    pub title: &'static str,
    /// Its line of help.
    pub help: &'static str,
    pub kind: Kind,
    /// Whether a project may hold it (decision of the owner, 2026-11-20).
    pub project: bool,
}

pub const THEME: &str = "appearance.theme";
pub const LOOK: &str = "appearance.look";
pub const REDUCED_MOTION: &str = "appearance.reduced_motion";
pub const BLOCK_FILL: &str = "appearance.block_fill";
pub const LIBRARY_COLORS: &str = "appearance.library_colors";
pub const COLOR_BY_TYPE: &str = "appearance.color_by_type";
pub const STANDARD_EVERYWHERE: &str = "view.standard_everywhere";
pub const VALUES_SHOWN: &str = "view.values_shown";
pub const PREVIEW_TABS: &str = "view.preview_tabs";
pub const DRAG_PANS: &str = "editor.drag_pans";
pub const FIX_EVERY_NODE: &str = "editor.fix_every_node";
pub const MARK_FIXED: &str = "editor.mark_fixed";
pub const REPLACE_WITH_ALT: &str = "editor.replace_with_alt";
pub const ACCEPT_MISMATCH: &str = "editor.accept_mismatch";
pub const ROUTE_WITH_SHIFT_ALT: &str = "editor.route_with_shift_alt";
pub const AUTOPATCH: &str = "editor.autopatch";
pub const INTO_WIRE: &str = "editor.into_wire";
pub const RECORD_VALUES: &str = "run.record_values";
pub const AI_ENABLED: &str = "ai.enabled";
pub const AI_POLICIES: &str = "ai.policies";

/// The policies an agent attached to the IDE may have (ide.md 8), as `ai.policies` writes them.
pub const POLICIES: [&str; 3] = ["ask", "edits", "all"];

/// The groups, in the order of the dialog, each with its title.
pub const GROUPS: &[(&str, &str)] = &[("appearance", "Appearance"), ("view", "View"), ("editor", "Editor"), ("run", "Run"), ("ai", "AI")];

const fn key(id: &'static str, title: &'static str, help: &'static str, kind: Kind, project: bool) -> Key {
    Key { id, title, help, kind, project }
}

/// The keys, in the order of the table of commands.md 9.7.
pub const KEYS: &[Key] = &[
    key(THEME, "Theme", "The theme of the window and of the canvas.", Kind::Choice(&["light", "dark"], "light"), false),
    key(LOOK, "Look", "Studio: graphite and indigo. Lab: slate and teal.", Kind::Choice(&["studio", "lab"], "studio"), false),
    key(REDUCED_MOTION, "Reduced motion", "No glide when nodes move.", Kind::Bool(false), false),
    key(
        BLOCK_FILL,
        "Block fill",
        "Neutral draws every block in the neutral colours, whatever its file writes.",
        Kind::Choice(&["code", "neutral"], "code"),
        false,
    ),
    key(LIBRARY_COLORS, "Library colors", "The colour of a library's blocks, in place of the colour of its mask.", Kind::LibraryColors, false),
    key(
        COLOR_BY_TYPE,
        "Color wires by type",
        "The data wires, their terminals and their names in the colour of their family.",
        Kind::Bool(true),
        false,
    ),
    key(STANDARD_EVERYWHERE, "Standard view everywhere", "Every node in standard view, its terminals named.", Kind::Bool(false), false),
    key(VALUES_SHOWN, "Values on the wires", "The values of the last run on the wires.", Kind::Bool(true), false),
    key(PREVIEW_TABS, "Preview tabs", "A single click on a file opens it as a preview.", Kind::Bool(true), false),
    key(DRAG_PANS, "Drag on the empty canvas pans", "Off, a drag on the empty canvas selects with a rectangle.", Kind::Bool(false), true),
    key(FIX_EVERY_NODE, "Fix every node at the first move", "A move fixes every free node of the function.", Kind::Bool(false), true),
    key(MARK_FIXED, "Mark the fixed nodes", "A pin on each fixed node.", Kind::Bool(true), false),
    key(
        REPLACE_WITH_ALT,
        "Replace a wire only with Alt",
        "A wire dropped on an input that holds something replaces it only with Alt down.",
        Kind::Bool(false),
        true,
    ),
    key(ACCEPT_MISMATCH, "Accept a wire of the wrong type", "The wire is written, and the check reports it.", Kind::Bool(false), true),
    key(ROUTE_WITH_SHIFT_ALT, "Move a wire with Shift+Alt", "In place of Alt, for a desktop that keeps Alt+drag.", Kind::Bool(false), true),
    key(AUTOPATCH, "Autopatch from the keyboard", "Quick insertion wires the new node to the selection.", Kind::Bool(true), true),
    key(INTO_WIRE, "Insert into a wire on a drop", "A node dropped on a wire goes into it.", Kind::Bool(true), true),
    key(RECORD_VALUES, "Record values", "A run records its values, its trace, for the wires.", Kind::Bool(false), false),
    key(AI_ENABLED, "AI agents", "Off, no agent attaches to the window, and the window shows nothing of AI.", Kind::Bool(true), false),
    key(AI_POLICIES, "Policies of the agents", "The policy kept for an MCP client on a project.", Kind::Policies, false),
];

/// Whether a key is one of the settings of AI and of the agents, which an agent neither reads nor writes (ide.md 8).
pub fn of_ai(id: &str) -> bool {
    id.starts_with("ai.")
}

/// The key `id`, if it is one.
pub fn find(id: &str) -> Option<&'static Key> {
    KEYS.iter().find(|key| key.id == id)
}

impl Key {
    pub fn group(&self) -> &'static str {
        self.id.split_once('.').map_or(self.id, |(group, _)| group)
    }

    pub fn name(&self) -> &'static str {
        self.id.split_once('.').map_or(self.id, |(_, name)| name)
    }

    pub fn default_value(&self) -> Value {
        match self.kind {
            Kind::Bool(on) => Value::Bool(on),
            Kind::Choice(_, default) => Value::String(default.to_owned()),
            Kind::LibraryColors | Kind::Policies => Value::Object(Map::new()),
        }
    }

    /// Why `value` is not one of this key, if it is not.
    pub fn refuses(&self, value: &Value) -> Option<String> {
        match self.kind {
            Kind::Bool(_) => (!value.is_boolean()).then(|| format!("`{}` is `true` or `false`", self.id)),
            Kind::Choice(names, _) => (!value.as_str().is_some_and(|name| names.contains(&name)))
                .then(|| format!("`{}` is one of {}", self.id, names.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", "))),
            Kind::LibraryColors => library_colors_refused(value).map(|why| format!("`{}`: {why}", self.id)),
            Kind::Policies => policies_refused(value).map(|why| format!("`{}`: {why}", self.id)),
        }
    }
}

/// At most this many libraries in `appearance.library_colors` *(proposed)*.
const MAX_LIBRARIES: usize = 1_000;

fn library_colors_refused(value: &Value) -> Option<String> {
    let Some(libraries) = value.as_object() else { return Some("a table of libraries".to_owned()) };
    if libraries.len() > MAX_LIBRARIES {
        return Some(format!("at most {MAX_LIBRARIES} libraries"));
    }
    for (library, colour) in libraries {
        if library.is_empty() || library.len() > 256 {
            return Some("the name of a library holds 1 to 256 bytes".to_owned());
        }
        if !colour.as_str().is_some_and(|text| labcraft_graph::Rgb::parse(text).is_some()) {
            return Some(format!("`{library}` is `\"#RRGGBB\"`"));
        }
    }
    None
}

/// At most this many clients, and projects for each, in `ai.policies` *(proposed)*.
const MAX_POLICIES: usize = 1_000;

fn policies_refused(value: &Value) -> Option<String> {
    let Some(clients) = value.as_object() else { return Some("a table of MCP clients".to_owned()) };
    if clients.len() > MAX_POLICIES {
        return Some(format!("at most {MAX_POLICIES} clients"));
    }
    for (client, projects) in clients {
        if client.is_empty() || client.len() > 256 {
            return Some("the name of a client holds 1 to 256 bytes".to_owned());
        }
        let Some(projects) = projects.as_object() else { return Some(format!("`{client}` is a table of projects")) };
        if projects.len() > MAX_POLICIES {
            return Some(format!("at most {MAX_POLICIES} projects for `{client}`"));
        }
        for (project, policy) in projects {
            if project.is_empty() || project.len() > 4096 {
                return Some("the folder of a project holds 1 to 4096 bytes".to_owned());
            }
            if !policy.as_str().is_some_and(|policy| POLICIES.contains(&policy)) {
                return Some(format!("the policy of `{client}` on `{project}` is `ask`, `edits` or `all`"));
            }
        }
    }
    None
}

/// The policy that `ai.policies` keeps for `client` on the project at `project`, if it keeps one.
pub fn kept_policy<'a>(policies: &'a Value, client: &str, project: &str) -> Option<&'a str> {
    policies.get(client)?.get(project)?.as_str().filter(|policy| POLICIES.contains(policy))
}

/// `policies`, a value of `ai.policies`, with `policy` kept for `client` on the project at `project`.
pub fn keep_policy(policies: &Value, client: &str, project: &str, policy: &str) -> Value {
    let mut clients = policies.as_object().cloned().unwrap_or_default();
    let projects = clients.entry(client.to_owned()).or_insert_with(|| json!({}));
    if !projects.is_object() {
        *projects = json!({});
    }
    if let Some(projects) = projects.as_object_mut() {
        projects.insert(project.to_owned(), json!(policy));
    }
    Value::Object(clients)
}

/// `policies` without the policy of `client` on the project at `project`: a client left with none goes, and none left
/// is `null`, which takes the key out.
pub fn forget_policy(policies: &Value, client: &str, project: &str) -> Value {
    let mut clients = policies.as_object().cloned().unwrap_or_default();
    if let Some(projects) = clients.get_mut(client).and_then(Value::as_object_mut) {
        projects.remove(project);
        if projects.is_empty() {
            clients.remove(client);
        }
    }
    if clients.is_empty() { Value::Null } else { Value::Object(clients) }
}

/// The policies that `policies` keeps, each as client, project and policy, in the order of the table: those that
/// [`kept_policy`] would give.
pub fn kept_policies(policies: &Value) -> Vec<(&str, &str, &str)> {
    let Some(clients) = policies.as_object() else { return Vec::new() };
    clients
        .iter()
        .flat_map(|(client, projects)| {
            projects.as_object().into_iter().flatten().filter_map(move |(project, policy)| {
                policy.as_str().filter(|policy| POLICIES.contains(policy)).map(|policy| (client.as_str(), project.as_str(), policy))
            })
        })
        .collect()
}

/// Where the value of a key comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum From {
    Project,
    User,
    Default,
}

impl From {
    pub fn as_str(self) -> &'static str {
        match self {
            From::Project => "project",
            From::User => "user",
            From::Default => "default",
        }
    }
}

/// The keys of the two stores, `group.name` → a value that the key takes: what is wrong in a store is not here.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stores {
    pub user: Map<String, Value>,
    pub project: Map<String, Value>,
    /// The host gave the keys of the user ([`Session::set_user_settings`]): until it does, `user` is not theirs, and the
    /// window gives them.
    pub given: bool,
}

impl Stores {
    /// The keys of the user alone, as a window without a session holds them.
    pub fn of_user(keys: &Map<String, Value>) -> Stores {
        Stores { user: valid_keys(keys), project: Map::new(), given: true }
    }
}

impl Stores {
    /// The value of `key`, and where it comes from: the project, then the user, then the default.
    pub fn get(&self, key: &Key) -> (Value, From) {
        if key.project
            && let Some(value) = self.project.get(key.id)
        {
            return (value.clone(), From::Project);
        }
        match self.user.get(key.id) {
            Some(value) => (value.clone(), From::User),
            None => (key.default_value(), From::Default),
        }
    }

    /// What every key gives, as the window and the drawings read it.
    pub fn values(&self) -> Values {
        let on = |id: &str| find(id).is_some_and(|key| self.get(key).0.as_bool().unwrap_or_default());
        let named = |id: &str| find(id).and_then(|key| self.get(key).0.as_str().map(str::to_owned)).unwrap_or_default();
        let mut library_colors = BTreeMap::new();
        if let Some(key) = find(LIBRARY_COLORS)
            && let Value::Object(libraries) = self.get(key).0
        {
            for (library, colour) in libraries {
                if let Some(colour) = colour.as_str().and_then(labcraft_graph::Rgb::parse) {
                    library_colors.insert(library, colour);
                }
            }
        }
        Values {
            dark: named(THEME) == "dark",
            look: named(LOOK),
            reduced_motion: on(REDUCED_MOTION),
            neutral_blocks: named(BLOCK_FILL) == "neutral",
            library_colors,
            by_type: on(COLOR_BY_TYPE),
            standard_everywhere: on(STANDARD_EVERYWHERE),
            values_shown: on(VALUES_SHOWN),
            preview_tabs: on(PREVIEW_TABS),
            drag_pans: on(DRAG_PANS),
            fix_every_node: on(FIX_EVERY_NODE),
            mark_fixed: on(MARK_FIXED),
            replace_with_alt: on(REPLACE_WITH_ALT),
            accept_mismatch: on(ACCEPT_MISMATCH),
            route_with_shift_alt: on(ROUTE_WITH_SHIFT_ALT),
            autopatch: on(AUTOPATCH),
            into_wire: on(INTO_WIRE),
            record_values: on(RECORD_VALUES),
            ai: on(AI_ENABLED),
        }
    }
}

/// The keys of a store that a host keeps (the user's), those it does not know or whose value is wrong left out.
pub fn valid_keys(keys: &Map<String, Value>) -> Map<String, Value> {
    keys.iter()
        .filter(|(id, value)| find(id).is_some_and(|key| key.refuses(value).is_none()))
        .map(|(id, value)| (id.clone(), value.clone()))
        .collect()
}

/// Writes `value` (`None`: takes the key out) in the keys of the user, as `settings.set` does with no project: the
/// start page, which has no session. What it held, or why the value is refused.
pub fn set_user(keys: &mut Map<String, Value>, id: &str, value: Option<Value>) -> Result<Option<Value>, String> {
    let Some(key) = find(id) else { return Err(format!("no setting has the key `{id}`")) };
    if let Some(value) = &value
        && let Some(why) = key.refuses(value)
    {
        return Err(why);
    }
    Ok(put_in(keys, key.id, value))
}

/// What the keys give, typed.
#[derive(Clone, Debug, PartialEq)]
pub struct Values {
    pub dark: bool,
    pub look: String,
    pub reduced_motion: bool,
    /// `appearance.block_fill` is `neutral`.
    pub neutral_blocks: bool,
    /// The `color` that the user gives the blocks of a library, by its name.
    pub library_colors: BTreeMap<String, labcraft_graph::Rgb>,
    /// `appearance.color_by_type`.
    pub by_type: bool,
    pub standard_everywhere: bool,
    pub values_shown: bool,
    pub preview_tabs: bool,
    pub drag_pans: bool,
    pub fix_every_node: bool,
    pub mark_fixed: bool,
    pub replace_with_alt: bool,
    pub accept_mismatch: bool,
    pub route_with_shift_alt: bool,
    pub autopatch: bool,
    pub into_wire: bool,
    pub record_values: bool,
    /// `ai.enabled`: the switch of AI (ide.md 8).
    pub ai: bool,
}

impl Default for Values {
    fn default() -> Values {
        Stores::default().values()
    }
}

/// The store of the project, `.labcraft/config.toml`: read when the session opens, and again before each write, so
/// that a write changes only its key of the file as it is then.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ProjectFile;

/// The most that `.labcraft/config.toml` holds *(proposed)*.
const MAX_PROJECT_FILE: u64 = 1 << 20;

pub(crate) const PROJECT_FILE: &str = ".labcraft/config.toml";

fn project_path(root: &Path) -> std::path::PathBuf {
    root.join(".labcraft").join("config.toml")
}

impl ProjectFile {
    /// The keys of `.labcraft/config.toml`: none when it is not there, or does not read.
    pub(crate) fn read(ws: &dyn Workspace, root: &Path) -> Map<String, Value> {
        ProjectFile::doc(ws, root).ok().flatten().map(|doc| keys_in(&doc, |key| key.project)).unwrap_or_default()
    }

    /// The file as TOML, none when it is not there; why it does not read, or is not in the project.
    fn doc(ws: &dyn Workspace, root: &Path) -> Result<Option<toml_edit::DocumentMut>, String> {
        let path = project_path(root);
        if ws.link_metadata(&path).is_err() {
            return Ok(None);
        }
        // Never through a link that leads out of the project, as for a write.
        if !crate::runs::in_place(ws, root, PROJECT_FILE) {
            return Err("it leads out of the project through a link".to_owned());
        }
        let text = match ws.read(&path, MAX_PROJECT_FILE) {
            Ok(Read::Bytes(bytes)) => String::from_utf8(bytes).map_err(|_| "it is not UTF-8".to_owned())?,
            Ok(Read::TooLarge(size)) => return Err(format!("it holds {size} bytes, over {MAX_PROJECT_FILE}")),
            Err(error) => return Err(crate::walk::io_text(&error)),
        };
        text.parse::<toml_edit::DocumentMut>().map(Some).map_err(|error| error.message().to_owned())
    }

    /// Writes `value` (`None`: takes the key out) in the file as it is now, the rest of it as it was, and makes
    /// `.labcraft` and its `.gitignore`. The keys the file then holds.
    fn write(ws: &Arc<dyn Workspace>, root: &Path, key: &Key, value: Option<&Value>) -> Result<Map<String, Value>, CommandError> {
        let mut doc = ProjectFile::doc(&**ws, root)
            .map_err(|why| {
                CommandError::new("file.unparsed", format!("`{PROJECT_FILE}` does not read as TOML ({why}): mend it, then set the key again"))
            })?
            .unwrap_or_default();
        put(&mut doc, key, value).map_err(|why| CommandError::new("file.unparsed", format!("`{PROJECT_FILE}`: {why}")))?;
        crate::runs::labcraft_folder(&**ws, root).map_err(|why| CommandError::new("file.unwritable", why))?;
        ws.write(&project_path(root), doc.to_string().as_bytes())
            .map_err(|error| CommandError::new("file.unwritable", format!("`{PROJECT_FILE}` cannot be written: {}", crate::walk::io_text(&error))))?;
        Ok(keys_in(&doc, |key| key.project))
    }
}

/// The keys that `doc` holds, among those `taken` keeps, each with a value it takes.
fn keys_in(doc: &toml_edit::DocumentMut, taken: impl Fn(&Key) -> bool) -> Map<String, Value> {
    let mut keys = Map::new();
    for key in KEYS.iter().filter(|key| taken(key)) {
        let Some(value) = doc.get(key.group()).and_then(|group| group.get(key.name())).and_then(json_of) else { continue };
        if key.refuses(&value).is_none() {
            keys.insert(key.id.to_owned(), value);
        }
    }
    keys
}

/// Writes `value` in `doc` (`None`: takes the key out, and its table when it is left empty), the rest as it was.
fn put(doc: &mut toml_edit::DocumentMut, key: &Key, value: Option<&Value>) -> Result<(), String> {
    match value {
        Some(value) => {
            let Some(item) = toml_of(value) else { return Err(format!("`{}` has no value in TOML", key.id)) };
            // Implicit: a group of tables alone, as `ai`, has no header of its own.
            let group = doc.entry(key.group()).or_insert_with(|| {
                let mut table = toml_edit::Table::new();
                table.set_implicit(true);
                toml_edit::Item::Table(table)
            });
            let Some(group) = group.as_table_like_mut() else {
                return Err(format!("`{}` is not a table: mend it, then set the key again", key.group()));
            };
            // A value that does not change keeps its line as written, its comment with it.
            if group.get(key.name()).and_then(json_of).as_ref() != Some(value) {
                group.insert(key.name(), item);
            }
        }
        None => {
            if let Some(group) = doc.get_mut(key.group()).and_then(toml_edit::Item::as_table_like_mut) {
                group.remove(key.name());
                let commented = doc
                    .get(key.group())
                    .and_then(toml_edit::Item::as_table)
                    .and_then(|table| table.decor().prefix())
                    .and_then(toml_edit::RawString::as_str)
                    .is_some_and(|prefix| prefix.contains('#'));
                if doc.get(key.group()).and_then(toml_edit::Item::as_table_like).is_some_and(toml_edit::TableLike::is_empty) && !commented {
                    doc.remove(key.group());
                }
            }
        }
    }
    Ok(())
}

/// The keys of the user that the text of their `settings.toml` holds (commands.md 9.7), or why it does not read.
pub fn read_keys(text: &str) -> Result<Map<String, Value>, String> {
    let doc = text.parse::<toml_edit::DocumentMut>().map_err(|error| error.message().to_owned())?;
    Ok(keys_in(&doc, |_| true))
}

/// The text of the `settings.toml` of the user once the keys went from `before` to `after`: each key that changed set
/// or taken out, the rest of the file as it was (comments, what LabCraft does not read, and the keys another process
/// wrote meanwhile); or why it does not read, and so is not written.
pub fn write_keys(text: &str, before: &Map<String, Value>, after: &Map<String, Value>) -> Result<String, String> {
    let mut doc = text.parse::<toml_edit::DocumentMut>().map_err(|error| error.message().to_owned())?;
    for key in KEYS {
        let value = after.get(key.id).filter(|value| key.refuses(value).is_none());
        if value != before.get(key.id).filter(|value| key.refuses(value).is_none()) {
            put(&mut doc, key, value)?;
        }
    }
    Ok(doc.to_string())
}

/// A value of TOML as JSON: a boolean, a string, a table of strings (`appearance.library_colors`), or a table of
/// tables of strings (`ai.policies`).
fn json_of(item: &toml_edit::Item) -> Option<Value> {
    json_within(item, 2)
}

fn json_within(item: &toml_edit::Item, tables: u32) -> Option<Value> {
    if let Some(table) = item.as_table_like() {
        let inner = tables.checked_sub(1)?;
        let mut map = Map::new();
        for (name, item) in table.iter() {
            let value = match item.as_str() {
                Some(text) => Value::String(text.to_owned()),
                None => json_within(item, inner).filter(Value::is_object)?,
            };
            map.insert(name.to_owned(), value);
        }
        return Some(Value::Object(map));
    }
    let value = item.as_value()?;
    if let Some(on) = value.as_bool() {
        return Some(Value::Bool(on));
    }
    value.as_str().map(|text| Value::String(text.to_owned()))
}

fn toml_of(value: &Value) -> Option<toml_edit::Item> {
    match value {
        Value::Bool(on) => Some(toml_edit::value(*on)),
        Value::String(text) => Some(toml_edit::value(text.as_str())),
        Value::Object(map) => {
            let mut table = toml_edit::Table::new();
            table.set_implicit(true);
            for (name, value) in map {
                let item = match value {
                    Value::Object(_) => toml_of(value)?,
                    _ => toml_edit::value(value.as_str()?),
                };
                table.insert(name, item);
            }
            Some(toml_edit::Item::Table(table))
        }
        _ => None,
    }
}

/// `settings.get`; `agent`: an agent asks, and the settings of AI are left out.
pub(crate) fn get(session: &Session, params: &Params, agent: bool) -> Result<Value, CommandError> {
    let stores = &session.settings.stores;
    let row = |key: &Key| {
        let (value, from) = stores.get(key);
        json!({ "key": key.id, "value": value, "from": from.as_str(), "project": key.project })
    };
    let rows: Vec<Value> = match params.get("key").and_then(Value::as_str) {
        Some(id) => vec![row(known(id)?)],
        None => KEYS.iter().filter(|key| !(agent && of_ai(key.id))).map(row).collect(),
    };
    Ok(json!({ "settings": rows }))
}

fn known(id: &str) -> Result<&'static Key, CommandError> {
    find(id).ok_or_else(|| {
        let ids: Vec<&str> = KEYS.iter().map(|key| key.id).collect();
        let near: Vec<&str> = labcraft_runtime::simulation::nearest(id, &ids).into_iter().collect();
        CommandError::of("params.invalid", "key", format!("no setting has the key `{id}`")).expecting(&near)
    })
}

/// `settings.set`.
pub(crate) fn set(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let key = known(params.text("key")?)?;
    let value = params.get("value").filter(|value| !value.is_null()).cloned();
    if let Some(value) = &value
        && let Some(why) = key.refuses(value)
    {
        return Err(CommandError::of("params.invalid", "value", why));
    }
    let project = params.get("scope").and_then(Value::as_str) == Some("project");
    if project && !key.project {
        return Err(CommandError::of("params.invalid", "scope", format!("`{}` is the user's: a project may not hold it", key.id)));
    }
    let (scope, before) = if project {
        let ws = session.ws.clone();
        let root = session.root.clone();
        let keys = ProjectFile::write(&ws, &root, key, value.as_ref())?;
        session.wrote_now.push((PROJECT_FILE.to_owned(), None));
        let before = std::mem::replace(&mut session.settings.stores.project, keys).remove(key.id);
        ("project", before)
    } else {
        ("user", write_user(session, key.id, value)?)
    };
    crate::view::settings_changed(session);
    Ok(json!({ "before": before.unwrap_or(Value::Null), "scope": scope }))
}

/// Writes a key of the user (`None` takes it out), kept by the host first: a store that cannot be written is not
/// changed. Gives what the store held.
pub(crate) fn write_user(session: &mut Session, id: &str, value: Option<Value>) -> Result<Option<Value>, CommandError> {
    let mut user = session.settings.stores.user.clone();
    let before = put_in(&mut user, id, value.clone());
    if let Some(keep) = &mut session.settings.keep {
        keep(&session.settings.stores.user, &user)?;
    }
    session.settings.stores.user = user;
    if let Some(drawn) = &mut session.settings.drawn {
        put_in(drawn, id, value);
    }
    Ok(before)
}

fn put_in(keys: &mut Map<String, Value>, id: &str, value: Option<Value>) -> Option<Value> {
    match value {
        Some(value) => keys.insert(id.to_owned(), value),
        None => keys.remove(id),
    }
}

/// Called with the keys of the user before and after each command that changes them: the host writes those that
/// changed, or says why it cannot (`file.unparsed`, `file.unwritable`), and the keys stay as they were.
pub type KeepUser = Box<dyn FnMut(&Map<String, Value>, &Map<String, Value>) -> Result<(), CommandError> + Send>;

/// What a session holds of the settings.
#[derive(Default)]
pub(crate) struct Held {
    pub(crate) stores: Stores,
    pub(crate) keep: Option<KeepUser>,
    /// The keys of the user that the view follows in a session that does not follow their store (the command line):
    /// those that its commands set. None: it follows `stores.user`.
    pub(crate) drawn: Option<Map<String, Value>>,
}

impl Held {
    pub(crate) fn read(ws: &dyn Workspace, root: &Path) -> Held {
        Held { stores: Stores { user: Map::new(), project: ProjectFile::read(ws, root), given: false }, keep: None, drawn: None }
    }

    /// The stores that the view follows.
    pub(crate) fn drawn(&self) -> Stores {
        match &self.drawn {
            Some(user) => Stores { user: user.clone(), ..self.stores.clone() },
            None => self.stores.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_is_a_group_and_a_name_its_default_taken() {
        for key in KEYS {
            // The settings of AI are listed with the switch of AI, not in a group of the dialog (ide.md 8).
            assert!(GROUPS.iter().any(|(group, _)| *group == key.group()) || of_ai(key.id), "{}", key.id);
            assert_eq!(key.refuses(&key.default_value()), None, "{}", key.id);
            assert_eq!(key.id.matches('.').count(), 1, "{}", key.id);
        }
    }

    #[test]
    fn the_project_wins_for_a_key_it_may_hold_only() {
        let mut stores = Stores::default();
        stores.user.insert(DRAG_PANS.to_owned(), json!(false));
        stores.project.insert(DRAG_PANS.to_owned(), json!(true));
        stores.project.insert(THEME.to_owned(), json!("dark"));
        assert_eq!(stores.get(find(DRAG_PANS).unwrap()), (json!(true), From::Project));
        assert_eq!(stores.get(find(THEME).unwrap()), (json!("light"), From::Default));
        assert!(stores.values().drag_pans);
        assert!(!stores.values().dark);
    }

    #[test]
    fn the_file_of_the_user_keeps_its_comments_and_loses_a_key_taken_out() {
        let text = "# mine\n[appearance]\ntheme = \"dark\" # night\nunknown = 1\n\n[editor]\ndrag_pans = true\n";
        let keys = read_keys(text).unwrap();
        assert_eq!(keys.get(THEME), Some(&json!("dark")));
        assert_eq!(keys.get(DRAG_PANS), Some(&json!(true)));
        let mut after = keys.clone();
        after.remove(DRAG_PANS);
        after.insert(LIBRARY_COLORS.to_owned(), json!({ "visa": "#2563EB" }));
        let written = write_keys(text, &keys, &after).unwrap();
        assert!(written.starts_with("# mine\n[appearance]\ntheme = \"dark\" # night\nunknown = 1\n"), "{written}");
        assert!(!written.contains("drag_pans") && !written.contains("[editor]"), "{written}");
        assert_eq!(read_keys(&written).unwrap(), after);
        assert!(write_keys("[appearance", &keys, &after).is_err(), "a file that does not read is not written over");
    }

    /// commands.md 9.7: a line whose value is wrong is ignored where it is written, and a write changes only its key:
    /// those that another process wrote meanwhile stay.
    #[test]
    fn a_write_of_the_user_changes_only_its_key() {
        let text = "[appearance]\ntheme = \"blue\"\nblock_fill = \"neutral\"\n\n[editor]\ndrag_pans = true\n";
        // What this process read: `drag_pans` was written by another one since.
        let before = Map::from_iter([(BLOCK_FILL.to_owned(), json!("neutral"))]);
        let mut after = before.clone();
        after.insert(MARK_FIXED.to_owned(), json!(false));
        let written = write_keys(text, &before, &after).unwrap();
        assert!(written.starts_with("[appearance]\ntheme = \"blue\"\nblock_fill = \"neutral\"\n\n[editor]\ndrag_pans = true\n"), "{written}");
        assert!(written.contains("mark_fixed = false"), "{written}");
    }

    /// Taking out the last key of a table takes the table out, unless a comment stands above it (commands.md 9.7).
    #[test]
    fn a_table_left_empty_stays_under_its_comment() {
        let before = Map::from_iter([(DRAG_PANS.to_owned(), json!(true))]);
        let commented = "# the rules of the team\n[editor]\ndrag_pans = true\n";
        assert_eq!(write_keys(commented, &before, &Map::new()).unwrap(), "# the rules of the team\n[editor]\n");
        assert_eq!(write_keys("[editor]\ndrag_pans = true\n", &before, &Map::new()).unwrap(), "");
    }

    #[test]
    fn library_colors_take_a_colour_each() {
        let key = find(LIBRARY_COLORS).unwrap();
        assert_eq!(key.refuses(&json!({ "visa": "#2563eb" })), None);
        assert!(key.refuses(&json!({ "visa": "blue" })).is_some());
        assert!(key.refuses(&json!({ "visa": { "light": "#2563EB" } })).is_some());
        assert!(key.refuses(&json!({ "": "#2563EB" })).is_some());
        assert!(key.refuses(&json!(["visa"])).is_some());
    }

    #[test]
    fn a_policy_is_kept_and_forgotten_and_the_last_one_takes_the_key_out() {
        let kept = keep_policy(&Value::Null, "claude", "/p", "edits");
        assert_eq!(kept, json!({ "claude": { "/p": "edits" } }));
        let two = keep_policy(&kept, "claude", "/q", "all");
        assert_eq!(kept_policies(&two), [("claude", "/p", "edits"), ("claude", "/q", "all")]);
        assert_eq!(forget_policy(&two, "claude", "/q"), kept);
        assert_eq!(forget_policy(&kept, "claude", "/p"), Value::Null);
        assert_eq!(forget_policy(&kept, "cursor", "/p"), kept, "another client: as it was");
        assert!(kept_policies(&json!({ "claude": { "/p": "sometimes" } })).is_empty(), "only a policy that is one");
    }

    /// `ai.policies` (ide.md 8): a client, a project, a policy; written as tables of tables, and read back the same.
    #[test]
    fn the_policies_kept_are_written_and_read_back() {
        let key = find(AI_POLICIES).unwrap();
        let policies = json!({ "Claude Code": { "/home/ada/bench": "edits", "D:\\bench": "all" }, "cursor": { "/p": "ask" } });
        assert_eq!(key.refuses(&policies), None);
        assert!(key.refuses(&json!({ "claude": { "/p": "everything" } })).is_some());
        assert!(key.refuses(&json!({ "claude": "edits" })).is_some());
        assert!(key.refuses(&json!({ "": { "/p": "ask" } })).is_some());
        let after = Map::from_iter([(AI_POLICIES.to_owned(), policies.clone())]);
        let written = write_keys("[appearance]\ntheme = \"dark\"\n", &Map::new(), &after).unwrap();
        // No header for `ai` nor `ai.policies`: tables of tables only.
        assert!(written.starts_with("[appearance]\ntheme = \"dark\"\n\n[ai.policies.\"Claude Code\"]\n"), "{written}");
        assert_eq!(read_keys(&written).unwrap().get(AI_POLICIES), Some(&policies), "{written}");
        assert_eq!(kept_policy(&policies, "Claude Code", "/home/ada/bench"), Some("edits"));
        assert_eq!(kept_policy(&policies, "Claude Code", "/elsewhere"), None);
        assert!(of_ai(AI_POLICIES) && !of_ai(THEME));
    }
}
