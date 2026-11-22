//! What the window shows of the agents attached (ide.md 8), besides the questions of [`crate::agents`]: the pill of
//! [`crate::pill`], and for each step an agent makes, the nodes and wires it touched highlighted for a moment, a dot on
//! the tab of a function it edited while that function was not shown, and a short notice of what it did, with Show and
//! Undo. The switch of AI off takes all of it away.

use std::collections::{HashMap, HashSet};

use egui::RichText;
use labcraft_editor::Touched;
use labcraft_engine::view::Published;

use crate::agents::{Asked, Connected};
use crate::host::LastStep;
use crate::pill::Pill;

/// How long the notice of an edit of an agent stays *(proposed)*, while the pointer is not over it.
pub const NOTICE_SECONDS: f64 = 8.0;

/// How long after a step its drawing may come and still be highlighted *(proposed)*: a later change is another's.
const DRAWING_WAIT: f64 = 5.0;

/// The notice of the last step of an agent.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub step: u64,
    pub author: String,
    pub label: String,
    /// The function it edited, the first one when it edited several; none for a file that holds no function.
    pub function: Option<String>,
    /// Since when it shows, on the clock of egui: the pointer over it starts it again.
    pub since: f64,
    /// What it touched in that function, once its drawing came: Show highlights it again.
    pub touched: Option<Touched>,
}

/// What a click on the notice asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Clicked {
    Show(String),
    /// Undo the step, by its number.
    Undo(u64),
}

/// The state of the agents in the window.
#[derive(Debug, Default)]
pub struct AgentView {
    /// The agents attached, as the app gives them.
    pub connected: Vec<Connected>,
    /// What the user asked of them, for the app.
    pub asked: Vec<Asked>,
    pub pill: Pill,
    pub notice: Option<Notice>,
    /// The functions an agent edited while they were not shown: a dot on their tab, until they are.
    pub dots: HashSet<String>,
    /// The last step seen, whoever made it.
    step_seen: Option<u64>,
    /// The functions whose next drawing shows a step of an agent, since when: what changed is highlighted.
    pending: HashMap<String, f64>,
    /// A function whose touched nodes are to be highlighted again, once it is shown (Show).
    pub again: Option<(String, Touched)>,
}

impl AgentView {
    /// Nothing of AI: the switch off. The steps made meanwhile, newest first, are seen: none gets a notice later.
    pub fn clear(&mut self, steps: &[LastStep]) {
        self.notice = None;
        self.dots.clear();
        self.pending.clear();
        self.again = None;
        self.pill.close();
        self.see(steps);
    }

    /// Another session, whose steps count from 1 again.
    pub fn reset(&mut self) {
        *self = AgentView::default();
    }

    /// The newest of `steps` is seen; the last one seen before it, if any.
    fn see(&mut self, steps: &[LastStep]) -> Option<u64> {
        let seen = self.step_seen;
        if let Some(newest) = steps.first() {
            // Numbers are never given twice: an undo shows an older step, which is not new.
            self.step_seen = Some(seen.map_or(newest.number, |seen| seen.max(newest.number)));
        }
        seen
    }

    /// The last steps of the history, newest first, as the session published them at `now`. Each new step of an agent,
    /// oldest first, puts a dot on the tabs of the functions it edited that no window shows, and has the next drawing
    /// of those that one shows highlighted; the newest gets the notice. A new step of the user, or from outside, after
    /// it is not highlighted as the agent's.
    pub fn follow(&mut self, steps: &[LastStep], view: &Published, now: f64) {
        self.pending.retain(|_, since| now - *since < DRAWING_WAIT);
        let shown = |function: &str| view.windows().any(|window| window.function.as_deref() == Some(function));
        self.dots.retain(|function| view.opened.contains(function) && !shown(function));
        let seen = self.see(steps);
        for step in steps.iter().rev().filter(|step| seen.is_none_or(|seen| step.number > seen)) {
            let functions: Vec<&str> =
                view.functions.iter().filter(|listed| step.files.contains(&listed.file)).map(|listed| listed.function.as_str()).collect();
            if !step.agent {
                for function in functions {
                    self.pending.remove(function);
                }
                continue;
            }
            for &function in &functions {
                if shown(function) {
                    self.pending.insert(function.to_owned(), now);
                } else if view.opened.iter().any(|opened| opened == function) {
                    self.dots.insert(function.to_owned());
                }
            }
            self.notice = Some(Notice {
                step: step.number,
                author: step.author.clone(),
                label: step.label.clone(),
                // The function shown, where it is highlighted, else the first one.
                function: functions.iter().find(|function| shown(function)).or(functions.first()).map(|function| (*function).to_owned()),
                since: now,
                touched: None,
            });
        }
    }

    /// Whether the next drawing of `function` that changes something shows a step of an agent.
    pub fn is_pending(&self, function: &str) -> bool {
        self.pending.contains_key(function)
    }

    /// The drawing of `function` that shows the step came, and what it touched: no longer waited for, and kept for
    /// Show.
    pub fn drawn(&mut self, function: &str, touched: &Touched) {
        self.pending.remove(function);
        if let Some(notice) = &mut self.notice
            && notice.function.as_deref() == Some(function)
        {
            notice.touched = Some(touched.clone());
        }
    }

    /// The notice, in the bottom right corner of the window, while it lasts: what was clicked. `undoable`: its step is
    /// still the last one, which undo would take back.
    pub fn show_notice(&mut self, ctx: &egui::Context, now: f64, undoable: bool, bottom_right: egui::Pos2) -> Option<Clicked> {
        let notice = self.notice.as_ref()?;
        if now - notice.since >= NOTICE_SECONDS {
            self.notice = None;
            return None;
        }
        let mut clicked = None;
        let mut dismissed = false;
        let area = egui::Area::new(egui::Id::unique("ide-agent-notice"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .fixed_pos(bottom_right - egui::vec2(12.0, 12.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(420.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&notice.author).strong());
                        let what = match &notice.function {
                            Some(function) => format!("{} in {function}", notice.label),
                            None => notice.label.clone(),
                        };
                        ui.add(egui::Label::new(what).truncate());
                        if let Some(function) = &notice.function
                            && ui.button("Show").on_hover_text("Show the function, and what the agent changed").clicked()
                        {
                            clicked = Some(Clicked::Show(function.clone()));
                        }
                        let hover = if undoable { "Undo this step (Ctrl+Z)" } else { "A step came after it: undo that one first" };
                        if ui.add_enabled(undoable, egui::Button::new("Undo")).on_hover_text(hover).on_disabled_hover_text(hover).clicked() {
                            clicked = Some(Clicked::Undo(notice.step));
                        }
                        dismissed = kit_shell::icon::button(ui, kit_shell::icon::Icon::Close, true, "Dismiss").clicked();
                    });
                });
            });
        // The pointer that moves over it repaints; else it goes when its time is up.
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((notice.since + NOTICE_SECONDS - now).max(0.0)));
        if area.response.hovered()
            && let Some(notice) = &mut self.notice
        {
            notice.since = now;
        }
        if dismissed || matches!(clicked, Some(Clicked::Undo(_))) {
            self.notice = None;
        }
        clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labcraft_engine::view::{Listed, Shown};

    fn view(shown: &str, opened: &[&str]) -> Published {
        let listed = |function: &str| Listed {
            function: function.to_owned(),
            file: format!("{function}.lcf"),
            drawable: true,
            errors: 0,
            warnings: 0,
            needs: Vec::new(),
        };
        Published {
            functions: vec![listed("a"), listed("b"), listed("c")],
            opened: opened.iter().map(|function| (*function).to_owned()).collect(),
            main: Shown { function: Some(shown.to_owned()), ..Shown::default() },
            ..Published::default()
        }
    }

    fn step(number: u64, author: &str, files: &[&str]) -> LastStep {
        LastStep {
            number,
            author: author.to_owned(),
            agent: author != "user",
            label: "Add Node".to_owned(),
            files: files.iter().map(|file| (*file).to_owned()).collect(),
        }
    }

    #[test]
    fn a_step_of_an_agent_gets_a_notice_a_highlight_where_shown_and_a_dot_where_not() {
        let mut agents = AgentView::default();
        let view = view("a", &["a", "b"]);
        agents.follow(&[step(1, "claude", &["a.lcf", "b.lcf", "c.lcf"])], &view, 10.0);
        let notice = agents.notice.clone().unwrap();
        assert_eq!((notice.step, notice.author.as_str(), notice.function.as_deref()), (1, "claude", Some("a")));
        assert!(agents.is_pending("a") && !agents.is_pending("b"), "highlighted where a window shows it");
        assert_eq!(agents.dots, HashSet::from(["b".to_owned()]), "a dot on a tab not shown; none for a function with no tab");
        // The same step again changes nothing; a step of the user gets no notice.
        agents.notice = None;
        agents.follow(&[step(1, "claude", &["a.lcf"])], &view, 11.0);
        agents.follow(&[step(2, "user", &["a.lcf"]), step(1, "claude", &["a.lcf"])], &view, 12.0);
        assert!(agents.notice.is_none());
        // The tab shown loses its dot.
        agents.follow(&[], &self::view("b", &["a", "b"]), 13.0);
        assert!(agents.dots.is_empty());
    }

    #[test]
    fn a_drawing_that_comes_too_late_is_not_highlighted() {
        let mut agents = AgentView::default();
        let view = view("a", &["a"]);
        agents.follow(&[step(1, "claude", &["a.lcf"])], &view, 10.0);
        agents.follow(&[], &view, 10.0 + DRAWING_WAIT);
        assert!(!agents.is_pending("a"));
    }

    #[test]
    fn each_step_of_a_burst_counts_and_a_step_of_the_user_after_one_takes_its_highlight() {
        let mut agents = AgentView::default();
        let view = view("a", &["a", "b"]);
        agents.follow(&[step(1, "claude", &["c.lcf"])], &view, 1.0);
        // Three steps between two frames, newest first: each is followed, the newest has the notice.
        let burst = [step(4, "claude", &["c.lcf"]), step(3, "user", &["a.lcf"]), step(2, "claude", &["a.lcf", "b.lcf"])];
        agents.follow(&burst, &view, 2.0);
        assert_eq!(agents.notice.as_ref().map(|notice| notice.step), Some(4));
        assert!(agents.dots.contains("b"), "the step before still puts its dot");
        assert!(!agents.is_pending("a"), "the user changed `a` after the agent: not the agent's to highlight");
        // An undo shows an older step: not new.
        agents.notice = None;
        agents.follow(&[step(3, "user", &["a.lcf"]), step(2, "claude", &["a.lcf"])], &view, 3.0);
        assert!(agents.notice.is_none());
        // AI off: the steps made meanwhile are seen, and get no notice once it is on again.
        agents.clear(&[step(5, "claude", &["a.lcf"])]);
        agents.follow(&[step(5, "claude", &["a.lcf"])], &view, 4.0);
        assert!(agents.notice.is_none());
    }
}
