//! The text form of a program (`docs/ir.md`, section 8), for people, diffs and snapshot tests. Uses are written by the name of
//! what they read. Regions are written with an explicit stack.

use std::fmt::Write as _;

use labcraft_types::{Interner, LibraryId, Ty, TyId};

use crate::{
    Arm, Body, Clause, ConstId, ConstValue, Conversion, Function, FunctionId, Node, NodeKind, Order, Pattern, PortOf, Program, Region, Skip, Source,
    Static, Use,
};

/// The program as text, one function after another, `functions[0]` first.
pub fn print(program: &Program, names: &Interner) -> String {
    let mut out = String::new();
    for function in &program.functions {
        Printer { program, names, function }.function(&mut out);
    }
    out
}

/// One function of the program as text: what a snapshot of its file holds.
pub fn print_function(program: &Program, function: FunctionId, names: &Interner) -> String {
    let mut out = String::new();
    if let Some(function) = program.functions.get(function.0 as usize) {
        Printer { program, names, function }.function(&mut out);
    }
    out
}

/// About how much of one constant is written, in bytes, before `…`: each use of a constant costs a few bytes of its file, so
/// the text form stays within a constant factor of the file.
const CONSTANT_TEXT: usize = 80;

struct Printer<'p> {
    program: &'p Program,
    names: &'p Interner,
    function: &'p Function,
}

/// What is left to write.
enum Task<'p> {
    Region { region: &'p Region, indent: usize, label: String },
    Line(String),
}

impl<'p> Printer<'p> {
    fn viewer(&self) -> LibraryId {
        self.function.library
    }

    fn ty(&self, ty: TyId) -> String {
        self.program.types.display(ty, self.viewer(), self.names)
    }

    fn name(&self, name: labcraft_types::Name) -> &str {
        self.names.resolve(name)
    }

    fn function(&self, out: &mut String) {
        let function = self.function;
        let inputs: Vec<String> = function
            .inputs
            .iter()
            .map(|port| {
                let dots = if port.variadic { "..." } else { "" };
                let default = port.default.map_or_else(String::new, |default| format!(" = {}", self.constant(default)));
                format!("{}: {}{dots}{default}", self.name(port.name), self.ty(port.ty))
            })
            .collect();
        let outputs: Vec<String> = function.outputs.iter().map(|port| format!("{}: {}", self.name(port.name), self.ty(port.ty))).collect();
        let _ = writeln!(out, "fn {}({}) -> ({})", function.shown, inputs.join(", "), outputs.join(", "));
        for element in &function.elements {
            let ty = element.ty.map_or_else(|| "untyped".to_owned(), |ty| self.ty(ty));
            let default = element.default.map_or_else(String::new, |default| format!(" = {}", self.constant(default)));
            let port = match element.port {
                Some(PortOf::Input(input)) => format!(" (input {input})"),
                Some(PortOf::Output(output)) => format!(" (output {output})"),
                None => String::new(),
            };
            let _ = writeln!(out, "  element {}: {} {ty}{default}{port}", self.name(element.name), self.name(element.kind));
        }
        match &function.body {
            Body::Native { library: _, function: native, runs, always } => {
                let runs = if *runs { "" } else { ", unavailable" };
                let always = if *always { ", always" } else { "" };
                let _ = writeln!(out, "  native {}{runs}{always}", self.name(*native));
            }
            Body::Region(region) => self.regions(out, region),
        }
    }

    fn regions(&self, out: &mut String, region: &'p Region) {
        let mut tasks = vec![Task::Region { region, indent: 1, label: "region".to_owned() }];
        while let Some(task) = tasks.pop() {
            match task {
                Task::Line(line) => {
                    out.push_str(&line);
                    out.push('\n');
                }
                Task::Region { region, indent, label } => {
                    let pad = "  ".repeat(indent);
                    let args: Vec<String> = (0..region.args.len()).map(|index| self.arg_name(region, index)).collect();
                    let results: Vec<String> = region.results.iter().map(|result| self.use_text(region, result)).collect();
                    let _ = writeln!(out, "{pad}{label} ({}) -> ({})", args.join(", "), results.join(", "));
                    // Pushed in reverse, so that they come out in order.
                    let mut lines: Vec<Task<'p>> = Vec::new();
                    for node in &region.nodes {
                        lines.push(Task::Line(format!("{pad}  {}", self.node(region, node))));
                        match &node.kind {
                            NodeKind::Loop { region: inner, .. } | NodeKind::For { region: inner, .. } => {
                                lines.push(Task::Region { region: inner, indent: indent + 2, label: "region".to_owned() });
                            }
                            NodeKind::Match { arms } | NodeKind::Handle { arms } => {
                                let selector = node.inputs.first().map(|input| input.ty);
                                for arm in arms {
                                    lines.push(Task::Region {
                                        region: &arm.region,
                                        indent: indent + 2,
                                        label: format!("arm {}", self.patterns(arm, selector)),
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                    tasks.extend(lines.into_iter().rev());
                }
            }
        }
    }

    fn arg_name(&self, region: &Region, index: usize) -> String {
        match region.args.get(index).and_then(|arg| arg.names.first()) {
            Some(name) => self.name(*name).to_owned(),
            None => format!("${index}"),
        }
    }

    fn output_name(&self, region: &Region, node: u32, output: u32) -> String {
        let Some(node) = region.nodes.get(node as usize) else { return format!("?{node}") };
        match node.outputs.get(output as usize).and_then(|value| value.names.first()) {
            Some(name) => self.name(*name).to_owned(),
            None => format!("{}.{output}", self.name(node.name)),
        }
    }

    fn use_text(&self, region: &Region, used: &Use) -> String {
        let source_ty = match &used.source {
            Source::Arg(index) => region.args.get(*index as usize).map(|arg| arg.ty),
            Source::Out { node, output } => {
                region.nodes.get(*node as usize).and_then(|node| node.outputs.get(*output as usize)).map(|value| value.ty)
            }
            Source::Const(constant) => self.program.constants.get(constant.0 as usize).map(|constant| constant.ty),
            Source::Array(_) => None,
        };
        let mut text = match &used.source {
            Source::Arg(index) => self.arg_name(region, *index as usize),
            Source::Out { node, output } => self.output_name(region, *node, *output),
            Source::Const(constant) => self.constant(*constant),
            Source::Array(items) => {
                let items: Vec<String> = items.iter().map(|item| self.use_text(region, item)).collect();
                format!("[{}]", items.join(", "))
            }
        };
        let mut ty = source_ty;
        for field in &used.fields {
            let (name, next) = self.field(ty, *field);
            let _ = write!(text, ".{name}");
            ty = next;
        }
        if let Some(conversion) = &used.convert {
            let _ = write!(text, " as {}", self.ty(used.ty));
            let (Conversion::Number { scale, .. } | Conversion::ToDuration { scale } | Conversion::FromDuration { scale, .. }) = conversion;
            {
                let mut steps = Vec::new();
                if let Some(add) = scale.add {
                    steps.push(format!("+{add:?}"));
                }
                if let Some(multiply) = scale.multiply {
                    steps.push(format!("*{multiply:?}"));
                }
                if let Some(divide) = scale.divide {
                    steps.push(format!("/{divide:?}"));
                }
                if let Some(subtract) = scale.subtract {
                    steps.push(format!("-{subtract:?}"));
                }
                if !steps.is_empty() {
                    let _ = write!(text, " ({})", steps.join(" "));
                }
            }
        }
        text
    }

    fn node(&self, region: &Region, node: &Node) -> String {
        let outputs: Vec<String> = (0..node.outputs.len()).map(|output| self.output_name_or_skip(node, output)).collect();
        let name = self.name(node.name);
        let mut line = format!("{} ", node.position);
        if name.starts_with('#') || name.starts_with('@') {
            let _ = write!(line, "{name}: ");
        }
        if !outputs.is_empty() {
            let _ = write!(line, "{} = ", outputs.join(", "));
        }
        let inputs: Vec<String> = node.inputs.iter().map(|input| self.use_text(region, input)).collect();
        let inputs = inputs.join(", ");
        let rhs = match &node.kind {
            NodeKind::Constant => format!("constant {inputs}"),
            NodeKind::Primitive { library, name, statics, .. } => {
                let mut all: Vec<String> = Vec::new();
                if !inputs.is_empty() {
                    all.push(inputs);
                }
                all.extend(statics.iter().map(|item| self.static_text(item)));
                format!("primitive {}.{}({})", library.as_str(), self.name(*name), all.join(", "))
            }
            NodeKind::Native { function, .. } => format!("native {}({inputs})", self.shown(function.0)),
            NodeKind::Call { function, .. } => format!("call {}({inputs})", self.shown(function.0)),
            NodeKind::Construct { ty, variant } => match variant {
                Some(variant) => format!("construct {}.{}({inputs})", self.ty(*ty), self.variant_name(*ty, *variant)),
                None => format!("construct {} {{{inputs}}}", self.ty(*ty)),
            },
            NodeKind::BuildArray => format!("array [{inputs}]"),
            NodeKind::Loop { continue_on_error, carries, collects, .. } => {
                let mode = if *continue_on_error { " continue_on_error" } else { "" };
                format!("loop{mode} (carries {carries}, collects {collects}) ({inputs})")
            }
            NodeKind::For { parallel, clauses, carries, collects, .. } => {
                let mode = if *parallel { " parallel" } else { "" };
                let clauses: Vec<&str> = clauses.iter().map(|clause| if *clause == Clause::Range { "range" } else { "each" }).collect();
                format!("for{mode} [{}] (carries {carries}, collects {collects}) ({inputs})", clauses.join(", "))
            }
            NodeKind::Match { .. } => format!("match ({inputs})"),
            NodeKind::Handle { .. } => format!("handle ({inputs})"),
        };
        line.push_str(&rhs);
        if !node.after.is_empty() {
            let after: Vec<String> = node
                .after
                .iter()
                .map(|order| match order {
                    Order::Value(used) => self.use_text(region, used),
                    Order::Node(other) => {
                        region.nodes.get(*other as usize).map_or_else(|| format!("?{other}"), |other| self.name(other.name).to_owned())
                    }
                })
                .collect();
            let _ = write!(line, " after({})", after.join(", "));
        }
        match &node.skip {
            Skip::Never => {}
            Skip::Runs => line.push_str("  runs"),
            Skip::OnError { err, always, .. } => {
                let always = if *always { ", always" } else { "" };
                let _ = write!(line, "  skip(err: {err}{always})");
            }
            Skip::ByCallee => line.push_str("  skip(by callee)"),
        }
        line
    }

    fn output_name_or_skip(&self, node: &Node, output: usize) -> String {
        node.outputs.get(output).and_then(|value| value.names.first()).map_or_else(|| "_".to_owned(), |name| self.name(*name).to_owned())
    }

    fn shown(&self, function: u32) -> &str {
        self.program.functions.get(function as usize).map_or("?", |function| &function.shown)
    }

    fn static_text(&self, item: &Static) -> String {
        match item {
            Static::Unit(unit) => format!("[{unit}]"),
            Static::Category(category) => category.to_string(),
            Static::Element(element) => {
                let name = self.function.elements.get(element.0 as usize).map_or("?", |element| self.name(element.name));
                format!("element {name}")
            }
            Static::Template(template) => format!("template {template:?}"),
        }
    }

    /// The name of a field, by its index in what the type stands for, and the type of the field.
    fn field(&self, ty: Option<TyId>, field: u32) -> (String, Option<TyId>) {
        let definition = ty.map(|ty| self.program.definitions.get(&ty).copied().flatten().unwrap_or(ty));
        match definition.map(|definition| self.program.types.get(definition)) {
            Some(Ty::Struct(fields)) => match fields.get(field as usize) {
                Some((name, ty)) => (self.name(*name).to_owned(), Some(*ty)),
                None => (field.to_string(), None),
            },
            Some(Ty::Waveform(_)) => (["t0", "dt", "samples"].get(field as usize).map_or_else(|| field.to_string(), |name| (*name).to_owned()), None),
            _ => (field.to_string(), None),
        }
    }

    fn variant_name(&self, ty: TyId, variant: u32) -> String {
        let definition = self.program.definitions.get(&ty).copied().flatten().unwrap_or(ty);
        match self.program.types.get(definition) {
            Ty::Enum(variants) => variants.get(variant as usize).map_or_else(|| format!("#{variant}"), |variant| self.name(variant.name).to_owned()),
            _ => format!("#{variant}"),
        }
    }

    fn patterns(&self, arm: &Arm, selector: Option<TyId>) -> String {
        let patterns: Vec<String> = arm
            .patterns
            .iter()
            .map(|pattern| match pattern {
                Pattern::Literal(constant) => self.constant(*constant),
                Pattern::Range { low, high, inclusive } => format!("{low}..{}{high}", if *inclusive { "=" } else { "" }),
                Pattern::Variant { variant, binds } => {
                    let name = selector.map_or_else(|| format!("#{variant}"), |ty| self.variant_name(ty, *variant));
                    if *binds == 0 { name } else { format!("{name} binds {binds}") }
                }
                Pattern::Category { category, wildcard, binds } => {
                    let star = if *wildcard { ".*" } else { "" };
                    let binds = if *binds { " binds 1" } else { "" };
                    format!("{category}{star}{binds}")
                }
                Pattern::Code { code, binds } => format!("code {code}{}", if *binds { " binds 1" } else { "" }),
                Pattern::Ok => "ok".to_owned(),
                Pattern::Wildcard { binds } => format!("_{}", if *binds { " binds 1" } else { "" }),
            })
            .collect();
        patterns.join(", ")
    }

    /// A constant as the language would write it, its unit after it, cut past [`CONSTANT_TEXT`] bytes: constants share
    /// what they hold, so one written out can be exponentially larger than the table. Arrays nest as deep as `check` let them.
    fn constant(&self, id: ConstId) -> String {
        let mut out = String::new();
        let mut budget = CONSTANT_TEXT;
        self.write_constant(id, &mut out, &mut budget);
        out
    }

    fn write_constant(&self, id: ConstId, out: &mut String, budget: &mut usize) {
        if *budget == 0 {
            out.push('…');
            return;
        }
        let before = out.len();
        let Some(constant) = self.program.constants.get(id.0 as usize) else {
            let _ = write!(out, "?{}", id.0);
            return;
        };
        let unit = match self.program.types.get(constant.ty) {
            Ty::Number { unit, .. } if *unit != labcraft_types::Unit::ONE => format!(" {unit}"),
            _ => String::new(),
        };
        let items = |out: &mut String, budget: &mut usize, items: &[ConstId]| {
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                if *budget == 0 {
                    out.push('…');
                    break;
                }
                self.write_constant(*item, out, budget);
            }
        };
        match &constant.value {
            ConstValue::Bool(value) => out.push_str(&value.to_string()),
            ConstValue::Int(value) => {
                let _ = write!(out, "{value}{unit}");
            }
            ConstValue::Float(value) => {
                let _ = write!(out, "{value:?}{unit}");
            }
            ConstValue::Complex { re, im } => {
                let _ = write!(out, "({re:?}, {im:?}){unit}");
            }
            ConstValue::Text(text) => {
                // A text of a megabyte, used a million times, is written as its first characters each time.
                let shown: String = text.chars().take(*budget).collect();
                let _ = write!(out, "{shown:?}");
                if shown.len() < text.len() {
                    out.push('…');
                }
            }
            ConstValue::Bytes(bytes) => {
                let _ = write!(out, "bytes({})", bytes.len());
            }
            ConstValue::Duration(ns) => {
                let _ = write!(out, "{ns} ns");
            }
            ConstValue::NoError => out.push_str("none"),
            ConstValue::Array(values) => {
                out.push('[');
                items(out, budget, values);
                out.push(']');
            }
            ConstValue::Struct(fields) => {
                let _ = write!(out, "{} {{", self.ty(constant.ty));
                items(out, budget, fields);
                out.push('}');
            }
            ConstValue::Variant { variant, values } => {
                let _ = write!(out, "{}.{}", self.ty(constant.ty), self.variant_name(constant.ty, *variant));
                if !values.is_empty() {
                    out.push('(');
                    items(out, budget, values);
                    out.push(')');
                }
            }
            ConstValue::Function(function) => {
                let _ = write!(out, "fn {}", self.shown(function.0));
            }
            ConstValue::Primitive { library, name } => {
                let _ = write!(out, "{}.{}", library.as_str(), self.name(*name));
            }
            ConstValue::InvalidHandle => {
                let _ = write!(out, "invalid {}", self.ty(constant.ty));
            }
            ConstValue::ToTheEnd => out.push('…'),
            ConstValue::Unsettled => {
                let _ = write!(out, "unsettled {}", self.ty(constant.ty));
            }
        }
        *budget = budget.saturating_sub(out.len().saturating_sub(before).max(1));
    }
}
