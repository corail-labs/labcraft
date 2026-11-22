//! The agents attached to the window (ide.md 8; architecture 12.8): the control channel, on `127.0.0.1`, that
//! `labcraft-cli mcp` finds by the file of [`labcraft_cli::attach`]; for each connection, the MCP server of `mcp`, its
//! calls run by the session of the window and signed by the agent; the policy of each client, and the questions that
//! wait for the user, which the window shows ([`labcraft_ide::agents`]).
//!
//! **Threads.** One accepts the connections; each connection has its own, which reads and relays as `mcp::serve_with`
//! does, its calls sent to the thread of the session as [`Message::Agent`]. A call waits there for the policy of its
//! client, then, if the policy says so, for the user's answer, as long as the agent waits: never on the thread of the
//! window, nor on the thread that reads the agent. A question unanswered in [`ANSWER_WAIT`] is a refusal.
//!
//! **Policies kept.** The window gives the agents `ai.policies` (commands.md 9.7) whenever it changes: a client that
//! has one for this project is given it at its first connection, without a question.
//!
//! **Runs.** The session of the window never waits for a run of an agent: `run.start` goes to it with `wait` `0 s`,
//! and the wait, as `run.wait` and `run.trace`, is answered off it, by the runs of the session, also when they come by
//! `commands.execute`. A call on a run that the server answers at once is judged without waiting: on the agent's own
//! run, or under `All`, it runs; otherwise it is refused. As the connection closes, a run that the agent started and
//! that still goes is aborted; one of the user is not.

use std::io::BufReader;
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use labcraft_cli::attach;
use labcraft_engine::registry::{self, Effect};
use labcraft_engine::settings::kept_policy;
use labcraft_engine::{Author, CommandError, Waiter};
use labcraft_ide::agents::{Answer, Asks, Connected, Policy, Question, runs};
use labcraft_mcp::{Executor, Gate, Served};
use serde_json::{Value, json};

use crate::held::{Call, Flags};
use crate::session::Message;

/// Why a connection is refused while AI is off (ide.md 8): `labcraft-cli mcp` says it, and stops.
pub const AI_OFF: &str = "AI is turned off in this window of LabCraft (Settings, AI agents)";

/// How long a question waits for the user's answer *(proposed)* (ide.md 8): unanswered, the agent is refused.
pub const ANSWER_WAIT: Duration = Duration::from_secs(60);

/// The connections served at once *(proposed)*: one more is refused.
pub const MAX_CONNECTIONS: usize = 16;

/// The connections that have not proved they hold the token yet, at once *(proposed)*: one more is closed at once, so
/// that a local process cannot make the window hold threads without end.
pub const MAX_GREETING: usize = 8;

/// How often a call that waits for the user looks whether its agent still waits.
const LOOK: Duration = Duration::from_millis(100);

/// What the policy of a connection is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Standing {
    /// No name yet: it has not said `initialize`, nor called anything.
    Unnamed,
    /// The question of its first connection waits: its id.
    Asking(u64),
    Given(Policy),
    Refused,
}

struct Connection {
    id: u64,
    /// The name of its client, which `ai.policies` keys, and the one it signs with, once named.
    client: Option<(String, String)>,
    standing: Standing,
    /// The runs it started.
    runs: Vec<String>,
    /// Its stream, which the user's disconnection, or AI turned off, shuts.
    stream: Option<TcpStream>,
}

impl Connection {
    /// Its calls are refused from now on, and its connection closes.
    fn refuse(&mut self) {
        self.standing = Standing::Refused;
        if let Some(stream) = &self.stream {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

struct Waiting {
    question: Question,
    connection: u64,
    asked: Instant,
    answer: Option<Answer>,
}

#[derive(Default)]
struct State {
    next: u64,
    connections: Vec<Connection>,
    questions: Vec<Waiting>,
    /// `ai.policies`, as the window last gave it.
    kept: Value,
}

impl State {
    fn next_id(&mut self) -> u64 {
        self.next = self.next.wrapping_add(1);
        self.next
    }

    fn connection(&mut self, id: u64) -> Option<&mut Connection> {
        self.connections.iter_mut().find(|connection| connection.id == id)
    }
}

/// The agents of the window: their connections, their policies, the questions that wait.
pub struct Agents {
    /// The canonical folder of the project, as `ai.policies` keys it.
    project: String,
    /// How long a question waits: [`ANSWER_WAIT`], shorter in the tests.
    wait: Duration,
    state: Mutex<State>,
    changed: Condvar,
    /// The questions changed since the window last took them.
    asked: AtomicBool,
    /// The switch of AI (`ai.enabled`, ide.md 8): off, a connection is refused once it proved it holds the token.
    enabled: AtomicBool,
    /// Asks the window for a frame: a question came or went.
    wake: Box<dyn Fn() + Send + Sync>,
}

impl Agents {
    pub fn new(project: &Path, wake: Box<dyn Fn() + Send + Sync>) -> Agents {
        Agents::waiting(project, ANSWER_WAIT, wake)
    }

    /// As [`Agents::new`], a question waiting `wait`.
    pub fn waiting(project: &Path, wait: Duration, wake: Box<dyn Fn() + Send + Sync>) -> Agents {
        Agents {
            project: project.to_string_lossy().into_owned(),
            wait,
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            asked: AtomicBool::new(false),
            enabled: AtomicBool::new(true),
            wake,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The questions changed: the window takes them again.
    fn questions_changed(&self) {
        self.asked.store(true, Ordering::SeqCst);
        (self.wake)();
    }

    /// The policies the user kept, `ai.policies` (commands.md 9.7), as the window holds them now.
    pub fn keep(&self, policies: Value) {
        self.lock().kept = policies;
    }

    /// A connection that proved it holds the token: its id; why it is refused when AI is off, or when
    /// [`MAX_CONNECTIONS`] are served.
    fn connected(&self) -> Result<u64, String> {
        if !self.enabled() {
            return Err(AI_OFF.to_owned());
        }
        let mut state = self.lock();
        if state.connections.len() >= MAX_CONNECTIONS {
            return Err(format!("{MAX_CONNECTIONS} agents are attached already"));
        }
        let id = state.next_id();
        state.connections.push(Connection { id, client: None, standing: Standing::Unnamed, runs: Vec::new(), stream: None });
        Ok(id)
    }

    /// The stream of a connection admitted, which [`Agents::disconnect`] shuts. AI turned off meanwhile: shut at once.
    fn hold_stream(&self, connection: u64, stream: TcpStream) {
        let mut state = self.lock();
        let off = !self.enabled();
        if let Some(held) = state.connection(connection) {
            held.stream = Some(stream);
            // Refused before its stream was held: by the switch, or by the user.
            if off || held.standing == Standing::Refused {
                held.refuse();
            }
        }
    }

    /// The connection closed: its questions go.
    fn disconnected(&self, connection: u64) {
        let mut state = self.lock();
        state.connections.retain(|held| held.id != connection);
        state.questions.retain(|waiting| waiting.connection != connection);
        drop(state);
        self.changed.notify_all();
        self.questions_changed();
    }

    /// The agents attached, as the pill of the window shows them (ide.md 8): those that gave their name and that the user
    /// did not refuse, in the order they came.
    pub fn connected_list(&self) -> Vec<Connected> {
        let state = self.lock();
        state
            .connections
            .iter()
            .filter_map(|held| {
                let (client, signed) = held.client.clone()?;
                let policy = match held.standing {
                    Standing::Given(policy) => Some(policy),
                    Standing::Asking(_) => None,
                    Standing::Unnamed | Standing::Refused => return None,
                };
                Some(Connected { id: held.id, client, signed, project: self.project.clone(), policy })
            })
            .collect()
    }

    /// The user disconnects an agent, from the pill: its calls are refused, and its connection closes.
    pub fn disconnect(&self, connection: u64) {
        let mut state = self.lock();
        if let Some(held) = state.connection(connection) {
            held.refuse();
        }
        state.questions.retain(|waiting| waiting.connection != connection);
        drop(state);
        self.changed.notify_all();
        self.questions_changed();
    }

    /// The user gives an agent another policy, from the pill, for this connection (ide.md 8): a call of it that waits for
    /// the user's answer and that the new policy lets run, runs. One whose first connection is still asked is let be:
    /// the question answers it.
    pub fn set_policy(&self, connection: u64, policy: Policy) {
        let mut state = self.lock();
        let Some(held) = state.connection(connection).filter(|held| matches!(held.standing, Standing::Given(_))) else { return };
        held.standing = Standing::Given(policy);
        for waiting in state.questions.iter_mut().filter(|waiting| waiting.connection == connection && waiting.answer.is_none()) {
            if let Asks::Command { effects, .. } = &waiting.question.asks
                && !asks(policy, effects)
            {
                waiting.answer = Some(Answer::Allow);
            }
        }
        drop(state);
        self.changed.notify_all();
        self.questions_changed();
    }

    /// The switch of AI (`ai.enabled`, ide.md 8): off, every agent attached is disconnected and the questions go, and a
    /// new connection is refused, so that `labcraft-cli mcp` stops rather than edit the project from a session of its
    /// own.
    pub fn set_enabled(&self, on: bool) {
        if self.enabled.swap(on, Ordering::SeqCst) == on || on {
            return;
        }
        let mut state = self.lock();
        state.connections.iter_mut().for_each(Connection::refuse);
        state.questions.clear();
        drop(state);
        self.changed.notify_all();
        self.questions_changed();
    }

    /// Whether AI is on.
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// The name a connection signs with: that of its client, numbered when another connection signs with it already
    /// (`agent`, `agent 2`); and its policy, kept or asked.
    fn introduce(&self, connection: u64, author: Author) -> Author {
        let Author::Agent(name) = author else { return author };
        let mut state = self.lock();
        let taken = |state: &State, candidate: &str| {
            state.connections.iter().any(|held| held.id != connection && held.client.as_ref().is_some_and(|(_, signed)| signed == candidate))
        };
        // The pill names it from now on.
        (self.wake)();
        let mut signed = name.clone();
        let mut number = 1_u32;
        while taken(&state, &signed) {
            number = number.saturating_add(1);
            signed = format!("{name} {number}");
        }
        if let Some(held) = state.connection(connection) {
            held.client = Some((name.clone(), signed.clone()));
        }
        let standing = state.connection(connection).map(|held| held.standing);
        match standing {
            Some(Standing::Unnamed) => self.ask_connect(&mut state, connection, &name, &signed),
            // Asked before `initialize`, by the name `agent`: asked again by its name.
            Some(Standing::Asking(question)) => {
                state.questions.retain(|waiting| waiting.question.id != question);
                if let Some(held) = state.connection(connection) {
                    held.standing = Standing::Unnamed;
                }
                self.ask_connect(&mut state, connection, &name, &signed);
                self.changed.notify_all();
            }
            _ => {}
        }
        Author::Agent(signed)
    }

    /// The policy of a connection still unnamed: the one kept for its client on this project, else the question of
    /// its first connection. The state held.
    fn ask_connect(&self, state: &mut State, connection: u64, client: &str, signed: &str) {
        if let Some(policy) = kept_policy(&state.kept, client, &self.project).and_then(Policy::named) {
            if let Some(held) = state.connection(connection) {
                held.standing = Standing::Given(policy);
            }
            return;
        }
        let id = self.ask(state, connection, client, signed, Asks::Connect);
        if let Some(held) = state.connection(connection) {
            held.standing = Standing::Asking(id);
        }
    }

    /// Asks the user, the state held: the id of the question.
    #[expect(clippy::disallowed_methods, reason = "the desktop app, never built for WASM, where the clock traps")]
    fn ask(&self, state: &mut State, connection: u64, client: &str, signed: &str, asks: Asks) -> u64 {
        let id = state.next_id();
        let question = Question {
            id,
            client: client.to_owned(),
            signed: signed.to_owned(),
            project: self.project.clone(),
            asks,
            seconds_left: self.wait.as_secs(),
        };
        state.questions.push(Waiting { question, connection, asked: Instant::now(), answer: None });
        self.questions_changed();
        id
    }

    /// Waits for the answer to the question `id`, at most until it expires, and while `waiter` still waits; none: not
    /// answered, or its connection went. The question goes once answered.
    fn wait_for<'a>(&'a self, mut state: MutexGuard<'a, State>, id: u64, waiter: &dyn Waiter) -> (MutexGuard<'a, State>, Option<Answer>) {
        loop {
            let Some(at) = state.questions.iter().position(|waiting| waiting.question.id == id) else { return (state, None) };
            let Some((answer, asked)) = state.questions.get(at).map(|waiting| (waiting.answer, waiting.asked)) else { return (state, None) };
            let left = self.wait.saturating_sub(asked.elapsed());
            if answer.is_some() || left.is_zero() {
                state.questions.remove(at);
                self.questions_changed();
                return (state, answer);
            }
            if waiter.stopped() {
                // The question of a call goes with it; that of the first connection is the connection's, and stays.
                if state.questions.get(at).is_some_and(|waiting| waiting.question.asks != Asks::Connect) {
                    state.questions.remove(at);
                    self.questions_changed();
                }
                return (state, None);
            }
            state = self.changed.wait_timeout(state, left.min(LOOK)).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    /// The name of the client of `author`, and the one it signs with.
    fn names(state: &mut State, connection: u64, author: &Author) -> (String, String) {
        state.connection(connection).and_then(|held| held.client.clone()).unwrap_or_else(|| {
            let name = match author {
                Author::Agent(name) => name.clone(),
                _ => "agent".to_owned(),
            };
            (name.clone(), name)
        })
    }

    /// Whether the call `id` of the connection may run (ide.md 8): it waits for the policy of the client, then, when
    /// the policy says so, for the user's answer, as long as `waiter` waits.
    fn allow(&self, connection: u64, author: &Author, id: &str, params: &Value, waiter: &dyn Waiter) -> Result<(), CommandError> {
        let mut state = self.lock();
        let (client, signed) = Agents::names(&mut state, connection, author);
        let policy = loop {
            let standing = state.connection(connection).map_or(Standing::Refused, |held| held.standing);
            match standing {
                Standing::Given(policy) => break policy,
                Standing::Refused => return Err(not_allowed(format!("the user did not let {signed} work on this project"))),
                // A call before `initialize`: the first connection asks, by the name it signs with.
                Standing::Unnamed => self.ask_connect(&mut state, connection, &client, &signed),
                Standing::Asking(question) => {
                    let (held, answer) = self.wait_for(state, question, waiter);
                    state = held;
                    if waiter.stopped() {
                        return Err(not_allowed("the agent stopped waiting for the user".to_owned()));
                    }
                    // Another call of the connection may have taken the answer first, or `initialize` asked again.
                    if let Some(held) = state.connection(connection)
                        && held.standing == Standing::Asking(question)
                    {
                        held.standing = match answer {
                            Some(Answer::Policy { policy, .. }) => Standing::Given(policy),
                            _ => Standing::Refused,
                        };
                    }
                }
            }
        };
        let effects = labcraft_mcp::effects_of(id, params);
        if !asks(policy, &effects) {
            return Ok(());
        }
        let label = command_label(id, params);
        let question = self.ask(&mut state, connection, &client, &signed, Asks::Command { label: label.clone(), effects });
        let (mut state, answer) = self.wait_for(state, question, waiter);
        let refused = state.connection(connection).is_none_or(|held| held.standing == Standing::Refused);
        match answer {
            Some(Answer::Allow) => Ok(()),
            Some(_) => Err(not_allowed(format!("the user refused `{label}`"))),
            None if waiter.stopped() => Err(not_allowed("the agent stopped waiting for the user".to_owned())),
            None if refused => Err(not_allowed(format!("the user disconnected {signed}"))),
            None => Err(not_allowed(format!("the user did not answer about `{label}` in {} s", self.wait.as_secs()))),
        }
    }

    /// Whether a call on a run that the server answers at once may run, without waiting: one with no effect, once the
    /// user let the client in; one on the agent's own run, or any under `All`. `going`: the run going, when the call
    /// names none.
    fn allow_at_once(&self, connection: u64, id: &str, params: &Value, going: impl FnOnce() -> Option<String>) -> Result<(), CommandError> {
        let mut state = self.lock();
        let standing = state.connection(connection).map_or(Standing::Refused, |held| held.standing);
        let policy = match standing {
            Standing::Given(policy) => policy,
            Standing::Refused => return Err(not_allowed("the user did not let this agent work on this project".to_owned())),
            Standing::Unnamed | Standing::Asking(_) => {
                return Err(not_allowed("the user has not answered yet whether this agent may work on this project".to_owned()));
            }
        };
        if policy == Policy::All || labcraft_mcp::effects_of(id, params).is_empty() {
            return Ok(());
        }
        let run = params.get("run").and_then(Value::as_str).map(str::to_owned).or_else(going);
        let own = run.is_some_and(|run| state.connection(connection).is_some_and(|held| held.runs.contains(&run)));
        if own {
            Ok(())
        } else {
            Err(not_allowed(format!("`{id}` acts on a run that this agent did not start: only the user may, under its policy")))
        }
    }

    /// The connection started the run `run`.
    fn started(&self, connection: u64, run: &str) {
        if let Some(held) = self.lock().connection(connection) {
            held.runs.push(run.to_owned());
        }
    }

    /// The runs the connection started.
    fn runs_of(&self, connection: u64) -> Vec<String> {
        self.lock().connection(connection).map(|held| held.runs.clone()).unwrap_or_default()
    }

    /// The questions that wait for the user, the oldest first, with the seconds each has left: when they changed since
    /// the last call, or `waiting` (the window shows one, whose seconds go down); none otherwise. A question that
    /// expired goes, and the first connection it asked about is refused.
    pub fn questions(&self, waiting: bool) -> Option<Vec<Question>> {
        if !self.asked.swap(false, Ordering::SeqCst) && !waiting {
            return None;
        }
        let mut state = self.lock();
        let expired: Vec<(u64, u64)> = state
            .questions
            .iter()
            .filter(|asked| asked.answer.is_none() && asked.asked.elapsed() >= self.wait)
            .map(|asked| (asked.question.id, asked.connection))
            .collect();
        for (question, connection) in &expired {
            state.questions.retain(|asked| asked.question.id != *question);
            if let Some(held) = state.connection(*connection)
                && held.standing == Standing::Asking(*question)
            {
                held.standing = Standing::Refused;
            }
        }
        if !expired.is_empty() {
            self.changed.notify_all();
        }
        let questions = state
            .questions
            .iter()
            .filter(|asked| asked.answer.is_none())
            .map(|asked| Question { seconds_left: self.wait.saturating_sub(asked.asked.elapsed()).as_secs(), ..asked.question.clone() })
            .collect();
        Some(questions)
    }

    /// The user answered the question `id`; one that went, or expired, is let be. The answer to a first connection
    /// gives its policy at once, so that the pill shows it before the agent calls again.
    pub fn answer(&self, id: u64, answer: Answer) {
        let mut state = self.lock();
        let wait = self.wait;
        let Some(at) =
            state.questions.iter().position(|waiting| waiting.question.id == id && waiting.answer.is_none() && waiting.asked.elapsed() < wait)
        else {
            return;
        };
        let connect = state.questions.get(at).filter(|waiting| waiting.question.asks == Asks::Connect).map(|waiting| waiting.connection);
        match connect {
            Some(connection) => {
                state.questions.remove(at);
                if let Some(held) = state.connection(connection)
                    && held.standing == Standing::Asking(id)
                {
                    held.standing = match answer {
                        Answer::Policy { policy, .. } => Standing::Given(policy),
                        _ => Standing::Refused,
                    };
                }
            }
            None => {
                if let Some(waiting) = state.questions.get_mut(at) {
                    waiting.answer = Some(answer);
                }
            }
        }
        drop(state);
        self.changed.notify_all();
        if connect.is_some() {
            self.questions_changed();
        }
    }
}

/// Whether `policy` asks the user before a call with `effects` runs (ide.md 8).
fn asks(policy: Policy, effects: &[Effect]) -> bool {
    match policy {
        Policy::Ask => !effects.is_empty(),
        Policy::Edits => effects.iter().any(|effect| runs(*effect)),
        Policy::All => false,
    }
}

/// What the dialog says of a call: the label of its command, of the one `commands.execute` runs, or a batch.
fn command_label(id: &str, params: &Value) -> String {
    let named = match id {
        "commands.execute" => params.get("id").and_then(Value::as_str).unwrap_or(id),
        "commands.batch" => {
            let count = params.get("commands").and_then(Value::as_array).map_or(0, Vec::len);
            return format!("Make {count} edits at once");
        }
        other => other,
    };
    registry::command(named).map_or_else(|| named.to_owned(), |command| command.label.to_owned())
}

fn not_allowed(message: String) -> CommandError {
    CommandError::new("agent.not_allowed", message)
}

/// The gate of one connection.
struct Gating {
    agents: Arc<Agents>,
    connection: u64,
    served: Served,
}

impl Gate for Gating {
    fn introduce(&self, author: Author) -> Author {
        self.agents.introduce(self.connection, author)
    }

    fn allow(&self, author: &Author, id: &str, params: &Value, waiter: &dyn Waiter) -> Result<(), CommandError> {
        self.agents.allow(self.connection, author, id, params, waiter)
    }

    fn allow_at_once(&self, _: &Author, id: &str, params: &Value) -> Result<(), CommandError> {
        let going = || {
            let status = self.served.runs.execute(&*self.served.ws, &self.served.root, "run.status", &json!({})).ok()?;
            status.get("run").and_then(Value::as_str).map(str::to_owned)
        };
        self.agents.allow_at_once(self.connection, id, params, going)
    }
}

/// The calls of one connection, run by the session of the window.
struct Attached {
    agents: Arc<Agents>,
    connection: u64,
    session: Sender<Message>,
    served: Served,
}

impl Attached {
    /// Sends a call to the thread of the session, and waits for its result. While the call waits for the end of a
    /// gesture of the user, its client is told so each second (`waiting for the user`) and may cancel it: it then
    /// never runs (architecture 12.5).
    #[expect(clippy::disallowed_methods, reason = "the desktop app, never built for WASM, where the clock traps")]
    fn send(&self, author: &Author, id: &str, params: Value, waiter: &dyn Waiter) -> Result<Value, CommandError> {
        let (reply, result) = std::sync::mpsc::channel();
        let flags = Arc::new(Flags::default());
        let call = Call { author: author.clone(), id: id.to_owned(), params, reply, flags: Arc::clone(&flags) };
        if self.session.send(Message::Agent(call)).is_err() {
            return Err(CommandError::internal("the window closed its project"));
        }
        let mut told: Option<Instant> = None;
        loop {
            match result.recv_timeout(LOOK) {
                Ok(result) => return result,
                Err(RecvTimeoutError::Disconnected) => return Err(CommandError::internal("the window closed its project before it answered")),
                Err(RecvTimeoutError::Timeout) => {}
            }
            // A request cancelled before it runs never runs, and gets no answer (mcp.md 4); one that runs is waited for, so
            // that what it did is known (a run started is the client's).
            if waiter.stopped() && flags.cancel() {
                return Err(CommandError::internal("the client cancelled the call"));
            }
            if flags.held() && told.is_none_or(|told| told.elapsed() >= HELD_PROGRESS) {
                waiter.waiting_for_the_user();
                told = Some(Instant::now());
            }
        }
    }

    fn on_runs(&self, id: &str, params: &Value, waiter: &dyn Waiter) -> Result<Value, CommandError> {
        self.served.runs.execute_waiting(&*self.served.ws, &self.served.root, id, params, waiter)
    }

    /// `run.start`: started by the session without waiting, then waited for off it.
    fn start(&self, author: &Author, params: &Value, waiter: &dyn Waiter) -> Result<Value, CommandError> {
        let wait = params.get("wait").cloned();
        // A `wait` that the session would refuse goes to it as given: it refuses it, and starts nothing.
        let judged = wait.as_ref().is_none_or(|wait| {
            wait.as_str().and_then(|text| labcraft_engine::runs::duration_ns(text).ok()).is_some_and(|ns| (0..=MAX_WAIT_NS).contains(&ns))
        });
        if !judged {
            return self.send(author, "run.start", params.clone(), waiter);
        }
        let mut started = params.clone();
        if let Some(params) = started.as_object_mut() {
            params.insert("wait".to_owned(), json!("0 s"));
        }
        let report = self.send(author, "run.start", started, waiter)?;
        let Some(run) = report.get("run").and_then(Value::as_str).map(str::to_owned) else { return Ok(report) };
        self.agents.started(self.connection, &run);
        if report.get("status").and_then(Value::as_str) != Some("running") {
            return Ok(report);
        }
        // The wait the agent asked for, `30 s` by default as `run.start` gives it (commands.md 6.4).
        let wait = wait.unwrap_or_else(|| json!("30 s"));
        self.on_runs("run.wait", &json!({ "run": run, "wait": wait }), waiter)
    }
}

/// How often a client whose call waits for a gesture is told so.
const HELD_PROGRESS: Duration = Duration::from_secs(1);

/// The longest `wait` of `run.start` (commands.md 6.4), in nanoseconds.
const MAX_WAIT_NS: i64 = 300_000_000_000;

impl Executor for Attached {
    fn execute(&mut self, author: &Author, id: &str, params: &Value, waiter: &dyn Waiter) -> Result<Value, CommandError> {
        // The same calls through `commands.execute` (mcp.md 2).
        let (id, params) = match (id, params.get("id").and_then(Value::as_str)) {
            ("commands.execute", Some(inner @ ("run.start" | "run.wait" | "run.trace"))) => {
                (inner, params.get("params").cloned().unwrap_or_else(|| json!({})))
            }
            _ => (id, params.clone()),
        };
        match id {
            // The calls on a run that the server does not answer at once (`at_once`), and that need no session.
            "run.wait" | "run.trace" => self.on_runs(id, &params, waiter),
            "run.start" => self.start(author, &params, waiter),
            _ => self.send(author, id, params, waiter),
        }
    }

    fn closed(&mut self, served: &Served) {
        for run in self.agents.runs_of(self.connection) {
            let going = served.runs.execute(&*served.ws, &served.root, "run.status", &json!({ "run": run }));
            if going.is_ok_and(|report| report.get("status").and_then(Value::as_str) == Some("running")) {
                let _ = served.runs.execute(&*served.ws, &served.root, "run.abort", &json!({ "run": run }));
            }
        }
    }
}

/// The connections served, each by its number: the channel shuts them as it closes.
type Streams = Arc<Mutex<Vec<(u64, TcpStream)>>>;

/// The channel of the window, open: the file that gives it, the thread that accepts, the connections served. Dropped,
/// it closes all of them.
pub struct Channel {
    agents: Arc<Agents>,
    published: attach::Published,
    port: u16,
    closing: Arc<AtomicBool>,
    streams: Streams,
    accepting: Option<std::thread::JoinHandle<()>>,
}

/// What the thread that accepts hands each connection.
#[derive(Clone)]
struct Serving {
    token: String,
    session: Sender<Message>,
    served: Served,
    agents: Arc<Agents>,
    greeting: Arc<AtomicUsize>,
}

impl Channel {
    /// Opens the channel of the project at `root` (canonical), whose session `session` runs: none when there is no
    /// folder for the channel ([`attach::folder`]); why it cannot open otherwise. `served`: what the session serves.
    pub fn open(root: &Path, session: Sender<Message>, served: Served, agents: Arc<Agents>) -> Result<Option<Channel>, String> {
        let Some(folder) = attach::folder() else { return Ok(None) };
        Channel::open_in(&folder, root, session, served, agents).map(Some)
    }

    /// As [`Channel::open`], its file in `folder`: for the tests.
    pub fn open_in(folder: &Path, root: &Path, session: Sender<Message>, served: Served, agents: Arc<Agents>) -> Result<Channel, String> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|error| format!("the channel of the agents: {error}"))?;
        let port = listener.local_addr().map_err(|error| error.to_string())?.port();
        let published = attach::publish(folder, root, port)?;
        let serving =
            Serving { token: published.token().to_owned(), session, served, agents: agents.clone(), greeting: Arc::new(AtomicUsize::new(0)) };
        let closing = Arc::new(AtomicBool::new(false));
        let streams: Streams = Arc::new(Mutex::new(Vec::new()));
        let (closed, held) = (closing.clone(), streams.clone());
        let accepting = std::thread::Builder::new()
            .name("labcraft-agents".to_owned())
            .spawn(move || accept(&listener, &closed, &held, &serving))
            .map_err(|error| error.to_string())?;
        Ok(Channel { agents, published, port, closing, streams, accepting: Some(accepting) })
    }

    /// The agents that come by it.
    pub fn agents(&self) -> &Agents {
        &self.agents
    }

    /// The file that gives the channel.
    pub fn file(&self) -> PathBuf {
        self.published.path().to_path_buf()
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        self.closing.store(true, Ordering::SeqCst);
        // Wakes the thread that accepts.
        let _ = TcpStream::connect((Ipv4Addr::LOCALHOST, self.port));
        for (_, stream) in self.streams.lock().unwrap_or_else(PoisonError::into_inner).drain(..) {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Some(accepting) = self.accepting.take() {
            let _ = accepting.join();
        }
    }
}

fn accept(listener: &TcpListener, closing: &AtomicBool, streams: &Streams, serving: &Serving) {
    let mut number = 0_u64;
    for stream in listener.incoming() {
        if closing.load(Ordering::SeqCst) {
            return;
        }
        let Ok(stream) = stream else { continue };
        if serving.greeting.fetch_add(1, Ordering::SeqCst) >= MAX_GREETING {
            serving.greeting.fetch_sub(1, Ordering::SeqCst);
            let _ = stream.shutdown(Shutdown::Both);
            continue;
        }
        let Ok(held) = stream.try_clone() else {
            serving.greeting.fetch_sub(1, Ordering::SeqCst);
            continue;
        };
        number = number.wrapping_add(1);
        let own = number;
        streams.lock().unwrap_or_else(PoisonError::into_inner).push((own, held));
        let (handed, kept) = (serving.clone(), streams.clone());
        let spawned = std::thread::Builder::new().name("labcraft-agent".to_owned()).stack_size(labcraft_format::STACK_SIZE).spawn(move || {
            serve_connection(stream, &handed);
            kept.lock().unwrap_or_else(PoisonError::into_inner).retain(|(held, _)| *held != own);
        });
        if spawned.is_err() {
            streams.lock().unwrap_or_else(PoisonError::into_inner).retain(|(held, _)| *held != own);
            serving.greeting.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// One connection: the proofs of the token, then MCP until it closes.
fn serve_connection(mut stream: TcpStream, serving: &Serving) {
    let mut connection = None;
    let greeted = attach::greet(&mut stream, &serving.token, || {
        connection = Some(serving.agents.connected()?);
        Ok(())
    });
    serving.greeting.fetch_sub(1, Ordering::SeqCst);
    let Some(connection) = connection else { return };
    if let Ok(held) = stream.try_clone() {
        serving.agents.hold_stream(connection, held);
    }
    if greeted.is_ok()
        && let Ok(output) = stream.try_clone()
    {
        let options = labcraft_mcp::Options { viewer: true, version: env!("CARGO_PKG_VERSION").to_owned(), max_request: labcraft_mcp::MAX_REQUEST };
        let gate = Gating { agents: serving.agents.clone(), connection, served: serving.served.clone() };
        let mut executor = Attached { agents: serving.agents.clone(), connection, session: serving.session.clone(), served: serving.served.clone() };
        let mut input = BufReader::new(stream);
        labcraft_mcp::serve_with(&mut executor, &serving.served, &gate, &mut input, Box::new(output), &options);
    }
    serving.agents.disconnected(connection);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A waiter that waits as long as the test says.
    #[derive(Default)]
    struct Stopping(AtomicBool);

    impl Waiter for Stopping {
        fn stopped(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn agents(wait: Duration) -> Arc<Agents> {
        Arc::new(Agents::waiting(Path::new("/p"), wait, Box::new(|| {})))
    }

    /// Answers the first question that comes: what it asked.
    fn answer(agents: &Agents, answer: Answer) -> Asks {
        loop {
            if let Some(question) = agents.questions(true).unwrap().first() {
                agents.answer(question.id, answer);
                return question.asks.clone();
            }
            std::thread::yield_now();
        }
    }

    #[test]
    fn each_policy_asks_for_what_it_does_not_let_run() {
        let (files, run, both) = (vec![Effect::Files], vec![Effect::Run], vec![Effect::Files, Effect::Run]);
        assert!(!asks(Policy::Ask, &[]));
        assert!(asks(Policy::Ask, &files) && asks(Policy::Ask, &run));
        assert!(!asks(Policy::Edits, &files) && asks(Policy::Edits, &run) && asks(Policy::Edits, &both));
        assert!(!asks(Policy::All, &both));
    }

    #[test]
    fn two_clients_of_one_name_are_numbered_and_asked_by_the_name_of_the_client() {
        let agents = agents(ANSWER_WAIT);
        let (one, two, three) = (agents.connected().unwrap(), agents.connected().unwrap(), agents.connected().unwrap());
        assert_eq!(agents.introduce(one, Author::Agent("agent".to_owned())), Author::Agent("agent".to_owned()));
        assert_eq!(agents.introduce(two, Author::Agent("agent".to_owned())), Author::Agent("agent 2".to_owned()));
        assert_eq!(agents.introduce(three, Author::Agent("claude".to_owned())), Author::Agent("claude".to_owned()));
        agents.disconnected(one);
        let four = agents.connected().unwrap();
        assert_eq!(agents.introduce(four, Author::Agent("agent".to_owned())), Author::Agent("agent".to_owned()));
        let asked = agents.questions(true).unwrap();
        assert_eq!(asked.len(), 3, "the first connection of each asks");
        assert_eq!((asked[0].client.as_str(), asked[0].signed.as_str()), ("agent", "agent 2"));
    }

    #[test]
    fn a_policy_kept_for_the_client_on_this_project_answers_at_once() {
        let agents = agents(ANSWER_WAIT);
        agents.keep(json!({ "claude": { "/p": "edits" }, "cursor": { "/q": "all" } }));
        let (one, two, three) = (agents.connected().unwrap(), agents.connected().unwrap(), agents.connected().unwrap());
        // Two clients of one name: both are given the policy of the name.
        let claude = agents.introduce(one, Author::Agent("claude".to_owned()));
        let claude_2 = agents.introduce(two, Author::Agent("claude".to_owned()));
        agents.introduce(three, Author::Agent("cursor".to_owned()));
        let waiter = Stopping::default();
        assert!(agents.allow(one, &claude, "node.delete", &json!({}), &waiter).is_ok());
        assert!(agents.allow(two, &claude_2, "node.delete", &json!({}), &waiter).is_ok());
        let asked = agents.questions(true).unwrap();
        assert_eq!(asked.iter().map(|question| question.client.as_str()).collect::<Vec<_>>(), ["cursor"], "kept for another project");
    }

    #[test]
    fn a_call_waits_for_the_policy_and_the_user() {
        let agents = agents(ANSWER_WAIT);
        let connection = agents.connected().unwrap();
        let claude = agents.introduce(connection, Author::Agent("claude".to_owned()));
        let (asked, author) = (agents.clone(), claude.clone());
        let edit = std::thread::spawn(move || asked.allow(connection, &author, "node.delete", &json!({}), &Stopping::default()));
        assert_eq!(answer(&agents, Answer::Policy { policy: Policy::Ask, keep: false }), Asks::Connect);
        assert!(matches!(answer(&agents, Answer::Refuse), Asks::Command { .. }));
        assert_eq!(edit.join().unwrap().unwrap_err().code, "agent.not_allowed");
        // A read runs under `Ask`, without a question.
        assert!(agents.allow(connection, &claude, "file.read", &json!({}), &Stopping::default()).is_ok());
        assert!(agents.questions(true).unwrap().is_empty());
    }

    #[test]
    fn a_refused_connection_refuses_every_call() {
        let agents = agents(ANSWER_WAIT);
        let connection = agents.connected().unwrap();
        let claude = agents.introduce(connection, Author::Agent("claude".to_owned()));
        let question = agents.questions(true).unwrap().first().unwrap().id;
        agents.answer(question, Answer::Refuse);
        assert_eq!(agents.allow(connection, &claude, "file.read", &json!({}), &Stopping::default()).unwrap_err().code, "agent.not_allowed");
        assert!(agents.questions(true).unwrap().is_empty());
    }

    #[test]
    fn a_question_unanswered_expires_and_refuses_even_with_no_call_waiting() {
        let agents = agents(Duration::from_millis(50));
        let connection = agents.connected().unwrap();
        let claude = agents.introduce(connection, Author::Agent("claude".to_owned()));
        let question_id = agents.questions(true).unwrap()[0].id;
        std::thread::sleep(Duration::from_millis(80));
        assert!(agents.questions(true).unwrap().is_empty(), "the window no longer shows it");
        agents.answer(question_id, Answer::Policy { policy: Policy::All, keep: false });
        assert_eq!(agents.allow(connection, &claude, "file.read", &json!({}), &Stopping::default()).unwrap_err().code, "agent.not_allowed");
    }

    #[test]
    fn an_agent_that_stops_waiting_takes_the_question_of_its_call_away_not_that_of_its_connection() {
        let agents = agents(ANSWER_WAIT);
        let connection = agents.connected().unwrap();
        let claude = agents.introduce(connection, Author::Agent("claude".to_owned()));
        let waiter = Arc::new(Stopping::default());
        let (asked, author, stops) = (agents.clone(), claude, waiter.clone());
        let edit = std::thread::spawn(move || asked.allow(connection, &author, "node.delete", &json!({}), &*stops));
        while agents.questions(true).unwrap().is_empty() {
            std::thread::yield_now();
        }
        waiter.0.store(true, Ordering::SeqCst);
        assert_eq!(edit.join().unwrap().unwrap_err().code, "agent.not_allowed");
        // The question of the first connection stays: the user still answers it.
        let asked = agents.questions(true).unwrap();
        assert_eq!(asked.iter().map(|question| question.asks.clone()).collect::<Vec<_>>(), [Asks::Connect]);
        agents.answer(asked[0].id, Answer::Policy { policy: Policy::Edits, keep: false });
        let claude = Author::Agent("claude".to_owned());
        assert!(agents.allow(connection, &claude, "node.delete", &json!({}), &Stopping::default()).is_ok());
        // A call's question goes with it.
        let waiter = Arc::new(Stopping::default());
        let (asked, author, stops) = (agents.clone(), claude, waiter.clone());
        let run = std::thread::spawn(move || asked.allow(connection, &author, "run.start", &json!({}), &*stops));
        while agents.questions(true).unwrap().is_empty() {
            std::thread::yield_now();
        }
        waiter.0.store(true, Ordering::SeqCst);
        assert_eq!(run.join().unwrap().unwrap_err().code, "agent.not_allowed");
        assert!(agents.questions(true).unwrap().is_empty());
    }

    #[test]
    fn the_pill_lists_the_agents_named_and_not_refused_and_changes_their_policy() {
        let agents = agents(ANSWER_WAIT);
        let (one, two, _unnamed) = (agents.connected().unwrap(), agents.connected().unwrap(), agents.connected().unwrap());
        let claude = agents.introduce(one, Author::Agent("claude".to_owned()));
        agents.introduce(two, Author::Agent("cursor".to_owned()));
        assert_eq!(answer(&agents, Answer::Policy { policy: Policy::Ask, keep: false }), Asks::Connect);
        let listed = agents.connected_list();
        // The third gave no name; `cursor` is still asked.
        assert_eq!(
            listed.iter().map(|agent| (agent.signed.as_str(), agent.policy)).collect::<Vec<_>>(),
            [("claude", Some(Policy::Ask)), ("cursor", None)]
        );
        assert_eq!(listed[0].project, "/p");
        // Under `Edits` from the pill, an edit no longer asks.
        agents.set_policy(one, Policy::Edits);
        assert!(agents.allow(one, &claude, "node.delete", &json!({}), &Stopping::default()).is_ok());
        assert!(agents.questions(true).unwrap().iter().all(|question| question.asks == Asks::Connect));
        // A policy for one still asked is let be: the question answers it.
        agents.set_policy(two, Policy::All);
        assert_eq!(agents.connected_list()[1].policy, None);
        // Disconnected: refused, and no longer listed; its question goes.
        agents.disconnect(two);
        assert_eq!(agents.connected_list().len(), 1);
        assert!(agents.questions(true).unwrap().is_empty());
    }

    /// Waits until a call of the agent waits for the user's answer.
    fn until_asked(agents: &Agents) {
        while !agents.questions(true).unwrap().iter().any(|question| matches!(question.asks, Asks::Command { .. })) {
            std::thread::yield_now();
        }
    }

    #[test]
    fn a_call_that_waits_runs_once_the_pill_lets_it_and_says_so_when_its_agent_is_disconnected() {
        let agents = agents(ANSWER_WAIT);
        let connection = agents.connected().unwrap();
        let claude = agents.introduce(connection, Author::Agent("claude".to_owned()));
        assert_eq!(answer(&agents, Answer::Policy { policy: Policy::Ask, keep: false }), Asks::Connect);
        let (asked, author) = (agents.clone(), claude.clone());
        let edit = std::thread::spawn(move || asked.allow(connection, &author, "node.delete", &json!({}), &Stopping::default()));
        until_asked(&agents);
        // `Edits` lets an edit run: the question waiting is answered.
        agents.set_policy(connection, Policy::Edits);
        assert!(edit.join().unwrap().is_ok());
        let (asked, author) = (agents.clone(), claude);
        let run = std::thread::spawn(move || asked.allow(connection, &author, "run.start", &json!({}), &Stopping::default()));
        until_asked(&agents);
        agents.disconnect(connection);
        assert_eq!(run.join().unwrap().unwrap_err().message, "the user disconnected claude");
    }

    #[test]
    fn ai_off_refuses_every_agent_and_lets_none_in_until_it_is_on_again() {
        let agents = agents(ANSWER_WAIT);
        let one = agents.connected().unwrap();
        let claude = agents.introduce(one, Author::Agent("claude".to_owned()));
        assert!(!agents.questions(true).unwrap().is_empty());
        agents.set_enabled(false);
        assert!(!agents.enabled());
        assert!(agents.questions(true).unwrap().is_empty(), "the questions go");
        assert!(agents.connected_list().is_empty());
        assert_eq!(agents.allow(one, &claude, "file.read", &json!({}), &Stopping::default()).unwrap_err().code, "agent.not_allowed");
        assert_eq!(agents.connected().unwrap_err(), AI_OFF);
        agents.set_enabled(true);
        assert!(agents.connected().is_ok());
    }

    #[test]
    fn a_call_on_a_run_answered_at_once_never_waits() {
        let agents = agents(ANSWER_WAIT);
        let connection = agents.connected().unwrap();
        agents.introduce(connection, Author::Agent("claude".to_owned()));
        let none = || None;
        // Before the user's answer: refused at once.
        assert_eq!(agents.allow_at_once(connection, "run.status", &json!({}), none).unwrap_err().code, "agent.not_allowed");
        answer(&agents, Answer::Policy { policy: Policy::Edits, keep: false });
        let waiter = Stopping::default();
        assert!(agents.allow(connection, &Author::Agent("claude".to_owned()), "file.read", &json!({}), &waiter).is_ok());
        assert!(agents.allow_at_once(connection, "run.status", &json!({}), none).is_ok());
        // Its own run, named or going; not another.
        agents.started(connection, "r1");
        assert!(agents.allow_at_once(connection, "run.abort", &json!({ "run": "r1" }), none).is_ok());
        assert!(agents.allow_at_once(connection, "run.abort", &json!({}), || Some("r1".to_owned())).is_ok());
        assert_eq!(agents.allow_at_once(connection, "run.abort", &json!({}), || Some("r2".to_owned())).unwrap_err().code, "agent.not_allowed");
    }
}
