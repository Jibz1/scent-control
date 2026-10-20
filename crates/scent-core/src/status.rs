//! Whether a diffuser is reachable, and whether a reading is still current.
//! Times are seconds since 1970, as `i64`.

/// How often to ping each diffuser.
pub const PING_EVERY_S: i64 = 30;
/// How long after a ping to wait for the answer before calling it offline.
pub const ANSWER_WITHIN_S: i64 = 15;
/// How long after hearing from a diffuser to still call it online.
pub const ONLINE_FOR_S: i64 = 75;
/// How old a reading may get before the app greys it out.
pub const STALE_AFTER_S: i64 = 300;

/// What we can say about a diffuser's connection right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Online {
    /// Heard from within the last [`ONLINE_FOR_S`].
    Online,
    /// Pinged within the last [`ANSWER_WITHIN_S`] and no answer yet.
    Waiting,
    /// Pinged longer ago than [`ANSWER_WITHIN_S`] and nothing heard since.
    Offline,
    /// No recent evidence either way.
    Unknown,
}

/// `ping_at` is when we last pinged, `heard_at` when we last heard anything from
/// the diffuser. Hearing something at or after the last ping (or with no ping at
/// all) settles it: online if recent, unknown if not. Otherwise a ping is still
/// outstanding: waiting for [`ANSWER_WITHIN_S`], then offline.
pub fn online_status(ping_at: Option<i64>, heard_at: Option<i64>, now: i64) -> Online {
    match (ping_at, heard_at) {
        (ping, Some(heard)) if ping.is_none_or(|p| heard >= p) => {
            if now.saturating_sub(heard) <= ONLINE_FOR_S {
                Online::Online
            } else {
                Online::Unknown
            }
        }
        (Some(ping), _) => {
            if now.saturating_sub(ping) <= ANSWER_WITHIN_S {
                Online::Waiting
            } else {
                Online::Offline
            }
        }
        (None, _) => Online::Unknown,
    }
}

/// A reading taken at `at` is stale once more than [`STALE_AFTER_S`] have passed.
pub fn is_stale(at: i64, now: i64) -> bool {
    now.saturating_sub(at) > STALE_AFTER_S
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_792_510_200;

    // Ported from tools/tests/test_control_server.py (online_status_from_ping_and_ack).
    #[test]
    fn online_status_from_ping_and_answer() {
        // never pinged, never heard
        assert_eq!(online_status(None, None, NOW), Online::Unknown);
        // pinged 10 s ago, answered 1 s later
        assert_eq!(
            online_status(Some(NOW - 10), Some(NOW - 9), NOW),
            Online::Online
        );
        // pinged 10 s ago; the last thing heard is older than the ping
        assert_eq!(
            online_status(Some(NOW - 10), Some(NOW - 120), NOW),
            Online::Waiting
        );
        // pinged 30 s ago, still nothing newer
        assert_eq!(
            online_status(Some(NOW - 30), Some(NOW - 610), NOW),
            Online::Offline
        );
    }

    #[test]
    fn pinged_but_never_heard_waits_then_goes_offline() {
        assert_eq!(online_status(Some(NOW - 1), None, NOW), Online::Waiting);
        assert_eq!(online_status(Some(NOW - 15), None, NOW), Online::Waiting);
        assert_eq!(online_status(Some(NOW - 16), None, NOW), Online::Offline);
    }

    #[test]
    fn heard_recently_counts_as_online_even_without_a_ping() {
        assert_eq!(online_status(None, Some(NOW - 75), NOW), Online::Online);
        assert_eq!(online_status(None, Some(NOW - 76), NOW), Online::Unknown);
    }

    #[test]
    fn an_old_answer_after_the_last_ping_is_unknown_not_offline() {
        // Heard after the ping, but long ago: we have no fresh evidence either way.
        assert_eq!(
            online_status(Some(NOW - 600), Some(NOW - 590), NOW),
            Online::Unknown
        );
    }

    #[test]
    fn an_answer_in_the_same_second_as_the_ping_counts() {
        assert_eq!(
            online_status(Some(NOW - 5), Some(NOW - 5), NOW),
            Online::Online
        );
    }

    #[test]
    fn a_reading_is_stale_only_after_300_seconds() {
        assert!(!is_stale(NOW, NOW));
        assert!(!is_stale(NOW - 300, NOW));
        assert!(is_stale(NOW - 301, NOW));
        // A timestamp from the future (clock step) is not stale.
        assert!(!is_stale(NOW + 60, NOW));
    }

    #[test]
    fn extreme_timestamps_do_not_overflow() {
        assert!(is_stale(i64::MIN, i64::MAX));
        assert_eq!(
            online_status(Some(i64::MIN), Some(i64::MIN), i64::MAX),
            Online::Unknown
        );
    }

    #[test]
    fn the_timings_are_the_agreed_ones() {
        assert_eq!(
            (PING_EVERY_S, ANSWER_WITHIN_S, ONLINE_FOR_S, STALE_AFTER_S),
            (30, 15, 75, 300)
        );
    }
}
