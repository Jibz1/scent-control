//! The open diffuser: whether it answers, what it last reported, what its schedule
//! says it should be doing, and the controls. As in System Settings, the readings and
//! the controls are each a grouped box, "Status" (with Read Status at the end of its
//! heading) and "Controls".

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};
use scent_core::{Command, Level, Via, expected_state};

use super::format::{
    expected_text, intensity, model_text, on_off, online_text, pending_text, power_source,
    reading_text, signal,
};
use super::theme::{GRID, Palette, status_color, title1};
use super::widgets::{
    NoticeKind, footnote, grouped, grouped_with, notice, plain_button, row, secondary, segmented,
    status_dot, switch, value_with_note,
};
use crate::hub::{UiCommand, on_our_broker};
use crate::route::TransportPref;
use crate::state::{AppState, DeviceView, Event, Field, Reading};

/// How long "Sending…" stays up if no answer ever comes. A Bluetooth attempt is over in
/// well under this, and the hub gives up on one itself after 30 s.
const SENDING_GIVE_UP: Duration = Duration::from_secs(40);

/// The three intensities and what the segments call them.
const LEVELS: [(Option<u8>, &str); 3] = [(Some(1), "Low"), (Some(2), "Medium"), (Some(3), "High")];

/// The "Send Over" choices.
const ROUTES: [(TransportPref, &str); 3] = [
    (TransportPref::Auto, "Auto"),
    (TransportPref::Wifi, "WiFi"),
    (TransportPref::Bluetooth, "Bluetooth"),
];

/// How a line of the readings looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Fresh,
    /// Reported more than 300 s ago: in the secondary label color.
    Stale,
    /// Sent, and the diffuser hasn't reported on it: orange.
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadingRow {
    pub name: &'static str,
    pub text: String,
    pub tone: Tone,
}

impl ReadingRow {
    /// The value, and what follows it: when and over what it was reported, or that it
    /// was sent and is waited for. "not reported yet" has nothing after it.
    pub fn parts(&self) -> (&str, Option<&str>) {
        match self.text.split_once(" · ") {
            Some((value, note)) => (value, Some(note)),
            None => (&self.text, None),
        }
    }
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

/// The power to show on the switch: the one we sent and is not yet reported, else the
/// one reported, else off.
fn shown_power(device: &DeviceView) -> bool {
    device.pending_power().unwrap_or_else(|| {
        device
            .readings
            .get(&Field::Power)
            .is_some_and(|reading| reading.value != 0)
    })
}

/// What the person just set with the switch or the intensity segments, for the
/// controls to show until the hub says the command went out (the state then shows it as
/// sent) or failed. Without it the switch would flip back for as long as a Bluetooth
/// connection takes.
#[derive(Debug, Clone, Copy)]
struct Asked {
    power: Option<bool>,
    intensity: Option<u8>,
    at: Instant,
}

#[derive(Default)]
pub struct DevicePanel {
    /// The "Send Over" choice for each diffuser. Auto until it is changed.
    prefs: HashMap<String, TransportPref>,
    /// The diffusers with a command on its way, and since when. A guess: the first
    /// `Sent` or `Failed` for a diffuser ends it, even if more commands are queued.
    sending: HashMap<String, Instant>,
    asked: HashMap<String, Asked>,
}

impl DevicePanel {
    pub fn on_event(&mut self, event: &Event) {
        match event {
            Event::Sent {
                device_id,
                power,
                intensity,
                ..
            } => {
                self.sending.remove(device_id);
                // Only the command for what the person set last ends it: an earlier one
                // (on, then off before it went) leaves the newer choice showing.
                self.forget_asked(device_id, |asked| {
                    if power.is_some() && *power == asked.power {
                        asked.power = None;
                    }
                    if intensity.is_some() && *intensity == asked.intensity {
                        asked.intensity = None;
                    }
                });
            }
            Event::Failed {
                device_id: Some(device_id),
                what,
                ..
            } => {
                self.sending.remove(device_id);
                // A failure ends what its command was for, if its name says; a failed
                // read or schedule leaves the controls alone.
                let action = what.rsplit(' ').next().unwrap_or(what);
                let was_for = |cmd: Command| action == cmd.action(Via::Wifi);
                self.forget_asked(device_id, |asked| {
                    if asked
                        .power
                        .is_some_and(|on| was_for(Command::TurnOn { on }))
                    {
                        asked.power = None;
                    }
                    let level = asked.intensity.and_then(Level::new);
                    if level.is_some_and(|level| was_for(Command::SetLevel(level))) {
                        asked.intensity = None;
                    }
                });
            }
            _ => {}
        }
    }

    /// Changes what the person set on `device_id` and is on its way, and forgets it once
    /// nothing is left.
    fn forget_asked(&mut self, device_id: &str, change: impl FnOnce(&mut Asked)) {
        if let Some(asked) = self.asked.get_mut(device_id) {
            change(asked);
            if asked.power.is_none() && asked.intensity.is_none() {
                self.asked.remove(device_id);
            }
        }
    }

    fn is_sending(&self, id: &str) -> bool {
        self.sending
            .get(id)
            .is_some_and(|since| since.elapsed() < SENDING_GIVE_UP)
    }

    /// What the person set on this diffuser and is still on its way, if anything.
    fn asked(&self, id: &str) -> Option<Asked> {
        self.asked
            .get(id)
            .filter(|asked| asked.at.elapsed() < SENDING_GIVE_UP)
            .copied()
    }

    /// Shows the diffuser and returns the commands the controls asked for.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        device: &DeviceView,
        now: i64,
    ) -> Vec<UiCommand> {
        let mut commands = Vec::new();
        let mut pref = self.prefs.get(&device.id).copied().unwrap_or_default();
        let palette = Palette::of(ui);

        header(ui, state, device, now);
        if !on_our_broker(device) {
            notice(
                ui,
                NoticeKind::Warning,
                "This diffuser hasn't been heard on your broker, so WiFi commands may not reach it. Bluetooth works while this Mac is in range.",
            );
        }
        ui.add_space(GRID / 2.0);

        let read_status = |ui: &mut egui::Ui| {
            plain_button(ui, "Read Status")
                .on_hover_text(
                    "Asks the diffuser for its state the way Send Over says. Over Bluetooth this reads what it last reported.",
                )
                .clicked()
        };
        let mut read = false;
        grouped_with(
            ui,
            "Status",
            |ui| read = read_status(ui),
            |ui| {
                for reading in reading_rows(device, now) {
                    row(ui, reading.name, |ui| {
                        let (value, note) = reading.parts();
                        let color = match reading.tone {
                            Tone::Fresh => palette.label,
                            Tone::Stale => palette.secondary_label,
                            Tone::Pending => palette.orange,
                        };
                        value_with_note(ui, RichText::new(value).color(color), note);
                    });
                }
            },
        );
        if read {
            commands.push(UiCommand::ReadStatus {
                device_id: device.id.clone(),
                pref,
            });
        }
        if let Some(line) = expected_line(device, now) {
            footnote(ui, &line);
        }
        ui.add_space(GRID);

        let send = |cmd: Command, pref: TransportPref| UiCommand::Send {
            device_id: device.id.clone(),
            cmd,
            pref,
        };
        let asked = self.asked(&device.id);
        let mut power = asked
            .and_then(|asked| asked.power)
            .unwrap_or_else(|| shown_power(device));
        let mut level = asked
            .and_then(|asked| asked.intensity)
            .or_else(|| shown_intensity(device).and_then(|level| u8::try_from(level).ok()));
        let mut now_asked = asked.unwrap_or(Asked {
            power: None,
            intensity: None,
            at: Instant::now(),
        });
        grouped(ui, "Controls", |ui| {
            row(ui, "Send Over", |ui| {
                segmented(ui, &mut pref, &ROUTES, "Send Over")
                    .response
                    .on_hover_text("Auto: Bluetooth when the diffuser is in range, otherwise WiFi");
            });
            row(ui, "Power", |ui| {
                if switch(ui, &mut power, "Power").changed() {
                    commands.push(send(Command::TurnOn { on: power }, pref));
                    now_asked.power = Some(power);
                }
            });
            row(ui, "Intensity", |ui| {
                if let Some(Some(number)) = segmented(ui, &mut level, &LEVELS, "Intensity").clicked
                    && let Some(chosen) = Level::new(number)
                {
                    commands.push(send(Command::SetLevel(chosen), pref));
                    now_asked.intensity = Some(number);
                }
            });
        });
        // A read writes nothing and may say nothing, so only a send is waited for.
        if commands
            .iter()
            .any(|command| matches!(command, UiCommand::Send { .. }))
        {
            self.sending.insert(device.id.clone(), Instant::now());
            now_asked.at = Instant::now();
            self.asked.insert(device.id.clone(), now_asked);
        }
        if self.is_sending(&device.id) {
            ui.add_space(GRID);
            ui.horizontal(|ui| {
                ui.spinner();
                secondary(ui, "Sending…");
            });
        }

        self.prefs.insert(device.id.clone(), pref);
        commands
    }
}

/// The diffuser's name, whether it answers, and its model and ID.
fn header(ui: &mut egui::Ui, state: &AppState, device: &DeviceView, now: i64) {
    let palette = Palette::of(ui);
    let online = state.online(&device.id, now);
    ui.label(
        RichText::new(&device.label)
            .text_style(title1())
            .color(palette.label),
    );
    let model = model_text(device.info.as_ref());
    let meta = match (model.is_empty(), device.label == device.id) {
        (true, true) => String::new(),
        (true, false) => device.id.clone(),
        (false, true) => model,
        (false, false) => format!("{model} · {}", device.id),
    };
    ui.horizontal(|ui| {
        let words = online_text(online);
        status_dot(ui, status_color(&palette, online), words);
        ui.label(RichText::new(words).color(palette.label));
        if !meta.is_empty() {
            ui.label(RichText::new(format!("· {meta}")).color(palette.secondary_label));
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
    fn what_we_sent_shows_orange_until_the_diffuser_reports() {
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
            "High · sent, waiting for the diffuser"
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
    fn a_reading_splits_into_its_value_and_its_note() {
        let reading = |text: &str| ReadingRow {
            name: "Power",
            text: text.to_owned(),
            tone: Tone::Fresh,
        };
        assert_eq!(
            reading("On · reported just now over WiFi").parts(),
            ("On", Some("reported just now over WiFi"))
        );
        assert_eq!(
            reading("High · sent, waiting for the diffuser").parts(),
            ("High", Some("sent, waiting for the diffuser"))
        );
        assert_eq!(
            reading("not reported yet").parts(),
            ("not reported yet", None)
        );
    }

    #[test]
    fn what_the_person_set_shows_until_its_command_goes_out_or_fails() {
        let mut panel = DevicePanel::default();
        panel.asked.insert(
            ID.to_owned(),
            Asked {
                power: Some(false),
                intensity: Some(2),
                at: Instant::now(),
            },
        );
        // The power command went out, so the state shows it as sent from here on; the
        // intensity is still on its way.
        panel.on_event(&Event::Sent {
            device_id: ID.to_owned(),
            summary: String::new(),
            via: Via::Wifi,
            at: NOW,
            power: Some(false),
            intensity: None,
            profile: None,
        });
        let asked = panel.asked(ID).unwrap();
        assert_eq!((asked.power, asked.intensity), (None, Some(2)));
        // Another diffuser's failure leaves it be; its own clears it.
        let failed = |device: &str| Event::Failed {
            device_id: Some(device.to_owned()),
            what: "SET_LEVEL".to_owned(),
            error: "No answer.".to_owned(),
            at: 0,
        };
        panel.on_event(&failed("TESTDEVICE02"));
        assert!(panel.asked(ID).is_some());
        panel.on_event(&failed(ID));
        assert!(panel.asked(ID).is_none());
    }

    #[test]
    fn the_override_ends_only_with_its_own_command() {
        let sent = |power, intensity| Event::Sent {
            device_id: ID.to_owned(),
            summary: String::new(),
            via: Via::Wifi,
            at: NOW,
            power,
            intensity,
            profile: None,
        };
        let failed = |what: &str| Event::Failed {
            device_id: Some(ID.to_owned()),
            what: what.to_owned(),
            error: "No answer.".to_owned(),
            at: 0,
        };
        let mut panel = DevicePanel::default();
        // Turned off and on again before the first command went out.
        panel.asked.insert(
            ID.to_owned(),
            Asked {
                power: Some(true),
                intensity: Some(2),
                at: Instant::now(),
            },
        );
        panel.on_event(&sent(Some(false), None));
        assert_eq!(panel.asked(ID).unwrap().power, Some(true));
        panel.on_event(&sent(Some(true), None));
        assert_eq!(panel.asked(ID).unwrap().power, None);
        // A failure ends only what its command was for.
        panel.on_event(&failed("read status"));
        panel.on_event(&failed("Bluetooth TURN_ON"));
        assert_eq!(panel.asked(ID).unwrap().intensity, Some(2));
        panel.on_event(&failed("Bluetooth SET_LEVEL"));
        assert!(panel.asked(ID).is_none());
    }

    #[test]
    fn the_switch_shows_the_power_sent_then_the_power_reported() {
        let mut d = device();
        assert!(!shown_power(&d));
        read(&mut d, Field::Power, 1, 60, Via::Wifi);
        assert!(shown_power(&d));
        d.sent = Some(Sent {
            power: Some(false),
            intensity: None,
            at: NOW - 5,
        });
        assert!(!shown_power(&d));
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
