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
                _ => {
                    if !limit(&text, &mut args, &mut common.limits)? && !own(&text, &mut args)? {
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

/// The limits of `docs/cli.md`, section 1, which every command takes: whether `option` is one of them.
fn limit(option: &str, args: &mut Iter<'_, OsString>, limits: &mut Limits) -> Result<bool, String> {
    match option {
        "--max-report" => limits.max_report = number("--max-report", args.next())?,
        "--max-file-size" => {
            let size = number("--max-file-size", args.next())?;
            if size > MAX_FILE_SIZE {
                return Err(format!("--max-file-size is at most {MAX_FILE_SIZE}"));
            }
            limits.max_file_size = size;
        }
        "--max-nesting" => {
            let nesting = number("--max-nesting", args.next())?;
            *limits = limits.clone().with_max_nesting(nesting).ok_or_else(|| format!("--max-nesting is at most {MAX_NESTING}"))?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// `labcraft-cli mcp` (`docs/mcp.md`, 1).
#[derive(Debug)]
pub struct McpOptions {
    /// `--project`, or the current folder.
    pub project: Option<PathBuf>,
    pub lib_paths: Vec<PathBuf>,
    pub limits: Limits,
}

pub fn parse_mcp(args: &[OsString]) -> Result<McpOptions, String> {
    let mut options = McpOptions { project: None, lib_paths: Vec::new(), limits: Limits::default() };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        match text.as_ref() {
            "--project" => {
                if options.project.is_some() {
                    return Err("--project is given once".to_owned());
                }
                options.project = Some(PathBuf::from(args.next().ok_or("--project needs a folder")?));
            }
            "--lib-path" => options.lib_paths.push(PathBuf::from(args.next().ok_or("--lib-path needs a folder")?)),
            _ if limit(&text, &mut args, &mut options.limits)? => {}
            _ if text.starts_with('-') => return Err(format!("unknown option `{text}`")),
            _ => return Err(format!("`mcp` takes no path but that of --project, not `{text}`")),
        }
    }
    Ok(options)
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
    /// `--trace FILE`: the values of a run on the one function asked for (`docs/cli.md`, 7.3).
    pub trace: Option<PathBuf>,
    /// `--max-trace-size BYTES` (`docs/cli.md`, 7.4).
    pub max_trace_size: u64,
}

/// The default and the most of `--max-diagram-nodes` (`docs/cli.md`, 7.6).
pub const DEFAULT_DIAGRAM_NODES: u64 = 20_000;
pub const MAX_DIAGRAM_NODES: u64 = 100_000;

/// The options that take one value, which `diagram` refuses twice (`docs/cli.md`, 7.1).
const ONCE: [&str; 8] =
    ["--out", "--theme", "--max-diagram-nodes", "--max-report", "--max-file-size", "--max-nesting", "--trace", "--max-trace-size"];

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
    let (mut trace, mut max_trace_size) = (None, labcraft_engine::drawings::DEFAULT_TRACE_SIZE);
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
        "--trace" => {
            trace = Some(PathBuf::from(rest.next().ok_or("--trace needs a file")?));
            Ok(true)
        }
        "--max-trace-size" => {
            max_trace_size = count(option, text_of(option, rest.next())?, 1, labcraft_engine::drawings::MAX_TRACE_SIZE)?;
            Ok(true)
        }
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
    if trace.is_some() && functions.len() != 1 {
        return Err("--trace draws the values of a run on one function: give exactly one FUNCTION".to_owned());
    }
    let check = CheckOptions { deny_warnings: false, json: common.json, lib_paths, limits: common.limits, folders: vec![folder] };
    Ok(DiagramOptions { check, out, theme, max_nodes, functions, trace, max_trace_size })
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

/// Where the trace of a run goes (`docs/cli.md`, 6.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceTo {
    /// `.labcraft/runs/` under `FOLDER`.
    Folder,
    File(PathBuf),
    None,
}

/// A `--max-` limit given: a number, or `none`, which removes the default (`--max-iterations`, `--max-nodes`, `--max-made`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    Count(u64),
    None,
}

/// `labcraft-cli run` (`docs/cli.md`, section 6).
#[derive(Debug)]
pub struct RunOptions {
    pub json: bool,
    pub deny_warnings: bool,
    pub lib_paths: Vec<PathBuf>,
    pub limits: Limits,
    pub folder: PathBuf,
    pub function: String,
    pub sim: Option<PathBuf>,
    pub args_file: Option<PathBuf>,
    /// `--arg NAME=VALUE`, in the order given.
    pub args: Vec<(String, String)>,
    /// `--set ELEMENT=VALUE@TIME`: the element, the value, the time, as written.
    pub set: Vec<(String, String, String)>,
    pub schedule_seed: Option<u64>,
    /// `--real-time` (spec 8).
    pub real_time: bool,
    /// Durations as written, already judged: `30s`, `"30 s"`.
    pub timeout: Option<String>,
    pub max_real_time: Option<String>,
    pub max_iterations: Option<Limit>,
    pub max_nodes: Option<Limit>,
    pub max_made: Option<Limit>,
    pub max_call_depth: Option<u64>,
    pub max_memory: Option<u64>,
    pub trace: TraceTo,
}

/// The options of `run` that take one value, which it refuses twice (`docs/cli.md`, 6.2).
const RUN_ONCE: [&str; 14] = [
    "--sim",
    "--args",
    "--schedule-seed",
    "--timeout",
    "--max-iterations",
    "--max-nodes",
    "--max-call-depth",
    "--max-memory",
    "--max-made",
    "--max-real-time",
    "--trace",
    "--max-report",
    "--max-file-size",
    "--max-nesting",
];

/// The most of `--max-call-depth`, and the least of `--max-memory` (`docs/cli.md`, 6.2).
pub const MAX_CALL_DEPTH: u64 = 4096;
pub const MIN_MEMORY: u64 = 1 << 20;
/// The most of a count: 2^63 - 1.
const MAX_COUNT: u64 = i64::MAX as u64;

/// What starts a trace (`docs/commands.md`, section 8): a file that exists and does not start so is not replaced.
const TRACE_START: &[u8] = b"{\"schema\":\"labcraft-trace/1\"";

fn text_of<'a>(option: &str, value: Option<&'a OsString>) -> Result<&'a str, String> {
    let value = value.ok_or_else(|| format!("{option} needs a value"))?;
    value.to_str().ok_or_else(|| format!("{option}: `{}` is not Unicode", value.to_string_lossy()))
}

/// A duration of the command line, as the language writes it, the space before the unit optional (`docs/cli.md`, 6.2).
fn duration(option: &str, text: &str, above_zero: bool) -> Result<i64, String> {
    match labcraft_engine::runs::duration_ns(text) {
        Ok(ns) if ns > 0 || (ns == 0 && !above_zero) => Ok(ns),
        Ok(_) if above_zero => Err(format!("{option}: `{text}` is not above 0")),
        Ok(_) => Err(format!("{option}: `{text}` is below 0")),
        Err(_) if text.trim().parse::<f64>().is_ok() => {
            Err(format!("{option}: `{text}` has no unit, and seconds are not assumed: write `{}s` or `\"{} s\"`", text.trim(), text.trim()))
        }
        Err(why) => Err(format!("{option}: `{text}` is no duration, `30s` or `\"500 ms\"`: {why}")),
    }
}

fn count(option: &str, text: &str, least: u64, most: u64) -> Result<u64, String> {
    match text.parse::<u64>() {
        Ok(value) if (least..=most).contains(&value) => Ok(value),
        _ => Err(format!("{option} is a whole number from {least} to {most}, not `{text}`")),
    }
}

/// Names joined by `.`, each a name of the language (spec 14.1).
fn is_function_path(text: &str) -> bool {
    !text.is_empty() && text.split('.').all(labcraft_format::is_name)
}

/// `--trace FILE` never replaces a program, a file of arguments or anything that is not a trace (`docs/cli.md`, 6.2).
fn judge_trace_file(path: &Path) -> Result<(), String> {
    // Windows drops the dots and spaces that end a name: `x.lcf.` is `x.lcf`.
    let name = path.file_name().map(|name| name.to_string_lossy().trim_end_matches(['.', ' ']).to_lowercase()).unwrap_or_default();
    if [".lcf", ".lct", ".toml", ".json"].iter().any(|extension| name.ends_with(extension)) {
        return Err(format!("--trace: `{}` would replace a program or a file of LabCraft, not a trace", path.display()));
    }
    // On Windows, `x.lcf:s` names the stream `s` of the file `x.lcf`, which writing it makes.
    if cfg!(windows) && name.contains(':') {
        return Err(format!("--trace: `{}` names a stream of a file, on Windows", path.display()));
    }
    match std::fs::metadata(path) {
        Err(_) => Ok(()),
        Ok(metadata) if metadata.is_dir() => Err(format!("--trace: `{}` is a folder", path.display())),
        Ok(_) => {
            let mut start = [0_u8; TRACE_START.len()];
            let read = std::fs::File::open(path).and_then(|mut file| std::io::Read::read_exact(&mut file, &mut start));
            if read.is_ok() && start == TRACE_START {
                Ok(())
            } else {
                Err(format!("--trace: `{}` exists and is not a trace, so it is not replaced", path.display()))
            }
        }
    }
}

pub fn parse_run(args: &[OsString]) -> Result<RunOptions, String> {
    let mut seen: Vec<&str> = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let text = arg.to_string_lossy();
        if text == "--" {
            break;
        }
        if let Some(&once) = RUN_ONCE.iter().find(|once| **once == text) {
            if seen.contains(&once) {
                return Err(format!("{once} is given twice"));
            }
            seen.push(once);
            rest.next();
        } else if matches!(text.as_ref(), "--lib-path" | "--arg" | "--set") {
            rest.next();
        }
    }
    let mut options = RunOptions {
        json: false,
        deny_warnings: false,
        lib_paths: Vec::new(),
        limits: Limits::default(),
        folder: PathBuf::new(),
        function: String::new(),
        sim: None,
        args_file: None,
        args: Vec::new(),
        set: Vec::new(),
        schedule_seed: None,
        real_time: false,
        timeout: None,
        max_real_time: None,
        max_iterations: None,
        max_nodes: None,
        max_made: None,
        max_call_depth: None,
        max_memory: None,
        trace: TraceTo::Folder,
    };
    let (mut real_time, mut trace_file, mut no_trace) = (false, None, false);
    let common = parse_common(args, |option, rest| {
        match option {
            "--deny-warnings" => options.deny_warnings = true,
            "--real-time" => real_time = true,
            "--no-trace" => no_trace = true,
            "--lib-path" => options.lib_paths.push(PathBuf::from(rest.next().ok_or("--lib-path needs a folder")?)),
            "--sim" => options.sim = Some(PathBuf::from(rest.next().ok_or("--sim needs a file")?)),
            "--args" => options.args_file = Some(PathBuf::from(rest.next().ok_or("--args needs a file")?)),
            "--trace" => trace_file = Some(PathBuf::from(rest.next().ok_or("--trace needs a file")?)),
            "--arg" => {
                let text = text_of(option, rest.next())?;
                let (name, value) = text.split_once('=').ok_or_else(|| format!("--arg `{text}` has no `=`: write NAME=VALUE"))?;
                if !labcraft_format::is_name(name) {
                    return Err(format!("--arg: `{name}` is not a name, so no input has it"));
                }
                if options.args.iter().any(|(known, _)| known == name) {
                    return Err(format!("--arg gives the input `{name}` twice"));
                }
                options.args.push((name.to_owned(), value.to_owned()));
            }
            "--set" => {
                let text = text_of(option, rest.next())?;
                let (element, setting) = text.split_once('=').ok_or_else(|| format!("--set `{text}` has no `=`: write ELEMENT=VALUE@TIME"))?;
                let (value, time) =
                    setting.rsplit_once('@').ok_or_else(|| format!("--set `{text}` has no `@TIME`: write ELEMENT=VALUE@TIME, `@0s`"))?;
                if !labcraft_format::is_name(element) {
                    return Err(format!("--set: `{element}` is not a name, so no element has it"));
                }
                duration("--set", time, false)?;
                options.set.push((element.to_owned(), value.to_owned(), time.to_owned()));
            }
            "--schedule-seed" => {
                let text = text_of(option, rest.next())?;
                options.schedule_seed = Some(text.parse().map_err(|_| format!("--schedule-seed is a whole number from 0 to 2^64-1, not `{text}`"))?);
            }
            "--timeout" | "--max-real-time" => {
                let text = text_of(option, rest.next())?;
                duration(option, text, true)?;
                let slot = if option == "--timeout" { &mut options.timeout } else { &mut options.max_real_time };
                *slot = Some(text.to_owned());
            }
            "--max-iterations" | "--max-nodes" | "--max-made" => {
                let text = text_of(option, rest.next())?;
                let limit = if text == "none" { Limit::None } else { Limit::Count(count(option, text, 1, MAX_COUNT)?) };
                let slot = match option {
                    "--max-iterations" => &mut options.max_iterations,
                    "--max-nodes" => &mut options.max_nodes,
                    _ => &mut options.max_made,
                };
                *slot = Some(limit);
            }
            "--max-call-depth" => options.max_call_depth = Some(count(option, text_of(option, rest.next())?, 1, MAX_CALL_DEPTH)?),
            "--max-memory" => options.max_memory = Some(count(option, text_of(option, rest.next())?, MIN_MEMORY, MAX_COUNT)?),
            _ => return Ok(false),
        }
        Ok(true)
    })?;
    let mut positional = common.paths.into_iter();
    let (Some(folder), Some(function), None) = (positional.next(), positional.next(), positional.next()) else {
        return Err("run takes FOLDER and FUNCTION, and nothing else".to_owned());
    };
    let function = function.to_str().unwrap_or_default().to_owned();
    if !is_function_path(&function) {
        return Err(format!("`{function}` is not names joined by `.`: `check_voltage`, `dmm_driver.measure`"));
    }
    if real_time {
        // What only deterministic mode has (cli.md 6.2).
        if options.max_real_time.is_some() {
            return Err("--max-real-time bounds a deterministic run: with --real-time, --timeout counts real time already".to_owned());
        }
        if options.schedule_seed.is_some() {
            return Err("--schedule-seed orders a deterministic run: a run in real time takes its nodes as they are ready".to_owned());
        }
    }
    options.real_time = real_time;
    options.trace = match (trace_file, no_trace) {
        (Some(_), true) => return Err("--trace and --no-trace together".to_owned()),
        (Some(file), false) => {
            judge_trace_file(&file)?;
            TraceTo::File(file)
        }
        (None, true) => TraceTo::None,
        (None, false) => TraceTo::Folder,
    };
    options.json = common.json;
    options.limits = common.limits;
    options.folder = folder;
    options.function = function;
    Ok(options)
}

/// `run` takes a folder (`docs/cli.md`, section 6), and a file is wrong usage; a path that does not exist is data.
pub fn refuse_a_file_as_folder(options: &RunOptions) -> Result<(), String> {
    let is_a_file = |path: &&PathBuf| Path::metadata(path).is_ok_and(|metadata| !metadata.is_dir());
    match std::iter::once(&options.folder).chain(&options.lib_paths).find(is_a_file) {
        Some(file) => Err(format!("`{}` is a file, and run takes a folder", file.display())),
        None => Ok(()),
    }
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
            // `--trace` draws one function (`docs/cli.md`, 7.1), within at most 4 GiB (7.4).
            &["--svg", "--out", "d", "--trace", "t.jsonl", "."],
            &["--svg", "--out", "d", "--trace", "t.jsonl", ".", "f", "g"],
            &["--svg", "--out", "d", "--trace", "t.jsonl", "--trace", "u.jsonl", ".", "f"],
            &["--svg", "--out", "d", "--trace", "t.jsonl", "--max-trace-size", "0", ".", "f"],
            &["--svg", "--out", "d", "--trace", "t.jsonl", "--max-trace-size", "4294967297", ".", "f"],
            &["--svg", "--out", "d", "--deny-warnings", "."],
        ];
        for args in cases {
            assert!(diagram(args).is_err(), "{args:?}");
        }
        // After `--`, a path; and a value is not an option given twice.
        assert_eq!(diagram(&["--svg", "--out", "--out", "--", "--theme"]).unwrap().check.folders, [PathBuf::from("--theme")]);
        let traced = diagram(&["--svg", "--out", "d", "--trace", "t.jsonl", "--max-trace-size", "4294967296", ".", "f"]).unwrap();
        assert_eq!((traced.trace, traced.max_trace_size), (Some(PathBuf::from("t.jsonl")), 4 << 30));
        assert_eq!(diagram(&["--svg", "--out", "d", ".", "f"]).unwrap().max_trace_size, 512 << 20);
    }

    fn run(args: &[&str]) -> Result<RunOptions, String> {
        parse_run(&os(args))
    }

    #[test]
    fn run_takes_its_options() {
        let options = run(&[
            "examples",
            "check_voltage",
            "--json",
            "--arg",
            "resource=SIM::DMM0",
            "--arg",
            "low=220V",
            "--set",
            "note=a@b@500ms",
            "--sim",
            "b.toml",
            "--args",
            "a.json",
            "--timeout",
            "30s",
            "--max-iterations",
            "none",
            "--max-nodes",
            "5",
            "--max-call-depth",
            "4096",
            "--max-memory",
            "1048576",
            "--max-real-time",
            "1 min",
            "--schedule-seed",
            "18446744073709551615",
            "--no-trace",
            "--deny-warnings",
            "--lib-path",
            "l",
        ])
        .unwrap();
        assert_eq!((options.folder, options.function.as_str()), (PathBuf::from("examples"), "check_voltage"));
        assert_eq!(options.args, [("resource".to_owned(), "SIM::DMM0".to_owned()), ("low".to_owned(), "220V".to_owned())]);
        // The time is after the last `@`, so a value may hold one (cli.md 6.4).
        assert_eq!(options.set, [("note".to_owned(), "a@b".to_owned(), "500ms".to_owned())]);
        assert_eq!((options.sim, options.args_file), (Some(PathBuf::from("b.toml")), Some(PathBuf::from("a.json"))));
        assert_eq!((options.timeout.as_deref(), options.max_real_time.as_deref()), (Some("30s"), Some("1 min")));
        assert_eq!((options.max_iterations, options.max_nodes), (Some(Limit::None), Some(Limit::Count(5))));
        assert_eq!((options.max_call_depth, options.max_memory, options.schedule_seed), (Some(4096), Some(1 << 20), Some(u64::MAX)));
        assert!(options.json && options.deny_warnings && options.trace == TraceTo::None);
        assert_eq!(options.lib_paths, [PathBuf::from("l")]);
        let plain = run(&["f", "a.b"]).unwrap();
        assert!(plain.trace == TraceTo::Folder && plain.args.is_empty() && plain.timeout.is_none());
    }

    #[test]
    fn run_refuses_what_cli_6_2_calls_wrong_usage() {
        let cases: &[&[&str]] = &[
            &["f"],
            &["f", "g", "h"],
            &["f", "a..b"],
            &["f", "1x"],
            &["f", "g", "--arg", "x"],
            &["f", "g", "--arg", "next=1"],
            &["f", "g", "--arg", "x=1", "--arg", "x=2"],
            &["f", "g", "--set", "e=1"],
            &["f", "g", "--set", "e=1@soon"],
            &["f", "g", "--set", "e=1@-1s"],
            &["f", "g", "--set", "e=1@5"],
            &["f", "g", "--timeout", "30"],
            &["f", "g", "--timeout", "0s"],
            &["f", "g", "--timeout", "inf s"],
            &["f", "g", "--timeout", "1s", "--timeout", "2s"],
            &["f", "g", "--max-real-time", "-1s"],
            &["f", "g", "--max-nodes", "0"],
            &["f", "g", "--max-nodes", "9223372036854775808"],
            &["f", "g", "--max-call-depth", "4097"],
            &["f", "g", "--max-call-depth", "none"],
            &["f", "g", "--max-memory", "1048575"],
            &["f", "g", "--schedule-seed", "18446744073709551616"],
            &["f", "g", "--schedule-seed", "-1"],
            &["f", "g", "--real-time", "--max-real-time", "1s"],
            &["f", "g", "--schedule-seed", "1", "--real-time"],
            &["f", "g", "--trace", "t.jsonl", "--no-trace"],
            &["f", "g", "--trace", "main.LCF"],
            &["f", "g", "--trace", "args.Json"],
            &["f", "g", "--trace", "x.lcf."],
            &["f", "g", "--trace", "x.toml. ."],
            #[cfg(windows)]
            &["f", "g", "--trace", "p/x.lcf:s"],
            &["f", "g", "--unknown"],
            &["f", "g", "--sim"],
        ];
        for args in cases {
            assert!(run(args).is_err(), "{args:?}");
        }
        let message = run(&["f", "g", "--timeout", "30"]).unwrap_err();
        assert!(message.contains("`30s`"), "the form to write: {message}");
    }

    #[cfg(unix)]
    #[test]
    fn a_path_need_not_be_unicode() {
        use std::os::unix::ffi::OsStringExt as _;
        let path = OsString::from_vec(vec![b'a', 0xFF]);
        assert_eq!(parse_fmt(std::slice::from_ref(&path)).unwrap().paths, [PathBuf::from(path)]);
    }
}
