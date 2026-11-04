//! Workspace tooling: `cargo xtask <command>` (`docs/architecture.md`, section 7).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod codes;
mod layers;
mod libs;
mod licences;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use serde_json::Value;

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci        fmt --check, layers, licences, libs, codes, clippy -D warnings, tests, wasm; stops at the first failure
  layers    a crate depends only on lower layers
  licences  no GPL, LGPL or AGPL dependency (D-23)
  libs      no crate of crates/ depends on libs/
  codes     the enumeration Code equals the tables of docs/cli.md, section 5
  wasm     format and check build for wasm32-unknown-unknown, and the harness passes there
  fuzz-licences  the same as licences, for fuzz/, outside the workspace; not part of ci: the job fuzz runs it
";

const WASM_TARGET: &str = "wasm32-unknown-unknown";
const WASM_RUNNER: &str = "wasm-bindgen-test-runner";
/// About the call stack of V8 in a worker, half of the 984 KiB of a page: the web viewer works in a worker
/// (`docs/architecture.md`, sections 4 and 5). The runner passes this to Node.
const WASM_ENGINE_STACK: &str = "--stack-size=492";

fn main() -> ExitCode {
    let command = std::env::args().nth(1);
    let result = match command.as_deref() {
        Some("ci") => ci(),
        Some("layers") => check_layers(),
        Some("licences") => check_licences(),
        Some("fuzz-licences") => check_fuzz_licences(),
        Some("libs") => check_libs(),
        Some("codes") => check_codes(),
        Some("wasm") => wasm(),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn ci() -> Result<(), String> {
    run(&["fmt", "--all", "--check"])?;
    check_layers()?;
    check_licences()?;
    check_libs()?;
    check_codes()?;
    run(&["clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings"])?;
    run(&["test", "--locked", "--workspace"])?;
    wasm()
}

fn check_layers() -> Result<(), String> {
    let crates = layers::from_metadata(&metadata(false)?)?;
    report("layers", crates.len(), "packages", layers::check(&crates))
}

fn check_licences() -> Result<(), String> {
    let packages = licences::from_metadata(&metadata(true)?)?;
    report("licences", packages.len(), "packages", licences::check(&packages))
}

/// `fuzz/` is a workspace of its own, which `cargo metadata` at the root does not see. `--locked` also refuses a
/// stale `fuzz/Cargo.lock`.
fn check_fuzz_licences() -> Result<(), String> {
    let mut command = cargo();
    command.args(["metadata", "--format-version", "1", "--locked", "--manifest-path", "fuzz/Cargo.toml"]);
    let packages = licences::from_metadata(&metadata_of(command)?)?;
    report("fuzz-licences", packages.len(), "packages", licences::check(&packages))
}

fn check_libs() -> Result<(), String> {
    let metadata = metadata(false)?;
    let root = metadata["workspace_root"].as_str().ok_or("metadata: no workspace_root")?;
    let packages = libs::from_metadata(&metadata)?;
    report("libs", packages.len(), "packages", libs::check(Path::new(root), &packages))
}

fn check_codes() -> Result<(), String> {
    let path = root().join("docs").join("cli.md");
    let document = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let in_code = codes::in_code();
    report("codes", in_code.len(), "codes", codes::check(&codes::from_document(&document)?, &in_code))
}

fn wasm() -> Result<(), String> {
    let runner = Command::new(WASM_RUNNER).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status();
    if runner.is_err() {
        let version = wasm_bindgen_version(&metadata(true)?).unwrap_or("<the version of wasm-bindgen in Cargo.lock>".to_owned());
        return Err(format!("{WASM_RUNNER} is missing; install it with\n  cargo install wasm-bindgen-cli --version {version} --locked"));
    }
    run(&["build", "--locked", "--target", WASM_TARGET, "--package", "labcraft-format", "--package", "labcraft-check"])?;
    wasm_tests()
}

/// The tests run here only through an attribute that the harness imports for WASM: without it, the runner
/// finds no test and succeeds. So some test must have passed.
fn wasm_tests() -> Result<(), String> {
    let args = ["test", "--locked", "--target", WASM_TARGET, "--package", "labcraft-harness", "--lib"];
    let what = format!("cargo {}", args.join(" "));
    eprintln!("$ {what}");
    let output =
        cargo().args(args).env("NODE_ARGS", WASM_ENGINE_STACK).stderr(Stdio::inherit()).output().map_err(|error| format!("{what}: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    print!("{stdout}");
    if !output.status.success() {
        return Err(format!("{what}: {}", output.status));
    }
    match passed_tests(&stdout) {
        0 => Err(format!("{what}: no test ran")),
        _ => Ok(()),
    }
}

/// The tests passed, from the lines `test result: ok. 6 passed; 0 failed; …`.
fn passed_tests(output: &str) -> usize {
    output
        .lines()
        .filter(|line| line.contains("test result:"))
        .filter_map(|line| line.split(" passed").next()?.rsplit(' ').next()?.parse::<usize>().ok())
        .sum()
}

/// The runner must have the same version as the library that the tests link.
fn wasm_bindgen_version(metadata: &Value) -> Option<String> {
    let packages = metadata["packages"].as_array()?;
    let package = packages.iter().find(|package| package["name"] == "wasm-bindgen")?;
    Some(package["version"].as_str()?.to_owned())
}

fn report<V: std::fmt::Display>(check: &str, count: usize, what: &str, violations: Vec<V>) -> Result<(), String> {
    if violations.is_empty() {
        println!("{check}: {count} {what}, no violation");
        return Ok(());
    }
    for violation in &violations {
        eprintln!("{check}: {violation}");
    }
    Err(format!("{check}: {} violation(s)", violations.len()))
}

fn run(args: &[&str]) -> Result<(), String> {
    let what = format!("cargo {}", args.join(" "));
    eprintln!("$ {what}");
    let status = cargo().args(args).status().map_err(|error| format!("{what}: {error}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what}: {status}")) }
}

fn metadata(with_dependencies: bool) -> Result<Value, String> {
    let mut command = cargo();
    command.args(["metadata", "--format-version", "1", "--locked"]);
    if !with_dependencies {
        command.arg("--no-deps");
    }
    metadata_of(command)
}

fn metadata_of(mut command: Command) -> Result<Value, String> {
    let output = command.output().map_err(|error| format!("cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!("cargo metadata:\n{}", String::from_utf8_lossy(&output.stderr)));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| format!("cargo metadata: {error}"))
}

fn cargo() -> Command {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.current_dir(root());
    command
}

fn root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap_or(manifest).to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::passed_tests;

    #[test]
    fn the_passed_tests_add_up_over_the_results() {
        let output = "running 6 tests\ntest result: ok. 6 passed; 0 failed; 0 ignored\n\ntest result: ok. 1 passed; 0 failed\n";
        assert_eq!(passed_tests(output), 7);
    }

    #[test]
    fn no_result_or_no_test_is_nothing_passed() {
        assert_eq!(passed_tests("no tests to run!\n"), 0);
        assert_eq!(passed_tests("test result: ok. 0 passed; 0 failed\n"), 0);
    }

    #[test]
    fn colours_do_not_hide_the_count() {
        assert_eq!(passed_tests("test result: \u{1b}[32mok\u{1b}[0m. 6 passed; 0 failed\n"), 6);
    }
}
