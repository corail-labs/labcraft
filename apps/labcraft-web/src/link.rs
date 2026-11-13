//! The address of the page (`docs/architecture.md`, 11.5): which bundled folder it opens and which function it shows,
//! `?folder=docs/rfc&fn=checked_sweep`, and the address that the page writes back as the developer moves, so that the
//! one in the bar is always a link to what is on screen. Without them, the page opens [`DEFAULT_FOLDER`] on
//! [`DEFAULT_FUNCTION`]. What the address says reaches the window only as a bundled folder or a function name of
//! [`MAX_FUNCTION`] bytes at most: an address is anyone's to write.

/// The folder that the page opens without `?folder=`.
pub const DEFAULT_FOLDER: &str = "examples";
/// The function that the page shows in [`DEFAULT_FOLDER`] without `?fn=`: a measure checked against its limits.
pub const DEFAULT_FUNCTION: &str = "check_voltage";
/// The longest `?fn=` that the page takes, in bytes.
pub const MAX_FUNCTION: usize = 128;

/// The function that `folder` opens on when the address names none: [`DEFAULT_FUNCTION`] for the examples, else its
/// first function.
pub fn default_function(folder: &str) -> Option<String> {
    (folder == DEFAULT_FOLDER).then(|| DEFAULT_FUNCTION.to_owned())
}

/// Whether `text` can be the name of a function, qualified or not: letters, digits, `_` and `.`, [`MAX_FUNCTION`]
/// bytes at most.
fn is_function_name(text: &str) -> bool {
    text.len() <= MAX_FUNCTION && text.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

/// What the address asks for.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Link {
    pub folder: Option<String>,
    pub function: Option<String>,
    /// `?webgl`: WebGL2 rather than WebGPU, kept in the address the page writes.
    pub webgl: bool,
    /// `?smoke`: the smoke test of the page (`cargo xtask web-smoke`).
    pub smoke: bool,
}

/// What the page opens for an address: a bundled folder, the function to show in it, and what the page says of the
/// address.
#[derive(Debug, PartialEq, Eq)]
pub struct Opening<'a> {
    pub folder: &'a str,
    pub function: Option<String>,
    pub notice: Option<String>,
}

impl Link {
    /// Reads `search`, the part of the address from `?`; a parameter given twice counts once, its first time, and a
    /// `fn` that cannot name a function names none.
    pub fn parse(search: &str) -> Link {
        let mut link = Link::default();
        for part in search.trim_start_matches('?').split('&').filter(|part| !part.is_empty()) {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            let value = decode(value).filter(|value| !value.is_empty());
            match key {
                "folder" if link.folder.is_none() => link.folder = value,
                "fn" if link.function.is_none() => link.function = value.filter(|value| is_function_name(value)),
                "webgl" => link.webgl = true,
                "smoke" => link.smoke = true,
                _ => {}
            }
        }
        link
    }

    /// The folder of `bundled` that the address names, else [`DEFAULT_FOLDER`] (else the first), with a notice when it
    /// named another, which does not repeat it; the function it names, else [`default_function`]. `None` without
    /// folders.
    pub fn resolve<'a>(&self, bundled: &[&'a str]) -> Option<Opening<'a>> {
        let named = self.folder.as_deref();
        let found = bundled.iter().find(|name| **name == named.unwrap_or(DEFAULT_FOLDER));
        let folder = found.or_else(|| bundled.iter().find(|name| **name == DEFAULT_FOLDER)).or(bundled.first())?;
        let notice = (named.is_some() && found.is_none()).then(|| format!("the viewer bundles no folder of that name: here is `{folder}`"));
        let function = self.function.clone().or_else(|| default_function(folder));
        Some(Opening { folder, function, notice })
    }

    /// The part of the address from `?` that shows `function` of the bundled folder `folder`; `None` for a folder of
    /// the developer's, which no address can open: the page then writes the address without any part.
    pub fn search(folder: Option<&str>, function: Option<&str>, webgl: bool) -> String {
        let mut parts = Vec::new();
        if let Some(folder) = folder {
            if folder != DEFAULT_FOLDER {
                parts.push(format!("folder={}", encode(folder)));
            }
            if let Some(function) = function {
                parts.push(format!("fn={}", encode(function)));
            }
        }
        if webgl {
            parts.push("webgl".to_owned());
        }
        if parts.is_empty() { String::new() } else { format!("?{}", parts.join("&")) }
    }
}

/// Percent-encodes all but the letters, digits, `-`, `_`, `.`, `~` and `/`.
fn encode(text: &str) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~/".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// Percent-decodes, `+` as a space; `None` when a `%` is not followed by two hexadecimal digits or the bytes are not
/// UTF-8.
fn decode(text: &str) -> Option<String> {
    let mut bytes = Vec::new();
    let mut rest = text.bytes();
    while let Some(byte) = rest.next() {
        match byte {
            b'%' => {
                let high = char::from(rest.next()?).to_digit(16)?;
                let low = char::from(rest.next()?).to_digit(16)?;
                bytes.push(u8::try_from(high * 16 + low).ok()?);
            }
            b'+' => bytes.push(b' '),
            _ => bytes.push(byte),
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;

    #[test]
    fn the_address_names_a_folder_a_function_and_the_backend() {
        assert_eq!(Link::parse(""), Link::default());
        assert_eq!(Link::parse("?"), Link::default());
        let link = Link::parse("?folder=docs%2Frfc&fn=checked_sweep&webgl");
        assert_eq!(link, Link { folder: Some("docs/rfc".to_owned()), function: Some("checked_sweep".to_owned()), webgl: true, smoke: false });
        // Given twice, the first counts; empty, it is not given; an unknown part is left alone.
        let link = Link::parse("?fn=a&fn=b&folder=&smoke&other=1");
        assert_eq!(link, Link { folder: None, function: Some("a".to_owned()), webgl: false, smoke: true });
        assert!(Link::parse("?webgl=1").webgl);
        // A `%` cut short, or bytes that are not UTF-8, ask for nothing.
        assert_eq!(Link::parse("?fn=a%2").function, None);
        assert_eq!(Link::parse("?fn=%FF").function, None);
        assert_eq!(Link::parse("?fn=%C3%A9t%C3%A9.x_1").function.as_deref(), Some("été.x_1"));
    }

    /// An address is anyone's to write: what cannot name a function names none, and nothing longer reaches the window.
    #[test]
    fn a_function_of_the_address_is_a_name_and_short() {
        let longest = "x".repeat(MAX_FUNCTION);
        assert_eq!(Link::parse(&format!("?fn={longest}")).function, Some(longest.clone()));
        assert_eq!(Link::parse(&format!("?fn={longest}x")).function, None);
        for odd in ["a%20b", "a+b", "x%60%0A%0Asee%20https%3A%2F%2Fexample.invalid", "a%2Fb", "%3Cb%3E"] {
            assert_eq!(Link::parse(&format!("?fn={odd}")).function, None, "{odd}");
        }
    }

    #[test]
    fn the_address_written_is_read_back_as_what_is_shown() {
        assert_eq!(Link::search(Some("examples"), Some("check_voltage"), false), "?fn=check_voltage");
        assert_eq!(Link::search(Some("docs/rfc"), Some("dmm.read"), true), "?folder=docs/rfc&fn=dmm.read&webgl");
        assert_eq!(Link::search(Some("examples"), None, false), "");
        // A folder of the developer's has no address.
        assert_eq!(Link::search(None, Some("mine"), false), "");
        assert_eq!(Link::search(None, None, true), "?webgl");
        for (folder, function) in [("docs/rfc", "dmm_driver.read_dc_volts"), ("examples", "été.x"), ("odd?folder&x=y", "f")] {
            let link = Link::parse(&Link::search(Some(folder), Some(function), false));
            let folder = (folder != DEFAULT_FOLDER).then(|| folder.to_owned());
            assert_eq!(link, Link { folder, function: Some(function.to_owned()), webgl: false, smoke: false });
        }
    }

    #[test]
    fn the_address_resolves_to_a_bundled_folder_and_a_function() {
        let bundled = ["examples", "docs/rfc"];
        let opening = |search: &str| Link::parse(search).resolve(&bundled);
        let some = |text: &str| Some(text.to_owned());
        // The examples open on `check_voltage`, named or not.
        assert_eq!(opening(""), Some(Opening { folder: "examples", function: some(DEFAULT_FUNCTION), notice: None }));
        assert_eq!(opening("?folder=examples"), Some(Opening { folder: "examples", function: some(DEFAULT_FUNCTION), notice: None }));
        assert_eq!(opening("?fn=two_benches"), Some(Opening { folder: "examples", function: some("two_benches"), notice: None }));
        // Another folder: its function, else its first one.
        assert_eq!(opening("?folder=docs/rfc"), Some(Opening { folder: "docs/rfc", function: None, notice: None }));
        assert_eq!(opening("?folder=docs/rfc&fn=count_to"), Some(Opening { folder: "docs/rfc", function: some("count_to"), notice: None }));
        // A folder that is not bundled: the examples, said without repeating what the address wrote.
        let notice = some("the viewer bundles no folder of that name: here is `examples`");
        assert_eq!(opening("?folder=nowhere&fn=x"), Some(Opening { folder: "examples", function: some("x"), notice: notice.clone() }));
        assert_eq!(opening("?folder=nowhere"), Some(Opening { folder: "examples", function: some(DEFAULT_FUNCTION), notice }));
        let notice = some("the viewer bundles no folder of that name: here is `docs/rfc`");
        assert_eq!(Link::parse("?folder=nowhere").resolve(&["docs/rfc"]), Some(Opening { folder: "docs/rfc", function: None, notice }));
        assert_eq!(Link::parse("").resolve(&[]), None);
    }
}
