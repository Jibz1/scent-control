//! The sidebar, a source list: the known diffusers under "Diffusers", each with its
//! status dot, name and model, then what the Bluetooth scan found under "Found Nearby",
//! and the scan button at the bottom.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, FontSelection, Rect, RichText, Sense, Spinner, TextStyle, TextWrapMode,
    WidgetInfo, WidgetText, WidgetType, pos2, vec2,
};
use scent_core::Online;

use super::format::{online_text, scan_signal};
use super::theme::{CONTROL_RADIUS, GRID, Palette, status_color, subheadline};
use super::widgets::plain_button;
use crate::hub::{SCAN_FAILED, UiCommand};
use crate::state::{AppState, DeviceView, Event, ScanHit};

/// How long "Scanning…" stays up if no answer ever comes. The hub gives up on a scan
/// itself after 30 s.
const SCAN_GIVE_UP: Duration = Duration::from_secs(35);

/// A diffuser's row: its name over its model.
const DEVICE_ROW_HEIGHT: f32 = 40.0;

/// A row of what the scan found: one line.
const HIT_ROW_HEIGHT: f32 = 24.0;

/// Where a row's text starts, past the status dot.
const TEXT_INSET: f32 = 26.0;

/// Where the status dot's center is, from the row's left edge.
const DOT_INSET: f32 = 13.0;

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
    /// diffuser that is open, and changes when another is clicked.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        selected: &mut Option<String>,
        now: i64,
    ) -> Vec<UiCommand> {
        let mut commands = Vec::new();
        let scanning = self.is_scanning();
        // The scan button along the bottom, so the lists above have the rest.
        egui::Panel::bottom("sidebar_actions")
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 12)))
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
                self.found_nearby(ui, state, scanning);
            });
        commands
    }

    fn found_nearby(&self, ui: &mut egui::Ui, state: &AppState, scanning: bool) {
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
            hit_row(ui, hit);
        }
    }
}

/// A section heading in the sidebar: Subheadline, in the secondary label color.
fn section_heading(ui: &mut egui::Ui, title: &str) {
    let color = Palette::of(ui).secondary_label;
    ui.horizontal(|ui| {
        ui.add_space(DOT_INSET - 4.0);
        ui.label(RichText::new(title).text_style(subheadline()).color(color));
    });
    ui.add_space(2.0);
}

/// Secondary text in the sidebar, lined up with the rows' text.
fn note(ui: &mut egui::Ui, text: &str) {
    let color = Palette::of(ui).secondary_label;
    ui.horizontal(|ui| {
        ui.add_space(TEXT_INSET);
        ui.add(egui::Label::new(RichText::new(text).text_style(subheadline()).color(color)).wrap());
    });
}

/// Lays `text` out on one line in `style`, cut short with "…" past `width`.
fn one_line(
    ui: &egui::Ui,
    text: &str,
    style: TextStyle,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    WidgetText::from(text).into_galley(
        ui,
        Some(TextWrapMode::Truncate),
        width,
        FontSelection::Style(style),
    )
}

/// A diffuser's row: the status dot, the name, and the model under it in secondary
/// text. The chosen row is an inset rounded rectangle in the accent color with white
/// text. Whether it answers is in words in the tooltip and to accessibility.
fn device_row(
    ui: &mut egui::Ui,
    device: &DeviceView,
    online: Online,
    chosen: bool,
) -> egui::Response {
    let palette = Palette::of(ui);
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, DEVICE_ROW_HEIGHT), Sense::click());
    let words = online_text(online);
    let model = model_line(device);
    response.widget_info(|| {
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
        let text_width = rect.width() - TEXT_INSET - 8.0;
        let name = one_line(ui, &device.label, TextStyle::Body, text_width);
        let model = one_line(ui, &model, subheadline(), text_width);
        let lines = name.size().y + model.size().y;
        let top = rect.center().y - lines / 2.0;
        let name_at = pos2(rect.left() + TEXT_INSET, top);
        let dot = pos2(rect.left() + DOT_INSET, name_at.y + name.size().y / 2.0);
        // On the accent color the dot gets a white edge, so that it stands apart.
        if chosen {
            painter.circle_filled(dot, 5.0, Color32::WHITE);
        }
        painter.circle_filled(dot, 4.0, status_color(&palette, online));
        painter.galley(name_at, name.clone(), text);
        painter.galley(pos2(name_at.x, top + name.size().y), model, secondary);
    }
    response.on_hover_text(words)
}

/// A row of what the scan found: the name, and its signal in secondary text.
fn hit_row(ui: &mut egui::Ui, hit: &ScanHit) {
    let palette = Palette::of(ui);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, HIT_ROW_HEIGHT), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let signal = one_line(ui, &scan_signal(hit.rssi), subheadline(), width);
    let name_width = rect.width() - TEXT_INSET - signal.size().x - 16.0;
    let name = one_line(ui, &hit.name, TextStyle::Body, name_width);
    let text = Rect::from_min_max(
        pos2(rect.left() + TEXT_INSET, rect.top()),
        pos2(rect.right() - 8.0, rect.bottom()),
    );
    let painter = ui.painter();
    let name_at = Align2::LEFT_CENTER.align_size_within_rect(name.size(), text);
    painter.galley(name_at.min, name, palette.label);
    let signal_at = Align2::RIGHT_CENTER.align_size_within_rect(signal.size(), text);
    painter.galley(signal_at.min, signal, palette.secondary_label);
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = "TESTDEVICE01";

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
