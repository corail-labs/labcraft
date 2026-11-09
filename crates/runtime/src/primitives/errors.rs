//! `errors` (appendix A.2, spec 7.4).

use std::sync::Arc;

use labcraft_ir::Static;

use super::{Call, Host, Outcome, input, room_for, wrong};
use crate::fault::Fault;
use crate::value::{ErrorData, VALUE_COST, Value, error_size};

fn category<'s>(call: &'s Call<'_>, name: &str) -> Result<&'s str, Fault> {
    call.statics
        .iter()
        .find_map(|item| match item {
            Static::Category(category) => Some(&**category),
            _ => None,
        })
        .ok_or_else(|| wrong(name))
}

fn message(call: &Call<'_>, at: usize, name: &str) -> Result<String, Fault> {
    match input(call, at)? {
        Value::String(text) => Ok(text.to_string()),
        _ => Err(wrong(name)),
    }
}

/// An error of the category and the code given.
pub fn make(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let Value::I32(code) = *input(call, 0)? else { return Err(wrong("errors.make")) };
    let message = message(call, 1, "errors.make")?;
    Ok(Outcome::Done(vec![host.error(category(call, "errors.make")?, i64::from(code), message)]))
}

/// The error of the category given when the condition is true, `none` otherwise.
pub fn raise_if(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let Value::Bool(condition) = *input(call, 0)? else { return Err(wrong("errors.raise_if")) };
    if !condition {
        return Ok(Outcome::Done(vec![Value::NONE]));
    }
    let message = message(call, 1, "errors.raise_if")?;
    Ok(Outcome::Fail(host.error(category(call, "errors.raise_if")?, 0, message)))
}

/// The first argument that holds an error becomes the main error, and the others go into its `others`, which stays flat:
/// its own, then each other one without its own, followed by its own (7.4, commands.md 5). With no error, `none`.
/// The main error changes in place when nothing else holds it; it asks for the room of what it adds, which is a copy of the
/// errors it gathers when they are shared, so `errors.merge(e, e)` in a loop reaches the memory limit of the run.
pub fn merge(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let variadic = call.variadic.ok_or_else(|| wrong("errors.merge"))?;
    let start = variadic.start as usize;
    let parts = call.inputs.get_mut(start..start.saturating_add(variadic.count as usize)).ok_or_else(|| wrong("errors.merge"))?;
    let mut errors = parts.iter_mut().filter_map(|part| match std::mem::replace(part, Value::NONE) {
        Value::Error(Some(error)) => Some(error),
        _ => None,
    });
    let Some(mut main) = errors.next() else { return Ok(Outcome::Done(vec![Value::NONE])) };
    let rest: Vec<Arc<ErrorData>> = errors.collect();
    if rest.is_empty() {
        return Ok(Outcome::Done(vec![Value::Error(Some(main))]));
    }
    let mut added = 0_u64;
    let mut gathered = Vec::new();
    for other in &rest {
        let mut alone = ErrorData::clone(other);
        alone.others = Arc::default();
        added = added.saturating_add(VALUE_COST).saturating_add(error_size(&alone));
        gathered.push(Arc::new(alone));
        for inner in other.others.iter() {
            added = added.saturating_add(VALUE_COST).saturating_add(error_size(inner));
            gathered.push(Arc::clone(inner));
        }
    }
    room_for(host, added, "errors.merge")?;
    Arc::make_mut(&mut Arc::make_mut(&mut main).others).extend(gathered);
    Ok(Outcome::Done(vec![Value::Error(Some(main))]))
}
