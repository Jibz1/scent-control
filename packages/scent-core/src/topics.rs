/// Commands to a diffuser are published to this prefix plus its device ID.
pub const DEVICE_PREFIX: &str = "scentiment-mqtt-prod/device/";

/// What the app subscribes to: everything under the prefix. Subscribe again on every ConnAck.
pub const SUBSCRIBE_ALL: &str = "scentiment-mqtt-prod/#";

/// The topic to publish a command to. Check the ID with [`valid_device_id`] first;
/// this does not, so an ID with `/`, `+` or `#` would build a different topic.
pub fn device_topic(device_id: &str) -> String {
    format!("{DEVICE_PREFIX}{device_id}")
}

/// Our own prefix, written once: the constants below are built from it.
macro_rules! app_prefix {
    () => {
        "scent-control/v1/"
    };
}

/// Where Scent Control keeps what it stores on the broker for its other copies. Never
/// under Scentiment's prefix: the diffusers subscribe there, and a retained message
/// there would be replayed to a diffuser every time it connects.
pub const APP_PREFIX: &str = app_prefix!();

/// What the app subscribes to besides [`SUBSCRIBE_ALL`]: everything under our prefix.
/// Subscribe again on every ConnAck.
pub const SUBSCRIBE_APP: &str = concat!(app_prefix!(), "#");

/// The retained copy of the last schedule sent to a diffuser is kept at this prefix
/// plus its device ID.
pub const SCHEDULE_PREFIX: &str = concat!(app_prefix!(), "schedule/");

/// The topic of a diffuser's stored schedule, or `None` when `device_id` is not a
/// device ID, so that no ID can build a topic outside [`SCHEDULE_PREFIX`].
pub fn schedule_topic(device_id: &str) -> Option<String> {
    valid_device_id(device_id).then(|| format!("{SCHEDULE_PREFIX}{device_id}"))
}

/// The device ID a stored schedule's topic is for, or `None` when the topic is not one.
pub fn schedule_topic_device(topic: &str) -> Option<&str> {
    topic
        .strip_prefix(SCHEDULE_PREFIX)
        .filter(|device_id| valid_device_id(device_id))
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
    fn our_own_topics_are_under_one_prefix_apart_from_scentiments() {
        assert_eq!(APP_PREFIX, "scent-control/v1/");
        assert_eq!(SUBSCRIBE_APP, "scent-control/v1/#");
        assert_eq!(SCHEDULE_PREFIX, "scent-control/v1/schedule/");
        for ours in [SUBSCRIBE_APP, SCHEDULE_PREFIX] {
            assert!(ours.starts_with(APP_PREFIX), "{ours}");
        }
        // The diffusers subscribe under Scentiment's prefix: nothing of ours is there.
        assert!(!APP_PREFIX.starts_with("scentiment-mqtt-prod/"));
        assert!(!"scentiment-mqtt-prod/".starts_with(APP_PREFIX));
    }

    #[test]
    fn a_schedule_topic_is_built_only_for_a_valid_device_id() {
        assert_eq!(
            schedule_topic("TESTDEVICE01").as_deref(),
            Some("scent-control/v1/schedule/TESTDEVICE01")
        );
        for bad in [
            "",
            "#",
            "+",
            "TESTDEVICE01/../x",
            "TESTDEVICE01/#",
            "testdevice01",
            "../../scentiment-mqtt-prod/device/TESTDEVICE01",
        ] {
            assert_eq!(schedule_topic(bad), None, "{bad:?}");
        }
        // Whatever it is given, it never builds a topic a diffuser listens on.
        for id in [
            "TESTDEVICE01",
            "TESTDEVICE02",
            "scentiment-mqtt-prod/device/X",
        ] {
            if let Some(topic) = schedule_topic(id) {
                assert!(topic.starts_with(SCHEDULE_PREFIX), "{topic}");
                assert!(!topic.starts_with("scentiment-mqtt-prod/"), "{topic}");
                assert!(!topic.contains(['+', '#']), "{topic}");
            }
        }
    }

    #[test]
    fn the_device_of_a_schedule_topic_is_read_back_only_when_valid() {
        assert_eq!(
            schedule_topic_device("scent-control/v1/schedule/TESTDEVICE01"),
            Some("TESTDEVICE01")
        );
        for other in [
            "scent-control/v1/schedule/",
            "scent-control/v1/schedule/testdevice01",
            "scent-control/v1/schedule/TESTDEVICE01/x",
            "scent-control/v1/other/TESTDEVICE01",
            "scent-control/v2/schedule/TESTDEVICE01",
            "scentiment-mqtt-prod/device/TESTDEVICE01",
            "",
        ] {
            assert_eq!(schedule_topic_device(other), None, "{other:?}");
        }
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
