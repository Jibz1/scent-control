//! A day and a week of a schedule, drawn: the "Today" strip in the open diffuser's
//! Schedule box, and the week grid that previews the Edit Schedule sheet's times. The
//! geometry is pure: times become fractions of the day, from 0 at midnight to 1 at the
//! next.
//!
//! Days are numbered as scent-core numbers them, 0 for Sunday to 6 for Saturday, which
//! is also the order of the sheet's day toggles (S M T W T F S).

use eframe::egui::{
    self, Align2, Color32, Rect, Response, Sense, Stroke, StrokeKind, TextStyle, WidgetInfo,
    WidgetType, pos2, vec2,
};
use scent_core::{HhMm, Profile, local_weekday_and_time};

use super::format::intensity;
use super::theme::{GRID, Palette, subheadline};
use super::widgets::{hairline, one_line};

const MINUTES_PER_DAY: f32 = 24.0 * 60.0;

/// The days, from Sunday (0): what the grid shows beside each row, and what is read out.
const DAYS: [(&str, &str); 7] = [
    ("S", "Sunday"),
    ("M", "Monday"),
    ("T", "Tuesday"),
    ("W", "Wednesday"),
    ("T", "Thursday"),
    ("F", "Friday"),
    ("S", "Saturday"),
];

/// The hours marked under a strip or the grid.
const MARKED_HOURS: [u32; 3] = [6, 12, 18];

/// The Today strip's bar, and the week grid's rows and the gaps between them.
const STRIP_HEIGHT: f32 = 10.0;
const GRID_ROW: f32 = 12.0;
const GRID_GAP: f32 = 4.0;
/// The column of day letters at the grid's left.
const DAY_COLUMN: f32 = 20.0;
/// The room under a strip or the grid for the hours.
const HOURS_HEIGHT: f32 = 16.0;
/// The narrowest a time is drawn, so that a few minutes still show.
const MIN_SPAN_WIDTH: f32 = 3.0;

/// One timed entry on one day, as fractions of the day.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub from: f32,
    pub to: f32,
    /// 1 to 3.
    pub level: u8,
    /// Whether it runs: an entry can be kept in the table switched off.
    pub runs: bool,
}

/// What a schedule holds for one day: its timed entries, and the always-on entry's level
/// when it is on.
#[derive(Debug, Clone, PartialEq)]
pub struct DayPlan {
    pub always_on: Option<u8>,
    pub spans: Vec<Span>,
}

/// A time of day as a fraction of the day.
fn fraction(time: HhMm) -> f32 {
    (f32::from(time.hour) * 60.0 + f32::from(time.minute)) / MINUTES_PER_DAY
}

/// The timed entries on `weekday` (0 is Sunday), switched off or not, in order of their
/// start, and the always-on entry's level when it is on.
pub fn day_plan(profile: &Profile, weekday: u8) -> DayPlan {
    let mut spans: Vec<Span> = profile
        .entries
        .iter()
        .filter(|entry| entry.weekdays.contains(&weekday))
        .map(|entry| Span {
            from: fraction(entry.start),
            to: fraction(entry.end),
            level: entry.level.get(),
            runs: entry.enable,
        })
        .collect();
    spans.sort_by(|a, b| a.from.total_cmp(&b.from));
    DayPlan {
        always_on: profile
            .always_on
            .enable
            .then_some(profile.always_on.level.get()),
        spans,
    }
}

/// When the diffuser runs on `weekday` (0 is Sunday), as fractions of the day: the whole
/// day when the always-on entry is on (it runs whenever no time does), else the times
/// that are switched on, in order. The drawings use [`day_plan`], which keeps the times
/// switched off too.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the brief's interface; tested, not drawn from")
)]
pub fn windows_for_day(profile: &Profile, weekday: u8) -> Vec<(f32, f32)> {
    let plan = day_plan(profile, weekday);
    if plan.always_on.is_some() {
        return vec![(0.0, 1.0)];
    }
    plan.spans
        .iter()
        .filter(|span| span.runs)
        .map(|span| (span.from, span.to))
        .collect()
}

/// Where now is in the day, `local_minutes` after midnight in the diffuser's zone.
pub fn now_fraction(local_minutes: u32) -> f32 {
    (local_minutes as f32 / MINUTES_PER_DAY).min(1.0)
}

/// One entry in words: "07:00 to 09:00 at Medium", and ", switched off" when it is.
fn entry_words(entry: &scent_core::Entry) -> String {
    let off = if entry.enable { "" } else { ", switched off" };
    format!(
        "{} to {} at {}{off}",
        entry.start,
        entry.end,
        intensity(i64::from(entry.level.get()))
    )
}

/// The always-on entry in words, when it is on: "the rest of the day" after times, "all
/// day" alone (`span` is "day" or "week").
fn always_on_words(profile: &Profile, after_times: bool, span: &str) -> Option<String> {
    profile.always_on.enable.then(|| {
        let level = intensity(i64::from(profile.always_on.level.get()));
        if after_times {
            let rest = if span == "day" { "day" } else { "time" };
            format!("always on at {level} the rest of the {rest}")
        } else {
            format!("always on at {level} all {span}")
        }
    })
}

/// One day of the schedule in words, for VoiceOver: its times, then the always-on entry.
pub fn day_summary(profile: &Profile, weekday: u8) -> String {
    let mut parts: Vec<String> = profile
        .entries
        .iter()
        .filter(|entry| entry.weekdays.contains(&weekday))
        .map(entry_words)
        .collect();
    parts.extend(always_on_words(profile, !parts.is_empty(), "day"));
    if parts.is_empty() {
        return "Nothing scheduled".to_owned();
    }
    super::format::capitalized(&parts.join("; "))
}

/// The week in words, day by day from Sunday, for VoiceOver: "Monday 07:00 to 09:00 at
/// Medium; …", then the always-on entry.
pub fn week_summary(profile: &Profile) -> String {
    let mut parts = Vec::new();
    for (day, (_, name)) in (0_u8..).zip(DAYS) {
        for entry in profile.entries.iter().filter(|e| e.weekdays.contains(&day)) {
            parts.push(format!("{name} {}", entry_words(entry)));
        }
    }
    parts.extend(always_on_words(profile, !parts.is_empty(), "week"));
    if parts.is_empty() {
        return "No times, and always on is off".to_owned();
    }
    super::format::capitalized(&parts.join("; "))
}

/// The colors a timeline is drawn in.
struct Inks {
    /// The empty day.
    track: Color32,
    /// The always-on entry, under the times: a light tint of the accent, plainly not the
    /// grey of a day with nothing, and lighter than a time switched off.
    always: Color32,
    /// A time that runs.
    runs: Color32,
    /// A time kept switched off: the accent at 40%.
    off: Color32,
    /// Around each time, so that one over another shows its edge.
    edge: Color32,
}

impl Inks {
    fn of(ui: &egui::Ui, ground: Color32) -> Inks {
        let palette = Palette::of(ui);
        Inks {
            track: Palette::tint(palette.label, 7),
            always: Palette::tint(palette.accent, 24),
            runs: palette.accent,
            off: Palette::tint(palette.accent, 40),
            edge: ground,
        }
    }
}

/// One day's bar in `rect`: the track, the always-on tint, then each time.
fn paint_day(painter: &egui::Painter, rect: Rect, plan: &DayPlan, inks: &Inks) {
    let radius = rect.height() / 2.0;
    painter.rect_filled(rect, radius, inks.track);
    if plan.always_on.is_some() {
        painter.rect_filled(rect, radius, inks.always);
    }
    let x = |at: f32| egui::lerp(rect.x_range(), at);
    for span in &plan.spans {
        let (left, right) = (x(span.from), x(span.to));
        let right = right.max(left + MIN_SPAN_WIDTH).min(rect.right());
        let left = left.min(right - MIN_SPAN_WIDTH);
        let bar = Rect::from_x_y_ranges(left..=right, rect.y_range());
        let fill = if span.runs { inks.runs } else { inks.off };
        painter.rect(
            bar,
            radius,
            fill,
            Stroke::new(1.0, inks.edge),
            StrokeKind::Outside,
        );
    }
}

/// The hours under a bar that runs across `x_range`, at `top`.
fn paint_hours(ui: &egui::Ui, x_range: egui::Rangef, top: f32) {
    let color = Palette::of(ui).secondary_label;
    for hour in MARKED_HOURS {
        let galley = one_line(ui, format!("{hour:02}:00"), subheadline(), f32::INFINITY);
        let x = egui::lerp(x_range, hour as f32 / 24.0);
        let at = Align2::CENTER_TOP.anchor_size(pos2(x, top), galley.size());
        ui.painter().galley(at.min, galley, color);
    }
}

/// Faint lines at the marked hours, across `rect`, for the eye to follow.
fn paint_hour_lines(ui: &egui::Ui, rect: Rect) {
    let stroke = Stroke::new(hairline(ui), Palette::of(ui).separator);
    for hour in MARKED_HOURS {
        let x = egui::lerp(rect.x_range(), hour as f32 / 24.0);
        ui.painter().vline(x, rect.y_range(), stroke);
    }
}

/// The "Today" strip, under its title: the 24 hours of today in the schedule's zone, its
/// times in the accent color, and a mark for now. `ground` is what it is drawn on. One
/// element to accessibility, named "Today", its value the day in words.
pub fn today_strip(ui: &mut egui::Ui, profile: &Profile, now: i64, ground: Color32) -> Response {
    let palette = Palette::of(ui);
    let width = ui.available_width();
    // Its title is drawn with it, so that it is read once, as the strip's name.
    let title = one_line(ui, "Today", TextStyle::Body, width);
    let above = title.size().y + GRID;
    let size = vec2(width, above + STRIP_HEIGHT + 6.0 + HOURS_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let (weekday, time) = local_weekday_and_time(now, profile.timezone_hours);
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::Image, true, "Today");
        info.current_text_value = Some(day_summary(profile, weekday));
        info
    });
    if !ui.is_rect_visible(rect) {
        return response;
    }
    ui.painter().galley(rect.min, title, palette.label);
    let bar = Rect::from_min_size(rect.min + vec2(0.0, above + 3.0), vec2(width, STRIP_HEIGHT));
    paint_day(
        ui.painter(),
        bar,
        &day_plan(profile, weekday),
        &Inks::of(ui, ground),
    );
    // Now: a line through the bar, a little longer than it is tall.
    let minutes = u32::from(time.hour) * 60 + u32::from(time.minute);
    let x = egui::lerp(bar.x_range(), now_fraction(minutes));
    let reach = (bar.top() - 3.0)..=(bar.bottom() + 3.0);
    ui.painter()
        .vline(x, reach.clone(), Stroke::new(2.0, ground));
    ui.painter()
        .vline(x, reach, Stroke::new(1.5, palette.label));
    paint_hours(ui, bar.x_range(), bar.bottom() + 4.0);
    response
}

/// The week grid: a row of 24 hours for each day, Sunday first, each time in the accent
/// color (40% when it is switched off) and the always-on entry as a faint tint across
/// the row. Overlapping times show their edges. One element to accessibility, its value
/// the week in words.
pub fn week_grid(ui: &mut egui::Ui, profile: &Profile, ground: Color32) -> Response {
    let palette = Palette::of(ui);
    let width = ui.available_width();
    let rows = 7.0 * GRID_ROW + 6.0 * GRID_GAP;
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, rows + 4.0 + HOURS_HEIGHT), Sense::hover());
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::Image, true, "Week");
        info.current_text_value = Some(week_summary(profile));
        info
    });
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let bars = Rect::from_min_max(
        pos2(rect.left() + DAY_COLUMN, rect.top()),
        pos2(rect.right(), rect.top() + rows),
    );
    paint_hour_lines(ui, bars);
    let inks = Inks::of(ui, ground);
    for (day, (letter, _)) in (0_u8..).zip(DAYS) {
        let top = bars.top() + f32::from(day) * (GRID_ROW + GRID_GAP);
        let row = Rect::from_x_y_ranges(bars.x_range(), top..=top + GRID_ROW);
        paint_day(ui.painter(), row, &day_plan(profile, day), &inks);
        let galley = one_line(ui, letter, subheadline(), f32::INFINITY);
        let at = Align2::LEFT_CENTER.anchor_size(pos2(rect.left(), row.center().y), galley.size());
        ui.painter().galley(at.min, galley, palette.secondary_label);
    }
    paint_hours(ui, bars.x_range(), bars.bottom() + 4.0);
    response
}

#[cfg(test)]
mod tests {
    use scent_core::{AlwaysOn, Entry, HhMm, Level, Profile, ScheduleFlag};

    use super::*;

    fn profile(always_on: bool, entries: Vec<Entry>) -> Profile {
        Profile {
            always_on: AlwaysOn {
                enable: always_on,
                level: Level::new(1).unwrap(),
            },
            entries,
            timezone_hours: -5.0,
            flag: ScheduleFlag::Replace,
        }
    }

    fn entry(start: &str, end: &str, weekdays: &[u8], level: u8, enable: bool) -> Entry {
        Entry {
            start: HhMm::parse(start).unwrap(),
            end: HhMm::parse(end).unwrap(),
            weekdays: weekdays.to_vec(),
            level: Level::new(level).unwrap(),
            enable,
        }
    }

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4
    }

    #[test]
    fn day_windows() {
        // Tuesday is 2 (0 is Sunday, as scent-core numbers the days).
        let late = profile(false, vec![entry("21:30", "22:30", &[2], 2, true)]);
        let windows = windows_for_day(&late, 2);
        assert_eq!(windows.len(), 1);
        assert!(close(windows[0], (0.8958, 0.9375)), "{windows:?}");
        // Another weekday has none.
        assert!(windows_for_day(&late, 3).is_empty());
        // The always-on entry fills the day.
        let always = profile(true, vec![entry("21:30", "22:30", &[2], 2, true)]);
        assert_eq!(windows_for_day(&always, 4), [(0.0, 1.0)]);
        assert_eq!(windows_for_day(&always, 2), [(0.0, 1.0)]);
        // A time that is switched off doesn't run.
        let off = profile(false, vec![entry("07:00", "09:00", &[2], 2, false)]);
        assert!(windows_for_day(&off, 2).is_empty());
    }

    #[test]
    fn windows_come_in_order_of_their_start() {
        let p = profile(
            false,
            vec![
                entry("18:00", "19:00", &[1], 1, true),
                entry("06:00", "07:30", &[1, 2], 3, true),
            ],
        );
        let windows = windows_for_day(&p, 1);
        assert_eq!(windows.len(), 2);
        assert!(close(windows[0], (0.25, 0.3125)) && close(windows[1], (0.75, 0.7917)));
    }

    #[test]
    fn the_spans_of_a_day_keep_the_times_that_are_switched_off() {
        let p = profile(
            true,
            vec![
                entry("07:00", "09:00", &[1, 2, 3, 4, 5], 2, true),
                entry("12:00", "13:00", &[2], 3, false),
            ],
        );
        let day = day_plan(&p, 2);
        assert_eq!(day.always_on, Some(1));
        let spans: Vec<(u8, bool)> = day
            .spans
            .iter()
            .map(|span| (span.level, span.runs))
            .collect();
        assert_eq!(spans, [(2, true), (3, false)]);
        assert!(close((day.spans[1].from, day.spans[1].to), (0.5, 0.5417)));
        // Sunday has neither time.
        assert!(day_plan(&p, 0).spans.is_empty());
    }

    #[test]
    fn now_is_a_fraction_of_the_day() {
        assert_eq!(now_fraction(0), 0.0);
        assert_eq!(now_fraction(12 * 60), 0.5);
        assert!((now_fraction(21 * 60 + 30) - 0.8958).abs() < 1e-4);
        // Past the end of the day is the end of the day.
        assert_eq!(now_fraction(24 * 60 + 5), 1.0);
    }

    #[test]
    fn the_week_is_read_out_day_by_day() {
        let p = profile(
            true,
            vec![
                entry("07:00", "09:00", &[1, 2], 2, true),
                entry("18:30", "22:00", &[0], 1, false),
            ],
        );
        assert_eq!(
            week_summary(&p),
            "Sunday 18:30 to 22:00 at Low, switched off; Monday 07:00 to 09:00 at Medium; \
             Tuesday 07:00 to 09:00 at Medium; always on at Low the rest of the time"
        );
        assert_eq!(
            week_summary(&profile(false, vec![])),
            "No times, and always on is off"
        );
        assert_eq!(
            day_summary(&p, 1),
            "07:00 to 09:00 at Medium; always on at Low the rest of the day"
        );
        assert_eq!(day_summary(&profile(false, vec![]), 3), "Nothing scheduled");
        // Always on alone is all day, all week.
        assert_eq!(
            week_summary(&profile(true, vec![])),
            "Always on at Low all week"
        );
        assert_eq!(
            day_summary(&profile(true, vec![]), 2),
            "Always on at Low all day"
        );
    }
}
