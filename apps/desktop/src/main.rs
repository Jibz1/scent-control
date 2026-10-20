//! Scent Control: native macOS app for Scentiment diffusers.
//!
//! macOS only. Bluetooth works only when this binary runs from the app bundle that
//! `scripts/bundle-macos.sh` builds (its Info.plist declares the Bluetooth usage
//! description), so launch it with `open "target/Scent Control.app"`.

#[cfg(not(target_os = "macos"))]
compile_error!("scent-desktop is macOS only");

#[cfg(target_os = "macos")]
mod app;

// The hub and the screens (later tasks) are what use the state and the store; until
// then the binary on its own would flag them as dead code.
#[cfg(target_os = "macos")]
#[allow(dead_code)]
mod state;

// The screens (Task 9) start the hub and read the broker password from the Keychain;
// the hub starts the two workers and decides the route. Until then the binary on its
// own would flag all of these as dead code.
#[cfg(target_os = "macos")]
#[allow(dead_code)]
mod ble;

#[cfg(target_os = "macos")]
#[allow(dead_code)]
mod hub;

#[cfg(target_os = "macos")]
#[allow(dead_code)]
mod mqtt;

#[cfg(target_os = "macos")]
#[allow(dead_code)]
mod route;

#[cfg(target_os = "macos")]
#[allow(dead_code)]
mod secrets;

#[cfg(target_os = "macos")]
#[allow(dead_code)]
mod store;

#[cfg(target_os = "macos")]
#[allow(dead_code)]
mod transport;

#[cfg(target_os = "macos")]
use app::ScentApp;

#[cfg(target_os = "macos")]
fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Scent Control")
            .with_inner_size([1100.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Scent Control",
        options,
        Box::new(|cc| Ok(Box::new(ScentApp::new(cc)))),
    )
}
