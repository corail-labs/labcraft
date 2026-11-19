//! Every path of a model with what it stands at (`docs/architecture.md`, section 3): what a rename of a function or of a
//! type goes through, in each file of a project (commands.md 6.7, `function.rename`, `type.rename`). It walks the model
//! as [`File::renumber`] does, and says nothing of what a path means: the caller judges that.
//!
//! It recurses on the model, as `Clone` and `Eq` do: the nesting limit bounds it.

use crate::{
    Arg, Arm, Body, Branching, Call, Carry, Clause, ClauseKind, Constant, Decl, File, For, ForIn, Loop, Operand, OrderRef, Path, Pattern, Rhs,
    Signature, Statement, StatementKind, StructLit, TypeExpr, Yield,
};

/// Where a path stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathAt {
    /// A `use` line: a library.
    Use,
    /// A type written in a signature, a carry, a type argument, a field of a struct type, or the bound of a generic.
    Type,
    /// What a call calls: a function, or a variant that holds values (`Reading.Value(v)`).
    Callee,
    /// An operand: a value and its fields (`step.low`), a function passed as a value, or a variant (`Verdict.Pass`).
    Operand,
    /// The type of a struct construction (`Step { … }`).
    Struct,
    /// The variant of a pattern (`State.Init`, `Reading.Value(v)`).
    Variant,
    /// The category of a pattern of `handle` (`io.timeout`).
    Category,
}

impl File {
    /// Calls `each` on every path of the file but those of its `panel`, `layout` and `icon` sections, which name no
    /// function and no type.
    pub fn paths_mut(&mut self, each: &mut dyn FnMut(PathAt, &mut Path)) {
        let mut walk = Walk(each);
        for line in &mut self.uses {
            (walk.0)(PathAt::Use, &mut line.path);
        }
        match &mut self.decl {
            Decl::Function(function) => {
                walk.signature(&mut function.signature);
                walk.body(&mut function.body);
            }
            Decl::Extern(function) => {
                walk.args(&mut function.args);
                walk.signature(&mut function.signature);
            }
            Decl::Type(decl) => {
                for generic in &mut decl.generics {
                    if let Some(bound) = &mut generic.bound {
                        (walk.0)(PathAt::Type, bound);
                    }
                }
                walk.type_expr(&mut decl.ty);
            }
        }
    }
}

struct Walk<'a>(&'a mut dyn FnMut(PathAt, &mut Path));

impl Walk<'_> {
    fn signature(&mut self, signature: &mut Signature) {
        for generic in &mut signature.generics {
            if let Some(bound) = &mut generic.bound {
                (self.0)(PathAt::Type, bound);
            }
        }
        for param in &mut signature.params {
            self.type_expr(&mut param.ty);
            match &mut param.default {
                Some(Constant::Operand(operand)) => self.operand(operand),
                Some(Constant::Call(call)) => self.call(call),
                Some(Constant::Struct(literal)) => self.struct_literal(literal),
                None => {}
            }
        }
        for output in &mut signature.outputs {
            self.type_expr(&mut output.ty);
        }
    }

    fn type_expr(&mut self, ty: &mut TypeExpr) {
        match ty {
            TypeExpr::Named(named) => {
                (self.0)(PathAt::Type, &mut named.path);
                for arg in &mut named.args {
                    self.type_expr(arg);
                }
            }
            TypeExpr::Array(array) => self.type_expr(&mut array.element),
            TypeExpr::Struct(structure) => {
                for field in &mut structure.fields {
                    self.type_expr(&mut field.ty);
                }
            }
            TypeExpr::Enum(enumeration) => {
                for variant in &mut enumeration.variants {
                    for payload in &mut variant.payload {
                        self.type_expr(payload);
                    }
                }
            }
            TypeExpr::Fn(function) => {
                for param in &mut function.params {
                    self.type_expr(&mut param.ty);
                }
                for output in &mut function.outputs {
                    self.type_expr(output);
                }
            }
        }
    }

    fn operand(&mut self, operand: &mut Operand) {
        match operand {
            Operand::Path(path) => (self.0)(PathAt::Operand, path),
            Operand::Array(_, values) => {
                for value in values {
                    self.operand(value);
                }
            }
            Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) | Operand::Bracket(..) | Operand::Hole(_) => {}
        }
    }

    fn args(&mut self, args: &mut [Arg]) {
        for arg in args {
            match arg {
                Arg::Value { value, .. } => self.operand(value),
                Arg::After { refs, .. } => {
                    for order in refs {
                        if let OrderRef::Value(operand) = order {
                            self.operand(operand);
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
        match &mut statement.kind {
            StatementKind::Binding { value, .. } => self.rhs(value),
            StatementKind::Call(call) => self.call(call),
            StatementKind::Gather(_, operand) | StatementKind::Until(operand) => self.operand(operand),
        }
    }

    fn rhs(&mut self, rhs: &mut Rhs) {
        match rhs {
            Rhs::Call(call) => self.call(call),
            Rhs::Match(branching) | Rhs::Handle(branching) => self.branching(branching),
            Rhs::For(structure) => {
                let For { clauses, body, .. } = &mut **structure;
                self.clauses(clauses);
                self.body(body);
            }
            Rhs::Loop(structure) => {
                let Loop { clauses, body, .. } = &mut **structure;
                self.clauses(clauses);
                self.body(body);
            }
            Rhs::Struct(literal) => self.struct_literal(literal),
            Rhs::Operand(operand) => self.operand(operand),
        }
    }

    fn struct_literal(&mut self, literal: &mut StructLit) {
        let StructLit { path, fields, .. } = literal;
        (self.0)(PathAt::Struct, path);
        for field in fields {
            self.operand(&mut field.value);
        }
    }

    fn call(&mut self, call: &mut Call) {
        (self.0)(PathAt::Callee, &mut call.callee);
        for ty in &mut call.type_args {
            self.type_expr(ty);
        }
        self.args(&mut call.args);
    }

    fn branching(&mut self, branching: &mut Branching) {
        self.operand(&mut branching.selector);
        for arm in &mut branching.arms {
            self.arm(arm);
        }
    }

    fn arm(&mut self, arm: &mut Arm) {
        self.pattern(&mut arm.pattern);
        for pattern in &mut arm.others {
            self.pattern(pattern);
        }
        self.body(&mut arm.body);
        let Yield { values, .. } = &mut arm.yields;
        for value in values {
            self.operand(value);
        }
    }

    fn pattern(&mut self, pattern: &mut Pattern) {
        match pattern {
            Pattern::Literal(operand) => self.operand(operand),
            Pattern::Variant { path, .. } => (self.0)(PathAt::Variant, path),
            Pattern::Category { path, .. } => (self.0)(PathAt::Category, path),
            Pattern::Range { .. } | Pattern::Ok { .. } | Pattern::Wildcard { .. } | Pattern::Code { .. } => {}
        }
    }

    fn clauses(&mut self, clauses: &mut [Clause]) {
        for Clause { kind, .. } in clauses {
            match kind {
                ClauseKind::In(ForIn { source, end, .. }) => {
                    self.operand(source);
                    if let Some(end) = end {
                        self.operand(end);
                    }
                }
                ClauseKind::Carry(Carry { ty, value, .. }) => {
                    if let Some(ty) = ty {
                        self.type_expr(ty);
                    }
                    self.operand(value);
                }
            }
        }
    }
}
