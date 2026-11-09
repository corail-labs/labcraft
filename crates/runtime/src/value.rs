//! A value of a run (`docs/architecture.md`, section 10.3). What can be large is behind an `Arc`, so that a wire read by ten
//! nodes is not copied ten times, and a change works on a copy only when the value is shared (D-11). A number is held in
//! the unit of its type, which the value does not carry: its type does.

use std::sync::Arc;

use labcraft_ir::ConstId;

/// A value on a wire.
#[derive(Clone, Debug)]
pub enum Value {
    Bool(bool),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    F32(f32),
    F64(f64),
    C64(f32, f32),
    C128(f64, f64),
    String(Arc<str>),
    Bytes(Arc<[u8]>),
    /// Whole nanoseconds (spec 3.1).
    Duration(i64),
    /// Nanoseconds on the clock of the run; in real time, since 1970-01-01T00:00:00Z.
    Timestamp(i64),
    /// `none`, or an error (spec 7.1).
    Error(Option<Arc<ErrorData>>),
    /// One dimension: `[T]`, whose items may be arrays (`[[T]]`).
    Array(Arc<Items>),
    /// Two dimensions or more, rectangular: `[T; 2]` (spec 3.2).
    Grid(Arc<Grid>),
    /// The fields in declaration order; a `waveform` is `t0`, `dt`, `samples`.
    Struct(Arc<[Value]>),
    Variant(Arc<VariantValue>),
    /// A function reference: the constant it was written as (`docs/ir.md`, section 4), since a reference is always a
    /// constant of the program.
    Function(ConstId),
    /// What a library made, for an opaque type.
    Handle(Handle),
    /// The `length` of `array.slice` with no wire, which reaches the end of the array: no other value is ever this.
    ToTheEnd,
}

/// The items of an array, with their logical size, kept as they change so that pushing to an array in a loop does not
/// count the array again each time (cli.md 6.2).
#[derive(Clone, Debug, Default)]
pub struct Items {
    items: Vec<Value>,
    /// Of the items only: the array adds its own [`VALUE_COST`].
    size: u64,
}

impl Items {
    pub fn new(items: Vec<Value>) -> Self {
        let size = items.iter().fold(0_u64, |total, item| total.saturating_add(logical_size(item)));
        Items { items, size }
    }

    pub fn items(&self) -> &[Value] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn push(&mut self, item: Value) {
        self.size = self.size.saturating_add(logical_size(&item));
        self.items.push(item);
    }

    /// Replaces the item at `index`, if there is one.
    pub fn set(&mut self, index: usize, item: Value) -> bool {
        let Some(slot) = self.items.get_mut(index) else { return false };
        self.size = self.size.saturating_sub(logical_size(slot)).saturating_add(logical_size(&item));
        *slot = item;
        true
    }

    pub fn into_items(self) -> Vec<Value> {
        self.items
    }

    /// The logical size of the items.
    pub fn size(&self) -> u64 {
        self.size
    }
}

/// A rectangular array: its length at each level, then its items, row after row.
#[derive(Clone, Debug)]
pub struct Grid {
    shape: Box<[usize]>,
    items: Items,
}

impl Grid {
    /// `None` when the shape has fewer than two levels, or does not hold exactly the items given.
    pub fn new(shape: Box<[usize]>, items: Vec<Value>) -> Option<Self> {
        let count = shape.iter().try_fold(1_usize, |total, length| total.checked_mul(*length))?;
        (shape.len() >= 2 && count == items.len()).then(|| Grid { shape, items: Items::new(items) })
    }

    /// An empty grid of this many levels: no row.
    pub fn empty(dimensions: usize) -> Self {
        Grid { shape: vec![0; dimensions.max(2)].into_boxed_slice(), items: Items::default() }
    }

    pub fn shape(&self) -> &[usize] {
        &self.shape
    }

    pub fn items(&self) -> &[Value] {
        self.items.items()
    }

    pub fn size(&self) -> u64 {
        self.items.size()
    }

    /// How many rows: the length of its first level.
    pub fn rows(&self) -> usize {
        self.shape.first().copied().unwrap_or(0)
    }

    /// Items in one row, at every level below the first.
    pub fn row_len(&self) -> usize {
        self.shape.iter().skip(1).product()
    }

    /// The row at this index: an array of one level less, `[T]` for an `[T; 2]`.
    pub fn row(&self, index: usize) -> Option<Value> {
        if index >= self.rows() {
            return None;
        }
        let length = self.row_len();
        let start = index.checked_mul(length)?;
        let items = self.items().get(start..start.checked_add(length)?)?.to_vec();
        match self.shape.get(1..) {
            Some([_]) => Some(Value::array(items)),
            Some(inner) => Grid::new(inner.into(), items).map(|grid| Value::Grid(Arc::new(grid))),
            None => None,
        }
    }

    /// Its rows, each an array of one level less: what `array.set`, `push` and `concat` work on, which give an `[[T]]`
    /// (appendix A.2), since nothing keeps the lengths of rows equal there.
    pub fn to_rows(&self) -> Vec<Value> {
        (0..self.rows()).filter_map(|index| self.row(index)).collect()
    }
}

#[derive(Clone, Debug)]
pub struct VariantValue {
    /// Its index in its enumeration.
    pub index: u32,
    pub values: Box<[Value]>,
}

/// What an opaque value designates: the library that made it, by its index in the libraries of the run, and its number
/// there. [`Handle::INVALID`] is the default of an opaque type (spec 7.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Handle {
    pub library: u32,
    pub number: u64,
}

impl Handle {
    pub const INVALID: Handle = Handle { library: u32::MAX, number: 0 };

    pub fn is_valid(self) -> bool {
        self != Handle::INVALID
    }
}

/// An error (spec 7.1).
#[derive(Clone, Debug, PartialEq)]
pub struct ErrorData {
    pub category: Arc<str>,
    pub code: i64,
    pub message: Arc<str>,
    /// Where it was made, which the interpreter gives.
    pub origin: Option<Arc<Origin>>,
    /// Flat: the errors that `errors.merge` gathered, each without its own (commands.md 5). A `Vec` behind its `Arc`, so that a
    /// merge that nothing else holds appends in place, and a loop that gathers errors one by one is not quadratic.
    pub others: Arc<Vec<Arc<ErrorData>>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Origin {
    pub place: Place,
    /// Innermost first.
    pub callers: Arc<[Place]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub function: Arc<str>,
    pub node: Arc<str>,
    pub file: Arc<str>,
    pub line: u64,
}

impl Value {
    pub const NONE: Value = Value::Error(None);

    pub fn string(text: impl Into<Arc<str>>) -> Value {
        Value::String(text.into())
    }

    pub fn array(items: Vec<Value>) -> Value {
        Value::Array(Arc::new(Items::new(items)))
    }

    pub fn error(error: ErrorData) -> Value {
        Value::Error(Some(Arc::new(error)))
    }

    /// The error it holds, if it is an `error` that is not `none`.
    pub fn as_error(&self) -> Option<&Arc<ErrorData>> {
        match self {
            Value::Error(error) => error.as_ref(),
            _ => None,
        }
    }
}

/// What each value costs, before what it holds (cli.md 6.2).
pub const VALUE_COST: u64 = 16;

fn text_cost(bytes: usize) -> u64 {
    VALUE_COST.saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX))
}

/// The logical size of a value (cli.md 6.2): 16 bytes for each value, and inside it for each item of an array, each field
/// of a struct and each value of a variant; plus the bytes of each number by its type (1 for a `bool`, 4 for an `f32`, 8
/// for an `f64`, an `i64` or a `duration`, twice that for a complex number, 16 for a `timestamp`), and the bytes of a
/// `string` and of `bytes`. An error counts its fields as a struct does. A function reference and a handle count as an
/// `i64`. Arrays keep the size of their items, so this does not walk them.
pub fn logical_size(value: &Value) -> u64 {
    let own: u64 = match value {
        Value::Bool(_) | Value::I8(_) | Value::U8(_) => 1,
        Value::I16(_) | Value::U16(_) => 2,
        Value::I32(_) | Value::U32(_) | Value::F32(_) => 4,
        Value::I64(_) | Value::U64(_) | Value::F64(_) | Value::Duration(_) | Value::C64(..) => 8,
        Value::C128(..) | Value::Timestamp(_) => 16,
        Value::Function(_) | Value::Handle(_) => 8,
        Value::ToTheEnd => 4,
        Value::String(text) => return text_cost(text.len()),
        Value::Bytes(bytes) => return text_cost(bytes.len()),
        Value::Error(None) => 0,
        Value::Error(Some(error)) => error_size(error),
        Value::Array(items) => items.size(),
        Value::Grid(grid) => grid.size(),
        Value::Struct(fields) => fields.iter().fold(0_u64, |total, field| total.saturating_add(logical_size(field))),
        Value::Variant(variant) => variant.values.iter().fold(4_u64, |total, value| total.saturating_add(logical_size(value))),
    };
    VALUE_COST.saturating_add(own)
}

fn place_size(place: &Place) -> u64 {
    text_cost(place.function.len())
        .saturating_add(text_cost(place.node.len()))
        .saturating_add(text_cost(place.file.len()))
        .saturating_add(VALUE_COST + 8)
}

/// Its fields: category, code, message, origin, others.
pub(crate) fn error_size(error: &ErrorData) -> u64 {
    let origin = error.origin.as_ref().map_or(VALUE_COST, |origin| {
        let callers = origin.callers.iter().fold(VALUE_COST, |total, place| total.saturating_add(VALUE_COST).saturating_add(place_size(place)));
        VALUE_COST.saturating_add(place_size(&origin.place)).saturating_add(callers)
    });
    let others = error.others.iter().fold(VALUE_COST, |total, other| total.saturating_add(VALUE_COST).saturating_add(error_size(other)));
    text_cost(error.category.len())
        .saturating_add(VALUE_COST + 8)
        .saturating_add(text_cost(error.message.len()))
        .saturating_add(origin)
        .saturating_add(others)
}
