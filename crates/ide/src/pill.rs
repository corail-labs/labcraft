//! The pill of the agents in the top bar (ide.md 8): no agent, or which one is attached and its policy, or that its edit
//! waits for the end of the user's gesture; a click opens its menu, where the user changes the policy of an agent for this
//! connection, keeps it for that client and that project (`ai.policies`), or disconnects it: by the commands
//! `agents.set_policy` and `agents.disconnect` (#298), as the command palette and a key call them.

use craft_keymap::CommandRef;
use egui::RichText;
use kit_shell::status::Segment;
use labcraft_engine::settings::{AI_POLICIES, forget_policy, kept_policy};
use serde_json::{Map, Value, json};

use crate::agents::{self, Connected, Policy};

/// The command of the pill: it opens its menu.
pub const COMMAND: &str = "ide.agents";

/// What the menu asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    /// A command of the window about an agent attached: `agents.disconnect` or `agents.set_policy`.
    Command(CommandRef),
    /// The new value of `ai.policies` once a policy is no longer kept (null: none left).
    Forget(Value),
    /// The `Settings` dialog, where the switch of AI and the policies kept are.
    Settings,
}

/// The menu, open under the pill or not.
#[derive(Debug, Default)]
pub struct Pill {
    /// Where the pill is, while its menu is open.
    open: Option<egui::Rect>,
}

/// The segment of the top bar. `waiting`: the agent whose edit waits for the end of the gesture (architecture 12.5).
pub fn segment(connected: &[Connected], waiting: Option<&str>) -> Segment {
    let segment = match (waiting, connected) {
        (Some(agent), _) => Segment::new(format!("● {agent} waits for your gesture")).hover(format!(
            "Its edit runs when your gesture ends, on what it gives; after {} s it gives up and changes nothing",
            labcraft_engine::gesture::WAIT.as_secs()
        )),
        (None, []) => Segment::new("No agent").hover("No AI agent is attached to this window"),
        (None, [one]) => match one.policy {
            Some(policy) => Segment::new(format!("● {} · {}", one.signed, policy.label())).hover(format!(
                "{} works on this project under the policy {}: click to change it",
                one.signed,
                policy.label()
            )),
            None => Segment::new(format!("● {} asks to connect", one.signed)).hover("Answer in the dialog of the window"),
        },
        (None, many) => Segment::new(format!("● {} agents", many.len())).hover(
            many.iter()
                .map(|agent| format!("{} · {}", agent.signed, agent.policy.map_or("asks to connect", Policy::label)))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    };
    segment.command(COMMAND).opens()
}

impl Pill {
    /// The pill was clicked: its menu opens under it, or closes.
    pub fn toggle(&mut self, below: egui::Rect) {
        self.open = if self.open.is_some() { None } else { Some(below) };
    }

    pub fn close(&mut self) {
        self.open = None;
    }

    /// The menu, while it is open: what it asks for. `settings`: the keys of the user, which hold the policies kept.
    pub fn show(&mut self, ctx: &egui::Context, connected: &[Connected], settings: &Map<String, Value>) -> Vec<Out> {
        let mut out = Vec::new();
        let Some(below) = self.open else { return out };
        let kept = settings.get(AI_POLICIES).unwrap_or(&Value::Null);
        let area = egui::Area::new(egui::Id::unique("ide-agent-pill"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(below.right_bottom() + egui::vec2(0.0, 4.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(300.0);
                    if connected.is_empty() {
                        ui.label(RichText::new("No agent").strong());
                        ui.label(RichText::new("An MCP client started in this project with `labcraft-cli mcp` attaches here.").weak());
                    }
                    for (at, agent) in connected.iter().enumerate() {
                        if at > 0 {
                            ui.separator();
                        }
                        agent_rows(ui, agent, kept, &mut out);
                    }
                    ui.separator();
                    if ui.button("AI settings…").clicked() {
                        out.push(Out::Settings);
                    }
                });
            });
        // A click on the pill itself is its toggle, drawn before.
        let on_the_pill = ctx.input(|input| input.pointer.interact_pos().is_some_and(|at| below.contains(at)));
        let done = out.iter().any(|asked| match asked {
            Out::Settings => true,
            Out::Command(command) => command.id == agents::DISCONNECT,
            Out::Forget(_) => false,
        });
        if done || ctx.input(|input| input.key_pressed(egui::Key::Escape)) || (area.response.clicked_elsewhere() && !on_the_pill) {
            self.open = None;
        }
        out
    }
}

/// The rows of one agent in the menu: its name and Disconnect, its policy, and whether it is kept. `kept`: `ai.policies`.
fn agent_rows(ui: &mut egui::Ui, agent: &Connected, kept: &Value, out: &mut Vec<Out>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(&agent.signed).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("Disconnect").clicked() {
                out.push(Out::Command(CommandRef::with_params(agents::DISCONNECT, json!({ "agent": agent.signed }))));
            }
        });
    });
    let Some(policy) = agent.policy else {
        ui.label(RichText::new("Asks to connect: answer in the dialog of the window").weak());
        return;
    };
    let remembered = kept_policy(kept, &agent.client, &agent.project) == Some(policy.as_str());
    for choice in Policy::ALL {
        let chosen = ui.radio(policy == choice, RichText::new(choice.label()).strong()).on_hover_text(choice.help()).clicked();
        if chosen && choice != policy {
            // A policy kept follows the change.
            out.push(Out::Command(set_policy(agent, choice, remembered)));
        }
    }
    let mut keep = remembered;
    if ui.checkbox(&mut keep, "Remember for this client and this project").changed() {
        out.push(if keep { Out::Command(set_policy(agent, policy, true)) } else { Out::Forget(forget_policy(kept, &agent.client, &agent.project)) });
    }
}

/// `agents.set_policy` for the agent.
fn set_policy(agent: &Connected, policy: Policy, keep: bool) -> CommandRef {
    CommandRef::with_params(agents::SET_POLICY, json!({ "agent": agent.signed, "policy": policy.as_str(), "keep": keep }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: u64, signed: &str, policy: Option<Policy>) -> Connected {
        Connected { id, client: signed.to_owned(), signed: signed.to_owned(), project: "/p".to_owned(), policy }
    }

    #[test]
    fn the_pill_says_who_is_attached_its_policy_and_a_wait() {
        assert_eq!(segment(&[], None).text, "No agent");
        assert_eq!(segment(&[agent(1, "claude", Some(Policy::Edits))], None).text, "● claude · Edits");
        assert_eq!(segment(&[agent(1, "claude", None)], None).text, "● claude asks to connect");
        let two = [agent(1, "claude", Some(Policy::Ask)), agent(2, "cursor", Some(Policy::All))];
        assert_eq!(segment(&two, None).text, "● 2 agents");
        assert_eq!(segment(&two, None).hover.as_deref(), Some("claude · Ask\ncursor · All"));
        assert_eq!(segment(&two, Some("cursor")).text, "● cursor waits for your gesture");
        let pill = segment(&[], None);
        assert!(pill.opens && pill.command.as_deref() == Some(COMMAND));
    }

    /// The menu asks by the commands of the window (#298), the agent named as its steps are signed.
    #[test]
    fn the_menu_calls_the_commands_of_the_agents() {
        let command = set_policy(&agent(2, "claude 2", Some(Policy::Ask)), Policy::All, true);
        assert_eq!(command.id, "agents.set_policy");
        assert_eq!(command.params, serde_json::json!({ "agent": "claude 2", "policy": "all", "keep": true }));
        assert!(labcraft_engine::registry::command("agents.set_policy").is_some_and(|command| command.window));
        assert!(labcraft_engine::registry::command("agents.disconnect").is_some_and(|command| command.window));
    }
}
