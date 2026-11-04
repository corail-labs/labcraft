//! Runs the acceptance corpora against `format` and `check`, natively and in WASM, with no command
//! line (`docs/architecture.md`, section 8). So far: the families Encoding, Truncation and Nesting of
//! scenario 4, through `read`. The other families and the rest of each one come with their steps.

/// A file of a corpus, with its path from the root of the repository.
pub struct Embedded {
    pub path: &'static str,
    pub bytes: &'static [u8],
}

/// A stored case of `tests/acceptance/hostile`, and its `expected.json`.
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
    use std::collections::BTreeSet;

    use labcraft_check as _;
    use labcraft_format::{STACK_SIZE, read};
    use labcraft_types::{Code, Interner, Limits, MAX_NESTING};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::deep::{DEEP, Generator};
    use super::{HOSTILE, PROGRAMS};

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

    /// What `fmt` reports on these comes from `read` alone. `changes` waits for `write` (step 5), and the
    /// half of `expected.json` for `check` waits for step 7.
    #[test]
    fn encoding_gives_the_codes_and_the_exit_that_fmt_expects() {
        let cases: Vec<_> = HOSTILE.iter().filter(|case| case.name.starts_with("encoding-")).collect();
        assert_eq!(cases.len(), 12);
        for case in cases {
            let expected: serde_json::Value = serde_json::from_str(case.expected).unwrap();
            let expected = &expected["fmt"];
            let codes: BTreeSet<&str> = expected["codes"].as_array().unwrap().iter().map(|code| code.as_str().unwrap()).collect();
            let [file] = case.files else { panic!("{}: one file expected", case.name) };
            let mut names = Interner::new();
            let result = read(file.bytes, &mut names, &Limits::default());
            let found: BTreeSet<&str> = result.report.diagnostics().iter().map(|diagnostic| diagnostic.code().as_str()).collect();
            assert_eq!(found, codes, "{}", case.name);
            assert_eq!(u64::from(result.report.errors() > 0), expected["exit"].as_u64().unwrap(), "{}", case.name);
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
            let limits = Limits::default().with_max_nesting(MAX_NESTING).unwrap();
            for (kind, deep) in DEEP {
                assert_limit(kind, deep, MAX_NESTING, &limits);
                let mut names = Interner::new();
                let text = deep(MAX_NESTING as usize);
                let file = read(text.as_bytes(), &mut names, &limits).file.unwrap();
                let copy = file.clone();
                assert!(file == copy, "{kind}");
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
