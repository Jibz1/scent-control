//! The words the screens use: ages, readings, the power source, the expected state.
//! Ported from the Phase 0 test bench (`tools/control.html`), so the two say the same
//! things. Pure: nothing here draws, reads the clock or touches the network.

use scent_core::{DeviceInfo, Expected, Online, Source, Via, is_stale};

use crate::state::{BrokerState, Reading};

const NOT_REPORTED: &str = "not reported yet";

/// The Mac's offset from UTC, in seconds, at the moment `at` (seconds since 1970),
/// so that daylight saving is the one in force then. 0 if the system can't say.
pub fn local_utc_offset_secs(at: i64) -> i64 {
    let time = at as libc::time_t;
    // SAFETY: an all-zero `tm` is valid (numbers, and a null zone name pointer).
    let mut local: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `localtime_r` reads `time` and writes only to `local`; both are valid
    // and exclusively ours for the call.
    let filled = unsafe { libc::localtime_r(&time, &mut local) };
    if filled.is_null() {
        0
    } else {
        local.tm_gmtoff as i64
    }
}

/// Hours, minutes and seconds of the day at `utc_offset_secs` from UTC.
fn time_of_day(at: i64, utc_offset_secs: i64) -> (i64, i64, i64) {
    let secs = at.saturating_add(utc_offset_secs).rem_euclid(86_400);
    (secs / 3600, secs / 60 % 60, secs % 60)
}

/// `HH:MM:SS` at `utc_offset_secs` from UTC.
pub fn clock_hms(at: i64, utc_offset_secs: i64) -> String {
    let (hour, minute, second) = time_of_day(at, utc_offset_secs);
    format!("{hour:02}:{minute:02}:{second:02}")
}

/// How long ago `at` was, as the bench says it: "just now", "N min ago" (whole
/// minutes, rounded), or from an hour on the clock time it was, "at HH:MM", on this
/// Mac's clock.
pub fn ago(at: i64, now: i64) -> String {
    ago_in(at, now, local_utc_offset_secs(at))
}

fn ago_in(at: i64, now: i64, utc_offset_secs: i64) -> String {
    let since = now.saturating_sub(at).max(0);
    let minutes = since.saturating_add(30) / 60;
    match minutes {
        0 => "just now".to_owned(),
        1..=59 => format!("{minutes} min ago"),
        _ => {
            let (hour, minute, _) = time_of_day(at, utc_offset_secs);
            format!("at {hour:02}:{minute:02}")
        }
    }
}

pub fn via_name(via: Via) -> &'static str {
    match via {
        Via::Bluetooth => "Bluetooth",
        Via::Wifi => "WiFi",
    }
}

/// A reading as one line: the value, when it was reported and over what, such as
/// "On · reported 3 min ago over WiFi". The flag is whether it is stale (see
/// `scent_core::is_stale`), for the screen to grey it out. `format` turns the number
/// into words. Nothing reported yet says so, and is not stale.
pub fn reading_text(
    reading: Option<&Reading>,
    now: i64,
    format: impl Fn(i64) -> String,
) -> (String, bool) {
    let Some(reading) = reading else {
        return (NOT_REPORTED.to_owned(), false);
    };
    let text = format!(
        "{} · reported {} over {}",
        format(reading.value),
        ago(reading.at, now),
        via_name(reading.via)
    );
    (text, is_stale(reading.at, now))
}

/// A value we sent that the diffuser hasn't reported on yet.
pub fn pending_text(value: &str) -> String {
    format!("{value} · sent, waiting for the diffuser")
}

pub fn on_off(power: i64) -> String {
    let text = if power == 0 { "Off" } else { "On" };
    text.to_owned()
}

/// The intensity with its name, "2 (medium)". 0 is "–": the diffuser is off.
pub fn intensity(level: i64) -> String {
    match level {
        0 => "–".to_owned(),
        1 => "1 (low)".to_owned(),
        2 => "2 (medium)".to_owned(),
        3 => "3 (high)".to_owned(),
        other => other.to_string(),
    }
}

pub fn signal(dbm: i64) -> String {
    format!("{dbm} dBm")
}

/// What the diffuser runs on. Charging 2 or battery 200 is plugged in; the bench has
/// no other reading for a battery that is not charging than charging 0.
pub fn power_source(charging: Option<i64>, battery: Option<i64>) -> String {
    match (charging, battery) {
        (Some(2), _) | (_, Some(200)) => "plugged in".to_owned(),
        (Some(1), _) => "charging".to_owned(),
        (Some(0), Some(percent)) => format!("battery {percent}%"),
        (Some(0), None) => "battery".to_owned(),
        _ => NOT_REPORTED.to_owned(),
    }
}

/// What the schedule this app last sent says the diffuser should be doing now. It
/// says where that comes from, because the diffuser may hold a different one (the
/// Scentiment app, or another copy of this one, could have sent its own).
pub fn expected_text(expected: &Expected) -> String {
    const BASED_ON: &str = "Based on the schedule this app last sent, it should be";
    if !expected.power {
        return format!("{BASED_ON} off now.");
    }
    let level = expected
        .level
        .map_or_else(String::new, |level| format!(" at level {}", level.get()));
    let until = expected
        .until
        .map_or_else(String::new, |until| format!(" until {until}"));
    let source = match expected.source {
        Source::Entry { start, end } => format!("schedule {start}–{end}"),
        Source::AlwaysOn => "always-on entry".to_owned(),
    };
    format!("{BASED_ON} on{level}{until} ({source}).")
}

/// What the badge says about whether the diffuser answers its pings.
pub fn online_text(online: Online) -> &'static str {
    match online {
        Online::Online => "Answering",
        Online::Waiting => "Pinged, waiting for an answer",
        Online::Offline => "Not answering pings",
        Online::Unknown => "Not pinged yet",
    }
}

pub fn broker_text(broker: &BrokerState) -> String {
    match broker {
        BrokerState::Connecting => "Connecting to the broker…".to_owned(),
        BrokerState::Connected => "Connected to the broker".to_owned(),
        BrokerState::LoginRefused(why) => format!("Broker login refused: {why}"),
        BrokerState::Disconnected(why) => format!("Not connected to the broker: {why}"),
    }
}

/// What the Bluetooth scan heard of a diffuser's signal.
pub fn scan_signal(rssi: Option<i16>) -> String {
    rssi.map_or_else(|| "signal unknown".to_owned(), |dbm| signal(dbm.into()))
}

/// The model and firmware a diffuser told us, such as "Diffuser Air 2, firmware
/// 2.30"; empty when it told us neither.
pub fn model_text(info: Option<&DeviceInfo>) -> String {
    let Some(info) = info else {
        return String::new();
    };
    let firmware = (!info.firmware.is_empty()).then(|| format!("firmware {}", info.firmware));
    [
        Some(info.model_name.clone()).filter(|m| !m.is_empty()),
        firmware,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(", ")
}

#[cfg(test)]
mod tests {
    use scent_core::{DeviceInfo, Expected, HhMm, Level, Online, Source, Via};

    use super::*;
    use crate::state::{BrokerState, Reading};

    /// 2026-10-20 15:30:00 UTC.
    const NOW: i64 = 1_792_510_200;
    const UTC_MINUS_5: i64 = -5 * 3600;

    fn reading(value: i64, age: i64, via: Via) -> Reading {
        Reading {
            value,
            at: NOW - age,
            via,
        }
    }

    #[test]
    fn ago_formats() {
        assert_eq!(ago_in(NOW, NOW, 0), "just now");
        // Whole minutes, rounded as the bench rounds them.
        assert_eq!(ago_in(NOW - 29, NOW, 0), "just now");
        assert_eq!(ago_in(NOW - 30, NOW, 0), "1 min ago");
        assert_eq!(ago_in(NOW - 180, NOW, 0), "3 min ago");
        assert_eq!(ago_in(NOW - 3569, NOW, 0), "59 min ago");
        // An hour or more: the clock time, in the time zone given (13:30 UTC is 08:30
        // at UTC-5).
        assert_eq!(ago_in(NOW - 3570, NOW, UTC_MINUS_5), "at 09:30");
        assert_eq!(ago_in(NOW - 7200, NOW, UTC_MINUS_5), "at 08:30");
        assert_eq!(ago_in(NOW - 7200, NOW, 0), "at 13:30");
        // A clock that stepped back is not "in the future".
        assert_eq!(ago_in(NOW + 90, NOW, 0), "just now");
    }

    #[test]
    fn extreme_timestamps_do_not_overflow() {
        // A saved file can hold anything.
        let _ = ago_in(i64::MIN, i64::MAX, 0);
        let _ = ago_in(i64::MAX, i64::MIN, 0);
        let _ = clock_hms(i64::MAX, i64::MAX);
        let _ = clock_hms(i64::MIN, i64::MIN);
    }

    #[test]
    fn ago_shows_a_clock_time_in_the_macs_time_zone() {
        let text = ago(NOW - 7200, NOW);
        let clock = text.strip_prefix("at ").expect(&text);
        assert!(HhMm::parse(clock).is_some(), "{text}");
        assert_eq!(ago(NOW - 180, NOW), "3 min ago");
    }

    #[test]
    fn the_time_zone_offset_is_a_real_one() {
        let offset = local_utc_offset_secs(NOW);
        assert!((-14 * 3600..=14 * 3600).contains(&offset), "{offset}");
    }

    #[test]
    fn clock_times_use_the_offset() {
        assert_eq!(clock_hms(NOW, 0), "15:30:00");
        assert_eq!(clock_hms(NOW + 7, UTC_MINUS_5), "10:30:07");
        // Across midnight in both directions.
        assert_eq!(clock_hms(NOW + 9 * 3600, 0), "00:30:00");
        assert_eq!(clock_hms(NOW - 16 * 3600, UTC_MINUS_5), "18:30:00");
    }

    #[test]
    fn power_source_wording() {
        assert_eq!(power_source(Some(2), Some(200)), "plugged in");
        assert_eq!(power_source(Some(1), Some(80)), "charging");
        assert_eq!(power_source(Some(0), Some(80)), "battery 80%");
        assert_eq!(power_source(None, None), "not reported yet");
        // The same cases the bench handles.
        assert_eq!(power_source(None, Some(200)), "plugged in");
        assert_eq!(power_source(Some(2), None), "plugged in");
        assert_eq!(power_source(Some(0), None), "battery");
        assert_eq!(power_source(None, Some(80)), "not reported yet");
    }

    #[test]
    fn reading_is_stale_after_300_s() {
        let (_, stale) = reading_text(Some(&reading(1, 300, Via::Wifi)), NOW, on_off);
        assert!(!stale);
        let (text, stale) = reading_text(Some(&reading(1, 301, Via::Wifi)), NOW, on_off);
        assert!(stale);
        assert_eq!(text, "On · reported 5 min ago over WiFi");
    }

    #[test]
    fn reading_text_names_the_value_the_age_and_the_source() {
        let (text, stale) = reading_text(Some(&reading(0, 0, Via::Bluetooth)), NOW, on_off);
        assert_eq!(text, "Off · reported just now over Bluetooth");
        assert!(!stale);
        let (text, stale) = reading_text(Some(&reading(-66, 120, Via::Wifi)), NOW, signal);
        assert_eq!(text, "-66 dBm · reported 2 min ago over WiFi");
        assert!(!stale);
        // Nothing yet is not stale: there is nothing to grey out.
        assert_eq!(
            reading_text(None, NOW, on_off),
            ("not reported yet".to_owned(), false)
        );
    }

    #[test]
    fn a_sent_value_waits_for_the_diffuser() {
        assert_eq!(pending_text("On"), "On · sent, waiting for the diffuser");
    }

    #[test]
    fn values_have_the_benchs_words() {
        assert_eq!((on_off(1), on_off(0)), ("On".to_owned(), "Off".to_owned()));
        assert_eq!(intensity(1), "1 (low)");
        assert_eq!(intensity(2), "2 (medium)");
        assert_eq!(intensity(3), "3 (high)");
        assert_eq!(
            (intensity(0), intensity(9)),
            ("–".to_owned(), "9".to_owned())
        );
        assert_eq!(signal(-66), "-66 dBm");
    }

    fn expected(power: bool, level: u8, source: Source, until: Option<&str>) -> Expected {
        Expected {
            power,
            level: Level::new(level),
            source,
            until: until.and_then(HhMm::parse),
        }
    }

    #[test]
    fn expected_text_wording() {
        let off = expected(false, 0, Source::AlwaysOn, None);
        assert_eq!(
            expected_text(&off),
            "Based on the schedule this app last sent, it should be off now."
        );
        let entry = Source::Entry {
            start: HhMm::parse("21:30").unwrap(),
            end: HhMm::parse("22:30").unwrap(),
        };
        assert_eq!(
            expected_text(&expected(true, 1, entry, Some("22:30"))),
            "Based on the schedule this app last sent, it should be on at level 1 until 22:30 (schedule 21:30–22:30)."
        );
        // The always-on entry has no end.
        assert_eq!(
            expected_text(&expected(true, 2, Source::AlwaysOn, None)),
            "Based on the schedule this app last sent, it should be on at level 2 (always-on entry)."
        );
    }

    #[test]
    fn the_online_badge_says_what_the_bench_says() {
        assert_eq!(online_text(Online::Online), "Answering");
        assert_eq!(
            online_text(Online::Waiting),
            "Pinged, waiting for an answer"
        );
        assert_eq!(online_text(Online::Offline), "Not answering pings");
        assert_eq!(online_text(Online::Unknown), "Not pinged yet");
    }

    #[test]
    fn the_broker_line_says_what_the_hub_said() {
        assert_eq!(
            broker_text(&BrokerState::Connecting),
            "Connecting to the broker…"
        );
        assert_eq!(
            broker_text(&BrokerState::Connected),
            "Connected to the broker"
        );
        assert_eq!(
            broker_text(&BrokerState::Disconnected(
                "No broker password saved.".into()
            )),
            "Not connected to the broker: No broker password saved."
        );
        assert_eq!(
            broker_text(&BrokerState::LoginRefused("Not authorized.".into())),
            "Broker login refused: Not authorized."
        );
    }

    #[test]
    fn signal_strength_of_a_scan_hit() {
        assert_eq!(scan_signal(Some(-62)), "-62 dBm");
        assert_eq!(scan_signal(None), "signal unknown");
    }

    #[test]
    fn the_model_line_leaves_out_what_the_diffuser_did_not_say() {
        let mut info = DeviceInfo {
            manufacturer_id: "01".into(),
            model_name: "Diffuser Air 2".into(),
            variant_name: "Black".into(),
            device_id: "TESTDEVICE01".into(),
            firmware: "2.30".into(),
            wifi_configured: true,
            wifi_status: 3,
        };
        assert_eq!(model_text(Some(&info)), "Diffuser Air 2, firmware 2.30");
        info.model_name.clear();
        assert_eq!(model_text(Some(&info)), "firmware 2.30");
        info.firmware.clear();
        assert_eq!(model_text(Some(&info)), "");
        assert_eq!(model_text(None), "");
    }
}
