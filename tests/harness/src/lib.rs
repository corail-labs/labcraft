//! Runs the acceptance corpora against `format` and `check`, natively and in WASM, with no command
//! line (`docs/architecture.md`, section 8): the families Truncation and Nesting of scenario 4, through
//! `read`, scenario 5, through `format`, the corpus of scenario 2, and what `format` and `check` say about
//! the stored cases of scenario 4. The families that need files run through the program, in `apps/labcraft-cli/tests`.

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

/// A case of the corpus of runs (`tests/acceptance/runs/`): the parameters of `run.start` in `run.json`, the parts of its
/// report that it asserts in `expected.json`, and the programs and simulation files of its own folder.
pub struct RunCase {
    pub name: &'static str,
    pub files: &'static [Embedded],
    pub run: &'static str,
    pub expected: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/corpus.rs"));

#[cfg(test)]
#[path = "../../../crates/format/tests/deep/mod.rs"]
mod deep;

#[cfg(test)]
mod engine;

#[cfg(test)]
mod routing;

#[cfg(test)]
mod runs;

#[cfg(test)]
mod scenario3;

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::collections::{BTreeMap, BTreeSet};

    use labcraft_check::{Content, Found, Libraries, LibrarySource, Origin, SourceFile, check, check_for_run};
    use labcraft_format::{STACK_SIZE, canonical_order, canonicalize, format, read, write};
    use labcraft_graph::Decl;
    use labcraft_types::{Code, Interner, Limits, MAX_NESTING, Report};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::deep::{DEEP, Generator};
    use super::{DIAGRAMS, Embedded, HOSTILE, PROGRAMS, SNAPSHOTS, TYPE_ERRORS};

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

    /// The half of `expected.json` for `check`.
    #[test]
    fn the_stored_hostile_files_give_what_check_expects() {
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

    /// A run of any function of the repository reaches what it runs, in every library, and nothing refuses it
    /// (`docs/architecture.md`, 10.1): the bodies of `scpi` are typed as the folder is, and every body has its order.
    #[test]
    fn every_function_of_the_repository_can_be_run() {
        for root in ["examples/", "libs/"] {
            let files = PROGRAMS.iter().filter(|program| program.path.starts_with(root));
            let mut names = Interner::new();
            let checked = check_for_run(project(root, files), &Shipped, &mut names, &Limits::default());
            assert_eq!(checked.report.errors() + checked.report.warnings(), 0, "{root}: {:?}", checked.report.diagnostics());
            assert_eq!(checked.bodies.errors() + checked.bodies.warnings(), 0, "{root}: {:?}", checked.bodies.diagnostics());
            let mut runs = 0;
            for (index, file) in checked.files.iter().enumerate() {
                assert_eq!(checked.counts_of(index).errors + checked.counts_of(index).warnings, 0, "{}", file.shown);
                let Decl::Function(function) = &file.file.decl else { continue };
                // These files are canonical: each body has the order it is written in.
                let orders = file.orders.as_ref().unwrap();
                let written: Vec<usize> = (0..function.body.statements.len()).collect();
                assert_eq!(orders.of_function(), Some(Ok(written.as_slice())), "{}", file.shown);
                assert_eq!(canonical_order(&function.body, Some(&function.signature), &names).as_deref(), Ok(written.as_slice()));
                if !checked.libraries[file.library as usize].checked {
                    continue;
                }
                let reach = checked.reach(index);
                assert!(reach.unresolved.is_empty() && !reach.refuses(&checked, true), "{}: {reach:?}", file.shown);
                runs += 1;
            }
            assert!(runs >= 2, "{root}: {runs}");
        }
    }

    /// Worked out by hand from the files: `check_voltage` calls `visa.open`, `scpi.query_f64` and `visa.close`, and
    /// `scpi.query_f64` calls `visa.query`; it uses `Verdict`, and `visa.session` through the signatures of `visa`.
    #[test]
    fn a_run_of_check_voltage_reaches_what_its_files_say() {
        let files = PROGRAMS.iter().filter(|program| program.path.starts_with("examples/"));
        let mut names = Interner::new();
        let checked = check_for_run(project("examples/", files), &Shipped, &mut names, &Limits::default());
        let reach = checked.reach(checked.function_named("check_voltage", &names).unwrap());
        let shown = |files: &[u32]| -> BTreeSet<&str> { files.iter().map(|file| checked.files[*file as usize].shown.as_str()).collect() };
        let functions = [
            "check_voltage.lcf",
            "<labcraft>/visa/open.lcf",
            "<labcraft>/scpi/query_f64.lcf",
            "<labcraft>/visa/close.lcf",
            "<labcraft>/visa/query.lcf",
        ];
        assert_eq!(shown(&reach.functions), functions.into_iter().collect());
        assert_eq!(shown(&reach.types), ["Verdict.lct", "<labcraft>/visa/session.lct"].into_iter().collect());
        // What the manifests say (`libs/visa/labcraft.toml`, `libs/scpi/labcraft.toml`).
        let library = |name: &str| checked.libraries.iter().find(|library| library.name == name).unwrap();
        let texts = |set: &[labcraft_types::Name]| -> BTreeSet<String> { set.iter().map(|name| names.resolve(*name).to_owned()).collect() };
        let visa = library("visa");
        assert_eq!(texts(&visa.always), ["close".to_owned()].into_iter().collect());
        assert_eq!(texts(&visa.resources), ["session".to_owned()].into_iter().collect());
        assert_eq!(visa.categories, ["io.timeout", "visa.invalid_session", "visa.resource_not_found"]);
        let scpi = library("scpi");
        assert!(scpi.always.is_empty() && scpi.resources.is_empty());
        assert_eq!(scpi.categories, ["scpi.instrument_error"]);
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

    /// The IR of every function of the repository (`docs/ir.md`, section 8): it lowers, `ir::verify` finds nothing, and its
    /// text form is its snapshot in `tests/harness/snapshots/ir/`. A change of lowering shows there, in review. With
    /// `LABCRAFT_BLESS=1`, natively, the snapshots are written again; the files they replace are read by hand before a commit.
    #[test]
    fn every_function_of_the_repository_lowers_to_its_snapshot() {
        on_the_stack_of_a_host(|| {
            let mut written = BTreeMap::new();
            for root in ["examples/", "libs/", "docs/rfc/"] {
                let files = PROGRAMS.iter().filter(|program| program.path.starts_with(root));
                let mut names = Interner::new();
                let checked = check_for_run(project(root, files), &Shipped, &mut names, &Limits::default());
                for (index, file) in checked.files.iter().enumerate() {
                    if !checked.libraries[file.library as usize].checked || matches!(file.file.decl, Decl::Type(_)) {
                        continue;
                    }
                    let program = labcraft_ir::lower(&checked, index, &mut names).unwrap_or_else(|error| panic!("{root}{}: {error}", file.shown));
                    let violations = labcraft_ir::verify(&program);
                    assert!(
                        violations.is_empty(),
                        "{root}{}: {violations:?}
{}",
                        file.shown,
                        labcraft_ir::print(&program, &names)
                    );
                    // What `LC4007` bounds is what lowering builds.
                    if let (Decl::Function(function), Some(labcraft_ir::Body::Region(region))) =
                        (&file.file.decl, program.functions.first().map(|function| &function.body))
                    {
                        let counted = labcraft_check::passed_in(function, &file.resolved, u64::MAX).count;
                        assert_eq!(counted, region.passed_in(), "{root}{}: the values passed in", file.shown);
                        let terminals = labcraft_check::terminals(function, &file.resolved, &checked.libraries, &names, u64::MAX).count;
                        assert_eq!(terminals, region.terminals(), "{root}{}: the terminals of the calls", file.shown);
                    }
                    let text = labcraft_ir::print_function(&program, labcraft_ir::FunctionId(0), &names);
                    let stem = file.shown.rsplit_once('.').map_or(file.shown.as_str(), |(stem, _)| stem).replace(char::from(92), "/");
                    written.insert(format!("tests/harness/snapshots/ir/{root}{stem}.ir"), text);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            if std::env::var_os("LABCRAFT_BLESS").is_some() {
                let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                for (path, text) in &written {
                    let path = base.join(path);
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(path, text).unwrap();
                }
                return;
            }
            let stored: BTreeMap<&str, &str> =
                SNAPSHOTS.iter().map(|snapshot| (snapshot.path, std::str::from_utf8(snapshot.bytes).unwrap())).collect();
            let paths: Vec<&str> = written.keys().map(String::as_str).collect();
            assert_eq!(stored.keys().copied().collect::<Vec<_>>(), paths, "snapshots without a function, or functions without a snapshot");
            for (path, text) in &written {
                assert_eq!(stored.get(path.as_str()).copied(), Some(text.as_str()), "{path}: write the snapshots again with LABCRAFT_BLESS=1");
            }
        });
    }

    /// What the runtime makes of every program of the repository (`docs/architecture.md`, 10.3, 10.5): each constant is a
    /// value of its type, which writes as text when its type has one (5.4), and each primitive that a node calls has a body.
    /// A default that Q-21 has not settled is the one constant with no value yet.
    #[test]
    fn every_constant_of_the_repository_is_a_value_and_every_primitive_has_a_body() {
        use labcraft_ir::{ConstId, ConstValue, NodeKind, Region};
        use labcraft_runtime::{Env, Text, Writer, constant};
        on_the_stack_of_a_host(|| {
            let bodies: BTreeSet<(&str, &str)> = labcraft_runtime::primitives::bodies().map(|(library, name)| (library.as_str(), name)).collect();
            let (mut constants, mut primitives) = (0, 0);
            for root in ["examples/", "libs/", "docs/rfc/"] {
                let files = PROGRAMS.iter().filter(|program| program.path.starts_with(root));
                let mut names = Interner::new();
                let checked = check_for_run(project(root, files), &Shipped, &mut names, &Limits::default());
                for (index, file) in checked.files.iter().enumerate() {
                    if !checked.libraries[file.library as usize].checked || matches!(file.file.decl, Decl::Type(_)) {
                        continue;
                    }
                    let program = labcraft_ir::lower(&checked, index, &mut names).unwrap_or_else(|error| panic!("{root}{}: {error}", file.shown));
                    let env = Env { program: &program, names: &names };
                    let viewer = program.functions[0].library;
                    for (id, found) in program.constants.iter().enumerate() {
                        if matches!(found.value, ConstValue::Unsettled) {
                            continue;
                        }
                        let value = constant(env, ConstId(id as u32)).unwrap_or_else(|fault| panic!("{root}{}: constant {id}: {fault}", file.shown));
                        let textual = !matches!(
                            found.value,
                            ConstValue::Function(_) | ConstValue::Primitive { .. } | ConstValue::InvalidHandle | ConstValue::ToTheEnd
                        );
                        if textual {
                            let mut text = Text::new(1 << 20);
                            Writer { env, viewer }
                                .write(&mut text, &value, found.ty, None)
                                .unwrap_or_else(|fault| panic!("{root}{}: the text of constant {id}: {fault}", file.shown));
                        }
                        constants += 1;
                    }
                    let mut pending: Vec<&Region> = program
                        .functions
                        .iter()
                        .filter_map(|function| match &function.body {
                            labcraft_ir::Body::Region(region) => Some(region),
                            labcraft_ir::Body::Native { .. } => None,
                        })
                        .collect();
                    while let Some(region) = pending.pop() {
                        for node in &region.nodes {
                            match &node.kind {
                                NodeKind::Primitive { library, name, .. } => {
                                    let name = names.resolve(*name);
                                    assert!(
                                        bodies.contains(&(library.as_str(), name)),
                                        "{root}{}: no body for {}.{name}",
                                        file.shown,
                                        library.as_str()
                                    );
                                    primitives += 1;
                                }
                                NodeKind::Loop { region, .. } | NodeKind::For { region, .. } => pending.push(region),
                                NodeKind::Match { arms } | NodeKind::Handle { arms } => pending.extend(arms.iter().map(|arm| &arm.region)),
                                _ => {}
                            }
                        }
                    }
                }
            }
            assert!(constants > 100 && primitives > 50, "{constants} constants, {primitives} primitive nodes: the repository reached");
        });
    }

    /// The SVG of every function of the repository (`docs/architecture.md`, 11.6), as its file gives it and with every node
    /// in standard view, compared byte for byte with its snapshot in `tests/harness/snapshots/diagram/`, natively and in
    /// WASM (scenario 6, point 2). With `LABCRAFT_BLESS=1`, natively, the snapshots are written again; the files they
    /// replace are read by hand before a commit, and opened in a browser.
    #[test]
    fn every_function_of_the_repository_draws_its_snapshot() {
        use labcraft_diagram::{DiagramLimits, Drawn, Theme, ViewState, draw, svg};
        on_the_stack_of_a_host(|| {
            let mut written = BTreeMap::new();
            // The rules of routing, on every drawing (notation 2.2).
            let mut faults: Vec<String> = Vec::new();
            for root in ["examples/", "libs/", "docs/rfc/"] {
                let files = PROGRAMS.iter().filter(|program| program.path.starts_with(root));
                let mut names = Interner::new();
                let checked = check(project(root, files), &Shipped, &mut names, &Limits::default());
                for (index, file) in checked.files.iter().enumerate() {
                    if !checked.libraries[file.library as usize].checked || matches!(file.file.decl, Decl::Type(_)) {
                        continue;
                    }
                    let stem = file.shown.rsplit_once('.').map_or(file.shown.as_str(), |(stem, _)| stem).replace(char::from(92), "/");
                    for (suffix, standard_everywhere) in [("", false), (".standard", true)] {
                        let view = ViewState { standard_everywhere, ..ViewState::default() };
                        let Drawn::Drawing(drawing) = draw(&checked, index, &names, &view, &DiagramLimits::default()) else {
                            panic!("{root}{}: not drawn", file.shown)
                        };
                        faults.extend(super::routing::faults(&drawing).into_iter().map(|fault| format!("{root}{stem}{suffix}: {fault}")));
                        written.insert(format!("tests/harness/snapshots/diagram/{root}{stem}{suffix}.svg"), svg(&drawing, Theme::Both));
                    }
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            if std::env::var_os("LABCRAFT_BLESS").is_some() {
                let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                for (path, text) in &written {
                    let path = base.join(path);
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(path, text).unwrap();
                }
            }
            assert!(faults.is_empty(), "the rules of routing (notation 2.2):\n{}", faults.join("\n"));
            #[cfg(not(target_arch = "wasm32"))]
            if std::env::var_os("LABCRAFT_BLESS").is_some() {
                return;
            }
            let stored: BTreeMap<&str, &str> =
                DIAGRAMS.iter().map(|snapshot| (snapshot.path, std::str::from_utf8(snapshot.bytes).unwrap())).collect();
            let paths: Vec<&str> = written.keys().map(String::as_str).collect();
            assert_eq!(stored.keys().copied().collect::<Vec<_>>(), paths, "snapshots without a function, or functions without a snapshot");
            for (path, text) in &written {
                assert_eq!(stored.get(path.as_str()).copied(), Some(text.as_str()), "{path}: write the snapshots again with LABCRAFT_BLESS=1");
            }
        });
    }

    /// Every function of the repository on the canvas of `editor` (architecture 11.4), without a window: light and dark,
    /// its arms unfolded and in tabs, each arm shown, at the three levels of zoom of notation 7. Nothing panics, and no
    /// colour is that of an unknown token.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn every_function_of_the_repository_paints_on_the_canvas() {
        use labcraft_diagram::{DiagramLimits, Drawn, ViewState, draw};
        use labcraft_editor::{Canvas, Shown, Theme};
        on_the_stack_of_a_host(|| {
            let unknown = egui::Color32::from_rgb(255, 0, 255);
            let ctx = egui::Context::default();
            let mut fonts = egui::FontDefinitions::empty();
            fonts.font_data.insert(
                "hack".to_owned(),
                std::sync::Arc::new(egui::FontData::from_static(include_bytes!("../../../assets/fonts/Hack-Regular.ttf"))),
            );
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.insert(family, vec!["hack".to_owned()]);
            }
            ctx.set_fonts(fonts);
            labcraft_editor::install_fonts(&ctx);
            let mut time = 0.0;
            let mut frames = 0usize;
            for root in ["examples/", "libs/", "docs/rfc/"] {
                let files = PROGRAMS.iter().filter(|program| program.path.starts_with(root));
                let mut names = Interner::new();
                let checked = check(project(root, files), &Shipped, &mut names, &Limits::default());
                for (index, file) in checked.files.iter().enumerate() {
                    if !checked.libraries[file.library as usize].checked || matches!(file.file.decl, Decl::Type(_)) {
                        continue;
                    }
                    for tabs in [false, true] {
                        let view = ViewState { tabs, ..ViewState::default() };
                        let Drawn::Drawing(drawing) = draw(&checked, index, &names, &view, &DiagramLimits::default()) else {
                            panic!("{root}{}: not drawn", file.shown)
                        };
                        let drawing = std::sync::Arc::new(drawing);
                        let mut choices = vec![Shown::default()];
                        for arm in &drawing.arms {
                            choices.push(Shown { arms: [(arm.structure.clone(), arm.arm)].into() });
                        }
                        for theme in [Theme::Light, Theme::Dark] {
                            for shown in &choices {
                                for zoom in [1.0, 0.5, 0.3] {
                                    let mut canvas = Canvas::new();
                                    canvas.set_drawing(drawing.clone(), false);
                                    canvas.set_viewport(zoom, egui::Vec2::ZERO);
                                    time += 1.0;
                                    let input = egui::RawInput {
                                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(4000.0, 4000.0))),
                                        time: Some(time),
                                        ..egui::RawInput::default()
                                    };
                                    let mut output = ctx.run_ui(input, |ui| {
                                        canvas.show(ui, theme, shown);
                                    });
                                    let primitives = ctx.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
                                    output.drop_without_applying_deltas();
                                    let mut painted = 0usize;
                                    for primitive in &primitives {
                                        if let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive {
                                            painted += mesh.vertices.len();
                                            assert!(
                                                mesh.vertices.iter().all(|vertex| vertex.color != unknown),
                                                "{root}{}: a colour of an unknown token",
                                                file.shown
                                            );
                                        }
                                    }
                                    assert!(painted > 0, "{root}{}: nothing painted", file.shown);
                                    frames += 1;
                                }
                            }
                        }
                    }
                }
            }
            assert!(frames > 200, "{frames} frames");
        });
    }

    /// The trace of a run, natively and in WASM, where the viewer reads it: what the writer writes the reader reads, the
    /// cuts and the numbers of commands.md 5 are the same text, and the SHA-256 of FIPS 180-2 (B.1) is the same digest.
    #[test]
    fn a_trace_reads_back_the_same_on_every_target() {
        use labcraft_trace::{Event, Expected, ExpectedNode, ExpectedWire, Header, Input, ReadLimits, Reader, TraceWriter, ValueWriter};

        let source = PROGRAMS.iter().find(|program| program.path == "examples/check_voltage.lcf").unwrap().bytes;
        let sha = labcraft_trace::sha256_hex(source);
        assert_eq!(labcraft_trace::sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let files = [("check_voltage.lcf", sha.as_str())];
        let header = Header { run: "r1", function: "check_voltage", mode: "deterministic", files: &files };
        let mut writer = TraceWriter::new(Vec::new(), &header);
        let low = |w: &mut ValueWriter<'_>| w.f64(1.5e-7, Some("V"));
        let inputs = [Input { name: "low", ty: "f64[V]", value: &low }];
        writer.event(0, &Event::Call { frame: 0, caller: None, function: "check_voltage", inputs: &inputs, types: &[("raw", "[f64]")] });
        let raw = |w: &mut ValueWriter<'_>| {
            let count = w.begin_array(100_000);
            for index in 0..count {
                w.f64(index as f64 + 0.5, None);
            }
            w.end();
        };
        writer.event(250, &Event::Ran { frame: 0, node: "raw", loops: &[], iter: &[], out: &[("raw", &raw)], types: &[] });
        writer.end(1_000, "ok");
        let bytes = writer.into_inner();

        let wire = |name: &str, ty: &str| ExpectedWire { name: name.into(), ty: ty.into() };
        let expected = Expected {
            function: "check_voltage".into(),
            inputs: vec![wire("low", "f64[V]")],
            nodes: vec![ExpectedNode { name: "raw".into(), wires: vec![wire("raw", "[f64]")], ..ExpectedNode::default() }],
        };
        let mut reader = Reader::new(expected, ReadLimits::default());
        for part in bytes.chunks(7) {
            assert!(reader.feed(part));
        }
        let trace = reader.finish();
        assert_eq!((trace.end.as_deref(), trace.ignored, trace.cut()), (Some("ok"), 0, false));
        assert_eq!(trace.input(0), Some(r#""1.5e-7 V""#));
        // Worked out by hand: 0.5, 1.5, … 15.5, the 16 of `$head`.
        let head: Vec<String> = (0..16).map(|index| format!("{index}.5")).collect();
        assert_eq!(trace.value(0, 0, &[]), Some(format!(r#"{{"$len":100000,"$head":[{}]}}"#, head.join(",")).as_str()));
        let header = trace.header.unwrap();
        assert!(labcraft_trace::stale(&header, &[("check_voltage.lcf", Some(source))]).is_empty());
        assert_eq!(labcraft_trace::stale(&header, &[("check_voltage.lcf", Some(b"changed"))]), ["check_voltage.lcf"]);
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
