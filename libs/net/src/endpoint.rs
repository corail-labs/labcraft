//! Endpoints (`docs/io.md`, section 2): `tcp:HOST:PORT` and `udp:HOST:PORT`, the host in lower case for its ASCII letters,
//! an IPv6 address in brackets whether it was given with them or not, the port in decimal without zeros in front. The name a
//! program gives and the name a simulation file gives reduce to the same text, so that they compare.

use std::net::{IpAddr, Ipv6Addr};

use labcraft_runtime::io::{INVALID_ARGUMENT, Reach};
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

fn scheme(udp: bool) -> &'static str {
    if udp { "udp" } else { "tcp" }
}

/// The host as an endpoint writes it, from a host without brackets.
fn written(host: &str) -> String {
    let lower = host.to_ascii_lowercase();
    if lower.contains(':') { format!("[{lower}]") } else { lower }
}

/// The longest DNS name, in bytes.
const MAX_NAME: usize = 253;

/// A host (decision of the owner, 2026-11-19): an IP address, IPv4, or IPv6 with or without brackets (io.md 2), or a DNS
/// name of at most 253 bytes of ASCII letters, digits, `-`, `.` and `_`. It is given back without its brackets; any other
/// text is no host.
pub(crate) fn host_of(host: &str) -> Option<&str> {
    if let Some(inner) = host.strip_prefix('[').and_then(|inner| inner.strip_suffix(']')) {
        return inner.parse::<Ipv6Addr>().is_ok().then_some(inner);
    }
    let name =
        !host.is_empty() && host.len() <= MAX_NAME && host.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_'));
    (name || host.parse::<IpAddr>().is_ok()).then_some(host)
}

/// The endpoint of `net.connect(host, port)` or `net.open_udp(host, port)`: a text that is no host is
/// `net.host_not_found` at once, before the simulation file and any question (io.md 3); port 0 is `io.invalid_argument`
/// (io.md 8).
pub(crate) fn of_call(udp: bool, host: &str, port: u16) -> Result<Endpoint, NativeOutcome> {
    let Some(bare) = host_of(host) else {
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
    Ok(Endpoint { text: format!("{}:{}:{port}", scheme(udp), written(bare)), given: format!("{host}:{port}"), host: bare.to_owned(), port })
}

/// The endpoint a simulation file names (`endpoint = "tcp:10.0.0.7:502"`), as a program reaches it, and whether it is UDP;
/// `None` when it is not `tcp:HOST:PORT` or `udp:HOST:PORT`.
pub(crate) fn of_file(text: &str) -> Option<(String, bool)> {
    let (udp, rest) = match (text.strip_prefix("tcp:"), text.strip_prefix("udp:")) {
        (Some(rest), _) => (false, rest),
        (None, Some(rest)) => (true, rest),
        (None, None) => return None,
    };
    let (host, port) = match rest.strip_prefix('[') {
        Some(inner) => inner.split_once("]:").filter(|(host, _)| host.parse::<Ipv6Addr>().is_ok())?,
        None => rest.rsplit_once(':').and_then(|(host, port)| Some((host_of(host)?, port)))?,
    };
    if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    // `0502` is 502 (io.md 2); `0` and `00` are no port.
    let digits = port.trim_start_matches('0');
    if digits.is_empty() {
        return None;
    }
    let port: u16 = digits.parse().ok()?;
    Some((format!("{}:{}:{port}", scheme(udp), written(host)), udp))
}
