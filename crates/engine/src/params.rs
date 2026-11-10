//! The parameters of a call, judged against the schema of the command (`docs/commands.md`, section 2): an unknown
//! parameter is an error, never ignored; a missing one takes its default, or is an error if it has none; a value of the
//! wrong JSON type, out of its range or not one of its values is `params.invalid`, naming the parameter.

use serde_json::{Map, Value};

use crate::error::{CommandError, did_you_mean};
use crate::registry::Command;

/// The parameters of a call, judged, with their defaults.
#[derive(Debug)]
pub struct Params {
    values: Map<String, Value>,
}

/// Judges the parameters of a call of `command`.
pub fn judge(command: &Command, params: &Value) -> Result<Params, CommandError> {
    let given = match params {
        Value::Object(given) => given,
        Value::Null => &Map::new(),
        _ => return Err(CommandError::new("params.invalid", "the parameters are a JSON object")),
    };
    let names = command.parameters();
    for name in given.keys() {
        if !names.contains(&name.as_str()) {
            let message = format!("unknown parameter `{name}`{}", did_you_mean(name, &names));
            return Err(CommandError::of("params.unknown", name, message).expecting(&names));
        }
    }
    let mut values = Map::new();
    for (name, schema) in &command.params.properties {
        match given.get(*name) {
            Some(value) => {
                let value = if schema["type"] == "integer" || schema["type"].as_array().is_some_and(|types| types.iter().any(|ty| ty == "integer")) {
                    as_integer(value).unwrap_or_else(|| value.clone())
                } else {
                    value.clone()
                };
                judge_value(name, schema, &value)?;
                values.insert((*name).to_owned(), value);
            }
            None if command.required().contains(name) => {
                return Err(CommandError::of("params.missing", name, format!("`{name}` is required")).expecting(command.required()));
            }
            None => {
                if let Some(default) = schema.get("default") {
                    values.insert((*name).to_owned(), default.clone());
                }
            }
        }
    }
    Ok(Params { values })
}

/// A number with no fraction is an integer, as JSON Schema 2020-12 counts it: `2.0` is `2`.
fn as_integer(value: &Value) -> Option<Value> {
    let number = value.as_f64().filter(|number| number.fract() == 0.0 && number.abs() < 9.007_199_254_740_992e15)?;
    if value.is_f64() {
        // Within 2^53, the conversion is exact.
        #[allow(clippy::cast_possible_truncation)]
        let integer = number as i64;
        return Some(Value::from(integer));
    }
    None
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_u64() || number.is_i64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// The value against the part of the schema the registry uses: `type`, `enum`, `minimum`, `maximum`, and `items`
/// and `properties` one level down.
fn judge_value(name: &str, schema: &Value, value: &Value) -> Result<(), CommandError> {
    let invalid = |message: String| CommandError::of("params.invalid", name, message);
    let types: Vec<&str> = match &schema["type"] {
        Value::String(one) => vec![one.as_str()],
        Value::Array(many) => many.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let found = type_name(value);
    if !types.is_empty() && !types.iter().any(|ty| *ty == found || (*ty == "number" && found == "integer")) {
        return Err(invalid(format!("`{name}` is {} ({found} given)", types.join(" or "))));
    }
    if let Some(allowed) = schema["enum"].as_array()
        && !allowed.contains(value)
    {
        let names: Vec<&str> = allowed.iter().filter_map(Value::as_str).collect();
        return Err(invalid(format!("`{name}` is one of {}", quoted(&names))).expecting(&names));
    }
    if let Some(number) = value.as_u64().map(u128::from).or_else(|| value.as_i64().map(|_| 0)) {
        let negative = value.as_i64().is_some_and(|signed| signed < 0);
        let minimum = schema["minimum"].as_u64();
        let maximum = schema["maximum"].as_u64();
        if let Some(minimum) = minimum
            && (negative || number < u128::from(minimum))
        {
            return Err(invalid(format!("`{name}` is at least {minimum}")));
        }
        if let Some(maximum) = maximum
            && !negative
            && number > u128::from(maximum)
        {
            return Err(invalid(format!("`{name}` is at most {maximum}")));
        }
    }
    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        for (index, item) in array.iter().enumerate() {
            judge_value(&format!("{name}[{index}]"), items, item)?;
        }
    }
    if let (Some(properties), Some(object)) = (schema["properties"].as_object(), value.as_object()) {
        let names: Vec<&str> = properties.keys().map(String::as_str).collect();
        if schema["additionalProperties"] == Value::Bool(false) {
            for key in object.keys() {
                if !properties.contains_key(key) {
                    let message = format!("`{name}` has no field `{key}`{}", did_you_mean(key, &names));
                    return Err(CommandError::of("params.unknown", &format!("{name}.{key}"), message).expecting(&names));
                }
            }
        }
        for required in schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if !object.contains_key(required) {
                return Err(CommandError::of("params.missing", &format!("{name}.{required}"), format!("`{name}` needs `{required}`")));
            }
        }
        for (key, item) in object {
            if let Some(item_schema) = properties.get(key) {
                judge_value(&format!("{name}.{key}"), item_schema, item)?;
            }
        }
    }
    Ok(())
}

fn quoted(names: &[&str]) -> String {
    names.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ")
}

impl Params {
    /// The value given or its default, if it has one.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.values.get(name).filter(|value| !value.is_null())
    }

    /// Whether the call gave `null` for the parameter: `max_iterations: null` removes the default (commands.md 6.4).
    pub fn is_null(&self, name: &str) -> bool {
        self.values.get(name).is_some_and(Value::is_null)
    }

    pub fn str(&self, name: &str) -> Option<&str> {
        self.get(name).and_then(Value::as_str)
    }

    pub fn bool(&self, name: &str) -> bool {
        self.get(name).and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn u64(&self, name: &str) -> Option<u64> {
        self.get(name).and_then(Value::as_u64)
    }

    /// A required string: the registry has made sure it is there.
    pub fn text(&self, name: &str) -> Result<&str, CommandError> {
        self.str(name).ok_or_else(|| CommandError::of("params.missing", name, format!("`{name}` is required")))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::registry::command;

    fn error_of(id: &str, params: Value) -> CommandError {
        judge(command(id).unwrap(), &params).unwrap_err()
    }

    #[test]
    fn a_misspelt_parameter_is_an_error_that_names_it_and_the_nearest() {
        let error = error_of("file.read", json!({ "path": "a.lcf", "lnes": 2 }));
        assert_eq!(error.code, "params.unknown");
        assert_eq!(error.parameter.as_deref(), Some("lnes"));
        assert!(error.message.contains("did you mean `lines`"), "{}", error.message);
        assert_eq!(error.expected, ["path", "from", "lines"]);
    }

    #[test]
    fn a_missing_parameter_without_a_default_is_an_error_and_one_with_a_default_takes_it() {
        let error = error_of("file.read", json!({}));
        assert_eq!((error.code, error.parameter.as_deref()), ("params.missing", Some("path")));
        let params = judge(command("file.read").unwrap(), &json!({ "path": "a.lcf" })).unwrap();
        assert_eq!(params.u64("from"), Some(1));
        assert_eq!(params.get("lines"), None);
    }

    #[test]
    fn a_value_of_the_wrong_type_or_out_of_range_is_invalid() {
        assert_eq!(error_of("file.read", json!({ "path": 3 })).code, "params.invalid");
        let error = error_of("file.read", json!({ "path": "a.lcf", "from": 0 }));
        assert_eq!((error.code, error.message.as_str()), ("params.invalid", "`from` is at least 1"));
        assert_eq!(error_of("file.read", json!({ "path": "a.lcf", "from": -1 })).code, "params.invalid");
        assert_eq!(error_of("file.read", json!({ "path": "a.lcf", "from": 1.5 })).code, "params.invalid");
        let params = judge(command("file.read").unwrap(), &json!({ "path": "a.lcf", "from": 2.0 })).unwrap();
        assert_eq!(params.u64("from"), Some(2), "2.0 is an integer");
        assert_eq!(error_of("file.read", json!([1])).code, "params.invalid");
    }

    #[test]
    fn a_value_out_of_an_enumeration_gives_the_choices() {
        let error = error_of("run.start", json!({ "function": "f", "mode": "fast" }));
        assert_eq!(error.code, "params.invalid");
        assert_eq!(error.expected, ["deterministic", "real_time"]);
    }

    #[test]
    fn the_items_of_an_array_are_judged_by_their_own_schema() {
        let error = error_of("run.start", json!({ "function": "f", "set": [{ "element": "s", "value": true, "at": "1 s", "when": 1 }] }));
        assert_eq!((error.code, error.parameter.as_deref()), ("params.unknown", Some("set[0].when")));
        let error = error_of("run.start", json!({ "function": "f", "set": [{ "element": "s", "value": true }] }));
        assert_eq!((error.code, error.parameter.as_deref()), ("params.missing", Some("set[0].at")));
    }

    #[test]
    fn null_is_a_value_of_a_limit_that_it_removes() {
        let params = judge(command("run.start").unwrap(), &json!({ "function": "f", "max_iterations": null })).unwrap();
        assert_eq!(params.get("max_iterations"), None);
        assert_eq!(error_of("run.start", json!({ "function": "f", "max_call_depth": null })).code, "params.invalid");
    }
}
