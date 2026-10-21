//! Scent Control core: pure protocol logic with no I/O.
//!
//! The [`Command`] enum is the allow-list. It has no variant for firmware
//! updates, hard or factory resets, light commands or free-form JSON.

mod command;
mod level;
mod message;
mod names;
mod schedule;
mod status;
mod topics;

pub use command::{BLE_CHUNK, Command, Secret, Via, chunks};
pub use level::Level;
pub use message::{
    Ack, Connected, DeviceInfo, Direction, MqttMessage, NoteKind, Notification, Status, parse_info,
    parse_mqtt, parse_notification,
};
pub use names::{NameInfo, device_id_from_name, is_scentiment_name, parse_name};
pub use schedule::{
    AlwaysOn, Entry, Expected, HhMm, Profile, ScheduleFlag, Source, expected_state,
    local_weekday_and_time, trial_entry,
};
pub use status::{
    ANSWER_WITHIN_S, ONLINE_FOR_S, Online, PING_EVERY_S, STALE_AFTER_S, is_stale, online_status,
};
pub use topics::{DEVICE_PREFIX, SUBSCRIBE_ALL, device_topic, valid_device_id};
