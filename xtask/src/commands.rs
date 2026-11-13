//! `cargo xtask commands`: the registry of `engine` equals the tables of `docs/commands.md`, section 6. The document is
//! the contract with agents, so a command, a parameter, whether it is required, or its default, cannot change on one
//! side only.
//!
//! A row is `` | `id` | parameters | returns | ``. The parameters are a list, separated by commas, of `` `name` ``, or
//! `` *`name`* `` for an optional one, each followed by what it says in parentheses, or by `:` and a description
//! (`` `state`: the view state ``); the list ends at the first item in another form, and a parameter after that end is
//! an error. A cell that starts in prose gives its list after its first colon, outside code and parentheses, that a
//! parameter follows (`diagram.svg`: "The options of `labcraft-cli diagram` …: `folder`, …"); prose with no list is an
//! error, so that no row is left unchecked. When the parentheses of a parameter hold only a value in code
//! (`` (`false`) ``, `` (`30 s`) ``) or a number (`(20,000)`), that is its default. Defaults are compared both ways
//! (decision of the owner, 2026-11-13): the registry gives the default the document writes, and a default of the
//! registry is written as a value, so that an agent reads every default in the document.

use std::collections::BTreeSet;

use serde_json::Value;

const START: &str = "## 6. The catalog of M0b";

/// A parameter: its name, whether it is required, and its default when the document writes one as a value.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub required: bool,
    pub default: Option<Value>,
}

/// A command as the document writes it.
#[derive(Debug, PartialEq)]
pub struct Row {
    pub id: String,
    /// Each parameter, in order.
    pub params: Vec<Param>,
}

fn is_table_row(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

fn is_heading(first: &str) -> bool {
    first == "Command" || first.starts_with("---")
}

/// Each character with its byte index, whether it is outside code (a backtick is not), and the depth of the
/// parentheses outside code that hold it, a parenthesis being held by itself. The one scan behind the readers below.
fn scan(text: &str) -> Vec<(usize, char, bool, usize)> {
    let (mut in_code, mut depth) = (false, 0usize);
    text.char_indices()
        .map(|(at, c)| {
            if c == '`' {
                in_code = !in_code;
            }
            let outside = !in_code && c != '`';
            let held = match c {
                '(' if outside => {
                    depth += 1;
                    depth
                }
                ')' if outside && depth > 0 => {
                    depth -= 1;
                    depth + 1
                }
                _ => depth,
            };
            (at, c, outside, held)
        })
        .collect()
}

/// An item without what parentheses hold, so that `` (`false`) `` names no parameter, and what its first parentheses
/// hold, when it has some.
fn split_item(item: &str) -> (String, Option<String>) {
    let (mut bare, mut held, mut closed) = (String::new(), String::new(), false);
    for (_, c, outside, depth) in scan(item) {
        if depth == 0 {
            bare.push(c);
        } else if outside && depth == 1 && c == ')' {
            closed = true;
        } else if !closed && !(outside && depth == 1 && c == '(') {
            held.push(c);
        }
    }
    (bare, closed.then_some(held))
}

/// Splits at the commas outside code and outside parentheses.
fn items(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut start = 0;
    for (at, c, outside, depth) in scan(text) {
        if c == ',' && outside && depth == 0 {
            found.push(text.get(start..at).unwrap_or_default());
            start = at + 1;
        }
    }
    found.push(text.get(start..).unwrap_or_default());
    found.into_iter().map(str::trim).filter(|item| !item.is_empty()).collect()
}

fn is_name(text: &str) -> bool {
    text.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The names in code of an item, each with whether it is optional, that is between stars: `` *`run`* or *`function`* ``
/// names two optional ones.
fn names_of(item: &str) -> Vec<(String, bool)> {
    let parts: Vec<&str> = item.split('`').collect();
    parts
        .iter()
        .enumerate()
        .skip(1)
        .step_by(2)
        .filter(|(_, name)| is_name(name))
        .map(|(at, name)| ((*name).to_owned(), parts.get(at - 1).is_some_and(|before| before.ends_with('*'))))
        .collect()
}

/// Whether an item names a parameter: it starts with `` `name` `` or `` *`name` ``.
fn is_parameter(item: &str) -> bool {
    let rest = item.strip_prefix('*').unwrap_or(item);
    rest.strip_prefix('`').and_then(|rest| rest.split_once('`')).is_some_and(|(name, _)| is_name(name))
}

/// A default written as a value, kept as the document writes it: in code, as a string (`` `false` `` gives "false"),
/// or a number (`20,000`, its thousands grouped by three); `None` for prose. [`same_default`] compares it.
fn default_of(held: &str) -> Option<Value> {
    let held = held.trim();
    if let Some(code) = held.strip_prefix('`').and_then(|rest| rest.strip_suffix('`')).filter(|code| !code.contains('`')) {
        return Some(Value::String(code.to_owned()));
    }
    let mut groups = held.split(',');
    let first = groups.next().unwrap_or_default();
    let grouped = groups.all(|group| group.len() == 3 && group.chars().all(|c| c.is_ascii_digit()));
    if grouped && !first.is_empty() && first.chars().all(|c| c.is_ascii_digit()) && (first.len() <= 3 || !held.contains(',')) {
        return held.replace(',', "").parse::<u64>().ok().map(Value::from);
    }
    None
}

/// Whether the default of the registry is the one the document writes: a string equal to the code, or a value that the
/// code reads as in JSON (`` `false` ``, `` `{}` ``); a number equal to the number.
fn same_default(written: &Value, registry: &Value) -> bool {
    match (written, registry) {
        (Value::String(code), Value::String(text)) => code == text,
        (Value::String(code), value) => serde_json::from_str::<Value>(code).is_ok_and(|read| read == *value),
        (number, value) => number == value,
    }
}

/// The item up to the description of its parameter, the `:` after its name in code (`` `state`: the view state ``).
fn head_of(item: &str) -> &str {
    let mut before = ' ';
    for (at, c, outside, depth) in scan(item) {
        if c == ':' && outside && depth == 0 && before == '`' {
            return item.get(..at).unwrap_or(item);
        }
        before = c;
    }
    item
}

/// Where the list starts in a cell that starts in prose: after the first colon, outside code and parentheses, that a
/// parameter follows.
fn after_prose(cell: &str) -> Option<&str> {
    scan(cell)
        .into_iter()
        .filter(|&(_, c, outside, depth)| c == ':' && outside && depth == 0)
        .map(|(at, ..)| cell.get(at + 1..).unwrap_or_default().trim_start())
        .find(|rest| rest.starts_with('`') || rest.starts_with('*'))
}

fn read_params(cell: &str) -> Result<Vec<Param>, String> {
    let cell = cell.trim();
    if cell.is_empty() {
        return Ok(Vec::new());
    }
    let list = if cell.starts_with('`') || cell.starts_with('*') {
        cell
    } else {
        after_prose(cell).ok_or("the parameters are prose, with no list after a colon")?
    };
    let mut params = Vec::new();
    let mut ended = false;
    for item in items(list) {
        if ended || (!item.starts_with('`') && !item.starts_with('*')) {
            // The list has ended: a parameter written after this would be left unchecked.
            if is_parameter(item) {
                return Err(format!("the parameter `{item}` comes after the end of the list; write it before the description"));
            }
            ended = true;
            continue;
        }
        let (bare, held) = split_item(head_of(item));
        let names = names_of(&bare);
        if names.is_empty() {
            return Err(format!("cannot read the parameter `{item}`"));
        }
        let default = held.as_deref().and_then(default_of);
        params.extend(names.into_iter().map(|(name, optional)| Param { name, required: !optional, default: default.clone() }));
    }
    Ok(params)
}

/// The rows of every table of section 6, or what is wrong with the document.
pub fn from_document(document: &str) -> Result<Vec<Row>, String> {
    let mut lines = document.lines().skip_while(|line| *line != START);
    lines.next().ok_or(format!("docs/commands.md: no heading `{START}`"))?;
    let section = lines.take_while(|line| !line.starts_with("## "));
    let mut rows: Vec<Row> = Vec::new();
    for line in section.filter(|line| is_table_row(line)) {
        let cells: Vec<&str> = line.trim().trim_start_matches('|').trim_end_matches('|').split('|').map(str::trim).collect();
        let first = cells.first().copied().unwrap_or_default();
        if is_heading(first) {
            continue;
        }
        let id = first.strip_prefix('`').and_then(|rest| rest.strip_suffix('`')).filter(|id| id.contains('.'));
        let Some(id) = id else { return Err(format!("docs/commands.md: cannot read the row `{line}`")) };
        let params = read_params(cells.get(1).copied().unwrap_or_default()).map_err(|error| format!("docs/commands.md, `{id}`: {error}"))?;
        if rows.iter().any(|row| row.id == id) {
            return Err(format!("docs/commands.md: `{id}` appears twice"));
        }
        rows.push(Row { id: id.to_owned(), params });
    }
    Ok(rows)
}

/// The commands of the registry, as rows: each parameter with whether it is required and its default, in order.
pub fn in_registry() -> Vec<Row> {
    labcraft_engine::registry::commands()
        .iter()
        .map(|command| {
            let required = command.required();
            let params = command
                .params
                .properties
                .iter()
                .map(|(name, schema)| Param { name: (*name).to_owned(), required: required.contains(name), default: schema.get("default").cloned() })
                .collect();
            Row { id: command.id.to_owned(), params }
        })
        .collect()
}

fn show_default(default: Option<&Value>) -> String {
    default.map_or_else(|| "none".to_owned(), |value| format!("`{value}`"))
}

/// A default as the document writes it: the code, or the number.
fn show_written(default: Option<&Value>) -> String {
    match default {
        Some(Value::String(code)) => format!("`{code}`"),
        Some(number) => number.to_string(),
        None => "none written as a value".to_owned(),
    }
}

/// The names, an optional one between stars.
fn show_params(params: &[Param]) -> String {
    params.iter().map(|param| if param.required { param.name.clone() } else { format!("*{}*", param.name) }).collect::<Vec<_>>().join(", ")
}

pub fn check(documented: &[Row], registered: &[Row]) -> Vec<String> {
    let mut violations = Vec::new();
    let mut tools = BTreeSet::new();
    for Row { id, params } in registered {
        if !tools.insert(id.replace('.', "_")) {
            violations.push(format!("{id}: gives the MCP tool name of another command (docs/mcp.md, section 2)"));
        }
        let Some(row) = documented.iter().find(|row| row.id == *id) else {
            violations.push(format!("{id}: in the registry, not in docs/commands.md"));
            continue;
        };
        let same = |a: &Param, b: &Param| a.name == b.name && a.required == b.required;
        if row.params.len() != params.len() || !row.params.iter().zip(params).all(|(a, b)| same(a, b)) {
            violations.push(format!(
                "{id}: parameters `{}` in the registry, `{}` in docs/commands.md",
                show_params(params),
                show_params(&row.params)
            ));
            continue;
        }
        // Both ways: an agent reads every default in the document (decision of the owner, 2026-11-13).
        for (written, param) in row.params.iter().zip(params) {
            let same = match (&written.default, &param.default) {
                (Some(written), Some(registry)) => same_default(written, registry),
                (None, None) => true,
                _ => false,
            };
            if !same {
                violations.push(format!(
                    "{id}: `{}` defaults to {} in the registry, {} in docs/commands.md",
                    param.name,
                    show_default(param.default.as_ref()),
                    show_written(written.default.as_ref())
                ));
            }
        }
    }
    for row in documented.iter().filter(|row| !registered.iter().any(|known| known.id == row.id)) {
        violations.push(format!("{}: in docs/commands.md, not in the registry", row.id));
    }
    violations
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const DOCUMENT: &str = "\
## 6. The catalog of M0b

| Command | Parameters | Returns |
|---|---|---|
| `file.read` | `path`, *`from`* (1), *`lines`* (all) | The text |
| `run.trace` | *`run`* or *`function`*, *`limit`* (500) | Events |
| `view.set_state` | `function`, `state`: the view state (`{}`), in part, as `{ \"arms\": 1 }`, each part optional | An entry |
| `diagram.svg` | The options (as in cli.md: `x`) of `labcraft-cli diagram`, less the file: `folder`, *`theme`* (`both`), *`max_nodes`* (20,000), *`trace`* (a trace: of one function) | The SVG |
| `project.describe` | | The root |
| `run.start` | `function`, *`args`* (`{}`), *`deny_warnings`* (`false`), *`wait`* (`30 s`), *`timeout`* (`30`), *`count`* (1,2) | A report |
| `run.pick` | `a` or *`b`* | One |

## 7. Next
";

    fn param(name: &str, required: bool, default: Option<Value>) -> Param {
        Param { name: name.to_owned(), required, default }
    }

    fn named(params: &[(&str, bool)]) -> Vec<Param> {
        params.iter().map(|(name, required)| param(name, *required, None)).collect()
    }

    fn registry_of(list: Vec<(String, Vec<Param>)>) -> Vec<Row> {
        list.into_iter().map(|(id, params)| Row { id, params }).collect()
    }

    /// The expected values are read off `DOCUMENT` by hand, from the rules of the module comment.
    #[test]
    fn the_rows_give_ids_parameters_whether_each_is_required_and_defaults_written_as_values() {
        let rows = from_document(DOCUMENT).unwrap();
        assert_eq!(
            rows[0],
            Row {
                id: "file.read".to_owned(),
                params: vec![param("path", true, None), param("from", false, Some(json!(1))), param("lines", false, None)]
            }
        );
        assert_eq!(rows[1].params, vec![param("run", false, None), param("function", false, None), param("limit", false, Some(json!(500)))]);
        // The parentheses of a description are not a default.
        assert_eq!(rows[2].params, named(&[("function", true), ("state", true)]));
        // Prose, then the list after the colon that a parameter follows: not the colon inside the parentheses of the
        // prose, nor that inside the parentheses of `trace`.
        assert_eq!(
            rows[3].params,
            vec![
                param("folder", true, None),
                param("theme", false, Some(json!("both"))),
                param("max_nodes", false, Some(json!(20_000))),
                param("trace", false, None),
            ]
        );
        assert_eq!(rows[4].params, Vec::new());
        // A default in code is kept as written; `1,2` is not a number grouped by thousands.
        assert_eq!(
            rows[5].params,
            vec![
                param("function", true, None),
                param("args", false, Some(json!("{}"))),
                param("deny_warnings", false, Some(json!("false"))),
                param("wait", false, Some(json!("30 s"))),
                param("timeout", false, Some(json!("30"))),
                param("count", false, None),
            ]
        );
        // Each name has its own stars.
        assert_eq!(rows[6].params, named(&[("a", true), ("b", false)]));
    }

    #[test]
    fn a_difference_on_either_side_is_said() {
        let rows = from_document(DOCUMENT).unwrap();
        let registered = registry_of(vec![
            ("file.read".to_owned(), vec![param("path", true, None), param("from", true, None), param("lines", false, None)]),
            ("run.trace".to_owned(), vec![param("run", false, None), param("function", false, None), param("limit", false, Some(json!(500)))]),
            ("view.set_state".to_owned(), named(&[("function", true), ("state", true)])),
            // `trace` missing: what #128 fixed, which the prose of this cell used to hide.
            (
                "diagram.svg".to_owned(),
                vec![param("folder", true, None), param("theme", false, Some(json!("both"))), param("max_nodes", false, Some(json!(20_000)))],
            ),
            ("file.write".to_owned(), Vec::new()),
            (
                "run.start".to_owned(),
                vec![
                    param("function", true, None),
                    param("args", false, None),
                    param("deny_warnings", false, Some(json!(true))),
                    param("wait", false, Some(json!("30 s"))),
                    // `30` in code reads as the number 30, and as the string "30" for a string.
                    param("timeout", false, Some(json!(30))),
                    // A default of the registry that the document does not write as a value.
                    param("count", false, Some(json!(12))),
                ],
            ),
            ("run.pick".to_owned(), named(&[("a", true), ("b", false)])),
        ]);
        let violations = check(&rows, &registered);
        assert_eq!(
            violations,
            [
                "file.read: parameters `path, from, *lines*` in the registry, `path, *from*, *lines*` in docs/commands.md",
                "diagram.svg: parameters `folder, *theme*, *max_nodes*` in the registry, `folder, *theme*, *max_nodes*, *trace*` in docs/commands.md",
                "file.write: in the registry, not in docs/commands.md",
                "run.start: `args` defaults to none in the registry, `{}` in docs/commands.md",
                "run.start: `deny_warnings` defaults to `true` in the registry, `false` in docs/commands.md",
                "run.start: `count` defaults to `12` in the registry, none written as a value in docs/commands.md",
                "project.describe: in docs/commands.md, not in the registry",
            ]
        );
    }

    #[test]
    fn two_ids_that_give_one_tool_name_are_refused() {
        let registered = registry_of(vec![("a.b_c".to_owned(), Vec::new()), ("a_b.c".to_owned(), Vec::new())]);
        let rows = vec![Row { id: "a.b_c".to_owned(), params: Vec::new() }, Row { id: "a_b.c".to_owned(), params: Vec::new() }];
        assert_eq!(check(&rows, &registered), ["a_b.c: gives the MCP tool name of another command (docs/mcp.md, section 2)"]);
    }

    #[test]
    fn a_row_that_does_not_read_is_an_error() {
        assert!(from_document("## 6. The catalog of M0b\n| file.read | `path` | x |\n").is_err());
        assert!(from_document("## 6. The catalog of M0b\n| `a.b` | x | y |\n| `a.b` | x | y |\n").is_err());
        // Prose with no list after a colon is no longer left unchecked.
        let prose = from_document("## 6. The catalog of M0b\n| `a.b` | The options of `c` | y |\n").unwrap_err();
        assert!(prose.contains("no list after a colon"), "{prose}");
        // Nor is a parameter written after the end of the list.
        let after = from_document("## 6. The catalog of M0b\n| `a.b` | `x`, the rest, *`y`* (`1`) | z |\n").unwrap_err();
        assert!(after.contains("comes after the end of the list"), "{after}");
    }
}
