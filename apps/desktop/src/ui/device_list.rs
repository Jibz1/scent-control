//! The sidebar, a source list: the known diffusers under "Diffusers", each with its
//! status dot, name and model, then what the Bluetooth scan found under "Found Nearby",
//! each with "Add…", and the scan button at the bottom.

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Color32, Galley, Pos2, RichText, Sense, Spinner, TextStyle, WidgetInfo, WidgetType, pos2,
    vec2,
};
use scent_core::Online;

use super::OpenSheet;
use super::add_sheet::{Target, name_from_scan};
use super::format::{online_text, scan_signal};
use super::theme::{CONTROL_RADIUS, GRID, Palette, status_color, subheadline};
use super::widgets::{MIN_BUTTON_WIDTH, one_line, plain_button, secondary};
use crate::hub::{SCAN_FAILED, UiCommand};
use crate::state::{AppState, DeviceView, Event, ScanHit};

/// How long "Scanning…" stays up if no answer ever comes. The hub gives up on a scan
/// itself after 30 s.
const SCAN_GIVE_UP: Duration = Duration::from_secs(35);

/// A diffuser's row: its name over its model.
const DEVICE_ROW_HEIGHT: f32 = 40.0;

/// A row of what the scan found: its name over its signal, as a diffuser's row.
const HIT_ROW_HEIGHT: f32 = DEVICE_ROW_HEIGHT;

/// Where the status dot starts, from the row's left edge, and where its center is.
const DOT_INSET: f32 = GRID;
const DOT_CENTER: f32 = DOT_INSET + 4.0;

/// Where a row's text starts: past the 8 pt dot and a gap as wide.
const TEXT_INSET: f32 = DOT_INSET + 2.0 * GRID;

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

/// The model a diffuser told us, for the second line of its row. Without one, its ID
/// when it goes by another name.
fn model_line(device: &DeviceView) -> String {
    match &device.info {
        Some(info) if !info.model_name.is_empty() => info.model_name.clone(),
        _ if device.label != device.id => device.id.clone(),
        _ => String::new(),
    }
}

/// Whether a diffuser answers, in words short enough for the sidebar: "Waiting" for
/// "Pinged, waiting for an answer". The detail pane and the row's tooltip have the whole
/// wording.
fn sidebar_words(online: Online) -> &'static str {
    match online {
        Online::Waiting => "Waiting",
        other => online_text(other),
    }
}

/// What a diffuser's row says on its second line: its model, then whether it answers,
/// in words, so that the status dot's color is never the only signal.
fn row_parts(device: &DeviceView, online: Online) -> (String, &'static str) {
    (model_line(device), sidebar_words(online))
}

/// The second line in full, as [`second_line`] lays it out when there is room.
#[cfg(test)]
fn row_detail(device: &DeviceView, online: Online) -> String {
    match row_parts(device, online) {
        (model, words) if model.is_empty() => words.to_owned(),
        (model, words) => format!("{model}{SEPARATOR}{words}"),
    }
}

/// Between the model and the words on a row's second line.
const SEPARATOR: &str = " · ";

/// The least room worth giving the model, cut short: a few letters and "…". With less,
/// it is left out.
const MIN_MODEL_WIDTH: f32 = 3.0 * GRID;

/// How wide a row's text may be in a row `row_width` wide: past the dot, with a margin
/// on the right.
fn row_text_width(row_width: f32) -> f32 {
    row_width - TEXT_INSET - GRID
}

/// A row's second line, laid out to fit.
struct SecondLine {
    /// The model, cut short with "…" if need be, and what follows it; none when there is
    /// no model or no room for it.
    model: Option<Arc<Galley>>,
    separator: Option<Arc<Galley>>,
    words: Arc<Galley>,
}

impl SecondLine {
    #[cfg(test)]
    fn width(&self) -> f32 {
        [&self.model, &self.separator, &Some(self.words.clone())]
            .into_iter()
            .flatten()
            .map(|galley| galley.size().x)
            .sum()
    }

    fn height(&self) -> f32 {
        self.words.size().y
    }

    fn paint(self, painter: &egui::Painter, at: Pos2, color: Color32) {
        let mut x = at.x;
        for galley in [self.model, self.separator, Some(self.words)]
            .into_iter()
            .flatten()
        {
            let width = galley.size().x;
            painter.galley(pos2(x, at.y), galley, color);
            x += width;
        }
    }
}

/// A row's second line, the model and the words of [`row_parts`], laid out in `width`:
/// the model is cut short first, or left out, so that the words fit; they are cut only
/// if they can't fit alone.
fn second_line(ui: &egui::Ui, (model, words): (&str, &str), width: f32) -> SecondLine {
    let words = one_line(ui, words, subheadline(), width);
    let separator = one_line(ui, SEPARATOR, subheadline(), f32::INFINITY);
    let room = width - words.size().x - separator.size().x;
    let model = (!model.is_empty() && room >= MIN_MODEL_WIDTH)
        .then(|| one_line(ui, model, subheadline(), room));
    SecondLine {
        separator: model.is_some().then_some(separator),
        model,
        words,
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

    fn is_scanning(&self) -> bool {
        self.scanning
            .is_some_and(|since| since.elapsed() < SCAN_GIVE_UP)
    }

    /// Shows the list in the sidebar and returns what was asked for. `selected` is the
    /// diffuser that is open, and changes when another is clicked; `sheet` is set when
    /// "Add…" was clicked on what the scan found.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        selected: &mut Option<String>,
        now: i64,
        sheet: &mut Option<OpenSheet>,
    ) -> Vec<UiCommand> {
        let mut commands = Vec::new();
        let scanning = self.is_scanning();
        // The scan button along the bottom, so the lists above have the rest.
        egui::Panel::bottom("sidebar_actions")
            .frame(
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(GRID as i8, 2 * GRID as i8)),
            )
            .show_separator_line(false)
            .resizable(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let scan =
                        ui.add_enabled_ui(!scanning, |ui| plain_button(ui, "Scan for Diffusers"));
                    if scan.inner.clicked() {
                        self.scanning = Some(Instant::now());
                        commands.push(UiCommand::Scan);
                    }
                    if scanning {
                        let color = Palette::of(ui).secondary_label;
                        ui.add(Spinner::new().size(14.0).color(color));
                    }
                });
            });

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                section_heading(ui, "Diffusers");
                if state.devices.is_empty() {
                    note(
                        ui,
                        "No diffusers yet. Scan to find the ones in Bluetooth range.",
                    );
                }
                for device in state.devices.values() {
                    let online = state.online(&device.id, now);
                    let chosen = selected.as_deref() == Some(device.id.as_str());
                    if device_row(ui, device, online, chosen).clicked() {
                        *selected = Some(device.id.clone());
                    }
                }
                ui.add_space(2.0 * GRID);
                section_heading(ui, "Found Nearby");
                self.found_nearby(ui, state, scanning, sheet);
            });
        commands
    }

    fn found_nearby(
        &self,
        ui: &mut egui::Ui,
        state: &AppState,
        scanning: bool,
        sheet: &mut Option<OpenSheet>,
    ) {
        let hits = unknown_hits(state);
        if scanning {
            note(ui, "Scanning…");
        }
        if hits.is_empty() {
            if self.scanned && !scanning {
                note(ui, "No new diffusers found nearby.");
            }
            return;
        }
        for hit in hits {
            if hit_row(ui, hit) {
                *sheet = Some(OpenSheet::Add(Target {
                    ble_id: hit.ble_id.clone(),
                    name: name_from_scan(&hit.name),
                }));
            }
        }
    }
}

/// A section heading in the sidebar: Subheadline, in the secondary label color, lined
/// up with the status dots.
fn section_heading(ui: &mut egui::Ui, title: &str) {
    let color = Palette::of(ui).secondary_label;
    ui.horizontal(|ui| {
        ui.add_space(DOT_INSET);
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

/// A diffuser's row: the status dot and the name, and under it in secondary text the
/// model and whether it answers, in words ([`row_detail`]). The chosen row is an inset
/// rounded rectangle in the accent color with white text. The tooltip and the
/// accessibility label have the words in full.
fn device_row(
    ui: &mut egui::Ui,
    device: &DeviceView,
    online: Online,
    chosen: bool,
) -> egui::Response {
    let palette = Palette::of(ui);
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, DEVICE_ROW_HEIGHT), Sense::click());
    // The whole wording, which the row's second line may shorten.
    let words = online_text(online);
    response.widget_info(|| {
        let model = model_line(device);
        let label = [device.label.as_str(), model.as_str(), words]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        WidgetInfo::selected(WidgetType::SelectableLabel, true, chosen, label)
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let (text, secondary) = if chosen {
            painter.rect_filled(rect, CONTROL_RADIUS, palette.accent);
            (Color32::WHITE, Color32::from_white_alpha(204))
        } else {
            (palette.label, palette.secondary_label)
        };
        let text_width = row_text_width(rect.width());
        let name = one_line(ui, &device.label, TextStyle::Body, text_width);
        let (model, short) = row_parts(device, online);
        let detail = second_line(ui, (&model, short), text_width);
        let lines = name.size().y + detail.height();
        let top = rect.center().y - lines / 2.0;
        let name_at = pos2(rect.left() + TEXT_INSET, top);
        let dot = pos2(rect.left() + DOT_CENTER, name_at.y + name.size().y / 2.0);
        // On the accent color the dot gets a white edge, so that it stands apart.
        if chosen {
            painter.circle_filled(dot, 5.0, Color32::WHITE);
        }
        painter.circle_filled(dot, 4.0, status_color(&palette, online));
        let detail_at = pos2(name_at.x, top + name.size().y);
        painter.galley(name_at, name, text);
        detail.paint(painter, detail_at, secondary);
    }
    response.on_hover_text(words)
}

/// A row of what the scan found: the name, its signal under it in secondary text, and
/// "Add…" at the end. True when Add… was clicked.
fn hit_row(ui: &mut egui::Ui, hit: &ScanHit) -> bool {
    let palette = Palette::of(ui);
    ui.horizontal(|ui| {
        let width = (ui.available_width() - MIN_BUTTON_WIDTH - GRID).max(0.0);
        let (rect, response) = ui.allocate_exact_size(vec2(width, HIT_ROW_HEIGHT), Sense::hover());
        let signal = scan_signal(hit.rssi);
        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Label, true, format!("{}, {signal}", hit.name))
        });
        if ui.is_rect_visible(rect) {
            let text_width = (rect.width() - TEXT_INSET).max(0.0);
            let name = one_line(ui, &hit.name, TextStyle::Body, text_width);
            let signal = one_line(ui, &signal, subheadline(), text_width);
            let top = rect.center().y - (name.size().y + signal.size().y) / 2.0;
            let name_at = pos2(rect.left() + TEXT_INSET, top);
            let signal_at = pos2(name_at.x, top + name.size().y);
            let painter = ui.painter();
            painter.galley(name_at, name, palette.label);
            painter.galley(signal_at, signal, palette.secondary_label);
        }
        plain_button(ui, "Add…")
            .on_hover_text("Puts this diffuser on your broker")
            .clicked()
    })
    .inner
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;
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

    #[test]
    fn a_row_says_in_words_whether_the_diffuser_answers() {
        // Color is never the only signal: the second line has the words too.
        assert_eq!(
            row_detail(&with_model("Diffuser Air 2"), Online::Online),
            "Diffuser Air 2 · Answering"
        );
        assert_eq!(
            row_detail(&DeviceView::new(ONE), Online::Unknown),
            "Not pinged yet"
        );
        let mut named = DeviceView::new(ONE);
        named.label = "Bedroom".to_owned();
        assert_eq!(
            row_detail(&named, Online::Offline),
            "TESTDEVICE01 · Not answering pings"
        );
    }

    #[test]
    fn the_sidebar_says_waiting_in_one_word() {
        assert_eq!(
            row_detail(&with_model("Diffuser Air 2"), Online::Waiting),
            "Diffuser Air 2 · Waiting"
        );
    }

    #[test]
    fn at_the_narrowest_sidebar_the_model_is_cut_and_the_words_are_not() {
        // The width a row's text has when the sidebar is as narrow as it goes.
        let width = row_text_width(SIDEBAR_MIN_WIDTH - 2.0 * GRID);
        let mut frames = 0;
        let mut harness = Harness::new_ui_state(
            |ui, lines: &mut Vec<(Option<bool>, String, bool, f32)>| {
                // The app's text styles, in egui's fonts: the first frame installs them.
                frames += 1;
                if frames == 1 {
                    theme::install(ui.ctx(), Path::new("/nonexistent"));
                    ui.ctx().request_repaint();
                    return;
                }
                lines.clear();
                for online in [Online::Waiting, Online::Offline, Online::Unknown] {
                    let line = second_line(ui, ("Diffuser HVAC 2", sidebar_words(online)), width);
                    lines.push((
                        line.model.as_ref().map(|model| model.elided),
                        line.words.text().to_owned(),
                        line.words.elided,
                        line.width(),
                    ));
                }
            },
            Vec::new(),
        );
        harness.run();
        let lines = harness.state();
        assert_eq!(lines[0].1, "Waiting");
        assert_eq!(lines[1].1, "Not answering pings");
        for (_, words, elided, used) in lines {
            assert!(!elided, "{words:?} was cut short");
            assert!(*used <= width + 0.5, "{used} > {width}");
        }
        // The longest words leave the model too little room: it is cut, or left out.
        assert_ne!(lines[1].0, Some(false));
    }

    #[test]
    fn what_the_scan_found_is_read_out() {
        let state = AppState {
            scan: vec![hit("ble-new", "Scentiment")],
            ..AppState::default()
        };
        let harness = Harness::new_ui_state(
            |ui, list: &mut DeviceList| {
                let mut selected = None;
                let _ = list.show(ui, &state, &mut selected, 0, &mut None);
            },
            DeviceList::default(),
        );
        let _ = harness.get_by_value("Scentiment, -60 dBm");
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
