//! The settings sheet: where the broker is, who we are on it, and its password.
//!
//! The password is typed into a masked field, goes to the Keychain through the app,
//! and is not kept here: the sheet hands it over as a [`Secret`] and empties the field.

use eframe::egui::text_edit::TextEditState;
use eframe::egui::{self, Id, Key, RichText};
use scent_core::Secret;

use super::format::broker_text;
use super::red;
use crate::mqtt::{BrokerSettings, BrokerUrl};
use crate::state::BrokerState;

const WIDTH: f32 = 420.0;

/// What the person saved.
pub struct Submitted {
    pub settings: BrokerSettings,
    /// The password typed this time. `None` leaves the saved one as it is.
    pub password: Option<Secret>,
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
pub struct SettingsSheet {
    open: bool,
    url: String,
    username: String,
    route_id: String,
    /// What is typed in the masked field. Emptied when the sheet closes or saves.
    password: String,
    error: Option<String>,
}

impl SettingsSheet {
    fn password_id() -> Id {
        Id::new("settings_broker_password")
    }

    /// Opens the sheet with the current settings in it and the password field empty.
    pub fn open(&mut self, current: &BrokerSettings) {
        self.url.clone_from(&current.url);
        self.username.clone_from(&current.username);
        self.route_id.clone_from(&current.route_id);
        self.password = String::new();
        self.error = None;
        self.open = true;
    }

    /// Shuts the sheet after a save that worked.
    pub fn close(&mut self, ctx: &egui::Context) {
        self.forget_password(ctx);
        self.open = false;
    }

    /// Keeps the sheet open to show what went wrong with the save.
    pub fn fail(&mut self, error: String) {
        self.error = Some(error);
    }

    /// Empties the password field, and the copy of it the text box keeps for undoing.
    fn forget_password(&mut self, ctx: &egui::Context) {
        self.password = String::new();
        TextEditState::default().store(ctx, Self::password_id());
    }

    /// Shows the sheet when it is open. Returns the settings when Save was pressed and
    /// they are valid; the sheet stays open until the app says how saving went
    /// ([`close`](Self::close) or [`fail`](Self::fail)).
    pub fn show(&mut self, ctx: &egui::Context, broker: &BrokerState) -> Option<Submitted> {
        if !self.open {
            return None;
        }
        let mut save = false;
        let mut cancel = false;
        let modal = egui::Modal::new(Id::new("settings_sheet")).show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.heading("Broker settings");
            ui.add_space(6.0);
            let mut enter = false;
            egui::Grid::new("settings_fields")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Broker address");
                    enter |= field(ui, egui::TextEdit::singleline(&mut self.url));
                    ui.end_row();
                    ui.label("Username");
                    enter |= field(ui, egui::TextEdit::singleline(&mut self.username));
                    ui.end_row();
                    ui.label("Route ID");
                    enter |= field(ui, egui::TextEdit::singleline(&mut self.route_id));
                    ui.end_row();
                    ui.label("Password");
                    let password = egui::TextEdit::singleline(&mut self.password)
                        .id(Self::password_id())
                        .password(true)
                        .hint_text("Saved in the Keychain");
                    enter |= field(ui, password);
                    ui.end_row();
                });
            ui.add_space(4.0);
            ui.weak("Leave the password empty to keep the one saved in the Keychain.");
            ui.add_space(4.0);
            ui.label(broker_text(broker));
            if let Some(error) = &self.error {
                let color = red(ui);
                ui.label(RichText::new(error).color(color));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() || enter {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if cancel || modal.should_close() {
            self.close(ctx);
            return None;
        }
        if !save {
            return None;
        }
        match validate(&self.url, &self.username, &self.route_id) {
            Ok(settings) => {
                self.error = None;
                let typed = std::mem::take(&mut self.password);
                self.forget_password(ctx);
                let password = (!typed.is_empty()).then(|| Secret::new(typed));
                Some(Submitted { settings, password })
            }
            Err(error) => {
                self.error = Some(error);
                None
            }
        }
    }
}

/// A one-line text box that fills the column. True when Enter was pressed in it.
fn field(ui: &mut egui::Ui, edit: egui::TextEdit<'_>) -> bool {
    let response = ui.add(edit.desired_width(f32::INFINITY));
    response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn opening_the_sheet_loads_the_settings_and_empties_the_password() {
        let mut sheet = SettingsSheet {
            password: "left over".to_owned(),
            error: Some("old".to_owned()),
            ..SettingsSheet::default()
        };
        let settings = BrokerSettings {
            url: "wss://broker.example/mqtt".into(),
            username: "someone".into(),
            route_id: "route-1".into(),
        };
        sheet.open(&settings);
        assert!(sheet.open);
        assert_eq!(
            (&sheet.url[..], &sheet.username[..], &sheet.route_id[..]),
            ("wss://broker.example/mqtt", "someone", "route-1")
        );
        assert!(sheet.password.is_empty() && sheet.error.is_none());
    }
}
