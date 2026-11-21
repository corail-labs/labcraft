//! Workspace tooling: `cargo xtask <command>` (`docs/architecture.md`, section 7).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod assets;
mod clean_room;
mod codes;
mod commands;
mod hooks;
mod layers;
mod libs;
mod licences;
mod notices;
mod setup;
mod unicode;
mod web;

/// The function of 1,000 nodes and 2,000 wires of `bounds`.
#[path = "../../tests/harness/tests/frames_case/mod.rs"]
mod frames_case;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use serde_json::Value;

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci        lint, test, bounds, wasm; stops at the first failure
  setup     installs what ci needs and is missing (the WASM target, wasm-bindgen-test-runner), saying each command;
            checks Python and, on Windows, the path of the repository
  lint      fmt --check, layers, licences, libs, codes, commands, assets, clean-room, clippy -D warnings
  test      the tests of the workspace, through cargo-nextest when it is installed, then the doctests
  layers    a crate depends only on lower layers
  licences  no GPL, LGPL or AGPL dependency (D-23)
  libs      no crate of crates/ depends on libs/ or on a native crate of examples/, and none embeds their files but
            crates/check/src/core_blocks.rs those of libs/core
  codes     the enumeration Code equals the tables of docs/cli.md, section 5, labcraft_visa::CODES those of simulation.md, 7, labcraft_net::CODES that of libs/net, and labcraft_serial::CODES that of libs/serial
  commands  the registry of engine equals the tables of docs/commands.md, section 6: ids, parameters, which are required, their defaults
  assets    every asset has its line in ATTRIBUTION.md, and every file that it names exists
  clean-room no LabVIEW file in the repository, tracked or about to be added (D-23)
  bounds    fmt, check, diagram and run of the cases of scenario 4 stay within their bounds, in release; memory on Linux only
  wasm      format, check, trace, ir, runtime, net, serial, visa, results_log, diagram, engine, editor and labcraft-web build for wasm32-unknown-unknown, and the harness and labcraft-web pass there
  hooks     the Python tests of the hook of the agents (.claude/hooks); not part of ci
  web       clippy for WASM of labcraft-web, then trunk build --release into target/web; not part of ci
  web-serve serves target/web on http://127.0.0.1:8770/, to try the viewer in a browser; not part of ci
  web-smoke target/web in headless Chrome, as a browser opens it and with WebGL2 asked for: the page draws, traps its worker and draws again; not part of ci
  frames    builds labcraft in release and opens it on the function of 1,000 nodes and 2,000 wires of bounds, timing
            each frame (LABCRAFT_FRAMES), to see 60 frames/s by hand before a tag; not part of ci
  figures [--against FILE]  the size and the figure of stability of each example, in Markdown, for the summary of CI;
            with --against, fails when a figure of stability is worse than in FILE (scenario 5 of M1); not part of ci
  unicode   writes the tables of types::text from the Unicode Character Database, downloaded once into target/; not part of ci
  fuzz-licences  the same as licences, for fuzz/, outside the workspace; not part of ci: the job fuzz runs it
  notices TARGET OUT  writes into OUT the licences of the crates built into the binaries of a release for TARGET;
            not part of ci: the workflow release runs it
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
        Some("notices") => match (std::env::args().nth(2), std::env::args().nth(3)) {
            (Some(target), Some(out)) => write_notices(&target, Path::new(&out)),
            _ => Err(format!("notices needs a target and a file\n\n{USAGE}")),
        },
        Some("libs") => check_libs(),
        Some("codes") => check_codes(),
        Some("commands") => check_commands(),
        Some("assets") => check_assets(),
        Some("unicode") => unicode::generate(&root()),
        Some("wasm") => wasm(),
        Some("bounds") => bounds(),
        Some("figures") => match (std::env::args().nth(2).as_deref(), std::env::args().nth(3)) {
            (None, _) => figures(None),
            (Some("--against"), Some(file)) => figures(Some(Path::new(&file))),
            _ => Err(format!("figures takes nothing, or `--against FILE`\n\n{USAGE}")),
        },
        Some("frames") => frames(),
        Some("web") => web::build(&root()),
        Some("web-serve") => web::serve(&root()),
        Some("web-smoke") => web::smoke(&root()),
        Some("hooks") => hooks::run(&root()),
        Some("clean-room") => clean_room::check(&root()),
        Some("setup") => setup::run(&root()),
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
    clean_room::check(&root())?;
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

/// 60 frames/s with 1,000 nodes and 2,000 wires, by hand (`docs/architecture.md`, 11.6): the function of `bounds` written
/// into `target/frames/`, and the viewer, in release, opened on it with `LABCRAFT_FRAMES`, until its window closes.
fn frames() -> Result<(), String> {
    let folder = root().join("target").join("frames");
    std::fs::create_dir_all(&folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    let file = folder.join(frames_case::FILE);
    std::fs::write(&file, frames_case::nodes_1000_wires_2000()).map_err(|error| format!("{}: {error}", file.display()))?;
    run(&["build", "--locked", "--release", "--package", "labcraft"])?;
    let viewer = root().join("target").join("release").join(format!("labcraft{}", std::env::consts::EXE_SUFFIX));
    eprintln!("$ LABCRAFT_FRAMES=1 {} {}", viewer.display(), folder.display());
    let status = Command::new(&viewer)
        .arg("--")
        .arg(&folder)
        .env("LABCRAFT_FRAMES", "1")
        .status()
        .map_err(|error| format!("{}: {error}", viewer.display()))?;
    if status.success() { Ok(()) } else { Err(format!("{}: {status}", viewer.display())) }
}

/// The figures of the viewer (`docs/visual-notation.md`, 2.1 and 7), on standard output and nothing else, so that CI can
/// add them to its summary: the test writes its table to a file, its own output goes to standard error. With `against`,
/// a table of figures written once by the code of a tag: a figure of stability worse than there fails (scenario 5 of
/// `docs/acceptance-m1.md`, point 2).
fn figures(against: Option<&Path>) -> Result<(), String> {
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
    let Some(against) = against else { return Ok(()) };
    let file = if against.is_absolute() { against.to_path_buf() } else { root().join(against) };
    let then = std::fs::read_to_string(&file).map_err(|error| format!("{}: {error}", file.display()))?;
    let worse = worse_figures(&stability(&then), &stability(&table));
    eprintln!("figures: {} functions against {}", stability(&then).len(), against.display());
    if worse.is_empty() { Ok(()) } else { Err(format!("figures of stability worse than in {}:\n{}", against.display(), worse.join("\n"))) }
}

/// The figures of stability of a table of `figures`, by function: the nodes that move and those whose column changes,
/// each a mean and a most.
fn stability(table: &str) -> std::collections::BTreeMap<String, [f64; 4]> {
    let mut found = std::collections::BTreeMap::new();
    for line in table.lines().filter(|line| line.starts_with("| `")) {
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        let (Some(name), Some(moved), Some(column)) = (cells.first(), cells.get(5), cells.get(6)) else { continue };
        let pair = |cell: &str| -> Option<(f64, f64)> {
            let (mean, most) = cell.split_once('/')?;
            // A function with no wire probed has `–` for its mean: nothing moves. Anything else reads, or the row does not.
            let mean = match mean.trim() {
                "–" => 0.0,
                mean => mean.parse::<f64>().ok()?,
            };
            Some((mean, most.trim().parse::<f64>().ok()?))
        };
        if let (Some(moved), Some(column)) = (pair(moved), pair(column)) {
            found.insert(name.trim_matches('`').to_owned(), [moved.0, moved.1, column.0, column.1]);
        }
    }
    found
}

/// Each function whose figures of stability are worse now than then; a function that is new has none to be worse than.
fn worse_figures(then: &std::collections::BTreeMap<String, [f64; 4]>, now: &std::collections::BTreeMap<String, [f64; 4]>) -> Vec<String> {
    const NAMES: [&str; 4] = ["moved (mean)", "moved (most)", "column changed (mean)", "column changed (most)"];
    let mut worse = Vec::new();
    for (function, before) in then {
        let Some(after) = now.get(function) else {
            worse.push(format!("  `{function}`: no figure now"));
            continue;
        };
        for ((name, was), is) in NAMES.iter().zip(before).zip(after) {
            if is > was {
                worse.push(format!("  `{function}`: {name} {is} against {was}"));
            }
        }
    }
    worse
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

/// The notices of a release for `target` (`notices.rs`), from the metadata of the workspace filtered for it.
fn write_notices(target: &str, out: &Path) -> Result<(), String> {
    let mut command = cargo();
    command.args(["metadata", "--format-version", "1", "--locked", "--filter-platform", target]);
    let crates = notices::reached(&metadata_of(command)?, notices::roots(target))?;
    let mut with_texts = Vec::new();
    for krate in crates {
        let mut texts = Vec::new();
        for file in notices::files_of(&krate) {
            let bytes = std::fs::read(&file).map_err(|error| format!("{}: {error}", file.display()))?;
            let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            texts.push((name, String::from_utf8_lossy(&bytes).replace("\r\n", "\n")));
        }
        with_texts.push((krate, texts));
    }
    let text = notices::render(target, &with_texts);
    std::fs::write(out, &text).map_err(|error| format!("{}: {error}", out.display()))?;
    let bare = with_texts.iter().filter(|(_, texts)| texts.is_empty()).count();
    println!("notices: {} crates for {target}, {} without a licence file, {} bytes in {}", with_texts.len(), bare, text.len(), out.display());
    Ok(())
}

fn check_libs() -> Result<(), String> {
    let metadata = metadata(false)?;
    let root = metadata["workspace_root"].as_str().ok_or("metadata: no workspace_root")?;
    let packages = libs::from_metadata(&metadata)?;
    let mut violations = libs::check(Path::new(root), &packages);
    violations.extend(libs::embeddings(&libs::sources(Path::new(root))?));
    report("libs", packages.len(), "packages", violations)
}

fn check_codes() -> Result<(), String> {
    let path = root().join("docs").join("cli.md");
    let document = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let in_code = codes::in_code();
    report("codes", in_code.len(), "codes", codes::check(&codes::from_document(&document)?, &in_code, "docs/cli.md", "the enumeration Code"))?;
    // The codes of each library with native bodies equal the table of its document (`docs/architecture.md`, 10.6).
    for library in codes::LIBRARIES {
        let path = root().join(library.document);
        let document = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        report(&format!("codes of {}", library.name), library.codes.len(), "codes", library.violations(&document)?)?;
    }
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
        "labcraft-net",
        "--package",
        "labcraft-serial",
        "--package",
        "labcraft-visa",
        "--package",
        "labcraft-results-log",
        "--package",
        "labcraft-diagram",
        "--package",
        "labcraft-edit",
        "--package",
        "labcraft-engine",
        "--package",
        "labcraft-editor",
        "--package",
        "labcraft-ide",
        "--package",
        "kit-shell",
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

    #[test]
    fn a_figure_of_stability_worse_than_then_fails() {
        use super::{stability, worse_figures};
        let a = "| `a` | 3 | 10 × 10 | 100 % | 2 | 1.5 / 2 | 0.5 / 1 |\n";
        let b = "| `b` | 1 | 5 × 5 | 100 % | 0 | – / 0 | – / 0 |\n";
        let then = stability(&format!("| Function | Nodes |\n{a}{b}"));
        assert_eq!(then.get("a"), Some(&[1.5, 2.0, 0.5, 1.0]));
        assert_eq!(then.get("b"), Some(&[0.0, 0.0, 0.0, 0.0]));
        // The same, and a function that is new: nothing worse.
        let same = stability(&format!("{a}{b}| `c` | 9 | 1 × 1 | 1 % | 9 | 9.0 / 9 | 9.0 / 9 |\n"));
        assert_eq!(worse_figures(&then, &same), Vec::<String>::new());
        let worse = stability("| `a` | 3 | 10 × 10 | 100 % | 2 | 1.0 / 3 | 0.5 / 1 |\n");
        assert_eq!(worse_figures(&then, &worse), ["  `a`: moved (most) 3 against 2", "  `b`: no figure now"]);
    }
}
