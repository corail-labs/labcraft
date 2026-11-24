//! A wire being drawn (notation 2.4, 2.5, 2.6): from a terminal, by a drag or by W, to a terminal that would take it.
//! Nothing is written meanwhile. The terminals that would take it get a ring; near one, the wire snaps to it on a square
//! route, the one of a drag (decision of the owner, 2026-11-17), its route coming with the drop; over one that would
//! refuse it, the wire is dashed in `mark.invalid` and the reason stands where it would be dropped. The drop is one
//! `wire.connect`, which judges the wire again (architecture 12.6).

use egui::{Pos2, Vec2};
use labcraft_diagram::{Drawing, Holds, Port, Refusal, Verdict, occupied, wiring};

/// A terminal within this many units of the pointer takes a wire dropped there *(proposed)*, or 10 points on the screen
/// when that is more.
pub(crate) const SNAP: f32 = 12.0;
/// A wire asked for keeps showing this long at most, waiting for the new drawing.
pub(crate) const ASKED_SECONDS: f64 = 3.0;
/// How long an old wire takes to retract when a new one replaces it (notation 2.4) *(proposed)*.
pub(crate) const RETRACT_SECONDS: f64 = 0.2;

/// How the wiring goes on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum State {
    /// The primary button draws it.
    Dragging,
    /// From the keyboard (W): Tab goes through the terminals that would take it, Enter wires it.
    Keys,
    /// Asked of the host: it shows until the new drawing comes, or the host says it failed.
    Asked,
}

/// What Ctrl and a drag take from a wired terminal (notation 2.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Taken {
    /// A new wire.
    Nothing,
    /// The end of the wire of this input, by its index in [`Drawing::ports`]: it goes to another input.
    Input(usize),
    /// Every wire of this output's value, by the inputs they reach: they come from another output.
    Readers(Vec<usize>),
}

pub(crate) struct Wiring {
    /// The terminal it starts from, by its index in [`Drawing::ports`]; for Ctrl and a drag on an input, the output of
    /// its wire; on an output, that output.
    pub from: usize,
    pub taken: Taken,
    /// What each terminal would do with it.
    pub verdicts: Vec<Option<Verdict>>,
    /// The pointer, in units.
    pub at: Pos2,
    /// The terminal it would end at: the one it snaps to, or the one that refuses it.
    pub over: Option<usize>,
    pub state: State,
    /// When the canvas first saw it asked for, by the time of egui.
    pub asked_at: Option<f64>,
    /// A branch drawn from a wire (notation 2.4): it starts at its junction on the wire.
    pub branch: Option<Branch>,
}

/// Where a branch starts: its junction, in hundredths, on the grid of 8 of the frame, on a segment of the wire, and
/// whether that segment is vertical; the body and the origin of its frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Branch {
    pub at: (i64, i64),
    pub vertical: bool,
    pub body: usize,
    pub origin: (i64, i64),
}

/// Points in hundredths, or in units.
type Path = Vec<(i64, i64)>;

/// How far a branch from a vertical goes across before it turns to its reader, in hundredths *(proposed)*.
const ACROSS: i64 = 1_600;

/// What a drop would do, the modifiers and the settings counted (notation 2.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Drop {
    /// `through`: how each loop that the wire leaves takes the value out, `carry` or `collect`, from the inside out.
    Wire {
        replace: bool,
        accept_mismatch: bool,
        through: Vec<&'static str>,
    },
    Refused(Refusal),
}

/// The settings and the modifiers that a drop reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Rules {
    /// "Replace only with Alt" is on.
    pub replace_with_alt: bool,
    /// "Accept it" is on: a wire of the wrong type is written, and `check` gives `LC3001`.
    pub accept_mismatch: bool,
    /// Alt is down.
    pub alt: bool,
}

impl Wiring {
    /// A wire from the port `from`; none from a port that no command wires yet.
    pub fn new(drawing: &Drawing, from: usize, at: Pos2, state: State) -> Option<Wiring> {
        drawing.ports.get(from).filter(|port| !port.name.is_empty())?;
        Some(Wiring { from, taken: Taken::Nothing, verdicts: wiring(drawing, from), at, over: None, state, asked_at: None, branch: None })
    }

    /// Ctrl and a drag on a wired terminal: the end of its wire (an input), or every wire of its value (an output).
    pub fn taking(drawing: &Drawing, from: usize, at: Pos2) -> Option<Wiring> {
        let port = drawing.ports.get(from)?;
        if port.input {
            // The output of its wire: the one of its value that the input sees.
            if port.holds != Holds::Wire || port.name.is_empty() {
                return None;
            }
            let source = source_of(drawing, port)?;
            let verdicts = wiring(drawing, source);
            return Some(Wiring {
                from: source,
                taken: Taken::Input(from),
                verdicts,
                at,
                over: None,
                state: State::Dragging,
                asked_at: None,
                branch: None,
            });
        }
        let readers: Vec<usize> = drawing
            .ports
            .iter()
            .enumerate()
            .filter(|(_, reader)| reader.input && !reader.name.is_empty() && reader.holds == Holds::Wire && reader.value == port.value)
            .filter(|(_, reader)| source_of(drawing, reader) == Some(from))
            .map(|(at, _)| at)
            .collect();
        if readers.is_empty() {
            return None;
        }
        // Another output takes them when each of them would take it.
        let mut verdicts: Vec<Option<Verdict>> = vec![None; drawing.ports.len()];
        for (at, candidate) in drawing.ports.iter().enumerate() {
            if candidate.input || at == from || candidate.name.is_empty() {
                continue;
            }
            if let Some(slot) = verdicts.get_mut(at) {
                *slot = Some(Verdict::Takes { replaces: true, exits: Vec::new() });
            }
        }
        for &reader in &readers {
            for (at, verdict) in wiring(drawing, reader).into_iter().enumerate() {
                // A reader that the new output would reach only out of a loop: Ctrl and a drag takes no value out.
                let verdict = match verdict {
                    Some(Verdict::Takes { exits, .. }) if !exits.is_empty() => Some(out_by_ctrl()),
                    other => other,
                };
                if let (Some(Verdict::Refused { .. }), Some(slot)) = (&verdict, verdicts.get_mut(at))
                    && matches!(slot, Some(Verdict::Takes { .. }))
                {
                    *slot = verdict;
                }
            }
        }
        Some(Wiring { from, taken: Taken::Readers(readers), verdicts, at, over: None, state: State::Dragging, asked_at: None, branch: None })
    }

    /// A drag on the wire of `group` that starts a branch (with the modifier, or plain on `branch`, notation 2.6),
    /// pressed at `at` within `reach` units of it (notation 2.5): a branch of its value, from the nearest point of the
    /// grid of 8 on the wire. None for a value that no command wires from.
    pub fn branch(drawing: &Drawing, group: usize, at: Pos2, reach: f32) -> Option<Wiring> {
        let wire = drawing.groups.get(group)?.wire.as_ref()?;
        let source = drawing.ports.iter().position(|port| !port.input && port.group == Some(group) && !port.name.is_empty())?;
        let origin = drawing.bodies.get(wire.body)?.origin;
        let point = ((at.x * 100.0).round() as i64, (at.y * 100.0).round() as i64);
        let reach = (reach * 100.0).round() as i64;
        let snap = |value: i64, start: i64, low: i64, high: i64| {
            let snapped = crate::rerouting::snap(value, start);
            if snapped < low {
                snapped + crate::rerouting::GRID
            } else if snapped > high {
                snapped - crate::rerouting::GRID
            } else {
                snapped
            }
        };
        let mut best: Option<(i64, Branch)> = None;
        for (_, path) in &wire.readers {
            for pair in path.windows(2) {
                let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) else { continue };
                let vertical = a.0 == b.0;
                let (low, high) = if vertical { (a.1.min(b.1), a.1.max(b.1)) } else { (a.0.min(b.0), a.0.max(b.0)) };
                let (along, across, line, start) = if vertical { (point.1, point.0, a.0, origin.1) } else { (point.0, point.1, a.1, origin.0) };
                if along < low - reach || along > high + reach {
                    continue;
                }
                let distance = (across - line).abs();
                if distance > reach {
                    continue;
                }
                let on = snap(along.clamp(low, high), start, low, high);
                if on < low || on > high {
                    continue;
                }
                let at = if vertical { (line, on) } else { (on, line) };
                if best.is_none_or(|(nearest, _)| distance < nearest) {
                    best = Some((distance, Branch { at, vertical, body: wire.body, origin }));
                }
            }
        }
        let (_, branch) = best?;
        let mut wiring = Wiring::new(drawing, source, unit(branch.at), State::Dragging)?;
        wiring.branch = Some(branch);
        Some(wiring)
    }

    /// The route that a branch takes to `input`, in hundredths, and its points in units of its frame: down (or across,
    /// from a vertical) from its junction, then to the reader, as the drawing routes a wire through them (notation 2.2).
    /// None but for a branch to an input of its own body; the others take their junction automatically.
    fn branch_route(&self, input: &Port) -> Option<(Path, Path)> {
        let branch = self.branch?;
        if input.body != branch.body || !input.input || input.name.starts_with("output ") {
            return None;
        }
        // A value of a variadic parameter has no name that `wire.route` takes yet (spec 11): its junction is automatic.
        let numbered = |name: &str| name.contains('.') || name.starts_with(|letter: char| letter.is_ascii_digit());
        if input.name.split_once('.').is_some_and(|(_, name)| numbered(name)) {
            return None;
        }
        let (j, to) = (branch.at, (input.x, input.y));
        // A junction that the file cannot write where it is, off the grid of units, is left to the drawing.
        if (j.0 - branch.origin.0) % 100 != 0 || (j.1 - branch.origin.1) % 100 != 0 {
            return None;
        }
        let units = |(x, y): (i64, i64)| (crate::rerouting::unit(x - branch.origin.0), crate::rerouting::unit(y - branch.origin.1));
        if branch.vertical {
            let x = j.0 + if to.0 >= j.0 { ACROSS } else { -ACROSS };
            Some((labcraft_diagram::simplify(vec![j, (x, j.1), (x, to.1), to]), vec![units(j), units((x, to.1))]))
        } else {
            Some((labcraft_diagram::simplify(vec![j, (j.0, to.1), to]), vec![units(j)]))
        }
    }

    /// The paths of the wire as it is drawn now, in units: square from its start to the pointer, or to the terminal it
    /// snaps to; a branch snapped to an input of its body, by the route its points will give it.
    pub fn paths(&self, drawing: &Drawing) -> Vec<Vec<Pos2>> {
        if let Some(input) = self.over.and_then(|over| drawing.ports.get(over))
            && let Some((path, _)) = self.branch_route(input)
        {
            return vec![path.into_iter().map(unit).collect()];
        }
        self.ends(drawing).into_iter().map(|(from, to)| route(from, to)).collect()
    }

    /// The terminal under the pointer that is an end of this wire, within `reach` units, shown.
    pub fn find_over(&mut self, drawing: &Drawing, reach: f32, visible: &dyn Fn(Option<usize>) -> bool) {
        let at = self.at;
        self.over = drawing
            .ports
            .iter()
            .enumerate()
            .filter(|(index, port)| self.verdicts.get(*index).is_some_and(Option::is_some) && visible(port.within))
            .map(|(index, port)| (index, centre(port).distance(at)))
            .filter(|(_, distance)| *distance <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index);
    }

    /// What a drop on `port` would do.
    pub fn drop_on(&self, drawing: &Drawing, port: usize, rules: Rules) -> Option<Drop> {
        let verdict = self.verdicts.get(port)?.as_ref()?;
        let target = drawing.ports.get(port)?;
        // The input that the wire reaches, whichever end the gesture holds; for Ctrl and a drag on an output, its readers,
        // which it replaces.
        let input = if target.input { target } else { drawing.ports.get(self.from)? };
        let source = if target.input { drawing.ports.get(self.from)? } else { target };
        let holds = matches!(input.holds, Holds::Wire | Holds::Constant(_)) && !input.order;
        let refused_without_alt = |replace: bool| replace && rules.replace_with_alt && !rules.alt && !matches!(self.taken, Taken::Readers(_));
        Some(match verdict {
            Verdict::Takes { replaces, .. } if refused_without_alt(*replaces) => Drop::Refused(occupied(input)),
            Verdict::Takes { replaces, exits } if exits.is_empty() => Drop::Wire { replace: *replaces, accept_mismatch: false, through: Vec::new() },
            // Ctrl and a drag moves the end of a wire: a new one takes a value out of a loop.
            Verdict::Takes { .. } if self.taken != Taken::Nothing => Drop::Refused(match out_by_ctrl() {
                Verdict::Refused { refusal, .. } => refusal,
                Verdict::Takes { .. } => return None,
            }),
            // Out of loops, the type is that of the passages chosen: a value in an array per `collect` (spec 6.2).
            Verdict::Takes { replaces, exits } => {
                let through = passages(drawing, source, input, exits, rules.alt);
                let collects = through.iter().filter(|passage| **passage == "collect").count();
                match labcraft_diagram::passage_refusal(drawing, source, input, collects) {
                    None => Drop::Wire { replace: *replaces, accept_mismatch: false, through },
                    Some(_) if rules.accept_mismatch => Drop::Wire { replace: *replaces, accept_mismatch: true, through },
                    Some(refusal) => Drop::Refused(refusal),
                }
            }
            Verdict::Refused { wrong_type: true, .. } if rules.accept_mismatch => {
                if refused_without_alt(holds) {
                    Drop::Refused(occupied(input))
                } else {
                    Drop::Wire { replace: holds, accept_mismatch: true, through: Vec::new() }
                }
            }
            Verdict::Refused { refusal, .. } => Drop::Refused(refusal.clone()),
        })
    }

    /// The terminals that would take it now, in the order of the text: Tab goes through them.
    pub fn takers(&self, drawing: &Drawing, rules: Rules, visible: &dyn Fn(Option<usize>) -> bool) -> Vec<usize> {
        (0..drawing.ports.len())
            .filter(|&at| drawing.ports.get(at).is_some_and(|port| visible(port.within)))
            .filter(|&at| matches!(self.drop_on(drawing, at, rules), Some(Drop::Wire { .. })))
            .collect()
    }

    /// The source and the end of the wire as it is drawn now, in units: from the start to the pointer, or to the
    /// terminal it snaps to.
    pub fn ends(&self, drawing: &Drawing) -> Vec<(Pos2, Pos2)> {
        let Some(start) = drawing.ports.get(self.from) else { return Vec::new() };
        let end = self.over.and_then(|over| drawing.ports.get(over)).map_or(self.at, centre);
        if let Some(branch) = self.branch {
            return vec![(unit(branch.at), end)];
        }
        match &self.taken {
            // Every reader, from the output under the pointer.
            Taken::Readers(readers) => readers.iter().filter_map(|&reader| drawing.ports.get(reader)).map(|reader| (end, centre(reader))).collect(),
            _ if start.input => vec![(end, centre(start))],
            _ => vec![(centre(start), end)],
        }
    }

    /// The intent of a drop on `port`: the names of the command.
    pub fn intent(&self, drawing: &Drawing, port: usize, replace: bool, accept_mismatch: bool, through: Vec<&'static str>) -> Option<crate::Intent> {
        let start = drawing.ports.get(self.from)?;
        let end = drawing.ports.get(port)?;
        let (source, target) = if start.input { (end, start) } else { (start, end) };
        Some(match &self.taken {
            Taken::Nothing => {
                // A branch names its wire by its value and the input it reaches, as `wire.route` names it.
                let route = self.branch_route(target).map(|(_, points)| (format!("{} -> {}", source.value, target.name), points));
                let arm = (!target.arm.is_empty()).then(|| target.arm.clone());
                crate::Intent::Wire { from: start_name(drawing, source), to: target.name.clone(), arm, replace, accept_mismatch, route, through }
            }
            Taken::Input(input) => {
                let old = drawing.ports.get(*input)?;
                if *input == port {
                    return None;
                }
                let arm = |port: &Port| (!port.arm.is_empty()).then(|| port.arm.clone());
                crate::Intent::Rewire {
                    disconnect: vec![(format!("{} -> {}", old.value, wire_end(old)), arm(old))],
                    connect: vec![(source.name.clone(), target.name.clone(), replace, accept_mismatch, arm(target))],
                }
            }
            Taken::Readers(readers) => crate::Intent::Rewire {
                disconnect: Vec::new(),
                connect: readers
                    .iter()
                    .filter_map(|&reader| drawing.ports.get(reader))
                    .map(|reader| {
                        (end.name.clone(), reader.name.clone(), true, accept_mismatch, (!reader.arm.is_empty()).then(|| reader.arm.clone()))
                    })
                    .collect(),
            },
        })
    }
}

/// Ctrl and a drag out of a loop: refused, a new wire takes the value out (notation 2.6).
fn out_by_ctrl() -> Verdict {
    let message = "Ctrl and a drag does not take a value out of a loop: draw a new wire from it".to_owned();
    Verdict::Refused { refusal: Refusal { code: "wire.out_of_scope", message }, wrong_type: false }
}

/// How a wire names where it starts: the name of its value, or `node.output` when that name is also a value of another
/// body (9.2), which the command would find ambiguous out of its structure.
fn start_name(drawing: &Drawing, source: &Port) -> String {
    let elsewhere = drawing.ports.iter().any(|port| !port.input && !port.notch && port.name == source.name && port.body != source.body);
    let node = source.placed.and_then(|placed| drawing.placed.get(placed)).filter(|node| !node.name.is_empty() && node.body == source.body);
    match node {
        Some(node) if elsewhere && !source.input && !source.frame && !source.caption.is_empty() && source.caption != "_" => {
            format!("{}.{}", node.name, source.caption)
        }
        _ => source.name.clone(),
    }
}

/// How each loop that a wire from `source` to `input` leaves takes the value out (notation 2.5, "Out of a loop";
/// decisions of the owner, 2026-11-18): the type of the input chooses, a `last` for the value's type, a `collect` for
/// that type in one array per loop; otherwise each loop its default, a `collect` for a `for` and a `last` for a `loop`.
/// Alt takes the other one. An `error` that a loop gives as a value of its type is a carry instead, from `none`, so that
/// a `loop` stops on it (spec 6.3, Q-7), and Alt takes a `last`; a `for parallel` has no carry (spec 6.2).
pub(crate) fn passages(drawing: &Drawing, source: &Port, input: &Port, exits: &[usize], alt: bool) -> Vec<&'static str> {
    if exits.is_empty() {
        return Vec::new();
    }
    let text = |ty: Option<usize>| ty.and_then(|ty| drawing.types.get(ty)).cloned().unwrap_or_default();
    let (found, wanted) = (text(source.ty), text(input.ty));
    let count = exits.len();
    let body = |at: &usize| drawing.bodies.get(*at);
    let chosen: Vec<&'static str> = if !found.is_empty() && wanted == found {
        vec!["last"; count]
    } else if !found.is_empty() && wanted == format!("{}{found}{}", "[".repeat(count), "]".repeat(count)) {
        vec!["collect"; count]
    } else {
        exits.iter().map(|at| if body(at).is_some_and(|found| found.each) { "collect" } else { "last" }).collect()
    };
    let error = found == "error";
    chosen
        .into_iter()
        .zip(exits)
        .map(|(passage, at)| {
            let carried = error && passage == "last" && !body(at).is_some_and(|found| found.parallel);
            match (alt, passage) {
                (false, _) if carried => "carry",
                (false, _) => passage,
                (true, _) if carried => "last",
                (true, "last") => "collect",
                (true, _) => "last",
            }
        })
        .collect()
}

/// What a wire out of loops shows beside the pointer (notation 2.5): the mark of what each loop makes, `•]` for a `last`,
/// `[•]` for a `collect` and `↻` for a carry, from the inside out.
pub(crate) fn passage_marks(through: &[&str]) -> String {
    through
        .iter()
        .map(|passage| match *passage {
            "collect" => "[•] collect",
            "carry" => "↻ carry",
            _ => "•] last value",
        })
        .collect::<Vec<_>>()
        .join("  ·  ")
}

/// The centre of a port, in units.
pub(crate) fn centre(port: &Port) -> Pos2 {
    Pos2::new(port.x as f32, port.y as f32) / 100.0
}

/// The end of a wire into `input` as `wire.disconnect` names it (commands.md 9.2): `node.input`, or an output of the
/// function by its name.
fn wire_end(input: &Port) -> String {
    input.name.strip_prefix("output ").map_or_else(|| input.name.clone(), str::to_owned)
}

/// The output whose value the input reads, the one it sees: in its body, or in a body around it.
fn source_of(drawing: &Drawing, input: &Port) -> Option<usize> {
    let mut body = Some(input.body);
    let mut seen = 0;
    while let Some(at) = body {
        if let Some(found) = drawing.ports.iter().position(|port| !port.input && port.body == at && port.name == input.value && !port.name.is_empty())
        {
            return Some(found);
        }
        body = drawing.bodies.get(at).and_then(|found| found.owner).and_then(|owner| drawing.placed.get(owner)).map(|node| node.body);
        seen += 1;
        if seen > drawing.bodies.len() {
            return None;
        }
    }
    None
}

/// An old wire that a new one replaces, retracting toward its source (notation 2.4): its route, in units, its colour
/// and its width; whether the new drawing came, and when the first frame of it started the retract.
pub(crate) struct Retract {
    pub points: Vec<Pos2>,
    pub token: &'static str,
    pub width: f32,
    pub drawn: bool,
    pub since: Option<f64>,
}

impl Retract {
    /// The old wire into `input`: the route of its reader that ends nearest the input.
    pub fn of(drawing: &Drawing, input: usize) -> Option<Retract> {
        let port = drawing.ports.get(input)?;
        let group = drawing.groups.get(port.group?)?;
        let wire = group.wire.as_ref()?;
        let at = centre(port);
        let (_, points) = wire.readers.iter().min_by(|a, b| {
            let end = |reader: &(Option<String>, Vec<(i64, i64)>)| reader.1.last().map_or(f32::MAX, |&point| unit(point).distance(at));
            end(a).total_cmp(&end(b))
        })?;
        let points = points.iter().map(|&(x, y)| Pos2::new(x as f32, y as f32) / 100.0).collect();
        Some(Retract { points, token: port.style.token, width: port.style.width as f32 / 100.0, drawn: false, since: None })
    }

    /// The part of its route still drawn at `t` from 0 to 1: from its start, shorter as `t` grows.
    pub fn part(&self, t: f32) -> Vec<Pos2> {
        let lengths: Vec<f32> = self.points.windows(2).map(|pair| pair.first().zip(pair.get(1)).map_or(0.0, |(a, b)| a.distance(*b))).collect();
        let total: f32 = lengths.iter().sum();
        let mut left = total * (1.0 - t.clamp(0.0, 1.0));
        let mut out = Vec::new();
        for (pair, length) in self.points.windows(2).zip(lengths) {
            let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) else { continue };
            if out.is_empty() {
                out.push(a);
            }
            if left >= length {
                out.push(b);
                left -= length;
            } else {
                if left > 0.0 {
                    out.push(a + (b - a) * (left / length));
                }
                break;
            }
        }
        out
    }
}

/// A point of a route, in units, from hundredths.
pub(crate) fn unit((x, y): (i64, i64)) -> Pos2 {
    Pos2::new(x as f32, y as f32) / 100.0
}

/// The square route from `from` to `to`, in units, as the preview of a drag routes a wire (notation 2.5).
pub(crate) fn route(from: Pos2, to: Pos2) -> Vec<Pos2> {
    let hundredths = |at: Pos2| ((at.x * 100.0).round() as i64, (at.y * 100.0).round() as i64);
    labcraft_diagram::square(hundredths(from), hundredths(to)).into_iter().map(unit).collect()
}

/// Where the reason of a refusal stands: just past the point of the drop.
pub(crate) fn reason_at(at: Pos2) -> Pos2 {
    at + Vec2::new(12.0, 12.0)
}

#[cfg(test)]
mod tests {
    use super::{Retract, route};
    use egui::Pos2;

    #[test]
    fn a_wire_retracts_toward_its_source() {
        let retract = Retract {
            points: vec![Pos2::new(0.0, 0.0), Pos2::new(10.0, 0.0), Pos2::new(10.0, 10.0)],
            token: "",
            width: 1.0,
            drawn: false,
            since: None,
        };
        assert_eq!(retract.part(0.0), vec![Pos2::new(0.0, 0.0), Pos2::new(10.0, 0.0), Pos2::new(10.0, 10.0)]);
        assert_eq!(retract.part(0.5), vec![Pos2::new(0.0, 0.0), Pos2::new(10.0, 0.0)]);
        assert_eq!(retract.part(0.75), vec![Pos2::new(0.0, 0.0), Pos2::new(5.0, 0.0)]);
    }

    #[test]
    fn a_wire_being_drawn_goes_square_through_the_middle() {
        assert_eq!(
            route(Pos2::new(0.0, 0.0), Pos2::new(100.0, 40.0)),
            vec![Pos2::new(0.0, 0.0), Pos2::new(50.0, 0.0), Pos2::new(50.0, 40.0), Pos2::new(100.0, 40.0)]
        );
    }
}
