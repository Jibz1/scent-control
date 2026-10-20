//! Which way a command goes to a diffuser, and when to give up on Bluetooth for a
//! while. Pure: nothing here touches Bluetooth, the network or the clock.

use scent_core::Via;

/// How long Auto leaves Bluetooth alone after it failed to reach a diffuser. A failed
/// connect takes about 13 s, so without this every command to a diffuser that is out
/// of range would wait that long before it went over WiFi.
pub const BLE_RETRY_AFTER_S: i64 = 120;

/// How the person wants a command to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransportPref {
    /// Bluetooth when the diffuser is nearby, WiFi otherwise.
    #[default]
    Auto,
    Wifi,
    Bluetooth,
}

impl TransportPref {
    /// What is tried first: Bluetooth, unless WiFi is forced.
    pub fn first_choice(self) -> Via {
        match self {
            TransportPref::Wifi => Via::Wifi,
            TransportPref::Auto | TransportPref::Bluetooth => Via::Bluetooth,
        }
    }
}

/// The transports to try, in order. `ble_known` is whether Bluetooth is worth trying
/// for this diffuser (it has a Bluetooth ID, and for Auto, [`ble_worth_trying`]);
/// `on_broker` is whether the broker is connected. Only available transports are
/// kept. Auto is Bluetooth first, then WiFi; a forced transport has no fallback.
pub fn route(pref: TransportPref, ble_known: bool, on_broker: bool) -> Vec<Via> {
    let bluetooth = ble_known.then_some(Via::Bluetooth);
    let wifi = on_broker.then_some(Via::Wifi);
    match pref {
        TransportPref::Auto => [bluetooth, wifi].into_iter().flatten().collect(),
        TransportPref::Wifi => wifi.into_iter().collect(),
        TransportPref::Bluetooth => bluetooth.into_iter().collect(),
    }
}

/// Whether Auto should try Bluetooth now. `failed_at` is when Bluetooth last failed to
/// reach the diffuser, `heard_ble` when the diffuser last spoke over Bluetooth. Yes
/// when nothing has failed, when it was heard over Bluetooth after the failure (so it
/// is in range again), or when [`BLE_RETRY_AFTER_S`] have passed.
pub fn ble_worth_trying(failed_at: Option<i64>, heard_ble: Option<i64>, now: i64) -> bool {
    match failed_at {
        None => true,
        Some(failed) => {
            heard_ble.is_some_and(|heard| heard > failed)
                || now.saturating_sub(failed) >= BLE_RETRY_AFTER_S
        }
    }
}

/// Why `route` came back empty. `has_ble_id` is whether the diffuser has a Bluetooth
/// ID at all, which tells "never found over Bluetooth" from "failed lately".
pub fn why_no_route(pref: TransportPref, has_ble_id: bool) -> String {
    match pref {
        TransportPref::Wifi => "The broker isn't connected.".to_owned(),
        TransportPref::Bluetooth => "No Bluetooth ID for this diffuser.".to_owned(),
        TransportPref::Auto if has_ble_id => format!(
            "Bluetooth failed less than {} minutes ago and the broker isn't connected.",
            BLE_RETRY_AFTER_S / 60
        ),
        TransportPref::Auto => "No Bluetooth ID and the broker isn't connected.".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_is_bluetooth_first() {
        assert_eq!(
            route(TransportPref::Auto, true, true),
            vec![Via::Bluetooth, Via::Wifi]
        );
    }

    #[test]
    fn auto_without_bluetooth_id_uses_wifi() {
        assert_eq!(route(TransportPref::Auto, false, true), vec![Via::Wifi]);
    }

    #[test]
    fn auto_without_the_broker_uses_bluetooth() {
        assert_eq!(
            route(TransportPref::Auto, true, false),
            vec![Via::Bluetooth]
        );
    }

    #[test]
    fn forced_transports_have_no_fallback() {
        assert_eq!(route(TransportPref::Wifi, true, true), vec![Via::Wifi]);
        assert_eq!(
            route(TransportPref::Bluetooth, true, false),
            vec![Via::Bluetooth]
        );
        assert_eq!(
            route(TransportPref::Bluetooth, true, true),
            vec![Via::Bluetooth]
        );
    }

    #[test]
    fn nothing_available_is_an_empty_route() {
        assert!(route(TransportPref::Auto, false, false).is_empty());
    }

    #[test]
    fn a_forced_transport_that_is_unavailable_is_an_empty_route() {
        assert!(route(TransportPref::Wifi, true, false).is_empty());
        assert!(route(TransportPref::Bluetooth, false, true).is_empty());
    }

    #[test]
    fn the_default_is_auto() {
        assert_eq!(TransportPref::default(), TransportPref::Auto);
    }

    #[test]
    fn the_first_choice_is_bluetooth_unless_wifi_is_forced() {
        assert_eq!(TransportPref::Auto.first_choice(), Via::Bluetooth);
        assert_eq!(TransportPref::Bluetooth.first_choice(), Via::Bluetooth);
        assert_eq!(TransportPref::Wifi.first_choice(), Via::Wifi);
    }

    #[test]
    fn bluetooth_is_skipped_for_2_minutes_after_it_failed() {
        let failed = 1_000;
        assert_eq!(BLE_RETRY_AFTER_S, 120);
        // Nothing has failed: try it.
        assert!(ble_worth_trying(None, None, failed));
        assert!(ble_worth_trying(None, Some(failed - 500), failed));
        // Right after a failure, and up to the last second before 2 minutes: skip it.
        assert!(!ble_worth_trying(Some(failed), None, failed));
        assert!(!ble_worth_trying(Some(failed), None, failed + 119));
        assert!(ble_worth_trying(Some(failed), None, failed + 120));
        assert!(ble_worth_trying(Some(failed), None, failed + 10_000));
        // Hearing it over Bluetooth from before the failure changes nothing.
        assert!(!ble_worth_trying(
            Some(failed),
            Some(failed - 5),
            failed + 10
        ));
    }

    #[test]
    fn hearing_it_over_bluetooth_ends_the_cooldown() {
        let failed = 1_000;
        assert!(ble_worth_trying(Some(failed), Some(failed + 1), failed + 2));
        assert!(ble_worth_trying(
            Some(failed),
            Some(failed + 60),
            failed + 61
        ));
        // Heard in the very second it failed is not "after".
        assert!(!ble_worth_trying(Some(failed), Some(failed), failed + 2));
    }

    #[test]
    fn why_there_is_no_route() {
        assert_eq!(
            why_no_route(TransportPref::Auto, false),
            "No Bluetooth ID and the broker isn't connected."
        );
        assert_eq!(
            why_no_route(TransportPref::Auto, true),
            "Bluetooth failed less than 2 minutes ago and the broker isn't connected."
        );
        assert_eq!(
            why_no_route(TransportPref::Wifi, true),
            "The broker isn't connected."
        );
        assert_eq!(
            why_no_route(TransportPref::Bluetooth, false),
            "No Bluetooth ID for this diffuser."
        );
    }
}
