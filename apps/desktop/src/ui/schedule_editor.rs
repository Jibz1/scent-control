//! The Edit Schedule sheet: the diffuser's timetable as last sent from Scent Control (or
//! empty), to change and send again. The diffuser keeps the table and runs it on its own;
//! sending it replaces the whole table, over WiFi (see `hub::UiCommand::SendSchedule`).
//!
//! What is typed is a [`Draft`]; [`check`] says whether it makes a [`Profile`], and
//! what is wrong where when it doesn't. "Send Schedule" waits until it does. Above the
//! times, a week grid previews the draft as it is typed ([`preview`]), so that overlaps
//! and gaps show before it is sent.

use eframe::egui::{self, Align, Id, Key, KeyboardShortcut, Modifiers, RichText, ScrollArea};
use scent_core::{
    AlwaysOn, Entry, HhMm, Level, Profile, ScheduleFlag, local_weekday_and_time, trial_entry,
};

use super::clock::local_utc_offset_secs;
use super::format::display_name;
use super::theme::{GRID, Palette, TOOLBAR_HEIGHT, headline};
use super::timeline::week_grid;
use super::widgets::{
    alert, day_toggles, default_button, footnote, free_row, grouped, grouped_with, plain_button,
    problem, row, secondary, segmented, sheet, switch,
};
use crate::hub::UiCommand;
use crate::state::{AppState, DeviceView};

/// The sheet's width, and the confirmation's.
pub const WIDTH: f32 = 560.0;
const ALERT_WIDTH: f32 = 360.0;

/// What the sheet holds besides its scrolling part: its title and note, the week grid,
/// and the buttons.
const WEEK_ROOM: f32 = 410.0;

/// The width of a time field (`HH:MM`) and of a number field.
const TIME_FIELD: f32 = 52.0;
const NUMBER_FIELD: f32 = 40.0;

/// The three intensities and what the segments call them.
const LEVELS: [(u8, &str); 3] = [(1, "Low"), (2, "Medium"), (3, "High")];

/// The intensity a new time, the always-on entry and a test time start at.
const DEFAULT_LEVEL: u8 = 1;

/// How soon a test time starts, and how long it runs unless the person says otherwise.
const TEST_STARTS_IN_MIN: u16 = 3;
const TEST_MINUTES: &str = "3";
const ADD_TEST_TIME: &str = "Add a Test Time Starting in 3 Minutes";

/// How many entries a diffuser keeps: 7 on firmware up to 2.21, otherwise 100 (see
/// `docs/findings.md`, "Schedules: stored on the device"). The always-on entry is one.
const SMALL_TABLE: usize = 7;
const LARGE_TABLE: usize = 100;
const LAST_SMALL_TABLE: [u32; 2] = [2, 21];

/// The UTC offsets there are, in hours.
const OFFSETS: std::ops::RangeInclusive<f64> = -12.0..=14.0;

const START_FORMAT: &str = "Enter the start as HH:MM on a 24-hour clock, such as 07:30.";
const END_FORMAT: &str = "Enter the end as HH:MM on a 24-hour clock, such as 21:00.";
const END_FIRST: &str =
    "The end must be after the start. A time can't run past midnight: split it in two.";
const NO_DAYS: &str = "Pick at least one day.";
const BAD_DAY: &str = "Days run from Sunday to Saturday.";
const BAD_LEVEL: &str = "Pick a level: Low, Medium or High.";
const BAD_OFFSET: &str = "Enter the offset from UTC in hours, such as -5 or 5.5.";
const OFFSET_RANGE: &str = "Offsets from UTC run from -12 to +14 hours.";
const BAD_TEST_MINUTES: &str =
    "A test runs for at least 1 minute: enter a whole number of minutes.";
const PAST_MIDNIGHT: &str = "That test would run past midnight, which a time can't. Make it shorter, or add it after midnight.";

/// One timed entry as typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftEntry {
    /// `HH:MM`, as typed.
    pub start: String,
    pub end: String,
    /// 0 for Sunday to 6 for Saturday.
    pub days: Vec<u8>,
    /// 1 to 3.
    pub level: u8,
    /// Whether it runs: an entry can be kept in the table switched off.
    pub runs: bool,
}

impl DraftEntry {
    /// A new time: 08:00 to 09:00 every day, at Low, running.
    pub fn new() -> DraftEntry {
        DraftEntry {
            start: "08:00".to_owned(),
            end: "09:00".to_owned(),
            days: (0..7).collect(),
            level: DEFAULT_LEVEL,
            runs: true,
        }
    }

    fn from_entry(entry: &Entry) -> DraftEntry {
        DraftEntry {
            start: entry.start.to_string(),
            end: entry.end.to_string(),
            days: entry.weekdays.clone(),
            level: entry.level.get(),
            runs: entry.enable,
        }
    }
}

/// The schedule as typed in the sheet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    pub always_on: bool,
    pub always_level: u8,
    pub entries: Vec<DraftEntry>,
    /// The offset from UTC in hours, as typed: the diffuser's times are in that zone.
    pub offset: String,
    /// How long a test time runs, in minutes, as typed.
    pub test_minutes: String,
}

impl Draft {
    /// Nothing yet: no times, and always-on off at Low, in the zone `offset_hours` from UTC.
    pub fn empty(offset_hours: f64) -> Draft {
        Draft {
            always_on: false,
            always_level: DEFAULT_LEVEL,
            entries: Vec::new(),
            offset: format_hours(offset_hours),
            test_minutes: TEST_MINUTES.to_owned(),
        }
    }

    /// The schedule that was sent, to change.
    pub fn from_profile(profile: &Profile) -> Draft {
        Draft {
            always_on: profile.always_on.enable,
            always_level: profile.always_on.level.get(),
            entries: profile.entries.iter().map(DraftEntry::from_entry).collect(),
            offset: format_hours(profile.timezone_hours),
            test_minutes: TEST_MINUTES.to_owned(),
        }
    }
}

/// The draft as the week grid shows it while it is typed: the times that make sense so
/// far (one with a mistake is left out until it is fixed), switched on or off, and the
/// always-on entry. `None` only if no level can be made.
pub fn preview(draft: &Draft) -> Option<Profile> {
    let level = Level::new(draft.always_level).or_else(|| Level::new(DEFAULT_LEVEL))?;
    Some(Profile {
        always_on: AlwaysOn {
            enable: draft.always_on,
            level,
        },
        entries: draft
            .entries
            .iter()
            .filter_map(|entry| check_entry(entry).ok())
            .collect(),
        timezone_hours: parse_offset(&draft.offset).unwrap_or(0.0),
        flag: ScheduleFlag::Replace,
    })
}

/// What is wrong with a [`Draft`], where. Each is the first problem found there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Problems {
    pub always_on: Option<String>,
    /// For each entry, in order.
    pub entries: Vec<Option<String>>,
    /// There are more times than the diffuser keeps.
    pub count: Option<String>,
    pub offset: Option<String>,
    pub test_minutes: Option<String>,
}

/// The [`Profile`] the draft makes, replacing the whole table, or what is wrong with it.
/// `max_times` is how many times the diffuser keeps (see [`max_times`]).
pub fn check(draft: &Draft, max_times: usize) -> Result<Profile, Problems> {
    let mut problems = Problems::default();
    let always_level = Level::new(draft.always_level);
    if always_level.is_none() {
        problems.always_on = Some(BAD_LEVEL.to_owned());
    }
    let mut entries = Vec::with_capacity(draft.entries.len());
    for entry in &draft.entries {
        match check_entry(entry) {
            Ok(entry) => {
                entries.push(entry);
                problems.entries.push(None);
            }
            Err(problem) => problems.entries.push(Some(problem.to_owned())),
        }
    }
    if draft.entries.len() > max_times {
        problems.count = Some(format!(
            "This diffuser keeps at most {max_times} times. Remove some."
        ));
    }
    let offset = parse_offset(&draft.offset)
        .map_err(|problem| problems.offset = Some(problem))
        .ok();
    if let Err(problem) = parse_test_minutes(&draft.test_minutes) {
        problems.test_minutes = Some(problem);
    }
    match (always_level, offset) {
        (Some(level), Some(timezone_hours)) if problems == Problems::fine(draft) => Ok(Profile {
            always_on: AlwaysOn {
                enable: draft.always_on,
                level,
            },
            entries,
            timezone_hours,
            flag: ScheduleFlag::Replace,
        }),
        _ => Err(problems),
    }
}

impl Problems {
    /// The first problem, in the order the sheet shows them, with where it is, for the
    /// line beside Send Schedule: the problem itself may be scrolled out of view.
    pub fn first(&self) -> Option<String> {
        let at = |place: &str, problem: &Option<String>| {
            problem
                .as_ref()
                .map(|problem| format!("{place}: {problem}"))
        };
        at("Always on", &self.always_on)
            .or_else(|| at("Time zone", &self.offset))
            .or_else(|| {
                self.entries
                    .iter()
                    .enumerate()
                    .find_map(|(i, problem)| at(&format!("Time {}", i + 1), problem))
            })
            .or_else(|| at("Times", &self.count))
            .or_else(|| at("Test time", &self.test_minutes))
    }

    /// No problem anywhere in `draft`.
    fn fine(draft: &Draft) -> Problems {
        Problems {
            entries: vec![None; draft.entries.len()],
            ..Problems::default()
        }
    }
}

/// The entry `entry` makes, or the first thing wrong with it.
fn check_entry(entry: &DraftEntry) -> Result<Entry, &'static str> {
    let start = HhMm::parse(entry.start.trim()).ok_or(START_FORMAT)?;
    let end = HhMm::parse(entry.end.trim()).ok_or(END_FORMAT)?;
    if start >= end {
        return Err(END_FIRST);
    }
    if entry.days.is_empty() {
        return Err(NO_DAYS);
    }
    if entry.days.iter().any(|&day| day > 6) {
        return Err(BAD_DAY);
    }
    let level = Level::new(entry.level).ok_or(BAD_LEVEL)?;
    let mut weekdays = entry.days.clone();
    weekdays.sort_unstable();
    weekdays.dedup();
    Ok(Entry {
        start,
        end,
        weekdays,
        level,
        enable: entry.runs,
    })
}

/// The offset from UTC, in hours, as typed: such as `-5`, `+5.5` or `−3.5`.
pub fn parse_offset(text: &str) -> Result<f64, String> {
    let text = text.trim().replace('\u{2212}', "-");
    let text = text.strip_prefix('+').unwrap_or(&text);
    // Only digits, a sign and a point: no "inf", "NaN" or exponents.
    let plain = !text.is_empty()
        && text
            .bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_digit() || b == b'.' || (i == 0 && b == b'-'));
    let hours = plain
        .then(|| text.parse::<f64>().ok())
        .flatten()
        .filter(|hours| hours.is_finite())
        .ok_or_else(|| BAD_OFFSET.to_owned())?;
    if OFFSETS.contains(&hours) {
        Ok(hours)
    } else {
        Err(OFFSET_RANGE.to_owned())
    }
}

/// How long a test time runs, as typed: a whole number of minutes, at least 1.
pub fn parse_test_minutes(text: &str) -> Result<u16, String> {
    text.trim()
        .parse::<u16>()
        .ok()
        .filter(|&minutes| minutes >= 1)
        .ok_or_else(|| BAD_TEST_MINUTES.to_owned())
}

/// A time for today that starts [`TEST_STARTS_IN_MIN`] minutes after `now` in the
/// diffuser's zone (`offset_hours` from UTC) and runs `minutes`, at Low.
pub fn test_time(now: i64, offset_hours: f64, minutes: u16) -> Result<DraftEntry, String> {
    let (weekday, local) = local_weekday_and_time(now, offset_hours);
    let level = Level::new(DEFAULT_LEVEL).ok_or_else(|| BAD_LEVEL.to_owned())?;
    let entry = trial_entry(weekday, local, TEST_STARTS_IN_MIN, minutes, level)
        .map_err(|_| PAST_MIDNIGHT.to_owned())?;
    Ok(DraftEntry::from_entry(&entry))
}

/// How many times a diffuser with this firmware keeps, besides its always-on entry. A
/// firmware that can't be read gets the smaller number.
pub fn max_times(firmware: Option<&str>) -> usize {
    let version: Option<Vec<u32>> = firmware.and_then(|firmware| {
        firmware
            .split('.')
            .map(|part| part.trim().parse().ok())
            .collect()
    });
    let large = version.is_some_and(|version| version.as_slice() > LAST_SMALL_TABLE.as_slice());
    let table = if large { LARGE_TABLE } else { SMALL_TABLE };
    table - 1
}

/// The note under the time zone: whether the offset typed (`offset`) is this Mac's own
/// (`mac_hours` from UTC), and the Mac's when it isn't, as it isn't when the schedule
/// was sent from another zone or before a change of daylight saving.
pub fn zone_note(offset: &str, mac_hours: f64) -> String {
    let same = parse_offset(offset).is_ok_and(|hours| (hours - mac_hours).abs() < 1e-9);
    if same {
        "The times are in this zone, which is this Mac's.".to_owned()
    } else {
        format!(
            "The times are in this zone. This Mac is at {} now.",
            utc_offset(mac_hours)
        )
    }
}

/// An offset from UTC as people write it: `UTC-4`, `UTC+5.5`, `UTC`.
fn utc_offset(hours: f64) -> String {
    if hours == 0.0 {
        "UTC".to_owned()
    } else if hours > 0.0 {
        format!("UTC+{}", format_hours(hours))
    } else {
        format!("UTC{}", format_hours(hours))
    }
}

/// Hours as people write them: `-4`, `5.5`.
pub fn format_hours(hours: f64) -> String {
    if hours.fract() == 0.0 {
        format!("{}", hours as i64)
    } else {
        format!("{hours}")
    }
}

/// What the person did in the sheet or its confirmation this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// Cancel, Escape, ⌘W or the window's close button: the sheet closes.
    Cancel,
    /// Send Schedule: asks to confirm.
    Send,
    /// Replace, in the confirmation.
    Replace,
    /// Cancel in the confirmation: back to the sheet.
    Back,
}

/// The Edit Schedule sheet, and its confirmation.
#[derive(Default)]
pub struct ScheduleEditor {
    /// The diffuser whose schedule is open, while the sheet is.
    device_id: Option<String>,
    draft: Draft,
    /// Whether "Replace …'s stored schedule?" is up.
    confirming: bool,
    /// Why the last test time couldn't be added.
    test_problem: Option<String>,
    /// A time was just added: the times scroll to show it.
    added: bool,
    /// This Mac's offset from UTC, in hours, when the sheet opened.
    mac_hours: f64,
}

impl ScheduleEditor {
    /// Opens the sheet for `device`, with the last schedule sent to it from Scent
    /// Control, or an empty one in this Mac's time zone.
    pub fn open(&mut self, device: &DeviceView, now: i64) {
        let mac = local_utc_offset_secs(now) as f64 / 3600.0;
        self.mac_hours = mac;
        self.draft = device
            .sent_profile
            .as_ref()
            .map_or_else(|| Draft::empty(mac), Draft::from_profile);
        self.device_id = Some(device.id.clone());
        self.confirming = false;
        self.test_problem = None;
    }

    #[cfg(test)]
    pub fn is_open(&self) -> bool {
        self.device_id.is_some()
    }

    fn close(&mut self) {
        self.device_id = None;
        self.confirming = false;
        self.test_problem = None;
    }

    /// What is typed, for the snapshots to fill in.
    #[cfg(test)]
    pub fn draft_mut(&mut self) -> &mut Draft {
        &mut self.draft
    }

    /// Shows the sheet when it is open. Returns the schedule to send once the person
    /// has confirmed it; the sheet then closes.
    pub fn show(&mut self, ctx: &egui::Context, state: &AppState, now: i64) -> Option<UiCommand> {
        let device_id = self.device_id.clone()?;
        let Some(device) = state.devices.get(&device_id) else {
            self.close();
            return None;
        };
        let limit = max_times(device.info.as_ref().map(|info| info.firmware.as_str()));
        let checked = check(&self.draft, limit);
        // Keys first: Return can't both open the confirmation and answer it.
        let was_confirming = self.confirming;
        let close_window = KeyboardShortcut::new(Modifiers::COMMAND, Key::W);
        let (escape, command_w, closing) = ctx.input_mut(|input| {
            (
                input.key_pressed(Key::Escape),
                input.consume_shortcut(&close_window),
                input.viewport().close_requested(),
            )
        });
        let enter = ctx.input(|input| input.key_pressed(Key::Enter))
            && ctx.memory(|memory| memory.focused().is_none());

        let mut action = sheet(ctx, Id::new("schedule_editor"), WIDTH, |ui| {
            self.contents(ui, device, &checked, limit, now)
        });
        if command_w || closing || (escape && !was_confirming) {
            action = Some(Action::Cancel);
        } else if !was_confirming && enter && checked.is_ok() {
            action = Some(Action::Send);
        }
        if action == Some(Action::Send) && checked.is_ok() {
            self.confirming = true;
        }
        if self.confirming {
            let answer = self.confirmation(ctx, &display_name(device));
            if was_confirming {
                action = answer.or(if escape {
                    Some(Action::Back)
                } else if enter {
                    Some(Action::Replace)
                } else {
                    action
                });
            }
        }
        match (action, checked) {
            (Some(Action::Cancel), _) => {
                self.close();
                None
            }
            (Some(Action::Back), _) => {
                self.confirming = false;
                None
            }
            (Some(Action::Replace), Ok(profile)) => {
                self.close();
                Some(UiCommand::SendSchedule { device_id, profile })
            }
            _ => None,
        }
    }

    /// "Replace …'s stored schedule?", with Cancel and Replace.
    fn confirmation(&self, ctx: &egui::Context, label: &str) -> Option<Action> {
        alert(ctx, Id::new("replace_schedule"), ALERT_WIDTH, |ui| {
            let palette = Palette::of(ui);
            ui.label(
                RichText::new(format!("Replace {label}'s stored schedule?"))
                    .text_style(headline())
                    .color(palette.label),
            );
            ui.add_space(GRID / 2.0);
            secondary(
                ui,
                "Its whole timetable is replaced with this one, then it is told the time.",
            );
            ui.add_space(2.0 * GRID);
            let mut answer = None;
            egui::Sides::new().show(
                ui,
                |_| {},
                |ui| {
                    if default_button(ui, "Replace").clicked() {
                        answer = Some(Action::Replace);
                    }
                    if plain_button(ui, "Cancel").clicked() {
                        answer = Some(Action::Back);
                    }
                },
            );
            answer
        })
    }

    /// What the sheet holds: the always-on entry, the times, the time zone, and Cancel
    /// and Send Schedule.
    fn contents(
        &mut self,
        ui: &mut egui::Ui,
        device: &DeviceView,
        checked: &Result<Profile, Problems>,
        limit: usize,
        now: i64,
    ) -> Option<Action> {
        let palette = Palette::of(ui);
        let problems = checked.as_ref().err();
        ui.label(
            RichText::new("Edit Schedule")
                .text_style(headline())
                .color(palette.label),
        );
        ui.add_space(GRID / 2.0);
        secondary(
            ui,
            &format!(
                "{}'s timetable. The diffuser keeps it and runs it on its own; it goes over WiFi.",
                display_name(device)
            ),
        );
        ui.add_space(2.0 * GRID);
        // The week, as typed so far: it stays in view while the times scroll.
        if let Some(week) = preview(&self.draft) {
            grouped(ui, "Week", |ui| {
                free_row(ui, |ui| week_grid(ui, &week, palette.grouped));
            });
            ui.add_space(2.0 * GRID);
        }
        // The sheet fits the window; what doesn't fit scrolls. A sheet lays out in the
        // size it had the frame before, so the scrolling part is told its height outright
        // (`min_scrolled_height`) rather than taking the room it is given, or it would
        // grow a little each frame from the first guess.
        let room = (ui.ctx().content_rect().height() - TOOLBAR_HEIGHT - WEEK_ROOM).max(160.0);
        ScrollArea::vertical()
            .max_height(room)
            .min_scrolled_height(room)
            .auto_shrink([false, true])
            .show(ui, |ui| self.tables(ui, problems, limit, now));
        ui.add_space(2.0 * GRID);
        let mut action = None;
        // Why Send Schedule can't be pressed, beside it.
        let first = problems.and_then(Problems::first);
        egui::Sides::new().shrink_left().wrap().show(
            ui,
            |ui| {
                if let Some(first) = &first {
                    ui.add(
                        egui::Label::new(
                            RichText::new(first)
                                .text_style(super::theme::subheadline())
                                .color(palette.red),
                        )
                        .wrap(),
                    );
                }
            },
            |ui| {
                let send =
                    ui.add_enabled_ui(checked.is_ok(), |ui| default_button(ui, "Send Schedule"));
                if send.inner.clicked() {
                    action = Some(Action::Send);
                }
                if plain_button(ui, "Cancel").clicked() {
                    action = Some(Action::Cancel);
                }
            },
        );
        action
    }

    /// The boxes in the sheet's scrolling part.
    fn tables(&mut self, ui: &mut egui::Ui, problems: Option<&Problems>, limit: usize, now: i64) {
        let draft = &mut self.draft;
        grouped(ui, "Always On", |ui| {
            row(ui, "Always on", |ui| {
                switch(ui, &mut draft.always_on, "Always on")
            });
            row(ui, "Level", |ui| {
                segmented(ui, &mut draft.always_level, &LEVELS, "Level")
            });
        });
        match problems.and_then(|problems| problems.always_on.as_deref()) {
            Some(text) => problem(ui, text),
            None => footnote(ui, "Runs whenever none of the times below does."),
        };
        ui.add_space(2.0 * GRID);

        grouped(ui, "Time Zone", |ui| {
            row(ui, "Offset from UTC", |ui| {
                // Right to left: the unit, then the field before it.
                ui.label(RichText::new("hours").color(Palette::of(ui).secondary_label));
                ui.add(number_field(&mut draft.offset, "utc_offset"));
            });
        });
        match problems.and_then(|problems| problems.offset.as_deref()) {
            Some(text) => problem(ui, text),
            None => footnote(ui, &zone_note(&draft.offset, self.mac_hours)),
        };
        ui.add_space(2.0 * GRID);

        let mut remove = None;
        for (i, entry) in draft.entries.iter_mut().enumerate() {
            grouped_with(
                ui,
                &format!("Time {}", i + 1),
                |ui| {
                    if plain_button(ui, "Remove").clicked() {
                        remove = Some(i);
                    }
                },
                |ui| entry_rows(ui, entry, i),
            );
            let said = problems.and_then(|problems| problems.entries.get(i).cloned().flatten());
            if let Some(text) = said {
                problem(ui, &text);
            }
            ui.add_space(2.0 * GRID);
        }
        if std::mem::take(&mut self.added) {
            ui.scroll_to_cursor(Some(Align::Max));
        }
        if let Some(i) = remove {
            draft.entries.remove(i);
        }
        if draft.entries.is_empty() {
            secondary(
                ui,
                "No times yet. Without any, Always On says what the diffuser does.",
            );
            ui.add_space(GRID);
        }

        let room_for_more = draft.entries.len() < limit;
        let test_ready =
            parse_offset(&draft.offset).is_ok() && parse_test_minutes(&draft.test_minutes).is_ok();
        let mut add_test = false;
        ui.horizontal_wrapped(|ui| {
            let add = ui.add_enabled_ui(room_for_more, |ui| plain_button(ui, "Add Time"));
            if add.inner.clicked() {
                draft.entries.push(DraftEntry::new());
                self.added = true;
            }
            let test = ui.add_enabled_ui(room_for_more && test_ready, |ui| {
                plain_button(ui, ADD_TEST_TIME)
            });
            add_test = test.inner.clicked();
            ui.label(RichText::new("for").color(Palette::of(ui).secondary_label));
            ui.add(number_field(&mut draft.test_minutes, "test_minutes"));
            ui.label(RichText::new("min").color(Palette::of(ui).secondary_label));
        });
        let below = [
            problems.and_then(|problems| problems.count.clone()),
            problems.and_then(|problems| problems.test_minutes.clone()),
            self.test_problem.clone(),
        ];
        for text in below.into_iter().flatten() {
            problem(ui, &text);
        }
        if add_test {
            self.add_test_time(now);
        }
    }

    fn add_test_time(&mut self, now: i64) {
        let (Ok(offset), Ok(minutes)) = (
            parse_offset(&self.draft.offset),
            parse_test_minutes(&self.draft.test_minutes),
        ) else {
            return;
        };
        match test_time(now, offset, minutes) {
            Ok(entry) => {
                self.draft.entries.push(entry);
                self.test_problem = None;
                self.added = true;
            }
            Err(problem) => self.test_problem = Some(problem),
        }
    }
}

/// The rows of one time: when, on which days, how strong, and whether it runs.
fn entry_rows(ui: &mut egui::Ui, entry: &mut DraftEntry, i: usize) {
    row(ui, "Time", |ui| {
        // Right to left: the end, "to", then the start.
        ui.add(time_field(&mut entry.end, ("end", i)));
        ui.label(RichText::new("to").color(Palette::of(ui).secondary_label));
        ui.add(time_field(&mut entry.start, ("start", i)));
    });
    row(ui, "Days", |ui| day_toggles(ui, &mut entry.days, "Days"));
    row(ui, "Level", |ui| {
        segmented(ui, &mut entry.level, &LEVELS, "Level")
    });
    row(ui, "Runs", |ui| switch(ui, &mut entry.runs, "Runs"));
}

/// A field for a time of day, `HH:MM`.
fn time_field<'a>(text: &'a mut String, salt: (&str, usize)) -> egui::TextEdit<'a> {
    egui::TextEdit::singleline(text)
        .id_salt(salt)
        .desired_width(TIME_FIELD)
        .char_limit(5)
        .hint_text("HH:MM")
        .horizontal_align(Align::Center)
}

/// A short field for a number.
fn number_field<'a>(text: &'a mut String, salt: &str) -> egui::TextEdit<'a> {
    egui::TextEdit::singleline(text)
        .id_salt(salt)
        .desired_width(NUMBER_FIELD)
        .char_limit(6)
        .horizontal_align(Align::Center)
}

#[cfg(test)]
mod tests {
    use scent_core::{AlwaysOn, Entry, HhMm, Level, Profile, ScheduleFlag};

    use super::*;

    /// 2026-10-20 15:30:00 UTC, a Tuesday.
    const NOW: i64 = 1_792_510_200;

    fn entry(start: &str, end: &str, days: &[u8]) -> DraftEntry {
        DraftEntry {
            start: start.to_owned(),
            end: end.to_owned(),
            days: days.to_vec(),
            level: 2,
            runs: true,
        }
    }

    fn draft(entries: Vec<DraftEntry>) -> Draft {
        Draft {
            always_on: false,
            always_level: 1,
            entries,
            offset: "-5".to_owned(),
            test_minutes: "3".to_owned(),
        }
    }

    fn hhmm(text: &str) -> HhMm {
        HhMm::parse(text).unwrap()
    }

    #[test]
    fn a_valid_draft_is_the_profile_to_send() {
        let mut d = draft(vec![
            entry("07:00", "09:00", &[1, 2, 3, 4, 5]),
            entry("18:30", "22:00", &[0, 6]),
        ]);
        d.entries[1].runs = false;
        d.entries[1].level = 3;
        d.always_on = true;
        let profile = check(&d, 99).unwrap();
        assert_eq!(
            profile,
            Profile {
                always_on: AlwaysOn {
                    enable: true,
                    level: Level::new(1).unwrap(),
                },
                entries: vec![
                    Entry {
                        start: hhmm("07:00"),
                        end: hhmm("09:00"),
                        weekdays: vec![1, 2, 3, 4, 5],
                        level: Level::new(2).unwrap(),
                        enable: true,
                    },
                    Entry {
                        start: hhmm("18:30"),
                        end: hhmm("22:00"),
                        weekdays: vec![0, 6],
                        level: Level::new(3).unwrap(),
                        enable: false,
                    },
                ],
                timezone_hours: -5.0,
                // The whole table is replaced.
                flag: ScheduleFlag::Replace,
            }
        );
        // No times and always-on off is a valid schedule too: it clears the table.
        assert!(check(&draft(vec![]), 99).is_ok());
    }

    #[test]
    fn what_is_wrong_is_said_next_to_it() {
        // (what is typed, what is said about it)
        let entries: [(DraftEntry, &str); 9] = [
            (entry("7:00", "09:00", &[1]), "HH:MM"),
            (entry("07:00", "24:00", &[1]), "HH:MM"),
            (entry("", "09:00", &[1]), "HH:MM"),
            // Windows can't cross midnight, or be empty.
            (entry("22:00", "06:00", &[1]), "after the start"),
            (entry("09:00", "09:00", &[1]), "after the start"),
            (entry("07:00", "09:00", &[]), "at least one day"),
            (entry("07:00", "09:00", &[1, 7]), "Sunday to Saturday"),
            (entry("07:00", "09:00", &[255]), "Sunday to Saturday"),
            (
                DraftEntry {
                    level: 0,
                    ..entry("07:00", "09:00", &[1])
                },
                "level",
            ),
        ];
        for (bad, said) in entries {
            let shown = format!("{:?}", (&bad.start, &bad.end, &bad.days));
            let problems =
                check(&draft(vec![entry("06:00", "06:30", &[0]), bad]), 99).expect_err(&shown);
            assert_eq!(problems.entries[0], None, "{shown}");
            let problem = problems.entries[1].as_deref().unwrap_or_default();
            assert!(problem.contains(said), "{shown}: {problem:?}");
            assert_eq!(
                (&problems.offset, &problems.test_minutes, &problems.count),
                (&None, &None, &None),
                "{shown}"
            );
        }
    }

    #[test]
    fn the_first_problem_says_where_it_is() {
        assert_eq!(Problems::default().first(), None);
        let mut d = draft(vec![
            entry("07:00", "09:00", &[1]),
            entry("22:00", "06:00", &[1]),
        ]);
        let first = check(&d, 99).unwrap_err().first().unwrap();
        assert!(
            first.starts_with("Time 2: The end must be after the start"),
            "{first}"
        );
        d.offset = "east".to_owned();
        let first = check(&d, 99).unwrap_err().first().unwrap();
        assert!(first.starts_with("Time zone: "), "{first}");
        let mut d = draft(vec![]);
        d.test_minutes = "0".to_owned();
        let first = check(&d, 99).unwrap_err().first().unwrap();
        assert!(first.starts_with("Test time: "), "{first}");
        let over = draft(vec![entry("07:00", "08:00", &[1]); 3]);
        let first = check(&over, 2).unwrap_err().first().unwrap();
        assert!(first.starts_with("Times: "), "{first}");
    }

    #[test]
    fn the_utc_offset_is_a_real_one() {
        for (typed, hours) in [
            ("-5", -5.0),
            (" +5.5 ", 5.5),
            ("\u{2212}3.5", -3.5),
            ("0", 0.0),
            ("-12", -12.0),
            ("14", 14.0),
        ] {
            assert_eq!(parse_offset(typed), Ok(hours), "{typed:?}");
        }
        for typed in [
            "", "abc", "5h", "inf", "-inf", "NaN", "1e400", "14.5", "-13",
        ] {
            assert!(parse_offset(typed).is_err(), "{typed:?}");
            let mut d = draft(vec![]);
            d.offset = typed.to_owned();
            let problems = check(&d, 99).expect_err(typed);
            assert!(problems.offset.is_some(), "{typed:?}");
        }
    }

    #[test]
    fn a_test_runs_for_at_least_a_minute() {
        assert_eq!(parse_test_minutes("3"), Ok(3));
        assert_eq!(parse_test_minutes(" 10 "), Ok(10));
        for typed in ["0", "", "-1", "abc", "1.5", "70000"] {
            let error = parse_test_minutes(typed).expect_err(typed);
            assert!(error.contains("at least 1 minute"), "{typed:?}: {error}");
            let mut d = draft(vec![]);
            d.test_minutes = typed.to_owned();
            let problems = check(&d, 99).expect_err(typed);
            assert_eq!(problems.test_minutes.as_deref(), Some(error.as_str()));
        }
    }

    #[test]
    fn a_diffuser_holds_only_so_many_times() {
        let full = draft(vec![entry("07:00", "08:00", &[1]); 6]);
        assert!(check(&full, 6).is_ok());
        let over = draft(vec![entry("07:00", "08:00", &[1]); 7]);
        let problems = check(&over, 6).unwrap_err();
        assert!(
            problems
                .count
                .as_deref()
                .is_some_and(|said| said.contains('6')),
            "{problems:?}"
        );
        // Firmware up to 2.21 keeps 7 entries, later firmware 100; the always-on entry
        // is one of them. A firmware we can't read gets the smaller number.
        assert_eq!(max_times(Some("2.30")), 99);
        assert_eq!(max_times(Some("3.0")), 99);
        assert_eq!(max_times(Some("2.21")), 6);
        assert_eq!(max_times(Some("1.3.8")), 6);
        assert_eq!(max_times(Some("")), 6);
        assert_eq!(max_times(Some("beta")), 6);
        assert_eq!(max_times(None), 6);
    }

    #[test]
    fn the_test_time_starts_in_3_minutes_in_the_diffusers_time_zone() {
        // 11:30 on Tuesday at UTC-4.
        let test = test_time(NOW, -4.0, 3).unwrap();
        assert_eq!((test.start.as_str(), test.end.as_str()), ("11:33", "11:36"));
        assert_eq!(test.days, [2]);
        assert_eq!((test.level, test.runs), (1, true));
        // At UTC+8:30 it is midnight, Wednesday.
        let test = test_time(NOW, 8.5, 10).unwrap();
        assert_eq!((test.start.as_str(), test.end.as_str()), ("00:03", "00:13"));
        assert_eq!(test.days, [3]);
        // A test that would run past midnight can't be made.
        let late = NOW + 8 * 3600 + 25 * 60; // 23:55 UTC
        let error = test_time(late, 0.0, 3).unwrap_err();
        assert!(error.contains("midnight"), "{error}");
    }

    #[test]
    fn the_draft_starts_from_what_was_sent_or_from_nothing() {
        let sent = Profile {
            always_on: AlwaysOn {
                enable: true,
                level: Level::new(3).unwrap(),
            },
            entries: vec![Entry {
                start: hhmm("06:05"),
                end: hhmm("07:45"),
                weekdays: vec![0, 3],
                level: Level::new(2).unwrap(),
                enable: false,
            }],
            timezone_hours: 5.5,
            flag: ScheduleFlag::Replace,
        };
        let d = Draft::from_profile(&sent);
        assert_eq!(d.offset, "5.5");
        assert_eq!(check(&d, 99).unwrap(), sent);

        let empty = Draft::empty(-4.0);
        assert!(!empty.always_on);
        assert_eq!(empty.always_level, 1);
        assert!(empty.entries.is_empty());
        assert_eq!(empty.offset, "-4");
        assert_eq!(empty.test_minutes, "3");
        // A new time starts at Low, running.
        let new = DraftEntry::new();
        assert_eq!((new.level, new.runs), (1, true));
        assert!(check(&draft(vec![new]), 99).is_ok());
    }

    #[test]
    fn after_a_relaunch_the_editor_opens_from_the_schedule_saved_with_the_diffuser() {
        use crate::mqtt::BrokerSettings;
        use crate::store::{Store, TempDir, sent_schedule};
        let dir = TempDir::new("editor-after-relaunch");
        let store = Store {
            dir: dir.path().to_owned(),
        };
        let mut device = DeviceView::new("TESTDEVICE01");
        device.sent_profile = Some(sent_schedule());
        let mut state = AppState::default();
        state.devices.insert(device.id.clone(), device);
        store.save(&state, &BrokerSettings::default()).unwrap();

        // The next run: what was saved, and nothing else.
        let loaded = store.load().state;
        let mut editor = ScheduleEditor::default();
        editor.open(&loaded.devices["TESTDEVICE01"], NOW);
        assert_eq!(editor.draft, Draft::from_profile(&sent_schedule()));
        // So Send goes out with the times it holds, not an empty table.
        assert_eq!(check(&editor.draft, 99), Ok(sent_schedule()));
    }

    #[test]
    fn the_time_zone_note_names_the_macs_offset_when_it_differs() {
        assert_eq!(
            zone_note("-4", -4.0),
            "The times are in this zone, which is this Mac's."
        );
        assert_eq!(
            zone_note("-5", -4.0),
            "The times are in this zone. This Mac is at UTC-4 now."
        );
        assert_eq!(
            zone_note("-4", 5.5),
            "The times are in this zone. This Mac is at UTC+5.5 now."
        );
        assert_eq!(
            zone_note("+1", 0.0),
            "The times are in this zone. This Mac is at UTC now."
        );
    }

    #[test]
    fn the_week_previews_the_draft_as_it_is_typed() {
        let mut d = draft(vec![
            entry("07:00", "09:00", &[1, 2, 3, 4, 5]),
            entry("22:00", "06:00", &[0]),
        ]);
        d.entries[0].level = 2;
        d.always_on = true;
        let week = preview(&d).unwrap();
        // The time with a mistake waits until it is fixed; the other shows at once.
        assert_eq!(week.entries.len(), 1);
        assert_eq!(week.entries[0].weekdays, [1, 2, 3, 4, 5]);
        assert!(week.always_on.enable);
        // A time switched off is still shown (fainter), as the diffuser keeps it.
        d.entries[1].end = "23:00".to_owned();
        d.entries[1].runs = false;
        let week = preview(&d).unwrap();
        assert_eq!(week.entries.len(), 2);
        assert!(!week.entries[1].enable);
        // A level that can't be is shown at Low until it is fixed.
        d.always_level = 0;
        assert_eq!(preview(&d).unwrap().always_on.level.get(), 1);
    }

    #[test]
    fn hours_are_written_as_people_write_them() {
        assert_eq!(format_hours(-4.0), "-4");
        assert_eq!(format_hours(0.0), "0");
        assert_eq!(format_hours(5.5), "5.5");
        assert_eq!(format_hours(-3.5), "-3.5");
        assert_eq!(format_hours(5.75), "5.75");
    }
}
