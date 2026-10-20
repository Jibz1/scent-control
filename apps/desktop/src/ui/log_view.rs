//! The log: the conversation with the diffusers, newest last.

use eframe::egui::{self, Color32, RichText};
use scent_core::{Direction, Via};

use super::clock::{clock_hms, local_utc_offset_secs};
use super::format::via_name;
use super::{amber, subtle_fill};
use crate::state::{AppState, LogEntry};

/// How many lines the log shows. The state keeps more (see `state::LOG_CAP`).
const SHOWN: usize = 300;

/// A WiFi command and the broker's copy of it are one message when they are this many
/// seconds apart at most.
const ECHO_WITHIN_S: u64 = 5;

/// Height of the log when it is open, before the person drags it.
const OPEN_HEIGHT: f32 = 220.0;

/// Whether this entry is a command to a diffuser on the broker: one we sent over WiFi,
/// or a copy of a command that someone published to its topic.
fn is_wifi_command(entry: &LogEntry) -> bool {
    entry.direction == Direction::ToDevice
        && entry.via == Some(Via::Wifi)
        && entry.device_id.is_some()
}

/// Whether `other` is the broker's copy of the WiFi command `send`: the same command,
/// as text, to the same diffuser, within [`ECHO_WITHIN_S`]. Two different commands
/// (turn on, then turn off) are never copies of each other.
fn is_echo_of(send: &LogEntry, other: &LogEntry) -> bool {
    send.sent
        && is_wifi_command(send)
        && !other.sent
        && is_wifi_command(other)
        && other.device_id == send.device_id
        && other.action == send.action
        && other.text == send.text
        && other.at.abs_diff(send.at) <= ECHO_WITHIN_S
}

/// The log with each command we sent over WiFi and the broker's copy of it as one line.
///
/// We subscribe to everything under the Scentiment prefix, so the broker hands our own
/// publish back to us, and the log would show every WiFi command twice: once as sent
/// and once as heard. Only the copy is hidden, never a command we sent: each send
/// hides the closest copy of it (see [`is_echo_of`]), and a send the broker never
/// echoed, or a command that someone else published, stays. Everything is kept in order.
pub fn collapse_echoes<'a>(entries: impl IntoIterator<Item = &'a LogEntry>) -> Vec<&'a LogEntry> {
    let entries: Vec<&LogEntry> = entries.into_iter().collect();
    let mut hidden = vec![false; entries.len()];
    for send in &entries {
        let echo = entries
            .iter()
            .enumerate()
            .filter(|&(i, other)| !hidden[i] && is_echo_of(send, other))
            .min_by_key(|(_, other)| other.at.abs_diff(send.at))
            .map(|(i, _)| i);
        if let Some(i) = echo {
            hidden[i] = true;
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
    /// Something the diffuser or the broker said, or a command on the diffuser's topic
    /// that we did not send.
    Heard,
    /// Something that went wrong. These are `ToDevice` with no `via`: nothing went out.
    Problem,
}

fn kind(entry: &LogEntry) -> Kind {
    match (entry.direction, entry.via) {
        (Direction::FromDevice, _) => Kind::Heard,
        (Direction::ToDevice, Some(_)) if entry.sent => Kind::Sent,
        (Direction::ToDevice, Some(_)) => Kind::Heard,
        (Direction::ToDevice, None) => Kind::Problem,
    }
}

/// Who a line is about: "You to Bedroom over WiFi" for what we sent, "To Bedroom over
/// WiFi" for a command that someone else put on its topic, "Bedroom over Bluetooth"
/// for what it said, or just "Broker" or the diffuser's name for a note of the app's own.
fn who(entry: &LogEntry, subject: &str) -> String {
    match (kind(entry), entry.direction, entry.via, &entry.device_id) {
        (Kind::Sent, _, Some(via), _) => format!("You to {subject} over {}", via_name(via)),
        (Kind::Heard, Direction::ToDevice, Some(via), _) => {
            format!("To {subject} over {}", via_name(via))
        }
        (Kind::Heard, _, Some(via), Some(_)) => format!("{subject} over {}", via_name(via)),
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
    use scent_core::{Command, Direction, Level, Via, device_topic};

    use super::*;
    use crate::state::{Event, LogEntry};

    const ONE: &str = "TESTDEVICE01";
    const TWO: &str = "TESTDEVICE02";

    fn entry(
        at: i64,
        device: &str,
        via: Option<Via>,
        direction: Direction,
        sent: bool,
        action: &str,
        text: &str,
    ) -> LogEntry {
        LogEntry {
            at,
            device_id: Some(device.to_owned()),
            via,
            direction,
            action: action.to_owned(),
            text: text.to_owned(),
            sent,
        }
    }

    /// What the app logs when it sends `cmd` over WiFi: the redacted frame.
    fn wifi_sent(at: i64, device: &str, cmd: &Command) -> LogEntry {
        let (action, text) = (cmd.action(Via::Wifi), cmd.redacted(Via::Wifi));
        entry(
            at,
            device,
            Some(Via::Wifi),
            Direction::ToDevice,
            true,
            action,
            &text,
        )
    }

    /// What the app logs when the broker hands `cmd` back: the body as received.
    fn wifi_echo(at: i64, device: &str, cmd: &Command) -> LogEntry {
        let text = String::from_utf8(cmd.mqtt_body()).unwrap();
        let action = cmd.action(Via::Wifi);
        entry(
            at,
            device,
            Some(Via::Wifi),
            Direction::ToDevice,
            false,
            action,
            &text,
        )
    }

    fn on(on: bool) -> Command {
        Command::TurnOn { on }
    }

    fn texts(shown: &[&LogEntry]) -> Vec<String> {
        shown.iter().map(|entry| entry.text.clone()).collect()
    }

    fn ats(shown: &[&LogEntry]) -> Vec<i64> {
        shown.iter().map(|entry| entry.at).collect()
    }

    #[test]
    fn a_wifi_send_and_the_brokers_echo_of_it_are_one_line() {
        let log = [
            wifi_sent(100, ONE, &on(true)),
            wifi_echo(101, ONE, &on(true)),
        ];
        let shown = collapse_echoes(&log);
        // The send stays.
        assert_eq!(ats(&shown), [100]);
        assert!(shown[0].sent);
    }

    #[test]
    fn the_echo_may_come_first_and_the_send_still_stays() {
        let log = [
            wifi_echo(100, ONE, &on(true)),
            wifi_sent(100, ONE, &on(true)),
        ];
        let shown = collapse_echoes(&log);
        assert_eq!(shown.len(), 1);
        assert!(shown[0].sent);
    }

    #[test]
    fn it_is_an_echo_only_of_the_same_command_to_the_same_diffuser_within_5_s() {
        let send = wifi_sent(100, ONE, &on(true));
        let others = [
            wifi_echo(101, TWO, &on(true)),
            wifi_echo(101, ONE, &Command::SetLevel(Level::new(1).unwrap())),
            wifi_echo(101, ONE, &on(false)),
            wifi_echo(106, ONE, &on(true)),
        ];
        for other in others {
            let log = [send.clone(), other];
            assert_eq!(collapse_echoes(&log).len(), 2, "{log:?}");
        }
        // 5 s is still the same message, 6 s is another.
        let log = [send.clone(), wifi_echo(105, ONE, &on(true))];
        assert_eq!(collapse_echoes(&log).len(), 1);
    }

    #[test]
    fn a_send_is_never_hidden_by_another_send() {
        // Turn on, no echo (just after a reconnect, say), then turn off: both are shown.
        let log = [
            wifi_sent(100, ONE, &on(true)),
            wifi_sent(102, ONE, &on(false)),
        ];
        assert_eq!(collapse_echoes(&log).len(), 2);
        // The same command twice, with no echo, is two commands.
        let log = [
            wifi_sent(100, ONE, &on(true)),
            wifi_sent(102, ONE, &on(true)),
        ];
        assert_eq!(collapse_echoes(&log).len(), 2);
    }

    #[test]
    fn interleaved_commands_each_keep_their_own_line() {
        // Turn on and turn off inside one round trip: both sends, then both echoes.
        let log = [
            wifi_sent(100, ONE, &on(true)),
            wifi_sent(100, ONE, &on(false)),
            wifi_echo(101, ONE, &on(true)),
            wifi_echo(101, ONE, &on(false)),
        ];
        let shown = collapse_echoes(&log);
        assert_eq!(
            texts(&shown),
            [on(true).redacted(Via::Wifi), on(false).redacted(Via::Wifi)]
        );
        assert!(shown.iter().all(|entry| entry.sent));
        // And with the echoes the other way round.
        let log = [
            wifi_sent(100, ONE, &on(true)),
            wifi_sent(100, ONE, &on(false)),
            wifi_echo(101, ONE, &on(false)),
            wifi_echo(101, ONE, &on(true)),
        ];
        assert_eq!(collapse_echoes(&log).len(), 2);
    }

    #[test]
    fn each_send_hides_at_most_one_echo() {
        // Two identical commands, one echo: both sends show, the echo is theirs.
        let log = [
            wifi_sent(100, ONE, &on(true)),
            wifi_sent(101, ONE, &on(true)),
            wifi_echo(101, ONE, &on(true)),
        ];
        assert_eq!(ats(&collapse_echoes(&log)), [100, 101]);
        // Two identical commands, two echoes.
        let log = [
            wifi_sent(100, ONE, &on(true)),
            wifi_sent(101, ONE, &on(true)),
            wifi_echo(101, ONE, &on(true)),
            wifi_echo(102, ONE, &on(true)),
        ];
        assert_eq!(ats(&collapse_echoes(&log)), [100, 101]);
        // One command, two copies on the topic: the second one is someone else's.
        let log = [
            wifi_sent(100, ONE, &on(true)),
            wifi_echo(100, ONE, &on(true)),
            wifi_echo(101, ONE, &on(true)),
        ];
        assert_eq!(collapse_echoes(&log).len(), 2);
    }

    #[test]
    fn only_a_wifi_command_has_an_echo() {
        let text = on(true).redacted(Via::Wifi);
        let bluetooth = entry(
            100,
            ONE,
            Some(Via::Bluetooth),
            Direction::ToDevice,
            true,
            "TURN_ON",
            &text,
        );
        let reply = entry(
            101,
            ONE,
            Some(Via::Wifi),
            Direction::FromDevice,
            false,
            "TURN_ON",
            &text,
        );
        let failure = entry(102, ONE, None, Direction::ToDevice, false, "TURN_ON", &text);
        let log = [
            bluetooth.clone(),
            wifi_sent(100, ONE, &on(true)),
            reply.clone(),
            reply,
            failure,
        ];
        // The Bluetooth command, the WiFi one, both replies and the failure.
        assert_eq!(collapse_echoes(&log).len(), 5);
        // A copy on the topic is not the echo of a Bluetooth command.
        let log = [bluetooth, wifi_echo(100, ONE, &on(true))];
        assert_eq!(collapse_echoes(&log).len(), 2);
    }

    #[test]
    fn the_real_events_collapse() {
        // The text of a send and of the broker's copy of it, as the state logs them.
        let mut state = AppState::default();
        for cmd in [on(true), Command::SetLevel(Level::new(2).unwrap())] {
            state.apply_at(
                Event::Sent {
                    device_id: ONE.to_owned(),
                    summary: cmd.redacted(Via::Wifi),
                    via: Via::Wifi,
                    at: 100,
                    power: None,
                    intensity: None,
                    profile: None,
                },
                100,
            );
            state.apply_at(
                Event::Mqtt {
                    topic: device_topic(ONE),
                    body: cmd.mqtt_body(),
                    at: 101,
                },
                101,
            );
        }
        assert_eq!(state.log.len(), 4);
        let shown = collapse_echoes(&state.log);
        assert_eq!(shown.len(), 2);
        assert!(shown.iter().all(|entry| entry.sent));
    }

    #[test]
    fn the_rest_of_the_log_keeps_its_order() {
        let text = on(true).redacted(Via::Wifi);
        let reply = |at, device: &str, via, action: &str| {
            entry(
                at,
                device,
                Some(via),
                Direction::FromDevice,
                false,
                action,
                &text,
            )
        };
        let log = [
            reply(99, ONE, Via::Wifi, "update_status"),
            wifi_sent(100, ONE, &on(true)),
            reply(100, TWO, Via::Bluetooth, "status"),
            wifi_echo(101, ONE, &on(true)),
            reply(104, ONE, Via::Wifi, "update_status"),
        ];
        assert_eq!(ats(&collapse_echoes(&log)), [99, 100, 100, 104]);
        assert!(collapse_echoes(&Vec::<LogEntry>::new()).is_empty());
    }

    #[test]
    fn a_line_says_who_it_is_about_and_over_what() {
        let sent = wifi_sent(1, ONE, &on(true));
        assert_eq!(kind(&sent), Kind::Sent);
        assert_eq!(who(&sent, "Bedroom"), "You to Bedroom over WiFi");

        let bluetooth = entry(
            1,
            ONE,
            Some(Via::Bluetooth),
            Direction::ToDevice,
            true,
            "TURN_ON",
            "{}",
        );
        assert_eq!(who(&bluetooth, "Bedroom"), "You to Bedroom over Bluetooth");

        let heard = entry(
            1,
            ONE,
            Some(Via::Bluetooth),
            Direction::FromDevice,
            false,
            "status",
            "{}",
        );
        assert_eq!(kind(&heard), Kind::Heard);
        assert_eq!(who(&heard, "Bedroom"), "Bedroom over Bluetooth");

        // A failure: nothing went out, so it names no transport.
        let problem = entry(
            1,
            ONE,
            None,
            Direction::ToDevice,
            false,
            "Bluetooth TURN_ON",
            "x",
        );
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
            sent: false,
        };
        assert_eq!(subject(&broker, &AppState::default()), "Broker");
        assert_eq!(who(&broker, "Broker"), "Broker");
    }

    #[test]
    fn a_command_on_the_topic_that_we_did_not_send_is_not_called_ours() {
        // The bench, or a server, publishing to the diffuser: only the broker's copy.
        let other = wifi_echo(1, ONE, &on(true));
        assert_eq!(kind(&other), Kind::Heard);
        assert_eq!(who(&other, "Bedroom"), "To Bedroom over WiFi");
    }
}
