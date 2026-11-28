//! Real serial ports, over the crate `serial2` (`docs/io.md`, sections 7 and 11). Each function here blocks, and runs in the
//! `Blocking` work of its call, away from the thread of the diagram. A port is a real stream of `runtime::stream`, behind a
//! mutex that its work owns while it runs; the runtime releases a port only once no call of the run is in progress
//! (`docs/architecture.md`, 10.6), so a release never waits for that mutex long.

use std::io::ErrorKind;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use labcraft_runtime::io::{CLOSED, INVALID_ARGUMENT, TIMEOUT, UNSUPPORTED};
use labcraft_runtime::stream::real::{Buffered, Stop, Wire, code, locked};
use labcraft_runtime::stream::{Failure, Need, Taken};
use serial2::{CharSize, FlowControl, SerialPort};

use labcraft_runtime::io::endpoint::{com_number, serial_port};

use crate::endpoint::{PORT_BUSY, PORT_NOT_FOUND};
use crate::settings::{Flow, Parity, Settings, StopBits};

/// What a read that only takes what the system holds already waits for it: the shortest timeout `serial2` takes on Windows.
const AT_ONCE: Duration = Duration::from_millis(1);

/// The category of an error of the open (`libs/serial/README.md`, `open`), from its kind alone, so that it is the same on
/// every system: a port that does not exist is not found; one open elsewhere, which Windows says as access denied, or that
/// the system will not give (no right to it on Linux), is busy; a setting the system refuses is an invalid argument; a
/// timeout is `io.timeout`; any other error, such as a name that is no port (a file that is not a terminal), is not found.
pub(crate) fn open_category(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::NotFound => PORT_NOT_FOUND,
        ErrorKind::PermissionDenied | ErrorKind::ResourceBusy => PORT_BUSY,
        ErrorKind::InvalidInput | ErrorKind::InvalidData | ErrorKind::Unsupported => INVALID_ARGUMENT,
        ErrorKind::TimedOut | ErrorKind::WouldBlock => TIMEOUT,
        _ => PORT_NOT_FOUND,
    }
}

fn open_error(error: &std::io::Error, endpoint: &str) -> Failure {
    // Outside Windows, a refused right is seldom another program: say what usually grants it, so that "busy" does not
    // send the user looking for one.
    let hint = if !cfg!(windows) && error.kind() == ErrorKind::PermissionDenied {
        " (the system refuses this user the port: on Linux, the group that owns it, often `dialout`, grants it)"
    } else {
        ""
    };
    Failure { category: open_category(error.kind()), code: code(error), message: format!("{endpoint}: {error}{hint}") }
}

/// The ports the system has (`libs/serial/README.md`, `list`): their names, as an endpoint writes them (`COM3`), sorted in
/// code point order, each once; on macOS only the `/dev/cu.*` ones. A name that is not UTF-8 cannot be opened by a text,
/// and is left out. A system that cannot list them gives `io.unsupported`, code 0 as io.md 7 says, its error in the message.
pub(crate) fn list() -> Result<Vec<String>, Failure> {
    let found =
        SerialPort::available_ports().map_err(|error| Failure::new(UNSUPPORTED, format!("this system does not list its serial ports: {error}")))?;
    let mut ports: Vec<String> = found
        .iter()
        .filter_map(|path| path.to_str())
        .filter(|name| !cfg!(target_os = "macos") || name.starts_with("/dev/cu."))
        .filter_map(serial_port)
        .collect();
    ports.sort();
    ports.dedup();
    Ok(ports)
}

/// A port as a stream reads it, with the timeouts it was given last, in whole milliseconds as `serial2` gives them to the
/// system: each is given again only when it changes.
pub(crate) struct Wired {
    port: SerialPort,
    read_ms: Option<u128>,
    write_ms: Option<u128>,
}

impl Wired {
    fn new(port: SerialPort) -> Self {
        Wired { port, read_ms: None, write_ms: None }
    }
}

impl Wire for Wired {
    fn read_within(&mut self, buffer: &mut [u8], wait: Duration) -> std::io::Result<usize> {
        let wait = wait.max(AT_ONCE);
        if self.read_ms != Some(wait.as_millis()) && self.port.set_read_timeout(wait).is_ok() {
            self.read_ms = Some(wait.as_millis());
        }
        self.port.read(buffer)
    }

    fn read_now(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.read_within(buffer, AT_ONCE)
    }

    fn write_within(&mut self, data: &[u8], wait: Duration) -> std::io::Result<usize> {
        let wait = wait.max(AT_ONCE);
        if self.write_ms != Some(wait.as_millis()) && self.port.set_write_timeout(wait).is_ok() {
            self.write_ms = Some(wait.as_millis());
        }
        self.port.write(data)
    }

    /// `serial2` returns from a read as soon as it has one byte, and gives all the system holds when it holds some
    /// (`ReadIntervalTimeout` and `ReadTotalTimeoutMultiplier` at `MAXDWORD` on Windows; `poll`, then one `read`, elsewhere):
    /// a read shorter than the buffer took what was there, and a board that prints without end no longer keeps the drain.
    fn ends_drain(&self, read: usize, offered: usize) -> bool {
        read < offered
    }
}

/// The port of a `Blocking` work.
pub(crate) type Shared = Arc<Mutex<Buffered<Wired>>>;

/// A real port.
pub(crate) struct Port {
    state: Shared,
    broken: Arc<AtomicBool>,
}

/// Gives `port` these settings, each in the call `serial2` has for it.
fn apply(settings: Settings, port: &mut serial2::Settings) -> std::io::Result<()> {
    port.set_baud_rate(settings.baud)?;
    port.set_char_size(match settings.data_bits {
        5 => CharSize::Bits5,
        6 => CharSize::Bits6,
        7 => CharSize::Bits7,
        _ => CharSize::Bits8,
    });
    port.set_stop_bits(match settings.stop_bits {
        StopBits::One => serial2::StopBits::One,
        StopBits::Two => serial2::StopBits::Two,
    });
    port.set_parity(match settings.parity {
        Parity::None => serial2::Parity::None,
        Parity::Odd => serial2::Parity::Odd,
        Parity::Even => serial2::Parity::Even,
    });
    port.set_flow_control(match settings.flow {
        Flow::None => FlowControl::None,
        Flow::Software => FlowControl::XonXoff,
        Flow::Hardware => FlowControl::RtsCts,
    });
    Ok(())
}

/// What an open does: a raw port, then its settings.
fn for_open(settings: Settings) -> impl FnOnce(serial2::Settings) -> std::io::Result<serial2::Settings> {
    move |mut port: serial2::Settings| {
        port.set_raw();
        apply(settings, &mut port)?;
        Ok(port)
    }
}

impl Port {
    /// Opens the port `name` (`COM3`, `/dev/ttyUSB0`) with its settings. A `COM` name names no port outside Windows (io.md 2),
    /// and is not found without asking the system, which would read it as a file of the current folder.
    pub(crate) fn open(name: &str, settings: Settings, timeout: Duration, endpoint: &str) -> Result<Port, Failure> {
        if !cfg!(windows) && com_number(name).is_some() {
            return Err(Failure::new(PORT_NOT_FOUND, format!("{endpoint}: a `COM` name names no port on this system")));
        }
        let port = SerialPort::open(name, for_open(settings)).map_err(|error| open_error(&error, endpoint))?;
        let broken = Arc::new(AtomicBool::new(false));
        let state = Buffered::new(Wired::new(port), endpoint.to_owned(), timeout, Arc::clone(&broken));
        Ok(Port { state: Arc::new(Mutex::new(state)), broken })
    }

    pub(crate) fn handle(&self) -> Shared {
        Arc::clone(&self.state)
    }

    pub(crate) fn broken(&self) -> bool {
        self.broken.load(Ordering::Relaxed)
    }
}

/// Empties the input (`settle`): what the system holds, and what waits in the port.
pub(crate) fn discard(shared: &Shared) {
    let mut state = locked(shared);
    let _ = state.wire.port.discard_input_buffer();
    state.forget();
}

/// `configure`: gives the open port these settings, as `open` gave them, without closing it. It reads the settings the port
/// has, changes the five, and sets them again in one call (`serial2`: `get_configuration`, `set_configuration`), so that the
/// port stays raw and DTR does not move: no close, no new open, which is what toggles DTR and resets a board; no call to
/// empty the input, so the bytes that wait stay, in the system and in the buffer of the port. A setting the system refuses is
/// `io.invalid_argument`: the kinds an open says as one (`open_category`), and the `Other` of `serial2` when the system set
/// something else than asked (it reads the settings back, outside Windows). A port whose settings cannot be read, or any
/// other error, went away: `io.closed`, and the port is closed, as for a read.
pub(crate) fn reconfigure(shared: &Shared, settings: Settings) -> Result<(), Failure> {
    let mut state = locked(shared);
    let mut port = match state.wire.port.get_configuration() {
        Ok(port) => port,
        Err(error) => return Err(broke(&state, &error)),
    };
    let result = apply(settings, &mut port).and_then(|()| state.wire.port.set_configuration(&port));
    result.map_err(|error| {
        if error.kind() == ErrorKind::Other || open_category(error.kind()) == INVALID_ARGUMENT {
            Failure {
                category: INVALID_ARGUMENT,
                code: code(&error),
                message: format!("{}: the system refuses these settings: {error}", state.endpoint),
            }
        } else {
            broke(&state, &error)
        }
    })
}

/// A port that went away under a call that is not a read or a write: it is marked, so that the call closes it.
fn broke(state: &Buffered<Wired>, error: &std::io::Error) -> Failure {
    state.broken.store(true, Ordering::Relaxed);
    gone(Some(error), &state.endpoint)
}

/// `io.closed` when the port went away, with the error of the system if there was one.
fn gone(error: Option<&std::io::Error>, endpoint: &str) -> Failure {
    match error {
        Some(error) => Failure { category: CLOSED, code: code(error), message: format!("{endpoint}: the port went away: {error}") },
        None => Failure::new(CLOSED, format!("{endpoint}: the port went away")),
    }
}

/// Writes every byte within the timeout of the port, counted from `since`, the start of the call.
pub(crate) fn write(shared: &Shared, data: &[u8], since: Instant) -> Result<(), Failure> {
    let mut state = locked(shared);
    state.write_all(data, since).map_err(|stop| match stop {
        Stop::TimedOut(failure) => failure,
        // The port took none of the bytes: it went away, and is closed, as for a read.
        Stop::Ended => {
            state.broken.store(true, Ordering::Relaxed);
            gone(None, &state.endpoint)
        }
        Stop::Error(error) => gone(Some(&error), &state.endpoint),
    })
}

/// A read that started at `since`: what `need` takes of the bytes that wait, waiting for more up to the timeout of the port;
/// `io.timeout` then, taking nothing. A port that went away gives what had arrived, then `io.closed`, and is closed.
pub(crate) fn read(shared: &Shared, need: &Need, since: Instant) -> Result<Taken, Failure> {
    let mut state = locked(shared);
    state.read(need, since).map_err(|stop| match stop {
        Stop::TimedOut(failure) => failure,
        Stop::Ended => {
            state.broken.store(true, Ordering::Relaxed);
            gone(None, &state.endpoint)
        }
        Stop::Error(error) => gone(Some(&error), &state.endpoint),
    })
}
