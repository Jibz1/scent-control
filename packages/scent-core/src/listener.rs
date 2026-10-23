//! What the listener (apps/server) shares on the broker, as its readers decode it: each
//! diffuser's record at `scent-control/v1/state/<deviceId>` and the listener's heartbeat at
//! `scent-control/v1/listener` (spec 2026-10-22-listener-design, §6.4 and §6.5). The
//! contract is `fixtures/scent-control/`.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::status::ONLINE_FOR_S;

/// A time further ahead of the reader's clock than this makes a message unusable.
pub const DAY_AHEAD_S: i64 = 86_400;

/// The readings a record can hold. Others are dropped, so that a later listener can add one
/// without breaking older readers.
pub const FIELD_KEYS: [&str; 8] = [
    "power",
    "intensity",
    "charging",
    "battery",
    "fan",
    "gas_pump",
    "light",
    "rssi",
];

const VERSION: i64 = 1;

/// One value a diffuser reported, with when the listener heard it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamped {
    pub value: i64,
    pub at: i64,
}

/// What a diffuser said about itself when it last joined the broker, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectedAt {
    pub fw: String,
    pub model: String,
    pub variant: String,
    pub manufacturer: String,
    pub at: i64,
}

/// A diffuser's record as the listener publishes it. Times are seconds since 1970, on the
/// listener's clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedState {
    pub updated_at: i64,
    pub heard_at: Option<i64>,
    pub ping_at: Option<i64>,
    /// By [`FIELD_KEYS`] name.
    pub fields: BTreeMap<String, Stamped>,
    pub connected: Option<ConnectedAt>,
}

/// The listener's heartbeat, or its will (`running: false`, no time).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heartbeat {
    pub running: bool,
    pub at: Option<i64>,
}

/// Why a message can't be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unusable {
    /// Another version's.
    Version,
    /// Not JSON, or not the shape the version says.
    Invalid,
    /// A time more than [`DAY_AHEAD_S`] ahead of the reader's clock.
    Ahead,
}

fn object(body: &[u8]) -> Result<Map<String, Value>, Unusable> {
    match serde_json::from_slice(body) {
        Ok(Value::Object(object)) => Ok(object),
        _ => Err(Unusable::Invalid),
    }
}

fn check_version(object: &Map<String, Value>) -> Result<(), Unusable> {
    match object.get("v").and_then(Value::as_i64) {
        Some(VERSION) => Ok(()),
        Some(_) => Err(Unusable::Version),
        None => Err(Unusable::Invalid),
    }
}

/// An optional time: absent is fine, anything but an integer is not, nor one too far ahead.
fn time(value: Option<&Value>, now: i64) -> Result<Option<i64>, Unusable> {
    let Some(value) = value else {
        return Ok(None);
    };
    let at = value.as_i64().ok_or(Unusable::Invalid)?;
    if at > now.saturating_add(DAY_AHEAD_S) {
        return Err(Unusable::Ahead);
    }
    Ok(Some(at))
}

fn required_time(value: Option<&Value>, now: i64) -> Result<i64, Unusable> {
    time(value, now)?.ok_or(Unusable::Invalid)
}

fn text(object: &Map<String, Value>, key: &str) -> Result<String, Unusable> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(Unusable::Invalid)
}

fn connected_from(value: &Value, now: i64) -> Result<ConnectedAt, Unusable> {
    let object = value.as_object().ok_or(Unusable::Invalid)?;
    Ok(ConnectedAt {
        fw: text(object, "fw")?,
        model: text(object, "model")?,
        variant: text(object, "variant")?,
        manufacturer: text(object, "manufacturer")?,
        at: required_time(object.get("at"), now)?,
    })
}

/// A diffuser's record, read back. Field keys it doesn't know are dropped; a known one in
/// the wrong shape makes the whole record unusable.
pub fn decode_state(body: &[u8], now: i64) -> Result<SharedState, Unusable> {
    let object = object(body)?;
    check_version(&object)?;
    let updated_at = required_time(object.get("updated_at"), now)?;
    let heard_at = time(object.get("heard_at"), now)?;
    let ping_at = time(object.get("ping_at"), now)?;
    let raw = object
        .get("fields")
        .and_then(Value::as_object)
        .ok_or(Unusable::Invalid)?;
    let mut fields = BTreeMap::new();
    for key in FIELD_KEYS {
        let Some(entry) = raw.get(key) else {
            continue;
        };
        let entry = entry.as_object().ok_or(Unusable::Invalid)?;
        let value = entry
            .get("value")
            .and_then(Value::as_i64)
            .ok_or(Unusable::Invalid)?;
        let at = required_time(entry.get("at"), now)?;
        fields.insert(key.to_owned(), Stamped { value, at });
    }
    let connected = object
        .get("connected")
        .map(|value| connected_from(value, now))
        .transpose()?;
    Ok(SharedState {
        updated_at,
        heard_at,
        ping_at,
        fields,
        connected,
    })
}

/// The listener's heartbeat or will, read back.
pub fn decode_heartbeat(body: &[u8], now: i64) -> Result<Heartbeat, Unusable> {
    let object = object(body)?;
    check_version(&object)?;
    let running = object
        .get("running")
        .and_then(Value::as_bool)
        .ok_or(Unusable::Invalid)?;
    let at = time(object.get("at"), now)?;
    Ok(Heartbeat { running, at })
}

/// Whether the listener is running: its last heartbeat said so within [`ONLINE_FOR_S`].
/// Otherwise what it shared may be out of date.
pub fn listener_running(heartbeat: Option<&Heartbeat>, now: i64) -> bool {
    heartbeat.is_some_and(|beat| {
        beat.running
            && beat
                .at
                .is_some_and(|at| now.saturating_sub(at) <= ONLINE_FOR_S)
    })
}

/// Whether a reading taken at `reading_at` still counts: one from before the diffuser's last
/// reconnect does not (owner, 2026-10-22). One from the same second does.
pub fn is_current(reading_at: i64, connected_at: Option<i64>) -> bool {
    connected_at.is_none_or(|connected| reading_at >= connected)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_792_510_200;

    fn state(extra: &str) -> Vec<u8> {
        format!(r#"{{"v":1,"updated_at":{NOW},"fields":{{}}{extra}}}"#).into_bytes()
    }

    #[test]
    fn another_version_is_unusable() {
        let other = format!(r#"{{"v":2,"updated_at":{NOW},"fields":{{}}}}"#);
        assert_eq!(decode_state(other.as_bytes(), NOW), Err(Unusable::Version));
        let beat = format!(r#"{{"v":2,"running":true,"at":{NOW}}}"#);
        assert_eq!(
            decode_heartbeat(beat.as_bytes(), NOW),
            Err(Unusable::Version)
        );
    }

    #[test]
    fn a_time_more_than_a_day_ahead_is_unusable() {
        let ahead = NOW + 86_401;
        for extra in [
            format!(r#","heard_at":{ahead}"#),
            format!(r#","ping_at":{ahead}"#),
            format!(
                r#","connected":{{"fw":"","model":"","variant":"","manufacturer":"","at":{ahead}}}"#
            ),
        ] {
            assert_eq!(
                decode_state(&state(&extra), NOW),
                Err(Unusable::Ahead),
                "{extra}"
            );
        }
        let field = format!(
            r#"{{"v":1,"updated_at":{NOW},"fields":{{"power":{{"value":1,"at":{ahead}}}}}}}"#
        );
        assert_eq!(decode_state(field.as_bytes(), NOW), Err(Unusable::Ahead));
        let updated = format!(r#"{{"v":1,"updated_at":{ahead},"fields":{{}}}}"#);
        assert_eq!(decode_state(updated.as_bytes(), NOW), Err(Unusable::Ahead));
        let beat = format!(r#"{{"v":1,"running":true,"at":{ahead}}}"#);
        assert_eq!(decode_heartbeat(beat.as_bytes(), NOW), Err(Unusable::Ahead));
        let edge = format!(r#"{{"v":1,"updated_at":{},"fields":{{}}}}"#, NOW + 86_400);
        assert!(decode_state(edge.as_bytes(), NOW).is_ok());
    }

    #[test]
    fn an_empty_or_misshapen_body_is_invalid() {
        for body in [
            Vec::new(),
            b"not json".to_vec(),
            b"[]".to_vec(),
            br#"{"updated_at":1,"fields":{}}"#.to_vec(),
            br#"{"v":1,"fields":{}}"#.to_vec(),
            br#"{"v":1,"updated_at":"now","fields":{}}"#.to_vec(),
            br#"{"v":1,"updated_at":1}"#.to_vec(),
            state(r#","heard_at":1.5"#),
            state(r#","connected":{"fw":2.3,"model":"","variant":"","manufacturer":"","at":1}"#),
            br#"{"v":1,"updated_at":1,"fields":{"power":{"value":"1","at":1}}}"#.to_vec(),
            br#"{"v":1,"updated_at":1,"fields":{"power":1}}"#.to_vec(),
        ] {
            assert_eq!(
                decode_state(&body, NOW),
                Err(Unusable::Invalid),
                "{}",
                String::from_utf8_lossy(&body)
            );
        }
        for body in [
            &b""[..],
            b"{}",
            br#"{"v":1}"#,
            br#"{"v":1,"running":"yes"}"#,
        ] {
            assert_eq!(decode_heartbeat(body, NOW), Err(Unusable::Invalid));
        }
    }

    #[test]
    fn field_keys_it_does_not_know_are_dropped() {
        let body = format!(
            r#"{{"v":1,"updated_at":{NOW},"fields":{{"power":{{"value":1,"at":{NOW}}},"smell":{{"value":3,"at":{NOW}}},"odd":"x"}}}}"#
        );
        let state = decode_state(body.as_bytes(), NOW).expect("usable");
        assert_eq!(
            state.fields.into_iter().collect::<Vec<_>>(),
            [("power".to_owned(), Stamped { value: 1, at: NOW })]
        );
    }

    #[test]
    fn the_listener_runs_for_75_seconds_after_its_heartbeat() {
        let beat = Heartbeat {
            running: true,
            at: Some(NOW),
        };
        assert!(listener_running(Some(&beat), NOW + 75));
        assert!(!listener_running(Some(&beat), NOW + 76));
        let will = Heartbeat {
            running: false,
            at: None,
        };
        assert!(!listener_running(Some(&will), NOW));
        assert!(!listener_running(
            Some(&Heartbeat {
                running: true,
                at: None
            }),
            NOW
        ));
        assert!(!listener_running(None, NOW));
    }

    #[test]
    fn a_reading_in_the_same_second_as_the_reconnect_is_current() {
        assert!(is_current(NOW, Some(NOW)));
        assert!(!is_current(NOW - 1, Some(NOW)));
        assert!(is_current(NOW - 1000, None));
    }
}
