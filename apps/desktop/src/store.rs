//! The app's memory between runs: the diffusers it knows, their names and their
//! last readings, and where the broker is, in `desktop.json`.
//!
//! The file holds devices and the broker's address and nothing else: no log, no live
//! status, and no secrets (the broker password lives in the Keychain, and WiFi
//! passwords are never kept).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;

use scent_core::{DeviceInfo, Via, valid_device_id};
use serde::{Deserialize, Serialize};

use crate::mqtt::{BrokerSettings, BrokerUrl};
use crate::state::{AppState, DeviceView, Field, Reading};

const FILE_NAME: &str = "desktop.json";
const TEMP_NAME: &str = "desktop.json.tmp";
/// A file we could not read is moved here so the next save doesn't overwrite it.
const CORRUPT_NAME: &str = "desktop.json.corrupt";

const BAD_BROKER_ADDRESS: &str =
    "The saved broker address wasn't valid, so the default is used. Check it in Settings.";

pub struct Store {
    pub dir: PathBuf,
}

/// What [`Store::load`] found.
#[derive(Debug)]
pub struct Loaded {
    pub state: AppState,
    /// The default settings when the file doesn't say.
    pub broker: BrokerSettings,
    /// Set when there was a file and it could not be used, for the screens to show.
    pub notice: Option<String>,
}

impl Loaded {
    fn fresh(notice: Option<String>) -> Loaded {
        Loaded {
            state: AppState::default(),
            broker: BrokerSettings::default(),
            notice,
        }
    }
}

impl Store {
    /// `~/Library/Application Support/ScentControl`.
    pub fn default_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("ScentControl")
    }

    fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    /// What was saved, or an empty state and the default settings when there is
    /// nothing usable: no file yet, a file we can't read, or one that isn't what we
    /// wrote. The problem goes to stderr. A file that is damaged is kept as
    /// `desktop.json.corrupt`, and the notice says so. No file yet is not a problem.
    pub fn load(&self) -> Loaded {
        let path = self.path();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                eprintln!("scent-control: no saved state at {} yet", path.display());
                return Loaded::fresh(None);
            }
            Err(error) => {
                eprintln!("scent-control: can't read {}: {error}", path.display());
                return Loaded::fresh(Some(format!(
                    "Scent Control couldn't read its saved diffusers from {}: {error}. \
                     It started empty, and the next save replaces that file.",
                    path.display()
                )));
            }
        };
        match serde_json::from_slice::<Stored>(&bytes) {
            Ok(stored) => stored.into_loaded(),
            Err(error) => {
                eprintln!(
                    "scent-control: {} is not usable ({error}); starting empty",
                    path.display()
                );
                let aside = self.dir.join(CORRUPT_NAME);
                let notice = match fs::rename(&path, &aside) {
                    Ok(()) => format!(
                        "Scent Control's saved diffusers file was damaged, so it started \
                         empty. The damaged file was kept as {CORRUPT_NAME} in {}.",
                        self.dir.display()
                    ),
                    Err(error) => {
                        eprintln!(
                            "scent-control: can't keep it as {}: {error}",
                            aside.display()
                        );
                        format!(
                            "Scent Control's saved diffusers file ({}) was damaged, so it \
                             started empty. It couldn't be set aside ({error}), so the next \
                             save replaces it.",
                            path.display()
                        )
                    }
                };
                Loaded::fresh(Some(notice))
            }
        }
    }

    /// What [`save`](Self::save) writes for this state: the devices and the broker
    /// settings, as JSON. Only what the file holds is in it, so two states give the
    /// same bytes exactly when what would be saved is the same.
    pub fn contents(state: &AppState, broker: &BrokerSettings) -> io::Result<Vec<u8>> {
        serde_json::to_vec_pretty(&Stored::from_state(state, broker)).map_err(io::Error::other)
    }

    /// Writes the devices and the broker settings (see [`contents`](Self::contents)).
    /// The app writes through `contents` and `write` instead, so as to leave the file
    /// alone when it would not change.
    #[cfg(test)]
    pub fn save(&self, state: &AppState, broker: &BrokerSettings) -> io::Result<()> {
        self.write(&Self::contents(state, broker)?)
    }

    /// Writes the file. It is written whole under another name and then renamed over
    /// the old one, so a crash leaves the old file or the new one.
    pub fn write(&self, json: &[u8]) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let temp = self.dir.join(TEMP_NAME);
        let written = (|| {
            let mut file = File::create(&temp)?;
            file.write_all(json)?;
            file.sync_all()?;
            fs::rename(&temp, self.path())
        })();
        if written.is_err() {
            let _ = fs::remove_file(&temp);
        }
        written
    }
}

/// The saved file. Its own types keep serde out of the state and the readings'
/// source readable (`"wifi"`, `"bluetooth"`).
#[derive(Serialize, Deserialize)]
struct Stored {
    devices: Vec<StoredDevice>,
    /// Missing from files written before the settings were kept.
    #[serde(default)]
    broker: StoredBroker,
}

/// Where the broker is. Not the password: that is in the Keychain.
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct StoredBroker {
    url: String,
    username: String,
    route_id: String,
}

impl Default for StoredBroker {
    fn default() -> StoredBroker {
        StoredBroker::from(&BrokerSettings::default())
    }
}

impl From<&BrokerSettings> for StoredBroker {
    fn from(broker: &BrokerSettings) -> StoredBroker {
        StoredBroker {
            url: broker.url.clone(),
            username: broker.username.clone(),
            route_id: broker.route_id.clone(),
        }
    }
}

impl From<StoredBroker> for BrokerSettings {
    fn from(broker: StoredBroker) -> BrokerSettings {
        BrokerSettings {
            url: broker.url,
            username: broker.username,
            route_id: broker.route_id,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct StoredDevice {
    id: String,
    label: String,
    #[serde(default)]
    ble_id: Option<String>,
    #[serde(default)]
    info: Option<StoredInfo>,
    #[serde(default)]
    readings: BTreeMap<String, StoredReading>,
}

#[derive(Serialize, Deserialize)]
struct StoredInfo {
    manufacturer_id: String,
    model_name: String,
    variant_name: String,
    device_id: String,
    firmware: String,
    wifi_configured: bool,
    wifi_status: u8,
}

#[derive(Serialize, Deserialize)]
struct StoredReading {
    value: i64,
    at: i64,
    via: String,
}

impl Stored {
    fn from_state(state: &AppState, broker: &BrokerSettings) -> Stored {
        Stored {
            broker: broker.into(),
            devices: state
                .devices
                .values()
                .map(|device| StoredDevice {
                    id: device.id.clone(),
                    label: device.label.clone(),
                    ble_id: device.ble_id.clone(),
                    info: device.info.as_ref().map(StoredInfo::from),
                    readings: device
                        .readings
                        .iter()
                        .map(|(field, reading)| (field.key().to_owned(), reading.into()))
                        .collect(),
                })
                .collect(),
        }
    }

    /// Anything that can't be used is left out, with a note on stderr: a device
    /// whose ID is not a device ID, a reading of a field or source we don't know.
    fn into_loaded(self) -> Loaded {
        let mut state = AppState::default();
        for stored in self.devices {
            if !valid_device_id(&stored.id) {
                eprintln!(
                    "scent-control: ignoring saved device {:?}: not a device ID",
                    stored.id
                );
                continue;
            }
            let mut device = DeviceView::new(&stored.id);
            device.label = stored.label;
            device.ble_id = stored.ble_id;
            device.info = stored.info.map(DeviceInfo::from);
            for (key, reading) in stored.readings {
                let field = Field::from_key(&key);
                let via = via_from_key(&reading.via);
                match (field, via) {
                    (Some(field), Some(via)) => {
                        let (value, at) = (reading.value, reading.at);
                        device.readings.insert(field, Reading { value, at, via });
                    }
                    _ => eprintln!(
                        "scent-control: ignoring saved reading {key:?} of {}: unknown field or source",
                        stored.id
                    ),
                }
            }
            state.devices.insert(stored.id, device);
        }
        let mut broker = BrokerSettings::from(self.broker);
        // A hand-edited address that can't be used would otherwise surface later as a
        // missing password. The default is no worse than none; who we are on the broker
        // stays.
        let notice = BrokerUrl::parse(&broker.url).err().map(|error| {
            eprintln!("scent-control: the saved broker address is not usable: {error}");
            broker.url = BrokerSettings::default().url;
            BAD_BROKER_ADDRESS.to_owned()
        });
        Loaded {
            state,
            broker,
            notice,
        }
    }
}

impl From<&DeviceInfo> for StoredInfo {
    fn from(info: &DeviceInfo) -> StoredInfo {
        StoredInfo {
            manufacturer_id: info.manufacturer_id.clone(),
            model_name: info.model_name.clone(),
            variant_name: info.variant_name.clone(),
            device_id: info.device_id.clone(),
            firmware: info.firmware.clone(),
            wifi_configured: info.wifi_configured,
            wifi_status: info.wifi_status,
        }
    }
}

impl From<StoredInfo> for DeviceInfo {
    fn from(info: StoredInfo) -> DeviceInfo {
        DeviceInfo {
            manufacturer_id: info.manufacturer_id,
            model_name: info.model_name,
            variant_name: info.variant_name,
            device_id: info.device_id,
            firmware: info.firmware,
            wifi_configured: info.wifi_configured,
            wifi_status: info.wifi_status,
        }
    }
}

impl From<&Reading> for StoredReading {
    fn from(reading: &Reading) -> StoredReading {
        StoredReading {
            value: reading.value,
            at: reading.at,
            via: via_key(reading.via).to_owned(),
        }
    }
}

fn via_key(via: Via) -> &'static str {
    match via {
        Via::Bluetooth => "bluetooth",
        Via::Wifi => "wifi",
    }
}

fn via_from_key(key: &str) -> Option<Via> {
    [Via::Bluetooth, Via::Wifi]
        .into_iter()
        .find(|&via| via_key(via) == key)
}

/// A folder under the system temp folder that is removed when this goes away.
#[cfg(test)]
pub(crate) struct TempDir(PathBuf);

#[cfg(test)]
impl TempDir {
    /// `tag` must be different for each test, as tests run in parallel.
    pub(crate) fn new(tag: &str) -> TempDir {
        let path =
            std::env::temp_dir().join(format!("scent-control-test-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        &self.0
    }
}

#[cfg(test)]
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use scent_core::{Command, Secret, Via};

    use super::*;
    use crate::state::{Event, Field};

    const ID1: &str = "TESTDEVICE01";

    fn store(dir: &TempDir) -> Store {
        Store {
            dir: dir.path().to_owned(),
        }
    }

    #[test]
    fn default_dir_is_scent_control_in_application_support() {
        let dir = Store::default_dir();
        assert!(
            dir.ends_with("Library/Application Support/ScentControl"),
            "{dir:?}"
        );
    }

    #[test]
    fn a_missing_file_gives_an_empty_state() {
        let dir = TempDir::new("missing");
        assert_eq!(store(&dir).load().state, AppState::default());
        // Not even the folder exists yet.
        let nested = Store {
            dir: dir.path().join("not/there"),
        };
        assert_eq!(nested.load().state, AppState::default());
    }

    #[test]
    fn save_creates_the_folder_and_leaves_no_temp_file() {
        let dir = TempDir::new("save");
        let nested = Store {
            dir: dir.path().join("a/b"),
        };
        nested
            .save(&AppState::default(), &BrokerSettings::default())
            .unwrap();
        nested
            .save(&AppState::default(), &BrokerSettings::default())
            .unwrap();
        let names: Vec<String> = fs::read_dir(&nested.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["desktop.json"]);
    }

    #[test]
    fn the_file_holds_devices_and_nothing_secret() {
        let dir = TempDir::new("secrets");
        let mut s = AppState::default();
        s.apply_at(
            Event::BleInfo {
                ble_id: "ble-1".into(),
                info: scent_core::DeviceInfo {
                    manufacturer_id: "01".into(),
                    model_name: "Diffuser".into(),
                    variant_name: "Black".into(),
                    device_id: ID1.into(),
                    firmware: "2.30".into(),
                    wifi_configured: true,
                    wifi_status: 3,
                },
                at: 1,
            },
            2,
        );
        let wifi = Command::WifiConnect {
            ssid: "ExampleWiFi".into(),
            password: Secret::new("hunter2!".into()),
        };
        // The echo of our own command, and what was reported about sending it.
        s.apply_at(
            Event::Mqtt {
                topic: "scentiment-mqtt-prod/device/TESTDEVICE01".into(),
                body: wifi.mqtt_body(),
                at: 3,
            },
            4,
        );
        s.apply_at(
            Event::Sent {
                device_id: ID1.into(),
                summary: wifi.redacted(Via::Bluetooth),
                via: Via::Bluetooth,
                at: 5,
                power: None,
                intensity: None,
                profile: None,
            },
            6,
        );
        store(&dir).save(&s, &BrokerSettings::default()).unwrap();

        let text = fs::read_to_string(dir.path().join("desktop.json")).unwrap();
        assert!(
            !text.contains("hunter2!") && !text.contains("password"),
            "{text}"
        );
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        keys.sort();
        // The devices, and where the broker is. The password is neither.
        assert_eq!(keys, ["broker", "devices"]);
    }

    fn custom_broker() -> BrokerSettings {
        BrokerSettings {
            url: "wss://broker.example.test:8443/mqtt".into(),
            username: "someone".into(),
            route_id: "route-9".into(),
        }
    }

    #[test]
    fn the_broker_settings_round_trip_and_the_password_is_not_among_them() {
        let dir = TempDir::new("broker-round-trip");
        store(&dir)
            .save(&AppState::default(), &custom_broker())
            .unwrap();

        let loaded = store(&dir).load();
        assert_eq!(loaded.broker, custom_broker());
        assert_eq!(loaded.notice, None);

        let text = fs::read_to_string(dir.path().join("desktop.json")).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut keys: Vec<&String> = json["broker"].as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(keys, ["route_id", "url", "username"]);
        assert!(!text.to_lowercase().contains("password"), "{text}");
    }

    #[test]
    fn a_file_with_no_broker_object_gives_the_default_settings() {
        // Written before the settings were kept, or by hand.
        let dir = TempDir::new("no-broker");
        fs::write(dir.path().join("desktop.json"), r#"{"devices":[]}"#).unwrap();
        let loaded = store(&dir).load();
        assert_eq!(loaded.broker, BrokerSettings::default());
        // Not damaged: nothing to tell the person, and the file stays where it is.
        assert_eq!(loaded.notice, None);
        assert!(dir.path().join("desktop.json").exists());
        assert!(!dir.path().join("desktop.json.corrupt").exists());
    }

    #[test]
    fn a_broker_object_with_gaps_is_filled_from_the_defaults() {
        let dir = TempDir::new("partial-broker");
        fs::write(
            dir.path().join("desktop.json"),
            r#"{"devices":[],"broker":{"username":"someone"}}"#,
        )
        .unwrap();
        let loaded = store(&dir).load();
        let defaults = BrokerSettings::default();
        assert_eq!(loaded.broker.username, "someone");
        assert_eq!(loaded.broker.url, defaults.url);
        assert_eq!(loaded.broker.route_id, defaults.route_id);
        assert_eq!(loaded.notice, None);
    }

    #[test]
    fn a_saved_broker_address_that_is_not_valid_is_replaced_and_the_notice_says_so() {
        for bad in [
            "ws://broker.example/mqtt",
            "",
            "wss://bad host/mqtt",
            "wss://x:0/mqtt",
        ] {
            let dir = TempDir::new("bad-broker-address");
            let file = format!(
                r#"{{"devices":[],"broker":{{"url":{bad:?},"username":"someone","route_id":"route-9"}}}}"#
            );
            fs::write(dir.path().join("desktop.json"), file).unwrap();
            let loaded = store(&dir).load();
            // The address goes back to the default; who we are on the broker stays.
            let defaults = BrokerSettings::default();
            assert_eq!(loaded.broker.url, defaults.url, "{bad:?}");
            assert_eq!(loaded.broker.username, "someone", "{bad:?}");
            assert_eq!(loaded.broker.route_id, "route-9", "{bad:?}");
            let notice = loaded.notice.expect("a notice");
            assert!(notice.contains("broker address"), "{notice}");
            assert!(notice.contains("Settings"), "{notice}");
            // The devices are fine and the file stays where it is.
            assert!(dir.path().join("desktop.json").exists());
            assert!(!dir.path().join("desktop.json.corrupt").exists());
        }
    }

    #[test]
    fn what_is_written_changes_only_when_what_is_saved_changes() {
        let broker = BrokerSettings::default();
        let mut s = AppState::default();
        s.apply_at(
            Event::Mqtt {
                topic: "scentiment-mqtt-prod/user/route-test-1".into(),
                body: br#"{"action":"update_status","payload":{"deviceId":"TESTDEVICE01","data":"S:1,L:2"}}"#.to_vec(),
                at: 10,
            },
            10,
        );
        let before = Store::contents(&s, &broker).unwrap();
        assert_eq!(Store::contents(&s.clone(), &broker).unwrap(), before);

        // Live things the file doesn't hold.
        let device = s.devices.get_mut(ID1).unwrap();
        device.ping_at = Some(99);
        device.heard_wifi = Some(99);
        device.heard_ble = Some(99);
        s.apply_at(Event::BrokerConnected, 99);
        s.apply_at(
            Event::Pinged {
                device_id: ID1.into(),
                at: 100,
            },
            100,
        );
        assert_eq!(Store::contents(&s, &broker).unwrap(), before);

        // A reading, a name or the broker's address does change it.
        let mut changed = s.clone();
        changed
            .devices
            .get_mut(ID1)
            .unwrap()
            .readings
            .get_mut(&Field::Power)
            .unwrap()
            .value = 0;
        assert_ne!(Store::contents(&changed, &broker).unwrap(), before);
        let mut renamed = s.clone();
        renamed.devices.get_mut(ID1).unwrap().label = "Bedroom".into();
        assert_ne!(Store::contents(&renamed, &broker).unwrap(), before);
        let elsewhere = BrokerSettings {
            username: "someone".into(),
            ..BrokerSettings::default()
        };
        assert_ne!(Store::contents(&s, &elsewhere).unwrap(), before);
    }

    #[test]
    fn a_damaged_file_comes_with_a_notice_that_names_where_it_went() {
        let dir = TempDir::new("notice");
        fs::write(dir.path().join("desktop.json"), "{").unwrap();
        let loaded = store(&dir).load();
        assert_eq!(loaded.state, AppState::default());
        assert_eq!(loaded.broker, BrokerSettings::default());
        let notice = loaded.notice.expect("a notice");
        assert!(notice.contains("desktop.json.corrupt"), "{notice}");

        // No file yet is not damage; neither is a good one.
        let fresh = TempDir::new("no-notice");
        assert_eq!(store(&fresh).load().notice, None);
        store(&fresh)
            .save(&AppState::default(), &BrokerSettings::default())
            .unwrap();
        assert_eq!(store(&fresh).load().notice, None);
    }

    #[test]
    fn loading_skips_what_it_cannot_use_and_keeps_the_rest() {
        let dir = TempDir::new("tolerant");
        fs::write(
            dir.path().join("desktop.json"),
            r#"{"devices":[
                {"id":"TESTDEVICE01","label":"Bedroom","readings":{
                    "power":{"value":1,"at":10,"via":"wifi"},
                    "oil":{"value":9,"at":10,"via":"wifi"},
                    "battery":{"value":80,"at":10,"via":"carrier pigeon"}}},
                {"id":"not an id","label":"x"}
            ]}"#,
        )
        .unwrap();
        let loaded = store(&dir).load().state;
        assert_eq!(loaded.devices.keys().collect::<Vec<_>>(), [ID1]);
        let d = &loaded.devices[ID1];
        assert_eq!(d.label, "Bedroom");
        assert_eq!(d.readings.keys().collect::<Vec<_>>(), [&Field::Power]);
        assert_eq!(d.ble_id, None);
    }

    #[test]
    fn a_file_that_does_not_fit_the_format_is_treated_as_corrupt() {
        for bad in ["", "[]", r#"{"devices":5}"#, "\u{0}\u{1}"] {
            let dir = TempDir::new("bad-shape");
            fs::write(dir.path().join("desktop.json"), bad).unwrap();
            assert_eq!(store(&dir).load().state, AppState::default(), "{bad:?}");
        }
        let dir = TempDir::new("bad-bytes");
        fs::write(dir.path().join("desktop.json"), [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(store(&dir).load().state, AppState::default());
    }
}
