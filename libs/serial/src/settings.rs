//! The settings of a port (`libs/serial/README.md`, `open`): what an open gives, and what a simulated device expects. An
//! open with settings its device does not expect gets no response from it, and the report says which differ (io.md 6;
//! decision of the owner, 2026-11-20).

use labcraft_runtime::native::{Setting, SettingDiff};

/// `serial.Parity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Parity {
    None,
    Odd,
    Even,
}

/// `serial.StopBits`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StopBits {
    One,
    Two,
}

/// `serial.Flow`: XON/XOFF, RTS/CTS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Flow {
    None,
    Software,
    Hardware,
}

/// Each value of `serial.Parity`, in the order of its declaration, with its word in a file and in the report.
pub(crate) const PARITY: [(&str, Parity); 3] = [("none", Parity::None), ("odd", Parity::Odd), ("even", Parity::Even)];
/// Each value of `serial.StopBits`, in the order of its declaration, with its number.
const STOP_BITS: [(i64, StopBits); 2] = [(1, StopBits::One), (2, StopBits::Two)];
/// Each value of `serial.Flow`, in the order of its declaration, with its word.
pub(crate) const FLOW: [(&str, Flow); 3] = [("none", Flow::None), ("software", Flow::Software), ("hardware", Flow::Hardware)];

/// The value at `index` of its declaration, as a variant carries it.
fn of_index<K, T: Copy>(table: &[(K, T)], index: u32) -> Option<T> {
    table.get(usize::try_from(index).ok()?).map(|(_, value)| *value)
}

/// What names `value` in its table.
fn name_of<K: Copy + Default, T: PartialEq>(table: &[(K, T)], value: T) -> K {
    table.iter().find(|(_, other)| *other == value).map(|(name, _)| *name).unwrap_or_default()
}

impl Parity {
    pub(crate) fn of_index(index: u32) -> Option<Parity> {
        of_index(&PARITY, index)
    }

    pub(crate) fn setting(self) -> Setting {
        Setting::Text(name_of(&PARITY, self))
    }
}

impl StopBits {
    pub(crate) fn of_index(index: u32) -> Option<StopBits> {
        of_index(&STOP_BITS, index)
    }

    pub(crate) fn of_number(number: i64) -> Option<StopBits> {
        STOP_BITS.iter().find(|(other, _)| *other == number).map(|(_, stop)| *stop)
    }

    pub(crate) fn setting(self) -> Setting {
        Setting::Integer(name_of(&STOP_BITS, self))
    }
}

impl Flow {
    pub(crate) fn of_index(index: u32) -> Option<Flow> {
        of_index(&FLOW, index)
    }

    pub(crate) fn setting(self) -> Setting {
        Setting::Text(name_of(&FLOW, self))
    }
}

/// The lowest and highest `data_bits` of `open`.
pub(crate) const DATA_BITS: std::ops::RangeInclusive<u8> = 5..=8;

/// What an open gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    /// Above 0.
    pub(crate) baud: u32,
    /// 5 to 8.
    pub(crate) data_bits: u8,
    pub(crate) parity: Parity,
    pub(crate) stop_bits: StopBits,
    pub(crate) flow: Flow,
}

/// What a simulated device expects: each setting it names; one it does not name is any.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Expected {
    pub(crate) baud: Option<u32>,
    pub(crate) data_bits: Option<u8>,
    pub(crate) parity: Option<Parity>,
    pub(crate) stop_bits: Option<StopBits>,
    pub(crate) flow: Option<Flow>,
}

/// The settings by their names, in the order of `open`.
pub(crate) const NAMES: [&str; 5] = ["baud", "data_bits", "parity", "stop_bits", "flow"];

fn number(number: u32) -> Setting {
    Setting::Integer(i64::from(number))
}

impl Settings {
    /// Each setting, in the order of [`NAMES`].
    fn all(&self) -> [Setting; 5] {
        [number(self.baud), number(u32::from(self.data_bits)), self.parity.setting(), self.stop_bits.setting(), self.flow.setting()]
    }
}

impl Expected {
    /// Each setting the device expects, in the order of [`NAMES`]; `None`: any.
    pub(crate) fn all(&self) -> [Option<Setting>; 5] {
        [
            self.baud.map(number),
            self.data_bits.map(|bits| number(u32::from(bits))),
            self.parity.map(Parity::setting),
            self.stop_bits.map(StopBits::setting),
            self.flow.map(Flow::setting),
        ]
    }

    /// The settings of an open that differ from what the device expects, in the order of `open`: none when it takes them.
    pub(crate) fn differ(&self, opened: &Settings) -> Vec<SettingDiff> {
        NAMES
            .into_iter()
            .zip(self.all())
            .zip(opened.all())
            .filter_map(|((name, expected), opened)| {
                expected.filter(|expected| *expected != opened).map(|expected| SettingDiff { name, expected, opened })
            })
            .collect()
    }
}
