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
use labcraft_edit::{Callable, EditError, Files, Input, Judge, Kind, Lookup, Op, Point};
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
pub const EDITING: &[&str] = &["node.add", "constant.add", "node.delete", "wire.connect", "wire.disconnect"];

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

/// A parameter that a later step builds (decision of the owner, 2026-11-15).
fn not_yet(params: &Params, name: &str, step: &str) -> Result<(), CommandError> {
    match params.get(name) {
        Some(value) if !value.is_null() => Err(CommandError::of("params.invalid", name, format!("`{name}` comes with M1 step {step}"))),
        _ => Ok(()),
    }
}

/// The operation of an editing command, from its parameters.
fn op_of(session: &Session, id: &str, params: &Params) -> Result<(Op, Target), CommandError> {
    let target = target(session, params.text("file")?)?;
    let file = target.shown.clone();
    let name = params.str("name").map(str::to_owned);
    let op = match id {
        "node.add" => {
            not_yet(params, "inside", "5")?;
            not_yet(params, "arm", "5")?;
            not_yet(params, "into", "4")?;
            Op::NodeAdd { file, call: params.text("call")?.to_owned(), at: point(params, "at")?, name }
        }
        "constant.add" => {
            not_yet(params, "inside", "5")?;
            not_yet(params, "arm", "5")?;
            Op::ConstantAdd { file, value: params.text("value")?.to_owned(), at: point(params, "at")?, name }
        }
        "node.delete" => {
            let given = params.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
            let mut nodes = Vec::with_capacity(given.len());
            for node in &given {
                let Some(node) = node.as_str() else {
                    return Err(CommandError::of("params.invalid", "nodes", "each node is named by a text: `reading`, `#close`"));
                };
                nodes.push(node.to_owned());
            }
            Op::NodeDelete { file, nodes }
        }
        "wire.connect" => {
            not_yet(params, "arm", "5")?;
            Op::WireConnect {
                file,
                from: params.text("from")?.to_owned(),
                to: params.text("to")?.to_owned(),
                replace: params.bool("replace"),
                accept_mismatch: params.bool("accept_mismatch"),
            }
        }
        "wire.disconnect" => Op::WireDisconnect { file, wire: params.text("wire")?.to_owned() },
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

/// What `edit` asks of the program, from the check of the session (architecture 12.3).
struct SessionJudge<'a> {
    session: &'a Session,
    /// The check of the root before the command.
    kept: &'a FoldersChecked,
    /// The texts of a batch not written yet.
    memory: &'a BTreeMap<PathBuf, Vec<u8>>,
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

/// `view.select`: each name of the selection names what the function shown has, as an editing command names it
/// (commands.md 9.2; decision of the owner, 2026-11-16).
pub(crate) fn selectable(session: &mut Session, nodes: &[String], wires: &[String], terminals: &[String]) -> Result<(), CommandError> {
    if nodes.is_empty() && wires.is_empty() && terminals.is_empty() {
        return Ok(());
    }
    let Some(file) = session.view.shown_file().map(str::to_owned) else {
        return Err(CommandError::of("params.invalid", "function", "the function shown has no file, so nothing of it can be selected"));
    };
    let target = target(session, &file)?;
    let text = read_text(session, &target)?;
    let root = session.root.clone();
    session.checked(&root)?;
    let kept = session.kept_root().ok_or_else(|| CommandError::internal("the check of the root is not kept"))?;
    let memory = BTreeMap::new();
    let judge = SessionJudge { session, kept: &kept.checked, memory: &memory };
    let mut files = Files::new();
    files.insert(&target.shown, &text);
    labcraft_edit::selectable(&files, &target.shown, nodes, wires, terminals, &judge, &session.config.limits).map_err(command_error)
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
        let judge = SessionJudge { session, kept: &kept.checked, memory: &memory };
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
    let names: Map<String, Value> = edited.names.iter().map(|(key, value)| ((*key).to_owned(), json!(value))).collect();
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
    let list = params.get("commands").and_then(Value::as_array).cloned().unwrap_or_default();
    if list.len() > MAX_BATCH {
        return Err(CommandError::of("params.invalid", "commands", format!("a batch holds at most {MAX_BATCH} commands")));
    }
    judge_revisions(session, params)?;
    let before = session.root_check_document()?;
    let mut texts = Texts::default();
    let mut results = Vec::with_capacity(list.len());
    let mut labels = Vec::new();
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
        let given = item.get("params").cloned().unwrap_or(Value::Null);
        if given.get("expected_revision").is_some() {
            return Err(at(CommandError::of(
                "params.unknown",
                "expected_revision",
                "the commands of a batch take no `expected_revision`: the batch does",
            )));
        }
        let judged = crate::params::judge(command, &given).map_err(at)?;
        let edited = apply_one(session, id, &judged, &mut texts).map_err(at)?;
        let (names, renamed) = names_json(&edited);
        results.push(json!({ "names": names, "renamed": renamed }));
        labels.push(command.label);
    }
    let label = params.str("label").map(str::to_owned).unwrap_or_else(|| labels.join(", "));
    let mut extra = Map::new();
    extra.insert("commands".to_owned(), Value::Array(results));
    finish(session, author, label, texts, &before, extra)
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
