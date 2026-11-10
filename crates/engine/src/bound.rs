//! The bound of a result (`docs/commands.md`, section 2): at most 4 MiB of JSON *(proposed)*, whatever the project.
//! Each command bounds its own lists first; what still passes the bound is cut here, as section 5 cuts a value: a string
//! over 1,000 code points shows `{ "$len", "$head" }`, an array keeps the elements that fit and ends with
//! `{ "$cut": true }`, an object gives `{ "$cut": true }` as the value of a field that does not fit, and nothing nests
//! deeper than 64 levels. The result then says `"truncated": true` at its top level. The small fields of the top level
//! (at most 4 KiB each: the schema, the status, the counts, the summary, the id of a run, the error) are kept whole
//! first, then the large ones take what is left, in their order.

use serde_json::{Map, Value, json};

/// At most this much JSON in a result *(proposed)*.
pub const MAX_RESULT: usize = 4 << 20;
/// A string longer than this, in code points, is cut (section 5).
const MAX_STRING: usize = 1000;
/// Levels of arrays and objects, the marks of a cut included (section 5).
const MAX_DEPTH: usize = 64;
/// What a mark of a cut costs, at most, with the comma before it.
const MARK: usize = 16;

/// The size of a value as JSON, without spaces.
pub fn size(value: &Value) -> usize {
    serde_json::to_string(value).map_or(usize::MAX, |text| text.len())
}

fn cut_mark() -> Value {
    json!({ "$cut": true })
}

/// The mark of a cut, its cost taken from the budget.
fn charged_mark(budget: &mut usize) -> Value {
    *budget = budget.saturating_sub(MARK);
    cut_mark()
}

/// The result within the bound: as it is when it fits, cut otherwise, and then marked `truncated`.
pub fn fit(value: Value, max: usize) -> Value {
    if size(&value) <= max {
        return value;
    }
    let mut room = max.saturating_sub(MARK + "\"truncated\":true,".len());
    loop {
        let mut budget = room;
        let mut shaped = shape_top(value.clone(), &mut budget);
        if let Some(object) = shaped.as_object_mut() {
            object.insert("truncated".to_owned(), Value::Bool(true));
        }
        // The count above is close, not exact: what still passes is cut again, with less room.
        let over = size(&shaped).saturating_sub(max);
        if over == 0 || room == 0 {
            return shaped;
        }
        room = room.saturating_sub(over.saturating_mul(2).max(MARK));
    }
}

/// The fields of an object, in order.
type Fields = Vec<(String, Value)>;

/// A field of the top level this small is never cut.
const SMALL: usize = 4096;

/// The top level: its small fields whole, then the others.
fn shape_top(value: Value, budget: &mut usize) -> Value {
    let Value::Object(fields) = value else { return shape(value, budget, 1) };
    *budget = budget.saturating_sub(2);
    let (small, large): (Fields, Fields) = fields.into_iter().partition(|(key, field)| key_size(key) + size(field) <= SMALL);
    let mut kept = Map::new();
    for (key, field) in small {
        // Whole in size, but cut in depth and in its long strings as any value is.
        let mut room = SMALL;
        let shaped = shape(field, &mut room, 2);
        *budget = budget.saturating_sub(key_size(&key) + size(&shaped));
        kept.insert(key, shaped);
    }
    for (key, field) in large {
        let name = key_size(&key);
        if *budget <= name + MARK {
            kept.insert(key, cut_mark());
            continue;
        }
        *budget -= name;
        let before = *budget;
        *budget = budget.saturating_sub(MARK);
        let shaped = shape(field, budget, 2);
        *budget = (*budget + MARK).min(before);
        kept.insert(key, shaped);
    }
    Value::Object(kept)
}

/// A key with its quotes, its colon and its comma, its escapes counted.
fn key_size(key: &str) -> usize {
    serde_json::to_string(key).map_or(usize::MAX, |text| text.len()) + 2
}

/// Takes at most `budget` bytes of the value, and says how many are left.
fn shape(value: Value, budget: &mut usize, depth: usize) -> Value {
    match value {
        Value::String(text) if text.chars().count() > MAX_STRING => {
            let length = text.chars().count();
            let head: String = text.chars().take(MAX_STRING).collect();
            let cut = json!({ "$len": length, "$head": head });
            take(cut, budget, depth)
        }
        Value::Array(items) => {
            if depth >= MAX_DEPTH || *budget < MARK {
                return charged_mark(budget);
            }
            *budget = budget.saturating_sub(2);
            let mut kept = Vec::new();
            let mut items = items.into_iter();
            for item in items.by_ref() {
                if *budget <= MARK {
                    kept.push(cut_mark());
                    break;
                }
                let before = *budget;
                *budget = budget.saturating_sub(MARK);
                let shaped = shape(item, budget, depth + 1);
                let marked = is_mark(&shaped);
                *budget = (*budget + MARK).min(before);
                *budget = budget.saturating_sub(1);
                kept.push(shaped);
                if marked {
                    break;
                }
            }
            Value::Array(kept)
        }
        Value::Object(fields) => {
            if depth >= MAX_DEPTH || *budget < MARK {
                return charged_mark(budget);
            }
            *budget = budget.saturating_sub(2);
            let mut kept = Map::new();
            for (key, field) in fields {
                let name = key_size(&key);
                if *budget <= name + MARK {
                    kept.insert("$cut".to_owned(), Value::Bool(true));
                    break;
                }
                *budget -= name;
                let before = *budget;
                *budget = budget.saturating_sub(MARK);
                let shaped = shape(field, budget, depth + 1);
                *budget = (*budget + MARK).min(before);
                kept.insert(key, shaped);
            }
            Value::Object(kept)
        }
        leaf => take(leaf, budget, depth),
    }
}

fn is_mark(value: &Value) -> bool {
    value.as_object().is_some_and(|object| object.len() == 1 && object.get("$cut") == Some(&Value::Bool(true)))
}

/// A whole value, if it fits; the mark of a cut otherwise.
fn take(value: Value, budget: &mut usize, depth: usize) -> Value {
    let cost = size(&value);
    if cost <= *budget && depth <= MAX_DEPTH {
        *budget -= cost;
        value
    } else {
        charged_mark(budget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_that_fits_is_given_as_it_is() {
        let value = json!({ "a": [1, 2, 3], "b": "x".repeat(5000) });
        assert_eq!(fit(value.clone(), MAX_RESULT), value);
    }

    #[test]
    fn a_result_too_large_is_cut_within_the_bound_and_says_so() {
        let value = json!({ "file": "a.toml", "items": (0..100_000).map(|index| json!({ "name": format!("item {index}"), "text": "y".repeat(100) })).collect::<Vec<_>>() });
        let max = 64 * 1024;
        let shaped = fit(value, max);
        assert!(size(&shaped) <= max, "{}", size(&shaped));
        assert_eq!(shaped["truncated"], true);
        assert_eq!(shaped["file"], "a.toml", "what comes first stays");
        let items = shaped["items"].as_array().unwrap();
        assert!(items.len() > 100, "{}", items.len());
        assert_eq!(items.last().unwrap(), &json!({ "$cut": true }));
        assert_eq!(items[0], json!({ "name": "item 0", "text": "y".repeat(100) }));
    }

    #[test]
    fn the_status_and_the_counts_are_never_cut_whatever_their_place() {
        let diagnostics: Vec<Value> =
            (0..20_000).map(|index| json!({ "code": "LC2001", "message": format!("unknown name {index}"), "file": "a.lcf" })).collect();
        let value = json!({ "diagnostics": diagnostics, "schema": "labcraft-check/1", "status": "error", "summary": { "errors": 20_000 } });
        let shaped = fit(value, 64 * 1024);
        assert_eq!((shaped["schema"].as_str(), shaped["status"].as_str()), (Some("labcraft-check/1"), Some("error")));
        assert_eq!(shaped["summary"]["errors"], 20_000);
        assert_eq!(shaped["diagnostics"].as_array().unwrap().last().unwrap(), &json!({ "$cut": true }));
        assert!(size(&shaped) <= 64 * 1024);
    }

    #[test]
    fn keys_count_with_their_escapes() {
        let value: Map<String, Value> = (0..5000).map(|index| (format!("\u{1}{index}"), json!(index))).collect();
        let shaped = fit(json!({ "file": "a.toml", "map": value }), 16 * 1024);
        assert_eq!(shaped["file"], "a.toml");
        assert!(size(&shaped) <= 16 * 1024, "{}", size(&shaped));
        assert!(shaped["map"].as_object().unwrap().len() > 100);
    }

    #[test]
    fn a_long_string_shows_its_length_and_its_head() {
        let shaped = fit(json!({ "text": "é".repeat(3000), "more": "z".repeat(3000) }), 8000);
        assert_eq!(shaped["text"]["$len"], 3000);
        assert_eq!(shaped["text"]["$head"].as_str().unwrap().chars().count(), 1000);
        assert_eq!(shaped["more"]["$len"], 3000, "a small field is cut as any value is");
        assert!(size(&shaped) <= 8000);
    }

    #[test]
    fn nothing_nests_deeper_than_64_levels() {
        let mut value = json!("x".repeat(2000));
        for _ in 0..100 {
            value = json!([value]);
        }
        let shaped = fit(json!({ "deep": value }), 1000);
        let mut depth = 0;
        let mut at = &shaped;
        while let Some(inner) = at.as_array().and_then(|items| items.first()).or_else(|| at.get("deep")) {
            depth += 1;
            at = inner;
        }
        assert!(depth <= MAX_DEPTH, "{depth}");
    }
}
