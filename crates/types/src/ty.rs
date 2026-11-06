//! Types, interned: a value refers to its type by [`TyId`]. So `a1 = [a0]`, `a2 = [a1]`… cannot make
//! a deep structure to drop, compare or print, and depth has a limit (`LC1006`).

use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;

use crate::limits::MAX_NESTING;
use crate::name::{Interner, Name};
use crate::report::{MAX_TEXT, cut_text};
use crate::unit::Unit;

/// `LC1006`: an array of more than 32 dimensions *(bound proposed)*.
pub const MAX_ARRAY_DIMENSIONS: u32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TyId(u32);

impl TyId {
    /// [`Ty::Error`], in every table.
    pub const ERROR: TyId = TyId(0);
}

/// A library once `check` has found it. Two libraries can bear one name, and their types are
/// different: a folder `visa` in a project replaces `visa` for that project, not for `scpi`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LibraryId(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumberKind {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    C64,
    C128,
}

impl NumberKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NumberKind::I8 => "i8",
            NumberKind::I16 => "i16",
            NumberKind::I32 => "i32",
            NumberKind::I64 => "i64",
            NumberKind::U8 => "u8",
            NumberKind::U16 => "u16",
            NumberKind::U32 => "u32",
            NumberKind::U64 => "u64",
            NumberKind::F32 => "f32",
            NumberKind::F64 => "f64",
            NumberKind::C64 => "c64",
            NumberKind::C128 => "c128",
        }
    }

    pub fn is_integer(self) -> bool {
        !self.is_inexact()
    }

    /// A floating-point or complex number, which a change of scale can go to (spec 3.4).
    pub fn is_inexact(self) -> bool {
        matches!(self, NumberKind::F32 | NumberKind::F64 | NumberKind::C64 | NumberKind::C128)
    }

    pub fn is_complex(self) -> bool {
        matches!(self, NumberKind::C64 | NumberKind::C128)
    }

    /// Bits of an integer, with whether it has a sign.
    fn integer(self) -> Option<(u32, bool)> {
        match self {
            NumberKind::I8 => Some((8, true)),
            NumberKind::I16 => Some((16, true)),
            NumberKind::I32 => Some((32, true)),
            NumberKind::I64 => Some((64, true)),
            NumberKind::U8 => Some((8, false)),
            NumberKind::U16 => Some((16, false)),
            NumberKind::U32 => Some((32, false)),
            NumberKind::U64 => Some((64, false)),
            _ => None,
        }
    }

    /// The closed list of implicit widening (spec 3.4): nothing can be lost. A kind does not widen to itself.
    pub fn widens_to(self, other: NumberKind) -> bool {
        match (self.integer(), other.integer()) {
            (Some((bits, signed)), Some((other_bits, other_signed))) => match (signed, other_signed) {
                (true, true) | (false, false) => bits < other_bits,
                (false, true) => bits < other_bits,
                (true, false) => false,
            },
            (Some((bits, _)), None) => other == NumberKind::F64 && bits <= 32,
            (None, _) => matches!((self, other), (NumberKind::F32, NumberKind::F64) | (NumberKind::C64, NumberKind::C128)),
        }
    }

    /// The smallest kind that both widen to (spec 3.4): `i32` and `f64` give `f64`, and so do `i32` and `f32`.
    pub fn meeting(self, other: NumberKind) -> Option<NumberKind> {
        const BY_SIZE: [NumberKind; 12] = [
            NumberKind::I8,
            NumberKind::U8,
            NumberKind::I16,
            NumberKind::U16,
            NumberKind::I32,
            NumberKind::U32,
            NumberKind::I64,
            NumberKind::U64,
            NumberKind::F32,
            NumberKind::F64,
            NumberKind::C64,
            NumberKind::C128,
        ];
        let reaches = |from: NumberKind, to: NumberKind| from == to || from.widens_to(to);
        BY_SIZE.into_iter().find(|kind| reaches(self, *kind) && reaches(other, *kind))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Variant {
    pub name: Name,
    pub values: Vec<TyId>,
}

/// An argument of a function type: `fn(f64[V], err: error)`. Its name says how `call` can wire it,
/// and is not part of the type (spec 4): see [`TypeTable::matches`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FnParam {
    pub name: Option<Name>,
    pub ty: TyId,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    /// Unknown because of an error upstream. It matches everything, so that one error gives one
    /// diagnostic (`docs/cli.md`, section 3).
    Error,
    Bool,
    Number {
        kind: NumberKind,
        unit: Unit,
    },
    String,
    Bytes,
    Timestamp,
    Duration,
    /// The language's `error` type.
    ErrorValue,
    Array {
        element: TyId,
        dimensions: u8,
    },
    Struct(Vec<(Name, TyId)>),
    Enum(Vec<Variant>),
    Waveform(TyId),
    Queue(TyId),
    Ref(TyId),
    Function {
        params: Vec<FnParam>,
        outputs: Vec<TyId>,
    },
    /// A type declared in a `.lct` file: its library and its name are its identity.
    Named {
        library: LibraryId,
        name: Name,
    },
    /// The variable of a generic function: `T`.
    Variable(Name),
}

impl Ty {
    fn children(&self) -> Vec<TyId> {
        match self {
            Ty::Error | Ty::Bool | Ty::Number { .. } | Ty::String | Ty::Bytes | Ty::Timestamp | Ty::Duration | Ty::ErrorValue => Vec::new(),
            Ty::Named { .. } | Ty::Variable(_) => Vec::new(),
            Ty::Array { element: child, .. } | Ty::Waveform(child) | Ty::Queue(child) | Ty::Ref(child) => vec![*child],
            Ty::Struct(fields) => fields.iter().map(|(_, ty)| *ty).collect(),
            Ty::Enum(variants) => variants.iter().flat_map(|variant| variant.values.iter().copied()).collect(),
            Ty::Function { params, outputs } => params.iter().map(|param| param.ty).chain(outputs.iter().copied()).collect(),
        }
    }

    /// The same type with the names of function arguments removed, at any depth.
    fn without_argument_names(&self, shape: impl Fn(TyId) -> TyId) -> Ty {
        match self {
            Ty::Array { element, dimensions } => Ty::Array { element: shape(*element), dimensions: *dimensions },
            Ty::Struct(fields) => Ty::Struct(fields.iter().map(|(name, ty)| (*name, shape(*ty))).collect()),
            Ty::Enum(variants) => Ty::Enum(
                variants.iter().map(|variant| Variant { name: variant.name, values: variant.values.iter().map(|ty| shape(*ty)).collect() }).collect(),
            ),
            Ty::Waveform(element) => Ty::Waveform(shape(*element)),
            Ty::Queue(element) => Ty::Queue(shape(*element)),
            Ty::Ref(element) => Ty::Ref(shape(*element)),
            Ty::Function { params, outputs } => Ty::Function {
                params: params.iter().map(|param| FnParam { name: None, ty: shape(param.ty) }).collect(),
                outputs: outputs.iter().map(|ty| shape(*ty)).collect(),
            },
            other => other.clone(),
        }
    }
}

/// `LC1006`: a type nested too deep, or an array with too many dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeOutOfBounds;

#[derive(Clone, Debug)]
struct Entry {
    ty: Arc<Ty>,
    /// Levels of nesting: `f64` is 0, `[f64]` is 1.
    depth: u32,
    /// [`Ty::Error`] is somewhere inside.
    has_error: bool,
    /// This type with the names of function arguments removed.
    shape: TyId,
}

#[derive(Clone, Debug)]
pub struct TypeTable {
    max_depth: u32,
    indices: HashMap<Arc<Ty>, TyId>,
    types: Vec<Entry>,
    libraries: Vec<Name>,
}

impl TypeTable {
    /// `max_depth` comes from `Limits::max_nesting`, under the hard ceiling.
    pub fn new(max_depth: u32) -> Self {
        let error = Arc::new(Ty::Error);
        TypeTable {
            max_depth: max_depth.min(MAX_NESTING),
            indices: HashMap::from([(Arc::clone(&error), TyId::ERROR)]),
            types: vec![Entry { ty: error, depth: 0, has_error: true, shape: TyId::ERROR }],
            libraries: Vec::new(),
        }
    }

    /// Each call makes a different library, even with the same name. `name` is what a program writes
    /// before the name of a type from elsewhere: `visa`, `a.b`.
    pub fn add_library(&mut self, name: Name) -> LibraryId {
        let id = LibraryId(u32::try_from(self.libraries.len()).unwrap_or(u32::MAX));
        self.libraries.push(name);
        id
    }

    pub fn intern(&mut self, ty: Ty) -> Result<TyId, TypeOutOfBounds> {
        if let Some(id) = self.indices.get(&ty) {
            return Ok(*id);
        }
        if let Ty::Array { dimensions, .. } = ty
            && (dimensions == 0 || u32::from(dimensions) > MAX_ARRAY_DIMENSIONS)
        {
            return Err(TypeOutOfBounds);
        }
        let children = ty.children();
        let depth = if children.is_empty() { 0 } else { 1 + children.iter().map(|child| self.depth(*child)).max().unwrap_or(0) };
        if depth > self.max_depth {
            return Err(TypeOutOfBounds);
        }
        let has_error = children.iter().any(|child| self.has_error(*child));
        let shape_ty = ty.without_argument_names(|child| self.shape(child));
        // Interning the shape cannot recurse further: all its children are shapes already. It comes
        // first, since it takes an id.
        let shape = if shape_ty == ty { None } else { Some(self.intern(shape_ty)?) };
        let id = TyId(u32::try_from(self.types.len()).map_err(|_| TypeOutOfBounds)?);
        let ty = Arc::new(ty);
        self.types.push(Entry { ty: Arc::clone(&ty), depth, has_error, shape: shape.unwrap_or(id) });
        self.indices.insert(ty, id);
        Ok(id)
    }

    /// [`Ty::Error`] for an id of another table.
    pub fn get(&self, id: TyId) -> &Ty {
        self.types.get(id.0 as usize).map_or(&Ty::Error, |entry| &entry.ty)
    }

    fn depth(&self, id: TyId) -> u32 {
        self.types.get(id.0 as usize).map_or(0, |entry| entry.depth)
    }

    /// [`Ty::Error`] is somewhere inside: an error has been reported, and what this type would have been stays unknown.
    pub fn has_error(&self, id: TyId) -> bool {
        self.types.get(id.0 as usize).is_none_or(|entry| entry.has_error)
    }

    fn shape(&self, id: TyId) -> TyId {
        self.types.get(id.0 as usize).map_or(TyId::ERROR, |entry| entry.shape)
    }

    /// Whether two types are the same, in constant time, whatever their size. The names of function
    /// arguments do not count: `fn(x: f64[V])` is `fn(f64[V])` (spec 4). A type with an error
    /// anywhere inside matches everything: that error has been reported, and a second diagnostic
    /// about the same mistake helps nobody.
    pub fn matches(&self, a: TyId, b: TyId) -> bool {
        self.shape(a) == self.shape(b) || self.has_error(a) || self.has_error(b)
    }

    /// A type as written in the language, units included, cut at [`MAX_TEXT`] code points, as seen
    /// from `viewer`: its own types have no prefix (`Verdict`), and those of other libraries do
    /// (`visa.session`). The cut is what bounds the work: a type shared many times is small here, and
    /// huge once written. A type unknown because of an error upstream is written `unknown`.
    pub fn display(&self, id: TyId, viewer: LibraryId, names: &Interner) -> String {
        let mut writer = TypeWriter { types: self, viewer, names, out: String::new() };
        writer.ty(id);
        cut_text(&writer.out).into_owned()
    }
}

struct TypeWriter<'a> {
    types: &'a TypeTable,
    viewer: LibraryId,
    names: &'a Interner,
    out: String,
}

impl TypeWriter<'_> {
    /// Enough is written: past this many bytes, the text is over [`MAX_TEXT`] code points.
    fn is_full(&self) -> bool {
        self.out.len() > MAX_TEXT * 4
    }

    fn text(&mut self, text: &str) {
        self.out.push_str(text);
    }

    /// Only what can be shown: a name of 8 MiB can be written in a million diagnostics.
    fn name(&mut self, name: Name) {
        let text = self.names.resolve(name);
        let shown = text.char_indices().nth(MAX_TEXT + 1).map_or(text, |(end, _)| text.get(..end).unwrap_or(text));
        self.out.push_str(shown);
    }

    fn list<T>(&mut self, items: &[T], mut write: impl FnMut(&mut Self, &T)) {
        for (index, item) in items.iter().enumerate() {
            if self.is_full() {
                return;
            }
            if index > 0 {
                self.text(", ");
            }
            write(self, item);
        }
    }

    fn ty(&mut self, id: TyId) {
        if self.is_full() {
            return;
        }
        let types = self.types;
        match types.get(id) {
            Ty::Error => self.text("unknown"),
            Ty::Bool => self.text("bool"),
            Ty::String => self.text("string"),
            Ty::Bytes => self.text("bytes"),
            Ty::Timestamp => self.text("timestamp"),
            Ty::Duration => self.text("duration"),
            Ty::ErrorValue => self.text("error"),
            Ty::Number { kind, unit } => {
                self.text(kind.as_str());
                if *unit != Unit::ONE {
                    let _ = write!(self.out, "[{unit}]");
                }
            }
            Ty::Array { element, dimensions } => {
                self.text("[");
                self.ty(*element);
                if *dimensions > 1 {
                    let _ = write!(self.out, "; {dimensions}");
                }
                self.text("]");
            }
            Ty::Struct(fields) => {
                self.text("struct { ");
                self.list(fields, |writer, (name, ty)| {
                    writer.name(*name);
                    writer.text(": ");
                    writer.ty(*ty);
                });
                self.text(" }");
            }
            Ty::Enum(variants) => {
                self.text("enum { ");
                self.list(variants, |writer, variant| {
                    writer.name(variant.name);
                    if !variant.values.is_empty() {
                        writer.text("(");
                        writer.list(&variant.values, |writer, ty| writer.ty(*ty));
                        writer.text(")");
                    }
                });
                self.text(" }");
            }
            Ty::Waveform(element) => self.generic("waveform", *element),
            Ty::Queue(element) => self.generic("queue", *element),
            Ty::Ref(element) => self.generic("ref", *element),
            Ty::Function { params, outputs } => {
                self.text("fn(");
                self.list(params, |writer, param| {
                    if let Some(name) = param.name {
                        writer.name(name);
                        writer.text(": ");
                    }
                    writer.ty(param.ty);
                });
                self.text(") -> (");
                self.list(outputs, |writer, ty| writer.ty(*ty));
                self.text(")");
            }
            Ty::Named { library, name } => {
                if *library != self.viewer
                    && let Some(library) = types.libraries.get(library.0 as usize)
                {
                    self.name(*library);
                    self.text(".");
                }
                self.name(*name);
            }
            Ty::Variable(name) => self.name(*name),
        }
    }

    fn generic(&mut self, name: &str, element: TyId) {
        self.text(name);
        self.text("<");
        self.ty(element);
        self.text(">");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        table: TypeTable,
        names: Interner,
        root: LibraryId,
    }

    fn fixture() -> Fixture {
        let mut names = Interner::new();
        let mut table = TypeTable::new(256);
        let root = table.add_library(names.intern("root"));
        Fixture { table, names, root }
    }

    impl Fixture {
        fn number(&mut self, kind: NumberKind, unit: &str) -> TyId {
            let unit = if unit.is_empty() { Unit::ONE } else { crate::unit::parse_unit(unit, 256).unwrap() };
            self.table.intern(Ty::Number { kind, unit }).unwrap()
        }

        fn volts(&mut self) -> TyId {
            self.number(NumberKind::F64, "V")
        }

        fn show(&mut self, ty: Ty, expected: &str) {
            let id = self.table.intern(ty).unwrap();
            assert_eq!(self.table.display(id, self.root, &self.names), expected);
        }

        /// `a1 = struct { x: a0, y: a0 }`, 200 times: small in the table, 2^200 once written.
        fn shared(&mut self, leaf: TyId) -> TyId {
            let (x, y) = (self.names.intern("x"), self.names.intern("y"));
            let mut ty = leaf;
            for _ in 0..200 {
                ty = self.table.intern(Ty::Struct(vec![(x, ty), (y, ty)])).unwrap();
            }
            ty
        }
    }

    #[test]
    fn widening_is_the_closed_list_of_the_spec() {
        use NumberKind::{C64, C128, F32, F64, I8, I16, I32, I64, U8, U16, U32, U64};
        let all = [I8, I16, I32, I64, U8, U16, U32, U64, F32, F64, C64, C128];
        let list = [
            (I8, I16),
            (I8, I32),
            (I8, I64),
            (I16, I32),
            (I16, I64),
            (I32, I64),
            (U8, U16),
            (U8, U32),
            (U8, U64),
            (U16, U32),
            (U16, U64),
            (U32, U64),
            (U8, I16),
            (U8, I32),
            (U8, I64),
            (U16, I32),
            (U16, I64),
            (U32, I64),
            (I8, F64),
            (I16, F64),
            (I32, F64),
            (U8, F64),
            (U16, F64),
            (U32, F64),
            (F32, F64),
            (C64, C128),
        ];
        for from in all {
            for to in all {
                assert_eq!(from.widens_to(to), list.contains(&(from, to)), "{from:?} to {to:?}");
            }
        }
    }

    #[test]
    fn two_kinds_meet_at_the_smallest_that_both_widen_to() {
        use NumberKind::{C64, C128, F32, F64, I8, I16, I32, I64, U8, U32, U64};
        let meetings = [
            (I32, I32, Some(I32)),
            (I32, F64, Some(F64)),
            (F64, I32, Some(F64)),
            (I32, F32, Some(F64)),
            (I16, I32, Some(I32)),
            (I8, U8, Some(I16)),
            (I32, U32, Some(I64)),
            (C64, C128, Some(C128)),
            (I64, F64, None),
            (U64, I8, None),
            (F64, C64, None),
        ];
        for (a, b, expected) in meetings {
            assert_eq!(a.meeting(b), expected, "{a:?} and {b:?}");
            assert_eq!(b.meeting(a), expected, "{b:?} and {a:?}");
        }
    }

    #[test]
    fn equal_types_are_one_id() {
        let mut f = fixture();
        let a = f.number(NumberKind::F64, "V/A");
        assert_eq!(a, f.number(NumberKind::F64, "\u{03A9}"));
        assert_ne!(a, f.number(NumberKind::F64, "m\u{03A9}"));
        assert_ne!(a, f.number(NumberKind::F32, "\u{03A9}"));
        assert_eq!(f.table.intern(Ty::Error), Ok(TyId::ERROR));
    }

    #[test]
    fn ids_stay_valid_when_a_shape_is_interned_on_the_way() {
        let mut f = fixture();
        let volts = f.volts();
        let x = f.names.intern("x");
        let named = f.table.intern(Ty::Function { params: vec![FnParam { name: Some(x), ty: volts }], outputs: vec![] }).unwrap();
        assert!(matches!(f.table.get(named), Ty::Function { params, .. } if params[0].name == Some(x)));
        assert_eq!(f.table.intern(Ty::Function { params: vec![FnParam { name: Some(x), ty: volts }], outputs: vec![] }), Ok(named));
        let bare = f.table.intern(Ty::Function { params: vec![FnParam { name: None, ty: volts }], outputs: vec![] }).unwrap();
        assert_ne!(bare, named);
        assert_eq!(f.table.shape(named), bare);
        assert_eq!(f.table.shape(bare), bare);
    }

    #[test]
    fn types_are_written_as_in_the_language() {
        let mut f = fixture();
        let (volts, plain) = (f.volts(), f.number(NumberKind::F64, ""));
        let boolean = f.table.intern(Ty::Bool).unwrap();
        let error = f.table.intern(Ty::ErrorValue).unwrap();
        let (voltage, ok, pass, value, err) =
            (f.names.intern("voltage"), f.names.intern("ok"), f.names.intern("Pass"), f.names.intern("Value"), f.names.intern("err"));
        f.show(Ty::Array { element: plain, dimensions: 1 }, "[f64]");
        f.show(Ty::Array { element: plain, dimensions: 2 }, "[f64; 2]");
        f.show(Ty::Struct(vec![(voltage, volts), (ok, boolean)]), "struct { voltage: f64[V], ok: bool }");
        f.show(Ty::Enum(vec![Variant { name: pass, values: vec![] }, Variant { name: value, values: vec![volts] }]), "enum { Pass, Value(f64[V]) }");
        f.show(Ty::Waveform(volts), "waveform<f64[V]>");
        f.show(Ty::Queue(plain), "queue<f64>");
        f.show(Ty::Ref(plain), "ref<f64>");
        f.show(
            Ty::Function { params: vec![FnParam { name: None, ty: volts }, FnParam { name: Some(err), ty: error }], outputs: vec![volts, error] },
            "fn(f64[V], err: error) -> (f64[V], error)",
        );
        f.show(Ty::Function { params: vec![FnParam { name: None, ty: volts }], outputs: vec![volts] }, "fn(f64[V]) -> (f64[V])");
        f.show(Ty::Error, "unknown");
    }

    #[test]
    fn a_named_type_is_written_from_where_it_is_seen() {
        let mut f = fixture();
        let (visa, session, verdict) = (f.names.intern("visa"), f.names.intern("session"), f.names.intern("Verdict"));
        let library = f.table.add_library(visa);
        let root = f.root;
        f.show(Ty::Named { library, name: session }, "visa.session");
        f.show(Ty::Named { library: root, name: verdict }, "Verdict");
        let id = f.table.intern(Ty::Named { library, name: session }).unwrap();
        assert_eq!(f.table.display(id, library, &f.names), "session");
    }

    #[test]
    fn two_libraries_with_one_name_have_different_types() {
        let mut f = fixture();
        let (visa, session) = (f.names.intern("visa"), f.names.intern("session"));
        let (shipped, project) = (f.table.add_library(visa), f.table.add_library(visa));
        let a = f.table.intern(Ty::Named { library: shipped, name: session }).unwrap();
        let b = f.table.intern(Ty::Named { library: project, name: session }).unwrap();
        assert_ne!(a, b);
        assert!(!f.table.matches(a, b));
    }

    #[test]
    fn a_type_has_one_identity_wherever_it_is_named_from() {
        let mut f = fixture();
        let (a, reading) = (f.names.intern("a"), f.names.intern("Reading"));
        let library = f.table.add_library(a);
        let inside = f.table.intern(Ty::Named { library, name: reading }).unwrap();
        let from_the_root = f.table.intern(Ty::Named { library, name: reading }).unwrap();
        assert_eq!(inside, from_the_root);
    }

    #[test]
    fn depth_has_a_limit_of_that_many_levels() {
        let mut table = TypeTable::new(4);
        let mut ty = table.intern(Ty::Bool).unwrap();
        for _ in 0..4 {
            ty = table.intern(Ty::Array { element: ty, dimensions: 1 }).unwrap();
        }
        assert_eq!(table.intern(Ty::Array { element: ty, dimensions: 1 }), Err(TypeOutOfBounds));
        assert_eq!(table.intern(Ty::Struct(vec![(Interner::new().intern("x"), ty)])), Err(TypeOutOfBounds));
    }

    #[test]
    fn the_depth_limit_has_a_hard_ceiling() {
        let mut f = fixture();
        f.table = TypeTable::new(u32::MAX);
        let mut ty = TyId::ERROR;
        let mut levels = 0;
        while let Ok(next) = f.table.intern(Ty::Ref(ty)) {
            ty = next;
            levels += 1;
        }
        assert_eq!(levels, MAX_NESTING);
        assert_eq!(f.table.display(ty, f.root, &f.names).chars().count(), MAX_TEXT);
    }

    #[test]
    fn arrays_have_one_to_32_dimensions() {
        let mut f = fixture();
        let plain = f.number(NumberKind::F64, "");
        assert!(f.table.intern(Ty::Array { element: plain, dimensions: 32 }).is_ok());
        assert_eq!(f.table.intern(Ty::Array { element: plain, dimensions: 33 }), Err(TypeOutOfBounds));
        assert_eq!(f.table.intern(Ty::Array { element: plain, dimensions: 0 }), Err(TypeOutOfBounds));
    }

    #[test]
    fn a_long_name_is_written_cut() {
        let mut f = fixture();
        let long = "x".repeat(MAX_TEXT * 2);
        let name = f.names.intern(&long);
        let root = f.root;
        let ty = f.table.intern(Ty::Named { library: root, name }).unwrap();
        let shown = f.table.display(ty, root, &f.names);
        assert_eq!(shown.chars().count(), MAX_TEXT);
        assert!(shown.ends_with('\u{2026}'));
    }

    #[test]
    fn a_type_shared_many_times_writes_quickly() {
        let mut f = fixture();
        let leaf = f.number(NumberKind::F64, "");
        let ty = f.shared(leaf);
        let shown = f.table.display(ty, f.root, &f.names);
        assert_eq!(shown.chars().count(), MAX_TEXT);
        assert!(shown.ends_with('\u{2026}'));
    }

    #[test]
    fn a_type_shared_many_times_compares_quickly() {
        let mut f = fixture();
        let (one, other) = (f.number(NumberKind::F64, ""), f.number(NumberKind::I32, ""));
        let (a, b, c) = (f.shared(one), f.shared(TyId::ERROR), f.shared(other));
        assert!(f.table.matches(a, a));
        assert!(f.table.matches(a, b));
        assert!(!f.table.matches(a, c));
    }

    #[test]
    fn an_error_anywhere_inside_matches_everything() {
        let mut f = fixture();
        let (volts, amps) = (f.volts(), f.number(NumberKind::F64, "A"));
        assert!(f.table.matches(volts, volts));
        assert!(f.table.matches(volts, TyId::ERROR));
        assert!(f.table.matches(TyId::ERROR, volts));
        assert!(!f.table.matches(volts, amps));
        let of_volts = f.table.intern(Ty::Array { element: volts, dimensions: 1 }).unwrap();
        let of_amps = f.table.intern(Ty::Array { element: amps, dimensions: 1 }).unwrap();
        let of_error = f.table.intern(Ty::Array { element: TyId::ERROR, dimensions: 1 }).unwrap();
        assert!(f.table.matches(of_volts, of_error));
        assert!(f.table.matches(of_error, volts), "the error is reported already");
        assert!(!f.table.matches(of_volts, of_amps));
        assert!(!f.table.matches(of_volts, volts));
    }

    #[test]
    fn names_of_function_arguments_do_not_count() {
        let mut f = fixture();
        let volts = f.volts();
        let (x, v) = (f.names.intern("x"), f.names.intern("v"));
        let mut function = |name, outputs: Vec<TyId>| {
            let ty = Ty::Function { params: vec![FnParam { name, ty: volts }], outputs };
            f.table.intern(ty).unwrap()
        };
        let (bare, named, renamed, longer) =
            (function(None, vec![volts]), function(Some(x), vec![volts]), function(Some(v), vec![volts]), function(None, vec![volts, volts]));
        assert!(f.table.matches(bare, named));
        assert!(f.table.matches(named, renamed));
        assert!(!f.table.matches(bare, longer));
        assert_eq!(f.table.display(named, f.root, &f.names), "fn(x: f64[V]) -> (f64[V])");
    }

    #[test]
    fn names_of_function_arguments_do_not_count_inside_other_types() {
        let mut f = fixture();
        let volts = f.volts();
        let x = f.names.intern("x");
        let named = f.table.intern(Ty::Function { params: vec![FnParam { name: Some(x), ty: volts }], outputs: vec![] }).unwrap();
        let bare = f.table.intern(Ty::Function { params: vec![FnParam { name: None, ty: volts }], outputs: vec![] }).unwrap();
        let wrap = |table: &mut TypeTable, element| table.intern(Ty::Array { element, dimensions: 1 }).unwrap();
        let (a, b) = (wrap(&mut f.table, named), wrap(&mut f.table, bare));
        assert_ne!(a, b);
        assert!(f.table.matches(a, b));
        let (c, d) = (wrap(&mut f.table, a), wrap(&mut f.table, b));
        assert!(f.table.matches(c, d));
    }

    #[test]
    fn depth_is_not_changed_by_a_shape() {
        let mut table = TypeTable::new(2);
        let x = Interner::new().intern("x");
        let leaf = table.intern(Ty::Bool).unwrap();
        let function = table.intern(Ty::Function { params: vec![FnParam { name: Some(x), ty: leaf }], outputs: vec![] }).unwrap();
        assert!(table.intern(Ty::Ref(function)).is_ok());
    }

    #[test]
    fn an_id_of_another_table_does_not_fail() {
        let mut other = fixture();
        other.number(NumberKind::F64, "");
        let foreign = other.number(NumberKind::F32, "");
        let f = fixture();
        assert_eq!(f.table.get(foreign), &Ty::Error);
        assert_eq!(f.table.display(foreign, f.root, &f.names), "unknown");
        assert!(f.table.matches(foreign, TyId::ERROR));
    }
}
