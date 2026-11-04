//! The lexer on the files of the repository: the Encoding family of scenario 4
//! (`tests/acceptance/hostile`), and every program, which must read to the end.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use labcraft_format::{Lexer, decode};
use labcraft_types::{Diagnostic, Interner, Limits, Report};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// What decoding and reading the tokens of a file report.
fn lex(bytes: &[u8]) -> Vec<Diagnostic> {
    let (mut names, mut report) = (Interner::new(), Report::new(1000));
    match decode(bytes, &Limits::default()) {
        Err(diagnostic) => report.push(*diagnostic),
        Ok(text) => {
            let errors: Vec<Box<Diagnostic>> = Lexer::new(text, &mut names, &mut report).filter_map(Result::err).collect();
            errors.into_iter().for_each(|error| report.push(*error));
        }
    }
    report.finish();
    report.diagnostics().to_vec()
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

fn programs() -> Vec<PathBuf> {
    let root = repository();
    let mut files = Vec::new();
    for folder in ["examples", "libs", "docs/rfc", "tests/acceptance/type-errors"] {
        program_files(&root.join(folder), &mut files);
    }
    assert!(files.len() > 20, "{} files found", files.len());
    files
}

#[test]
fn the_encoding_family_gives_its_codes() {
    let folder = repository().join("tests/acceptance/hostile");
    let mut cases = 0;
    for entry in fs::read_dir(folder).unwrap() {
        let case = entry.unwrap().path();
        let name = case.file_name().unwrap().to_string_lossy().into_owned();
        if !name.starts_with("encoding-") {
            continue;
        }
        cases += 1;
        let expected: serde_json::Value = serde_json::from_slice(&fs::read(case.join("expected.json")).unwrap()).unwrap();
        // `fmt` reports what comes from reading the file alone. `LC0101`, the header, is for the parser.
        let mut expected: BTreeSet<String> =
            expected["fmt"]["codes"].as_array().unwrap().iter().map(|code| code.as_str().unwrap().to_owned()).collect();
        expected.remove("LC0101");
        let found: BTreeSet<String> =
            lex(&fs::read(case.join("f.lcf")).unwrap()).iter().map(|diagnostic| diagnostic.code().as_str().to_owned()).collect();
        assert_eq!(found, expected, "{name}");
    }
    assert_eq!(cases, 12);
}

#[test]
fn every_program_reads_to_the_end() {
    for file in programs() {
        let found = lex(&fs::read(&file).unwrap());
        assert!(found.is_empty(), "{}: {found:?}", file.display());
    }
}

#[test]
fn no_stored_hostile_file_crashes_the_lexer() {
    let mut files = Vec::new();
    program_files(&repository().join("tests/acceptance/hostile"), &mut files);
    assert!(files.len() > 20);
    for file in files {
        lex(&fs::read(&file).unwrap());
    }
}

/// The part of the Truncation family that this step can run, byte by byte: a cut can fall inside a
/// character, a string or a comment.
#[test]
fn no_prefix_of_a_file_crashes_the_lexer() {
    let mut files = programs();
    program_files(&repository().join("tests/acceptance/hostile"), &mut files);
    for file in files {
        let bytes = fs::read(&file).unwrap();
        for end in 0..=bytes.len() {
            lex(bytes.get(..end).unwrap());
        }
    }
}
