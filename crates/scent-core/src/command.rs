use std::fmt;

use serde_json::{Value, json};

use crate::{Level, Profile};

/// Largest piece written to the Bluetooth characteristic in one go. Scentiment's
/// app asks for an MTU of 180, so a write carries 177 bytes after the 3-byte header.
pub const BLE_CHUNK: usize = 177;

const BLE_TERMINATOR: &str = "-|";
const MASK: &str = "***";

/// Splits a Bluetooth frame into writes of at most [`BLE_CHUNK`] bytes.
pub fn chunks(frame: &[u8]) -> std::slice::Chunks<'_, u8> {
    frame.chunks(BLE_CHUNK)
}

fn shown(secret: &Secret, reveal: bool) -> &str {
    if reveal { secret.expose() } else { MASK }
}

/// How a command reaches the diffuser. Two commands differ between the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Via {
    Bluetooth,
    Wifi,
}

/// A value that must never reach a log or a debug print. `Debug` and `Display`
/// show `***`; only [`Secret::expose`] gives the text back.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Secret {
        Secret(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(MASK)
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(MASK)
    }
}

/// Everything the app may send to a diffuser, and nothing else. There is no
/// variant for firmware updates, hard or factory resets, light commands or
/// free-form JSON, so none of those can be expressed, let alone sent.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    TurnOn {
        on: bool,
    },
    SetLevel(Level),
    Ping {
        userid: Option<String>,
    },
    GetStatus,
    SyncTime {
        server_time: u64,
    },
    SaveMqtt {
        broker: String,
        username: String,
        password: Secret,
    },
    WifiConnect {
        ssid: String,
        password: Secret,
    },
    SoftRestart,
    /// Sets the schedule table, or only the always-on entry. Follow it with
    /// `SyncTime`, as Scentiment's app does; the other order did not run on the Air 2.
    UpdateProfile(Profile),
}

impl Command {
    /// The `action` field of the JSON envelope.
    pub fn action(&self, via: Via) -> &'static str {
        match (self, via) {
            (Command::TurnOn { .. }, _) => "TURN_ON",
            (Command::SetLevel(_), _) => "SET_LEVEL",
            (Command::Ping { .. }, _) => "PING",
            (Command::GetStatus, _) => "GET_STATUS",
            (Command::SyncTime { .. }, _) => "SYNC_TIME",
            (Command::SaveMqtt { .. }, _) => "SAVE_MQTT",
            (Command::WifiConnect { .. }, _) => "WIFI_CONNECT",
            (Command::SoftRestart, Via::Bluetooth) => "SECRET_COMMAND",
            (Command::SoftRestart, Via::Wifi) => "RESET",
            (Command::UpdateProfile(_), _) => "UPDATE_PROFILE",
        }
    }

    /// The `payload` field of the JSON envelope, secrets included, exactly as the
    /// device needs it. `None` when the command has no payload.
    pub fn payload(&self, via: Via) -> Option<Value> {
        self.build_payload(via, true)
    }

    /// The full Bluetooth frame: compact JSON followed by `-|`. Split it with [`chunks`] to write it.
    pub fn ble_frame(&self) -> Vec<u8> {
        let mut frame = self.envelope(Via::Bluetooth, true).to_string().into_bytes();
        frame.extend_from_slice(BLE_TERMINATOR.as_bytes());
        frame
    }

    /// The MQTT message body: compact JSON, no terminator.
    pub fn mqtt_body(&self) -> Vec<u8> {
        self.envelope(Via::Wifi, true).to_string().into_bytes()
    }

    /// What goes in a log: the frame for `via` with every password shown as `***`.
    /// Bluetooth keeps its `-|` terminator, as the Python tools' `redact_frame` does.
    pub fn redacted(&self, via: Via) -> String {
        let mut text = self.envelope(via, false).to_string();
        if via == Via::Bluetooth {
            text.push_str(BLE_TERMINATOR);
        }
        text
    }

    fn envelope(&self, via: Via, reveal_secrets: bool) -> Value {
        let mut envelope = json!({ "action": self.action(via) });
        if let Some(payload) = self.build_payload(via, reveal_secrets) {
            envelope["payload"] = payload;
        }
        envelope
    }

    /// Keys are written in the order the Python tools use, so frames match byte for byte.
    fn build_payload(&self, via: Via, reveal_secrets: bool) -> Option<Value> {
        Some(match self {
            Command::TurnOn { on } => json!({ "on": u8::from(*on) }),
            Command::SetLevel(level) => json!({ "intensity": level.get() }),
            Command::Ping { userid: None } => return None,
            Command::Ping {
                userid: Some(userid),
            } => json!({ "userid": userid }),
            Command::GetStatus => json!({}),
            Command::SyncTime { server_time } => json!({ "serverTime": server_time }),
            Command::SaveMqtt {
                broker,
                username,
                password,
            } => {
                json!({ "broker": broker, "username": username, "password": shown(password, reveal_secrets) })
            }
            Command::WifiConnect { ssid, password } => {
                json!({ "ssid": ssid, "password": shown(password, reveal_secrets) })
            }
            Command::SoftRestart => match via {
                Via::Bluetooth => json!({ "command": "restart" }),
                Via::Wifi => json!({ "mode": "soft" }),
            },
            Command::UpdateProfile(profile) => profile.to_payload(),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        AlwaysOn, Command, Entry, HhMm, Level, Profile, ScheduleFlag, Secret, Via, chunks,
    };

    #[test]
    fn ble_frames_are_compact_json_with_terminator() {
        assert_eq!(
            Command::SetLevel(Level::new(2).unwrap()).ble_frame(),
            br#"{"action":"SET_LEVEL","payload":{"intensity":2}}-|"#.to_vec()
        );
        assert_eq!(
            Command::Ping { userid: None }.ble_frame(),
            br#"{"action":"PING"}-|"#.to_vec()
        );
        assert_eq!(
            Command::TurnOn { on: false }.mqtt_body(),
            br#"{"action":"TURN_ON","payload":{"on":0}}"#.to_vec()
        );
    }

    #[test]
    fn soft_restart_differs_per_transport() {
        assert_eq!(
            Command::SoftRestart.ble_frame(),
            br#"{"action":"SECRET_COMMAND","payload":{"command":"restart"}}-|"#.to_vec()
        );
        assert_eq!(
            Command::SoftRestart.mqtt_body(),
            br#"{"action":"RESET","payload":{"mode":"soft"}}"#.to_vec()
        );
    }

    #[test]
    fn secrets_never_appear_in_redacted_or_debug_output() {
        let c = Command::WifiConnect {
            ssid: "ExampleWiFi".into(),
            password: Secret::new("hunter2!".into()),
        };
        assert!(
            !c.redacted(Via::Bluetooth).contains("hunter2!")
                && c.redacted(Via::Bluetooth).contains(r#""password":"***""#)
        );
        assert!(!format!("{c:?}").contains("hunter2!"));
        assert!(
            String::from_utf8(c.ble_frame())
                .unwrap()
                .contains("hunter2!")
        ); // the device does need it
    }

    #[test]
    fn level_is_one_to_three() {
        assert!(Level::new(0).is_none() && Level::new(4).is_none() && Level::new(1).is_some());
    }

    #[test]
    fn long_frames_split_into_177_byte_chunks_that_rejoin() {
        let c = Command::SaveMqtt {
            broker: format!("wss://{}/mqtt", "a".repeat(300)),
            username: "u".into(),
            password: Secret::new("p".into()),
        };
        let frame = c.ble_frame();
        let parts: Vec<&[u8]> = chunks(&frame).collect();
        assert!(
            parts.iter().all(|p| p.len() <= 177)
                && parts.concat() == frame
                && parts.len() == frame.len().div_ceil(177)
        );
    }

    // Expected bytes below were produced by tools/scentiment_ble.py (encode_command and redact_frame).
    #[test]
    fn frames_match_the_python_tools_byte_for_byte() {
        let wifi = Command::WifiConnect {
            ssid: "ExampleWiFi".into(),
            password: Secret::new("hunter2!".into()),
        };
        assert_eq!(wifi.ble_frame(), br#"{"action":"WIFI_CONNECT","payload":{"ssid":"ExampleWiFi","password":"hunter2!"}}-|"#.to_vec());
        let mqtt = Command::SaveMqtt {
            broker: "wss://b.example.test/mqtt".into(),
            username: "dev1".into(),
            password: Secret::new("s3cret".into()),
        };
        assert_eq!(mqtt.ble_frame(), br#"{"action":"SAVE_MQTT","payload":{"broker":"wss://b.example.test/mqtt","username":"dev1","password":"s3cret"}}-|"#.to_vec());
        assert_eq!(
            Command::TurnOn { on: true }.ble_frame(),
            br#"{"action":"TURN_ON","payload":{"on":1}}-|"#.to_vec()
        );
        assert_eq!(
            Command::Ping {
                userid: Some("route-test-1".into())
            }
            .ble_frame(),
            br#"{"action":"PING","payload":{"userid":"route-test-1"}}-|"#.to_vec()
        );
        assert_eq!(
            Command::GetStatus.ble_frame(),
            br#"{"action":"GET_STATUS","payload":{}}-|"#.to_vec()
        );
        assert_eq!(
            Command::SyncTime {
                server_time: 1_790_000_000
            }
            .ble_frame(),
            br#"{"action":"SYNC_TIME","payload":{"serverTime":1790000000}}-|"#.to_vec()
        );
        let quoted = Command::WifiConnect {
            ssid: "Caf\u{e9} \"5G\"".into(),
            password: Secret::new("p\\w".into()),
        };
        assert_eq!(quoted.ble_frame(), "{\"action\":\"WIFI_CONNECT\",\"payload\":{\"ssid\":\"Caf\u{e9} \\\"5G\\\"\",\"password\":\"p\\\\w\"}}-|".as_bytes());
    }

    #[test]
    fn redacted_text_matches_the_python_redact_frame() {
        let mqtt = Command::SaveMqtt {
            broker: "wss://b.example.test/mqtt".into(),
            username: "dev1".into(),
            password: Secret::new("s3cret".into()),
        };
        let shown = r#"{"action":"SAVE_MQTT","payload":{"broker":"wss://b.example.test/mqtt","username":"dev1","password":"***"}}"#;
        assert_eq!(mqtt.redacted(Via::Bluetooth), format!("{shown}-|"));
        assert_eq!(mqtt.redacted(Via::Wifi), shown);
        assert_eq!(
            Command::Ping { userid: None }.redacted(Via::Wifi),
            r#"{"action":"PING"}"#
        );
    }

    // Expected bytes below were produced by tools/scentiment_ble.py (build_profile and encode_command).
    #[test]
    fn update_profile_frames_match_the_python_tools_on_both_transports() {
        let timed = Profile {
            always_on: AlwaysOn {
                enable: false,
                level: Level::new(1).unwrap(),
            },
            entries: vec![Entry {
                start: HhMm::parse("07:00").unwrap(),
                end: HhMm::parse("09:00").unwrap(),
                weekdays: vec![1, 2, 3, 4, 5],
                level: Level::new(3).unwrap(),
                enable: true,
            }],
            timezone_hours: -5.0,
            flag: ScheduleFlag::Replace,
        };
        let body = r#"{"action":"UPDATE_PROFILE","payload":{"profile":{"schedule":[{"index":1,"startTime":"00:00","endTime":"23:59","weekdays":[0,1,2,3,4,5,6],"workingState":{"level":1,"fan":0,"led":0,"timer":0},"enable":false,"isFactory":true},{"startTime":"07:00","endTime":"09:00","weekdays":[1,2,3,4,5],"workingState":{"level":3,"fan":0,"led":0,"timer":0},"enable":true}]},"timezone":-5.0,"schedule_flag":1}}"#;
        let c = Command::UpdateProfile(timed);
        assert_eq!(c.ble_frame(), format!("{body}-|").into_bytes());
        assert_eq!(c.mqtt_body(), body.as_bytes());

        let always_on = Command::UpdateProfile(Profile {
            always_on: AlwaysOn {
                enable: true,
                level: Level::new(1).unwrap(),
            },
            entries: vec![],
            timezone_hours: 5.5,
            flag: ScheduleFlag::AlwaysOnOnly,
        });
        assert_eq!(
            always_on.ble_frame(),
            br#"{"action":"UPDATE_PROFILE","payload":{"profile":{"schedule":[{"index":1,"startTime":"00:00","endTime":"23:59","weekdays":[0,1,2,3,4,5,6],"workingState":{"level":1,"fan":0,"led":0,"timer":0},"enable":true,"isFactory":true}]},"timezone":5.5,"schedule_flag":0}}-|"#.to_vec()
        );
    }

    #[test]
    fn update_profile_has_nothing_to_redact() {
        let c = Command::UpdateProfile(Profile {
            always_on: AlwaysOn {
                enable: false,
                level: Level::new(2).unwrap(),
            },
            entries: vec![],
            timezone_hours: 0.0,
            flag: ScheduleFlag::Replace,
        });
        for via in [Via::Bluetooth, Via::Wifi] {
            assert_eq!(c.action(via), "UPDATE_PROFILE");
            let frame = String::from_utf8(match via {
                Via::Bluetooth => c.ble_frame(),
                Via::Wifi => c.mqtt_body(),
            })
            .unwrap();
            assert_eq!(c.redacted(via), frame);
        }
    }

    #[test]
    fn secret_prints_stars_and_exposes_only_on_request() {
        let s = Secret::new("hunter2!".into());
        assert_eq!(
            (format!("{s}"), format!("{s:?}")),
            ("***".to_string(), "***".to_string())
        );
        assert_eq!(s.expose(), "hunter2!");
    }

    #[test]
    fn level_round_trips_and_chunks_cover_the_boundary() {
        assert_eq!(
            (1..=3)
                .map(|n| Level::new(n).unwrap().get())
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(chunks(&[0u8; 177]).count(), 1);
        assert_eq!(
            chunks(&[0u8; 178]).map(<[u8]>::len).collect::<Vec<_>>(),
            vec![177, 1]
        );
        assert_eq!(chunks(&[]).count(), 0);
    }
}
