//! The editing commands of `docs/commands.md`, section 6.7 (architecture 12.5): each judges its parameters and paths,
//! judges `expected_revision`, reads the files through the `Workspace`, calls `edit::apply`, writes each text that
//! changed (atomic writes), records a step, checks the root again and returns the result of 9.3. And `commands.batch`,
//! `commands.execute` and `history.*`.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use labcraft_check::primitives::{PRIMITIVES, PrimitiveLibrary, primitive};
use labcraft_check::{Checked, DeclaredFunction, DeclaredLibrary};
use labcraft_diagram::{DiagramLimits, Drawn, style};
use labcraft_edit::{Callable, EditError, Files, Input, Judge, Kind, Lookup, Op, Passage, Point, StructureKind, StructureOption, Through, Within};
use labcraft_types::{Diagnostic, Interner, Severity, Ty};
use serde_json::{Map, Value, json};

use crate::error::CommandError;
use crate::files::{bytes_of, text_of};
use crate::folders::FoldersChecked;
use crate::history::{Author, Change};
use crate::params::Params;
use crate::paths::{self, Place, Wanted};
use crate::registry::{self, Command};
use crate::session::{Session, check_of};
use crate::workspace::{Entry, Metadata, Read, Workspace};

/// At most this many diagnostics in a result *(proposed)*, then `truncated` (commands.md 9.3).
pub const MAX_DIAGNOSTICS: usize = 200;
/// At most this many commands in one `commands.batch` *(proposed)*.
pub const MAX_BATCH: usize = 1_000;

/// The editing commands built, which `Session::dispatch` sends here (commands.md 6.7).
pub const EDITING: &[&str] = &[
    "node.add",
    "constant.add",
    "node.delete",
    "wire.connect",
    "wire.disconnect",
    "node.move",
    "frame.resize",
    "node.place",
    "node.set_view",
    "layout.arrange",
    "wire.route",
    "input.set_constant",
    "input.clear_constant",
    "input.remove_value",
    "value.rename",
    "node.set_label",
    "node.set_template",
    "fragment.paste",
    "structure.surround",
    "structure.add_arm",
    "structure.set_patterns",
    "structure.add_carry",
    "structure.add_clause",
    "structure.add_collect",
    "structure.add_last",
    "structure.remove_part",
    "structure.set_passage",
    "structure.set_option",
];

/// The revision of a file: the SHA-256 of its bytes, `sha256:` and 64 lowercase hexadecimal digits (architecture 12.5).
pub fn revision(bytes: &[u8]) -> String {
    format!("sha256:{}", labcraft_trace::sha256_hex(bytes))
}

/// A file that a command edits, judged.
struct Target {
    path: PathBuf,
    shown: String,
}

/// The `.lcf` of the project that `file` names (commands.md 9.2).
fn target(session: &Session, text: &str) -> Result<Target, CommandError> {
    let (path, shown) = paths::existing_in_project(&*session.ws, &session.root, text, "file", Wanted::File)?;
    if !path.extension().is_some_and(|extension| extension == "lcf") {
        return Err(CommandError::of("path.invalid", "file", format!("`{shown}` is not a function file: an editing command takes a `.lcf`")));
    }
    Ok(Target { path, shown })
}

fn read_text(session: &Session, target: &Target) -> Result<String, CommandError> {
    let place = Place::Project { path: target.path.clone(), shown: target.shown.clone(), exists: true };
    let (bytes, shown) = bytes_of(session, &place, "file")?;
    text_of(bytes, "file", &shown)
}

/// The bytes of a file now, `None` when it is missing.
pub(crate) fn bytes_now(session: &Session, path: &Path) -> Result<Option<Vec<u8>>, CommandError> {
    match session.ws.read(path, session.config.limits.max_file_size) {
        Ok(Read::Bytes(bytes)) => Ok(Some(bytes)),
        Ok(Read::TooLarge(_)) => Err(CommandError::new("file.too_large", "a file of the step is over the limit of `--max-file-size`")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CommandError::new("file.unreadable", format!("a file of the step cannot be read: {}", crate::walk::io_text(&error)))),
    }
}

/// What a file of a step holds now. A file that the step expects missing, and that the system reaches only under another
/// spelling of its name (its case, on Windows and macOS), is missing: `file.rename` of `Gain.lcf` to `gain.lcf`.
fn held_now(session: &Session, path: &Path, missing: bool) -> Result<Option<Vec<u8>>, CommandError> {
    let now = bytes_now(session, path)?;
    Ok(now.filter(|_| !missing || crate::external::same_name(session, path)))
}

/// `expected_revision`: each file it names must have that revision, or the command changes nothing (`file.stale`).
fn judge_revisions(session: &Session, params: &Params) -> Result<(), CommandError> {
    let Some(expected) = params.get("expected_revision") else { return Ok(()) };
    if expected.is_null() {
        return Ok(());
    }
    let Some(expected) = expected.as_object() else {
        return Err(CommandError::of("params.invalid", "expected_revision", "`expected_revision` maps a path to a revision"));
    };
    for (file, wanted) in expected {
        let Some(wanted) = wanted.as_str() else {
            return Err(CommandError::of("params.invalid", "expected_revision", format!("the revision of `{file}` is a text")));
        };
        let (path, shown) = paths::existing_in_project(&*session.ws, &session.root, file, "expected_revision", Wanted::File)?;
        let now = bytes_now(session, &path)?.map(|bytes| revision(&bytes)).unwrap_or_default();
        if now != wanted {
            return Err(CommandError::of("file.stale", "expected_revision", format!("`{shown}` changed since that revision: it is now `{now}`")));
        }
    }
    Ok(())
}

fn point(params: &Params, name: &str) -> Result<Option<Point>, CommandError> {
    let Some(value) = params.get(name).filter(|value| !value.is_null()) else { return Ok(None) };
    let pair = value.as_array().filter(|pair| pair.len() == 2).and_then(|pair| Some((pair.first()?.as_i64()?, pair.get(1)?.as_i64()?)));
    match pair {
        Some((x, y)) => Ok(Some(Point { x, y })),
        None => Err(CommandError::of("params.invalid", name, format!("`{name}` is `[x, y]`, two integers"))),
    }
}

/// A position that the command needs.
fn required_point(params: &Params, name: &str) -> Result<Point, CommandError> {
    point(params, name)?.ok_or_else(|| CommandError::of("params.invalid", name, format!("`{name}` is `[x, y]`, two integers")))
}

/// A list of positions, `[[x, y], …]`; empty when the parameter is not given.
fn points(params: &Params, name: &str) -> Result<Vec<Point>, CommandError> {
    let Some(given) = params.get(name).filter(|value| !value.is_null()) else { return Ok(Vec::new()) };
    let invalid = || CommandError::of("params.invalid", name, format!("`{name}` is a list of points, `[x, y]` each, two integers"));
    let Some(given) = given.as_array() else { return Err(invalid()) };
    if given.len() > labcraft_edit::MAX_VIA {
        return Err(CommandError::of("params.invalid", name, format!("a route has at most {} points", labcraft_edit::MAX_VIA)));
    }
    given
        .iter()
        .map(|value| {
            let pair = value.as_array().filter(|pair| pair.len() == 2).and_then(|pair| Some((pair.first()?.as_i64()?, pair.get(1)?.as_i64()?)));
            pair.map(|(x, y)| Point { x, y }).ok_or_else(invalid)
        })
        .collect()
}

/// A list of names, each a text; none when the parameter is not given.
fn texts(params: &Params, name: &str) -> Result<Option<Vec<String>>, CommandError> {
    let Some(given) = params.get(name).filter(|value| !value.is_null()) else { return Ok(None) };
    let Some(given) = given.as_array() else {
        return Err(CommandError::of("params.invalid", name, format!("`{name}` is a list of names")));
    };
    let mut found = Vec::with_capacity(given.len());
    for value in given {
        let Some(text) = value.as_str() else {
            return Err(CommandError::of("params.invalid", name, format!("each of `{name}` is named by a text: `reading`, `#close`")));
        };
        found.push(text.to_owned());
    }
    Ok(Some(found))
}

/// Where a command puts what it adds: `inside` a structure, in its `arm` (commands.md 6.7).
fn within(params: &Params, inside: &str, arm: &str) -> Within {
    Within { inside: params.str(inside).map(str::to_owned), arm: params.str(arm).map(str::to_owned) }
}

/// `through` of `wire.connect`: `last`, `collect` or `carry` for every loop, or a list of them, one per loop (commands.md
/// 6.7).
fn through(params: &Params) -> Result<Option<Through>, CommandError> {
    let Some(given) = params.get("through").filter(|value| !value.is_null()) else { return Ok(None) };
    let invalid = || CommandError::of("params.invalid", "through", "`through` is `last`, `collect` or `carry`, or a list of them, one per loop");
    let passage = |value: &serde_json::Value| value.as_str().and_then(passage_of).ok_or_else(invalid);
    match given.as_array() {
        Some(list) if !list.is_empty() => Ok(Some(Through::Each(list.iter().map(passage).collect::<Result<_, _>>()?))),
        Some(_) => Err(invalid()),
        None => Ok(Some(Through::All(passage(given)?))),
    }
}

/// A passage out of a loop as `through` and `structure.set_passage` write it.
fn passage_of(text: &str) -> Option<Passage> {
    match text {
        "carry" => Some(Passage::Carry),
        "collect" => Some(Passage::Collect),
        "last" => Some(Passage::Last),
        _ => None,
    }
}

/// The operation of an editing command, from its parameters.
fn op_of(session: &Session, id: &str, params: &Params) -> Result<(Op, Target), CommandError> {
    let target = target(session, params.text("file")?)?;
    let file = target.shown.clone();
    let name = params.str("name").map(str::to_owned);
    let op = match id {
        "node.add" => {
            let into = params.str("into").map(str::to_owned);
            Op::NodeAdd { file, call: params.text("call")?.to_owned(), at: point(params, "at")?, name, into, within: within(params, "inside", "arm") }
        }
        "constant.add" => {
            Op::ConstantAdd { file, value: params.text("value")?.to_owned(), at: point(params, "at")?, name, within: within(params, "inside", "arm") }
        }
        "node.delete" => Op::NodeDelete { file, nodes: texts(params, "nodes")?.unwrap_or_default() },
        "node.move" => Op::NodeMove { file, nodes: texts(params, "nodes")?.unwrap_or_default(), by: required_point(params, "by")? },
        // `size` is required (the schema), and `null` removes the entry (notation 2.4).
        "frame.resize" => Op::FrameResize { file, inside: params.str("inside").map(str::to_owned), size: point(params, "size")? },
        "node.place" => Op::NodePlace { file, node: params.text("node")?.to_owned(), at: required_point(params, "at")? },
        "node.set_view" => {
            let view = match params.text("view")? {
                "standard" => Some(labcraft_graph::View::Standard),
                "compact" => Some(labcraft_graph::View::Compact),
                "default" => None,
                other => {
                    return Err(CommandError::of("params.invalid", "view", format!("`{other}` is no view: `standard`, `compact` or `default`")));
                }
            };
            Op::NodeSetView { file, nodes: texts(params, "nodes")?.unwrap_or_default(), view }
        }
        "layout.arrange" => Op::LayoutArrange {
            file,
            nodes: texts(params, "nodes")?,
            wires: texts(params, "wires")?.unwrap_or_default(),
            terminals: texts(params, "terminals")?.unwrap_or_default(),
            within: within(params, "inside", "arm"),
        },
        "wire.connect" => Op::WireConnect {
            file,
            from: params.text("from")?.to_owned(),
            to: params.text("to")?.to_owned(),
            replace: params.bool("replace"),
            accept_mismatch: params.bool("accept_mismatch"),
            arm: params.str("arm").map(str::to_owned),
            through: through(params)?,
        },
        "wire.disconnect" => Op::WireDisconnect { file, wire: params.text("wire")?.to_owned(), arm: params.str("arm").map(str::to_owned) },
        "wire.route" => Op::WireRoute { file, wire: params.text("wire")?.to_owned(), via: points(params, "via")? },
        "input.set_constant" => Op::InputSetConstant {
            file,
            to: params.text("to")?.to_owned(),
            value: params.text("value")?.to_owned(),
            replace: params.bool("replace"),
            accept_mismatch: params.bool("accept_mismatch"),
            arm: params.str("arm").map(str::to_owned),
        },
        "input.clear_constant" => Op::InputClearConstant { file, to: params.text("to")?.to_owned(), arm: params.str("arm").map(str::to_owned) },
        "input.remove_value" => Op::InputRemoveValue { file, to: params.text("to")?.to_owned() },
        "value.rename" => Op::ValueRename { file, value: params.text("value")?.to_owned(), name: params.text("name")?.to_owned() },
        "node.set_label" => {
            let line = params.u64("line").and_then(|line| u32::try_from(line).ok()).filter(|line| *line > 0);
            let Some(line) = line else {
                return Err(CommandError::of("params.invalid", "line", "`line` is a line of the file, from 1"));
            };
            Op::NodeSetLabel { file, line, label: params.text("label")?.to_owned() }
        }
        "node.set_template" => Op::NodeSetTemplate { file, node: params.text("node")?.to_owned(), template: params.text("template")?.to_owned() },
        "fragment.paste" => Op::FragmentPaste {
            file,
            text: params.text("text")?.to_owned(),
            at: required_point(params, "at")?,
            within: within(params, "inside", "arm"),
        },
        "structure.surround" => {
            let kind = params.text("kind")?;
            let Some(kind) = StructureKind::parse(kind) else {
                return Err(CommandError::of("params.invalid", "kind", format!("`{kind}` is no structure: `match`, `handle`, `for` or `loop`")));
            };
            Op::StructureSurround {
                file,
                kind,
                nodes: texts(params, "nodes")?.unwrap_or_default(),
                selector: params.str("selector").map(str::to_owned),
                over: params.str("over").map(str::to_owned),
                arm: params.str("arm").map(str::to_owned),
                within: within(params, "inside", "inside_arm"),
                at: point(params, "at")?,
            }
        }
        "structure.add_arm" => {
            Op::StructureAddArm { file, structure: params.text("structure")?.to_owned(), patterns: params.text("patterns")?.to_owned() }
        }
        "structure.set_patterns" => Op::StructureSetPatterns {
            file,
            structure: params.text("structure")?.to_owned(),
            arm: params.text("arm")?.to_owned(),
            patterns: params.text("patterns")?.to_owned(),
        },
        "structure.add_carry" => Op::StructureAddCarry {
            file,
            structure: params.text("structure")?.to_owned(),
            name: params.text("name")?.to_owned(),
            initial: params.str("initial").map(str::to_owned),
            ty: params.str("type").map(str::to_owned),
        },
        "structure.add_clause" => {
            Op::StructureAddClause { file, structure: params.text("structure")?.to_owned(), over: params.text("over")?.to_owned(), name }
        }
        "structure.add_collect" => {
            Op::StructureAddCollect { file, structure: params.text("structure")?.to_owned(), value: params.text("value")?.to_owned(), name }
        }
        "structure.add_last" => {
            Op::StructureAddLast { file, structure: params.text("structure")?.to_owned(), value: params.text("value")?.to_owned(), name }
        }
        "structure.remove_part" => {
            Op::StructureRemovePart { file, structure: params.text("structure")?.to_owned(), part: params.text("part")?.to_owned() }
        }
        "structure.set_passage" => {
            let text = params.text("to")?;
            let Some(to) = passage_of(text) else {
                return Err(CommandError::of("params.invalid", "to", format!("`{text}` is no passage: `carry`, `collect` or `last`")));
            };
            Op::StructureSetPassage { file, structure: params.text("structure")?.to_owned(), part: params.text("part")?.to_owned(), to }
        }
        "structure.set_option" => {
            let option = match params.text("option")? {
                "continue_on_error" => StructureOption::ContinueOnError,
                "parallel" => StructureOption::Parallel,
                other => {
                    return Err(CommandError::of("params.invalid", "option", format!("`{other}` is no option: `continue_on_error` or `parallel`")));
                }
            };
            let Some(on) = params.get("on").and_then(serde_json::Value::as_bool) else {
                return Err(CommandError::of("params.invalid", "on", "`on` is `true` or `false`"));
            };
            Op::StructureSetOption { file, structure: params.text("structure")?.to_owned(), option, on }
        }
        other => return Err(CommandError::internal(format!("`{other}` is no editing command"))),
    };
    Ok((op, target))
}

fn command_error(error: EditError) -> CommandError {
    let mut found = match error.parameter {
        Some(parameter) => CommandError::of(error.code, parameter, error.message),
        None => CommandError::new(error.code, error.message),
    };
    found = found.expecting(&error.expected);
    found
}

/// A workspace that reads some files from memory: the texts of a batch not written yet, or a text that a wire would
/// write, for the check that judges it. It writes nothing.
struct Overlay<'a> {
    inner: &'a dyn Workspace,
    files: &'a BTreeMap<PathBuf, Vec<u8>>,
}

impl Workspace for Overlay<'_> {
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.inner.canonicalize(path)
    }
    fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        self.inner.absolute(path)
    }
    fn current(&self) -> Option<PathBuf> {
        self.inner.current()
    }
    fn metadata(&self, path: &Path) -> io::Result<Metadata> {
        self.inner.metadata(path)
    }
    fn link_metadata(&self, path: &Path) -> io::Result<Metadata> {
        self.inner.link_metadata(path)
    }
    fn read_dir(&self, path: &Path, each: &mut dyn FnMut(io::Result<Entry>)) -> io::Result<()> {
        self.inner.read_dir(path, each)
    }
    fn read(&self, path: &Path, max: u64) -> io::Result<Read> {
        match self.files.get(path) {
            Some(bytes) if bytes.len() as u64 > max => Ok(Read::TooLarge(bytes.len() as u64)),
            Some(bytes) => Ok(Read::Bytes(bytes.clone())),
            None => self.inner.read(path, max),
        }
    }
    fn write(&self, _: &Path, _: &[u8]) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }
    fn create_folders(&self, _: &Path) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }
    fn rename(&self, _: &Path, _: &Path) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }
    fn remove_file(&self, _: &Path) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }
}

/// The last text typed by [`SessionJudge::value_type`]: its file, its text, and its check, none when every text in memory
/// is the one on the disk.
type Typed = (String, String, Option<Arc<FoldersChecked>>);

/// What `edit` asks of the program, from the check of the session (architecture 12.3).
struct SessionJudge<'a> {
    session: &'a Session,
    /// The check of the root before the command.
    kept: &'a FoldersChecked,
    /// The texts of a batch not written yet.
    memory: &'a BTreeMap<PathBuf, Vec<u8>>,
    /// The last check that `value_type` made of a text that is not the one on the disk, with that file and text: a
    /// structure asks the type of several values of one text (commands.md 9.6).
    typed: std::cell::RefCell<Option<Typed>>,
}

impl SessionJudge<'_> {
    fn checked(&self) -> Option<&(Checked, Interner)> {
        self.kept.checked.first()
    }
}

fn inputs_of(function: &DeclaredFunction, names: &Interner) -> Vec<Input> {
    let variadic_at = function.params.iter().position(|param| param.variadic);
    function
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| Input {
            name: names.resolve(param.name).to_owned(),
            required: !param.has_default && !param.variadic,
            variadic: param.variadic,
            named_only: variadic_at.is_some_and(|at| index > at),
        })
        .collect()
}

fn function_of(function: &DeclaredFunction, names: &Interner, library: Option<&str>) -> Callable {
    Callable {
        kind: Kind::Function,
        library: library.map(str::to_owned),
        inputs: inputs_of(function, names),
        outputs: function.outputs.iter().map(|output| names.resolve(output.name).to_owned()).collect(),
    }
}

/// A struct type of a library, or a variant of an enumeration that holds values.
fn type_callable(
    checked: &Checked,
    names: &Interner,
    library: &DeclaredLibrary,
    type_name: &str,
    variant: Option<&str>,
    used: Option<&str>,
) -> Option<Callable> {
    let declared = library.types.get(&names.find(type_name)?)?;
    match (checked.types.get(declared.definition), variant) {
        (Ty::Struct(fields), None) => Some(Callable {
            kind: Kind::Struct,
            library: used.map(str::to_owned),
            inputs: fields
                .iter()
                .map(|(name, _)| Input { name: names.resolve(*name).to_owned(), required: true, variadic: false, named_only: false })
                .collect(),
            outputs: vec![type_name.to_owned()],
        }),
        (Ty::Enum(variants), Some(variant)) => {
            let found = variants.iter().find(|candidate| names.resolve(candidate.name) == variant)?;
            if found.values.is_empty() {
                return None;
            }
            Some(Callable {
                kind: Kind::Variant,
                library: used.map(str::to_owned),
                inputs: found.values.iter().map(|_| Input { name: String::new(), required: true, variadic: false, named_only: false }).collect(),
                outputs: vec![type_name.to_owned()],
            })
        }
        _ => None,
    }
}

impl Judge for SessionJudge<'_> {
    fn callable(&self, file: &str, call: &str) -> Lookup {
        let segments: Vec<&str> = call.split('.').collect();
        if let [library, name] = segments.as_slice()
            && let Some(library_of) = PrimitiveLibrary::from_name(library)
        {
            return match primitive(library_of, name) {
                Some(found) => {
                    let variadic_at = found.params.iter().position(|param| param.variadic);
                    Lookup::Found(Callable {
                        kind: Kind::Function,
                        library: Some((*library).to_owned()),
                        inputs: found
                            .params
                            .iter()
                            .enumerate()
                            .map(|(index, param)| Input {
                                name: param.name.to_owned(),
                                required: param.default.is_none() && !param.variadic,
                                variadic: param.variadic,
                                named_only: variadic_at.is_some_and(|at| index > at),
                            })
                            .collect(),
                        outputs: found.outputs.unwrap_or(&[]).iter().map(|output| (*output).to_owned()).collect(),
                    })
                }
                None => Lookup::Unknown(self.nearest(call)),
            };
        }
        if let Some(found) = self.checked().and_then(|(checked, names)| find_in(checked, names, file, &segments)) {
            return Lookup::Found(found);
        }
        // A library that no file uses yet: the check finds it once the file uses it, as `node.add` will write.
        if let [library, _, ..] = segments.as_slice()
            && let Some(found) = self
                .with_use(file, library)
                .and_then(|kept| kept.checked.first().and_then(|(checked, names)| find_in(checked, names, file, &segments)))
        {
            return Lookup::Found(found);
        }
        Lookup::Unknown(self.nearest(call))
    }

    fn errors(&self, file: &str, text: &str) -> Vec<Diagnostic> {
        let session = self.session;
        let mut files = self.memory.clone();
        files.insert(session.root.join(file), text.as_bytes().to_vec());
        // The kept check says it when every text in memory is the one on the disk: the text before a command.
        let as_on_disk = files.iter().all(|(path, bytes)| bytes_now(session, path).ok().flatten().as_deref() == Some(bytes.as_slice()));
        let report = if as_on_disk { self.kept.report.diagnostics().to_vec() } else { self.check_with(&files).report.diagnostics().to_vec() };
        report.into_iter().filter(|diagnostic| diagnostic.severity() == Severity::Error && diagnostic.file == file).collect()
    }

    fn template(&self, literal: &str) -> Result<Vec<String>, String> {
        labcraft_check::template_read(&labcraft_check::read_escapes(literal))
    }

    fn places(&self, file: &str, text: &str) -> Vec<labcraft_edit::Place> {
        let Some(drawing) = self.drawing(file, text) else { return Vec::new() };
        drawing
            .placed
            .iter()
            .enumerate()
            .filter_map(|(at, placed)| {
                let (x, y) = drawing.in_frame(at)?;
                // In units, as `layout` writes them: hundredths rounded to the nearest unit.
                let unit = |hundredths: i64| (hundredths + 50).div_euclid(100);
                let absolute = Point { x: unit(placed.x), y: unit(placed.y + placed.over) };
                Some(labcraft_edit::Place { line: placed.line, name: placed.name.clone(), at: Point { x: unit(x), y: unit(y) }, absolute })
            })
            .collect()
    }

    fn frames(&self, file: &str, text: &str) -> Vec<labcraft_edit::FrameDrawn> {
        let Some(drawing) = self.drawing(file, text) else { return Vec::new() };
        // In units, rounded up: a size of fewer would be under what the frame holds.
        let unit = |hundredths: i64| (hundredths + 99).div_euclid(100);
        let size = |(width, height): (i64, i64)| Point { x: unit(width), y: unit(height) };
        let mut found = Vec::new();
        if let Some(least) = drawing.bodies.first().and_then(|body| body.least) {
            found.push(labcraft_edit::FrameDrawn { structure: None, line: 0, least: size(least) });
        }
        for placed in drawing.placed.iter().filter(|placed| !placed.holds.is_empty() && !placed.name.is_empty()) {
            found.push(labcraft_edit::FrameDrawn { structure: Some(placed.name.clone()), line: placed.line, least: size(placed.least) });
        }
        found
    }

    fn value_type(&self, file: &str, text: &str, line: u32, name: &str) -> labcraft_edit::ValueType {
        // Once per text: whether every text in memory is the one on the disk (the kept check says it), else a check of
        // them; the files are read again only for a new text.
        let mut typed = self.typed.borrow_mut();
        let known =
            typed.as_ref().filter(|(known_file, known_text, _)| known_file == file && known_text == text).map(|(_, _, checked)| checked.clone());
        let checked = match known {
            Some(checked) => checked,
            None => {
                let session = self.session;
                let mut files = self.memory.clone();
                files.insert(session.root.join(file), text.as_bytes().to_vec());
                let as_on_disk = files.iter().all(|(path, bytes)| bytes_now(session, path).ok().flatten().as_deref() == Some(bytes.as_slice()));
                let checked = (!as_on_disk).then(|| Arc::new(self.check_with(&files)));
                *typed = Some((file.to_owned(), text.to_owned(), checked.clone()));
                checked
            }
        };
        drop(typed);
        let kept = match &checked {
            Some(fresh) => &**fresh,
            None => self.kept,
        };
        let Some((checked, names)) = kept.checked.first() else { return labcraft_edit::ValueType::default() };
        let Some(index) = checked.files.iter().position(|found| found.shown == file) else { return labcraft_edit::ValueType::default() };
        value_type::of(checked, names, index, line, name)
    }

    fn wires(&self, file: &str, text: &str) -> Vec<labcraft_edit::WireDrawn> {
        let Some(drawing) = self.drawing(file, text) else { return Vec::new() };
        let mut found = Vec::new();
        for wire in drawing.groups.iter().filter_map(|group| group.wire.as_ref()) {
            let Some(body) = drawing.bodies.get(wire.body) else { continue };
            let (ox, oy) = body.origin;
            for ((_, path), entry) in wire.readers.iter().zip(&wire.entries) {
                let Some(name) = entry else { continue };
                let path = path.iter().map(|&(x, y)| (x - ox, y - oy)).collect();
                found.push(labcraft_edit::WireDrawn { name: name.clone(), value: wire.value.clone(), body: wire.body, path });
            }
        }
        found
    }
}

/// What `call` names in the check, seen from `file`.
fn find_in(checked: &Checked, names: &Interner, file: &str, segments: &[&str]) -> Option<Callable> {
    let local = checked.files.iter().find(|candidate| candidate.shown == file).and_then(|found| checked.libraries.get(found.library as usize));
    let library_named = |name: &str| checked.libraries.iter().find(|library| !library.name.is_empty() && library.name == name);
    fn function_in<'l>(library: &'l DeclaredLibrary, names: &Interner, name: &str) -> Option<&'l DeclaredFunction> {
        names.find(name).and_then(|name| library.functions.get(&name))
    }
    match segments {
        [name] => local.and_then(|library| {
            function_in(library, names, name)
                .map(|function| function_of(function, names, None))
                .or_else(|| type_callable(checked, names, library, name, None, None))
        }),
        [first, second] => match library_named(first) {
            Some(library) => function_in(library, names, second)
                .map(|function| function_of(function, names, Some(first)))
                .or_else(|| type_callable(checked, names, library, second, None, Some(first))),
            None => local.and_then(|library| type_callable(checked, names, library, first, Some(second), None)),
        },
        [first, type_name, variant] => {
            library_named(first).and_then(|library| type_callable(checked, names, library, type_name, Some(variant), Some(first)))
        }
        _ => None,
    }
}

impl SessionJudge<'_> {
    /// The drawing of the function of `file` with `text` in place of the file, in the view of the session: what the
    /// window would show (architecture 12.3). None when it does not draw.
    fn drawing(&self, file: &str, text: &str) -> Option<labcraft_diagram::Drawing> {
        let session = self.session;
        let mut files = self.memory.clone();
        files.insert(session.root.join(file), text.as_bytes().to_vec());
        let as_on_disk = files.iter().all(|(path, bytes)| bytes_now(session, path).ok().flatten().as_deref() == Some(bytes.as_slice()));
        let fresh;
        let kept = if as_on_disk {
            self.kept
        } else {
            fresh = self.check_with(&files);
            &fresh
        };
        let (checked, names) = kept.checked.first()?;
        let index = checked.files.iter().position(|found| found.shown == file)?;
        let (state, style) = session.view.state_of(file);
        let work = || style::scope(&style, || labcraft_diagram::draw(checked, index, names, &state, &DiagramLimits::default()));
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
            Ok(Drawn::Drawing(drawing)) => Some(drawing),
            _ => None,
        }
    }

    /// A check of the root with these texts in place of those on the disk.
    fn check_with(&self, files: &BTreeMap<PathBuf, Vec<u8>>) -> FoldersChecked {
        let session = self.session;
        let overlay = Overlay { inner: &*session.ws, files };
        let config = &session.config;
        check_of(&overlay, &config.shipped, &session.root, &session.root, &config.lib_paths, &config.limits)
    }

    /// A check of the root where `file` also uses `library`, when the file reads and the library is not used yet.
    fn with_use(&self, file: &str, library: &str) -> Option<FoldersChecked> {
        if !labcraft_format::is_name(library)
            || self.checked().is_some_and(|(checked, _)| checked.libraries.iter().any(|found| found.name == library))
        {
            return None;
        }
        let session = self.session;
        let path = session.root.join(file);
        let text = match self.memory.get(&path) {
            Some(bytes) => String::from_utf8(bytes.clone()).ok()?,
            None => String::from_utf8(bytes_now(session, &path).ok()??).ok()?,
        };
        // After the header line: a `use` line reads there, whatever follows.
        let (header, rest) = text.split_once('\n')?;
        let mut files = self.memory.clone();
        files.insert(path, format!("{header}\n\nuse {library}\n{rest}").into_bytes());
        Some(self.check_with(&files))
    }

    /// The name of a call nearest to `call`.
    fn nearest(&self, call: &str) -> Vec<String> {
        let mut known: Vec<String> = PRIMITIVES.iter().map(|found| format!("{}.{}", found.library.as_str(), found.name)).collect();
        if let Some((checked, names)) = self.checked() {
            for library in &checked.libraries {
                for name in library.functions.keys().chain(library.types.keys()) {
                    let name = names.resolve(*name);
                    known.push(if library.name.is_empty() { name.to_owned() } else { format!("{}.{name}", library.name) });
                }
            }
        }
        let known: Vec<&str> = known.iter().map(String::as_str).collect();
        labcraft_runtime::simulation::nearest(call, &known).into_iter().map(str::to_owned).collect()
    }
}

/// The texts of a command or a batch, as they go: the bytes before it, and the text now.
#[derive(Default)]
struct Texts {
    /// By path: its name as shown, the bytes before, the text now.
    files: BTreeMap<PathBuf, (String, Vec<u8>, String)>,
}

impl Texts {
    fn memory(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        self.files.iter().map(|(path, (_, _, text))| (path.clone(), text.as_bytes().to_vec())).collect()
    }
}

/// `view.select`: each name of the selection names what the function of the window has, in its `file`, as an editing
/// command names it (commands.md 9.2; decision of the owner, 2026-11-16).
pub(crate) fn selectable(
    session: &mut Session,
    file: Option<&str>,
    nodes: &[String],
    wires: &[String],
    terminals: &[String],
) -> Result<(), CommandError> {
    if nodes.is_empty() && wires.is_empty() && terminals.is_empty() {
        return Ok(());
    }
    let Some(file) = file else {
        return Err(CommandError::of("params.invalid", "function", "the function shown has no file, so nothing of it can be selected"));
    };
    reading(session, file, |files, shown, judge, limits| labcraft_edit::selectable(files, shown, nodes, wires, terminals, judge, limits))
}

/// What reads the file `file` of the project with the judge of the kept check of the root, writing nothing: `view.select`
/// and `fragment.copy`. `read` takes the file as `edit` reads it, its name as shown, the judge and the limits.
fn reading<T>(
    session: &mut Session,
    file: &str,
    read: impl FnOnce(&Files, &str, &SessionJudge, &labcraft_types::Limits) -> Result<T, EditError>,
) -> Result<T, CommandError> {
    let target = target(session, file)?;
    let text = read_text(session, &target)?;
    let root = session.root.clone();
    session.checked(&root)?;
    let kept = session.kept_root().ok_or_else(|| CommandError::internal("the check of the root is not kept"))?;
    let memory = BTreeMap::new();
    let judge = SessionJudge { session, kept: &kept.checked, memory: &memory, typed: Default::default() };
    let mut files = Files::new();
    files.insert(&target.shown, &text);
    read(&files, &target.shown, &judge, &session.config.limits).map_err(command_error)
}

/// `fragment.copy`: the nodes as the text of a function `clipboard` (commands.md 6.7). It writes nothing, and is not a
/// step of history.
pub(crate) fn copy(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let nodes = texts(params, "nodes")?.unwrap_or_default();
    let copied = reading(session, params.text("file")?, |files, shown, judge, limits| labcraft_edit::copy(files, shown, &nodes, judge, limits))?;
    Ok(json!({ "text": copied }))
}

/// One editing command on the texts in memory: what it named and renamed.
fn apply_one(session: &mut Session, id: &str, params: &Params, texts: &mut Texts) -> Result<labcraft_edit::Edited, CommandError> {
    let (op, target) = op_of(session, id, params)?;
    if !texts.files.contains_key(&target.path) {
        let text = read_text(session, &target)?;
        texts.files.insert(target.path.clone(), (target.shown.clone(), text.as_bytes().to_vec(), text));
    }
    let root = session.root.clone();
    let memory = texts.memory();
    // The check of the root, kept; then read through the session, which the judge borrows as a whole.
    session.checked(&root)?;
    let edited = {
        let kept = session.kept_root().ok_or_else(|| CommandError::internal("the check of the root is not kept"))?;
        let judge = SessionJudge { session, kept: &kept.checked, memory: &memory, typed: Default::default() };
        let mut files = Files::new();
        for (shown, _, text) in texts.files.values() {
            files.insert(shown, text);
        }
        labcraft_edit::apply(&files, &op, &judge, &session.config.limits).map_err(command_error)?
    };
    for (shown, text) in &edited.texts {
        if let Some((_, (_, _, now))) = texts.files.iter_mut().find(|(_, (name, _, _))| name == shown) {
            *now = text.clone();
        }
    }
    Ok(edited)
}

fn names_json(edited: &labcraft_edit::Edited) -> (Value, Value) {
    let mut names: Map<String, Value> = edited.names.iter().map(|(key, value)| ((*key).to_owned(), json!(value))).collect();
    names.extend(edited.lists.iter().map(|(key, value)| ((*key).to_owned(), json!(value))));
    let renamed: Vec<Value> = edited.renamed.iter().map(|renamed| json!({ "file": renamed.file, "from": renamed.from, "to": renamed.to })).collect();
    (Value::Object(names), Value::Array(renamed))
}

/// The diagnostics of each file, by its name as shown.
fn by_file(document: &Value) -> BTreeMap<String, Vec<Value>> {
    let mut found: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for diagnostic in document.get("diagnostics").and_then(Value::as_array).into_iter().flatten() {
        let file = diagnostic.get("file").and_then(Value::as_str).unwrap_or_default().to_owned();
        found.entry(file).or_default().push(diagnostic.clone());
    }
    found
}

/// Writes what changed, records the step, checks the root, and gives the result of 9.3.
fn finish(
    session: &mut Session,
    author: &Author,
    label: String,
    texts: Texts,
    before: &Value,
    extra: Map<String, Value>,
) -> Result<Value, CommandError> {
    let mut changes = Vec::new();
    for (path, (shown, old, now)) in texts.files {
        if old == now.as_bytes() {
            continue;
        }
        let before = shared_bytes(session, &path, old);
        changes.push(Change { path, shown, before: Some(before), after: Some(Arc::from(now.into_bytes())) });
    }
    write_changes(session, &changes, false)?;
    let step = record(session, author, label, changes.clone());
    result(session, step, &changes, false, before, extra)
}

/// The bytes of a file before a step: those of the last write of the session when they are the same, shared with the
/// history rather than copied (architecture 12.5).
pub(crate) fn shared_bytes(session: &Session, path: &Path, bytes: Vec<u8>) -> Arc<[u8]> {
    session.known.shared(path, &bytes).unwrap_or_else(|| Arc::from(bytes))
}

/// A step of history for what a command wrote, unless it wrote nothing or the session keeps no history. Its number.
pub(crate) fn record(session: &mut Session, author: &Author, label: String, changes: Vec<Change>) -> Option<u64> {
    if changes.is_empty() || !session.keeps_history {
        return None;
    }
    let time = session.config.clock.utc_ns();
    Some(session.history.record(author.clone(), label, time, changes))
}

/// The session wrote a change, or undid it: the bytes of its last write to the file, which tell an external write
/// (`external`), and the revision that its line of the journal gives.
pub(crate) fn note(session: &mut Session, change: &Change, backwards: bool) {
    let bytes = if backwards { &change.before } else { &change.after };
    session.known.wrote(&change.path, &change.shown, bytes.clone());
    session.wrote_now.push((change.shown.clone(), bytes.as_ref().map(|bytes| revision(bytes))));
}

/// Writes the bytes after each change, or before for undo; `None` removes the file.
fn write_changes(session: &mut Session, changes: &[Change], backwards: bool) -> Result<(), CommandError> {
    if changes.is_empty() {
        return Ok(());
    }
    session.forget();
    // Removals first: undoing `file.rename` of `Gain.lcf` to `gain.lcf` removes `gain.lcf`, then writes `Gain.lcf`, which is
    // one file where names are compared without their case.
    let bytes_of = |change: &Change| if backwards { change.before.is_some() } else { change.after.is_some() };
    let ordered = changes.iter().filter(|change| !bytes_of(change)).chain(changes.iter().filter(|change| bytes_of(change)));
    for change in ordered {
        let bytes = if backwards { &change.before } else { &change.after };
        let unwritable = |error: io::Error| {
            CommandError::of("file.unwritable", "file", format!("`{}` cannot be written: {}", change.shown, crate::walk::io_text(&error)))
        };
        match bytes {
            Some(bytes) => {
                if let Some(folder) = change.path.parent() {
                    session.ws.create_folders(folder).map_err(unwritable)?;
                }
                session.ws.write(&change.path, bytes).map_err(unwritable)?;
            }
            None => session.ws.remove_file(&change.path).map_err(unwritable)?,
        }
        note(session, change, backwards);
    }
    Ok(())
}

/// The result of 9.3 for the files of a step.
fn result(
    session: &mut Session,
    step: Option<u64>,
    changes: &[Change],
    backwards: bool,
    before: &Value,
    extra: Map<String, Value>,
) -> Result<Value, CommandError> {
    let document = session.root_check_document()?;
    let files: Vec<Value> = changes
        .iter()
        .map(|change| {
            let bytes = if backwards { &change.before } else { &change.after };
            json!({ "file": change.shown, "revision": bytes.as_ref().map(|bytes| revision(bytes)) })
        })
        .collect();
    let written: BTreeSet<&str> = changes.iter().map(|change| change.shown.as_str()).collect();
    let (old, new) = (by_file(before), by_file(&document));
    let mut diagnostics = Vec::new();
    for (file, list) in &new {
        if written.contains(file.as_str()) || old.get(file) != Some(list) {
            diagnostics.extend(list.iter().cloned());
        }
    }
    let truncated = diagnostics.len() > MAX_DIAGNOSTICS;
    diagnostics.truncate(MAX_DIAGNOSTICS);
    let summary = document.get("summary").cloned().unwrap_or(Value::Null);
    let mut out = Map::new();
    out.insert("step".to_owned(), json!(step));
    out.insert("files".to_owned(), Value::Array(files));
    out.extend(extra);
    out.insert("diagnostics".to_owned(), Value::Array(diagnostics));
    out.insert("summary".to_owned(), summary);
    out.insert("truncated".to_owned(), json!(truncated));
    Ok(Value::Object(out))
}

/// An editing command of 6.7.
pub(crate) fn edit(session: &mut Session, author: &Author, command: &Command, params: &Params) -> Result<Value, CommandError> {
    judge_revisions(session, params)?;
    let before = session.root_check_document()?;
    let mut texts = Texts::default();
    let edited = apply_one(session, command.id, params, &mut texts)?;
    let (names, renamed) = names_json(&edited);
    let mut extra = Map::new();
    extra.insert("names".to_owned(), names);
    extra.insert("renamed".to_owned(), renamed);
    finish(session, author, command.label.to_owned(), texts, &before, extra)
}

/// `commands.batch`: its commands on the texts in memory, written once at the end, one step (commands.md 6.7).
pub(crate) fn batch(session: &mut Session, author: &Author, params: &Params) -> Result<Value, CommandError> {
    let mut list = params.get("commands").and_then(Value::as_array).cloned().unwrap_or_default();
    if list.len() > MAX_BATCH {
        return Err(CommandError::of("params.invalid", "commands", format!("a batch holds at most {MAX_BATCH} commands")));
    }
    judge_revisions(session, params)?;
    let before = session.root_check_document()?;
    let mut texts = Texts::default();
    let mut early = label_first(session, &mut list, &mut texts);
    let mut results = Vec::with_capacity(list.len());
    let mut labels = Vec::new();
    // The node that each command named (`names.node`), for the `$i` of a later one.
    let mut named: Vec<Option<String>> = Vec::with_capacity(list.len());
    for (index, item) in list.iter().enumerate() {
        let at = |error: CommandError| CommandError { index: u32::try_from(index).ok(), ..error };
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| at(CommandError::of("params.invalid", "commands", "each command is `{ \"id\", \"params\" }`")))?;
        let command =
            registry::command(id).ok_or_else(|| at(CommandError::of("command.unknown", "commands", format!("no command has the id `{id}`"))))?;
        // The commands that edit, built or planned; a planned one is not enabled yet. The commands of files make steps too,
        // but write the disk at once: they are not commands of 6.7.
        if !command.undoable || id == "commands.batch" || id.starts_with("file.") {
            return Err(at(CommandError::of(
                "params.invalid",
                "commands",
                format!("`{id}` does not go in a batch: it takes the commands that edit (6.7)"),
            )));
        }
        if let Some(reason) = session.not_enabled(command) {
            return Err(at(CommandError::new("command.not_enabled", format!("`{id}` is not enabled: {reason}"))));
        }
        let mut given = item.get("params").cloned().unwrap_or(Value::Null);
        earlier_names(&mut given, &named, index).map_err(at)?;
        if given.get("expected_revision").is_some() {
            return Err(at(CommandError::of(
                "params.unknown",
                "expected_revision",
                "the commands of a batch take no `expected_revision`: the batch does",
            )));
        }
        let judged = crate::params::judge(command, &given).map_err(at)?;
        let edited = apply_one(session, id, &judged, &mut texts).map_err(at)?;
        let (names, mut renamed) = names_json(&edited);
        // A node named earlier and renamed now (folded into an output, given a label) keeps its `$i`: the name it had
        // may be taken by another node later in the batch. What a paste renames is of its fragment, not a node already
        // there.
        for earlier in named.iter_mut().flatten().filter(|_| id != "fragment.paste") {
            // In their order: a node given a label then folded is renamed twice by one command.
            for change in &edited.renamed {
                if change.from == *earlier {
                    earlier.clone_from(&change.to);
                }
            }
        }
        named.push(edited.names.get("node").cloned());
        if let (Some(given), Value::Array(list)) = (early.remove(&index), &mut renamed) {
            list.splice(0..0, given);
        }
        results.push(json!({ "names": names, "renamed": renamed }));
        labels.push(command.label);
    }
    let label = params.str("label").map(str::to_owned).unwrap_or_else(|| labels.join(", "));
    let mut extra = Map::new();
    extra.insert("commands".to_owned(), Value::Array(results));
    finish(session, author, label, texts, &before, extra)
}

/// The parameters of the editing commands that name nodes, wires or inputs (commands.md 9.2): the only ones where a batch
/// reads `@N`, so that a constant, a call, a label or a path that holds `@` is never touched.
const NAMING: &[&str] = &["node", "nodes", "wire", "wires", "from", "to", "into", "inside", "structure"];

/// Each `$i` of the naming parameters of `params` replaced by the node that command `i` of the batch named, an earlier one
/// (commands.md 6.7; decision of the owner, 2026-11-17): `{ "to": "$0.a" }` after a `node.add`. A `$i` of a command that
/// is not earlier, or that named no node, is `params.invalid`.
fn earlier_names(params: &mut Value, named: &[Option<String>], index: usize) -> Result<(), CommandError> {
    let Some(map) = params.as_object_mut() else { return Ok(()) };
    let mut budget = EARLIER_BUDGET;
    for key in NAMING {
        if let Some(value) = map.get_mut(*key) {
            replace_earlier(value, key, named, index, &mut budget)?;
        }
    }
    Ok(())
}

/// The bytes that the `$i` of one command may bring in all *(proposed)*: a batch never grows past its request by much.
const EARLIER_BUDGET: usize = 1 << 20;

/// A `$i` is a whole name, at the start of a value (`$0`, `$0.a`) or of the end of a wire after its arrow (`x -> $0.a`):
/// once in each end, so that a value never grows by more than two names.
fn replace_earlier(value: &mut Value, key: &str, named: &[Option<String>], index: usize, budget: &mut usize) -> Result<(), CommandError> {
    match value {
        Value::String(text) => {
            let found = tokens(text, '$');
            let starts: Vec<usize> = std::iter::once(0)
                .chain(text.find("->").map(|arrow| arrow + 2 + text.get(arrow + 2..).map_or(0, |rest| rest.len() - rest.trim_start().len())))
                .collect();
            // A whole name: a `$i` ends where the name does (`$0`, `$0.a`, not `$0abc`), and its number has no leading zero.
            let whole = |&(start, end, _): &(usize, usize, u32)| {
                starts.contains(&start)
                    && text.get(end..).and_then(|rest| rest.chars().next()).is_none_or(|next| next == '.' || next.is_whitespace())
                    && !text.get(start + 1..end).is_some_and(|digits| digits.len() > 1 && digits.starts_with('0'))
            };
            if found.len() > 2 || !found.iter().all(whole) {
                return Err(CommandError::of(
                    "params.invalid",
                    key,
                    "a `$i` is the whole name of a node, at the start of a value or of the end of a wire (`$0`, `$0.a`, `x -> $0.a`)",
                ));
            }
            let mut out = String::with_capacity(text.len());
            let mut last = 0;
            for (start, end, earlier) in found {
                let name = usize::try_from(earlier).ok().filter(|&earlier| earlier < index).and_then(|earlier| named.get(earlier).cloned().flatten());
                let Some(name) = name else {
                    return Err(CommandError::of(
                        "params.invalid",
                        key,
                        format!("`${earlier}` names the node that an earlier command of the batch named, and command {earlier} is not one that did"),
                    ));
                };
                out.push_str(text.get(last..start).unwrap_or_default());
                *budget = budget.checked_sub(name.len()).ok_or_else(|| {
                    CommandError::of("params.invalid", key, format!("the `$i` of a command name at most {EARLIER_BUDGET} bytes in all"))
                })?;
                out.push_str(&name);
                last = end;
            }
            out.push_str(text.get(last..).unwrap_or_default());
            *text = out;
        }
        Value::Array(list) => {
            for value in list {
                replace_earlier(value, key, named, index, budget)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// The lines that the naming parameters of `params` name as nodes do (`@41`, `x -> @41.b`), each once.
fn lines_in(params: &Value, found: &mut BTreeSet<u32>) {
    for key in NAMING {
        if let Some(value) = params.get(*key) {
            lines_named(value, found);
        }
    }
}

/// The lines that a value names as nodes do, each once.
fn lines_named(value: &Value, found: &mut BTreeSet<u32>) {
    match value {
        Value::String(text) => found.extend(line_tokens(text).into_iter().map(|(_, _, line)| line)),
        Value::Array(list) => list.iter().for_each(|value| lines_named(value, found)),
        Value::Object(map) => map.values().for_each(|value| lines_named(value, found)),
        _ => {}
    }
}

/// Each `@N` of a text, where it starts and ends, and `N`: an `@` that no letter, digit or `_` comes before, then digits.
fn line_tokens(text: &str) -> Vec<(usize, usize, u32)> {
    tokens(text, '@')
}

/// Each `sigil` and the number after it (`@41`, `$0`), where it starts and ends, and the number: a `sigil` that no letter,
/// digit or `_` comes before, then digits.
fn tokens(text: &str, sigil: char) -> Vec<(usize, usize, u32)> {
    let mut found = Vec::new();
    let mut previous: Option<char> = None;
    for (at, letter) in text.char_indices() {
        let free = previous.is_none_or(|before| !(before.is_alphanumeric() || before == '_'));
        if letter == sigil && free {
            let rest = text.get(at + 1..).unwrap_or_default();
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(line) = digits.parse::<u32>() {
                found.push((at, at + 1 + digits.len(), line));
            }
        }
        previous = Some(letter);
    }
    found
}

/// Each `@N` of the strings of `value` replaced by how `renamed` names that node now.
fn substitute(value: &mut Value, renamed: &BTreeMap<String, String>) {
    match value {
        Value::String(text) => {
            let mut out = String::with_capacity(text.len());
            let mut last = 0;
            for (start, end, _) in line_tokens(text) {
                let token = text.get(start..end).unwrap_or_default();
                if let Some(name) = renamed.get(token) {
                    out.push_str(text.get(last..start).unwrap_or_default());
                    out.push_str(name);
                    last = end;
                }
            }
            out.push_str(text.get(last..).unwrap_or_default());
            *text = out;
        }
        Value::Array(list) => list.iter_mut().for_each(|value| substitute(value, renamed)),
        Value::Object(map) => map.values_mut().for_each(|value| substitute(value, renamed)),
        _ => {}
    }
}

/// Before the commands of a batch, the nodes that they name by their line (`@N`) and that the `layout` section cannot
/// name get their labels, in one pass over each file as it was, and each `@N` is replaced by how the node is named
/// then: a label that a command gives would otherwise move lines (spec 5.2), and a later `@N` name another node
/// (commands.md 6.7; decision of the owner, 2026-11-17). What each command gave a label to is in its `renamed`, by the
/// index of the first command that names it. A file that cannot be labelled so is left to the commands, which say why.
fn label_first(session: &mut Session, list: &mut [Value], texts: &mut Texts) -> BTreeMap<usize, Vec<Value>> {
    let mut by_file: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    for item in list.iter() {
        let Some(params) = item.get("params") else { continue };
        let Some(file) = params.get("file").and_then(Value::as_str) else { continue };
        let mut lines = BTreeSet::new();
        lines_in(params, &mut lines);
        if !lines.is_empty() {
            by_file.entry(file.to_owned()).or_default().extend(lines);
        }
    }
    let mut early: BTreeMap<usize, Vec<Value>> = BTreeMap::new();
    for (file, lines) in by_file {
        let Ok(target) = target(session, &file) else { continue };
        if !texts.files.contains_key(&target.path) {
            let Ok(text) = read_text(session, &target) else { continue };
            texts.files.insert(target.path.clone(), (target.shown.clone(), text.as_bytes().to_vec(), text));
        }
        let Some((shown, _, now)) = texts.files.get(&target.path) else { continue };
        let mut files = Files::new();
        files.insert(shown, now);
        let lines: Vec<u32> = lines.into_iter().collect();
        let Ok(labelled) = labcraft_edit::label_lines(&files, shown, &lines, &session.config.limits) else { continue };
        let renamed: BTreeMap<String, String> = labelled.renamed.iter().map(|renamed| (renamed.from.clone(), renamed.to.clone())).collect();
        let given = labelled.texts.into_values().next();
        let mut said = BTreeSet::new();
        for (index, item) in list.iter_mut().enumerate() {
            if item.get("params").and_then(|params| params.get("file")).and_then(Value::as_str) != Some(file.as_str()) {
                continue;
            }
            let mut named = BTreeSet::new();
            if let Some(params) = item.get("params") {
                lines_in(params, &mut named);
            }
            // Only the nodes that got their label now: a node that had one is named by it already.
            let new = labelled.lists.get("labelled").cloned().unwrap_or_default();
            for renamed in labelled.renamed.iter().filter(|renamed| new.contains(&renamed.from)) {
                let line = renamed.from.trim_start_matches('@').parse::<u32>().unwrap_or_default();
                if named.contains(&line) && said.insert(line) {
                    early.entry(index).or_default().push(json!({ "file": renamed.file, "from": renamed.from, "to": renamed.to }));
                }
            }
            if let Some(params) = item.get_mut("params") {
                for key in NAMING {
                    if let Some(value) = params.get_mut(*key) {
                        substitute(value, &renamed);
                    }
                }
            }
        }
        if let (Some(given), Some((_, _, now))) = (given, texts.files.get_mut(&target.path)) {
            *now = given;
        }
    }
    early
}

/// `commands.execute`: the command `id`, as its own tool would run it (commands.md 6.7).
pub(crate) fn execute(session: &mut Session, author: &Author, params: &Params) -> Result<Value, CommandError> {
    let id = params.text("id")?;
    if matches!(id, "commands.execute" | "commands.batch") {
        return Err(CommandError::of("params.invalid", "id", format!("`{id}` is not run through `commands.execute`")));
    }
    let given = params.get("params").cloned().unwrap_or(Value::Null);
    session.execute_as(author, id, &given)
}

fn step_text(step: &crate::history::Step) -> String {
    format!("the step {} ({}, by {})", step.number, step.label, step.author.name())
}

/// `history.undo`: the last step, whoever made it, for the user; only its own, for an agent (9.5).
pub(crate) fn undo(session: &mut Session, author: &Author) -> Result<Value, CommandError> {
    let Some(step) = session.history.to_undo().cloned() else {
        return Err(CommandError::new("history.empty", "there is nothing to undo"));
    };
    if author.is_agent() && step.author != *author {
        return Err(CommandError::new("history.not_yours", format!("the last step is not yours: {}", step_text(&step))));
    }
    for change in &step.changes {
        let now = held_now(session, &change.path, change.after.is_none())?;
        if now.as_deref() != change.after.as_deref() {
            return Err(CommandError::new("file.stale", format!("`{}` changed since {}", change.shown, step_text(&step))));
        }
    }
    let before = session.root_check_document()?;
    write_changes(session, &step.changes, true)?;
    session.history.undo(author.clone());
    let mut extra = Map::new();
    extra.insert("author".to_owned(), json!(step.author.name()));
    extra.insert("label".to_owned(), json!(step.label));
    result(session, Some(step.number), &step.changes, true, &before, extra)
}

/// `history.redo`: the last step undone; for an agent, only one it undid itself (9.5).
pub(crate) fn redo(session: &mut Session, author: &Author) -> Result<Value, CommandError> {
    let Some((step, by)) = session.history.to_redo().cloned() else {
        return Err(CommandError::new("history.empty", "there is nothing to redo"));
    };
    if author.is_agent() && by != *author {
        return Err(CommandError::new("history.not_yours", format!("{} was undone by {}, not by you", step_text(&step), by.name())));
    }
    for change in &step.changes {
        let now = held_now(session, &change.path, change.before.is_none())?;
        if now.as_deref() != change.before.as_deref() {
            return Err(CommandError::new("file.stale", format!("`{}` changed since {} was undone", change.shown, step_text(&step))));
        }
    }
    let before = session.root_check_document()?;
    write_changes(session, &step.changes, false)?;
    session.history.redo();
    let mut extra = Map::new();
    extra.insert("author".to_owned(), json!(step.author.name()));
    extra.insert("label".to_owned(), json!(step.label));
    result(session, Some(step.number), &step.changes, false, &before, extra)
}

/// `history.list`.
pub(crate) fn list(session: &Session, params: &Params) -> Value {
    let limit = usize::try_from(params.u64("limit").unwrap_or(50)).unwrap_or(usize::MAX);
    let steps: Vec<Value> = session
        .history
        .steps()
        .take(limit)
        .map(|step| {
            json!({
                "step": step.number,
                "author": step.author.name(),
                "label": step.label,
                "files": step.changes.iter().map(|change| change.shown.as_str()).collect::<Vec<_>>(),
                "time": labcraft_trace::number::timestamp_text(step.time_ns),
            })
        })
        .collect();
    json!({
        "steps": steps,
        "undo": session.history.to_undo().map(|step| step.number),
        "redo": session.history.to_redo().map(|(step, _)| step.number),
        "truncated": session.history.len() > limit,
    })
}

#[cfg(test)]
mod tests;
mod value_type;
