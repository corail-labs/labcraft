//! Exit codes and streams of the command line (`docs/cli.md`, section 1).

use std::process::{Command, Output};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_labcraft-cli")).args(args).output().expect("the program starts")
}

#[test]
fn version_and_help_succeed_on_stdout() {
    for args in [&["version"][..], &["--help"][..]] {
        let output = cli(args);
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        assert!(!output.stdout.is_empty(), "{args:?}");
        assert!(output.stderr.is_empty(), "{args:?}");
    }
}

#[test]
fn wrong_usage_is_exit_2_with_nothing_on_stdout() {
    let cases: &[&[&str]] = &[
        &[],
        &["nonsense"],
        &["version", "extra"],
        &["check"],
        &["check", "--deny-warnings"],
        &["check", "--nonsense", "somewhere"],
        &["check", "somewhere", "--lib-path"],
        &["check", "Cargo.toml"],
        &["check", "somewhere", "--lib-path", "Cargo.toml"],
        &["fmt"],
        &["fmt", "--check"],
        &["fmt", "--nonsense", "somewhere"],
        &["fmt", "somewhere", "--max-report"],
        &["fmt", "somewhere", "--max-report", "many"],
        &["fmt", "somewhere", "--max-file-size", "-1"],
        &["fmt", "somewhere", "--max-file-size", "1073741825"],
        &["fmt", "somewhere", "--max-nesting", "1025"],
        // `mcp setup` (cli.md 10): nothing is written on wrong usage.
        &["mcp", "setup", "--client", "claude-desktop"],
        &["mcp", "setup", "--client"],
        &["mcp", "setup", "here"],
        &["mcp", "setup", "--json"],
    ];
    for args in cases {
        let output = cli(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(!output.stderr.is_empty(), "{args:?}");
    }
}
