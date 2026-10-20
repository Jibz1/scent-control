//! Scent Control core: pure protocol logic with no I/O.
//!
//! The [`Command`] enum is the allow-list. It has no variant for firmware
//! updates, hard or factory resets, light commands or free-form JSON.

mod command;
mod level;

pub use command::{BLE_CHUNK, Command, Secret, Via, chunks};
pub use level::Level;
