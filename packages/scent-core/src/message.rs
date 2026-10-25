use serde_json::{Map, Value};

use crate::topics::DEVICE_PREFIX;

/// Status array positions. Indices 0-3 are a header; 7, 10 and 13 are read by
/// nobody (7 is not the oil level, as once assumed).
const POWER: usize = 4;
const CHARGING: usize = 5;
const BATTERY: usize = 6;
const FAN: usize = 8;
const INTENSITY: usize = 9;
const GAS_PUMP: usize = 11;
const LIGHT: usize = 12;

/// What a diffuser reported about itself. A field is `None` when the message
/// didn't carry it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Status {
    pub power: Option<i64>,
    pub intensity: Option<i64>,
    pub charging: Option<i64>,
    pub battery: Option<i64>,
    pub fan: Option<i64>,
    pub gas_pump: Option<i64>,
    pub light: Option<i64>,
}

/// The shape of a Bluetooth notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    /// `S:1,L:3,C:2,Bat:200`
    StatusText,
    /// `{"data":[85,170,11,255,...]}`
    StatusArray,
    /// One `key:value` pair, such as `enable:1` or `led:50`.
    Value,
    /// Valid JSON that is not a status array.
    Json,
    Unknown,
}

/// A Bluetooth notification as the device sent it, plus what could be read from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// The text without the `-|` terminator or surrounding whitespace.
    pub raw: String,
    pub kind: NoteKind,
    pub status: Status,
}

/// Reads one Bluetooth notification. Never fails: anything unreadable is `Unknown`.
pub fn parse_notification(bytes: &[u8]) -> Notification {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    let text = text.strip_suffix("-|").unwrap_or(text).trim();
    let (kind, status) = classify(text);
    Notification {
        raw: text.to_owned(),
        kind,
        status,
    }
}

fn classify(text: &str) -> (NoteKind, Status) {
    if text.starts_with(['{', '['])
        && let Ok(json) = serde_json::from_str::<Value>(text)
    {
        return match json.get("data").and_then(Value::as_array) {
            Some(array) => (NoteKind::StatusArray, status_from_array(array)),
            None => (NoteKind::Json, Status::default()),
        };
    }
    if let Some(status) = status_from_text(text) {
        return (NoteKind::StatusText, status);
    }
    if let Some(status) = status_from_value(text) {
        return (NoteKind::Value, status);
    }
    (NoteKind::Unknown, Status::default())
}

fn status_from_array(array: &[Value]) -> Status {
    let at = |index: usize| array.get(index).and_then(Value::as_i64);
    Status {
        power: at(POWER),
        intensity: at(INTENSITY),
        charging: at(CHARGING),
        battery: at(BATTERY),
        fan: at(FAN),
        gas_pump: at(GAS_PUMP),
        light: at(LIGHT),
    }
}

/// `S:1,L:3,C:2,Bat:200`: at least two pairs, every key one we know.
fn status_from_text(text: &str) -> Option<Status> {
    let mut status = Status::default();
    let mut pairs = 0;
    for part in text.split(',') {
        let (key, value) = part.split_once(':')?;
        let field = match key.trim().to_ascii_lowercase().as_str() {
            "s" => &mut status.power,
            "l" => &mut status.intensity,
            "c" => &mut status.charging,
            "bat" => &mut status.battery,
            _ => return None,
        };
        *field = number(value);
        pairs += 1;
    }
    (pairs > 1).then_some(status)
}

/// One `key:value` pair. Keys we don't map (like `led`) still make it a value.
fn status_from_value(text: &str) -> Option<Status> {
    let (key, value) = text.split_once(':')?;
    let key = key.trim().to_ascii_lowercase();
    if key.is_empty() {
        return None;
    }
    let mut status = Status::default();
    match key.as_str() {
        "enable" => status.power = number(value),
        "level" => status.intensity = number(value),
        "battery" => status.battery = number(value),
        _ => {}
    }
    Some(status)
}

fn number(text: &str) -> Option<i64> {
    text.trim().parse().ok()
}

/// What the 0xFEF4 info characteristic holds: `manufacturer|model|variant|id|firmware|wifiConfigured|wifiStatus`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub manufacturer_id: String,
    pub model_name: String,
    pub variant_name: String,
    pub device_id: String,
    pub firmware: String,
    pub wifi_configured: bool,
    pub wifi_status: u8,
}

/// Reads the info string. Fields after the seventh are ignored.
pub fn parse_info(text: &str) -> Result<DeviceInfo, String> {
    let fields: Vec<&str> = text.trim().split('|').map(str::trim).collect();
    let [
        manufacturer,
        model,
        variant,
        device_id,
        firmware,
        configured,
        status,
        ..,
    ] = fields[..]
    else {
        return Err(format!(
            "expected 7 '|'-separated fields in the 0xFEF4 info string, got {}: {text:?}",
            fields.len()
        ));
    };
    Ok(DeviceInfo {
        manufacturer_id: manufacturer.to_owned(),
        model_name: model.to_owned(),
        variant_name: variant.to_owned(),
        device_id: device_id.to_owned(),
        firmware: firmware.to_owned(),
        wifi_configured: configured.eq_ignore_ascii_case("true"),
        wifi_status: status.parse().map_err(|_| {
            format!("the WiFi status field is not a number from 0 to 255: {status:?}")
        })?,
    })
}

/// Which way an MQTT message was going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Published to a `device/<id>` topic.
    ToDevice,
    FromDevice,
}

/// The result of the first `PING` with a user ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ack {
    Added,
    AlreadyAdded,
    Other(String),
}

/// A diffuser announcing that it joined the broker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connected {
    pub fw: String,
    pub model: String,
    pub variant: String,
    pub manufacturer: String,
}

/// One MQTT message, read as far as it can be.
#[derive(Debug, Clone, PartialEq)]
pub struct MqttMessage {
    pub direction: Direction,
    pub action: Option<String>,
    /// From the payload's `deviceId`, else from a `device/<id>` topic. Not checked
    /// with [`valid_device_id`](crate::valid_device_id).
    pub device_id: Option<String>,
    /// Set when the payload's `data` held something readable. It can be empty when
    /// the data parsed but had no status fields.
    pub status: Option<Status>,
    pub rssi: Option<i64>,
    /// Only for `acknowledgement` messages.
    pub ack: Option<Ack>,
    /// Only for `device_connected` messages.
    pub connected: Option<Connected>,
    /// False when the body was not valid JSON as received. The message may still
    /// have been repaired and read.
    pub valid_json: bool,
    /// The body as text, exactly as received (bad bytes replaced).
    pub raw: String,
}

/// Reads one MQTT message. Never fails: unreadable bodies give a message with
/// nothing in it but `raw` and `valid_json: false`.
///
/// Firmware 2.30 sends notifications with `"data":enable:0` (no quotes). That is
/// repaired by quoting the value and parsing again.
pub fn parse_mqtt(topic: &str, body: &[u8]) -> MqttMessage {
    let raw = String::from_utf8_lossy(body).into_owned();
    let parsed = serde_json::from_str::<Value>(&raw).ok();
    let mut message = MqttMessage {
        direction: if topic.starts_with(DEVICE_PREFIX) {
            Direction::ToDevice
        } else {
            Direction::FromDevice
        },
        action: None,
        device_id: topic.strip_prefix(DEVICE_PREFIX).map(str::to_owned),
        status: None,
        rssi: None,
        ack: None,
        connected: None,
        valid_json: parsed.is_some(),
        raw,
    };
    let document = parsed.or_else(|| serde_json::from_str(&quote_unquoted_data(&message.raw)).ok());
    let Some(root) = document.as_ref().and_then(Value::as_object) else {
        return message;
    };
    message.action = text_of(root, "action").map(str::to_owned);
    let Some(payload) = root.get("payload").and_then(Value::as_object) else {
        return message;
    };
    if let Some(id) = text_of(payload, "deviceId").filter(|id| !id.is_empty()) {
        message.device_id = Some(id.to_owned());
    }
    message.rssi = payload.get("rssi").and_then(Value::as_i64);
    message.status = payload.get("data").and_then(status_from_data);
    match message.action.as_deref() {
        Some("acknowledgement") => message.ack = ack_from(payload),
        Some("device_connected") => message.connected = Some(connected_from(payload)),
        _ => {}
    }
    message
}

fn text_of<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

/// `data` is Bluetooth-style text (`"S:1,L:3"`, a status array as a string) or,
/// on some firmware, the array's JSON object itself.
fn status_from_data(data: &Value) -> Option<Status> {
    let note = match data {
        Value::String(text) => parse_notification(text.as_bytes()),
        Value::Object(_) => parse_notification(data.to_string().as_bytes()),
        _ => return None,
    };
    (note.kind != NoteKind::Unknown).then_some(note.status)
}

/// The acknowledgement's `msg` is itself JSON text: `{"user_id":"...","result":"ADDED"}`.
fn ack_from(payload: &Map<String, Value>) -> Option<Ack> {
    let msg: Value = serde_json::from_str(text_of(payload, "msg")?).ok()?;
    Some(match msg.get("result")?.as_str()? {
        "ADDED" => Ack::Added,
        "ALREADY_ADDED" => Ack::AlreadyAdded,
        other => Ack::Other(other.to_owned()),
    })
}

/// Fields the device left out come back empty: the connect itself is what matters.
fn connected_from(payload: &Map<String, Value>) -> Connected {
    let field = |key| text_of(payload, key).unwrap_or_default().to_owned();
    Connected {
        fw: field("fw"),
        model: field("model"),
        variant: field("variant"),
        manufacturer: field("manufacturer"),
    }
}

const DATA_KEY: &str = "\"data\":";

/// Puts quotes around every `"data":` value that doesn't start with `"`, `{` or
/// `[`. The value runs to the next `,` or `}`. Meant for text that failed to parse
/// as JSON; valid JSON is better left alone.
fn quote_unquoted_data(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    let mut rest = text;
    while let Some(at) = rest.find(DATA_KEY) {
        let (head, tail) = rest.split_at(at + DATA_KEY.len());
        out.push_str(head);
        rest = tail;
        if matches!(rest.chars().next(), None | Some('"' | '{' | '[')) {
            continue;
        }
        let end = rest.find([',', '}']).unwrap_or(rest.len());
        let (value, tail) = rest.split_at(end);
        out.push_str(&Value::String(value.trim().to_owned()).to_string());
        rest = tail;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_TOPIC: &str = "scentiment-mqtt-prod/user/example-route";

    // parse_notification: cases ported from tools/tests/test_scentiment_ble.py

    #[test]
    fn status_text_reads_the_four_keys() {
        let n = parse_notification(b"S:1,L:3,C:2,Bat:200-|");
        assert_eq!(n.kind, NoteKind::StatusText);
        assert_eq!(n.raw, "S:1,L:3,C:2,Bat:200");
        assert_eq!(
            n.status,
            Status {
                power: Some(1),
                intensity: Some(3),
                charging: Some(2),
                battery: Some(200),
                ..Status::default()
            }
        );
    }

    #[test]
    fn status_array_from_scent_assistant_issue_42() {
        let n =
            parse_notification(br#"{"data":[85,170,11,255,0,2,200,100,255,0,0,255,255,0,48,106]}"#);
        assert_eq!(n.kind, NoteKind::StatusArray);
        assert_eq!(
            n.status,
            Status {
                power: Some(0),
                charging: Some(2),
                battery: Some(200),
                fan: Some(255),
                intensity: Some(0),
                gas_pump: Some(255),
                light: Some(255),
            }
        );
    }

    #[test]
    fn status_index_7_is_not_read_as_oil() {
        let n =
            parse_notification(br#"{"data":[85,170,11,255,1,2,200,50,255,2,3,255,255,0,92,106]}"#);
        assert_eq!(
            n.status,
            Status {
                power: Some(1),
                charging: Some(2),
                battery: Some(200),
                fan: Some(255),
                intensity: Some(2),
                gas_pump: Some(255),
                light: Some(255)
            }
        );
    }

    #[test]
    fn a_short_array_reads_only_the_indices_present() {
        let n = parse_notification(br#"{"data":[85,170,11,255,1,0,90]}"#);
        assert_eq!(n.kind, NoteKind::StatusArray);
        assert_eq!(
            n.status,
            Status {
                power: Some(1),
                charging: Some(0),
                battery: Some(90),
                ..Status::default()
            }
        );
    }

    #[test]
    fn array_entries_that_are_not_integers_are_skipped() {
        let n = parse_notification(br#"{"data":[85,170,11,255,"x",1.5,90]}"#);
        assert_eq!(n.kind, NoteKind::StatusArray);
        assert_eq!(
            n.status,
            Status {
                battery: Some(90),
                ..Status::default()
            }
        );
    }

    #[test]
    fn single_values_map_to_status_fields() {
        let power = parse_notification(b"enable:1");
        assert_eq!(power.kind, NoteKind::Value);
        assert_eq!(power.status.power, Some(1));
        assert_eq!(parse_notification(b"level:2").status.intensity, Some(2));
        assert_eq!(parse_notification(b" battery:80 ").status.battery, Some(80));
        assert_eq!(parse_notification(b"ENABLE:0").status.power, Some(0));
    }

    #[test]
    fn values_without_a_status_field_are_still_values() {
        let n = parse_notification(b"led:50");
        assert_eq!(n.kind, NoteKind::Value);
        assert_eq!(n.status, Status::default());
        assert_eq!(parse_notification(b"enable:soon").status, Status::default());
    }

    #[test]
    fn json_without_a_data_array_is_json() {
        let n = parse_notification(br#"{"other":1}"#);
        assert_eq!((n.kind, n.status), (NoteKind::Json, Status::default()));
        assert_eq!(parse_notification(b"[1,2]").kind, NoteKind::Json);
        assert_eq!(parse_notification(br#"{"data":"x"}"#).kind, NoteKind::Json);
    }

    #[test]
    fn parse_notification_never_panics() {
        for (bytes, kind) in [
            (&b"\xff\xfe"[..], NoteKind::Unknown),
            (b"", NoteKind::Unknown),
            (b"-|", NoteKind::Unknown),
            (b"hello", NoteKind::Unknown),
            (b"{not json", NoteKind::Unknown),
            (b"S:1,", NoteKind::Value),
            (b":5", NoteKind::Unknown),
        ] {
            assert_eq!(parse_notification(bytes).kind, kind, "{bytes:?}");
        }
        assert_eq!(parse_notification(b"hello").raw, "hello");
    }

    // parse_info: ported from the same file

    #[test]
    fn parse_info_reads_seven_fields() {
        assert_eq!(
            parse_info("1|Diffuser Air 2|Black|TESTDEVICE01|2.3.1|true|3"),
            Ok(DeviceInfo {
                manufacturer_id: "1".into(),
                model_name: "Diffuser Air 2".into(),
                variant_name: "Black".into(),
                device_id: "TESTDEVICE01".into(),
                firmware: "2.3.1".into(),
                wifi_configured: true,
                wifi_status: 3,
            })
        );
    }

    #[test]
    fn parse_info_trims_fields_and_ignores_extras() {
        let info = parse_info(" 03 | Hvac |Black| TESTDEVICE02 |2.30| False |2|extra\r\n").unwrap();
        assert_eq!(info.model_name, "Hvac");
        assert_eq!(info.device_id, "TESTDEVICE02");
        assert!(!info.wifi_configured);
        assert_eq!(info.wifi_status, 2);
        assert!(
            parse_info("01|Diffuser|Black|X|2.30|TRUE|0")
                .unwrap()
                .wifi_configured
        );
    }

    #[test]
    fn parse_info_rejects_short_text_and_names_the_count() {
        let error = parse_info("1|Diffuser Air 2|Black").unwrap_err();
        assert!(error.contains('7') && error.contains('3'), "{error}");
        assert!(parse_info("").is_err());
    }

    #[test]
    fn parse_info_rejects_a_status_that_is_not_a_small_number() {
        let error = parse_info("01|Diffuser|Black|TESTDEVICE01|2.30|true|ok").unwrap_err();
        assert!(error.contains("ok"), "{error}");
        assert!(parse_info("01|Diffuser|Black|TESTDEVICE01|2.30|true|300").is_err());
    }

    // parse_mqtt: cases ported from tools/tests/test_control_server.py

    #[test]
    fn repairs_the_unquoted_notification_data() {
        // Exactly what an Air 2 (firmware 2.30) sent on 2026-10-20 in reply to GET_STATUS.
        let raw = br#"{"action":"notification","payload":{"deviceId":"TESTDEVICE01","data":enable:0,"rssi":-46}}"#;
        let m = parse_mqtt(USER_TOPIC, raw);
        assert_eq!(m.action.as_deref(), Some("notification"));
        assert_eq!(m.device_id.as_deref(), Some("TESTDEVICE01"));
        assert!(!m.valid_json);
        assert_eq!(m.rssi, Some(-46));
        assert_eq!(m.status.unwrap().power, Some(0));
        assert_eq!(m.raw.as_bytes(), raw);
    }

    #[test]
    fn reads_a_status_array_inside_update_status() {
        let raw = br#"{"action":"update_status","payload":{"deviceId":"TESTDEVICE01","data":"{\"data\":[85,170,11,255,1,2,200,100,255,3,3,255,255,0,41,106]}"}}"#;
        let m = parse_mqtt(USER_TOPIC, raw);
        assert!(m.valid_json);
        let status = m.status.unwrap();
        assert_eq!((status.power, status.intensity), (Some(1), Some(3)));
    }

    #[test]
    fn reads_a_status_array_given_as_an_object() {
        // Firmware 2.30 over MQTT sends the array as an object, not a string (observed 2026-10-20).
        let raw = br#"{"action":"update_status","payload":{"deviceId":"TESTDEVICE01","data":{"data":[85,170,11,255,1,2,200,50,255,2,3,255,255,0,92,106]},"rssi":-66}}"#;
        let m = parse_mqtt(USER_TOPIC, raw);
        assert!(m.valid_json);
        let status = m.status.unwrap();
        assert_eq!((status.power, status.intensity), (Some(1), Some(2)));
        assert_eq!(m.rssi, Some(-66));
    }

    #[test]
    fn takes_the_device_id_from_a_device_topic() {
        let m = parse_mqtt(
            "scentiment-mqtt-prod/device/TESTDEVICE01",
            br#"{"action":"PING","payload":{"userid":"example-route"}}"#,
        );
        assert_eq!(m.direction, Direction::ToDevice);
        assert_eq!(m.device_id.as_deref(), Some("TESTDEVICE01"));
        assert_eq!(m.action.as_deref(), Some("PING"));
        assert_eq!(m.status, None);
    }

    #[test]
    fn the_payload_device_id_wins_over_the_topic() {
        let m = parse_mqtt(
            "scentiment-mqtt-prod/device/TESTDEVICE01",
            br#"{"action":"X","payload":{"deviceId":"TESTDEVICE02"}}"#,
        );
        assert_eq!(m.device_id.as_deref(), Some("TESTDEVICE02"));
        let empty = parse_mqtt(
            "scentiment-mqtt-prod/device/TESTDEVICE01",
            br#"{"action":"X","payload":{"deviceId":""}}"#,
        );
        assert_eq!(empty.device_id.as_deref(), Some("TESTDEVICE01"));
    }

    #[test]
    fn reads_device_connected() {
        let m = parse_mqtt(
            "scentiment-mqtt-prod/server-notifications/device",
            br#"{"action":"device_connected","payload":{"deviceId":"TESTDEVICE01","fw":"2.30","model":"Diffuser","variant":"Black","manufacturer":"01"}}"#,
        );
        assert_eq!(m.direction, Direction::FromDevice);
        assert_eq!(m.device_id.as_deref(), Some("TESTDEVICE01"));
        assert_eq!(
            m.connected,
            Some(Connected {
                fw: "2.30".into(),
                model: "Diffuser".into(),
                variant: "Black".into(),
                manufacturer: "01".into(),
            })
        );
    }

    #[test]
    fn reads_acknowledgement_results() {
        let ack = |result: &str| {
            let msg =
                serde_json::json!({ "user_id": "example-route", "result": result }).to_string();
            let body = serde_json::json!({
                "action": "acknowledgement",
                "payload": { "deviceId": "TESTDEVICE01", "msg": msg }
            });
            parse_mqtt(USER_TOPIC, body.to_string().as_bytes()).ack
        };
        assert_eq!(ack("ADDED"), Some(Ack::Added));
        assert_eq!(ack("ALREADY_ADDED"), Some(Ack::AlreadyAdded));
        assert_eq!(ack("NOPE"), Some(Ack::Other("NOPE".into())));

        for payload in [
            r#"{"msg":"not json"}"#,
            r#"{"msg":"{\"user_id\":\"x\"}"}"#,
            r#"{"msg":5}"#,
            r#"{}"#,
        ] {
            let body = format!(r#"{{"action":"acknowledgement","payload":{payload}}}"#);
            assert_eq!(
                parse_mqtt(USER_TOPIC, body.as_bytes()).ack,
                None,
                "{payload}"
            );
        }
        let elsewhere = parse_mqtt(
            USER_TOPIC,
            br#"{"action":"other","payload":{"msg":"{\"result\":\"ADDED\"}"}}"#,
        );
        assert_eq!(elsewhere.ack, None);
    }

    #[test]
    fn parse_mqtt_never_panics() {
        for body in [
            &b"\xff\xfe"[..],
            b"",
            b"{",
            b"null",
            br#"{"action":5}"#,
            br#"{"payload":{"data":{"data":"x"}}}"#,
        ] {
            let m = parse_mqtt("scentiment-mqtt-prod/user/example-route", body);
            assert!(!m.raw.is_empty() || body.is_empty());
        }
    }

    #[test]
    fn bodies_that_are_not_messages_give_an_empty_result() {
        let m = parse_mqtt("anything", b"\xff not json at all");
        assert_eq!((m.action, m.status, m.device_id), (None, None, None));
        assert!(!m.valid_json && !m.raw.is_empty());

        let null = parse_mqtt("anything", b"null");
        assert!(null.valid_json && null.action.is_none());
        assert!(!parse_mqtt("anything", b"").valid_json);
        // A non-string action is not an action.
        assert_eq!(parse_mqtt("anything", br#"{"action":5}"#).action, None);
    }

    #[test]
    fn the_repair_quotes_only_values_that_are_not_json_values() {
        assert_eq!(
            quote_unquoted_data(r#"{"a":{"data":enable:0,"rssi":-1}}"#),
            r#"{"a":{"data":"enable:0","rssi":-1}}"#
        );
        assert_eq!(quote_unquoted_data(r#"{"data":S:1}"#), r#"{"data":"S:1"}"#);
        // The value runs to the end of the text when nothing closes it.
        assert_eq!(quote_unquoted_data(r#"{"data":abc"#), r#"{"data":"abc""#);
        for untouched in [
            r#"{"data":"x"}"#,
            r#"{"data":{"data":[1]}}"#,
            r#"{"data":[1,2]}"#,
            "no data key",
            "é",
        ] {
            assert_eq!(quote_unquoted_data(untouched), untouched);
        }
        assert_eq!(
            quote_unquoted_data(r#"{"data":a,"x":{"data":b}}"#),
            r#"{"data":"a","x":{"data":"b"}}"#
        );
        // Quotes inside the value are escaped, not left to break the JSON.
        assert_eq!(quote_unquoted_data(r#"{"data":a"b}"#), r#"{"data":"a\"b"}"#);
    }
}
