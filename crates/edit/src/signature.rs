//! The function itself (commands.md 6.7, "Functions, signatures and types"): its documentation (`function.set_doc`), its
//! inputs and outputs (`signature.*`), and the text of a new function, which `function.create` writes. A rename of an input
//! also updates the calls that name it in the project, in the same step (decision of the owner, 2026-11-15).

use std::collections::BTreeSet;

use labcraft_format::{format, is_name, read};
use labcraft_graph::{
    Arg, Attached, Comment, CommentKind, Constant, LayoutKind, Operand, OrderRef, Output, Param, Rhs, StatementKind, Target, TypeExpr,
};
use labcraft_types::{Limits, Name};

use crate::delete::{readers, take_out};
use crate::model::Edit;
use crate::names::rename;
use crate::places::{args_mut, operand_mut, rename_read};
use crate::scope::{self, Bound, Value};
use crate::wire::{edit_node_text, node_name_of, statement_node};
use crate::{EditError, Edited, Files, Judge, Renamed, cut};

/// A terminal of the frame of the function, as `terminal` names it: `input x` or `output x`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Terminal {
    Input(String),
    Output(String),
}

impl Terminal {
    /// `input x` or `output x`, with one or more spaces between.
    pub fn parse(text: &str) -> Option<Terminal> {
        let (side, name) = text.trim().split_once(char::is_whitespace)?;
        let name = name.trim();
        if name.is_empty() || name.contains(char::is_whitespace) {
            return None;
        }
        match side {
            "input" => Some(Terminal::Input(name.to_owned())),
            "output" => Some(Terminal::Output(name.to_owned())),
            _ => None,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Terminal::Input(name) | Terminal::Output(name) => name,
        }
    }

    fn side(&self) -> &'static str {
        match self {
            Terminal::Input(_) => "input",
            Terminal::Output(_) => "output",
        }
    }

    pub(crate) fn shown(&self) -> String {
        format!("{} {}", self.side(), self.name())
    }
}

/// The text of a new function `name`, with its documentation, in canonical form (commands.md 6.7, `function.create`):
/// `fn name() -> () {}`. A name that is not one of the language is `name.invalid` of `path`, since the file names it
/// (R-22).
pub fn new_function(name: &str, doc: Option<&str>, limits: &Limits) -> Result<String, EditError> {
    if !is_name(name) {
        return Err(EditError::new(
            "name.invalid",
            format!("`{}` is not a name for a function: a letter, then letters, digits and `_`, and no reserved word (R-22)", cut(name)),
        )
        .of("path"));
    }
    let mut text = String::from("labcraft 1\n\n");
    for line in doc_lines(doc.unwrap_or_default()) {
        text.push_str("///");
        text.push_str(&line.text);
        text.push('\n');
    }
    text.push_str(&format!("fn {name}() -> () {{\n}}\n"));
    let mut names = labcraft_types::Interner::new();
    let formatted = format(text.as_bytes(), &mut names, limits);
    formatted.text.ok_or_else(|| {
        let why = formatted.report.diagnostics().first().map(|diagnostic| diagnostic.message.clone()).unwrap_or_default();
        EditError::invalid("doc", format!("the function would not read with this documentation: {why}"))
    })
}

/// The lines of a documentation, as comments `///`: one per line of the text, its ending spaces cut, the empty lines at
/// its start and its end left out; an empty text has none.
pub(crate) fn doc_lines(doc: &str) -> Vec<Comment> {
    let doc = doc.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<&str> = doc.split('\n').map(str::trim_end).collect();
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    let start = lines.iter().position(|line| !line.is_empty()).unwrap_or(lines.len());
    lines
        .into_iter()
        .skip(start)
        .map(|line| Comment { kind: CommentKind::Doc, text: if line.is_empty() { Box::from("") } else { format!(" {line}").into_boxed_str() } })
        .collect()
}

/// `function.set_doc`: the documentation where it was among the comments before the function, or after them.
pub(crate) fn set_doc(edit: &mut Edit, doc: &str) -> Result<(), EditError> {
    let lines = doc_lines(doc);
    if lines.iter().any(|line| line.text.chars().any(|c| c.is_control() && c != '\t')) {
        return Err(EditError::invalid("doc", "a documentation holds no control character but the tab and the line ending"));
    }
    let function = edit.body_function_mut()?;
    let before = function.comments.before.to_vec();
    let first = before.iter().position(|comment| comment.kind == CommentKind::Doc);
    let mut kept: Vec<Comment> = before.iter().filter(|comment| comment.kind != CommentKind::Doc).cloned().collect();
    let at = first.map_or(kept.len(), |first| before.iter().take(first).filter(|comment| comment.kind != CommentKind::Doc).count());
    kept.splice(at..at, lines);
    function.comments.before = kept.into_boxed_slice();
    Ok(())
}

/// Whether a type is `error`: the last terminal of a side, by convention, before which a new one goes.
fn is_error(edit: &Edit, ty: &TypeExpr) -> bool {
    matches!(ty, TypeExpr::Named(named) if named.path.rest.is_empty() && named.args.is_empty() && named.unit.is_none() && edit.text(named.path.first) == "error")
}

/// Where a new terminal goes among `count`: `position`, or the last, before `err` when the last is an `error`.
fn place_of(count: usize, last_is_error: bool, position: Option<u64>) -> Result<usize, EditError> {
    match position {
        None => Ok(if last_is_error { count.saturating_sub(1) } else { count }),
        Some(position) => match usize::try_from(position) {
            Ok(position) if position <= count => Ok(position),
            _ => Err(EditError::invalid("position", format!("the function has {count} terminals on that side: `position` is from 0 to {count}"))),
        },
    }
}

/// The names that a new terminal, or a terminal renamed, would meet on its side: the terminals of that side, the values
/// of the body, the labels and the libraries the file uses. An input and an output may bear the same name (`err`), as
/// long as no line of the body binds it but the one that gives the output. A new output may bear the name of a value
/// that a line of the body of the function binds (`given`): that line gives it, as a file writes it (spec 5.2).
fn taken_for(edit: &Edit, terminal: &Terminal, except: Option<Name>, given: bool) -> BTreeSet<String> {
    let mut taken = BTreeSet::new();
    let Some(function) = edit.function() else { return taken };
    match terminal {
        Terminal::Input(_) => {
            taken.extend(function.signature.params.iter().filter(|param| Some(param.name) != except).map(|param| edit.text(param.name).to_owned()))
        }
        Terminal::Output(_) => {
            taken.extend(
                function.signature.outputs.iter().filter(|output| Some(output.name) != except).map(|output| edit.text(output.name).to_owned()),
            );
        }
    }
    for value in scope::values(function) {
        let gives_it = given && matches!(terminal, Terminal::Output(_)) && value.seen_in.is_empty() && matches!(value.bound, Bound::Line(..));
        // `err = e2` gives the output `err`: an input `err` beside it is read where `err` is read (R-2), so only while
        // nothing reads the value of that line.
        let beside = matches!(terminal, Terminal::Input(_)) && gives_unread_output(function, &value);
        if !matches!(value.bound, Bound::Input) && Some(value.name) != except && !gives_it && !beside {
            taken.insert(edit.text(value.name).to_owned());
        }
    }
    for (label, _) in scope::labels(function) {
        taken.insert(edit.text(label).to_owned());
    }
    for library in &edit.file.uses {
        taken.insert(edit.text(library.path.first).to_owned());
    }
    taken
}

/// Whether `value` is bound by a line of the body of the function to the name of one of its outputs, which gives that
/// output, and nothing reads it.
fn gives_unread_output(function: &labcraft_graph::Function, value: &Value) -> bool {
    value.seen_in.is_empty()
        && matches!(value.bound, Bound::Line(..))
        && function.signature.outputs.iter().any(|output| output.name == value.name)
        && readers(function, value, &[]).is_empty()
}

/// The target of the line of `value` named `to`, and nothing else: an output that shares its name with an input, whose
/// readers read the input (R-2).
fn rename_target(edit: &mut Edit, value: &Value, to: Name) -> Result<(), EditError> {
    if let Bound::Line(loc, index) = &value.bound
        && let Some(statement) = scope::statement_mut(edit.body_function_mut()?, loc)
        && let StatementKind::Binding { targets, .. } = &mut statement.kind
        && let Some(Target::Name(_, name)) = targets.get_mut(*index)
    {
        *name = to;
    }
    Ok(())
}

/// The `wire` entries of `layout` that end at the output `from` (`wire e3 -> err`, spec 11): renamed to `to`, or taken out
/// with none.
fn wires_to_output(edit: &mut Edit, from: Name, to: Option<Name>) {
    let Some(layout) = &mut edit.file.layout else { return };
    let ends_here = |end: &labcraft_graph::TerminalRef| !end.node.is_label && end.node.name == from && end.terminal.is_none();
    match to {
        Some(to) => {
            for item in &mut layout.items {
                if let LayoutKind::Wire { to: end, .. } = &mut item.kind
                    && ends_here(end)
                {
                    end.node.name = to;
                }
            }
        }
        None => layout.items.retain(|item| !matches!(&item.kind, LayoutKind::Wire { to: end, .. } if ends_here(end))),
    }
}

/// A name that a terminal takes: one name of the language, free on its side.
fn free_name(edit: &Edit, terminal: &Terminal, name: &str, except: Option<Name>, given: bool, parameter: &'static str) -> Result<(), EditError> {
    if !is_name(name) {
        return Err(EditError::new(
            "name.invalid",
            format!("`{}` is not a name: a letter, then letters, digits and `_`, and no reserved word", cut(name)),
        )
        .of(parameter));
    }
    if taken_for(edit, terminal, except, given).contains(name) {
        return Err(EditError::new("name.taken", format!("`{}` is already a name of this function", cut(name))).of(parameter));
    }
    Ok(())
}

/// A type in text, read by the grammar, with the `use` its library needs.
fn type_of(edit: &mut Edit, text: &str) -> Result<TypeExpr, EditError> {
    let ty = crate::parts::read_type(edit, text)
        .ok_or_else(|| EditError::invalid("type", format!("`{}` is not a type of the language (spec 3.2)", cut(text))))?;
    crate::parts::use_library_of(edit, text);
    Ok(ty)
}

/// A default value in text, as a file writes it after `=` (spec 4, R-21): a constant, a variant with its values, a struct
/// of constants. A bare name is a value, not a constant.
fn default_of(edit: &mut Edit, text: &str) -> Result<Constant, EditError> {
    let shown = cut(text);
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains(['\n', '\r']) {
        return Err(EditError::invalid("default", format!("`{shown}` is not one constant of the language")));
    }
    let wrapped = format!("labcraft 1\n\nfn f(x: any = {trimmed}) -> () {{\n}}\n");
    let limits = edit.limits().clone();
    let reading = read(wrapped.as_bytes(), &mut edit.names, &limits);
    let Some(file) = reading.file else {
        let why = reading.report.diagnostics().first().map(|diagnostic| diagnostic.message.clone()).unwrap_or_default();
        return Err(EditError::invalid("default", format!("`{shown}` does not read: {why}")));
    };
    let labcraft_graph::Decl::Function(function) = file.decl else {
        return Err(EditError::invalid("default", format!("`{shown}` is not one constant of the language")));
    };
    let [param] = function.signature.params.as_slice() else {
        return Err(EditError::invalid("default", format!("`{shown}` is not one constant of the language")));
    };
    match &param.default {
        Some(Constant::Operand(Operand::Path(path))) if path.rest.is_empty() => Err(EditError::invalid(
            "default",
            format!("`{shown}` is a value, not a constant: a literal, an array of them, or a variant such as `Verdict.Pass`"),
        )),
        Some(Constant::Operand(Operand::Hole(_))) | None => {
            Err(EditError::invalid("default", format!("`{shown}` is not one constant of the language")))
        }
        Some(constant) => Ok(constant.clone()),
    }
}

/// The input or the output that `terminal` names, by its place on its side, or `edit.unknown`.
pub(crate) fn find(edit: &Edit, terminal: &Terminal) -> Result<usize, EditError> {
    let function = edit.body_function()?;
    let names: Vec<Name> = match terminal {
        Terminal::Input(_) => function.signature.params.iter().map(|param| param.name).collect(),
        Terminal::Output(_) => function.signature.outputs.iter().map(|output| output.name).collect(),
    };
    names.iter().position(|name| edit.text(*name) == terminal.name()).ok_or_else(|| {
        let known: Vec<String> = names.iter().map(|name| format!("{} {}", terminal.side(), edit.text(*name))).collect();
        EditError::new("edit.unknown", format!("this function has no {}", cut(&terminal.shown())))
            .of("terminal")
            .expecting(crate::refs::nearest(&terminal.shown(), &known))
    })
}

/// `signature.add`.
pub(crate) fn add(
    edit: &mut Edit,
    terminal: &Terminal,
    ty: &str,
    default: Option<&str>,
    position: Option<u64>,
    edited: &mut Edited,
) -> Result<(), EditError> {
    free_name(edit, terminal, terminal.name(), None, true, "terminal")?;
    let ty = type_of(edit, ty)?;
    let name = edit.intern(terminal.name());
    match terminal {
        Terminal::Input(_) => {
            let default = default.map(|text| default_of(edit, text)).transpose()?;
            let function = edit.body_function()?;
            let last_is_error = function.signature.params.last().is_some_and(|param| is_error(edit, &param.ty));
            let at = place_of(function.signature.params.len(), last_is_error, position)?;
            let id = edit.id();
            let param = Param { id, comments: Attached::default(), name, ty, default, variadic: false };
            edit.body_function_mut()?.signature.params.insert(at, param);
        }
        Terminal::Output(_) => {
            if default.is_some() {
                return Err(EditError::invalid("default", "an output has no default value: only an input has one (spec 4)"));
            }
            let function = edit.body_function()?;
            let last_is_error = function.signature.outputs.last().is_some_and(|output| is_error(edit, &output.ty));
            let at = place_of(function.signature.outputs.len(), last_is_error, position)?;
            let id = edit.id();
            edit.body_function_mut()?.signature.outputs.insert(at, Output { id, comments: Attached::default(), name, ty });
        }
    }
    edited.names.insert("terminal", terminal.shown());
    Ok(())
}

/// The value of the body that gives the output `name`: a line of the body of the function that binds it.
fn output_value(edit: &Edit, name: Name) -> Option<Value> {
    let function = edit.function()?;
    scope::values(function).into_iter().find(|value| value.name == name && value.seen_in.is_empty() && matches!(value.bound, Bound::Line(..)))
}

/// Whether the line of a value is `o = v`, which gives the output `o` the value `v`.
fn is_alias(edit: &Edit, value: &Value) -> bool {
    let Bound::Line(loc, _) = &value.bound else { return false };
    edit.function().and_then(|function| scope::statement(function, loc)).is_some_and(|statement| {
        matches!(&statement.kind, StatementKind::Binding { targets, value: Rhs::Operand(Operand::Path(path)) } if path.rest.is_empty() && targets.len() == 1)
    })
}

/// `signature.remove`: an input, and what reads it, which becomes unwired or gets a hole (9.1); an output, and its wire,
/// whose node gives back the name that inserting it would have given (2.4).
pub(crate) fn remove(edit: &mut Edit, judge: &dyn Judge, terminal: &Terminal, edited: &mut Edited) -> Result<(), EditError> {
    let at = find(edit, terminal)?;
    let function = edit.body_function()?.clone();
    // Its `input x at y` or `output x at y` entry goes with it, a comment on it too: a terminal of that name added later
    // starts at its automatic place (spec 11).
    let removed = match terminal {
        Terminal::Input(_) => function.signature.params.get(at).map(|param| (false, param.name)),
        Terminal::Output(_) => function.signature.outputs.get(at).map(|output| (true, output.name)),
    };
    if let (Some((output, name)), Some(layout)) = (removed, &mut edit.file.layout) {
        layout
            .items
            .retain(|item| !matches!(&item.kind, LayoutKind::Terminal { output: kind, name: named, .. } if *kind == output && *named == name));
    }
    // Its line of `terminals` too (11.2).
    if let Some((output, name)) = removed {
        crate::block::remove_line(edit, output, name)?;
    }
    match terminal {
        Terminal::Input(_) => {
            let Some(param) = function.signature.params.get(at) else { return Err(EditError::internal("an input that went")) };
            let name = param.name;
            let value = Value { name, bound: Bound::Input, seen_in: Vec::new() };
            let found = readers(&function, &value, &[]);
            take_out(edit, judge, name, false, &found)?;
            edit.body_function_mut()?.signature.params.remove(at);
            // The entries of `layout` of the wires that left it: no value of that name is left for them, the line that gives
            // an output of that name apart, which nothing reads (R-2).
            let left = edit.body_function()?.clone();
            let other_value = scope::values(&left).iter().any(|other| other.name == name && !gives_unread_output(&left, other));
            if !other_value && let Some(layout) = &mut edit.file.layout {
                layout.items.retain(|item| !matches!(&item.kind, LayoutKind::Wire { from, .. } if !from.node.is_label && from.node.name == name));
            }
        }
        Terminal::Output(_) => {
            let Some(output) = function.signature.outputs.get(at) else { return Err(EditError::internal("an output that went")) };
            let name = output.name;
            let shared = function.signature.params.iter().any(|param| param.name == name);
            if let Some(value) = output_value(edit, name) {
                if is_alias(edit, &value) {
                    // What reads the output reads what it gave (`z = y` after `y = x` becomes `z = x`); beside the input of
                    // the same name, what reads `err` reads the input (R-2), and stays.
                    if !shared && let Some(source) = alias_source(edit, &value) {
                        rename_reads(edit, &value, source)?;
                        wires_from(edit, name, source);
                    }
                    if let Bound::Line(loc, _) = &value.bound
                        && let Some(body) = scope::body_mut(edit.body_function_mut()?, &loc.body)
                        && loc.index < body.statements.len()
                    {
                        body.statements.remove(loc.index);
                    }
                } else {
                    let back = node_name_of(edit, judge, &function, &value);
                    let back_name = edit.intern(&back);
                    // Beside the input of the same name, what reads `err` reads the input: only the line changes.
                    if shared {
                        rename_target(edit, &value, back_name)?;
                    } else {
                        rename(edit, &value, back_name)?;
                    }
                    edited.renamed.push(Renamed { file: edit.path.clone(), from: edit.text(name).to_owned(), to: back });
                }
            }
            if shared {
                wires_to_output(edit, name, None);
            }
            edit.body_function_mut()?.signature.outputs.remove(at);
        }
    }
    Ok(())
}

/// Renames the input `from` in the body of the function: each place that reads it, each `after:` that waits for it, and
/// the `wire` entries of `layout` that start at it (spec 11).
fn rename_input(edit: &mut Edit, from: Name, to: Name) -> Result<(), EditError> {
    let value = Value { name: from, bound: Bound::Input, seen_in: Vec::new() };
    rename_reads(edit, &value, to)?;
    let target = edit.body_function_mut()?;
    for param in &mut target.signature.params {
        if param.name == from {
            param.name = to;
        }
    }
    wires_from(edit, from, to);
    Ok(())
}

/// The `wire` entries of `layout` that start at the value `from` start at `to` (spec 11).
fn wires_from(edit: &mut Edit, from: Name, to: Name) {
    if let Some(layout) = &mut edit.file.layout {
        for item in &mut layout.items {
            if let LayoutKind::Wire { from: start, .. } = &mut item.kind
                && !start.node.is_label
                && start.node.name == from
            {
                start.node.name = to;
            }
        }
    }
}

/// Each place that reads `value`, and each `after:` that waits for it, reads `to` instead.
fn rename_reads(edit: &mut Edit, value: &Value, to: Name) -> Result<(), EditError> {
    let from = value.name;
    let function = edit.body_function()?.clone();
    let found = readers(&function, value, &[]);
    let target = edit.body_function_mut()?;
    for reader in found {
        match reader {
            crate::delete::Reader::Slot(loc, slot) => {
                if let Some(operand) = scope::statement_mut(target, &loc).and_then(|statement| operand_mut(statement, slot)) {
                    rename_read(operand, from, to);
                }
            }
            crate::delete::Reader::After(loc) => {
                let Some(args) = scope::statement_mut(target, &loc).and_then(args_mut) else { continue };
                for arg in args.iter_mut() {
                    if let Arg::After { refs, .. } = arg {
                        for reference in refs.iter_mut() {
                            if let OrderRef::Value(operand) = reference {
                                rename_read(operand, from, to);
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// The value that the line `o = v` of `value` gives: `v`.
fn alias_source(edit: &Edit, value: &Value) -> Option<Name> {
    let Bound::Line(loc, _) = &value.bound else { return None };
    let statement = scope::statement(edit.function()?, loc)?;
    match &statement.kind {
        StatementKind::Binding { value: Rhs::Operand(Operand::Path(path)), .. } if path.rest.is_empty() => Some(path.first),
        _ => None,
    }
}

/// `signature.rename` in the file of the function: the input or the output, in the signature, the body and `layout`.
fn rename_terminal(edit: &mut Edit, terminal: &Terminal, new: &str, edited: &mut Edited) -> Result<(), EditError> {
    let at = find(edit, terminal)?;
    let function = edit.body_function()?.clone();
    let renamed = match terminal {
        Terminal::Input(_) => Terminal::Input(new.to_owned()),
        Terminal::Output(_) => Terminal::Output(new.to_owned()),
    };
    edited.names.insert("terminal", renamed.shown());
    if new == terminal.name() {
        return Ok(());
    }
    match terminal {
        Terminal::Input(_) => {
            let from = function.signature.params.get(at).map(|param| param.name).ok_or_else(|| EditError::internal("an input that went"))?;
            // A value of the body by the new name would hide it, an output that a line gives included.
            free_name(edit, terminal, new, Some(from), false, "name")?;
            let to = edit.intern(new);
            rename_input(edit, from, to)?;
            rename_entry(edit, false, from, to)?;
            Ok(())
        }
        Terminal::Output(_) => {
            let from = function.signature.outputs.get(at).map(|output| output.name).ok_or_else(|| EditError::internal("an output that went"))?;
            let given = output_value(edit, from);
            free_name(edit, terminal, new, Some(from), false, "name")?;
            let to = edit.intern(new);
            let shared = function.signature.params.iter().any(|param| param.name == from);
            if let Some(value) = &given {
                if shared {
                    // `err = e3` beside the input `err`: what reads `err` reads the input; only the line changes.
                    rename_target(edit, value, to)?;
                    wires_to_output(edit, from, Some(to));
                } else {
                    // Onto the name of an input, the value of the output would be read in place of the input: only while
                    // nothing reads it (notation 2.4).
                    if function.signature.params.iter().any(|param| param.name == to) && !readers(&function, value, &[]).is_empty() {
                        return Err(EditError::new(
                            "name.taken",
                            format!("`{}` is an input, and the value of the output has readers that would read it", cut(new)),
                        )
                        .of("name"));
                    }
                    rename(edit, value, to)?;
                }
            }
            if let Some(output) = edit.body_function_mut()?.signature.outputs.get_mut(at) {
                output.name = to;
            }
            rename_entry(edit, true, from, to)?;
            Ok(())
        }
    }
}

/// The `input x at y` or `output x at y` entry of a terminal renamed follows it (spec 11), and its line of `terminals`
/// (11.2).
fn rename_entry(edit: &mut Edit, output: bool, from: Name, to: Name) -> Result<(), EditError> {
    if let Some(layout) = &mut edit.file.layout {
        for item in &mut layout.items {
            if let LayoutKind::Terminal { output: kind, name, .. } = &mut item.kind
                && *kind == output
                && *name == from
            {
                *name = to;
            }
        }
    }
    crate::block::rename_line(edit, output, from, to)
}

/// The calls of `edit` to the function of `target` give the input `from` as `to`: their named argument (`high: 250 V`)
/// and the end of their `wire` entries (`wire x -> c.high`). Whether it changed anything.
fn rename_in_calls(edit: &mut Edit, judge: &dyn Judge, target: &str, from: &str, to: &str) -> Result<bool, EditError> {
    let function = edit.body_function()?.clone();
    let path = edit.path.clone();
    let Some(from_name) = edit.names.find(from) else { return Ok(false) };
    let to_name = edit.intern(to);
    let mut changed = false;
    let mut nodes: Vec<(Name, bool)> = Vec::new();
    for loc in scope::all(&function) {
        let Some(statement) = scope::statement(&function, &loc) else { continue };
        let call = match &statement.kind {
            StatementKind::Call(call) | StatementKind::Binding { value: Rhs::Call(call), .. } => call,
            _ => continue,
        };
        if judge.function_file(&path, &edit_node_text(edit, call)).as_deref() != Some(target) {
            continue;
        }
        if let Some(node) = statement_node(statement) {
            nodes.push(node);
        }
        let Some(args) = scope::statement_mut(edit.body_function_mut()?, &loc).and_then(args_mut) else { continue };
        for arg in args.iter_mut() {
            if let Arg::Value { name: Some(name), .. } = arg
                && *name == from_name
            {
                *name = to_name;
                changed = true;
            }
        }
    }
    if let Some(layout) = &mut edit.file.layout {
        for item in &mut layout.items {
            if let LayoutKind::Wire { to: end, .. } = &mut item.kind
                && end.terminal == Some(from_name)
                && nodes.contains(&(end.node.name, end.node.is_label))
            {
                end.terminal = Some(to_name);
                changed = true;
            }
        }
    }
    Ok(changed)
}

/// `signature.rename`: the file of the function, and, for an input, each file of `files` with a call that names it.
/// A file that would change and does not parse is `file.unparsed`, and nothing is written (commands.md 6.7).
pub(crate) fn rename_everywhere(
    files: &Files,
    path: &str,
    text: &str,
    terminal: &Terminal,
    new: &str,
    judge: &dyn Judge,
    limits: &Limits,
) -> Result<Edited, EditError> {
    let mut edited = Edited::default();
    let mut edit = Edit::read(path, text, limits)?;
    let function_name = edit.text(edit.body_function()?.name).to_owned();
    rename_terminal(&mut edit, terminal, new, &mut edited)?;
    let input = matches!(terminal, Terminal::Input(_)) && terminal.name() != new;
    if input {
        // A function that calls itself.
        rename_in_calls(&mut edit, judge, path, terminal.name(), new)?;
    }
    let new_text = crate::route::follow(path, text, edit.finish()?, judge, limits)?;
    if new_text != text {
        edited.texts.insert(path.to_owned(), new_text);
    }
    if !input {
        return Ok(edited);
    }
    // A call is written `name(` or `library.name(`: only a file that writes that can call it, and only such a file that
    // does not parse stops the rename.
    let call = format!("{function_name}(");
    for (other, other_text) in files.iter() {
        if other == path || !other.ends_with(".lcf") || !other_text.contains(call.as_str()) {
            continue;
        }
        let mut caller = match Edit::read(other, other_text, limits) {
            Ok(caller) => caller,
            // An `extern` declaration: no body, no call.
            Err(error) if error.code == "params.invalid" => continue,
            Err(error) => return Err(error),
        };
        if rename_in_calls(&mut caller, judge, path, terminal.name(), new)? {
            let written = caller.finish()?;
            if written != other_text {
                edited.texts.insert(other.to_owned(), written);
            }
        }
    }
    Ok(edited)
}

/// `signature.set_type`.
pub(crate) fn set_type(edit: &mut Edit, terminal: &Terminal, ty: &str) -> Result<(), EditError> {
    let at = find(edit, terminal)?;
    let ty = type_of(edit, ty)?;
    let signature = &mut edit.body_function_mut()?.signature;
    match terminal {
        Terminal::Input(_) => {
            if let Some(param) = signature.params.get_mut(at) {
                param.ty = ty;
            }
        }
        Terminal::Output(_) => {
            if let Some(output) = signature.outputs.get_mut(at) {
                output.ty = ty;
            }
        }
    }
    Ok(())
}

/// `signature.set_default`: `None` makes the input required.
pub(crate) fn set_default(edit: &mut Edit, terminal: &Terminal, default: Option<&str>) -> Result<(), EditError> {
    if matches!(terminal, Terminal::Output(_)) {
        return Err(EditError::invalid("terminal", "an output has no default value: only an input has one (spec 4)"));
    }
    let at = find(edit, terminal)?;
    let default = default.map(|text| default_of(edit, text)).transpose()?;
    if let Some(param) = edit.body_function_mut()?.signature.params.get_mut(at) {
        if param.variadic && default.is_some() {
            return Err(EditError::invalid("terminal", format!("`{}` is variadic: it takes no default value (spec 5.4)", cut(terminal.name()))));
        }
        param.default = default;
    }
    Ok(())
}
