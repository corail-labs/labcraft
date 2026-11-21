//! How the window talks to a session (`docs/architecture.md`, 12.7): it sends a command and reads the last snapshot of
//! what the session publishes. A thread and a channel on the desktop, a worker on the web: the window is written once.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use labcraft_engine::CommandError;
use labcraft_engine::view::Published;
use serde_json::Value;

/// What the window reads of the session: the snapshot of the view (architecture 12.5), and what the app says of the
/// project around it.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub view: Arc<Published>,
    /// The project open, as the app shows it: its folder.
    pub project: Option<String>,
    /// What the app must say: a folder that cannot be watched, a file of `--style` that does not read.
    pub notices: Vec<String>,
    /// Which session of the app this is: another one, even of a project of the same name, is a new project for the
    /// window. The desktop opens one project per window, and gives 0.
    pub session: u64,
    /// The agent whose editing command waits for the end of the gesture of the user, by its name (architecture 12.5).
    pub waiting: Option<String>,
}

/// The result of a command, when it comes.
pub struct Reply(Receiver<Result<Value, CommandError>>);

impl Reply {
    /// A reply and the sender that gives it.
    pub fn channel() -> (Sender<Result<Value, CommandError>>, Reply) {
        let (sender, receiver) = mpsc::channel();
        (sender, Reply(receiver))
    }

    /// A reply that is already there.
    pub fn ready(result: Result<Value, CommandError>) -> Reply {
        let (sender, reply) = Reply::channel();
        let _ = sender.send(result);
        reply
    }

    /// The result once it has come; a session gone before it answered is an `internal` error.
    pub fn take(&self) -> Option<Result<Value, CommandError>> {
        match self.0.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(CommandError::internal("the session ended before it answered"))),
        }
    }

    /// Waits for the result, at most `timeout`: for the tests and the apps on the desktop, never on the web, where a
    /// thread cannot wait.
    pub fn wait(&self, timeout: std::time::Duration) -> Option<Result<Value, CommandError>> {
        match self.0.recv_timeout(timeout) {
            Ok(result) => Some(result),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => Some(Err(CommandError::internal("the session ended before it answered"))),
        }
    }
}

/// A session, as the window reaches it.
pub trait Host {
    /// Sends a command, signed by the window; its result comes on the reply.
    fn send(&self, id: &str, params: Value) -> Reply;

    /// What the session last published.
    fn snapshot(&self) -> Snapshot;

    /// The zoom of the canvas of the window that shows `function`, which `view.get_state` returns: given by the window,
    /// not a command.
    fn set_zoom(&self, function: &str, zoom: f32);

    /// The keys of the store of the user (commands.md 9.7), which the window keeps, for a session that does not hold them
    /// yet (`Stores::given`): not a command, as the zoom.
    fn set_user_settings(&self, keys: &serde_json::Map<String, Value>);

    /// The gesture of the user in progress, or none: said as it starts and as it ends, so that an editing command of an
    /// agent waits for its end (architecture 12.5). Not a command, as the zoom.
    fn set_gesture(&self, gesture: Option<labcraft_engine::Gesture>);
}
