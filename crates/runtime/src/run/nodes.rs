//! The interpreter (`docs/architecture.md`, 10.5): each node by its kind (`docs/ir.md`, section 4), its skip (section 5), and
//! the limits of cli.md 6.2 as it goes. A structure is a task that starts the instances of its regions and waits for them.

use std::rc::Rc;
use std::sync::Arc;

use labcraft_check::PrimitiveLibrary;
use labcraft_check::primitives::primitive;
use labcraft_ir::{Body, ConstId, ConstValue, FunctionId, Node, NodeKind, Pattern, Region, Skip};
use labcraft_types::{Ty, TyId};

use super::machine::{Clause, ForState, Halt, InstanceId, LoopLink, LoopState, Machine, Parent, State, Task, memory_fault};
use super::position::{Pos, child};
use super::{Caller, SetBy};
use crate::convert::{Env, constant, grid, integer};
use crate::fault::{Fault, FaultCode};
use crate::primitives::{self, Call, Host, Outcome, outputs_on_error};
use crate::value::{Value, VariantValue};

/// A host that only answers `room`, for the outputs of a node that skips or fails.
pub(super) struct Quiet {
    pub(super) room: u64,
}

impl Host for Quiet {
    fn now(&mut self) -> i64 {
        0
    }
    fn element(&mut self, _: labcraft_ir::ElementId) -> Result<Value, Fault> {
        Err(Fault::bug("a panel read while a node skips"))
    }
    fn set_element(&mut self, _: labcraft_ir::ElementId, _: Value) -> Result<(), Fault> {
        Err(Fault::bug("a panel written while a node skips"))
    }
    fn room(&self) -> u64 {
        self.room
    }
    fn error(&self, category: &str, code: i64, message: String) -> Value {
        Value::error(crate::value::ErrorData {
            category: Arc::from(category),
            code,
            message: Arc::from(message),
            origin: None,
            others: Arc::default(),
        })
    }
}

/// What a primitive sees of the run, at its node.
struct NodeHost<'m, 'p, 'o> {
    machine: &'m mut Machine<'p, 'o>,
    instance: InstanceId,
    node: u32,
    frame: usize,
}

impl Host for NodeHost<'_, '_, '_> {
    fn now(&mut self) -> i64 {
        self.machine.now
    }
    fn element(&mut self, element: labcraft_ir::ElementId) -> Result<Value, Fault> {
        self.machine.element(self.frame, element)
    }
    fn set_element(&mut self, element: labcraft_ir::ElementId, value: Value) -> Result<(), Fault> {
        self.machine.set_element(self.frame, element, value, SetBy::Program)
    }
    fn room(&self) -> u64 {
        self.machine.memory.room()
    }
    fn error(&self, category: &str, code: i64, message: String) -> Value {
        // `error_at` caches the callers of the frame, which needs `&mut`; an error made here is rare enough to build them.
        let Some(found) = self.machine.instance(self.instance) else { return Quiet { room: 0 }.error(category, code, message) };
        let Some(frame) = self.machine.frame(found.frame) else { return Quiet { room: 0 }.error(category, code, message) };
        let Some(node) = found.region.nodes.get(self.node as usize) else { return Quiet { room: 0 }.error(category, code, message) };
        let place = self.machine.place(frame.function, node);
        let callers = self.machine.callers_now(found.frame);
        Value::error(crate::value::ErrorData {
            category: Arc::from(category),
            code,
            message: Arc::from(message),
            origin: Some(Arc::new(crate::value::Origin { place, callers })),
            others: Arc::default(),
        })
    }
}

/// An array from its items, in the type of what it is: a grid for an `[T; 2]`, whose rows must have one length.
pub(super) fn array_of(env: Env<'_>, items: Vec<Value>, ty: TyId) -> Result<Value, Fault> {
    match env.ty(ty) {
        Ty::Array { dimensions, .. } if *dimensions >= 2 => grid(items, usize::from(*dimensions))
            .ok_or_else(|| Fault::out_of_range("the rows of an array of two dimensions or more must have the same length (spec 3.2)")),
        _ => Ok(Value::array(items)),
    }
}

/// A node whose inputs are all there starts.
pub(super) fn start(machine: &mut Machine<'_, '_>, instance: InstanceId, index: u32, pos: Pos) -> Result<(), Halt> {
    let Some(found) = machine.instance(instance) else { return Ok(()) };
    let region = found.region;
    let Some(node) = region.nodes.get(index as usize) else { return Err(machine.halt(instance, index, Fault::bug("no such node"))) };
    if let Some(limit) = machine.settings.max_nodes
        && machine.nodes_run >= limit
    {
        let fault = Fault::new(FaultCode::Nodes, format!("the run has run its limit of {limit} nodes (--max-nodes)"));
        return Err(machine.halt(instance, index, fault));
    }
    machine.nodes_run += 1;
    let mut inputs = Vec::with_capacity(node.inputs.len());
    for used in &node.inputs {
        match machine.take(instance, used) {
            Ok(value) => inputs.push(value),
            Err(fault) => return Err(machine.halt(instance, index, fault)),
        }
    }
    if let Skip::OnError { err, always: false, .. } = &node.skip
        && inputs.get(*err as usize).is_some_and(|value| value.as_error().is_some())
    {
        let types: Vec<TyId> = node.outputs.iter().map(|output| output.ty).collect();
        let host = Quiet { room: machine.memory.room() };
        let outputs =
            outputs_on_error(machine.env(), &node.skip, &inputs, &types, None, &host).map_err(|fault| machine.halt(instance, index, fault))?;
        return machine.finish(instance, index, outputs, true);
    }
    let result = run_node(machine, instance, index, node, inputs, pos);
    result.map_err(|fault| machine.halt(instance, index, fault)).and_then(|halt| match halt {
        Some(halt) => Err(halt),
        None => Ok(()),
    })
}

/// Runs a node by its kind. `Ok(Some)` is a fault already located elsewhere.
fn run_node<'p>(
    machine: &mut Machine<'p, '_>,
    instance: InstanceId,
    index: u32,
    node: &'p Node,
    inputs: Vec<Value>,
    pos: Pos,
) -> Result<Option<Halt>, Fault> {
    let env = machine.env();
    let located = |result: Result<(), Halt>| -> Result<Option<Halt>, Fault> { Ok(result.err()) };
    match &node.kind {
        NodeKind::Constant => located(machine.finish(instance, index, inputs, false)),
        NodeKind::Construct { ty, variant } => {
            let value = match (env.ty(*ty), variant) {
                (Ty::Enum(_), Some(variant)) => Value::Variant(Arc::new(VariantValue { index: *variant, values: inputs.into() })),
                (Ty::Struct(_) | Ty::Waveform(_), None) => Value::Struct(inputs.into()),
                _ => return Err(Fault::bug("a construction of a type that is neither a struct nor an enumeration")),
            };
            located(machine.finish(instance, index, vec![value], false))
        }
        NodeKind::BuildArray => {
            let ty = node.outputs.first().map(|output| output.ty).ok_or_else(|| Fault::bug("an array node without its output"))?;
            room(machine, &inputs)?;
            let value = array_of(env, inputs, ty)?;
            located(machine.finish(instance, index, vec![value], false))
        }
        NodeKind::Primitive { library, name, statics, variadic } => {
            let function = frame_function(machine, instance)?;
            let viewer =
                machine.program.functions.get(function.0 as usize).map(|function| function.library).ok_or_else(|| Fault::bug("no function"))?;
            let call = Call {
                env,
                viewer,
                input_types: node.inputs.iter().map(|used| used.ty).collect(),
                output_types: node.outputs.iter().map(|output| output.ty).collect(),
                inputs,
                statics,
                variadic: *variadic,
            };
            primitive_call(machine, instance, index, pos, *library, machine.names.resolve(*name), call, &node.skip)
        }
        NodeKind::Native { function, variadic } => native(machine, instance, index, pos, *function, inputs, *variadic, &node.skip),
        NodeKind::Call { function, variadic } => located(call_function(machine, instance, index, pos, *function, gathered(inputs, *variadic)?)),
        NodeKind::Loop { continue_on_error, carries, region, .. } => {
            let carries = *carries as usize;
            if inputs.len() < carries {
                return Err(Fault::bug("a loop without its carries"));
            }
            let mut inputs = inputs;
            let passed = inputs.split_off(carries);
            hold_all(machine, &passed)?;
            let state = LoopState { region, iteration: 0, carries, passed, collects: Vec::new(), continue_on_error: *continue_on_error };
            let task = machine.new_task(Task { instance, node: index, pos, state: State::Loop(state) });
            located(start_iteration(machine, task, inputs))
        }
        NodeKind::For { parallel, clauses, carries, region, collects } => {
            let mut values = inputs.into_iter();
            let mut read = Vec::with_capacity(clauses.len());
            let mut count = u64::MAX;
            for (at, clause) in clauses.iter().enumerate() {
                match clause {
                    labcraft_ir::Clause::Each => {
                        let array = values.next().ok_or_else(|| Fault::bug("a `for` without its array"))?;
                        let length = match &array {
                            Value::Array(items) => items.len(),
                            Value::Grid(grid) => grid.rows(),
                            _ => return Err(Fault::bug("a `for` over something that is no array")),
                        };
                        count = count.min(u64::try_from(length).unwrap_or(u64::MAX));
                        read.push(Clause::Each(array));
                    }
                    labcraft_ir::Clause::Range => {
                        let (Some(low), Some(high)) = (values.next(), values.next()) else { return Err(Fault::bug("a range without its bounds")) };
                        let (Some(low), Some(high)) = (integer_of(&low), integer_of(&high)) else {
                            return Err(Fault::bug("a range whose bounds are not integers"));
                        };
                        let ty = region.args.get(at).map(|arg| arg.ty).ok_or_else(|| Fault::bug("a range without its index"))?;
                        let kind = env.number_kind(ty).filter(|kind| kind.is_integer()).ok_or_else(|| Fault::bug("an index that is no integer"))?;
                        count = count.min(u64::try_from(high.saturating_sub(low).max(0)).unwrap_or(u64::MAX));
                        read.push(Clause::Range { low, kind });
                    }
                }
            }
            let rest: Vec<Value> = values.collect();
            let carries = *carries as usize;
            if rest.len() < carries {
                return Err(Fault::bug("a `for` without its carries"));
            }
            let mut carried = rest;
            let passed = carried.split_off(carries);
            for clause in &read {
                if let Clause::Each(array) = clause {
                    hold_all(machine, std::slice::from_ref(array))?;
                }
            }
            hold_all(machine, &passed)?;
            let collects = vec![Vec::new(); *collects as usize];
            let state = ForState {
                region,
                parallel: *parallel,
                iteration: 0,
                ended: 0,
                count: if count == u64::MAX { 0 } else { count },
                clauses: read,
                carry_count: carried.len(),
                carries: carried,
                passed,
                collects,
            };
            let task = machine.new_task(Task { instance, node: index, pos, state: State::For(state) });
            located(start_for(machine, task))
        }
        NodeKind::Match { arms } | NodeKind::Handle { arms } => {
            let mut inputs = inputs.into_iter();
            let selector = inputs.next().ok_or_else(|| Fault::bug("a `match` without its selector"))?;
            let passed: Vec<Value> = inputs.collect();
            let mut taken = None;
            'arms: for (at, arm) in arms.iter().enumerate() {
                for pattern in &arm.patterns {
                    if let Some(binds) = matches(env, &selector, pattern)? {
                        taken = Some((at, arm, binds));
                        break 'arms;
                    }
                }
            }
            let Some((at, arm, mut args)) = taken else { return Err(Fault::bug("a value that no arm of its `match` takes")) };
            let found = machine.instance(instance).ok_or_else(|| Fault::bug("an arm of a region that has ended"))?;
            let (frame, iter, loops) = (found.frame, Rc::clone(&found.iter), found.loops.clone());
            let number = machine.frame(frame).map_or(0, |frame| frame.number);
            machine.observer.arm(machine.now, number, node, &iter, at);
            args.extend(passed);
            let task = machine.new_task(Task { instance, node: index, pos: Rc::clone(&pos), state: State::Arm });
            machine.new_instance(&arm.region, args, frame, pos, iter, loops, Parent::Task(task))?;
            Ok(None)
        }
    }
}

fn frame_function(machine: &Machine<'_, '_>, instance: InstanceId) -> Result<FunctionId, Fault> {
    machine.instance(instance).and_then(|found| machine.frame(found.frame)).map(|frame| frame.function).ok_or_else(|| Fault::bug("a node of no call"))
}

/// The room a node needs for a new value of these items.
fn room(machine: &Machine<'_, '_>, items: &[Value]) -> Result<(), Fault> {
    let size = items.iter().fold(crate::value::VALUE_COST, |total, item| total.saturating_add(crate::value::logical_size(item)));
    if size > machine.memory.room() {
        return Err(memory_fault("this array", size));
    }
    Ok(())
}

fn hold_all(machine: &mut Machine<'_, '_>, values: &[Value]) -> Result<(), Fault> {
    for (at, value) in values.iter().enumerate() {
        if !machine.memory.hold(value) {
            for held in values.iter().take(at) {
                machine.memory.release(held);
            }
            return Err(memory_fault("what a loop keeps", crate::value::logical_size(value)));
        }
    }
    Ok(())
}

fn release_all(machine: &mut Machine<'_, '_>, values: &[Value]) {
    for value in values {
        machine.memory.release(value);
    }
}

fn integer_of(value: &Value) -> Option<i128> {
    Some(match value {
        Value::I8(x) => i128::from(*x),
        Value::I16(x) => i128::from(*x),
        Value::I32(x) => i128::from(*x),
        Value::I64(x) => i128::from(*x),
        Value::U8(x) => i128::from(*x),
        Value::U16(x) => i128::from(*x),
        Value::U32(x) => i128::from(*x),
        Value::U64(x) => i128::from(*x),
        _ => return None,
    })
}

/// Whether the selector takes this pattern, and what it binds.
fn matches(env: Env<'_>, selector: &Value, pattern: &Pattern) -> Result<Option<Vec<Value>>, Fault> {
    Ok(match pattern {
        Pattern::Literal(id) => {
            let literal = constant(env, *id)?;
            primitives::equal(env, selector, &literal)?.then(Vec::new)
        }
        Pattern::Range { low, high, inclusive } => {
            let value = integer_of(selector).ok_or_else(|| Fault::bug("a range pattern on a value that is no integer"))?;
            let inside = value >= *low && if *inclusive { value <= *high } else { value < *high };
            inside.then(Vec::new)
        }
        Pattern::Variant { variant, binds } => match selector {
            Value::Variant(found) if found.index == *variant => Some(found.values.iter().take(*binds as usize).cloned().collect()),
            Value::Variant(_) => None,
            _ => return Err(Fault::bug("a variant pattern on a value that is no variant")),
        },
        Pattern::Category { category, wildcard, binds } => match selector.as_error() {
            Some(error) => {
                let same = if *wildcard {
                    error.category.strip_prefix(&**category).is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
                } else {
                    *error.category == **category
                };
                same.then(|| if *binds { vec![selector.clone()] } else { Vec::new() })
            }
            None => None,
        },
        Pattern::Code { code, binds } => match selector.as_error() {
            Some(error) if i128::from(error.code) == *code => Some(if *binds { vec![selector.clone()] } else { Vec::new() }),
            _ => None,
        },
        Pattern::Ok => selector.as_error().is_none().then(Vec::new),
        Pattern::Wildcard { binds } => Some(if *binds { vec![selector.clone()] } else { Vec::new() }),
    })
}

/// A primitive on one call, and what its outcome makes of the node: outputs, outputs of a failure, a wait, or a call of the
/// function it was given (`core.call`).
#[allow(clippy::too_many_arguments)]
fn primitive_call<'p>(
    machine: &mut Machine<'p, '_>,
    instance: InstanceId,
    index: u32,
    pos: Pos,
    library: PrimitiveLibrary,
    name: &str,
    mut call: Call<'p>,
    skip: &Skip,
) -> Result<Option<Halt>, Fault> {
    let frame = machine.instance(instance).map(|found| found.frame).ok_or_else(|| Fault::bug("a node of a region that has ended"))?;
    let outcome = {
        let mut host = NodeHost { machine: &mut *machine, instance, node: index, frame };
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| primitives::run(library, name, &mut call, &mut host)))
            .map_err(|_| Fault::bug(format!("the primitive {}.{name} panicked; this is a bug of LabCraft", library.as_str())))??
    };
    match outcome {
        Outcome::Done(values) => Ok(machine.finish(instance, index, values, false).err()),
        Outcome::Fail(error) => {
            let host = Quiet { room: machine.memory.room() };
            let outputs = outputs_on_error(machine.env(), skip, &call.inputs, &call.output_types, Some(error), &host)?;
            Ok(machine.finish(instance, index, outputs, false).err())
        }
        Outcome::Wait(ns) => {
            let task = machine.new_task(Task { instance, node: index, pos, state: State::Wait });
            machine.sleep(task, ns)?;
            Ok(None)
        }
        Outcome::Call { function, inputs } => {
            let Value::Function(id) = function else { return Err(Fault::bug("`core.call` of a value that is no function")) };
            reference(machine, instance, index, pos, id, inputs, call.input_types.split_off(1.min(call.input_types.len())), call.output_types)
        }
    }
}

/// A call of a function reference (`core.call`): the skip of the function it designates, then the function (`docs/ir.md`,
/// section 5). Its inputs and their types are in the places of the function.
#[allow(clippy::too_many_arguments)]
fn reference<'p>(
    machine: &mut Machine<'p, '_>,
    instance: InstanceId,
    index: u32,
    pos: Pos,
    id: ConstId,
    inputs: Vec<Value>,
    input_types: Vec<TyId>,
    output_types: Vec<TyId>,
) -> Result<Option<Halt>, Fault> {
    let program = machine.program;
    let found = program.constants.get(id.0 as usize).ok_or_else(|| Fault::bug("a reference to no constant"))?;
    match &found.value {
        ConstValue::Function(function) => {
            let callee = program.functions.get(function.0 as usize).ok_or_else(|| Fault::bug("a reference to no function"))?;
            match &callee.body {
                Body::Region(_) => Ok(call_function(machine, instance, index, pos, *function, inputs).err()),
                Body::Native { skip, .. } => {
                    if skips(skip, &inputs) {
                        let host = Quiet { room: machine.memory.room() };
                        let outputs = outputs_on_error(machine.env(), skip, &inputs, &output_types, None, &host)?;
                        return Ok(machine.finish(instance, index, outputs, true).err());
                    }
                    native_body(machine, instance, index, pos, *function, inputs.clone(), (inputs, skip))
                }
            }
        }
        ConstValue::Primitive { library, name, skip } => {
            if skips(skip, &inputs) {
                let host = Quiet { room: machine.memory.room() };
                let outputs = outputs_on_error(machine.env(), skip, &inputs, &output_types, None, &host)?;
                return Ok(machine.finish(instance, index, outputs, true).err());
            }
            let text = machine.names.resolve(*name);
            let (inputs, input_types, variadic) = spread(machine.env(), *library, text, inputs, input_types)?;
            let function = frame_function(machine, instance)?;
            let viewer = program.functions.get(function.0 as usize).map(|function| function.library).ok_or_else(|| Fault::bug("no function"))?;
            let call = Call { env: machine.env(), viewer, inputs, input_types, output_types, statics: &[], variadic };
            primitive_call(machine, instance, index, pos, *library, text, call, skip)
        }
        ConstValue::InvalidFunction { defaults } => invalid_function(machine, instance, index, defaults, &inputs, &input_types, &output_types),
        _ => Err(Fault::bug("a function reference that designates no function")),
    }
}

fn skips(skip: &Skip, inputs: &[Value]) -> bool {
    matches!(skip, Skip::OnError { err, always: false, .. } if inputs.get(*err as usize).is_some_and(|value| value.as_error().is_some()))
}

/// The inputs of a primitive, their types, and where its variadic parameter sits among them.
type Spread = (Vec<Value>, Vec<TyId>, Option<labcraft_ir::Variadic>);

/// A variadic parameter of a primitive called through a reference comes as one array: its items become inputs again.
fn spread(env: Env<'_>, library: PrimitiveLibrary, name: &str, mut inputs: Vec<Value>, mut input_types: Vec<TyId>) -> Result<Spread, Fault> {
    let Some(signature) = primitive(library, name) else { return Err(Fault::bug("a primitive that the table does not have")) };
    let Some(at) = signature.params.iter().position(|param| param.variadic) else { return Ok((inputs, input_types, None)) };
    if at >= inputs.len() {
        return Err(Fault::bug("a variadic parameter beyond the inputs"));
    }
    let array = inputs.remove(at);
    let array_ty = input_types.remove(at);
    let element = match env.ty(array_ty) {
        Ty::Array { element, dimensions: 1 } => *element,
        _ => return Err(Fault::bug("a variadic parameter that is no array")),
    };
    let items: Vec<Value> = match array {
        Value::Array(items) => Arc::unwrap_or_clone(items).into_items(),
        _ => return Err(Fault::bug("a variadic parameter that is no array")),
    };
    let count = u32::try_from(items.len()).map_err(|_| Fault::bug("a variadic parameter of more than 2^32 items"))?;
    let tail = inputs.split_off(at);
    let tail_types = input_types.split_off(at);
    inputs.extend(items);
    inputs.extend(tail);
    input_types.extend(std::iter::repeat_n(element, count as usize));
    input_types.extend(tail_types);
    let start = u32::try_from(at).unwrap_or(u32::MAX);
    Ok((inputs, input_types, Some(labcraft_ir::Variadic { start, count })))
}

/// A call of the default of a function type (Q-21, decision of the owner, 2026-11-10): it runs nothing. Its outputs take the
/// defaults of their types, and those of type `error` the first error it received on an input of type `error`, or else a new
/// one, `core.invalid_function`. A type with no output of type `error` cannot say it: `LC8008`.
fn invalid_function(
    machine: &mut Machine<'_, '_>,
    instance: InstanceId,
    index: u32,
    defaults: &[ConstId],
    inputs: &[Value],
    input_types: &[TyId],
    output_types: &[TyId],
) -> Result<Option<Halt>, Fault> {
    let received =
        inputs.iter().zip(input_types).find(|(value, ty)| machine.is_error_type(**ty) && value.as_error().is_some()).map(|(value, _)| value.clone());
    if !output_types.iter().any(|ty| machine.is_error_type(*ty)) {
        return Err(Fault::out_of_range(
            "a call of a function reference that designates no function (the default of its type, Q-21), whose type has no output of type `error` to say it",
        ));
    }
    let skipped = received.is_some();
    let error = match received {
        Some(error) => error,
        None => machine.error_at(
            instance,
            index,
            "core.invalid_function",
            0,
            "this function reference designates no function: it is the default of its type (Q-21)".to_owned(),
        ),
    };
    let mut outputs = Vec::with_capacity(output_types.len());
    for (ty, default) in output_types.iter().zip(defaults) {
        outputs.push(if machine.is_error_type(*ty) { error.clone() } else { constant(machine.env(), *default)? });
    }
    if outputs.len() != output_types.len() {
        return Err(Fault::bug("the default of a function type without a default for each of its outputs"));
    }
    Ok(machine.finish(instance, index, outputs, skipped).err())
}

/// The inputs of a node in the places of the signature: the arguments of a variadic parameter as one array (spec 4).
fn gathered(mut inputs: Vec<Value>, variadic: Option<labcraft_ir::Variadic>) -> Result<Vec<Value>, Fault> {
    let Some(labcraft_ir::Variadic { start, count }) = variadic else { return Ok(inputs) };
    let start = start as usize;
    let end = start.saturating_add(count as usize);
    if end > inputs.len() {
        return Err(Fault::bug("a variadic parameter beyond the inputs"));
    }
    let items: Vec<Value> = inputs.drain(start..end).collect();
    inputs.insert(start, Value::array(items));
    Ok(inputs)
}

/// A node of an `extern` function: the call goes to its library, its variadic parameter as one array (spec 4).
#[allow(clippy::too_many_arguments)]
fn native<'p>(
    machine: &mut Machine<'p, '_>,
    instance: InstanceId,
    index: u32,
    pos: Pos,
    function: FunctionId,
    inputs: Vec<Value>,
    variadic: Option<labcraft_ir::Variadic>,
    skip: &'p Skip,
) -> Result<Option<Halt>, Fault> {
    let call_inputs = gathered(inputs.clone(), variadic)?;
    native_body(machine, instance, index, pos, function, call_inputs, (inputs, skip))
}

fn native_body<'p>(
    machine: &mut Machine<'p, '_>,
    instance: InstanceId,
    index: u32,
    pos: Pos,
    function: FunctionId,
    inputs: Vec<Value>,
    failed: (Vec<Value>, &'p Skip),
) -> Result<Option<Halt>, Fault> {
    let program = machine.program;
    let callee = program.functions.get(function.0 as usize).ok_or_else(|| Fault::bug("a call of no function"))?;
    let Body::Native { library, function: name, runs, .. } = &callee.body else {
        return Err(Fault::bug("a native call of a function written in LabCraft"));
    };
    if !*runs {
        let (pass, skip) = failed;
        if !matches!(skip, Skip::OnError { .. }) {
            return Err(Fault::out_of_range(format!("`{}` cannot run in M0 (spec 4.1), and has no `err` to say it", callee.shown)));
        }
        let error = machine.error_at(
            instance,
            index,
            "extern.unavailable",
            0,
            format!("`{}` is an external function, which M0 does not run (spec 4.1)", callee.shown),
        );
        let types = machine.output_types(instance, index);
        let host = Quiet { room: machine.memory.room() };
        let outputs = outputs_on_error(machine.env(), skip, &pass, &types, Some(error), &host)?;
        return Ok(machine.finish(instance, index, outputs, false).err());
    }
    let Some(slot) = machine.native_of.get(library).copied() else {
        return Err(Fault::bug(format!("no native library runs `{}`", callee.shown)));
    };
    let name = machine.names.resolve(*name);
    Ok(machine.start_native(instance, index, pos, slot, name, inputs, failed).err())
}

/// A call of a function written in LabCraft: a frame one deeper, whose body runs as an instance.
fn call_function<'p>(
    machine: &mut Machine<'p, '_>,
    instance: InstanceId,
    index: u32,
    pos: Pos,
    function: FunctionId,
    inputs: Vec<Value>,
) -> Result<(), Halt> {
    let program = machine.program;
    let fail = |machine: &Machine<'p, '_>, fault: Fault| machine.halt(instance, index, fault);
    let Some(callee) = program.functions.get(function.0 as usize) else { return Err(fail(machine, Fault::bug("a call of no function"))) };
    let Body::Region(region) = &callee.body else {
        return Err(fail(machine, Fault::bug("a call of an `extern` function as one written in LabCraft")));
    };
    let Some(found) = machine.instance(instance) else { return Err(fail(machine, Fault::bug("a call from a region that has ended"))) };
    let (parent, iter, loops) = (found.frame, Rc::clone(&found.iter), found.loops.clone());
    let Some(caller) = machine.frame(parent) else { return Err(fail(machine, Fault::bug("a call from no frame"))) };
    let (depth, caller_function, caller_number) = (caller.depth.saturating_add(1), caller.function, caller.number);
    if depth > machine.settings.max_call_depth {
        let message = format!(
            "the call of `{}` goes {depth} calls deep, past the limit of {} (--max-call-depth): {}",
            callee.shown,
            machine.settings.max_call_depth,
            chain(machine, parent, &callee.shown)
        );
        return Err(fail(machine, Fault::new(FaultCode::Depth, message)));
    }
    let Some(node) = found_node(machine, instance, index) else { return Err(fail(machine, Fault::bug("no such node"))) };
    let site = machine.place(caller_function, node);
    let frame = machine.new_frame(function, Some(parent), Some(site), depth, &inputs).map_err(|fault| fail(machine, fault))?;
    let number = machine.frame(frame).map_or(0, |frame| frame.number);
    machine.observer.call(machine.now, number, Some(Caller { frame: caller_number, node, iter: &iter }), function, &inputs);
    let task = machine.new_task(Task { instance, node: index, pos: Rc::clone(&pos), state: State::Call { frame } });
    machine.new_instance(region, inputs, frame, pos, Rc::from([]), loops, Parent::Task(task)).map_err(|fault| fail(machine, fault))?;
    Ok(())
}

fn found_node<'p>(machine: &Machine<'p, '_>, instance: InstanceId, index: u32) -> Option<&'p Node> {
    machine.instance(instance).and_then(|found| found.region.nodes.get(index as usize))
}

/// The chain of calls to a frame, shortened to its first and last three.
fn chain(machine: &Machine<'_, '_>, frame: usize, callee: &str) -> String {
    let mut names = vec![callee.to_owned()];
    let mut at = Some(frame);
    while let Some(id) = at {
        let Some(found) = machine.frame(id) else { break };
        names.push(machine.program.functions.get(found.function.0 as usize).map_or(String::new(), |function| function.shown.to_string()));
        at = machine.frame_parent(id);
    }
    names.reverse();
    if names.len() > 6 {
        let skipped = names.len() - 6;
        let tail = names.split_off(names.len() - 3);
        names.truncate(3);
        names.push(format!("… {skipped} more …"));
        names.extend(tail);
    }
    names.join(" → ")
}

/// Starts the next iteration of a `loop`, on its carries: past `--max-iterations`, `LC8001`.
fn start_iteration(machine: &mut Machine<'_, '_>, task: usize, carries: Vec<Value>) -> Result<(), Halt> {
    let Some(Task { instance, node, pos, state: State::Loop(state) }) = machine.task(task) else { return Ok(()) };
    let (instance, index, pos, iteration, region) = (*instance, *node, Rc::clone(pos), state.iteration, state.region);
    if let Some(limit) = machine.settings.max_iterations
        && iteration >= limit
    {
        let fault = Fault::new(
            FaultCode::Iterations,
            format!("the loop reached its limit of {limit} iterations (--max-iterations), at iteration {}", iteration.saturating_add(1)),
        );
        let mut halt = machine.halt(instance, index, fault);
        halt.iteration = Some(iteration);
        return Err(halt);
    }
    let mut args = carries;
    if let Some(Task { state: State::Loop(state), .. }) = machine.task(task) {
        args.extend(state.passed.iter().cloned());
    }
    iteration_instance(machine, task, instance, index, &pos, region, iteration, args)
}

#[allow(clippy::too_many_arguments)]
fn iteration_instance<'p>(
    machine: &mut Machine<'p, '_>,
    task: usize,
    instance: InstanceId,
    index: u32,
    pos: &Pos,
    region: &'p Region,
    iteration: u64,
    args: Vec<Value>,
) -> Result<(), Halt> {
    let Some(found) = machine.instance(instance) else { return Err(machine.halt(instance, index, Fault::bug("a loop of a region that has ended"))) };
    let frame = found.frame;
    let mut iter: Vec<u64> = found.iter.to_vec();
    iter.push(iteration);
    let function = machine.frame(frame).map_or(FunctionId(0), |frame| frame.function);
    let Some(name) = found.region.nodes.get(index as usize).map(|node| node.name) else {
        return Err(machine.halt(instance, index, Fault::bug("no such node")));
    };
    let loops = Some(Rc::new(LoopLink { function, node: name, iteration, parent: found.loops.clone() }));
    // The first arguments of a `for` are its clauses: an element of an array that the loop counts, an index, or a row of a
    // grid, which is a copy and is counted. The elements of arrays before any other clause are left out of the count.
    let free = match machine.task(task).map(|task| &task.state) {
        Some(State::For(state)) => state.clauses.iter().take_while(|clause| matches!(clause, Clause::Each(Value::Array(_)))).count(),
        _ => 0,
    };
    machine
        .new_instance_with(region, args, free, frame, child(pos, iteration), iter.into(), loops, Parent::Task(task))
        .map_err(|fault| machine.halt(instance, index, fault))?;
    Ok(())
}

/// The arguments of iteration `i` of a `for`: an element or an index for each clause, then the carries, then what is passed in.
fn for_args(state: &ForState<'_>, iteration: u64, carries: Vec<Value>) -> Result<Vec<Value>, Fault> {
    let at = usize::try_from(iteration).map_err(|_| Fault::bug("an iteration beyond the memory"))?;
    let mut args = Vec::with_capacity(state.region.args.len());
    for clause in &state.clauses {
        args.push(match clause {
            Clause::Each(Value::Array(items)) => items.items().get(at).cloned().ok_or_else(|| Fault::bug("an iteration past its array"))?,
            Clause::Each(Value::Grid(grid)) => grid.row(at).ok_or_else(|| Fault::bug("an iteration past its array"))?,
            Clause::Each(_) => return Err(Fault::bug("a `for` over something that is no array")),
            Clause::Range { low, kind } => {
                let value = low.checked_add(i128::from(iteration)).ok_or_else(|| Fault::bug("an index past the range of an i128"))?;
                integer(*kind, value).ok_or_else(|| Fault::bug("an index outside the type of its range"))?
            }
        });
    }
    args.extend(carries);
    args.extend(state.passed.iter().cloned());
    Ok(args)
}

/// Starts a `for`: its first iteration, or every iteration at once for a parallel one, each at its own pace (spec 6.2, 8).
fn start_for(machine: &mut Machine<'_, '_>, task: usize) -> Result<(), Halt> {
    let Some(Task { instance, node, pos, state: State::For(state) }) = machine.task(task) else { return Ok(()) };
    let (instance, index, pos, count, parallel, region) = (*instance, *node, Rc::clone(pos), state.count, state.parallel, state.region);
    if count == 0 {
        return end_for(machine, task);
    }
    let starts = if parallel { count } else { 1 };
    for iteration in 0..starts {
        let args = match machine.task_mut(task) {
            Some(Task { state: State::For(state), .. }) => {
                let carries = std::mem::take(&mut state.carries);
                state.iteration = iteration.saturating_add(1);
                for_args(state, iteration, carries)
            }
            _ => return Ok(()),
        };
        let args = args.map_err(|fault| machine.halt(instance, index, fault))?;
        iteration_instance(machine, task, instance, index, &pos, region, iteration, args)?;
    }
    Ok(())
}

/// A `for` ends: its carries, then an array of each `collect`, in the order of the iterations.
fn end_for(machine: &mut Machine<'_, '_>, task: usize) -> Result<(), Halt> {
    let Some(Task { instance, node, state: State::For(state), .. }) = machine.take_task(task) else { return Ok(()) };
    for clause in &state.clauses {
        if let Clause::Each(array) = clause {
            machine.memory.release(array);
        }
    }
    release_all(machine, &state.passed);
    let mut outputs = state.carries;
    for collect in state.collects {
        let items: Vec<Value> = collect.into_iter().flatten().collect();
        release_all(machine, &items);
        outputs.push(Value::array(items));
    }
    machine.finish(instance, node, outputs, false)
}

/// An instance whose nodes have all ended gives its results to what started it.
pub(super) fn complete(machine: &mut Machine<'_, '_>, id: InstanceId) -> Result<(), Halt> {
    let Some(found) = machine.instance(id) else { return Ok(()) };
    let iteration = found.iter.last().copied().unwrap_or(0);
    let parent = found.parent;
    let located = match parent {
        Parent::Task(task) => machine.task(task).map(|task| (task.instance, task.node)),
        Parent::Top => None,
    };
    let results = match machine.results(id) {
        Ok((results, _)) => results,
        Err(fault) => {
            return Err(match located {
                Some((instance, node)) => machine.halt(instance, node, fault),
                None => machine.halt_at(None, fault),
            });
        }
    };
    let Parent::Task(task) = parent else {
        machine.end_frame(0);
        machine.finished(results);
        return Ok(());
    };
    let Some((instance, node)) = located else { return Ok(()) };
    match machine.task(task).map(|task| &task.state) {
        Some(State::Call { frame }) => {
            let frame = *frame;
            machine.take_task(task);
            machine.end_frame(frame);
            machine.finish(instance, node, results, false)
        }
        Some(State::Arm) => {
            machine.take_task(task);
            machine.finish(instance, node, results, false)
        }
        Some(State::Loop(_)) => end_iteration(machine, task, instance, node, results),
        Some(State::For(_)) => end_for_iteration(machine, task, instance, node, iteration, results),
        _ => Err(machine.halt(instance, node, Fault::bug("a region that ends for no structure"))),
    }
}

/// An iteration of a `for` ended: its results are the next carries, then its collects, kept by iteration.
fn end_for_iteration(
    machine: &mut Machine<'_, '_>,
    task: usize,
    instance: InstanceId,
    node: u32,
    iteration: u64,
    results: Vec<Value>,
) -> Result<(), Halt> {
    let Some(Task { state: State::For(state), .. }) = machine.task(task) else { return Ok(()) };
    let carries = state.carries_count();
    if results.len() < carries {
        return Err(machine.halt(instance, node, Fault::bug("an iteration of a `for` without its carries")));
    }
    let mut next = results;
    let collected = next.split_off(carries);
    hold_all(machine, &collected).map_err(|fault| machine.halt(instance, node, fault))?;
    let at = usize::try_from(iteration).unwrap_or(usize::MAX);
    let (more, parallel) = match machine.task_mut(task) {
        Some(Task { state: State::For(state), .. }) => {
            state.ended = state.ended.saturating_add(1);
            for (collect, value) in state.collects.iter_mut().zip(collected) {
                if collect.len() <= at {
                    collect.resize(at.saturating_add(1), None);
                }
                if let Some(place) = collect.get_mut(at) {
                    *place = Some(value);
                }
            }
            if !state.parallel {
                state.carries = next;
            }
            (state.ended < state.count, state.parallel)
        }
        _ => return Ok(()),
    };
    if !more {
        return end_for(machine, task);
    }
    if parallel {
        return Ok(());
    }
    let env_task = machine.task_mut(task);
    let Some(Task { instance, node, pos, state: State::For(state) }) = env_task else { return Ok(()) };
    let (instance, index, pos, region, iteration) = (*instance, *node, Rc::clone(pos), state.region, state.iteration);
    let carries = std::mem::take(&mut state.carries);
    state.iteration = iteration.saturating_add(1);
    let args = for_args(state, iteration, carries);
    let args = args.map_err(|fault| machine.halt(instance, index, fault))?;
    iteration_instance(machine, task, instance, index, &pos, region, iteration, args)
}

/// An iteration of a `loop` ended: its results are the next carries, the condition of its `until`, then its collects. It
/// ends on the condition, or on an error in a carry of type `error` unless `continue_on_error` (Q-7).
fn end_iteration(machine: &mut Machine<'_, '_>, task: usize, instance: InstanceId, node: u32, results: Vec<Value>) -> Result<(), Halt> {
    let Some(Task { state: State::Loop(state), .. }) = machine.task(task) else { return Ok(()) };
    let (carries, region, continue_on_error) = (state.carries, state.region, state.continue_on_error);
    if results.len() < carries.saturating_add(1) {
        return Err(machine.halt(instance, node, Fault::bug("an iteration of a loop without its condition")));
    }
    let mut next = results;
    let mut rest = next.split_off(carries);
    let until = rest.remove(0);
    let Value::Bool(until) = until else { return Err(machine.halt(instance, node, Fault::bug("the condition of an `until` that is no boolean"))) };
    let failed = !continue_on_error && next.iter().zip(&region.args).any(|(value, arg)| machine.is_error_type(arg.ty) && value.as_error().is_some());
    hold_all(machine, &rest).map_err(|fault| machine.halt(instance, node, fault))?;
    if let Some(Task { state: State::Loop(state), .. }) = machine.task_mut(task) {
        if state.collects.len() < rest.len() {
            state.collects.resize(rest.len(), Vec::new());
        }
        for (collect, value) in state.collects.iter_mut().zip(rest) {
            collect.push(value);
        }
        state.iteration = state.iteration.saturating_add(1);
    }
    if until || failed {
        let Some(Task { state: State::Loop(state), .. }) = machine.take_task(task) else { return Ok(()) };
        release_all(machine, &state.passed);
        let mut outputs = next;
        let expected = machine.output_types(instance, node).len();
        let mut collects = state.collects;
        collects.resize(expected.saturating_sub(outputs.len()), Vec::new());
        for collect in collects {
            release_all(machine, &collect);
            outputs.push(Value::array(collect));
        }
        return machine.finish(instance, node, outputs, false);
    }
    start_iteration(machine, task, next)
}
