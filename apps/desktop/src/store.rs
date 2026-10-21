//! The app's memory between runs: the diffusers it knows, their names, their last
//! readings and the last schedule this Mac sent each, where the broker is, and how the
//! app looks, in `desktop.json`.
//!
//! The file holds devices, the broker's address and the appearance and nothing else: no
//! log, no live status, and no secrets (the broker password lives in the Keychain, and
//! WiFi passwords are never kept; a schedule is times, days and levels).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;

use scent_core::{
    AlwaysOn, DeviceInfo, Entry, HhMm, Level, Profile, ScheduleFlag, Via, valid_device_id,
};
use serde::{Deserialize, Serialize};

use crate::mqtt::{BrokerSettings, BrokerUrl};
use crate::state::{AppState, DeviceView, Field, Reading};
use crate::ui::theme::Appearance;

const FILE_NAME: &str = "desktop.json";
const TEMP_NAME: &str = "desktop.json.tmp";
/// A file we could not read is moved here so the next save doesn't overwrite it.
const CORRUPT_NAME: &str = "desktop.json.corrupt";

const BAD_BROKER_ADDRESS: &str =
    "The saved broker address wasn't valid, so the default is used. Check it in Settings.";

/// [`BAD_BROKER_ADDRESS`] for a build given no broker, which has no default to use.
const BAD_BROKER_ADDRESS_NO_DEFAULT: &str =
    "The saved broker address wasn't valid, so no broker is set. Add it in Settings.";

pub struct Store {
    pub dir: PathBuf,
}

/// What [`Store::load`] found.
#[derive(Debug)]
pub struct Loaded {
    pub state: AppState,
    /// The broker saved, or the build's whole default (see [`BrokerSettings::default`])
    /// when the file doesn't say or saved an empty address. That default may be no broker.
    pub broker: BrokerSettings,
    /// How the app looks, as chosen in Settings: the system's when the file doesn't say.
    pub appearance: Appearance,
    /// Set when there was a file and it could not be used, for the screens to show.
    pub notice: Option<String>,
    /// True when there was no file at all: the app has never saved anything here.
    pub first_run: bool,
}

impl Loaded {
    fn fresh(notice: Option<String>) -> Loaded {
        Loaded {
            state: AppState::default(),
            broker: BrokerSettings::default(),
            appearance: Appearance::System,
            notice,
            first_run: false,
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

    /// What was saved, or an empty state and the build's default settings when there is
    /// nothing usable: no file yet, a file we can't read, or one that isn't what we
    /// wrote. The problem goes to stderr. A file that is damaged is kept as
    /// `desktop.json.corrupt`, and the notice says so. No file yet is not a problem.
    pub fn load(&self) -> Loaded {
        let path = self.path();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                eprintln!("scent-control: no saved state at {} yet", path.display());
                return Loaded {
                    first_run: true,
                    ..Loaded::fresh(None)
                };
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

    /// What [`write`](Self::write) writes for this state: the devices, the broker
    /// settings and the appearance, as JSON. Only what the file holds is in it, so two states give the
    /// same bytes exactly when what would be saved is the same.
    pub fn contents(
        state: &AppState,
        broker: &BrokerSettings,
        appearance: Appearance,
    ) -> io::Result<Vec<u8>> {
        serde_json::to_vec_pretty(&Stored::from_state(state, broker, appearance))
            .map_err(io::Error::other)
    }

    /// Writes the devices and the broker settings, with the system's appearance (see
    /// [`contents`](Self::contents)).
    /// The app writes through `contents` and `write` instead, so as to leave the file
    /// alone when it would not change.
    #[cfg(test)]
    pub fn save(&self, state: &AppState, broker: &BrokerSettings) -> io::Result<()> {
        self.write(&Self::contents(state, broker, Appearance::System)?)
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
    /// How the app looks. Left out while it follows the system, and missing from files
    /// written before it was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ui: Option<StoredUi>,
}

/// The app's own settings, apart from the broker's.
#[derive(Serialize, Deserialize)]
struct StoredUi {
    /// `"light"` or `"dark"`. Missing, or one the app doesn't know, means the system's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    appearance: Option<String>,
}

impl StoredUi {
    /// `None` for the system's appearance, so that the file leaves `ui` out.
    fn for_appearance(appearance: Appearance) -> Option<StoredUi> {
        (appearance != Appearance::System).then(|| StoredUi {
            appearance: Some(appearance_key(appearance).to_owned()),
        })
    }

    /// The appearance saved in `ui`, or the system's.
    fn appearance(ui: Option<StoredUi>) -> Appearance {
        let Some(key) = ui.and_then(|ui| ui.appearance) else {
            return Appearance::System;
        };
        appearance_from_key(&key).unwrap_or_else(|| {
            eprintln!("scent-control: ignoring the saved appearance {key:?}: not one we know");
            Appearance::System
        })
    }
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
    /// The room it is in, as the person named it. Left out when it is in none, and
    /// missing from files written before rooms were kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    room: Option<String>,
    #[serde(default)]
    ble_id: Option<String>,
    #[serde(default)]
    info: Option<StoredInfo>,
    #[serde(default)]
    readings: BTreeMap<String, StoredReading>,
    /// The last schedule this Mac sent it: what the screens show for today and the
    /// editor opens from. Left out when none was sent, and missing from files written
    /// before it was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sent_profile: Option<StoredProfile>,
}

/// A schedule as the editor shows it: times as `HH:MM`, days from 0 (Sunday) to 6,
/// levels from 1 to 3, and the zone's offset from UTC in hours. Nothing in it is secret.
#[derive(Serialize, Deserialize)]
struct StoredProfile {
    always_on: StoredAlwaysOn,
    entries: Vec<StoredEntry>,
    timezone_hours: f64,
    /// What the last `UPDATE_PROFILE` sent: `"replace"` or `"always_on_only"`.
    flag: String,
}

#[derive(Serialize, Deserialize)]
struct StoredAlwaysOn {
    enable: bool,
    level: u8,
}

#[derive(Serialize, Deserialize)]
struct StoredEntry {
    start: String,
    end: String,
    weekdays: Vec<u8>,
    level: u8,
    enable: bool,
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
    fn from_state(state: &AppState, broker: &BrokerSettings, appearance: Appearance) -> Stored {
        Stored {
            broker: broker.into(),
            ui: StoredUi::for_appearance(appearance),
            devices: state
                .devices
                .values()
                .map(|device| StoredDevice {
                    id: device.id.clone(),
                    label: device.label.clone(),
                    room: device.room.clone(),
                    ble_id: device.ble_id.clone(),
                    info: device.info.as_ref().map(StoredInfo::from),
                    readings: device
                        .readings
                        .iter()
                        .map(|(field, reading)| (field.key().to_owned(), reading.into()))
                        .collect(),
                    // A zone that isn't a number would be written as `null`, which would
                    // make the whole file unreadable. The editor never sends one.
                    sent_profile: device
                        .sent_profile
                        .as_ref()
                        .filter(|profile| profile.timezone_hours.is_finite())
                        .map(StoredProfile::from),
                })
                .collect(),
        }
    }

    /// Anything that can't be used is left out, with a note on stderr: a device
    /// whose ID is not a device ID, a reading of a field or source we don't know, a
    /// schedule with a time, day, level or flag that can't be.
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
            // Tidied as the app tidies what is typed, as it may have been typed here.
            device.room = stored
                .room
                .map(|room| crate::state::clean_name(&room))
                .filter(|room| !room.is_empty());
            device.ble_id = stored.ble_id;
            device.info = stored.info.map(DeviceInfo::from);
            device.sent_profile = stored.sent_profile.and_then(|profile| {
                let profile = profile.into_profile();
                if profile.is_none() {
                    eprintln!(
                        "scent-control: ignoring the saved schedule of {}: it can't be used",
                        stored.id
                    );
                }
                profile
            });
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
        let (broker, notice) =
            resolve_broker(BrokerSettings::from(self.broker), BrokerSettings::default());
        Loaded {
            state,
            broker,
            appearance: StoredUi::appearance(self.ui),
            notice,
            first_run: false,
        }
    }
}

/// The broker to use from the one saved, given the build's `default`, and a notice if
/// the saved one had to be replaced.
///
/// An empty saved address means none was saved: a build given no broker wrote the file
/// before one was added in Settings. Then the whole of the build's default is used, which
/// may be none, as the username and route ID saved with it belong to no broker. A saved
/// address that can't be used (edited by hand) would otherwise surface later as a missing
/// password: it is replaced by the default's, and who we are on the broker stays.
fn resolve_broker(
    mut broker: BrokerSettings,
    default: BrokerSettings,
) -> (BrokerSettings, Option<String>) {
    if !broker.is_set() {
        return (default, None);
    }
    let Err(error) = BrokerUrl::parse(&broker.url) else {
        return (broker, None);
    };
    eprintln!("scent-control: the saved broker address is not usable: {error}");
    let notice = if default.is_set() {
        BAD_BROKER_ADDRESS
    } else {
        BAD_BROKER_ADDRESS_NO_DEFAULT
    };
    broker.url = default.url;
    (broker, Some(notice.to_owned()))
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

impl From<&Profile> for StoredProfile {
    fn from(profile: &Profile) -> StoredProfile {
        StoredProfile {
            always_on: StoredAlwaysOn {
                enable: profile.always_on.enable,
                level: profile.always_on.level.get(),
            },
            entries: profile
                .entries
                .iter()
                .map(|entry| StoredEntry {
                    start: entry.start.to_string(),
                    end: entry.end.to_string(),
                    weekdays: entry.weekdays.clone(),
                    level: entry.level.get(),
                    enable: entry.enable,
                })
                .collect(),
            timezone_hours: profile.timezone_hours,
            flag: flag_key(profile.flag).to_owned(),
        }
    }
}

impl StoredProfile {
    /// The schedule, or `None` if any part of it can't be.
    fn into_profile(self) -> Option<Profile> {
        let entries = self
            .entries
            .into_iter()
            .map(|entry| {
                if entry.weekdays.iter().any(|&day| day > 6) {
                    return None;
                }
                Some(Entry {
                    start: HhMm::parse(&entry.start)?,
                    end: HhMm::parse(&entry.end)?,
                    weekdays: entry.weekdays,
                    level: Level::new(entry.level)?,
                    enable: entry.enable,
                })
            })
            .collect::<Option<Vec<Entry>>>()?;
        Some(Profile {
            always_on: AlwaysOn {
                enable: self.always_on.enable,
                level: Level::new(self.always_on.level)?,
            },
            entries,
            timezone_hours: Some(self.timezone_hours).filter(|hours| hours.is_finite())?,
            flag: flag_from_key(&self.flag)?,
        })
    }
}

fn flag_key(flag: ScheduleFlag) -> &'static str {
    match flag {
        ScheduleFlag::Replace => "replace",
        ScheduleFlag::AlwaysOnOnly => "always_on_only",
    }
}

fn flag_from_key(key: &str) -> Option<ScheduleFlag> {
    [ScheduleFlag::Replace, ScheduleFlag::AlwaysOnOnly]
        .into_iter()
        .find(|&flag| flag_key(flag) == key)
}

fn appearance_key(appearance: Appearance) -> &'static str {
    match appearance {
        Appearance::System => "system",
        Appearance::Light => "light",
        Appearance::Dark => "dark",
    }
}

fn appearance_from_key(key: &str) -> Option<Appearance> {
    [Appearance::System, Appearance::Light, Appearance::Dark]
        .into_iter()
        .find(|&appearance| appearance_key(appearance) == key)
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

/// A schedule as the editor sends it: two times, one switched off, always on at
/// High, half an hour off a whole zone.
#[cfg(test)]
pub(crate) fn sent_schedule() -> scent_core::Profile {
    use scent_core::{AlwaysOn, Entry, HhMm, Level, Profile, ScheduleFlag};
    let time = |start: &str, end: &str, weekdays: Vec<u8>, level: u8, enable: bool| Entry {
        start: HhMm::parse(start).unwrap(),
        end: HhMm::parse(end).unwrap(),
        weekdays,
        level: Level::new(level).unwrap(),
        enable,
    };
    Profile {
        always_on: AlwaysOn {
            enable: true,
            level: Level::new(3).unwrap(),
        },
        entries: vec![
            time("07:00", "09:30", vec![1, 2, 3, 4, 5], 2, true),
            time("21:15", "23:59", vec![0, 6], 1, false),
        ],
        timezone_hours: 5.5,
        flag: ScheduleFlag::Replace,
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
    fn only_a_missing_file_is_the_first_run() {
        let dir = TempDir::new("first-run");
        assert!(store(&dir).load().first_run);
        let nested = Store {
            dir: dir.path().join("not/there"),
        };
        assert!(nested.load().first_run);
        // Once anything is saved, it isn't.
        store(&dir)
            .save(&AppState::default(), &BrokerSettings::default())
            .unwrap();
        assert!(!store(&dir).load().first_run);
        // Nor is a damaged file: something was saved before.
        let damaged = TempDir::new("first-run-damaged");
        fs::write(damaged.path().join("desktop.json"), "{").unwrap();
        assert!(!store(&damaged).load().first_run);
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

    #[test]
    fn the_last_schedule_sent_is_saved_with_its_diffuser() {
        let dir = TempDir::new("schedule-round-trip");
        let mut s = AppState::default();
        let mut one = DeviceView::new(ID1);
        one.sent_profile = Some(sent_schedule());
        s.devices.insert(ID1.to_owned(), one);
        // Only the always-on entry went out last; the times kept are the ones before.
        let mut two = DeviceView::new("TESTDEVICE02");
        two.sent_profile = Some(scent_core::Profile {
            flag: scent_core::ScheduleFlag::AlwaysOnOnly,
            ..sent_schedule()
        });
        s.devices.insert("TESTDEVICE02".to_owned(), two);
        s.devices
            .insert("TESTDEVICE03".to_owned(), DeviceView::new("TESTDEVICE03"));
        store(&dir).save(&s, &BrokerSettings::default()).unwrap();

        let loaded = store(&dir).load();
        assert_eq!(loaded.notice, None);
        assert_eq!(loaded.state.devices, s.devices);
        // It is saved only once there is one, and holds nothing secret.
        let text = fs::read_to_string(dir.path().join("desktop.json")).unwrap();
        assert_eq!(text.matches("sent_profile").count(), 2, "{text}");
        assert!(!text.to_lowercase().contains("password"), "{text}");
    }

    #[test]
    fn a_file_from_before_schedules_were_saved_loads_cleanly() {
        let dir = TempDir::new("no-schedule");
        fs::write(
            dir.path().join("desktop.json"),
            r#"{"devices":[{"id":"TESTDEVICE01","label":"Bedroom","ble_id":"ble-1"}]}"#,
        )
        .unwrap();
        let loaded = store(&dir).load();
        assert_eq!(loaded.notice, None);
        let device = &loaded.state.devices[ID1];
        assert_eq!(device.label, "Bedroom");
        assert_eq!(device.sent_profile, None);
        assert!(dir.path().join("desktop.json").exists());
        assert!(!dir.path().join("desktop.json.corrupt").exists());
    }

    #[test]
    fn a_room_is_saved_with_its_diffuser_and_left_out_when_it_has_none() {
        let dir = TempDir::new("room-round-trip");
        let mut s = AppState::default();
        let mut one = DeviceView::new(ID1);
        one.label = "Bedside".to_owned();
        one.room = Some("Bedroom".to_owned());
        s.devices.insert(ID1.to_owned(), one);
        s.devices
            .insert("TESTDEVICE02".to_owned(), DeviceView::new("TESTDEVICE02"));
        store(&dir).save(&s, &BrokerSettings::default()).unwrap();

        let loaded = store(&dir).load();
        assert_eq!(loaded.notice, None);
        assert_eq!(loaded.state.devices, s.devices);
        let text = fs::read_to_string(dir.path().join("desktop.json")).unwrap();
        assert_eq!(text.matches("\"room\"").count(), 1, "{text}");
    }

    #[test]
    fn a_file_from_before_rooms_were_saved_loads_cleanly() {
        let dir = TempDir::new("no-room");
        fs::write(
            dir.path().join("desktop.json"),
            r#"{"devices":[{"id":"TESTDEVICE01","label":"Bedside","ble_id":"ble-1"}]}"#,
        )
        .unwrap();
        let loaded = store(&dir).load();
        assert_eq!(loaded.notice, None);
        let device = &loaded.state.devices[ID1];
        assert_eq!(device.label, "Bedside");
        assert_eq!(device.room, None);
        assert!(dir.path().join("desktop.json").exists());
        assert!(!dir.path().join("desktop.json.corrupt").exists());
    }

    #[test]
    fn a_room_typed_into_the_file_is_tidied_as_the_app_tidies_it() {
        for (typed, room) in [
            ("  Living   Room ", Some("Living Room")),
            ("   ", None),
            ("", None),
        ] {
            let dir = TempDir::new("hand-typed-room");
            let file = format!(
                r#"{{"devices":[{{"id":"TESTDEVICE01","label":"Bedside","room":{typed:?}}}]}}"#
            );
            fs::write(dir.path().join("desktop.json"), file).unwrap();
            let loaded = store(&dir).load();
            assert_eq!(loaded.notice, None, "{typed:?}");
            assert_eq!(loaded.state.devices[ID1].room.as_deref(), room, "{typed:?}");
        }
    }

    #[test]
    fn a_saved_schedule_that_cannot_be_used_is_left_out() {
        let good = r#"{"start":"07:00","end":"09:00","weekdays":[1],"level":2,"enable":true}"#;
        for bad in [
            r#"{"start":"7:00","end":"09:00","weekdays":[1],"level":2,"enable":true}"#,
            r#"{"start":"07:00","end":"24:00","weekdays":[1],"level":2,"enable":true}"#,
            r#"{"start":"07:00","end":"09:00","weekdays":[7],"level":2,"enable":true}"#,
            r#"{"start":"07:00","end":"09:00","weekdays":[1],"level":4,"enable":true}"#,
        ] {
            let dir = TempDir::new("bad-schedule");
            let file = format!(
                r#"{{"devices":[{{"id":"TESTDEVICE01","label":"Bedroom","sent_profile":
                    {{"always_on":{{"enable":false,"level":1}},"entries":[{good},{bad}],
                      "timezone_hours":-5.0,"flag":"replace"}}}}]}}"#
            );
            fs::write(dir.path().join("desktop.json"), file).unwrap();
            let loaded = store(&dir).load();
            // The diffuser is kept, without the schedule; the file is not damaged.
            assert_eq!(loaded.notice, None, "{bad}");
            let device = &loaded.state.devices[ID1];
            assert_eq!(device.label, "Bedroom", "{bad}");
            assert_eq!(device.sent_profile, None, "{bad}");
            assert!(!dir.path().join("desktop.json.corrupt").exists(), "{bad}");
        }
        // So is one whose always-on level or flag can't be.
        for (level, flag) in [(0, "replace"), (1, "sometimes")] {
            let dir = TempDir::new("bad-schedule-head");
            let file = format!(
                r#"{{"devices":[{{"id":"TESTDEVICE01","label":"Bedroom","sent_profile":
                    {{"always_on":{{"enable":false,"level":{level}}},"entries":[{good}],
                      "timezone_hours":-5.0,"flag":"{flag}"}}}}]}}"#
            );
            fs::write(dir.path().join("desktop.json"), file).unwrap();
            let loaded = store(&dir).load();
            assert_eq!(loaded.notice, None, "{level} {flag}");
            let device = &loaded.state.devices[ID1];
            assert_eq!(device.sent_profile, None, "{level} {flag}");
        }
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
        // The gaps come from this build's default. Without a default address that leaves
        // the address empty, which is no broker saved: the whole default, none, is used.
        let filled = BrokerSettings {
            username: "someone".to_owned(),
            ..BrokerSettings::default()
        };
        let expected = if filled.is_set() {
            filled
        } else {
            BrokerSettings::default()
        };
        assert_eq!(loaded.broker, expected);
        assert_eq!(loaded.notice, None);
    }

    #[test]
    fn the_appearance_is_saved_under_ui_and_left_out_while_it_follows_the_system() {
        let broker = BrokerSettings::default();
        for (appearance, key) in [
            (Appearance::Light, Some("light")),
            (Appearance::Dark, Some("dark")),
            (Appearance::System, None),
        ] {
            let dir = TempDir::new("appearance-round-trip");
            let json = Store::contents(&AppState::default(), &broker, appearance).unwrap();
            store(&dir).write(&json).unwrap();

            let loaded = store(&dir).load();
            assert_eq!(loaded.appearance, appearance);
            assert_eq!(loaded.notice, None, "{appearance:?}");

            let text = fs::read_to_string(dir.path().join("desktop.json")).unwrap();
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(json["ui"]["appearance"].as_str(), key, "{text}");
            assert_eq!(json.get("ui").is_some(), key.is_some(), "{text}");
        }
    }

    #[test]
    fn a_file_from_before_the_appearance_was_saved_follows_the_system() {
        let dir = TempDir::new("no-ui");
        fs::write(
            dir.path().join("desktop.json"),
            r#"{"devices":[{"id":"TESTDEVICE01","label":"Bedroom"}],"broker":{"url":"wss://broker.example.test/mqtt","username":"someone"}}"#,
        )
        .unwrap();
        let loaded = store(&dir).load();
        assert_eq!(loaded.appearance, Appearance::System);
        // Not damaged: the rest is as it was, nothing to tell, and the file stays.
        assert_eq!(loaded.notice, None);
        assert_eq!(loaded.state.devices[ID1].label, "Bedroom");
        assert_eq!(loaded.broker.username, "someone");
        assert!(dir.path().join("desktop.json").exists());
        assert!(!dir.path().join("desktop.json.corrupt").exists());
    }

    #[test]
    fn an_appearance_the_app_does_not_know_follows_the_system() {
        for ui in [
            "{}",
            r#"{"appearance":null}"#,
            r#"{"appearance":"sepia"}"#,
            r#"{"appearance":"dark","later":1}"#,
        ] {
            let dir = TempDir::new("odd-ui");
            let file = format!(r#"{{"devices":[],"ui":{ui}}}"#);
            fs::write(dir.path().join("desktop.json"), file).unwrap();
            let loaded = store(&dir).load();
            let expected = if ui.contains("dark") {
                Appearance::Dark
            } else {
                Appearance::System
            };
            assert_eq!(loaded.appearance, expected, "{ui}");
            assert_eq!(loaded.notice, None, "{ui}");
            assert!(!dir.path().join("desktop.json.corrupt").exists(), "{ui}");
        }
    }

    #[test]
    fn a_saved_broker_address_that_is_not_valid_is_replaced_and_the_notice_says_so() {
        for bad in [
            "ws://broker.example/mqtt",
            "wss://bad host/mqtt",
            "wss://x:0/mqtt",
        ] {
            let dir = TempDir::new("bad-broker-address");
            let file = format!(
                r#"{{"devices":[],"broker":{{"url":{bad:?},"username":"someone","route_id":"route-9"}}}}"#
            );
            fs::write(dir.path().join("desktop.json"), file).unwrap();
            let loaded = store(&dir).load();
            // The address goes back to the build's default, which may be none; who we are
            // on the broker stays.
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
    fn an_empty_saved_broker_address_is_not_saved_and_gets_this_builds_default() {
        // Written by a build that was given no broker, before one was added in Settings:
        // the empty address alone, and an empty address with a username.
        for broker in [
            r#"{"url":"","username":"","route_id":""}"#,
            r#"{"url":"  ","username":"someone","route_id":"route-9"}"#,
        ] {
            let dir = TempDir::new("empty-broker-address");
            let file = format!(
                r#"{{"devices":[{{"id":"TESTDEVICE01","label":"Bedroom"}}],"broker":{broker}}}"#
            );
            fs::write(dir.path().join("desktop.json"), file).unwrap();
            let loaded = store(&dir).load();
            // The whole of this build's default, whatever the rest of the object says.
            assert_eq!(loaded.broker, BrokerSettings::default(), "{broker}");
            // Nothing wrong with the file: no notice, and it stays where it is.
            assert_eq!(loaded.notice, None, "{broker}");
            assert_eq!(loaded.state.devices[ID1].label, "Bedroom", "{broker}");
            assert!(!dir.path().join("desktop.json.corrupt").exists());
        }
    }

    #[test]
    fn a_file_saved_by_a_build_with_no_broker_loads_cleanly_with_this_builds_default() {
        let dir = TempDir::new("no-broker-round-trip");
        let none = BrokerSettings::from_values(None, None, None);
        store(&dir).save(&AppState::default(), &none).unwrap();
        let loaded = store(&dir).load();
        assert_eq!(loaded.broker, BrokerSettings::default());
        assert_eq!(loaded.notice, None);
    }

    /// No broker, as a build given none has: an empty address, and nothing else either.
    fn none() -> BrokerSettings {
        BrokerSettings::from_values(None, None, None)
    }

    #[test]
    fn an_empty_saved_broker_takes_the_whole_default_it_is_given() {
        // The empty address alone, and an empty address with a username and route ID.
        let alone = none();
        let with_a_name = BrokerSettings {
            url: " ".to_owned(),
            username: "someone".to_owned(),
            route_id: "route-9".to_owned(),
        };
        for saved in [alone, with_a_name] {
            // A build with a broker: all of it, not the address alone, so the Keychain
            // account, the pings and Settings have its username and route ID too.
            let example = BrokerSettings::example();
            assert_eq!(
                resolve_broker(saved.clone(), example.clone()),
                (example, None),
                "{saved:?}"
            );
            // A build with none: none at all.
            assert_eq!(
                resolve_broker(saved.clone(), none()),
                (none(), None),
                "{saved:?}"
            );
        }
    }

    #[test]
    fn a_good_saved_broker_is_kept_whatever_the_build_has() {
        let saved = custom_broker();
        for default in [BrokerSettings::example(), none()] {
            assert_eq!(
                resolve_broker(saved.clone(), default),
                (saved.clone(), None)
            );
        }
    }

    #[test]
    fn a_bad_saved_address_says_whether_the_builds_default_took_its_place() {
        let bad = BrokerSettings {
            url: "ws://broker.example/mqtt".to_owned(),
            ..custom_broker()
        };
        // A default to use: it is used, and the notice says so.
        let (broker, notice) = resolve_broker(bad.clone(), BrokerSettings::example());
        assert_eq!(broker.url, BrokerSettings::example().url);
        assert_eq!(broker.username, "someone");
        let notice = notice.expect("a notice");
        assert!(notice.contains("so the default is used"), "{notice}");
        // None: there is no broker now, and the notice doesn't claim a default.
        let (broker, notice) = resolve_broker(bad, none());
        assert!(!broker.is_set());
        let notice = notice.expect("a notice");
        assert!(!notice.contains("default"), "{notice}");
        assert!(notice.contains("broker address"), "{notice}");
        assert!(notice.contains("Settings"), "{notice}");
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
        let contents = |s: &AppState, broker: &BrokerSettings| {
            Store::contents(s, broker, Appearance::System).unwrap()
        };
        let before = contents(&s, &broker);
        assert_eq!(contents(&s.clone(), &broker), before);

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
        assert_eq!(contents(&s, &broker), before);

        // A reading, a name, a room, a schedule sent or the broker's address does change
        // it.
        let mut changed = s.clone();
        changed
            .devices
            .get_mut(ID1)
            .unwrap()
            .readings
            .get_mut(&Field::Power)
            .unwrap()
            .value = 0;
        assert_ne!(contents(&changed, &broker), before);
        let mut renamed = s.clone();
        renamed.devices.get_mut(ID1).unwrap().label = "Bedroom".into();
        assert_ne!(contents(&renamed, &broker), before);
        let mut moved = s.clone();
        moved.devices.get_mut(ID1).unwrap().room = Some("Bedroom".into());
        assert_ne!(contents(&moved, &broker), before);
        let mut scheduled = s.clone();
        scheduled.devices.get_mut(ID1).unwrap().sent_profile = Some(sent_schedule());
        assert_ne!(contents(&scheduled, &broker), before);
        let elsewhere = BrokerSettings {
            username: "someone".into(),
            ..BrokerSettings::default()
        };
        assert_ne!(contents(&s, &elsewhere), before);
        // So does the appearance chosen in Settings.
        let dark = Store::contents(&s, &broker, Appearance::Dark).unwrap();
        assert_ne!(dark, before);
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
