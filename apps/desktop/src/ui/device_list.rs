//! The diffuser list on the left: each known diffuser with its model and whether it
//! answers, then the Bluetooth scan and what it found.

use std::time::{Duration, Instant};

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{self, Align, FontId, Layout, RichText, WidgetText};
use scent_core::Online;

use super::format::{model_text, online_text, scan_signal};
use super::online_color;
use crate::hub::{SCAN_FAILED, UiCommand};
use crate::state::{AppState, DeviceView, Event, ScanHit};

/// How long "Scanning…" stays up if no answer ever comes. The hub gives up on a scan
/// itself after 30 s.
const SCAN_GIVE_UP: Duration = Duration::from_secs(35);

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

    /// Shows the list and returns what was asked for. `selected` is the diffuser that
    /// is open, and changes when another is clicked.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        selected: &mut Option<String>,
        now: i64,
    ) -> Vec<UiCommand> {
        let mut commands = Vec::new();
        ui.heading("Diffusers");
        if state.devices.is_empty() {
            ui.weak("No diffusers yet. Scan to find the ones in Bluetooth range.");
        }
        // Justified, so that each row is as wide as the list and its text sits at the left.
        ui.with_layout(Layout::top_down_justified(Align::Min), |ui| {
            for device in state.devices.values() {
                let online = state.online(&device.id, now);
                let chosen = selected.as_deref() == Some(device.id.as_str());
                let row = WidgetText::from(row_text(ui, device, online));
                if ui.selectable_label(chosen, row).clicked() {
                    *selected = Some(device.id.clone());
                }
            }
        });

        ui.separator();
        ui.horizontal(|ui| {
            let scanning = self.is_scanning();
            if ui
                .add_enabled(!scanning, egui::Button::new("Scan for diffusers"))
                .clicked()
            {
                self.scanning = Some(Instant::now());
                commands.push(UiCommand::Scan);
            }
            if scanning {
                ui.spinner();
                ui.weak("Scanning…");
            }
        });
        self.found_nearby(ui, state);
        commands
    }

    fn found_nearby(&self, ui: &mut egui::Ui, state: &AppState) {
        let hits = unknown_hits(state);
        if hits.is_empty() {
            if self.scanned && !self.is_scanning() {
                ui.weak("No new diffusers found nearby.");
            }
            return;
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Found nearby").strong());
        for hit in hits {
            ui.horizontal_wrapped(|ui| {
                ui.label(&hit.name);
                ui.weak(scan_signal(hit.rssi));
            });
        }
    }
}

/// A row: the diffuser's name, its model, and whether it answers, in that color.
fn row_text(ui: &egui::Ui, device: &DeviceView, online: Online) -> LayoutJob {
    let format = |size: f32, color| TextFormat {
        font_id: FontId::proportional(size),
        color,
        ..TextFormat::default()
    };
    let mut job = LayoutJob::default();
    job.append(
        &device.label,
        0.0,
        format(15.0, ui.visuals().strong_text_color()),
    );
    let model = model_text(device.info.as_ref());
    if !model.is_empty() {
        job.append(
            &format!("\n{model}"),
            0.0,
            format(12.0, ui.visuals().weak_text_color()),
        );
    }
    job.append(
        &format!("\n{}", online_text(online)),
        0.0,
        format(12.0, online_color(ui, online)),
    );
    job
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
