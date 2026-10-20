//! The app shell: it owns the hub, the shared state and the store, and each frame it
//! folds the hub's events into the state and shows the screens.
//!
//! The state is shared with the hub, which only reads it. The lock is held to fold
//! events in and to copy the state out; the screens draw and the store writes from the
//! copies.

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::hub::{Hub, UiCommand};
use crate::mqtt::BrokerSettings;
use crate::secrets::{broker_password, set_broker_password};
use crate::state::{AppState, BrokerState, Event, unix_now};
use crate::store::{Loaded, Store};
use crate::transport::lock;
use crate::ui::Screens;
use crate::ui::settings::Submitted;

/// The least time between two saves of `desktop.json`.
const SAVE_EVERY: Duration = Duration::from_secs(10);

/// How often the screens redraw when nothing happens, so that ages stay current.
const REDRAW_EVERY: Duration = Duration::from_secs(1);

pub struct ScentApp {
    state: Arc<Mutex<AppState>>,
    events: Receiver<Event>,
    hub: Hub,
    store: Store,
    /// Where the broker is, as last saved. The password is in the Keychain.
    broker: BrokerSettings,
    screens: Screens,
    /// Whether something that is saved has changed since the last save.
    dirty: bool,
    saved_at: Instant,
    /// The error of the last failed save, so that it is told once and not every time.
    save_error: Option<String>,
}

impl ScentApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let store = Store {
            dir: Store::default_dir(),
        };
        let Loaded {
            state,
            broker,
            notice,
        } = store.load();
        // The Keychain may ask the person before it answers; nothing is drawn until it has.
        let password = broker_password(&broker);
        let state = Arc::new(Mutex::new(state));
        let ctx = cc.egui_ctx.clone();
        let (hub, events) = Hub::start(broker.clone(), password, Arc::clone(&state), move || {
            ctx.request_repaint();
        });
        ScentApp {
            state,
            events,
            hub,
            store,
            broker,
            screens: Screens::new(notice),
            dirty: false,
            saved_at: Instant::now(),
            save_error: None,
        }
    }

    /// Folds what the hub has sent into the state.
    fn drain_events(&mut self) {
        let events: Vec<Event> = self.events.try_iter().collect();
        if events.is_empty() {
            return;
        }
        for event in &events {
            self.screens.on_event(event);
        }
        let mut state = lock(&self.state);
        for event in events {
            state.apply(event);
        }
        drop(state);
        self.dirty = true;
    }

    /// Stores the settings, the password and the new connection, in that order, then
    /// tells the sheet how it went.
    fn apply_settings(&mut self, ctx: &egui::Context, submitted: Submitted) {
        let Submitted { settings, password } = submitted;
        self.broker = settings.clone();
        self.save();
        let keychain = password
            .as_ref()
            .and_then(|password| set_broker_password(&settings, password).err());
        // With nothing typed, the saved password: it may belong to other settings now.
        let password = password.or_else(|| broker_password(&settings));
        // The hub doesn't announce a connection attempt, so say so here.
        lock(&self.state).broker = BrokerState::Connecting;
        self.hub.submit(UiCommand::Reconnect { settings, password });
        match keychain {
            // The typed password is still used, for this run.
            Some(error) => self.screens.settings.fail(error),
            None => self.screens.settings.close(ctx),
        }
    }

    /// Writes `desktop.json` when something changed and it has been long enough.
    fn save_if_due(&mut self) {
        if self.dirty && self.saved_at.elapsed() >= SAVE_EVERY {
            self.save();
        }
    }

    /// Writes `desktop.json` now. Copies the devices under the lock and writes outside
    /// it. A failure is told to the person once, until a save works again.
    fn save(&mut self) {
        let devices = lock(&self.state).devices.clone();
        let snapshot = AppState {
            devices,
            ..AppState::default()
        };
        self.dirty = false;
        self.saved_at = Instant::now();
        match self.store.save(&snapshot, &self.broker) {
            Ok(()) => self.save_error = None,
            Err(error) => {
                let error = format!(
                    "Scent Control couldn't save its diffusers to {}: {error}",
                    self.store.dir.display()
                );
                eprintln!("scent-control: {error}");
                if self.save_error.as_ref() != Some(&error) {
                    self.screens.set_notice(error.clone());
                }
                self.save_error = Some(error);
                self.dirty = true;
            }
        }
    }
}

impl eframe::App for ScentApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_events();
        let now = unix_now();
        let view = lock(&self.state).clone();

        let requests = self.screens.show(ui, &view, &self.broker, now);
        for command in requests.commands {
            self.hub.submit(command);
        }
        if let Some(submitted) = requests.settings {
            self.apply_settings(ui.ctx(), submitted);
        }

        self.save_if_due();
        ui.ctx().request_repaint_after(REDRAW_EVERY);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.drain_events();
        self.save();
    }
}
