//! `cargo xtask setup`: installs what `cargo xtask ci` needs beyond the toolchain of `rust-toolchain.toml`, which
//! rustup installs by itself, and says each command before it runs it. What it does not install, it only checks:
//! Python for `cargo xtask hooks`, and, on Windows, the path of the repository.

use std::path::Path;
use std::process::{Command, Stdio};

/// Installs what is missing, then fails when something would still stop `ci` here.
/// The path comes first, so that a refusal does not wait for an install.
pub fn run(root: &Path) -> Result<(), String> {
    let path = root.to_string_lossy();
    if let Some(folder) = escaping_folder(&path).filter(|_| cfg!(windows)) {
        return Err(format!(
            "setup: the folder `{folder}` of {path} starts with `x` or `u`. On Windows the WASM test runner writes the \
             path raw into JavaScript, where `\\{folder}` reads as an escape, so `cargo xtask wasm` fails with \"test \
             exited abnormally\". Rename the folder, or clone or add the worktree elsewhere, then run setup again"
        ));
    }
    wasm_target(root)?;
    wasm_runner(root)?;
    optional_tools(root);
    println!("ready: run `cargo xtask ci`");
    Ok(())
}

/// `rust-toolchain.toml` names the target, and rustup adds it with the toolchain; an older install may lack it.
fn wasm_target(root: &Path) -> Result<(), String> {
    let installed = output(root, "rustup", &["target", "list", "--installed"])
        .ok_or("setup: rustup was not found; install Rust from https://rustup.rs, then run `cargo xtask setup` again")?;
    if installed.lines().any(|line| line.trim() == crate::WASM_TARGET) {
        println!("ok   target {}", crate::WASM_TARGET);
        return Ok(());
    }
    install(root, "rustup", &["target", "add", crate::WASM_TARGET])
}

/// The runner must have the version of `wasm-bindgen` that `Cargo.lock` gives the tests; another version fails them.
fn wasm_runner(root: &Path) -> Result<(), String> {
    let wanted = crate::wasm_bindgen_version(&crate::metadata(true)?).ok_or("setup: no wasm-bindgen in Cargo.lock")?;
    let found = output(root, crate::WASM_RUNNER, &["--version"]);
    match found.as_deref().and_then(runner_version) {
        Some(version) if version == wanted => {
            println!("ok   {} {wanted}", crate::WASM_RUNNER);
            Ok(())
        }
        Some(version) => {
            println!("{} is {version}; the tests need {wanted}", crate::WASM_RUNNER);
            install(root, "cargo", &["install", "wasm-bindgen-cli", "--version", &wanted, "--locked"])
        }
        None => install(root, "cargo", &["install", "wasm-bindgen-cli", "--version", &wanted, "--locked"]),
    }
}

/// Not needed by `ci`: said, never installed.
fn optional_tools(root: &Path) {
    match crate::hooks::interpreter(root) {
        Ok(python) => println!("ok   {python}, for `cargo xtask hooks`"),
        Err(problem) => println!("note {problem}. Only `cargo xtask hooks` and the hook of Claude Code need it, not `ci`"),
    }
    if output(root, "cargo", &["nextest", "--version"]).is_some() {
        println!("ok   cargo-nextest");
    } else {
        println!("note cargo-nextest is not installed; `cargo xtask test` works without it, more slowly");
    }
}

/// Runs a command that installs, after saying it, with its output on the terminal.
fn install(root: &Path, program: &str, args: &[&str]) -> Result<(), String> {
    let what = format!("{program} {}", args.join(" "));
    println!("$ {what}");
    let status = Command::new(program).current_dir(root).args(args).status().map_err(|error| format!("{what}: {error}"))?;
    if status.success() { Ok(()) } else { Err(format!("setup: {what}: {status}")) }
}

/// Standard output of a command that succeeded; nothing when it is missing or fails.
fn output(root: &Path, program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).current_dir(root).args(args).stderr(Stdio::null()).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `wasm-bindgen-test-runner 0.2.100` gives `0.2.100`.
fn runner_version(output: &str) -> Option<&str> {
    output.lines().next()?.split_whitespace().nth(1)
}

/// The first folder of a path whose name starts with `x` or `u`, which a JavaScript string reads as `\x` or `\u`.
fn escaping_folder(path: &str) -> Option<&str> {
    path.split(['\\', '/']).skip(1).find(|folder| folder.starts_with(['x', 'u']))
}

#[cfg(test)]
mod tests {
    use super::{escaping_folder, runner_version};

    // Written by hand: the output of `wasm-bindgen-test-runner --version`, and paths of Windows.
    #[test]
    fn the_version_is_the_second_word() {
        assert_eq!(runner_version("wasm-bindgen-test-runner 0.2.100\n"), Some("0.2.100"));
        assert_eq!(runner_version(""), None);
    }

    #[test]
    fn a_folder_that_starts_with_x_or_u_is_found() {
        assert_eq!(escaping_folder(r"D:\ideas\LabCraft\.claude\worktrees\xtask-spec"), Some("xtask-spec"));
        assert_eq!(escaping_folder(r"C:\users\me\LabCraft"), Some("users"));
        assert_eq!(escaping_folder(r"D:\ideas\LabCraft\.claude\worktrees\spec-xtask"), None);
        assert_eq!(escaping_folder(r"D:\Users\LabCraft"), None);
        assert_eq!(escaping_folder("u:/LabCraft"), None);
    }
}
