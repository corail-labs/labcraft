//! Endpoints (`docs/io.md`, section 2), as one text that compares: `tcp:HOST:PORT` and `udp:HOST:PORT`, the host in lower
//! case for its ASCII letters, an IPv6 address in brackets whether it was given with them or not, the port in decimal without
//! zeros in front; `serial:PORT`, `COM<n>` upper case and any other name as it is; and the VISA names that reduce to one.
//! The name a program gives, the name a simulation file gives and the name of `--allow-hardware` reduce to the same text,
//! so that one consent covers them.

use std::net::{IpAddr, Ipv6Addr};

/// The longest DNS name, in bytes.
const MAX_NAME: usize = 253;

/// A host (decision of the owner, 2026-11-19): an IP address, IPv4, or IPv6 with or without brackets (io.md 2), or a DNS
/// name of at most 253 bytes of ASCII letters, digits, `-`, `.` and `_`. It is given back without its brackets; any other
/// text is no host.
pub fn host(host: &str) -> Option<&str> {
    if let Some(inner) = host.strip_prefix('[').and_then(|inner| inner.strip_suffix(']')) {
        return inner.parse::<Ipv6Addr>().is_ok().then_some(inner);
    }
    let name =
        !host.is_empty() && host.len() <= MAX_NAME && host.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_'));
    (name || host.parse::<IpAddr>().is_ok()).then_some(host)
}

/// The host as an endpoint writes it, from a host without brackets.
fn written_host(host: &str) -> String {
    let lower = host.to_ascii_lowercase();
    if lower.contains(':') { format!("[{lower}]") } else { lower }
}

/// A port written in decimal, zeros in front allowed (`0502` is 502, io.md 2), from 1 to 65,535; any other text is none
/// (`+502` is no port, though Rust would read it).
fn port(digits: &str) -> Option<u16> {
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|port| *port != 0)
}

/// The text of a TCP or UDP endpoint, from a host without brackets that [`host`] read.
pub fn tcp_or_udp_text(udp: bool, host: &str, port: u16) -> String {
    format!("{}:{}:{port}", if udp { "udp" } else { "tcp" }, written_host(host))
}

/// `tcp:HOST:PORT` or `udp:HOST:PORT`, as an endpoint writes it, and whether it is UDP; `None` for any other text.
pub fn tcp_or_udp(text: &str) -> Option<(String, bool)> {
    let (udp, rest) = match (text.strip_prefix("tcp:"), text.strip_prefix("udp:")) {
        (Some(rest), _) => (false, rest),
        (None, Some(rest)) => (true, rest),
        (None, None) => return None,
    };
    let (bare, digits) = match rest.strip_prefix('[') {
        Some(inner) => inner.split_once("]:").filter(|(bare, _)| bare.parse::<Ipv6Addr>().is_ok())?,
        None => rest.rsplit_once(':').and_then(|(given, digits)| Some((host(given)?, digits)))?,
    };
    Some((tcp_or_udp_text(udp, bare, port(digits)?), udp))
}

/// `COM<n>`, `n` a number from 1 written without zeros in front, in any case: its number, written in decimal.
pub fn com_number(name: &str) -> Option<&str> {
    let prefix = name.get(..3)?;
    let digits = name.get(3..)?;
    let leading = digits.bytes().next()?;
    (prefix.eq_ignore_ascii_case("com") && (b'1'..=b'9').contains(&leading) && digits.bytes().all(|byte| byte.is_ascii_digit())).then_some(digits)
}

/// A serial port as an endpoint writes it (io.md 2): `COM<n>` upper case, any other name as it is. A name is not empty and
/// holds no control character (the smallest rule that io.md 2 leaves: a name it compares exactly, with no character that no
/// system takes in a port name); any other text is no name.
pub fn serial_port(name: &str) -> Option<String> {
    if name.is_empty() || name.chars().any(char::is_control) {
        return None;
    }
    Some(match com_number(name) {
        Some(number) => format!("COM{number}"),
        None => name.to_owned(),
    })
}

/// `serial:PORT`, as an endpoint writes it (`serial:com3` is `serial:COM3`); `None` for any other text.
pub fn serial(text: &str) -> Option<String> {
    Some(format!("serial:{}", serial_port(text.strip_prefix("serial:")?)?))
}

/// `text` without `suffix`, compared without case, as VISA reads its words.
fn strip_suffix_ignore_case<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    let at = text.len().checked_sub(suffix.len())?;
    let (head, tail) = (text.get(..at)?, text.get(at..)?);
    tail.eq_ignore_ascii_case(suffix).then_some(head)
}

/// `text` without `prefix`, compared without case.
fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let (head, tail) = (text.get(..prefix.len())?, text.get(prefix.len()..)?);
    head.eq_ignore_ascii_case(prefix).then_some(tail)
}

/// The endpoint a VISA name reduces to (io.md 2), its VISA words without case: `TCPIP[n]::HOST::PORT::SOCKET` to
/// `tcp:HOST:PORT`, the host as [`host`] reads it, an IPv6 address in brackets only, since `::` also separates the words
/// (`TCPIP0::fe80::1::SOCKET` is the host `fe80` and the port 1); `ASRL<n>::INSTR`, `n` from 1,
/// to `serial:COM<n>`; `ASRL<path>::INSTR`, a path that starts with `/`, to `serial:<path>`, compared exactly. `None` for
/// any other text: `ASRL0::INSTR`, and the VISA forms that come later (VXI-11, HiSLIP, USB, GPIB), which the library tells
/// apart.
pub fn of_visa(name: &str) -> Option<String> {
    if let Some(rest) = strip_suffix_ignore_case(name, "::SOCKET") {
        let rest = strip_prefix_ignore_case(rest, "TCPIP")?;
        let (board, rest) = rest.split_once("::")?;
        if !board.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let (given, digits) = rest.rsplit_once("::")?;
        if given.contains(':') && !given.starts_with('[') {
            return None;
        }
        return Some(tcp_or_udp_text(false, host(given)?, port(digits)?));
    }
    let inner = strip_prefix_ignore_case(strip_suffix_ignore_case(name, "::INSTR")?, "ASRL")?;
    if inner.starts_with('/') {
        return serial_port(inner).map(|path| format!("serial:{path}"));
    }
    let leading = inner.bytes().next()?;
    ((b'1'..=b'9').contains(&leading) && inner.bytes().all(|byte| byte.is_ascii_digit())).then(|| format!("serial:COM{inner}"))
}

/// The endpoint of `text`: an endpoint of io.md 2 or a VISA name that reduces to one, as `--allow-hardware` takes it
/// (io.md 4); `None` for any other text. Names are exact: no wildcard.
pub fn of_text(text: &str) -> Option<String> {
    tcp_or_udp(text).map(|(endpoint, _)| endpoint).or_else(|| serial(text)).or_else(|| of_visa(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The values of the issue #232 and of io.md 2, read from the document, not from this code.
    #[test]
    fn every_form_of_io_md_2_reduces_to_its_endpoint() {
        let cases = [
            ("tcp:10.0.0.5:5025", "tcp:10.0.0.5:5025"),
            ("TCPIP0::10.0.0.5::5025::SOCKET", "tcp:10.0.0.5:5025"),
            ("tcpip0::10.0.0.5::5025::socket", "tcp:10.0.0.5:5025"),
            ("TCPIP::10.0.0.5::5025::SOCKET", "tcp:10.0.0.5:5025"),
            ("tcp:LOCALHOST:502", "tcp:localhost:502"),
            ("TCPIP0::h::0502::SOCKET", "tcp:h:502"),
            ("TCPIP0::DMM.Local::5025::SOCKET", "tcp:dmm.local:5025"),
            ("tcp:[fe80::1]:502", "tcp:[fe80::1]:502"),
            ("tcp:FE80::1:502", "tcp:[fe80::1]:502"),
            ("TCPIP0::[fe80::1]::502::SOCKET", "tcp:[fe80::1]:502"),
            ("TCPIP0::fe80::1::SOCKET", "tcp:fe80:1"),
            ("udp:10.0.0.5:0502", "udp:10.0.0.5:502"),
            ("serial:com3", "serial:COM3"),
            ("serial:COM3", "serial:COM3"),
            ("ASRL3::INSTR", "serial:COM3"),
            ("asrl12::instr", "serial:COM12"),
            ("serial:/dev/ttyUSB0", "serial:/dev/ttyUSB0"),
            ("ASRL/dev/ttyUSB0::INSTR", "serial:/dev/ttyUSB0"),
            ("serial:COM03", "serial:COM03"),
        ];
        for (given, endpoint) in cases {
            assert_eq!(of_text(given).as_deref(), Some(endpoint), "{given}");
        }
    }

    #[test]
    fn a_text_that_is_no_endpoint_and_no_such_visa_name_is_none() {
        for given in [
            "banana",
            "ASRL0",
            "ASRL0::INSTR",
            "ASRL03::INSTR",
            "ASRL::INSTR",
            "tcp:10.0.0.5",
            "tcp:10.0.0.5:0",
            "tcp:10.0.0.5:65536",
            "tcp:10.0.0.5:+502",
            "tcp:10.0.0.*:502",
            "tcp::502",
            "serial:",
            "serial:COM\u{7}",
            "TCPIP0::10.0.0.5::0::SOCKET",
            "TCPIP0::fe80::1::502::SOCKET",
            "TCPIPX::10.0.0.5::5025::SOCKET",
            "TCPIP0::10.0.0.5::INSTR",
            "TCPIP0::10.0.0.5::hislip0::INSTR",
            "USB0::0x1234::0x5678::SN::INSTR",
            "GPIB0::5::INSTR",
            "SIM::DMM0",
            "",
        ] {
            assert_eq!(of_text(given), None, "{given:?}");
        }
    }

    #[test]
    fn a_name_is_never_resolved() {
        assert_ne!(of_text("tcp:localhost:502"), of_text("tcp:127.0.0.1:502"));
    }
}
