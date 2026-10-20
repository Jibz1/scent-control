//! Renders the screens to `target/snapshots/*.png`, so that the look can be judged
//! without a screen: the main window in light and dark with the first diffuser open,
//! the Settings window, and the notices. Uses the Mac's own fonts; the PNGs stay
//! under `target/` and are never committed. Run it with
//! `cargo test -p scent-desktop render_screens -- --ignored`.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, Theme, Vec2, os::OperatingSystem, pos2, vec2};
use egui_kittest::Harness;
use scent_core::{AlwaysOn, DeviceInfo, Direction, Level, Profile, ScheduleFlag, Via};

use super::Screens;
use super::settings::{self, SettingsWindow};
use super::tests::{NOW, ONE, TWO};
use super::theme;
use crate::hub::NO_BROKER_PASSWORD;
use crate::mqtt::BrokerSettings;
use crate::state::{
    AppState, BrokerState, DeviceView, Event, Field, LogEntry, Reading, ScanHit, Sent,
};

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
