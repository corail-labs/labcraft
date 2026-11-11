//! `cargo xtask spec`: the Python checks of the spec, the examples, the RFC programs and the shipped libraries, with
//! the tests of the proof of concept. Not part of `ci`, which needs no Python (`docs/architecture.md`, section 7).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The checks, in the order they run. This is the only list: the workflow `checks` and AGENTS.md call `cargo xtask spec`.
const CHECKS: &[(&str, &[&str])] = &[
    ("grammar, examples and spec code blocks", &["tools/lcf-view/lcf_view.py", "examples", "libs", "--spec", "docs/language-spec-v0.md"]),
    ("check examples", &["tools/lcf-poc/lcf.py", "check", "examples"]),
    ("check docs/rfc", &["tools/lcf-poc/lcf.py", "check", "docs/rfc"]),
    ("check libs/visa", &["tools/lcf-poc/lcf.py", "check", "libs/visa"]),
    ("check libs/scpi", &["tools/lcf-poc/lcf.py", "check", "libs/scpi"]),
    ("canonical form", &["tools/lcf-poc/lcf.py", "fmt", "--check", "--spec", "docs/language-spec-v0.md", "examples", "docs/rfc", "libs"]),
    ("proof of concept tests", &["-m", "unittest", "discover", "tools/lcf-poc/tests"]),
    ("MCP round trip", &["tools/lcf-poc/tests/mcp_smoke.py"]),
];

/// Tried in this order. On Windows, `python3` is often the stub of the Microsoft Store, which fails the probe.
const INTERPRETERS: &[&str] = &["python", "python3"];

const REQUIREMENTS: &[&str] = &["tools", "lcf-poc", "requirements.txt"];

/// The requirements file, with the separators of the platform, so that the command of a message can be pasted.
fn requirements(root: &Path) -> PathBuf {
    REQUIREMENTS.iter().fold(root.to_path_buf(), |path, part| path.join(part))
}

/// Writes `missing <distribution>` for each name on its command line that is not installed, and `old <version>` before
/// 3.11 (`tomllib`). It catches nothing else, so a probe that fails some other way is an interpreter that does not work.
/// `sys.stdout.write` and `%`, so that Python 2 reports its version too.
const PROBE: &str = "\
import sys
if sys.version_info < (3, 11):
    sys.stdout.write('old %d.%d.%d\\n' % tuple(sys.version_info[:3]))
    sys.exit(0)
from importlib import metadata
for name in sys.argv[1:]:
    try:
        metadata.distribution(name)
    except metadata.PackageNotFoundError:
        sys.stdout.write('missing ' + name + '\\n')
";

pub fn run(root: &Path) -> Result<(), String> {
    let path = requirements(root);
    let requirements = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let python = interpreter(root, &distributions(&requirements))?;
    let mut failed = 0;
    for (name, args) in CHECKS {
        let output = python_command(root, python).args(*args).output().map_err(|error| format!("{python}: {error}"))?;
        if output.status.success() {
            println!("ok   {name}");
            continue;
        }
        failed += 1;
        println!("FAIL {name}");
        println!("$ {python} {} ({})", args.join(" "), output.status);
        print!("{}", text(&output));
    }
    println!("{}", summary(CHECKS.len(), failed));
    match failed {
        0 => Ok(()),
        _ => Err("spec: a check failed, see FAIL above".to_owned()),
    }
}

fn summary(checks: usize, failed: usize) -> String {
    format!("spec: {checks} checks, {failed} failed")
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

/// What the probe says of an interpreter that ran it.
#[derive(Debug, PartialEq)]
enum Probe {
    Ready,
    Old(String),
    Missing(Vec<String>),
}

fn probe(report: &str) -> Probe {
    if let Some(version) = report.lines().find_map(|line| line.strip_prefix("old ")) {
        return Probe::Old(version.trim().to_owned());
    }
    let missing: Vec<String> = report.lines().filter_map(|line| line.strip_prefix("missing ")).map(|name| name.trim().to_owned()).collect();
    if missing.is_empty() { Probe::Ready } else { Probe::Missing(missing) }
}

/// The first interpreter that is recent enough and has every distribution. When none has, the problem of the first
/// one that runs, which says what to install.
fn interpreter(root: &Path, distributions: &[&str]) -> Result<&'static str, String> {
    let requirements = requirements(root);
    let mut problem = None;
    for &python in INTERPRETERS {
        let Ok(output) = python_command(root, python).arg("-c").arg(PROBE).args(distributions).output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let install = format!("install them with\n  {python} -m pip install -r {}", requirements.display());
        let found = match probe(&String::from_utf8_lossy(&output.stdout)) {
            Probe::Ready => return Ok(python),
            Probe::Old(version) => format!("{python} is Python {version}; the Python tools need 3.11 or later"),
            Probe::Missing(names) => {
                let names: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
                let verb = if names.len() == 1 { "is" } else { "are" };
                format!("{python}: {} {verb} not installed; {install}", names.join(", "))
            }
        };
        problem.get_or_insert(found);
    }
    Err(problem.unwrap_or_else(|| {
        format!(
            "no Python found ({}); install Python 3.11 or later, then\n  python -m pip install -r {}",
            INTERPRETERS.join(", "),
            requirements.display()
        )
    }))
}

/// The tools print `…` and `→`; on Windows a pipe would otherwise take the code page of the console.
fn python_command(root: &Path, python: &str) -> Command {
    let mut command = Command::new(python);
    command.current_dir(root).env("PYTHONIOENCODING", "utf-8");
    command
}

/// The names of the distributions in a requirements file: each line that is not a comment or an option such as `-r`,
/// up to its version.
fn distributions(requirements: &str) -> Vec<&str> {
    requirements
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim())
        .filter(|line| !line.is_empty() && !line.starts_with('-'))
        .filter_map(|line| line.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))).next())
        .filter(|name| !name.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Probe, distributions, probe};

    #[test]
    fn the_names_stop_at_the_version_and_comments_and_options_are_skipped() {
        let requirements = "# a comment\nlark>=1.2,<2\n\nmcp>=1.2,<2  # why\n-r other.txt\nmy_pkg.x[extra]==1\nplain\n";
        assert_eq!(distributions(requirements), ["lark", "mcp", "my_pkg.x", "plain"]);
    }

    #[test]
    fn the_probe_tells_old_from_missing_from_ready() {
        assert_eq!(probe(""), Probe::Ready);
        assert_eq!(probe("old 2.7.18\n"), Probe::Old("2.7.18".to_owned()));
        assert_eq!(probe("old 3.10.12\r\n"), Probe::Old("3.10.12".to_owned()));
        assert_eq!(probe("missing lark\r\nmissing mcp\r\n"), Probe::Missing(vec!["lark".to_owned(), "mcp".to_owned()]));
    }
}
