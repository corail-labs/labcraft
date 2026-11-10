//! Values in JSON (`docs/commands.md`, 5), against their types, through the writer of `trace`, so that the report, the trace
//! and the reader of the viewer cut a value the same way (`docs/architecture.md`, 10.7). `trace` never sees a [`Value`]:
//! this module turns one into calls of its [`ValueWriter`].
//!
//! It recurses on a value as deep as its type, as the rest of `runtime` does; the writer stops writing at 64 levels.

use labcraft_ir::ConstValue;
use labcraft_trace::value::OTHERS_MAX;
use labcraft_trace::{ErrorValue, Origin as TraceOrigin, Place as TracePlace, ValueWriter, WriteValue};
use labcraft_types::{LibraryId, Ty, TyId, Unit};

use crate::convert::Env;
use crate::native::HandleInfo;
use crate::value::{ErrorData, Handle, Place, Value};

/// What a library says of a handle at the time a value is written: the report and the trace show whether it is open.
pub trait Describe {
    fn describe(&self, handle: Handle) -> Option<HandleInfo>;
}

/// No library: a handle shows only its type.
pub struct NoHandles;

impl Describe for NoHandles {
    fn describe(&self, _: Handle) -> Option<HandleInfo> {
        None
    }
}

/// What writes values: their types, the library they are seen from, which names their types (`Verdict`, `scpi.Reading`),
/// and what describes their handles.
#[derive(Clone, Copy)]
pub struct Json<'a> {
    pub env: Env<'a>,
    pub viewer: LibraryId,
    pub handles: &'a dyn Describe,
}

/// A value with its type, which [`ValueWriter`] can write.
pub struct Typed<'a> {
    pub json: Json<'a>,
    pub value: &'a Value,
    pub ty: TyId,
}

impl WriteValue for Typed<'_> {
    fn write_value(&self, writer: &mut ValueWriter<'_>) {
        self.json.value(writer, self.value, self.ty);
    }
}

impl<'a> Json<'a> {
    pub fn typed(self, value: &'a Value, ty: TyId) -> Typed<'a> {
        Typed { json: self, value, ty }
    }

    /// The text of a type as a report and the trace write it: `f64[V]`, `visa.session`, `[results_log.StepResult]`.
    pub fn type_text(&self, ty: TyId) -> String {
        self.env.program.types.display(ty, self.viewer, self.env.names)
    }

    pub fn value(&self, writer: &mut ValueWriter<'_>, value: &Value, ty: TyId) {
        match (value, self.env.ty(ty)) {
            (Value::Array(items), Ty::Array { element, .. }) => {
                let count = writer.begin_array(items.len());
                for item in items.items().iter().take(count) {
                    if writer.spent() {
                        break;
                    }
                    self.value(writer, item, *element);
                }
                writer.end();
            }
            (Value::Grid(grid), Ty::Array { element, .. }) => self.grid_level(writer, grid.items(), grid.shape(), *element),
            (Value::Struct(fields), Ty::Struct(declared)) => {
                writer.begin_object();
                for (field, (name, field_ty)) in fields.iter().zip(declared) {
                    if writer.spent() {
                        break;
                    }
                    writer.key(self.env.names.resolve(*name));
                    self.value(writer, field, *field_ty);
                }
                writer.end();
            }
            (Value::Struct(fields), Ty::Waveform(element)) => {
                let [t0, dt, samples] = &**fields else {
                    writer.null();
                    return;
                };
                writer.begin_object();
                writer.key("t0");
                self.scalar(writer, t0, Unit::ONE);
                writer.key("dt");
                self.scalar(writer, dt, Unit::ONE);
                writer.key("samples");
                match samples {
                    Value::Array(items) => {
                        let count = writer.begin_array(items.len());
                        for item in items.items().iter().take(count) {
                            if writer.spent() {
                                break;
                            }
                            self.value(writer, item, *element);
                        }
                        writer.end();
                    }
                    _ => writer.null(),
                }
                writer.end();
            }
            (Value::Variant(variant), Ty::Enum(variants)) => {
                let Some(declared) = variants.get(variant.index as usize) else {
                    writer.null();
                    return;
                };
                let variant_name = self.env.names.resolve(declared.name);
                let name = match self.env.program.types.get(ty) {
                    Ty::Named { .. } => format!("{}.{variant_name}", self.type_text(ty)),
                    _ => variant_name.to_owned(),
                };
                if variant.values.is_empty() {
                    writer.variant(&name);
                    return;
                }
                let count = writer.begin_variant(&name, variant.values.len());
                for (value, value_ty) in variant.values.iter().zip(&declared.values).take(count) {
                    if writer.spent() {
                        break;
                    }
                    self.value(writer, value, *value_ty);
                }
                writer.end_variant();
            }
            (Value::Function(constant), _) => {
                writer.begin_object();
                writer.key("function");
                match self.env.program.constants.get(constant.0 as usize).map(|constant| &constant.value) {
                    Some(ConstValue::Function(function)) => match self.env.program.functions.get(function.0 as usize) {
                        Some(function) => writer.string(&function.shown),
                        None => writer.null(),
                    },
                    Some(ConstValue::Primitive { library, name, .. }) => {
                        writer.string(&format!("{}.{}", library.as_str(), self.env.names.resolve(*name)));
                    }
                    // The default of a function type, which names no function (Q-21).
                    _ => writer.null(),
                }
                writer.end();
            }
            (Value::Handle(handle), _) => {
                writer.begin_object();
                writer.key("opaque");
                writer.string(&self.type_text(ty));
                if let Some(info) = handle.is_valid().then(|| self.handles.describe(*handle)).flatten() {
                    writer.key("resource");
                    writer.string(&info.designates);
                    writer.key("open");
                    writer.bool(info.open);
                } else {
                    writer.key("open");
                    writer.bool(false);
                }
                writer.end();
            }
            (value, defined) => {
                let unit = match defined {
                    Ty::Number { unit, .. } => *unit,
                    _ => Unit::ONE,
                };
                self.scalar(writer, value, unit);
            }
        }
    }

    /// One level of a rectangular array: its rows, or at the last level its items.
    fn grid_level(&self, writer: &mut ValueWriter<'_>, items: &[Value], shape: &[usize], element: TyId) {
        let Some((length, inner)) = shape.split_first() else { return };
        let count = writer.begin_array(*length);
        let stride: usize = inner.iter().product();
        for index in 0..count {
            if writer.spent() {
                break;
            }
            if inner.is_empty() {
                match items.get(index) {
                    Some(item) => self.value(writer, item, element),
                    None => writer.null(),
                }
            } else {
                let start = index.saturating_mul(stride);
                let part = items.get(start..start.saturating_add(stride)).unwrap_or_default();
                self.grid_level(writer, part, inner, element);
            }
        }
        writer.end();
    }

    fn scalar(&self, writer: &mut ValueWriter<'_>, value: &Value, unit: Unit) {
        let text = if unit == Unit::ONE { None } else { Some(unit.to_string()) };
        let unit = text.as_deref();
        match value {
            Value::Bool(value) => writer.bool(*value),
            Value::I8(x) => writer.i64(i64::from(*x), unit),
            Value::I16(x) => writer.i64(i64::from(*x), unit),
            Value::I32(x) => writer.i64(i64::from(*x), unit),
            Value::I64(x) => writer.i64(*x, unit),
            Value::U8(x) => writer.u64(u64::from(*x), unit),
            Value::U16(x) => writer.u64(u64::from(*x), unit),
            Value::U32(x) => writer.u64(u64::from(*x), unit),
            Value::U64(x) => writer.u64(*x, unit),
            Value::F32(x) => writer.f32(*x, unit),
            Value::F64(x) => writer.f64(*x, unit),
            Value::C64(re, im) => writer.complex(f64::from(*re), f64::from(*im), true, unit),
            Value::C128(re, im) => writer.complex(*re, *im, false, unit),
            Value::String(text) => writer.string(text),
            Value::Bytes(bytes) => writer.bytes(bytes),
            Value::Duration(ns) => writer.duration(*ns),
            Value::Timestamp(ns) => writer.timestamp(*ns),
            Value::Error(None) => writer.null(),
            Value::Error(Some(error)) => write_error(writer, error),
            // A value that does not hold its type, or the `ToTheEnd` of `array.slice`, which no wire carries: nothing to say.
            Value::Array(_) | Value::Grid(_) | Value::Struct(_) | Value::Variant(_) | Value::Function(_) | Value::Handle(_) | Value::ToTheEnd => {
                writer.null()
            }
        }
    }
}

/// An error of spec 7.1, with the errors that `errors.merge` gathered.
pub fn write_error(writer: &mut ValueWriter<'_>, error: &ErrorData) {
    let callers: Vec<TracePlace<'_>> = error.origin.as_ref().map(|origin| origin.callers.iter().map(place).collect()).unwrap_or_default();
    // Only the others that are written: a merge of a million errors costs 32 here, not a million, at each event.
    let shown = error.others.get(..error.others.len().min(OTHERS_MAX)).unwrap_or_default();
    let others: Vec<Vec<TracePlace<'_>>> =
        shown.iter().map(|other| other.origin.as_ref().map(|origin| origin.callers.iter().map(place).collect()).unwrap_or_default()).collect();
    let others: Vec<ErrorValue<'_>> = shown.iter().zip(&others).map(|(other, callers)| error_value(other, callers, &[])).collect();
    let mut value = error_value(error, &callers, &others);
    value.others_left = (error.others.len() - shown.len()) as u64;
    writer.error(&value);
}

fn error_value<'a>(error: &'a ErrorData, callers: &'a [TracePlace<'a>], others: &'a [ErrorValue<'a>]) -> ErrorValue<'a> {
    ErrorValue {
        category: &error.category,
        code: error.code,
        message: &error.message,
        origin: error.origin.as_ref().map(|origin| TraceOrigin { place: place(&origin.place), callers }),
        others,
        others_left: 0,
    }
}

fn place(place: &Place) -> TracePlace<'_> {
    TracePlace { function: &place.function, node: &place.node, file: &place.file, line: place.line }
}
