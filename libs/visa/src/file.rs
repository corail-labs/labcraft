//! The `visa` section of a simulation file, judged before the run (`libs/visa/simulation.md`, sections 2, 3, 5 and 7): each
//! problem is a diagnostic with its code, at its key or its value, and every problem of the file is said, not the first only.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use labcraft_runtime::native::{LibraryDiagnostic, Section};
use labcraft_runtime::simulation::{Places, float_of, integer_of, nearest};
use labcraft_types::Severity;
use toml::Spanned;
use toml::de::{DeTable, DeValue};

/// What a file says of its instruments, judged, for every run that uses it.
#[derive(Debug)]
pub struct Bench {
    pub(crate) instruments: Vec<InstrumentSpec>,
    /// Of every instrument, by a number of the bench: a `source` names one by it.
    pub(crate) quantities: Vec<QuantitySpec>,
}

#[derive(Debug)]
pub(crate) struct InstrumentSpec {
    pub(crate) resource: String,
    pub(crate) idn: String,
    /// Nanoseconds on the clock of the run.
    pub(crate) latency: i64,
    pub(crate) commands: HashMap<String, String>,
    /// A query and the number of its quantity in the bench.
    pub(crate) measure: HashMap<String, usize>,
    pub(crate) set: HashMap<String, usize>,
    pub(crate) accept: Vec<String>,
    pub(crate) timeouts: HashSet<String>,
    pub(crate) error_on: HashMap<String, (i64, String)>,
    pub(crate) error_queue: Vec<(i64, String)>,
}

#[derive(Debug)]
pub(crate) struct QuantitySpec {
    pub(crate) value: f64,
    pub(crate) initial: f64,
    pub(crate) start: i64,
    pub(crate) settle: i64,
    pub(crate) noise: f64,
    pub(crate) seed: u64,
    /// The quantity whose true value it takes: itself, or the end of its chain of sources.
    pub(crate) root: usize,
}

const MAX_TEXT: usize = 1000;
const MAX_INSTRUMENTS: usize = 256;
const MAX_QUANTITIES: usize = 1000;
const MAX_ENTRIES: usize = 10_000;
/// The size of the error queue (simulation.md 4) *(proposed)*.
pub(crate) const QUEUE_SIZE: usize = 32;
const DAY_MS: f64 = 86_400_000.0;
const MAX_VALUE: f64 = 1e100;
const INSTRUMENT_KEYS: &[&str] = &["resource", "idn", "latency_ms", "commands", "measure", "set", "accept", "quantity", "faults"];
const QUANTITY_KEYS: &[&str] = &["value", "initial", "start_ms", "settle_ms", "noise", "seed", "source"];
const FAULT_KEYS: &[&str] = &["timeout", "error_on", "error_queue"];
/// What the simulator answers itself.
pub(crate) const RESERVED: &[&str] = &["*IDN?", "SYST:ERR?", "*CLS", "*RST"];

type Value<'a> = Spanned<DeValue<'a>>;

/// A time of the file in milliseconds, as nanoseconds (simulation.md 3): times 10^6 as an `f64`, then to the nearest integer,
/// ties to even. A time is at most one day, so it fits.
pub(crate) fn nanoseconds(ms: f64) -> i64 {
    (ms * 1e6).round_ties_even() as i64
}

struct Reader<'s> {
    places: &'s Places<'s>,
    problems: Vec<LibraryDiagnostic>,
}

impl Reader<'_> {
    fn say(&mut self, code: &'static str, message: String, at: Range<usize>) {
        self.problems.push(LibraryDiagnostic { code, severity: Severity::Error, message, span: Some(self.places.span(at)), fix: None });
    }

    fn table<'v, 'a>(&mut self, value: &'v Value<'a>, what: &str) -> Option<&'v DeTable<'a>> {
        match value.get_ref() {
            DeValue::Table(table) => Some(table),
            other => {
                self.say("visa:S002", format!("{what} is a table, and this is {}", other.type_str()), value.span());
                None
            }
        }
    }

    /// The keys of a table that the format has; each other one is `visa:S001`, with the nearest key.
    fn keys(&mut self, table: &DeTable<'_>, known: &[&str], what: &str) {
        for (key, _) in table.iter() {
            let name = key.get_ref().as_ref();
            if !known.contains(&name) {
                let fix = nearest(name, known);
                let hint = fix.map_or_else(String::new, |fix| format!(": did you mean `{fix}`?"));
                self.problems.push(LibraryDiagnostic {
                    code: "visa:S001",
                    severity: Severity::Error,
                    message: format!("{what} has no key `{}`{hint}", shown(name)),
                    span: Some(self.places.span(key.span())),
                    fix: fix.map(str::to_owned),
                });
            }
        }
    }

    fn entries(&mut self, count: usize, at: Range<usize>, what: &str) -> bool {
        if count > MAX_ENTRIES {
            self.say("visa:S010", format!("{what} has {count} entries, more than {MAX_ENTRIES} (simulation.md 7)"), at);
            return false;
        }
        true
    }

    /// A text of at most 1,000 code points, with no control character (simulation.md 2).
    fn text<'v>(&mut self, value: &'v Value<'_>, what: &str) -> Option<&'v str> {
        let DeValue::String(text) = value.get_ref() else {
            self.say("visa:S002", format!("{what} is a text, and this is {}", value.get_ref().type_str()), value.span());
            return None;
        };
        self.text_rules(text, value.span(), what).then_some(text.as_ref())
    }

    fn text_rules(&mut self, text: &str, at: Range<usize>, what: &str) -> bool {
        if text.chars().count() > MAX_TEXT || text.chars().any(char::is_control) {
            self.say("visa:S008", format!("{what} is a text of at most {MAX_TEXT} characters, with no control character"), at);
            return false;
        }
        true
    }

    /// A command as a key (simulation.md 2, `visa:S011`): not empty, no space at an end, not one the simulator answers itself,
    /// and a query when `query`.
    fn command(&mut self, key: &Spanned<std::borrow::Cow<'_, str>>, query: bool, what: &str) -> Option<String> {
        let text = key.get_ref().as_ref();
        self.command_text(text, key.span(), query, what)
    }

    fn command_text(&mut self, text: &str, at: Range<usize>, query: bool, what: &str) -> Option<String> {
        if !self.text_rules(text, at.clone(), what) {
            return None;
        }
        if text.is_empty() || text.starts_with(' ') || text.ends_with(' ') {
            self.say("visa:S011", format!("{what}: `{}` is empty, or has a space at an end", shown(text)), at);
            return None;
        }
        if RESERVED.contains(&text) {
            self.say("visa:S011", format!("{what}: `{text}` is answered by the simulator itself"), at);
            return None;
        }
        if query && !text.ends_with('?') {
            self.say("visa:S011", format!("{what}: `{}` is not a query, which ends with `?`", shown(text)), at);
            return None;
        }
        Some(text.to_owned())
    }

    /// A number of the file, an integer or a float, within `low..=high`, finite.
    fn number(&mut self, table: &DeTable<'_>, key: &str, default: f64, low: f64, high: f64, what: &str) -> f64 {
        let Some(value) = get(table, key) else { return default };
        let read = match value.get_ref() {
            DeValue::Integer(integer) => integer_of(integer).map(|x| x as f64),
            DeValue::Float(float) => float_of(float),
            other => {
                self.say("visa:S002", format!("{what}: `{key}` is a number, and this is {}", other.type_str()), value.span());
                return default;
            }
        };
        match read {
            Some(x) if x.is_finite() && (low..=high).contains(&x) => x,
            _ => {
                self.say("visa:S005", format!("{what}: `{key}` is out of its range, {} to {}", plain(low), plain(high)), value.span());
                default
            }
        }
    }

    /// `[code, "message"]`, the code an `i32` (simulation.md 5).
    fn error(&mut self, value: &Value<'_>, what: &str) -> Option<(i64, String)> {
        let DeValue::Array(items) = value.get_ref() else {
            self.say("visa:S009", format!("{what}: an error is `[code, \"message\"]`"), value.span());
            return None;
        };
        let (Some(code), Some(message), 2) = (items.first(), items.get(1), items.len()) else {
            self.say("visa:S009", format!("{what}: an error is `[code, \"message\"]`"), value.span());
            return None;
        };
        let DeValue::Integer(integer) = code.get_ref() else {
            self.say("visa:S009", format!("{what}: an error is `[code, \"message\"]`, its code an integer"), code.span());
            return None;
        };
        let DeValue::String(_) = message.get_ref() else {
            self.say("visa:S009", format!("{what}: an error is `[code, \"message\"]`, its message a text"), message.span());
            return None;
        };
        let code_value = integer_of(integer).filter(|code| i32::try_from(*code).is_ok());
        let Some(code_value) = code_value else {
            self.say("visa:S005", format!("{what}: an error code is from -2^31 to 2^31-1"), code.span());
            return None;
        };
        let message = self.text(message, what)?;
        Some((code_value, message.to_owned()))
    }
}

fn get<'v, 'a>(table: &'v DeTable<'a>, key: &str) -> Option<&'v Value<'a>> {
    table.iter().find(|(name, _)| name.get_ref().as_ref() == key).map(|(_, value)| value)
}

/// What a message shows of a text of the file: its start, its control characters escaped.
fn shown(text: &str) -> String {
    let start: String = text.chars().take(64).flat_map(char::escape_debug).collect();
    if text.chars().count() > 64 { format!("{start}…") } else { start }
}

fn plain(x: f64) -> String {
    if x.abs() >= 1e6 { format!("{x:e}") } else { format!("{x}") }
}

fn is_ascii_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A quantity as written, before its sources are resolved.
struct Written {
    instrument: usize,
    name: String,
    source: Option<(String, Range<usize>)>,
}

/// Judges the `visa` section: a `Bench`, or every problem found.
pub(crate) fn read(section: &Section<'_>) -> Result<Bench, Vec<LibraryDiagnostic>> {
    let mut reader = Reader { places: section.places, problems: Vec::new() };
    let mut bench = Bench { instruments: Vec::new(), quantities: Vec::new() };
    let mut written: Vec<Written> = Vec::new();
    if let Some(table) = reader.table(section.value, "`visa`") {
        reader.keys(table, &["instrument"], "`visa`");
        if let Some(instruments) = get(table, "instrument") {
            match instruments.get_ref() {
                DeValue::Array(items) if items.len() > MAX_INSTRUMENTS => {
                    reader.say("visa:S010", format!("{} instruments, more than {MAX_INSTRUMENTS} (simulation.md 7)", items.len()), instruments.span())
                }
                DeValue::Array(items) => {
                    let mut seen: HashMap<String, ()> = HashMap::new();
                    for item in items.iter() {
                        instrument(&mut reader, item, &mut bench, &mut written, &mut seen);
                    }
                }
                other => {
                    reader.say("visa:S002", format!("`visa.instrument` is an array of tables, and this is {}", other.type_str()), instruments.span())
                }
            }
        }
    }
    resolve_sources(&mut reader, &mut bench, &written);
    if reader.problems.is_empty() { Ok(bench) } else { Err(reader.problems) }
}

fn instrument(reader: &mut Reader<'_>, item: &Value<'_>, bench: &mut Bench, written: &mut Vec<Written>, seen: &mut HashMap<String, ()>) {
    let Some(table) = reader.table(item, "an instrument") else { return };
    let Some(resource_value) = get(table, "resource") else {
        reader.say("visa:S003", "an instrument without `resource`".to_owned(), item.span());
        return;
    };
    let Some(resource) = reader.text(resource_value, "`resource`").map(str::to_owned) else { return };
    if !resource.starts_with("SIM::") {
        reader.say("visa:S003", format!("the resource `{}` does not start with `SIM::`", shown(&resource)), resource_value.span());
        return;
    }
    if seen.insert(resource.clone(), ()).is_some() {
        reader.say("visa:S004", format!("the resource `{}` is defined twice", shown(&resource)), resource_value.span());
        return;
    }
    let what = format!("`{}`", shown(&resource));
    reader.keys(table, INSTRUMENT_KEYS, &what);
    let idn = match get(table, "idn") {
        Some(value) => reader.text(value, &format!("{what}: `idn`")).map(str::to_owned).unwrap_or_default(),
        None => format!("SIM,{resource},0,1.0"),
    };
    let latency = nanoseconds(reader.number(table, "latency_ms", 0.0, 0.0, DAY_MS, &what));
    let index = bench.instruments.len();

    // Quantities first: `measure` and `set` name them.
    let mut names: HashMap<String, usize> = HashMap::new();
    let mut with_source: HashSet<usize> = HashSet::new();
    if let Some(value) = get(table, "quantity")
        && let Some(quantities) = reader.table(value, &format!("{what}: `quantity`"))
    {
        if quantities.len() > MAX_QUANTITIES {
            reader.say("visa:S010", format!("{what} has {} quantities, more than {MAX_QUANTITIES}", quantities.len()), value.span());
        } else {
            for (key, value) in quantities.iter() {
                let name = key.get_ref().as_ref();
                let where_ = format!("{what}: quantity `{}`", shown(name));
                if !is_ascii_name(name) {
                    reader.say(
                        "visa:S008",
                        format!("{where_}: a quantity name is an ASCII name (a letter or `_`, then letters, digits or `_`)"),
                        key.span(),
                    );
                    continue;
                }
                if let Some(id) = quantity(reader, value, &where_, bench) {
                    let source = get_table(value)
                        .and_then(|table| get(table, "source"))
                        .and_then(|source| reader.text(source, &format!("{where_}: `source`")).map(|text| (text.to_owned(), source.span())));
                    if source.is_some() {
                        with_source.insert(id);
                    }
                    names.insert(name.to_owned(), id);
                    written.push(Written { instrument: index, name: name.to_owned(), source });
                }
            }
        }
    }

    let mut spec = InstrumentSpec {
        resource,
        idn,
        latency,
        commands: HashMap::new(),
        measure: HashMap::new(),
        set: HashMap::new(),
        accept: Vec::new(),
        timeouts: HashSet::new(),
        error_on: HashMap::new(),
        error_queue: Vec::new(),
    };
    if let Some(value) = get(table, "commands")
        && let Some(commands) = reader.table(value, &format!("{what}: `commands`"))
        && reader.entries(commands.len(), value.span(), &format!("{what}: `commands`"))
    {
        for (key, answer) in commands.iter() {
            let label = format!("{what}: `commands`");
            if let (Some(command), Some(answer)) = (reader.command(key, true, &label), reader.text(answer, &label)) {
                spec.commands.insert(command, answer.to_owned());
            }
        }
    }
    let mut measured_at: HashMap<String, Range<usize>> = HashMap::new();
    for (field, query) in [("measure", true), ("set", false)] {
        let Some(value) = get(table, field) else { continue };
        let label = format!("{what}: `{field}`");
        let Some(entries) = reader.table(value, &label) else { continue };
        if !reader.entries(entries.len(), value.span(), &label) {
            continue;
        }
        for (key, named) in entries.iter() {
            let (Some(command), Some(name)) = (reader.command(key, query, &label), reader.text(named, &label)) else { continue };
            match names.get(name) {
                None => reader.say(
                    "visa:S006",
                    format!("{label}: `{command}` names the quantity `{}`, which {what} does not have", shown(name)),
                    named.span(),
                ),
                Some(id) if field == "set" && with_source.contains(id) => {
                    reader.say("visa:S006", format!("{label}: `{command}` sets `{name}`, which takes its value from a `source`"), named.span());
                }
                Some(id) if field == "measure" => {
                    measured_at.insert(command.clone(), key.span());
                    spec.measure.insert(command, *id);
                }
                Some(id) => {
                    spec.set.insert(command, *id);
                }
            }
        }
    }
    // In code point order, so that two reads of a file say the same.
    let mut both: Vec<&String> = spec.commands.keys().filter(|command| spec.measure.contains_key(*command)).collect();
    both.sort();
    for command in both {
        let at = measured_at.get(command).cloned().unwrap_or_else(|| item.span());
        reader.say("visa:S011", format!("{what}: `{command}` is in both `commands` and `measure`"), at);
    }
    if let Some(value) = get(table, "accept") {
        accept(reader, value, &what, &mut spec);
    }
    if let Some(value) = get(table, "faults")
        && let Some(faults) = reader.table(value, &format!("{what}: `faults`"))
    {
        reader.keys(faults, FAULT_KEYS, &format!("{what}: `faults`"));
        self_faults(reader, faults, &what, &mut spec);
    }
    bench.instruments.push(spec);
}

fn get_table<'v, 'a>(value: &'v Value<'a>) -> Option<&'v DeTable<'a>> {
    match value.get_ref() {
        DeValue::Table(table) => Some(table),
        _ => None,
    }
}

/// One quantity: its number in the bench, or `None` after its problems are said.
fn quantity(reader: &mut Reader<'_>, value: &Value<'_>, what: &str, bench: &mut Bench) -> Option<usize> {
    let table = reader.table(value, what)?;
    reader.keys(table, QUANTITY_KEYS, what);
    let has_source = get(table, "source").is_some();
    if has_source {
        for key in ["value", "initial", "start_ms", "settle_ms"] {
            if let Some(moving) = get(table, key) {
                reader.say("visa:S007", format!("{what}: a quantity with a `source` takes its value from it, and has no `{key}`"), moving.span());
            }
        }
    }
    let target = reader.number(table, "value", 0.0, -MAX_VALUE, MAX_VALUE, what);
    let initial = reader.number(table, "initial", target, -MAX_VALUE, MAX_VALUE, what);
    let start = nanoseconds(reader.number(table, "start_ms", 0.0, 0.0, DAY_MS, what));
    let settle = nanoseconds(reader.number(table, "settle_ms", 0.0, 0.0, DAY_MS, what));
    let noise = reader.number(table, "noise", 0.0, 0.0, MAX_VALUE, what);
    let seed = match get(table, "seed") {
        None => {
            if noise > 0.0 {
                reader.say("visa:S003", format!("{what}: a quantity with a noise needs its own `seed`"), value.span());
            }
            0
        }
        Some(seed) => match seed.get_ref() {
            DeValue::Integer(integer) => match integer_of(integer).and_then(|seed| u64::try_from(seed).ok()) {
                Some(seed) => seed,
                None => {
                    reader.say("visa:S005", format!("{what}: `seed` is from 0 to 2^63-1"), seed.span());
                    0
                }
            },
            other => {
                reader.say("visa:S002", format!("{what}: `seed` is an integer, and this is {}", other.type_str()), seed.span());
                0
            }
        },
    };
    let id = bench.quantities.len();
    bench.quantities.push(QuantitySpec { value: target, initial, start, settle, noise, seed, root: id });
    Some(id)
}

fn accept(reader: &mut Reader<'_>, value: &Value<'_>, what: &str, spec: &mut InstrumentSpec) {
    let label = format!("{what}: `accept`");
    let DeValue::Array(items) = value.get_ref() else {
        reader.say("visa:S002", format!("{label} is an array of texts, and this is {}", value.get_ref().type_str()), value.span());
        return;
    };
    if !reader.entries(items.len(), value.span(), &label) {
        return;
    }
    for item in items.iter() {
        if let Some(prefix) = reader.text(item, &label) {
            spec.accept.push(prefix.to_owned());
        }
    }
}

fn self_faults(reader: &mut Reader<'_>, faults: &DeTable<'_>, what: &str, spec: &mut InstrumentSpec) {
    if let Some(value) = get(faults, "timeout") {
        let label = format!("{what}: `faults.timeout`");
        match value.get_ref() {
            DeValue::Array(items) if reader.entries(items.len(), value.span(), &label) => {
                for item in items.iter() {
                    let Some(text) = reader.text(item, &label).map(str::to_owned) else { continue };
                    if let Some(query) = reader.command_text(&text, item.span(), true, &label) {
                        spec.timeouts.insert(query);
                    }
                }
            }
            DeValue::Array(_) => {}
            other => reader.say("visa:S002", format!("{label} is an array of queries, and this is {}", other.type_str()), value.span()),
        }
    }
    if let Some(value) = get(faults, "error_on") {
        let label = format!("{what}: `faults.error_on`");
        if let Some(entries) = reader.table(value, &label)
            && reader.entries(entries.len(), value.span(), &label)
        {
            for (key, entry) in entries.iter() {
                if let (Some(command), Some(error)) = (reader.command(key, false, &label), reader.error(entry, &label)) {
                    spec.error_on.insert(command, error);
                }
            }
        }
    }
    if let Some(value) = get(faults, "error_queue") {
        let label = format!("{what}: `faults.error_queue`");
        match value.get_ref() {
            DeValue::Array(items) if items.len() > QUEUE_SIZE => {
                reader.say("visa:S010", format!("{label} holds {} errors, more than {QUEUE_SIZE}, the size of the queue", items.len()), value.span())
            }
            DeValue::Array(items) => {
                for item in items.iter() {
                    if let Some(error) = reader.error(item, &label) {
                        spec.error_queue.push(error);
                    }
                }
            }
            other => reader.say("visa:S002", format!("{label} is an array of errors, and this is {}", other.type_str()), value.span()),
        }
    }
}

/// Each `source` names a quantity of the file, `SIM::PSU0.voltage`; each chain of sources ends, followed once, with no
/// recursion, and each quantity of the chain takes its end as its root.
fn resolve_sources(reader: &mut Reader<'_>, bench: &mut Bench, written: &[Written]) {
    let mut by_name: HashMap<(&str, &str), usize> = HashMap::new();
    for (id, quantity) in written.iter().enumerate() {
        if let Some(instrument) = bench.instruments.get(quantity.instrument) {
            by_name.insert((instrument.resource.as_str(), quantity.name.as_str()), id);
        }
    }
    // The quantity each one names, by its number.
    let mut next: Vec<Option<usize>> = vec![None; written.len()];
    for (id, quantity) in written.iter().enumerate() {
        let Some((source, at)) = &quantity.source else { continue };
        let named = source.rsplit_once('.').and_then(|(resource, name)| by_name.get(&(resource, name)).copied());
        match named {
            Some(target) => {
                if let Some(slot) = next.get_mut(id) {
                    *slot = Some(target);
                }
            }
            None => reader.say("visa:S007", format!("the source `{}` names no quantity of the file", shown(source)), at.clone()),
        }
    }
    let mut root: Vec<Option<usize>> = vec![None; written.len()];
    for start in 0..written.len() {
        let mut path = Vec::new();
        let mut on_path = HashSet::new();
        let mut current = start;
        let end = loop {
            if let Some(known) = root.get(current).copied().flatten() {
                break Some(known);
            }
            if !on_path.insert(current) {
                break None;
            }
            path.push(current);
            match next.get(current).copied().flatten() {
                Some(following) => current = following,
                None => break Some(current),
            }
        };
        match end {
            Some(end) => {
                for id in path {
                    if let Some(slot) = root.get_mut(id) {
                        *slot = Some(end);
                    }
                }
            }
            None => {
                if let Some((_, at)) = written.get(start).and_then(|quantity| quantity.source.as_ref()) {
                    reader.say("visa:S007", "this chain of sources loops".to_owned(), at.clone());
                }
                for id in path {
                    if let Some(slot) = root.get_mut(id) {
                        *slot = Some(id);
                    }
                }
            }
        }
    }
    for (spec, end) in bench.quantities.iter_mut().zip(root) {
        if let Some(end) = end {
            spec.root = end;
        }
    }
}
