//! `cargo xtask licences`: no GPL, LGPL or AGPL dependency (D-23), on any platform, tests and tools
//! included. A package offering another choice (`MIT OR GPL-2.0`) passes, since that choice is ours.
//! A package whose licence cannot be read is a violation: it cannot be said to pass.

use std::iter::Peekable;
use std::vec::IntoIter;

use serde_json::Value;

const REFUSED_PREFIXES: &[&str] = &["GPL", "LGPL", "AGPL"];

pub struct Package {
    name: String,
    version: String,
    /// The SPDX expression of `Cargo.toml`, absent when a package only gives a licence file.
    license: Option<String>,
}

pub fn from_metadata(metadata: &Value) -> Result<Vec<Package>, String> {
    let packages = metadata["packages"].as_array().ok_or("metadata: no packages")?;
    let mut out = Vec::new();
    for package in packages {
        out.push(Package {
            name: package["name"].as_str().ok_or("metadata: a package without a name")?.to_owned(),
            version: package["version"].as_str().unwrap_or_default().to_owned(),
            license: package["license"].as_str().map(str::to_owned),
        });
    }
    Ok(out)
}

pub fn check(packages: &[Package]) -> Vec<String> {
    let mut violations = Vec::new();
    for package in packages {
        let who = format!("{} {}", package.name, package.version);
        match package.license.as_deref().map(acceptable) {
            Some(Ok(true)) => {}
            Some(Ok(false)) => {
                violations.push(format!("{who}: `{}` leaves only a GPL family licence", package.license.as_deref().unwrap_or_default()))
            }
            Some(Err(error)) => violations.push(format!("{who}: {error}")),
            None => violations.push(format!("{who}: no SPDX licence in its manifest")),
        }
    }
    violations
}

/// Evaluates an SPDX expression: can it be taken without a licence of the GPL family?
fn acceptable(expression: &str) -> Result<bool, String> {
    let spaced = expression.replace('(', " ( ").replace(')', " ) ").replace('/', " OR ");
    let mut tokens = spaced.split_whitespace().map(str::to_owned).collect::<Vec<_>>().into_iter().peekable();
    let result = any_of(&mut tokens, expression)?;
    match tokens.next() {
        None => Ok(result),
        Some(token) => Err(format!("cannot read `{expression}` at `{token}`")),
    }
}

type Tokens = Peekable<IntoIter<String>>;

fn any_of(tokens: &mut Tokens, expression: &str) -> Result<bool, String> {
    let mut result = all_of(tokens, expression)?;
    while tokens.next_if(|token| token.eq_ignore_ascii_case("OR")).is_some() {
        result |= all_of(tokens, expression)?;
    }
    Ok(result)
}

fn all_of(tokens: &mut Tokens, expression: &str) -> Result<bool, String> {
    let mut result = one(tokens, expression)?;
    while tokens.next_if(|token| token.eq_ignore_ascii_case("AND")).is_some() {
        result &= one(tokens, expression)?;
    }
    Ok(result)
}

fn one(tokens: &mut Tokens, expression: &str) -> Result<bool, String> {
    let unreadable = || format!("cannot read `{expression}`");
    let token = tokens.next().ok_or_else(unreadable)?;
    if token == "(" {
        let result = any_of(tokens, expression)?;
        return if tokens.next().as_deref() == Some(")") { Ok(result) } else { Err(unreadable()) };
    }
    if token == ")" || ["OR", "AND", "WITH"].iter().any(|word| token.eq_ignore_ascii_case(word)) {
        return Err(unreadable());
    }
    // An exception does not change the family: `GPL-2.0 WITH Classpath-exception-2.0` stays refused.
    if tokens.next_if(|next| next.eq_ignore_ascii_case("WITH")).is_some() {
        tokens.next().ok_or_else(unreadable)?;
    }
    let upper = token.to_ascii_uppercase();
    Ok(!REFUSED_PREFIXES.iter().any(|prefix| upper.starts_with(prefix)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissive_licences_pass() {
        for expression in
            ["MIT", "Apache-2.0", "MIT OR Apache-2.0", "MIT/Apache-2.0", "Apache-2.0 WITH LLVM-exception", "(MIT OR Apache-2.0) AND Unicode-3.0"]
        {
            assert_eq!(acceptable(expression), Ok(true), "{expression}");
        }
    }

    #[test]
    fn the_gpl_family_is_refused() {
        for expression in [
            "GPL-3.0-only",
            "LGPL-2.1-or-later",
            "AGPL-3.0",
            "gpl-2.0",
            "MIT AND GPL-2.0",
            "GPL-2.0 WITH Classpath-exception-2.0",
            "(MIT OR Apache-2.0) AND LGPL-3.0",
        ] {
            assert_eq!(acceptable(expression), Ok(false), "{expression}");
        }
    }

    #[test]
    fn a_choice_that_avoids_the_family_passes() {
        for expression in ["MIT OR GPL-2.0", "GPL-2.0 OR MIT", "LGPL-3.0/Apache-2.0", "(GPL-2.0 AND MIT) OR BSD-3-Clause"] {
            assert_eq!(acceptable(expression), Ok(true), "{expression}");
        }
    }

    #[test]
    fn an_unreadable_expression_is_an_error() {
        for expression in ["", "MIT OR", "AND MIT", "(MIT", "MIT)", "MIT Apache-2.0", "MIT WITH", "OR"] {
            assert!(acceptable(expression).is_err(), "{expression}");
        }
    }

    #[test]
    fn every_failure_is_a_violation() {
        let package = |license: Option<&str>| Package { name: "p".to_owned(), version: "1".to_owned(), license: license.map(str::to_owned) };
        assert!(check(&[package(Some("MIT"))]).is_empty());
        for license in [Some("GPL-3.0"), Some("MIT OR"), None] {
            assert_eq!(check(&[package(license)]).len(), 1, "{license:?}");
        }
    }

    #[test]
    fn metadata_gives_the_model() {
        let metadata = serde_json::json!({"packages": [{"name": "a", "version": "1.0.0", "license": "GPL-3.0"}, {"name": "b", "version": "2.0.0", "license": null}]});
        let found = check(&from_metadata(&metadata).unwrap());
        assert!(found[0].starts_with("a 1.0.0"), "{found:?}");
        assert!(found[1].starts_with("b 2.0.0"), "{found:?}");
    }
}
