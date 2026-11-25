//! The command line of the IDE (`docs/cli.md`, section 8), without opening a window.

use std::process::Command;

fn run(args: &[&str]) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_labcraft")).args(args).output().unwrap();
    (output.status.code(), String::from_utf8_lossy(&output.stdout).into_owned(), String::from_utf8_lossy(&output.stderr).into_owned())
}

#[test]
fn help_and_version_print_and_exit_with_0() {
    let (code, out, _) = run(&["--help"]);
    assert_eq!(code, Some(0));
    assert!(out.starts_with("usage: labcraft [--lib-path DIR]... [--theme light|dark] [--style FILE] [FOLDER | FILE]"), "{out}");
    let (code, out, _) = run(&["--version"]);
    assert_eq!((code, out.trim()), (Some(0), concat!("labcraft ", env!("CARGO_PKG_VERSION"))));
}

#[test]
fn wrong_usage_exits_with_2_and_says_why() {
    let notes = std::env::temp_dir().join(format!("labcraft-notes-{}.txt", std::process::id()));
    std::fs::write(&notes, "notes\n").unwrap();
    let not_a_program = format!("`{}` is neither a folder nor a .lcf or .lct file", notes.display());
    for (args, said) in [
        (&["a", "b"][..], "one folder or file at most"),
        (&["--theme", "blue"][..], "--theme takes light or dark"),
        (&["--watch"][..], "unknown option `--watch`"),
        (&[notes.to_str().unwrap()][..], not_a_program.as_str()),
    ] {
        let (code, out, err) = run(args);
        assert_eq!(code, Some(2), "{args:?}");
        assert!(out.is_empty());
        assert!(err.starts_with(&format!("labcraft: {said}\nusage: labcraft")), "{err}");
    }
    let _ = std::fs::remove_file(&notes);
}

/// `labcraft FILE` for a project that a window shows (cli.md 8): the file goes to that window, from the root of the
/// project, and the process ends with 0, opening no window of its own. The window is played by the test: the file of
/// its channel, and its side of the proofs.
#[test]
fn a_file_of_a_project_that_a_window_shows_goes_to_that_window() {
    use labcraft_cli::attach;
    let scratch = std::env::temp_dir().join(format!("labcraft-command-line-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(scratch.join("project/.labcraft")).unwrap();
    std::fs::create_dir_all(scratch.join("project/power")).unwrap();
    std::fs::create_dir_all(scratch.join("channel")).unwrap();
    let file = scratch.join("project/power/check_power.lcf");
    std::fs::write(&file, "labcraft 1\n\nfn check_power() -> () {}\n").unwrap();
    let root = std::fs::canonicalize(scratch.join("project")).unwrap();
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let published = attach::publish(&scratch.join("channel"), &root, listener.local_addr().unwrap().port()).unwrap();
    let token = published.token().to_owned();
    let window = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut asked = None;
        attach::greet(&mut stream, &token, |hello| {
            asked = hello.show.clone();
            Ok(())
        })
        .unwrap();
        asked
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_labcraft")).env(attach::FOLDER_VARIABLE, scratch.join("channel")).arg(&file).spawn().unwrap();
    // Not handed over, it would open a window: the test gives up after 30 s rather than wait for it.
    let mut status = None;
    for _ in 0..600 {
        status = child.try_wait().unwrap();
        if status.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let Some(status) = status else {
        let _ = child.kill();
        panic!("labcraft did not hand the file to the window");
    };
    assert_eq!(status.code(), Some(0));
    assert_eq!(window.join().unwrap().as_deref(), Some("power/check_power.lcf"));
    drop(published);
    let _ = std::fs::remove_dir_all(&scratch);
}
