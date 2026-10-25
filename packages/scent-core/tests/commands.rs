//! The golden commands: `fixtures/commands/commands.json` holds the exact text this
//! crate writes for a fixed set of commands. The Swift port in
//! `apps/ios/Packages/ScentProtocol` builds the same commands and is tested against the
//! same file, so the two codecs agree byte for byte.
//!
//! After a deliberate change to what a command writes, regenerate the file from this
//! crate with `SCENT_BLESS=1 cargo test -p scent-core --test commands`, then update the
//! Swift list of commands to match.

use std::env;
use std::fs;
use std::path::PathBuf;

use scent_core::{AlwaysOn, Command, Entry, HhMm, Level, Profile, ScheduleFlag, Secret, Via};
use serde_json::{Value, json};

fn level(n: u8) -> Level {
    Level::new(n).unwrap()
}

fn entry(start: &str, end: &str, weekdays: &[u8], n: u8, enable: bool) -> Entry {
    Entry {
        start: HhMm::parse(start).unwrap(),
        end: HhMm::parse(end).unwrap(),
        weekdays: weekdays.to_vec(),
        level: level(n),
        enable,
    }
}

/// An always-on-only profile at `tz` hours: the shortest `UPDATE_PROFILE`, for checking
/// how the time zone is written.
fn zone(tz: f64) -> Command {
    Command::UpdateProfile(Profile {
        always_on: AlwaysOn {
            enable: true,
            level: level(1),
        },
        entries: vec![],
        timezone_hours: tz,
        flag: ScheduleFlag::AlwaysOnOnly,
    })
}

/// The commands in the golden file, in its order. The Swift test has the same list.
fn commands() -> Vec<(&'static str, Command)> {
    let fake = || Secret::new("example-password".into());
    vec![
        ("turn_on", Command::TurnOn { on: true }),
        ("turn_off", Command::TurnOn { on: false }),
        ("set_level_1", Command::SetLevel(level(1))),
        ("set_level_2", Command::SetLevel(level(2))),
        ("set_level_3", Command::SetLevel(level(3))),
        ("ping", Command::Ping { userid: None }),
        (
            "ping_with_userid",
            Command::Ping {
                userid: Some("example-route".into()),
            },
        ),
        (
            "ping_with_empty_userid",
            Command::Ping {
                userid: Some(String::new()),
            },
        ),
        ("get_status", Command::GetStatus),
        (
            "sync_time",
            Command::SyncTime {
                server_time: 1_792_510_200,
            },
        ),
        ("sync_time_zero", Command::SyncTime { server_time: 0 }),
        (
            "sync_time_max",
            Command::SyncTime {
                server_time: u64::MAX,
            },
        ),
        (
            "save_mqtt",
            Command::SaveMqtt {
                broker: "wss://broker.example.test/mqtt".into(),
                username: "example-user".into(),
                password: fake(),
            },
        ),
        (
            "wifi_connect",
            Command::WifiConnect {
                ssid: "ExampleWiFi".into(),
                password: fake(),
            },
        ),
        (
            "wifi_connect_escapes",
            Command::WifiConnect {
                ssid:
                    "Caf\u{e9} \"5G\" / \u{1f33f}\\ \t\n\r\u{8}\u{c}\u{0}\u{1}\u{1f}\u{7f}\u{2028}"
                        .into(),
                password: Secret::new("p\\w\"\u{e4}\u{1}".into()),
            },
        ),
        ("soft_restart", Command::SoftRestart),
        (
            "update_profile_replace",
            Command::UpdateProfile(Profile {
                always_on: AlwaysOn {
                    enable: false,
                    level: level(1),
                },
                entries: vec![
                    entry("07:00", "09:00", &[1, 2, 3, 4, 5], 3, true),
                    entry("18:30", "22:00", &[0, 6], 2, true),
                    entry("12:00", "12:30", &[3], 1, false),
                ],
                timezone_hours: 5.5,
                flag: ScheduleFlag::Replace,
            }),
        ),
        (
            "update_profile_always_on_only",
            Command::UpdateProfile(Profile {
                always_on: AlwaysOn {
                    enable: true,
                    level: level(2),
                },
                entries: vec![entry("07:00", "09:00", &[1], 3, true)],
                timezone_hours: -5.0,
                flag: ScheduleFlag::AlwaysOnOnly,
            }),
        ),
        (
            "update_profile_replace_empty",
            Command::UpdateProfile(Profile {
                always_on: AlwaysOn {
                    enable: false,
                    level: level(3),
                },
                entries: vec![],
                timezone_hours: 0.0,
                flag: ScheduleFlag::Replace,
            }),
        ),
        ("update_profile_zone_zero", zone(0.0)),
        ("update_profile_zone_negative_zero", zone(-0.0)),
        ("update_profile_zone_quarter", zone(5.75)),
        ("update_profile_zone_negative_half", zone(-9.5)),
        ("update_profile_zone_fourteen", zone(14.0)),
        ("update_profile_zone_tenth", zone(0.1)),
        ("update_profile_zone_third", zone(1.0 / 3.0)),
        // Plain decimals from 1e-5 up to below 1e16, exponents outside that.
        ("update_profile_zone_1e15", zone(1e15)),
        ("update_profile_zone_1e16", zone(1e16)),
        ("update_profile_zone_1e-5", zone(1e-5)),
        ("update_profile_zone_1e-6", zone(1e-6)),
        ("update_profile_zone_small", zone(1.25e-5)),
        ("update_profile_zone_tiny", zone(-4.5e-9)),
        ("update_profile_zone_large", zone(-1.234_567_89e20)),
        ("update_profile_zone_1e23", zone(1e23)),
        ("update_profile_zone_max", zone(f64::MAX)),
        ("update_profile_zone_min_positive", zone(f64::MIN_POSITIVE)),
        ("update_profile_zone_subnormal", zone(5e-324)),
        // serde_json writes a number that isn't finite as null.
        ("update_profile_zone_nan", zone(f64::NAN)),
        ("update_profile_zone_infinity", zone(f64::INFINITY)),
    ]
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).expect("frames are UTF-8")
}

/// What the golden file should hold, as this crate writes it.
fn rendered() -> Value {
    let cases: Vec<Value> = commands()
        .into_iter()
        .map(|(name, command)| {
            json!({
                "name": name,
                "ble_frame": text(command.ble_frame()),
                "mqtt_body": text(command.mqtt_body()),
                "redacted_bluetooth": command.redacted(Via::Bluetooth),
                "redacted_wifi": command.redacted(Via::Wifi),
            })
        })
        .collect();
    json!({ "commands": cases })
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/commands/commands.json")
}

#[test]
fn commands_match_the_golden_file() {
    let path = golden_path();
    let actual = rendered();
    if env::var_os("SCENT_BLESS").is_some() {
        let pretty = serde_json::to_string_pretty(&actual).unwrap();
        fs::write(&path, format!("{pretty}\n")).unwrap();
    }
    let golden: Value = serde_json::from_str(
        &fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .unwrap();
    let names = |v: &Value| -> Vec<String> {
        v["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(names(&golden), names(&actual), "the list of commands");
    for (want, got) in golden["commands"]
        .as_array()
        .unwrap()
        .iter()
        .zip(actual["commands"].as_array().unwrap())
    {
        for field in [
            "ble_frame",
            "mqtt_body",
            "redacted_bluetooth",
            "redacted_wifi",
        ] {
            assert_eq!(got[field], want[field], "{} {field}", want["name"]);
        }
    }
    assert_eq!(golden, actual);
}

#[test]
fn only_the_frames_carry_the_fake_password() {
    for (name, command) in commands() {
        for via in [Via::Bluetooth, Via::Wifi] {
            assert!(
                !command.redacted(via).contains("example-password"),
                "{name}"
            );
        }
        let carries = matches!(
            command,
            Command::SaveMqtt { .. } | Command::WifiConnect { .. }
        ) && name != "wifi_connect_escapes";
        assert_eq!(
            text(command.ble_frame()).contains("example-password"),
            carries,
            "{name}"
        );
        assert_eq!(
            text(command.mqtt_body()).contains("example-password"),
            carries,
            "{name}"
        );
    }
}
