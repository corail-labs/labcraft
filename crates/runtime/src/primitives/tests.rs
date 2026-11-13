//! The bodies against what appendix A and the spec say. Every expected value is worked out by hand, or with Python's IEEE 754
//! doubles and `struct` for `f32` where the digits are long (each such case says so), never read from this code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::HashMap;
use std::sync::Arc;

use labcraft_check::PrimitiveLibrary;
use labcraft_check::primitives::PRIMITIVES;
use labcraft_ir::{ConstId, ConstValue, Constant, ElementId, Program, Skip, Static, VERSION, Variadic};
use labcraft_types::{Interner, LibraryId, NumberKind, Ty, TyId, TypeTable, Unit, Variant, parse_unit};

use super::{Call, Host, Outcome, bodies, outputs_on_error, run};
use crate::convert::{Env, constant, convert, read};
use crate::fault::{Fault, FaultCode};
use crate::value::{ErrorData, Handle, Value, logical_size};

/// A program with no function: its types and constants are what a call reads.
struct World {
    program: Program,
    names: Interner,
    library: LibraryId,
}

impl World {
    fn new() -> Self {
        let mut names = Interner::new();
        let mut types = TypeTable::new(64);
        let library = types.add_library(names.intern("main"));
        World {
            program: Program { version: VERSION, functions: Vec::new(), constants: Vec::new(), types, definitions: HashMap::new() },
            names,
            library,
        }
    }

    fn ty(&mut self, ty: Ty) -> TyId {
        self.program.types.intern(ty).unwrap()
    }

    fn number(&mut self, kind: NumberKind, unit: &str) -> TyId {
        let unit = if unit.is_empty() { Unit::ONE } else { parse_unit(unit, 8).unwrap() };
        self.ty(Ty::Number { kind, unit })
    }

    fn named(&mut self, name: &str, definition: Ty) -> TyId {
        let name = self.names.intern(name);
        let named = self.ty(Ty::Named { library: self.library, name });
        let defined = self.ty(definition);
        self.program.definitions.insert(named, Some(defined));
        named
    }

    fn constant(&mut self, ty: TyId, value: ConstValue) -> ConstId {
        self.program.constants.push(Constant { ty, value });
        ConstId(u32::try_from(self.program.constants.len() - 1).unwrap())
    }

    fn env(&self) -> Env<'_> {
        Env { program: &self.program, names: &self.names }
    }
}

/// A run seen from one node: its clock, its panel, its memory.
pub(super) struct FakeHost {
    clock: i64,
    panel: HashMap<u32, Value>,
    room: u64,
}

impl FakeHost {
    pub(super) fn new() -> Self {
        FakeHost { clock: 0, panel: HashMap::new(), room: 1 << 30 }
    }
}

impl Host for FakeHost {
    fn now(&mut self) -> i64 {
        self.clock
    }
    fn element(&mut self, element: ElementId) -> Result<Value, Fault> {
        Ok(self.panel.get(&element.0).cloned().unwrap_or(Value::Bool(false)))
    }
    fn set_element(&mut self, element: ElementId, value: Value) -> Result<(), Fault> {
        self.panel.insert(element.0, value);
        Ok(())
    }
    fn room(&self) -> u64 {
        self.room
    }
    fn error(&self, category: &str, code: i64, message: String) -> Value {
        Value::error(ErrorData { category: category.into(), code, message: message.into(), origin: None, others: Arc::default() })
    }
}

/// One call of `library.name`.
struct Case<'w> {
    world: &'w World,
    inputs: Vec<Value>,
    input_types: Vec<TyId>,
    output_types: Vec<TyId>,
    statics: Vec<Static>,
    variadic: Option<Variadic>,
}

impl<'w> Case<'w> {
    fn new(world: &'w World, inputs: Vec<(Value, TyId)>, outputs: Vec<TyId>) -> Self {
        let (inputs, input_types) = inputs.into_iter().unzip();
        Case { world, inputs, input_types, output_types: outputs, statics: Vec::new(), variadic: None }
    }

    fn statics(mut self, statics: Vec<Static>) -> Self {
        self.statics = statics;
        self
    }

    fn variadic(mut self, start: u32, count: u32) -> Self {
        self.variadic = Some(Variadic { start, count });
        self
    }

    fn run_with(&self, library: PrimitiveLibrary, name: &str, host: &mut FakeHost) -> (Result<Outcome, Fault>, Vec<Value>) {
        let mut call = Call {
            env: self.world.env(),
            viewer: self.world.library,
            inputs: self.inputs.clone(),
            input_types: self.input_types.clone(),
            output_types: self.output_types.clone(),
            statics: &self.statics,
            variadic: self.variadic,
        };
        let outcome = run(library, name, &mut call, host);
        (outcome, call.inputs)
    }

    fn outcome(&self, library: PrimitiveLibrary, name: &str) -> Result<Outcome, Fault> {
        self.run_with(library, name, &mut FakeHost::new()).0
    }

    /// Its outputs, when it does not fail.
    fn done(&self, library: PrimitiveLibrary, name: &str) -> Vec<Value> {
        match self.outcome(library, name) {
            Ok(Outcome::Done(outputs)) => outputs,
            other => panic!("{}.{name}: {other:?}", library.as_str()),
        }
    }

    fn one(&self, library: PrimitiveLibrary, name: &str) -> Value {
        self.done(library, name).into_iter().next().unwrap()
    }

    fn fault(&self, library: PrimitiveLibrary, name: &str) -> FaultCode {
        match self.outcome(library, name) {
            Err(fault) => fault.code,
            other => panic!("{}.{name}: no fault but {other:?}", library.as_str()),
        }
    }

    /// The error it fails with, and what is left of its inputs.
    fn fails(&self, library: PrimitiveLibrary, name: &str) -> (ErrorData, Vec<Value>) {
        match self.run_with(library, name, &mut FakeHost::new()) {
            (Ok(Outcome::Fail(error)), inputs) => (ErrorData::clone(error.as_error().unwrap()), inputs),
            (other, _) => panic!("{}.{name}: no failure but {other:?}", library.as_str()),
        }
    }
}

use PrimitiveLibrary::{Array, Core, Errors, String as Strings, Time, Ui};

fn f64_of(value: &Value) -> f64 {
    match value {
        Value::F64(x) => *x,
        other => panic!("not an f64: {other:?}"),
    }
}

fn i32s(values: &[i32]) -> Value {
    Value::array(values.iter().map(|x| Value::I32(*x)).collect())
}

fn i32s_of(value: &Value) -> Vec<i32> {
    match value {
        Value::Array(items) => items.items().iter().map(|item| if let Value::I32(x) = item { *x } else { panic!("{item:?}") }).collect(),
        other => panic!("not an array: {other:?}"),
    }
}

fn texts(values: &[&str]) -> Value {
    Value::array(values.iter().map(|text| Value::string(*text)).collect())
}

fn texts_of(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => {
            items.items().iter().map(|item| if let Value::String(x) = item { x.to_string() } else { panic!("{item:?}") }).collect()
        }
        other => panic!("not an array: {other:?}"),
    }
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.to_string(),
        other => panic!("not a string: {other:?}"),
    }
}

#[test]
fn every_signature_has_a_body_and_no_body_lacks_one() {
    let signatures: Vec<(PrimitiveLibrary, &str)> = PRIMITIVES.iter().map(|primitive| (primitive.library, primitive.name)).collect();
    let mut with_body: Vec<(PrimitiveLibrary, &str)> = bodies().collect();
    for signature in &signatures {
        assert!(with_body.contains(signature), "no body for {}.{}", signature.0.as_str(), signature.1);
    }
    for body in &with_body {
        assert!(signatures.contains(body), "a body with no signature: {}.{}", body.0.as_str(), body.1);
    }
    let count = with_body.len();
    with_body.dedup();
    assert_eq!(with_body.len(), count, "each body once");
}

#[test]
fn integers_add_in_their_kind_and_overflow_is_a_fault() {
    let mut world = World::new();
    let i32_ty = world.number(NumberKind::I32, "");
    let u8_ty = world.number(NumberKind::U8, "");
    let case = |a: Value, b: Value, ty: TyId| Case::new(&world, vec![(a, ty), (b, ty)], vec![ty]);
    assert!(matches!(case(Value::I32(2), Value::I32(3), i32_ty).one(Core, "add"), Value::I32(5)));
    assert!(matches!(case(Value::I32(2), Value::I32(3), i32_ty).one(Core, "sub"), Value::I32(-1)));
    assert!(matches!(case(Value::I32(-4), Value::I32(3), i32_ty).one(Core, "mul"), Value::I32(-12)));
    // Q-25, decision of the owner: a fault, LC8008.
    assert_eq!(case(Value::I32(i32::MAX), Value::I32(1), i32_ty).fault(Core, "add"), FaultCode::OutOfRange);
    assert_eq!(case(Value::I32(i32::MIN), Value::I32(1), i32_ty).fault(Core, "sub"), FaultCode::OutOfRange);
    assert_eq!(case(Value::I32(65_536), Value::I32(32_768), i32_ty).fault(Core, "mul"), FaultCode::OutOfRange, "2^31");
    assert_eq!(case(Value::U8(200), Value::U8(100), u8_ty).fault(Core, "add"), FaultCode::OutOfRange);
    assert_eq!(case(Value::U8(3), Value::U8(5), u8_ty).fault(Core, "sub"), FaultCode::OutOfRange);
    assert!(matches!(case(Value::U8(200), Value::U8(55), u8_ty).one(Core, "add"), Value::U8(255)));
}

#[test]
fn reals_follow_ieee_754() {
    let mut world = World::new();
    let f64_ty = world.number(NumberKind::F64, "");
    let f32_ty = world.number(NumberKind::F32, "");
    let i32_ty = world.number(NumberKind::I32, "");
    // Python: 0.1 + 0.2 == 0.30000000000000004.
    let sum = Case::new(&world, vec![(Value::F64(0.1), f64_ty), (Value::F64(0.2), f64_ty)], vec![f64_ty]).one(Core, "add");
    assert_eq!(f64_of(&sum), 0.300_000_000_000_000_04);
    // Python with `struct`: the f32 sum of f32(0.1) and f32(0.2) has the bits 0x3e99999a, which is also f32(0.3).
    let sum = Case::new(&world, vec![(Value::F32(0.1), f32_ty), (Value::F32(0.2), f32_ty)], vec![f32_ty]).one(Core, "add");
    assert!(matches!(sum, Value::F32(x) if x.to_bits() == 0x3e99_999a));
    // Integers give an f64 (3.4): 3 / 2 is 1.5; x / 0 is inf or nan.
    let div = |a: i32, b: i32| f64_of(&Case::new(&world, vec![(Value::I32(a), i32_ty), (Value::I32(b), i32_ty)], vec![f64_ty]).one(Core, "div"));
    assert_eq!(div(3, 2), 1.5);
    assert_eq!(div(1, 0), f64::INFINITY);
    assert_eq!(div(-1, 0), f64::NEG_INFINITY);
    assert!(div(0, 0).is_nan());
}

#[test]
fn complex_numbers_have_fixed_steps() {
    let mut world = World::new();
    let c128 = world.number(NumberKind::C128, "");
    let pair = |a: (f64, f64), b: (f64, f64)| Case::new(&world, vec![(Value::C128(a.0, a.1), c128), (Value::C128(b.0, b.1), c128)], vec![c128]);
    // By hand: (1 + 2i)(3 + 4i) = 3 + 4i + 6i + 8i² = -5 + 10i.
    assert!(matches!(pair((1.0, 2.0), (3.0, 4.0)).one(Core, "mul"), Value::C128(re, im) if re == -5.0 && im == 10.0));
    // By hand, Smith: |3| < |4|, r = 0.75, den = 3·0.75 + 4 = 6.25; re = (0.75 + 2) / 6.25, im = (1.5 - 1) / 6.25, each the
    // division of two exact doubles, so the nearest double to 0.44 and to 0.08.
    assert!(matches!(pair((1.0, 2.0), (3.0, 4.0)).one(Core, "div"), Value::C128(re, im) if re == 0.44 && im == 0.08));
    let one = |z: (f64, f64)| Case::new(&world, vec![(Value::C128(z.0, z.1), c128)], vec![c128]);
    // |3 + 4i|: larger 4, ratio 0.75, 4·√1.5625 = 4·1.25 = 5, every step exact.
    assert!(matches!(one((3.0, 4.0)).one(Core, "abs"), Value::C128(re, im) if re == 5.0 && im == 0.0));
    // √(3 + 4i): t = √((5 + 3) / 2) = 2, then 4 / (2·2) = 1, so 2 + i. √(-4): t = √((4 + 4) / 2) = 2, so 0 + 2i.
    assert!(matches!(one((3.0, 4.0)).one(Core, "sqrt"), Value::C128(re, im) if re == 2.0 && im == 1.0));
    assert!(matches!(one((-4.0, 0.0)).one(Core, "sqrt"), Value::C128(re, im) if re == 0.0 && im == 2.0));
}

#[test]
fn abs_and_neg_keep_the_range_of_their_kind() {
    let mut world = World::new();
    let i32_ty = world.number(NumberKind::I32, "");
    let u8_ty = world.number(NumberKind::U8, "");
    let one = |x: Value, ty: TyId| Case::new(&world, vec![(x, ty)], vec![ty]);
    assert!(matches!(one(Value::I32(-5), i32_ty).one(Core, "abs"), Value::I32(5)));
    assert_eq!(one(Value::I32(i32::MIN), i32_ty).fault(Core, "abs"), FaultCode::OutOfRange);
    assert_eq!(one(Value::I32(i32::MIN), i32_ty).fault(Core, "neg"), FaultCode::OutOfRange);
    assert!(matches!(one(Value::U8(0), u8_ty).one(Core, "neg"), Value::U8(0)));
    assert_eq!(one(Value::U8(1), u8_ty).fault(Core, "neg"), FaultCode::OutOfRange);
}

#[test]
fn sqrt_and_pow_give_reals_and_pow_squares() {
    let mut world = World::new();
    let f64_ty = world.number(NumberKind::F64, "");
    let i32_ty = world.number(NumberKind::I32, "");
    let sqrt = |x: Value, ty: TyId| f64_of(&Case::new(&world, vec![(x, ty)], vec![f64_ty]).one(Core, "sqrt"));
    // Python: math.sqrt(2) == 1.4142135623730951.
    assert_eq!(sqrt(Value::F64(2.0), f64_ty), std::f64::consts::SQRT_2, "1.4142135623730951");
    assert_eq!(sqrt(Value::I32(9), i32_ty), 3.0);
    assert!(sqrt(Value::F64(-1.0), f64_ty).is_nan());
    let pow = |x: Value, ty: TyId, n: i32| f64_of(&Case::new(&world, vec![(x, ty), (Value::I32(n), i32_ty)], vec![f64_ty]).one(Core, "pow"));
    assert_eq!(pow(Value::I32(2), i32_ty, 10), 1024.0);
    assert_eq!(pow(Value::F64(2.0), f64_ty, -2), 0.25);
    assert_eq!(pow(Value::F64(5.0), f64_ty, 0), 1.0);
    // Python, the same squaring step by step: 1.1^10 is 2.593742460100002, where the platform's `pow` (CPython, libm) gives
    // 2.5937424601000023: the fixed sequence is what every platform gives.
    assert_eq!(pow(Value::F64(1.1), f64_ty, 10), 2.593_742_460_100_002);
    assert_eq!(pow(Value::F64(1.1), f64_ty, 3), 1.331_000_000_000_000_4, "Python: 1.1·(1.1·1.1)");
}

#[test]
fn min_and_max_keep_nan_and_the_sign_of_zero() {
    let mut world = World::new();
    let f64_ty = world.number(NumberKind::F64, "");
    let i32_ty = world.number(NumberKind::I32, "");
    let pair =
        |a: f64, b: f64, name: &str| f64_of(&Case::new(&world, vec![(Value::F64(a), f64_ty), (Value::F64(b), f64_ty)], vec![f64_ty]).one(Core, name));
    assert!(pair(f64::NAN, 1.0, "min").is_nan());
    assert!(pair(1.0, f64::NAN, "min").is_nan(), "whatever the order");
    assert!(pair(1.0, f64::NAN, "max").is_nan());
    assert!(pair(0.0, -0.0, "min").is_sign_negative());
    assert!(pair(-0.0, 0.0, "max").is_sign_positive());
    assert_eq!(pair(2.0, 3.0, "min"), 2.0);
    assert_eq!(pair(2.0, 3.0, "max"), 3.0);
    let ints = Case::new(&world, vec![(Value::I32(3), i32_ty), (Value::I32(2), i32_ty)], vec![i32_ty]);
    assert!(matches!(ints.one(Core, "min"), Value::I32(2)));
    assert!(matches!(ints.one(Core, "max"), Value::I32(3)));
}

#[test]
fn comparisons_follow_ieee_754() {
    let mut world = World::new();
    let f64_ty = world.number(NumberKind::F64, "");
    let i32_ty = world.number(NumberKind::I32, "");
    let bool_ty = world.ty(Ty::Bool);
    let compare = |a: Value, b: Value, name: &str| {
        matches!(Case::new(&world, vec![(a, f64_ty), (b, f64_ty)], vec![bool_ty]).one(Core, name), Value::Bool(true))
    };
    assert!(compare(Value::F64(1.0), Value::F64(2.0), "lt"));
    assert!(!compare(Value::F64(2.0), Value::F64(2.0), "lt"));
    assert!(compare(Value::F64(2.0), Value::F64(2.0), "le"));
    assert!(compare(Value::F64(3.0), Value::F64(2.0), "gt"));
    assert!(compare(Value::F64(2.0), Value::F64(2.0), "ge"));
    for name in ["lt", "le", "gt", "ge"] {
        assert!(!compare(Value::F64(f64::NAN), Value::F64(1.0), name), "{name}: nan compares false");
    }
    // Two kinds meet in the smallest that both widen to (3.4).
    let mixed = Case::new(&world, vec![(Value::I32(1), i32_ty), (Value::F64(1.5), f64_ty)], vec![bool_ty]);
    assert!(matches!(mixed.one(Core, "lt"), Value::Bool(true)));
}

#[test]
fn equality_compares_what_values_hold() {
    let mut world = World::new();
    let any = world.number(NumberKind::F64, "");
    let bool_ty = world.ty(Ty::Bool);
    let eq = |a: Value, b: Value| matches!(Case::new(&world, vec![(a, any), (b, any)], vec![bool_ty]).one(Core, "eq"), Value::Bool(true));
    assert!(!eq(Value::F64(f64::NAN), Value::F64(f64::NAN)), "nan equals nothing");
    assert!(eq(Value::F64(-0.0), Value::F64(0.0)));
    assert!(eq(i32s(&[1, 2]), i32s(&[1, 2])));
    assert!(!eq(i32s(&[1, 2]), i32s(&[1, 2, 3])));
    let nans = Value::array(vec![Value::F64(f64::NAN)]);
    assert!(!eq(nans.clone(), nans), "the same array holding nan is not equal to itself");
    assert!(eq(Value::string("a"), Value::string("a")));
    assert!(!eq(Value::string("a"), Value::string("b")));
    let point = |x: f64| Value::Struct(vec![Value::F64(x), Value::Bool(true)].into());
    assert!(eq(point(1.0), point(1.0)));
    assert!(!eq(point(1.0), point(2.0)));
    let error = |message: &str, line: u64| {
        let place = crate::value::Place { function: "f".into(), node: "n".into(), file: "f.lcf".into(), line };
        Value::error(ErrorData {
            category: "io.timeout".into(),
            code: 5,
            message: message.into(),
            origin: Some(Arc::new(crate::value::Origin { place, callers: Arc::from([]) })),
            others: Arc::default(),
        })
    };
    assert!(eq(error("late", 1), error("late", 2)), "not where they were made");
    assert!(!eq(error("late", 1), error("early", 1)));
    assert!(eq(Value::NONE, Value::NONE));
    assert!(!eq(Value::NONE, error("late", 1)));
    assert!(eq(Value::Handle(Handle { library: 0, number: 3 }), Value::Handle(Handle { library: 0, number: 3 })));
    let ne = Case::new(&world, vec![(Value::I32(1), any), (Value::I32(2), any)], vec![bool_ty]).one(Core, "ne");
    assert!(matches!(ne, Value::Bool(true)));
}

#[test]
fn booleans_and_select() {
    let mut world = World::new();
    let bool_ty = world.ty(Ty::Bool);
    let two = |a: bool, b: bool, name: &str| {
        matches!(Case::new(&world, vec![(Value::Bool(a), bool_ty), (Value::Bool(b), bool_ty)], vec![bool_ty]).one(Core, name), Value::Bool(true))
    };
    assert!(two(true, true, "and") && !two(true, false, "and"));
    assert!(two(false, true, "or") && !two(false, false, "or"));
    assert!(matches!(Case::new(&world, vec![(Value::Bool(true), bool_ty)], vec![bool_ty]).one(Core, "not"), Value::Bool(false)));
    let select = |condition: bool| {
        Case::new(&world, vec![(Value::Bool(condition), bool_ty), (Value::string("yes"), bool_ty), (Value::string("no"), bool_ty)], vec![bool_ty])
            .one(Core, "select")
    };
    assert_eq!(text_of(&select(true)), "yes");
    assert_eq!(text_of(&select(false)), "no");
}

#[test]
fn units_come_and_go() {
    let mut world = World::new();
    let f64_ty = world.number(NumberKind::F64, "");
    let millivolts = world.number(NumberKind::F64, "mV");
    let celsius = world.number(NumberKind::F64, "\u{00B0}C");
    let volts = parse_unit("V", 8).unwrap();
    let kelvins = parse_unit("K", 8).unwrap();
    let with = Case::new(&world, vec![(Value::F64(230.0), f64_ty)], vec![millivolts]).statics(vec![Static::Unit(volts)]);
    assert_eq!(f64_of(&with.one(Core, "with_unit")), 230.0, "the same number, in the unit its type now says");
    // Python: 230 / 1000 == 0.23, and 25 + 273.15 == 298.15.
    let unitless = Case::new(&world, vec![(Value::F64(230.0), millivolts)], vec![f64_ty]).statics(vec![Static::Unit(volts)]);
    assert_eq!(f64_of(&unitless.one(Core, "unitless")), 0.23);
    let unitless = Case::new(&world, vec![(Value::F64(25.0), celsius)], vec![f64_ty]).statics(vec![Static::Unit(kelvins)]);
    assert_eq!(f64_of(&unitless.one(Core, "unitless")), 298.15);
}

#[test]
fn narrowing_to_i32_and_its_faults() {
    let mut world = World::new();
    let f64_ty = world.number(NumberKind::F64, "");
    let i32_ty = world.number(NumberKind::I32, "");
    let f32_ty = world.number(NumberKind::F32, "");
    let narrow = |x: f64, name: &str| Case::new(&world, vec![(Value::F64(x), f64_ty)], vec![i32_ty]).outcome(Core, name);
    let int = |x: f64, name: &str| match narrow(x, name) {
        Ok(Outcome::Done(outputs)) => match outputs.as_slice() {
            [Value::I32(x)] => *x,
            other => panic!("{other:?}"),
        },
        other => panic!("{name}({x}): {other:?}"),
    };
    let faults = |x: f64, name: &str| matches!(narrow(x, name), Err(Fault { code: FaultCode::OutOfRange, .. }));
    assert_eq!(int(2.7, "to_i32"), 2);
    assert_eq!(int(-2.7, "to_i32"), -2, "toward zero");
    assert_eq!(int(2_147_483_647.9, "to_i32"), i32::MAX);
    assert_eq!(int(-2_147_483_648.5, "to_i32"), i32::MIN);
    assert!(faults(2_147_483_648.0, "to_i32"));
    assert!(faults(-2_147_483_649.0, "to_i32"));
    assert!(faults(f64::NAN, "to_i32"));
    assert!(faults(f64::INFINITY, "to_i32"));
    // A half to even (decision of the owner, 2026-11-09).
    assert_eq!(int(2.5, "round_to_i32"), 2);
    assert_eq!(int(3.5, "round_to_i32"), 4);
    assert_eq!(int(-2.5, "round_to_i32"), -2);
    assert_eq!(int(0.499_999_999_999_999_94, "round_to_i32"), 0, "the double just below 0.5");
    assert!(faults(2_147_483_647.5, "round_to_i32"), "to even is 2^31");
    assert!(faults(f64::NAN, "round_to_i32"));
    let to_f32 = |x: f64| Case::new(&world, vec![(Value::F64(x), f64_ty)], vec![f32_ty]).one(Core, "to_f32");
    assert!(matches!(to_f32(0.1), Value::F32(x) if x == 0.1_f32));
    assert!(matches!(to_f32(1e39), Value::F32(x) if x == f32::INFINITY));
}

#[test]
fn core_call_gives_the_function_its_inputs() {
    let mut world = World::new();
    let f64_ty = world.number(NumberKind::F64, "");
    let reference = world.constant(f64_ty, ConstValue::Unsettled);
    let case = Case::new(&world, vec![(Value::Function(reference), f64_ty), (Value::F64(1.0), f64_ty)], vec![f64_ty]);
    match case.outcome(Core, "call") {
        Ok(Outcome::Call { function: Value::Function(id), inputs }) => {
            assert_eq!(id, reference);
            assert!(matches!(inputs.as_slice(), [Value::F64(x)] if *x == 1.0));
        }
        other => panic!("{other:?}"),
    }
}

/// An array type of `i32`, with the default constants of `array.get` and the `Skip` of a call of it.
fn array_world() -> (World, TyId, TyId, TyId, TyId) {
    let mut world = World::new();
    let i32_ty = world.number(NumberKind::I32, "");
    let array_ty = world.ty(Ty::Array { element: i32_ty, dimensions: 1 });
    let error_ty = world.ty(Ty::ErrorValue);
    let grid_ty = world.ty(Ty::Array { element: i32_ty, dimensions: 2 });
    (world, i32_ty, array_ty, error_ty, grid_ty)
}

#[test]
fn get_and_set_fail_out_of_the_array() {
    let (mut world, i32_ty, array_ty, error_ty, _) = array_world();
    let zero = world.constant(i32_ty, ConstValue::Int(0));
    let none = world.constant(error_ty, ConstValue::NoError);
    let empty = world.constant(array_ty, ConstValue::Array(Vec::new()));
    let get = |index: i32| {
        Case::new(&world, vec![(i32s(&[10, 20, 30]), array_ty), (Value::I32(index), i32_ty), (Value::NONE, error_ty)], vec![i32_ty, error_ty])
    };
    assert!(matches!(get(1).done(Array, "get").as_slice(), [Value::I32(20), Value::Error(None)]));
    let (error, inputs) = get(-1).fails(Array, "get");
    assert_eq!(&*error.category, "array.index_out_of_range");
    assert_eq!(&*error.message, "index -1 outside the array (size 3)");
    assert_eq!(get(3).fails(Array, "get").0.message.as_ref(), "index 3 outside the array (size 3)");
    // Its outputs are those of a node that fails (7.2): `item` takes its default, `err` the error.
    let skip = Skip::OnError { err: 2, always: false, defaults: vec![zero, none], through: vec![None, Some(2)] };
    let outputs = outputs_on_error(world.env(), &skip, &inputs, &[], Some(Value::error(error)), &FakeHost::new()).unwrap();
    assert!(matches!(outputs.as_slice(), [Value::I32(0), Value::Error(Some(error))] if &*error.category == "array.index_out_of_range"));

    let set = |index: i32| {
        Case::new(
            &world,
            vec![(i32s(&[10, 20, 30]), array_ty), (Value::I32(index), i32_ty), (Value::I32(99), i32_ty), (Value::NONE, error_ty)],
            vec![array_ty, error_ty],
        )
    };
    assert_eq!(i32s_of(&set(2).done(Array, "set")[0]), [10, 20, 99]);
    // An error of `array.set` gives the array unchanged: the output `array` passes its input through.
    let (error, inputs) = set(5).fails(Array, "set");
    let skip = Skip::OnError { err: 3, always: false, defaults: vec![empty, none], through: vec![Some(0), Some(3)] };
    let outputs = outputs_on_error(world.env(), &skip, &inputs, &[], Some(Value::error(error)), &FakeHost::new()).unwrap();
    assert_eq!(i32s_of(&outputs[0]), [10, 20, 30]);
}

#[test]
fn a_failed_slice_passes_its_array_through() {
    let (mut world, i32_ty, array_ty, error_ty, grid_ty) = array_world();
    let none = world.constant(error_ty, ConstValue::NoError);
    let empty = world.constant(array_ty, ConstValue::Array(Vec::new()));
    let slice = |start: i32, length: Value| {
        Case::new(
            &world,
            vec![(i32s(&[10, 20, 30, 40]), array_ty), (Value::I32(start), i32_ty), (length, i32_ty), (Value::NONE, error_ty)],
            vec![array_ty, error_ty],
        )
    };
    assert_eq!(i32s_of(&slice(1, Value::I32(2)).done(Array, "slice")[0]), [20, 30]);
    assert_eq!(i32s_of(&slice(1, Value::ToTheEnd).done(Array, "slice")[0]), [20, 30, 40], "with no wire, to the end");
    assert_eq!(i32s_of(&slice(4, Value::ToTheEnd).done(Array, "slice")[0]), [] as [i32; 0], "nothing left: empty, no error");
    assert_eq!(i32s_of(&slice(0, Value::I32(0)).done(Array, "slice")[0]), [] as [i32; 0]);
    for (start, length) in [(-1, Value::ToTheEnd), (5, Value::ToTheEnd), (1, Value::I32(-1)), (3, Value::I32(2))] {
        let (error, inputs) = slice(start, length).fails(Array, "slice");
        assert_eq!(&*error.category, "array.index_out_of_range");
        // Decision of the owner (2026-11-09): rule 7.2, the output `array` passes its input through.
        let skip = Skip::OnError { err: 3, always: false, defaults: vec![empty, none], through: vec![Some(0), Some(3)] };
        let outputs = outputs_on_error(world.env(), &skip, &inputs, &[], Some(Value::error(error)), &FakeHost::new()).unwrap();
        assert_eq!(i32s_of(&outputs[0]), [10, 20, 30, 40], "start {start}");
    }
    // Whole rows of an `[i32; 2]`: rows 1 and 2 of [[1, 2], [3, 4], [5, 6]] are [[3, 4], [5, 6]].
    let rows = crate::convert::grid(vec![i32s(&[1, 2]), i32s(&[3, 4]), i32s(&[5, 6])], 2).unwrap();
    let case = Case::new(
        &world,
        vec![(rows, grid_ty), (Value::I32(1), i32_ty), (Value::ToTheEnd, i32_ty), (Value::NONE, error_ty)],
        vec![grid_ty, error_ty],
    );
    match &case.done(Array, "slice")[0] {
        Value::Grid(grid) => {
            assert_eq!(grid.shape(), [2, 2]);
            assert!(matches!(grid.items(), [Value::I32(3), Value::I32(4), Value::I32(5), Value::I32(6)]));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn arrays_grow_repeat_and_reshape() {
    let (world, i32_ty, array_ty, error_ty, grid_ty) = array_world();
    let len = Case::new(&world, vec![(i32s(&[1, 2, 3]), array_ty)], vec![i32_ty]).one(Array, "len");
    assert!(matches!(len, Value::I32(3)));
    let push = Case::new(&world, vec![(i32s(&[1]), array_ty), (Value::I32(2), i32_ty)], vec![array_ty]).one(Array, "push");
    assert_eq!(i32s_of(&push), [1, 2]);
    let concat = Case::new(&world, vec![(i32s(&[1]), array_ty), (i32s(&[]), array_ty), (i32s(&[2, 3]), array_ty)], vec![array_ty]).variadic(0, 3);
    assert_eq!(i32s_of(&concat.one(Array, "concat")), [1, 2, 3]);
    let repeat =
        |count: i32| Case::new(&world, vec![(Value::I32(7), i32_ty), (Value::I32(count), i32_ty), (Value::NONE, error_ty)], vec![array_ty, error_ty]);
    assert_eq!(i32s_of(&repeat(3).done(Array, "repeat")[0]), [7, 7, 7]);
    assert_eq!(i32s_of(&repeat(0).done(Array, "repeat")[0]), [] as [i32; 0]);
    assert_eq!(&*repeat(-1).fails(Array, "repeat").0.category, "array.negative_size");
    // cli.md 6.2: `array.repeat("", 2147483647)` counts 32 GiB, and the limit stops it before anything is built.
    let big = Case::new(&world, vec![(Value::string(""), i32_ty), (Value::I32(i32::MAX), i32_ty), (Value::NONE, error_ty)], vec![array_ty, error_ty]);
    assert_eq!(big.fault(Array, "repeat"), FaultCode::Memory);

    let rows = |items: Vec<Value>| Case::new(&world, vec![(Value::array(items), array_ty), (Value::NONE, error_ty)], vec![grid_ty, error_ty]);
    match &rows(vec![i32s(&[1, 2]), i32s(&[3, 4])]).done(Array, "from_rows")[0] {
        Value::Grid(grid) => assert_eq!(grid.shape(), [2, 2]),
        other => panic!("{other:?}"),
    }
    match &rows(Vec::new()).done(Array, "from_rows")[0] {
        Value::Grid(grid) => assert_eq!(grid.rows(), 0, "no row at all: empty"),
        other => panic!("{other:?}"),
    }
    let (error, _) = rows(vec![i32s(&[1, 2]), i32s(&[3])]).fails(Array, "from_rows");
    assert_eq!(&*error.category, "array.unequal_rows");
    assert_eq!(&*error.message, "row 1 has a length of 1, and row 0 of 2");

    let grid = crate::convert::grid(vec![i32s(&[1, 2]), i32s(&[3, 4])], 2).unwrap();
    let to_rows = Case::new(&world, vec![(grid, grid_ty)], vec![array_ty]).one(Array, "to_rows");
    let Value::Array(rows) = to_rows else { panic!() };
    assert_eq!(rows.items().iter().map(i32s_of).collect::<Vec<_>>(), [vec![1, 2], vec![3, 4]]);
}

#[test]
fn sums_go_from_the_first_item_to_the_last() {
    let mut world = World::new();
    let f64_ty = world.number(NumberKind::F64, "");
    let i32_ty = world.number(NumberKind::I32, "");
    let reals = world.ty(Ty::Array { element: f64_ty, dimensions: 1 });
    let ints = world.ty(Ty::Array { element: i32_ty, dimensions: 1 });
    // Python: ((0 + 0.1) + 0.2) + 0.3 == 0.6000000000000001.
    let values = Value::array(vec![Value::F64(0.1), Value::F64(0.2), Value::F64(0.3)]);
    assert_eq!(f64_of(&Case::new(&world, vec![(values, reals)], vec![f64_ty]).one(Array, "sum")), 0.600_000_000_000_000_1);
    assert!(matches!(Case::new(&world, vec![(i32s(&[]), ints)], vec![i32_ty]).one(Array, "sum"), Value::I32(0)));
    assert!(matches!(Case::new(&world, vec![(i32s(&[1, 2, 3]), ints)], vec![i32_ty]).one(Array, "sum"), Value::I32(6)));
    assert_eq!(Case::new(&world, vec![(i32s(&[i32::MAX, 1]), ints)], vec![i32_ty]).fault(Array, "sum"), FaultCode::OutOfRange);
}

#[test]
fn texts_join_and_split() {
    let mut world = World::new();
    let string_ty = world.ty(Ty::String);
    let strings = world.ty(Ty::Array { element: string_ty, dimensions: 1 });
    let concat = Case::new(&world, vec![(Value::string("a"), string_ty), (Value::string("bc"), string_ty)], vec![string_ty]).variadic(0, 2);
    assert_eq!(text_of(&concat.one(Strings, "concat")), "abc");
    assert_eq!(text_of(&Case::new(&world, vec![(texts(&["x", "", "y"]), strings)], vec![string_ty]).one(Strings, "join")), "xy");
    let split =
        |text: &str, separator: &str| Case::new(&world, vec![(Value::string(text), string_ty), (Value::string(separator), string_ty)], vec![strings]);
    assert_eq!(texts_of(&split("a,,b", ",").one(Strings, "split")), ["a", "", "b"]);
    assert_eq!(texts_of(&split("", ",").one(Strings, "split")), [""]);
    assert_eq!(texts_of(&split("1.5;2", ";").one(Strings, "split")), ["1.5", "2"]);
    assert_eq!(split("ab", "").fault(Strings, "split"), FaultCode::OutOfRange, "as the proof of concept refuses it");
}

#[test]
fn numbers_read_from_texts() {
    let mut world = World::new();
    let string_ty = world.ty(Ty::String);
    let f64_ty = world.number(NumberKind::F64, "");
    let i32_ty = world.number(NumberKind::I32, "");
    let error_ty = world.ty(Ty::ErrorValue);
    let read =
        |text: &str, _: &str, out: TyId| Case::new(&world, vec![(Value::string(text), string_ty), (Value::NONE, error_ty)], vec![out, error_ty]);
    let real = |text: &str| match read(text, "to_f64", f64_ty).outcome(Strings, "to_f64") {
        Ok(Outcome::Done(outputs)) => Some(f64_of(&outputs[0])),
        Ok(Outcome::Fail(_)) => None,
        other => panic!("{other:?}"),
    };
    assert_eq!(real(" 1.5 "), Some(1.5), "the spaces around do not count");
    assert_eq!(real("1e3"), Some(1000.0));
    assert_eq!(real("-2.5E-1"), Some(-0.25));
    assert_eq!(real(".5"), Some(0.5));
    assert_eq!(real("5."), Some(5.0));
    assert_eq!(real("+7"), Some(7.0));
    assert!(real("NaN").is_some_and(f64::is_nan));
    assert_eq!(real("-INF"), Some(f64::NEG_INFINITY));
    for text in ["", "abc", "1e", "1e+", ".", "infinity", "0x10", "1_000", "1,5", "--1", "1.2.3", "e5"] {
        assert_eq!(real(text), None, "{text:?}");
    }
    // Beyond the f64, a failure rather than an infinity; below it, zero with its sign (decision of the owner, 2026-11-13).
    // Worked out by hand: f64::MAX is about 1.797e308, its smallest subnormal about 4.9e-324.
    for text in ["1e999", "-1e999", "2e308", "+1.8e308"] {
        assert_eq!(real(text), None, "{text:?}");
    }
    assert_eq!(real("1.7e308"), Some(1.7e308));
    // The boundary is where the nearest f64 becomes infinite: halfway between f64::MAX (1.7976931348623157e308) and
    // 2^1024, about 1.7976931348623158079e308. Below it the text rounds down to f64::MAX, above it fails.
    assert_eq!(real("1.7976931348623158e308"), Some(f64::MAX));
    assert_eq!(real("1.7976931348623159e308"), None);
    assert_eq!(real("\u{1c}1"), None, "U+001C is not white space");
    assert_eq!(real("1e-999").map(f64::to_bits), Some(0.0f64.to_bits()));
    assert_eq!(real("-1e-999").map(f64::to_bits), Some((-0.0f64).to_bits()));
    assert_eq!(real("\u{a0}\t2\r\n"), Some(2.0), "any Unicode white space around");
    let (error, _) = read("1e999", "to_f64", f64_ty).fails(Strings, "to_f64");
    assert_eq!((&*error.category, &*error.message), ("string.parse", "beyond an f64: \"1e999\""));
    let (error, _) = read("abc", "to_f64", f64_ty).fails(Strings, "to_f64");
    assert_eq!(&*error.category, "string.parse");
    assert_eq!(&*error.message, "not a number: \"abc\"");
    let long = "x".repeat(100);
    let (error, _) = read(&long, "to_f64", f64_ty).fails(Strings, "to_f64");
    assert_eq!(error.message.chars().count(), "not a number: ".len() + 64 + 2 + 1, "its start, between quotes, then `…`");

    let int = |text: &str| match read(text, "to_i32", i32_ty).outcome(Strings, "to_i32") {
        Ok(Outcome::Done(outputs)) => match outputs[0] {
            Value::I32(x) => Some(x),
            _ => panic!(),
        },
        Ok(Outcome::Fail(_)) => None,
        other => panic!("{other:?}"),
    };
    assert_eq!(int("42"), Some(42));
    assert_eq!(int(" -7 "), Some(-7));
    assert_eq!(int("+3"), Some(3));
    assert_eq!(int("-2147483648"), Some(i32::MIN));
    assert_eq!(int("\u{3000}5\u{85}"), Some(5), "any Unicode white space around, as for to_f64");
    for text in ["2147483648", "1.0", "", "+", "1e3", "0x1", "\u{1c}12"] {
        assert_eq!(int(text), None, "{text:?}");
    }
}

#[test]
fn time_reads_the_clock_of_the_run() {
    let mut world = World::new();
    let duration = world.ty(Ty::Duration);
    let timestamp = world.ty(Ty::Timestamp);
    let error_ty = world.ty(Ty::ErrorValue);
    let mut host = FakeHost::new();
    host.clock = 1_500;
    let now = Case::new(&world, Vec::new(), vec![timestamp]);
    assert!(matches!(now.run_with(Time, "now", &mut host).0, Ok(Outcome::Done(outputs)) if matches!(outputs.as_slice(), [Value::Timestamp(1_500)])));
    let elapsed = Case::new(&world, vec![(Value::Timestamp(500), timestamp)], vec![duration]);
    assert!(
        matches!(elapsed.run_with(Time, "elapsed", &mut host).0, Ok(Outcome::Done(outputs)) if matches!(outputs.as_slice(), [Value::Duration(1_000)]))
    );
    let far = Case::new(&world, vec![(Value::Timestamp(i64::MIN), timestamp)], vec![duration]);
    assert!(matches!(far.run_with(Time, "elapsed", &mut host).0, Err(Fault { code: FaultCode::OutOfRange, .. })));
    let wait = |ns: i64| Case::new(&world, vec![(Value::Duration(ns), duration), (Value::NONE, error_ty)], vec![error_ty]).outcome(Time, "wait");
    assert!(matches!(wait(2_000), Ok(Outcome::Wait(2_000))));
    assert!(matches!(wait(-5), Ok(Outcome::Wait(0))), "a negative duration waits nothing");
}

#[test]
fn errors_are_made_raised_and_merged_flat() {
    let mut world = World::new();
    let i32_ty = world.number(NumberKind::I32, "");
    let string_ty = world.ty(Ty::String);
    let bool_ty = world.ty(Ty::Bool);
    let error_ty = world.ty(Ty::ErrorValue);
    let none = world.constant(error_ty, ConstValue::NoError);
    let make = Case::new(&world, vec![(Value::I32(-410), i32_ty), (Value::string("interrupted"), string_ty)], vec![error_ty])
        .statics(vec![Static::Category("scpi.instrument_error".into())]);
    let made = make.one(Errors, "make");
    let made = made.as_error().unwrap();
    assert_eq!((&*made.category, made.code, &*made.message), ("scpi.instrument_error", -410, "interrupted"));

    let raise = |condition: bool| {
        Case::new(&world, vec![(Value::Bool(condition), bool_ty), (Value::string("late"), string_ty), (Value::NONE, error_ty)], vec![error_ty])
            .statics(vec![Static::Category("io.timeout".into())])
    };
    assert!(matches!(raise(false).done(Errors, "raise_if").as_slice(), [Value::Error(None)]));
    let (error, inputs) = raise(true).fails(Errors, "raise_if");
    let skip = Skip::OnError { err: 2, always: false, defaults: vec![none], through: vec![Some(2)] };
    let outputs = outputs_on_error(world.env(), &skip, &inputs, &[], Some(Value::error(error)), &FakeHost::new()).unwrap();
    assert!(matches!(outputs.as_slice(), [Value::Error(Some(error))] if &*error.category == "io.timeout" && &*error.message == "late"));

    let error = |category: &str, others: Vec<ErrorData>| ErrorData {
        category: category.into(),
        code: 0,
        message: "".into(),
        origin: None,
        others: Arc::new(others.into_iter().map(Arc::new).collect()),
    };
    // By hand (7.4, commands.md 5): the first error is the main one; its others, then each other error without its own,
    // followed by its own.
    let a = Value::error(error("a", vec![error("a1", Vec::new())]));
    let b = Value::error(error("b", vec![error("b1", Vec::new())]));
    let merge =
        Case::new(&world, vec![(Value::NONE, error_ty), (a, error_ty), (Value::NONE, error_ty), (b, error_ty)], vec![error_ty]).variadic(0, 4);
    let merged = merge.one(Errors, "merge");
    let merged = merged.as_error().unwrap();
    assert_eq!(&*merged.category, "a");
    let others: Vec<(&str, usize)> = merged.others.iter().map(|other| (&*other.category, other.others.len())).collect();
    assert_eq!(others, [("a1", 0), ("b", 0), ("b1", 0)]);
    let nothing = Case::new(&world, vec![(Value::NONE, error_ty), (Value::NONE, error_ty)], vec![error_ty]).variadic(0, 2);
    assert!(matches!(nothing.done(Errors, "merge").as_slice(), [Value::Error(None)]));
}

#[test]
fn a_node_that_skips_passes_on_the_error_it_received() {
    let mut world = World::new();
    let f64_ty = world.number(NumberKind::F64, "");
    let error_ty = world.ty(Ty::ErrorValue);
    let zero = world.constant(f64_ty, ConstValue::Float(0.0));
    let none = world.constant(error_ty, ConstValue::NoError);
    let received = Value::error(ErrorData { category: "io.timeout".into(), code: 0, message: "".into(), origin: None, others: Arc::default() });
    let own = Value::error(ErrorData { category: "visa.closed".into(), code: 0, message: "".into(), origin: None, others: Arc::default() });
    let skip = Skip::OnError { err: 1, always: true, defaults: vec![zero, none], through: vec![None, Some(1)] };
    let inputs = [Value::string("session"), received];
    let category = |outputs: &[Value]| outputs.get(1).and_then(Value::as_error).map(|error| error.category.to_string());
    let skipped = outputs_on_error(world.env(), &skip, &inputs, &[], None, &FakeHost::new()).unwrap();
    assert!(matches!(skipped[0], Value::F64(x) if x == 0.0), "the default of its type");
    assert_eq!(category(&skipped).as_deref(), Some("io.timeout"));
    // `always`: it ran and failed, and passes on the error it received in preference to its own.
    let failed = outputs_on_error(world.env(), &skip, &inputs, &[], Some(own.clone()), &FakeHost::new()).unwrap();
    assert_eq!(category(&failed).as_deref(), Some("io.timeout"));
    let clean = [Value::string("session"), Value::NONE];
    assert_eq!(category(&outputs_on_error(world.env(), &skip, &clean, &[], Some(own), &FakeHost::new()).unwrap()).as_deref(), Some("visa.closed"));
}

#[test]
fn the_panel_belongs_to_the_host() {
    let mut world = World::new();
    let bool_ty = world.ty(Ty::Bool);
    let mut host = FakeHost::new();
    let set = Case::new(&world, vec![(Value::Bool(true), bool_ty)], vec![bool_ty]).statics(vec![Static::Element(ElementId(3))]);
    assert!(matches!(set.run_with(Ui, "set", &mut host).0, Ok(Outcome::Done(outputs)) if matches!(outputs.as_slice(), [Value::Bool(true)])));
    let get = Case::new(&world, Vec::new(), vec![bool_ty]).statics(vec![Static::Element(ElementId(3))]);
    assert!(matches!(get.run_with(Ui, "get", &mut host).0, Ok(Outcome::Done(outputs)) if matches!(outputs.as_slice(), [Value::Bool(true)])));
}

/// `string.format` of one field, `{v}` with its format, on a value of the given type.
fn formatted(world: &World, format: &str, value: Value, ty: TyId) -> String {
    let case = Case::new(world, vec![(value, ty)], vec![ty]).statics(vec![Static::Template(format!("{{v{format}}}").into())]);
    text_of(&case.one(Strings, "format"))
}

#[test]
fn format_writes_values_as_the_reports_do() {
    let mut world = World::new();
    let volts = world.number(NumberKind::F64, "V");
    let f64_ty = world.number(NumberKind::F64, "");
    let f32_ty = world.number(NumberKind::F32, "");
    let i32_ty = world.number(NumberKind::I32, "");
    let c128 = world.number(NumberKind::C128, "");
    let bool_ty = world.ty(Ty::Bool);
    let string_ty = world.ty(Ty::String);
    let bytes_ty = world.ty(Ty::Bytes);
    let duration = world.ty(Ty::Duration);
    let timestamp = world.ty(Ty::Timestamp);
    let error_ty = world.ty(Ty::ErrorValue);
    let strings = world.ty(Ty::Array { element: string_ty, dimensions: 1 });
    let voltages = world.ty(Ty::Array { element: volts, dimensions: 1 });
    let grid_ty = world.ty(Ty::Array { element: i32_ty, dimensions: 2 });
    let pass = world.names.intern("Pass");
    let fail = world.names.intern("Fail");
    let verdict = world.named("Verdict", Ty::Enum(vec![Variant { name: pass, values: Vec::new() }, Variant { name: fail, values: Vec::new() }]));
    let value_name = world.names.intern("Value");
    let overload = world.names.intern("Overload");
    let reading =
        world.named("Reading", Ty::Enum(vec![Variant { name: value_name, values: vec![volts] }, Variant { name: overload, values: Vec::new() }]));
    let (low, high) = (world.names.intern("low"), world.names.intern("high"));
    let limits = world.named("Limits", Ty::Struct(vec![(low, volts), (high, volts)]));

    // The text of each type, worked out by hand from the decision of the owner (2026-11-09): as the reports write them.
    assert_eq!(formatted(&world, "", Value::F64(230.071_217_3), volts), "230.0712173 V");
    assert_eq!(formatted(&world, "", Value::F64(12.0), f64_ty), "12.0", "a float keeps `.0`");
    assert_eq!(formatted(&world, "", Value::I32(12), i32_ty), "12");
    assert_eq!(formatted(&world, "", Value::F32(0.1), f32_ty), "0.1", "the shortest digits of the f32");
    assert_eq!(formatted(&world, "", Value::F64(1.5e-7), f64_ty), "1.5e-7", "outside 1e-5 to 1e16, an exponent");
    assert_eq!(formatted(&world, "", Value::F64(f64::NAN), f64_ty), "nan");
    assert_eq!(formatted(&world, "", Value::C128(1.0, -2.0), c128), "1.0 - 2.0i");
    assert_eq!(formatted(&world, "", Value::Bool(true), bool_ty), "true");
    assert_eq!(formatted(&world, "", Value::string("abc"), string_ty), "abc", "a text alone, as it is");
    assert_eq!(formatted(&world, "", Value::Bytes(Arc::from([0x0a_u8, 0xff, 0x03].as_slice())), bytes_ty), "0aff03");
    assert_eq!(formatted(&world, "", Value::Duration(100_000_000), duration), "0.1 s");
    assert_eq!(formatted(&world, "", Value::Timestamp(250), timestamp), "1970-01-01T00:00:00.000000250Z");
    assert_eq!(formatted(&world, "", Value::NONE, error_ty), "none");
    let timeout = Value::error(ErrorData {
        category: "io.timeout".into(),
        code: -1_073_807_339,
        message: "no answer".into(),
        origin: None,
        others: Arc::new(vec![Arc::new(ErrorData {
            category: "dmm.overload".into(),
            code: 0,
            message: "".into(),
            origin: None,
            others: Arc::default(),
        })]),
    });
    assert_eq!(formatted(&world, "", timeout, error_ty), "io.timeout (-1073807339): no answer; dmm.overload");
    assert_eq!(formatted(&world, "", texts(&["a", "b\"c"]), strings), "[\"a\", \"b\\\"c\"]", "a text inside, as a literal");
    assert_eq!(formatted(&world, "", Value::array(vec![Value::F64(1.0), Value::F64(0.2)]), voltages), "[1.0 V, 0.2 V]");
    let variant = |index: u32, values: Vec<Value>| Value::Variant(Arc::new(crate::value::VariantValue { index, values: values.into() }));
    assert_eq!(formatted(&world, "", variant(0, Vec::new()), verdict), "Verdict.Pass");
    assert_eq!(formatted(&world, "", variant(0, vec![Value::F64(230.0)]), reading), "Reading.Value(230.0 V)");
    let bounds = Value::Struct(vec![Value::F64(220.0), Value::F64(240.0)].into());
    assert_eq!(formatted(&world, "", bounds, limits), "Limits { low: 220.0 V, high: 240.0 V }");
    let rows = crate::convert::grid(vec![i32s(&[1, 2]), i32s(&[3, 4])], 2).unwrap();
    assert_eq!(formatted(&world, "", rows, grid_ty), "[[1, 2], [3, 4]]");
    let empty_rows = crate::convert::grid(vec![i32s(&[]), i32s(&[])], 2).unwrap();
    assert_eq!(formatted(&world, "", empty_rows, grid_ty), "[[], []]");
}

#[test]
fn format_applies_its_parts() {
    let mut world = World::new();
    let volts = world.number(NumberKind::F64, "V");
    let f64_ty = world.number(NumberKind::F64, "");
    let i32_ty = world.number(NumberKind::I32, "");
    let string_ty = world.ty(Ty::String);
    let duration = world.ty(Ty::Duration);
    let voltages = world.ty(Ty::Array { element: volts, dimensions: 1 });
    // Python: '%.3f' % 1.2345 == '1.234' (1.2345 is a little below), and 1.2345 * 1000 == 1234.5, '%.3f' of it '1.234500'.
    assert_eq!(formatted(&world, ":.3", Value::F64(1.2345), volts), "1.234 V");
    assert_eq!(formatted(&world, ":.3 mV", Value::F64(1.2345), volts), "1234.500 mV");
    assert_eq!(formatted(&world, ":mV", Value::F64(1.5), volts), "1500.0 mV", "a unit alone, the shortest digits");
    // The examples of 5.4, by hand.
    assert_eq!(formatted(&world, ":#06x", Value::I32(255), i32_ty), "0x00ff");
    assert_eq!(formatted(&world, ":02", Value::I32(1), i32_ty), "01");
    assert_eq!(formatted(&world, ":*^12", Value::string("abc"), string_ty), "****abc*****", "what `^` cannot split goes right");
    assert_eq!(formatted(&world, ":.3", Value::I32(5), i32_ty), "5.000");
    // Python: '%+.2f' % 1.23456, '%e' % 1234.5, '%.3g' % 1234.5, '%g' % 0.0001, '%g' % 0.00001, '%g' % 100000.0, '%g' % 1e6.
    assert_eq!(formatted(&world, ":+.2", Value::F64(1.23456), f64_ty), "+1.23");
    assert_eq!(formatted(&world, ":e", Value::F64(1234.5), f64_ty), "1.234500e+03");
    assert_eq!(formatted(&world, ":E", Value::F64(1234.5), f64_ty), "1.234500E+03");
    assert_eq!(formatted(&world, ":.3g", Value::F64(1234.5), f64_ty), "1.23e+03");
    assert_eq!(formatted(&world, ":g", Value::F64(0.0001), f64_ty), "0.0001");
    assert_eq!(formatted(&world, ":g", Value::F64(0.00001), f64_ty), "1e-05");
    assert_eq!(formatted(&world, ":g", Value::F64(100_000.0), f64_ty), "100000");
    assert_eq!(formatted(&world, ":g", Value::F64(1e6), f64_ty), "1e+06");
    assert_eq!(formatted(&world, ":f", Value::F64(0.5), f64_ty), "0.500000", "six decimals, as C++");
    // Python: '%08.3f' % -1.5 == '-001.500'.
    assert_eq!(formatted(&world, ":08.3", Value::F64(-1.5), f64_ty), "-001.500");
    assert_eq!(formatted(&world, ":08.2", Value::F64(f64::NAN), f64_ty), "     nan", "no zeros before nan");
    assert_eq!(formatted(&world, ":>8", Value::F64(1.5), f64_ty), "     1.5");
    assert_eq!(formatted(&world, ":8", Value::F64(1.5), f64_ty), "     1.5", "a number to the right");
    assert_eq!(formatted(&world, ":5", Value::string("ab"), string_ty), "ab   ", "a text to the left");
    assert_eq!(formatted(&world, ":x", Value::I32(-255), i32_ty), "-ff");
    assert_eq!(formatted(&world, ":#o", Value::I32(8), i32_ty), "010");
    assert_eq!(formatted(&world, ":#o", Value::I32(0), i32_ty), "0");
    assert_eq!(formatted(&world, ":#b", Value::I32(5), i32_ty), "0b101");
    assert_eq!(formatted(&world, ":X", Value::I32(255), i32_ty), "FF");
    assert_eq!(formatted(&world, ":+d", Value::I32(5), i32_ty), "+5");
    // Python: '%.1f' % 1.25 == '1.2' and '%.1f' % 2.35 == '2.4': the format goes to each value inside.
    assert_eq!(formatted(&world, ":.1", Value::array(vec![Value::F64(1.25), Value::F64(2.35)]), voltages), "[1.2 V, 2.4 V]");
    assert_eq!(formatted(&world, ":.3 ms", Value::Duration(1_500_000), duration), "1.500 ms");
    assert_eq!(formatted(&world, ":.2", Value::Duration(100_000_000), duration), "0.10 s");
}

#[test]
fn format_fills_the_template_and_respects_the_memory_of_the_run() {
    let mut world = World::new();
    let volts = world.number(NumberKind::F64, "V");
    let string_ty = world.ty(Ty::String);
    let template = "U = {u:.3 mV}, {{{name}}} and {u}";
    let case = Case::new(&world, vec![(Value::F64(0.5), volts), (Value::string("dmm"), string_ty)], vec![string_ty])
        .statics(vec![Static::Template(template.into())]);
    assert_eq!(text_of(&case.one(Strings, "format")), "U = 500.000 mV, {dmm} and 0.5 V");
    let mut host = FakeHost::new();
    host.room = 10;
    assert!(matches!(case.run_with(Strings, "format", &mut host).0, Err(Fault { code: FaultCode::Memory, .. })));
}

#[test]
fn wires_convert_as_spec_3_3_says() {
    use labcraft_ir::{Conversion, Scale};
    let millivolts_to_volts =
        Conversion::Number { to: NumberKind::F64, scale: labcraft_ir::scale_between(&parse_unit("mV", 8).unwrap(), &parse_unit("V", 8).unwrap()) };
    // Python: 230 / 1000 == 0.23.
    assert!(matches!(convert(Value::F64(230.0), &millivolts_to_volts), Ok(Value::F64(x)) if x == 0.23));
    let widen = Conversion::Number { to: NumberKind::I64, scale: Scale::default() };
    assert!(matches!(convert(Value::I32(-3), &widen), Ok(Value::I64(-3))));
    let to_f64 = Conversion::Number { to: NumberKind::F64, scale: Scale::default() };
    assert!(matches!(convert(Value::I32(7), &to_f64), Ok(Value::F64(x)) if x == 7.0));
    // To a duration (3.1): 0.1 s is 100,000,000 ns; not finite, or beyond an i64 of nanoseconds, LC8008 (Q-25).
    let seconds = Conversion::ToDuration { scale: Scale::default() };
    assert!(matches!(convert(Value::F64(0.1), &seconds), Ok(Value::Duration(100_000_000))));
    assert!(matches!(convert(Value::F64(f64::INFINITY), &seconds), Err(Fault { code: FaultCode::OutOfRange, .. })));
    assert!(matches!(convert(Value::F64(f64::NAN), &seconds), Err(Fault { code: FaultCode::OutOfRange, .. })));
    assert!(matches!(convert(Value::F64(1e10), &seconds), Err(Fault { code: FaultCode::OutOfRange, .. })), "1e19 ns");
    // From a duration: 1,500,000,000 ns is 1.5 s, times 1000 for milliseconds (Python: 1500000000 / 1e9 * 1000 == 1500.0).
    let to_millis = Conversion::FromDuration { to: NumberKind::F64, scale: labcraft_ir::scale_between(&Unit::SECOND, &parse_unit("ms", 8).unwrap()) };
    assert!(matches!(convert(Value::Duration(1_500_000_000), &to_millis), Ok(Value::F64(x)) if x == 1500.0));
}

#[test]
fn constants_take_their_types() {
    let mut world = World::new();
    let i32_ty = world.number(NumberKind::I32, "");
    let f32_ty = world.number(NumberKind::F32, "");
    let u8_ty = world.number(NumberKind::U8, "");
    let row = world.ty(Ty::Array { element: i32_ty, dimensions: 1 });
    let grid_ty = world.ty(Ty::Array { element: i32_ty, dimensions: 2 });
    let opaque = world.ty(Ty::Named { library: world.library, name: Interner::new().intern("x") });
    let ints: Vec<ConstId> = (1..=4).map(|x| world.constant(i32_ty, ConstValue::Int(x))).collect();
    let first = world.constant(row, ConstValue::Array(ints[..2].to_vec()));
    let second = world.constant(row, ConstValue::Array(ints[2..].to_vec()));
    let rows = world.constant(grid_ty, ConstValue::Array(vec![first, second]));
    let small = world.constant(f32_ty, ConstValue::Float(f64::from(0.1_f32)));
    let byte = world.constant(u8_ty, ConstValue::Int(255));
    let handle = world.constant(opaque, ConstValue::InvalidHandle);
    let unsettled = world.constant(i32_ty, ConstValue::Unsettled);
    match constant(world.env(), rows).unwrap() {
        Value::Grid(grid) => {
            assert_eq!(grid.shape(), [2, 2]);
            assert!(matches!(grid.items(), [Value::I32(1), Value::I32(2), Value::I32(3), Value::I32(4)]));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(constant(world.env(), small).unwrap(), Value::F32(x) if x == 0.1_f32));
    assert!(matches!(constant(world.env(), byte).unwrap(), Value::U8(255)));
    assert!(matches!(constant(world.env(), handle).unwrap(), Value::Handle(handle) if !handle.is_valid()));
    assert_eq!(constant(world.env(), unsettled).unwrap_err().code, FaultCode::Bug, "Q-21 is settled with the interpreter");
}

#[test]
fn a_wire_reads_its_fields() {
    let mut world = World::new();
    let f64_ty = world.number(NumberKind::F64, "");
    let inner = Value::Struct(vec![Value::F64(1.0), Value::F64(2.0)].into());
    let outer = Value::Struct(vec![Value::Bool(true), inner].into());
    let used = labcraft_ir::Use { source: labcraft_ir::Source::Arg(0), fields: vec![1, 0], convert: None, ty: f64_ty, origin: labcraft_graph_id() };
    assert!(matches!(read(&outer, &used), Ok(Value::F64(x)) if x == 1.0));
}

fn labcraft_graph_id() -> labcraft_graph::Id {
    let place = labcraft_types::Position { line: 1, column: 1 };
    labcraft_graph::SourceMap::default().add(labcraft_types::Span { start: place, end: place })
}

#[test]
fn the_logical_size_is_that_of_cli_md() {
    // By hand from cli.md 6.2: 16 for each value, then the bytes of what it holds.
    assert_eq!(logical_size(&Value::F64(1.0)), 16 + 8);
    assert_eq!(logical_size(&Value::Bool(true)), 16 + 1);
    assert_eq!(logical_size(&Value::F32(1.0)), 16 + 4);
    assert_eq!(logical_size(&Value::C128(1.0, 2.0)), 16 + 16);
    assert_eq!(logical_size(&Value::Timestamp(0)), 16 + 16);
    assert_eq!(logical_size(&Value::string("ab")), 16 + 2);
    assert_eq!(logical_size(&Value::string("")), 16, "an empty string still costs something");
    assert_eq!(logical_size(&Value::array(vec![Value::F64(1.0); 3])), 16 + 3 * 24);
    assert_eq!(logical_size(&Value::array(Vec::new())), 16);
    assert_eq!(logical_size(&Value::Struct(vec![Value::F64(1.0), Value::Bool(false)].into())), 16 + 24 + 17);
    let nested = Value::array(vec![Value::array(vec![Value::I32(1)]), Value::array(Vec::new())]);
    assert_eq!(logical_size(&nested), 16 + (16 + 20) + 16);
}

#[test]
fn a_change_asks_for_what_it_adds_unless_the_array_is_shared() {
    let mut world = World::new();
    let f64_ty = world.number(NumberKind::F64, "");
    let reals = world.ty(Ty::Array { element: f64_ty, dimensions: 1 });
    let statics: Vec<Static> = Vec::new();
    let push = |array: Value, room: u64| {
        let mut call = Call {
            env: world.env(),
            viewer: world.library,
            inputs: vec![array, Value::F64(1.0)],
            input_types: vec![reals, f64_ty],
            output_types: vec![reals],
            statics: &statics,
            variadic: None,
        };
        let mut host = FakeHost::new();
        host.room = room;
        run(Array, "push", &mut call, &mut host).map(|_| ())
    };
    // 1,000 f64: 16 + 1,000 · 24 bytes. A push of one f64 adds 24; with 100 bytes left, it fits when nothing else holds the
    // array, which then grows in place, and not when it is shared, since the push works on a copy (D-11).
    let big = Value::array(vec![Value::F64(0.0); 1000]);
    assert_eq!(push(big.clone(), 100).unwrap_err().code, FaultCode::Memory, "shared: a whole copy");
    drop(big);
    assert!(push(Value::array(vec![Value::F64(0.0); 1000]), 100).is_ok(), "alone: only what it adds");
    assert_eq!(push(Value::array(vec![Value::F64(0.0); 1000]), 10).unwrap_err().code, FaultCode::Memory);
}

#[test]
fn only_the_sum_has_to_fit_its_kind() {
    let mut world = World::new();
    let i32_ty = world.number(NumberKind::I32, "");
    let ints = world.ty(Ty::Array { element: i32_ty, dimensions: 1 });
    let sum = Case::new(&world, vec![(i32s(&[i32::MAX, 1, -1]), ints)], vec![i32_ty]).one(Array, "sum");
    assert!(matches!(sum, Value::I32(i32::MAX)), "by hand: 2^31 - 1 + 1 - 1");
}

#[test]
fn zeros_count_the_unit_and_a_complex_number_is_one_text() {
    let mut world = World::new();
    let volts = world.number(NumberKind::F64, "V");
    let duration = world.ty(Ty::Duration);
    let c128 = world.number(NumberKind::C128, "");
    // By hand: `1.50 V` is 6 characters, 2 zeros make 8; `1.500 s` is 7, one zero; `1.00 - 2.00i` is 12, two zeros make 14.
    assert_eq!(formatted(&world, ":08.2", Value::F64(1.5), volts), "001.50 V");
    assert_eq!(formatted(&world, ":08.2 V", Value::F64(1.5), volts), "001.50 V");
    assert_eq!(formatted(&world, ":08.3", Value::Duration(1_500_000_000), duration), "01.500 s");
    assert_eq!(formatted(&world, ":014.2", Value::C128(1.0, -2.0), c128), "001.00 - 2.00i");
    assert_eq!(formatted(&world, ":+08.2", Value::F64(1.5), volts), "+01.50 V", "after the sign");
}

#[test]
fn a_text_inside_has_the_escapes_of_the_language_only() {
    let mut world = World::new();
    let string_ty = world.ty(Ty::String);
    let strings = world.ty(Ty::Array { element: string_ty, dimensions: 1 });
    // 14.1: `\"`, `\\`, `\n`, `\t`, `\r`; a control character as `\u{1b}` (decision of the owner); a combining mark as it is.
    let shown = formatted(&world, "", texts(&["a\"b\\c\nd\te\rf", "\u{1b}[31m\u{0}\u{85}", "hi\u{902}"]), strings);
    assert_eq!(shown, r#"["a\"b\\c\nd\te\rf", "\u{1b}[31m\u{0}\u{85}", "hi"#.to_owned() + "\u{902}\"]");
}

#[test]
fn a_merge_asks_for_the_room_of_what_it_gathers() {
    let mut world = World::new();
    let error_ty = world.ty(Ty::ErrorValue);
    let e = Value::error(ErrorData { category: "io.timeout".into(), code: 0, message: "late".into(), origin: None, others: Arc::default() });
    let merge = Case::new(&world, vec![(e.clone(), error_ty), (e, error_ty)], vec![error_ty]).variadic(0, 2);
    let mut host = FakeHost::new();
    host.room = 10;
    assert!(matches!(merge.run_with(Errors, "merge", &mut host).0, Err(Fault { code: FaultCode::Memory, .. })));
    let merged = merge.one(Errors, "merge");
    assert_eq!(merged.as_error().unwrap().others.len(), 1);
}

#[test]
fn grids_with_no_row_are_equal() {
    let mut world = World::new();
    let any = world.number(NumberKind::F64, "");
    let bool_ty = world.ty(Ty::Bool);
    let empty = Value::Grid(Arc::new(crate::value::Grid::empty(2)));
    let sliced = Value::Grid(Arc::new(crate::value::Grid::new(vec![0, 1].into_boxed_slice(), Vec::new()).unwrap()));
    let eq = Case::new(&world, vec![(empty, any), (sliced, any)], vec![bool_ty]).one(Core, "eq");
    assert!(matches!(eq, Value::Bool(true)), "both write `[]`");
    let grid = |shape: Vec<usize>| Value::Grid(Arc::new(crate::value::Grid::new(shape.into_boxed_slice(), Vec::new()).unwrap()));
    let eq = |a: Vec<usize>, b: Vec<usize>| {
        matches!(Case::new(&world, vec![(grid(a), any), (grid(b), any)], vec![bool_ty]).one(Core, "eq"), Value::Bool(true))
    };
    assert!(eq(vec![2, 0, 0], vec![2, 0, 5]), "both write `[[], []]`");
    assert!(!eq(vec![2, 0, 0], vec![3, 0, 0]), "`[[], []]` and `[[], [], []]`");
}

#[test]
fn rows_of_a_grid_ask_for_their_room() {
    let mut world = World::new();
    let f64_ty = world.number(NumberKind::F64, "");
    let i32_ty = world.number(NumberKind::I32, "");
    let row_ty = world.ty(Ty::Array { element: f64_ty, dimensions: 1 });
    let grid_ty = world.ty(Ty::Array { element: f64_ty, dimensions: 2 });
    let error_ty = world.ty(Ty::ErrorValue);
    // Two rows of 1,000 f64: each row is a copy of 16 + 24,000 bytes, which 100 bytes of room cannot hold.
    let rows = crate::convert::grid(vec![Value::array(vec![Value::F64(0.0); 1000]); 2], 2).unwrap();
    let mut host = FakeHost::new();
    host.room = 100;
    let get = Case::new(&world, vec![(rows.clone(), grid_ty), (Value::I32(0), i32_ty), (Value::NONE, error_ty)], vec![row_ty, error_ty]);
    assert!(matches!(get.run_with(Array, "get", &mut host).0, Err(Fault { code: FaultCode::Memory, .. })));
    let push = Case::new(&world, vec![(rows, grid_ty), (Value::array(Vec::new()), row_ty)], vec![row_ty]);
    assert!(matches!(push.run_with(Array, "push", &mut host).0, Err(Fault { code: FaultCode::Memory, .. })));
}
