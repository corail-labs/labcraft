//! `cargo xtask hooks`: the tests of the hook that `.claude/settings.json` runs before each tool call of an agent
//! (AGENTS.md, D-23). The hook is Python, with the standard library only. Not part of `ci`, which needs no Python
//! (`docs/architecture.md`, section 7).

use std::path::Path;
use std::process::{Command, Output};

const TESTS: &[&str] = &["-m", "unittest", "discover", "-s", ".claude/hooks", "-p", "test_*.py"];

/// Tried in this order. On Windows, `python3` is often the stub of the Microsoft Store, which fails the probe.
const INTERPRETERS: &[&str] = &["python", "python3"];

/// Writes `old <version>` before 3.11; `sys.stdout.write` and `%`, so that Python 2 reports its version too.
const PROBE: &str = "\
import sys
if sys.version_info < (3, 11):
    sys.stdout.write('old %d.%d.%d\\n' % tuple(sys.version_info[:3]))
";

pub fn run(root: &Path) -> Result<(), String> {
    let python = interpreter(root)?;
    let output = python_command(root, python).args(TESTS).output().map_err(|error| format!("{python}: {error}"))?;
    if output.status.success() {
        println!("ok   hooks of the agents");
        return Ok(());
    }
    println!("FAIL hooks of the agents");
    println!("$ {python} {} ({})", TESTS.join(" "), output.status);
    print!("{}", text(&output));
    Err("hooks: the tests failed, see FAIL above".to_owned())
}

/// Standard output, then standard error, each ending with a new line.
fn text(output: &Output) -> String {
    let mut text = String::new();
    for stream in [&output.stdout, &output.stderr] {
        let part = String::from_utf8_lossy(stream);
        text.push_str(&part);
        if !part.is_empty() && !part.ends_with('\n') {
            text.push('\n');
        }
    }
    text
}

/// The version an interpreter that ran the probe is too old at, if it is.
fn too_old(report: &str) -> Option<String> {
    report.lines().find_map(|line| line.strip_prefix("old ")).map(|version| version.trim().to_owned())
}

/// The first interpreter that is recent enough. When none is, the problem of the first one that runs.
pub(crate) fn interpreter(root: &Path) -> Result<&'static str, String> {
    let mut problem = None;
    for &python in INTERPRETERS {
        let Ok(output) = python_command(root, python).arg("-c").arg(PROBE).output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        match too_old(&String::from_utf8_lossy(&output.stdout)) {
            None => return Ok(python),
            Some(version) => {
                problem.get_or_insert(format!("{python} is Python {version}; the hooks need 3.11 or later"));
            }
        }
    }
    Err(problem.unwrap_or_else(|| format!("no Python found ({}); install Python 3.11 or later", INTERPRETERS.join(", "))))
}

/// The tests print `…`; on Windows a pipe would otherwise take the code page of the console.
fn python_command(root: &Path, python: &str) -> Command {
    let mut command = Command::new(python);
    command.current_dir(root).env("PYTHONIOENCODING", "utf-8");
    command
}

#[cfg(test)]
mod tests {
    use super::too_old;

    #[test]
    fn the_probe_tells_old_from_ready() {
        assert_eq!(too_old(""), None);
        assert_eq!(too_old("old 2.7.18\n"), Some("2.7.18".to_owned()));
        assert_eq!(too_old("old 3.10.12\r\n"), Some("3.10.12".to_owned()));
    }
}
