//! Runs the acceptance corpora against `format` and `check`, natively and in WASM, with no command
//! line (`docs/architecture.md`, section 8). The corpora arrive with step 4 of section 9; until then
//! this proves that the crates link and that tests really run on both targets.

#[cfg(test)]
mod tests {
    use labcraft_check as _;
    use labcraft_format as _;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn runs_on_the_target_it_was_built_for() {
        assert_eq!(usize::BITS == 32, cfg!(target_arch = "wasm32"));
    }
}
