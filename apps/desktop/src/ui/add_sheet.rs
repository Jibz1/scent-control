//! The Add Diffuser sheet: names a diffuser found over Bluetooth and puts it on our
//! broker (see `add_flow`), showing each step as the hub reports it.
//!
//! It opens from a diffuser's "Add…" button under Found Nearby, or from the notice on a
//! diffuser that hasn't been heard on our broker. When the diffuser isn't on WiFi yet,
//! the hub stops and the sheet asks for the network; and when an add that gave one
//! fails, Try Again asks for it again, filled in, with the password to type again. Its password is typed into a masked
//! field and handed over as a [`Secret`] once; the field, and the copy of it the text
//! box keeps for undoing, are emptied on Add, Cancel, Escape, ⌘W and when the window
//! closes.

use eframe::egui::text_edit::TextEditState;
use eframe::egui::{
    self, Id, Key, KeyboardShortcut, Modifiers, Rect, RichText, Sense, Spinner, Stroke, TextStyle,
    WidgetInfo, WidgetType, pos2, vec2,
};
use scent_core::{Secret, parse_name};

use super::theme::{GRID, Palette, headline};
use super::widgets::{
    NoticeKind, default_button, footnote, grouped, notice_with, one_line, plain_button, row,
    secondary, sheet,
};
use crate::add_flow::{AddRequest, AddStep};
use crate::hub::UiCommand;
use crate::state::{Adding, AppState};

/// The sheet's width.
pub const WIDTH: f32 = 460.0;

/// The width of the text fields.
const FIELD_WIDTH: f32 = 240.0;

/// The height of a step's line, and the size of the mark beside it.
const STEP_HEIGHT: f32 = 22.0;
const MARK_SIZE: f32 = 14.0;

const READ: &str = "Reading the diffuser…";
const MOVE: &str = "Moving it to your broker…";
const JOIN: &str = "Joining your WiFi…";
const RESTART: &str = "Restarting…";
const CONNECT: &str = "Waiting for it to connect…";
const DONE: &str = "Done";

/// The name for a diffuser from its Bluetooth name, which says its model by a letter.
pub fn name_from_scan(scan_name: &str) -> String {
    let name = parse_name(scan_name);
    match name.model_code.as_deref() {
        Some("D") => "Air 2".to_owned(),
        Some("H") => "HVAC 2".to_owned(),
        _ => name.model.unwrap_or_else(|| "Diffuser".to_owned()),
    }
}

/// The name for a diffuser from the model it gave in its info: "Diffuser" is the Air 2,
/// "Hvac" the HVAC 2.
pub fn name_from_model(model: &str) -> String {
    let lower = model.to_ascii_lowercase();
    if lower.contains("hvac") {
        "HVAC 2".to_owned()
    } else if lower.contains("air") || lower == "diffuser" {
        "Air 2".to_owned()
    } else if model.is_empty() {
        "Diffuser".to_owned()
    } else {
        model.to_owned()
    }
}

/// How far a step of the list has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Done,
    /// Under way.
    Now,
    Later,
    /// Where the add stopped.
    Failed,
}

impl Mark {
    /// The mark in words, for accessibility: color and shape are never the only signal.
    fn words(self) -> &'static str {
        match self {
            Mark::Done => "done",
            Mark::Now => "under way",
            Mark::Later => "not started",
            Mark::Failed => "failed",
        }
    }
}

/// Which line of the list a step is on: the ping is part of the wait.
fn line_of(step: &AddStep) -> Option<&'static str> {
    match step {
        AddStep::ReadInfo => Some(READ),
        AddStep::SaveMqtt => Some(MOVE),
        AddStep::WifiConnect => Some(JOIN),
        AddStep::Restart => Some(RESTART),
        AddStep::WaitConnected | AddStep::Ping => Some(CONNECT),
        AddStep::Done(_) => Some(DONE),
        AddStep::NeedWifi | AddStep::Failed(_) => None,
    }
}

/// The steps of an add, each with how far it has got. `step` is where the add is,
/// `reached` the last step the hub worked on, and `with_wifi` whether a network was
/// given, which makes joining it a step. Empty when the add failed before any step.
pub fn step_list(
    step: &AddStep,
    reached: Option<&AddStep>,
    with_wifi: bool,
) -> Vec<(&'static str, Mark)> {
    let with_wifi = with_wifi
        || matches!(step, AddStep::NeedWifi | AddStep::WifiConnect)
        || reached == Some(&AddStep::WifiConnect);
    let lines: Vec<&str> = [READ, MOVE, JOIN, RESTART, CONNECT, DONE]
        .into_iter()
        .filter(|line| with_wifi || *line != JOIN)
        .collect();
    let at = |line: &str| lines.iter().position(|shown| *shown == line);
    // Where the list stands: the line under way (or failed), and how it is marked.
    let (current, mark) = match step {
        AddStep::Failed(_) => match reached.and_then(line_of).and_then(at) {
            Some(i) => (i, Mark::Failed),
            None => return Vec::new(),
        },
        AddStep::NeedWifi => (1, Mark::Later),
        AddStep::Done(_) => (lines.len(), Mark::Done),
        running => (line_of(running).and_then(at).unwrap_or(0), Mark::Now),
    };
    lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let shown = match i.cmp(&current) {
                std::cmp::Ordering::Less => Mark::Done,
                std::cmp::Ordering::Equal => mark,
                std::cmp::Ordering::Greater => Mark::Later,
            };
            (line, shown)
        })
        .collect()
}

/// The diffuser the sheet adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub ble_id: String,
    /// The name the field starts with.
    pub name: String,
}

/// Where the sheet is, from the add the state follows for its diffuser.
enum Phase<'a> {
    /// Nothing asked for yet in this sheet.
    Ready,
    Running(&'a Adding),
    NeedWifi(&'a Adding),
    /// Try Again after an add that gave a network: the network again, before adding.
    RetryWifi,
    Done(&'a Adding),
    Failed(&'a Adding, &'a str),
}

/// What the person did in the sheet this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Add,
    TryAgain,
    /// Cancel, Done, Escape, ⌘W or the window's close button.
    Close,
}

#[derive(Default)]
pub struct AddSheet {
    /// The diffuser being added, while the sheet is open.
    target: Option<Target>,
    name: String,
    network: String,
    /// What is typed in the masked field. Emptied when the sheet adds or closes.
    password: String,
    /// Whether Add was pressed since the sheet opened. Until then an earlier add's
    /// outcome for this diffuser isn't shown, only one still under way.
    submitted: bool,
    /// Try Again was pressed after an add that gave a network: it is asked for again.
    retry_wifi: bool,
}

impl AddSheet {
    fn password_id() -> Id {
        Id::new("add_sheet_wifi_password")
    }

    /// Opens the sheet for `target`, with its name in the field and the network empty.
    pub fn open(&mut self, target: Target) {
        self.name.clone_from(&target.name);
        self.target = Some(target);
        self.network = String::new();
        self.password = String::new();
        self.submitted = false;
        self.retry_wifi = false;
    }

    pub fn is_open(&self) -> bool {
        self.target.is_some()
    }

    /// Whether the sheet is open for the diffuser at `ble_id`.
    pub fn is_for(&self, ble_id: &str) -> bool {
        self.target
            .as_ref()
            .is_some_and(|target| target.ble_id == ble_id)
    }

    /// As though Add had been pressed, for the snapshots to show the add's progress.
    #[cfg(test)]
    pub fn as_submitted(&mut self) {
        self.submitted = true;
    }

    /// As though Try Again had been pressed after an add that gave `network`.
    #[cfg(test)]
    pub fn as_retrying(&mut self, network: &str) {
        self.submitted = true;
        self.retry_wifi = true;
        network.clone_into(&mut self.network);
    }

    fn close(&mut self, ctx: &egui::Context) {
        self.forget_password(ctx);
        self.target = None;
        self.network = String::new();
        self.submitted = false;
    }

    /// Empties the password field, and the copy of it the text box keeps for undoing.
    fn forget_password(&mut self, ctx: &egui::Context) {
        self.password = String::new();
        TextEditState::default().store(ctx, Self::password_id());
    }

    fn phase<'a>(&self, state: &'a AppState) -> Phase<'a> {
        let ble_id = self.target.as_ref().map(|target| target.ble_id.as_str());
        let Some(adding) = ble_id.and_then(|ble_id| state.adding.get(ble_id)) else {
            return Phase::Ready;
        };
        match &adding.step {
            step if step.is_running() => Phase::Running(adding),
            _ if !self.submitted => Phase::Ready,
            _ if self.retry_wifi => Phase::RetryWifi,
            AddStep::NeedWifi => Phase::NeedWifi(adding),
            AddStep::Done(_) => Phase::Done(adding),
            AddStep::Failed(why) => Phase::Failed(adding, why),
            _ => Phase::Running(adding),
        }
    }

    /// Shows the sheet when it is open. Returns the add to hand the hub when Add or Try
    /// Again was pressed; the sheet stays open to show how it goes.
    pub fn show(&mut self, ctx: &egui::Context, state: &AppState) -> Option<UiCommand> {
        if !self.is_open() {
            return None;
        }
        let phase = self.phase(state);
        let close_window = KeyboardShortcut::new(Modifiers::COMMAND, Key::W);
        let (escape, command_w, closing) = ctx.input_mut(|input| {
            (
                input.key_pressed(Key::Escape),
                input.consume_shortcut(&close_window),
                input.viewport().close_requested(),
            )
        });
        let enter = ctx.input(|input| input.key_pressed(Key::Enter))
            && ctx.memory(|memory| memory.focused().is_none());
        let mut action = sheet(ctx, Id::new("add_sheet"), WIDTH, |ui| {
            self.contents(ui, &phase)
        });
        if escape || command_w || closing {
            action = Some(Action::Close);
        } else if action.is_none() && enter {
            action = match phase {
                Phase::Done(_) => Some(Action::Close),
                _ if self.can_add(&phase) => Some(Action::Add),
                _ => None,
            };
        }
        let wifi = matches!(phase, Phase::NeedWifi(_) | Phase::RetryWifi);
        let given = match phase {
            Phase::Failed(adding, _) => adding.network.clone(),
            _ => None,
        };
        match action? {
            Action::Close => {
                self.close(ctx);
                None
            }
            Action::Add => Some(self.submit(ctx, wifi)),
            // The network it was given may be what kept it off the broker: ask for it
            // again, with its password to type again, before anything is sent.
            Action::TryAgain => match given {
                Some(network) => {
                    self.network = network;
                    self.forget_password(ctx);
                    self.retry_wifi = true;
                    None
                }
                None => Some(self.submit(ctx, false)),
            },
        }
    }

    /// Whether Add can be pressed: not while an add is under way or done, and with a
    /// network named when one is asked for.
    fn can_add(&self, phase: &Phase<'_>) -> bool {
        match phase {
            Phase::Ready => true,
            Phase::NeedWifi(_) | Phase::RetryWifi => !self.network.trim().is_empty(),
            Phase::Running(_) | Phase::Done(_) | Phase::Failed(..) => false,
        }
    }

    /// What Add hands the hub: the name, and the network when `wifi`. The password
    /// moves out of the field into the [`Secret`], and the field and the text box's
    /// undo copy are emptied.
    fn submit(&mut self, ctx: &egui::Context, wifi: bool) -> UiCommand {
        let typed = std::mem::take(&mut self.password);
        self.forget_password(ctx);
        let wifi = wifi.then(|| (self.network.trim().to_owned(), Secret::new(typed)));
        self.submitted = true;
        self.retry_wifi = false;
        let ble_id = self
            .target
            .as_ref()
            .map(|target| target.ble_id.clone())
            .unwrap_or_default();
        UiCommand::Add(AddRequest {
            ble_id,
            label: self.name.trim().to_owned(),
            wifi,
        })
    }

    /// What the sheet holds: the name, the network when it is asked for, the steps, what
    /// went wrong, and the buttons.
    fn contents(&mut self, ui: &mut egui::Ui, phase: &Phase<'_>) -> Option<Action> {
        let palette = Palette::of(ui);
        let mut action = None;
        ui.label(
            RichText::new("Add Diffuser")
                .text_style(headline())
                .color(palette.label),
        );
        ui.add_space(GRID / 2.0);
        secondary(
            ui,
            "Scent Control reads the diffuser over Bluetooth, points it at your broker and restarts it. Keep this Mac near it, with the Scentiment app closed.",
        );
        ui.add_space(2.0 * GRID);

        let editable = matches!(
            phase,
            Phase::Ready | Phase::NeedWifi(_) | Phase::RetryWifi | Phase::Failed(..)
        );
        let mut enter = false;
        ui.add_enabled_ui(editable, |ui| {
            grouped(ui, "", |ui| {
                row(ui, "Name", |ui| {
                    let name = egui::TextEdit::singleline(&mut self.name)
                        .id(Id::new("add_sheet_name"))
                        .hint_text("Name");
                    enter |= field(ui, name);
                });
            });
        });
        footnote(ui, "What it is called in the list.");

        let wifi_note = match phase {
            Phase::NeedWifi(_) => Some("It isn't on WiFi yet."),
            Phase::RetryWifi => Some("Check the network, and type its password again."),
            _ => None,
        };
        if let Some(why) = wifi_note {
            ui.add_space(2.0 * GRID);
            grouped(ui, "WiFi", |ui| {
                row(ui, "Network", |ui| {
                    let network = egui::TextEdit::singleline(&mut self.network)
                        .id(Id::new("add_sheet_network"))
                        .hint_text("Network name");
                    enter |= field(ui, network);
                });
                row(ui, "Password", |ui| {
                    let password = egui::TextEdit::singleline(&mut self.password)
                        .id(Self::password_id())
                        .password(true)
                        .hint_text("WiFi password");
                    enter |= field(ui, password);
                });
            });
            footnote(
                ui,
                &format!(
                    "{why} 2.4 GHz networks only. The password goes to the diffuser once and isn't kept."
                ),
            );
        }

        let steps = match phase {
            Phase::Ready | Phase::RetryWifi => Vec::new(),
            Phase::Running(adding)
            | Phase::NeedWifi(adding)
            | Phase::Done(adding)
            | Phase::Failed(adding, _) => step_list(
                &adding.step,
                adding.reached.as_ref(),
                adding.network.is_some(),
            ),
        };
        if !steps.is_empty() {
            ui.add_space(2.0 * GRID);
            for (line, mark) in steps {
                step_line(ui, line, mark);
            }
        }
        if let Phase::Running(_) = phase {
            ui.add_space(GRID / 2.0);
            secondary(ui, "Closing this sheet doesn't stop the add.");
        }
        if let Phase::Done(_) = phase {
            ui.add_space(GRID / 2.0);
            secondary(ui, "It's on your broker, and will answer pings shortly.");
        }
        if let Phase::Failed(_, why) = phase {
            ui.add_space(2.0 * GRID);
            let (again, _) = notice_with(ui, NoticeKind::Error, why, |ui| {
                plain_button(ui, "Try Again").clicked()
            });
            if again {
                action = Some(Action::TryAgain);
            }
        }

        ui.add_space(2.0 * GRID);
        let can_add = self.can_add(phase);
        egui::Sides::new().show(
            ui,
            |_| {},
            |ui| {
                if let Phase::Done(_) = phase {
                    if default_button(ui, "Done").clicked() {
                        action = Some(Action::Close);
                    }
                    return;
                }
                if !matches!(phase, Phase::Failed(..)) {
                    let add = ui.add_enabled_ui(can_add, |ui| default_button(ui, "Add"));
                    if add.inner.clicked() {
                        action = Some(Action::Add);
                    }
                }
                if plain_button(ui, "Cancel").clicked() {
                    action = Some(Action::Close);
                }
            },
        );
        if action.is_none() && enter && can_add {
            action = Some(Action::Add);
        }
        action
    }
}

/// A one-line text box of the fields' width. True when Enter was pressed in it.
fn field(ui: &mut egui::Ui, edit: egui::TextEdit<'_>) -> bool {
    let response = ui.add(edit.desired_width(FIELD_WIDTH));
    response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter))
}

/// One step of the add: a mark for how far it has got, then what it is. A step to come
/// is in the secondary label color.
fn step_line(ui: &mut egui::Ui, line: &str, mark: Mark) {
    let palette = Palette::of(ui);
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), STEP_HEIGHT), Sense::hover());
    response.widget_info(|| {
        WidgetInfo::labeled(WidgetType::Label, true, format!("{line} {}", mark.words()))
    });
    if !ui.is_rect_visible(rect) {
        return;
    }
    let at = Rect::from_center_size(
        pos2(rect.left() + MARK_SIZE / 2.0, rect.center().y),
        vec2(MARK_SIZE, MARK_SIZE),
    );
    let painter = ui.painter();
    let radius = MARK_SIZE / 2.0;
    let white = Stroke::new(1.6, egui::Color32::WHITE);
    match mark {
        Mark::Done => {
            painter.circle_filled(at.center(), radius, palette.green);
            let tick = [
                pos2(at.left() + 3.5, at.center().y + 0.5),
                pos2(at.center().x - 1.0, at.bottom() - 4.0),
                pos2(at.right() - 3.5, at.top() + 4.0),
            ];
            painter.line_segment([tick[0], tick[1]], white);
            painter.line_segment([tick[1], tick[2]], white);
        }
        Mark::Now => Spinner::new()
            .size(MARK_SIZE)
            .color(palette.secondary_label)
            .paint_at(ui, at),
        Mark::Later => {
            painter.circle_stroke(
                at.center(),
                radius - 0.75,
                Stroke::new(1.5, Palette::tint(palette.secondary_label, 60)),
            );
        }
        Mark::Failed => {
            painter.circle_filled(at.center(), radius, palette.red);
            let x = at.center().x;
            painter.line_segment(
                [pos2(x, at.top() + 3.5), pos2(x, at.center().y + 1.5)],
                white,
            );
            painter.circle_filled(pos2(x, at.bottom() - 3.5), 1.0, egui::Color32::WHITE);
        }
    }
    let color = if mark == Mark::Later {
        palette.secondary_label
    } else {
        palette.label
    };
    let text = one_line(ui, line, TextStyle::Body, rect.width() - MARK_SIZE - GRID);
    let text_at = pos2(at.right() + GRID, rect.center().y - text.size().y / 2.0);
    ui.painter().galley(text_at, text, color);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use eframe::egui::{Key, Modifiers, ViewportEvent, ViewportId};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

    use super::super::theme;
    use super::*;
    use crate::add_flow::AddStep;
    use crate::state::Adding;

    const BLE: &str = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";

    #[test]
    fn the_name_comes_from_the_model() {
        assert_eq!(name_from_scan("Scentiment-D B n 0A1B2C3D4E5F"), "Air 2");
        assert_eq!(name_from_scan("Scentiment-H B n 0A1B2C3D4E5F"), "HVAC 2");
        assert_eq!(name_from_scan("Scentiment-M W"), "Mini");
        assert_eq!(name_from_scan("Scentiment"), "Diffuser");
        // From what the diffuser told us about itself.
        assert_eq!(name_from_model("Diffuser"), "Air 2");
        assert_eq!(name_from_model("Diffuser Air 2"), "Air 2");
        assert_eq!(name_from_model("Hvac"), "HVAC 2");
        assert_eq!(name_from_model("Diffuser HVAC 2"), "HVAC 2");
        assert_eq!(name_from_model(""), "Diffuser");
    }

    #[test]
    fn the_steps_say_how_far_it_got() {
        use Mark::{Done, Failed, Later, Now};
        let read = "Reading the diffuser…";
        let r#move = "Moving it to your broker…";
        let join = "Joining your WiFi…";
        let restart = "Restarting…";
        let connect = "Waiting for it to connect…";
        let done = "Done";
        assert_eq!(
            step_list(&AddStep::SaveMqtt, Some(&AddStep::SaveMqtt), false),
            [
                (read, Done),
                (r#move, Now),
                (restart, Later),
                (connect, Later),
                (done, Later)
            ]
        );
        // WiFi given: joining it is a step. The ping is part of the wait.
        for step in [AddStep::WaitConnected, AddStep::Ping] {
            assert_eq!(
                step_list(&step, Some(&step), true),
                [
                    (read, Done),
                    (r#move, Done),
                    (join, Done),
                    (restart, Done),
                    (connect, Now),
                    (done, Later)
                ]
            );
        }
        let finished = AddStep::Done("TESTDEVICE01".to_owned());
        assert!(
            step_list(&finished, Some(&AddStep::Ping), false)
                .iter()
                .all(|(_, mark)| *mark == Done)
        );
        // A failure marks the step it happened at.
        let failed = AddStep::Failed("No.".to_owned());
        assert_eq!(
            step_list(&failed, Some(&AddStep::Restart), false),
            [
                (read, Done),
                (r#move, Done),
                (restart, Failed),
                (connect, Later),
                (done, Later)
            ]
        );
        // One that happened before any step, such as no broker password: no steps.
        assert!(step_list(&failed, None, false).is_empty());
        // Waiting for the network: the read is done, and joining it is to come.
        assert_eq!(
            step_list(&AddStep::NeedWifi, Some(&AddStep::ReadInfo), false),
            [
                (read, Done),
                (r#move, Later),
                (join, Later),
                (restart, Later),
                (connect, Later),
                (done, Later)
            ]
        );
    }

    /// The sheet with what it handed over, for a harness to drive. The modal is drawn
    /// over the harness's window.
    struct Rig {
        sheet: AddSheet,
        state: AppState,
        sent: Vec<UiCommand>,
    }

    fn harness(sheet: AddSheet, state: AppState) -> Harness<'static, Rig> {
        let mut frames = 0;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 600.0))
            .with_os(egui::os::OperatingSystem::Mac)
            .build_ui_state(
                move |ui, rig: &mut Rig| {
                    frames += 1;
                    if frames == 1 {
                        theme::install(ui.ctx(), Path::new("/nonexistent"));
                        ui.ctx().request_repaint();
                        return;
                    }
                    if let Some(command) = rig.sheet.show(ui.ctx(), &rig.state) {
                        rig.sent.push(command);
                    }
                },
                Rig {
                    sheet,
                    state,
                    sent: Vec::new(),
                },
            );
        harness.run_steps(2);
        harness
    }

    /// The state, following an add of the diffuser at `step`.
    fn adding_at(step: AddStep, reached: Option<AddStep>) -> AppState {
        adding_with(step, reached, None)
    }

    /// As [`adding_at`], for an add that gave `network`.
    fn adding_with(step: AddStep, reached: Option<AddStep>, network: Option<&str>) -> AppState {
        let adding = Adding {
            ble_id: BLE.to_owned(),
            label: "Air 2".to_owned(),
            network: network.map(str::to_owned),
            step,
            reached,
        };
        AppState {
            adding: [(BLE.to_owned(), adding)].into(),
            ..AppState::default()
        }
    }

    fn opened() -> AddSheet {
        let mut sheet = AddSheet::default();
        sheet.open(Target {
            ble_id: BLE.to_owned(),
            name: "Air 2".to_owned(),
        });
        sheet
    }

    /// The sheet asking for the network, after the hub said so, with "ExampleWiFi" and
    /// "hunter2" typed, so that the password box's undo copy holds it too.
    fn typed_in() -> Harness<'static, Rig> {
        let mut sheet = opened();
        sheet.submitted = true;
        let state = adding_at(AddStep::NeedWifi, Some(AddStep::ReadInfo));
        let mut harness = harness(sheet, state);
        let network = harness.get_by(|node| node.placeholder() == Some("Network name"));
        network.focus();
        network.type_text("ExampleWiFi");
        harness.run_steps(2);
        let password = harness.get_by(|node| node.placeholder() == Some("WiFi password"));
        password.focus();
        password.type_text("hunter2");
        harness.run_steps(2);
        assert_eq!(harness.state().sheet.password, "hunter2");
        assert!(undo_kept(&harness));
        harness
    }

    /// Whether the password box keeps something to undo to, which would hold what was
    /// typed.
    fn undo_kept(harness: &Harness<'_, Rig>) -> bool {
        let state = TextEditState::load(&harness.ctx, AddSheet::password_id());
        let typed = (egui::text::CCursorRange::default(), "hunter2".to_owned());
        state.is_some_and(|state| {
            let undoer = state.undoer();
            undoer.is_in_flux() || undoer.has_undo(&typed)
        })
    }

    /// The sheet is shut, holds the password nowhere, and handed nothing over.
    fn forgotten(harness: &Harness<'_, Rig>) {
        let rig = harness.state();
        assert!(!rig.sheet.is_open());
        assert!(rig.sheet.password.is_empty());
        assert!(!undo_kept(harness));
        assert!(rig.sent.is_empty());
    }

    #[test]
    fn cancel_forgets_the_wifi_password() {
        let mut harness = typed_in();
        harness.get_by_label("Cancel").click();
        harness.run_steps(2);
        forgotten(&harness);
    }

    #[test]
    fn escape_forgets_the_wifi_password() {
        let mut harness = typed_in();
        harness.key_press(Key::Escape);
        harness.run_steps(2);
        forgotten(&harness);
    }

    #[test]
    fn command_w_forgets_the_wifi_password() {
        let mut harness = typed_in();
        harness.key_press_modifiers(Modifiers::COMMAND, Key::W);
        harness.run_steps(2);
        forgotten(&harness);
    }

    #[test]
    fn closing_the_window_forgets_the_wifi_password() {
        let mut harness = typed_in();
        harness
            .input_mut()
            .viewports
            .entry(ViewportId::ROOT)
            .or_default()
            .events
            .push(ViewportEvent::Close);
        harness.run_steps(2);
        forgotten(&harness);
    }

    #[test]
    fn adding_hands_over_the_wifi_password_and_empties_the_field() {
        let mut harness = typed_in();
        harness.get_by_label("Add").click();
        harness.run_steps(2);
        let rig = harness.state();
        let [UiCommand::Add(request)] = &rig.sent[..] else {
            panic!("one Add");
        };
        assert_eq!(request.ble_id, BLE);
        assert_eq!(request.label, "Air 2");
        let (network, password) = request.wifi.as_ref().expect("the network");
        assert_eq!(
            (network.as_str(), password.expose()),
            ("ExampleWiFi", "hunter2")
        );
        // The sheet stays open, to show the add, and holds the password nowhere.
        assert!(rig.sheet.is_open());
        assert!(rig.sheet.password.is_empty());
        assert!(!undo_kept(&harness));
    }

    #[test]
    fn add_hands_over_the_name_and_no_network_when_none_was_asked_for() {
        let mut harness = harness(opened(), AppState::default());
        harness.get_by_label("Add").click();
        harness.run_steps(2);
        let [UiCommand::Add(request)] = &harness.state().sent[..] else {
            panic!("one Add");
        };
        assert_eq!(
            (request.ble_id.as_str(), request.label.as_str()),
            (BLE, "Air 2")
        );
        assert!(request.wifi.is_none());
        // Nothing asks for a network before the hub says it is needed.
        assert!(
            harness
                .query_by(|node| node.placeholder() == Some("WiFi password"))
                .is_none()
        );
    }

    #[test]
    fn a_failure_says_why_and_try_again_starts_over() {
        let mut sheet = opened();
        sheet.submitted = true;
        let state = adding_at(
            AddStep::Failed("The diffuser didn't join your broker.".to_owned()),
            Some(AddStep::WaitConnected),
        );
        let mut harness = harness(sheet, state);
        let _ = harness.get_by_label("The diffuser didn't join your broker.");
        harness.get_by_label("Try Again").click();
        harness.run_steps(2);
        let [UiCommand::Add(request)] = &harness.state().sent[..] else {
            panic!("one Add");
        };
        assert!(request.wifi.is_none());
    }

    #[test]
    fn try_again_after_a_network_was_given_asks_for_it_again() {
        // The wait failed: the advice is to check the WiFi password, so trying again
        // starts from the network, filled in, with the password to type again.
        let mut sheet = opened();
        sheet.submitted = true;
        let failed = AddStep::Failed("The diffuser didn't join your broker.".to_owned());
        let state = adding_with(failed, Some(AddStep::WaitConnected), Some("ExampleWiFi"));
        let mut harness = harness(sheet, state);
        harness.get_by_label("Try Again").click();
        harness.run_steps(2);
        assert!(harness.state().sent.is_empty(), "nothing sent yet");
        assert_eq!(harness.state().sheet.network, "ExampleWiFi");
        let password = harness.get_by(|node| node.placeholder() == Some("WiFi password"));
        password.focus();
        password.type_text("hunter2");
        harness.run_steps(2);
        harness.get_by_label("Add").click();
        harness.run_steps(2);
        let [UiCommand::Add(request)] = &harness.state().sent[..] else {
            panic!("one Add");
        };
        let (network, password) = request.wifi.as_ref().expect("the network");
        assert_eq!(
            (network.as_str(), password.expose()),
            ("ExampleWiFi", "hunter2")
        );
        assert!(harness.state().sheet.password.is_empty());
    }

    #[test]
    fn the_sheet_follows_only_its_own_diffusers_add() {
        // Another diffuser is being added: this one is still to start.
        let mut state = adding_at(AddStep::WaitConnected, Some(AddStep::WaitConnected));
        let other = state.adding.remove(BLE).unwrap();
        state.adding.insert(
            "ble-other".to_owned(),
            Adding {
                ble_id: "ble-other".to_owned(),
                ..other
            },
        );
        let mut harness = harness(opened(), state);
        harness.get_by_label("Add").click();
        harness.run_steps(2);
        assert_eq!(harness.state().sent.len(), 1);
    }

    #[test]
    fn when_it_is_done_done_closes_the_sheet() {
        let mut sheet = opened();
        sheet.submitted = true;
        let state = adding_at(
            AddStep::Done("TESTDEVICE01".to_owned()),
            Some(AddStep::Ping),
        );
        let mut harness = harness(sheet, state);
        assert!(harness.query_by_label("Cancel").is_none());
        let _ = harness.get_by_label("It's on your broker, and will answer pings shortly.");
        harness.get_by_label("Done").click();
        harness.run_steps(2);
        assert!(!harness.state().sheet.is_open());
        assert!(harness.state().sent.is_empty());
    }
}
