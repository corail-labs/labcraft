//! `labcraft [--lib-path DIR]... [--theme light|dark] [--style FILE] [FOLDER | FILE]` (`docs/cli.md`, section 8).

use std::ffi::OsString;
use std::path::PathBuf;

use labcraft_diagram::Theme;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// The folder given, or the project of the file given ([`crate::target`]).
    pub folder: Option<PathBuf>,
    /// The file given, from `folder`, its folders joined by `/`: shown once the project is open. Not read here.
    pub file: Option<String>,
    pub lib_paths: Vec<PathBuf>,
    /// The theme the viewer starts in; without it, the one of its preferences.
    pub theme: Option<Theme>,
    /// A file of values of the notation (notation 10), read again each time it is saved.
    pub style: Option<PathBuf>,
}

pub const USAGE: &str = "\
usage: labcraft [--lib-path DIR]... [--theme light|dark] [--style FILE] [FOLDER | FILE]

Opens FOLDER, a folder of LabCraft files, as a project in the window of the IDE, and draws its functions again
when its files change. Without FOLDER, the window opens on the start page. FILE, a .lcf or a .lct, opens the
nearest folder above it that was opened as a project (it holds .labcraft), else its own folder, and shows it; a
window that shows that project already shows it instead.

options:
  --lib-path DIR       also look for libraries in this folder; can be given several times
  --theme light|dark   the theme, instead of the one of the preferences
  --style FILE         draw with the values of the notation in FILE, read again each time it is saved;
                       written with the defaults when it does not exist
  --help               print this text
  --version            print the version
  --                   what follows is a path, even if it starts with -
";

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asked {
    Open(Options),
    Help,
    Version,
}

/// Reads the arguments after the name of the program; wrong usage is the message to print before exit 2.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Asked, String> {
    let mut options = Options::default();
    let mut args = args.into_iter();
    let mut paths_only = false;
    while let Some(arg) = args.next() {
        let text = arg.to_str().map(str::to_owned);
        match text.as_deref() {
            Some("--") if !paths_only => paths_only = true,
            Some("--help" | "-h") if !paths_only => return Ok(Asked::Help),
            Some("--version") if !paths_only => return Ok(Asked::Version),
            Some("--lib-path") if !paths_only => {
                let Some(path) = args.next() else { return Err("--lib-path needs a folder".to_owned()) };
                options.lib_paths.push(PathBuf::from(path));
            }
            Some("--theme") if !paths_only => {
                if options.theme.is_some() {
                    return Err("--theme is given twice".to_owned());
                }
                let theme = args.next().as_ref().and_then(|value| value.to_str()).and_then(Theme::from_name);
                options.theme = Some(theme.ok_or("--theme takes light or dark")?);
            }
            Some("--style") if !paths_only => {
                if options.style.is_some() {
                    return Err("--style is given twice".to_owned());
                }
                let Some(path) = args.next() else { return Err("--style needs a file".to_owned()) };
                options.style = Some(PathBuf::from(path));
            }
            Some(option) if !paths_only && option.starts_with('-') && option.len() > 1 => return Err(format!("unknown option `{option}`")),
            _ => {
                if options.folder.is_some() {
                    return Err("one folder or file at most".to_owned());
                }
                options.folder = Some(PathBuf::from(arg));
            }
        }
    }
    Ok(Asked::Open(options))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Result<Asked, String> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn a_folder_its_library_paths_and_a_theme() {
        assert_eq!(parsed(&[]), Ok(Asked::Open(Options::default())));
        assert_eq!(
            parsed(&["--lib-path", "a", "--theme", "dark", "--lib-path", "b", "examples"]),
            Ok(Asked::Open(Options {
                folder: Some("examples".into()),
                file: None,
                lib_paths: vec!["a".into(), "b".into()],
                theme: Some(Theme::Dark),
                style: None
            }))
        );
        assert_eq!(parsed(&["--", "-odd"]), Ok(Asked::Open(Options { folder: Some("-odd".into()), ..Options::default() })));
        assert_eq!(parsed(&["--help", "x"]), Ok(Asked::Help));
        assert_eq!(parsed(&["--style", "notation.toml"]), Ok(Asked::Open(Options { style: Some("notation.toml".into()), ..Options::default() })));
    }

    #[test]
    fn wrong_usage_says_why() {
        for (args, said) in [
            (&["a", "b"][..], "one folder or file at most"),
            (&["--theme", "blue"][..], "--theme takes light or dark"),
            (&["--theme", "dark", "--theme", "light"][..], "--theme is given twice"),
            (&["--lib-path"][..], "--lib-path needs a folder"),
            (&["--style"][..], "--style needs a file"),
            (&["--style", "a", "--style", "b"][..], "--style is given twice"),
            (&["--watch"][..], "unknown option `--watch`"),
        ] {
            assert_eq!(parsed(args), Err(said.to_owned()), "{args:?}");
        }
    }
}
