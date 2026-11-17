//! The inputs and outputs of an entry of `functions.list` (`docs/commands.md`, 6.1), as data: what the engine writes and
//! a client reads, so that no client reads the signature text again to find its types.

use serde::{Deserialize, Serialize};

use crate::{FitKey, MAX_NESTING, NumberKind, Unit, parse_unit};

/// What the rule of fit (spec 3.3, 3.4) judges a type by, in a form that holds outside a type table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SlotFit {
    /// A number: its kind as the language writes it (`f64`), and its unit, absent when it has none (`V`, `m/s^2`).
    Number {
        number: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
    },
    /// A type of no parts other than a number: `bool`, `string`, `bytes`, `timestamp`, `duration`, `error`.
    Base { base: String },
    /// A type variable of the signature, with its bound (`Number`), which takes any type its bound allows.
    Variable {
        variable: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bound: Option<String>,
    },
    /// A number that the call works out from the type variables or the parameters it names: `A·B`, `√N`, `f64[unit]`.
    Numbers { numbers: Vec<String> },
    /// An array of a type variable (`[T]`, `[[T]]`, `[T; 2]`): any array of its element, with its number of dimensions
    /// when it is written.
    Array {
        array: Box<SlotFit>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dimensions: Option<u32>,
    },
    /// Any other type, by its text, an alias followed to what it names: two such types match when their texts are equal,
    /// a `…` matching any part (`fn(…) -> (…)` takes any function).
    Shape { shape: String },
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// An input, an output, a field of a struct or a value of a variant.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    /// Empty for a value of a variant, which has none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// As the language writes it, as the signature does.
    #[serde(rename = "type", default)]
    pub ty: String,
    /// What the rule of fit reads; absent for an input that takes only a literal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit: Option<SlotFit>,
    /// An input with no default, and not variadic: a call wires it or fills it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub variadic: bool,
    /// An input wired by its name only: one with a default after a variadic one (spec 5.4, R-11).
    #[serde(default, skip_serializing_if = "is_false")]
    pub named_only: bool,
    /// An input that takes only text written in the call, never a wire: a `kind` (spec A.1), the template of
    /// `string.format` (5.4).
    #[serde(default, skip_serializing_if = "is_false")]
    pub literal: bool,
}

/// A variant of an enum, with its values.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantSlots {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<Slot>,
}

/// The parts of an entry that hold types: those of a function, or those of a type (a struct, an enum, an alias). An
/// opaque type has none.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Typed {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<Slot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<Slot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Slot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<VariantSlots>,
    /// What an alias names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is: Option<Slot>,
    /// The parts are cut: what is left out would have made the entry too large.
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

impl SlotFit {
    /// The fit of a type of no parts written as the language writes it (`f64`, `duration`), unit apart: None for any other.
    pub fn of_base(text: &str) -> Option<SlotFit> {
        if NumberKind::from_name(text).is_some() {
            return Some(SlotFit::Number { number: text.to_owned(), unit: None });
        }
        BASES.contains(&text).then(|| SlotFit::Base { base: text.to_owned() })
    }

    /// The key that [`FitKey::fit`] judges a number or a type of no parts by; None for any other, and for a unit the
    /// language does not read.
    pub fn key(&self) -> Option<FitKey> {
        match self {
            SlotFit::Number { number, unit } => {
                let unit = match unit {
                    None => Unit::ONE,
                    Some(unit) => parse_unit(unit, MAX_NESTING).ok()?,
                };
                Some(FitKey::Number(NumberKind::from_name(number)?, unit))
            }
            SlotFit::Base { base } => Some(match base.as_str() {
                "bool" => FitKey::Bool,
                "string" => FitKey::String,
                "bytes" => FitKey::Bytes,
                "timestamp" => FitKey::Timestamp,
                "duration" => FitKey::Duration,
                "error" => FitKey::Error,
                _ => return None,
            }),
            _ => None,
        }
    }
}

/// The types of no parts other than the numbers, as [`SlotFit::Base`] names them.
const BASES: [&str; 6] = ["bool", "string", "bytes", "timestamp", "duration", "error"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_form_of_fit_reads_back() {
        let slot = Slot {
            name: "limit".to_owned(),
            ty: "f64[V]".to_owned(),
            fit: Some(SlotFit::Number { number: "f64".to_owned(), unit: Some("V".to_owned()) }),
            required: true,
            ..Slot::default()
        };
        let written = serde_json::to_value(&slot).unwrap_or_default();
        assert_eq!(written, serde_json::json!({ "name": "limit", "type": "f64[V]", "fit": { "number": "f64", "unit": "V" }, "required": true }));
        for fit in [
            serde_json::json!({ "base": "duration" }),
            serde_json::json!({ "variable": "N", "bound": "Number" }),
            serde_json::json!({ "numbers": ["A", "B"] }),
            serde_json::json!({ "array": { "variable": "T" }, "dimensions": 2 }),
            serde_json::json!({ "shape": "visa.session" }),
        ] {
            let read: Option<SlotFit> = serde_json::from_value(fit.clone()).ok();
            assert_eq!(read.and_then(|read| serde_json::to_value(read).ok()), Some(fit));
        }
    }
}
