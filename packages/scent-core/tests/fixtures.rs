//! The fixture contract: every captured message in `fixtures/scentiment/` must
//! parse the way the Phase 0 tools read it.

use std::fs;
use std::path::PathBuf;

use scent_core::{
    Ack, Direction, NoteKind, parse_info, parse_mqtt, parse_notification, valid_device_id,
};
use serde_json::Value;

const SYNTHETIC_IDS: [&str; 2] = ["TESTDEVICE01", "TESTDEVICE02"];

struct Fixture {
    name: String,
    device: String,
    channel: String,
    topic: Option<String>,
    text: String,
}

fn fixtures() -> Vec<Fixture> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/scentiment");
    let mut found = Vec::new();
    for entry in fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let field = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let required = |key: &str| field(key).unwrap_or_else(|| panic!("{name}: no {key}"));
        found.push(Fixture {
            device: required("device"),
            channel: required("channel"),
            topic: field("topic"),
            text: required("text"),
            name,
        });
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

fn fixture(name: &str) -> Fixture {
    fixtures()
        .into_iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no fixture named {name}"))
}

fn on_channel(channel: &str) -> Vec<Fixture> {
    fixtures()
        .into_iter()
        .filter(|f| f.channel == channel)
        .collect()
}

fn parse_mqtt_fixture(f: &Fixture) -> scent_core::MqttMessage {
    let topic = f
        .topic
        .as_deref()
        .unwrap_or_else(|| panic!("{}: no topic", f.name));
    parse_mqtt(topic, f.text.as_bytes())
}

#[test]
fn the_captured_fixtures_are_all_found() {
    // Phase 0 captured 16. More may be added; fewer means a file went missing.
    let all = fixtures();
    assert!(
        all.len() >= 16,
        "{:?}",
        all.iter().map(|f| &f.name).collect::<Vec<_>>()
    );
    assert!(on_channel("ble-info").len() >= 2);
    assert!(on_channel("ble-notify").len() >= 7);
    assert!(on_channel("mqtt").len() >= 7);
}

#[test]
fn info_fixtures_parse_with_a_synthetic_device_id() {
    for f in on_channel("ble-info") {
        let info = parse_info(&f.text).unwrap_or_else(|e| panic!("{}: {e}", f.name));
        assert!(
            SYNTHETIC_IDS.contains(&info.device_id.as_str()),
            "{}",
            f.name
        );
        let expected = if f.device == "air2" {
            "TESTDEVICE01"
        } else {
            "TESTDEVICE02"
        };
        assert_eq!(info.device_id, expected, "{}", f.name);
        assert!(
            info.wifi_configured && info.firmware == "2.30",
            "{}",
            f.name
        );
    }
}

#[test]
fn notify_fixtures_are_recognised() {
    for f in on_channel("ble-notify") {
        let kind = parse_notification(f.text.as_bytes()).kind;
        assert_ne!(kind, NoteKind::Unknown, "{}", f.name);
        let expected = if f.name.contains("status-array") {
            NoteKind::StatusArray
        } else if f.name.contains("status-text") {
            NoteKind::StatusText
        } else {
            NoteKind::Value
        };
        assert_eq!(kind, expected, "{}", f.name);
    }
}

#[test]
fn mqtt_fixtures_parse_with_an_action_and_a_synthetic_device_id() {
    for f in on_channel("mqtt") {
        let m = parse_mqtt_fixture(&f);
        assert!(m.action.is_some(), "{}", f.name);
        let id = m
            .device_id
            .as_deref()
            .unwrap_or_else(|| panic!("{}: no device id", f.name));
        assert!(
            SYNTHETIC_IDS.contains(&id) && valid_device_id(id),
            "{}",
            f.name
        );
        assert_eq!(m.direction, Direction::FromDevice, "{}", f.name);
        assert_eq!(m.raw, f.text, "{}", f.name);
        // The device's own invalid JSON is the only message that needs repair.
        assert_eq!(
            m.valid_json,
            f.name != "air2-mqtt-notification-invalid-json",
            "{}",
            f.name
        );
    }
}

#[test]
fn the_invalid_json_fixture_is_repaired() {
    let m = parse_mqtt_fixture(&fixture("air2-mqtt-notification-invalid-json"));
    assert!(!m.valid_json);
    assert_eq!(m.action.as_deref(), Some("notification"));
    assert_eq!(m.status.expect("repaired status").power, Some(0));
    assert_eq!(m.rssi, Some(-46));
}

#[test]
fn the_status_object_fixture_gives_a_status() {
    let m = parse_mqtt_fixture(&fixture("air2-mqtt-update-status-object"));
    let status = m.status.expect("status");
    assert_eq!(
        (status.power, status.intensity, status.battery),
        (Some(1), Some(2), Some(200))
    );
    assert_eq!(m.rssi, Some(-66));
}

#[test]
fn the_ack_fixtures_carry_their_result() {
    let added = parse_mqtt_fixture(&fixture("air2-mqtt-ack-added"));
    assert_eq!(added.ack, Some(Ack::Added));
    let again = parse_mqtt_fixture(&fixture("air2-mqtt-ack-already-added"));
    assert_eq!(again.ack, Some(Ack::AlreadyAdded));
}

#[test]
fn the_device_connected_fixtures_carry_model_info() {
    for name in [
        "air2-mqtt-device-connected-server-notifications",
        "air2-mqtt-device-connected-user",
        "hvac-mqtt-device-connected-user",
    ] {
        let connected = parse_mqtt_fixture(&fixture(name)).connected.expect(name);
        assert_eq!(connected.fw, "2.30", "{name}");
        assert_eq!(connected.variant, "Black", "{name}");
    }
    let hvac = parse_mqtt_fixture(&fixture("hvac-mqtt-device-connected-user"));
    assert_eq!(hvac.connected.unwrap().model, "Hvac");
}
