//! The sidebar, a source list: the known diffusers under "Diffusers", each on a glyph
//! tile with its status dot and what it is doing in words, then what the Bluetooth scan
//! found under "Found Nearby", with its signal. Along the bottom, the broker's state and
//! the scan button.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Color32, Rect, RichText, Sense, Spinner, TextStyle, WidgetInfo, WidgetType, pos2, vec2,
};
use scent_core::Online;

use super::format::{
    Model, broker_summary, broker_text, display_name, intensity, name_from_scan, online_text,
    signal, signal_quality,
};
use super::glyphs::{self, Ink, solid};
use super::theme::{CONTROL_RADIUS, GRID, Palette, broker_color, status_color, subheadline};
use super::widgets::{one_line, plain_button, secondary};
use crate::hub::{SCAN_FAILED, UiCommand};
use crate::state::{AppState, BrokerState, DeviceView, Event, Field, ScanHit};

/// How long "Scanning…" stays up if no answer ever comes. The hub gives up on a scan
/// itself after 30 s.
const SCAN_GIVE_UP: Duration = Duration::from_secs(35);

/// A row: a 28 pt tile with 8 pt above and below it.
const ROW_HEIGHT: f32 = 44.0;
const TILE: f32 = 28.0;

/// Where the tile starts, from the row's left edge, and where the text starts: past the
/// tile and a gap.
const TILE_INSET: f32 = GRID;
const TEXT_INSET: f32 = TILE_INSET + TILE + 10.0;

/// The status dot on the tile's corner, and the ring around it in the row's color.
const DOT_RADIUS: f32 = 4.0;
const DOT_RING: f32 = 1.5;

/// The signal bars at the end of a Found Nearby row.
const BARS: egui::Vec2 = vec2(16.0, 11.0);

/// What is selected in the sidebar: a diffuser this app knows, or one found nearby.
pub struct Selection<'a> {
    /// The diffuser whose panel is open, by its device ID.
    pub device: &'a mut Option<String>,
    /// The diffuser found nearby that is selected instead, by its Bluetooth ID.
    pub found: &'a mut Option<String>,
}

#[derive(Default)]
pub struct DeviceList {
    /// When the scan was asked for, while it may still be running.
    scanning: Option<Instant>,
    /// Whether a scan has come back in this session, so that "nothing found" can be
    /// told from "not scanned yet".
    scanned: bool,
}

/// The scan's hits that are not diffusers we already know: no known diffuser has that
/// Bluetooth ID.
pub fn unknown_hits(state: &AppState) -> Vec<&ScanHit> {
    state
        .scan
        .iter()
        .filter(|hit| {
            !state
                .devices
                .values()
                .any(|device| device.ble_id.as_deref() == Some(hit.ble_id.as_str()))
        })
        .collect()
}

/// The power and intensity a diffuser is set to as far as this app knows: what was sent
/// and not yet reported on, else what it reported.
fn power_and_level(device: &DeviceView) -> (Option<bool>, Option<u8>) {
    let power = device.pending_power().or_else(|| {
        let reading = device.readings.get(&Field::Power)?;
        Some(reading.value != 0)
    });
    let level = device.pending_intensity().or_else(|| {
        let reading = device.readings.get(&Field::Intensity)?;
        u8::try_from(reading.value).ok()
    });
    (power, level.filter(|level| (1..=3).contains(level)))
}

/// What a diffuser's row says under its name, in words, so that the dot's color is never
/// the only signal: what it is doing when it answers ("Diffusing at Medium", "Off"),
/// else "Waiting", "Not answering" or "Not pinged yet".
pub fn second_line(device: &DeviceView, online: Online) -> String {
    match online {
        Online::Waiting => "Waiting".to_owned(),
        Online::Offline => "Not answering".to_owned(),
        Online::Unknown => online_text(Online::Unknown).to_owned(),
        Online::Online => match power_and_level(device) {
            (Some(true), Some(level)) => format!("Diffusing at {}", intensity(i64::from(level))),
            (Some(true), None) => "Diffusing".to_owned(),
            (Some(false), _) => "Off".to_owned(),
            (None, _) => online_text(Online::Online).to_owned(),
        },
    }
}

/// How many mist arcs a diffuser's tile shows: its level while it diffuses.
fn tile_arcs(device: &DeviceView) -> f32 {
    match power_and_level(device) {
        (Some(true), Some(level)) => f32::from(level),
        _ => 0.0,
    }
}

impl DeviceList {
    pub fn on_event(&mut self, event: &Event) {
        match event {
            Event::Scanned(_) => {
                self.scanning = None;
                self.scanned = true;
            }
            Event::Failed { what, .. } if what == SCAN_FAILED => self.scanning = None,
            _ => {}
        }
    }

    pub fn is_scanning(&self) -> bool {
        self.scanning
            .is_some_and(|since| since.elapsed() < SCAN_GIVE_UP)
    }

    /// Starts a scan: the command to hand the hub. "Scanning…" shows until it ends.
    pub fn scan(&mut self) -> UiCommand {
        self.scanning = Some(Instant::now());
        UiCommand::Scan
    }

    /// Shows the list in the sidebar and returns what was asked for. `selection` changes
    /// when another row is clicked.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        selection: Selection<'_>,
        now: i64,
    ) -> Vec<UiCommand> {
        let mut commands = Vec::new();
        let scanning = self.is_scanning();
        // The broker's state and the scan button along the bottom, so the lists above
        // have the rest.
        egui::Panel::bottom("sidebar_actions")
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: GRID as i8,
                right: GRID as i8,
                top: GRID as i8,
                bottom: 2 * GRID as i8,
            }))
            .show_separator_line(false)
            .resizable(false)
            .show(ui, |ui| {
                broker_status(ui, &state.broker);
                ui.add_space(GRID);
                ui.horizontal(|ui| {
                    let scan =
                        ui.add_enabled_ui(!scanning, |ui| plain_button(ui, "Scan for Diffusers"));
                    if scan.inner.clicked() {
                        commands.push(self.scan());
                    }
                    if scanning {
                        let color = Palette::of(ui).secondary_label;
                        ui.add(Spinner::new().size(14.0).color(color));
                    }
                });
            });

        let Selection {
            device: chosen,
            found,
        } = selection;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                // With none, the detail pane's empty state says how to find them.
                if !state.devices.is_empty() {
                    section_heading(ui, "Diffusers");
                }
                for device in state.devices.values() {
                    let online = state.online(&device.id, now);
                    let selected = found.is_none() && chosen.as_deref() == Some(device.id.as_str());
                    if device_row(ui, device, online, selected).clicked() {
                        *chosen = Some(device.id.clone());
                        *found = None;
                    }
                }
                let hits = unknown_hits(state);
                if hits.is_empty() && !scanning && !self.scanned {
                    return;
                }
                if !state.devices.is_empty() {
                    ui.add_space(2.0 * GRID);
                }
                section_heading(ui, "Found Nearby");
                if scanning {
                    note(ui, "Scanning…");
                }
                if hits.is_empty() && self.scanned && !scanning {
                    note(ui, "No new diffusers found nearby.");
                }
                for hit in hits {
                    let selected = found.as_deref() == Some(hit.ble_id.as_str());
                    if hit_row(ui, hit, selected).clicked() {
                        *found = Some(hit.ble_id.clone());
                    }
                }
            });
        commands
    }
}

/// A section heading in the sidebar: Subheadline, in the secondary label color, lined
/// up with the tiles.
fn section_heading(ui: &mut egui::Ui, title: &str) {
    let color = Palette::of(ui).secondary_label;
    ui.horizontal(|ui| {
        ui.add_space(TILE_INSET);
        ui.label(RichText::new(title).text_style(subheadline()).color(color));
    });
    ui.add_space(2.0);
}

/// Secondary text in the sidebar, lined up with the rows' text.
fn note(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(TEXT_INSET);
        secondary(ui, text);
    });
}

/// The broker's state at the foot of the sidebar: a dot and a few words, the whole
/// sentence with the reason in the tooltip. One element to accessibility, whose value is
/// that sentence (the dot has no node of its own, so it isn't read twice).
fn broker_status(ui: &mut egui::Ui, broker: &BrokerState) {
    let palette = Palette::of(ui);
    let width = ui.available_width();
    let words = one_line(
        ui,
        RichText::new(broker_summary(broker)).color(palette.secondary_label),
        subheadline(),
        width - TEXT_INSET + TILE_INSET,
    );
    let height = words.size().y.max(8.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let text = broker_text(broker);
    // Static text, read out in full: its value is the whole sentence.
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &text));
    // The dot under the tiles' column, the words lined up with the rows' text.
    let dot = pos2(rect.left() + TILE_INSET + TILE / 2.0, rect.center().y);
    ui.painter()
        .circle_filled(dot, DOT_RADIUS, broker_color(&palette, broker));
    let at = pos2(
        rect.left() + TEXT_INSET - GRID,
        rect.center().y - words.size().y / 2.0,
    );
    ui.painter().galley(at, words, palette.secondary_label);
    response.on_hover_text(text);
}

/// The colors of a row: its fill and text, and what its tile and status dot sit on.
struct RowInks {
    fill: Option<Color32>,
    name: Color32,
    detail: Color32,
    ground: Color32,
}

fn row_inks(ui: &egui::Ui, selected: bool) -> RowInks {
    let palette = Palette::of(ui);
    if selected {
        RowInks {
            fill: Some(palette.accent),
            name: Color32::WHITE,
            detail: Color32::from_white_alpha(204),
            ground: palette.accent,
        }
    } else {
        RowInks {
            fill: None,
            name: palette.label,
            detail: palette.secondary_label,
            ground: palette.sidebar,
        }
    }
}

/// The tile a row's glyph sits on, at the row's left: tinted with the accent when
/// `lit`, grey otherwise, and white on a selected row. Returns the glyph's colors.
fn row_tile(ui: &egui::Ui, row: Rect, inks: &RowInks, lit: bool, selected: bool) -> (Rect, Ink) {
    let palette = Palette::of(ui);
    let tile = Rect::from_min_size(
        pos2(row.left() + TILE_INSET, row.center().y - TILE / 2.0),
        vec2(TILE, TILE),
    );
    let (fill, glyph) = if selected {
        (Color32::from_white_alpha(56), Color32::WHITE)
    } else if lit {
        (Palette::tint(palette.accent, 16), palette.accent)
    } else {
        (Palette::tint(palette.gray, 18), palette.gray)
    };
    let fill = solid(fill, inks.ground);
    glyphs::tile(ui.painter(), tile, fill);
    let ink = Ink {
        body: glyph,
        arcs: glyph,
        ground: fill,
        track: None,
    };
    (tile, ink)
}

/// A row's name, and the words under it, past the tile.
fn row_text(ui: &egui::Ui, row: Rect, inks: &RowInks, name: &str, detail: &str, end: f32) {
    let width = (row.right() - end - row.left() - TEXT_INSET - GRID).max(0.0);
    let name = one_line(ui, name, TextStyle::Body, width);
    let detail = one_line(ui, detail, subheadline(), width);
    let top = row.center().y - (name.size().y + detail.size().y) / 2.0;
    let name_at = pos2(row.left() + TEXT_INSET, top);
    let detail_at = pos2(name_at.x, top + name.size().y);
    let painter = ui.painter();
    painter.galley(name_at, name, inks.name);
    painter.galley(detail_at, detail, inks.detail);
}

/// A diffuser's row: its tile (the glyph for its model, with arcs for its level while it
/// diffuses, tinted when it answers) with the status dot on the tile's corner, its name,
/// and under it what it is doing ([`second_line`]). The selected row is an inset rounded
/// rectangle in the accent color with white text. The tooltip says whether it answers
/// in full.
fn device_row(
    ui: &mut egui::Ui,
    device: &DeviceView,
    online: Online,
    selected: bool,
) -> egui::Response {
    let palette = Palette::of(ui);
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
    let name = display_name(device);
    let detail = second_line(device, online);
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            true,
            selected,
            format!("{name}, {detail}"),
        )
    });
    if ui.is_rect_visible(rect) {
        let inks = row_inks(ui, selected);
        if let Some(fill) = inks.fill {
            ui.painter().rect_filled(rect, CONTROL_RADIUS, fill);
        }
        let (tile, ink) = row_tile(ui, rect, &inks, online == Online::Online, selected);
        let arcs = tile_arcs(device);
        let at = glyphs::still(tile, arcs);
        match Model::of(device) {
            Model::Hvac2 => glyphs::hvac2(ui.painter(), at, &ink, arcs),
            _ => glyphs::air2(ui.painter(), at, &ink, arcs),
        }
        // The dot on the tile's lower trailing corner, ringed in the row's color.
        let dot = tile.right_bottom() - vec2(DOT_RADIUS - 1.0, DOT_RADIUS - 1.0);
        let painter = ui.painter();
        painter.circle_filled(dot, DOT_RADIUS + DOT_RING, inks.ground);
        painter.circle_filled(dot, DOT_RADIUS, status_color(&palette, online));
        row_text(ui, rect, &inks, &name, &detail, 0.0);
    }
    response.on_hover_text(online_text(online))
}

/// A row of what the scan found: its tile (grey: it isn't on the broker), the name its
/// model gives it, its signal in words, and the signal bars at the end, the dBm in the
/// tooltip. Selecting it opens the way to add it.
fn hit_row(ui: &mut egui::Ui, hit: &ScanHit, selected: bool) -> egui::Response {
    let palette = Palette::of(ui);
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
    let name = name_from_scan(&hit.name);
    let quality = hit.rssi.map(|dbm| signal_quality(dbm.into()));
    let detail = quality.map_or_else(
        || "Signal unknown".to_owned(),
        |quality| format!("{} signal", quality.words()),
    );
    let dbm = hit
        .rssi
        .map_or_else(|| "No signal reading".to_owned(), |dbm| signal(dbm.into()));
    response.widget_info(|| {
        let mut info = WidgetInfo::selected(
            WidgetType::SelectableLabel,
            true,
            selected,
            format!("{name}, {detail}"),
        );
        info.current_text_value = Some(dbm.clone());
        info
    });
    if ui.is_rect_visible(rect) {
        let inks = row_inks(ui, selected);
        if let Some(fill) = inks.fill {
            ui.painter().rect_filled(rect, CONTROL_RADIUS, fill);
        }
        let (tile, ink) = row_tile(ui, rect, &inks, false, selected);
        let at = glyphs::still(tile, 0.0);
        match Model::from_scan(&hit.name) {
            Model::Hvac2 => glyphs::hvac2(ui.painter(), at, &ink, 0.0),
            _ => glyphs::air2(ui.painter(), at, &ink, 0.0),
        }
        let mut end = 0.0;
        if let Some(quality) = quality {
            let bars = Rect::from_min_size(
                pos2(rect.right() - GRID - BARS.x, rect.center().y - BARS.y / 2.0),
                BARS,
            );
            let (on, off) = if selected {
                (Color32::WHITE, Color32::from_white_alpha(90))
            } else {
                (
                    solid(palette.label, palette.sidebar),
                    Palette::tint(palette.label, 15),
                )
            };
            glyphs::signal_bars(ui.painter(), bars, on, off, quality.bars());
            end = BARS.x + GRID;
        }
        row_text(ui, rect, &inks, &name, &detail, end);
    }
    response.on_hover_text(dbm)
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use scent_core::DeviceInfo;

    use std::path::Path;

    use super::super::theme::{self, SIDEBAR_MIN_WIDTH};
    use super::*;

    const ONE: &str = "TESTDEVICE01";

    fn with_model(model: &str) -> DeviceView {
        let mut device = DeviceView::new(ONE);
        device.info = Some(DeviceInfo {
            manufacturer_id: "TEST".into(),
            model_name: model.into(),
            variant_name: "TEST".into(),
            device_id: ONE.into(),
            firmware: "2.30".into(),
            wifi_configured: true,
            wifi_status: 3,
        });
        device
    }

    fn read(device: &mut DeviceView, field: Field, value: i64) {
        let reading = crate::state::Reading {
            value,
            at: 0,
            via: scent_core::Via::Wifi,
        };
        device.readings.insert(field, reading);
    }

    #[test]
    fn a_row_says_in_words_what_the_diffuser_is_doing() {
        // Color is never the only signal: the second line has the words.
        let mut d = with_model("Diffuser Air 2");
        assert_eq!(second_line(&d, Online::Online), "Answering");
        read(&mut d, Field::Power, 1);
        assert_eq!(second_line(&d, Online::Online), "Diffusing");
        read(&mut d, Field::Intensity, 3);
        assert_eq!(second_line(&d, Online::Online), "Diffusing at High");
        assert_eq!(tile_arcs(&d), 3.0);
        read(&mut d, Field::Power, 0);
        assert_eq!(second_line(&d, Online::Online), "Off");
        assert_eq!(tile_arcs(&d), 0.0);
        // What was sent and not yet reported on counts.
        d.sent = Some(crate::state::Sent {
            power: Some(true),
            intensity: Some(1),
            at: 10,
        });
        assert_eq!(second_line(&d, Online::Online), "Diffusing at Low");
        // Not answering says so, whatever it last reported.
        assert_eq!(second_line(&d, Online::Offline), "Not answering");
        assert_eq!(second_line(&d, Online::Unknown), "Not pinged yet");
    }

    #[test]
    fn the_sidebar_says_waiting_in_one_word() {
        assert_eq!(
            second_line(&with_model("Diffuser Air 2"), Online::Waiting),
            "Waiting"
        );
    }

    #[test]
    fn at_the_narrowest_sidebar_the_words_are_not_cut() {
        // The width a row's text has when the sidebar is as narrow as it goes.
        let width = SIDEBAR_MIN_WIDTH - 2.0 * GRID - TEXT_INSET - GRID;
        let mut frames = 0;
        let mut harness = Harness::new_ui_state(
            |ui, lines: &mut Vec<(String, bool)>| {
                // The app's text styles, in egui's fonts: the first frame installs them.
                frames += 1;
                if frames == 1 {
                    theme::install(ui.ctx(), Path::new("/nonexistent"));
                    ui.ctx().request_repaint();
                    return;
                }
                lines.clear();
                for words in ["Diffusing at Medium", "Not answering", "Not pinged yet"] {
                    let line = one_line(ui, words, subheadline(), width);
                    lines.push((line.text().to_owned(), line.elided));
                }
            },
            Vec::new(),
        );
        harness.run();
        for (words, elided) in harness.state() {
            assert!(!elided, "{words:?} was cut short");
        }
    }

    #[test]
    fn what_the_scan_found_is_read_out() {
        let state = AppState {
            scan: vec![hit("ble-new", "Scentiment")],
            ..AppState::default()
        };
        let harness = Harness::new_ui_state(
            |ui, list: &mut DeviceList| {
                let (mut device, mut found) = (None, None);
                let selection = Selection {
                    device: &mut device,
                    found: &mut found,
                };
                let _ = list.show(ui, &state, selection, 0);
            },
            DeviceList::default(),
        );
        // Its signal in words; the dBm are its value, and in the tooltip.
        let hit = harness.get_by_label("Diffuser, Good signal");
        assert_eq!(hit.accesskit_node().value().as_deref(), Some("-60 dBm"));
    }

    fn hit(ble_id: &str, name: &str) -> ScanHit {
        ScanHit {
            ble_id: ble_id.to_owned(),
            name: name.to_owned(),
            rssi: Some(-60),
        }
    }

    #[test]
    fn found_nearby_leaves_out_the_diffusers_we_know() {
        let mut state = AppState::default();
        let mut known = DeviceView::new(ONE);
        known.ble_id = Some("ble-known".to_owned());
        state.devices.insert(ONE.to_owned(), known);
        // A diffuser we know by ID but have not met over Bluetooth has no Bluetooth ID,
        // so it matches no hit.
        state
            .devices
            .insert("TESTDEVICE02".to_owned(), DeviceView::new("TESTDEVICE02"));
        state.scan = vec![hit("ble-known", "Air 2 known"), hit("ble-new", "Air 2 new")];

        let names: Vec<&str> = unknown_hits(&state)
            .iter()
            .map(|hit| hit.name.as_str())
            .collect();
        assert_eq!(names, ["Air 2 new"]);
    }

    #[test]
    fn a_scan_ends_with_its_result_or_its_failure() {
        let mut list = DeviceList {
            scanning: Some(Instant::now()),
            scanned: false,
        };
        assert!(list.is_scanning());
        list.on_event(&Event::Scanned(vec![]));
        assert!(!list.is_scanning() && list.scanned);

        list.scanning = Some(Instant::now());
        list.on_event(&Event::Failed {
            device_id: None,
            what: SCAN_FAILED.to_owned(),
            error: "Bluetooth is off.".to_owned(),
            at: 0,
        });
        assert!(!list.is_scanning());

        // Another failure is not the scan's.
        list.scanning = Some(Instant::now());
        list.on_event(&Event::Failed {
            device_id: Some(ONE.to_owned()),
            what: "TURN_ON".to_owned(),
            error: "No answer.".to_owned(),
            at: 0,
        });
        assert!(list.is_scanning());
    }
}
