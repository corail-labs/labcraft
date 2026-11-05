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

/// `check` takes folders (`docs/cli.md`, section 3), and a file is wrong usage. A path that does not exist is not: it is
/// data, and gets a diagnostic.
pub fn refuse_files(options: &CheckOptions) -> Result<(), String> {
    let is_a_file = |path: &&PathBuf| Path::metadata(path).is_ok_and(|metadata| !metadata.is_dir());
    match options.folders.iter().chain(&options.lib_paths).find(is_a_file) {
        Some(file) => Err(format!("`{}` is a file, and `check` takes folders", file.display())),
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

    #[cfg(unix)]
    #[test]
    fn a_path_need_not_be_unicode() {
        use std::os::unix::ffi::OsStringExt as _;
        let path = OsString::from_vec(vec![b'a', 0xFF]);
        assert_eq!(parse_fmt(std::slice::from_ref(&path)).unwrap().paths, [PathBuf::from(path)]);
    }
}
