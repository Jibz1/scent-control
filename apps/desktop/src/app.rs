//! The app shell: it owns the hub, the shared state and the store, and each frame it
//! folds the hub's events into the state and shows the screens.
//!
//! The state is shared with the hub, which only reads it. The lock is held to fold
//! events in and to copy the state out; the screens draw and the store writes from the
//! copies.
//!
//! The saved file is written only when what it holds has changed (new readings, a new
//! name, other broker settings), not for every ping and reply: a write is a full
//! flush to disk, and it is done here, on the thread that draws.

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
    /// What `desktop.json` holds as far as we know: what was loaded or last written.
    /// `None` when the file is to be rewritten whatever it holds (it wasn't usable).
    written: Option<Vec<u8>>,
    /// When we last looked at whether to save.
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
        let written = notice
            .is_none()
            .then(|| Store::contents(&state, &broker).ok())
            .flatten();
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
            written,
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
    }

    /// Stores the password, the settings and the new connection, in that order, then
    /// tells the sheet how it went. If the Keychain won't take a typed password,
    /// nothing is changed: the settings are not saved and the connection that works is
    /// left alone. The password is not kept, so the sheet says to type it again.
    fn apply_settings(&mut self, ctx: &egui::Context, submitted: Submitted) {
        let Submitted { settings, password } = submitted;
        if let Some(password) = &password
            && let Err(error) = set_broker_password(&settings, password)
        {
            let error = error.trim_end_matches('.');
            self.screens.settings.fail(format!(
                "{error}. Nothing was saved and the connection is unchanged. Type the password again to try again."
            ));
            return;
        }
        self.broker = settings.clone();
        self.save();
        // With nothing typed, the saved password: it may belong to other settings now.
        let password = password.or_else(|| broker_password(&settings));
        // The hub doesn't announce a connection attempt, so say so here.
        lock(&self.state).broker = BrokerState::Connecting;
        self.hub.submit(UiCommand::Reconnect { settings, password });
        self.screens.settings.close(ctx);
    }

    /// Writes `desktop.json` if it is time to look and something saved has changed.
    fn save_if_due(&mut self) {
        if self.saved_at.elapsed() >= SAVE_EVERY {
            self.save();
        }
    }

    /// Writes `desktop.json` if what it would hold differs from what it holds. Copies
    /// the devices under the lock and writes outside it. A failure is told to the
    /// person once, until a save works again.
    fn save(&mut self) {
        let devices = lock(&self.state).devices.clone();
        let snapshot = AppState {
            devices,
            ..AppState::default()
        };
        self.saved_at = Instant::now();
        let written = Store::contents(&snapshot, &self.broker).and_then(|json| {
            if self.written.as_ref() == Some(&json) {
                return Ok(None);
            }
            self.store.write(&json).map(|()| Some(json))
        });
        match written {
            Ok(Some(json)) => {
                self.written = Some(json);
                self.save_error = None;
            }
            Ok(None) => {}
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
