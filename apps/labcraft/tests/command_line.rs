//! The command line of the viewer (`docs/cli.md`, section 8), without opening a window.

use std::process::Command;

fn run(args: &[&str]) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_labcraft")).args(args).output().unwrap();
    (output.status.code(), String::from_utf8_lossy(&output.stdout).into_owned(), String::from_utf8_lossy(&output.stderr).into_owned())
}

#[test]
fn help_and_version_print_and_exit_with_0() {
    let (code, out, _) = run(&["--help"]);
    assert_eq!(code, Some(0));
    assert!(out.starts_with("usage: labcraft [--lib-path DIR]... [--theme light|dark] [--style FILE] [FOLDER]"), "{out}");
    let (code, out, _) = run(&["--version"]);
    assert_eq!((code, out.trim()), (Some(0), concat!("labcraft ", env!("CARGO_PKG_VERSION"))));
}

#[test]
fn wrong_usage_exits_with_2_and_says_why() {
    for (args, said) in [
        (&["a", "b"][..], "one folder at most"),
        (&["--theme", "blue"][..], "--theme takes light or dark"),
        (&["--watch"][..], "unknown option `--watch`"),
    ] {
        let (code, out, err) = run(args);
        assert_eq!(code, Some(2), "{args:?}");
        assert!(out.is_empty());
        assert!(err.starts_with(&format!("labcraft: {said}\nusage: labcraft")), "{err}");
    }
}
