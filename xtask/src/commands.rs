//! `cargo xtask commands`: the registry of `engine` equals the tables of `docs/commands.md`, section 6. The document is
//! the contract with agents, so a command, a parameter, or whether it is required, cannot change on one side only.
//!
//! A row is `` | `id` | parameters | returns | ``. The parameters are a list, separated by commas, of `` `name` ``, or
//! `` *`name`* `` for an optional one, each followed by what it says in parentheses; the list ends at the first item
//! in another form (`` `state`: the view state… ``). A cell that does not start with a parameter describes them in prose,
//! and only the id is compared (`diagram.svg`, whose options are those of `labcraft-cli diagram`).

use std::collections::BTreeSet;

const START: &str = "## 6. The catalog of M0b";

/// A command as the document writes it.
#[derive(Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    /// Each parameter and whether it is required, in order; `None` when the cell is prose.
    pub params: Option<Vec<(String, bool)>>,
}

fn is_table_row(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

fn is_heading(first: &str) -> bool {
    first == "Command" || first.starts_with("---")
}

/// The text with what parentheses hold taken out, so that `` (`false`) `` names no parameter.
fn without_parentheses(text: &str) -> String {
    let mut depth = 0usize;
    let mut kept = String::new();
    let mut in_code = false;
    for c in text.chars() {
        match c {
            '`' => {
                in_code = !in_code;
                if depth == 0 {
                    kept.push(c);
                }
            }
            '(' if !in_code => depth += 1,
            ')' if !in_code && depth > 0 => depth -= 1,
            _ if depth == 0 => kept.push(c),
            _ => {}
        }
    }
    kept
}

/// Splits at the commas outside code.
fn items(text: &str) -> Vec<String> {
    let mut found = vec![String::new()];
    let mut in_code = false;
    for c in text.chars() {
        if c == '`' {
            in_code = !in_code;
        }
        if c == ',' && !in_code {
            found.push(String::new());
        } else if let Some(last) = found.last_mut() {
            last.push(c);
        }
    }
    found.into_iter().map(|item| item.trim().to_owned()).filter(|item| !item.is_empty()).collect()
}

fn is_name(text: &str) -> bool {
    text.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The names in code of an item: `` *`run`* or *`function`* `` names two.
fn names_of(item: &str) -> Vec<String> {
    item.split('`').skip(1).step_by(2).filter(|name| is_name(name)).map(str::to_owned).collect()
}

fn read_params(cell: &str) -> Result<Option<Vec<(String, bool)>>, String> {
    let cell = without_parentheses(cell);
    let cell = cell.trim();
    if cell.is_empty() {
        return Ok(Some(Vec::new()));
    }
    if !cell.starts_with('`') && !cell.starts_with('*') {
        return Ok(None);
    }
    let mut params = Vec::new();
    for item in items(cell) {
        if !item.starts_with('`') && !item.starts_with('*') {
            break;
        }
        // What follows a `:` describes the parameter.
        let head = item.split_once("`:").map_or(item.as_str(), |(head, _)| head);
        let names = names_of(&format!("{head}`"));
        if names.is_empty() {
            return Err(format!("cannot read the parameter `{item}`"));
        }
        let optional = item.starts_with('*');
        params.extend(names.into_iter().map(|name| (name, !optional)));
    }
    Ok(Some(params))
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

/// A command of the registry: its id, and its parameters with whether each is required, in order.
pub type Registered = (String, Vec<(String, bool)>);

pub fn in_registry() -> Vec<Registered> {
    labcraft_engine::registry::commands()
        .iter()
        .map(|command| {
            let required = command.required();
            (command.id.to_owned(), command.parameters().into_iter().map(|name| (name.to_owned(), required.contains(&name))).collect())
        })
        .collect()
}

pub fn check(documented: &[Row], registered: &[Registered]) -> Vec<String> {
    let mut violations = Vec::new();
    let mut tools = BTreeSet::new();
    for (id, params) in registered {
        if !tools.insert(id.replace('.', "_")) {
            violations.push(format!("{id}: gives the MCP tool name of another command (docs/mcp.md, section 2)"));
        }
        let Some(row) = documented.iter().find(|row| row.id == *id) else {
            violations.push(format!("{id}: in the registry, not in docs/commands.md"));
            continue;
        };
        if let Some(written) = &row.params
            && written != params
        {
            let show = |params: &[(String, bool)]| {
                params.iter().map(|(name, required)| if *required { name.clone() } else { format!("*{name}*") }).collect::<Vec<_>>().join(", ")
            };
            violations.push(format!("{id}: parameters `{}` in the registry, `{}` in docs/commands.md", show(params), show(written)));
        }
    }
    for row in documented.iter().filter(|row| !registered.iter().any(|(id, _)| *id == row.id)) {
        violations.push(format!("{}: in docs/commands.md, not in the registry", row.id));
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: &str = "\
## 6. The catalog of M0b

| Command | Parameters | Returns |
|---|---|---|
| `file.read` | `path`, *`from`* (1), *`lines`* (all) | The text |
| `run.trace` | *`run`* or *`function`*, *`limit`* (500) | Events |
| `view.set_state` | `function`, `state`: the view state, in part, as `{ \"arms\": 1 }`, each part optional | An entry |
| `diagram.svg` | The options of `labcraft-cli diagram`, less the file | The SVG |
| `project.describe` | | The root |

## 7. Next
";

    fn named(params: &[(&str, bool)]) -> Vec<(String, bool)> {
        params.iter().map(|(name, required)| ((*name).to_owned(), *required)).collect()
    }

    #[test]
    fn the_rows_give_ids_parameters_and_whether_each_is_required() {
        let rows = from_document(DOCUMENT).unwrap();
        assert_eq!(rows[0], Row { id: "file.read".to_owned(), params: Some(named(&[("path", true), ("from", false), ("lines", false)])) });
        assert_eq!(rows[1].params, Some(named(&[("run", false), ("function", false), ("limit", false)])));
        assert_eq!(rows[2].params, Some(named(&[("function", true), ("state", true)])));
        assert_eq!(rows[3].params, None, "prose");
        assert_eq!(rows[4].params, Some(Vec::new()));
    }

    #[test]
    fn a_difference_on_either_side_is_said() {
        let rows = from_document(DOCUMENT).unwrap();
        let registered = vec![
            ("file.read".to_owned(), named(&[("path", true), ("from", true), ("lines", false)])),
            ("run.trace".to_owned(), named(&[("run", false), ("function", false), ("limit", false)])),
            ("view.set_state".to_owned(), named(&[("function", true), ("state", true)])),
            ("diagram.svg".to_owned(), named(&[("folder", true)])),
            ("file.write".to_owned(), Vec::new()),
        ];
        let violations = check(&rows, &registered);
        assert_eq!(
            violations,
            [
                "file.read: parameters `path, from, *lines*` in the registry, `path, *from*, *lines*` in docs/commands.md",
                "file.write: in the registry, not in docs/commands.md",
                "project.describe: in docs/commands.md, not in the registry",
            ]
        );
    }

    #[test]
    fn two_ids_that_give_one_tool_name_are_refused() {
        let registered = vec![("a.b_c".to_owned(), Vec::new()), ("a_b.c".to_owned(), Vec::new())];
        let rows = vec![Row { id: "a.b_c".to_owned(), params: Some(Vec::new()) }, Row { id: "a_b.c".to_owned(), params: Some(Vec::new()) }];
        assert_eq!(check(&rows, &registered), ["a_b.c: gives the MCP tool name of another command (docs/mcp.md, section 2)"]);
    }

    #[test]
    fn a_row_that_does_not_read_is_an_error() {
        assert!(from_document("## 6. The catalog of M0b\n| file.read | `path` | x |\n").is_err());
        assert!(from_document("## 6. The catalog of M0b\n| `a.b` | x | y |\n| `a.b` | x | y |\n").is_err());
    }
}
