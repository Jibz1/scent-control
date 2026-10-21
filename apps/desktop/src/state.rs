//! What the app knows about its diffusers, and how each event changes it.
//!
//! The workers (Bluetooth, MQTT, the hub) send [`Event`]s; [`AppState::apply`] folds
//! them in; the screens read the result. Nothing here touches Bluetooth, the network
//! or the disk. Times are seconds since 1970, as `i64`.

use std::collections::{BTreeMap, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use scent_core::{
    APP_PREFIX, Ack, Connected, DeviceInfo, Direction, MqttMessage, NoteKind, Online, Profile,
    ScheduleFlag, Status, Via, online_status, parse_mqtt, parse_notification,
    schedule_topic_device, valid_device_id,
};

use crate::add_flow::AddStep;
use crate::shared_schedule::{self, Unreadable};

/// How many log entries the app keeps. The oldest go first.
pub const LOG_CAP: usize = 500;

/// `wifi_status` 3 in the info string: connected (2 or 3 is on firmware above 2.0).
const WIFI_CONNECTED: u8 = 3;

/// The log's label for a message with no `action`.
const NO_ACTION: &str = "(no action)";

/// What the log calls an add's outcome.
const ADD: &str = "Add";

/// What the log says when a diffuser has been added.
const ADDED: &str = "Added: it joined your broker.";

/// What a diffuser says once it has taken `SAVE_MQTT`, over Bluetooth or as the `msg` of
/// an acknowledgement on the broker. Matched in any case.
const CREDENTIALS_SAVED: &str = "credentials saved";

/// What the log calls a schedule read from the broker that changed what the app knew.
pub const SHARED_SCHEDULE: &str = "shared schedule";

/// What the log calls a message under our own prefix that the app can't read.
pub const UNREADABLE_SCHEDULE: &str = "unreadable shared schedule";

/// What the log calls a stored schedule that another version of the app wrote. The
/// reason is kept in the action as it is logged, so the log never reads the message
/// again.
pub const OTHER_VERSION_SCHEDULE: &str = "shared schedule from another version";

/// The most characters a diffuser's name or room may have.
pub const NAME_MAX_CHARS: usize = 40;

/// One value a diffuser reported, with when and over what it arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    pub value: i64,
    pub at: i64,
    pub via: Via,
}

/// What a diffuser can report, one [`Reading`] each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    Power,
    Intensity,
    Charging,
    Battery,
    /// WiFi signal strength in dBm. Only MQTT messages carry it.
    Rssi,
}

impl Field {
    /// The name used in the saved file.
    pub fn key(self) -> &'static str {
        match self {
            Field::Power => "power",
            Field::Intensity => "intensity",
            Field::Charging => "charging",
            Field::Battery => "battery",
            Field::Rssi => "rssi",
        }
    }

    pub fn from_key(key: &str) -> Option<Field> {
        [
            Field::Power,
            Field::Intensity,
            Field::Charging,
            Field::Battery,
            Field::Rssi,
        ]
        .into_iter()
        .find(|field| field.key() == key)
    }
}

/// The last power and intensity we told a diffuser to take. Over WiFi the diffuser
/// often says nothing back, so the screens show these as "sent, waiting for the
/// diffuser" until a later report; see [`DeviceView::pending_power`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    pub power: Option<bool>,
    pub intensity: Option<u8>,
    /// When the newest of these was sent.
    pub at: i64,
}

/// Everything the app knows about one diffuser.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceView {
    /// The 12-character device ID.
    pub id: String,
    /// What the user calls it. The ID until they rename it.
    pub label: String,
    /// The room it is in, as the person named it; none until they give it one. Kept on
    /// this Mac only: the sidebar groups the diffusers by it.
    pub room: Option<String>,
    /// The Bluetooth peripheral ID, once it has been found over Bluetooth.
    pub ble_id: Option<String>,
    pub info: Option<DeviceInfo>,
    pub readings: BTreeMap<Field, Reading>,
    /// When we last heard from it over WiFi (any message).
    pub heard_wifi: Option<i64>,
    /// When we last heard from it over Bluetooth.
    pub heard_ble: Option<i64>,
    /// When we last pinged it over WiFi.
    pub ping_at: Option<i64>,
    pub sent: Option<Sent>,
    /// The schedule table we believe the diffuser holds: the last one sent from Scent
    /// Control, by this copy of the app or, as read from the broker, by another (see
    /// `shared_schedule`). Saved with the diffuser, so that it is known again after a
    /// relaunch.
    pub sent_profile: Option<Profile>,
    /// When `sent_profile` was sent, so that the newer of it and a copy on the broker
    /// wins. `None` for a schedule saved before the time was kept. Saved with it.
    pub sent_profile_at: Option<i64>,
    /// When it last said it joined the broker (`device_connected`, on either topic).
    /// An add waits for one that comes after the restart.
    pub connected_at: Option<i64>,
    /// When its info was last read over Bluetooth, so that an add can tell the info it
    /// just read from what was known before.
    pub info_at: Option<i64>,
    /// When it last said "Credentials saved", after an add's `SAVE_MQTT`.
    pub credentials_saved_at: Option<i64>,
}

impl DeviceView {
    pub fn new(id: &str) -> DeviceView {
        DeviceView {
            id: id.to_owned(),
            label: id.to_owned(),
            room: None,
            ble_id: None,
            info: None,
            readings: BTreeMap::new(),
            heard_wifi: None,
            heard_ble: None,
            ping_at: None,
            sent: None,
            sent_profile: None,
            sent_profile_at: None,
            connected_at: None,
            info_at: None,
            credentials_saved_at: None,
        }
    }

    /// The power we sent that the diffuser has not reported on since. A report made
    /// at the same second as the send counts as a later one.
    pub fn pending_power(&self) -> Option<bool> {
        let sent = self.sent?;
        let power = sent.power?;
        (!self.reported_since(Field::Power, sent.at)).then_some(power)
    }

    /// The intensity we sent that the diffuser has not reported on since.
    pub fn pending_intensity(&self) -> Option<u8> {
        let sent = self.sent?;
        let intensity = sent.intensity?;
        (!self.reported_since(Field::Intensity, sent.at)).then_some(intensity)
    }

    fn reported_since(&self, field: Field, at: i64) -> bool {
        self.readings.get(&field).is_some_and(|r| r.at >= at)
    }

    /// Takes in what a message reported. Only the fields it carries are touched: a
    /// status with nothing in it (such as `led:50`) leaves every reading as it was.
    fn merge_readings(&mut self, status: &Status, rssi: Option<i64>, at: i64, via: Via) {
        let reported = [
            (Field::Power, status.power),
            (Field::Intensity, status.intensity),
            (Field::Charging, status.charging),
            (Field::Battery, status.battery),
            (Field::Rssi, rssi),
        ];
        for (field, value) in reported {
            if let Some(value) = value {
                self.readings.insert(field, Reading { value, at, via });
            }
        }
    }

    /// A `device_connected` message at `at`: the diffuser joined the broker. It fills
    /// in what the message carries and leaves the rest of what Bluetooth told us.
    fn note_connected(&mut self, connected: &Connected, at: i64) {
        self.connected_at = Some(at);
        let id = &self.id;
        let info = self.info.get_or_insert_with(|| DeviceInfo {
            manufacturer_id: String::new(),
            model_name: String::new(),
            variant_name: String::new(),
            device_id: id.clone(),
            firmware: String::new(),
            wifi_configured: true,
            wifi_status: WIFI_CONNECTED,
        });
        info.wifi_configured = true;
        let carried = [
            (&mut info.manufacturer_id, &connected.manufacturer),
            (&mut info.model_name, &connected.model),
            (&mut info.variant_name, &connected.variant),
            (&mut info.firmware, &connected.fw),
        ];
        for (field, value) in carried {
            if !value.is_empty() {
                field.clone_from(value);
            }
        }
    }
}

/// The add the screens started last, and where it is. Not saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adding {
    /// The diffuser being added, by its Bluetooth ID.
    pub ble_id: String,
    /// What it is to be called once it is added; empty keeps the name it has.
    pub label: String,
    /// The room it goes in once it is added, empty for none; `None` for an add the
    /// screens didn't start, which leaves it where it is.
    pub room: Option<String>,
    /// Its label and room as the add began (`None` for a diffuser not known yet), so that
    /// a name or room given in Details while the add waits, which is the more recent, is
    /// not undone when it is done.
    pub had_label: Option<String>,
    pub had_room: Option<String>,
    /// The network it was given to join, if any: never its password.
    pub network: Option<String>,
    /// Where it is, as the hub last said.
    pub step: AddStep,
    /// The last step the hub was working on, which is where a failure happened. `None`
    /// until the hub has started one.
    pub reached: Option<AddStep>,
}

/// A Bluetooth device seen while scanning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanHit {
    pub ble_id: String,
    pub name: String,
    pub rssi: Option<i16>,
}

/// Something that happened, reported by a worker or the hub.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A message on the broker: the topic and the body, as received.
    Mqtt {
        topic: String,
        body: Vec<u8>,
        at: i64,
    },
    /// A notification from the diffuser whose Bluetooth peripheral ID is `ble_id`.
    BleNotification {
        ble_id: String,
        bytes: Vec<u8>,
        at: i64,
    },
    /// The info string read from a diffuser over Bluetooth. This is how a
    /// diffuser found over Bluetooth becomes a known device.
    BleInfo {
        ble_id: String,
        info: DeviceInfo,
        at: i64,
    },
    /// The result of a scan. Replaces the previous one.
    Scanned(Vec<ScanHit>),
    /// A command went out. `summary` is what the log shows: `Command::redacted`.
    /// `power`, `intensity` and `profile` say what it asked for, when it asked.
    Sent {
        device_id: String,
        summary: String,
        via: Via,
        at: i64,
        power: Option<bool>,
        intensity: Option<u8>,
        profile: Option<Profile>,
    },
    /// An add has reached `step` (see `add_flow`). The diffuser is known by its
    /// Bluetooth ID until its info has been read.
    Adding {
        ble_id: String,
        step: AddStep,
    },
    /// A `PING` went to the diffuser over WiFi.
    Pinged {
        device_id: String,
        at: i64,
    },
    /// Something failed: `what` was being attempted, `error` is why it didn't work.
    Failed {
        device_id: Option<String>,
        what: String,
        error: String,
        at: i64,
    },
    BrokerConnected,
    BrokerLoginRefused(String),
    BrokerDisconnected(String),
}

/// A change the person made to how this Mac knows a diffuser, in the screens. What they
/// typed is tidied with [`clean_name`] as it is made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// Renames it. An empty name takes its name away: the screens call it by its model.
    Name { device_id: String, name: String },
    /// Puts it in a room, spelled as the room in use when it is one in other capitals. An
    /// empty room takes it out of any.
    Room { device_id: String, room: String },
}

/// A name or a room as typed, tidied: no spaces at either end, one space between words, no
/// control characters, and at most [`NAME_MAX_CHARS`] characters. Empty when nothing is
/// left.
pub fn clean_name(typed: &str) -> String {
    let printable: String = typed
        .chars()
        .filter(|c| c.is_whitespace() || !c.is_control())
        .collect();
    let words = printable.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = words.chars().take(NAME_MAX_CHARS).collect();
    cut.trim_end().to_owned()
}

/// Whether two rooms are one, as the person sees them: capitals don't make another room.
pub fn same_room(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum BrokerState {
    #[default]
    Connecting,
    Connected,
    LoginRefused(String),
    Disconnected(String),
}

/// One line of the conversation log.
///
/// Notes the app makes itself carry no device: broker changes are logged as
/// `FromDevice` over WiFi with the action `broker`, and failures as `ToDevice` with
/// no `via`, the action being what was attempted. The `text` never holds a password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub at: i64,
    pub device_id: Option<String>,
    pub via: Option<Via>,
    pub direction: Direction,
    pub action: String,
    pub text: String,
    /// Whether this app sent the command (an `Event::Sent`), as against something it
    /// heard on the broker. A command we publish over WiFi comes back as a second
    /// entry, the broker's copy, which is not `sent`; so is a command someone else
    /// published to the diffuser's topic.
    pub sent: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppState {
    pub devices: BTreeMap<String, DeviceView>,
    pub scan: Vec<ScanHit>,
    pub broker: BrokerState,
    /// The conversation, oldest first, at most [`LOG_CAP`] entries.
    pub log: VecDeque<LogEntry>,
    /// The adds the screens started, by Bluetooth ID. Not saved.
    pub adding: BTreeMap<String, Adding>,
}

impl AppState {
    /// Folds one event in. Events with no time of their own (the broker ones) are
    /// logged at the current time.
    pub fn apply(&mut self, event: Event) {
        self.apply_at(event, unix_now());
    }

    /// [`apply`](Self::apply) with the clock given, for events that carry no time.
    pub fn apply_at(&mut self, event: Event, now: i64) {
        match event {
            Event::Mqtt { topic, body, at } => self.on_mqtt(&topic, &body, at),
            Event::BleNotification { ble_id, bytes, at } => {
                self.on_ble_notification(&ble_id, &bytes, at);
            }
            Event::BleInfo { ble_id, info, at } => self.on_ble_info(ble_id, info, at),
            Event::Scanned(hits) => self.scan = hits,
            Event::Sent {
                device_id,
                summary,
                via,
                at,
                power,
                intensity,
                profile,
            } => {
                self.push_log(LogEntry {
                    at,
                    device_id: Some(device_id.clone()),
                    via: Some(via),
                    direction: Direction::ToDevice,
                    action: action_of(&summary),
                    text: summary,
                    sent: true,
                });
                if let Some(device) = self.devices.get_mut(&device_id) {
                    if power.is_some() || intensity.is_some() {
                        remember_sent(device, power, intensity, at);
                    }
                    if let Some(profile) = profile {
                        let earlier = device.sent_profile.take();
                        device.sent_profile = Some(remember_profile(earlier, profile));
                        device.sent_profile_at = Some(at);
                    }
                }
            }
            Event::Adding { ble_id, step } => self.on_adding(&ble_id, step, now),
            Event::Pinged { device_id, at } => {
                if let Some(device) = self.devices.get_mut(&device_id) {
                    device.ping_at = Some(at);
                }
            }
            Event::Failed {
                device_id,
                what,
                error,
                at,
            } => self.push_log(LogEntry {
                at,
                device_id,
                via: None,
                direction: Direction::ToDevice,
                action: what,
                text: error,
                sent: false,
            }),
            Event::BrokerConnected => self.set_broker(BrokerState::Connected, now),
            Event::BrokerLoginRefused(why) => self.set_broker(BrokerState::LoginRefused(why), now),
            Event::BrokerDisconnected(why) => self.set_broker(BrokerState::Disconnected(why), now),
        }
    }

    /// Starts following an add of the diffuser at `ble_id`, to be called `label` and put in
    /// `room`, given `network` to join if any. The screens call it as they hand the add to
    /// the hub. An add of it already under way is left as it is: the hub turns the new one
    /// down.
    pub fn begin_add(&mut self, ble_id: &str, label: &str, room: &str, network: Option<&str>) {
        if self
            .adding
            .get(ble_id)
            .is_some_and(|adding| adding.step.is_running())
        {
            return;
        }
        let known = self
            .devices
            .values()
            .find(|device| device.ble_id.as_deref() == Some(ble_id));
        let adding = Adding {
            ble_id: ble_id.to_owned(),
            label: label.to_owned(),
            room: Some(room.to_owned()),
            had_label: known.map(|device| device.label.clone()),
            had_room: known.and_then(|device| device.room.clone()),
            network: network.map(str::to_owned),
            step: AddStep::ReadInfo,
            reached: None,
        };
        self.adding.insert(ble_id.to_owned(), adding);
    }

    /// The hub says where the add of `ble_id` is. Once it is done, the diffuser takes its
    /// name and its room, each unless it was changed in Details since the add began. How
    /// each add ended is logged, as its sheet may have been closed: a failure as one, and an
    /// add as heard on the broker, where it showed up.
    fn on_adding(&mut self, ble_id: &str, step: AddStep, now: i64) {
        let adding = self
            .adding
            .entry(ble_id.to_owned())
            .or_insert_with(|| Adding {
                ble_id: ble_id.to_owned(),
                label: String::new(),
                room: None,
                had_label: None,
                had_room: None,
                network: None,
                step: AddStep::ReadInfo,
                reached: None,
            });
        if step.is_running() {
            adding.reached = Some(step.clone());
        }
        adding.step = step.clone();
        let given = adding.clone();
        let (via, direction, text) = match step {
            AddStep::Done(id) => {
                // A diffuser not known as the add began had its ID for a label, no room.
                let (label, room) = self.devices.get(&id).map_or((false, false), |device| {
                    let had_label = given.had_label.as_deref().unwrap_or(&id);
                    (device.label == had_label, device.room == given.had_room)
                });
                if label && !given.label.is_empty() {
                    self.edit(Edit::Name {
                        device_id: id.clone(),
                        name: given.label,
                    });
                }
                if let Some(room) = given.room.filter(|_| room) {
                    self.edit(Edit::Room {
                        device_id: id,
                        room,
                    });
                }
                (Some(Via::Wifi), Direction::FromDevice, ADDED.to_owned())
            }
            AddStep::Failed(why) => (None, Direction::ToDevice, why),
            _ => return,
        };
        let device_id = self
            .devices
            .values()
            .find(|device| device.ble_id.as_deref() == Some(ble_id))
            .map(|device| device.id.clone());
        self.push_log(LogEntry {
            at: now,
            device_id,
            via,
            direction,
            action: ADD.to_owned(),
            text,
            sent: false,
        });
    }

    /// Makes a change the person asked for in the screens. A diffuser this app doesn't
    /// know is left alone.
    pub fn edit(&mut self, edit: Edit) {
        match edit {
            Edit::Name { device_id, name } => {
                if let Some(device) = self.devices.get_mut(&device_id) {
                    let name = clean_name(&name);
                    device.label = if name.is_empty() { device_id } else { name };
                }
            }
            Edit::Room { device_id, room } => {
                let room = self.room_called(&room, &device_id);
                if let Some(device) = self.devices.get_mut(&device_id) {
                    device.room = room;
                }
            }
        }
    }

    /// The rooms the diffusers are in, each once, in alphabetical order. Capitals don't
    /// make another room: the first spelling, by device ID, is the one listed.
    pub fn rooms(&self) -> Vec<String> {
        let mut rooms: Vec<String> = Vec::new();
        for room in self.devices.values().filter_map(|d| d.room.as_deref()) {
            if !rooms.iter().any(|listed| same_room(listed, room)) {
                rooms.push(room.to_owned());
            }
        }
        rooms.sort_by_cached_key(|room| room.to_lowercase());
        rooms
    }

    /// `typed` as the room of `device_id`: tidied, and spelled as the room another
    /// diffuser is in when it is that one in other capitals. `None` when nothing is left.
    fn room_called(&self, typed: &str, device_id: &str) -> Option<String> {
        let room = clean_name(typed);
        if room.is_empty() {
            return None;
        }
        let in_use = self
            .devices
            .values()
            .filter(|device| device.id != device_id)
            .filter_map(|device| device.room.as_deref())
            .find(|other| same_room(other, &room));
        Some(in_use.map_or(room, str::to_owned))
    }

    /// Whether the diffuser answers pings. Only WiFi counts: the pings go over WiFi,
    /// so hearing it over Bluetooth says nothing about its WiFi link.
    pub fn online(&self, id: &str, now: i64) -> Online {
        self.devices.get(id).map_or(Online::Unknown, |device| {
            online_status(device.ping_at, device.heard_wifi, now)
        })
    }

    fn on_mqtt(&mut self, topic: &str, body: &[u8], at: i64) {
        if topic.starts_with(APP_PREFIX) {
            self.on_app_message(topic, body, at);
            return;
        }
        let MqttMessage {
            direction,
            action,
            device_id,
            status,
            rssi,
            ack,
            connected,
            raw,
            ..
        } = parse_mqtt(topic, body);
        // The ping round (our `PING`, then the diffuser's "already added") happens every
        // 30 s for each diffuser. Logged, it would push the conversation out of the
        // capped log within hours. It is still heard, below.
        let routine = ack == Some(Ack::AlreadyAdded)
            || (direction == Direction::ToDevice && action.as_deref() == Some("PING"));
        let credentials_saved =
            action.as_deref() == Some("acknowledgement") && says_credentials_saved(&raw);
        if !routine {
            self.push_log(LogEntry {
                at,
                device_id: device_id.clone(),
                via: Some(Via::Wifi),
                direction,
                action: action.unwrap_or_else(|| NO_ACTION.to_owned()),
                text: raw,
                sent: false,
            });
        }
        // Our own commands come back to us too (we hear everything under the
        // prefix). They are logged above, except the ping round, and tell us nothing
        // about the diffuser.
        if direction != Direction::FromDevice {
            return;
        }
        let Some(id) = device_id.filter(|id| valid_device_id(id)) else {
            return;
        };
        let device = self
            .devices
            .entry(id)
            .or_insert_with_key(|id| DeviceView::new(id));
        device.heard_wifi = Some(at);
        if let Some(connected) = &connected {
            device.note_connected(connected, at);
        }
        if credentials_saved {
            device.credentials_saved_at = Some(at);
        }
        device.merge_readings(&status.unwrap_or_default(), rssi, at, Via::Wifi);
    }

    /// A message under our own prefix: the copy of a diffuser's last schedule that a
    /// copy of the app stored on the broker (see `shared_schedule`). The newer of it and
    /// what the app knows wins. A diffuser not known yet becomes known, as one heard on
    /// the broker does. It is logged only when it changes the schedule the app knew, so
    /// that our own copy heard back, and every copy again on each reconnect, add nothing.
    /// One that can't be used changes nothing and is logged, masked, once, with why
    /// (another version's, or unreadable) in its action.
    fn on_app_message(&mut self, topic: &str, body: &[u8], at: i64) {
        // An empty retained message is how a stored one is taken off the broker.
        if body.is_empty() {
            return;
        }
        let text = String::from_utf8_lossy(body).into_owned();
        let device_id = schedule_topic_device(topic);
        let decoded = device_id
            .ok_or(Unreadable::Invalid)
            .and_then(|_| shared_schedule::decode(body, at));
        let (id, shared) = match (device_id, decoded) {
            (Some(id), Ok(shared)) => (id, shared),
            (_, unusable) => {
                let action = if matches!(unusable, Err(Unreadable::Version)) {
                    OTHER_VERSION_SCHEDULE
                } else {
                    UNREADABLE_SCHEDULE
                };
                self.push_log(LogEntry {
                    at,
                    device_id: device_id.map(str::to_owned),
                    via: Some(Via::Wifi),
                    direction: Direction::FromDevice,
                    action: action.to_owned(),
                    text,
                    sent: false,
                });
                return;
            }
        };
        let device = self
            .devices
            .entry(id.to_owned())
            .or_insert_with_key(|id| DeviceView::new(id));
        if device
            .sent_profile_at
            .is_some_and(|known| shared.sent_at <= known)
        {
            return;
        }
        let changed = device.sent_profile.as_ref() != Some(&shared.profile);
        device.sent_profile = Some(shared.profile);
        device.sent_profile_at = Some(shared.sent_at);
        if changed {
            self.push_log(LogEntry {
                at,
                device_id: Some(id.to_owned()),
                via: Some(Via::Wifi),
                direction: Direction::FromDevice,
                action: SHARED_SCHEDULE.to_owned(),
                text,
                sent: false,
            });
        }
    }

    fn on_ble_notification(&mut self, ble_id: &str, bytes: &[u8], at: i64) {
        let note = parse_notification(bytes);
        let device = self
            .devices
            .values_mut()
            .find(|device| device.ble_id.as_deref() == Some(ble_id));
        let device_id = device.as_ref().map(|device| device.id.clone());
        if let Some(device) = device {
            device.heard_ble = Some(at);
            device.merge_readings(&note.status, None, at, Via::Bluetooth);
            if says_credentials_saved(&note.raw) {
                device.credentials_saved_at = Some(at);
            }
        }
        let action = match note.kind {
            NoteKind::StatusText | NoteKind::StatusArray => "status",
            NoteKind::Value => "value",
            NoteKind::Json => "json",
            NoteKind::Unknown => "unknown",
        };
        self.push_log(LogEntry {
            at,
            device_id,
            via: Some(Via::Bluetooth),
            direction: Direction::FromDevice,
            action: action.to_owned(),
            text: note.raw,
            sent: false,
        });
    }

    fn on_ble_info(&mut self, ble_id: String, info: DeviceInfo, at: i64) {
        let text = info_text(&info);
        let device_id = valid_device_id(&info.device_id).then(|| info.device_id.clone());
        if let Some(id) = &device_id {
            let device = self
                .devices
                .entry(id.clone())
                .or_insert_with_key(|id| DeviceView::new(id));
            device.ble_id = Some(ble_id);
            device.info = Some(info);
            device.info_at = Some(at);
            device.heard_ble = Some(at);
        }
        self.push_log(LogEntry {
            at,
            device_id,
            via: Some(Via::Bluetooth),
            direction: Direction::FromDevice,
            action: "info".to_owned(),
            text,
            sent: false,
        });
    }

    /// Records a change of broker state. A repeat of the state we're already in is
    /// not news: the retry loop would otherwise fill the log with it.
    fn set_broker(&mut self, state: BrokerState, now: i64) {
        if self.broker == state {
            return;
        }
        let text = match &state {
            BrokerState::Connecting => "Connecting to the broker.".to_owned(),
            BrokerState::Connected => "Connected to the broker.".to_owned(),
            BrokerState::LoginRefused(why) => format!("Broker login refused: {why}"),
            BrokerState::Disconnected(why) => format!("Disconnected from the broker: {why}"),
        };
        self.broker = state;
        self.push_log(LogEntry {
            at: now,
            device_id: None,
            via: Some(Via::Wifi),
            direction: Direction::FromDevice,
            action: "broker".to_owned(),
            text,
            sent: false,
        });
    }

    /// Adds to the log, hiding any password in the text and dropping the oldest
    /// entry when the log is full.
    fn push_log(&mut self, mut entry: LogEntry) {
        entry.text = mask_passwords(&entry.text);
        if self.log.len() == LOG_CAP {
            self.log.pop_front();
        }
        self.log.push_back(entry);
    }
}

/// Seconds since 1970 now; the time the workers stamp their events with.
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

/// Whether a diffuser's message says it took `SAVE_MQTT` (see [`CREDENTIALS_SAVED`]).
fn says_credentials_saved(text: &str) -> bool {
    text.to_ascii_lowercase().contains(CREDENTIALS_SAVED)
}

/// The `action` of a command's redacted frame (Bluetooth frames end in `-|`).
fn action_of(summary: &str) -> String {
    let frame = summary.strip_suffix("-|").unwrap_or(summary);
    serde_json::from_str::<serde_json::Value>(frame)
        .ok()
        .and_then(|json| json.get("action")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| "sent".to_owned())
}

/// The info string as the diffuser holds it, one field per `|`.
fn info_text(info: &DeviceInfo) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}",
        info.manufacturer_id,
        info.model_name,
        info.variant_name,
        info.device_id,
        info.firmware,
        info.wifi_configured,
        info.wifi_status
    )
}

/// Notes a power or intensity command. What an earlier command asked for stays
/// pending unless the diffuser has reported on it since, so a report is never
/// hidden by a later send of something else (all share one `at`).
fn remember_sent(device: &mut DeviceView, power: Option<bool>, intensity: Option<u8>, at: i64) {
    device.sent = Some(Sent {
        power: power.or(device.pending_power()),
        intensity: intensity.or(device.pending_intensity()),
        at,
    });
}

/// What the diffuser holds after `UPDATE_PROFILE` with `sent`. With flag 0 only
/// the always-on entry went out, so the timed entries stay what they were before;
/// we know of none if nothing was sent earlier. Flag 1 replaces the whole table.
pub(crate) fn remember_profile(earlier: Option<Profile>, sent: Profile) -> Profile {
    match sent.flag {
        ScheduleFlag::Replace => sent,
        ScheduleFlag::AlwaysOnOnly => Profile {
            entries: earlier.map(|earlier| earlier.entries).unwrap_or_default(),
            ..sent
        },
    }
}

const MASK: &str = "***";
const PASSWORD: &str = "password";

/// `text` with the value of every `password` key replaced by `"***"`.
///
/// Works on the text itself, so it also covers what is not valid JSON (the
/// firmware sends some) or is cut short, and JSON inside a JSON string. A key
/// counts when its name contains `password` in any case (`wifi_password`). Whatever
/// the value is, a string, a number or `null`, it is shown as `"***"`. Masking
/// text that is already masked changes nothing.
pub fn mask_passwords(text: &str) -> String {
    let lower = text.to_ascii_lowercase(); // same byte positions as `text`
    let mut masked = String::with_capacity(text.len());
    let mut copied = 0; // `text[..copied]` is already in `masked`
    let mut search = 0;
    while let Some(found) = lower[search..].find(PASSWORD) {
        let key = search + found;
        search = key + PASSWORD.len();
        let Some(value) = password_value(text.as_bytes(), key) else {
            continue;
        };
        masked.push_str(&text[copied..value.start]);
        let quote = format!("{}\"", "\\".repeat(value.escapes));
        masked.push_str(&format!("{quote}{MASK}{quote}"));
        copied = value.end;
        search = search.max(value.end);
    }
    masked.push_str(&text[copied..]);
    masked
}

/// Where a password's value sits in the text.
struct PasswordValue {
    start: usize,
    end: usize,
    /// Backslashes in front of each quote of a string value: 0 in plain JSON, more
    /// when the JSON sits inside a string.
    escapes: usize,
}

/// The value of the key that has `password` at byte `key`, if there is one.
fn password_value(bytes: &[u8], key: usize) -> Option<PasswordValue> {
    // The key is a quoted name with "password" somewhere in it.
    let mut name_start = key;
    while name_start > 0 && is_name_byte(bytes[name_start - 1]) {
        name_start -= 1;
    }
    if name_start == 0 || bytes[name_start - 1] != b'"' {
        return None;
    }
    let mut i = key + PASSWORD.len();
    while bytes.get(i).is_some_and(|&b| is_name_byte(b)) {
        i += 1;
    }
    while bytes.get(i) == Some(&b'\\') {
        i += 1;
    }
    if bytes.get(i) != Some(&b'"') {
        return None;
    }
    i = skip_spaces(bytes, i + 1);
    if bytes.get(i) != Some(&b':') {
        return None;
    }
    let start = skip_spaces(bytes, i + 1);
    let escapes = bytes[start..].iter().take_while(|&&b| b == b'\\').count();
    let quote = start + escapes;
    if bytes.get(quote) == Some(&b'"') {
        let end = end_of_string(bytes, quote + 1, escapes);
        return Some(PasswordValue {
            start,
            end,
            escapes,
        });
    }
    // Not a string: it runs to the next delimiter.
    let end = bytes[start..]
        .iter()
        .position(|&b| matches!(b, b',' | b'}' | b']') || b.is_ascii_whitespace())
        .map_or(bytes.len(), |n| start + n);
    Some(PasswordValue {
        start,
        end,
        escapes: 0,
    })
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn skip_spaces(bytes: &[u8], mut i: usize) -> usize {
    while bytes.get(i).is_some_and(|b| b.is_ascii_whitespace()) {
        i += 1;
    }
    i
}

/// The position just after the quote that closes the string whose content starts at
/// `from`; the end of the text if it is never closed.
fn end_of_string(bytes: &[u8], from: usize, escapes: usize) -> usize {
    if escapes == 0 {
        let mut i = from;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => i += 2,
                b'"' => return i + 1,
                _ => i += 1,
            }
        }
    } else {
        // In JSON inside a string, the closing quote has as many backslashes in
        // front of it as the opening one had; a quote inside the value has more.
        let mut run = 0;
        for (n, &b) in bytes[from..].iter().enumerate() {
            match b {
                b'\\' => run += 1,
                b'"' if run == escapes => return from + n + 1,
                _ => run = 0,
            }
        }
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use scent_core::{
        AlwaysOn, Command, DeviceInfo, Direction, Entry, HhMm, Level, Online, Profile,
        ScheduleFlag, Secret, Status, Via, parse_mqtt,
    };

    use super::*;
    use crate::add_flow::AddStep;
    use crate::mqtt::BrokerSettings;
    use crate::store::{Store, TempDir};
    use crate::ui::theme::Appearance;

    const ID1: &str = "TESTDEVICE01";
    const ID2: &str = "TESTDEVICE02";
    const BLE1: &str = "ble-peripheral-1";
    /// The clock for events that carry no time of their own.
    const NOW: i64 = 5_000;
    const USER_TOPIC: &str = "scentiment-mqtt-prod/user/route-test-1";
    const DEVICE1_TOPIC: &str = "scentiment-mqtt-prod/device/TESTDEVICE01";
    /// Power 1, intensity 2, charging 2, battery 200, rssi -66.
    const UPDATE_STATUS: &str =
        include_str!("../../../fixtures/scentiment/air2-mqtt-update-status-object.json");
    /// The reply to every `PING` after the first.
    const ACK_ALREADY_ADDED: &str =
        include_str!("../../../fixtures/scentiment/air2-mqtt-ack-already-added.json");
    /// The reply to the first `PING` with a route ID.
    const ACK_ADDED: &str = include_str!("../../../fixtures/scentiment/air2-mqtt-ack-added.json");

    // Builders

    fn fixture_event(fixture: &str, at: i64) -> Event {
        let json: serde_json::Value = serde_json::from_str(fixture).unwrap();
        Event::Mqtt {
            topic: json["topic"].as_str().unwrap().to_owned(),
            body: json["text"].as_str().unwrap().as_bytes().to_vec(),
            at,
        }
    }

    fn mqtt(topic: &str, body: &str, at: i64) -> Event {
        Event::Mqtt {
            topic: topic.to_owned(),
            body: body.as_bytes().to_vec(),
            at,
        }
    }

    /// An `update_status` message whose `data` is Bluetooth-style text such as `S:1,L:2`.
    fn report(id: &str, data: &str, at: i64) -> Event {
        let body = format!(
            r#"{{"action":"update_status","payload":{{"deviceId":"{id}","data":"{data}"}}}}"#
        );
        mqtt(USER_TOPIC, &body, at)
    }

    fn note(ble_id: &str, text: &str, at: i64) -> Event {
        Event::BleNotification {
            ble_id: ble_id.to_owned(),
            bytes: text.as_bytes().to_vec(),
            at,
        }
    }

    fn info(id: &str) -> DeviceInfo {
        DeviceInfo {
            manufacturer_id: "01".into(),
            model_name: "Diffuser".into(),
            variant_name: "Black".into(),
            device_id: id.into(),
            firmware: "2.30".into(),
            wifi_configured: true,
            wifi_status: 3,
        }
    }

    fn ble_info(ble_id: &str, id: &str, at: i64) -> Event {
        Event::BleInfo {
            ble_id: ble_id.to_owned(),
            info: info(id),
            at,
        }
    }

    fn sent(id: &str, power: Option<bool>, intensity: Option<u8>, at: i64) -> Event {
        Event::Sent {
            device_id: id.to_owned(),
            summary: Command::TurnOn { on: true }.redacted(Via::Wifi),
            via: Via::Wifi,
            at,
            power,
            intensity,
            profile: None,
        }
    }

    fn sent_profile(id: &str, profile: Profile, at: i64) -> Event {
        Event::Sent {
            device_id: id.to_owned(),
            summary: Command::UpdateProfile(profile.clone()).redacted(Via::Wifi),
            via: Via::Wifi,
            at,
            power: None,
            intensity: None,
            profile: Some(profile),
        }
    }

    fn failed(device_id: Option<&str>, what: &str, error: &str, at: i64) -> Event {
        Event::Failed {
            device_id: device_id.map(str::to_owned),
            what: what.to_owned(),
            error: error.to_owned(),
            at,
        }
    }

    /// An Air 2 the app already knows over Bluetooth.
    fn known_air2() -> AppState {
        let mut s = AppState::default();
        s.apply_at(ble_info(BLE1, ID1, 1), NOW);
        s
    }

    fn level(n: u8) -> Level {
        Level::new(n).unwrap()
    }

    fn window(start: &str, end: &str) -> Entry {
        Entry {
            start: HhMm::parse(start).unwrap(),
            end: HhMm::parse(end).unwrap(),
            weekdays: vec![2],
            level: level(3),
            enable: true,
        }
    }

    fn profile(always_on: bool, entries: Vec<Entry>, flag: ScheduleFlag) -> Profile {
        Profile {
            always_on: AlwaysOn {
                enable: always_on,
                level: level(1),
            },
            entries,
            timezone_hours: -5.0,
            flag,
        }
    }

    // Readings

    // Port of tools/tests/test_control_server.py (apply_event keeps each reading's time and source).
    #[test]
    fn apply_mqtt_keeps_each_reading_with_time_and_source() {
        let mut s = AppState::default();
        s.apply_at(fixture_event(UPDATE_STATUS, 1_000), NOW);
        s.apply_at(ble_info(BLE1, ID1, 1_500), NOW);
        s.apply_at(note(BLE1, "enable:0-|", 2_000), NOW);

        let d = &s.devices[ID1];
        assert_eq!(
            d.readings[&Field::Power],
            Reading {
                value: 0,
                at: 2_000,
                via: Via::Bluetooth
            }
        );
        assert_eq!(
            d.readings[&Field::Charging],
            Reading {
                value: 2,
                at: 1_000,
                via: Via::Wifi
            }
        );
        assert_eq!(d.readings[&Field::Battery].value, 200);
        assert_eq!(
            d.readings[&Field::Intensity],
            Reading {
                value: 2,
                at: 1_000,
                via: Via::Wifi
            }
        );
        assert_eq!(
            d.readings[&Field::Rssi],
            Reading {
                value: -66,
                at: 1_000,
                via: Via::Wifi
            }
        );
        assert_eq!((d.heard_wifi, d.heard_ble), (Some(1_000), Some(2_000)));
    }

    #[test]
    fn empty_status_does_not_wipe_readings() {
        let mut s = known_air2();
        s.apply_at(fixture_event(UPDATE_STATUS, 1_000), NOW);
        let before = s.devices[ID1].readings.clone();
        assert_eq!(before.len(), 5);

        // `led:50` is a value we don't map: it parses to a status with nothing in it.
        let led =
            r#"{"action":"notification","payload":{"deviceId":"TESTDEVICE01","data":"led:50"}}"#;
        assert_eq!(
            parse_mqtt(USER_TOPIC, led.as_bytes()).status,
            Some(Status::default())
        );
        s.apply_at(mqtt(USER_TOPIC, led, 3_000), NOW);
        s.apply_at(note(BLE1, "led:50-|", 3_100), NOW);

        let d = &s.devices[ID1];
        assert_eq!(d.readings, before);
        // The diffuser did speak, so it was heard.
        assert_eq!((d.heard_wifi, d.heard_ble), (Some(3_000), Some(3_100)));
    }

    #[test]
    fn a_status_with_some_fields_updates_only_those() {
        let mut s = known_air2();
        s.apply_at(report(ID1, "S:1,L:2,C:2,Bat:200", 1_000), NOW);
        s.apply_at(report(ID1, "S:0,L:3", 2_000), NOW);

        let d = &s.devices[ID1];
        assert_eq!(d.readings[&Field::Power].value, 0);
        assert_eq!(d.readings[&Field::Intensity].value, 3);
        assert_eq!(d.readings[&Field::Charging].at, 1_000);
        assert_eq!(d.readings[&Field::Battery].at, 1_000);
    }

    #[test]
    fn only_valid_device_ids_make_a_device() {
        let mut s = AppState::default();
        s.apply_at(report("nope", "S:1,L:2", 100), NOW);
        s.apply_at(report("TESTDEVICE01/x", "S:1,L:2", 100), NOW);
        s.apply_at(mqtt(USER_TOPIC, "{ not json", 100), NOW);
        assert!(s.devices.is_empty());
        assert_eq!(s.log.len(), 3, "every message is logged, readable or not");

        s.apply_at(report(ID2, "S:1,L:2", 200), NOW);
        assert_eq!(s.devices.keys().collect::<Vec<_>>(), [ID2]);
        assert_eq!(s.devices[ID2].label, ID2);
    }

    #[test]
    fn messages_to_a_device_are_logged_but_change_nothing() {
        let mut s = known_air2();
        // (A `PING` is a message to a device too, but a routine one: see below.)
        let command = r#"{"action":"TURN_ON","payload":{"on":1}}"#;
        s.apply_at(mqtt(DEVICE1_TOPIC, command, 100), NOW);
        s.apply_at(
            mqtt("scentiment-mqtt-prod/device/TESTDEVICE02", command, 101),
            NOW,
        );
        assert_eq!(s.devices.len(), 1, "no device appears from an echo");
        assert_eq!(s.devices[ID1].heard_wifi, None);
        let echoes: Vec<&LogEntry> = s
            .log
            .iter()
            .filter(|entry| entry.direction == Direction::ToDevice)
            .collect();
        assert_eq!(echoes.len(), 2);
        let entry = echoes[0];
        assert_eq!(entry.device_id.as_deref(), Some(ID1));
        assert_eq!(entry.via, Some(Via::Wifi));
        assert_eq!(entry.action, "TURN_ON");
        assert_eq!(entry.text, command);
    }

    #[test]
    fn ble_notifications_are_matched_to_a_device_by_ble_id() {
        let mut s = known_air2();
        s.apply_at(note(BLE1, "S:1,L:3,C:2,Bat:200-|", 300), NOW);
        let d = &s.devices[ID1];
        assert_eq!(d.readings[&Field::Intensity].via, Via::Bluetooth);
        assert_eq!(d.heard_ble, Some(300));
        let entry = s.log.back().unwrap();
        assert_eq!(
            (entry.device_id.as_deref(), entry.via, entry.direction),
            (Some(ID1), Some(Via::Bluetooth), Direction::FromDevice)
        );
        assert_eq!(
            (entry.action.as_str(), entry.text.as_str()),
            ("status", "S:1,L:3,C:2,Bat:200")
        );

        // A peripheral we don't know: logged, nothing else.
        s.apply_at(note("someone-elses", "enable:1-|", 400), NOW);
        assert_eq!(s.devices.len(), 1);
        assert_eq!(s.devices[ID1].heard_ble, Some(300));
        let entry = s.log.back().unwrap();
        assert_eq!(entry.device_id, None);
        assert_eq!(
            (entry.action.as_str(), entry.text.as_str()),
            ("value", "enable:1")
        );
    }

    #[test]
    fn reading_the_info_adds_a_device_with_its_bluetooth_id() {
        let mut s = AppState::default();
        s.apply_at(ble_info(BLE1, ID1, 50), NOW);
        let d = &s.devices[ID1];
        assert_eq!(d.ble_id.as_deref(), Some(BLE1));
        assert_eq!(d.info, Some(info(ID1)));
        assert_eq!((d.label.as_str(), d.heard_ble), (ID1, Some(50)));

        // Reading it again keeps what the user named it.
        s.devices.get_mut(ID1).unwrap().label = "Bedroom".into();
        s.apply_at(ble_info(BLE1, ID1, 60), NOW);
        assert_eq!(s.devices[ID1].label, "Bedroom");
        assert_eq!(s.devices.len(), 1);

        // An ID that isn't a device ID adds nothing.
        s.apply_at(ble_info("ble-2", "not an id", 70), NOW);
        assert_eq!(s.devices.len(), 1);
    }

    #[test]
    fn device_connected_fills_in_what_the_device_says_about_itself() {
        let connected = r#"{"action":"device_connected","payload":{"deviceId":"TESTDEVICE01","fw":"2.30","model":"Diffuser","variant":"Black","manufacturer":"01"}}"#;
        let topic = "scentiment-mqtt-prod/server-notifications/device";

        let mut s = AppState::default();
        s.apply_at(mqtt(topic, connected, 700), NOW);
        assert_eq!(s.devices[ID1].info, Some(info(ID1)));
        assert_eq!(s.devices[ID1].heard_wifi, Some(700));

        // With a Bluetooth reading already there, only what the message carries changes.
        let mut s = AppState::default();
        let mut old = info(ID1);
        old.firmware = "2.21".into();
        old.wifi_configured = false;
        old.wifi_status = 1;
        s.apply_at(
            Event::BleInfo {
                ble_id: BLE1.into(),
                info: old,
                at: 1,
            },
            NOW,
        );
        s.apply_at(mqtt(topic, connected, 700), NOW);
        let got = s.devices[ID1].info.clone().unwrap();
        assert_eq!(got.firmware, "2.30");
        assert!(got.wifi_configured);
        assert_eq!(got.wifi_status, 1, "the Bluetooth reading's status stays");
    }

    #[test]
    fn scan_results_replace_the_previous_list() {
        let hit = |name: &str| ScanHit {
            ble_id: format!("ble-{name}"),
            name: name.to_owned(),
            rssi: Some(-60),
        };
        let mut s = AppState::default();
        s.apply_at(Event::Scanned(vec![hit("a"), hit("b")]), NOW);
        assert_eq!(s.scan.len(), 2);
        s.apply_at(Event::Scanned(vec![hit("c")]), NOW);
        assert_eq!(s.scan, vec![hit("c")]);
    }

    // Sent commands

    #[test]
    fn sent_is_pending_until_a_later_report() {
        let mut s = known_air2();
        s.apply_at(sent(ID1, Some(true), None, 100), NOW);
        assert_eq!(s.devices[ID1].pending_power(), Some(true));

        // A report from before the send says nothing about it.
        s.apply_at(report(ID1, "S:0,L:1", 90), NOW);
        assert_eq!(s.devices[ID1].pending_power(), Some(true));

        s.apply_at(report(ID1, "S:1,L:1", 110), NOW);
        assert_eq!(s.devices[ID1].pending_power(), None);
    }

    #[test]
    fn pending_is_tracked_per_field() {
        let mut s = known_air2();
        s.apply_at(sent(ID1, Some(true), Some(2), 100), NOW);
        s.apply_at(note(BLE1, "enable:1-|", 110), NOW);
        let d = &s.devices[ID1];
        assert_eq!((d.pending_power(), d.pending_intensity()), (None, Some(2)));

        s.apply_at(note(BLE1, "level:2-|", 120), NOW);
        assert_eq!(s.devices[ID1].pending_intensity(), None);
    }

    #[test]
    fn a_later_send_keeps_unconfirmed_values_and_drops_confirmed_ones() {
        // Power was confirmed at 110, so sending an intensity at 120 must not
        // bring "waiting for the diffuser" back for the power.
        let mut s = known_air2();
        s.apply_at(sent(ID1, Some(true), None, 100), NOW);
        s.apply_at(report(ID1, "S:1,L:1", 110), NOW);
        s.apply_at(sent(ID1, None, Some(3), 120), NOW);
        let d = &s.devices[ID1];
        assert_eq!((d.pending_power(), d.pending_intensity()), (None, Some(3)));

        // Over WiFi the diffuser often says nothing: both stay pending.
        let mut s = known_air2();
        s.apply_at(sent(ID1, Some(true), None, 200), NOW);
        s.apply_at(sent(ID1, None, Some(2), 220), NOW);
        let d = &s.devices[ID1];
        assert_eq!(
            (d.pending_power(), d.pending_intensity()),
            (Some(true), Some(2))
        );
    }

    #[test]
    fn a_send_without_power_or_intensity_leaves_the_pending_values_alone() {
        let mut s = known_air2();
        s.apply_at(sent(ID1, Some(true), Some(2), 100), NOW);
        s.apply_at(sent(ID1, None, None, 105), NOW);
        let d = &s.devices[ID1];
        assert_eq!(
            d.sent,
            Some(Sent {
                power: Some(true),
                intensity: Some(2),
                at: 100
            })
        );
    }

    #[test]
    fn a_send_to_an_unknown_device_changes_no_device() {
        let mut s = AppState::default();
        s.apply_at(sent(ID1, Some(true), None, 100), NOW);
        assert!(s.devices.is_empty());
        assert_eq!(s.log.len(), 1);
    }

    #[test]
    fn sends_and_failures_are_logged_with_redacted_text() {
        let wifi = Command::WifiConnect {
            ssid: "ExampleWiFi".into(),
            password: Secret::new("hunter2!".into()),
        };
        let mut s = known_air2();
        for via in [Via::Bluetooth, Via::Wifi] {
            s.apply_at(
                Event::Sent {
                    device_id: ID1.into(),
                    summary: wifi.redacted(via),
                    via,
                    at: 10,
                    power: None,
                    intensity: None,
                    profile: None,
                },
                NOW,
            );
        }
        s.apply_at(failed(Some(ID1), "TURN_ON", "timed out", 11), NOW);
        s.apply_at(failed(None, "scan", "Bluetooth is off", 12), NOW);

        let log: Vec<&LogEntry> = s.log.iter().skip(1).collect(); // the first is the info read
        assert_eq!(log.len(), 4);
        assert_eq!(
            (log[0].via, log[0].direction, log[0].action.as_str()),
            (Some(Via::Bluetooth), Direction::ToDevice, "WIFI_CONNECT")
        );
        assert_eq!(log[0].text, wifi.redacted(Via::Bluetooth));
        assert!(log[0].text.contains(r#""password":"***""#));
        assert_eq!(log[1].via, Some(Via::Wifi));
        assert_eq!(
            (log[2].device_id.as_deref(), log[2].action.as_str()),
            (Some(ID1), "TURN_ON")
        );
        assert_eq!(log[2].text, "timed out");
        assert_eq!(log[3].device_id, None);
        assert_eq!(log[3].at, 12);
    }

    // Schedules

    // Port of tools/tests/test_control_server.py (sent profiles are remembered and flag 0 only
    // replaces the always-on entry).
    #[test]
    fn sent_profile_flag_0_replaces_only_the_always_on_entry() {
        let timed = window("10:24", "10:27");
        let mut s = known_air2();
        s.apply_at(
            sent_profile(
                ID1,
                profile(false, vec![timed.clone()], ScheduleFlag::Replace),
                100,
            ),
            NOW,
        );
        // Flag 0 sends the always-on entry alone, so whatever timed entries this
        // profile holds are not what the diffuser has.
        let other = window("18:00", "19:00");
        s.apply_at(
            sent_profile(
                ID1,
                profile(true, vec![other], ScheduleFlag::AlwaysOnOnly),
                200,
            ),
            NOW,
        );

        let kept = s.devices[ID1].sent_profile.as_ref().unwrap();
        assert!(kept.always_on.enable, "the always-on entry is the new one");
        assert_eq!(
            kept.entries,
            vec![timed],
            "the timed entries are the old ones"
        );
        assert_eq!(kept.flag, ScheduleFlag::AlwaysOnOnly);
        assert_eq!(kept.timezone_hours, -5.0);
    }

    #[test]
    fn a_replace_profile_is_stored_as_it_was_sent() {
        let mut s = known_air2();
        s.apply_at(
            sent_profile(
                ID1,
                profile(false, vec![window("10:24", "10:27")], ScheduleFlag::Replace),
                100,
            ),
            NOW,
        );
        let second = profile(true, vec![window("21:30", "22:30")], ScheduleFlag::Replace);
        s.apply_at(sent_profile(ID1, second.clone(), 200), NOW);
        assert_eq!(s.devices[ID1].sent_profile, Some(second));

        let none = profile(true, Vec::new(), ScheduleFlag::Replace);
        s.apply_at(sent_profile(ID1, none.clone(), 300), NOW);
        assert_eq!(s.devices[ID1].sent_profile, Some(none));
    }

    #[test]
    fn flag_0_with_no_earlier_profile_claims_no_timed_entries() {
        let mut s = known_air2();
        s.apply_at(
            sent_profile(
                ID1,
                profile(
                    true,
                    vec![window("10:24", "10:27")],
                    ScheduleFlag::AlwaysOnOnly,
                ),
                100,
            ),
            NOW,
        );
        let kept = s.devices[ID1].sent_profile.as_ref().unwrap();
        assert!(kept.always_on.enable);
        assert!(kept.entries.is_empty());
    }

    #[test]
    fn a_sent_schedule_is_remembered_with_when_it_was_sent() {
        let mut s = known_air2();
        assert_eq!(s.devices[ID1].sent_profile_at, None);
        let p = profile(true, vec![window("10:24", "10:27")], ScheduleFlag::Replace);
        s.apply_at(sent_profile(ID1, p, 100), NOW);
        assert_eq!(s.devices[ID1].sent_profile_at, Some(100));
        // A command that isn't a schedule leaves it alone.
        s.apply_at(sent(ID1, Some(true), None, 150), NOW);
        assert_eq!(s.devices[ID1].sent_profile_at, Some(100));
    }

    // Schedules stored on the broker by any copy of the app

    /// The retained copy of `profile`, sent at `sent_at`, as the broker hands it over at
    /// `at`.
    fn stored_copy(id: &str, profile: &Profile, sent_at: i64, at: i64) -> Event {
        Event::Mqtt {
            topic: format!("scent-control/v1/schedule/{id}"),
            body: crate::shared_schedule::encode(profile, sent_at).unwrap(),
            at,
        }
    }

    fn morning() -> Profile {
        profile(false, vec![window("07:00", "08:00")], ScheduleFlag::Replace)
    }

    fn evening() -> Profile {
        profile(true, vec![window("21:30", "22:30")], ScheduleFlag::Replace)
    }

    #[test]
    fn a_newer_schedule_on_the_broker_is_adopted_and_logged_once() {
        let mut s = known_air2();
        s.apply_at(sent_profile(ID1, morning(), 100), NOW);
        let saved = Store::contents(&s, &BrokerSettings::default(), Appearance::System).unwrap();
        let logged = s.log.len();

        s.apply_at(stored_copy(ID1, &evening(), 200, 300), NOW);

        let device = &s.devices[ID1];
        assert_eq!(device.sent_profile, Some(evening()));
        assert_eq!(device.sent_profile_at, Some(200));
        // A copy on the broker is not the diffuser speaking.
        assert_eq!(device.heard_wifi, None);
        let new: Vec<&LogEntry> = s.log.iter().skip(logged).collect();
        assert_eq!(new.len(), 1, "{new:?}");
        let entry = new[0];
        assert_eq!(
            (
                entry.at,
                entry.device_id.as_deref(),
                entry.via,
                entry.direction,
                entry.action.as_str(),
                entry.sent
            ),
            (
                300,
                Some(ID1),
                Some(Via::Wifi),
                Direction::FromDevice,
                SHARED_SCHEDULE,
                false
            )
        );
        // ...and desktop.json takes it at the next save, as it does any change.
        assert_ne!(
            Store::contents(&s, &BrokerSettings::default(), Appearance::System).unwrap(),
            saved
        );
    }

    #[test]
    fn an_older_schedule_on_the_broker_is_ignored() {
        let mut s = known_air2();
        s.apply_at(sent_profile(ID1, morning(), 200), NOW);
        let before = s.clone();
        s.apply_at(stored_copy(ID1, &evening(), 199, 300), NOW);
        assert_eq!(s, before);
        // One sent at the same second is not newer either.
        s.apply_at(stored_copy(ID1, &evening(), 200, 301), NOW);
        assert_eq!(s, before);
    }

    #[test]
    fn our_own_copy_heard_back_changes_nothing_and_logs_nothing() {
        let mut s = known_air2();
        s.apply_at(sent_profile(ID1, evening(), 100), NOW);
        let before = s.clone();
        // As the broker hands it back at once, and again on every reconnect.
        s.apply_at(stored_copy(ID1, &evening(), 100, 101), NOW);
        s.apply_at(stored_copy(ID1, &evening(), 100, 900), NOW);
        assert_eq!(s, before);
    }

    #[test]
    fn a_newer_copy_of_the_schedule_already_known_logs_nothing() {
        let mut s = known_air2();
        s.apply_at(sent_profile(ID1, evening(), 100), NOW);
        let logged = s.log.len();
        // Another copy of the app sent the same schedule again.
        s.apply_at(stored_copy(ID1, &evening(), 150, 160), NOW);
        assert_eq!(s.devices[ID1].sent_profile, Some(evening()));
        assert_eq!(s.devices[ID1].sent_profile_at, Some(150));
        assert_eq!(s.log.len(), logged);
    }

    #[test]
    fn a_schedule_known_without_a_time_gives_way_to_the_brokers() {
        // As loaded from a desktop.json written before the time was kept.
        let mut s = known_air2();
        s.devices.get_mut(ID1).unwrap().sent_profile = Some(morning());
        s.apply_at(stored_copy(ID1, &evening(), 5, 10), NOW);
        assert_eq!(s.devices[ID1].sent_profile, Some(evening()));
        assert_eq!(s.devices[ID1].sent_profile_at, Some(5));
        assert_eq!(s.log.back().unwrap().action, SHARED_SCHEDULE);
    }

    #[test]
    fn a_schedule_for_a_diffuser_not_known_yet_makes_it_known() {
        // As on_mqtt does for a diffuser heard on the broker: another copy of the app
        // knows it, so this one does now, by its ID until it is named.
        let mut s = AppState::default();
        s.apply_at(stored_copy(ID2, &evening(), 100, 110), NOW);
        let device = &s.devices[ID2];
        assert_eq!(device.label, ID2);
        assert_eq!(device.sent_profile, Some(evening()));
        assert_eq!(device.sent_profile_at, Some(100));
        assert_eq!((device.heard_wifi, device.ble_id.as_ref()), (None, None));
        assert_eq!(s.log.len(), 1);
        assert_eq!(s.log[0].device_id.as_deref(), Some(ID2));
        assert_eq!(s.log[0].action, SHARED_SCHEDULE);
    }

    #[test]
    fn a_stored_schedule_that_cannot_be_read_is_ignored_with_one_masked_line() {
        let good = crate::shared_schedule::encode(&evening(), 100).unwrap();
        let mut later: serde_json::Value = serde_json::from_slice(&good).unwrap();
        later["v"] = serde_json::json!(2);
        let later = serde_json::to_vec(&later).unwrap();
        let secret = br#"{"v":1,"password":"hunter2","sent_at":1}"#.to_vec();
        // Heard at 60: a copy sent more than a day after that can't be right.
        let future = crate::shared_schedule::encode(&evening(), 60 + 86_401).unwrap();
        const TOPIC1: &str = "scent-control/v1/schedule/TESTDEVICE01";
        let cases: [(&str, Vec<u8>, Option<&str>, &str); 7] = [
            // Why is kept as it is logged, so the log never reads the message again.
            (TOPIC1, later, Some(ID1), OTHER_VERSION_SCHEDULE),
            (
                TOPIC1,
                b"{ not json".to_vec(),
                Some(ID1),
                UNREADABLE_SCHEDULE,
            ),
            (TOPIC1, secret, Some(ID1), UNREADABLE_SCHEDULE),
            (TOPIC1, future, Some(ID1), UNREADABLE_SCHEDULE),
            // A good schedule, on a topic that isn't a diffuser's.
            (
                "scent-control/v1/schedule/testdevice01",
                good.clone(),
                None,
                UNREADABLE_SCHEDULE,
            ),
            (
                "scent-control/v1/schedule/TESTDEVICE01/x",
                good.clone(),
                None,
                UNREADABLE_SCHEDULE,
            ),
            (
                "scent-control/v1/something-new",
                good,
                None,
                UNREADABLE_SCHEDULE,
            ),
        ];
        for (topic, body, device_id, action) in cases {
            let mut s = known_air2();
            s.apply_at(sent_profile(ID1, morning(), 50), NOW);
            let before = s.clone();
            s.apply_at(mqtt(topic, &String::from_utf8_lossy(&body), 60), NOW);
            assert_eq!(s.devices, before.devices, "{topic}");
            assert_eq!(s.log.len(), before.log.len() + 1, "{topic}");
            let entry = s.log.back().unwrap();
            assert_eq!(entry.action, action, "{topic}");
            assert_eq!(entry.device_id.as_deref(), device_id, "{topic}");
            assert_eq!(
                (entry.via, entry.direction),
                (Some(Via::Wifi), Direction::FromDevice)
            );
            assert!(!entry.text.contains("hunter2"), "{}", entry.text);
        }
    }

    #[test]
    fn a_cleared_copy_is_ignored_without_a_word() {
        // An empty retained message is how a stored one is taken off the broker.
        let mut s = known_air2();
        s.apply_at(sent_profile(ID1, morning(), 50), NOW);
        let before = s.clone();
        s.apply_at(mqtt("scent-control/v1/schedule/TESTDEVICE01", "", 60), NOW);
        assert_eq!(s, before);
    }

    #[test]
    fn the_echo_of_update_profile_is_not_read_back() {
        let mut s = known_air2();
        let p = profile(true, vec![window("10:24", "10:27")], ScheduleFlag::Replace);
        let body = String::from_utf8(Command::UpdateProfile(p).mqtt_body()).unwrap();
        s.apply_at(mqtt(DEVICE1_TOPIC, &body, 100), NOW);
        assert_eq!(s.devices[ID1].sent_profile, None);
        assert_eq!(s.log.back().unwrap().action, "UPDATE_PROFILE");
    }

    // Broker

    #[test]
    fn login_refused_shows_in_state() {
        let mut s = AppState::default();
        assert_eq!(s.broker, BrokerState::Connecting);
        s.apply(Event::BrokerLoginRefused("Not authorized".into()));
        assert_eq!(s.broker, BrokerState::LoginRefused("Not authorized".into()));
        assert_eq!(s.log.len(), 1);
        let entry = &s.log[0];
        assert!(entry.text.contains("Not authorized"));
        assert_eq!(entry.device_id, None);
        assert!(
            entry.at > 0,
            "an event with no time of its own is stamped now"
        );
    }

    #[test]
    fn broker_changes_are_logged_once_each() {
        let mut s = AppState::default();
        s.apply_at(Event::BrokerConnected, 10);
        s.apply_at(Event::BrokerConnected, 11); // resubscribed on another ConnAck
        assert_eq!(s.broker, BrokerState::Connected);
        assert_eq!(s.log.len(), 1);

        s.apply_at(Event::BrokerDisconnected("connection reset".into()), 20);
        s.apply_at(Event::BrokerDisconnected("connection reset".into()), 25); // retry failed alike
        assert_eq!(
            s.broker,
            BrokerState::Disconnected("connection reset".into())
        );
        assert_eq!(s.log.len(), 2);
        assert_eq!(s.log[1].at, 20);

        s.apply_at(Event::BrokerConnected, 30);
        assert_eq!(s.log.len(), 3);
    }

    #[test]
    fn online_follows_pings_and_what_the_diffuser_says_over_wifi() {
        let mut s = known_air2();
        assert_eq!(s.online(ID1, 100), Online::Unknown);
        assert_eq!(s.online("TESTDEVICE09", 100), Online::Unknown);

        s.apply_at(
            Event::Pinged {
                device_id: ID1.into(),
                at: 100,
            },
            NOW,
        );
        assert_eq!(s.online(ID1, 110), Online::Waiting);
        assert_eq!(s.online(ID1, 116), Online::Offline);

        // Bluetooth says nothing about whether the diffuser answers on WiFi.
        s.apply_at(note(BLE1, "enable:1-|", 120), NOW);
        assert_eq!(s.online(ID1, 121), Online::Offline);

        s.apply_at(report(ID1, "S:1,L:1", 125), NOW);
        assert_eq!(s.online(ID1, 126), Online::Online);
        assert_eq!(s.online(ID1, 125 + 76), Online::Unknown);

        // A ping for a diffuser we don't know is dropped.
        s.apply_at(
            Event::Pinged {
                device_id: ID2.into(),
                at: 130,
            },
            NOW,
        );
        assert!(!s.devices.contains_key(ID2));
    }

    // Log

    #[test]
    fn log_is_capped_at_500() {
        let mut s = AppState::default();
        for at in 0..600 {
            s.apply_at(failed(None, "x", "y", at), NOW);
        }
        assert_eq!(s.log.len(), 500);
        assert_eq!(LOG_CAP, 500);
        assert_eq!(s.log.front().unwrap().at, 100);
        assert_eq!(s.log.back().unwrap().at, 599);
    }

    #[test]
    fn log_masks_passwords_in_echoed_commands() {
        // The app subscribes to everything under the prefix, so it hears its own
        // SAVE_MQTT and WIFI_CONNECT go out, passwords and all.
        let secret = || Secret::new("hunter2!".into());
        let wifi = Command::WifiConnect {
            ssid: "ExampleWiFi".into(),
            password: secret(),
        };
        let save = Command::SaveMqtt {
            broker: "wss://broker.example.test/mqtt".into(),
            username: "scent-test".into(),
            password: secret(),
        };
        let mut s = known_air2();
        for (n, command) in [wifi, save].iter().enumerate() {
            let body = String::from_utf8(command.mqtt_body()).unwrap();
            assert!(body.contains("hunter2!"), "the device does need it");
            s.apply_at(mqtt(DEVICE1_TOPIC, &body, 100 + n as i64), NOW);
        }
        // Not valid JSON, but the password must still not get through.
        let broken = r#"{"action":"WIFI_CONNECT","payload":{"data":oops,"password":"hunter2!"}}"#;
        s.apply_at(mqtt(DEVICE1_TOPIC, broken, 103), NOW);

        let echoes: Vec<&LogEntry> = s
            .log
            .iter()
            .filter(|e| e.direction == Direction::ToDevice)
            .collect();
        assert_eq!(echoes.len(), 3);
        for entry in echoes {
            assert!(!entry.text.contains("hunter2!"), "{}", entry.text);
            assert!(entry.text.contains(r#""password":"***""#), "{}", entry.text);
        }
        assert!(!format!("{s:?}").contains("hunter2!"));
    }

    #[test]
    fn mask_passwords_replaces_every_password_value() {
        let masked = |text: &str| mask_passwords(text);
        assert_eq!(
            masked(
                r#"{"action":"WIFI_CONNECT","payload":{"ssid":"ExampleWiFi","password":"hunter2!"}}"#
            ),
            r#"{"action":"WIFI_CONNECT","payload":{"ssid":"ExampleWiFi","password":"***"}}"#
        );
        // Spaces, any case, escapes inside the value, a value that is not a string.
        assert_eq!(
            masked(r#"{"Password" : "a\"b\\"}"#),
            r#"{"Password" : "***"}"#
        );
        assert_eq!(
            masked(r#"{"password":12345,"x":1}"#),
            r#"{"password":"***","x":1}"#
        );
        assert_eq!(masked(r#"{"password":null}"#), r#"{"password":"***"}"#);
        // A longer key name that ends or starts with "password".
        assert_eq!(
            masked(r#"{"wifi_password":"x","mqttPassword":"y","password_hint":"z"}"#),
            r#"{"wifi_password":"***","mqttPassword":"***","password_hint":"***"}"#
        );
        // Every occurrence, nested ones included.
        assert_eq!(
            masked(r#"{"a":{"password":"1"},"b":[{"password":"2"}],"password":"3"}"#),
            r#"{"a":{"password":"***"},"b":[{"password":"***"}],"password":"***"}"#
        );
    }

    #[test]
    fn mask_passwords_works_on_text_that_is_not_valid_json() {
        let masked = |text: &str| mask_passwords(text);
        assert_eq!(
            masked(r#"{"data":enable:0,"password":"p@ss"}"#),
            r#"{"data":enable:0,"password":"***"}"#
        );
        assert_eq!(
            masked(r#"{"password":hunter2,"x":1}"#),
            r#"{"password":"***","x":1}"#
        );
        // Cut off in the middle of the value: what is there is hidden.
        assert_eq!(masked(r#"{"password":"abc"#), r#"{"password":"***""#);
        assert_eq!(
            masked(r#"password:"abc""#),
            r#"password:"abc""#,
            "no quoted key, no match"
        );
        // JSON inside a JSON string, as the quotes then arrive escaped.
        assert_eq!(
            masked(r#"{"msg":"{\"password\":\"hunter2\",\"x\":1}"}"#),
            r#"{"msg":"{\"password\":\"***\",\"x\":1}"}"#
        );
        assert_eq!(
            masked(r#"{"msg":"{\"password\":\"a\\\"b\",\"x\":1}"}"#),
            r#"{"msg":"{\"password\":\"***\",\"x\":1}"}"#
        );
        // Non-ASCII text around the value is kept whole.
        assert_eq!(
            masked(r#"{"note":"héllo","password":"pässwörd","z":"ü"}"#),
            r#"{"note":"héllo","password":"***","z":"ü"}"#
        );
    }

    #[test]
    fn mask_passwords_leaves_other_text_alone() {
        for text in [
            "",
            "S:1,L:3,C:2,Bat:200",
            r#"{"action":"PING"}"#,
            r#"{"message":"password reset"}"#,
            r#"{"message":"password"}"#,
            r#"{"hint":"your password: unknown"}"#,
            "password",
        ] {
            assert_eq!(mask_passwords(text), text);
        }
        // Masking twice is the same as masking once (summaries arrive already redacted).
        let once = mask_passwords(r#"{"ssid":"x","password":"hunter2!"}"#);
        assert_eq!(mask_passwords(&once), once);
    }

    // Routine traffic

    /// A `PING` from this app, as the broker hands it back on the diffuser's topic.
    fn our_ping(at: i64) -> Event {
        let ping = Command::Ping {
            userid: Some("route-test-1".to_owned()),
        };
        Event::Mqtt {
            topic: DEVICE1_TOPIC.to_owned(),
            body: ping.mqtt_body(),
            at,
        }
    }

    #[test]
    fn a_ping_round_is_not_logged_but_the_diffuser_is_still_heard() {
        let mut s = AppState::default();
        s.apply_at(our_ping(100), NOW);
        s.apply_at(fixture_event(ACK_ALREADY_ADDED, 101), NOW);
        // Neither the ping nor its answer fills the log...
        assert!(s.log.is_empty(), "{:?}", s.log);
        // ...but the answer is still the diffuser being heard on our broker.
        let device = &s.devices[ID1];
        assert_eq!(device.heard_wifi, Some(101));
        // And the ping we sent says nothing about the diffuser.
        let mut s = AppState::default();
        s.apply_at(our_ping(100), NOW);
        assert!(s.devices.is_empty());
        // A round from a diffuser already in the log leaves the log as it was.
        let mut s = AppState::default();
        s.apply_at(fixture_event(UPDATE_STATUS, 90), NOW);
        let before = s.log.len();
        for round in 0..3 {
            s.apply_at(our_ping(100 + round * 30), NOW);
            s.apply_at(fixture_event(ACK_ALREADY_ADDED, 101 + round * 30), NOW);
        }
        assert_eq!(s.log.len(), before);
        assert_eq!(s.devices[ID1].heard_wifi, Some(161));
    }

    #[test]
    fn the_first_registration_and_every_other_reply_are_still_logged() {
        let mut s = AppState::default();
        s.apply_at(fixture_event(ACK_ADDED, 100), NOW);
        s.apply_at(fixture_event(UPDATE_STATUS, 101), NOW);
        let ack = |msg: &str| {
            let body = serde_json::json!({
                "action": "acknowledgement",
                "payload": { "deviceId": ID1, "msg": msg },
            });
            mqtt(USER_TOPIC, &body.to_string(), 102)
        };
        // Another acknowledgement with a result, and one that isn't JSON.
        s.apply_at(
            ack(r#"{"user_id":"route-test-1","result":"BATTERY_MODE"}"#),
            NOW,
        );
        s.apply_at(ack("Credentials saved"), NOW);
        // A command that isn't a ping, and a ping that isn't an acknowledgement.
        s.apply_at(
            Event::Mqtt {
                topic: DEVICE1_TOPIC.to_owned(),
                body: Command::TurnOn { on: true }.mqtt_body(),
                at: 103,
            },
            NOW,
        );
        s.apply_at(mqtt(USER_TOPIC, r#"{"action":"PING"}"#, 104), NOW);
        let actions: Vec<&str> = s.log.iter().map(|entry| entry.action.as_str()).collect();
        assert_eq!(
            actions,
            [
                "acknowledgement",
                "update_status",
                "acknowledgement",
                "acknowledgement",
                "TURN_ON",
                "PING"
            ]
        );
    }

    // Adding a diffuser

    /// `device_connected` on the topic it uses before it knows a user ID.
    const CONNECTED_FIRST: &str = include_str!(
        "../../../fixtures/scentiment/air2-mqtt-device-connected-server-notifications.json"
    );
    /// `device_connected` on our user topic, after a soft restart.
    const CONNECTED_AGAIN: &str =
        include_str!("../../../fixtures/scentiment/air2-mqtt-device-connected-user.json");

    #[test]
    fn device_connected_on_either_topic_says_when_it_joined() {
        for fixture in [CONNECTED_FIRST, CONNECTED_AGAIN] {
            let mut s = AppState::default();
            s.apply_at(fixture_event(fixture, 700), NOW);
            assert_eq!(s.devices[ID1].connected_at, Some(700));
            // A later one moves it on.
            s.apply_at(fixture_event(fixture, 800), NOW);
            assert_eq!(s.devices[ID1].connected_at, Some(800));
        }
        // Nothing else does: a status report, our own ping, the info over Bluetooth.
        let mut s = known_air2();
        s.apply_at(fixture_event(UPDATE_STATUS, 900), NOW);
        s.apply_at(our_ping(901), NOW);
        s.apply_at(fixture_event(ACK_ALREADY_ADDED, 902), NOW);
        s.apply_at(ble_info(BLE1, ID1, 903), NOW);
        assert_eq!(s.devices[ID1].connected_at, None);
    }

    #[test]
    fn the_info_read_over_bluetooth_says_when_it_was_read() {
        let mut s = AppState::default();
        s.apply_at(ble_info(BLE1, ID1, 10), NOW);
        assert_eq!(s.devices[ID1].info_at, Some(10));
        // Hearing it otherwise is not reading its info.
        s.apply_at(note(BLE1, "S:1,L:2", 20), NOW);
        s.apply_at(fixture_event(CONNECTED_AGAIN, 30), NOW);
        assert_eq!(s.devices[ID1].info_at, Some(10));
        s.apply_at(ble_info(BLE1, ID1, 40), NOW);
        assert_eq!(s.devices[ID1].info_at, Some(40));
    }

    #[test]
    fn credentials_saved_is_heard_over_bluetooth_or_the_broker() {
        // Over Bluetooth, from the diffuser it is linked to.
        let mut s = known_air2();
        s.apply_at(note(BLE1, "Credentials saved-|", 50), NOW);
        assert_eq!(s.devices[ID1].credentials_saved_at, Some(50));
        // As an acknowledgement on the broker.
        let body = serde_json::json!({
            "action": "acknowledgement",
            "payload": { "deviceId": ID1, "msg": "Credentials saved" },
        });
        let mut s = known_air2();
        s.apply_at(mqtt(USER_TOPIC, &body.to_string(), 60), NOW);
        assert_eq!(s.devices[ID1].credentials_saved_at, Some(60));
        // Other notifications and acknowledgements are not it.
        let mut s = known_air2();
        s.apply_at(note(BLE1, "enable:1", 70), NOW);
        s.apply_at(fixture_event(ACK_ADDED, 71), NOW);
        s.apply_at(fixture_event(ACK_ALREADY_ADDED, 72), NOW);
        assert_eq!(s.devices[ID1].credentials_saved_at, None);
        // Nor is our own SAVE_MQTT, as the broker hands it back.
        let save = Command::SaveMqtt {
            broker: "wss://broker.example.test/mqtt".into(),
            username: "user".into(),
            password: Secret::new("x".into()),
        };
        s.apply_at(
            Event::Mqtt {
                topic: DEVICE1_TOPIC.to_owned(),
                body: save.mqtt_body(),
                at: 73,
            },
            NOW,
        );
        assert_eq!(s.devices[ID1].credentials_saved_at, None);
    }

    fn adding(ble_id: &str, step: AddStep) -> Event {
        Event::Adding {
            ble_id: ble_id.to_owned(),
            step,
        }
    }

    #[test]
    fn an_add_follows_its_steps_and_names_the_diffuser_when_done() {
        let mut s = AppState::default();
        s.begin_add(BLE1, "Bedroom", "", None);
        let current = s.adding[BLE1].clone();
        assert_eq!(
            (current.ble_id.as_str(), current.label.as_str()),
            (BLE1, "Bedroom")
        );
        assert_eq!(current.network, None);
        assert_eq!((current.step, current.reached), (AddStep::ReadInfo, None));

        s.apply_at(adding(BLE1, AddStep::ReadInfo), NOW);
        // Reading the info is what makes it a known diffuser, under its ID.
        s.apply_at(ble_info(BLE1, ID1, 10), NOW);
        assert_eq!(s.devices[ID1].label, ID1);
        for step in [AddStep::SaveMqtt, AddStep::Restart, AddStep::WaitConnected] {
            s.apply_at(adding(BLE1, step.clone()), NOW);
            let current = &s.adding[BLE1];
            assert_eq!(current.step, step);
            assert_eq!(current.reached.as_ref(), Some(&step));
        }
        s.apply_at(adding(BLE1, AddStep::Done(ID1.to_owned())), NOW);
        assert_eq!(s.devices[ID1].label, "Bedroom");
        let current = &s.adding[BLE1];
        assert_eq!(current.step, AddStep::Done(ID1.to_owned()));
        assert_eq!(current.reached, Some(AddStep::WaitConnected));
    }

    #[test]
    fn an_add_with_no_name_keeps_the_device_id() {
        let mut s = known_air2();
        s.begin_add(BLE1, "", "", None);
        s.apply_at(adding(BLE1, AddStep::Done(ID1.to_owned())), NOW);
        assert_eq!(s.devices[ID1].label, ID1);
    }

    #[test]
    fn a_failed_add_says_why_in_the_log_and_where_it_stopped() {
        let mut s = known_air2();
        let before = s.log.len();
        s.begin_add(BLE1, "Bedroom", "", None);
        s.apply_at(adding(BLE1, AddStep::SaveMqtt), NOW);
        s.apply_at(adding(BLE1, AddStep::Failed("No answer.".into())), NOW);
        let current = &s.adding[BLE1];
        assert_eq!(current.step, AddStep::Failed("No answer.".into()));
        assert_eq!(current.reached, Some(AddStep::SaveMqtt));
        assert_eq!(s.devices[ID1].label, ID1);
        let logged = s.log.iter().skip(before).collect::<Vec<_>>();
        assert_eq!(logged.len(), 1, "{logged:?}");
        assert_eq!(logged[0].action, "Add");
        assert_eq!(logged[0].text, "No answer.");
        assert_eq!(logged[0].device_id.as_deref(), Some(ID1));
        assert_eq!(logged[0].via, None);
        // A new add starts over.
        s.begin_add(BLE1, "Kitchen", "", Some("ExampleWiFi"));
        let current = &s.adding[BLE1];
        assert_eq!(
            (current.step.clone(), current.reached.clone()),
            (AddStep::ReadInfo, None)
        );
        assert_eq!(current.label, "Kitchen");
        // It remembers the network it was given, though never its password.
        assert_eq!(current.network.as_deref(), Some("ExampleWiFi"));
    }

    #[test]
    fn two_adds_are_followed_side_by_side_and_both_logged() {
        const BLE2: &str = "ble-peripheral-2";
        let mut s = AppState::default();
        s.begin_add(BLE1, "Bedroom", "", None);
        s.begin_add(BLE2, "Hallway", "", None);
        s.apply_at(ble_info(BLE1, ID1, 10), NOW);
        s.apply_at(ble_info(BLE2, ID2, 11), NOW);
        let before = s.log.len();
        s.apply_at(adding(BLE1, AddStep::WaitConnected), NOW);
        s.apply_at(adding(BLE2, AddStep::SaveMqtt), NOW);
        s.apply_at(adding(BLE1, AddStep::Failed("No.".into())), NOW);
        s.apply_at(adding(BLE2, AddStep::Done(ID2.to_owned())), NOW);

        assert_eq!(s.adding[BLE1].step, AddStep::Failed("No.".into()));
        assert_eq!(s.adding[BLE2].step, AddStep::Done(ID2.to_owned()));
        // Each takes its own name, and only when it is done.
        assert_eq!(s.devices[ID1].label, ID1);
        assert_eq!(s.devices[ID2].label, "Hallway");
        // Both outcomes are in the log: the failure as one, the add as heard on the
        // broker, where it showed up.
        let logged: Vec<_> = s.log.iter().skip(before).collect();
        assert_eq!(logged.len(), 2, "{logged:?}");
        assert!(logged.iter().all(|entry| entry.action == "Add"));
        assert_eq!(
            (logged[0].device_id.as_deref(), logged[0].via),
            (Some(ID1), None)
        );
        assert_eq!(
            (
                logged[1].device_id.as_deref(),
                logged[1].via,
                logged[1].direction
            ),
            (Some(ID2), Some(Via::Wifi), Direction::FromDevice)
        );
        // Progress is not logged.
        assert!(!logged.iter().any(|entry| entry.text.contains("SaveMqtt")));
    }

    #[test]
    fn an_add_under_way_is_not_started_over() {
        let mut s = known_air2();
        s.begin_add(BLE1, "Bedroom", "", None);
        s.apply_at(adding(BLE1, AddStep::WaitConnected), NOW);
        s.begin_add(BLE1, "Kitchen", "", None);
        let current = &s.adding[BLE1];
        assert_eq!(current.step, AddStep::WaitConnected);
        assert_eq!(current.label, "Bedroom");
    }

    #[test]
    fn an_add_puts_the_diffuser_in_its_room_when_done() {
        let mut s = known_air2();
        s.edit(Edit::Room {
            device_id: ID1.into(),
            room: "Office".into(),
        });
        s.begin_add(BLE1, "Bedside", " bedroom ", None);
        assert_eq!(s.adding[BLE1].room.as_deref(), Some(" bedroom "));
        // Not before it is done.
        s.apply_at(adding(BLE1, AddStep::WaitConnected), NOW);
        assert_eq!(s.devices[ID1].room.as_deref(), Some("Office"));
        s.apply_at(adding(BLE1, AddStep::Done(ID1.to_owned())), NOW);
        assert_eq!(s.devices[ID1].room.as_deref(), Some("bedroom"));
        // The sheet starts from the room it is in: emptied, it takes it out of it.
        s.begin_add(BLE1, "Bedside", "", None);
        s.apply_at(adding(BLE1, AddStep::Done(ID1.to_owned())), NOW);
        assert_eq!(s.devices[ID1].room, None);
    }

    #[test]
    fn a_name_or_room_changed_in_details_while_an_add_waits_is_kept_when_it_is_done() {
        let mut s = known_air2();
        let edit = |s: &mut AppState, name: &str, room: &str| {
            let device_id = ID1.to_owned();
            s.edit(Edit::Name {
                device_id: device_id.clone(),
                name: name.into(),
            });
            s.edit(Edit::Room {
                device_id,
                room: room.into(),
            });
        };
        edit(&mut s, "Bedroom", "Upstairs");
        // Set Up Again…, as the sheet opens it, then a rename and a move while it waits:
        // the most recent change is the person's, and wins.
        s.begin_add(BLE1, "Bedroom", "Upstairs", None);
        s.apply_at(adding(BLE1, AddStep::WaitConnected), NOW);
        edit(&mut s, "Bedside", "Office");
        s.apply_at(adding(BLE1, AddStep::Done(ID1.to_owned())), NOW);
        assert_eq!(s.devices[ID1].label, "Bedside");
        assert_eq!(s.devices[ID1].room.as_deref(), Some("Office"));
        // Unchanged since, they take what the sheet said.
        s.begin_add(BLE1, "Nightstand", "Bedroom", None);
        s.apply_at(adding(BLE1, AddStep::Done(ID1.to_owned())), NOW);
        assert_eq!(s.devices[ID1].label, "Nightstand");
        assert_eq!(s.devices[ID1].room.as_deref(), Some("Bedroom"));
    }

    #[test]
    fn a_new_diffuser_named_in_details_while_it_is_added_keeps_that_name() {
        let mut s = AppState::default();
        s.begin_add(BLE1, "Mini", "Office", None);
        // Its info is read: it is in the sidebar, and can be renamed there.
        s.apply_at(ble_info(BLE1, ID1, 10), NOW);
        s.edit(Edit::Name {
            device_id: ID1.into(),
            name: "Desk".into(),
        });
        s.apply_at(adding(BLE1, AddStep::Done(ID1.to_owned())), NOW);
        assert_eq!(s.devices[ID1].label, "Desk");
        // Its room was not changed since: the sheet's is given.
        assert_eq!(s.devices[ID1].room.as_deref(), Some("Office"));
    }

    #[test]
    fn an_add_the_screens_did_not_start_leaves_the_room_alone() {
        let mut s = known_air2();
        s.edit(Edit::Room {
            device_id: ID1.into(),
            room: "Office".into(),
        });
        s.apply_at(adding(BLE1, AddStep::Done(ID1.to_owned())), NOW);
        assert_eq!(s.devices[ID1].room.as_deref(), Some("Office"));
        assert_eq!(s.devices[ID1].label, ID1);
    }

    // Names and rooms

    #[test]
    fn a_typed_name_is_tidied() {
        assert_eq!(clean_name("  Bedside  "), "Bedside");
        assert_eq!(clean_name("Living \t  Room"), "Living Room");
        assert_eq!(clean_name("Bed\u{7}side"), "Bedside");
        assert_eq!(clean_name(" \n "), "");
        // At most 40 characters, counted as characters rather than bytes.
        let long = "Ä".repeat(50);
        assert_eq!(clean_name(&long), "Ä".repeat(NAME_MAX_CHARS));
        assert_eq!(NAME_MAX_CHARS, 40);
        // Cut at a space, it doesn't end with one.
        let spaced = format!("{} b", "a".repeat(39));
        assert_eq!(clean_name(&spaced), "a".repeat(39));
    }

    #[test]
    fn a_rename_is_tidied_and_an_empty_name_goes_back_to_none() {
        let mut s = known_air2();
        let rename = |s: &mut AppState, id: &str, name: &str| {
            s.edit(Edit::Name {
                device_id: id.into(),
                name: name.into(),
            });
        };
        rename(&mut s, ID1, "  Bedside  ");
        assert_eq!(s.devices[ID1].label, "Bedside");
        // No name: the label is its ID again, which the screens never show.
        rename(&mut s, ID1, "   ");
        assert_eq!(s.devices[ID1].label, ID1);
        // A diffuser this app doesn't know is not made up.
        rename(&mut s, ID2, "Hallway");
        assert!(!s.devices.contains_key(ID2));
    }

    #[test]
    fn a_room_takes_the_spelling_of_one_in_use_and_an_empty_room_is_none() {
        const ID3: &str = "TESTDEVICE03";
        let mut s = AppState::default();
        for id in [ID1, ID2, ID3] {
            s.devices.insert(id.to_owned(), DeviceView::new(id));
        }
        let room = |s: &mut AppState, id: &str, room: &str| {
            s.edit(Edit::Room {
                device_id: id.into(),
                room: room.into(),
            });
        };
        room(&mut s, ID1, " Living  Room ");
        assert_eq!(s.devices[ID1].room.as_deref(), Some("Living Room"));
        // Typed in other capitals, it is the same room.
        room(&mut s, ID2, "living room");
        assert_eq!(s.devices[ID2].room.as_deref(), Some("Living Room"));
        // The only diffuser in a room can change how it is spelled.
        room(&mut s, ID3, "office");
        room(&mut s, ID3, "Office");
        assert_eq!(s.devices[ID3].room.as_deref(), Some("Office"));
        room(&mut s, ID1, "  ");
        assert_eq!(s.devices[ID1].room, None);
        room(&mut s, "TESTDEVICE09", "Office");
        assert!(!s.devices.contains_key("TESTDEVICE09"));
    }

    #[test]
    fn the_rooms_in_use_are_listed_once_in_alphabetical_order() {
        let mut s = AppState::default();
        assert!(s.rooms().is_empty());
        // As a hand-edited file could have them: the first spelling, by device ID, wins.
        for (id, room) in [
            (ID1, Some("Office")),
            (ID2, Some("bedroom")),
            ("TESTDEVICE03", Some("Bedroom")),
            ("TESTDEVICE04", None),
            ("TESTDEVICE05", Some("Attic")),
        ] {
            let mut device = DeviceView::new(id);
            device.room = room.map(str::to_owned);
            s.devices.insert(id.to_owned(), device);
        }
        assert_eq!(s.rooms(), ["Attic", "bedroom", "Office"]);
    }

    #[test]
    fn an_add_the_screens_did_not_start_is_still_followed() {
        let mut s = known_air2();
        s.apply_at(adding(BLE1, AddStep::Failed("No.".into())), NOW);
        assert_eq!(s.adding[BLE1].step, AddStep::Failed("No.".into()));
        assert_eq!(s.log.back().unwrap().action, "Add");
    }

    // A diffuser can repeat a command back over Bluetooth or the broker. When that is a
    // WIFI_CONNECT or a SAVE_MQTT, the password in it never reaches the log or the state.
    #[test]
    fn echoed_wifi_and_broker_logins_are_masked() {
        let echoes = [
            Command::WifiConnect {
                ssid: "ExampleWiFi".into(),
                password: Secret::new("hunter2".into()),
            },
            Command::SaveMqtt {
                broker: "wss://broker.example.test/mqtt".into(),
                username: "scent-test".into(),
                password: Secret::new("hunter2".into()),
            },
        ];
        let mut s = known_air2();
        for (n, echo) in (0..).zip(echoes) {
            // Over Bluetooth, as the frame was written, and as a JSON string inside a
            // notification.
            s.apply_at(
                Event::BleNotification {
                    ble_id: BLE1.to_owned(),
                    bytes: echo.ble_frame(),
                    at: 100 + n,
                },
                NOW,
            );
            let quoted =
                serde_json::to_string(&String::from_utf8(echo.mqtt_body()).unwrap()).unwrap();
            s.apply_at(note(BLE1, &format!(r#"{{"data":{quoted}}}"#), 110 + n), NOW);
            // On the broker: on the diffuser's topic, and in an acknowledgement from it.
            s.apply_at(
                Event::Mqtt {
                    topic: DEVICE1_TOPIC.to_owned(),
                    body: echo.mqtt_body(),
                    at: 120 + n,
                },
                NOW,
            );
            let ack = serde_json::json!({
                "action": "acknowledgement",
                "payload": { "deviceId": ID1, "msg": String::from_utf8(echo.mqtt_body()).unwrap() },
            });
            s.apply_at(mqtt(USER_TOPIC, &ack.to_string(), 130 + n), NOW);
        }
        let everything = format!("{s:?}");
        assert!(!everything.contains("hunter2"), "{everything}");
        let masked = s
            .log
            .iter()
            .filter(|entry| entry.text.contains("***"))
            .count();
        assert_eq!(masked, 8, "{:?}", s.log);
        // The rest of what was echoed is kept.
        assert!(s.log.iter().any(|entry| entry.text.contains("ExampleWiFi")));
        assert!(s.log.iter().any(|entry| entry.text.contains("scent-test")));
    }

    // Local store

    #[test]
    fn store_round_trips_devices_and_survives_a_corrupt_file() {
        let dir = TempDir::new("round-trip");
        let store = Store {
            dir: dir.path().to_owned(),
        };

        let mut s = AppState::default();
        s.apply_at(ble_info(BLE1, ID1, 10), NOW);
        s.apply_at(fixture_event(UPDATE_STATUS, 1_000), NOW);
        s.apply_at(note(BLE1, "enable:0-|", 2_000), NOW);
        s.apply_at(report(ID2, "S:1,L:1", 3_000), NOW);
        s.devices.get_mut(ID1).unwrap().label = "Bedroom".into();
        // The last schedule sent is remembered with the diffuser, and when it went.
        s.apply_at(
            sent_profile(ID1, profile(true, vec![], ScheduleFlag::Replace), 4_002),
            NOW,
        );
        // Things that are live, not remembered.
        s.apply_at(
            Event::Pinged {
                device_id: ID1.into(),
                at: 4_000,
            },
            NOW,
        );
        s.apply_at(sent(ID1, Some(true), None, 4_001), NOW);
        s.apply_at(Event::BrokerConnected, NOW);
        s.apply_at(fixture_event(CONNECTED_AGAIN, 4_003), NOW);
        s.apply_at(note(BLE1, "Credentials saved", 4_004), NOW);
        s.begin_add(BLE1, "Bedroom", "", Some("ExampleWiFi"));

        store.save(&s, &BrokerSettings::default()).unwrap();
        let loaded = store.load().state;

        let (before, after) = (&s.devices[ID1], &loaded.devices[ID1]);
        assert_eq!(after.id, ID1);
        assert_eq!(after.label, "Bedroom");
        assert_eq!(after.ble_id, before.ble_id);
        assert_eq!(after.info, before.info);
        assert_eq!(after.readings, before.readings);
        assert_eq!(after.readings[&Field::Power].via, Via::Bluetooth);
        assert_eq!(after.readings[&Field::Charging].via, Via::Wifi);
        assert_eq!(loaded.devices[ID2].readings, s.devices[ID2].readings);
        assert_eq!(loaded.devices[ID2].ble_id, None);
        assert_eq!(
            (after.heard_wifi, after.heard_ble, after.ping_at),
            (None, None, None)
        );
        assert!(after.sent.is_none());
        assert!(before.sent_profile.is_some());
        assert_eq!(after.sent_profile, before.sent_profile);
        assert_eq!(after.sent_profile_at, Some(4_002));
        assert_eq!(loaded.devices[ID2].sent_profile, None);
        assert_eq!(
            (
                after.connected_at,
                after.info_at,
                after.credentials_saved_at
            ),
            (None, None, None)
        );
        assert!(loaded.adding.is_empty());
        assert!(loaded.log.is_empty() && loaded.scan.is_empty());
        assert_eq!(loaded.broker, BrokerState::Connecting);

        // A file that is not JSON: an empty state, and the file kept aside.
        let file = dir.path().join("desktop.json");
        fs::write(&file, "{").unwrap();
        assert_eq!(store.load().state, AppState::default());
        assert_eq!(
            fs::read_to_string(dir.path().join("desktop.json.corrupt")).unwrap(),
            "{"
        );
        assert!(!file.exists());
    }
}
