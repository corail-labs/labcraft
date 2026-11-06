//! Runs the acceptance corpora against `format` and `check`, natively and in WASM, with no command
//! line (`docs/architecture.md`, section 8). So far: the families Encoding, Truncation and Nesting of
//! scenario 4, through `read`, scenario 5, through `format`, the corpus of scenario 2, and what passes 1 to 4 of `check` can
//! say about the stored cases. The other families and the rest of each one come with their steps.

/// A file of a corpus, with its path from the root of the repository.
pub struct Embedded {
    pub path: &'static str,
    pub bytes: &'static [u8],
}

/// A stored case of `tests/acceptance/hostile` or `tests/acceptance/type-errors`, and its `expected.json`.
pub struct Case {
    pub name: &'static str,
    pub files: &'static [Embedded],
    pub expected: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/corpus.rs"));

#[cfg(test)]
#[path = "../../../crates/format/tests/deep/mod.rs"]
mod deep;

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::collections::{BTreeMap, BTreeSet};

    use labcraft_check::{Content, Found, Libraries, LibrarySource, Origin, SourceFile, check};
    use labcraft_format::{STACK_SIZE, canonicalize, format, read, write};
    use labcraft_types::{Code, Interner, Limits, MAX_NESTING, Report};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::deep::{DEEP, Generator};
    use super::{Embedded, HOSTILE, PROGRAMS, TYPE_ERRORS};

    /// In WASM the stack is the one that `.cargo/config.toml` links.
    #[cfg(target_arch = "wasm32")]
    fn on_the_stack_of_a_host(work: impl FnOnce()) {
        work();
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn on_the_stack_of_a_host(work: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new().stack_size(STACK_SIZE).spawn(work).unwrap().join().unwrap();
    }

    fn codes_of(bytes: &[u8], limits: &Limits) -> Vec<Code> {
        let mut names = Interner::new();
        read(bytes, &mut names, limits).report.diagnostics().iter().map(|diagnostic| diagnostic.code()).collect()
    }

    #[test]
    fn runs_on_the_target_it_was_built_for() {
        assert_eq!(usize::BITS == 32, cfg!(target_arch = "wasm32"));
    }

    /// What `fmt` expects of every stored case.
    #[test]
    fn the_stored_hostile_files_give_what_fmt_expects() {
        assert_eq!(HOSTILE.len(), 30);
        for case in HOSTILE {
            let expected: serde_json::Value = serde_json::from_str(case.expected).unwrap();
            let expected = &expected["fmt"];
            let codes: BTreeSet<&str> = expected["codes"].as_array().unwrap().iter().map(|code| code.as_str().unwrap()).collect();
            let mut found = BTreeSet::new();
            let mut errors = 0;
            let mut changes = false;
            for file in case.files {
                let mut names = Interner::new();
                let result = format(file.bytes, &mut names, &Limits::default());
                found.extend(result.report.diagnostics().iter().map(|diagnostic| diagnostic.code().as_str()));
                errors += result.report.errors();
                changes |= result.text.as_ref().is_some_and(|text| text.as_bytes() != file.bytes);
            }
            assert_eq!(found, codes, "{}", case.name);
            assert_eq!(u64::from(errors > 0), expected["exit"].as_u64().unwrap(), "{}", case.name);
            assert_eq!(changes, expected["changes"].as_bool().unwrap(), "{}", case.name);
        }
    }

    /// The stored cases whose codes come with later steps.
    fn later() -> Vec<&'static str> {
        include_str!("../../acceptance/hostile-later.txt").lines().filter(|line| !line.starts_with('#')).collect()
    }

    /// The libraries shipped with LabCraft.
    struct Shipped;

    impl Libraries for Shipped {
        fn find(&self, name: &str, _: Origin) -> Found {
            labcraft_libs::source(name).map_or(Found::NotFound, Found::Library)
        }
    }

    /// The folders under a root, from the paths of its files, with what a walk of them would give.
    #[derive(Default)]
    struct Tree {
        files: Vec<SourceFile>,
        folders: BTreeMap<String, Tree>,
    }

    impl Tree {
        fn into_source(self, name: &str) -> LibrarySource {
            LibrarySource {
                files: self.files,
                sub_libraries: self.folders.into_iter().map(|(name, tree)| tree.into_source(&name)).collect(),
                ..LibrarySource::new(name.to_owned(), name.to_owned(), Origin::Project)
            }
        }
    }

    /// `root` is the start of the path of every file, with its last `/`.
    fn project<'a>(root: &str, files: impl IntoIterator<Item = &'a Embedded>) -> LibrarySource {
        let mut tree = Tree::default();
        for file in files {
            let relative = file.path.strip_prefix(root).unwrap();
            let mut parts: Vec<&str> = relative.split('/').collect();
            parts.pop();
            let mut folder = &mut tree;
            for part in parts {
                folder = folder.folders.entry(part.to_owned()).or_default();
            }
            folder.files.push(SourceFile { shown: relative.to_owned(), content: Content::Bytes(Cow::Borrowed(file.bytes)) });
        }
        tree.into_source("")
    }

    fn checked_codes(root: LibrarySource) -> BTreeSet<&'static str> {
        let mut names = Interner::new();
        let checked = check(root, &Shipped, &mut names, &Limits::default());
        checked.report.diagnostics().iter().map(|diagnostic| diagnostic.code().as_str()).collect()
    }

    /// The half of `expected.json` for `check`, as far as the passes that exist can tell.
    #[test]
    fn the_stored_hostile_files_give_what_check_expects_so_far() {
        for case in HOSTILE {
            let expected: serde_json::Value = serde_json::from_str(case.expected).unwrap();
            let expected = &expected["check"];
            let codes: BTreeSet<&str> = expected["codes"].as_array().unwrap().iter().map(|code| code.as_str().unwrap()).collect();
            let found = checked_codes(project(&format!("tests/acceptance/hostile/{}/", case.name), case.files));
            assert!(found.is_subset(&codes), "{}: {found:?} is not in {codes:?}", case.name);
            if !later().contains(&case.name) {
                assert_eq!(found, codes, "{}", case.name);
            }
        }
    }

    /// What the corpus compares of `related`: the file and the span (`tests/acceptance/README.md`).
    fn places(related: &serde_json::Value) -> Vec<serde_json::Value> {
        related
            .as_array()
            .map_or_else(Vec::new, |list| list.iter().map(|item| serde_json::json!({ "file": item["file"], "span": item["span"] })).collect())
    }

    /// Scenario 2: each program gives its one diagnostic, as the program does, here with no command line.
    #[test]
    fn the_programs_of_scenario_2_give_their_one_diagnostic() {
        assert_eq!(TYPE_ERRORS.len(), 9);
        for case in TYPE_ERRORS {
            let expected: serde_json::Value = serde_json::from_str(case.expected).unwrap();
            let mut names = Interner::new();
            let root = project(&format!("tests/acceptance/type-errors/{}/", case.name), case.files);
            let mut checked = check(root, &Shipped, &mut names, &Limits::default());
            checked.report.finish();
            let document = serde_json::to_value(checked.report.check_document(checked.counts, false)).unwrap();
            assert_eq!(document["schema"], expected["schema"], "{}", case.name);
            assert_eq!(document["status"], expected["status"], "{}", case.name);
            assert_eq!(document["summary"]["errors"], expected["summary"]["errors"], "{}", case.name);
            assert_eq!(document["summary"]["warnings"], expected["summary"]["warnings"], "{}", case.name);
            let (found, wanted) = (document["diagnostics"].as_array().unwrap(), expected["diagnostics"].as_array().unwrap());
            assert_eq!(found.len(), wanted.len(), "{}: {found:?}", case.name);
            for (found, wanted) in found.iter().zip(wanted) {
                for field in ["code", "severity", "file", "span", "function", "node", "wires", "expected", "found"] {
                    assert_eq!(found[field], wanted[field], "{}: {field}", case.name);
                }
                assert_eq!(places(&found["related"]), places(&wanted["related"]), "{}: related", case.name);
                for mention in
                    wanted["fix_mentions"].as_array().into_iter().flatten().chain(Some(&wanted["fix_mentions"])).filter_map(|text| text.as_str())
                {
                    assert!(found["fixes"].to_string().contains(mention), "{}: no fix mentions {mention}", case.name);
                }
            }
        }
    }

    #[test]
    fn the_programs_of_the_repository_are_clean_under_check() {
        for root in ["examples/", "libs/"] {
            let files = PROGRAMS.iter().filter(|program| program.path.starts_with(root));
            assert_eq!(checked_codes(project(root, files)), BTreeSet::new(), "{root}");
        }
    }

    /// Scenario 5, points 1 and 4, on what the WASM build embeds: every program is canonical (`libs/` and
    /// `examples/`, with comments, `panel`, `layout` and `icon`), and comes back from CR LF endings.
    #[test]
    fn every_program_is_written_back_byte_for_byte() {
        for program in PROGRAMS {
            let text = std::str::from_utf8(program.bytes).unwrap();
            let mut names = Interner::new();
            let once = format(program.bytes, &mut names, &Limits::default()).text;
            assert_eq!(once.as_deref(), Some(text), "{}", program.path);
            let crlf = text.replace('\n', "\r\n");
            let again = format(crlf.as_bytes(), &mut names, &Limits::default()).text;
            assert_eq!(again.as_deref(), Some(text), "{}", program.path);
        }
    }

    /// What the tool embeds is what WASM hosts will read, so it must read here too.
    #[test]
    fn the_embedded_libraries_are_canonical() {
        assert!(labcraft_libs::find("visa").is_some() && labcraft_libs::find("scpi").is_some());
        for library in labcraft_libs::LIBRARIES {
            assert!(!library.files.is_empty(), "{}", library.name);
            for file in library.files {
                let mut names = Interner::new();
                let text = format(file.bytes, &mut names, &Limits::default()).text;
                assert_eq!(text.as_deref().map(str::as_bytes), Some(file.bytes), "{}/{}", library.name, file.name);
            }
        }
    }

    /// Scenario 5, points 2 and 3, on every prefix that parses: what comes out reads again to the same model.
    #[test]
    fn saving_again_changes_nothing_on_every_prefix() {
        for program in PROGRAMS {
            for end in 0..=program.bytes.len() {
                let mut names = Interner::new();
                let limits = Limits::default();
                let reading = read(&program.bytes[..end], &mut names, &limits);
                let Some(file) = reading.file else { continue };
                let once = canonicalize(file, &reading.source_map, &mut names, &mut Report::new(1));
                let text = write(&once, &names, &limits).unwrap();
                let reading = read(text.as_bytes(), &mut names, &limits);
                let again = canonicalize(reading.file.unwrap(), &reading.source_map, &mut names, &mut Report::new(1));
                assert!(once == again, "{} cut at {end}", program.path);
                assert_eq!(write(&again, &names, &limits).unwrap(), text, "{} cut at {end}", program.path);
            }
        }
    }

    /// A cut can fall inside a character, a string or a comment. A prefix has a model or an error, never both.
    #[test]
    fn no_prefix_of_a_program_crashes_the_reader() {
        assert!(PROGRAMS.len() > 20, "{} programs", PROGRAMS.len());
        for program in PROGRAMS {
            for end in 0..=program.bytes.len() {
                let mut names = Interner::new();
                let result = read(&program.bytes[..end], &mut names, &Limits::default());
                let has_model = result.file.is_some();
                assert_eq!(has_model, result.report.errors() == 0, "{} cut at {end}", program.path);
                assert!(has_model || end < program.bytes.len(), "{}: {:?}", program.path, result.report.diagnostics());
            }
        }
    }

    fn assert_limit(kind: &str, deep: Generator, depth: u32, limits: &Limits) {
        assert_eq!(codes_of(deep(depth as usize).as_bytes(), limits), [], "{kind}");
        assert_eq!(codes_of(deep(depth as usize + 1).as_bytes(), limits), [Code::Lc0007], "{kind}");
    }

    #[test]
    fn nesting_is_refused_at_any_depth_above_the_limit() {
        on_the_stack_of_a_host(|| {
            for (kind, deep) in DEEP {
                assert_limit(kind, deep, 256, &Limits::default());
                assert_eq!(codes_of(deep(100_000).as_bytes(), &Limits::default()), [Code::Lc0007], "{kind}");
            }
        });
    }

    /// In WASM, a linked stack too small for this traps and ends the run; the stack of the engine gives a
    /// `RangeError`, which fails this test alone.
    #[test]
    fn the_stack_of_a_host_holds_the_ceiling_of_nesting() {
        on_the_stack_of_a_host(|| {
            // At the ceiling, the indentation alone can pass the default limit of size, and `write` must really write.
            let mut limits = Limits::default().with_max_nesting(MAX_NESTING).unwrap();
            limits.max_file_size = u64::MAX;
            for (kind, deep) in DEEP {
                assert_limit(kind, deep, MAX_NESTING, &limits);
                let mut names = Interner::new();
                let text = deep(MAX_NESTING as usize);
                let reading = read(text.as_bytes(), &mut names, &limits);
                let file = reading.file.unwrap();
                let copy = file.clone();
                assert!(file == copy, "{kind}");
                let canonical = canonicalize(file, &reading.source_map, &mut names, &mut Report::new(1));
                assert!(write(&canonical, &names, &limits).is_ok(), "{kind}");
                // `check` walks the body too, on the same stack, after reading it.
                let root = LibrarySource {
                    files: vec![SourceFile { shown: "deep.lcf".to_owned(), content: Content::Bytes(Cow::Owned(text.into_bytes())) }],
                    ..LibrarySource::new(String::new(), String::new(), Origin::Project)
                };
                let checked = check(root, &Shipped, &mut Interner::new(), &limits);
                assert!(!checked.report.diagnostics().iter().any(|diagnostic| diagnostic.code() == Code::Lc0099), "{kind}");
            }
        });
    }

    /// What was really linked, whatever the configuration says: an environment variable RUSTFLAGS replaces
    /// it. The stack comes first in the memory of a WASM build and grows down, so a local of a shallow
    /// function lies just under its size.
    #[cfg(target_arch = "wasm32")]
    #[test]
    fn the_wasm_build_links_the_stack_of_a_host() {
        let local = 0_u8;
        let address = std::ptr::from_ref(std::hint::black_box(&local)) as usize;
        assert!(address > STACK_SIZE / 2 && address <= STACK_SIZE, "a local at {address}, for a stack of {STACK_SIZE}");
    }
}
