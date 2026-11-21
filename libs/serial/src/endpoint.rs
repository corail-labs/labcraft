//! The endpoint of a serial port (`docs/io.md`, section 2): `serial:PORT`, whose form decides how it compares, never the
//! system that runs it. `COM<n>`, `n` from 1, is compared without case and written upper case (`com3` is `serial:COM3`);
//! any other name, a path such as `/dev/ttyUSB0`, is compared exactly. The name a program gives and the name a simulation
//! file gives reduce to the same text, so that they compare.

use labcraft_runtime::io::Reach;
use labcraft_runtime::native::NativeOutcome;
pub(crate) use labcraft_runtime::stream::shown;

pub(crate) const PORT_NOT_FOUND: &str = "serial.port_not_found";
pub(crate) const PORT_BUSY: &str = "serial.port_busy";

/// What an open reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Endpoint {
    /// `serial:COM3`
    pub(crate) text: String,
    /// What the program gave, which the question of consent shows (io.md 4): `com3`.
    pub(crate) given: String,
}

impl Endpoint {
    /// What the run asks the right to reach (io.md 4).
    pub(crate) fn reach(&self) -> Reach {
        Reach::Endpoint { endpoint: self.text.clone(), given: self.given.clone() }
    }

    /// What a message shows of it: its start, since the name is the program's and has no bound.
    pub(crate) fn shown(&self) -> String {
        shown(&self.text)
    }

    /// The name of the port, without `serial:`.
    #[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "read by the real open only, which WASM has not"))]
    pub(crate) fn port(&self) -> &str {
        self.text.strip_prefix("serial:").unwrap_or(&self.text)
    }
}

/// `COM<n>`, `n` a number from 1 written without zeros in front, in any case: its number, written in decimal.
pub(crate) fn com_number(name: &str) -> Option<&str> {
    let prefix = name.get(..3)?;
    let digits = name.get(3..)?;
    let leading = digits.bytes().next()?;
    (prefix.eq_ignore_ascii_case("com") && (b'1'..=b'9').contains(&leading) && digits.bytes().all(|byte| byte.is_ascii_digit())).then_some(digits)
}

/// A port as an endpoint writes it (io.md 2): `COM<n>` upper case, any other name as it is. A name is not empty and holds no
/// control character (the smallest rule that io.md 2 leaves: a name it compares exactly, with no character that no system
/// takes in a port name); any other text is no name.
pub(crate) fn port_of(name: &str) -> Option<String> {
    if name.is_empty() || name.chars().any(char::is_control) {
        return None;
    }
    Some(match com_number(name) {
        Some(number) => format!("COM{number}"),
        None => name.to_owned(),
    })
}

/// The endpoint of `serial.open(port)`: a text that is no name is `serial.port_not_found` at once, code 0, before the
/// simulation file and any question (io.md 3).
pub(crate) fn of_call(name: &str) -> Result<Endpoint, NativeOutcome> {
    let Some(port) = port_of(name) else {
        let message = if name.is_empty() {
            "an empty name names no port".to_owned()
        } else {
            format!("`{}` is no port name: a name holds no control character", shown(name))
        };
        return Err(NativeOutcome::Fail { category: PORT_NOT_FOUND, code: 0, message });
    };
    Ok(Endpoint { text: format!("serial:{port}"), given: name.to_owned() })
}

/// The endpoint a simulation file names (`endpoint = "serial:com3"`), as a program reaches it (`serial:COM3`); `None` when it
/// is not `serial:PORT`.
pub(crate) fn of_file(text: &str) -> Option<(String, ())> {
    let port = port_of(text.strip_prefix("serial:")?)?;
    Some((format!("serial:{port}"), ()))
}
