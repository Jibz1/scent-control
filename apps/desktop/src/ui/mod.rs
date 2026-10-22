//! The screens, as a Mac split view: the sidebar (the diffuser list) on the left under
//! the traffic lights, and on the right a toolbar, the open diffuser in a centered
//! column, and the log pane along the bottom. Settings is a window of its own; Add
//! Diffuser and Edit Schedule are sheets over the main window. Each file is one screen;
//! the look is in `theme`, the controls they share in `widgets`, and the drawn glyphs in
//! `glyphs`. This one lays them out, with the empty states, and holds what they ask of
//! the app.
//!
//! The screens only read the state they are given and hand back what they want done
//! ([`Requests`]); the app sends the commands to the hub, makes the edits to a diffuser's
//! name or room, and saves the settings.

pub mod add_sheet;
pub mod clock;
pub mod device_list;
pub mod device_panel;
pub mod format;
pub mod glyphs;
pub mod hero;
pub mod log_summary;
pub mod log_view;
pub mod schedule_editor;
pub mod settings;
pub mod theme;
pub mod timeline;
pub mod widgets;

#[cfg(test)]
mod icon;
#[cfg(test)]
mod screen_tests;
#[cfg(test)]
mod snapshots;

use eframe::egui::{
    self, Align, Frame, Id, Key, KeyboardShortcut, Layout, Margin, Sense, ViewportCommand,
};

use self::add_sheet::{AddSheet, Target};
use self::device_list::{DeviceList, Selection, unknown_hits};
use self::device_panel::DevicePanel;
use self::format::{Model, name_from_scan};
use self::log_view::LogView;
use self::schedule_editor::ScheduleEditor;
use self::settings::{ListenerLine, SettingsWindow, Submitted};
use self::theme::{
    Appearance, CONTENT_MAX_WIDTH, GRID, MARGIN, Palette, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH,
    SIDEBAR_TOP_INSET, SIDEBAR_WIDTH, TOOLBAR_HEIGHT,
};
use self::widgets::{
    NoticeKind, empty_state, gear_button, hairline, notice_with, plain_button, toolbar_button,
};
use crate::add_flow::AddStep;
use crate::hub::{NO_BROKER, NO_BROKER_PASSWORD, UiCommand};
use crate::mqtt::BrokerSettings;
use crate::state::{AppState, BrokerState, Edit, Event};

/// ⌘, opens Settings, as in every Mac app.
const SETTINGS_SHORTCUT: KeyboardShortcut =
    KeyboardShortcut::new(egui::Modifiers::COMMAND, Key::Comma);

/// A sheet a screen asks to open.
pub enum OpenSheet {
    /// Add Diffuser, for this diffuser.
    Add(Target),
    /// Edit Schedule, for the diffuser with this device ID.
    Schedule(String),
}

/// What the screens ask the app to do after a frame.
#[derive(Default)]
pub struct Requests {
    pub commands: Vec<UiCommand>,
    /// The settings were saved. The app stores them and reconnects, then tells the
    /// Settings window how that went.
    pub settings: Option<Submitted>,
    /// Changes to a diffuser's name or room, for the app to make to the state and save.
    pub edits: Vec<Edit>,
    /// The appearance chosen in Settings. It is shown already; the app saves it.
    pub appearance: Option<Appearance>,
}

/// What the screens remember between frames. The hub never sees any of it.
pub struct Screens {
    /// The diffuser whose panel is open.
    selected: Option<String>,
    /// A diffuser found nearby that is selected instead, by its Bluetooth ID.
    found: Option<String>,
    list: DeviceList,
    panel: DevicePanel,
    log: LogView,
    pub settings: SettingsWindow,
    add_sheet: AddSheet,
    schedule: ScheduleEditor,
    /// A problem with the saved file, until the person dismisses it.
    notice: Option<String>,
}

impl Screens {
    pub fn new(notice: Option<String>) -> Screens {
        Screens {
            selected: None,
            found: None,
            list: DeviceList::default(),
            panel: DevicePanel::default(),
            log: LogView::default(),
            settings: SettingsWindow::default(),
            add_sheet: AddSheet::default(),
            schedule: ScheduleEditor::default(),
            notice,
        }
    }

    /// Ends any typing in the open diffuser's name or room, as when the app quits: what was
    /// typed is kept, as it is when the field is left.
    pub fn end_typing(&mut self) -> Vec<Edit> {
        self.panel.end_typing()
    }

    /// Opens Settings on the app's first run (nothing saved yet) when there is no broker,
    /// for the app to call as it starts. Once closed it stays closed, and on later runs
    /// it isn't opened: the notice for it is the way back.
    pub fn ask_for_broker(&mut self, broker: &BrokerSettings, first_run: bool) {
        if first_run && !broker.is_set() {
            self.settings.open(broker);
        }
    }

    /// Shows a problem in a notice at the top, in place of any earlier one.
    pub fn set_notice(&mut self, notice: String) {
        self.notice = Some(notice);
    }

    /// Tells the screens about an event before it is folded into the state, for what
    /// only they care about (a scan has ended; a command has got through; the diffuser
    /// the add sheet is for was added, and is opened).
    pub fn on_event(&mut self, event: &Event) {
        self.list.on_event(event);
        self.panel.on_event(event);
        // Only the add this sheet is for: another one finishing doesn't take the
        // selection.
        if let Event::Adding {
            ble_id,
            step: AddStep::Done(device_id),
        } = event
            && self.add_sheet.is_for(ble_id)
        {
            self.selected = Some(device_id.clone());
            self.found = None;
        }
    }

    /// Opens the first diffuser when none is open, and lets go of a diffuser found
    /// nearby once it isn't (it was added, or a new scan missed it).
    fn keep_selection(&mut self, state: &AppState) {
        let open = self
            .selected
            .as_ref()
            .is_some_and(|id| state.devices.contains_key(id));
        if !open {
            self.selected = state.devices.keys().next().cloned();
        }
        let hits = unknown_hits(state);
        if let Some(ble_id) = &self.found
            && !hits.iter().any(|hit| &hit.ble_id == ble_id)
        {
            self.found = None;
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
        let mut sheet = None;
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
            .size_range(SIDEBAR_MIN_WIDTH..=SIDEBAR_MAX_WIDTH)
            .show_separator_line(false)
            .frame(sidebar)
            .show(ui, |ui| {
                let selection = Selection {
                    device: &mut self.selected,
                    found: &mut self.found,
                };
                let commands = self.list.show(ui, state, selection, now);
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
            .show(ui, |ui| self.toolbar(ui, broker))
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
                    // One column, centered, no wider than reads well.
                    let width = CONTENT_MAX_WIDTH.min(ui.available_width());
                    let side = ((ui.available_width() - width) / 2.0).max(0.0);
                    ui.horizontal_top(|ui| {
                        ui.add_space(side);
                        ui.vertical(|ui| {
                            ui.set_width(width);
                            self.detail(ui, state, broker, now, &mut requests, &mut sheet);
                        });
                    });
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

        match sheet {
            Some(OpenSheet::Add(target)) => self.add_sheet.open(target),
            Some(OpenSheet::Schedule(device_id)) => {
                if let Some(device) = state.devices.get(&device_id) {
                    self.schedule.open(device, now);
                }
            }
            None => {}
        }
        let sheets = [
            self.add_sheet.show(ui.ctx(), state),
            self.schedule.show(ui.ctx(), state, now),
        ];
        requests.commands.extend(sheets.into_iter().flatten());

        let listener = ListenerLine::of(state.listener.as_ref(), state.listener_seen_at, now);
        requests.settings = self.settings.show(ui.ctx(), &state.broker, &listener);
        requests.appearance = self.settings.take_appearance();
        requests
    }

    /// What the detail pane holds: the notices, then the open diffuser, or the diffuser
    /// found nearby that is selected, or an empty state.
    fn detail(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        broker: &BrokerSettings,
        now: i64,
        requests: &mut Requests,
        sheet: &mut Option<OpenSheet>,
    ) {
        self.notices(ui, state, broker);
        let hits = unknown_hits(state);
        let found = self
            .found
            .as_ref()
            .and_then(|ble_id| hits.iter().find(|hit| &hit.ble_id == ble_id));
        if let Some(hit) = found {
            requests.edits.extend(self.panel.end_typing());
            let name = name_from_scan(&hit.name);
            let model = Model::from_scan(&hit.name);
            let draw = move |painter: &egui::Painter, rect, color, ground| {
                let ink = glyphs::Ink {
                    body: color,
                    mist: color,
                    ground,
                    track: None,
                };
                match model {
                    Model::Hvac2 => glyphs::hvac2(painter, rect, &ink, 0.0),
                    _ => glyphs::air2(painter, rect, &ink, 0.0),
                }
            };
            let add = empty_state(
                ui,
                draw,
                &format!("{name} is nearby"),
                "Add it to put it on your broker and control it from this Mac.",
                "Add Diffuser…",
            );
            if add {
                *sheet = Some(OpenSheet::Add(Target {
                    ble_id: hit.ble_id.clone(),
                    name,
                    room: String::new(),
                }));
            }
            return;
        }
        if let Some(device) = self.selected.as_ref().and_then(|id| state.devices.get(id)) {
            let edits = &mut requests.edits;
            let commands = self.panel.show(ui, state, device, now, sheet, edits);
            requests.commands.extend(commands);
            return;
        }
        requests.edits.extend(self.panel.end_typing());
        let (title, sentence) = if state.devices.is_empty() {
            (
                "No diffusers yet",
                "Scan to find the diffusers in Bluetooth range.",
            )
        } else {
            (
                "No diffuser selected",
                "Choose one on the left, or scan for more nearby.",
            )
        };
        let scanning = self.list.is_scanning();
        let scan = ui
            .add_enabled_ui(!scanning, |ui| {
                empty_state(
                    ui,
                    glyphs::empty_diffuser,
                    title,
                    sentence,
                    "Scan for Diffusers",
                )
            })
            .inner;
        if scan {
            requests.commands.push(self.list.scan());
        }
    }

    /// The toolbar along the top of the detail pane: Show Log or Hide Log and the gear
    /// for Settings, on the right. The broker's state is at the foot of the sidebar.
    fn toolbar(&mut self, ui: &mut egui::Ui, broker: &BrokerSettings) {
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
        });
    }

    /// The notices at the top of the detail pane: a refused login, no broker set, no
    /// password saved, and a problem with the saved file.
    fn notices(&mut self, ui: &mut egui::Ui, state: &AppState, broker: &BrokerSettings) {
        let broker_notice = match &state.broker {
            BrokerState::LoginRefused(_) => Some((
                NoticeKind::Error,
                "Broker login refused. Check the username and password in Settings, then save to try again.",
            )),
            BrokerState::Disconnected(why) if why == NO_BROKER || why == NO_BROKER_PASSWORD => {
                Some((NoticeKind::Warning, why.as_str()))
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
                r#"{"action":"TURN_ON","payload":{"on":1}}"#,
            ),
            (
                Direction::ToDevice,
                Some(Via::Wifi),
                "TURN_ON",
                r#"{"action":"TURN_ON","payload":{"on":1}}"#,
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
        for family in theme::named_families() {
            fonts.families.insert(family, vec![]);
        }
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
            assert!(requests.edits.is_empty() && requests.appearance.is_none());
        }
        // The first diffuser is open, and the log and the Settings window draw too.
        assert_eq!(screens.selected.as_deref(), Some(ONE));
        screens.settings.open(&BrokerSettings::default());
        let requests = draw(&mut screens, &state);
        assert!(requests.settings.is_none() && requests.appearance.is_none());
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
