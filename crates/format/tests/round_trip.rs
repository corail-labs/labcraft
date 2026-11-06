//! Scenario 5 on the files of the repository (`docs/acceptance-m0.md`): saving again changes nothing.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use labcraft_format::{Lexer, TokenKind, canonicalize, format, read, write};
use labcraft_graph::{Decl, File, Rhs, StatementKind};
use labcraft_types::{Interner, Limits, Report};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn program_files(folder: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            program_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "lcf" || extension == "lct") {
            files.push(path);
        }
    }
}

fn files_under(folders: &[&str]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for folder in folders {
        program_files(&repository().join(folder), &mut files);
    }
    assert!(!files.is_empty());
    files
}

/// The whole files among the code blocks of the spec. A block with `…` is an illustration.
fn spec_files() -> Vec<String> {
    let spec = fs::read_to_string(repository().join("docs/language-spec-v0.md")).unwrap().replace("\r\n", "\n");
    let blocks: Vec<String> = spec
        .split("```text\n")
        .skip(1)
        .filter_map(|rest| rest.split("```").next())
        .filter(|block| block.starts_with("labcraft ") && !block.contains('…'))
        .map(str::to_owned)
        .collect();
    assert!(blocks.len() >= 3, "{} blocks", blocks.len());
    blocks
}

fn formatted(bytes: &[u8]) -> Option<String> {
    let mut names = Interner::new();
    format(bytes, &mut names, &Limits::default()).text
}

#[track_caller]
fn assert_written_back(what: &str, bytes: &[u8]) {
    let text = formatted(bytes).unwrap_or_else(|| panic!("{what}: no text"));
    assert_eq!(text, String::from_utf8_lossy(bytes), "{what}");
}

/// Point 1, with what `examples/` and `docs/rfc/` hold: comments, `panel`, `layout`, `icon`.
#[test]
fn every_canonical_file_is_written_back_byte_for_byte() {
    for file in files_under(&["examples", "libs", "docs/rfc", "tests/acceptance/type-errors"]) {
        assert_written_back(&file.display().to_string(), &fs::read(&file).unwrap());
    }
    for block in spec_files() {
        assert_written_back(block.lines().nth(2).unwrap_or(""), block.as_bytes());
    }
}

#[track_caller]
fn assert_saved_again_changes_nothing(what: &str, bytes: &[u8]) {
    let mut names = Interner::new();
    let reading = read(bytes, &mut names, &Limits::default());
    let Some(file) = reading.file else { return };
    let mut report = Report::new(1000);
    let as_read = lines_of_the_declaration(file.clone(), &names);
    let once = canonicalize(file, &reading.source_map, &mut names, &mut report);
    assert_eq!(as_read, lines_of_the_declaration(once.clone(), &names), "{what}: a line of the declaration changed");
    let text = write(&once, &names, &Limits::default()).unwrap();

    // Point 3, with the ids of the two models renumbered in the same way.
    let reading = read(text.as_bytes(), &mut names, &Limits::default());
    let again = reading.file.unwrap_or_else(|| panic!("{what}: {text}\n{:?}", reading.report.diagnostics()));
    let again = canonicalize(again, &reading.source_map, &mut names, &mut report);
    assert!(once == again, "{what}: the model changed:\n{text}");

    // Point 2.
    assert_eq!(write(&again, &names, &Limits::default()).unwrap(), text, "{what}");
    assert_eq!(comments_of(bytes), comments_of(text.as_bytes()), "{what}");
}

/// A line that moves changes the ids of what it holds, up to the last pattern of a list.
#[test]
fn the_default_values_are_renumbered() {
    let text = "labcraft 1\nuse b\nuse a\nuse a\nfn f(m: M = M.Fast, r: R = R.Value(0 V, [1]), l: L = L { low: 1 V, high: [2] }, n: [R] = [R.None]) -> () {}\n";
    assert_saved_again_changes_nothing("every form of a default value", text.as_bytes());
}

#[test]
fn the_patterns_of_a_list_are_renumbered() {
    let text = b"labcraft 1\n\nfn f() -> (b: i32) {\n    b = match a {\n        1, 2, _ => yield 1\n    }\n    a = g()\n}\n";
    assert_saved_again_changes_nothing("a list of patterns", text);
}

/// The lines of a model without what `fmt` may remove, in no order: ordering them is all that `canonicalize` may do
/// to a declaration, so it loses and invents none of them.
fn lines_of_the_declaration(mut file: File, names: &Interner) -> Vec<String> {
    file.uses.clear();
    file.panel = None;
    file.layout = None;
    let text = write(&file, names, &Limits::default()).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    lines.sort();
    lines
}

/// Each comment with its count, in its one spelling: moving them is allowed, and losing one is not.
fn comments_of(bytes: &[u8]) -> BTreeMap<String, usize> {
    let mut names = Interner::new();
    let mut report = Report::new(1000);
    let text = String::from_utf8_lossy(bytes).into_owned();
    let mut comments = BTreeMap::new();
    for token in Lexer::new(&text, &mut names, &mut report).flatten() {
        if let TokenKind::Comment(comment) | TokenKind::Doc(comment) = token.kind {
            *comments.entry(comment.trim_matches([' ', '\t']).to_owned()).or_insert(0) += 1;
        }
    }
    comments
}

/// Points 2 and 3, on every file that parses: also those that are not canonical, and every cut of the others,
/// which is what a file looks like while it is being written.
#[test]
fn saving_again_changes_nothing_on_every_file_and_every_prefix() {
    for file in files_under(&["examples", "libs", "docs/rfc", "tests/acceptance"]) {
        let bytes = fs::read(&file).unwrap();
        assert_saved_again_changes_nothing(&file.display().to_string(), &bytes);
        for end in 0..bytes.len() {
            assert_saved_again_changes_nothing(&format!("{} cut at {end}", file.display()), &bytes[..end]);
        }
    }
    for block in spec_files() {
        assert_saved_again_changes_nothing("a block of the spec", block.as_bytes());
    }
}

/// The corpus is canonical and has few comments inside bodies, so this one has something to reorder and to lose,
/// at every level.
#[test]
fn saving_again_loses_nothing_when_lines_move() {
    let text = "labcraft 1\nuse b // on b\n// about use\nuse a\n// doc\nfn f(\n// above k\nk: i32, // on k\n) -> (o: i32, // on o\n) { // open\n\
                // above z\nz = g(y) // on z\n// above y\ny = match k { // open y\n// above the arm\n_ => { // open the arm\n// above w\nw = h(v) // on w\n\
                // above v\nv = h() // on v\n// above the yield\nyield w // on the yield\n// end of the arm\n} // after the arm\n// end of the match\n} // on y\n\
                // above x\nx = for i in k, // on i\ncarry c = 1 // on c\n// end of the clauses\n{ // open x\n// above next\nnext c = d // on next\n\
                // above d\nd = h() // on d\n// end of the for\n} // after x\n// above o\no = z // on o\n// end of the body\n} // after f\n\
                // above the panel\npanel {\n// above b\nindicator o: led // on b\n// above a\ncontrol k: led // on a\n// end of the panel\n} // after the panel\n\
                // above the layout\nlayout {\n// above t\nnode z at (2, 2) // on t\n// above u\nnode y at (1, 1) // on u\n// end of the layout\n} // after the layout\n\
                // end of the file\n";
    assert_saved_again_changes_nothing("every comment on a line that moves", text.as_bytes());
    let mut names = Interner::new();
    let result = format(text.as_bytes(), &mut names, &Limits::default());
    let canonical = result.text.unwrap();
    for (earlier, later) in [("y = match", "z = g(y)"), ("v = h()", "w = h(v)"), ("d = h()", "next c = d"), ("z = g(y)", "o = z")] {
        assert!(canonical.find(earlier) < canonical.find(later), "{earlier}, {later}:\n{canonical}");
    }
    assert_eq!(result.report.diagnostics(), []);
}

/// Point 4: CR LF endings, and any number of them at the end of the file.
#[test]
fn line_endings_are_written_as_line_feeds() {
    for file in files_under(&["examples"]) {
        let bytes = fs::read(&file).unwrap();
        let canonical = String::from_utf8(bytes).unwrap();
        let crlf = canonical.replace('\n', "\r\n");
        assert_eq!(formatted(crlf.as_bytes()).unwrap(), canonical, "{}", file.display());
        let many_at_the_end = format!("{canonical}\n\n\n");
        assert_eq!(formatted(many_at_the_end.as_bytes()).unwrap(), canonical, "{}", file.display());
        let none_at_the_end = canonical.trim_end_matches('\n');
        assert_eq!(formatted(none_at_the_end.as_bytes()).unwrap(), canonical, "{}", file.display());
        let with_a_mark = format!("\u{FEFF}{canonical}");
        assert_eq!(formatted(with_a_mark.as_bytes()).unwrap(), canonical, "{}", file.display());
    }
}

fn reverse_lines(file: &mut File) {
    if let Decl::Function(function) = &mut file.decl {
        reverse_body(&mut function.body);
    }
}

/// Every line in the reverse of its order, but `collect`, whose written order means something.
fn reverse_body(body: &mut labcraft_graph::Body) {
    let collects = body.statements.iter().filter(|statement| matches!(statement.kind, StatementKind::Collect(_))).count();
    if collects < 2 {
        body.statements.reverse();
    }
    for statement in &mut body.statements {
        if let StatementKind::Binding { value, .. } = &mut statement.kind {
            match value {
                Rhs::Match(branching) | Rhs::Handle(branching) => {
                    for arm in &mut branching.arms {
                        reverse_body(&mut arm.body);
                    }
                }
                Rhs::For(structure) => reverse_body(&mut structure.body),
                Rhs::Loop(structure) => reverse_body(&mut structure.body),
                Rhs::Call(_) | Rhs::Struct(_) | Rhs::Operand(_) => {}
            }
        }
    }
}

/// The order of lines has no meaning (5.2): every program written the other way round comes back to its one text.
#[test]
fn the_order_of_lines_comes_back_whatever_was_written() {
    for file in files_under(&["examples", "libs", "docs/rfc", "tests/acceptance/type-errors"]) {
        let bytes = fs::read(&file).unwrap();
        let mut names = Interner::new();
        let reading = read(&bytes, &mut names, &Limits::default());
        let mut model = reading.file.unwrap();
        reverse_lines(&mut model);
        let reversed = write(&model, &names, &Limits::default()).unwrap();
        let mut names = Interner::new();
        let text = format(reversed.as_bytes(), &mut names, &Limits::default()).text;
        assert_eq!(text.as_deref(), std::str::from_utf8(&bytes).ok(), "{}\n{reversed}", file.display());
    }
}
