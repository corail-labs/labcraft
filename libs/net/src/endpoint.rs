//! Endpoints (`docs/io.md`, section 2): `tcp:HOST:PORT` and `udp:HOST:PORT`, the host in lower case for its ASCII letters,
//! an IPv6 address in brackets whether it was given with them or not, the port in decimal without zeros in front. The name a
//! program gives and the name a simulation file gives reduce to the same text, so that they compare.

use labcraft_runtime::io::{INVALID_ARGUMENT, Reach, endpoint};
use labcraft_runtime::native::NativeOutcome;
pub(crate) use labcraft_runtime::stream::shown;

pub(crate) const HOST_NOT_FOUND: &str = "net.host_not_found";

/// What a call reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Endpoint {
    /// `tcp:10.0.0.5:502`
    pub(crate) text: String,
    /// What the program gave, `HOST:PORT`, which the question of consent shows (io.md 4).
    pub(crate) given: String,
    /// The host to resolve: without brackets.
    pub(crate) host: String,
    pub(crate) port: u16,
}

impl Endpoint {
    /// What the run asks the right to reach (io.md 4).
    pub(crate) fn reach(&self) -> Reach {
        Reach::Endpoint { endpoint: self.text.clone(), given: self.given.clone() }
    }

    /// What a message shows of it: its start, since the host is the program's and has no bound.
    pub(crate) fn shown(&self) -> String {
        shown(&self.text)
    }
}

/// The endpoint of `net.connect(host, port)` or `net.open_udp(host, port)`: a text that is no host is
/// `net.host_not_found` at once, before the simulation file and any question (io.md 3); port 0 is `io.invalid_argument`
/// (io.md 8).
pub(crate) fn of_call(udp: bool, host: &str, port: u16) -> Result<Endpoint, NativeOutcome> {
    let Some(bare) = endpoint::host(host) else {
        let message = if host.is_empty() {
            "an empty host names no device".to_owned()
        } else {
            format!("`{}` is no host: an IP address, or a name of ASCII letters, digits, `-`, `.` and `_`, of at most 253 bytes", shown(host))
        };
        return Err(NativeOutcome::Fail { category: HOST_NOT_FOUND, code: 0, message });
    };
    if port == 0 {
        return Err(NativeOutcome::Fail {
            category: INVALID_ARGUMENT,
            code: 0,
            message: format!("port 0 on `{}`: a port is from 1 to 65,535", shown(host)),
        });
    }
    Ok(Endpoint { text: endpoint::tcp_or_udp_text(udp, bare, port), given: format!("{host}:{port}"), host: bare.to_owned(), port })
}

/// The endpoint a simulation file names (`endpoint = "tcp:10.0.0.7:502"`), as a program reaches it, and whether it is UDP;
/// `None` when it is not `tcp:HOST:PORT` or `udp:HOST:PORT`.
pub(crate) use labcraft_runtime::io::endpoint::tcp_or_udp as of_file;
