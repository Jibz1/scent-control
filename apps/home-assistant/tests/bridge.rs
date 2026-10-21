use scent_home_assistant::{bridge::Bridge, config::Config};
use serde_json::{Value, json};

const EXAMPLE: &str = include_str!("../config.example.json");
const REPORT: &str = "scentiment-mqtt-prod/user/route-test-1";
const PREFIX: &str = "scent-control/example-home/TESTDEVICE01";

fn bridge() -> Bridge {
    Bridge::new(Config::parse(EXAMPLE.as_bytes()).unwrap())
}

fn fixture(name: &str) -> (String, Vec<u8>) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/scentiment")
        .join(name);
    let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    (
        fixture["topic"].as_str().unwrap().to_owned(),
        fixture["text"].as_str().unwrap().as_bytes().to_vec(),
    )
}

fn report(bridge: &mut Bridge, file: &str, at: i64) {
    let (topic, body) = fixture(file);
    bridge.report(&topic, &body, false, at);
}

fn state(bridge: &Bridge, field: &str, at: i64) -> String {
    let topic = format!("{PREFIX}/{field}");
    String::from_utf8(
        bridge
            .snapshot(at)
            .into_iter()
            .find(|p| p.topic == topic)
            .unwrap()
            .payload,
    )
    .unwrap()
}

#[test]
fn discovery_defines_a_three_speed_fan_without_unsupported_capabilities() {
    let b = bridge();
    let config: Value = serde_json::from_slice(&b.discovery()[0].payload).unwrap();
    assert_eq!(config["speed_range_max"], 3);
    assert_eq!(config["speed_range_min"], 1);
    assert_eq!(config["optimistic"], false);
    assert_eq!(config["retain"], false);
    assert_eq!(config["qos"], 0);
    assert_eq!(config["availability_mode"], "all");
    assert_eq!(config["availability"].as_array().unwrap().len(), 2);
    assert_eq!(
        config["unique_id"],
        "scent_control_example-home_TESTDEVICE01"
    );
    assert_eq!(
        b.command_topics(),
        vec![format!("{PREFIX}/power/set"), format!("{PREFIX}/level/set")]
    );
    assert_eq!(state(&b, "power/state", 0), "None");
    assert_eq!(state(&b, "level/state", 0), "None");
    assert_eq!(state(&b, "availability", 0), "offline");
}

#[test]
fn real_fixtures_drive_state_including_firmware_invalid_json() {
    let mut b = bridge();
    report(&mut b, "air2-mqtt-update-status-object.json", 10);
    assert_eq!(state(&b, "power/state", 10), "ON");
    assert_eq!(state(&b, "level/state", 10), "2");
    assert_eq!(state(&b, "availability", 10), "online");
    report(&mut b, "air2-mqtt-notification-invalid-json.json", 20);
    assert_eq!(state(&b, "power/state", 20), "OFF");
    assert_eq!(state(&b, "level/state", 20), "2");
}

#[test]
fn pings_do_not_refresh_old_readings_and_fields_expire_independently() {
    let mut b = bridge();
    report(&mut b, "air2-mqtt-update-status-object.json", 10);
    report(&mut b, "air2-mqtt-notification-invalid-json.json", 20);
    report(&mut b, "air2-mqtt-ack-already-added.json", 310);
    assert_eq!(state(&b, "availability", 311), "online");
    assert_eq!(state(&b, "power/state", 311), "OFF");
    assert_eq!(state(&b, "level/state", 311), "None");
    assert_eq!(state(&b, "power/state", 321), "None");
    assert_eq!(state(&b, "availability", 385), "online");
    assert_eq!(state(&b, "availability", 386), "offline");
}

#[test]
fn commands_use_exact_codec_and_never_confirm_themselves() {
    let mut b = bridge();
    report(&mut b, "air2-mqtt-update-status-object.json", 10);
    let commands = b
        .command(&format!("{PREFIX}/level/set"), b"3", false, 11)
        .unwrap();
    assert_eq!(commands.len(), 2);
    assert_eq!(
        commands[0].payload,
        br#"{"action":"SET_LEVEL","payload":{"intensity":3}}"#
    );
    assert_eq!(
        commands[1].payload,
        br#"{"action":"TURN_ON","payload":{"on":1}}"#
    );
    assert!(
        commands
            .iter()
            .all(|p| !p.retain && p.topic == "scentiment-mqtt-prod/device/TESTDEVICE01")
    );
    assert_eq!(state(&b, "level/state", 11), "2");
    for (field, value) in [("power", b"OFF".as_slice()), ("level", b"0")] {
        let commands = b
            .command(&format!("{PREFIX}/{field}/set"), value, false, 11)
            .unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].payload,
            br#"{"action":"TURN_ON","payload":{"on":0}}"#
        );
    }
}

#[test]
fn turning_on_uses_reported_level_or_defaults_to_low() {
    let mut b = bridge();
    report(&mut b, "air2-mqtt-ack-already-added.json", 0);
    let commands = b
        .command(&format!("{PREFIX}/power/set"), b"ON", false, 1)
        .unwrap();
    assert!(String::from_utf8_lossy(&commands[0].payload).contains("\"intensity\":1"));
    report(&mut b, "air2-mqtt-update-status-object.json", 2);
    let commands = b
        .command(&format!("{PREFIX}/power/set"), b"ON", false, 3)
        .unwrap();
    assert!(String::from_utf8_lossy(&commands[0].payload).contains("\"intensity\":2"));
}

#[test]
fn invalid_retained_unknown_and_offline_commands_are_rejected() {
    let mut b = bridge();
    let topic = format!("{PREFIX}/power/set");
    assert!(b.command(&topic, b"ON", false, 0).is_err());
    report(&mut b, "air2-mqtt-ack-already-added.json", 0);
    assert!(b.command(&topic, b"ON", true, 0).is_err());
    assert!(b.command(&topic, b"ON", false, 76).is_err());
    assert!(
        b.command(
            "scent-control/example-home/TESTDEVICE02/power/set",
            b"ON",
            false,
            0
        )
        .is_err()
    );
    for invalid in [
        "4",
        "255",
        "-1",
        "1.5",
        "33",
        "100",
        "",
        "2\n",
        "{\"action\":\"RESET\"}",
    ] {
        assert!(
            b.command(&format!("{PREFIX}/level/set"), invalid.as_bytes(), false, 0)
                .is_err()
        );
    }
}

#[test]
fn retained_reports_command_echoes_and_other_routes_cannot_make_devices_available() {
    let mut b = bridge();
    let (_, body) = fixture("air2-mqtt-update-status-object.json");
    b.report(REPORT, &body, true, 10);
    b.report("scentiment-mqtt-prod/device/TESTDEVICE01", &body, false, 10);
    b.report("scentiment-mqtt-prod/user/another-home", &body, false, 10);
    b.report(REPORT, b"invalid", false, 10);
    b.report(REPORT, br#"{"action":"acknowledgement","payload":{"deviceId":"TESTDEVICE02","msg":"{\"result\":\"ADDED\"}"}}"#, false, 10);
    assert_eq!(state(&b, "availability", 10), "offline");
}

#[test]
fn reconnect_clears_state_and_new_sessions_never_replay_commands() {
    let mut b = bridge();
    report(&mut b, "air2-mqtt-update-status-object.json", 10);
    b.command(&format!("{PREFIX}/level/set"), b"3", false, 10)
        .unwrap();
    report(&mut b, "air2-mqtt-device-connected-user.json", 11);
    assert_eq!(state(&b, "power/state", 11), "None");
    assert_eq!(state(&b, "level/state", 11), "None");
    let mut fresh = Bridge::new(b.config);
    assert_eq!(state(&fresh, "availability", 0), "offline");
    let pings = fresh.pings(0);
    assert_eq!(pings.len(), 1);
    assert_eq!(
        pings[0].payload,
        br#"{"action":"PING","payload":{"userid":"route-test-1"}}"#
    );
    assert!(!pings[0].retain);
    assert!(fresh.pings(29).is_empty());
    assert_eq!(fresh.pings(30).len(), 1);
}

#[test]
fn hvac_has_its_own_discovery_and_reports_are_isolated() {
    let mut config: Value = serde_json::from_str(EXAMPLE).unwrap();
    config["devices"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"TESTDEVICE02", "name":"Example HVAC", "model":"hvac2"}));
    let mut b = Bridge::new(Config::parse(config.to_string().as_bytes()).unwrap());
    assert_eq!(b.discovery().len(), 2);
    let hvac: Value = serde_json::from_slice(&b.discovery()[1].payload).unwrap();
    assert_eq!(hvac["device"]["model"], "Diffuser HVAC 2");
    report(&mut b, "hvac-mqtt-device-connected-user.json", 10);
    assert_eq!(state(&b, "availability", 10), "offline");
    assert!(
        b.command(
            "scent-control/example-home/TESTDEVICE02/power/set",
            b"ON",
            false,
            10
        )
        .is_ok()
    );
}

#[test]
fn configuration_rejects_secrets_wildcards_duplicates_and_insecure_upstream() {
    for (path, value) in [
        ("/diffuser_broker/password", json!("sensitive-value")),
        ("/diffuser_broker/host", json!("user:sensitive-value@host")),
        (
            "/diffuser_broker/path",
            json!("/mqtt?password=sensitive-value"),
        ),
        ("/instance_id", json!("home/#")),
        ("/route_id", json!("other/+/home")),
        ("/devices/0/id", json!("TESTDEVICE0+")),
        ("/devices/0/model", json!("pro2")),
        ("/diffuser_broker/transport", json!("tcp")),
    ] {
        let mut config: Value = serde_json::from_str(EXAMPLE).unwrap();
        if path.ends_with("/password") {
            config["diffuser_broker"]["password"] = value;
        } else {
            *config.pointer_mut(path).unwrap() = value;
        }
        let error = Config::parse(config.to_string().as_bytes()).err().unwrap();
        assert!(!error.contains("sensitive-value"));
    }
    let mut config: Value = serde_json::from_str(EXAMPLE).unwrap();
    let device = config["devices"][0].clone();
    config["devices"].as_array_mut().unwrap().push(device);
    assert!(Config::parse(config.to_string().as_bytes()).is_err());
}
