//! A `Drawing` as data, for the web viewer, whose session runs in a worker and sends its drawings to the page in a
//! compact binary form (`docs/architecture.md`, 11.5). Behind the feature `serde`: nothing else needs it.
//!
//! A token is written as its index in [`TOKENS`], the same in the worker and the page, which are one build; a token that
//! it does not name is an error, never a colour made up. A segment of the path of an icon is a type of `check`, written
//! here through a mirror of its variants.

use labcraft_check::PathSegment;
use serde::de::Error as _;
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::shape::Token;
use crate::theme::TOKENS;

fn index_of(token: Token) -> Option<u16> {
    TOKENS.iter().position(|value| value.name == token).and_then(|index| u16::try_from(index).ok())
}

fn token_at(index: u16) -> Option<Token> {
    TOKENS.get(usize::from(index)).map(|value| value.name)
}

pub(crate) mod token {
    use super::*;

    pub(crate) fn serialize<S: Serializer>(token: &Token, serializer: S) -> Result<S::Ok, S::Error> {
        match index_of(token) {
            Some(index) => index.serialize(serializer),
            None => Err(S::Error::custom(format!("`{token}` is not a token of the theme"))),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Token, D::Error> {
        let index = u16::deserialize(deserializer)?;
        token_at(index).ok_or_else(|| D::Error::custom(format!("no token of the theme has the index {index}")))
    }
}

pub(crate) mod option_token {
    use super::*;

    pub(crate) fn serialize<S: Serializer>(token: &Option<Token>, serializer: S) -> Result<S::Ok, S::Error> {
        match token {
            Some(token) => match index_of(token) {
                Some(index) => Some(index).serialize(serializer),
                None => Err(S::Error::custom(format!("`{token}` is not a token of the theme"))),
            },
            None => None::<u16>.serialize(serializer),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Token>, D::Error> {
        match Option::<u16>::deserialize(deserializer)? {
            Some(index) => token_at(index).map(Some).ok_or_else(|| D::Error::custom(format!("no token of the theme has the index {index}"))),
            None => Ok(None),
        }
    }
}

/// The variants of [`PathSegment`], as `check` gives them.
#[derive(Serialize, Deserialize)]
#[serde(remote = "PathSegment")]
enum Segment {
    Move((f64, f64)),
    Line((f64, f64)),
    Cubic((f64, f64), (f64, f64), (f64, f64)),
    Quadratic((f64, f64), (f64, f64)),
    Arc { center: (f64, f64), radii: (f64, f64), rotation: f64, start: f64, delta: f64 },
    Close,
}

#[derive(Serialize)]
struct Written<'a>(#[serde(with = "Segment")] &'a PathSegment);

#[derive(Deserialize)]
struct Read(#[serde(with = "Segment")] PathSegment);

pub(crate) mod segments {
    use super::*;

    pub(crate) fn serialize<S: Serializer>(segments: &[PathSegment], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(segments.iter().map(Written))
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<PathSegment>, D::Error> {
        Ok(Vec::<Read>::deserialize(deserializer)?.into_iter().map(|Read(segment)| segment).collect())
    }
}
