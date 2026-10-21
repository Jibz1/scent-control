//! Live check of the MQTT worker against the test broker. Needs no Bluetooth, so it
//! runs from a plain shell:
//!
//! ```text
//! cargo run -p scent-desktop --example mqtt_roundtrip -- <deviceId>
//! ```
//!
//! It connects, sends a `PING` with the route ID to that diffuser's device topic and
//! prints what the broker delivers for 10 s: your own publish coming back, and the
//! diffuser's acknowledgement if it is online. The broker password comes from the
//! Keychain; `SCENT_BROKER_PASSWORD` in the environment is a development fallback
//! for when the Keychain has none. Bodies are printed with passwords masked.
//!
//! The app is a binary crate, so the example includes the modules it needs by path.
#![allow(dead_code)]

#[path = "../src/add_flow.rs"]
mod add_flow;
#[path = "../src/mqtt.rs"]
mod mqtt;
#[path = "../src/secrets.rs"]
mod secrets;
#[path = "../src/shared_schedule.rs"]
mod shared_schedule;
#[path = "../src/state.rs"]
mod state;
#[path = "../src/store.rs"]
mod store;
#[path = "../src/ui/theme.rs"]
mod theme;
/// Only the theme, for the appearance the store saves.
mod ui {
    pub(crate) use super::theme;
}

use std::process::ExitCode;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use scent_core::{Command, Secret, parse_mqtt, valid_device_id};

use mqtt::BrokerSettings;
use state::{Event, mask_passwords};

/// How long to wait for the broker to accept the connection.
const CONNECT_WITHIN: Duration = Duration::from_secs(15);
/// How long to print events after the `PING` went out.
const LISTEN_FOR: Duration = Duration::from_secs(10);
/// After a refused login, how long to watch for a second event. Longer than the 5 s
/// retry delay, so a worker that kept retrying would show up (if it has stopped, the
/// channel closes first).
const WATCH_AFTER_REFUSAL: Duration = Duration::from_secs(8);

fn main() -> ExitCode {
    let Some(device_id) = std::env::args().nth(1) else {
        eprintln!("usage: mqtt_roundtrip <deviceId>");
        return ExitCode::from(2);
    };
    if !valid_device_id(&device_id) {
        eprintln!("not a device ID: 12 upper-case letters or digits");
        return ExitCode::from(2);
    }

    let settings = BrokerSettings::default();
    let password = secrets::broker_password(&settings)
        .or_else(|| std::env::var("SCENT_BROKER_PASSWORD").ok().map(Secret::new));
    let Some(password) = password else {
        eprintln!("no broker password: save it in the Keychain, or set SCENT_BROKER_PASSWORD");
        return ExitCode::from(2);
    };

    println!("broker {} as {}", settings.url, settings.username);
    let route_id = settings.route_id.clone();
    let (events, received) = mpsc::channel();
    let handle = mqtt::spawn(settings, password, events, || {});

    let started = Instant::now();
    let mut deadline = started + CONNECT_WITHIN;
    let mut connected = false;
    let mut refused = false;
    loop {
        let wait = deadline.saturating_duration_since(Instant::now());
        match received.recv_timeout(wait) {
            Ok(Event::Mqtt { topic, body, .. }) => {
                let message = parse_mqtt(&topic, &body);
                println!(
                    "[{:>5.1}s] Mqtt {topic}\n          action={:?} ack={:?}\n          body={}",
                    started.elapsed().as_secs_f32(),
                    message.action,
                    message.ack,
                    mask_passwords(&message.raw),
                );
            }
            Ok(event) => {
                println!("[{:>5.1}s] {event:?}", started.elapsed().as_secs_f32());
                match event {
                    Event::BrokerConnected if !connected => {
                        connected = true;
                        let ping = Command::Ping {
                            userid: Some(route_id.clone()),
                        };
                        match handle.send(&device_id, &ping) {
                            Ok(()) => {
                                println!("          sent {}", ping.redacted(scent_core::Via::Wifi))
                            }
                            Err(error) => {
                                eprintln!("send failed: {error}");
                                return ExitCode::FAILURE;
                            }
                        }
                        deadline = Instant::now() + LISTEN_FOR;
                    }
                    Event::BrokerLoginRefused(_) => {
                        refused = true;
                        deadline = Instant::now() + WATCH_AFTER_REFUSAL;
                    }
                    _ => {}
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if refused {
                    println!("login refused, and nothing more followed in time");
                    return ExitCode::FAILURE;
                }
                if !connected {
                    eprintln!("the broker did not accept the connection in time");
                    return ExitCode::FAILURE;
                }
                println!("done");
                return ExitCode::SUCCESS;
            }
            Err(RecvTimeoutError::Disconnected) => {
                // The worker is gone and has dropped its end of the channel.
                println!("the worker stopped");
                return ExitCode::FAILURE;
            }
        }
    }
}
