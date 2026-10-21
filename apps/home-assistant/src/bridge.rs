//! Pure Home Assistant mapping. Only real device reports update displayed state.

use scent_core::{Ack, Command, Level, ONLINE_FOR_S, PING_EVERY_S, is_stale, parse_mqtt};
use serde_json::json;

use crate::config::Config;

pub const CONNECTED_TOPIC: &str = "scentiment-mqtt-prod/server-notifications/device";
pub const HA_STATUS: &str = "homeassistant/status";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
    pub topic: String,
    pub payload: Vec<u8>,
    pub retain: bool,
}

impl Publication {
    pub fn state(topic: String, payload: impl Into<Vec<u8>>) -> Self {
        Self {
            topic,
            payload: payload.into(),
            retain: true,
        }
    }

    fn command(id: &str, command: Command) -> Self {
        Self {
            topic: scent_core::device_topic(id),
            payload: command.mqtt_body(),
            retain: false,
        }
    }
}

/// Firmware reports only on a change, so a reading stays current, however old, until
/// a reset (a new session or a device reconnect) clears it.
#[derive(Default)]
struct Readings {
    heard_at: Option<i64>,
    power: Option<bool>,
    level: Option<u8>,
    /// When the diffuser last reported power or intensity.
    reported_at: Option<i64>,
    /// When we last sent it `GET_STATUS`.
    asked_at: Option<i64>,
}

impl Readings {
    fn available(&self, now: i64) -> bool {
        self.heard_at
            .is_some_and(|at| now.saturating_sub(at) <= ONLINE_FOR_S)
    }
}

pub struct Bridge {
    pub config: Config,
    readings: Vec<Readings>,
    last_ping: Option<i64>,
}

impl Bridge {
    pub fn new(config: Config) -> Self {
        let readings = config.devices.iter().map(|_| Readings::default()).collect();
        Self {
            config,
            readings,
            last_ping: None,
        }
    }

    pub fn availability_topic(&self) -> String {
        format!("{}/availability", self.config.prefix())
    }

    pub fn device_prefix(&self, index: usize) -> String {
        format!("{}/{}", self.config.prefix(), self.config.devices[index].id)
    }

    pub fn command_topics(&self) -> Vec<String> {
        (0..self.config.devices.len())
            .flat_map(|i| {
                [
                    format!("{}/power/set", self.device_prefix(i)),
                    format!("{}/level/set", self.device_prefix(i)),
                ]
            })
            .collect()
    }

    pub fn discovery(&self) -> Vec<Publication> {
        self.config
            .devices
            .iter()
            .enumerate()
            .map(|(i, device)| {
                let prefix = self.device_prefix(i);
                let unique = format!("scent_control_{}_{}", self.config.instance_id, device.id);
                let body = json!({
                    "name": null,
                    "unique_id": unique,
                    "device": {
                        "identifiers": [unique],
                        "name": device.name,
                        "manufacturer": "Scentiment",
                        "model": device.model.label()
                    },
                    "origin": {"name": "Scent Control", "sw_version": env!("CARGO_PKG_VERSION")},
                    "command_topic": format!("{prefix}/power/set"),
                    "state_topic": format!("{prefix}/power/state"),
                    "percentage_command_topic": format!("{prefix}/level/set"),
                    "percentage_state_topic": format!("{prefix}/level/state"),
                    "speed_range_min": 1,
                    "speed_range_max": 3,
                    "payload_on": "ON",
                    "payload_off": "OFF",
                    "optimistic": false,
                    "retain": false,
                    "qos": 0,
                    "availability_mode": "all",
                    "availability": [
                        {"topic": self.availability_topic()},
                        {"topic": format!("{prefix}/availability")}
                    ]
                });
                Publication::state(
                    format!("homeassistant/fan/{unique}/config"),
                    body.to_string(),
                )
            })
            .collect()
    }

    /// Republish after discovery / HA birth. Only availability depends on `now`.
    pub fn snapshot(&self, now: i64) -> Vec<Publication> {
        self.readings
            .iter()
            .enumerate()
            .flat_map(|(i, state)| {
                let prefix = self.device_prefix(i);
                let power = match state.power {
                    Some(true) => "ON",
                    Some(false) => "OFF",
                    None => "None",
                };
                let level = state
                    .level
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "None".to_owned());
                [
                    Publication::state(format!("{prefix}/power/state"), power),
                    Publication::state(format!("{prefix}/level/state"), level),
                    Publication::state(
                        format!("{prefix}/availability"),
                        if state.available(now) {
                            "online"
                        } else {
                            "offline"
                        },
                    ),
                ]
            })
            .collect()
    }

    pub fn report(&mut self, topic: &str, payload: &[u8], retained: bool, now: i64) {
        if retained || (topic != self.config.report_topic() && topic != CONNECTED_TOPIC) {
            return;
        }
        let message = parse_mqtt(topic, payload);
        if topic == CONNECTED_TOPIC && message.action.as_deref() != Some("device_connected") {
            return;
        }
        let Some(index) = self
            .config
            .devices
            .iter()
            .position(|d| Some(d.id.as_str()) == message.device_id.as_deref())
        else {
            return;
        };
        let reading = &mut self.readings[index];
        match message.action.as_deref() {
            Some("device_connected") if message.connected.is_some() => {
                // A restart is evidence of reachability, not a power/level change.
                *reading = Readings::default();
                reading.heard_at = Some(now);
            }
            Some("acknowledgement")
                if matches!(message.ack, Some(Ack::Added | Ack::AlreadyAdded)) =>
            {
                reading.heard_at = Some(now);
            }
            Some("notification" | "update_status") if message.status.is_some() => {
                let status = message.status.unwrap();
                reading.heard_at = Some(now);
                if let Some(power @ (0 | 1)) = status.power {
                    reading.power = Some(power == 1);
                    reading.reported_at = Some(now);
                }
                if let Some(level @ 0..=3) = status.intensity {
                    reading.level = Some(level as u8);
                    reading.reported_at = Some(now);
                }
            }
            _ => {}
        }
    }

    pub fn pings(&mut self, now: i64) -> Vec<Publication> {
        if self
            .last_ping
            .is_some_and(|at| now.saturating_sub(at) < PING_EVERY_S)
        {
            return Vec::new();
        }
        self.last_ping = Some(now);
        self.config
            .devices
            .iter()
            .map(|d| {
                Publication::command(
                    &d.id,
                    Command::Ping {
                        userid: Some(self.config.route_id.clone()),
                    },
                )
            })
            .collect()
    }

    /// Firmware reports only on a change, and a reset leaves nothing known. Ask each
    /// reachable diffuser for its status while its power or intensity is unknown, or
    /// once its last report is older than `STALE_AFTER_S`. `GET_STATUS` answers only
    /// sometimes, so asks are at least `STALE_AFTER_S` apart. Never changes readings.
    pub fn status_requests(&mut self, now: i64) -> Vec<Publication> {
        let due = |at: Option<i64>| at.is_none_or(|at| is_stale(at, now));
        self.readings
            .iter_mut()
            .zip(&self.config.devices)
            .filter_map(|(reading, device)| {
                let unknown = reading.power.is_none() || reading.level.is_none();
                let ask = reading.available(now)
                    && (unknown || due(reading.reported_at))
                    && due(reading.asked_at);
                ask.then(|| {
                    reading.asked_at = Some(now);
                    Publication::command(&device.id, Command::GetStatus)
                })
            })
            .collect()
    }

    /// No state optimism, schedule writes, or desired-state replay. The runtime gates
    /// this on both subscriptions being ready and discards queues after any outage.
    pub fn command(
        &self,
        topic: &str,
        payload: &[u8],
        retained: bool,
        now: i64,
    ) -> Result<Vec<Publication>, &'static str> {
        if retained {
            return Err("Ignored a retained command.");
        }
        let (index, field) = self
            .config
            .devices
            .iter()
            .enumerate()
            .find_map(|(i, _)| {
                let prefix = self.device_prefix(i);
                if topic == format!("{prefix}/power/set") {
                    Some((i, "power"))
                } else if topic == format!("{prefix}/level/set") {
                    Some((i, "level"))
                } else {
                    None
                }
            })
            .ok_or("Ignored a command for an unconfigured topic.")?;
        if !self.readings[index].available(now) {
            return Err("Diffuser is unavailable; command discarded.");
        }
        let id = &self.config.devices[index].id;
        let commands = match (field, payload) {
            ("power", b"OFF") | ("level", b"0") => vec![Command::TurnOn { on: false }],
            // As the desktop does: the diffuser comes back at its own intensity.
            ("power", b"ON") => vec![Command::TurnOn { on: true }],
            ("level", [level @ b'1'..=b'3']) => vec![
                Command::SetLevel(Level::new(level - b'0').unwrap()),
                Command::TurnOn { on: true },
            ],
            _ => return Err("Invalid command; expected ON/OFF or an integer level 0–3."),
        };
        Ok(commands
            .into_iter()
            .map(|c| Publication::command(id, c))
            .collect())
    }
}
