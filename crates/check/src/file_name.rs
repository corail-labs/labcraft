//! What a name must be to name a file or a folder on every system (R-22): the file bears the name of what it declares,
//! two names of one kind differ by more than their case, and no name is one that Windows keeps for a device.

use std::borrow::Cow;

use labcraft_types::normalize;

/// The names that Windows keeps for a device, whatever their case and their extension.
const KEPT_BY_WINDOWS: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5",
    "lpt6", "lpt7", "lpt8", "lpt9",
];

/// The last segment of the path, with its extension.
pub(crate) fn file_name(shown: &str) -> &str {
    shown.rsplit(['/', '\\']).next().unwrap_or(shown)
}

/// The name of the file, normalized (R-20), without its folder or its extension.
pub(crate) fn stem(shown: &str) -> Cow<'_, str> {
    let name = file_name(shown);
    normalize(name.rsplit_once('.').map_or(name, |(stem, _)| stem))
}

/// Two names with one key cannot be two files of one folder where case does not count (Windows, macOS). Character by
/// character, so that a final sigma is a sigma, as for NTFS; normalized again, since a capital mu becomes a mu, which R-20
/// spells as the micro sign.
pub(crate) fn case_key(text: &str) -> String {
    let lower: String = normalize(text).chars().flat_map(char::to_lowercase).collect();
    normalize(&lower).into_owned()
}

pub(crate) fn kept_by_windows(text: &str) -> bool {
    KEPT_BY_WINDOWS.contains(&case_key(text).as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stem_drops_the_folders_and_the_extension() {
        assert_eq!(stem("lib/sub/Verdict.lct"), "Verdict");
        assert_eq!(stem(r"lib\scale.lcf"), "scale");
        assert_eq!(stem("scale"), "scale");
        assert_eq!(stem("e\u{301}.lcf"), "\u{e9}", "normalized");
        assert_eq!(file_name("lib/a.b.lct"), "a.b.lct");
    }

    #[test]
    fn windows_keeps_its_device_names_whatever_the_case() {
        for name in ["con", "CON", "Nul", "com1", "COM9", "lpt1", "aux", "prn"] {
            assert!(kept_by_windows(name), "{name}");
        }
        for name in ["console", "com10", "com0", "lpt0", "com", "lpt", "null", "con_"] {
            assert!(!kept_by_windows(name), "{name}");
        }
    }

    #[test]
    fn the_case_key_ignores_case_and_spelling() {
        assert_eq!(case_key("StepResult"), case_key("stepresult"));
        assert_eq!(case_key("\u{c9}t\u{e9}"), case_key("e\u{301}t\u{e9}".to_uppercase().as_str()));
        assert_ne!(case_key("step_result"), case_key("StepResult"));
        assert_eq!(case_key("a\u{3a3}"), case_key("a\u{3c3}"), "a final sigma is a sigma");
        assert_ne!(case_key("\u{391}\u{3a3}"), case_key("\u{3b1}\u{3c2}"), "and a final one stays itself");
        assert_eq!(case_key("\u{b5}a"), case_key("\u{39c}a"), "a micro sign and a capital mu");
    }
}
