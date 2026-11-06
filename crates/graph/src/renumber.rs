//! Gives every id of a model its rank in the file, so that two models with the same content have the same ids
//! however their statements were written (`docs/architecture.md`, section 3).
//!
//! It recurses on the model, as `Clone` and `Eq` do: the nesting limit bounds it.

use crate::{
    Arg, Arm, Binding, Body, Branching, Call, Carry, Clause, ClauseKind, Constant, Decl, FieldInit, File, FnArg, For, ForIn, Generic, Icon, IconItem,
    IconShape, Id, Label, Layout, LayoutItem, LayoutKind, Loop, NodeRef, Operand, OrderRef, Panel, Path, Pattern, Point, Rhs, Signature, Statement,
    StatementKind, StructLit, Target, TerminalRef, TypeExpr, UnitExpr, UnitTerm, Yield,
};

impl File {
    /// After this, the spans of the source map that made this model no longer apply.
    pub fn renumber(&mut self) {
        let mut ids = Ids(0);
        for line in &mut self.uses {
            ids.next(&mut line.id);
            ids.path(&mut line.path);
        }
        match &mut self.decl {
            Decl::Function(function) => {
                ids.next(&mut function.id);
                ids.signature(&mut function.signature);
                ids.body(&mut function.body);
            }
            Decl::Extern(function) => {
                ids.next(&mut function.id);
                ids.args(&mut function.args);
                ids.signature(&mut function.signature);
            }
            Decl::Type(decl) => {
                ids.next(&mut decl.id);
                ids.generics(&mut decl.generics);
                ids.type_expr(&mut decl.ty);
            }
        }
        if let Some(panel) = &mut self.panel {
            ids.panel(panel);
        }
        if let Some(layout) = &mut self.layout {
            ids.layout(layout);
        }
        if let Some(icon) = &mut self.icon {
            ids.icon(icon);
        }
    }
}

struct Ids(u32);

impl Ids {
    fn next(&mut self, id: &mut Id) {
        *id = Id(self.0);
        self.0 = self.0.saturating_add(1);
    }

    fn path(&mut self, path: &mut Path) {
        self.next(&mut path.id);
    }

    fn generics(&mut self, generics: &mut [Generic]) {
        for generic in generics {
            self.next(&mut generic.id);
            if let Some(bound) = &mut generic.bound {
                self.path(bound);
            }
        }
    }

    fn signature(&mut self, signature: &mut Signature) {
        self.generics(&mut signature.generics);
        for param in &mut signature.params {
            self.next(&mut param.id);
            self.type_expr(&mut param.ty);
            match &mut param.default {
                Some(Constant::Operand(operand)) => self.operand(operand),
                Some(Constant::Call(call)) => self.call(call),
                Some(Constant::Struct(literal)) => self.struct_literal(literal),
                None => {}
            }
        }
        for output in &mut signature.outputs {
            self.next(&mut output.id);
            self.type_expr(&mut output.ty);
        }
    }

    fn type_expr(&mut self, ty: &mut TypeExpr) {
        match ty {
            TypeExpr::Named(named) => {
                self.next(&mut named.id);
                self.path(&mut named.path);
                self.types(&mut named.args);
                if let Some(unit) = &mut named.unit {
                    self.unit(unit);
                }
            }
            TypeExpr::Array(array) => {
                self.next(&mut array.id);
                self.type_expr(&mut array.element);
            }
            TypeExpr::Struct(structure) => {
                self.next(&mut structure.id);
                for field in &mut structure.fields {
                    self.next(&mut field.id);
                    self.type_expr(&mut field.ty);
                }
            }
            TypeExpr::Enum(enumeration) => {
                self.next(&mut enumeration.id);
                for variant in &mut enumeration.variants {
                    self.next(&mut variant.id);
                    self.types(&mut variant.payload);
                }
            }
            TypeExpr::Fn(function) => {
                self.next(&mut function.id);
                for FnArg { id, ty, .. } in &mut function.params {
                    self.next(id);
                    self.type_expr(ty);
                }
                self.types(&mut function.outputs);
            }
        }
    }

    fn types(&mut self, types: &mut [TypeExpr]) {
        for ty in types {
            self.type_expr(ty);
        }
    }

    fn unit(&mut self, unit: &mut UnitExpr) {
        self.next(&mut unit.id);
        self.unit_term(&mut unit.first);
        for (_, term) in &mut unit.rest {
            self.unit_term(term);
        }
    }

    fn unit_term(&mut self, term: &mut UnitTerm) {
        if let UnitTerm::Group(inner) = term {
            self.unit(inner);
        }
    }

    fn operand(&mut self, operand: &mut Operand) {
        match operand {
            Operand::Path(path) => self.path(path),
            Operand::Number(id, _, unit) => {
                self.next(id);
                if let Some(unit) = unit {
                    self.unit(unit);
                }
            }
            Operand::Text(id, _) | Operand::Bool(id, _) | Operand::None(id) => self.next(id),
            Operand::Array(id, values) => {
                self.next(id);
                self.operands(values);
            }
            Operand::Bracket(id, unit) => {
                self.next(id);
                self.unit(unit);
            }
        }
    }

    fn operands(&mut self, operands: &mut [Operand]) {
        for operand in operands {
            self.operand(operand);
        }
    }

    fn args(&mut self, args: &mut [Arg]) {
        for arg in args {
            match arg {
                Arg::Value { id, name, value } => {
                    self.operand(value);
                    // A positional argument shares the place of its value.
                    if name.is_some() {
                        self.next(id);
                    } else {
                        *id = value.id();
                    }
                }
                Arg::After { id, refs } => {
                    self.next(id);
                    for order in refs {
                        match order {
                            OrderRef::Value(operand) => self.operand(operand),
                            OrderRef::Label(Label { id, .. }) => self.next(id),
                        }
                    }
                }
            }
        }
    }

    fn body(&mut self, body: &mut Body) {
        for statement in &mut body.statements {
            self.statement(statement);
        }
    }

    fn statement(&mut self, statement: &mut Statement) {
        self.next(&mut statement.id);
        if let Some(label) = &mut statement.label {
            self.next(&mut label.id);
        }
        match &mut statement.kind {
            StatementKind::Binding { targets, value } => {
                for target in targets.iter_mut() {
                    match target {
                        Target::Name(id, _) | Target::Skip(id) | Target::Next(id, _) => self.next(id),
                    }
                }
                self.rhs(value);
            }
            StatementKind::Call(call) => self.call(call),
            StatementKind::Collect(operand) | StatementKind::Until(operand) => self.operand(operand),
        }
    }

    fn rhs(&mut self, rhs: &mut Rhs) {
        match rhs {
            Rhs::Call(call) => self.call(call),
            Rhs::Match(branching) | Rhs::Handle(branching) => self.branching(branching),
            Rhs::For(structure) => {
                let For { id, clauses, body, .. } = &mut **structure;
                self.next(id);
                self.clauses(clauses);
                self.body(body);
            }
            Rhs::Loop(structure) => {
                let Loop { id, clauses, body, .. } = &mut **structure;
                self.next(id);
                self.clauses(clauses);
                self.body(body);
            }
            Rhs::Struct(literal) => self.struct_literal(literal),
            Rhs::Operand(operand) => self.operand(operand),
        }
    }

    fn struct_literal(&mut self, literal: &mut StructLit) {
        let StructLit { id, path, fields } = literal;
        self.next(id);
        self.path(path);
        for FieldInit { id, value, .. } in fields {
            self.next(id);
            self.operand(value);
        }
    }

    fn call(&mut self, call: &mut Call) {
        self.next(&mut call.id);
        self.path(&mut call.callee);
        self.types(&mut call.type_args);
        self.args(&mut call.args);
    }

    fn branching(&mut self, branching: &mut Branching) {
        self.next(&mut branching.id);
        self.operand(&mut branching.selector);
        for arm in &mut branching.arms {
            self.arm(arm);
        }
    }

    fn arm(&mut self, arm: &mut Arm) {
        self.next(&mut arm.id);
        self.pattern(&mut arm.pattern);
        for pattern in &mut arm.others {
            self.pattern(pattern);
        }
        self.body(&mut arm.body);
        let Yield { id, values, .. } = &mut arm.yields;
        self.next(id);
        self.operands(values);
    }

    fn pattern(&mut self, pattern: &mut Pattern) {
        match pattern {
            Pattern::Literal(operand) => self.operand(operand),
            Pattern::Range { id, .. } | Pattern::Ok { id } => self.next(id),
            Pattern::Variant { id, path, bindings } => {
                self.next(id);
                self.path(path);
                for Binding { id, .. } in bindings {
                    self.next(id);
                }
            }
            Pattern::Wildcard { id, binding } | Pattern::Code { id, binding, .. } => {
                self.next(id);
                self.binding(binding);
            }
            Pattern::Category { id, path, binding, .. } => {
                self.next(id);
                self.path(path);
                self.binding(binding);
            }
        }
    }

    fn binding(&mut self, binding: &mut Option<Binding>) {
        if let Some(Binding { id, .. }) = binding {
            self.next(id);
        }
    }

    fn clauses(&mut self, clauses: &mut [Clause]) {
        for Clause { kind, .. } in clauses {
            match kind {
                ClauseKind::In(ForIn { id, source, end, .. }) => {
                    self.next(id);
                    self.operand(source);
                    if let Some(end) = end {
                        self.operand(end);
                    }
                }
                ClauseKind::Carry(Carry { id, ty, value, .. }) => {
                    self.next(id);
                    if let Some(ty) = ty {
                        self.type_expr(ty);
                    }
                    self.operand(value);
                }
            }
        }
    }

    fn panel(&mut self, panel: &mut Panel) {
        self.next(&mut panel.id);
        for item in &mut panel.items {
            self.next(&mut item.id);
            if let Some(args) = &mut item.args {
                self.args(args);
            }
        }
    }

    fn layout(&mut self, layout: &mut Layout) {
        self.next(&mut layout.id);
        for LayoutItem { id, kind, .. } in &mut layout.items {
            self.next(id);
            match kind {
                LayoutKind::Node { node, at, .. } => {
                    self.node_ref(node);
                    if let Some(at) = at {
                        self.point(at);
                    }
                }
                LayoutKind::Wire { from, to, via } => {
                    self.terminal_ref(from);
                    self.terminal_ref(to);
                    for point in via {
                        self.point(point);
                    }
                }
            }
        }
    }

    fn node_ref(&mut self, node: &mut NodeRef) {
        self.next(&mut node.id);
    }

    fn terminal_ref(&mut self, terminal: &mut TerminalRef) {
        self.next(&mut terminal.id);
        self.node_ref(&mut terminal.node);
    }

    fn point(&mut self, point: &mut Point) {
        self.next(&mut point.id);
    }

    fn icon(&mut self, icon: &mut Icon) {
        self.next(&mut icon.id);
        for IconItem { id, shape, accent, .. } in &mut icon.items {
            self.next(id);
            match shape {
                IconShape::Rect { from, to, .. } | IconShape::Line { from, to } => {
                    self.point(from);
                    self.point(to);
                }
                IconShape::Circle { center, .. } => self.point(center),
                IconShape::Path { .. } => {}
                IconShape::Text { at, .. } => self.point(at),
            }
            if let Some(accent) = accent {
                self.path(accent);
            }
        }
    }
}
