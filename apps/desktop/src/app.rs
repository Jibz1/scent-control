//! The app shell: a left panel for the diffuser list and a central panel for the rest.

use eframe::egui;

/// Width of the diffuser list panel when the window first opens.
const SIDE_PANEL_WIDTH: f32 = 260.0;

pub struct ScentApp;

impl ScentApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self
    }
}

impl eframe::App for ScentApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::left("diffusers")
            .default_size(SIDE_PANEL_WIDTH)
            .show(ui, |ui| {
                ui.heading("Diffusers");
            });
        egui::CentralPanel::default().show(ui, |_ui| {});
    }
}
