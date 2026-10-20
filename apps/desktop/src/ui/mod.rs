//! The screens, as a Mac split view: the sidebar (the diffuser list) on the left under
//! the traffic lights, and on the right a toolbar, the open diffuser, and the log pane
//! along the bottom. Settings is a window of its own. Each file is one screen; the look
//! is in `theme` and the controls they share in `widgets`. This one lays them out and
//! holds what they ask of the app.
//!
//! The screens only read the state they are given and hand back what they want done
//! ([`Requests`]); the app sends the commands to the hub and saves the settings.

pub mod clock;
pub mod device_list;
pub mod device_panel;
pub mod format;
pub mod log_view;
pub mod settings;
pub mod theme;
pub mod widgets;

#[cfg(test)]
mod screen_tests;
#[cfg(test)]
mod snapshots;

use eframe::egui::{
    self, Align, Frame, Id, Key, KeyboardShortcut, Layout, Margin, RichText, Sense, ViewportCommand,
};

use self::device_list::DeviceList;
use self::device_panel::DevicePanel;
use self::format::broker_text;
use self::log_view::LogView;
use self::settings::{SettingsWindow, Submitted};
use self::theme::{
    GRID, MARGIN, Palette, SIDEBAR_TOP_INSET, SIDEBAR_WIDTH, TOOLBAR_HEIGHT, broker_color,
};
use self::widgets::{
    NoticeKind, gear_button, hairline, notice_with, plain_button, status_dot, toolbar_button,
};
use crate::hub::{NO_BROKER_PASSWORD, UiCommand};
use crate::mqtt::BrokerSettings;
use crate::state::{AppState, BrokerState, Event};

/// ⌘, opens Settings, as in every Mac app.
const SETTINGS_SHORTCUT: KeyboardShortcut =
    KeyboardShortcut::new(egui::Modifiers::COMMAND, Key::Comma);

/// The widest the open diffuser's boxes get, so that rows stay readable in a wide
/// window.
const CONTENT_MAX_WIDTH: f32 = 640.0;

/// What the screens ask the app to do after a frame.
#[derive(Default)]
pub struct Requests {
    pub commands: Vec<UiCommand>,
    /// The settings were saved. The app stores them and reconnects, then tells the
    /// Settings window how that went.
    pub settings: Option<Submitted>,
}

/// What the screens remember between frames. The hub never sees any of it.
pub struct Screens {
    /// The diffuser whose panel is open.
    selected: Option<String>,
    list: DeviceList,
    panel: DevicePanel,
    log: LogView,
    pub settings: SettingsWindow,
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
            settings: SettingsWindow::default(),
            notice,
        }
    }

    /// Shows a problem in a notice at the top, in place of any earlier one.
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
        let palette = Palette::of(ui);
        if ui.input_mut(|input| input.consume_shortcut(&SETTINGS_SHORTCUT)) {
            self.settings.open_or_raise(ui.ctx(), broker);
        }

        // The band along the top, under the traffic lights and the toolbar, moves the
        // window. First, so that the toolbar's controls take the clicks over it.
        let band = egui::Rect::from_min_size(
            ui.max_rect().min,
            egui::vec2(ui.max_rect().width(), TOOLBAR_HEIGHT),
        );
        drag_window(ui, band);

        // The sidebar next, so that it runs the height of the window, under the traffic
        // lights. The panes draw no separator lines of their own (egui leaves a gap for
        // them that nothing fills); the hairlines between them are drawn at the end.
        let sidebar = Frame::new().fill(palette.sidebar).inner_margin(Margin {
            left: GRID as i8,
            right: GRID as i8,
            top: SIDEBAR_TOP_INSET as i8,
            bottom: 0,
        });
        let sidebar = egui::Panel::left("sidebar")
            .default_size(SIDEBAR_WIDTH)
            .size_range(200.0..=360.0)
            .show_separator_line(false)
            .frame(sidebar)
            .show(ui, |ui| {
                let commands = self.list.show(ui, state, &mut self.selected, now);
                requests.commands.extend(commands);
            })
            .response
            .rect;

        let toolbar = Frame::new()
            .fill(palette.window)
            .inner_margin(Margin::symmetric(MARGIN as i8, 0));
        let toolbar = egui::Panel::top("toolbar")
            .exact_size(TOOLBAR_HEIGHT)
            .show_separator_line(false)
            .frame(toolbar)
            .show(ui, |ui| self.toolbar(ui, state, broker))
            .response
            .rect;

        let log = self.log.show(ui, state, self.selected.as_deref(), now);

        let detail = Frame::new()
            .fill(palette.window)
            .inner_margin(Margin::same(MARGIN as i8));
        egui::CentralPanel::default().frame(detail).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_max_width(CONTENT_MAX_WIDTH.min(ui.available_width()));
                    self.notices(ui, state, broker);
                    match self.selected.as_ref().and_then(|id| state.devices.get(id)) {
                        Some(device) => {
                            let commands = self.panel.show(ui, state, device, now);
                            requests.commands.extend(commands);
                        }
                        None => {
                            ui.add_space(4.0 * GRID);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new(
                                        "Diffusers appear on the left once they are added or heard on the broker.",
                                    )
                                    .color(palette.secondary_label),
                                );
                            });
                        }
                    }
                });
        });

        // The hairlines between the panes, over their edges.
        let painter = ui.painter();
        let separator = egui::Stroke::new(hairline(ui), palette.separator);
        painter.vline(sidebar.right(), sidebar.y_range(), separator);
        painter.hline(toolbar.x_range(), toolbar.bottom(), separator);
        if let Some(log) = log {
            painter.hline(log.x_range(), log.top(), separator);
        }

        requests.settings = self.settings.show(ui.ctx(), &state.broker);
        requests
    }

    /// The toolbar along the top of the detail pane: the broker's state on the left, and
    /// Show Log or Hide Log and the gear for Settings on the right.
    fn toolbar(&mut self, ui: &mut egui::Ui, state: &AppState, broker: &BrokerSettings) {
        let palette = Palette::of(ui);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if gear_button(ui, "Settings")
                .on_hover_text("Settings (⌘,)")
                .clicked()
            {
                self.settings.open_or_raise(ui.ctx(), broker);
            }
            let log = if self.log.is_open() {
                "Hide Log"
            } else {
                "Show Log"
            };
            if toolbar_button(ui, log).clicked() {
                self.log.toggle();
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let text = broker_text(&state.broker);
                status_dot(ui, broker_color(&palette, &state.broker), &text);
                ui.add(
                    egui::Label::new(RichText::new(text).color(palette.secondary_label)).truncate(),
                );
            });
        });
    }

    /// The notices at the top of the detail pane: a refused login, no password saved,
    /// and a problem with the saved file.
    fn notices(&mut self, ui: &mut egui::Ui, state: &AppState, broker: &BrokerSettings) {
        let broker_notice = match &state.broker {
            BrokerState::LoginRefused(_) => Some((
                NoticeKind::Error,
                "Broker login refused. Check the username and password in Settings, then save to try again.",
            )),
            BrokerState::Disconnected(why) if why == NO_BROKER_PASSWORD => {
                Some((NoticeKind::Warning, NO_BROKER_PASSWORD))
            }
            _ => None,
        };
        if let Some((kind, text)) = broker_notice {
            let (open, _) = notice_with(ui, kind, text, |ui| {
                plain_button(ui, "Open Settings").clicked()
            });
            if open {
                self.settings.open_or_raise(ui.ctx(), broker);
            }
            ui.add_space(GRID);
        }
        if let Some(text) = &self.notice {
            let (dismissed, _) = notice_with(ui, NoticeKind::Warning, text, |ui| {
                plain_button(ui, "Dismiss").clicked()
            });
            if dismissed {
                self.notice = None;
            }
            ui.add_space(GRID);
        }
    }
}

/// Lets the person move the window by dragging `rect`, and zoom it with a double-click,
/// as a title bar does: the window's content runs under the title bar, so the title bar
/// itself can't. Add it before the controls in `rect`, which then take the clicks.
fn drag_window(ui: &mut egui::Ui, rect: egui::Rect) {
    // Clicks and drags, but no stop for the Tab key: there is nothing here to focus.
    let response = ui.interact(rect, Id::new("title_bar"), Sense::CLICK | Sense::DRAG);
    if response.double_clicked() {
        let zoomed = ui.input(|input| input.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(ViewportCommand::Maximized(!zoomed));
    } else if response.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use eframe::egui::{Context, FontDefinitions, Pos2, RawInput, Rect, vec2};
    use scent_core::{AlwaysOn, DeviceInfo, Direction, Level, Profile, ScheduleFlag, Via};

    use super::*;
    use crate::state::{DeviceView, Field, LogEntry, Reading, ScanHit, Sent};

    pub(super) const ONE: &str = "TESTDEVICE01";
    pub(super) const TWO: &str = "TESTDEVICE02";
    pub(super) const NOW: i64 = 1_792_510_200;

    /// Two diffusers, one with readings and a schedule sent, two scan hits and a log.
    pub(super) fn state() -> AppState {
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
        theme::install(&ctx, Path::new("/nonexistent"));
        // No fonts to load, to save time: the families are there with nothing in them.
        let mut fonts = FontDefinitions::empty();
        fonts.families.insert(theme::semibold(), vec![]);
        ctx.set_fonts(fonts);
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
        // The first diffuser is open, and the log and the Settings window draw too.
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
