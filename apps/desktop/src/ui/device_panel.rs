//! The open diffuser: its name and whether it answers, then the diffusion hero (what it
//! is doing, and the power and intensity to change it), then grouped boxes, as in
//! System Settings: "Schedule" (what this app sent, today's times, and Edit Schedule…),
//! "Details" (its name and room, to change, then the power source, the WiFi signal, the
//! last report, the firmware and the ID) and "Connection" (Send Over, Read Status, and Set
//! Up Again… for a diffuser on our broker).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{self, Id, Key, RichText, Sense, vec2};
use scent_core::{Command, Level, Online, Via, expected_state, is_stale};

use super::OpenSheet;
use super::add_sheet::Target;
use super::format::{
    Model, Said, ago, capitalized, display_name, expected_text, fallback_name, given_name,
    name_from_model, power_source, reading_text, signal, signal_quality, via_name,
};
use super::glyphs::{signal_bars, solid};
use super::hero::{self, HeroState};
use super::log_summary::failure_note;
use super::theme::{GRID, Palette, SECTION_GAP, title1};
use super::timeline::today_strip;
use super::widgets::{
    self, NoticeKind, combo_field, footnote, free_row, grouped, grouped_with, notice, notice_with,
    one_line, plain_button, row, segmented, text_field, value_with_note,
};
use crate::hub::{UiCommand, on_our_broker};
use crate::route::TransportPref;
use crate::state::{AppState, DeviceView, Edit, Event, Field, NAME_MAX_CHARS};

/// How long "Sending…" stays up if no answer ever comes. A Bluetooth attempt is over in
/// well under this, and the hub gives up on one itself after 30 s.
const SENDING_GIVE_UP: Duration = Duration::from_secs(40);

/// The "Send Over" choices.
const ROUTES: [(TransportPref, &str); 3] = [
    (TransportPref::Auto, "Auto"),
    (TransportPref::Wifi, "WiFi"),
    (TransportPref::Bluetooth, "Bluetooth"),
];

/// The signal bars beside the WiFi signal's word, and the space between them.
const BARS: egui::Vec2 = vec2(16.0, 11.0);
const BARS_GAP: f32 = 6.0;

/// What the hero says under the state while what was sent is waited for.
const WAITING: &str = "Sent, waiting for the diffuser";

/// What Set Up Again… does, under the Connection box.
const SET_UP_AGAIN: &str = "Points it at the broker in Settings and restarts it.";

/// The width of the Name and Room fields in Details, as the add sheet's.
const FIELD_WIDTH: f32 = super::add_sheet::FIELD_WIDTH;

/// What the Room field shows when the diffuser is in none, and what its list button is
/// called.
pub const ROOM_HINT: &str = "None";
pub const CHOOSE_ROOM: &str = "Choose a Room";

/// How a reading looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Fresh,
    /// Reported more than 300 s ago: in the secondary label color.
    Stale,
}

/// A reading for a Details row: what it says, and when and over what it was reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadingRow {
    pub name: &'static str,
    pub value: String,
    pub note: Option<String>,
    pub tone: Tone,
}

fn row_of(name: &'static str, said: Said) -> ReadingRow {
    ReadingRow {
        name,
        value: said.value,
        note: said.note,
        tone: if said.stale { Tone::Stale } else { Tone::Fresh },
    }
}

/// The readings in Details: the power source, from two readings, the newer of which tells
/// its age; and the WiFi signal, in words.
pub fn detail_rows(device: &DeviceView, now: i64) -> [ReadingRow; 2] {
    let charging = device.readings.get(&Field::Charging);
    let battery = device.readings.get(&Field::Battery);
    let newest = [charging, battery]
        .into_iter()
        .flatten()
        .max_by_key(|r| r.at);
    let source = reading_text(newest, now, |_| {
        power_source(charging.map(|r| r.value), battery.map(|r| r.value))
    });
    let wifi = reading_text(device.readings.get(&Field::Rssi), now, |dbm| {
        signal_quality(dbm).words().to_owned()
    });
    [row_of("Power source", source), row_of("WiFi signal", wifi)]
}

/// What the schedule this app last sent says the diffuser should be doing now. `None`
/// when this Mac has never sent it one (the last one it sent is saved with the
/// diffusers): the diffuser may hold one, but we don't know what it says.
pub fn expected_line(device: &DeviceView, now: i64) -> Option<String> {
    let profile = device.sent_profile.as_ref()?;
    Some(expected_text(&expected_state(profile, now)))
}

/// The add sheet for a diffuser known over Bluetooth, named as it is, or after its model,
/// and in the room it is in: to put it on our broker (Add to Broker…), or to point it at
/// the broker in Settings again once it is there (Set Up Again…).
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
    let room = device.room.clone().unwrap_or_default();
    Some(Target { ble_id, name, room })
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
        failed: false,
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

/// What is being typed in a Details field: for which diffuser, what the field held when the
/// typing began, and what it holds now.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Draft {
    device_id: String,
    was: String,
    text: String,
}

/// What a Details field for `device_id` shows: what is being typed in it, else `stored`.
fn shown_text(draft: Option<&Draft>, device_id: &str, stored: &str) -> String {
    match draft {
        Some(draft) if draft.device_id == device_id => draft.text.clone(),
        _ => stored.to_owned(),
    }
}

/// Follows the typing in a Details field for `device_id`, which shows `stored` when it
/// isn't being typed in and held `text` this frame. While the field has the focus
/// (`focused`), the typing is kept in `draft`. Once it hasn't, the typing is over: Escape drops it, and
/// otherwise it is what to change the field to, if that is a change. Typing left in another
/// diffuser's field, as this one was chosen, is over too.
fn typed(
    draft: &mut Option<Draft>,
    device_id: &str,
    stored: &str,
    text: String,
    focused: bool,
    escape: bool,
) -> Option<Draft> {
    let over = match draft.take() {
        Some(mine) if mine.device_id == device_id => {
            let mine = Draft { text, ..mine };
            if focused {
                *draft = Some(mine);
                return None;
            }
            (!escape).then_some(mine)
        }
        left => {
            if focused {
                *draft = Some(Draft {
                    device_id: device_id.to_owned(),
                    was: stored.to_owned(),
                    text,
                });
            }
            left
        }
    };
    over.filter(|draft| draft.text != draft.was)
}

#[derive(Default)]
pub struct DevicePanel {
    /// The "Send Over" choice for each diffuser. Auto until it is changed.
    prefs: HashMap<String, TransportPref>,
    /// The diffusers with a command on its way, and since when. A guess: the first
    /// `Sent` or `Failed` for a diffuser ends it, even if more commands are queued.
    sending: HashMap<String, Instant>,
    asked: HashMap<String, Asked>,
    /// What last failed for each diffuser, for its hero to say until the next send to it
    /// or report from it: with the log hidden, a failure is never silent.
    failures: HashMap<String, Failure>,
    /// The name and the room being typed in Details, until the field is left.
    name: Option<Draft>,
    room: Option<Draft>,
}

/// A failure as the hero says it, and when it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Failure {
    note: String,
    at: i64,
}

/// What the hero says while a command is on its way, by the route it was given.
fn sending_note(pref: TransportPref) -> &'static str {
    match pref {
        TransportPref::Bluetooth => "Sending over Bluetooth…",
        TransportPref::Wifi => "Sending over WiFi…",
        TransportPref::Auto => "Sending over Bluetooth or WiFi…",
    }
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
                self.failures.remove(device_id);
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
                error,
                at,
            } => {
                self.sending.remove(device_id);
                if let Some(note) = failure_note(what, error) {
                    let failure = Failure { note, at: *at };
                    self.failures.insert(device_id.clone(), failure);
                }
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

    /// What last failed for `device`, unless it has reported since.
    fn failure(&self, device: &DeviceView) -> Option<&str> {
        let failure = self.failures.get(&device.id)?;
        let reported = device
            .readings
            .values()
            .any(|reading| reading.at >= failure.at);
        (!reported).then_some(failure.note.as_str())
    }

    /// What the hero shows for `device`, with what is on its way to it (sent over
    /// `pref`) and what last failed.
    fn hero(&self, device: &DeviceView, pref: TransportPref, now: i64) -> HeroState {
        let sending = self.is_sending(&device.id);
        let mut hero = hero_state(device, self.asked(&device.id), sending, now);
        if sending {
            sending_note(pref).clone_into(&mut hero.note);
        } else if let Some(note) = self.failure(device) {
            note.clone_into(&mut hero.note);
            hero.failed = true;
        }
        hero
    }

    /// The Name and Room rows of Details: the name it was given (its model's name stands
    /// in the empty field), and the room it is in, typed or picked from the rooms in use.
    /// Returns the changes made in them.
    fn naming_rows(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        device: &DeviceView,
    ) -> Vec<Edit> {
        let mut edits = Vec::new();
        let escape = ui.input(|input| input.key_pressed(Key::Escape));
        let id = device.id.as_str();
        row(ui, "Name", |ui| {
            let stored = given_name(device).unwrap_or_default();
            let hint = fallback_name(device);
            let field = widgets::Field {
                id: Id::new("details_name").with(id),
                label: "Name",
                hint: &hint,
                width: FIELD_WIDTH,
                max_chars: NAME_MAX_CHARS,
            };
            let mut text = shown_text(self.name.as_ref(), id, stored);
            let focused = text_field(ui, &mut text, &field).has_focus();
            if let Some(done) = typed(&mut self.name, id, stored, text, focused, escape) {
                edits.push(Edit::Name {
                    device_id: done.device_id,
                    name: done.text,
                });
            }
        });
        row(ui, "Room", |ui| {
            let stored = device.room.as_deref().unwrap_or_default();
            let field = widgets::Field {
                id: Id::new("details_room").with(id),
                label: "Room",
                hint: ROOM_HINT,
                width: FIELD_WIDTH,
                max_chars: NAME_MAX_CHARS,
            };
            let mut text = shown_text(self.room.as_ref(), id, stored);
            let combo = combo_field(ui, &mut text, &field, &state.rooms(), CHOOSE_ROOM);
            let focused = combo.field.has_focus();
            if let Some(done) = typed(&mut self.room, id, stored, text, focused, escape) {
                edits.push(Edit::Room {
                    device_id: done.device_id,
                    room: done.text,
                });
            }
            if let Some(room) = combo.picked {
                edits.push(Edit::Room {
                    device_id: id.to_owned(),
                    room,
                });
            }
        });
        edits
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

    /// Ends the typing in Details when the panel isn't shown, such as when a diffuser found
    /// nearby is chosen: what was typed is kept, as it is when the field is left.
    pub fn end_typing(&mut self) -> Vec<Edit> {
        let name = self.name.take().filter(|draft| draft.text != draft.was);
        let room = self.room.take().filter(|draft| draft.text != draft.was);
        let names = name.map(|draft| Edit::Name {
            device_id: draft.device_id,
            name: draft.text,
        });
        let rooms = room.map(|draft| Edit::Room {
            device_id: draft.device_id,
            room: draft.text,
        });
        names.into_iter().chain(rooms).collect()
    }

    /// Shows the diffuser and returns the commands the controls asked for. `sheet` is set
    /// when Edit Schedule…, Add to Broker… or Set Up Again… was clicked, and the changes
    /// made to its name and room go to `edits`.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        device: &DeviceView,
        now: i64,
        sheet: &mut Option<OpenSheet>,
        edits: &mut Vec<Edit>,
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
        let shown = self.hero(device, pref, now);
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
            // A new try: what failed before is no longer the news.
            self.failures.remove(&device.id);
            self.sending.insert(device.id.clone(), Instant::now());
            now_asked.at = Instant::now();
            self.asked.insert(device.id.clone(), now_asked);
        }
        ui.add_space(SECTION_GAP);

        if schedule_box(ui, device, now) {
            *sheet = Some(OpenSheet::Schedule(device.id.clone()));
        }
        ui.add_space(SECTION_GAP);
        details_box(ui, device, now, |ui| {
            edits.extend(self.naming_rows(ui, state, device));
        });
        ui.add_space(SECTION_GAP);

        let read_status = |ui: &mut egui::Ui| {
            plain_button(ui, "Read Status")
                .on_hover_text(
                    "Asks the diffuser for its state the way Send Over says. Over Bluetooth this reads what it last reported.",
                )
                .clicked()
        };
        // Once it is on our broker, the notice and its Add to Broker… are gone: this is
        // the way to point it at the broker again, after the broker changes in Settings.
        let again = on_our_broker(device).then(|| add_target(device)).flatten();
        let mut read = false;
        let mut set_up = false;
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
                if again.is_some() {
                    row(ui, "Broker", |ui| {
                        set_up = plain_button(ui, "Set Up Again…").clicked();
                    });
                }
            },
        );
        if let Some(target) = again {
            footnote(ui, SET_UP_AGAIN);
            if set_up {
                *sheet = Some(OpenSheet::Add(target));
            }
        }
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

/// "Schedule": today's times when this app knows them (the last schedule this Mac sent,
/// saved with the diffusers), and what they say it should be doing now. True when Edit
/// Schedule… was clicked.
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

/// A reading's color: the label's, or the secondary color once it is old.
fn tone_color(palette: &Palette, tone: Tone) -> egui::Color32 {
    match tone {
        Tone::Fresh => palette.label,
        Tone::Stale => palette.secondary_label,
    }
}

/// "Details": the rows `naming_rows` adds first (its name and room), then what the
/// diffuser runs on, its WiFi signal (bars and words, the dBm in the tooltip), when it last
/// said anything, its firmware and its ID.
fn details_box(
    ui: &mut egui::Ui,
    device: &DeviceView,
    now: i64,
    naming_rows: impl FnOnce(&mut egui::Ui),
) {
    let palette = Palette::of(ui);
    let [source, wifi] = detail_rows(device, now);
    let value = |reading: &ReadingRow| {
        RichText::new(capitalized(&reading.value)).color(tone_color(&palette, reading.tone))
    };
    grouped(ui, "Details", |ui| {
        naming_rows(ui);
        row(ui, "Power source", |ui| {
            value_with_note(ui, value(&source), source.note.as_deref());
        });
        row(ui, "WiFi signal", |ui| {
            let shown = value_with_note(ui, value(&wifi), wifi.note.as_deref());
            if let Some(reading) = device.readings.get(&Field::Rssi) {
                signal_beside(ui, &shown, &wifi, reading.value);
            }
        });
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

/// The signal bars for `dbm`, on the first line of the WiFi signal's value (`shown`),
/// just before its word, taking no room of their own; the dBm in the whole value's
/// tooltip.
fn signal_beside(ui: &egui::Ui, shown: &egui::Response, wifi: &ReadingRow, dbm: i64) {
    let palette = Palette::of(ui);
    let word = one_line(
        ui,
        capitalized(&wifi.value),
        egui::TextStyle::Body,
        f32::INFINITY,
    );
    let line = word.size().y;
    let right = shown.rect.right() - word.size().x - BARS_GAP;
    let bars = egui::Rect::from_min_size(
        egui::pos2(
            right - BARS.x,
            shown.rect.top() + (line - BARS.y) / 2.0 + 0.5,
        ),
        BARS,
    );
    let on = match wifi.tone {
        Tone::Stale => palette.gray,
        Tone::Fresh => palette.label,
    };
    signal_bars(
        ui.painter(),
        bars,
        solid(on, palette.grouped),
        Palette::tint(palette.label, 15),
        signal_quality(dbm).bars(),
    );
    shown
        .clone()
        .union(ui.interact(bars, shown.id.with("bars"), Sense::hover()))
        .on_hover_text(signal(dbm));
}

#[cfg(test)]
mod tests {
    use scent_core::{AlwaysOn, Profile, ScheduleFlag, Via};

    use super::*;
    use crate::state::{Reading, Sent};

    const ID: &str = "TESTDEVICE01";
    const NOW: i64 = 1_792_510_200;

    fn device() -> DeviceView {
        DeviceView::new(ID)
    }

    fn read(device: &mut DeviceView, field: Field, value: i64, age: i64, via: Via) {
        let at = NOW - age;
        device.readings.insert(field, Reading { value, at, via });
    }

    fn row<'a>(rows: &'a [ReadingRow; 2], name: &str) -> &'a ReadingRow {
        rows.iter().find(|row| row.name == name).unwrap()
    }

    /// A row's value and note, as one line for the tests to compare.
    fn said(row: &ReadingRow) -> (&str, Option<&str>) {
        (&row.value, row.note.as_deref())
    }

    #[test]
    fn nothing_reported_says_so() {
        let rows = detail_rows(&device(), NOW);
        let names: Vec<&str> = rows.iter().map(|row| row.name).collect();
        assert_eq!(names, ["Power source", "WiFi signal"]);
        for row in &rows {
            assert_eq!(said(row), ("not reported yet", None));
            assert_eq!(row.tone, Tone::Fresh);
        }
    }

    #[test]
    fn readings_show_their_age_and_source_and_grey_out_when_stale() {
        let mut d = device();
        read(&mut d, Field::Charging, 2, 10, Via::Bluetooth);
        read(&mut d, Field::Rssi, -66, 301, Via::Wifi);
        let rows = detail_rows(&d, NOW);
        let source = row(&rows, "Power source");
        assert_eq!(
            said(source),
            ("plugged in", Some("reported just now over Bluetooth"))
        );
        assert_eq!(source.tone, Tone::Fresh);
        let wifi = row(&rows, "WiFi signal");
        assert_eq!(said(wifi), ("Good", Some("reported 5 min ago over WiFi")));
        assert_eq!(wifi.tone, Tone::Stale);
    }

    #[test]
    fn the_power_source_takes_its_age_from_the_newer_reading() {
        let mut d = device();
        read(&mut d, Field::Charging, 2, 400, Via::Wifi);
        read(&mut d, Field::Battery, 200, 90, Via::Bluetooth);
        let rows = detail_rows(&d, NOW);
        let source = row(&rows, "Power source");
        assert_eq!(
            said(source),
            ("plugged in", Some("reported 2 min ago over Bluetooth"))
        );
        assert_eq!(source.tone, Tone::Fresh);

        read(&mut d, Field::Battery, 200, 400, Via::Bluetooth);
        assert_eq!(row(&detail_rows(&d, NOW), "Power source").tone, Tone::Stale);
    }

    #[test]
    fn a_battery_level_alone_and_a_charging_value_we_cannot_read() {
        let mut d = device();
        read(&mut d, Field::Battery, 80, 90, Via::Wifi);
        let rows = detail_rows(&d, NOW);
        assert_eq!(
            said(row(&rows, "Power source")),
            ("battery 80%", Some("reported 2 min ago over WiFi"))
        );

        let mut d = device();
        read(&mut d, Field::Charging, 7, 600, Via::Wifi);
        let rows = detail_rows(&d, NOW);
        let source = row(&rows, "Power source");
        assert_eq!(said(source), ("not reported yet", None));
        assert_eq!(source.tone, Tone::Fresh);
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
                name: "HVAC 2".to_owned(),
                room: String::new(),
            })
        );
        d.label = "Hallway".to_owned();
        d.room = Some("Upstairs".to_owned());
        let target = add_target(&d).unwrap();
        assert_eq!(
            (target.name.as_str(), target.room.as_str()),
            ("Hallway", "Upstairs")
        );
    }

    #[test]
    fn typing_in_a_field_is_kept_until_it_is_left_and_escape_drops_it() {
        let mut draft = None;
        let mut typing = |device: &str, text: &str, focused: bool, escape: bool| {
            typed(
                &mut draft,
                device,
                "Bedroom",
                text.to_owned(),
                focused,
                escape,
            )
            .map(|done| (done.device_id, done.text))
        };
        // While the field has the focus, nothing is changed.
        assert_eq!(typing(ID, "Bedr", true, false), None);
        assert_eq!(typing(ID, "Bedside", true, false), None);
        // Left, it is the change to make.
        assert_eq!(
            typing(ID, "Bedside", false, false),
            Some((ID.to_owned(), "Bedside".to_owned()))
        );
        // Then nothing more: the typing is over.
        assert_eq!(typing(ID, "Bedroom", false, false), None);
        // Left with Escape, or left as it was, nothing.
        typing(ID, "Kitchen", true, false);
        assert_eq!(typing(ID, "Kitchen", false, true), None);
        typing(ID, "Bedroom", true, false);
        assert_eq!(typing(ID, "Bedroom", false, false), None);
        // Typed for one diffuser, and left as another was chosen: made for the first.
        typing(ID, "Bedside", true, false);
        assert_eq!(
            typing("TESTDEVICE02", "", false, false),
            Some((ID.to_owned(), "Bedside".to_owned()))
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

    fn failed(what: &str, device: &str) -> Event {
        Event::Failed {
            device_id: Some(device.to_owned()),
            what: what.to_owned(),
            error: "No answer.".to_owned(),
            at: NOW,
        }
    }

    #[test]
    fn a_failure_shows_in_the_hero_until_the_next_send_or_report() {
        let mut d = device();
        read(&mut d, Field::Power, 1, 60, Via::Wifi);
        let mut panel = DevicePanel::default();
        panel.on_event(&failed("Bluetooth TURN_ON", ID));
        let hero = panel.hero(&d, TransportPref::Auto, NOW);
        assert_eq!(
            hero.note,
            "Couldn't send the power command over Bluetooth: No answer."
        );
        assert!(hero.failed);
        let palette = Palette::for_dark(false);
        assert_eq!(hero.note_color(&palette), palette.red);
        // The next send to it clears it.
        panel.on_event(&Event::Sent {
            device_id: ID.to_owned(),
            summary: String::new(),
            via: Via::Wifi,
            at: NOW + 1,
            power: Some(true),
            intensity: None,
            profile: None,
        });
        assert!(!panel.hero(&d, TransportPref::Auto, NOW + 1).failed);
        // So does the next report from it.
        panel.on_event(&failed("SET_LEVEL", ID));
        assert!(panel.hero(&d, TransportPref::Auto, NOW).failed);
        read(&mut d, Field::Intensity, 2, -1, Via::Bluetooth);
        let hero = panel.hero(&d, TransportPref::Auto, NOW + 1);
        assert!(!hero.failed);
        assert_eq!(hero.note, "Reported just now over Bluetooth");
        // A routine ping isn't the person's, and another diffuser's failure isn't this one's.
        let mut panel = DevicePanel::default();
        panel.on_event(&failed("WiFi PING", ID));
        panel.on_event(&failed("TURN_ON", "TESTDEVICE02"));
        assert!(!panel.hero(&device(), TransportPref::Auto, NOW).failed);
    }

    #[test]
    fn while_a_command_is_on_its_way_the_note_says_how() {
        let mut d = device();
        read(&mut d, Field::Power, 1, 20, Via::Wifi);
        let mut panel = DevicePanel::default();
        panel.as_sending(ID, Some(false), None);
        let note = |pref| panel.hero(&d, pref, NOW).note;
        assert_eq!(note(TransportPref::Bluetooth), "Sending over Bluetooth…");
        assert_eq!(note(TransportPref::Wifi), "Sending over WiFi…");
        assert_eq!(note(TransportPref::Auto), "Sending over Bluetooth or WiFi…");
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
