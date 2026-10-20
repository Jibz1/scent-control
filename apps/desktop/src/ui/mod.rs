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

use eframe::egui::{
    self, Align, Frame, Id, Key, KeyboardShortcut, Layout, Margin, RichText, Sense, ViewportCommand,
};

use self::device_list::DeviceList;
use self::device_panel::DevicePanel;
use self::format::broker_text;
use self::log_view::LogView;
use self::settings::{SettingsWindow, Submitted};
use self::theme::{GRID, MARGIN, Palette, SIDEBAR_WIDTH, TOOLBAR_HEIGHT, broker_color};
use self::widgets::{
    NoticeKind, gear_button, notice_with, plain_button, status_dot, toolbar_button,
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
        // lights.
        let sidebar = Frame::new().fill(palette.sidebar).inner_margin(Margin {
            left: 10,
            right: 10,
            top: TOOLBAR_HEIGHT as i8,
            bottom: 0,
        });
        egui::Panel::left("sidebar")
            .default_size(SIDEBAR_WIDTH)
            .size_range(200.0..=360.0)
            .frame(sidebar)
            .show(ui, |ui| {
                let commands = self.list.show(ui, state, &mut self.selected, now);
                requests.commands.extend(commands);
            });

        let toolbar = Frame::new()
            .fill(palette.window)
            .inner_margin(Margin::symmetric(MARGIN as i8, 0));
        egui::Panel::top("toolbar")
            .exact_size(TOOLBAR_HEIGHT)
            .frame(toolbar)
            .show(ui, |ui| self.toolbar(ui, state, broker));

        self.log.show(ui, state, self.selected.as_deref(), now);

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
                ui.spacing_mut().item_spacing.x = 6.0;
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
    let response = ui.interact(rect, Id::new("title_bar"), Sense::click_and_drag());
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

    /// The screens with what they asked for, for a test harness to drive.
    struct Rig {
        screens: Screens,
        state: AppState,
        commands: Vec<UiCommand>,
    }

    /// A harness that draws the screens in egui's fonts, without a hub. The first frame
    /// installs the theme, which is in use from the next.
    fn harness(state: AppState) -> egui_kittest::Harness<'static, Rig> {
        let mut frames = 0;
        let rig = Rig {
            screens: Screens::new(None),
            state,
            commands: Vec::new(),
        };
        egui_kittest::Harness::builder()
            .with_size(theme::WINDOW_SIZE)
            .with_os(egui::os::OperatingSystem::Mac)
            .build_ui_state(
                move |ui, rig: &mut Rig| {
                    frames += 1;
                    if frames == 1 {
                        theme::install(ui.ctx(), Path::new("/nonexistent"));
                        ui.ctx().request_repaint();
                        return;
                    }
                    let requests =
                        rig.screens
                            .show(ui, &rig.state, &BrokerSettings::default(), NOW);
                    rig.commands.extend(requests.commands);
                },
                rig,
            )
    }

    #[test]
    fn the_switch_sends_the_power_and_shows_it_while_it_goes() {
        use egui::accesskit::{Role, Toggled};
        use egui_kittest::kittest::{NodeT as _, Queryable as _};
        let mut state = state();
        state.devices.get_mut(ONE).unwrap().sent = None;
        let mut harness = harness(state);
        harness.run_steps(2);
        // On, as reported.
        let power = harness.get_by_role_and_label(Role::Switch, "Power");
        assert_eq!(power.accesskit_node().toggled(), Some(Toggled::True));
        harness.get_by_role_and_label(Role::Switch, "Power").click();
        harness.run_steps(2);
        assert!(
            matches!(
                harness.state().commands[..],
                [UiCommand::Send {
                    cmd: scent_core::Command::TurnOn { on: false },
                    pref: crate::route::TransportPref::Auto,
                    ..
                }]
            ),
            "one command, to turn it off"
        );
        // Off while the command is on its way, though nothing says so yet.
        let power = harness.get_by_role_and_label(Role::Switch, "Power");
        assert_eq!(power.accesskit_node().toggled(), Some(Toggled::False));
    }

    #[test]
    fn the_toolbar_hides_the_log_and_opens_settings_as_does_command_comma() {
        use egui_kittest::kittest::Queryable as _;
        let mut harness = harness(state());
        harness.run_steps(2);
        harness.get_by_label("Hide Log").click();
        harness.run_steps(2);
        assert!(harness.query_by_label("Hide Log").is_none());
        harness.get_by_label("Show Log").click();
        harness.run_steps(2);
        assert!(harness.query_by_label("Hide Log").is_some());

        assert!(!harness.state().screens.settings.is_open());
        harness.get_by_label("Settings").click();
        harness.run_steps(2);
        assert!(harness.state().screens.settings.is_open());
        harness.get_by_label("Cancel").click();
        harness.run_steps(2);
        assert!(!harness.state().screens.settings.is_open());

        harness.key_press_modifiers(egui::Modifiers::COMMAND, Key::Comma);
        harness.run_steps(2);
        assert!(harness.state().screens.settings.is_open());
        assert!(harness.state().commands.is_empty());
    }

    #[test]
    fn a_log_line_can_be_copied() {
        use egui_kittest::kittest::Queryable as _;
        let mut harness = harness(state());
        harness.run_steps(2);
        let failure = |node: &egui_kittest::kittest::AccessKitNode<'_>| {
            node.value()
                .is_some_and(|line| line.ends_with("No answer."))
        };
        harness.get_by(failure).click_secondary();
        harness.run_steps(2);
        harness.get_by_label("Copy").click();
        harness.step();
        let copied = harness.output().platform_output.commands.iter().any(|command| {
            matches!(command, egui::OutputCommand::CopyText(text) if text == "No answer.")
        });
        assert!(copied, "{:?}", harness.output().platform_output.commands);
    }

    #[test]
    fn no_password_saved_is_a_notice_with_a_way_to_settings() {
        use egui_kittest::kittest::Queryable as _;
        let mut state = state();
        state.broker = BrokerState::Disconnected(NO_BROKER_PASSWORD.to_owned());
        let mut harness = harness(state);
        harness.run_steps(2);
        harness.get_by_label("Open Settings").click();
        harness.run_steps(2);
        assert!(harness.state().screens.settings.is_open());
    }

    /// Renders the screens to `target/snapshots/*.png`, so that the look can be judged
    /// without a screen: the main window in light and dark with the first diffuser open,
    /// the Settings window, and the notices. Uses the Mac's own fonts; the PNGs stay
    /// under `target/` and are never committed. Run it with
    /// `cargo test -p scent-desktop render_screens -- --ignored`.
    mod snapshots {
        use std::path::{Path, PathBuf};

        use eframe::egui::{Color32, Theme, Vec2, os::OperatingSystem, pos2};
        use egui_kittest::Harness;

        use super::super::settings;
        use super::super::theme;
        use super::*;
        use crate::state::Sent;

        fn out_dir() -> PathBuf {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots")
        }

        /// The diffusers the brief asks for: an Air 2 that answers, with fresh
        /// readings, an intensity sent and not yet reported, and a stale WiFi signal;
        /// an HVAC 2 that is silent; one diffuser found nearby; and a short log.
        fn snapshot_state() -> AppState {
            let info = |id: &str, model: &str| DeviceInfo {
                manufacturer_id: "TEST".into(),
                model_name: model.into(),
                variant_name: "TEST".into(),
                device_id: id.into(),
                firmware: "2.30".into(),
                wifi_configured: true,
                wifi_status: 3,
            };
            let mut state = AppState {
                broker: BrokerState::Connected,
                ..AppState::default()
            };
            let mut one = DeviceView::new(ONE);
            one.label = "Bedroom".to_owned();
            one.ble_id = Some("ble-1".to_owned());
            one.info = Some(info(ONE, "Diffuser Air 2"));
            one.ping_at = Some(NOW - 6);
            one.heard_wifi = Some(NOW - 5);
            for (field, value, age, via) in [
                (Field::Power, 1, 20, Via::Wifi),
                (Field::Intensity, 1, 60, Via::Wifi),
                (Field::Charging, 2, 20, Via::Wifi),
                (Field::Battery, 200, 20, Via::Wifi),
                (Field::Rssi, -66, 420, Via::Bluetooth),
            ] {
                let at = NOW - age;
                one.readings.insert(field, Reading { value, at, via });
            }
            one.sent = Some(Sent {
                power: None,
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
            let mut two = DeviceView::new(TWO);
            two.info = Some(info(TWO, "Diffuser HVAC 2"));
            two.ping_at = Some(NOW - 40);
            two.heard_wifi = Some(NOW - 900);
            two.readings.insert(
                Field::Power,
                Reading {
                    value: 0,
                    at: NOW - 900,
                    via: Via::Wifi,
                },
            );
            state.devices.insert(ONE.to_owned(), one);
            state.devices.insert(TWO.to_owned(), two);
            state.scan = vec![ScanHit {
                ble_id: "ble-7".into(),
                name: "Scentiment".into(),
                rssi: Some(-58),
            }];
            let log = [
                (
                    NOW - 40,
                    None,
                    Some(Via::Wifi),
                    Direction::FromDevice,
                    "broker",
                    "Connected to the broker.",
                    false,
                ),
                (
                    NOW - 30,
                    Some(ONE),
                    Some(Via::Wifi),
                    Direction::ToDevice,
                    "TURN_ON",
                    r#"{"action":"TURN_ON","data":{"enable":1}}"#,
                    true,
                ),
                (
                    NOW - 29,
                    Some(ONE),
                    Some(Via::Wifi),
                    Direction::ToDevice,
                    "TURN_ON",
                    r#"{"action":"TURN_ON","data":{"enable":1}}"#,
                    false,
                ),
                (
                    NOW - 20,
                    Some(ONE),
                    Some(Via::Wifi),
                    Direction::FromDevice,
                    "update_status",
                    r#"{"action":"update_status","device_id":"TESTDEVICE01","data":{"power":1,"charging":2,"battery":200,"intensity":1}}"#,
                    false,
                ),
                (
                    NOW - 2,
                    Some(ONE),
                    Some(Via::Bluetooth),
                    Direction::ToDevice,
                    "SET_LEVEL",
                    r#"{"action":"SET_LEVEL","data":{"level":3}}"#,
                    true,
                ),
                (
                    NOW - 1,
                    Some(TWO),
                    None,
                    Direction::ToDevice,
                    "Bluetooth TURN_ON",
                    "No answer.",
                    false,
                ),
            ];
            for (at, device, via, direction, action, text, sent) in log {
                state.log.push_back(LogEntry {
                    at,
                    device_id: device.map(str::to_owned),
                    via,
                    direction,
                    action: action.to_owned(),
                    text: text.to_owned(),
                    sent,
                });
            }
            state
        }

        /// Draws `draw` at `size` points, twice the pixels, in the light or dark look,
        /// and writes `target/snapshots/<name>.png`. The first frame installs the
        /// theme; the fonts it loads are in use from the next.
        fn render(name: &str, size: Vec2, dark: bool, mut draw: impl FnMut(&mut egui::Ui)) {
            let theme = if dark { Theme::Dark } else { Theme::Light };
            let mut frames = 0;
            let mut harness = Harness::builder()
                .with_size(size)
                .with_pixels_per_point(2.0)
                .with_theme(theme)
                .with_os(OperatingSystem::Mac)
                .wgpu()
                .build_ui(move |ui| {
                    frames += 1;
                    if frames == 1 {
                        theme::install(ui.ctx(), Path::new(theme::SYSTEM_FONTS));
                        ui.ctx().set_theme(theme);
                        ui.ctx().request_repaint();
                        return;
                    }
                    whole_window(ui, &mut draw);
                });
            harness.run_steps(4);
            save(name, &mut harness);
        }

        /// Calls `draw` with a ui that covers the whole render, as a window's does: the
        /// harness puts a margin around the ui it hands out.
        fn whole_window<R>(ui: &egui::Ui, draw: impl FnOnce(&mut egui::Ui) -> R) -> R {
            let builder = egui::UiBuilder::new()
                .layer_id(egui::LayerId::background())
                .max_rect(ui.ctx().content_rect());
            let mut window = egui::Ui::new(ui.ctx().clone(), egui::Id::new("snapshot"), builder);
            draw(&mut window)
        }

        fn save<S>(name: &str, harness: &mut Harness<'_, S>) {
            let image = harness.render().expect("render the screens");
            let dir = out_dir();
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(format!("{name}.png"));
            image.save(&path).unwrap();
            println!("wrote {}", path.display());
        }

        /// Where the traffic lights are, drawn into the snapshot only: the real ones are
        /// the system's and aren't in a render.
        fn traffic_lights(ui: &egui::Ui) {
            let colors = [
                Color32::from_rgb(0xFF, 0x5F, 0x57),
                Color32::from_rgb(0xFE, 0xBC, 0x2E),
                Color32::from_rgb(0x28, 0xC8, 0x40),
            ];
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("traffic_lights"),
            ));
            for (i, color) in colors.into_iter().enumerate() {
                painter.circle_filled(pos2(14.0 + 20.0 * i as f32, 14.0), 6.0, color);
            }
        }

        fn main_window(name: &str, dark: bool, state: AppState, notice: Option<&str>) {
            let mut screens = Screens::new(notice.map(str::to_owned));
            // A scan has come back, as the hit in the state says.
            screens.on_event(&Event::Scanned(state.scan.clone()));
            render(name, theme::WINDOW_SIZE.into(), dark, move |ui| {
                let _ = screens.show(ui, &state, &BrokerSettings::default(), NOW);
                traffic_lights(ui);
            });
        }

        /// The Settings window as the app draws it, at the height it asks for, showing
        /// a failed save.
        fn settings_window(name: &str, dark: bool) {
            let theme = if dark { Theme::Dark } else { Theme::Light };
            let mut window = SettingsWindow::default();
            window.open(&BrokerSettings::default());
            window.fail("The Keychain refused the password. Nothing was saved and the connection is unchanged. Type the password again to try again.".to_owned());
            let mut frames = 0;
            let mut harness = Harness::builder()
                .with_size(vec2(settings::WIDTH, 400.0))
                .with_pixels_per_point(2.0)
                .with_theme(theme)
                .with_os(OperatingSystem::Mac)
                .wgpu()
                .build_ui_state(
                    move |ui, window: &mut SettingsWindow| {
                        frames += 1;
                        if frames == 1 {
                            theme::install(ui.ctx(), Path::new(theme::SYSTEM_FONTS));
                            ui.ctx().set_theme(theme);
                            ui.ctx().request_repaint();
                            return;
                        }
                        let _ = whole_window(ui, |ui| window.window(ui, &BrokerState::Connected));
                    },
                    window,
                );
            harness.run_steps(4);
            let height = harness.state().fitted_height().expect("drawn");
            harness.set_size(vec2(settings::WIDTH, height));
            harness.run_steps(2);
            save(name, &mut harness);
        }

        #[test]
        #[ignore = "writes PNGs to look at; run on demand"]
        fn render_screens() {
            main_window("main_light", false, snapshot_state(), None);
            main_window("main_dark", true, snapshot_state(), None);

            let mut refused = snapshot_state();
            refused.broker = BrokerState::LoginRefused("Not authorized.".to_owned());
            let damaged = "Scent Control couldn't read its saved diffusers, so it started without them. The damaged file was kept as desktop.json.damaged.";
            main_window("notice_light", false, refused.clone(), Some(damaged));
            main_window("notice_dark", true, refused, Some(damaged));

            // Nothing known yet, and no password saved.
            let empty = AppState {
                broker: BrokerState::Disconnected(NO_BROKER_PASSWORD.to_owned()),
                ..AppState::default()
            };
            main_window("empty_light", false, empty, None);

            for (name, dark) in [("settings_light", false), ("settings_dark", true)] {
                settings_window(name, dark);
            }
        }
    }
}
