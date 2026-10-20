//! Schedules: the timetable a diffuser keeps on board, what it implies at a given
//! moment, and the `UPDATE_PROFILE` payload that puts it there.

use std::fmt;

use serde_json::{Value, json};

use crate::Level;

const MINUTES_PER_DAY: u32 = 24 * 60;
const SECONDS_PER_DAY: i64 = 86_400;

/// A time of day on a 24-hour clock. Build one with [`HhMm::parse`] so the hour
/// is below 24 and the minute below 60. Times order by the clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HhMm {
    pub hour: u8,
    pub minute: u8,
}

impl HhMm {
    /// Reads exactly `HH:MM`: two digits, a colon, two digits, a valid time.
    /// Anything else (`7:00`, `24:00`, `12:60`, stray spaces or signs) is `None`.
    pub fn parse(text: &str) -> Option<HhMm> {
        let &[h1, h2, b':', m1, m2] = text.as_bytes() else {
            return None;
        };
        let digit = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            _ => None,
        };
        let hour = digit(h1)? * 10 + digit(h2)?;
        let minute = digit(m1)? * 10 + digit(m2)?;
        (hour < 24 && minute < 60).then_some(HhMm { hour, minute })
    }

    /// `minutes` is below [`MINUTES_PER_DAY`].
    fn from_minutes(minutes: u32) -> HhMm {
        HhMm {
            hour: (minutes / 60) as u8,
            minute: (minutes % 60) as u8,
        }
    }

    fn minutes(self) -> u32 {
        u32::from(self.hour) * 60 + u32::from(self.minute)
    }
}

impl fmt::Display for HhMm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

/// A timed run: on at `level` from `start` up to (not including) `end` on each of
/// `weekdays` (0 = Sunday ... 6 = Saturday). It must not cross midnight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub start: HhMm,
    pub end: HhMm,
    pub weekdays: Vec<u8>,
    pub level: Level,
    pub enable: bool,
}

/// The fixed "factory" entry that covers the whole day and applies when no timed
/// entry does. The firmware cannot edit or delete it, only switch it on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlwaysOn {
    pub enable: bool,
    pub level: Level,
}

/// The `schedule_flag` of `UPDATE_PROFILE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleFlag {
    /// Change only the always-on entry. The diffuser keeps the timed entries it has.
    AlwaysOnOnly = 0,
    /// Replace the whole table.
    Replace = 1,
}

/// A diffuser's schedule table and the time zone its times are in.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub always_on: AlwaysOn,
    pub entries: Vec<Entry>,
    /// Offset from UTC in hours, such as `-5.0` or `5.5`.
    pub timezone_hours: f64,
    pub flag: ScheduleFlag,
}

fn working_state(level: Level) -> Value {
    json!({ "level": level.get(), "fan": 0, "led": 0, "timer": 0 })
}

impl AlwaysOn {
    fn to_json(self) -> Value {
        json!({
            "index": 1,
            "startTime": "00:00",
            "endTime": "23:59",
            "weekdays": [0, 1, 2, 3, 4, 5, 6],
            "workingState": working_state(self.level),
            "enable": self.enable,
            "isFactory": true,
        })
    }
}

impl Entry {
    fn to_json(&self) -> Value {
        json!({
            "startTime": self.start.to_string(),
            "endTime": self.end.to_string(),
            "weekdays": self.weekdays,
            "workingState": working_state(self.level),
            "enable": self.enable,
        })
    }
}

impl Profile {
    /// The payload of `UPDATE_PROFILE`, keys in the order the Phase 0 tools write them.
    /// The factory entry comes first. The timed entries follow, but only for
    /// [`ScheduleFlag::Replace`]: flag 0 sends the factory entry alone.
    pub fn to_payload(&self) -> Value {
        let mut schedule = vec![self.always_on.to_json()];
        if self.flag == ScheduleFlag::Replace {
            schedule.extend(self.entries.iter().map(Entry::to_json));
        }
        json!({
            "profile": { "schedule": schedule },
            "timezone": self.timezone_hours,
            "schedule_flag": self.flag as u8,
        })
    }
}

/// What decides the expected state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A timed entry that is running now.
    Entry {
        start: HhMm,
        end: HhMm,
    },
    AlwaysOn,
}

/// What a diffuser should be doing at a given moment, according to its schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expected {
    pub power: bool,
    /// `None` when it should be off.
    pub level: Option<Level>,
    pub source: Source,
    /// When the running entry ends. `None` for the always-on entry.
    pub until: Option<HhMm>,
}

/// The weekday (0 = Sunday ... 6 = Saturday) and clock time at UTC offset
/// `tz_hours`, for a UTC time in seconds since 1970. Seconds are dropped.
/// Out-of-range offsets give a valid, if meaningless, answer rather than a panic.
pub fn local_weekday_and_time(now_utc_secs: i64, tz_hours: f64) -> (u8, HhMm) {
    let local = now_utc_secs.saturating_add((tz_hours * 3600.0).round() as i64);
    let days = local.div_euclid(SECONDS_PER_DAY);
    let weekday = (days + 4).rem_euclid(7) as u8; // 1970-01-01 was a Thursday
    let minutes = local.rem_euclid(SECONDS_PER_DAY) / 60;
    (weekday, HhMm::from_minutes(minutes as u32))
}

/// What the diffuser should be doing at `now_utc_secs` (seconds since 1970, UTC).
///
/// A timed entry is running when it is enabled, today's weekday in the diffuser's
/// time zone is one of its weekdays, and `start <= now < end`. The first running
/// entry wins; otherwise the always-on entry decides. The `flag` plays no part: it
/// says what to send, while the table in `entries` says what the diffuser holds.
pub fn expected_state(profile: &Profile, now_utc_secs: i64) -> Expected {
    let (weekday, now) = local_weekday_and_time(now_utc_secs, profile.timezone_hours);
    let running = profile
        .entries
        .iter()
        .find(|e| e.enable && e.weekdays.contains(&weekday) && e.start <= now && now < e.end);
    match running {
        Some(e) => Expected {
            power: true,
            level: Some(e.level),
            source: Source::Entry {
                start: e.start,
                end: e.end,
            },
            until: Some(e.end),
        },
        None => Expected {
            power: profile.always_on.enable,
            level: profile.always_on.enable.then_some(profile.always_on.level),
            source: Source::AlwaysOn,
            until: None,
        },
    }
}

/// A one-day test entry for `weekday` that starts `start_in_min` minutes after `now`
/// and lasts `duration_min`. It fails if it would run past 23:59, since an entry
/// cannot cross midnight.
pub fn trial_entry(
    weekday: u8,
    now: HhMm,
    start_in_min: u16,
    duration_min: u16,
    level: Level,
) -> Result<Entry, String> {
    let start = now.minutes() + u32::from(start_in_min);
    let end = start + u32::from(duration_min);
    if end >= MINUTES_PER_DAY {
        return Err(
            "the test schedule would cross midnight; pick a start time that ends before 23:59"
                .to_string(),
        );
    }
    Ok(Entry {
        start: HhMm::from_minutes(start),
        end: HhMm::from_minutes(end),
        weekdays: vec![weekday],
        level,
        enable: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(n: u8) -> Level {
        Level::new(n).unwrap()
    }

    fn hhmm(text: &str) -> HhMm {
        HhMm::parse(text).unwrap()
    }

    fn off(n: u8) -> AlwaysOn {
        AlwaysOn {
            enable: false,
            level: level(n),
        }
    }

    fn window(start: &str, end: &str, weekdays: &[u8], n: u8) -> Entry {
        Entry {
            start: hhmm(start),
            end: hhmm(end),
            weekdays: weekdays.to_vec(),
            level: level(n),
            enable: true,
        }
    }

    fn profile(always_on: AlwaysOn, entries: Vec<Entry>, tz: f64, flag: ScheduleFlag) -> Profile {
        Profile {
            always_on,
            entries,
            timezone_hours: tz,
            flag,
        }
    }

    // 2026-10-18 is a Sunday.
    const SUN_MIDNIGHT_UTC: i64 = 1_792_281_600;

    // --- HhMm ---

    #[test]
    fn parse_accepts_hh_mm_and_nothing_else() {
        for (text, hour, minute) in [("00:00", 0, 0), ("07:05", 7, 5), ("23:59", 23, 59)] {
            assert_eq!(HhMm::parse(text), Some(HhMm { hour, minute }), "{text}");
        }
        for bad in [
            "",
            "7:00",
            "24:00",
            "12:60",
            "12-30",
            "1200",
            "+7:00",
            "12:3",
            "12:300",
            " 7:00",
            "12:30 ",
            "ab:cd",
            "12:30\n",
            "１２:３０",
        ] {
            assert_eq!(HhMm::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn display_is_two_digit_and_parses_back() {
        assert_eq!(HhMm { hour: 7, minute: 5 }.to_string(), "07:05");
        for text in ["00:00", "09:59", "12:30", "23:59"] {
            assert_eq!(hhmm(text).to_string(), text);
        }
    }

    #[test]
    fn times_order_by_the_clock() {
        assert!(hhmm("09:59") < hhmm("10:00"));
        assert!(hhmm("10:00") < hhmm("10:01"));
        assert!(hhmm("00:00") < hhmm("23:59"));
    }

    // --- local time ---

    #[test]
    fn weekday_zero_is_sunday_and_six_is_saturday() {
        for (days_after_sunday, weekday) in [(0, 0), (1, 1), (3, 3), (6, 6), (7, 0)] {
            let at = SUN_MIDNIGHT_UTC + days_after_sunday * 86_400;
            assert_eq!(
                local_weekday_and_time(at, 0.0),
                (weekday, hhmm("00:00")),
                "{days_after_sunday} days after Sunday"
            );
        }
        // 1970-01-01 was a Thursday, and the second before it a Wednesday.
        assert_eq!(local_weekday_and_time(0, 0.0), (4, hhmm("00:00")));
        assert_eq!(local_weekday_and_time(-1, 0.0), (3, hhmm("23:59")));
    }

    #[test]
    fn local_time_follows_the_offset_including_half_hours() {
        // Sunday 00:00 UTC is already Sunday 05:30 in India ...
        assert_eq!(
            local_weekday_and_time(SUN_MIDNIGHT_UTC, 5.5),
            (0, hhmm("05:30"))
        );
        // ... and still Saturday 20:30 in Newfoundland-style UTC-3:30.
        assert_eq!(
            local_weekday_and_time(SUN_MIDNIGHT_UTC, -3.5),
            (6, hhmm("20:30"))
        );
        // Seconds are dropped, not rounded.
        assert_eq!(
            local_weekday_and_time(SUN_MIDNIGHT_UTC + 59, 0.0),
            (0, hhmm("00:00"))
        );
    }

    #[test]
    fn absurd_inputs_give_a_valid_time_instead_of_panicking() {
        for (at, tz) in [
            (i64::MAX, f64::INFINITY),
            (i64::MIN, f64::NEG_INFINITY),
            (i64::MAX, 1e300),
            (0, f64::NAN),
        ] {
            let (weekday, time) = local_weekday_and_time(at, tz);
            assert!(weekday <= 6 && time.hour < 24 && time.minute < 60);
        }
    }

    // --- expected_state ---

    // Ported from tools/tests/test_control_server.py: a Tuesday 10:24-10:27 window at UTC-5.
    fn tuesday_window() -> Profile {
        profile(
            off(1),
            vec![window("10:24", "10:27", &[2], 3)],
            -5.0,
            ScheduleFlag::Replace,
        )
    }

    #[test]
    fn expected_state_during_and_after_a_window() {
        let p = tuesday_window();
        // 2026-10-20T15:25:00Z is Tuesday 10:25 at UTC-5.
        assert_eq!(
            expected_state(&p, 1_792_509_900),
            Expected {
                power: true,
                level: Some(level(3)),
                source: Source::Entry {
                    start: hhmm("10:24"),
                    end: hhmm("10:27")
                },
                until: Some(hhmm("10:27")),
            }
        );
        // Three minutes later the window is over and the always-on entry (off) decides.
        assert_eq!(
            expected_state(&p, 1_792_510_080),
            Expected {
                power: false,
                level: None,
                source: Source::AlwaysOn,
                until: None,
            }
        );
    }

    #[test]
    fn a_window_includes_its_start_minute_and_excludes_its_end_minute() {
        let p = tuesday_window();
        let at_1523_59 = 1_792_509_839;
        assert!(!expected_state(&p, at_1523_59).power); // 10:23:59
        assert!(expected_state(&p, at_1523_59 + 1).power); // 10:24:00
        assert!(expected_state(&p, 1_792_510_019).power); // 10:26:59
        assert!(!expected_state(&p, 1_792_510_020).power); // 10:27:00
    }

    #[test]
    fn expected_state_uses_the_local_weekday_across_midnight_utc() {
        // Wed 2026-10-21 03:00 UTC is Tue 22:00 at UTC-5
        let p = Profile {
            always_on: AlwaysOn {
                enable: false,
                level: Level::new(1).unwrap(),
            },
            timezone_hours: -5.0,
            flag: ScheduleFlag::Replace,
            entries: vec![Entry {
                start: HhMm::parse("21:30").unwrap(),
                end: HhMm::parse("22:30").unwrap(),
                weekdays: vec![2],
                level: Level::new(1).unwrap(),
                enable: true,
            }],
        };
        let e = expected_state(&p, 1_792_551_600); // 2026-10-21T03:00:00Z
        assert!(e.power && e.until == HhMm::parse("22:30"));
    }

    #[test]
    fn an_entry_on_another_weekday_or_switched_off_is_ignored() {
        let mut p = tuesday_window();
        p.entries[0].weekdays = vec![0, 1, 3, 4, 5, 6];
        assert!(!expected_state(&p, 1_792_509_900).power);

        let mut p = tuesday_window();
        p.entries[0].enable = false;
        assert!(!expected_state(&p, 1_792_509_900).power);
    }

    #[test]
    fn the_first_active_entry_wins() {
        let p = profile(
            off(1),
            vec![
                window("10:00", "11:00", &[2], 2),
                window("10:20", "10:30", &[2], 3),
            ],
            -5.0,
            ScheduleFlag::Replace,
        );
        let e = expected_state(&p, 1_792_509_900); // Tuesday 10:25
        assert_eq!(e.level, Some(level(2)));
        assert_eq!(e.until, Some(hhmm("11:00")));
    }

    #[test]
    fn outside_every_window_the_always_on_entry_decides() {
        let on = profile(
            AlwaysOn {
                enable: true,
                level: level(2),
            },
            vec![window("10:24", "10:27", &[2], 3)],
            -5.0,
            ScheduleFlag::Replace,
        );
        assert_eq!(
            expected_state(&on, 1_792_510_080),
            Expected {
                power: true,
                level: Some(level(2)),
                source: Source::AlwaysOn,
                until: None,
            }
        );
        // An off always-on entry reports no level, whatever level it stores.
        let e = expected_state(&profile(off(3), vec![], 0.0, ScheduleFlag::Replace), 0);
        assert_eq!((e.power, e.level), (false, None));
    }

    #[test]
    fn the_flag_only_decides_what_is_sent_not_what_is_expected() {
        // With flag 0 the diffuser keeps the timed entries it already has, so the
        // table in `entries` still describes it.
        let mut p = tuesday_window();
        p.flag = ScheduleFlag::AlwaysOnOnly;
        assert!(expected_state(&p, 1_792_509_900).power);
    }

    // --- UPDATE_PROFILE payload ---

    #[test]
    fn payload_shape_matches_the_firmware() {
        let p = Profile {
            always_on: AlwaysOn {
                enable: false,
                level: Level::new(1).unwrap(),
            },
            timezone_hours: -5.0,
            flag: ScheduleFlag::Replace,
            entries: vec![Entry {
                start: HhMm::parse("07:00").unwrap(),
                end: HhMm::parse("09:00").unwrap(),
                weekdays: vec![1, 2, 3, 4, 5],
                level: Level::new(1).unwrap(),
                enable: true,
            }],
        };
        assert_eq!(
            p.to_payload(),
            serde_json::json!({"profile":{"schedule":[
        {"index":1,"startTime":"00:00","endTime":"23:59","weekdays":[0,1,2,3,4,5,6],"workingState":{"level":1,"fan":0,"led":0,"timer":0},"enable":false,"isFactory":true},
        {"startTime":"07:00","endTime":"09:00","weekdays":[1,2,3,4,5],"workingState":{"level":1,"fan":0,"led":0,"timer":0},"enable":true}]},
        "timezone":-5.0,"schedule_flag":1})
        );
    }

    #[test]
    fn payload_keys_come_out_in_the_order_the_app_writes_them() {
        // Expected text from tools/scentiment_ble.py (build_profile). Value equality
        // ignores key order, so this compares the text.
        let p = profile(
            AlwaysOn {
                enable: false,
                level: level(1),
            },
            vec![window("07:00", "09:00", &[1, 2, 3, 4, 5], 3)],
            -5.0,
            ScheduleFlag::Replace,
        );
        assert_eq!(
            p.to_payload().to_string(),
            r#"{"profile":{"schedule":[{"index":1,"startTime":"00:00","endTime":"23:59","weekdays":[0,1,2,3,4,5,6],"workingState":{"level":1,"fan":0,"led":0,"timer":0},"enable":false,"isFactory":true},{"startTime":"07:00","endTime":"09:00","weekdays":[1,2,3,4,5],"workingState":{"level":3,"fan":0,"led":0,"timer":0},"enable":true}]},"timezone":-5.0,"schedule_flag":1}"#
        );
    }

    #[test]
    fn the_factory_entry_carries_the_always_on_settings() {
        let p = profile(
            AlwaysOn {
                enable: true,
                level: level(3),
            },
            vec![],
            5.5,
            ScheduleFlag::Replace,
        );
        let payload = p.to_payload();
        let schedule = payload["profile"]["schedule"].as_array().unwrap();
        // No timed entries: the schedule is only the factory entry, and flag 1 clears the rest.
        assert_eq!(schedule.len(), 1);
        assert_eq!(schedule[0]["enable"], true);
        assert_eq!(schedule[0]["workingState"]["level"], 3);
        assert_eq!(schedule[0]["isFactory"], true);
        assert_eq!(payload["timezone"], 5.5);
        assert_eq!(payload["schedule_flag"], 1);
    }

    #[test]
    fn always_on_only_sends_just_the_factory_entry_with_flag_zero() {
        let p = profile(
            AlwaysOn {
                enable: true,
                level: level(2),
            },
            vec![window("07:00", "09:00", &[1], 3)],
            -5.0,
            ScheduleFlag::AlwaysOnOnly,
        );
        assert_eq!(
            p.to_payload().to_string(),
            r#"{"profile":{"schedule":[{"index":1,"startTime":"00:00","endTime":"23:59","weekdays":[0,1,2,3,4,5,6],"workingState":{"level":2,"fan":0,"led":0,"timer":0},"enable":true,"isFactory":true}]},"timezone":-5.0,"schedule_flag":0}"#
        );
    }

    // --- trial_entry ---

    // Ported from tools/tests/test_scentiment_ble.py: Monday 14:58, start in 3, last 4, level 3.
    #[test]
    fn trial_entry_starts_soon_and_stays_on_one_day() {
        assert_eq!(
            trial_entry(1, hhmm("14:58"), 3, 4, level(3)),
            Ok(Entry {
                start: hhmm("15:01"),
                end: hhmm("15:05"),
                weekdays: vec![1],
                level: level(3),
                enable: true,
            })
        );
        let err = trial_entry(1, hhmm("23:58"), 3, 4, level(3)).unwrap_err();
        assert!(err.contains("midnight"), "{err}");
    }

    #[test]
    fn trial_entry_may_end_at_23_59_but_not_at_midnight() {
        let e = trial_entry(0, hhmm("23:50"), 0, 9, level(1)).unwrap();
        assert_eq!((e.start, e.end), (hhmm("23:50"), hhmm("23:59")));
        assert!(
            trial_entry(0, hhmm("23:50"), 0, 10, level(1))
                .unwrap_err()
                .contains("midnight")
        );
    }

    #[test]
    fn trial_entry_survives_huge_arguments() {
        assert!(
            trial_entry(0, hhmm("23:59"), u16::MAX, u16::MAX, level(1))
                .unwrap_err()
                .contains("midnight")
        );
    }

    #[test]
    fn a_trial_entry_is_active_when_it_says_so() {
        // The Mon 14:58 trial above, run at UTC: on at 15:01, off at 15:05.
        let now = SUN_MIDNIGHT_UTC + 86_400 + (14 * 60 + 58) * 60;
        let (weekday, time) = local_weekday_and_time(now, 0.0);
        let entry = trial_entry(weekday, time, 3, 4, level(3)).unwrap();
        let p = profile(off(1), vec![entry], 0.0, ScheduleFlag::Replace);
        assert!(!expected_state(&p, now).power);
        assert!(expected_state(&p, now + 3 * 60).power);
        assert!(!expected_state(&p, now + 7 * 60).power);
    }
}
