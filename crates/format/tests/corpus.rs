//! `read` on the files of the repository: the stored hostile files and every program, which must read
//! to the end.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use labcraft_format::read;
use labcraft_types::{Interner, Limits};

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

fn programs() -> Vec<PathBuf> {
    let root = repository();
    let mut files = Vec::new();
    for folder in ["examples", "libs", "docs/rfc", "tests/acceptance/type-errors"] {
        program_files(&root.join(folder), &mut files);
    }
    assert!(files.len() > 20, "{} files found", files.len());
    files
}

fn codes_of(bytes: &[u8]) -> BTreeSet<String> {
    let mut names = Interner::new();
    read(bytes, &mut names, &Limits::default()).report.diagnostics().iter().map(|diagnostic| diagnostic.code().as_str().to_owned()).collect()
}

/// What `fmt` reports on a stored case comes from `read`, apart from what canonical form adds at step 5.
#[test]
fn the_stored_hostile_files_give_the_codes_that_fmt_expects() {
    let folder = repository().join("tests/acceptance/hostile");
    let mut cases = 0;
    for entry in fs::read_dir(folder).unwrap() {
        let case = entry.unwrap().path();
        let name = case.file_name().unwrap().to_string_lossy().into_owned();
        cases += 1;
        let expected: serde_json::Value = serde_json::from_slice(&fs::read(case.join("expected.json")).unwrap()).unwrap();
        let mut expected: BTreeSet<String> =
            expected["fmt"]["codes"].as_array().unwrap().iter().map(|code| code.as_str().unwrap().to_owned()).collect();
        expected.remove("LC6002");
        let mut files = Vec::new();
        program_files(&case, &mut files);
        let found: BTreeSet<String> = files.iter().flat_map(|file| codes_of(&fs::read(file).unwrap())).collect();
        assert_eq!(found, expected, "{name}");
    }
    assert_eq!(cases, 30);
}

#[test]
fn every_program_reads_to_the_end() {
    for file in programs() {
        let mut names = Interner::new();
        let result = read(&fs::read(&file).unwrap(), &mut names, &Limits::default());
        assert!(result.file.is_some(), "{}: {:?}", file.display(), result.report.diagnostics());
    }
}

/// The part of the Truncation family at crate level, byte by byte: a cut can fall inside a character, a
/// string or a comment. A prefix has a model, or at least one error, and never both.
#[test]
fn no_prefix_of_a_file_crashes_the_parser() {
    let mut files = programs();
    program_files(&repository().join("tests/acceptance/hostile"), &mut files);
    for file in files {
        let bytes = fs::read(&file).unwrap();
        for end in 0..=bytes.len() {
            let mut names = Interner::new();
            let result = read(bytes.get(..end).unwrap(), &mut names, &Limits::default());
            assert_eq!(result.file.is_some(), result.report.errors() == 0, "{} cut at {end}", file.display());
        }
    }
}
