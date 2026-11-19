//! What every transport of M4 shares (`docs/io.md`; `docs/architecture.md`, 10.6): the categories of io.md 8, the table of
//! io.md 3 that says what a real name reaches, and the rights of io.md 4, judged once per endpoint and per run.

use std::any::Any;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use crate::native::{Job, Mode, NativeOutcome};

/// Nothing came, or could be sent, within the timeout (since M0).
pub const TIMEOUT: &str = "io.timeout";
/// The run has no right to this endpoint or this path (io.md 4).
pub const NOT_ALLOWED: &str = "io.not_allowed";
/// A real resource opened in deterministic mode (io.md 3).
pub const NEEDS_REAL_TIME: &str = "io.needs_real_time";
/// The host has no such transport, or the name is a form that comes later (io.md 2).
pub const UNSUPPORTED: &str = "io.unsupported";
/// The other end closed the connection, or the port went away.
pub const CLOSED: &str = "io.closed";
/// Bytes read as text that are not UTF-8.
pub const INVALID_TEXT: &str = "io.invalid_text";
/// `read_until` or `read_line` reached its `max` without its terminator.
pub const TOO_LONG: &str = "io.too_long";
/// A count or a `max` below 0, a baud rate of 0, a port number of 0.
pub const INVALID_ARGUMENT: &str = "io.invalid_argument";

/// The categories shared by the transports, in the order of the table of io.md 8.
pub const CATEGORIES: [&str; 8] = [TIMEOUT, NOT_ALLOWED, NEEDS_REAL_TIME, UNSUPPORTED, CLOSED, INVALID_TEXT, TOO_LONG, INVALID_ARGUMENT];

/// Who started a run (io.md 4).
#[derive(Clone, Debug, Default)]
pub enum StartedBy {
    #[default]
    /// `labcraft-cli run`, `labcraft-cli exec`, Run in the IDE: every endpoint and every file.
    Human,
    /// An MCP client, by its name, with what the human allowed it.
    Agent { client: String, rights: Rights },
}

/// What a run asks the right to reach (io.md 4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reach {
    /// `tcp:10.0.0.5:5025`, and the name the program gave (`TCPIP0::10.0.0.5::5025::SOCKET`), which the question shows too.
    Endpoint { endpoint: String, given: String },
    /// A file or a folder outside the project, or under a hidden name, with its `..` and its links resolved by the library. An
    /// answer for a folder holds for what is under it (io.md 4, folder by folder).
    Path(PathBuf),
}

impl Reach {
    fn key(&self) -> Asked {
        match self {
            Reach::Endpoint { endpoint, .. } => Asked::Endpoint(endpoint.clone()),
            Reach::Path(path) => Asked::Path(path.clone()),
        }
    }

    fn shown(&self) -> String {
        match self {
            Reach::Endpoint { endpoint, .. } => format!("`{endpoint}`"),
            Reach::Path(path) => format!("`{}`", path.display()),
        }
    }
}

/// What an answer holds for: an endpoint, whatever name reached it, or a path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Asked {
    Endpoint(String),
    Path(PathBuf),
}

/// The question of io.md 4, which the host asks the human: in the window of the IDE, the client, the project, the endpoint
/// or the folder, and three answers. It may wait for the answer; it is called on a thread of the pool in real time, on the
/// thread of the run in deterministic mode, which then stops. It returns `false` when the run it asks for is aborted: the host
/// that aborts the run takes its question away, by the id of the run.
pub trait Consent: Send + Sync {
    fn ask(&self, question: &Question<'_>) -> bool;
}

/// What the question says: the run that asks, by its id (`r3`), the client that started it, the project, and what it would
/// reach.
#[derive(Clone, Copy, Debug)]
pub struct Question<'a> {
    pub run: &'a str,
    pub client: &'a str,
    pub project: Option<&'a Path>,
    pub reach: &'a Reach,
}

/// What the human allows an agent, as the host knows it when a run starts (io.md 4): the endpoints of `--allow-hardware`, the
/// folders of `--allow-path` (their links resolved by the host), and the question of the window for the rest. The default
/// allows nothing and asks nothing.
#[derive(Clone, Default)]
pub struct Allowed {
    pub endpoints: BTreeSet<String>,
    pub paths: Vec<PathBuf>,
    pub consent: Option<Arc<dyn Consent>>,
}

impl std::fmt::Debug for Allowed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Allowed").field("endpoints", &self.endpoints).field("paths", &self.paths).field("asks", &self.consent.is_some()).finish()
    }
}

/// The rights of an agent in one run: what was allowed, and the answers given during the run, one per endpoint and per path,
/// shared by every library of the run.
#[derive(Clone)]
pub struct Rights(Arc<Granted>);

struct Granted {
    allowed: Allowed,
    /// The run, its client and its project, which the question names.
    run: String,
    client: String,
    project: Option<PathBuf>,
    answers: Mutex<HashMap<Asked, Answer>>,
    changed: Condvar,
    /// How many times a question waited for another, for the tests.
    #[cfg(test)]
    waits: std::sync::atomic::AtomicUsize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    /// Asked, and not answered yet: a second call waits for the first.
    Waiting,
    Given(bool),
}

impl std::fmt::Debug for Rights {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Rights").field(&self.0.allowed).finish()
    }
}

impl Rights {
    /// The rights of the run `run` (its id, `r3`), started by `client` on `project`: each run starts with no answer.
    pub fn of_run(allowed: Allowed, run: &str, client: &str, project: Option<&Path>) -> Rights {
        Rights(Arc::new(Granted {
            allowed,
            run: run.to_owned(),
            client: client.to_owned(),
            project: project.map(Path::to_path_buf),
            answers: Mutex::new(HashMap::new()),
            changed: Condvar::new(),
            #[cfg(test)]
            waits: std::sync::atomic::AtomicUsize::new(0),
        }))
    }

    fn judge(&self, reach: &Reach) -> Judged {
        let allowed = &self.0.allowed;
        let listed = match reach {
            Reach::Endpoint { endpoint, .. } => allowed.endpoints.contains(endpoint),
            Reach::Path(path) => allowed.paths.iter().any(|folder| path.starts_with(folder)),
        };
        if listed {
            return Judged::Allowed;
        }
        let answers = self.0.answers.lock().unwrap_or_else(PoisonError::into_inner);
        match answer_for(&answers, reach) {
            Some(Answer::Given(true)) => Judged::Allowed,
            Some(Answer::Given(false)) => Judged::NotAllowed(self.not_allowed(reach)),
            // A question waits, or none was asked: the work of the call asks, or waits for the answer.
            Some(Answer::Waiting) | None if allowed.consent.is_some() => Judged::Ask(self.asking(reach.clone())),
            Some(Answer::Waiting) | None => Judged::NotAllowed(self.not_allowed(reach)),
        }
    }

    /// The work that asks the host, or waits for the answer to a question already asked in this run, and keeps it.
    fn asking(&self, reach: Reach) -> Job {
        let rights = self.clone();
        Box::new(move || Box::new(Answered(rights.ask(&reach))) as Box<dyn Any + Send>)
    }

    fn ask(&self, reach: &Reach) -> bool {
        let key = reach.key();
        let granted = &*self.0;
        let mut answers = granted.answers.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            match answer_for(&answers, reach) {
                Some(Answer::Given(given)) => return given,
                Some(Answer::Waiting) => {
                    #[cfg(test)]
                    granted.waits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    answers = granted.changed.wait(answers).unwrap_or_else(PoisonError::into_inner);
                }
                None => break,
            }
        }
        answers.insert(key.clone(), Answer::Waiting);
        drop(answers);
        // Should the host panic, the answer is no, and whoever waits for it is told.
        let mut pending = Pending { granted, key, given: false };
        let question = Question { run: &granted.run, client: &granted.client, project: granted.project.as_deref(), reach };
        pending.given = granted.allowed.consent.as_ref().is_some_and(|consent| consent.ask(&question));
        pending.given
    }

    fn not_allowed(&self, reach: &Reach) -> NativeOutcome {
        let how = match (reach, self.0.allowed.consent.is_some()) {
            (_, true) => "it was not allowed in the window".to_owned(),
            (Reach::Endpoint { endpoint, .. }, false) => format!("the server allows it with `--allow-hardware {endpoint}`"),
            (Reach::Path(_), false) => "the server allows a folder with `--allow-path DIR`".to_owned(),
        };
        NativeOutcome::Fail { category: NOT_ALLOWED, code: 0, message: format!("the run has no right to reach {}: {how}", reach.shown()) }
    }
}

/// The answer that holds for `reach`: its own, else for a path the answer for the nearest folder above it, given or awaited.
fn answer_for(answers: &HashMap<Asked, Answer>, reach: &Reach) -> Option<Answer> {
    if let Some(answer) = answers.get(&reach.key()) {
        return Some(*answer);
    }
    let Reach::Path(path) = reach else { return None };
    answers
        .iter()
        .filter_map(|(asked, answer)| match (asked, answer) {
            (Asked::Path(folder), answer) if path.starts_with(folder) => Some((folder.components().count(), *answer)),
            (Asked::Endpoint(_) | Asked::Path(_), _) => None,
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, answer)| answer)
}

#[cfg(test)]
impl Rights {
    fn waits(&self) -> usize {
        self.0.waits.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// A question asked: its answer is kept when the host gives it, and is no when the host panics.
struct Pending<'a> {
    granted: &'a Granted,
    key: Asked,
    given: bool,
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        let mut answers = self.granted.answers.lock().unwrap_or_else(PoisonError::into_inner);
        answers.insert(self.key.clone(), Answer::Given(self.given));
        drop(answers);
        self.granted.changed.notify_all();
    }
}

/// What the work of a question returns, which [`Context::answered`] reads.
struct Answered(bool);

/// What the host says of a run (io.md 7), the same for every library of the run. The default is a run of the human, with no
/// simulation file and no project.
#[derive(Clone, Debug, Default)]
pub struct Context {
    /// A simulation file was given (`--sim`), with a section for the library or not: the file is the whole bench (io.md 3).
    pub simulation_file: bool,
    /// The folder given to `run`, the root of the MCP server, the project of the window; none for a run of a test.
    pub project: Option<PathBuf>,
    pub started_by: StartedBy,
}

/// Whether a run may reach an endpoint or a path (io.md 4).
pub enum Judged {
    Allowed,
    /// `io.not_allowed`, code 0, whose message names what and how it is allowed.
    NotAllowed(NativeOutcome),
    /// The question of the window: the call returns `Step::Blocking` with this work, then reads its answer with
    /// [`Context::answered`].
    Ask(Job),
}

impl std::fmt::Debug for Judged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Judged::Allowed => f.write_str("Allowed"),
            Judged::NotAllowed(outcome) => f.debug_tuple("NotAllowed").field(outcome).finish(),
            Judged::Ask(_) => f.write_str("Ask(..)"),
        }
    }
}

impl Context {
    /// The right of the run to reach `reach`: the human has every right; an agent what it was allowed, or what the human
    /// answered in this run, or the question.
    pub fn judge(&self, reach: &Reach) -> Judged {
        match &self.started_by {
            StartedBy::Human => Judged::Allowed,
            StartedBy::Agent { rights, .. } => rights.judge(reach),
        }
    }

    /// What the work of [`Judged::Ask`] returned: allowed, or `io.not_allowed`. What is not such an answer is no.
    pub fn answered(&self, reach: &Reach, done: Box<dyn Any + Send>) -> Result<(), NativeOutcome> {
        let yes = done.downcast::<Answered>().is_ok_and(|answered| answered.0);
        if yes {
            return Ok(());
        }
        Err(match &self.started_by {
            StartedBy::Agent { rights, .. } => rights.not_allowed(reach),
            StartedBy::Human => {
                NativeOutcome::Fail { category: NOT_ALLOWED, code: 0, message: format!("the run has no right to reach {}", reach.shown()) }
            }
        })
    }
}

/// What the open of a real name does next (io.md 3).
pub enum Open {
    /// The simulation file describes it: its device.
    Simulated,
    /// A simulation file was given and does not describe it: the category of the library, after the timeout of the open,
    /// never the hardware.
    NotFound,
    /// The hardware, with the right of the run.
    Real,
    /// The question of io.md 4, as [`Judged::Ask`]: [`Open::Real`] once [`Context::answered`] allows it.
    Ask(Job),
    /// `io.needs_real_time` or `io.not_allowed`, at once.
    Fail(NativeOutcome),
}

impl std::fmt::Debug for Open {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Open::Simulated => f.write_str("Simulated"),
            Open::NotFound => f.write_str("NotFound"),
            Open::Real => f.write_str("Real"),
            Open::Ask(_) => f.write_str("Ask(..)"),
            Open::Fail(outcome) => f.debug_tuple("Fail").field(outcome).finish(),
        }
    }
}

/// The table of io.md 3 for a real name (`tcp:`, `serial:`, a VISA name), once the library has read the name and knows that
/// it has the transport (`io.unsupported` otherwise, which comes first): with a simulation file, the device it describes
/// or not found; without one, `io.needs_real_time` in deterministic mode, else the right of the run.
pub fn open_real(mode: Mode, context: &Context, described: bool, reach: &Reach) -> Open {
    if context.simulation_file {
        return if described { Open::Simulated } else { Open::NotFound };
    }
    if mode == Mode::Deterministic {
        return Open::Fail(needs_real_time(reach));
    }
    match context.judge(reach) {
        Judged::Allowed => Open::Real,
        Judged::NotAllowed(outcome) => Open::Fail(outcome),
        Judged::Ask(job) => Open::Ask(job),
    }
}

/// `io.needs_real_time`, code 0 (io.md 3).
pub fn needs_real_time(reach: &Reach) -> NativeOutcome {
    let message = format!(
        "{} is real hardware, which a run reaches in real time only: run with `--real-time` (`mode: \"real_time\"` in `run.start`), or describe it in a simulation file",
        reach.shown()
    );
    NativeOutcome::Fail { category: NEEDS_REAL_TIME, code: 0, message }
}

/// `io.unsupported`, code 0: `what` says what the host does not have, or which form comes later (io.md 2).
pub fn unsupported(what: &str) -> NativeOutcome {
    NativeOutcome::Fail { category: UNSUPPORTED, code: 0, message: what.to_owned() }
}

#[cfg(test)]
mod tests;
