//! Scent Control core: pure protocol logic with no I/O.
//!
//! The [`Command`] enum is the allow-list. It has no variant for firmware
//! updates, hard or factory resets, light commands or free-form JSON.

mod command;
mod level;
mod message;
mod names;
mod topics;

pub use command::{BLE_CHUNK, Command, Secret, Via, chunks};
pub use level::Level;
pub use message::{
    Ack, Connected, DeviceInfo, Direction, MqttMessage, NoteKind, Notification, Status, parse_info,
    parse_mqtt, parse_notification,
};
pub use names::{NameInfo, device_id_from_name, is_scentiment_name, parse_name};
pub use topics::{DEVICE_PREFIX, SUBSCRIBE_ALL, device_topic, valid_device_id};
