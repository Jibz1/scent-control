//! Scent Control: native macOS app for Scentiment diffusers.
//!
//! macOS only. Bluetooth works only when this binary runs from the app bundle that
//! `scripts/bundle-macos.sh` builds (its Info.plist declares the Bluetooth usage
//! description), so launch it with `open "target/Scent Control.app"`.

#[cfg(not(target_os = "macos"))]
compile_error!("scent-desktop is macOS only");

#[cfg(target_os = "macos")]
mod add_flow;

#[cfg(target_os = "macos")]
mod app;

#[cfg(target_os = "macos")]
mod ble;

#[cfg(target_os = "macos")]
mod hub;

#[cfg(target_os = "macos")]
mod mqtt;

#[cfg(target_os = "macos")]
mod route;

#[cfg(target_os = "macos")]
mod secrets;

#[cfg(target_os = "macos")]
mod state;

#[cfg(target_os = "macos")]
mod store;

#[cfg(target_os = "macos")]
mod transport;

#[cfg(target_os = "macos")]
mod ui;

#[cfg(target_os = "macos")]
use app::ScentApp;

#[cfg(target_os = "macos")]
fn main() -> eframe::Result {
    // The content runs under a transparent title bar with no title, so the traffic
    // lights sit over the sidebar, as in a Mac app's split view.
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Scent Control")
            .with_inner_size(ui::theme::WINDOW_SIZE)
            .with_min_inner_size(ui::theme::MIN_WINDOW_SIZE)
            .with_fullsize_content_view(true)
            .with_title_shown(false)
            .with_titlebar_shown(false),
        ..Default::default()
    };
    eframe::run_native(
        "Scent Control",
        options,
        Box::new(|cc| Ok(Box::new(ScentApp::new(cc)))),
    )
}
