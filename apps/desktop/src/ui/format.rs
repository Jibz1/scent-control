//! The words the screens use: ages, readings, the power source, the expected state.
//! Ported from the Phase 0 test bench (`tools/control.html`), so the two say the same
//! things. Nothing here draws or touches the network, and none of it reads the clock:
//! the time is always passed in. The one thing that asks the system anything is [`ago`],
//! and so [`reading_text`]: for "at HH:MM" they look up the Mac's time zone (see
//! `clock`).

use scent_core::{DeviceInfo, Expected, Online, Source, Via, is_stale};

use super::clock::{local_utc_offset_secs, time_of_day};
use crate::state::{BrokerState, Reading};

const NOT_REPORTED: &str = "not reported yet";

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
/// into words. Nothing reported yet, or nothing that `format` can word, says "not
/// reported yet" with no age, and is not stale.
pub fn reading_text(
    reading: Option<&Reading>,
    now: i64,
    format: impl Fn(i64) -> String,
) -> (String, bool) {
    let Some(reading) = reading else {
        return (NOT_REPORTED.to_owned(), false);
    };
    let value = format(reading.value);
    if value == NOT_REPORTED {
        // A value we can't put in words has no age worth giving, and nothing to grey.
        return (value, false);
    }
    let text = format!(
        "{value} · reported {} over {}",
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

/// The intensity by the name the segmented control gives it, "Medium". 0 is "–": the
/// diffuser is off. A level with no name is its number.
pub fn intensity(level: i64) -> String {
    match level {
        0 => "–".to_owned(),
        1 => "Low".to_owned(),
        2 => "Medium".to_owned(),
        3 => "High".to_owned(),
        other => other.to_string(),
    }
}

pub fn signal(dbm: i64) -> String {
    format!("{dbm} dBm")
}

/// What the diffuser runs on. Charging 2 or battery 200 is plugged in, charging 1 is
/// charging, and otherwise a battery level is the battery's. Without a level, charging
/// 0 is just "battery"; anything else is not something we can word.
pub fn power_source(charging: Option<i64>, battery: Option<i64>) -> String {
    match (charging, battery) {
        (Some(2), _) | (_, Some(200)) => "plugged in".to_owned(),
        (Some(1), _) => "charging".to_owned(),
        (_, Some(percent)) => format!("battery {percent}%"),
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

/// The broker's state in a few words, for the toolbar. The whole of it, with the reason,
/// is [`broker_text`]: in the toolbar's tooltip, and in Settings.
pub fn broker_summary(broker: &BrokerState) -> &'static str {
    match broker {
        BrokerState::Connecting => "Connecting to the broker…",
        BrokerState::Connected => "Connected to the broker",
        BrokerState::LoginRefused(_) => "Broker login refused",
        BrokerState::Disconnected(_) => "Not connected to the broker",
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
    }

    #[test]
    fn ago_shows_a_clock_time_in_the_macs_time_zone() {
        let text = ago(NOW - 7200, NOW);
        let clock = text.strip_prefix("at ").expect(&text);
        assert!(HhMm::parse(clock).is_some(), "{text}");
        assert_eq!(ago(NOW - 180, NOW), "3 min ago");
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
        // A battery level with no charging flag is still a battery level.
        assert_eq!(power_source(None, Some(80)), "battery 80%");
        assert_eq!(power_source(Some(7), Some(80)), "battery 80%");
        assert_eq!(power_source(Some(7), None), "not reported yet");
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
    fn nothing_to_say_is_not_given_an_age() {
        // A reading we can't put in words is "not reported yet", with no "reported ..."
        // after it, and nothing to grey out.
        let odd = reading(7, 600, Via::Wifi);
        let (text, stale) = reading_text(Some(&odd), NOW, |_| power_source(Some(7), None));
        assert_eq!(text, "not reported yet");
        assert!(!stale);
    }

    #[test]
    fn a_sent_value_waits_for_the_diffuser() {
        assert_eq!(pending_text("On"), "On · sent, waiting for the diffuser");
    }

    #[test]
    fn values_have_the_benchs_words() {
        assert_eq!((on_off(1), on_off(0)), ("On".to_owned(), "Off".to_owned()));
        // As the segmented control names them.
        assert_eq!(intensity(1), "Low");
        assert_eq!(intensity(2), "Medium");
        assert_eq!(intensity(3), "High");
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
    fn the_toolbar_says_the_broker_state_in_short() {
        assert_eq!(
            broker_summary(&BrokerState::Connected),
            "Connected to the broker"
        );
        assert_eq!(
            broker_summary(&BrokerState::Connecting),
            "Connecting to the broker…"
        );
        assert_eq!(
            broker_summary(&BrokerState::LoginRefused("Not authorized.".into())),
            "Broker login refused"
        );
        assert_eq!(
            broker_summary(&BrokerState::Disconnected(
                "No broker password saved.".into()
            )),
            "Not connected to the broker"
        );
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
