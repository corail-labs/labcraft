//! What the window asks the user about an agent attached to it (ide.md 8): the first connection of a client, with the
//! three policies, and, under its policy, a command that waits for the user's answer. The app holds the channel and
//! the questions; the window shows them, one at a time, over everything, and gives the answers back. A policy that the
//! user keeps for a client and a project is written in `ai.policies` (commands.md 9.7), which the app reads: it then
//! answers the first connection without asking.

use egui::RichText;
use labcraft_engine::registry::Effect;
use labcraft_engine::settings::{AI_POLICIES, keep_policy};
use serde_json::{Map, Value};

/// What an agent may do without asking (ide.md 8), by the effects of a command (commands.md 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// A command with no effect runs; one with `files` or `run` waits for the user's answer.
    Ask,
    /// `files` runs; `run` asks.
    Edits,
    /// `files` and `run` run, on the simulator.
    All,
}

impl Policy {
    pub const ALL: [Policy; 3] = [Policy::Ask, Policy::Edits, Policy::All];

    /// Its name in `ai.policies`.
    pub fn as_str(self) -> &'static str {
        match self {
            Policy::Ask => "ask",
            Policy::Edits => "edits",
            Policy::All => "all",
        }
    }

    pub fn named(name: &str) -> Option<Policy> {
        Policy::ALL.into_iter().find(|policy| policy.as_str() == name)
    }

    /// What the dialog calls it.
    pub fn label(self) -> &'static str {
        match self {
            Policy::Ask => "Ask",
            Policy::Edits => "Edits",
            Policy::All => "All",
        }
    }

    /// Its line in the dialog.
    pub fn help(self) -> &'static str {
        match self {
            Policy::Ask => "It reads; each edit and each run waits for your answer.",
            Policy::Edits => "It reads and edits the files of the project; each run waits for your answer.",
            Policy::All => "It reads, edits and runs, on the simulator.",
        }
    }
}

/// Whether an effect runs a program: what `Edits` asks for (ide.md 8).
pub fn runs(effect: Effect) -> bool {
    matches!(effect, Effect::Run | Effect::Instruments)
}

/// What a question asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asks {
    /// The first connection of the client: which policy, or refuse.
    Connect,
    /// A command that the policy of the client does not let run alone: its label, and what it does.
    Command { label: String, effects: Vec<Effect> },
}

/// A question of the app about an agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub id: u64,
    /// The name of the client (`clientInfo.name`), as `ai.policies` keys it.
    pub client: String,
    /// The name its steps are signed with: the client's, numbered when two clients of one name are attached.
    pub signed: String,
    /// The canonical folder of the project, as `ai.policies` keys it.
    pub project: String,
    pub asks: Asks,
    /// How long it waits for the user, from when it was asked.
    pub seconds_left: u64,
}

/// An agent attached to the window, as its pill shows it (ide.md 8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connected {
    /// Its connection, by which the user disconnects it or changes its policy.
    pub id: u64,
    /// The name of its client (`clientInfo.name`), as `ai.policies` keys it.
    pub client: String,
    /// The name its steps are signed with.
    pub signed: String,
    /// The canonical folder of the project, as `ai.policies` keys it.
    pub project: String,
    /// Its policy; none while the question of its first connection waits.
    pub policy: Option<Policy>,
}

/// What the user asks of an agent attached, from the pill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    Disconnect(u64),
    /// Another policy, for this connection.
    Policy(u64, Policy),
}

/// The user's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// To [`Asks::Connect`]: the policy, and whether it is kept for that client and that project.
    Policy {
        policy: Policy,
        keep: bool,
    },
    /// To [`Asks::Command`].
    Allow,
    Refuse,
}

/// The questions shown, and the choices being made in the dialog.
#[derive(Debug, Default)]
pub struct Questions {
    asked: Vec<Question>,
    answers: Vec<(u64, Answer)>,
    /// The questions answered that the app may still list until it takes the answer, in the frame they were answered
    /// in: not asked twice.
    answered: Vec<u64>,
    /// The policy chosen in the dialog of the first connection, before `Connect`.
    chosen: Option<Policy>,
    keep: bool,
    /// The question the choices are for.
    shown: Option<u64>,
}

impl Questions {
    /// The questions that wait, as the app holds them.
    pub fn set(&mut self, questions: Vec<Question>) {
        self.answered.retain(|id| questions.iter().any(|question| question.id == *id));
        self.asked = questions.into_iter().filter(|question| !self.answered.contains(&question.id)).collect();
    }

    fn answer(&mut self, id: u64, answer: Answer) {
        self.answered.push(id);
        self.answers.push((id, answer));
        self.asked.retain(|question| question.id != id);
    }

    /// The answers given since the last call, for the app.
    pub fn take_answers(&mut self) -> Vec<(u64, Answer)> {
        std::mem::take(&mut self.answers)
    }

    /// Whether a question waits for the user.
    pub fn any(&self) -> bool {
        !self.asked.is_empty()
    }

    /// Shows the first question that waits, as a modal dialog. A policy kept is what `settings.set` must write:
    /// the new value of `ai.policies`.
    pub fn show(&mut self, ctx: &egui::Context, settings: &Map<String, Value>) -> Option<Value> {
        let question = self.asked.first()?.clone();
        if self.shown != Some(question.id) {
            self.shown = Some(question.id);
            self.chosen = None;
            self.keep = false;
        }
        let mut answer = None;
        egui::Modal::new(egui::Id::unique(("ide-agent-question", question.id))).show(ctx, |ui| {
            ui.set_width(460.0);
            match &question.asks {
                Asks::Connect => self.connect(ui, &question, &mut answer),
                Asks::Command { label, effects } => command(ui, &question, label, effects, &mut answer),
            }
            ui.add_space(2.0);
            ui.label(RichText::new(format!("Refused in {} s without an answer", question.seconds_left)).weak().small());
        });
        // A second that passes changes the text.
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        let answer = answer?;
        self.answer(question.id, answer);
        match answer {
            Answer::Policy { policy, keep: true } => {
                let kept = settings.get(AI_POLICIES).unwrap_or(&Value::Null);
                Some(keep_policy(kept, &question.client, &question.project, policy.as_str()))
            }
            _ => None,
        }
    }

    fn connect(&mut self, ui: &mut egui::Ui, question: &Question, answer: &mut Option<Answer>) {
        ui.label(RichText::new(format!("{} wants to connect", question.signed)).strong().size(16.0));
        ui.label(format!("An AI agent asks to work on {} through this window.", shown_folder(&question.project)));
        ui.add_space(6.0);
        for policy in Policy::ALL {
            let chosen = self.chosen == Some(policy);
            ui.radio_value(&mut self.chosen, Some(policy), RichText::new(policy.label()).strong());
            ui.indent(("ide-agent-policy", policy.as_str()), |ui| {
                let help = RichText::new(policy.help());
                ui.label(if chosen { help } else { help.weak() });
            });
        }
        ui.add_space(4.0);
        ui.checkbox(&mut self.keep, "Remember for this client and this project");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Refuse").clicked() {
                *answer = Some(Answer::Refuse);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let connect = ui.add_enabled(self.chosen.is_some(), egui::Button::new("Connect"));
                if connect.clicked()
                    && let Some(policy) = self.chosen
                {
                    *answer = Some(Answer::Policy { policy, keep: self.keep });
                }
            });
        });
    }
}

fn command(ui: &mut egui::Ui, question: &Question, label: &str, effects: &[Effect], answer: &mut Option<Answer>) {
    ui.label(RichText::new(format!("{} asks: {label}", question.signed)).strong().size(16.0));
    let what = match (effects.contains(&Effect::Files), effects.iter().any(|effect| runs(*effect))) {
        (true, true) => "It changes files of the project, and runs a program.",
        (true, false) => "It changes files of the project.",
        _ => "It runs a program, on the simulator.",
    };
    ui.label(what);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button("Refuse").clicked() {
            *answer = Some(Answer::Refuse);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Allow").clicked() {
                *answer = Some(Answer::Allow);
            }
        });
    });
}

/// A canonical folder as the user knows it: without the `\\?\` that Windows puts before a canonical path.
pub fn shown_folder(folder: &str) -> &str {
    folder.strip_prefix(r"\\?\").unwrap_or(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connect(id: u64, client: &str) -> Question {
        let (client, signed) = (client.to_owned(), format!("{client} 2"));
        Question { id, client, signed, project: "/p".to_owned(), asks: Asks::Connect, seconds_left: 60 }
    }

    #[test]
    fn a_question_answered_is_not_asked_again_while_the_app_still_lists_it() {
        let mut questions = Questions::default();
        questions.set(vec![connect(1, "claude"), connect(2, "cursor")]);
        questions.answer(1, Answer::Refuse);
        questions.set(vec![connect(1, "claude"), connect(2, "cursor")]);
        assert_eq!(questions.asked.iter().map(|question| question.id).collect::<Vec<_>>(), [2]);
        assert_eq!(questions.take_answers(), vec![(1, Answer::Refuse)]);
        questions.set(vec![connect(2, "cursor")]);
        assert!(questions.answered.is_empty(), "forgotten once the app no longer lists it");
    }

    #[test]
    fn the_folder_shown_has_no_prefix_of_windows() {
        assert_eq!(shown_folder(r"\\?\D:\bench"), r"D:\bench");
        assert_eq!(shown_folder("/home/ada/bench"), "/home/ada/bench");
    }
}
