//! Local configuration contains identities and environment variable names, never passwords.

use std::collections::HashSet;
use std::time::Duration;

use rumqttc::{LastWill, MqttOptions, Transport};
use scent_core::{Secret, valid_device_id};
use serde::Deserialize;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub instance_id: String,
    /// Must match the route already stored by the desktop/setup app.
    pub route_id: String,
    pub diffuser_broker: Broker,
    pub home_assistant_broker: Broker,
    pub devices: Vec<Device>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub model: Model,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Model {
    Air2,
    Hvac2,
}

impl Model {
    pub fn label(self) -> &'static str {
        match self {
            Self::Air2 => "Diffuser Air 2",
            Self::Hvac2 => "Diffuser HVAC 2",
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Broker {
    pub host: String,
    pub port: u16,
    pub transport: Protocol,
    #[serde(default = "mqtt_path")]
    pub path: String,
    pub username: String,
    pub password_env: String,
    /// Unencrypted TCP is opt-in for a trusted local HA broker only.
    #[serde(default)]
    pub allow_insecure: bool,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Wss,
    Mqtts,
    Tcp,
}

fn mqtt_path() -> String {
    "/mqtt".to_owned()
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

impl Config {
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        // Serde errors can quote invalid values; do not print untrusted config data.
        let config: Self = serde_json::from_slice(bytes)
            .map_err(|_| "Invalid configuration JSON or an unsupported field (passwords belong in environment variables).")?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if !identifier(&self.instance_id) || !identifier(&self.route_id) {
            return Err(
                "instance_id and route_id must be 1–64 letters, digits, underscores or hyphens.",
            );
        }
        self.diffuser_broker.validate(true)?;
        self.home_assistant_broker.validate(false)?;
        if self.devices.is_empty() || self.devices.len() > 32 {
            return Err("Configure between 1 and 32 devices.");
        }
        let mut ids = HashSet::new();
        for device in &self.devices {
            if !valid_device_id(&device.id) || !ids.insert(&device.id) {
                return Err("Each device needs a unique, valid 12-character device ID.");
            }
            if device.name.trim().is_empty()
                || device.name.len() > 128
                || device.name.chars().any(char::is_control)
            {
                return Err("Each device needs a name of 1–128 bytes without control characters.");
            }
        }
        Ok(())
    }

    pub fn prefix(&self) -> String {
        format!("scent-control/{}", self.instance_id)
    }

    pub fn report_topic(&self) -> String {
        format!("scentiment-mqtt-prod/user/{}", self.route_id)
    }
}

impl Broker {
    fn validate(&self, upstream: bool) -> Result<(), &'static str> {
        if self.host.is_empty()
            || self.host.len() > 253
            || !self
                .host
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
            || self.port == 0
        {
            return Err("Broker host must be a DNS name or IPv4 address, with a nonzero port.");
        }
        if !self.path.starts_with('/')
            || !self
                .path
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"/._-".contains(&c))
        {
            return Err(
                "Broker path must be an absolute path without credentials, query or fragment.",
            );
        }
        if self.username.is_empty() || !identifier(&self.password_env) {
            return Err("Each broker needs a username and a password_env variable name.");
        }
        if self.transport == Protocol::Tcp {
            let loopback = self.host == "localhost" || self.host == "127.0.0.1";
            if !self.allow_insecure || (upstream && !loopback) {
                return Err(
                    "TCP requires allow_insecure; the diffuser broker requires TLS except on loopback for tests.",
                );
            }
        }
        Ok(())
    }

    pub fn password(&self) -> Result<Secret, &'static str> {
        std::env::var(&self.password_env)
            .ok()
            .filter(|value| !value.is_empty())
            .map(Secret::new)
            .ok_or("A broker password environment variable is missing or empty.")
    }

    pub fn options(
        &self,
        id: String,
        password: &Secret,
        will: Option<LastWill>,
    ) -> Result<MqttOptions, &'static str> {
        let (host, transport) = match self.transport {
            Protocol::Wss => (
                format!("wss://{}:{}{}", self.host, self.port, self.path),
                system_roots(Transport::wss_with_default_config)?,
            ),
            Protocol::Mqtts => (
                self.host.clone(),
                system_roots(Transport::tls_with_default_config)?,
            ),
            Protocol::Tcp => (self.host.clone(), Transport::Tcp),
        };
        let mut options = MqttOptions::new(id, host, self.port);
        options
            .set_transport(transport)
            .set_clean_session(true)
            .set_keep_alive(Duration::from_secs(15))
            .set_max_packet_size(256 * 1024, 256 * 1024)
            .set_credentials(&self.username, password.expose());
        if let Some(will) = will {
            options.set_last_will(will);
        }
        Ok(options)
    }
}

/// rumqttc panics if any of the system's root certificates fails to load. Make that
/// an ordinary session failure, retried like the rest, as the desktop app does.
fn system_roots(transport: fn() -> Transport) -> Result<Transport, &'static str> {
    std::panic::catch_unwind(transport)
        .map_err(|_| "Could not load the system's root certificates.")
}
