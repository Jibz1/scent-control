//! The open diffuser: its name and whether it answers, then the diffusion hero (what it
//! is doing, and the power and intensity to change it), then grouped boxes, as in
//! System Settings: "Schedule" (what this app sent, today's times, and Edit Schedule…),
//! "Details" (the power source, the WiFi signal, the last report, the firmware and the
//! ID) and "Connection" (Send Over, and Read Status).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{self, Id, RichText, Sense, vec2};
use scent_core::{Command, Level, Online, Via, expected_state, is_stale};

use super::OpenSheet;
use super::add_sheet::Target;
use super::format::{
    Model, ago, capitalized, display_name, expected_text, intensity, name_from_model, on_off,
    pending_text, power_source, reading_text, signal, signal_quality, via_name,
};
use super::glyphs::signal_bars;
use super::hero::{self, HeroState};
use super::theme::{GRID, Palette, SECTION_GAP, title1};
use super::timeline::today_strip;
use super::widgets::{
    NoticeKind, footnote, free_row, grouped, grouped_with, notice, notice_with, plain_button, row,
    segmented, value_with_note,
};
use crate::hub::{UiCommand, on_our_broker};
use crate::route::TransportPref;
use crate::state::{AppState, DeviceView, Event, Field, Reading};

/// How long "Sending…" stays up if no answer ever comes. A Bluetooth attempt is over in
/// well under this, and the hub gives up on one itself after 30 s.
const SENDING_GIVE_UP: Duration = Duration::from_secs(40);

/// The "Send Over" choices.
const ROUTES: [(TransportPref, &str); 3] = [
    (TransportPref::Auto, "Auto"),
    (TransportPref::Wifi, "WiFi"),
    (TransportPref::Bluetooth, "Bluetooth"),
];

/// What the hero says under the state while what was sent is waited for.
const WAITING: &str = "Sent, waiting for the diffuser";

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

/// The add sheet for a diffuser known over Bluetooth that isn't on our broker yet, named
/// as it is, or after its model.
fn add_target(device: &DeviceView) -> Option<Target> {
    let ble_id = device.ble_id.clone()?;
    let name = if device.label == device.id {
        name_from_model(
            device
                .info
                .as_ref()
                .map_or("", |info| info.model_name.as_str()),
        )
    } else {
        device.label.clone()
    };
    Some(Target { ble_id, name })
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

/// The power to show: the one we sent and is not yet reported, else the one reported.
/// `None` when there is neither.
fn shown_power(device: &DeviceView) -> Option<bool> {
    device.pending_power().or_else(|| {
        device
            .readings
            .get(&Field::Power)
            .map(|reading| reading.value != 0)
    })
}

/// What the hero shows for `device`: what the person set and is on its way (`asked`)
/// first, then what was sent and not yet reported on, then what it reported; whether a
/// command is on its way (`sending`); and where the state comes from.
fn hero_state(device: &DeviceView, asked: Option<Asked>, sending: bool, now: i64) -> HeroState {
    let power = asked
        .and_then(|asked| asked.power)
        .or_else(|| shown_power(device));
    let level = asked.and_then(|asked| asked.intensity).or_else(|| {
        shown_intensity(device)
            .and_then(|level| u8::try_from(level).ok())
            .filter(|level| (1..=3).contains(level))
    });
    let power_reading = device.readings.get(&Field::Power);
    let pending = device.pending_power().is_some() || device.pending_intensity().is_some();
    let newest = [power_reading, device.readings.get(&Field::Intensity)]
        .into_iter()
        .flatten()
        .max_by_key(|reading| reading.at);
    let note = match newest {
        _ if pending => WAITING.to_owned(),
        Some(reading) => format!(
            "Reported {} over {}",
            ago(reading.at, now),
            via_name(reading.via)
        ),
        None => "Read Status asks it what it is doing".to_owned(),
    };
    HeroState {
        name: display_name(device),
        model: Some(Model::of(device)),
        power,
        level,
        sending,
        pending,
        stale: device.pending_power().is_none()
            && power_reading.is_some_and(|reading| is_stale(reading.at, now)),
        note,
    }
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

/// The job a failure is about, from the name the hub gives it: "Bluetooth TURN_ON",
/// "TURN_ON" (when both routes failed), "WiFi PING", "read status", "Schedule".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job<'a> {
    /// A command, by its action.
    Command(&'a str),
    /// A job that sends nothing the controls set: a read or a schedule.
    Other,
    /// A name the panel doesn't know, such as "Hub".
    Unknown,
}

/// The actions the hub can name in a failure, by the commands the app sends.
const ACTIONS: [&str; 10] = [
    "TURN_ON",
    "SET_LEVEL",
    "PING",
    "GET_STATUS",
    "SYNC_TIME",
    "SAVE_MQTT",
    "WIFI_CONNECT",
    "SECRET_COMMAND",
    "RESET",
    "UPDATE_PROFILE",
];

fn failed_job(what: &str) -> Job<'_> {
    let name = ["Bluetooth ", "WiFi "]
        .into_iter()
        .find_map(|via| what.strip_prefix(via))
        .unwrap_or(what);
    match name {
        "read status" | "Schedule" => Job::Other,
        action if ACTIONS.contains(&action) => Job::Command(action),
        _ => Job::Unknown,
    }
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
                // A failure ends what its command was for. One for another job (a read,
                // a schedule, a ping) leaves the controls alone; one that names no job
                // the panel knows, such as the hub losing the diffuser's queue, ends both.
                let failed = failed_job(what);
                self.forget_asked(device_id, |asked| {
                    let power = asked
                        .power
                        .map(|on| Command::TurnOn { on }.action(Via::Wifi));
                    let level = asked.intensity.and_then(Level::new);
                    let level = level.map(|level| Command::SetLevel(level).action(Via::Wifi));
                    match failed {
                        Job::Command(name) if power == Some(name) => asked.power = None,
                        Job::Command(name) if level == Some(name) => asked.intensity = None,
                        Job::Command(_) | Job::Other => {}
                        Job::Unknown => {
                            asked.power = None;
                            asked.intensity = None;
                        }
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

    /// What the person set on `device_id` and is on its way, for the snapshots.
    #[cfg(test)]
    pub fn as_sending(&mut self, device_id: &str, power: Option<bool>, intensity: Option<u8>) {
        let at = Instant::now();
        self.sending.insert(device_id.to_owned(), at);
        let asked = Asked {
            power,
            intensity,
            at,
        };
        self.asked.insert(device_id.to_owned(), asked);
    }

    /// Shows the diffuser and returns the commands the controls asked for. `sheet` is set
    /// when Edit Schedule… or Add to Broker… was clicked.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        device: &DeviceView,
        now: i64,
        sheet: &mut Option<OpenSheet>,
    ) -> Vec<UiCommand> {
        let mut commands = Vec::new();
        let mut pref = self.prefs.get(&device.id).copied().unwrap_or_default();

        header(ui, state.online(&device.id, now), device);
        if !on_our_broker(device) {
            ui.add_space(2.0 * GRID);
            let text = "This diffuser hasn't been heard on your broker, so WiFi commands may not reach it. Bluetooth works while this Mac is in range.";
            match add_target(device) {
                Some(target) => {
                    let (add, _) = notice_with(ui, NoticeKind::Warning, text, |ui| {
                        plain_button(ui, "Add to Broker…").clicked()
                    });
                    if add {
                        *sheet = Some(OpenSheet::Add(target));
                    }
                }
                None => {
                    notice(ui, NoticeKind::Warning, text);
                }
            }
        }
        ui.add_space(SECTION_GAP);

        let send = |cmd: Command, pref: TransportPref| UiCommand::Send {
            device_id: device.id.clone(),
            cmd,
            pref,
        };
        let asked = self.asked(&device.id);
        let shown = hero_state(device, asked, self.is_sending(&device.id), now);
        let changed = hero::show(ui, &shown, Id::new("hero").with(&device.id));
        let mut now_asked = asked.unwrap_or(Asked {
            power: None,
            intensity: None,
            at: Instant::now(),
        });
        if let Some(on) = changed.power {
            commands.push(send(Command::TurnOn { on }, pref));
            now_asked.power = Some(on);
        }
        if let Some(number) = changed.level
            && let Some(chosen) = Level::new(number)
        {
            commands.push(send(Command::SetLevel(chosen), pref));
            now_asked.intensity = Some(number);
        }
        if !commands.is_empty() {
            self.sending.insert(device.id.clone(), Instant::now());
            now_asked.at = Instant::now();
            self.asked.insert(device.id.clone(), now_asked);
        }
        ui.add_space(SECTION_GAP);

        if schedule_box(ui, device, now) {
            *sheet = Some(OpenSheet::Schedule(device.id.clone()));
        }
        ui.add_space(SECTION_GAP);
        details_box(ui, device, now);
        ui.add_space(SECTION_GAP);

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
            "Connection",
            |ui| read = read_status(ui),
            |ui| {
                row(ui, "Send Over", |ui| {
                    segmented(ui, &mut pref, &ROUTES, "Send Over")
                        .response
                        .on_hover_text(
                            "Auto: Bluetooth when the diffuser is in range, otherwise WiFi",
                        );
                });
            },
        );
        // A read writes nothing and may say nothing, so it isn't waited for.
        if read {
            commands.push(UiCommand::ReadStatus {
                device_id: device.id.clone(),
                pref,
            });
        }
        ui.add_space(SECTION_GAP);

        self.prefs.insert(device.id.clone(), pref);
        commands
    }
}

/// The diffuser's name in Title 1 and, under it, its model and whether it answers, in
/// words: "Air 2, answering over WiFi". The model is left out when it is the name.
fn header(ui: &mut egui::Ui, online: Online, device: &DeviceView) {
    let palette = Palette::of(ui);
    let name = display_name(device);
    // The two lines close, as a title and its subtitle.
    ui.spacing_mut().item_spacing.y = 2.0;
    ui.label(
        RichText::new(&name)
            .text_style(title1())
            .color(palette.label),
    );
    ui.label(RichText::new(header_line(device, &name, online)).color(palette.secondary_label));
    ui.spacing_mut().item_spacing.y = GRID;
}

/// The line under the name: the model, then whether it answers its pings.
fn header_line(device: &DeviceView, name: &str, online: Online) -> String {
    let answers = match online {
        Online::Online => "answering over WiFi",
        Online::Waiting => "pinged, waiting for an answer",
        Online::Offline => "not answering pings",
        Online::Unknown => "not pinged yet",
    };
    let model = match Model::of(device).name() {
        Some(model) => model.to_owned(),
        None => device
            .info
            .as_ref()
            .map_or_else(String::new, |info| info.model_name.clone()),
    };
    if model.is_empty() || model == name {
        capitalized(answers)
    } else {
        format!("{model}, {answers}")
    }
}

/// "Schedule": today's times when this app knows them (it sent the schedule this
/// session), and what they say it should be doing now. True when Edit Schedule… was
/// clicked.
fn schedule_box(ui: &mut egui::Ui, device: &DeviceView, now: i64) -> bool {
    let palette = Palette::of(ui);
    let mut edit = false;
    grouped_with(
        ui,
        "Schedule",
        |ui| edit = plain_button(ui, "Edit Schedule…").clicked(),
        |ui| {
            // Today's times when this app knows them; else why it doesn't.
            match &device.sent_profile {
                Some(profile) => {
                    free_row(ui, |ui| today_strip(ui, profile, now, palette.grouped));
                }
                None => {
                    row(ui, "Sent from this Mac", |ui| {
                        ui.label(RichText::new("Nothing yet").color(palette.secondary_label));
                    });
                }
            }
        },
    );
    let line = expected_line(device, now)
        .unwrap_or_else(|| "The diffuser keeps its schedule and runs it on its own.".to_owned());
    footnote(ui, &line);
    edit
}

/// A reading's color: the label's, the secondary color once it is old, orange while what
/// was sent is waited for.
fn tone_color(palette: &Palette, tone: Tone) -> egui::Color32 {
    match tone {
        Tone::Fresh => palette.label,
        Tone::Stale => palette.secondary_label,
        Tone::Pending => palette.orange,
    }
}

/// "Details": what the diffuser runs on, its WiFi signal (bars and words, the dBm in the
/// tooltip), when it last said anything, its firmware and its ID.
fn details_box(ui: &mut egui::Ui, device: &DeviceView, now: i64) {
    let palette = Palette::of(ui);
    let [_, _, source, _] = reading_rows(device, now);
    grouped(ui, "Details", |ui| {
        row(ui, "Power source", |ui| {
            let (value, note) = source.parts();
            let value = RichText::new(capitalized(value)).color(tone_color(&palette, source.tone));
            value_with_note(ui, value, note);
        });
        row(ui, "WiFi signal", |ui| wifi_signal(ui, device, now));
        row(ui, "Last report", |ui| {
            let heard = [
                device.heard_wifi.map(|at| (at, Via::Wifi)),
                device.heard_ble.map(|at| (at, Via::Bluetooth)),
            ]
            .into_iter()
            .flatten()
            .max_by_key(|(at, _)| *at);
            match heard {
                Some((at, via)) => {
                    let value = RichText::new(capitalized(&ago(at, now))).color(palette.label);
                    let note = format!("over {}", via_name(via));
                    value_with_note(ui, value, Some(&note));
                }
                None => {
                    let value = RichText::new("Nothing heard yet").color(palette.secondary_label);
                    value_with_note(ui, value, None);
                }
            }
        });
        row(ui, "Firmware", |ui| {
            let firmware = device
                .info
                .as_ref()
                .map(|info| info.firmware.as_str())
                .filter(|firmware| !firmware.is_empty());
            let (text, color) = match firmware {
                Some(firmware) => (firmware, palette.label),
                None => ("Not reported yet", palette.secondary_label),
            };
            value_with_note(ui, RichText::new(text).color(color), None);
        });
        row(ui, "Device ID", |ui| {
            value_with_note(ui, RichText::new(&device.id).color(palette.label), None);
        });
    });
}

/// The WiFi signal's row value: the words and when they were reported, then four bars
/// before them. The dBm are in the tooltip.
fn wifi_signal(ui: &mut egui::Ui, device: &DeviceView, now: i64) {
    let palette = Palette::of(ui);
    let Some(reading) = device.readings.get(&Field::Rssi) else {
        let value = RichText::new("Not reported yet").color(palette.secondary_label);
        value_with_note(ui, value, None);
        return;
    };
    let quality = signal_quality(reading.value);
    let (text, stale) = reading_text(Some(reading), now, |dbm| {
        signal_quality(dbm).words().to_owned()
    });
    let note = text.split_once(" · ").map(|(_, note)| note);
    let color = if stale {
        palette.secondary_label
    } else {
        palette.label
    };
    let words = value_with_note(ui, RichText::new(quality.words()).color(color), note);
    let (rect, bars) = ui.allocate_exact_size(vec2(18.0, words.rect.height()), Sense::hover());
    let first_line = ui.text_style_height(&egui::TextStyle::Body);
    let bars_rect = egui::Rect::from_min_size(
        rect.min + vec2(0.0, (first_line - 11.0) / 2.0 + 0.5),
        vec2(16.0, 11.0),
    );
    let on = if stale { palette.gray } else { palette.label };
    signal_bars(
        ui.painter(),
        bars_rect,
        super::glyphs::solid(on, palette.grouped),
        Palette::tint(palette.label, 15),
        quality.bars(),
    );
    let dbm = signal(reading.value);
    words.on_hover_text(&dbm);
    bars.on_hover_text(dbm);
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
    fn a_diffuser_known_over_bluetooth_can_be_added_under_its_name() {
        let mut d = device();
        assert_eq!(
            add_target(&d),
            None,
            "no Bluetooth ID: nothing to add it by"
        );
        d.ble_id = Some("ble-1".to_owned());
        d.info = Some(scent_core::DeviceInfo {
            manufacturer_id: "03".into(),
            model_name: "Hvac".into(),
            variant_name: "Black".into(),
            device_id: ID.into(),
            firmware: "2.30".into(),
            wifi_configured: true,
            wifi_status: 2,
        });
        assert_eq!(
            add_target(&d),
            Some(Target {
                ble_id: "ble-1".to_owned(),
                name: "HVAC 2".to_owned()
            })
        );
        d.label = "Hallway".to_owned();
        assert_eq!(add_target(&d).unwrap().name, "Hallway");
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
    fn a_failure_that_names_no_command_ends_both() {
        let held = || Asked {
            power: Some(true),
            intensity: Some(2),
            at: Instant::now(),
        };
        let failed = |what: &str| Event::Failed {
            device_id: Some(ID.to_owned()),
            what: what.to_owned(),
            error: "The hub lost its jobs.".to_owned(),
            at: 0,
        };
        // The hub's own failure, when a diffuser's queue stopped and took its jobs.
        let mut panel = DevicePanel::default();
        panel.asked.insert(ID.to_owned(), held());
        panel.on_event(&failed("Hub"));
        assert!(panel.asked(ID).is_none());
        // Any other name the panel doesn't know.
        panel.asked.insert(ID.to_owned(), held());
        panel.on_event(&failed("Something new"));
        assert!(panel.asked(ID).is_none());
        // A failed ping, read or schedule is about something else.
        panel.asked.insert(ID.to_owned(), held());
        for other in [
            "WiFi PING",
            "Bluetooth read status",
            "read status",
            "Schedule",
        ] {
            panel.on_event(&failed(other));
        }
        let asked = panel.asked(ID).unwrap();
        assert_eq!((asked.power, asked.intensity), (Some(true), Some(2)));
    }

    #[test]
    fn every_command_the_hub_can_name_is_known() {
        let profile = Profile {
            always_on: AlwaysOn {
                enable: false,
                level: Level::new(1).unwrap(),
            },
            entries: vec![],
            timezone_hours: 0.0,
            flag: ScheduleFlag::Replace,
        };
        let secret = || scent_core::Secret::new(String::new());
        let commands = [
            Command::TurnOn { on: true },
            Command::SetLevel(Level::new(2).unwrap()),
            Command::Ping { userid: None },
            Command::GetStatus,
            Command::SyncTime { server_time: 0 },
            Command::SaveMqtt {
                broker: String::new(),
                username: String::new(),
                password: secret(),
            },
            Command::WifiConnect {
                ssid: String::new(),
                password: secret(),
            },
            Command::SoftRestart,
            Command::UpdateProfile(profile),
        ];
        for cmd in &commands {
            for via in [Via::Bluetooth, Via::Wifi] {
                let action = cmd.action(via);
                assert_eq!(failed_job(action), Job::Command(action));
                let named = format!("{} {action}", super::super::format::via_name(via));
                assert_eq!(failed_job(&named), Job::Command(action));
            }
        }
        assert_eq!(failed_job("Bluetooth read status"), Job::Other);
        assert_eq!(failed_job("Hub"), Job::Unknown);
    }

    #[test]
    fn the_switch_shows_the_power_sent_then_the_power_reported() {
        let mut d = device();
        assert_eq!(shown_power(&d), None);
        read(&mut d, Field::Power, 1, 60, Via::Wifi);
        assert_eq!(shown_power(&d), Some(true));
        d.sent = Some(Sent {
            power: Some(false),
            intensity: None,
            at: NOW - 5,
        });
        assert_eq!(shown_power(&d), Some(false));
    }

    #[test]
    fn the_hero_shows_what_is_on_its_way_then_what_was_sent_then_what_was_reported() {
        let mut d = device();
        d.label = "Bedroom".to_owned();
        read(&mut d, Field::Power, 1, 20, Via::Wifi);
        read(&mut d, Field::Intensity, 2, 60, Via::Bluetooth);
        let hero = hero_state(&d, None, false, NOW);
        assert_eq!(hero.sentence(), "Bedroom, diffusing at Medium intensity");
        assert_eq!(hero.note, "Reported just now over WiFi");
        assert!(!hero.pending && !hero.sending && !hero.stale);
        // Sent and not reported on: shown, and waited for.
        d.sent = Some(Sent {
            power: None,
            intensity: Some(3),
            at: NOW - 2,
        });
        let hero = hero_state(&d, None, false, NOW);
        assert_eq!((hero.level, hero.pending), (Some(3), true));
        assert_eq!(hero.note, "Sent, waiting for the diffuser");
        // On its way: what the person set, and "Sending…".
        let asked = Asked {
            power: Some(false),
            intensity: None,
            at: Instant::now(),
        };
        let hero = hero_state(&d, Some(asked), true, NOW);
        assert_eq!(hero.state_words(), "Sending…");
        assert_eq!((hero.power, hero.level), (Some(false), Some(3)));
        // An old report is greyed; an intensity of 0 (it is off) is no intensity.
        let mut old = device();
        read(&mut old, Field::Power, 0, 900, Via::Wifi);
        read(&mut old, Field::Intensity, 0, 900, Via::Wifi);
        let hero = hero_state(&old, None, false, NOW);
        assert!(hero.stale);
        assert_eq!((hero.power, hero.level), (Some(false), None));
        assert_eq!(hero.note, "Reported 15 min ago over WiFi");
    }

    #[test]
    fn the_header_names_the_model_unless_it_is_the_name() {
        let mut d = device();
        d.info = Some(scent_core::DeviceInfo {
            manufacturer_id: "01".into(),
            model_name: "Diffuser".into(),
            variant_name: "Black".into(),
            device_id: ID.into(),
            firmware: "2.30".into(),
            wifi_configured: true,
            wifi_status: 3,
        });
        assert_eq!(
            header_line(&d, "Air 2", Online::Online),
            "Answering over WiFi"
        );
        assert_eq!(
            header_line(&d, "Bedroom", Online::Offline),
            "Air 2, not answering pings"
        );
        assert_eq!(
            header_line(&device(), "Diffuser CE01", Online::Unknown),
            "Not pinged yet"
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
