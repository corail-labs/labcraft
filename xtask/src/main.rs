//! Workspace tooling: `cargo xtask <command>` (`docs/architecture.md`, section 7).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod assets;
mod codes;
mod commands;
mod layers;
mod libs;
mod licences;
mod spec;
mod unicode;
mod web;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use serde_json::Value;

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci        lint, test, bounds, wasm; stops at the first failure
  lint      fmt --check, layers, licences, libs, codes, commands, assets, clippy -D warnings
  test      the tests of the workspace, through cargo-nextest when it is installed, then the doctests
  layers    a crate depends only on lower layers
  licences  no GPL, LGPL or AGPL dependency (D-23)
  libs      no crate of crates/ depends on libs/ or on a native crate of examples/
  codes     the enumeration Code equals the tables of docs/cli.md, section 5, and labcraft_visa::CODES those of simulation.md, 7
  commands  the registry of engine equals the tables of docs/commands.md, section 6: ids, parameters, which are required, their defaults
  assets    every asset has its line in ATTRIBUTION.md, and every file that it names exists
  bounds    fmt, check, diagram and run of the cases of scenario 4 stay within their bounds, in release; memory on Linux only
  wasm      format, check, trace, ir, runtime, visa, results_log, diagram, engine, editor and labcraft-web build for wasm32-unknown-unknown, and the harness and labcraft-web pass there
  spec      the Python checks of the spec, examples, docs/rfc and libs, and the proof of concept's tests; ok or FAIL for each, the output of failures only; not part of ci
  web       clippy for WASM of labcraft-web, then trunk build --release into target/web; not part of ci
  web-serve serves target/web on http://127.0.0.1:8770/, to try the viewer in a browser; not part of ci
  web-smoke target/web in headless Chrome, as a browser opens it and with WebGL2 asked for: the page draws, traps its worker and draws again; not part of ci
  figures   the size and the figure of stability of each example, in Markdown, for the summary of CI; not part of ci
  unicode   writes the tables of types::text from the Unicode Character Database, downloaded once into target/; not part of ci
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
        Some("lint") => lint(),
        Some("test") => test(),
        Some("layers") => check_layers(),
        Some("licences") => check_licences(),
        Some("fuzz-licences") => check_fuzz_licences(),
        Some("libs") => check_libs(),
        Some("codes") => check_codes(),
        Some("commands") => check_commands(),
        Some("assets") => check_assets(),
        Some("unicode") => unicode::generate(&root()),
        Some("wasm") => wasm(),
        Some("bounds") => bounds(),
        Some("figures") => figures(),
        Some("web") => web::build(&root()),
        Some("web-serve") => web::serve(&root()),
        Some("web-smoke") => web::smoke(&root()),
        Some("spec") => spec::run(&root()),
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

/// The whole gate. The workflow `checks` runs its parts in jobs of their own, side by side: `lint`, `test`, and `bounds`
/// with `wasm`.
fn ci() -> Result<(), String> {
    lint()?;
    test()?;
    bounds()?;
    wasm()
}

fn lint() -> Result<(), String> {
    run(&["fmt", "--all", "--check"])?;
    check_layers()?;
    check_licences()?;
    check_libs()?;
    check_codes()?;
    check_commands()?;
    check_assets()?;
    run(&["clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings"])
}

/// cargo-nextest runs the test binaries side by side, where `cargo test` runs them one after the other; without it the
/// gate still runs every test. nextest does not run doctests.
fn test() -> Result<(), String> {
    let nextest = cargo().args(["nextest", "--version"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    if !nextest.is_ok_and(|status| status.success()) {
        return run(&["test", "--locked", "--workspace"]);
    }
    run(&["nextest", "run", "--locked", "--workspace", "--no-fail-fast"])?;
    run(&["test", "--locked", "--workspace", "--doc"])
}

/// Scenario 4, point 3, in release: the crates of the workspace are not optimized in tests (`tests/harness/tests/bounds.rs`).
fn bounds() -> Result<(), String> {
    run(&[
        "test",
        "--locked",
        "--release",
        "--package",
        "labcraft-harness",
        "--test",
        "bounds",
        "--",
        "--ignored",
        "--exact",
        "every_case_stays_within_the_bounds",
        "--nocapture",
    ])
}

/// The figures of the viewer (`docs/visual-notation.md`, 2.1 and 7), on standard output and nothing else, so that CI can
/// add them to its summary: the test writes its table to a file, its own output goes to standard error.
fn figures() -> Result<(), String> {
    let path = root().join("target").join("figures.md");
    let args = [
        "test",
        "--locked",
        "--release",
        "--package",
        "labcraft-harness",
        "--test",
        "figures",
        "--",
        "--ignored",
        "--exact",
        "the_figures_of_the_examples",
    ];
    let what = format!("cargo {}", args.join(" "));
    // A table left by an earlier run is never printed for this one.
    if path.exists() {
        std::fs::remove_file(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    eprintln!("$ {what}");
    let output = cargo().args(args).env("LABCRAFT_FIGURES", &path).stderr(Stdio::inherit()).output().map_err(|error| format!("{what}: {error}"))?;
    eprint!("{}", String::from_utf8_lossy(&output.stdout));
    if !output.status.success() {
        return Err(format!("{what}: {}", output.status));
    }
    let table = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    print!("{table}");
    Ok(())
}

fn check_layers() -> Result<(), String> {
    let crates = layers::from_metadata(&metadata(false)?)?;
    report("layers", crates.len(), "packages", layers::check(&crates))?;
    let resolved = metadata(true)?;
    let count = resolved["packages"].as_array().map_or(0, Vec::len);
    report("fonts", count, "packages", layers::default_fonts(&resolved)?)
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
    report("codes", in_code.len(), "codes", codes::check(&codes::from_document(&document)?, &in_code))?;
    // The codes of each library with native bodies equal the table of its document (`docs/architecture.md`, 10.6).
    let path = root().join("libs").join("visa").join("simulation.md");
    let document = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let documented =
        codes::library_from_document(&document, "## 7. What is refused", "visa").map_err(|error| format!("{}: {error}", path.display()))?;
    let visa = codes::of_visa();
    let mut violations: Vec<String> = codes::check(&documented, &visa)
        .into_iter()
        .map(|violation| violation.replace("docs/cli.md", "libs/visa/simulation.md").replace("the enumeration Code", "labcraft_visa::CODES"))
        .collect();
    violations.extend(codes::twice(&visa));
    report("codes of visa", visa.len(), "codes", violations)?;
    let path = root().join("docs").join("cli.md");
    let document = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    report("codes of the simulation file", labcraft_runtime::simulation::CODES.len(), "codes", codes::of_the_simulation_file(&document))
}

fn check_commands() -> Result<(), String> {
    let path = root().join("docs").join("commands.md");
    let document = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let registered = commands::in_registry();
    report("commands", registered.len(), "commands", commands::check(&commands::from_document(&document)?, &registered))
}

fn check_assets() -> Result<(), String> {
    let root = root();
    let path = root.join("ATTRIBUTION.md");
    let attribution = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let files = assets::files(&root)?;
    report("assets", files.len(), "assets", assets::check(&root, &files, &attribution))
}

fn wasm() -> Result<(), String> {
    let runner = Command::new(WASM_RUNNER).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status();
    if runner.is_err() {
        let version = wasm_bindgen_version(&metadata(true)?).unwrap_or("<the version of wasm-bindgen in Cargo.lock>".to_owned());
        return Err(format!("{WASM_RUNNER} is missing; install it with\n  cargo install wasm-bindgen-cli --version {version} --locked"));
    }
    run(&[
        "build",
        "--locked",
        "--target",
        WASM_TARGET,
        "--package",
        "labcraft-format",
        "--package",
        "labcraft-check",
        "--package",
        "labcraft-trace",
        "--package",
        "labcraft-ir",
        "--package",
        "labcraft-runtime",
        "--package",
        "labcraft-visa",
        "--package",
        "labcraft-results-log",
        "--package",
        "labcraft-diagram",
        "--package",
        "labcraft-engine",
        "--package",
        "labcraft-editor",
        "--package",
        "labcraft-web",
    ])?;
    wasm_tests()
}

/// The tests run here only through an attribute that the harness imports for WASM: without it, the runner
/// finds no test and succeeds. So some test must have passed.
fn wasm_tests() -> Result<(), String> {
    // Each package on its own, so that each must have run some test.
    wasm_tests_of("labcraft-harness")?;
    wasm_tests_of("labcraft-web")
}

fn wasm_tests_of(package: &str) -> Result<(), String> {
    let args = ["test", "--locked", "--target", WASM_TARGET, "--package", package, "--lib"];
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
