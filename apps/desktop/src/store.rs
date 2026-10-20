//! The app's memory between runs: the diffusers it knows, their names and their
//! last readings, in `desktop.json`.
//!
//! The file holds devices and nothing else: no log, no live status, and no secrets
//! (the broker password lives in the Keychain, and WiFi passwords are never kept).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;

use scent_core::{DeviceInfo, Via, valid_device_id};
use serde::{Deserialize, Serialize};

use crate::state::{AppState, DeviceView, Field, Reading};

const FILE_NAME: &str = "desktop.json";
const TEMP_NAME: &str = "desktop.json.tmp";
/// A file we could not read is moved here so the next save doesn't overwrite it.
const CORRUPT_NAME: &str = "desktop.json.corrupt";

pub struct Store {
    pub dir: PathBuf,
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

    /// What was saved, or an empty state when there is nothing usable: no file yet,
    /// a file we can't read, or one that isn't what we wrote. The problem goes to
    /// stderr. A file that is damaged is kept as `desktop.json.corrupt`.
    pub fn load(&self) -> AppState {
        let path = self.path();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                eprintln!("scent-control: no saved state at {} yet", path.display());
                return AppState::default();
            }
            Err(error) => {
                eprintln!("scent-control: can't read {}: {error}", path.display());
                return AppState::default();
            }
        };
        match serde_json::from_slice::<Stored>(&bytes) {
            Ok(stored) => stored.into_state(),
            Err(error) => {
                eprintln!(
                    "scent-control: {} is not usable ({error}); starting empty",
                    path.display()
                );
                let aside = self.dir.join(CORRUPT_NAME);
                if let Err(error) = fs::rename(&path, &aside) {
                    eprintln!(
                        "scent-control: can't keep it as {}: {error}",
                        aside.display()
                    );
                }
                AppState::default()
            }
        }
    }

    /// Writes the devices. The file is written whole under another name and then
    /// renamed over the old one, so a crash leaves the old file or the new one.
    pub fn save(&self, state: &AppState) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let json =
            serde_json::to_vec_pretty(&Stored::from_state(state)).map_err(io::Error::other)?;
        let temp = self.dir.join(TEMP_NAME);
        let written = (|| {
            let mut file = File::create(&temp)?;
            file.write_all(&json)?;
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
    fn from_state(state: &AppState) -> Stored {
        Stored {
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
    fn into_state(self) -> AppState {
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
        state
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
        assert_eq!(store(&dir).load(), AppState::default());
        // Not even the folder exists yet.
        let nested = Store {
            dir: dir.path().join("not/there"),
        };
        assert_eq!(nested.load(), AppState::default());
    }

    #[test]
    fn save_creates_the_folder_and_leaves_no_temp_file() {
        let dir = TempDir::new("save");
        let nested = Store {
            dir: dir.path().join("a/b"),
        };
        nested.save(&AppState::default()).unwrap();
        nested.save(&AppState::default()).unwrap();
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
        store(&dir).save(&s).unwrap();

        let text = fs::read_to_string(dir.path().join("desktop.json")).unwrap();
        assert!(
            !text.contains("hunter2!") && !text.contains("password"),
            "{text}"
        );
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["devices"]);
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
        let loaded = store(&dir).load();
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
            assert_eq!(store(&dir).load(), AppState::default(), "{bad:?}");
        }
        let dir = TempDir::new("bad-bytes");
        fs::write(dir.path().join("desktop.json"), [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(store(&dir).load(), AppState::default());
    }
}
