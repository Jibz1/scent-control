//! The open diffuser: whether it answers, what it last reported, what its schedule
//! says it should be doing, and the buttons that control it.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};
use scent_core::{Command, Level, expected_state};

use super::format::{
    expected_text, intensity, model_text, on_off, online_text, pending_text, power_source,
    reading_text, signal,
};
use super::{amber, banner, dot, online_color};
use crate::hub::{UiCommand, on_our_broker};
use crate::route::TransportPref;
use crate::state::{AppState, DeviceView, Event, Field, Reading};

/// How long "Sending…" stays up if no answer ever comes. A Bluetooth attempt is over in
/// well under this, and the hub gives up on one itself after 30 s.
const SENDING_GIVE_UP: Duration = Duration::from_secs(40);

/// The three intensities and what the buttons call them.
const LEVELS: [(u8, &str); 3] = [(1, "Low"), (2, "Medium"), (3, "High")];

/// How a line of the readings looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Fresh,
    /// Reported more than 300 s ago: greyed out.
    Stale,
    /// Sent, and the diffuser hasn't reported on it: amber.
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadingRow {
    pub name: &'static str,
    pub text: String,
    pub tone: Tone,
}

fn reported(reading: Option<&Reading>, now: i64, format: impl Fn(i64) -> String) -> (String, Tone) {
    let (text, stale) = reading_text(reading, now, format);
    (text, if stale { Tone::Stale } else { Tone::Fresh })
}

/// The four readings: what the diffuser reported with its age and source, or, for the
/// power and intensity we sent that it hasn't reported on, what we sent.
pub fn reading_rows(device: &DeviceView, now: i64) -> [ReadingRow; 4] {
    let (power, power_tone) = match device.pending_power() {
        Some(on) => (pending_text(&on_off(i64::from(on))), Tone::Pending),
        None => reported(device.readings.get(&Field::Power), now, on_off),
    };
    let (level, level_tone) = match device.pending_intensity() {
        Some(level) => (pending_text(&intensity(i64::from(level))), Tone::Pending),
        None => reported(device.readings.get(&Field::Intensity), now, intensity),
    };
    // The power source comes from two readings; the newer one tells the age.
    let charging = device.readings.get(&Field::Charging);
    let battery = device.readings.get(&Field::Battery);
    let newest = [charging, battery]
        .into_iter()
        .flatten()
        .max_by_key(|r| r.at);
    let (source, source_tone) = reported(newest, now, |_| {
        power_source(charging.map(|r| r.value), battery.map(|r| r.value))
    });
    let (wifi, wifi_tone) = reported(device.readings.get(&Field::Rssi), now, signal);
    [
        ReadingRow {
            name: "Power",
            text: power,
            tone: power_tone,
        },
        ReadingRow {
            name: "Intensity",
            text: level,
            tone: level_tone,
        },
        ReadingRow {
            name: "Power source",
            text: source,
            tone: source_tone,
        },
        ReadingRow {
            name: "WiFi signal",
            text: wifi,
            tone: wifi_tone,
        },
    ]
}

/// What the schedule this app last sent says the diffuser should be doing now. `None`
/// when this app hasn't sent one since it started: the diffuser may hold one, but we
/// don't know what it says.
pub fn expected_line(device: &DeviceView, now: i64) -> Option<String> {
    let profile = device.sent_profile.as_ref()?;
    Some(expected_text(&expected_state(profile, now)))
}

/// The intensity to show as chosen: the one we sent and is not yet reported, else the
/// one reported.
fn shown_intensity(device: &DeviceView) -> Option<i64> {
    device.pending_intensity().map(i64::from).or_else(|| {
        device
            .readings
            .get(&Field::Intensity)
            .map(|reading| reading.value)
    })
}

#[derive(Default)]
pub struct DevicePanel {
    /// The "Send over" choice for each diffuser. Auto until it is changed.
    prefs: HashMap<String, TransportPref>,
    /// The diffusers with a command on its way, and since when. A guess: the first
    /// `Sent` or `Failed` for a diffuser ends it, even if more commands are queued.
    sending: HashMap<String, Instant>,
}

impl DevicePanel {
    pub fn on_event(&mut self, event: &Event) {
        match event {
            Event::Sent { device_id, .. }
            | Event::Failed {
                device_id: Some(device_id),
                ..
            } => {
                self.sending.remove(device_id);
            }
            _ => {}
        }
    }

    fn is_sending(&self, id: &str) -> bool {
        self.sending
            .get(id)
            .is_some_and(|since| since.elapsed() < SENDING_GIVE_UP)
    }

    /// Shows the diffuser and returns the commands the buttons asked for.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        device: &DeviceView,
        now: i64,
    ) -> Vec<UiCommand> {
        let mut commands = Vec::new();
        let mut pref = self.prefs.get(&device.id).copied().unwrap_or_default();

        header(ui, state, device, now);
        if !on_our_broker(device) {
            let color = amber(ui);
            banner(ui, color, |ui| {
                ui.label(
                    "This diffuser hasn't been heard on your broker, so WiFi commands may not reach it. Bluetooth works while this Mac is in range.",
                );
            });
        }
        ui.add_space(6.0);
        readings(ui, device, now);
        if let Some(line) = expected_line(device, now) {
            ui.add_space(4.0);
            ui.weak(line);
        }
        ui.separator();

        let send = |cmd: Command, pref: TransportPref| UiCommand::Send {
            device_id: device.id.clone(),
            cmd,
            pref,
        };
        egui::Grid::new("controls")
            .num_columns(2)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                ui.label("Send over");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut pref, TransportPref::Auto, "Auto")
                        .on_hover_text("Bluetooth when the diffuser is in range, otherwise WiFi");
                    ui.selectable_value(&mut pref, TransportPref::Wifi, "WiFi");
                    ui.selectable_value(&mut pref, TransportPref::Bluetooth, "Bluetooth");
                });
                ui.end_row();

                ui.label("Power");
                ui.horizontal(|ui| {
                    if ui.button("Turn on").clicked() {
                        commands.push(send(Command::TurnOn { on: true }, pref));
                    }
                    if ui.button("Turn off").clicked() {
                        commands.push(send(Command::TurnOn { on: false }, pref));
                    }
                });
                ui.end_row();

                ui.label("Intensity");
                ui.horizontal(|ui| {
                    let chosen = shown_intensity(device);
                    for (number, name) in LEVELS {
                        let on = chosen == Some(i64::from(number));
                        if ui.selectable_label(on, name).clicked()
                            && let Some(level) = Level::new(number)
                        {
                            commands.push(send(Command::SetLevel(level), pref));
                        }
                    }
                });
                ui.end_row();

                ui.label("Tools");
                ui.horizontal(|ui| {
                    let read = ui.button("Read status").on_hover_text(
                        "Asks the diffuser for its state the way chosen above. Over Bluetooth this reads what it last reported.",
                    );
                    if read.clicked() {
                        commands.push(UiCommand::ReadStatus {
                            device_id: device.id.clone(),
                            pref,
                        });
                    }
                });
                ui.end_row();
            });
        // A read writes nothing and may say nothing, so only a send is waited for.
        if commands
            .iter()
            .any(|command| matches!(command, UiCommand::Send { .. }))
        {
            self.sending.insert(device.id.clone(), Instant::now());
        }
        if self.is_sending(&device.id) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Sending…");
            });
        }

        self.prefs.insert(device.id.clone(), pref);
        commands
    }
}

fn header(ui: &mut egui::Ui, state: &AppState, device: &DeviceView, now: i64) {
    let online = state.online(&device.id, now);
    ui.horizontal(|ui| {
        ui.heading(&device.label);
        dot(ui, online_color(ui, online));
        ui.label(RichText::new(online_text(online)).color(online_color(ui, online)));
    });
    let model = model_text(device.info.as_ref());
    let meta = match (model.is_empty(), device.label == device.id) {
        (true, true) => String::new(),
        (true, false) => device.id.clone(),
        (false, true) => model,
        (false, false) => format!("{model} · {}", device.id),
    };
    if !meta.is_empty() {
        ui.weak(meta);
    }
}

fn readings(ui: &mut egui::Ui, device: &DeviceView, now: i64) {
    egui::Grid::new("readings")
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            for row in reading_rows(device, now) {
                ui.label(row.name);
                let color: Option<Color32> = match row.tone {
                    Tone::Fresh => None,
                    Tone::Stale => Some(ui.visuals().weak_text_color()),
                    Tone::Pending => Some(amber(ui)),
                };
                let text = RichText::new(row.text);
                ui.label(match color {
                    Some(color) => text.color(color),
                    None => text,
                });
                ui.end_row();
            }
        });
}

#[cfg(test)]
mod tests {
    use scent_core::{AlwaysOn, Profile, ScheduleFlag, Via};

    use super::*;
    use crate::state::Sent;

    const ID: &str = "TESTDEVICE01";
    const NOW: i64 = 1_792_510_200;

    fn device() -> DeviceView {
        DeviceView::new(ID)
    }

    fn read(device: &mut DeviceView, field: Field, value: i64, age: i64, via: Via) {
        let at = NOW - age;
        device.readings.insert(field, Reading { value, at, via });
    }

    fn row<'a>(rows: &'a [ReadingRow; 4], name: &str) -> &'a ReadingRow {
        rows.iter().find(|row| row.name == name).unwrap()
    }

    #[test]
    fn nothing_reported_says_so() {
        let rows = reading_rows(&device(), NOW);
        let names: Vec<&str> = rows.iter().map(|row| row.name).collect();
        assert_eq!(names, ["Power", "Intensity", "Power source", "WiFi signal"]);
        for row in &rows {
            assert_eq!(row.text, "not reported yet");
            assert_eq!(row.tone, Tone::Fresh);
        }
    }

    #[test]
    fn readings_show_their_age_and_source_and_grey_out_when_stale() {
        let mut d = device();
        read(&mut d, Field::Power, 1, 10, Via::Bluetooth);
        read(&mut d, Field::Rssi, -66, 301, Via::Wifi);
        let rows = reading_rows(&d, NOW);
        assert_eq!(
            row(&rows, "Power").text,
            "On · reported just now over Bluetooth"
        );
        assert_eq!(row(&rows, "Power").tone, Tone::Fresh);
        assert_eq!(
            row(&rows, "WiFi signal").text,
            "-66 dBm · reported 5 min ago over WiFi"
        );
        assert_eq!(row(&rows, "WiFi signal").tone, Tone::Stale);
    }

    #[test]
    fn what_we_sent_shows_amber_until_the_diffuser_reports() {
        let mut d = device();
        read(&mut d, Field::Power, 0, 60, Via::Wifi);
        read(&mut d, Field::Intensity, 1, 60, Via::Wifi);
        d.sent = Some(Sent {
            power: Some(true),
            intensity: Some(3),
            at: NOW - 5,
        });
        let rows = reading_rows(&d, NOW);
        assert_eq!(
            row(&rows, "Power").text,
            "On · sent, waiting for the diffuser"
        );
        assert_eq!(row(&rows, "Power").tone, Tone::Pending);
        assert_eq!(
            row(&rows, "Intensity").text,
            "3 (high) · sent, waiting for the diffuser"
        );
        assert_eq!(shown_intensity(&d), Some(3));

        // The diffuser reports after that: its word stands.
        read(&mut d, Field::Power, 1, 1, Via::Bluetooth);
        let rows = reading_rows(&d, NOW);
        assert_eq!(
            row(&rows, "Power").text,
            "On · reported just now over Bluetooth"
        );
        assert_eq!(row(&rows, "Power").tone, Tone::Fresh);
        assert_eq!(row(&rows, "Intensity").tone, Tone::Pending);
    }

    #[test]
    fn the_power_source_takes_its_age_from_the_newer_reading() {
        let mut d = device();
        read(&mut d, Field::Charging, 2, 400, Via::Wifi);
        read(&mut d, Field::Battery, 200, 90, Via::Bluetooth);
        let rows = reading_rows(&d, NOW);
        assert_eq!(
            row(&rows, "Power source").text,
            "plugged in · reported 2 min ago over Bluetooth"
        );
        assert_eq!(row(&rows, "Power source").tone, Tone::Fresh);

        read(&mut d, Field::Battery, 200, 400, Via::Bluetooth);
        assert_eq!(
            row(&reading_rows(&d, NOW), "Power source").tone,
            Tone::Stale
        );
    }

    #[test]
    fn a_battery_level_alone_and_a_charging_value_we_cannot_read() {
        let mut d = device();
        read(&mut d, Field::Battery, 80, 90, Via::Wifi);
        let rows = reading_rows(&d, NOW);
        assert_eq!(
            row(&rows, "Power source").text,
            "battery 80% · reported 2 min ago over WiFi"
        );

        let mut d = device();
        read(&mut d, Field::Charging, 7, 600, Via::Wifi);
        let rows = reading_rows(&d, NOW);
        assert_eq!(row(&rows, "Power source").text, "not reported yet");
        assert_eq!(row(&rows, "Power source").tone, Tone::Fresh);
    }

    #[test]
    fn the_expected_line_is_only_about_a_schedule_this_app_sent() {
        let mut d = device();
        assert_eq!(expected_line(&d, NOW), None);
        d.sent_profile = Some(Profile {
            always_on: AlwaysOn {
                enable: false,
                level: Level::new(1).unwrap(),
            },
            entries: vec![],
            timezone_hours: -5.0,
            flag: ScheduleFlag::Replace,
        });
        assert_eq!(
            expected_line(&d, NOW).as_deref(),
            Some("Based on the schedule this app last sent, it should be off now.")
        );
    }

    #[test]
    fn a_command_on_its_way_ends_with_the_first_answer() {
        let mut panel = DevicePanel::default();
        panel.sending.insert(ID.to_owned(), Instant::now());
        assert!(panel.is_sending(ID) && !panel.is_sending("TESTDEVICE02"));
        // Another diffuser's failure, or one with no diffuser, leaves it be.
        panel.on_event(&Event::Failed {
            device_id: Some("TESTDEVICE02".to_owned()),
            what: "TURN_ON".to_owned(),
            error: "No answer.".to_owned(),
            at: 0,
        });
        panel.on_event(&Event::Failed {
            device_id: None,
            what: "Hub".to_owned(),
            error: "Stopped.".to_owned(),
            at: 0,
        });
        assert!(panel.is_sending(ID));
        panel.on_event(&Event::Failed {
            device_id: Some(ID.to_owned()),
            what: "TURN_ON".to_owned(),
            error: "No answer.".to_owned(),
            at: 0,
        });
        assert!(!panel.is_sending(ID));
    }
}
