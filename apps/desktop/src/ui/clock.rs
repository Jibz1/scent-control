//! The Mac's clock as the screens need it: its offset from UTC, and the time of day at
//! an offset. This is the only place that asks the system for the time zone; the rest
//! of the screens are given the time and the offset.

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
pub fn time_of_day(at: i64, utc_offset_secs: i64) -> (i64, i64, i64) {
    let secs = at.saturating_add(utc_offset_secs).rem_euclid(86_400);
    (secs / 3600, secs / 60 % 60, secs % 60)
}

/// `HH:MM:SS` at `utc_offset_secs` from UTC.
pub fn clock_hms(at: i64, utc_offset_secs: i64) -> String {
    let (hour, minute, second) = time_of_day(at, utc_offset_secs);
    format!("{hour:02}:{minute:02}:{second:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-20 15:30:00 UTC.
    const NOW: i64 = 1_792_510_200;
    const UTC_MINUS_5: i64 = -5 * 3600;

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
    fn extreme_timestamps_do_not_overflow() {
        // A saved file can hold anything.
        let _ = clock_hms(i64::MAX, i64::MAX);
        let _ = clock_hms(i64::MIN, i64::MIN);
        let _ = time_of_day(i64::MAX, i64::MAX);
        let _ = local_utc_offset_secs(i64::MIN);
        let _ = local_utc_offset_secs(i64::MAX);
    }
}
