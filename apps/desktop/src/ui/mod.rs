//! The screens: a status bar along the top, the diffuser list on the left, the open
//! diffuser in the middle, the log along the bottom, and the settings sheet over all of
//! it. Each file is one screen. This one holds what they share: the colors, the frame
//! around them, and what they ask of the app.
//!
//! The screens only read the state they are given and hand back what they want done
//! ([`Requests`]); the app sends the commands to the hub and saves the settings.

pub mod clock;
pub mod device_list;
pub mod device_panel;
pub mod format;
pub mod log_view;
pub mod settings;

use eframe::egui::{self, Align, Color32, Layout, Sense, vec2};
use scent_core::Online;

use self::device_list::DeviceList;
use self::device_panel::DevicePanel;
use self::format::broker_text;
use self::log_view::LogView;
use self::settings::{SettingsSheet, Submitted};
use crate::hub::UiCommand;
use crate::mqtt::BrokerSettings;
use crate::state::{AppState, BrokerState, Event};

/// Width of the diffuser list when the window first opens.
const SIDE_PANEL_WIDTH: f32 = 260.0;

/// Green for answering, amber for waiting or pending, red for a refused login.
/// Dark mode gets lighter shades of the same colors.
pub fn green(ui: &egui::Ui) -> Color32 {
    pick(ui, (0x2e, 0x8b, 0x57), (0x5f, 0xc2, 0x8a))
}

pub fn amber(ui: &egui::Ui) -> Color32 {
    pick(ui, (0xb7, 0x79, 0x1f), (0xe0, 0xa9, 0x4e))
}

pub fn red(ui: &egui::Ui) -> Color32 {
    pick(ui, (0xb4, 0x53, 0x5a), (0xe0, 0x8a, 0x90))
}

fn pick(ui: &egui::Ui, light: (u8, u8, u8), dark: (u8, u8, u8)) -> Color32 {
    let (r, g, b) = if ui.visuals().dark_mode { dark } else { light };
    Color32::from_rgb(r, g, b)
}

/// A faint wash behind what we sent.
pub fn subtle_fill(ui: &egui::Ui) -> Color32 {
    ui.visuals().faint_bg_color
}

/// The badge color for whether a diffuser answers: green for answering, amber while
/// waiting, grey for the rest (not pinged yet, or silent).
pub fn online_color(ui: &egui::Ui, online: Online) -> Color32 {
    match online {
        Online::Online => green(ui),
        Online::Waiting => amber(ui),
        Online::Offline | Online::Unknown => ui.visuals().weak_text_color(),
    }
}

/// A small filled circle in `color`, the size of a line of text.
pub fn dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(
        vec2(10.0, ui.text_style_height(&egui::TextStyle::Body)),
        Sense::hover(),
    );
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// A box with a tinted background for a message that needs the person's eye.
pub fn banner<R>(
    ui: &mut egui::Ui,
    color: Color32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.2))
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui)
        })
        .inner
}

/// What the screens ask the app to do after a frame.
#[derive(Default)]
pub struct Requests {
    pub commands: Vec<UiCommand>,
    /// The settings sheet was saved. The app stores them and reconnects, then tells the
    /// sheet how that went.
    pub settings: Option<Submitted>,
}

/// What the screens remember between frames. The hub never sees any of it.
pub struct Screens {
    /// The diffuser whose panel is open.
    selected: Option<String>,
    list: DeviceList,
    panel: DevicePanel,
    log: LogView,
    pub settings: SettingsSheet,
    /// A problem with the saved file, until the person dismisses it.
    notice: Option<String>,
}

impl Screens {
    pub fn new(notice: Option<String>) -> Screens {
        Screens {
            selected: None,
            list: DeviceList::default(),
            panel: DevicePanel::default(),
            log: LogView::default(),
            settings: SettingsSheet::default(),
            notice,
        }
    }

    /// Shows a problem in the banner at the top, in place of any earlier one.
    pub fn set_notice(&mut self, notice: String) {
        self.notice = Some(notice);
    }

    /// Tells the screens about an event before it is folded into the state, for what
    /// only they care about (a scan has ended; a command has got through).
    pub fn on_event(&mut self, event: &Event) {
        self.list.on_event(event);
        self.panel.on_event(event);
    }

    /// Opens the first diffuser when none is open.
    fn keep_selection(&mut self, state: &AppState) {
        let open = self
            .selected
            .as_ref()
            .is_some_and(|id| state.devices.contains_key(id));
        if !open {
            self.selected = state.devices.keys().next().cloned();
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        broker: &BrokerSettings,
        now: i64,
    ) -> Requests {
        let mut requests = Requests::default();
        self.keep_selection(state);

        egui::Panel::top("status").show(ui, |ui| self.status_bar(ui, state, broker));
        // Before the side panel, so that the log runs the width of the window.
        self.log.show(ui, state, self.selected.as_deref(), now);
        egui::Panel::left("diffusers")
            .default_size(SIDE_PANEL_WIDTH)
            .show(ui, |ui| {
                let commands = self.list.show(ui, state, &mut self.selected, now);
                requests.commands.extend(commands);
            });
        egui::CentralPanel::default().show(ui, |ui| {
            match self.selected.as_ref().and_then(|id| state.devices.get(id)) {
                Some(device) => {
                    let commands = self.panel.show(ui, state, device, now);
                    requests.commands.extend(commands);
                }
                None => {
                    ui.weak(
                        "Diffusers appear on the left once they are added or heard on the broker.",
                    );
                }
            }
        });

        requests.settings = self.settings.show(ui.ctx(), &state.broker);
        requests
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, state: &AppState, broker: &BrokerSettings) {
        ui.horizontal(|ui| {
            let color = match &state.broker {
                BrokerState::Connected => green(ui),
                BrokerState::Connecting | BrokerState::Disconnected(_) => amber(ui),
                BrokerState::LoginRefused(_) => red(ui),
            };
            dot(ui, color);
            ui.label(broker_text(&state.broker));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Settings…").clicked() {
                    self.settings.open(broker);
                }
            });
        });
        if matches!(state.broker, BrokerState::LoginRefused(_)) {
            let color = red(ui);
            banner(ui, color, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        "Broker login refused. Check the username and password in Settings, then save to try again.",
                    );
                    if ui.button("Open settings").clicked() {
                        self.settings.open(broker);
                    }
                });
            });
        }
        if let Some(notice) = &self.notice {
            let color = amber(ui);
            let dismissed = banner(ui, color, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(notice);
                    ui.button("Dismiss").clicked()
                })
                .inner
            });
            if dismissed {
                self.notice = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Context, FontDefinitions, Pos2, RawInput, Rect};
    use scent_core::{AlwaysOn, DeviceInfo, Direction, Level, Profile, ScheduleFlag, Via};

    use super::*;
    use crate::state::{DeviceView, Field, LogEntry, Reading, ScanHit, Sent};

    const ONE: &str = "TESTDEVICE01";
    const TWO: &str = "TESTDEVICE02";
    const NOW: i64 = 1_792_510_200;

    fn state() -> AppState {
        let mut state = AppState::default();
        let mut one = DeviceView::new(ONE);
        one.label = "Bedroom".to_owned();
        one.ble_id = Some("ble-1".to_owned());
        one.info = Some(DeviceInfo {
            manufacturer_id: "TEST".into(),
            model_name: "Diffuser Air 2".into(),
            variant_name: "TEST".into(),
            device_id: ONE.into(),
            firmware: "2.30".into(),
            wifi_configured: true,
            wifi_status: 3,
        });
        one.heard_wifi = Some(NOW - 5);
        for (field, value, age, via) in [
            (Field::Power, 1, 30, Via::Wifi),
            (Field::Intensity, 2, 400, Via::Bluetooth),
            (Field::Charging, 2, 30, Via::Wifi),
            (Field::Battery, 200, 30, Via::Wifi),
            (Field::Rssi, -66, 30, Via::Wifi),
        ] {
            let at = NOW - age;
            one.readings.insert(field, Reading { value, at, via });
        }
        one.sent = Some(Sent {
            power: Some(false),
            intensity: Some(3),
            at: NOW - 2,
        });
        one.sent_profile = Some(Profile {
            always_on: AlwaysOn {
                enable: true,
                level: Level::new(1).unwrap(),
            },
            entries: vec![],
            timezone_hours: -5.0,
            flag: ScheduleFlag::Replace,
        });
        state.devices.insert(ONE.to_owned(), one);
        state.devices.insert(TWO.to_owned(), DeviceView::new(TWO));
        state.scan = vec![
            ScanHit {
                ble_id: "ble-1".into(),
                name: "Known".into(),
                rssi: Some(-50),
            },
            ScanHit {
                ble_id: "ble-9".into(),
                name: "New one".into(),
                rssi: None,
            },
        ];
        for (direction, via, action, text) in [
            (
                Direction::ToDevice,
                Some(Via::Wifi),
                "TURN_ON",
                r#"{"action":"TURN_ON"}"#,
            ),
            (
                Direction::ToDevice,
                Some(Via::Wifi),
                "TURN_ON",
                r#"{"action":"TURN_ON"}"#,
            ),
            (
                Direction::FromDevice,
                Some(Via::Wifi),
                "update_status",
                "{}",
            ),
            (Direction::ToDevice, None, "Bluetooth TURN_ON", "No answer."),
        ] {
            state.log.push_back(LogEntry {
                at: NOW,
                device_id: Some(ONE.to_owned()),
                via,
                direction,
                action: action.to_owned(),
                text: text.to_owned(),
                sent: direction == Direction::ToDevice && via.is_some(),
            });
        }
        state
    }

    /// Draws the screens twice on an egui context that has no window.
    fn draw(screens: &mut Screens, state: &AppState) -> Requests {
        let ctx = Context::default();
        ctx.set_fonts(FontDefinitions::empty());
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 720.0))),
            ..RawInput::default()
        };
        let mut requests = Requests::default();
        for _ in 0..2 {
            let output = ctx.run_ui(input.clone(), |ui| {
                requests = screens.show(ui, state, &BrokerSettings::default(), NOW);
            });
            output.drop_without_applying_deltas();
        }
        requests
    }

    #[test]
    fn the_screens_draw_without_a_window_and_ask_for_nothing_on_their_own() {
        let mut state = state();
        let mut screens = Screens::new(Some("The file was damaged.".to_owned()));
        for broker in [
            BrokerState::Connecting,
            BrokerState::Connected,
            BrokerState::LoginRefused("Not authorized.".to_owned()),
            BrokerState::Disconnected("No broker password saved.".to_owned()),
        ] {
            state.broker = broker;
            let requests = draw(&mut screens, &state);
            assert!(requests.commands.is_empty() && requests.settings.is_none());
        }
        // The first diffuser is open, and the log and the sheet draw too.
        assert_eq!(screens.selected.as_deref(), Some(ONE));
        screens.settings.open(&BrokerSettings::default());
        let requests = draw(&mut screens, &state);
        assert!(requests.settings.is_none());
        // With nothing known at all.
        let requests = draw(&mut Screens::new(None), &AppState::default());
        assert!(requests.commands.is_empty());
    }

    #[test]
    fn the_open_diffuser_stays_open_and_a_missing_one_is_replaced() {
        let state = state();
        let mut screens = Screens::new(None);
        screens.selected = Some(TWO.to_owned());
        let _ = draw(&mut screens, &state);
        assert_eq!(screens.selected.as_deref(), Some(TWO));
        screens.selected = Some("TESTDEVICE03".to_owned());
        let _ = draw(&mut screens, &state);
        assert_eq!(screens.selected.as_deref(), Some(ONE));
    }
}
