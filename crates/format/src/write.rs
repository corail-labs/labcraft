//! The text of a model: all of the layout of spec 5.2. It prints what is stored, bodies in their stored order, so
//! a canonical file comes from a model that `canonicalize` has made canonical.
//!
//! **Line width.** A form that may go on one line is tried on one line, in the text itself, and taken back if it
//! does not fit or meets a comment. Each try stops at the width of a line, whatever the size of what it tries, so
//! that a file nested deep does not measure its text once per level. A line is judged as a whole: when it does not
//! fit, every struct and enum directly on it is split, and each of their items is then judged on its own line.
//!
//! **Size.** Indentation makes the text of a deep file far larger than the file: 16 MiB of short lines at the
//! ceiling of nesting would take over 4 GiB. A text over `Limits::max_file_size` could not be read again, so it
//! is refused as soon as it passes that size (`LC6003`, `docs/cli.md`, section 2).
//!
//! It recurses on the model: the nesting limit bounds it (`crates/format/tests/stack.rs`).

use labcraft_graph::{
    Arg, Arm, Attached, Binding, Body, Branching, Call, Carry, Clause, ClauseKind, Comment, CommentKind, Constant, Decl, EnumType, Exponent, Extern,
    Field, File, FnType, For, ForIn, Function, Generic, Icon, IconItem, IconShape, IntBound, Layout, LayoutItem, LayoutKind, Loop, NamedType,
    NodeRef, Numeral, NumeralKind, Operand, OrderRef, Output, Panel, PanelDirection, PanelItem, Param, Path, Pattern, Point, Rhs, Signature,
    Statement, StatementKind, StructLit, StructType, Target, TerminalRef, TypeDecl, TypeExpr, UnitExpr, UnitOp, UnitTerm, VariantDecl, View,
};
use labcraft_types::{Interner, Limits, Name};

const WIDTH: usize = 100;

/// The canonical text would be over `Limits::max_file_size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLarge;

/// The text of a model, or [`TooLarge`]. Takes the stack of the nesting of the model: see [`crate::STACK_SIZE`].
pub fn write(file: &File, names: &Interner, limits: &Limits) -> Result<String, TooLarge> {
    let limit = usize::try_from(limits.max_file_size).unwrap_or(usize::MAX);
    let mut writer = Writer::new(names, limit);
    writer.file(file);
    if writer.over { Err(TooLarge) } else { Ok(writer.out) }
}

/// A statement on one line, up to its `{` for a structure: the `node.text` of a diagnostic (`docs/cli.md`, section 4).
pub fn write_line(statement: &Statement, names: &Interner) -> String {
    let mut writer = Writer::new(names, usize::MAX);
    writer.statement_head(statement);
    writer.out
}

/// The text of a reference of the `layout` section: `#close`, `mean.b`.
pub(crate) fn reference_text(node: NodeRef, terminal: Option<Name>, names: &Interner) -> String {
    let mut text = String::new();
    if node.is_label {
        text.push('#');
    }
    text.push_str(names.resolve(node.name));
    if let Some(terminal) = terminal {
        text.push('.');
        text.push_str(names.resolve(terminal));
    }
    text
}

fn is_empty(open: Option<&Comment>, body: &Body) -> bool {
    open.is_none() && body.statements.is_empty() && body.end.is_empty()
}

/// What a block adds to the line of its `{`: ` {`, or ` {}`.
fn block_width(open: Option<&Comment>, body: &Body) -> usize {
    if is_empty(open, body) { 3 } else { 2 }
}

struct Writer<'a> {
    out: String,
    names: &'a Interner,
    limit: usize,
    /// The code points of the current line.
    width: usize,
    /// In a try: text past the width, or a comment, sets `failed` and writes nothing more.
    trying: bool,
    failed: bool,
    over: bool,
}

impl<'a> Writer<'a> {
    fn new(names: &'a Interner, limit: usize) -> Self {
        Writer { out: String::new(), names, limit, width: 0, trying: false, failed: false, over: false }
    }

    // Text.

    fn push(&mut self, text: &str) {
        if self.over {
            return;
        }
        if self.trying {
            if self.failed {
                return;
            }
            let room = WIDTH.saturating_sub(self.width);
            let count = text.chars().take(room + 1).count();
            if count > room {
                self.failed = true;
                return;
            }
            self.width += count;
            self.out.push_str(text);
            return;
        }
        if text.len() > self.limit.saturating_sub(self.out.len()) {
            self.over = true;
            return;
        }
        self.out.push_str(text);
        self.width = self.width.saturating_add(text.chars().count());
    }

    fn name(&mut self, name: Name) {
        let names = self.names;
        self.push(names.resolve(name));
    }

    fn indent(&mut self, depth: usize) {
        for _ in 0..depth {
            if self.over {
                return;
            }
            self.push("    ");
        }
    }

    fn newline(&mut self) {
        self.push("\n");
        self.width = 0;
    }

    /// A form on one line, followed by `tail` more code points, if it fits. Otherwise nothing is written.
    fn attempt(&mut self, tail: usize, form: impl FnOnce(&mut Self)) -> bool {
        let (length, width) = (self.out.len(), self.width);
        self.trying = true;
        self.failed = false;
        form(self);
        let fits = !self.failed && self.width.saturating_add(tail) <= WIDTH;
        self.trying = false;
        self.failed = false;
        if !fits {
            self.out.truncate(length);
            self.width = width;
        } else if self.out.len() > self.limit {
            self.over = true;
        }
        fits
    }

    /// In a try, a comment means that the form does not go on one line. Elsewhere, what cannot hold one is on
    /// one line whatever the width (a statement), and the parser has moved its comments before it.
    fn no_comment(&mut self, comments: &Attached) {
        if !comments.before.is_empty() || comments.line.is_some() {
            self.failed = true;
        }
    }

    fn no_comments(&mut self, comments: &[Comment]) {
        if !comments.is_empty() {
            self.failed = true;
        }
    }

    fn list<T>(&mut self, items: &[T], separator: &str, item: impl Fn(&mut Self, &T)) {
        for (index, value) in items.iter().enumerate() {
            if self.trying && self.failed {
                return;
            }
            if index > 0 {
                self.push(separator);
            }
            item(self, value);
        }
    }

    // Comments.

    fn comment(&mut self, comment: &Comment) {
        self.push(match comment.kind {
            CommentKind::Line => "//",
            CommentKind::Doc => "///",
        });
        self.push(&comment.text);
    }

    fn comment_lines(&mut self, comments: &[Comment], depth: usize) {
        for comment in comments {
            self.indent(depth);
            self.comment(comment);
            self.newline();
        }
    }

    /// The end of a line: the comment that ends on it, if any.
    fn end_line(&mut self, comment: Option<&Comment>) {
        if let Some(comment) = comment {
            self.push(" ");
            self.comment(comment);
        }
        self.newline();
    }

    // The file.

    fn file(&mut self, file: &File) {
        self.push("labcraft ");
        self.push(&file.version.to_string());
        self.end_line(file.header_comment.as_ref());
        if !file.uses.is_empty() || !file.use_comments.is_empty() {
            self.newline();
            self.comment_lines(&file.use_comments, 0);
            for line in &file.uses {
                self.push("use ");
                self.path(&line.path);
                self.end_line(line.line.as_ref());
            }
        }
        self.newline();
        match &file.decl {
            Decl::Function(function) => self.function(function),
            Decl::Extern(function) => self.extern_function(function),
            Decl::Type(decl) => self.type_declaration(decl),
        }
        if let Some(panel) = &file.panel {
            self.newline();
            self.panel(panel);
        }
        if let Some(layout) = &file.layout {
            self.newline();
            self.layout(layout);
        }
        if let Some(icon) = &file.icon {
            self.newline();
            self.icon(icon);
        }
        if !file.end.is_empty() {
            self.newline();
            self.comment_lines(&file.end, 0);
        }
    }

    fn function(&mut self, function: &Function) {
        self.comment_lines(&function.comments.before, 0);
        let split = self.signature(function.name, &function.signature, block_width(function.open.as_ref(), &function.body));
        self.push(" ");
        // A split list of outputs ends with `) {` (5.2).
        self.block(function.open.as_ref(), &function.body, 0, !split);
        self.end_line(function.comments.line.as_deref());
    }

    fn extern_function(&mut self, function: &Extern) {
        self.comment_lines(&function.comments.before, 0);
        self.push("extern ");
        self.name(function.language);
        self.args(&function.args);
        self.newline();
        self.signature(function.name, &function.signature, 0);
        self.end_line(function.comments.line.as_deref());
    }

    fn type_declaration(&mut self, decl: &TypeDecl) {
        self.comment_lines(&decl.comments.before, 0);
        self.push("type ");
        self.name(decl.name);
        self.generics(&decl.generics);
        self.push(" = ");
        if !self.attempt(0, |writer| writer.type_expr(&decl.ty, 0, true)) {
            self.type_expr(&decl.ty, 0, false);
        }
        self.end_line(decl.comments.line.as_deref());
    }

    fn generics(&mut self, generics: &[Generic]) {
        if generics.is_empty() {
            return;
        }
        self.push("<");
        self.list(generics, ", ", |writer, generic| {
            writer.name(generic.name);
            if let Some(bound) = &generic.bound {
                writer.push(": ");
                writer.path(bound);
            }
        });
        self.push(">");
    }

    /// The last line of a signature, followed by `tail` more code points. True if the outputs were split.
    fn signature(&mut self, name: Name, signature: &Signature, tail: usize) -> bool {
        let head = |writer: &mut Self| {
            writer.push("fn ");
            writer.name(name);
            writer.generics(&signature.generics);
        };
        let one_line = self.attempt(tail, |writer| {
            head(writer);
            writer.flat_params(signature);
            writer.push(" -> ");
            writer.flat_outputs(signature);
        });
        if one_line {
            return false;
        }
        head(self);
        if signature.params.is_empty() && signature.params_end.is_empty() {
            self.push("()");
        } else {
            self.push("(");
            self.newline();
            for param in &signature.params {
                self.item(1, &param.comments, true, |writer, flat| writer.param(param, flat));
            }
            self.comment_lines(&signature.params_end, 1);
            self.push(")");
        }
        self.push(" -> ");
        if self.attempt(tail, |writer| writer.flat_outputs(signature)) {
            return false;
        }
        if signature.outputs.is_empty() && signature.outputs_end.is_empty() {
            self.push("()");
            return false;
        }
        self.push("(");
        self.newline();
        for output in &signature.outputs {
            self.item(1, &output.comments, true, |writer, flat| writer.output(output, flat));
        }
        self.comment_lines(&signature.outputs_end, 1);
        self.push(")");
        true
    }

    fn flat_params(&mut self, signature: &Signature) {
        self.no_comments(&signature.params_end);
        self.push("(");
        self.list(&signature.params, ", ", |writer, param| {
            writer.no_comment(&param.comments);
            writer.param(param, true);
        });
        self.push(")");
    }

    fn flat_outputs(&mut self, signature: &Signature) {
        self.no_comments(&signature.outputs_end);
        self.push("(");
        self.list(&signature.outputs, ", ", |writer, output| {
            writer.no_comment(&output.comments);
            writer.output(output, true);
        });
        self.push(")");
    }

    fn param(&mut self, param: &Param, flat: bool) {
        self.name(param.name);
        self.push(": ");
        self.type_expr(&param.ty, 1, flat);
        if let Some(default) = &param.default {
            self.push(" = ");
            match default {
                Constant::Operand(operand) => self.operand(operand),
                Constant::Call(call) => self.call(call),
                Constant::Struct(literal) => self.struct_literal(literal),
            }
        }
        if param.variadic {
            self.push("...");
        }
    }

    fn output(&mut self, output: &Output, flat: bool) {
        self.name(output.name);
        self.push(": ");
        self.type_expr(&output.ty, 1, flat);
    }

    /// An item of a split list, on its own lines: its comments, then the item, on one line if it fits.
    fn item(&mut self, depth: usize, comments: &Attached, comma: bool, item: impl Fn(&mut Self, bool)) {
        self.comment_lines(&comments.before, depth);
        self.indent(depth);
        if !self.attempt(usize::from(comma), |writer| item(writer, true)) {
            item(self, false);
        }
        if comma {
            self.push(",");
        }
        self.end_line(comments.line.as_deref());
    }

    // Types.

    /// `flat` writes everything on one line. Otherwise a struct or an enum is split, at the level of `depth`.
    fn type_expr(&mut self, ty: &TypeExpr, depth: usize, flat: bool) {
        match ty {
            TypeExpr::Named(named) => self.named_type(named, depth, flat),
            TypeExpr::Array(array) => {
                self.push("[");
                self.type_expr(&array.element, depth, flat);
                if let Some(dimensions) = array.dimensions {
                    self.push("; ");
                    self.name(dimensions);
                }
                self.push("]");
            }
            TypeExpr::Struct(structure) => self.struct_type(structure, depth, flat),
            TypeExpr::Enum(enumeration) => self.enum_type(enumeration, depth, flat),
            TypeExpr::Fn(function) => self.fn_type(function, depth, flat),
        }
    }

    fn named_type(&mut self, named: &NamedType, depth: usize, flat: bool) {
        self.path(&named.path);
        if !named.args.is_empty() {
            self.push("<");
            self.list(&named.args, ", ", |writer, ty| writer.type_expr(ty, depth, flat));
            self.push(">");
        }
        if let Some(unit) = &named.unit {
            self.push("[");
            self.unit(unit);
            self.push("]");
        }
    }

    fn struct_type(&mut self, structure: &StructType, depth: usize, flat: bool) {
        if flat {
            self.no_comments(&structure.end);
            self.push("struct { ");
            self.list(&structure.fields, ", ", |writer, field| {
                writer.no_comment(&field.comments);
                writer.field(field, depth, true);
            });
            self.push(" }");
            return;
        }
        self.push("struct {");
        self.newline();
        for field in &structure.fields {
            self.item(depth + 1, &field.comments, true, |writer, flat| writer.field(field, depth + 1, flat));
        }
        self.comment_lines(&structure.end, depth + 1);
        self.indent(depth);
        self.push("}");
    }

    fn field(&mut self, field: &Field, depth: usize, flat: bool) {
        self.name(field.name);
        self.push(": ");
        self.type_expr(&field.ty, depth, flat);
    }

    fn enum_type(&mut self, enumeration: &EnumType, depth: usize, flat: bool) {
        if flat {
            self.no_comments(&enumeration.end);
            self.push("enum { ");
            self.list(&enumeration.variants, ", ", |writer, variant| {
                writer.no_comment(&variant.comments);
                writer.variant(variant, depth, true);
            });
            self.push(" }");
            return;
        }
        self.push("enum {");
        self.newline();
        for variant in &enumeration.variants {
            self.item(depth + 1, &variant.comments, true, |writer, flat| writer.variant(variant, depth + 1, flat));
        }
        self.comment_lines(&enumeration.end, depth + 1);
        self.indent(depth);
        self.push("}");
    }

    fn variant(&mut self, variant: &VariantDecl, depth: usize, flat: bool) {
        self.name(variant.name);
        if !variant.payload.is_empty() {
            self.push("(");
            self.list(&variant.payload, ", ", |writer, ty| writer.type_expr(ty, depth, flat));
            self.push(")");
        }
    }

    fn fn_type(&mut self, function: &FnType, depth: usize, flat: bool) {
        self.push("fn(");
        self.list(&function.params, ", ", |writer, param| {
            if let Some(name) = param.name {
                writer.name(name);
                writer.push(": ");
            }
            writer.type_expr(&param.ty, depth, flat);
        });
        self.push(") -> (");
        self.list(&function.outputs, ", ", |writer, ty| writer.type_expr(ty, depth, flat));
        self.push(")");
    }

    fn unit(&mut self, unit: &UnitExpr) {
        self.unit_term(&unit.first);
        for (operator, term) in &unit.rest {
            self.push(match operator {
                UnitOp::Star => "*",
                UnitOp::MiddleDot => "\u{00B7}",
                UnitOp::Slash => "/",
            });
            self.unit_term(term);
        }
    }

    fn unit_term(&mut self, term: &UnitTerm) {
        match term {
            UnitTerm::Named { name, exponent } => {
                self.name(*name);
                self.exponent(*exponent);
            }
            UnitTerm::One { exponent } => {
                self.push("1");
                self.exponent(*exponent);
            }
            UnitTerm::Group(inner) => {
                self.push("(");
                self.unit(inner);
                self.push(")");
            }
        }
    }

    fn exponent(&mut self, exponent: Option<Exponent>) {
        if let Some(exponent) = exponent {
            self.push(if exponent.negative { "^-" } else { "^" });
            self.name(exponent.digits);
        }
    }

    // Values.

    fn path(&mut self, path: &Path) {
        self.name(path.first);
        for segment in &path.rest {
            self.push(".");
            self.name(*segment);
        }
    }

    fn numeral(&mut self, numeral: Numeral) {
        if numeral.negative {
            self.push("-");
        }
        match numeral.kind {
            NumeralKind::Int(text) | NumeralKind::Float(text) => self.name(text),
            NumeralKind::Inf => self.push("inf"),
            NumeralKind::Nan => self.push("nan"),
        }
    }

    fn int_bound(&mut self, bound: IntBound) {
        if bound.negative {
            self.push("-");
        }
        self.name(bound.digits);
    }

    fn operand(&mut self, operand: &Operand) {
        match operand {
            Operand::Path(path) => self.path(path),
            Operand::Number(_, numeral, unit) => {
                self.numeral(*numeral);
                if let Some(unit) = unit {
                    self.push(" ");
                    self.unit(unit);
                }
            }
            Operand::Text(_, text) => self.string(text),
            Operand::Bool(_, true) => self.push("true"),
            Operand::Bool(_, false) => self.push("false"),
            Operand::None(_) => self.push("none"),
            Operand::Array(_, values) => {
                self.push("[");
                self.list(values, ", ", Self::operand);
                self.push("]");
            }
            Operand::Bracket(_, unit) => {
                self.push("[");
                self.unit(unit);
                self.push("]");
            }
        }
    }

    fn string(&mut self, text: &str) {
        self.push("\"");
        self.push(text);
        self.push("\"");
    }

    fn args(&mut self, args: &[Arg]) {
        self.push("(");
        self.list(args, ", ", Self::arg);
        self.push(")");
    }

    fn arg(&mut self, arg: &Arg) {
        match arg {
            Arg::Value { name, value, .. } => {
                if let Some(name) = name {
                    self.name(*name);
                    self.push(": ");
                }
                self.operand(value);
            }
            Arg::After { refs, .. } => {
                self.push("after: ");
                let many = refs.len() > 1;
                if many {
                    self.push("(");
                }
                self.list(refs, ", ", |writer, order| match order {
                    OrderRef::Value(operand) => writer.operand(operand),
                    OrderRef::Label(label) => {
                        writer.push("#");
                        writer.name(label.name);
                    }
                });
                if many {
                    self.push(")");
                }
            }
        }
    }

    fn call(&mut self, call: &Call) {
        self.path(&call.callee);
        if !call.type_args.is_empty() {
            self.push("<");
            self.list(&call.type_args, ", ", |writer, ty| writer.type_expr(ty, 0, true));
            self.push(">");
        }
        self.args(&call.args);
    }

    fn struct_literal(&mut self, literal: &StructLit) {
        self.path(&literal.path);
        self.push(" { ");
        self.list(&literal.fields, ", ", |writer, field| {
            writer.name(field.name);
            writer.push(": ");
            writer.operand(&field.value);
        });
        self.push(" }");
    }

    // Statements.

    /// `{}` if `compact` allows it, or a `{` that ends its line, the statements, and a `}` alone at the level of `depth`.
    fn block(&mut self, open: Option<&Comment>, body: &Body, depth: usize, compact: bool) {
        if compact && is_empty(open, body) {
            self.push("{}");
            return;
        }
        self.push("{");
        self.end_line(open);
        self.statements(body, depth + 1);
        self.comment_lines(&body.end, depth + 1);
        self.indent(depth);
        self.push("}");
    }

    fn statements(&mut self, body: &Body, depth: usize) {
        for statement in &body.statements {
            self.statement(statement, depth);
        }
    }

    fn statement(&mut self, statement: &Statement, depth: usize) {
        self.comment_lines(&statement.comments.before, depth);
        self.indent(depth);
        self.label_and_targets(statement);
        match &statement.kind {
            StatementKind::Binding { value, .. } => self.rhs(value, depth),
            StatementKind::Call(_) | StatementKind::Collect(_) | StatementKind::Until(_) => self.simple_kind(&statement.kind),
        }
        self.end_line(statement.comments.line.as_deref());
    }

    /// What every statement starts with: its label, and the targets and the `=` of a binding.
    fn label_and_targets(&mut self, statement: &Statement) {
        if let Some(label) = statement.label {
            self.push("#");
            self.name(label.name);
            self.push(": ");
        }
        if let StatementKind::Binding { targets, .. } = &statement.kind {
            self.list(targets, ", ", |writer, target| match target {
                Target::Name(_, name) => writer.name(*name),
                Target::Skip(_) => writer.push("_"),
                Target::Next(_, name) => {
                    writer.push("next ");
                    writer.name(*name);
                }
            });
            self.push(" = ");
        }
    }

    fn simple_kind(&mut self, kind: &StatementKind) {
        match kind {
            StatementKind::Call(call) => self.call(call),
            StatementKind::Collect(operand) => {
                self.push("collect ");
                self.operand(operand);
            }
            StatementKind::Until(operand) => {
                self.push("until ");
                self.operand(operand);
            }
            StatementKind::Binding { .. } => {}
        }
    }

    /// See [`write_line`].
    fn statement_head(&mut self, statement: &Statement) {
        self.label_and_targets(statement);
        match &statement.kind {
            StatementKind::Binding { value, .. } => match value {
                Rhs::Call(call) => self.call(call),
                Rhs::Struct(literal) => self.struct_literal(literal),
                Rhs::Operand(operand) => self.operand(operand),
                Rhs::Match(branching) => self.selector("match ", &branching.selector),
                Rhs::Handle(branching) => self.selector("handle ", &branching.selector),
                Rhs::For(structure) => {
                    self.for_keyword(structure);
                    self.flat_clauses(&structure.clauses, &[]);
                    self.push(" {");
                }
                Rhs::Loop(structure) => {
                    self.loop_keyword(structure);
                    self.flat_clauses(&structure.clauses, &[]);
                    self.push(" {");
                }
            },
            kind => self.simple_kind(kind),
        }
    }

    fn selector(&mut self, keyword: &str, selector: &Operand) {
        self.push(keyword);
        self.operand(selector);
        self.push(" {");
    }

    fn rhs(&mut self, rhs: &Rhs, depth: usize) {
        match rhs {
            Rhs::Call(call) => self.call(call),
            Rhs::Struct(literal) => self.struct_literal(literal),
            Rhs::Operand(operand) => self.operand(operand),
            Rhs::Match(branching) => self.branching("match ", branching, depth),
            Rhs::Handle(branching) => self.branching("handle ", branching, depth),
            Rhs::For(structure) => {
                self.for_keyword(structure);
                self.clauses_and_block(&structure.clauses, &structure.clauses_end, structure.open.as_ref(), &structure.body, depth);
            }
            Rhs::Loop(structure) => {
                self.loop_keyword(structure);
                self.clauses_and_block(&structure.clauses, &structure.clauses_end, structure.open.as_ref(), &structure.body, depth);
            }
        }
    }

    fn for_keyword(&mut self, structure: &For) {
        self.push(if structure.parallel { "for parallel" } else { "for" });
    }

    fn loop_keyword(&mut self, structure: &Loop) {
        self.push(if structure.continue_on_error { "loop continue_on_error" } else { "loop" });
    }

    /// What follows the keyword of a `for` or a `loop`: on its line if it fits, otherwise one clause per line and
    /// a `{` alone at the level of the statement, even for an empty body (5.2).
    fn clauses_and_block(&mut self, clauses: &[Clause], end: &[Comment], open: Option<&Comment>, body: &Body, depth: usize) {
        let one_line = self.attempt(block_width(open, body), |writer| writer.flat_clauses(clauses, end)) || (clauses.is_empty() && end.is_empty());
        if one_line {
            self.push(" ");
        } else {
            self.newline();
            for (index, clause) in clauses.iter().enumerate() {
                let comma = index + 1 < clauses.len();
                self.item(depth + 1, &clause.comments, comma, |writer, flat| writer.clause(clause, depth + 1, flat));
            }
            self.comment_lines(end, depth + 1);
            self.indent(depth);
        }
        self.block(open, body, depth, one_line);
    }

    fn flat_clauses(&mut self, clauses: &[Clause], end: &[Comment]) {
        self.no_comments(end);
        if !clauses.is_empty() {
            self.push(" ");
        }
        self.list(clauses, ", ", |writer, clause| {
            writer.no_comment(&clause.comments);
            writer.clause(clause, 0, true);
        });
    }

    fn clause(&mut self, clause: &Clause, depth: usize, flat: bool) {
        match &clause.kind {
            ClauseKind::In(ForIn { name, source, end, .. }) => {
                self.name(*name);
                self.push(" in ");
                self.operand(source);
                if let Some(end) = end {
                    self.push("..");
                    self.operand(end);
                }
            }
            ClauseKind::Carry(Carry { name, ty, value, .. }) => {
                self.push("carry ");
                self.name(*name);
                if let Some(ty) = ty {
                    self.push(": ");
                    self.type_expr(ty, depth, flat);
                }
                self.push(" = ");
                self.operand(value);
            }
        }
    }

    fn branching(&mut self, keyword: &str, branching: &Branching, depth: usize) {
        // Never on one line, even empty (5.2).
        self.selector(keyword, &branching.selector);
        self.end_line(branching.open.as_ref());
        for arm in &branching.arms {
            self.arm(arm, depth + 1);
        }
        self.comment_lines(&branching.end, depth + 1);
        self.indent(depth);
        self.push("}");
    }

    fn arm(&mut self, arm: &Arm, depth: usize) {
        self.comment_lines(&arm.comments.before, depth);
        self.indent(depth);
        self.pattern(&arm.pattern);
        for pattern in &arm.others {
            self.push(", ");
            self.pattern(pattern);
        }
        self.push(" => ");
        let yields_only = is_empty(arm.open.as_ref(), &arm.body) && arm.yields.comments.before.is_empty() && arm.yields.comments.line.is_none();
        if yields_only {
            self.yield_values(&arm.yields.values);
        } else {
            self.push("{");
            self.end_line(arm.open.as_ref());
            self.statements(&arm.body, depth + 1);
            self.comment_lines(&arm.yields.comments.before, depth + 1);
            self.indent(depth + 1);
            self.yield_values(&arm.yields.values);
            self.end_line(arm.yields.comments.line.as_deref());
            self.comment_lines(&arm.body.end, depth + 1);
            self.indent(depth);
            self.push("}");
        }
        self.end_line(arm.comments.line.as_deref());
    }

    /// `yield a`, or `yield (a, b)`: parentheses around a single value are dropped.
    fn yield_values(&mut self, values: &[Operand]) {
        self.push("yield ");
        if let [value] = values {
            self.operand(value);
        } else {
            self.push("(");
            self.list(values, ", ", Self::operand);
            self.push(")");
        }
    }

    fn pattern(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Literal(operand) => self.operand(operand),
            Pattern::Range { low, high, inclusive, .. } => {
                self.int_bound(*low);
                self.push(if *inclusive { "..=" } else { ".." });
                self.int_bound(*high);
            }
            Pattern::Variant { path, bindings, .. } => {
                self.path(path);
                if !bindings.is_empty() {
                    self.push("(");
                    self.list(bindings, ", ", |writer, binding| writer.name(binding.name));
                    self.push(")");
                }
            }
            Pattern::Wildcard { binding, .. } => {
                self.push("_");
                self.binding(binding.as_ref());
            }
            Pattern::Ok { .. } => self.push("ok"),
            Pattern::Category { path, wildcard, binding, .. } => {
                self.path(path);
                if *wildcard {
                    self.push(".*");
                }
                self.binding(binding.as_ref());
            }
            Pattern::Code { code, binding, .. } => {
                self.push("code ");
                self.int_bound(*code);
                if binding.is_some() {
                    self.push(" ");
                }
                self.binding(binding.as_ref());
            }
        }
    }

    fn binding(&mut self, binding: Option<&Binding>) {
        if let Some(binding) = binding {
            self.push("(");
            self.name(binding.name);
            self.push(")");
        }
    }

    // Sections.

    /// `{}`, or a `{` that ends its line, the items, what is left, and a `}` alone on its line.
    fn section<I>(&mut self, items: &[I], end: &[Comment], item: impl Fn(&mut Self, &I)) {
        if items.is_empty() && end.is_empty() {
            self.push("{}");
            return;
        }
        self.push("{");
        self.newline();
        for value in items {
            item(self, value);
        }
        self.comment_lines(end, 1);
        self.push("}");
    }

    fn panel(&mut self, panel: &Panel) {
        self.comment_lines(&panel.comments.before, 0);
        self.push("panel ");
        self.section(&panel.items, &panel.end, Self::panel_item);
        self.end_line(panel.comments.line.as_deref());
    }

    fn panel_item(&mut self, item: &PanelItem) {
        self.comment_lines(&item.comments.before, 1);
        self.indent(1);
        self.push(match item.direction {
            PanelDirection::Control => "control ",
            PanelDirection::Indicator => "indicator ",
        });
        self.name(item.name);
        self.push(": ");
        self.name(item.kind);
        if let Some(args) = &item.args {
            self.args(args);
        }
        self.end_line(item.comments.line.as_deref());
    }

    fn layout(&mut self, layout: &Layout) {
        self.comment_lines(&layout.comments.before, 0);
        self.push("layout ");
        self.section(&layout.items, &layout.end, Self::layout_item);
        self.end_line(layout.comments.line.as_deref());
    }

    fn layout_item(&mut self, item: &LayoutItem) {
        self.comment_lines(&item.comments.before, 1);
        self.indent(1);
        match &item.kind {
            LayoutKind::Node { node, at, view } => {
                self.push("node ");
                self.node_ref(*node);
                if let Some(at) = at {
                    self.push(" at ");
                    self.point(*at);
                }
                if let Some(view) = view {
                    self.push(match view {
                        View::Standard => " view standard",
                        View::Compact => " view compact",
                    });
                }
            }
            LayoutKind::Wire { from, to, via } => {
                self.push("wire ");
                self.terminal_ref(*from);
                self.push(" -> ");
                self.terminal_ref(*to);
                if !via.is_empty() {
                    self.push(" via ");
                    self.list(via, " ", |writer, point| writer.point(*point));
                }
            }
        }
        self.end_line(item.comments.line.as_deref());
    }

    fn node_ref(&mut self, node: NodeRef) {
        if node.is_label {
            self.push("#");
        }
        self.name(node.name);
    }

    fn terminal_ref(&mut self, terminal: TerminalRef) {
        self.node_ref(terminal.node);
        if let Some(name) = terminal.terminal {
            self.push(".");
            self.name(name);
        }
    }

    fn point(&mut self, point: Point) {
        self.push("(");
        self.numeral(point.x);
        self.push(", ");
        self.numeral(point.y);
        self.push(")");
    }

    fn icon(&mut self, icon: &Icon) {
        self.comment_lines(&icon.comments.before, 0);
        self.push("icon ");
        self.name(icon.grid);
        self.push(" ");
        self.section(&icon.items, &icon.end, Self::icon_item);
        self.end_line(icon.comments.line.as_deref());
    }

    fn icon_item(&mut self, item: &IconItem) {
        self.comment_lines(&item.comments.before, 1);
        self.indent(1);
        match &item.shape {
            IconShape::Rect { from, to, radius } => {
                self.push("rect ");
                self.point(*from);
                self.push(" ");
                self.point(*to);
                self.keyword_number(" radius ", *radius);
            }
            IconShape::Circle { center, radius } => {
                self.push("circle ");
                self.point(*center);
                self.push(" ");
                self.numeral(*radius);
            }
            IconShape::Line { from, to } => {
                self.push("line ");
                self.point(*from);
                self.push(" ");
                self.point(*to);
            }
            IconShape::Path { data } => {
                self.push("path ");
                self.string(data);
            }
            IconShape::Text { text, at, size } => {
                self.push("text ");
                self.string(text);
                self.push(" at ");
                self.point(*at);
                self.keyword_number(" size ", *size);
            }
        }
        if item.fill {
            self.push(" fill");
        }
        if let Some(accent) = &item.accent {
            self.push(" accent ");
            self.path(accent);
        }
        self.end_line(item.comments.line.as_deref());
    }

    fn keyword_number(&mut self, keyword: &str, number: Option<Numeral>) {
        if let Some(number) = number {
            self.push(keyword);
            self.numeral(number);
        }
    }
}
