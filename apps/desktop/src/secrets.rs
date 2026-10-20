//! The broker password, kept in the macOS Keychain and nowhere else.
//!
//! Keychain service `scent-control`, account `<username>@<broker host>`.

use keyring::Entry;
use scent_core::Secret;

use crate::mqtt::{BrokerSettings, BrokerUrl};

const SERVICE: &str = "scent-control";

/// The Keychain account for these settings: `scent-test@test-broker-production.up.railway.app`.
fn account(settings: &BrokerSettings) -> Result<String, String> {
    let url = BrokerUrl::parse(&settings.url)?;
    Ok(format!("{}@{}", settings.username, url.host))
}

fn entry(settings: &BrokerSettings) -> Result<Entry, String> {
    Entry::new(SERVICE, &account(settings)?).map_err(|error| format!("Keychain: {error}"))
}

/// The saved broker password. `None` means one of two things the caller can't tell
/// apart: nothing is saved, or the Keychain couldn't be read (a read error is also
/// reported on stderr). So a prompt for the password should mention the Keychain.
pub fn broker_password(settings: &BrokerSettings) -> Option<Secret> {
    match entry(settings).ok()?.get_password() {
        Ok(password) => Some(Secret::new(password)),
        Err(keyring::Error::NoEntry) => None,
        Err(error) => {
            eprintln!("Keychain: couldn't read the broker password: {error}");
            None
        }
    }
}

/// Saves the broker password, replacing any earlier one.
pub fn set_broker_password(settings: &BrokerSettings, password: &Secret) -> Result<(), String> {
    entry(settings)?
        .set_password(password.expose())
        .map_err(|error| format!("Keychain: couldn't save the broker password: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mqtt::BrokerSettings;

    // These tests never touch the Keychain: they only check the entry's name.
    #[test]
    fn the_account_is_username_at_host() {
        assert_eq!(
            account(&BrokerSettings::default()).as_deref(),
            Ok("scent-test@test-broker-production.up.railway.app")
        );
    }

    #[test]
    fn a_bad_broker_address_has_no_account() {
        let settings = BrokerSettings {
            url: "ws://broker.example/mqtt".into(),
            ..BrokerSettings::default()
        };
        assert!(account(&settings).is_err());
    }
}
