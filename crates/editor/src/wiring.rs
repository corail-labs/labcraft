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
}

/// What a drop would do, the modifiers and the settings counted (notation 2.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Drop {
    Wire { replace: bool, accept_mismatch: bool },
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
        Some(Wiring { from, taken: Taken::Nothing, verdicts: wiring(drawing, from), at, over: None, state, asked_at: None })
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
            return Some(Wiring { from: source, taken: Taken::Input(from), verdicts, at, over: None, state: State::Dragging, asked_at: None });
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
                *slot = Some(Verdict::Takes { replaces: true });
            }
        }
        for &reader in &readers {
            for (at, verdict) in wiring(drawing, reader).into_iter().enumerate() {
                if let (Some(Verdict::Refused { .. }), Some(slot)) = (&verdict, verdicts.get_mut(at))
                    && matches!(slot, Some(Verdict::Takes { .. }))
                {
                    *slot = verdict;
                }
            }
        }
        Some(Wiring { from, taken: Taken::Readers(readers), verdicts, at, over: None, state: State::Dragging, asked_at: None })
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
        let holds = matches!(input.holds, Holds::Wire | Holds::Constant(_)) && !input.order;
        let refused_without_alt = |replace: bool| replace && rules.replace_with_alt && !rules.alt && !matches!(self.taken, Taken::Readers(_));
        Some(match verdict {
            Verdict::Takes { replaces } if refused_without_alt(*replaces) => Drop::Refused(occupied(input)),
            Verdict::Takes { replaces } => Drop::Wire { replace: *replaces, accept_mismatch: false },
            Verdict::Refused { wrong_type: true, .. } if rules.accept_mismatch => {
                if refused_without_alt(holds) {
                    Drop::Refused(occupied(input))
                } else {
                    Drop::Wire { replace: holds, accept_mismatch: true }
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
        match &self.taken {
            // Every reader, from the output under the pointer.
            Taken::Readers(readers) => readers.iter().filter_map(|&reader| drawing.ports.get(reader)).map(|reader| (end, centre(reader))).collect(),
            _ if start.input => vec![(end, centre(start))],
            _ => vec![(centre(start), end)],
        }
    }

    /// The intent of a drop on `port`: the names of the command.
    pub fn intent(&self, drawing: &Drawing, port: usize, replace: bool, accept_mismatch: bool) -> Option<crate::Intent> {
        let start = drawing.ports.get(self.from)?;
        let end = drawing.ports.get(port)?;
        let (source, target) = if start.input { (end, start) } else { (start, end) };
        Some(match &self.taken {
            Taken::Nothing => crate::Intent::Wire { from: source.name.clone(), to: target.name.clone(), replace, accept_mismatch },
            Taken::Input(input) => {
                let old = drawing.ports.get(*input)?;
                if *input == port {
                    return None;
                }
                crate::Intent::Rewire {
                    disconnect: vec![format!("{} -> {}", old.value, wire_end(old))],
                    connect: vec![(source.name.clone(), target.name.clone(), replace, accept_mismatch)],
                }
            }
            Taken::Readers(readers) => crate::Intent::Rewire {
                disconnect: Vec::new(),
                connect: readers
                    .iter()
                    .filter_map(|&reader| drawing.ports.get(reader))
                    .map(|reader| (end.name.clone(), reader.name.clone(), true, accept_mismatch))
                    .collect(),
            },
        })
    }
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
