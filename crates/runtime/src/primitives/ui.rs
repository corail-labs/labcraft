//! `ui` (appendix A.2, spec 10): the panel of the call, which the host keeps.

use labcraft_ir::Static;

use super::{Call, Host, Outcome, wrong};
use crate::fault::Fault;

fn element(call: &Call<'_>, name: &str) -> Result<labcraft_ir::ElementId, Fault> {
    match call.statics.first() {
        Some(Static::Element(element)) => Ok(*element),
        _ => Err(wrong(name)),
    }
}

pub fn get(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let value = host.element(element(call, "ui.get")?)?;
    Ok(Outcome::Done(vec![value]))
}

/// Writes the element, and gives the value on, to order what follows (`after:`).
pub fn set(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let at = element(call, "ui.set")?;
    let value = call.inputs.first().cloned().ok_or_else(|| wrong("ui.set"))?;
    host.set_element(at, value.clone())?;
    Ok(Outcome::Done(vec![value]))
}
