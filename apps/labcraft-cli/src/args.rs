//! The arguments of `fmt` and `check`, by hand, as in `gridcraft-cli` and `solidcraft-cli` (`docs/architecture.md`,
//! section 6). An error here is wrong usage: nothing is read or written (`docs/cli.md`, section 1).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::slice::Iter;

use labcraft_types::{Limits, MAX_FILE_SIZE, MAX_NESTING};

#[derive(Debug)]
pub struct FmtOptions {
    pub check: bool,
    pub json: bool,
    pub limits: Limits,
    pub paths: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct CheckOptions {
    pub deny_warnings: bool,
    pub json: bool,
    pub lib_paths: Vec<PathBuf>,
    pub limits: Limits,
    pub folders: Vec<PathBuf>,
}

/// What the two commands share.
struct Common {
    json: bool,
    limits: Limits,
    paths: Vec<PathBuf>,
}

/// `--` ends the options, since a path is data and can start with `-`. `own` takes the options of one command, and
/// says whether it knew this one.
fn parse_common<'a>(args: &'a [OsString], mut own: impl FnMut(&str, &mut Iter<'a, OsString>) -> Result<bool, String>) -> Result<Common, String> {
    let mut common = Common { json: false, limits: Limits::default(), paths: Vec::new() };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        if text == "--" {
            common.paths.extend(args.by_ref().map(PathBuf::from));
        } else if !text.starts_with('-') {
            common.paths.push(PathBuf::from(arg));
        } else {
            match text.as_ref() {
                "--json" => common.json = true,
                "--max-report" => common.limits.max_report = number("--max-report", args.next())?,
                "--max-file-size" => {
                    let size = number("--max-file-size", args.next())?;
                    if size > MAX_FILE_SIZE {
                        return Err(format!("--max-file-size is at most {MAX_FILE_SIZE}"));
                    }
                    common.limits.max_file_size = size;
                }
                "--max-nesting" => {
                    let nesting = number("--max-nesting", args.next())?;
                    common.limits = common.limits.with_max_nesting(nesting).ok_or_else(|| format!("--max-nesting is at most {MAX_NESTING}"))?;
                }
                _ => {
                    if !own(&text, &mut args)? {
                        return Err(format!("unknown option `{text}`"));
                    }
                }
            }
        }
    }
    if common.paths.is_empty() {
        return Err("no path given".to_owned());
    }
    Ok(common)
}

pub fn parse_fmt(args: &[OsString]) -> Result<FmtOptions, String> {
    let mut check = false;
    let common = parse_common(args, |option, _| {
        let known = option == "--check";
        check |= known;
        Ok(known)
    })?;
    Ok(FmtOptions { check, json: common.json, limits: common.limits, paths: common.paths })
}

pub fn parse_check(args: &[OsString]) -> Result<CheckOptions, String> {
    let mut deny_warnings = false;
    let mut lib_paths = Vec::new();
    let common = parse_common(args, |option, rest| match option {
        "--deny-warnings" => {
            deny_warnings = true;
            Ok(true)
        }
        "--lib-path" => {
            lib_paths.push(PathBuf::from(rest.next().ok_or("--lib-path needs a folder")?));
            Ok(true)
        }
        _ => Ok(false),
    })?;
    Ok(CheckOptions { deny_warnings, json: common.json, lib_paths, limits: common.limits, folders: common.paths })
}

/// Which theme the SVG holds (`docs/cli.md`, 7.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Both,
    Light,
    Dark,
}

/// `labcraft-cli diagram` (`docs/cli.md`, section 7). `check` holds what it checks: `FOLDER` alone, its `--lib-path`
/// folders and the options of section 1.
#[derive(Debug)]
pub struct DiagramOptions {
    pub check: CheckOptions,
    pub out: PathBuf,
    pub theme: Theme,
    pub max_nodes: u64,
    /// The functions asked for, each once, in the order given; none for every function.
    pub functions: Vec<String>,
}

/// The default and the most of `--max-diagram-nodes` (`docs/cli.md`, 7.6).
pub const DEFAULT_DIAGRAM_NODES: u64 = 20_000;
pub const MAX_DIAGRAM_NODES: u64 = 100_000;

/// The options that take one value, which `diagram` refuses twice (`docs/cli.md`, 7.1).
const ONCE: [&str; 6] = ["--out", "--theme", "--max-diagram-nodes", "--max-report", "--max-file-size", "--max-nesting"];

pub fn parse_diagram(args: &[OsString]) -> Result<DiagramOptions, String> {
    // An option that takes one value, given twice; what follows `--` is a path, and the value of an option is not one.
    let mut seen: Vec<&str> = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let text = arg.to_string_lossy();
        if text == "--" {
            break;
        }
        if let Some(&once) = ONCE.iter().find(|once| **once == text) {
            if seen.contains(&once) {
                return Err(format!("{once} is given twice"));
            }
            seen.push(once);
            rest.next();
        } else if text == "--lib-path" {
            rest.next();
        }
    }
    let (mut svg, mut out, mut theme, mut max_nodes) = (false, None, Theme::Both, DEFAULT_DIAGRAM_NODES);
    let mut lib_paths = Vec::new();
    let common = parse_common(args, |option, rest| match option {
        "--svg" => {
            svg = true;
            Ok(true)
        }
        "--out" => {
            out = Some(PathBuf::from(rest.next().ok_or("--out needs a folder")?));
            Ok(true)
        }
        "--lib-path" => {
            lib_paths.push(PathBuf::from(rest.next().ok_or("--lib-path needs a folder")?));
            Ok(true)
        }
        "--theme" => {
            theme = match rest.next().map(|value| value.to_string_lossy()).as_deref() {
                Some("light") => Theme::Light,
                Some("dark") => Theme::Dark,
                _ => return Err("--theme is `light` or `dark`".to_owned()),
            };
            Ok(true)
        }
        "--max-diagram-nodes" => {
            max_nodes = number("--max-diagram-nodes", rest.next())?;
            if !(1..=MAX_DIAGRAM_NODES).contains(&max_nodes) {
                return Err(format!("--max-diagram-nodes is from 1 to {MAX_DIAGRAM_NODES}"));
            }
            Ok(true)
        }
        // The values of a run come with a later version (`docs/cli.md`, 7.3).
        "--trace" | "--max-trace-size" => Err(format!("{option}: this version does not draw the values of a run yet")),
        _ => Ok(false),
    })?;
    if !svg {
        return Err("diagram needs --svg".to_owned());
    }
    let out = out.ok_or("diagram needs --out DIR")?;
    let mut paths = common.paths.into_iter();
    let folder = paths.next().ok_or("no folder given")?;
    let mut functions: Vec<String> = Vec::new();
    for function in paths {
        let text = function.to_string_lossy().into_owned();
        // A name of the language, or that of a folder or a file that `check` reads: only an empty one is no name.
        if text.split('.').any(str::is_empty) {
            return Err(format!("`{text}` is not the name of a function: names joined by `.`"));
        }
        if !functions.contains(&text) {
            functions.push(text);
        }
    }
    let check = CheckOptions { deny_warnings: false, json: common.json, lib_paths, limits: common.limits, folders: vec![folder] };
    Ok(DiagramOptions { check, out, theme, max_nodes, functions })
}

/// `check` and `diagram` take folders (`docs/cli.md`, sections 3 and 7), and a file is wrong usage. A path that does not exist is not: it is
/// data, and gets a diagnostic.
pub fn refuse_files(options: &CheckOptions) -> Result<(), String> {
    let is_a_file = |path: &&PathBuf| Path::metadata(path).is_ok_and(|metadata| !metadata.is_dir());
    match options.folders.iter().chain(&options.lib_paths).find(is_a_file) {
        Some(file) => Err(format!("`{}` is a file, and this command takes folders", file.display())),
        None => Ok(()),
    }
}

fn number<T: std::str::FromStr>(option: &str, value: Option<&OsString>) -> Result<T, String> {
    let value = value.ok_or_else(|| format!("{option} needs a whole number"))?.to_string_lossy();
    value.parse().map_err(|_| format!("{option} needs a whole number, not `{value}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn parse(args: &[&str]) -> Result<FmtOptions, String> {
        parse_fmt(&os(args))
    }

    #[test]
    fn the_defaults_are_those_of_the_documentation() {
        let options = parse(&["a"]).unwrap();
        assert!(!options.check && !options.json);
        assert_eq!(options.limits, Limits::default());
        assert_eq!(options.paths, [PathBuf::from("a")]);
    }

    #[test]
    fn options_and_paths_mix_in_any_order() {
        let options = parse(&["a", "--check", "b", "--json", "--max-report", "3", "--max-file-size", "9", "--max-nesting", "5"]).unwrap();
        assert!(options.check && options.json);
        assert_eq!((options.limits.max_report, options.limits.max_file_size, options.limits.max_nesting()), (3, 9, 5));
        assert_eq!(options.paths, [PathBuf::from("a"), PathBuf::from("b")]);
    }

    #[test]
    fn after_two_dashes_everything_is_a_path() {
        let options = parse(&["--", "--check", "-x"]).unwrap();
        assert!(!options.check);
        assert_eq!(options.paths, [PathBuf::from("--check"), PathBuf::from("-x")]);
    }

    #[test]
    fn wrong_usage_is_refused() {
        let cases: &[&[&str]] = &[
            &[],
            &["--check"],
            &["--"],
            &["--nonsense", "a"],
            &["-", "a"],
            &["--max-report=3", "a"],
            &["a", "--max-report"],
            &["a", "--max-report", "x"],
            &["a", "--max-report", "-1"],
            &["a", "--max-file-size", "1.5"],
            &["a", "--max-file-size", "1073741825"],
            &["a", "--max-file-size", "99999999999999"],
            &["a", "--max-nesting", "1025"],
            &["a", "--max-nesting", "99999999999"],
            &["a", "--deny-warnings"],
            &["a", "--lib-path", "b"],
        ];
        for args in cases {
            assert!(parse(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn the_ceilings_are_accepted() {
        assert_eq!(parse(&["a", "--max-nesting", "1024"]).unwrap().limits.max_nesting(), 1024);
        assert_eq!(parse(&["a", "--max-file-size", "1073741824"]).unwrap().limits.max_file_size, 1024 * 1024 * 1024);
    }

    #[test]
    fn check_takes_its_own_options_and_those_of_both() {
        let options = parse_check(&os(&["a", "--deny-warnings", "--lib-path", "x", "b", "--json", "--lib-path", "y", "--max-report", "3"])).unwrap();
        assert!(options.deny_warnings && options.json);
        assert_eq!(options.lib_paths, [PathBuf::from("x"), PathBuf::from("y")]);
        assert_eq!(options.folders, [PathBuf::from("a"), PathBuf::from("b")]);
        assert_eq!(options.limits.max_report, 3);
        let plain = parse_check(&os(&["a"])).unwrap();
        assert!(!plain.deny_warnings && !plain.json && plain.lib_paths.is_empty());
    }

    #[test]
    fn check_refuses_wrong_usage() {
        let cases: &[&[&str]] =
            &[&[], &["--deny-warnings"], &["--check", "a"], &["a", "--lib-path"], &["--lib-path", "a"], &["a", "--max-nesting", "1025"]];
        for args in cases {
            assert!(parse_check(&os(args)).is_err(), "{args:?}");
        }
    }

    fn diagram(args: &[&str]) -> Result<DiagramOptions, String> {
        parse_diagram(&os(args))
    }

    #[test]
    fn diagram_takes_its_options_and_those_of_section_1() {
        let options = diagram(&["--svg", "--out", "d", ".", "check_voltage", "dmm.measure", "check_voltage", "--json", "--theme", "dark"]).unwrap();
        assert_eq!(options.out, PathBuf::from("d"));
        assert_eq!(options.check.folders, [PathBuf::from(".")]);
        assert_eq!(options.functions, ["check_voltage", "dmm.measure"]);
        assert!(options.check.json && options.theme == Theme::Dark);
        assert_eq!(options.max_nodes, DEFAULT_DIAGRAM_NODES);
        let plain = diagram(&["--svg", "--out", "d", "--lib-path", "x", "--max-diagram-nodes", "100000", "."]).unwrap();
        assert!(plain.functions.is_empty() && plain.theme == Theme::Both && !plain.check.json);
        assert_eq!((plain.max_nodes, plain.check.lib_paths.as_slice()), (MAX_DIAGRAM_NODES, [PathBuf::from("x")].as_slice()));
    }

    #[test]
    fn diagram_refuses_wrong_usage() {
        let cases: &[&[&str]] = &[
            &["--out", "d", "."],
            &["--svg", "."],
            &["--svg", "--out", "d"],
            &["--svg", "--out", "d", ".", "a..b"],
            &["--svg", "--out", "d", ".", ".a"],
            &["--svg", "--out", "d", ".", "a."],
            &["--svg", "--out", "d", "--theme", "blue", "."],
            &["--svg", "--out", "d", "--out", "e", "."],
            &["--svg", "--out", "d", "--theme", "dark", "--theme", "dark", "."],
            &["--svg", "--out", "d", "--max-report", "3", "--max-report", "3", "."],
            &["--svg", "--out", "d", "--max-diagram-nodes", "0", "."],
            &["--svg", "--out", "d", "--max-diagram-nodes", "100001", "."],
            &["--svg", "--out", "d", "--trace", "t.jsonl", ".", "f"],
            &["--svg", "--out", "d", "--max-trace-size", "9", "."],
            &["--svg", "--out", "d", "--deny-warnings", "."],
        ];
        for args in cases {
            assert!(diagram(args).is_err(), "{args:?}");
        }
        // After `--`, a path; and a value is not an option given twice.
        assert_eq!(diagram(&["--svg", "--out", "--out", "--", "--theme"]).unwrap().check.folders, [PathBuf::from("--theme")]);
    }

    #[cfg(unix)]
    #[test]
    fn a_path_need_not_be_unicode() {
        use std::os::unix::ffi::OsStringExt as _;
        let path = OsString::from_vec(vec![b'a', 0xFF]);
        assert_eq!(parse_fmt(std::slice::from_ref(&path)).unwrap().paths, [PathBuf::from(path)]);
    }
}
