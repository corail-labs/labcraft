//! The corpora of `tests/acceptance/` are sound, whatever the program gives: each file is what it says it is, and the
//! expected reports use only what `docs/cli.md` defines (`tests/acceptance/README.md`). The other tests compare the
//! program with these reports; these check the reports against the documents.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn cases(family: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(repository().join("tests/acceptance").join(family))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    found.sort();
    assert!(!found.is_empty(), "{family}");
    found
}

fn expected(case: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(case.join("expected.json")).unwrap()).unwrap()
}

/// The codes of the tables of `docs/cli.md`, section 5, with their severity, and whether the row is marked `S2`.
fn codes() -> BTreeMap<String, (String, bool)> {
    let cli = fs::read_to_string(repository().join("docs/cli.md")).unwrap();
    let mut found = BTreeMap::new();
    for line in cli.lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        // `| `LC3001` | error | … | S2 |` splits into an empty cell, the cells, and an empty cell.
        let Some(code) = cells.get(1).and_then(|cell| cell.strip_prefix('`')).and_then(|cell| cell.strip_suffix('`')) else { continue };
        let severity = cells.get(2).copied().unwrap_or("");
        if code.starts_with("LC") && code.len() == 6 && matches!(severity, "error" | "warning") {
            let s2 = cells.len() >= 3 && cells[cells.len() - 2] == "S2";
            found.insert(code.to_owned(), (severity.to_owned(), s2));
        }
    }
    assert!(found.len() > 50, "{} codes in docs/cli.md", found.len());
    found
}

#[test]
fn each_code_of_scenario_2_has_one_case() {
    let s2: BTreeSet<String> = codes().into_iter().filter(|(_, (_, s2))| *s2).map(|(code, _)| code).collect();
    assert_eq!(s2.len(), 9);
    let mut found: Vec<String> =
        cases("type-errors").iter().map(|case| expected(case)["diagnostics"][0]["code"].as_str().unwrap().to_owned()).collect();
    found.sort();
    assert_eq!(found, s2.into_iter().collect::<Vec<_>>());
}

/// The text each span covers, read by hand in each program.
#[test]
fn the_span_points_at_the_fault() {
    let want = [
        ("arms-that-disagree", "yield \"none\""),
        ("carry-whose-type-changes", "next n"),
        ("function-reference-of-the-wrong-signature", "scale"),
        ("implicit-narrowing", "x"),
        ("incompatible-dimensions", "current"),
        ("output-of-the-wrong-dimension", "power"),
        ("unitless-of-another-dimension", "[A]"),
        ("variant-of-another-enumeration", "State.Init"),
        ("wrong-input-type", "v"),
    ];
    let names: Vec<String> = cases("type-errors").iter().map(|case| case.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, want.iter().map(|(name, _)| (*name).to_owned()).collect::<Vec<_>>());
    for (name, text) in want {
        let case = repository().join("tests/acceptance/type-errors").join(name);
        assert_eq!(covered(&case, &expected(&case)["diagnostics"][0]["span"]), text, "{name}");
    }
}

/// The text a span of one line covers in the file of the diagnostic; columns count characters from 1.
fn covered(case: &Path, span: &Value) -> String {
    let file = expected(case)["diagnostics"][0]["file"].as_str().unwrap().to_owned();
    let source = fs::read_to_string(case.join(file)).unwrap();
    let (line, end_line) = (span["start"]["line"].as_u64().unwrap(), span["end"]["line"].as_u64().unwrap());
    assert_eq!(line, end_line, "{}: a span of one line", case.display());
    let text: Vec<char> = source.split('\n').nth(usize::try_from(line).unwrap() - 1).unwrap().chars().collect();
    let column = |position: &Value| usize::try_from(position["column"].as_u64().unwrap()).unwrap() - 1;
    text[column(&span["start"])..column(&span["end"])].iter().collect()
}

/// The end of a wire as `docs/cli.md` 4.3 writes it: `until`, or a source (a name, `#name`, `@n`, `fn`, `next`,
/// `collect`, `yield`) with at most one `.member`.
fn is_terminal(end: &str) -> bool {
    let word = |text: &str| !text.is_empty() && text.chars().all(|c| c.is_alphanumeric() || c == '_');
    if end == "until" {
        return true;
    }
    let (source, member) = end.split_once('.').map_or((end, None), |(source, member)| (source, Some(member)));
    let source_ok = match source.strip_prefix('@') {
        Some(index) => !index.is_empty() && index.chars().all(|c| c.is_ascii_digit()),
        None => word(source.strip_prefix('#').unwrap_or(source)),
    };
    source_ok && member.is_none_or(word)
}

#[test]
fn the_ends_of_a_wire_are_those_of_the_cli() {
    for end in ["v", "#close", "@8.result", "next.n", "fn.x", "collect", "until"] {
        assert!(is_terminal(end), "{end}");
    }
    for end in ["", "@", "@x", "a.b.c", "a b", "#", "x.", "#a.b.c"] {
        assert!(!is_terminal(end), "{end}");
    }
}

#[test]
fn the_reports_of_scenario_2_use_what_the_cli_defines() {
    let codes = codes();
    for case in cases("type-errors") {
        let report = expected(&case);
        assert_eq!((&report["schema"], &report["status"]), (&Value::from("labcraft-check/1"), &Value::from("error")), "{}", case.display());
        assert_eq!(report["summary"]["errors"], 1, "{}", case.display());
        assert_eq!(report["summary"]["warnings"], 0, "{}", case.display());
        let diagnostics = report["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 1, "{}", case.display());
        let diagnostic = &diagnostics[0];
        let code = diagnostic["code"].as_str().unwrap();
        assert_eq!(codes.get(code).map(|(severity, _)| severity.as_str()), diagnostic["severity"].as_str(), "{}", case.display());
        let mut spans = vec![&diagnostic["span"]];
        spans.extend(diagnostic["related"].as_array().into_iter().flatten().map(|related| &related["span"]));
        for span in spans {
            let text = covered(&case, span);
            assert!(!text.trim().is_empty() && text == text.trim(), "{}: span covers {text:?}", case.display());
        }
        let file = diagnostic["file"].as_str().unwrap();
        let source = fs::read_to_string(case.join(file)).unwrap();
        assert!(source.lines().any(|line| Some(line.trim()) == diagnostic["node"]["text"].as_str()), "{}: node text", case.display());
        for wire in diagnostic["wires"].as_array().into_iter().flatten() {
            assert!(wire.as_object().unwrap().keys().all(|key| matches!(key.as_str(), "value" | "from" | "to")), "{}", case.display());
            for end in ["from", "to"].iter().filter_map(|key| wire[*key].as_str()) {
                assert!(is_terminal(end), "{}: `{end}` is not a terminal of docs/cli.md 4.3", case.display());
            }
        }
    }
}

#[test]
fn the_reports_of_the_hostile_files_use_what_the_cli_defines() {
    let codes = codes();
    // The codes `fmt` can give: docs/cli.md, section 2.
    let fmt_codes: BTreeSet<String> = (1..=10)
        .map(|n| format!("LC00{n:02}"))
        .chain(
            ["LC0099", "LC0101", "LC0102", "LC1001", "LC1002", "LC1003", "LC1004", "LC1005", "LC5004", "LC6001", "LC6002", "LC6003"]
                .map(str::to_owned),
        )
        .collect();
    for case in cases("hostile") {
        let report = expected(&case);
        for command in ["check", "fmt"] {
            let part = &report[command];
            let given: Vec<&str> = part["codes"].as_array().unwrap().iter().map(|code| code.as_str().unwrap()).collect();
            for code in &given {
                assert!(codes.contains_key(*code), "{}: {code}", case.display());
            }
            let errors = given.iter().any(|code| codes.get(*code).is_some_and(|(severity, _)| severity == "error"));
            // Exit 1 exactly when an error is reported: docs/cli.md, section 1.
            assert_eq!(part["exit"], u8::from(errors), "{} {command}", case.display());
        }
        for code in report["fmt"]["codes"].as_array().unwrap() {
            assert!(fmt_codes.contains(code.as_str().unwrap()), "{}: {code}", case.display());
        }
        assert!(report["fmt"].get("changes").is_some(), "{}", case.display());
    }
}

/// `.gitattributes` keeps these bytes as they are: a checkout that converted them would test nothing.
#[test]
fn the_bytes_of_the_hostile_files_are_stored_as_they_are() {
    let hostile = repository().join("tests/acceptance/hostile");
    assert!(fs::read(hostile.join("encoding-crlf/f.lcf")).unwrap().windows(2).any(|pair| pair == b"\r\n"));
    assert!(fs::read(hostile.join("encoding-bom/f.lcf")).unwrap().starts_with(b"\xef\xbb\xbf"));
    assert!(fs::read(hostile.join("encoding-binary/f.lcf")).unwrap().starts_with(b"\x00\xff"));
}
