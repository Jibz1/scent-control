//! The Settings window: where the broker is, who we are on it, and its password. It is a
//! window of its own, opened with ⌘, or the toolbar's gear button, with the broker
//! fields in a grouped box and Save as the default button.
//!
//! The password is typed into a masked field, goes to the Keychain through the app,
//! and is not kept here: the window hands it over as a [`Secret`] and empties the field.

use eframe::egui::text_edit::TextEditState;
use eframe::egui::{
    self, Id, Key, KeyboardShortcut, Modifiers, Pos2, RichText, ViewportBuilder, ViewportClass,
    ViewportCommand, ViewportId, pos2,
};
use scent_core::Secret;

use super::format::broker_text;
use super::theme::{GRID, MARGIN, Palette, broker_color};
use super::widgets::{default_button, footnote, grouped, plain_button, row, status_dot};
use crate::mqtt::{BrokerSettings, BrokerUrl};
use crate::state::BrokerState;

/// The window's width, and its height before it has measured what it holds.
pub const WIDTH: f32 = 560.0;
const FIRST_HEIGHT: f32 = 360.0;

/// The width of the text fields.
const FIELD_WIDTH: f32 = 340.0;

/// How far below the top of the main window the Settings window opens.
const BELOW_TOP: f32 = 90.0;

/// What the person saved.
pub struct Submitted {
    pub settings: BrokerSettings,
    /// The password typed this time. `None` leaves the saved one as it is.
    pub password: Option<Secret>,
}

/// What the person did in the window this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Save,
    /// Cancel, Escape, ⌘W or the window's close button.
    Cancel,
}

/// The settings as typed, or why they can't be used. Trims the fields.
fn validate(url: &str, username: &str, route_id: &str) -> Result<BrokerSettings, String> {
    let (url, username, route_id) = (url.trim(), username.trim(), route_id.trim());
    BrokerUrl::parse(url)?;
    if username.is_empty() {
        return Err("Enter the broker username.".to_owned());
    }
    if route_id.is_empty() {
        return Err("Enter the route ID.".to_owned());
    }
    Ok(BrokerSettings {
        url: url.to_owned(),
        username: username.to_owned(),
        route_id: route_id.to_owned(),
    })
}

#[derive(Default)]
pub struct SettingsWindow {
    open: bool,
    url: String,
    username: String,
    route_id: String,
    /// What is typed in the masked field. Emptied when the window closes or saves.
    password: String,
    error: Option<String>,
    /// Where the window opens: over the main window, worked out when it is first shown.
    position: Option<Pos2>,
    /// How tall the window was drawn last time, so that it fits what it holds.
    height: Option<f32>,
}

impl SettingsWindow {
    fn password_id() -> Id {
        Id::new("settings_broker_password")
    }

    fn viewport_id() -> ViewportId {
        ViewportId::from_hash_of("settings_window")
    }

    /// Opens the window with the current settings in it and the password field empty.
    pub fn open(&mut self, current: &BrokerSettings) {
        self.url.clone_from(&current.url);
        self.username.clone_from(&current.username);
        self.route_id.clone_from(&current.route_id);
        self.password = String::new();
        self.error = None;
        self.open = true;
    }

    /// Opens the window, or brings it to the front if it is open already, with what is
    /// typed in it kept.
    pub fn open_or_raise(&mut self, ctx: &egui::Context, current: &BrokerSettings) {
        if self.open {
            ctx.send_viewport_cmd_to(Self::viewport_id(), ViewportCommand::Focus);
        } else {
            self.open(current);
        }
    }

    /// Shuts the window after a save that worked, or when the person cancels.
    pub fn close(&mut self, ctx: &egui::Context) {
        self.forget_password(ctx);
        self.open = false;
        self.position = None;
        self.height = None;
    }

    /// Keeps the window open to show what went wrong with the save.
    pub fn fail(&mut self, error: String) {
        self.error = Some(error);
    }

    /// Empties the password field, and the copy of it the text box keeps for undoing.
    fn forget_password(&mut self, ctx: &egui::Context) {
        self.password = String::new();
        TextEditState::default().store(ctx, Self::password_id());
    }

    /// Shows the window when it is open. Returns the settings when Save was pressed and
    /// they are valid; the window stays open until the app says how saving went
    /// ([`close`](Self::close) or [`fail`](Self::fail)).
    ///
    /// The window is a viewport of its own. Where egui can't open one (a test's
    /// context), it is drawn inside the main window instead.
    pub fn show(&mut self, ctx: &egui::Context, broker: &BrokerState) -> Option<Submitted> {
        if !self.open {
            return None;
        }
        if self.position.is_none() {
            self.position = ctx
                .input(|input| input.viewport().outer_rect)
                .map(|main| pos2(main.center().x - WIDTH / 2.0, main.top() + BELOW_TOP));
        }
        let mut builder = ViewportBuilder::default()
            .with_title("Settings")
            .with_inner_size([WIDTH, self.height.unwrap_or(FIRST_HEIGHT)])
            .with_resizable(false)
            .with_minimize_button(false)
            .with_maximize_button(false);
        if let Some(position) = self.position {
            builder = builder.with_position(position);
        }
        let action = ctx.show_viewport_immediate(Self::viewport_id(), builder, |ui, class| {
            let closing = ui.input(|input| input.viewport().close_requested());
            let action = if class == ViewportClass::EmbeddedWindow {
                self.contents(ui, broker)
            } else {
                self.window(ui, broker)
            };
            if closing {
                Some(Action::Cancel)
            } else {
                action
            }
        });
        match action? {
            Action::Cancel => {
                self.close(ctx);
                None
            }
            Action::Save => match self.submit(ctx) {
                Ok(submitted) => {
                    self.error = None;
                    Some(submitted)
                }
                Err(error) => {
                    self.error = Some(error);
                    None
                }
            },
        }
    }

    /// The whole of the window: its background and margins, and what it holds. Notes
    /// how tall that is, for the window to fit it from the next frame.
    pub fn window(&mut self, ui: &mut egui::Ui, broker: &BrokerState) -> Option<Action> {
        let frame = egui::Frame::new()
            .fill(Palette::of(ui).window)
            .inner_margin(MARGIN);
        egui::CentralPanel::default()
            .frame(frame)
            .show(ui, |ui| {
                let action = self.contents(ui, broker);
                self.height = Some((ui.min_rect().height() + 2.0 * MARGIN).ceil());
                action
            })
            .inner
    }

    /// How tall the window wants to be, once it has been drawn.
    #[cfg(test)]
    pub fn fitted_height(&self) -> Option<f32> {
        self.height
    }

    #[cfg(test)]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// What the window holds: the broker fields in a grouped box, the connection's
    /// state, what went wrong, and Cancel and Save. Returns what the person did.
    pub fn contents(&mut self, ui: &mut egui::Ui, broker: &BrokerState) -> Option<Action> {
        let palette = Palette::of(ui);
        let mut enter = false;
        grouped(ui, "Broker", |ui| {
            row(ui, "Broker address", |ui| {
                enter |= field(ui, egui::TextEdit::singleline(&mut self.url));
            });
            row(ui, "Username", |ui| {
                enter |= field(ui, egui::TextEdit::singleline(&mut self.username));
            });
            row(ui, "Route ID", |ui| {
                enter |= field(ui, egui::TextEdit::singleline(&mut self.route_id));
            });
            row(ui, "Password", |ui| {
                let password = egui::TextEdit::singleline(&mut self.password)
                    .id(Self::password_id())
                    .password(true)
                    .hint_text("Saved in the Keychain");
                enter |= field(ui, password);
            });
        });
        footnote(
            ui,
            "Leave the password empty to keep the one saved in the Keychain.",
        );
        ui.add_space(GRID);
        ui.horizontal_wrapped(|ui| {
            let text = broker_text(broker);
            status_dot(ui, broker_color(&palette, broker), &text);
            ui.label(RichText::new(text).color(palette.label));
        });
        if let Some(error) = &self.error {
            ui.add_space(GRID / 2.0);
            ui.label(RichText::new(error).color(palette.red));
        }
        ui.add_space(2.0 * GRID);
        let (mut save, mut cancel) = (enter, false);
        // Cancel and Save at the right, Save last.
        egui::Sides::new().show(
            ui,
            |_| {},
            |ui| {
                save |= default_button(ui, "Save").clicked();
                cancel |= plain_button(ui, "Cancel").clicked();
            },
        );
        // Escape and ⌘W cancel; Return saves when no field or button has the focus (in a
        // field it saves already; on a button it presses that button).
        let close_window = KeyboardShortcut::new(Modifiers::COMMAND, Key::W);
        cancel |= ui.input_mut(|input| {
            input.key_pressed(Key::Escape) || input.consume_shortcut(&close_window)
        });
        let nothing_focused = ui.memory(|memory| memory.focused().is_none());
        save |= nothing_focused && ui.input(|input| input.key_pressed(Key::Enter));
        if cancel {
            Some(Action::Cancel)
        } else if save {
            Some(Action::Save)
        } else {
            None
        }
    }

    /// What Save hands over: the settings as typed and the password typed this time, if
    /// any. The password moves out of the field into the [`Secret`] and the field and
    /// the text box's undo copy are emptied. If the settings can't be used nothing is
    /// handed over and the password stays, so that it needn't be typed again.
    fn submit(&mut self, ctx: &egui::Context) -> Result<Submitted, String> {
        let settings = validate(&self.url, &self.username, &self.route_id)?;
        let typed = std::mem::take(&mut self.password);
        self.forget_password(ctx);
        let password = (!typed.is_empty()).then(|| Secret::new(typed));
        Ok(Submitted { settings, password })
    }
}

/// A one-line text box of the fields' width. True when Enter was pressed in it.
fn field(ui: &mut egui::Ui, edit: egui::TextEdit<'_>) -> bool {
    let response = ui.add(edit.desired_width(FIELD_WIDTH));
    response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use eframe::egui::{ViewportEvent, ViewportId};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

    use super::super::theme;
    use super::*;

    /// The window with what Save handed over, for a harness to drive. egui draws the
    /// window inside the harness's own, as it does wherever it can't open one.
    struct Rig {
        window: SettingsWindow,
        saved: Option<Submitted>,
    }

    /// The window open with the default settings and "hunter2" typed in the password
    /// field, so that the text box's undo copy holds it too.
    fn typed_in() -> Harness<'static, Rig> {
        let mut window = SettingsWindow::default();
        window.open(&BrokerSettings::default());
        let mut frames = 0;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(WIDTH, 480.0))
            .with_os(egui::os::OperatingSystem::Mac)
            .build_ui_state(
                move |ui, rig: &mut Rig| {
                    frames += 1;
                    if frames == 1 {
                        theme::install(ui.ctx(), Path::new("/nonexistent"));
                        ui.ctx().request_repaint();
                        return;
                    }
                    if let Some(saved) = rig.window.show(ui.ctx(), &BrokerState::Connected) {
                        rig.saved = Some(saved);
                    }
                },
                Rig {
                    window,
                    saved: None,
                },
            );
        harness.run_steps(2);
        let field = harness.get_by(|node| node.placeholder() == Some("Saved in the Keychain"));
        field.focus();
        field.type_text("hunter2");
        harness.run_steps(2);
        assert_eq!(harness.state().window.password, "hunter2");
        assert!(undo_kept(&harness));
        harness
    }

    /// Whether the password box keeps something to undo to, which would hold what was
    /// typed: a change on its way to being an undo point, or an undo point.
    fn undo_kept(harness: &Harness<'_, Rig>) -> bool {
        let state = TextEditState::load(&harness.ctx, SettingsWindow::password_id());
        let typed = (egui::text::CCursorRange::default(), "hunter2".to_owned());
        state.is_some_and(|state| {
            let undoer = state.undoer();
            undoer.is_in_flux() || undoer.has_undo(&typed)
        })
    }

    /// The window is shut and holds the password nowhere.
    fn forgotten(harness: &Harness<'_, Rig>) {
        let rig = harness.state();
        assert!(!rig.window.open);
        assert!(rig.window.password.is_empty());
        assert!(!undo_kept(harness));
        assert!(rig.saved.is_none());
    }

    #[test]
    fn cancel_forgets_the_password() {
        let mut harness = typed_in();
        harness.get_by_label("Cancel").click();
        harness.run_steps(2);
        forgotten(&harness);
    }

    #[test]
    fn escape_forgets_the_password() {
        let mut harness = typed_in();
        harness.key_press(Key::Escape);
        harness.run_steps(2);
        forgotten(&harness);
    }

    #[test]
    fn command_w_forgets_the_password() {
        let mut harness = typed_in();
        harness.key_press_modifiers(Modifiers::COMMAND, Key::W);
        harness.run_steps(2);
        forgotten(&harness);
    }

    #[test]
    fn the_close_button_forgets_the_password() {
        let mut harness = typed_in();
        // What the window's close button sends. egui draws the window inside the
        // harness's own here, so the request comes to that one.
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
    fn return_with_nothing_focused_saves() {
        let mut window = SettingsWindow::default();
        window.open(&BrokerSettings::default());
        let mut frames = 0;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(WIDTH, 480.0))
            .build_ui_state(
                move |ui, rig: &mut Rig| {
                    frames += 1;
                    if frames == 1 {
                        theme::install(ui.ctx(), Path::new("/nonexistent"));
                        ui.ctx().request_repaint();
                        return;
                    }
                    if let Some(saved) = rig.window.show(ui.ctx(), &BrokerState::Connected) {
                        rig.saved = Some(saved);
                    }
                },
                Rig {
                    window,
                    saved: None,
                },
            );
        harness.run_steps(2);
        harness.key_press(Key::Enter);
        harness.run_steps(2);
        let saved = harness.state().saved.as_ref().expect("Return saves");
        assert_eq!(saved.settings, BrokerSettings::default());
        assert!(saved.password.is_none());
    }

    #[test]
    fn good_settings_are_trimmed_and_kept() {
        let defaults = BrokerSettings::default();
        assert_eq!(
            validate(
                &format!(" {} ", defaults.url),
                " scent-test ",
                " route-test-1 "
            ),
            Ok(defaults)
        );
    }

    #[test]
    fn a_bad_broker_address_is_explained() {
        let error = validate("ws://broker.example/mqtt", "someone", "route-1").unwrap_err();
        assert!(error.contains("broker address"), "{error}");
        assert!(error.contains("wss://"), "{error}");
        assert!(validate("", "someone", "route-1").is_err());
        assert!(
            validate("wss://bad host/mqtt", "someone", "route-1")
                .unwrap_err()
                .contains("broker address")
        );
    }

    #[test]
    fn a_username_and_a_route_id_are_needed() {
        let url = "wss://broker.example/mqtt";
        assert_eq!(
            validate(url, "  ", "route-1").unwrap_err(),
            "Enter the broker username."
        );
        assert_eq!(
            validate(url, "someone", "").unwrap_err(),
            "Enter the route ID."
        );
    }

    fn open_window() -> SettingsWindow {
        let mut window = SettingsWindow::default();
        window.open(&BrokerSettings::default());
        window
    }

    #[test]
    fn saving_hands_over_the_password_and_empties_the_field() {
        let ctx = egui::Context::default();
        let mut window = open_window();
        window.password = "hunter2".to_owned();
        let submitted = window.submit(&ctx).unwrap();
        assert_eq!(submitted.settings, BrokerSettings::default());
        assert_eq!(submitted.password.unwrap().expose(), "hunter2");
        // Not in the window any more, in any field.
        assert!(window.password.is_empty());
        let everywhere = format!(
            "{} {} {} {:?}",
            window.url, window.username, window.route_id, window.error
        );
        assert!(!everywhere.contains("hunter2"), "{everywhere}");
    }

    #[test]
    fn saving_with_nothing_typed_leaves_the_saved_password_alone() {
        let ctx = egui::Context::default();
        let mut window = open_window();
        let submitted = window.submit(&ctx).unwrap();
        assert!(submitted.password.is_none());
        assert!(window.password.is_empty());
    }

    #[test]
    fn settings_that_do_not_validate_say_why_and_keep_what_was_typed() {
        let ctx = egui::Context::default();
        let mut window = open_window();
        window.url = "ws://broker.example/mqtt".to_owned();
        window.password = "hunter2".to_owned();
        let error = window.submit(&ctx).err().expect("not valid");
        assert!(error.contains("broker address"), "{error}");
        // Nothing was saved, so the password does not need typing again.
        assert_eq!(window.password, "hunter2");
    }

    #[test]
    fn opening_the_window_loads_the_settings_and_empties_the_password() {
        let mut window = SettingsWindow {
            password: "left over".to_owned(),
            error: Some("old".to_owned()),
            ..SettingsWindow::default()
        };
        let settings = BrokerSettings {
            url: "wss://broker.example/mqtt".into(),
            username: "someone".into(),
            route_id: "route-1".into(),
        };
        window.open(&settings);
        assert!(window.open);
        assert_eq!(
            (&window.url[..], &window.username[..], &window.route_id[..]),
            ("wss://broker.example/mqtt", "someone", "route-1")
        );
        assert!(window.password.is_empty() && window.error.is_none());
    }
}
