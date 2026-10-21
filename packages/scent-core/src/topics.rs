/// Commands to a diffuser are published to this prefix plus its device ID.
pub const DEVICE_PREFIX: &str = "scentiment-mqtt-prod/device/";

/// What the app subscribes to: everything under the prefix. Subscribe again on every ConnAck.
pub const SUBSCRIBE_ALL: &str = "scentiment-mqtt-prod/#";

/// The topic to publish a command to. Check the ID with [`valid_device_id`] first;
/// this does not, so an ID with `/`, `+` or `#` would build a different topic.
pub fn device_topic(device_id: &str) -> String {
    format!("{DEVICE_PREFIX}{device_id}")
}

/// A device ID is 12 characters, each an upper-case letter or a digit.
pub fn valid_device_id(device_id: &str) -> bool {
    device_id.len() == 12
        && device_id
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_match_the_apps_constants() {
        assert_eq!(DEVICE_PREFIX, "scentiment-mqtt-prod/device/");
        assert_eq!(SUBSCRIBE_ALL, "scentiment-mqtt-prod/#");
        assert_eq!(
            device_topic("TESTDEVICE01"),
            "scentiment-mqtt-prod/device/TESTDEVICE01"
        );
    }

    #[test]
    fn device_ids_are_twelve_upper_case_letters_or_digits() {
        for good in ["TESTDEVICE01", "A1B2C3D4E5F6", "000000000000"] {
            assert!(valid_device_id(good), "{good}");
        }
        for bad in [
            "",
            "#",
            "abc",
            "TESTDEVICE01/../x",
            "TESTDEVICE0",
            "TESTDEVICE011",
            "testdevice01",
            "TESTDEVICE01\n",
            "TESTDEVICE0é",
            "TESTDEVICE+1",
        ] {
            assert!(!valid_device_id(bad), "{bad:?}");
        }
    }
}
