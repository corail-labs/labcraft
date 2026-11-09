//! Lowering (`docs/ir.md`, section 2): from the `Checked` of a run to a [`Program`]. The function the run starts, then every
//! function it can reach, each lowered once, through a work list. A function is walked once to find its shape (`shape`), then
//! its bodies are lowered innermost first (`body`), so nothing recurses on what a file nests.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use labcraft_check::primitives::{ParamKind, primitive};
use labcraft_check::{Checked, CheckedFile, DeclaredFunction, Meaning, PrimitiveLibrary, read_escapes, unit_of};
use labcraft_graph::{Arg, Constant as Written, Decl, Id, Operand, Signature};
use labcraft_types::{Interner, LibraryId, Name, NumberKind, Ty, TyId, TypeTable, Unit};

use crate::constant::literal_number;
use crate::{Body, ConstId, ConstValue, Constant, Element, Function, FunctionId, FunctionKey, Port, PortOf, Program, Skip, VERSION, Value, Variadic};

mod body;
mod shape;

/// What lowering met that `check` should have refused: a bug of LabCraft (`LC8004`), never a diagnostic for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LowerError {
    /// The file being lowered, as reports show it.
    pub file: String,
    pub message: String,
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.file, self.message)
    }
}

/// Lowers the function of the file at `start` in `checked.files`, and every function it reaches. `checked` comes from
/// `check_for_run`, and its run is not refused (`Reach::refuses`). `names` is the interner `check` used: lowering adds
/// the names it gives nodes (`#close`, `@12`).
pub fn lower(checked: &Checked, start: usize, names: &mut Interner) -> Result<Program, LowerError> {
    let mut lowering = Lowering::new(checked, names);
    let file = checked.files.get(start).ok_or_else(|| LowerError { file: String::new(), message: format!("no file at index {start}") })?;
    let name = match &file.file.decl {
        Decl::Function(function) => function.name,
        Decl::Extern(function) => function.name,
        Decl::Type(_) => return Err(lowering.bug(file, "a run starts at a function, and this file declares a type")),
    };
    lowering.function_id(file.library, name, file)?;
    let mut functions = Vec::new();
    while let Some(id) = lowering.queue.pop_front() {
        functions.push(lowering.function(id)?);
    }
    let mut definitions = HashMap::new();
    for library in &checked.libraries {
        for declared in library.types.values() {
            definitions.insert(declared.ty, lowering.definition(declared.ty));
        }
    }
    Ok(Program { version: VERSION, functions, constants: lowering.constants, types: lowering.types, definitions })
}

/// A constant as a key, so that each one is stored once: a float by its bits.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Bool(bool),
    Int(i128),
    Float(u64),
    Complex(u64, u64),
    Text(Box<str>),
    Bytes(Box<[u8]>),
    Duration(i64),
    NoError,
    Array(Vec<ConstId>),
    Struct(Vec<ConstId>),
    Variant(u32, Vec<ConstId>),
    Function(FunctionId),
    Primitive(PrimitiveLibrary, Name),
    InvalidHandle,
    ToTheEnd,
    Unsettled,
}

fn key_of(value: &ConstValue) -> Key {
    match value {
        ConstValue::Bool(value) => Key::Bool(*value),
        ConstValue::Int(value) => Key::Int(*value),
        ConstValue::Float(value) => Key::Float(value.to_bits()),
        ConstValue::Complex { re, im } => Key::Complex(re.to_bits(), im.to_bits()),
        ConstValue::Text(text) => Key::Text(text.clone()),
        ConstValue::Bytes(bytes) => Key::Bytes(bytes.clone()),
        ConstValue::Duration(ns) => Key::Duration(*ns),
        ConstValue::NoError => Key::NoError,
        ConstValue::Array(items) => Key::Array(items.clone()),
        ConstValue::Struct(fields) => Key::Struct(fields.clone()),
        ConstValue::Variant { variant, values } => Key::Variant(*variant, values.clone()),
        ConstValue::Function(function) => Key::Function(*function),
        ConstValue::Primitive { library, name, .. } => Key::Primitive(*library, *name),
        ConstValue::InvalidHandle => Key::InvalidHandle,
        ConstValue::ToTheEnd => Key::ToTheEnd,
        ConstValue::Unsettled => Key::Unsettled,
    }
}

/// A function waiting in the work list, with its signature, made when it was first named: a call reads the defaults of its
/// callee before the callee is lowered.
struct Pending {
    file: usize,
    key: FunctionKey,
    inputs: Vec<Port>,
    outputs: Vec<Port>,
}

pub(crate) struct Lowering<'c> {
    checked: &'c Checked,
    names: &'c mut Interner,
    types: TypeTable,
    constants: Vec<Constant>,
    constant_ids: HashMap<(TyId, Key), ConstId>,
    ids: HashMap<(u32, Name), FunctionId>,
    pending: Vec<Option<Pending>>,
    /// The signature of each function named so far, by its id, which calls read.
    signatures: Vec<(Vec<Port>, Vec<Port>)>,
    queue: VecDeque<FunctionId>,
    /// From the id of a library in the table of types to its index in `Checked::libraries`.
    library_of: HashMap<LibraryId, usize>,
    /// Each literal once for each type it goes to, by its file, its id and that type: `a2 = [a1, a1]` would double the work at
    /// each line.
    literal_ids: HashMap<(usize, Id, TyId), ConstId>,
    /// The default of each type once: a struct of two fields of a struct of two fields… would double the work at each level.
    default_ids: HashMap<TyId, ConstId>,
    /// The outputs of the calls written alone, by the list of types that `check` shares between the calls of one callee (by
    /// its address, which stays while `checked` is borrowed): one list of values for all of them.
    alone_outputs: HashMap<(usize, usize), Arc<[Value]>>,
}

/// The named constants of a function: what an operand that names one stands for, as `[a1, a1]` holds `a1`.
pub(crate) type Literals<'f> = HashMap<Id, &'f Operand>;

impl<'c> Lowering<'c> {
    fn new(checked: &'c Checked, names: &'c mut Interner) -> Self {
        let library_of = checked.libraries.iter().enumerate().map(|(index, library)| (library.id, index)).collect();
        Lowering {
            checked,
            names,
            types: checked.types.clone(),
            constants: Vec::new(),
            constant_ids: HashMap::new(),
            ids: HashMap::new(),
            pending: Vec::new(),
            signatures: Vec::new(),
            queue: VecDeque::new(),
            library_of,
            literal_ids: HashMap::new(),
            default_ids: HashMap::new(),
            alone_outputs: HashMap::new(),
        }
    }

    pub(crate) fn bug(&self, file: &CheckedFile, message: impl Into<String>) -> LowerError {
        LowerError { file: file.shown.clone(), message: message.into() }
    }

    fn declared(&self, library: u32, name: Name) -> Option<&'c DeclaredFunction> {
        self.checked.libraries.get(library as usize).and_then(|library| library.functions.get(&name))
    }

    /// The id of a function, named for the first time: it joins the work list with its signature.
    pub(crate) fn function_id(&mut self, library: u32, name: Name, from: &CheckedFile) -> Result<FunctionId, LowerError> {
        if let Some(id) = self.ids.get(&(library, name)) {
            return Ok(*id);
        }
        let checked = self.checked;
        let index = self.declared(library, name).and_then(|declared| declared.file).map(|file| file as usize);
        let Some((index, file)) = index.and_then(|index| Some((index, checked.files.get(index)?))) else {
            let text = self.names.resolve(name).to_owned();
            return Err(self.bug(from, format!("`{text}` has no file that a run can lower")));
        };
        let id = FunctionId(u32::try_from(self.pending.len()).map_err(|_| self.bug(file, "too many functions"))?);
        self.ids.insert((library, name), id);
        let (inputs, outputs) = self.ports(file, library, name)?;
        self.signatures.push((inputs.clone(), outputs.clone()));
        self.pending.push(Some(Pending { file: index, key: FunctionKey { library, name }, inputs, outputs }));
        self.queue.push_back(id);
        Ok(id)
    }

    pub(crate) fn signature(&self, function: FunctionId) -> Option<&(Vec<Port>, Vec<Port>)> {
        self.signatures.get(function.0 as usize)
    }

    fn signature_of(file: &CheckedFile) -> Option<&Signature> {
        match &file.file.decl {
            Decl::Function(function) => Some(&function.signature),
            Decl::Extern(function) => Some(&function.signature),
            Decl::Type(_) => None,
        }
    }

    fn ports(&mut self, file: &'c CheckedFile, library: u32, name: Name) -> Result<(Vec<Port>, Vec<Port>), LowerError> {
        let Some(signature) = Self::signature_of(file) else { return Err(self.bug(file, "a function is declared by a type file")) };
        let Some(declared) = self.declared(library, name) else { return Err(self.bug(file, "a function that its library does not declare")) };
        let mut inputs = Vec::with_capacity(signature.params.len());
        for (param, declared) in signature.params.iter().zip(&declared.params) {
            let ty = Some(file.resolved.ty(param.id)).filter(|ty| *ty != TyId::ERROR).unwrap_or(declared.ty);
            let default = match &param.default {
                Some(written) => Some(self.written_constant(file, written, ty, &Literals::new())?),
                None => None,
            };
            inputs.push(Port { name: param.name, ty, default, variadic: param.variadic, origin: param.id });
        }
        let mut outputs = Vec::with_capacity(signature.outputs.len());
        for (output, declared) in signature.outputs.iter().zip(&declared.outputs) {
            let default = Some(self.default_of(declared.ty, file)?);
            outputs.push(Port { name: output.name, ty: declared.ty, default, variadic: false, origin: output.id });
        }
        if inputs.len() != signature.params.len() || outputs.len() != signature.outputs.len() {
            return Err(self.bug(file, "a signature that its declaration does not match"));
        }
        Ok((inputs, outputs))
    }

    fn function(&mut self, id: FunctionId) -> Result<Function, LowerError> {
        let checked = self.checked;
        let Some(Pending { file: index, key, inputs, outputs }) = self.pending.get_mut(id.0 as usize).and_then(Option::take) else {
            return Err(LowerError { file: String::new(), message: format!("function {} is lowered twice", id.0) });
        };
        let Some(file) = checked.files.get(index) else { return Err(LowerError { file: String::new(), message: "no file".to_owned() }) };
        let elements = self.elements(file, &inputs, &outputs)?;
        let body = match &file.file.decl {
            Decl::Function(function) => Body::Region(body::lower_function(self, file, function, &inputs, &outputs, &elements)?),
            Decl::Extern(function) => {
                let library = checked.libraries.get(key.library as usize).map(|library| library.id);
                let Some(library) = library else { return Err(self.bug(file, "an extern function without its library")) };
                let runs = self.names.resolve(function.language) == "rust";
                let native = function.args.iter().find_map(|arg| match arg {
                    Arg::Value { name: Some(name), value: Operand::Text(_, text), .. } if self.names.resolve(*name) == "function" => {
                        Some(read_escapes(text).into_owned())
                    }
                    _ => None,
                });
                let native = native.map_or(function.name, |text| self.names.intern(&text));
                let always = checked.libraries.get(key.library as usize).is_some_and(|library| library.always.contains(&key.name));
                // In the places of its own inputs, a variadic one as one array, as a `core.call` of it passes them (spec 4).
                let input_names: Vec<Name> = inputs.iter().map(|port| port.name).collect();
                let input_tys: Vec<TyId> = inputs.iter().map(|port| port.ty).collect();
                let output_names: Vec<Name> = outputs.iter().map(|port| port.name).collect();
                let output_tys: Vec<TyId> = outputs.iter().map(|port| port.ty).collect();
                let skip = self.on_error(file, &input_names, &input_tys, None, &output_names, &output_tys, always)?;
                Body::Native { library, function: native, runs, skip }
            }
            Decl::Type(_) => return Err(self.bug(file, "a type file is lowered as a function")),
        };
        let file_index = u32::try_from(index).map_err(|_| self.bug(file, "too many files"))?;
        let library = checked.libraries.get(key.library as usize).ok_or_else(|| self.bug(file, "a function of no library"))?;
        let name = self.names.resolve(key.name);
        let shown = if library.name.is_empty() { name.to_owned() } else { format!("{}.{name}", library.name) };
        Ok(Function { key, library: library.id, shown: shown.into(), file: file_index, inputs, outputs, elements, body })
    }

    fn elements(&mut self, file: &'c CheckedFile, inputs: &[Port], outputs: &[Port]) -> Result<Vec<Element>, LowerError> {
        let Some(panel) = &file.file.panel else { return Ok(Vec::new()) };
        let mut elements = Vec::with_capacity(panel.items.len());
        for item in &panel.items {
            let input = inputs.iter().position(|port| port.name == item.name);
            let output = outputs.iter().position(|port| port.name == item.name);
            // Of an input and an output of one name, an indicator shows the output (10).
            let port = match (input, output, item.direction) {
                (Some(_), Some(output), labcraft_graph::PanelDirection::Indicator) | (None, Some(output), _) => {
                    Some(PortOf::Output(index_u32(output)))
                }
                (Some(input), _, _) => Some(PortOf::Input(index_u32(input))),
                (None, None, _) => None,
            };
            let ty = Some(file.resolved.ty(item.id)).filter(|ty| *ty != TyId::ERROR);
            let given = item.args.iter().flatten().find_map(|arg| match arg {
                Arg::Value { name: Some(name), value, .. } if self.names.resolve(*name) == "default" => Some(value),
                _ => None,
            });
            let default = match (port, ty) {
                (Some(PortOf::Input(input)), _) => inputs.get(input as usize).and_then(|port| port.default),
                (_, None) => None,
                (_, Some(ty)) => Some(match given {
                    Some(value) => self.literal(file, value, ty, &Literals::new())?,
                    None => self.default_of(ty, file)?,
                }),
            };
            elements.push(Element { name: item.name, kind: item.kind, ty, default, port, origin: item.id });
        }
        Ok(elements)
    }

    // Constants.

    pub(crate) fn intern(&mut self, ty: TyId, value: ConstValue) -> ConstId {
        let key = (ty, key_of(&value));
        if let Some(id) = self.constant_ids.get(&key) {
            return *id;
        }
        let id = ConstId(u32::try_from(self.constants.len()).unwrap_or(u32::MAX));
        self.constants.push(Constant { ty, value });
        self.constant_ids.insert(key, id);
        id
    }

    pub(crate) fn plain(&mut self, ty: Ty, file: &CheckedFile) -> Result<TyId, LowerError> {
        self.types.intern(ty).map_err(|_| self.bug(file, "a type out of bounds"))
    }

    /// What a named type stands for, through its aliases. `None` for an opaque type, which has no definition.
    pub(crate) fn definition(&self, ty: TyId) -> Option<TyId> {
        let mut current = ty;
        // Each alias is a file, and a chain of them is no longer than the files of the run.
        for _ in 0..=self.checked.files.len() {
            let Ty::Named { library, name } = self.types.get(current) else { return Some(current) };
            let declared =
                self.library_of.get(library).and_then(|index| self.checked.libraries.get(*index)).and_then(|library| library.types.get(name))?;
            if declared.opaque {
                return None;
            }
            if declared.definition == current {
                return Some(current);
            }
            current = declared.definition;
        }
        Some(current)
    }

    /// One level of an array: `f64` in `[f64]`, `[f64]` in `[f64; 2]`.
    pub(crate) fn element_of(&mut self, ty: TyId, file: &CheckedFile) -> Result<TyId, LowerError> {
        match self.definition(ty).map(|definition| self.types.get(definition).clone()) {
            Some(Ty::Array { element, dimensions }) if dimensions <= 1 => Ok(element),
            Some(Ty::Array { element, dimensions }) => self.plain(Ty::Array { element, dimensions: dimensions - 1 }, file),
            _ => Err(self.bug(file, "an array constant for a type that is not an array")),
        }
    }

    /// The index of a variant in the enumeration of this type.
    pub(crate) fn variant_index(&self, ty: TyId, variant: Name) -> Option<(u32, Vec<TyId>)> {
        let Ty::Enum(variants) = self.types.get(self.definition(ty)?) else { return None };
        let index = variants.iter().position(|candidate| candidate.name == variant)?;
        Some((index_u32(index), variants.get(index)?.values.clone()))
    }

    /// Spec 7.2: the input named `err`, of type `error`, and what each output takes when the node does not run. The names and
    /// types are those of the parameters; `variadic` says where the inputs of a variadic one are, on a node that has them.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn on_error(
        &mut self,
        file: &CheckedFile,
        input_names: &[Name],
        input_tys: &[TyId],
        variadic: Option<Variadic>,
        output_names: &[Name],
        output_tys: &[TyId],
        always: bool,
    ) -> Result<Skip, LowerError> {
        // The input of a parameter, past the arguments of a variadic one.
        let input_of = |position: usize| -> Option<usize> {
            match variadic {
                Some(Variadic { start, count }) if position > start as usize => Some(position + count as usize - 1),
                Some(Variadic { start, .. }) if position == start as usize => None,
                _ => Some(position),
            }
        };
        let error_names = self.names.find("err");
        let types = &self.types;
        let err = input_names.iter().enumerate().find_map(|(position, name)| {
            let input = input_of(position)?;
            (Some(*name) == error_names && input_tys.get(input).is_some_and(|ty| matches!(types.get(*ty), Ty::ErrorValue))).then_some(input)
        });
        let Some(err) = err else { return Ok(Skip::Never) };
        let mut defaults = Vec::with_capacity(output_tys.len());
        for ty in output_tys {
            defaults.push(self.default_of(*ty, file)?);
        }
        let through = output_tys
            .iter()
            .enumerate()
            .map(|(output, ty)| {
                let name = output_names.get(output)?;
                let position = input_names.iter().position(|input| input == name)?;
                let input = input_of(position)?;
                input_tys.get(input).filter(|input| self.types.matches(**input, *ty)).map(|_| index_u32(input))
            })
            .collect();
        Ok(Skip::OnError { err: index_u32(err), always, defaults, through })
    }

    /// What a call of a primitive passed as a value does with an error: its parameters have their names in the table of
    /// primitives, and the types that its function type gives them.
    fn primitive_skip(&mut self, library: PrimitiveLibrary, name: Name, ty: TyId, file: &CheckedFile) -> Result<Skip, LowerError> {
        let text = self.names.resolve(name).to_owned();
        let signature = primitive(library, &text).ok_or_else(|| self.bug(file, "a primitive that the table does not have"))?;
        let definition = self.definition(ty).ok_or_else(|| self.bug(file, "a primitive of an opaque type"))?;
        let Ty::Function { params, outputs } = self.types.get(definition).clone() else {
            return Err(self.bug(file, "a primitive whose type is no function"));
        };
        let values: Vec<&str> = signature.params.iter().filter(|param| param.kind == ParamKind::Value).map(|param| param.name).collect();
        if values.len() != params.len() || values.len() != signature.params.len() {
            return Err(self.bug(file, "a primitive as a value whose type does not match its parameters"));
        }
        let input_names: Vec<Name> = values.iter().map(|name| self.names.intern(name)).collect();
        let input_tys: Vec<TyId> = params.iter().map(|param| param.ty).collect();
        let output_names: Vec<Name> = signature.outputs.unwrap_or(&[]).iter().map(|output| self.names.intern(output)).collect();
        self.on_error(file, &input_names, &input_tys, None, &output_names, &outputs, false)
    }

    /// The default value of a type (spec 7.2). Recursion follows the type, as deep as `check` let it nest.
    pub(crate) fn default_of(&mut self, ty: TyId, file: &CheckedFile) -> Result<ConstId, LowerError> {
        if let Some(known) = self.default_ids.get(&ty) {
            return Ok(*known);
        }
        let id = self.default_once(ty, file)?;
        self.default_ids.insert(ty, id);
        Ok(id)
    }

    fn default_once(&mut self, ty: TyId, file: &CheckedFile) -> Result<ConstId, LowerError> {
        let Some(definition) = self.definition(ty) else { return Ok(self.intern(ty, ConstValue::InvalidHandle)) };
        let value = match self.types.get(definition).clone() {
            Ty::Bool => ConstValue::Bool(false),
            Ty::Number { kind, .. } if kind.is_complex() => ConstValue::Complex { re: 0.0, im: 0.0 },
            Ty::Number { kind, .. } if kind.is_integer() => ConstValue::Int(0),
            Ty::Number { .. } => ConstValue::Float(0.0),
            Ty::String => ConstValue::Text("".into()),
            Ty::Bytes => ConstValue::Bytes(Box::default()),
            Ty::Duration => ConstValue::Duration(0),
            Ty::ErrorValue => ConstValue::NoError,
            Ty::Array { .. } => ConstValue::Array(Vec::new()),
            Ty::Struct(fields) => {
                let mut values = Vec::with_capacity(fields.len());
                for (_, field) in fields {
                    values.push(self.default_of(field, file)?);
                }
                ConstValue::Struct(values)
            }
            Ty::Enum(variants) => {
                let Some(first) = variants.first() else { return Err(self.bug(file, "an enumeration without variants")) };
                let mut values = Vec::with_capacity(first.values.len());
                for value in &first.values {
                    values.push(self.default_of(*value, file)?);
                }
                ConstValue::Variant { variant: 0, values }
            }
            Ty::Waveform(element) => {
                let t0 = self.plain(Ty::Timestamp, file)?;
                let dt = self.plain(Ty::Duration, file)?;
                let samples = self.plain(Ty::Array { element, dimensions: 1 }, file)?;
                let fields = vec![
                    self.intern(t0, ConstValue::Unsettled),
                    self.intern(dt, ConstValue::Duration(0)),
                    self.intern(samples, ConstValue::Array(Vec::new())),
                ];
                ConstValue::Struct(fields)
            }
            // Q-21.
            Ty::Timestamp | Ty::Function { .. } | Ty::Queue(_) | Ty::Ref(_) => ConstValue::Unsettled,
            Ty::Error | Ty::Variable(_) | Ty::Named { .. } => return Err(self.bug(file, "the default of a type that is not known")),
        };
        Ok(self.intern(ty, value))
    }

    /// A default of a parameter (R-21), in the type of that parameter.
    fn written_constant(&mut self, file: &CheckedFile, written: &Written, ty: TyId, literals: &Literals<'_>) -> Result<ConstId, LowerError> {
        match written {
            Written::Operand(operand) => self.literal(file, operand, ty, literals),
            Written::Call(call) => {
                let Some(Meaning::Variant { variant, .. }) = file.resolved.call(call.id).map(|bound| bound.callee) else {
                    return Err(self.bug(file, "a default that calls something other than a variant"));
                };
                let Some((index, payload)) = self.variant_index(ty, variant) else { return Err(self.bug(file, "a variant of no enumeration")) };
                let operands: Vec<&Operand> = call
                    .args
                    .iter()
                    .filter_map(|arg| match arg {
                        Arg::Value { value, .. } => Some(value),
                        Arg::After { .. } => None,
                    })
                    .collect();
                if operands.len() != payload.len() {
                    return Err(self.bug(file, "a variant with the wrong number of values"));
                }
                let mut values = Vec::with_capacity(payload.len());
                for (operand, value_ty) in operands.into_iter().zip(payload) {
                    values.push(self.literal(file, operand, value_ty, literals)?);
                }
                Ok(self.intern(ty, ConstValue::Variant { variant: index, values }))
            }
            Written::Struct(literal) => {
                let fields = match self.definition(ty).map(|definition| self.types.get(definition).clone()) {
                    Some(Ty::Struct(fields)) => fields,
                    _ => return Err(self.bug(file, "a struct default for a type that is not a struct")),
                };
                let mut values = Vec::with_capacity(fields.len());
                for (name, field_ty) in fields {
                    let Some(given) = literal.fields.iter().find(|field| field.name == name) else {
                        return Err(self.bug(file, "a struct default without one of its fields"));
                    };
                    values.push(self.literal(file, &given.value, field_ty, literals)?);
                }
                Ok(self.intern(ty, ConstValue::Struct(values)))
            }
        }
    }

    /// A literal, or what a name stands for when it is a constant, in the type it reaches (3.4). Arrays nest as deep as
    /// `check` let them, which the stack of the host holds (`format::STACK_SIZE`).
    pub(crate) fn literal(&mut self, file: &CheckedFile, operand: &Operand, ty: TyId, literals: &Literals<'_>) -> Result<ConstId, LowerError> {
        let key = (std::ptr::from_ref(file) as usize, operand.id(), ty);
        if let Some(known) = self.literal_ids.get(&key) {
            return Ok(*known);
        }
        let id = self.literal_once(file, operand, ty, literals)?;
        self.literal_ids.insert(key, id);
        Ok(id)
    }

    fn literal_once(&mut self, file: &CheckedFile, operand: &Operand, ty: TyId, literals: &Literals<'_>) -> Result<ConstId, LowerError> {
        let target = self.definition(ty).map(|definition| self.types.get(definition).clone());
        let value = match operand {
            Operand::Number(_, numeral, unit) => {
                let written = match unit {
                    Some(unit) => unit_of(unit, self.names).map_err(|_| self.bug(file, "a unit that does not read"))?,
                    None => Unit::ONE,
                };
                let Some(target) = target else { return Err(self.bug(file, "a number for an opaque type")) };
                literal_number(*numeral, &written, &target, self.names).map_err(|_| self.bug(file, "a number that does not fit its type"))?
            }
            Operand::Text(_, text) => ConstValue::Text(read_escapes(text).into()),
            Operand::Bool(_, value) => ConstValue::Bool(*value),
            Operand::None(_) => ConstValue::NoError,
            Operand::Array(_, values) => {
                let element = self.element_of(ty, file)?;
                let mut items = Vec::with_capacity(values.len());
                for value in values {
                    items.push(self.literal(file, value, element, literals)?);
                }
                ConstValue::Array(items)
            }
            Operand::Bracket(id, _) => {
                let element = self.element_of(ty, file)?;
                let item = match file.resolved.of(*id) {
                    Meaning::ArrayOfNumber => {
                        let Some(target) = self.definition(element).map(|definition| self.types.get(definition).clone()) else {
                            return Err(self.bug(file, "`[1]` for an opaque type"));
                        };
                        let one = labcraft_graph::Numeral { negative: false, kind: labcraft_graph::NumeralKind::Int(self.names.intern("1")) };
                        let value = literal_number(one, &Unit::ONE, &target, self.names).map_err(|_| self.bug(file, "`[1]` that does not fit"))?;
                        self.intern(element, value)
                    }
                    Meaning::ArrayOfValue(value) => match literals.get(&value) {
                        Some(literal) => self.literal(file, literal, element, literals)?,
                        None => return Err(self.bug(file, "an array of a wire taken for a constant")),
                    },
                    Meaning::ArrayOfFunction { library, name } => {
                        let function = self.function_id(library, name, file)?;
                        self.intern(element, ConstValue::Function(function))
                    }
                    _ => return Err(self.bug(file, "a bracket that holds no constant")),
                };
                ConstValue::Array(vec![item])
            }
            Operand::Path(path) => match file.resolved.of(path.id) {
                Meaning::Variant { variant, .. } => {
                    let Some((index, _)) = self.variant_index(ty, variant) else { return Err(self.bug(file, "a variant of no enumeration")) };
                    ConstValue::Variant { variant: index, values: Vec::new() }
                }
                Meaning::Function { library, name } => ConstValue::Function(self.function_id(library, name, file)?),
                Meaning::Primitive { library, name } => ConstValue::Primitive { library, name, skip: self.primitive_skip(library, name, ty, file)? },
                Meaning::Value(value) => match literals.get(&value) {
                    Some(literal) if path.rest.is_empty() => return self.literal(file, literal, ty, literals),
                    _ => return Err(self.bug(file, "a wire taken for a constant")),
                },
                _ => return Err(self.bug(file, "a path that is no constant")),
            },
        };
        Ok(self.intern(ty, value))
    }

    /// The type of a number of this kind, without a unit.
    pub(crate) fn number(&mut self, kind: NumberKind, file: &CheckedFile) -> Result<TyId, LowerError> {
        self.plain(Ty::Number { kind, unit: Unit::ONE }, file)
    }
}

pub(crate) fn index_u32(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}
