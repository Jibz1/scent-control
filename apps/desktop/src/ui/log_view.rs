//! The log: the conversation with the diffusers, newest last, in a pane along the
//! bottom of the detail pane that the toolbar shows and hides. Each entry is one line
//! in SF Mono 11 pt, and only the lines in view are laid out.

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{
    self, Align, Color32, FontSelection, Layout, Margin, Rect, RichText, Sense, TextStyle,
    TextWrapMode, WidgetInfo, WidgetText, WidgetType, pos2, vec2,
};
use scent_core::{Direction, Via};

use super::clock::{clock_hms, local_utc_offset_secs};
use super::format::via_name;
use super::theme::{GRID, Palette, headline};
use super::widgets::{BOX_PADDING_X, checkbox, secondary};
use crate::state::{AppState, LogEntry};

/// How many lines the log shows. The state keeps more (see `state::LOG_CAP`).
const SHOWN: usize = 300;

/// A WiFi command and the broker's copy of it are one message when they are this many
/// seconds apart at most.
const ECHO_WITHIN_S: u64 = 5;

/// Height of the log when it is open, before the person drags it.
pub(super) const OPEN_HEIGHT: f32 = 160.0;

/// The height of a line of the log.
const LINE_HEIGHT: f32 = 20.0;

/// The space before a line's text.
const LINE_INSET: f32 = 8.0;

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
///
/// This runs every frame, so only WiFi sends look for a copy, and only among the WiFi
/// commands we didn't send that are within [`ECHO_WITHIN_S`] of them.
pub fn collapse_echoes<'a>(entries: impl IntoIterator<Item = &'a LogEntry>) -> Vec<&'a LogEntry> {
    let entries: Vec<&LogEntry> = entries.into_iter().collect();
    // The entries that may be copies, by time and then by place in the log.
    let mut copies: Vec<(i64, usize)> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.sent && is_wifi_command(entry))
        .map(|(i, entry)| (entry.at, i))
        .collect();
    copies.sort_unstable();
    let window = ECHO_WITHIN_S as i64;
    let mut hidden = vec![false; entries.len()];
    for send in entries
        .iter()
        .filter(|entry| entry.sent && is_wifi_command(entry))
    {
        let first = copies.partition_point(|&(at, _)| at < send.at.saturating_sub(window));
        // The closest copy, and of two as close the earlier one in the log.
        let echo = copies[first..]
            .iter()
            .take_while(|&&(at, _)| at <= send.at.saturating_add(window))
            .filter(|&&(_, i)| !hidden[i] && is_echo_of(send, entries[i]))
            .min_by_key(|&&(at, i)| (at.abs_diff(send.at), i))
            .map(|&(_, i)| i);
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
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Shows the log if it is hidden, and hides it if it is shown.
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// The log pane along the bottom. Add it before the central panel above it. It
    /// slides open and shut when [`toggle`](Self::toggle)d. Returns where it is, when it
    /// is shown, for the separator above it.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        selected: Option<&str>,
        now: i64,
    ) -> Option<Rect> {
        let palette = Palette::of(ui);
        let frame = egui::Frame::new()
            .fill(palette.grouped)
            .inner_margin(Margin {
                left: BOX_PADDING_X as i8,
                right: BOX_PADDING_X as i8,
                top: GRID as i8,
                bottom: 0,
            });
        let only_selected = &mut self.only_selected;
        egui::Panel::bottom("log")
            .resizable(true)
            .default_size(OPEN_HEIGHT)
            .min_size(80.0)
            .show_separator_line(false)
            .frame(frame)
            .show_collapsible(ui, &mut self.open, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Log").text_style(headline()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        checkbox(ui, only_selected, "Only the selected diffuser");
                    });
                });
                ui.add_space(GRID / 2.0);
                let only = (*only_selected).then_some(selected).flatten();
                lines(ui, state, only, now);
            })
            .map(|shown| shown.response.rect)
    }
}

/// The lines of the log, of one diffuser if `only` names one. Only the lines in view
/// are laid out.
fn lines(ui: &mut egui::Ui, state: &AppState, only: Option<&str>, now: i64) {
    let wanted = state
        .log
        .iter()
        .filter(|entry| only.is_none_or(|id| entry.device_id.as_deref() == Some(id)));
    let lines = collapse_echoes(wanted);
    if lines.is_empty() {
        secondary(
            ui,
            "Nothing yet. Commands you send and everything the diffusers say appear here.",
        );
        // Take the rest of the pane, as the lines would, so that it keeps its height.
        ui.allocate_space(ui.available_size());
        return;
    }
    let lines = &lines[lines.len().saturating_sub(SHOWN)..];
    let offset = local_utc_offset_secs(now);
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show_rows(ui, LINE_HEIGHT, lines.len(), |ui, shown| {
            for entry in &lines[shown] {
                line(ui, entry, state, offset);
            }
        });
}

/// One entry on one line: the time, the action, who it is about, and the text, cut
/// short to fit. The whole text is in the tooltip when it was cut, and Copy in the
/// line's context menu puts it on the clipboard, since the lines are drawn, not
/// selectable text.
fn line(ui: &mut egui::Ui, entry: &LogEntry, state: &AppState, offset: i64) {
    let palette = Palette::of(ui);
    let width = ui.available_width();
    // Clicks for the context menu, but no stop for the Tab key on every line.
    let (rect, response) = ui.allocate_exact_size(vec2(width, LINE_HEIGHT), Sense::CLICK);
    response.context_menu(|ui| {
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(entry.text.clone());
        }
    });
    if !ui.is_rect_visible(rect) {
        return;
    }
    let fill = match kind(entry) {
        Kind::Sent => Palette::tint(palette.label, 5),
        Kind::Problem => Palette::tint(palette.orange, 12),
        Kind::Heard => Color32::TRANSPARENT,
    };
    ui.painter().rect_filled(rect, 4, fill);

    let font = TextStyle::Monospace.resolve(ui.style());
    let mut job = LayoutJob::default();
    let mut add = |text: &str, color: Color32| {
        let format = TextFormat {
            font_id: font.clone(),
            color,
            ..TextFormat::default()
        };
        job.append(text, 0.0, format);
    };
    let (clock, who) = (
        clock_hms(entry.at, offset),
        who(entry, &subject(entry, state)),
    );
    add(&clock, palette.secondary_label);
    add("  ", palette.label);
    add(&entry.action, palette.label);
    add("  ", palette.label);
    add(&who, palette.secondary_label);
    add("  ", palette.label);
    add(&entry.text, palette.label);
    response.widget_info(|| {
        let line = format!("{clock} {} {who} {}", entry.action, entry.text);
        WidgetInfo::labeled(WidgetType::Label, true, line)
    });
    let galley = WidgetText::from(job).into_galley(
        ui,
        Some(TextWrapMode::Truncate),
        rect.width() - 2.0 * LINE_INSET,
        FontSelection::Style(TextStyle::Monospace),
    );
    let top = rect.center().y - galley.size().y / 2.0;
    let elided = galley.elided;
    ui.painter()
        .galley(pos2(rect.left() + LINE_INSET, top), galley, palette.label);
    if elided {
        response.on_hover_ui(|ui| {
            ui.label(RichText::new(&entry.text).text_style(TextStyle::Monospace));
        });
    }
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
