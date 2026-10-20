//! Putting a diffuser on our broker, as a pure state machine: which step comes after
//! which, given what the diffuser said about itself, and when waiting for it has
//! failed. The hub runs the steps; nothing here touches Bluetooth, the network or the
//! clock.
//!
//! The order (see `docs/findings.md`, "Phase 0 results"): read the info over Bluetooth;
//! `SAVE_MQTT` with our broker; `WIFI_CONNECT` only when the diffuser isn't on WiFi and
//! the person gave a network; a soft restart; then wait up to 90 s for the diffuser's
//! `device_connected` on our broker, and `PING` it with our route ID.

use scent_core::{DeviceInfo, Secret};

/// How long a diffuser has to show up on our broker after it was told to restart.
pub const WAIT_CONNECTED_S: i64 = 90;

/// How long to wait after `SAVE_MQTT` for the diffuser's "Credentials saved" before it
/// is restarted anyway. It makes a test login with them within about 20 s.
pub const CREDENTIALS_WAIT_S: u64 = 25;

/// Why an add stops before it starts: `SAVE_MQTT` hands the diffuser our broker login.
pub const NO_BROKER_PASSWORD: &str = "Save the broker password in Settings first.";

/// What the add sheet asks the hub to do. Not `Debug`: it can hold a WiFi password.
pub struct AddRequest {
    /// The diffuser's Bluetooth ID, as the scan reported it.
    pub ble_id: String,
    /// What to call it once it is added. Empty leaves its device ID as its name.
    pub label: String,
    /// The network to join and its password, for a diffuser that isn't on WiFi yet.
    /// Sent once over Bluetooth and never kept.
    pub wifi: Option<(String, Secret)>,
}

/// Where an add is. The hub reports each step as it starts it (`Event::Adding`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddStep {
    /// Reading the info over Bluetooth, to learn its device ID and whether it is on WiFi.
    ReadInfo,
    /// It isn't on WiFi and no network was given: the person is asked for one, and adds
    /// again with it.
    NeedWifi,
    SaveMqtt,
    WifiConnect,
    Restart,
    /// Waiting for its `device_connected` on our broker.
    WaitConnected,
    Ping,
    /// On our broker, with this device ID.
    Done(String),
    /// Stopped, and why, in words for the person.
    Failed(String),
}

impl AddStep {
    /// Whether the hub is working on this step, as against waiting for the person or
    /// having finished.
    pub fn is_running(&self) -> bool {
        !matches!(
            self,
            AddStep::NeedWifi | AddStep::Done(_) | AddStep::Failed(_)
        )
    }
}

/// The step after `step`. `info` is what the diffuser said about itself, `wifi_given`
/// whether the person gave a network, and `connected_device_id` the device whose
/// `device_connected` was heard since the restart. Without the info nothing moves; a
/// step that waits for something stays where it is until it comes.
pub fn next(
    step: &AddStep,
    info: Option<&DeviceInfo>,
    wifi_given: bool,
    connected_device_id: Option<&str>,
) -> AddStep {
    let Some(info) = info else {
        return step.clone();
    };
    match step {
        AddStep::ReadInfo if wifi_given || info.wifi_configured => AddStep::SaveMqtt,
        AddStep::ReadInfo => AddStep::NeedWifi,
        AddStep::SaveMqtt if wifi_given => AddStep::WifiConnect,
        AddStep::SaveMqtt | AddStep::WifiConnect => AddStep::Restart,
        AddStep::Restart => AddStep::WaitConnected,
        AddStep::WaitConnected if connected_device_id == Some(info.device_id.as_str()) => {
            AddStep::Ping
        }
        AddStep::Ping => AddStep::Done(info.device_id.clone()),
        AddStep::WaitConnected | AddStep::NeedWifi | AddStep::Done(_) | AddStep::Failed(_) => {
            step.clone()
        }
    }
}

/// How the wait for `device_connected` stands at `now`, when it began at `started`
/// (both in seconds): `Ping` once it `connected`, still `WaitConnected` within
/// [`WAIT_CONNECTED_S`], and `Failed` after that, with what to check.
pub fn wait_outcome(started: i64, now: i64, connected: bool) -> AddStep {
    if connected {
        AddStep::Ping
    } else if now.saturating_sub(started) < WAIT_CONNECTED_S {
        AddStep::WaitConnected
    } else {
        AddStep::Failed(format!(
            "The diffuser didn't join your broker within {WAIT_CONNECTED_S} s. Check the WiFi password, that the network is 2.4 GHz (the diffuser can't use 5 GHz), and that the diffuser is within range of the router, then try again."
        ))
    }
}

#[cfg(test)]
mod tests {
    use scent_core::DeviceInfo;

    use super::*;

    const ONE: &str = "TESTDEVICE01";

    fn info(wifi_configured: bool) -> DeviceInfo {
        DeviceInfo {
            manufacturer_id: "01".into(),
            model_name: "Diffuser".into(),
            variant_name: "Black".into(),
            device_id: ONE.into(),
            firmware: "2.30".into(),
            wifi_configured,
            wifi_status: if wifi_configured { 3 } else { 0 },
        }
    }

    /// The steps from `step` on, each from the one before, until one leads nowhere new.
    fn walk(
        step: AddStep,
        info: &DeviceInfo,
        wifi_given: bool,
        connected: Option<&str>,
    ) -> Vec<AddStep> {
        let mut steps = vec![step];
        loop {
            let last = steps.last().unwrap();
            let next = next(last, Some(info), wifi_given, connected);
            if &next == last {
                return steps;
            }
            steps.push(next);
        }
    }

    #[test]
    fn configured_device_skips_wifi() {
        let steps = walk(AddStep::ReadInfo, &info(true), false, None);
        assert_eq!(
            steps,
            [
                AddStep::ReadInfo,
                AddStep::SaveMqtt,
                AddStep::Restart,
                AddStep::WaitConnected
            ]
        );
    }

    #[test]
    fn unconfigured_device_asks_for_wifi() {
        let unconfigured = info(false);
        assert_eq!(
            next(&AddStep::ReadInfo, Some(&unconfigured), false, None),
            AddStep::NeedWifi
        );
        // The flow stops there, for the person to give the network.
        assert_eq!(
            next(&AddStep::NeedWifi, Some(&unconfigured), false, None),
            AddStep::NeedWifi
        );
        let steps = walk(AddStep::ReadInfo, &unconfigured, true, None);
        assert_eq!(
            steps,
            [
                AddStep::ReadInfo,
                AddStep::SaveMqtt,
                AddStep::WifiConnect,
                AddStep::Restart,
                AddStep::WaitConnected
            ]
        );
    }

    #[test]
    fn waits_for_its_own_device_connected() {
        let info = info(true);
        let wait = AddStep::WaitConnected;
        assert_eq!(next(&wait, Some(&info), false, None), wait);
        assert_eq!(next(&wait, Some(&info), false, Some("TESTDEVICE02")), wait);
        assert_eq!(next(&wait, Some(&info), false, Some(ONE)), AddStep::Ping);
        assert_eq!(
            next(&AddStep::Ping, Some(&info), false, Some(ONE)),
            AddStep::Done(ONE.to_owned())
        );
        // Done and Failed lead nowhere.
        let done = AddStep::Done(ONE.to_owned());
        assert_eq!(next(&done, Some(&info), false, Some(ONE)), done);
        let failed = AddStep::Failed("No.".to_owned());
        assert_eq!(next(&failed, Some(&info), false, Some(ONE)), failed);
    }

    #[test]
    fn without_the_info_nothing_moves() {
        for step in [AddStep::ReadInfo, AddStep::SaveMqtt, AddStep::Ping] {
            assert_eq!(next(&step, None, true, Some(ONE)), step);
        }
    }

    #[test]
    fn times_out_after_90_s() {
        let started = 1_000;
        assert_eq!(
            wait_outcome(started, started, false),
            AddStep::WaitConnected
        );
        assert_eq!(
            wait_outcome(started, started + 89, false),
            AddStep::WaitConnected
        );
        // Connected in time, even at the last second, or at once.
        assert_eq!(wait_outcome(started, started + 89, true), AddStep::Ping);
        assert_eq!(wait_outcome(started, started, true), AddStep::Ping);
        let AddStep::Failed(why) = wait_outcome(started, started + 90, false) else {
            panic!("not a failure at 90 s");
        };
        for named in ["WiFi password", "2.4 GHz", "range"] {
            assert!(why.contains(named), "{why}");
        }
        assert!(matches!(
            wait_outcome(started, started + 600, false),
            AddStep::Failed(_)
        ));
    }

    #[test]
    fn running_steps_are_the_ones_the_hub_is_working_on() {
        for step in [
            AddStep::ReadInfo,
            AddStep::SaveMqtt,
            AddStep::WifiConnect,
            AddStep::Restart,
            AddStep::WaitConnected,
            AddStep::Ping,
        ] {
            assert!(step.is_running(), "{step:?}");
        }
        for step in [
            AddStep::NeedWifi,
            AddStep::Done(ONE.to_owned()),
            AddStep::Failed("No.".to_owned()),
        ] {
            assert!(!step.is_running(), "{step:?}");
        }
    }
}
