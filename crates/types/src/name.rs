//! Names of every kind, in their one spelling (R-20).

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use unicode_normalization::{UnicodeNormalization, is_nfc};

const GREEK_MU: char = '\u{03BC}';
const MICRO_SIGN: &str = "\u{00B5}";

/// The one spelling of a name or a unit (R-20): Unicode NFC, then U+03BC becomes U+00B5.
pub fn normalize(text: &str) -> Cow<'_, str> {
    if text.is_ascii() {
        return Cow::Borrowed(text);
    }
    let composed = if is_nfc(text) { Cow::Borrowed(text) } else { Cow::Owned(text.nfc().collect::<String>()) };
    if composed.contains(GREEK_MU) { Cow::Owned(composed.replace(GREEK_MU, MICRO_SIGN)) } else { composed }
}

/// A normalized name, as an index in the [`Interner`] that made it: a file of 16 MiB can hold 8
/// million occurrences of a name, and a pointer per occurrence would threaten the memory bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Name(u32);

#[derive(Clone, Debug, Default)]
pub struct Interner {
    indices: HashMap<Arc<str>, u32>,
    names: Vec<Arc<str>>,
}

impl Interner {
    pub fn new() -> Self {
        Self::default()
    }

    /// The only way to make a [`Name`], so that every name is normalized.
    pub fn intern(&mut self, text: &str) -> Name {
        let text = normalize(text);
        if let Some(index) = self.indices.get(text.as_ref()) {
            return Name(*index);
        }
        // More than 2^32 different names cannot fit under `Limits`; the last one stands for them.
        let index = u32::try_from(self.names.len()).unwrap_or(u32::MAX);
        let text: Arc<str> = Arc::from(text.as_ref());
        self.names.push(Arc::clone(&text));
        self.indices.insert(text, index);
        Name(index)
    }

    /// The name of this text, normalized, if something made it already: what no one interned is the name of nothing.
    pub fn find(&self, text: &str) -> Option<Name> {
        self.indices.get(normalize(text).as_ref()).map(|index| Name(*index))
    }

    /// The text of a name; empty for a name that another interner made.
    pub fn resolve(&self, name: Name) -> &str {
        self.names.get(name.0 as usize).map_or("", AsRef::as_ref)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec 14.1, version of Unicode: an update of the crate that moves it changes the spec in the same pull request.
    #[test]
    fn names_are_normalized_with_the_unicode_of_the_spec() {
        const SPEC: &str = include_str!("../../../docs/language-spec-v0.md");
        let (major, minor, _) = unicode_normalization::UNICODE_VERSION;
        assert!(SPEC.contains(&format!("Names are read with Unicode {major}.{minor}:")), "Unicode {major}.{minor}");
    }

    #[test]
    fn the_same_text_is_the_same_name() {
        let mut names = Interner::new();
        let voltage = names.intern("voltage");
        assert_eq!(names.intern("voltage"), voltage);
        assert_ne!(names.intern("current"), voltage);
        assert_eq!(names.resolve(voltage), "voltage");
        assert_eq!(names.find("voltage"), Some(voltage));
        assert_eq!(names.find("power"), None, "finding a name makes none");
        assert_eq!(names.find("power"), None);
    }

    #[test]
    fn composed_and_decomposed_are_one_name() {
        let mut names = Interner::new();
        assert_eq!(names.intern("\u{00E9}t\u{00E9}"), names.intern("e\u{0301}te\u{0301}"));
        let composed = names.intern("e\u{0301}");
        assert_eq!(names.resolve(composed), "\u{00E9}");
    }

    #[test]
    fn ohm_and_omega_are_one_name() {
        let mut names = Interner::new();
        assert_eq!(names.intern("\u{2126}"), names.intern("\u{03A9}"));
    }

    #[test]
    fn mu_and_the_micro_sign_are_one_name() {
        let mut names = Interner::new();
        let micro = names.intern("\u{00B5}V");
        assert_eq!(names.intern("\u{03BC}V"), micro);
        assert_eq!(names.resolve(micro), "\u{00B5}V");
    }

    #[test]
    fn compatibility_forms_stay_different() {
        // NFKC would make them one name; R-20 does not.
        let mut names = Interner::new();
        assert_ne!(names.intern("\u{FB01}"), names.intern("fi"));
    }

    #[test]
    fn a_name_of_another_interner_does_not_fail() {
        let mut other = Interner::new();
        other.intern("a");
        let foreign = other.intern("b");
        assert_eq!(Interner::new().resolve(foreign), "");
    }
}
