//! `cargo xtask codes`: the enumeration `Code` equals the tables of `docs/cli.md`, section 5. The
//! document is the contract with agents, so a code cannot change on one side only.

use std::collections::BTreeMap;

use labcraft_types::Code;

const START: &str = "## 5. Diagnostic codes";

/// The code and severity of each row, or what is wrong with the document. A line of the section that
/// mentions a code, and every row of a table in it, must be a code in the expected form: otherwise a
/// badly written row would go unseen, and the two lists could differ with this check passing.
pub fn from_document(document: &str) -> Result<BTreeMap<String, String>, String> {
    let mut lines = document.lines().skip_while(|line| *line != START);
    lines.next().ok_or(format!("docs/cli.md: no heading `{START}`"))?;
    let section = lines.take_while(|line| !line.starts_with("## "));
    let mut rows = BTreeMap::new();
    for line in section.filter(|line| (is_table_row(line) && !is_table_heading(line)) || mentions_a_code(line)) {
        let (code, severity) = read_row(line).ok_or(format!("docs/cli.md: cannot read the row `{line}`"))?;
        if rows.insert(code.to_owned(), severity.to_owned()).is_some() {
            return Err(format!("docs/cli.md: {code} appears twice"));
        }
    }
    Ok(rows)
}

fn is_code(text: &str) -> bool {
    text.strip_prefix("LC").is_some_and(|digits| digits.len() == 4 && digits.bytes().all(|byte| byte.is_ascii_digit()))
}

fn mentions_a_code(line: &str) -> bool {
    line.split(|c: char| !c.is_ascii_alphanumeric()).any(is_code)
}

fn is_table_row(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

/// The first two rows of a table: `| Code | Severity |…` and `|---|---|…`.
fn is_table_heading(line: &str) -> bool {
    let first_cell = line.trim_start().trim_start_matches('|').split('|').next().unwrap_or_default().trim();
    first_cell == "Code" || first_cell.starts_with("---")
}

fn read_row(line: &str) -> Option<(&str, &str)> {
    let mut cells = line.strip_prefix('|')?.split('|').map(str::trim);
    let code = cells.next()?.strip_prefix('`')?.strip_suffix('`')?;
    let severity = cells.next()?;
    (is_code(code) && matches!(severity, "error" | "warning")).then_some((code, severity))
}

pub fn check(documented: &BTreeMap<String, String>, codes: &[(&str, &str)]) -> Vec<String> {
    let mut violations = Vec::new();
    for (code, severity) in codes {
        match documented.get(*code) {
            None => violations.push(format!("{code}: in the enumeration Code, not in docs/cli.md")),
            Some(written) if written != severity => {
                violations.push(format!("{code}: {severity} in the enumeration Code, {written} in docs/cli.md"));
            }
            Some(_) => {}
        }
    }
    for code in documented.keys().filter(|code| !codes.iter().any(|(known, _)| known == code)) {
        violations.push(format!("{code}: in docs/cli.md, not in the enumeration Code"));
    }
    violations
}

pub fn in_code() -> Vec<(&'static str, &'static str)> {
    Code::ALL.iter().map(|code| (code.as_str(), code.severity().as_str())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: &str = "\
## 4. Reports

| `LC9999` | error | Not in section 5 |

## 5. Diagnostic codes

Reserved for later specifications: `LC7xxx` for `diagram`.

### Reading

| Code | Severity | Problem | Spec |
|---|---|---|---|
| `LC0001` | error | Path missing | scenario 4 |
| `LC0005` | warning | Bidirectional control, see `LC0003` | 2 |

### 6.1 A subsection does not end the section

| `LC0006` | error | Over the size limit | 6 |

## 6. Left for later

| `LC8888` | error | Not in section 5 |
";

    fn documented() -> BTreeMap<String, String> {
        from_document(DOCUMENT).unwrap()
    }

    #[test]
    fn only_section_5_counts() {
        let rows = documented();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows["LC0005"], "warning");
    }

    #[test]
    fn equal_lists_pass() {
        assert!(check(&documented(), &[("LC0001", "error"), ("LC0005", "warning"), ("LC0006", "error")]).is_empty());
    }

    #[test]
    fn each_difference_is_a_violation() {
        let found = check(&documented(), &[("LC0001", "warning"), ("LC0002", "error"), ("LC0006", "error")]);
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(found[0].contains("warning in the enumeration Code, error in docs/cli.md"), "{found:?}");
        assert!(found[1].contains("LC0002: in the enumeration Code"), "{found:?}");
        assert!(found[2].contains("LC0005: in docs/cli.md"), "{found:?}");
    }

    #[test]
    fn a_document_without_the_section_is_an_error() {
        assert!(from_document("## 4. Reports\n").is_err());
    }

    #[test]
    fn a_code_written_twice_is_an_error() {
        let document = format!("{START}\n| `LC0001` | error | a |\n| `LC0001` | error | b |\n");
        assert!(from_document(&document).unwrap_err().contains("twice"));
    }

    #[test]
    fn a_row_badly_written_is_an_error_not_a_row_left_out() {
        for row in [
            "  | `LC3023` | error | x |",
            "| LC3023 | error | x |",
            "| `LC3023` | fatal | x |",
            "| `LC3023` |",
            "The code `LC3023` is new.",
            "| Problem | see `LC3023` |",
            "| lc3023 | error | x |",
            "| `LC30231` | error | x |",
            "| `LC 3023` | error | x |",
        ] {
            let document = format!("{START}\n{row}\n");
            let error = from_document(&document).unwrap_err();
            assert!(error.contains("cannot read"), "{row}: {error}");
        }
    }

    #[test]
    fn a_row_without_spaces_is_still_a_row() {
        let rows = from_document(&format!("{START}\n|`LC3023`|error|x|\n")).unwrap();
        assert_eq!(rows["LC3023"], "error");
    }

    #[test]
    fn an_empty_list_fails_against_the_enumeration() {
        assert!(!check(&BTreeMap::new(), &in_code()).is_empty());
    }
}
