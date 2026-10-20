//! The log: the conversation with the diffusers, newest last.

use eframe::egui::{self, Color32, RichText};
use scent_core::{Direction, Via};

use super::format::{clock_hms, local_utc_offset_secs, via_name};
use super::{amber, subtle_fill};
use crate::state::{AppState, LogEntry};

/// How many lines the log shows. The state keeps more (see `state::LOG_CAP`).
const SHOWN: usize = 300;

/// A WiFi command and the broker's copy of it are one message when they are this many
/// seconds apart at most.
const ECHO_WITHIN_S: u64 = 5;

/// Height of the log when it is open, before the person drags it.
const OPEN_HEIGHT: f32 = 220.0;

/// Whether this entry is a command that went out over WiFi, or the broker's copy of one.
fn is_wifi_command(entry: &LogEntry) -> bool {
    entry.direction == Direction::ToDevice
        && entry.via == Some(Via::Wifi)
        && entry.device_id.is_some()
}

fn is_echo_of(first: &LogEntry, other: &LogEntry) -> bool {
    is_wifi_command(other)
        && other.device_id == first.device_id
        && other.action == first.action
        && other.at.abs_diff(first.at) <= ECHO_WITHIN_S
}

/// The log with each command we sent over WiFi and its echo as one line.
///
/// We subscribe to everything under the Scentiment prefix, so the broker hands our own
/// publish back to us, and the log would show every WiFi command twice: once as sent
/// and once as heard. The echo is the other `ToDevice` WiFi entry for the same device
/// and action within [`ECHO_WITHIN_S`]; each hides at most one, and the one that comes
/// first stays. Everything else is kept, in order.
pub fn collapse_echoes<'a>(entries: impl IntoIterator<Item = &'a LogEntry>) -> Vec<&'a LogEntry> {
    let entries: Vec<&LogEntry> = entries.into_iter().collect();
    let mut hidden = vec![false; entries.len()];
    for (i, first) in entries.iter().enumerate() {
        if hidden[i] || !is_wifi_command(first) {
            continue;
        }
        let echo = (i + 1..entries.len())
            .take_while(|&j| entries[j].at.saturating_sub(first.at) <= ECHO_WITHIN_S as i64)
            .find(|&j| !hidden[j] && is_echo_of(first, entries[j]));
        if let Some(j) = echo {
            hidden[j] = true;
        }
    }
    entries
        .into_iter()
        .zip(hidden)
        .filter_map(|(entry, hidden)| (!hidden).then_some(entry))
        .collect()
}

/// How a line is told apart from the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Something we sent.
    Sent,
    /// Something the diffuser or the broker said.
    Heard,
    /// Something that went wrong. These are `ToDevice` with no `via`: nothing went out.
    Problem,
}

fn kind(entry: &LogEntry) -> Kind {
    match (entry.direction, entry.via) {
        (Direction::FromDevice, _) => Kind::Heard,
        (Direction::ToDevice, Some(_)) => Kind::Sent,
        (Direction::ToDevice, None) => Kind::Problem,
    }
}

/// Who a line is about: "You to Bedroom over WiFi", "Bedroom over Bluetooth", or just
/// "Broker" or the diffuser's name for a note of the app's own.
fn who(entry: &LogEntry, subject: &str) -> String {
    match (kind(entry), entry.via, &entry.device_id) {
        (Kind::Sent, Some(via), _) => format!("You to {subject} over {}", via_name(via)),
        (Kind::Heard, Some(via), Some(_)) => format!("{subject} over {}", via_name(via)),
        _ => subject.to_owned(),
    }
}

/// The name a line goes by: the diffuser's, or "Broker" for what the broker did, or
/// "App" for what the app itself reports.
fn subject(entry: &LogEntry, state: &AppState) -> String {
    match &entry.device_id {
        Some(id) => state
            .devices
            .get(id)
            .map_or_else(|| id.clone(), |device| device.label.clone()),
        None if entry.action == "broker" => "Broker".to_owned(),
        None => "App".to_owned(),
    }
}

/// What the log screen remembers.
pub struct LogView {
    open: bool,
    only_selected: bool,
}

impl Default for LogView {
    fn default() -> LogView {
        // Open: a failed command is only explained here.
        LogView {
            open: true,
            only_selected: false,
        }
    }
}

impl LogView {
    /// The log along the bottom of the window. Add it before the panels beside it.
    pub fn show(&mut self, ui: &mut egui::Ui, state: &AppState, selected: Option<&str>, now: i64) {
        // Separate ids, so that the height the person dragged to is kept apart from
        // the one-line height when it is shut.
        let panel = if self.open {
            egui::Panel::bottom("log_open")
                .resizable(true)
                .default_size(OPEN_HEIGHT)
                .min_size(80.0)
        } else {
            egui::Panel::bottom("log_shut").resizable(false)
        };
        panel.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Conversation").strong());
                let label = if self.open { "Hide" } else { "Show" };
                if ui.button(label).clicked() {
                    self.open = !self.open;
                }
                if self.open {
                    ui.checkbox(&mut self.only_selected, "only the selected diffuser");
                }
            });
            if self.open {
                ui.separator();
                self.lines(ui, state, selected, now);
            }
        });
    }

    fn lines(&self, ui: &mut egui::Ui, state: &AppState, selected: Option<&str>, now: i64) {
        let only = self.only_selected.then_some(selected).flatten();
        let wanted = state
            .log
            .iter()
            .filter(|entry| only.is_none_or(|id| entry.device_id.as_deref() == Some(id)));
        let lines = collapse_echoes(wanted);
        if lines.is_empty() {
            ui.weak("Nothing yet. Commands you send and everything the diffusers say appear here.");
            return;
        }
        let offset = local_utc_offset_secs(now);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for entry in &lines[lines.len().saturating_sub(SHOWN)..] {
                    line(ui, entry, state, offset);
                }
            });
    }
}

fn line(ui: &mut egui::Ui, entry: &LogEntry, state: &AppState, offset: i64) {
    let fill = match kind(entry) {
        Kind::Sent => subtle_fill(ui),
        Kind::Problem => amber(ui).gamma_multiply(0.18),
        Kind::Heard => Color32::TRANSPARENT,
    };
    egui::Frame::new()
        .fill(fill)
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(clock_hms(entry.at, offset)).weak().small());
                ui.label(RichText::new(&entry.action).strong());
                ui.label(RichText::new(who(entry, &subject(entry, state))).weak());
            });
            ui.label(RichText::new(&entry.text).monospace().small());
        });
}

#[cfg(test)]
mod tests {
    use scent_core::{Direction, Via};

    use super::*;
    use crate::state::LogEntry;

    const ONE: &str = "TESTDEVICE01";
    const TWO: &str = "TESTDEVICE02";

    fn entry(
        at: i64,
        device: &str,
        via: Option<Via>,
        direction: Direction,
        action: &str,
    ) -> LogEntry {
        LogEntry {
            at,
            device_id: Some(device.to_owned()),
            via,
            direction,
            action: action.to_owned(),
            text: format!("{action} at {at}"),
        }
    }

    /// What this app sent over WiFi, or the broker handing it back to us: both are a
    /// `ToDevice` WiFi entry.
    fn wifi_out(at: i64, device: &str, action: &str) -> LogEntry {
        entry(at, device, Some(Via::Wifi), Direction::ToDevice, action)
    }

    fn ats(shown: &[&LogEntry]) -> Vec<i64> {
        shown.iter().map(|entry| entry.at).collect()
    }

    #[test]
    fn a_wifi_send_and_the_brokers_echo_of_it_are_one_line() {
        let log = [wifi_out(100, ONE, "TURN_ON"), wifi_out(101, ONE, "TURN_ON")];
        // The first of the two is the one that stays.
        assert_eq!(ats(&collapse_echoes(&log)), [100]);
    }

    #[test]
    fn a_send_and_its_echo_in_the_same_second_are_one_line() {
        let log = [
            wifi_out(100, ONE, "SET_LEVEL"),
            wifi_out(100, ONE, "SET_LEVEL"),
        ];
        assert_eq!(collapse_echoes(&log).len(), 1);
    }

    #[test]
    fn it_is_an_echo_only_for_the_same_device_and_action_within_5_s() {
        let log = [
            wifi_out(100, ONE, "TURN_ON"),
            wifi_out(101, TWO, "TURN_ON"),
            wifi_out(101, ONE, "SET_LEVEL"),
            wifi_out(106, ONE, "TURN_ON"),
        ];
        assert_eq!(collapse_echoes(&log).len(), 4);
        // 5 s is still the same message, 6 s is another.
        let log = [wifi_out(100, ONE, "TURN_ON"), wifi_out(105, ONE, "TURN_ON")];
        assert_eq!(collapse_echoes(&log).len(), 1);
    }

    #[test]
    fn only_a_wifi_command_has_an_echo() {
        let bluetooth = entry(
            100,
            ONE,
            Some(Via::Bluetooth),
            Direction::ToDevice,
            "TURN_ON",
        );
        let reply = entry(101, ONE, Some(Via::Wifi), Direction::FromDevice, "TURN_ON");
        let failure = entry(102, ONE, None, Direction::ToDevice, "TURN_ON");
        let log = [
            bluetooth.clone(),
            wifi_out(100, ONE, "TURN_ON"),
            reply.clone(),
            reply,
            failure,
        ];
        // The Bluetooth command, the WiFi one, both replies and the failure.
        assert_eq!(collapse_echoes(&log).len(), 5);
        // Two Bluetooth commands in a row are two commands.
        assert_eq!(collapse_echoes(&[bluetooth.clone(), bluetooth]).len(), 2);
    }

    #[test]
    fn each_message_hides_at_most_one_echo() {
        // Two commands, then both echoes.
        let log = [
            wifi_out(100, ONE, "TURN_ON"),
            wifi_out(101, ONE, "TURN_ON"),
            wifi_out(102, ONE, "TURN_ON"),
            wifi_out(103, ONE, "TURN_ON"),
        ];
        assert_eq!(collapse_echoes(&log).len(), 2);
        // Three entries cannot be fewer than two lines.
        assert_eq!(collapse_echoes(&log[..3]).len(), 2);
    }

    #[test]
    fn a_line_says_who_it_is_about_and_over_what() {
        let sent = wifi_out(1, ONE, "TURN_ON");
        assert_eq!(kind(&sent), Kind::Sent);
        assert_eq!(who(&sent, "Bedroom"), "You to Bedroom over WiFi");

        let heard = entry(
            1,
            ONE,
            Some(Via::Bluetooth),
            Direction::FromDevice,
            "status",
        );
        assert_eq!(kind(&heard), Kind::Heard);
        assert_eq!(who(&heard, "Bedroom"), "Bedroom over Bluetooth");

        // A failure: nothing went out, so it names no transport.
        let problem = entry(1, ONE, None, Direction::ToDevice, "Bluetooth TURN_ON");
        assert_eq!(kind(&problem), Kind::Problem);
        assert_eq!(who(&problem, "Bedroom"), "Bedroom");

        // The app's own note about the broker has no diffuser.
        let broker = LogEntry {
            at: 1,
            device_id: None,
            via: Some(Via::Wifi),
            direction: Direction::FromDevice,
            action: "broker".to_owned(),
            text: "Connected to the broker.".to_owned(),
        };
        assert_eq!(subject(&broker, &AppState::default()), "Broker");
        assert_eq!(who(&broker, "Broker"), "Broker");
    }

    #[test]
    fn the_rest_of_the_log_keeps_its_order() {
        let reply = entry(
            99,
            ONE,
            Some(Via::Wifi),
            Direction::FromDevice,
            "update_status",
        );
        let log = [
            reply,
            wifi_out(100, ONE, "TURN_ON"),
            entry(
                100,
                TWO,
                Some(Via::Bluetooth),
                Direction::FromDevice,
                "status",
            ),
            wifi_out(101, ONE, "TURN_ON"),
            entry(
                104,
                ONE,
                Some(Via::Wifi),
                Direction::FromDevice,
                "update_status",
            ),
        ];
        assert_eq!(ats(&collapse_echoes(&log)), [99, 100, 100, 104]);
        assert!(collapse_echoes(&Vec::<LogEntry>::new()).is_empty());
    }
}
