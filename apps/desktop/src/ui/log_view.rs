//! The log: the conversation with the diffusers, newest last, in a pane along the
//! bottom of the detail pane that the toolbar shows and hides. Hidden until it is
//! asked for, for this session.
//!
//! Each entry is a row that reads as a sentence (see `log_summary`), with a disclosure
//! triangle, a mark for what was sent (↗), heard (↙) or failed (!), and the time in
//! tabular figures. A row opens, on a click, to
//! the message itself, masked, in SF Mono. Only the rows in view are laid out.

use eframe::egui::style::ScrollAnimation;
use eframe::egui::text::LayoutJob;
use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, Id, Layout, Margin, Rect, RichText, Sense, TextStyle,
    Ui, WidgetInfo, WidgetType, pos2, vec2,
};
use scent_core::{Direction, Via};

use super::clock::{clock_hms, local_utc_offset_secs};
use super::format::{display_name, unknown_name};
use super::glyphs::{Mark, disclosure, mark, solid};
use super::log_summary::{detail, sentence};
use super::theme::{CONTROL_RADIUS, GRID, Palette, headline, subheadline};
use super::widgets::{BOX_PADDING_X, checkbox, one_line, secondary};
use crate::state::{AppState, LogEntry};

/// How many lines the log shows. The state keeps more (see `state::LOG_CAP`).
const SHOWN: usize = 300;

/// A WiFi command and the broker's copy of it are one message when they are this many
/// seconds apart at most.
const ECHO_WITHIN_S: u64 = 5;

/// Height of the log when it is open, before the person drags it.
pub(super) const OPEN_HEIGHT: f32 = 200.0;

/// The height of a row of the log, closed.
const ROW_HEIGHT: f32 = 26.0;

/// The space at either end of a row; the disclosure triangle's square, the mark's, and the
/// space after each.
const ROW_INSET: f32 = 8.0;
const DISCLOSURE: f32 = 10.0;
const MARK: f32 = 14.0;
const MARK_GAP: f32 = 6.0;
/// Where a row's sentence starts, past the triangle and the mark.
const TEXT_LEFT: f32 = ROW_INSET + DISCLOSURE + MARK_GAP + MARK + MARK_GAP;

/// The space inside an opened row's message box.
const MESSAGE_PADDING: f32 = 8.0;

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

/// The name a row goes by: the diffuser's, as the screens show it, or "Broker" for what
/// the broker did, or "App" for what the app itself reports.
fn subject(entry: &LogEntry, state: &AppState) -> String {
    match &entry.device_id {
        Some(id) => state
            .devices
            .get(id)
            .map_or_else(|| unknown_name(id), display_name),
        None if entry.action == "broker" => "Broker".to_owned(),
        None => "App".to_owned(),
    }
}

/// An entry, told apart from the others well enough to stay open as the log grows: what
/// it holds, and which of the entries that hold the same it is (the same command sent
/// twice in one second is two rows).
#[derive(Debug, Clone, PartialEq, Eq)]
struct RowKey {
    at: i64,
    device_id: Option<String>,
    action: String,
    text: String,
    nth: usize,
}

impl RowKey {
    /// The key of `entries[i]`.
    fn of(entries: &[&LogEntry], i: usize) -> RowKey {
        let entry = entries[i];
        let mut key = RowKey {
            at: entry.at,
            device_id: entry.device_id.clone(),
            action: entry.action.clone(),
            text: entry.text.clone(),
            nth: 0,
        };
        key.nth = entries[..i].iter().filter(|other| key.holds(other)).count();
        key
    }

    fn holds(&self, entry: &LogEntry) -> bool {
        self.at == entry.at
            && self.device_id == entry.device_id
            && self.action == entry.action
            && self.text == entry.text
    }

    /// Where the entry is in `entries`, if it is still there.
    fn find(&self, entries: &[&LogEntry]) -> Option<usize> {
        entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.holds(entry))
            .nth(self.nth)
            .map(|(i, _)| i)
    }
}

/// What the log screen remembers.
#[derive(Default)]
pub struct LogView {
    /// Shown or hidden: hidden at first, for each session.
    open: bool,
    only_selected: bool,
    /// The row that is open to its message.
    expanded: Option<RowKey>,
    /// A row was just opened: it is scrolled into view, its message and all.
    reveal: bool,
}

impl LogView {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Shows the log if it is hidden, and hides it if it is shown.
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Whether an opened row is still to be brought into view.
    #[cfg(test)]
    pub fn is_revealing(&self) -> bool {
        self.reveal
    }

    /// The log open, with the row of the first entry that `pick` likes open to its
    /// message, for the snapshots.
    #[cfg(test)]
    pub fn as_open(&mut self, state: &AppState, pick: impl Fn(&LogEntry) -> bool) {
        self.open = true;
        let entries: Vec<&LogEntry> = state.log.iter().collect();
        self.expanded = entries
            .iter()
            .position(|entry| pick(entry))
            .map(|i| RowKey::of(&entries, i));
        self.reveal = self.expanded.is_some();
    }

    /// The log pane along the bottom. Add it before the central panel above it. It
    /// slides open and shut when [`toggle`](Self::toggle)d. Returns where it is, when it
    /// is shown, for the separator above it.
    pub fn show(
        &mut self,
        ui: &mut Ui,
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
        let expanded = &mut self.expanded;
        let reveal = &mut self.reveal;
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
                rows(ui, state, only, now, (expanded, reveal));
            })
            .map(|shown| shown.response.rect)
    }
}

/// The rows of the log, of one diffuser if `only` names one. Only the rows in view are
/// laid out; the open one is as tall as its message needs, and is scrolled into view
/// when it has just been opened (`reveal`).
fn rows(
    ui: &mut Ui,
    state: &AppState,
    only: Option<&str>,
    now: i64,
    (expanded, reveal): (&mut Option<RowKey>, &mut bool),
) {
    let wanted = state
        .log
        .iter()
        .filter(|entry| only.is_none_or(|id| entry.device_id.as_deref() == Some(id)));
    let entries = collapse_echoes(wanted);
    if entries.is_empty() {
        secondary(
            ui,
            "Nothing yet. Commands you send and everything the diffusers say appear here.",
        );
        // Take the rest of the pane, as the rows would, so that it keeps its height.
        ui.allocate_space(ui.available_size());
        return;
    }
    let entries = &entries[entries.len().saturating_sub(SHOWN)..];
    let clock = Clock::of(ui, now);
    let open_at = expanded.as_ref().and_then(|key| key.find(entries));
    // The open row's message, laid out once for its height.
    let width = ui.available_width();
    let message = open_at.map(|at| Message::lay_out(ui, entries[at], width));
    let extra = message.as_ref().map_or(0.0, Message::height);
    let total = entries.len() as f32 * ROW_HEIGHT + extra;
    let mut toggled = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        // New rows keep the newest in view, except while an opened row is brought in.
        .stick_to_bottom(!*reveal)
        .show_viewport(ui, |ui, viewport| {
            ui.set_height(total);
            let origin = ui.max_rect().top();
            let x_range = ui.max_rect().x_range();
            let mut top = 0.0;
            for (i, entry) in entries.iter().enumerate() {
                let open = open_at == Some(i);
                let height = ROW_HEIGHT + if open { extra } else { 0.0 };
                let rect = Rect::from_x_y_ranges(x_range, (origin + top)..=(origin + top + height));
                // Scrolled to, at once and once: all of it, or its top when it is taller
                // than the pane.
                if open && std::mem::take(reveal) {
                    let fits = height <= viewport.height();
                    let align = (!fits).then_some(Align::Min);
                    ui.scroll_to_rect_animation(rect, align, ScrollAnimation::none());
                }
                if top + height >= viewport.min.y && top <= viewport.max.y {
                    let message = message.as_ref().filter(|_| open);
                    let subject = subject(entry, state);
                    if row(ui, rect, i, entry, &subject, &clock, message) {
                        toggled = Some(i);
                    }
                }
                top += height;
            }
        });
    // An opened row that isn't listed (filtered out, or gone from the log) is not
    // brought into view later.
    if open_at.is_none() {
        *reveal = false;
    }
    if let Some(i) = toggled {
        *expanded = (open_at != Some(i)).then(|| RowKey::of(entries, i));
        *reveal = expanded.is_some();
    }
}

/// An open row's message: what it was, and the masked text in SF Mono, wrapped.
struct Message {
    detail: std::sync::Arc<egui::Galley>,
    text: std::sync::Arc<egui::Galley>,
}

impl Message {
    fn lay_out(ui: &Ui, entry: &LogEntry, width: f32) -> Message {
        // The box runs from under the sentence to the row's end.
        let inner = width - TEXT_LEFT - ROW_INSET - 2.0 * MESSAGE_PADDING;
        let inner = inner.max(40.0);
        let palette = Palette::of(ui);
        let detail = one_line(
            ui,
            RichText::new(detail(entry)).color(palette.secondary_label),
            subheadline(),
            inner,
        );
        // Compact JSON has no spaces to wrap at: it wraps anywhere.
        let font = TextStyle::Monospace.resolve(ui.style());
        let mut job = LayoutJob::simple(entry.text.clone(), font, palette.label, inner);
        job.wrap.break_anywhere = true;
        let text = ui.ctx().fonts_mut(|fonts| fonts.layout_job(job));
        Message { detail, text }
    }

    /// How much taller it makes its row.
    fn height(&self) -> f32 {
        self.detail.size().y + GRID / 2.0 + self.text.size().y + 2.0 * MESSAGE_PADDING + GRID
    }
}

/// How a row is told apart from the others.
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

/// One row in `rect`: the chevron, the sentence (cut short to fit, the whole of it in the
/// tooltip), and the time; with `message`, the row is open and shows it under them. A
/// click opens or closes it; Copy in its context menu puts the masked message on the
/// clipboard. True when it was clicked.
fn row(
    ui: &mut Ui,
    rect: Rect,
    index: usize,
    entry: &LogEntry,
    subject: &str,
    clock: &Clock,
    message: Option<&Message>,
) -> bool {
    let palette = Palette::of(ui);
    let said = sentence(entry, subject);
    let time = clock_hms(entry.at, clock.offset);
    // Clicks and the context menu, but no stop for the Tab key on every row.
    let response = ui.interact(rect, Id::new("log_row").with(index), Sense::CLICK);
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::Button, true, &said);
        info.current_text_value = Some(time.clone());
        info
    });
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_expanded(message.is_some());
    });
    response.context_menu(|ui| {
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(entry.text.clone());
        }
    });
    let clicked = response.clicked();
    let hovered = response.hovered();
    if hovered {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if !ui.is_rect_visible(rect) {
        return clicked;
    }
    let kind = kind(entry);
    let fill = match kind {
        Kind::Problem => Palette::tint(palette.orange, 12),
        _ if message.is_some() || hovered => Palette::tint(palette.label, 5),
        _ => Color32::TRANSPARENT,
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CONTROL_RADIUS, fill);

    let line = Rect::from_min_size(rect.min, vec2(rect.width(), ROW_HEIGHT));
    let ground = solid(fill, palette.grouped);
    // One disclosure triangle, ▸ or ▾; then what happened: ↗ sent, ↙ heard, ! failed.
    let triangle = Rect::from_center_size(
        pos2(line.left() + ROW_INSET + DISCLOSURE / 2.0, line.center().y),
        vec2(DISCLOSURE, DISCLOSURE),
    );
    let secondary = solid(palette.secondary_label, ground);
    disclosure(painter, triangle, secondary, message.is_some());
    let (what, ink) = match kind {
        Kind::Sent => (Mark::Sent, palette.gray),
        Kind::Heard => (Mark::Heard, palette.gray),
        Kind::Problem => (Mark::Failed, palette.orange),
    };
    let at = Rect::from_center_size(
        pos2(triangle.right() + MARK_GAP + MARK / 2.0, line.center().y),
        vec2(MARK, MARK),
    );
    mark(painter, at, solid(ink, ground), what);

    let time_width = clock.paint(
        ui,
        &time,
        palette.secondary_label,
        line.right() - ROW_INSET,
        line.center().y,
    );
    let text_left = line.left() + TEXT_LEFT;
    let text_width = (line.right() - ROW_INSET - time_width - 2.0 * GRID - text_left).max(0.0);
    let galley = one_line(ui, &said, TextStyle::Body, text_width);
    let elided = galley.elided;
    let at = Align2::LEFT_CENTER.anchor_size(pos2(text_left, line.center().y), galley.size());
    ui.painter().galley(at.min, galley, palette.label);

    if let Some(message) = message {
        let mut y = line.bottom();
        ui.painter().galley(
            pos2(text_left, y),
            message.detail.clone(),
            palette.secondary_label,
        );
        y += message.detail.size().y + GRID / 2.0;
        let boxed = Rect::from_min_max(
            pos2(text_left, y),
            pos2(
                line.right() - ROW_INSET,
                y + message.text.size().y + 2.0 * MESSAGE_PADDING,
            ),
        );
        ui.painter()
            .rect_filled(boxed, CONTROL_RADIUS, Palette::tint(palette.label, 5));
        ui.painter().galley(
            boxed.min + vec2(MESSAGE_PADDING, MESSAGE_PADDING),
            message.text.clone(),
            palette.label,
        );
        // Read out too: the message is text, and what an open row is for.
        let shown = ui.interact(boxed, Id::new("log_message").with(index), Sense::hover());
        shown.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &entry.text));
    }
    if elided && message.is_none() {
        response.on_hover_text(&said);
    }
    clicked
}

/// How the rows tell the time: the Mac's offset from UTC, and the width of a digit in
/// Subheadline, measured once a frame.
struct Clock {
    offset: i64,
    /// The widest digit: every digit is given this much, as tabular figures are.
    digit: f32,
}

impl Clock {
    fn of(ui: &Ui, now: i64) -> Clock {
        let font = subheadline().resolve(ui.style());
        let digit = ui.ctx().fonts_mut(|fonts| {
            ('0'..='9')
                .map(|c| fonts.glyph_width(&font, c))
                .fold(0.0, f32::max)
        });
        Clock {
            offset: local_utc_offset_secs(now),
            digit,
        }
    }

    /// `text` in Subheadline with its digits in columns of one width, so that the times
    /// line up row under row; right-aligned at `right` and centered on `middle`. Returns
    /// how wide it is.
    fn paint(&self, ui: &Ui, text: &str, color: Color32, right: f32, middle: f32) -> f32 {
        tabular(ui, text, self.digit, color, (right, middle))
    }
}

/// `text` with each digit `digit` wide, right-aligned at `right` and centered on
/// `middle`.
fn tabular(ui: &Ui, text: &str, digit: f32, color: Color32, (right, middle): (f32, f32)) -> f32 {
    let glyphs: Vec<(std::sync::Arc<egui::Galley>, f32)> = text
        .chars()
        .map(|c| {
            let galley = one_line(ui, c.to_string(), subheadline(), f32::INFINITY);
            let width = if c.is_ascii_digit() {
                digit
            } else {
                galley.size().x
            };
            (galley, width)
        })
        .collect();
    let total: f32 = glyphs.iter().map(|(_, width)| width).sum();
    let mut x = right - total;
    for (galley, width) in glyphs {
        let at = Align2::CENTER_CENTER.anchor_size(pos2(x + width / 2.0, middle), galley.size());
        ui.painter().galley(at.min, galley, color);
        x += width;
    }
    total
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
    fn a_row_is_told_apart_by_what_happened() {
        let sent = wifi_sent(1, ONE, &on(true));
        assert_eq!(kind(&sent), Kind::Sent);
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
        // A command on the topic that we did not send (the bench, or a server).
        assert_eq!(kind(&wifi_echo(1, ONE, &on(true))), Kind::Heard);
    }

    #[test]
    fn a_row_goes_by_the_name_the_screens_show() {
        let mut state = AppState::default();
        let mut named = crate::state::DeviceView::new(ONE);
        named.label = "Bedroom".to_owned();
        state.devices.insert(ONE.to_owned(), named);
        assert_eq!(subject(&wifi_sent(1, ONE, &on(true)), &state), "Bedroom");
        // One the app doesn't know: never its raw ID.
        assert_eq!(
            subject(&wifi_sent(1, TWO, &on(true)), &state),
            "Diffuser CE02"
        );
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
        assert_eq!(subject(&broker, &state), "Broker");
    }

    #[test]
    fn an_open_row_stays_open_as_the_log_grows() {
        let log = [
            wifi_sent(1, ONE, &on(true)),
            wifi_sent(1, ONE, &on(true)),
            wifi_sent(2, ONE, &on(false)),
        ];
        let entries: Vec<&LogEntry> = log.iter().collect();
        // The same command twice in one second is two rows, each opened on its own.
        let second = RowKey::of(&entries, 1);
        assert_eq!(second.find(&entries), Some(1));
        assert_eq!(RowKey::of(&entries, 0).find(&entries), Some(0));
        // Older rows dropping off the top don't open another.
        let fewer: Vec<&LogEntry> = log[1..].iter().collect();
        assert_eq!(RowKey::of(&entries, 2).find(&fewer), Some(1));
        assert_eq!(second.find(&fewer), None);
    }
}
