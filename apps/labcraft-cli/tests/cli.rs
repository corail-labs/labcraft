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
        &["check", "somewhere"],
        &["fmt"],
        &["fmt", "--check"],
        &["fmt", "--nonsense", "somewhere"],
        &["fmt", "somewhere", "--max-report"],
        &["fmt", "somewhere", "--max-report", "many"],
        &["fmt", "somewhere", "--max-file-size", "-1"],
        &["fmt", "somewhere", "--max-file-size", "1073741825"],
        &["fmt", "somewhere", "--max-nesting", "1025"],
    ];
    for args in cases {
        let output = cli(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(!output.stderr.is_empty(), "{args:?}");
    }
}
