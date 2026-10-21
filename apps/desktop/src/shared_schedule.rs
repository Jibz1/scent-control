//! The copy of a diffuser's last schedule that the app keeps on our own broker, so
//! that every copy of the app knows it (interim, until the Phase 2 server keeps the
//! master copy).
//!
//! A diffuser can't say what schedule it holds, so the app remembers the last one it
//! sent. When it sends one, it also stores it as a retained message at
//! `scent-control/v1/schedule/<deviceId>` (see `scent_core::schedule_topic`), and every
//! copy that connects reads it back: the newer copy wins. The payload is
//! `{"v":1,"profile":<the schedule as desktop.json keeps it>,"sent_at":<unix seconds>}`.
//! It holds times, days, levels and a time zone offset: nothing secret.

use scent_core::Profile;
use serde::{Deserialize, Serialize};

use crate::store::StoredProfile;

/// The only version of the payload this app writes and reads.
const VERSION: u64 = 1;

/// A schedule stored on the broker, and when it was sent to the diffuser.
#[derive(Debug, Clone, PartialEq)]
pub struct SharedSchedule {
    pub profile: Profile,
    /// Seconds since 1970.
    pub sent_at: i64,
}

/// Why a stored schedule can't be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreadable {
    /// Its `v` is not 1: another version of the app wrote it.
    Version,
    /// It isn't JSON, or not ours, or its schedule can't be.
    Invalid,
}

/// The message as it is on the broker.
#[derive(Serialize, Deserialize)]
struct Payload {
    v: u64,
    profile: StoredProfile,
    sent_at: i64,
}

/// The payload for `profile`, sent at `sent_at`. `None` when the zone's offset is not a
/// number, which JSON can't hold (the editor never sends one).
pub fn encode(profile: &Profile, sent_at: i64) -> Option<Vec<u8>> {
    if !profile.timezone_hours.is_finite() {
        return None;
    }
    let payload = Payload {
        v: VERSION,
        profile: StoredProfile::from(profile),
        sent_at,
    };
    serde_json::to_vec(&payload).ok()
}

/// The schedule in a payload. The version is read first, so a payload from another
/// version is told apart from a broken one whatever else it holds.
pub fn decode(body: &[u8]) -> Result<SharedSchedule, Unreadable> {
    let json: serde_json::Value = serde_json::from_slice(body).map_err(|_| Unreadable::Invalid)?;
    match json.get("v") {
        None => return Err(Unreadable::Invalid),
        Some(v) if v.as_u64() == Some(VERSION) => {}
        Some(_) => return Err(Unreadable::Version),
    }
    let payload: Payload = serde_json::from_value(json).map_err(|_| Unreadable::Invalid)?;
    let profile = payload.profile.into_profile().ok_or(Unreadable::Invalid)?;
    Ok(SharedSchedule {
        profile,
        sent_at: payload.sent_at,
    })
}

#[cfg(test)]
mod tests {
    use scent_core::{Profile, ScheduleFlag};

    use super::*;
    use crate::store::sent_schedule;

    const SENT_AT: i64 = 1_790_000_000;

    fn json(body: &[u8]) -> serde_json::Value {
        serde_json::from_slice(body).unwrap()
    }

    #[test]
    fn a_schedule_goes_out_and_comes_back_the_same() {
        let body = encode(&sent_schedule(), SENT_AT).unwrap();
        assert_eq!(
            decode(&body),
            Ok(SharedSchedule {
                profile: sent_schedule(),
                sent_at: SENT_AT,
            })
        );
        // So does one where only the always-on entry went out last.
        let always_on_only = Profile {
            flag: ScheduleFlag::AlwaysOnOnly,
            ..sent_schedule()
        };
        let body = encode(&always_on_only, 7).unwrap();
        assert_eq!(decode(&body).unwrap().profile, always_on_only);
    }

    #[test]
    fn the_payload_is_the_version_the_schedule_as_saved_and_when_it_was_sent() {
        let body = encode(&sent_schedule(), SENT_AT).unwrap();
        let json = json(&body);
        let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["v", "profile", "sent_at"]);
        assert_eq!(json["v"], 1);
        assert_eq!(json["sent_at"], SENT_AT);
        // The schedule is in the form desktop.json keeps it in.
        assert_eq!(json["profile"]["flag"], "replace");
        assert_eq!(json["profile"]["timezone_hours"], 5.5);
        assert_eq!(json["profile"]["always_on"]["level"], 3);
        assert_eq!(json["profile"]["entries"][0]["start"], "07:00");
        assert_eq!(json["profile"]["entries"][0]["weekdays"][0], 1);
        // Nothing secret: a schedule is times, days and levels.
        let text = String::from_utf8(body).unwrap().to_lowercase();
        for secret in ["password", "secret", "token", "ssid"] {
            assert!(!text.contains(secret), "{text}");
        }
    }

    #[test]
    fn a_zone_that_is_not_a_number_is_not_written() {
        for hours in [f64::NAN, f64::INFINITY] {
            let profile = Profile {
                timezone_hours: hours,
                ..sent_schedule()
            };
            assert_eq!(encode(&profile, SENT_AT), None, "{hours}");
        }
    }

    #[test]
    fn only_version_1_is_read() {
        let body = encode(&sent_schedule(), SENT_AT).unwrap();
        for v in [
            serde_json::json!(0),
            serde_json::json!(2),
            serde_json::json!("1"),
        ] {
            let mut json = json(&body);
            json["v"] = v.clone();
            let bytes = serde_json::to_vec(&json).unwrap();
            assert_eq!(decode(&bytes), Err(Unreadable::Version), "{v}");
        }
        // A version 2 whose schedule looks nothing like ours is still a version 2.
        let later = br#"{"v":2,"profile":{"rooms":[]},"sent_at":"yesterday"}"#;
        assert_eq!(decode(later), Err(Unreadable::Version));
    }

    #[test]
    fn what_is_not_a_schedule_of_ours_is_invalid() {
        let good = encode(&sent_schedule(), SENT_AT).unwrap();
        let good = json(&good);
        let mut no_version = good.clone();
        no_version.as_object_mut().unwrap().remove("v");
        let mut no_time = good.clone();
        no_time.as_object_mut().unwrap().remove("sent_at");
        let mut bad_time = good.clone();
        bad_time["sent_at"] = serde_json::json!(1.5);
        let mut bad_level = good.clone();
        bad_level["profile"]["entries"][0]["level"] = serde_json::json!(4);
        let mut bad_day = good.clone();
        bad_day["profile"]["entries"][0]["weekdays"] = serde_json::json!([7]);
        let mut bad_flag = good.clone();
        bad_flag["profile"]["flag"] = serde_json::json!("sometimes");
        let mut bad_zone = good;
        bad_zone["profile"]["timezone_hours"] = serde_json::Value::Null;
        let shaped: Vec<Vec<u8>> = [
            no_version, no_time, bad_time, bad_level, bad_day, bad_flag, bad_zone,
        ]
        .iter()
        .map(|json| serde_json::to_vec(json).unwrap())
        .collect();
        let broken: [&[u8]; 6] = [
            b"",
            b"{",
            b"not json",
            b"[1]",
            b"{\"v\":1}",
            &[0xff, 0xfe, 0x00],
        ];
        for body in shaped.iter().map(Vec::as_slice).chain(broken) {
            assert_eq!(
                decode(body),
                Err(Unreadable::Invalid),
                "{}",
                String::from_utf8_lossy(body)
            );
        }
    }
}
