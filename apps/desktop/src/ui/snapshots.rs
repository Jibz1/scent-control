//! Renders the screens to `target/snapshots/*.png`, so that the look can be judged
//! without a screen: the main window in light and dark with the first diffuser
//! diffusing at Medium and its Today strip, then with a command on its way; a diffuser
//! that is off; the log open with one row open to its message; the empty state; a
//! diffuser found nearby; the Settings window; the notices; the Add Diffuser sheet as it
//! opens, asks for a network, waits, is done, fails and asks for the network again; the
//! Edit Schedule sheet with its week grid, valid and with a mistake; and the glyphs on
//! their own at 1x and 2x. Uses the Mac's own fonts; the PNGs stay under `target/` and
//! are never committed. Run it with
//! `cargo test -p scent-desktop render_screens -- --ignored`.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, Rect, Theme, Vec2, os::OperatingSystem, pos2, vec2};
use egui_kittest::Harness;
use scent_core::{
    AlwaysOn, Command, DeviceInfo, Direction, Entry, HhMm, Level, Profile, ScheduleFlag, Secret,
    Via,
};

use super::Screens;
use super::add_sheet::Target;
use super::glyphs::{self, Ink, Mark};
use super::schedule_editor::{DraftEntry, test_time};
use super::settings::{self, SettingsWindow};
use super::tests::{NOW, ONE, TWO};
use super::theme::{self, Palette};
use crate::add_flow::{AddStep, wait_outcome};
use crate::hub::NO_BROKER_PASSWORD;
use crate::mqtt::BrokerSettings;
use crate::state::{
    Adding, AppState, BrokerState, DeviceView, Event, Field, LogEntry, Reading, ScanHit,
};

fn out_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots")
}

/// A timed entry for the snapshots' schedule.
fn entry(start: &str, end: &str, weekdays: &[u8], level: u8, enable: bool) -> Entry {
    Entry {
        start: HhMm::parse(start).unwrap(),
        end: HhMm::parse(end).unwrap(),
        weekdays: weekdays.to_vec(),
        level: Level::new(level).unwrap(),
        enable,
    }
}

/// The diffusers the brief asks for: an Air 2 that answers, diffusing at Medium, with
/// fresh readings and a schedule sent this session (it is 10:30 on a Tuesday in its zone,
/// inside a time); an HVAC 2 that is silent and off; one diffuser found nearby; and a
/// short log in the shapes the app really logs.
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
    one.info = Some(info(ONE, "Diffuser"));
    one.ping_at = Some(NOW - 6);
    one.heard_wifi = Some(NOW - 5);
    for (field, value, age, via) in [
        (Field::Power, 1, 20, Via::Wifi),
        (Field::Intensity, 2, 20, Via::Wifi),
        (Field::Charging, 2, 20, Via::Wifi),
        (Field::Battery, 200, 20, Via::Wifi),
        (Field::Rssi, -61, 20, Via::Wifi),
    ] {
        let at = NOW - age;
        one.readings.insert(field, Reading { value, at, via });
    }
    one.sent_profile = Some(Profile {
        always_on: AlwaysOn {
            enable: false,
            level: Level::new(1).unwrap(),
        },
        entries: vec![
            entry("07:00", "09:00", &[1, 2, 3, 4, 5], 2, true),
            entry("10:00", "12:00", &[2], 2, true),
            entry("13:00", "14:00", &[2, 4], 3, false),
            entry("21:30", "23:00", &[0, 1, 2, 3, 4, 5, 6], 1, true),
        ],
        timezone_hours: -5.0,
        flag: ScheduleFlag::Replace,
    });
    let mut two = DeviceView::new(TWO);
    two.info = Some(info(TWO, "Hvac"));
    two.ble_id = Some("ble-2".to_owned());
    two.ping_at = Some(NOW - 40);
    two.heard_wifi = Some(NOW - 900);
    for (field, value) in [(Field::Power, 0), (Field::Charging, 2), (Field::Rssi, -74)] {
        let reading = Reading {
            value,
            at: NOW - 900,
            via: Via::Wifi,
        };
        two.readings.insert(field, reading);
    }
    state.devices.insert(ONE.to_owned(), one);
    state.devices.insert(TWO.to_owned(), two);
    state.scan = vec![ScanHit {
        ble_id: "ble-7".into(),
        name: "Scentiment-D B n 0A1B2C3D4E5F".into(),
        rssi: Some(-58),
    }];
    let save = Command::SaveMqtt {
        broker: "wss://broker.example/mqtt".to_owned(),
        username: "scent-test".to_owned(),
        password: Secret::new("hunter2".to_owned()),
    };
    let sent = |cmd: &Command, via| (cmd.redacted(via), cmd.action(via));
    let (on, turn_on) = sent(&Command::TurnOn { on: true }, Via::Wifi);
    let (medium, set_level) = sent(&Command::SetLevel(Level::new(2).unwrap()), Via::Bluetooth);
    let (saved, save_mqtt) = sent(&save, Via::Bluetooth);
    let status = r#"{"action":"update_status","payload":{"deviceId":"TESTDEVICE01","data":{"data":[85,170,11,255,1,2,200,50,255,2,3,255,255,0,92,106]},"rssi":-61}}"#;
    let log = [
        (
            NOW - 600,
            Some(TWO),
            Some(Via::Bluetooth),
            Direction::ToDevice,
            save_mqtt,
            saved.as_str(),
            true,
        ),
        (
            NOW - 120,
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
            turn_on,
            on.as_str(),
            true,
        ),
        (
            NOW - 29,
            Some(ONE),
            Some(Via::Wifi),
            Direction::ToDevice,
            turn_on,
            on.as_str(),
            false,
        ),
        (
            NOW - 26,
            Some(ONE),
            Some(Via::Bluetooth),
            Direction::ToDevice,
            set_level,
            medium.as_str(),
            true,
        ),
        (
            NOW - 20,
            Some(ONE),
            Some(Via::Wifi),
            Direction::FromDevice,
            "update_status",
            status,
            false,
        ),
        (
            NOW - 1,
            Some(TWO),
            None,
            Direction::ToDevice,
            "Bluetooth TURN_ON",
            "The diffuser didn't answer.",
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
            text: crate::state::mask_passwords(text),
            sent,
        });
    }
    state
}

/// Draws `draw` at `size` points, twice the pixels, in the light or dark look,
/// and writes `target/snapshots/<name>.png`. The first frame installs the
/// theme; the fonts it loads are in use from the next.
fn render(name: &str, size: Vec2, dark: bool, draw: impl FnMut(&mut egui::Ui)) -> Pixels {
    render_at(name, size, dark, 2.0, draw)
}

/// [`render`] at `scale` pixels to the point: 2 as on a Retina screen, 1 as on others.
fn render_at(
    name: &str,
    size: Vec2,
    dark: bool,
    scale: f32,
    mut draw: impl FnMut(&mut egui::Ui),
) -> Pixels {
    let theme = if dark { Theme::Dark } else { Theme::Light };
    let mut frames = 0;
    let mut harness = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(scale)
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
    // A sheet takes a few frames to measure what it holds.
    harness.run_steps(8);
    save(name, &mut harness)
}

/// Calls `draw` with a ui that covers the whole render, as a window's does: the
/// harness puts a margin around the ui it hands out. Under it is the window color, as
/// the app's clear color puts there.
fn whole_window<R>(ui: &egui::Ui, draw: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let rect = ui.ctx().content_rect();
    let builder = egui::UiBuilder::new()
        .layer_id(egui::LayerId::background())
        .max_rect(rect);
    let mut window = egui::Ui::new(ui.ctx().clone(), egui::Id::new("snapshot"), builder);
    window
        .painter()
        .rect_filled(rect, 0, Palette::of(&window).window);
    draw(&mut window)
}

/// Writes the render to `target/snapshots/<name>.png` and hands back its pixels.
fn save<S>(name: &str, harness: &mut Harness<'_, S>) -> Pixels {
    let image = harness.render().expect("render the screens");
    let dir = out_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.png"));
    image.save(&path).unwrap();
    println!("wrote {}", path.display());
    let pixels = Pixels {
        name: name.to_owned(),
        width: image.width(),
        rgba: image.as_raw().clone(),
    };
    pixels.assert_opaque();
    pixels
}

/// A render's pixels, for checking what was drawn.
struct Pixels {
    name: String,
    width: u32,
    rgba: Vec<u8>,
}

impl Pixels {
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        let i = 4 * (y * self.width + x) as usize;
        [0, 1, 2, 3].map(|c| self.rgba[i + c])
    }

    /// Every pixel is opaque: nothing of what is behind the window (eframe's clear
    /// color) shows through a gap.
    fn assert_opaque(&self) {
        let gaps = self.rgba.chunks(4).filter(|pixel| pixel[3] != 255).count();
        assert_eq!(gaps, 0, "{}: {gaps} pixels not opaque", self.name);
    }

    /// The seam that `points` cross, in points, is drawn in the separator color over
    /// one of `bases`, and everything else along them is one of those bases.
    fn assert_seam(
        &self,
        points: impl IntoIterator<Item = (f32, f32)>,
        separator: Color32,
        bases: &[Color32],
    ) {
        let close = |a: [u8; 4], b: [u8; 3]| (0..3).all(|c| a[c].abs_diff(b[c]) <= 3);
        let lines: Vec<[u8; 3]> = bases.iter().map(|&base| over(separator, base)).collect();
        let plain: Vec<[u8; 3]> = bases
            .iter()
            .map(|base| [base.r(), base.g(), base.b()])
            .collect();
        let mut seen = false;
        for (x, y) in points {
            let pixel = self.at((2.0 * x) as u32, (2.0 * y) as u32);
            let on_line = lines.iter().any(|&line| close(pixel, line));
            assert!(
                on_line || plain.iter().any(|&base| close(pixel, base)),
                "{}: {pixel:?} at ({x}, {y}) pt is neither a base {plain:?} nor the separator over one {lines:?}",
                self.name
            );
            if on_line {
                println!("{}: separator {pixel:?} at ({x}, {y}) pt", self.name);
            }
            seen |= on_line;
        }
        assert!(seen, "{}: no separator found", self.name);
    }
}

/// `top`, premultiplied, laid over the opaque `base`, blended as egui blends.
fn over(top: Color32, base: Color32) -> [u8; 3] {
    let keep = 1.0 - f32::from(top.a()) / 255.0;
    let (top, base) = (top.to_array(), base.to_array());
    [0, 1, 2].map(|c| (f32::from(top[c]) + f32::from(base[c]) * keep).round() as u8)
}

/// Half-point steps from `from` to `to`, across a seam.
fn across(from: f32, to: f32) -> impl Iterator<Item = f32> {
    (0..=((to - from) * 2.0) as u32).map(move |i| from + i as f32 / 2.0)
}

/// The sidebar's edge and the line under the toolbar, and the line above the log when
/// it is shown, are each the separator color, over an opaque window.
fn assert_seams(pixels: &Pixels, dark: bool, log: bool) {
    let palette = Palette::for_dark(dark);
    let [_, height] = theme::WINDOW_SIZE;
    let edge = theme::SIDEBAR_WIDTH;
    pixels.assert_seam(
        across(edge - 2.0, edge + 2.0).map(|x| (x, 300.0)),
        palette.separator,
        &[palette.sidebar, palette.window],
    );
    let toolbar = theme::TOOLBAR_HEIGHT;
    pixels.assert_seam(
        across(toolbar - 2.0, toolbar + 2.0).map(|y| (750.0, y)),
        palette.separator,
        &[palette.window],
    );
    if log {
        let log = height - super::log_view::OPEN_HEIGHT;
        pixels.assert_seam(
            across(log - 2.0, log + 2.0).map(|y| (750.0, y)),
            palette.separator,
            &[palette.window, palette.grouped],
        );
    }
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

fn main_window(name: &str, dark: bool, state: AppState, notice: Option<&str>) -> Pixels {
    let size = theme::WINDOW_SIZE.into();
    main_window_with(name, dark, size, state, notice, |_, _| {})
}

/// The main window at `size`, with `prepare` given the screens to open a sheet on first.
fn main_window_with(
    name: &str,
    dark: bool,
    size: Vec2,
    state: AppState,
    notice: Option<&str>,
    prepare: impl FnOnce(&mut Screens, &AppState),
) -> Pixels {
    let mut screens = Screens::new(notice.map(str::to_owned));
    // A scan has come back, as the hit in the state says.
    screens.on_event(&Event::Scanned(state.scan.clone()));
    prepare(&mut screens, &state);
    render(name, size, dark, move |ui| {
        let _ = screens.show(ui, &state, &BrokerSettings::default(), NOW);
        traffic_lights(ui);
    })
}

/// The diffuser found nearby, read over Bluetooth as the add began: not on WiFi yet.
const ADDED: &str = "TESTDEVICE03";
const ADDED_BLE: &str = "ble-7";

/// Where the Add Diffuser sheet is in a render.
enum AddRender {
    /// Just opened from the diffuser found nearby: nothing asked for yet.
    Ready,
    /// The add is at `step`, having last worked on `reached`; `network` was given.
    At {
        step: AddStep,
        reached: AddStep,
        network: Option<&'static str>,
    },
    /// The add that gave this network failed, and Try Again asks for it again.
    Retrying(&'static str),
}

/// The state while the diffuser found nearby is being added, at `step`.
fn adding_state(step: AddStep, reached: AddStep, network: Option<&str>) -> AppState {
    let mut state = snapshot_state();
    let mut found = DeviceView::new(ADDED);
    found.ble_id = Some(ADDED_BLE.to_owned());
    found.info = Some(DeviceInfo {
        manufacturer_id: "01".into(),
        model_name: "Diffuser".into(),
        variant_name: "Black".into(),
        device_id: ADDED.into(),
        firmware: "2.30".into(),
        wifi_configured: false,
        wifi_status: 0,
    });
    state.devices.insert(ADDED.to_owned(), found);
    let adding = Adding {
        ble_id: ADDED_BLE.to_owned(),
        label: "Air 2".to_owned(),
        network: network.map(str::to_owned),
        step,
        reached: Some(reached),
    };
    state.adding.insert(ADDED_BLE.to_owned(), adding);
    state
}

/// The Add Diffuser sheet over the main window, for the diffuser found nearby.
fn add_sheet(name: &str, dark: bool, at: AddRender) {
    let failed = || wait_outcome(0, 90, false);
    let state = match &at {
        AddRender::Ready => snapshot_state(),
        AddRender::At {
            step,
            reached,
            network,
        } => adding_state(step.clone(), reached.clone(), *network),
        AddRender::Retrying(network) => {
            adding_state(failed(), AddStep::WaitConnected, Some(network))
        }
    };
    let size = theme::WINDOW_SIZE.into();
    let _ = main_window_with(name, dark, size, state, None, move |screens, _| {
        screens.add_sheet.open(Target {
            ble_id: ADDED_BLE.to_owned(),
            name: "Air 2".to_owned(),
        });
        match at {
            AddRender::Ready => {}
            AddRender::At { .. } => screens.add_sheet.as_submitted(),
            AddRender::Retrying(network) => screens.add_sheet.as_retrying(network),
        }
    });
}

/// The Edit Schedule sheet over the main window, for the first diffuser, with `entries`.
/// The window is taller than it opens, so that the whole sheet shows; at the size it
/// opens the times scroll.
fn schedule_editor(name: &str, dark: bool, entries: Vec<DraftEntry>) {
    let size = vec2(theme::WINDOW_SIZE[0], 1260.0);
    let _ = main_window_with(
        name,
        dark,
        size,
        snapshot_state(),
        None,
        move |screens, state| {
            screens.schedule.open(&state.devices[ONE], NOW);
            let draft = screens.schedule.draft_mut();
            draft.offset = "-4".to_owned();
            draft.always_on = true;
            draft.entries = entries;
        },
    );
}

fn time(start: &str, end: &str, days: &[u8], level: u8) -> DraftEntry {
    DraftEntry {
        start: start.to_owned(),
        end: end.to_owned(),
        days: days.to_vec(),
        level,
        runs: true,
    }
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
    let _ = save(name, &mut harness);
}

/// The glyphs on their own, at `scale` pixels to the point: the sidebar's tiles (grey,
/// tinted at each level, and on a selected row), the hero's glyphs at each level and off,
/// the empty state's diffuser, the log's disclosure triangles and marks, and the signal
/// bars at each strength.
fn glyph_sheet(name: &str, dark: bool, scale: f32) {
    let _ = render_at(name, vec2(600.0, 360.0), dark, scale, |ui| {
        let palette = Palette::of(ui);
        let painter = ui.painter();
        let card = Rect::from_min_size(pos2(16.0, 16.0), vec2(568.0, 328.0));
        painter.rect_filled(card, 10, palette.grouped);
        let ground = palette.grouped;
        let draw = |rect: Rect, ink: &Ink, hvac: bool, level: f32| {
            if hvac {
                glyphs::hvac2(painter, rect, ink, level);
            } else {
                glyphs::air2(painter, rect, ink, level);
            }
        };
        // Tiles: grey, then tinted at levels 0 to 3, then on a selected row; the Air 2
        // and then the HVAC 2.
        for (row, hvac) in [(0, false), (1, true)] {
            let levels = [
                (false, 0.0),
                (true, 0.0),
                (true, 1.0),
                (true, 2.0),
                (true, 3.0),
            ];
            for (i, (lit, level)) in levels.into_iter().enumerate() {
                let at = pos2(32.0 + 40.0 * i as f32, 32.0 + 40.0 * row as f32);
                let tile = Rect::from_min_size(at, vec2(28.0, 28.0));
                let (fill, glyph) = if lit {
                    (Palette::tint(palette.accent, 16), palette.accent)
                } else {
                    (Palette::tint(palette.gray, 18), palette.gray)
                };
                let fill = glyphs::solid(fill, ground);
                glyphs::tile(painter, tile, fill);
                let ink = Ink {
                    body: glyph,
                    mist: glyph,
                    ground: fill,
                    track: None,
                };
                draw(tile, &ink, hvac, level);
            }
            let row_rect =
                Rect::from_min_size(pos2(232.0, 26.0 + 40.0 * row as f32), vec2(80.0, 40.0));
            painter.rect_filled(row_rect, 6, palette.accent);
            let tile = Rect::from_min_size(row_rect.min + vec2(6.0, 6.0), vec2(28.0, 28.0));
            let fill = glyphs::solid(Color32::from_white_alpha(56), palette.accent);
            glyphs::tile(painter, tile, fill);
            let ink = Ink {
                body: Color32::WHITE,
                mist: Color32::WHITE,
                ground: fill,
                track: None,
            };
            draw(tile, &ink, hvac, if hvac { 2.0 } else { 3.0 });
        }
        // The hero's glyphs at 64 pt: diffusing at Low, Medium and High, with the unlit
        // wisps as tracks, then off; the Air 2, then the HVAC 2 at Medium and off.
        let heroes = [
            (false, Some(1.0)),
            (false, Some(2.0)),
            (false, Some(3.0)),
            (false, None),
            (true, Some(2.0)),
            (true, None),
        ];
        for (i, (hvac, level)) in heroes.into_iter().enumerate() {
            let rect = Rect::from_min_size(pos2(28.0 + 70.0 * i as f32, 116.0), vec2(64.0, 64.0));
            let ink = match level {
                Some(_) => Ink {
                    body: palette.label,
                    mist: palette.accent,
                    ground,
                    track: Some(Palette::tint(palette.label, 12)),
                },
                None => Ink {
                    body: palette.gray,
                    mist: palette.gray,
                    ground,
                    track: None,
                },
            };
            draw(rect, &ink, hvac, level.unwrap_or(0.0));
        }
        // The empty state's diffuser, at 96 pt.
        let empty = Rect::from_min_size(pos2(456.0, 100.0), vec2(96.0, 96.0));
        glyphs::empty_diffuser(painter, empty, palette.gray, ground);
        // The log's disclosure triangles, closed and open, and its marks: sent, heard,
        // failed.
        let secondary = glyphs::solid(palette.secondary_label, ground);
        for (i, open) in [false, true].into_iter().enumerate() {
            let rect = Rect::from_min_size(pos2(32.0 + 20.0 * i as f32, 232.0), vec2(12.0, 12.0));
            glyphs::disclosure(painter, rect, secondary, open);
        }
        let gray = glyphs::solid(palette.gray, ground);
        let orange = glyphs::solid(palette.orange, ground);
        let marks = [
            (Mark::Sent, gray),
            (Mark::Heard, gray),
            (Mark::Failed, orange),
        ];
        for (i, (mark, color)) in marks.into_iter().enumerate() {
            let rect = Rect::from_min_size(pos2(92.0 + 24.0 * i as f32, 231.0), vec2(14.0, 14.0));
            glyphs::mark(painter, rect, color, mark);
        }
        // Signal bars, weak to excellent.
        for filled in 1..=4 {
            let rect = Rect::from_min_size(
                pos2(170.0 + 28.0 * f32::from(filled), 233.0),
                vec2(16.0, 11.0),
            );
            let on = glyphs::solid(palette.label, ground);
            glyphs::signal_bars(painter, rect, on, Palette::tint(palette.label, 15), filled);
        }
    });
}

/// The same times with and without the always-on entry, as the week grid and as the
/// Today strip, side by side: the always-on tint must read apart from the empty track and
/// from a time switched off.
fn always_on_compare(name: &str, dark: bool) {
    let _ = render_at(name, vec2(900.0, 300.0), dark, 2.0, |ui| {
        let palette = Palette::of(ui);
        let profile = |always: bool| Profile {
            always_on: AlwaysOn {
                enable: always,
                level: Level::new(1).unwrap(),
            },
            entries: vec![
                entry("07:00", "09:00", &[1, 2, 3, 4, 5], 2, true),
                entry("08:30", "10:00", &[1], 3, true),
                entry("13:00", "14:00", &[2, 4], 3, false),
                entry("21:30", "23:00", &[0, 6], 1, true),
            ],
            timezone_hours: -5.0,
            flag: ScheduleFlag::Replace,
        };
        for (i, always) in [false, true].into_iter().enumerate() {
            let column =
                Rect::from_min_size(pos2(16.0 + 440.0 * i as f32, 16.0), vec2(428.0, 268.0));
            ui.painter().rect_filled(column, 10, palette.grouped);
            let inside = column.shrink(16.0);
            let mut at = ui.new_child(egui::UiBuilder::new().max_rect(inside));
            let title = if always {
                "Always on at Low"
            } else {
                "No always on"
            };
            at.label(egui::RichText::new(title).color(palette.secondary_label));
            super::timeline::week_grid(&mut at, &profile(always), palette.grouped);
            at.add_space(8.0);
            super::timeline::today_strip(&mut at, &profile(always), NOW, palette.grouped);
        }
    });
}

/// The main window with the first diffuser open and a command on its way to it: the
/// person has just set High.
fn sending(name: &str, dark: bool) {
    let size = theme::WINDOW_SIZE.into();
    let _ = main_window_with(name, dark, size, snapshot_state(), None, |screens, _| {
        screens.panel.as_sending(ONE, None, Some(3));
    });
}

#[test]
#[ignore = "writes PNGs to look at; run on demand"]
fn render_screens() {
    let light = main_window("main_light", false, snapshot_state(), None);
    assert_seams(&light, false, false);
    let dark = main_window("main_dark", true, snapshot_state(), None);
    assert_seams(&dark, true, false);
    sending("sending_light", false);
    sending("sending_dark", true);

    // A command that failed: the hero says so in red, with the log hidden.
    let size = theme::WINDOW_SIZE.into();
    let _ = main_window_with(
        "failed_light",
        false,
        size,
        snapshot_state(),
        None,
        |screens, _| {
            screens.on_event(&Event::Failed {
                device_id: Some(ONE.to_owned()),
                what: "Bluetooth SET_LEVEL".to_owned(),
                error: "The diffuser didn't answer. Check that it is in range and switched on."
                    .to_owned(),
                at: NOW,
            });
        },
    );

    // The HVAC 2, off and not answering.
    let _ = main_window_with(
        "off_light",
        false,
        size,
        snapshot_state(),
        None,
        |screens, _| {
            screens.selected = Some(TWO.to_owned());
        },
    );

    // The log open, with the broker login's row open to its masked message.
    for (name, dark) in [("log_light", false), ("log_dark", true)] {
        let pixels = main_window_with(
            name,
            dark,
            size,
            snapshot_state(),
            None,
            |screens, state| {
                screens
                    .log
                    .as_open(state, |entry| entry.action == "SAVE_MQTT");
            },
        );
        assert_seams(&pixels, dark, true);
    }

    // The diffuser found nearby, selected.
    for (name, dark) in [("found_light", false), ("found_dark", true)] {
        let _ = main_window_with(name, dark, size, snapshot_state(), None, |screens, _| {
            screens.found = Some("ble-7".to_owned());
        });
    }

    let mut refused = snapshot_state();
    refused.broker = BrokerState::LoginRefused("Not authorized.".to_owned());
    let damaged = "Scent Control couldn't read its saved diffusers, so it started without them. The damaged file was kept as desktop.json.damaged.";
    let _ = main_window("notice_light", false, refused.clone(), Some(damaged));
    let _ = main_window("notice_dark", true, refused, Some(damaged));

    // Nothing known yet, and no password saved.
    let empty = AppState {
        broker: BrokerState::Disconnected(NO_BROKER_PASSWORD.to_owned()),
        ..AppState::default()
    };
    let empty_light = main_window("empty_light", false, empty.clone(), None);
    assert_seams(&empty_light, false, false);
    let _ = main_window("empty_dark", true, empty, None);

    for (name, dark) in [("settings_light", false), ("settings_dark", true)] {
        settings_window(name, dark);
    }

    // Adding the diffuser found nearby: it isn't on WiFi, so the sheet asks for the
    // network; then, with one given, it waits for the diffuser on the broker; and then
    // the wait has failed.
    add_sheet("add_ready_light", false, AddRender::Ready);
    add_sheet(
        "add_need_wifi_light",
        false,
        AddRender::At {
            step: AddStep::NeedWifi,
            reached: AddStep::ReadInfo,
            network: None,
        },
    );
    for (name, dark) in [("add_waiting_light", false), ("add_waiting_dark", true)] {
        let waiting = AddRender::At {
            step: AddStep::WaitConnected,
            reached: AddStep::WaitConnected,
            network: Some("ExampleWiFi"),
        };
        add_sheet(name, dark, waiting);
    }
    let failed = AddRender::At {
        step: wait_outcome(0, 90, false),
        reached: AddStep::WaitConnected,
        network: Some("ExampleWiFi"),
    };
    add_sheet("add_failed_light", false, failed);
    let done = AddRender::At {
        step: AddStep::Done(ADDED.to_owned()),
        reached: AddStep::Ping,
        network: Some("ExampleWiFi"),
    };
    add_sheet("add_done_light", false, done);
    // Try Again after that: the network again, with its password to type.
    add_sheet(
        "add_retry_wifi_light",
        false,
        AddRender::Retrying("ExampleWiFi"),
    );

    // Two times, one of them switched off, and a test time starting in 3 minutes (11:33
    // at UTC-4), with always on; then a time that ends before it starts, which the week
    // leaves out until it is fixed.
    let test = test_time(NOW, -4.0, 3).expect("a test time");
    let mut evening = time("18:30", "22:00", &[0, 6], 1);
    evening.runs = false;
    let valid = vec![
        time("07:00", "09:00", &[1, 2, 3, 4, 5], 2),
        time("08:30", "10:00", &[1], 3),
        evening,
        test,
    ];
    schedule_editor("schedule_valid_light", false, valid.clone());
    schedule_editor("schedule_valid_dark", true, valid);
    schedule_editor(
        "schedule_error_light",
        false,
        vec![
            time("07:00", "09:00", &[1, 2, 3, 4, 5], 2),
            time("22:00", "06:00", &[0, 6], 1),
        ],
    );

    // The always-on tint beside the empty track.
    always_on_compare("always_on_compare_light", false);
    always_on_compare("always_on_compare_dark", true);

    // The glyphs on their own, as a screen without Retina shows them and as one with.
    glyph_sheet("glyphs_1x", false, 1.0);
    glyph_sheet("glyphs_2x", false, 2.0);
    glyph_sheet("glyphs_dark_2x", true, 2.0);
}
