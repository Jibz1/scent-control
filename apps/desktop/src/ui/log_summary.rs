//! The log's rows in words: each entry as a short sentence, such as "Turned Bedroom on,
//! over WiFi" or "Bedroom reported on, Medium, plugged in". The (masked) message itself
//! shows only when a row is opened.
//!
//! Reads only what the entry holds, and only its masked `text`: a sentence never quotes
//! a payload that can carry a secret (`SAVE_MQTT`, `WIFI_CONNECT`).

use scent_core::{Direction, Status, Via, parse_info, parse_mqtt, parse_notification};
use serde_json::Value;

use super::format::{Model, intensity, power_source, via_name};
use crate::hub::SCAN_FAILED;
use crate::state::{LogEntry, OTHER_VERSION_SCHEDULE, SHARED_SCHEDULE, UNREADABLE_SCHEDULE};

/// What `power_source` says when it has nothing to go on.
const NOTHING: &str = "not reported yet";

/// The entry as a sentence about the diffuser called `label` (or "Broker", "App"): what
/// was sent, what was heard, or what failed and why. No full stop at the end, except
/// where an error's own sentence ends one.
pub fn sentence(entry: &LogEntry, label: &str) -> String {
    match (entry.direction, entry.via) {
        (Direction::ToDevice, None) => failure(entry, label),
        (Direction::ToDevice, Some(via)) if entry.sent => sent(entry, label, via),
        (Direction::ToDevice, Some(via)) => told(entry, label, via),
        (Direction::FromDevice, via) => heard(entry, label, via),
    }
}

/// What an opened row says above the message: the action's name and what happened to
/// it, for whoever reads the message itself.
pub fn detail(entry: &LogEntry) -> String {
    match (entry.direction, entry.via) {
        (Direction::ToDevice, None) => format!("{}, failed", entry.action),
        (Direction::ToDevice, Some(via)) if entry.sent => {
            format!("{}, sent over {}", entry.action, via_name(via))
        }
        (_, Some(via)) => format!("{}, heard over {}", entry.action, via_name(via)),
        (_, None) => entry.action.clone(),
    }
}

/// The JSON of a command, as logged: a Bluetooth frame ends in `-|`.
fn command_json(text: &str) -> Option<Value> {
    let text = text.trim();
    let frame = text.strip_suffix("-|").unwrap_or(text);
    serde_json::from_str(frame).ok()
}

/// A number from a command's `payload`, or from its `data` (the shape Scentiment's own
/// app used), under the first of `keys` that is there.
fn number(json: &Value, keys: &[&str]) -> Option<i64> {
    ["payload", "data"].into_iter().find_map(|part| {
        let part = json.get(part)?;
        keys.iter().find_map(|key| part.get(key)?.as_i64())
    })
}

/// The command in words, as the person did it: "Turned Bedroom on". `None` when the
/// action is one this app doesn't know, or its payload can't be read.
fn command_done(action: &str, json: Option<&Value>, label: &str) -> Option<String> {
    Some(match action {
        "TURN_ON" => match number(json?, &["on", "enable"])? {
            0 => format!("Turned {label} off"),
            _ => format!("Turned {label} on"),
        },
        "SET_LEVEL" => {
            let level = number(json?, &["intensity", "level"])?;
            format!("Set {label} to {} intensity", intensity(level))
        }
        "PING" => format!("Pinged {label}"),
        "GET_STATUS" => format!("Asked {label} for its status"),
        "SYNC_TIME" => format!("Told {label} the time"),
        "SAVE_MQTT" => format!("Pointed {label} at your broker"),
        "WIFI_CONNECT" => format!("Gave {label} a WiFi network to join"),
        "SECRET_COMMAND" | "RESET" => format!("Told {label} to restart"),
        "UPDATE_PROFILE" => schedule_sent(json, label),
        _ => return None,
    })
}

/// A schedule sent: how many times it holds, or that only the always-on entry went.
fn schedule_sent(json: Option<&Value>, label: &str) -> String {
    let payload = json.and_then(|json| json.get("payload"));
    let flag = payload.and_then(|payload| payload.get("schedule_flag")?.as_i64());
    let entries = payload
        .and_then(|payload| payload.pointer("/profile/schedule")?.as_array())
        .map(Vec::len);
    match (flag, entries) {
        (Some(0), _) => format!("Sent {label} its always-on setting"),
        // The factory entry comes first; the rest are the times.
        (_, Some(entries)) => match entries.saturating_sub(1) {
            0 => format!("Sent {label} a schedule with no times"),
            1 => format!("Sent {label} a schedule of 1 time"),
            times => format!("Sent {label} a schedule of {times} times"),
        },
        _ => format!("Sent {label} a schedule"),
    }
}

/// A command this app sent.
fn sent(entry: &LogEntry, label: &str, via: Via) -> String {
    let json = command_json(&entry.text);
    let done = command_done(&entry.action, json.as_ref(), label)
        .unwrap_or_else(|| format!("Sent {} to {label}", entry.action));
    format!("{done}, over {}", via_name(via))
}

/// A command on the diffuser's topic that this app didn't send: what it was told.
fn told(entry: &LogEntry, label: &str, via: Via) -> String {
    let json = command_json(&entry.text);
    let json = json.as_ref();
    let what = match entry.action.as_str() {
        "TURN_ON" => json
            .and_then(|json| number(json, &["on", "enable"]))
            .map(|on| if on == 0 { "turn off" } else { "turn on" }.to_owned()),
        "SET_LEVEL" => json
            .and_then(|json| number(json, &["intensity", "level"]))
            .map(|level| format!("set its intensity to {}", intensity(level))),
        "PING" => Some("answer a ping".to_owned()),
        "GET_STATUS" => Some("report its status".to_owned()),
        "SYNC_TIME" => Some("take the time".to_owned()),
        "SAVE_MQTT" => Some("use a new broker".to_owned()),
        "WIFI_CONNECT" => Some("join a WiFi network".to_owned()),
        "SECRET_COMMAND" | "RESET" => Some("restart".to_owned()),
        "UPDATE_PROFILE" => Some("take a new schedule".to_owned()),
        _ => None,
    };
    let over = via_name(via);
    match what {
        Some(what) => format!("{label} was told to {what}, over {over}"),
        None => format!("{label} was sent {}, over {over}", entry.action),
    }
}

/// A status in words: on or off, the intensity while on, and the power source, as far
/// as it says them. `None` when it says none of them.
fn status_words(status: &Status) -> Option<String> {
    let mut parts = Vec::new();
    match status.power {
        Some(0) => parts.push("off".to_owned()),
        Some(_) => parts.push("on".to_owned()),
        None => {}
    }
    if status.power != Some(0)
        && let Some(level @ 1..=3) = status.intensity
    {
        parts.push(intensity(level));
    }
    let source = power_source(status.charging, status.battery);
    if source != NOTHING {
        parts.push(source);
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// A status the diffuser reported, or that it sent something this app doesn't read.
fn reported(status: Option<Status>, label: &str) -> String {
    match status.as_ref().and_then(status_words) {
        Some(words) => format!("{label} reported {words}"),
        None => format!("{label} sent a value this app doesn't use"),
    }
}

/// Whether a diffuser's message says it took `SAVE_MQTT`.
fn says_credentials_saved(text: &str) -> bool {
    text.to_ascii_lowercase().contains("credentials saved")
}

/// Something the diffuser, or the broker, said.
fn heard(entry: &LogEntry, label: &str, via: Option<Via>) -> String {
    let text = entry.text.trim();
    match entry.action.as_str() {
        "broker" if entry.device_id.is_none() => {
            return text.strip_suffix('.').unwrap_or(text).to_owned();
        }
        "Add" => return format!("{label} joined your broker"),
        "info" => return identified(text, label),
        "acknowledgement" => return acknowledged(text, label),
        "device_connected" => return format!("{label} connected to the broker"),
        "initial_load" | "update_profile" => return format!("{label} sent its schedule"),
        SHARED_SCHEDULE => return format!("Loaded {label}'s schedule from the broker"),
        UNREADABLE_SCHEDULE | OTHER_VERSION_SCHEDULE => return ignored_schedule(entry, label),
        _ => {}
    }
    if says_credentials_saved(text) {
        return format!("{label} saved the broker login");
    }
    match (via, entry.action.as_str()) {
        (Some(Via::Bluetooth), "status" | "value" | "json" | "unknown") => {
            reported(Some(parse_notification(text.as_bytes()).status), label)
        }
        (Some(Via::Wifi), "update_status" | "notification") => {
            reported(parse_mqtt("", text.as_bytes()).status, label)
        }
        _ => format!("{label} sent {}", entry.action),
    }
}

/// A message under our own topics that the app ignored, and why, as the action kept it
/// when it was logged: the message itself is not read again.
fn ignored_schedule(entry: &LogEntry, label: &str) -> String {
    if entry.device_id.is_none() {
        return "Ignored a message under Scent Control's topics on the broker that it can't use"
            .to_owned();
    }
    let why = if entry.action == OTHER_VERSION_SCHEDULE {
        "another version of Scent Control stored it"
    } else {
        "it can't be read"
    };
    format!("Ignored {label}'s schedule on the broker: {why}")
}

/// The info string read over Bluetooth: the model and firmware it gives.
fn identified(text: &str, label: &str) -> String {
    let Ok(info) = parse_info(text) else {
        return format!("{label} sent its details");
    };
    let model = Model::from_info(&info.model_name)
        .name()
        .map_or_else(|| info.model_name.clone(), str::to_owned);
    let parts: Vec<String> = [
        Some(model).filter(|model| !model.is_empty()),
        Some(info.firmware)
            .filter(|firmware| !firmware.is_empty())
            .map(|firmware| format!("firmware {firmware}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        format!("{label} sent its details")
    } else {
        format!("{label} identified itself: {}", parts.join(", "))
    }
}

/// An acknowledgement: its `msg` says what it is about.
fn acknowledged(text: &str, label: &str) -> String {
    let msg = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|json| json.pointer("/payload/msg")?.as_str().map(str::to_owned))
        .unwrap_or_default();
    if says_credentials_saved(&msg) {
        return format!("{label} saved the broker login");
    }
    match msg.trim() {
        "BATTERY_MODE" => return format!("{label} switched to its battery"),
        "PLUGIN_MODE" => return format!("{label} was plugged in"),
        _ => {}
    }
    let inner = serde_json::from_str::<Value>(&msg).ok();
    match inner
        .as_ref()
        .and_then(|inner| inner.get("result")?.as_str())
    {
        Some("ADDED") => return format!("{label} confirmed it was added"),
        Some("ALREADY_ADDED") => return format!("{label} answered a ping"),
        _ => {}
    }
    if inner
        .as_ref()
        .is_some_and(|inner| inner.get("data").is_some())
    {
        return reported(Some(parse_notification(msg.as_bytes()).status), label);
    }
    format!("{label} sent an acknowledgement")
}

/// A failed job's name without the route it names, and " over Bluetooth" (or nothing)
/// for that route: "Bluetooth TURN_ON" is TURN_ON over Bluetooth.
fn job_and_route(what: &str) -> (&str, String) {
    let (via, name) = [(Via::Bluetooth, "Bluetooth "), (Via::Wifi, "WiFi ")]
        .into_iter()
        .find_map(|(via, prefix)| Some((Some(via), what.strip_prefix(prefix)?)))
        .unwrap_or((None, what));
    let over = via.map_or_else(String::new, |via| format!(" over {}", via_name(via)));
    (name, over)
}

/// What failed for the open diffuser, for its hero to say in red until the next send or
/// report: "Couldn't send the power command over Bluetooth: No answer." The diffuser
/// isn't named, as the hero is about it. `None` for the routine ping, which the person
/// didn't ask for.
pub fn failure_note(what: &str, error: &str) -> Option<String> {
    let (name, over) = job_and_route(what);
    let error = error.trim();
    let attempt = match name {
        "PING" => return None,
        "TURN_ON" => "send the power command",
        "SET_LEVEL" => "set the intensity",
        "GET_STATUS" | "read status" => "read its status",
        "SYNC_TIME" => "tell it the time",
        "SAVE_MQTT" => "point it at your broker",
        "WIFI_CONNECT" => "give it the WiFi network",
        "SECRET_COMMAND" | "RESET" => "restart it",
        "UPDATE_PROFILE" => "send its schedule",
        "Schedule" => "finish sending its schedule",
        "Add" => "add it",
        _ => return Some(error.to_owned()),
    };
    Some(format!("Couldn't {attempt}{over}: {error}"))
}

/// What a failure was trying to do, as the end of "Couldn't …".
fn attempt(name: &str, label: &str) -> Option<String> {
    Some(match name {
        "TURN_ON" => format!("send the power command to {label}"),
        "SET_LEVEL" => format!("set {label}'s intensity"),
        "PING" => format!("ping {label}"),
        "GET_STATUS" | "read status" => format!("read {label}'s status"),
        "SYNC_TIME" => format!("tell {label} the time"),
        "SAVE_MQTT" => format!("point {label} at your broker"),
        "WIFI_CONNECT" => format!("give {label} a WiFi network"),
        "SECRET_COMMAND" | "RESET" => format!("restart {label}"),
        "UPDATE_PROFILE" => format!("send {label} its schedule"),
        "Schedule" => format!("finish sending {label}'s schedule"),
        "Add" => format!("add {label}"),
        _ => return None,
    })
}

/// Something that failed: what it was trying to do, over what, and the error, which says
/// what happened and what to do about it.
fn failure(entry: &LogEntry, label: &str) -> String {
    let error = entry.text.trim();
    if entry.action == SCAN_FAILED {
        return format!("Couldn't scan for diffusers: {error}");
    }
    let (name, over) = job_and_route(&entry.action);
    // With no diffuser (an add before its info was read, or one turned down), the
    // label is the app's own: say "the diffuser" instead.
    let label = if entry.device_id.is_some() {
        label
    } else {
        "the diffuser"
    };
    match (attempt(name, label), &entry.device_id) {
        (Some(attempt), _) => format!("Couldn't {attempt}{over}: {error}"),
        (None, Some(_)) => format!("{label}: {error}"),
        (None, None) => error.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use scent_core::{Command, Direction, Level, Secret, Via};

    use super::*;
    use crate::state::{LogEntry, mask_passwords};

    const ID: &str = "TESTDEVICE01";

    fn entry(
        direction: Direction,
        via: Option<Via>,
        sent: bool,
        action: &str,
        text: &str,
    ) -> LogEntry {
        LogEntry {
            at: 0,
            device_id: Some(ID.to_owned()),
            via,
            direction,
            action: action.to_owned(),
            text: mask_passwords(text),
            sent,
        }
    }

    /// What the app logs when it sends `cmd` over `via`.
    fn sent(cmd: &Command, via: Via) -> LogEntry {
        entry(
            Direction::ToDevice,
            Some(via),
            true,
            cmd.action(via),
            &cmd.redacted(via),
        )
    }

    /// What the app logs when it hears `text` from the diffuser over WiFi.
    fn heard(action: &str, text: &str) -> LogEntry {
        entry(Direction::FromDevice, Some(Via::Wifi), false, action, text)
    }

    fn failed(what: &str, error: &str) -> LogEntry {
        entry(Direction::ToDevice, None, false, what, error)
    }

    fn level(n: u8) -> Level {
        Level::new(n).unwrap()
    }

    #[test]
    fn log_sentences() {
        let save = Command::SaveMqtt {
            broker: "wss://broker.example/mqtt".to_owned(),
            username: "example-user".to_owned(),
            password: Secret::new("hunter2".to_owned()),
        };
        let join = Command::WifiConnect {
            ssid: "ExampleWiFi".to_owned(),
            password: Secret::new("hunter2".to_owned()),
        };
        let table: Vec<(LogEntry, &str)> = vec![
            (
                sent(&Command::TurnOn { on: true }, Via::Wifi),
                "Turned Bedroom on, over WiFi",
            ),
            (
                sent(&Command::TurnOn { on: false }, Via::Bluetooth),
                "Turned Bedroom off, over Bluetooth",
            ),
            (
                sent(&Command::SetLevel(level(1)), Via::Wifi),
                "Set Bedroom to Low intensity, over WiFi",
            ),
            (
                sent(&Command::SetLevel(level(2)), Via::Wifi),
                "Set Bedroom to Medium intensity, over WiFi",
            ),
            (
                sent(&Command::SetLevel(level(3)), Via::Bluetooth),
                "Set Bedroom to High intensity, over Bluetooth",
            ),
            // The shape the firmware's own app uses, "enable" and "level", reads the same.
            (
                entry(
                    Direction::ToDevice,
                    Some(Via::Wifi),
                    true,
                    "TURN_ON",
                    r#"{"action":"TURN_ON","data":{"enable":1}}"#,
                ),
                "Turned Bedroom on, over WiFi",
            ),
            (
                sent(&Command::GetStatus, Via::Wifi),
                "Asked Bedroom for its status, over WiFi",
            ),
            (
                sent(&Command::SyncTime { server_time: 1 }, Via::Wifi),
                "Told Bedroom the time, over WiFi",
            ),
            (
                sent(
                    &Command::Ping {
                        userid: Some("route-test-1".to_owned()),
                    },
                    Via::Wifi,
                ),
                "Pinged Bedroom, over WiFi",
            ),
            (
                sent(&save, Via::Bluetooth),
                "Pointed Bedroom at your broker, over Bluetooth",
            ),
            (
                sent(&join, Via::Bluetooth),
                "Gave Bedroom a WiFi network to join, over Bluetooth",
            ),
            (
                sent(&Command::SoftRestart, Via::Bluetooth),
                "Told Bedroom to restart, over Bluetooth",
            ),
            (
                sent(&Command::SoftRestart, Via::Wifi),
                "Told Bedroom to restart, over WiFi",
            ),
            (
                entry(
                    Direction::ToDevice,
                    Some(Via::Wifi),
                    true,
                    "UPDATE_PROFILE",
                    r#"{"action":"UPDATE_PROFILE","payload":{"profile":{"schedule":[{"isFactory":true},{"startTime":"07:00"},{"startTime":"18:00"}]},"timezone":-5.0,"schedule_flag":1}}"#,
                ),
                "Sent Bedroom a schedule of 2 times, over WiFi",
            ),
            // A command on the diffuser's topic that this app didn't send.
            (
                entry(
                    Direction::ToDevice,
                    Some(Via::Wifi),
                    false,
                    "TURN_ON",
                    r#"{"action":"TURN_ON","payload":{"on":0}}"#,
                ),
                "Bedroom was told to turn off, over WiFi",
            ),
            // What the diffuser says.
            (
                heard(
                    "update_status",
                    r#"{"action":"update_status","payload":{"deviceId":"TESTDEVICE01","data":{"data":[85,170,11,255,1,2,200,50,255,2,3,255,255,0,92,106]},"rssi":-66}}"#,
                ),
                "Bedroom reported on, Medium, plugged in",
            ),
            (
                heard(
                    "notification",
                    r#"{"action":"notification","payload":{"deviceId":"TESTDEVICE01","data":enable:0,"rssi":-46}}"#,
                ),
                "Bedroom reported off",
            ),
            (
                entry(
                    Direction::FromDevice,
                    Some(Via::Bluetooth),
                    false,
                    "status",
                    "S:1,L:3,C:0,Bat:80",
                ),
                "Bedroom reported on, High, battery 80%",
            ),
            (
                entry(
                    Direction::FromDevice,
                    Some(Via::Bluetooth),
                    false,
                    "value",
                    "led:50",
                ),
                "Bedroom sent a value this app doesn't use",
            ),
            (
                heard(
                    "acknowledgement",
                    r#"{"action":"acknowledgement","payload":{"deviceId":"TESTDEVICE01","msg":"{\"user_id\":\"route-test-1\",\"result\":\"ADDED\"}"}}"#,
                ),
                "Bedroom confirmed it was added",
            ),
            (
                heard(
                    "acknowledgement",
                    r#"{"action":"acknowledgement","payload":{"deviceId":"TESTDEVICE01","msg":"Credentials saved"}}"#,
                ),
                "Bedroom saved the broker login",
            ),
            (
                heard(
                    "acknowledgement",
                    r#"{"action":"acknowledgement","payload":{"deviceId":"TESTDEVICE01","msg":"BATTERY_MODE"}}"#,
                ),
                "Bedroom switched to its battery",
            ),
            (
                heard(
                    "acknowledgement",
                    r#"{"action":"acknowledgement","payload":{"deviceId":"TESTDEVICE01","msg":"PLUGIN_MODE"}}"#,
                ),
                "Bedroom was plugged in",
            ),
            (
                heard(
                    "device_connected",
                    r#"{"action":"device_connected","payload":{"deviceId":"TESTDEVICE01","fw":"2.30","model":"Diffuser","variant":"Black","manufacturer":"01"}}"#,
                ),
                "Bedroom connected to the broker",
            ),
            (
                heard("Add", "Added: it joined your broker."),
                "Bedroom joined your broker",
            ),
            (
                entry(
                    Direction::FromDevice,
                    Some(Via::Bluetooth),
                    false,
                    "info",
                    "01|Diffuser|Black|TESTDEVICE01|2.30|true|3",
                ),
                "Bedroom identified itself: Air 2, firmware 2.30",
            ),
            // Failures say what failed and why.
            (
                failed("Bluetooth TURN_ON", "No answer."),
                "Couldn't send the power command to Bedroom over Bluetooth: No answer.",
            ),
            (
                failed(
                    "SET_LEVEL",
                    "Neither route worked. Bluetooth: Off. WiFi: Down.",
                ),
                "Couldn't set Bedroom's intensity: Neither route worked. Bluetooth: Off. WiFi: Down.",
            ),
            (
                failed("Bluetooth read status", "No answer."),
                "Couldn't read Bedroom's status over Bluetooth: No answer.",
            ),
            (
                failed("Add", "The diffuser didn't join your broker."),
                "Couldn't add Bedroom: The diffuser didn't join your broker.",
            ),
            (
                failed("Hub", "Something stopped."),
                "Bedroom: Something stopped.",
            ),
            // Anything else falls back to the action's name.
            (heard("OTA_SUCCESS", "{}"), "Bedroom sent OTA_SUCCESS"),
            (
                entry(
                    Direction::ToDevice,
                    Some(Via::Wifi),
                    true,
                    "SOMETHING_NEW",
                    "{}",
                ),
                "Sent SOMETHING_NEW to Bedroom, over WiFi",
            ),
        ];
        for (entry, expected) in &table {
            assert_eq!(sentence(entry, "Bedroom"), *expected, "{entry:?}");
        }
    }

    #[test]
    fn schedules_read_from_the_broker_read_as_sentences() {
        let body = crate::shared_schedule::encode(&crate::store::sent_schedule(), 100).unwrap();
        let body = String::from_utf8(body).unwrap();
        let loaded = heard(SHARED_SCHEDULE, &body);
        assert_eq!(
            sentence(&loaded, "Bedroom"),
            "Loaded Bedroom's schedule from the broker"
        );
        // Why is the action's, kept when it was logged: the text is not read again (here
        // it couldn't be).
        let later = heard(OTHER_VERSION_SCHEDULE, "{ not json");
        assert_eq!(
            sentence(&later, "Bedroom"),
            "Ignored Bedroom's schedule on the broker: another version of Scent Control stored it"
        );
        let broken = heard(UNREADABLE_SCHEDULE, "{ not json");
        assert_eq!(
            sentence(&broken, "Bedroom"),
            "Ignored Bedroom's schedule on the broker: it can't be read"
        );
        // On a topic that names no diffuser.
        let nobodys = LogEntry {
            device_id: None,
            ..heard(UNREADABLE_SCHEDULE, &body)
        };
        assert_eq!(
            sentence(&nobodys, "App"),
            "Ignored a message under Scent Control's topics on the broker that it can't use"
        );
    }

    #[test]
    fn a_secret_never_reaches_a_sentence() {
        let save = Command::SaveMqtt {
            broker: "wss://broker.example/mqtt".to_owned(),
            username: "example-user".to_owned(),
            password: Secret::new("hunter2".to_owned()),
        };
        // As logged (masked), and even as a frame that somehow wasn't.
        let masked = sent(&save, Via::Bluetooth);
        let mut bare = masked.clone();
        bare.text = String::from_utf8(save.ble_frame()).unwrap();
        for entry in [masked, bare] {
            let said = sentence(&entry, "Bedroom");
            for secret in [
                "hunter2",
                "***",
                "password",
                "example-user",
                "broker.example",
            ] {
                assert!(!said.contains(secret), "{said:?} has {secret:?}");
            }
        }
        let join = Command::WifiConnect {
            ssid: "ExampleWiFi".to_owned(),
            password: Secret::new("hunter2".to_owned()),
        };
        let said = sentence(&sent(&join, Via::Bluetooth), "Bedroom");
        assert!(
            !said.contains("hunter2") && !said.contains("ExampleWiFi"),
            "{said}"
        );
    }

    #[test]
    fn broker_notes_and_scan_failures_read_without_a_diffuser() {
        let broker = |text: &str| LogEntry {
            at: 0,
            device_id: None,
            via: Some(Via::Wifi),
            direction: Direction::FromDevice,
            action: "broker".to_owned(),
            text: text.to_owned(),
            sent: false,
        };
        assert_eq!(
            sentence(&broker("Connected to the broker."), "Broker"),
            "Connected to the broker"
        );
        assert_eq!(
            sentence(&broker("Broker login refused: Not authorized."), "Broker"),
            "Broker login refused: Not authorized"
        );
        let scan = LogEntry {
            device_id: None,
            ..failed("Bluetooth scan", "Bluetooth is off.")
        };
        assert_eq!(
            sentence(&scan, "App"),
            "Couldn't scan for diffusers: Bluetooth is off."
        );
        // An add that has no diffuser yet, or that the hub turned down.
        let add = LogEntry {
            device_id: None,
            ..failed("Add", "Another add of this diffuser is under way.")
        };
        assert_eq!(
            sentence(&add, "App"),
            "Couldn't add the diffuser: Another add of this diffuser is under way."
        );
        let hub = LogEntry {
            device_id: None,
            ..failed("Command", "The hub stopped.")
        };
        assert_eq!(sentence(&hub, "App"), "The hub stopped.");
    }

    #[test]
    fn the_hero_says_what_failed_without_naming_the_diffuser() {
        assert_eq!(
            failure_note("Bluetooth TURN_ON", "No answer."),
            Some("Couldn't send the power command over Bluetooth: No answer.".to_owned())
        );
        assert_eq!(
            failure_note("SET_LEVEL", "Neither route worked."),
            Some("Couldn't set the intensity: Neither route worked.".to_owned())
        );
        assert_eq!(
            failure_note("WiFi read status", "Down."),
            Some("Couldn't read its status over WiFi: Down.".to_owned())
        );
        assert_eq!(
            failure_note("Add", "No."),
            Some("Couldn't add it: No.".to_owned())
        );
        assert_eq!(failure_note("Hub", "Dropped."), Some("Dropped.".to_owned()));
        // The routine ping is not something the person did.
        assert_eq!(failure_note("WiFi PING", "Down."), None);
    }

    #[test]
    fn the_detail_names_the_action_and_the_route() {
        assert_eq!(
            detail(&sent(&Command::GetStatus, Via::Wifi)),
            "GET_STATUS, sent over WiFi"
        );
        assert_eq!(
            detail(&heard("update_status", "{}")),
            "update_status, heard over WiFi"
        );
        assert_eq!(
            detail(&failed("Bluetooth TURN_ON", "No answer.")),
            "Bluetooth TURN_ON, failed"
        );
    }
}
