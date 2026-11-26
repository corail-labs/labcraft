//! `?smoke`, the smoke test of the page (`cargo xtask web-smoke`; scenario 8 of `docs/acceptance-m1.md`): the page
//! opens the examples, draws, traps its worker and draws again; then, through the host the window talks to, adds a
//! node and wires it; presses Ctrl+Z twice, which must give the file back byte for byte, and Ctrl+Y twice, which must
//! give the edit back, as keys that egui hands the window; sets a setting of the user and one of the project; runs
//! `project.download` as the menu does, and posts the zip and the name the page would save it under to the server that
//! served it, once the zip counts as saved and leaving the page asks nothing; and loads itself again
//! (`?smoke&reloaded`), where it reads the setting of the user from the session and reports. One step each frame.

use std::sync::Arc;

use js_sys::Uint8Array;
use labcraft_diagram::Drawing;
use labcraft_ide::keys::DOWNLOAD;
use labcraft_ide::{Host as _, Prefs, Reply};
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::JsValue;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::RequestInit;

use super::{LEAVING_LOSES, PREFS_KEY, Page, address, js, stored};
use crate::link::{self, DEFAULT_FUNCTION};
use crate::supervisor::Supervisor;

/// How long the smoke test waits for each step, in seconds.
const PATIENCE: f64 = 90.0;

/// The key of the store of the user that the test sets, and its value: not its default, so that it is seen to stay.
pub const USER_KEY: (&str, bool) = ("editor.mark_fixed", false);
/// The key of the store of the project that the test sets, and its value, which `web-smoke` reads in the zip.
pub const PROJECT_KEY: (&str, bool) = ("editor.drag_pans", true);
/// The node the test adds to `check_voltage` ([`DEFAULT_FUNCTION`]), and the value it wires to its input `a`.
const CALL: &str = "core.not";
const FROM: &str = "in_range";
/// The steps of the edit, each one undone and redone: `node.add` and `wire.connect`.
const STEPS: usize = 2;

pub(super) enum Smoke {
    /// Waits for the first drawing, from the start of the page (the time of egui counts from it).
    Drawing {
        since: f64,
    },
    /// Trapped the worker after the first drawing, `before`; waits for a new worker to draw anew.
    Trapped {
        since: f64,
        first: String,
        before: Option<Arc<Drawing>>,
    },
    /// The edit, the undo and the redo, the settings and the download.
    Editing(Box<Editing>),
    /// Loaded again: waits for the project, then reads the setting of the user.
    Reloaded {
        since: f64,
        reply: Option<Reply>,
    },
    Done,
}

impl Smoke {
    /// Where the test starts, after the page loaded itself again or not.
    pub(super) fn start(reloaded: bool) -> Smoke {
        if reloaded { Smoke::Reloaded { since: 0.0, reply: None } } else { Smoke::Drawing { since: 0.0 } }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Add,
    Wire,
    Undo,
    Redo,
    UserSetting,
    ProjectSetting,
    /// Until the page kept the setting of the user in its storage, which the load after reads.
    Stored,
    Download,
    /// The zip came and counts as saved: until leaving the page asks nothing, which the reload then shows.
    Saved,
}

pub(super) struct Editing {
    /// When the step started.
    since: f64,
    step: Step,
    reply: Option<Reply>,
    /// What the test did, said to the server before the page loads again.
    said: String,
    file: String,
    before: Vec<u8>,
    edited: Vec<u8>,
    node: String,
    undone: usize,
    redone: usize,
    /// The zip and the name the page saves it under.
    zip: Option<(Result<Vec<u8>, String>, String)>,
    /// The key to press at the next frame: Z for undo, Y for redo, with Ctrl (Cmd on a Mac).
    press: Option<egui::Key>,
    /// The copy of the file before the last key.
    last: Vec<u8>,
    /// What goes to the server once the zip is saved.
    sent: Option<Sent>,
}

impl Page {
    /// `?smoke`: one step each frame, until it reports.
    pub(super) fn smoke(&mut self, now: f64, ctx: &egui::Context) {
        let Some(step) = self.smoke.take() else { return };
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        let snapshot = self.shared.snapshot();
        let drawing = snapshot.view.main.drawing.clone();
        let busy = self.shared.supervisor.borrow().busy();
        let drawn = !busy && drawing.is_some() && snapshot.notices.is_empty();
        let function = snapshot.view.main.function.clone().unwrap_or_default();
        self.smoke = Some(match step {
            Smoke::Drawing { .. } if drawn && self.started == 1 => {
                let groups = drawing.as_ref().map_or(0, |drawing| drawing.groups.len());
                self.shared.with(Supervisor::trap);
                self.perform();
                Smoke::Trapped { since: now, first: format!("drew `{function}` ({groups} groups)"), before: drawing }
            }
            // The new worker drew anew: not the drawing that the page kept.
            Smoke::Trapped { first, before, .. }
                if drawn && self.started == 2 && !before.as_ref().zip(drawing.as_ref()).is_some_and(|(before, now)| Arc::ptr_eq(before, now)) =>
            {
                let said = format!("{}; {first}; the worker trapped and restarted; drew `{function}` again", self.backend);
                let file = snapshot.view.functions.iter().find(|listed| listed.function == function).map(|listed| listed.file.clone());
                match (function == DEFAULT_FUNCTION, file) {
                    (true, Some(file)) => {
                        let before = self.page_copy(&file).unwrap_or_default();
                        let reply = self.send("node.add", json!({ "file": file, "call": CALL }));
                        let editing = Editing {
                            since: now,
                            step: Step::Add,
                            reply: Some(reply),
                            said,
                            file,
                            before,
                            edited: Vec::new(),
                            node: String::new(),
                            undone: 0,
                            redone: 0,
                            zip: None,
                            press: None,
                            last: Vec::new(),
                            sent: None,
                        };
                        Smoke::Editing(Box::new(editing))
                    }
                    _ => fail(&format!("{said}, where `{DEFAULT_FUNCTION}` was expected")),
                }
            }
            Smoke::Drawing { since } | Smoke::Trapped { since, .. } if now - since > PATIENCE => {
                fail(&format!("{} workers started, function {function:?}, notices {:?}", self.started, snapshot.notices))
            }
            Smoke::Editing(editing) => self.edit(now, editing),
            Smoke::Reloaded { since, reply: None } => {
                if self.has_project() && !busy && snapshot.view.folder.is_some() {
                    let reply = self.send("settings.get", json!({ "key": USER_KEY.0 }));
                    Smoke::Reloaded { since: now, reply: Some(reply) }
                } else if now - since > PATIENCE {
                    fail(&format!("after the reload, no project opened: notices {:?}", snapshot.notices))
                } else {
                    Smoke::Reloaded { since, reply: None }
                }
            }
            Smoke::Reloaded { since, reply: Some(reply) } => match reply.take() {
                None if now - since > PATIENCE => fail("after the reload, `settings.get` has no answer"),
                None => Smoke::Reloaded { since, reply: Some(reply) },
                Some(Err(error)) => fail(&format!("after the reload, `settings.get`: {}: {}", error.code, error.message)),
                Some(Ok(value)) => {
                    let setting = value.pointer("/settings/0").cloned().unwrap_or(Value::Null);
                    let (key, set) = USER_KEY;
                    if setting.get("value") == Some(&json!(set)) && setting.get("from") == Some(&json!("user")) {
                        report(&format!("ok: after the reload, the session holds `{key}` = {set}, from the store of the user"));
                        Smoke::Done
                    } else {
                        fail(&format!("after the reload, `{key}` is {setting}, where {set} from the store of the user was set"))
                    }
                }
            },
            other => other,
        });
    }

    /// The zip of a download and the name the page saves it under, when the test waits for it; else it goes back to the
    /// page.
    pub(super) fn smoke_takes(&mut self, zip: Result<Vec<u8>, String>, name: &str) -> Option<Result<Vec<u8>, String>> {
        match &mut self.smoke {
            Some(Smoke::Editing(editing)) if editing.step == Step::Download => {
                editing.zip = Some((zip, name.to_owned()));
                None
            }
            _ => Some(zip),
        }
    }

    /// The key the test presses, given to egui before the window reads its input, as a browser gives it.
    pub(super) fn smoke_keys(&mut self, ctx: &egui::Context) {
        let Some(Smoke::Editing(editing)) = &mut self.smoke else { return };
        let Some(key) = editing.press.take() else { return };
        let modifiers = if ctx.os() == egui::os::OperatingSystem::Mac {
            egui::Modifiers { mac_cmd: true, command: true, ..egui::Modifiers::default() }
        } else {
            egui::Modifiers { ctrl: true, command: true, ..egui::Modifiers::default() }
        };
        ctx.input_mut(|input| {
            for pressed in [true, false] {
                input.events.push(egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers });
            }
        });
    }

    /// Whether the copy of the file changed since the last key, once the session answered: the key did something.
    fn keyed(&self, editing: &Editing) -> Option<Vec<u8>> {
        let copy = self.page_copy(&editing.file).unwrap_or_default();
        (editing.press.is_none() && !self.shared.supervisor.borrow().busy() && copy != editing.last).then_some(copy)
    }

    /// A command of the session, through the host the window talks to, sent at once.
    fn send(&mut self, id: &str, params: Value) -> Reply {
        let reply = self.shared.send(id, params);
        self.perform();
        reply
    }

    /// The bytes of a file, as the page keeps them for a new worker.
    fn page_copy(&self, path: &str) -> Option<Vec<u8>> {
        let supervisor = self.shared.supervisor.borrow();
        supervisor.folder()?.files.iter().find(|file| file.path == path).map(|file| file.bytes.clone())
    }

    /// One step of the edit, once the command before it answered.
    fn edit(&mut self, now: f64, mut editing: Box<Editing>) -> Smoke {
        let step = editing.step;
        if now - editing.since > PATIENCE {
            return fail(&format!("{}; then {step:?} waited more than {PATIENCE} s", editing.said));
        }
        // A step that waits for no command (the storage, the zip) waits for its own thing.
        let answered = match editing.reply.take() {
            Some(reply) => match reply.take() {
                None => {
                    editing.reply = Some(reply);
                    return Smoke::Editing(editing);
                }
                Some(Err(error)) => return fail(&format!("{}; then {step:?}: {}: {}", editing.said, error.code, error.message)),
                Some(Ok(value)) => {
                    editing.since = now;
                    value
                }
            },
            None => Value::Null,
        };
        let file = editing.file.clone();
        match step {
            Step::Add => {
                let Some(node) = answered.pointer("/names/node").and_then(Value::as_str) else {
                    return fail(&format!("{}; then `node.add` named no node: {answered}", editing.said));
                };
                editing.node = node.to_owned();
                editing.reply = Some(self.send("wire.connect", json!({ "file": file, "from": FROM, "to": format!("{node}.a") })));
                editing.step = Step::Wire;
            }
            Step::Wire => {
                editing.edited = self.page_copy(&file).unwrap_or_default();
                let holds = String::from_utf8_lossy(&editing.edited).contains(&format!("{} = {CALL}(", editing.node));
                if editing.edited == editing.before || !holds {
                    return fail(&format!("{}; then the copy of `{file}` does not hold `{} = {CALL}(…)`", editing.said, editing.node));
                }
                editing.said = format!("{}; added `{}` ({CALL}) to `{file}` and wired `{FROM}` to it", editing.said, editing.node);
                editing.last = editing.edited.clone();
                editing.press = Some(egui::Key::Z);
                editing.step = Step::Undo;
            }
            // Each Ctrl+Z undoes one step: the first leaves the node, the second gives the file back.
            Step::Undo => {
                let Some(copy) = self.keyed(&editing) else { return Smoke::Editing(editing) };
                editing.undone += 1;
                editing.since = now;
                let back = copy == editing.before;
                if back != (editing.undone == STEPS) {
                    return fail(&format!("{}; then Ctrl+Z {} gave `{file}` back: {back}", editing.said, editing.undone));
                }
                editing.last = copy;
                editing.press = Some(if back { egui::Key::Y } else { egui::Key::Z });
                if back {
                    editing.step = Step::Redo;
                }
            }
            Step::Redo => {
                let Some(copy) = self.keyed(&editing) else { return Smoke::Editing(editing) };
                editing.redone += 1;
                editing.since = now;
                let again = copy == editing.edited;
                if again != (editing.redone == STEPS) {
                    return fail(&format!("{}; then Ctrl+Y {} gave the edit back: {again}", editing.said, editing.redone));
                }
                editing.last = copy;
                if again {
                    editing.said = format!("{}; Ctrl+Z twice gave `{file}` back byte for byte, Ctrl+Y twice the edit", editing.said);
                    let (key, value) = USER_KEY;
                    editing.reply = Some(self.send("settings.set", json!({ "key": key, "value": value })));
                    editing.step = Step::UserSetting;
                } else {
                    editing.press = Some(egui::Key::Y);
                }
            }
            Step::UserSetting => {
                let (key, value) = PROJECT_KEY;
                editing.reply = Some(self.send("settings.set", json!({ "key": key, "value": value, "scope": "project" })));
                editing.step = Step::ProjectSetting;
            }
            Step::ProjectSetting => {
                editing.said = format!(
                    "{}; set `{}` = {} for the user and `{}` = {} for the project",
                    editing.said, USER_KEY.0, USER_KEY.1, PROJECT_KEY.0, PROJECT_KEY.1
                );
                editing.step = Step::Stored;
                editing.since = now;
            }
            Step::Stored => {
                let kept = stored(PREFS_KEY).and_then(|json| serde_json::from_str::<Prefs>(&json).ok());
                if kept.is_some_and(|prefs| prefs.settings.get(USER_KEY.0) == Some(&json!(USER_KEY.1))) {
                    // As File > Download runs it.
                    self.ide.ask(DOWNLOAD);
                    editing.step = Step::Download;
                    editing.since = now;
                }
            }
            Step::Download => match editing.zip.take() {
                None => {}
                Some((Err(why), _)) => return fail(&format!("{}; then {why}", editing.said)),
                Some((Ok(zip), name)) => {
                    if !LEAVING_LOSES.with(std::cell::Cell::get) {
                        return fail(&format!("{}; then leaving the page lost nothing before the zip", editing.said));
                    }
                    // As the page does once `save` gave the zip to the browser (`save` itself is tried by hand).
                    self.shared.supervisor.borrow_mut().mark_saved();
                    let said = format!(
                        "{}; `{DOWNLOAD}` gave {} bytes, to save as `{name}`, and leaving the page then asks nothing; the page loads again",
                        editing.said,
                        zip.len()
                    );
                    editing.sent = Some(Sent { zip, name, file, node: editing.node.clone(), said });
                    editing.step = Step::Saved;
                    editing.since = now;
                }
            },
            // The window writes the flag at the next frame: the zip saved, leaving loses nothing.
            Step::Saved => {
                if !LEAVING_LOSES.with(std::cell::Cell::get)
                    && let Some(sent) = editing.sent.take()
                {
                    spawn_local(send_zip_and_reload(sent));
                    return Smoke::Done;
                }
            }
        }
        Smoke::Editing(editing)
    }
}

/// Reports a failure, which ends the test.
fn fail(text: &str) -> Smoke {
    report(&format!("fail: {text}"));
    Smoke::Done
}

/// Tells the server that served the page how the smoke test went.
fn report(text: &str) {
    let body = JsValue::from_str(text);
    spawn_local(async move {
        let _ = post("/smoke", &body).await;
    });
}

/// Posts to the server that served the page, and says whether it took it.
async fn post(path: &str, body: &JsValue) -> Result<(), String> {
    let window = web_sys::window().ok_or_else(|| "the page has no window".to_owned())?;
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_body(body);
    let response = JsFuture::from(window.fetch_with_str_and_init(path, &init)).await.map_err(js)?;
    let response: web_sys::Response = response.dyn_into().map_err(|_| "not a response".to_owned())?;
    if response.ok() { Ok(()) } else { Err(format!("{path}: {}", response.status())) }
}

/// What goes to the server once the zip came: the zip, the name the page saves it under, the file edited, the node
/// added, and what the test did.
struct Sent {
    zip: Vec<u8>,
    name: String,
    file: String,
    node: String,
    said: String,
}

/// Posts the zip, its name, the file edited and the node added in the address, then what the test did; then loads the
/// page again, once the server has both.
async fn send_zip_and_reload(sent: Sent) {
    let Sent { zip, name, file, node, said } = sent;
    let body = Uint8Array::from(zip.as_slice());
    let to = format!("/smoke-zip?name={}&file={}&node={}", link::encode(&name), link::encode(&file), link::encode(&node));
    if let Err(why) = post(&to, &body).await {
        report(&format!("fail: {said}; the zip did not reach the server: {why}"));
        return;
    }
    if let Err(why) = post("/smoke-step", &JsValue::from_str(&said)).await {
        report(&format!("fail: {said}; the server did not take it: {why}"));
        return;
    }
    let Some(window) = web_sys::window() else { return };
    let webgl = if address().webgl { "&webgl" } else { "" };
    let path = window.location().pathname().unwrap_or_default();
    if let Err(error) = window.location().replace(&format!("{path}?smoke&reloaded{webgl}")) {
        report(&format!("fail: {said}; the page could not load again: {}", js(error)));
    }
}
