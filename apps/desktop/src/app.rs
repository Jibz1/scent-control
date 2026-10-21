//! The app shell: it owns the hub, the shared state and the store, and each frame it
//! folds the hub's events into the state and shows the screens.
//!
//! The state is shared with the hub, which only reads it. The lock is held to fold
//! events in and to copy the state out; the screens draw and the store writes from the
//! copies. Only this thread changes the state, so the screens' copy is taken when it
//! changes, not every frame: a frame draws from the copy and locks nothing.
//!
//! The saved file is written only when what it holds has changed (new readings, a new
//! name, a schedule sent, other broker settings, another appearance), not for every ping
//! and reply: a write is a full flush to disk, and it is done here, on the thread that
//! draws.

use std::path::Path;
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
use crate::ui::theme::{self, Appearance};

/// The least time between two saves of `desktop.json`.
const SAVE_EVERY: Duration = Duration::from_secs(10);

/// How often the screens redraw when nothing happens, so that ages stay current.
const REDRAW_EVERY: Duration = Duration::from_secs(1);

pub struct ScentApp {
    state: Arc<Mutex<AppState>>,
    /// The screens' copy of the state, as it was after it last changed.
    view: AppState,
    events: Receiver<Event>,
    hub: Hub,
    store: Store,
    /// Where the broker is, as last saved. The password is in the Keychain.
    broker: BrokerSettings,
    /// How the app looks, as chosen in Settings and saved with the rest.
    appearance: Appearance,
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
        theme::install(&cc.egui_ctx, Path::new(theme::SYSTEM_FONTS));
        let store = Store {
            dir: Store::default_dir(),
        };
        let Loaded {
            state,
            broker,
            appearance,
            notice,
        } = store.load();
        theme::set_appearance(&cc.egui_ctx, appearance);
        // The Keychain may ask the person before it answers; nothing is drawn until it has.
        let password = broker_password(&broker);
        let written = notice
            .is_none()
            .then(|| Store::contents(&state, &broker, appearance).ok())
            .flatten();
        let view = state.clone();
        let state = Arc::new(Mutex::new(state));
        let ctx = cc.egui_ctx.clone();
        let (hub, events) = Hub::start(broker.clone(), password, Arc::clone(&state), move || {
            ctx.request_repaint();
        });
        ScentApp {
            state,
            view,
            events,
            hub,
            store,
            broker,
            appearance,
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
        self.change_state(|state| {
            for event in events {
                state.apply(event);
            }
        });
    }

    /// Changes the shared state, and the screens' copy of it to match.
    fn change_state(&mut self, change: impl FnOnce(&mut AppState)) {
        let mut state = lock(&self.state);
        change(&mut state);
        self.view.clone_from(&state);
    }

    /// Stores the password, the settings and the new connection, in that order, then
    /// tells the Settings window how it went. If the Keychain won't take a typed
    /// password, nothing is changed: the settings are not saved and the connection that
    /// works is left alone. The password is not kept, so the window says to type it again.
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
        self.change_state(|state| state.broker = BrokerState::Connecting);
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
        let written = Store::contents(&snapshot, &self.broker, self.appearance).and_then(|json| {
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

        let requests = self.screens.show(ui, &self.view, &self.broker, now);
        for command in requests.commands {
            // The state follows the add from here, so that its name and room are given
            // once it is done, even if the sheet has been closed.
            if let UiCommand::Add(request) = &command {
                let network = request.wifi.as_ref().map(|(network, _)| network.as_str());
                self.change_state(|state| {
                    state.begin_add(&request.ble_id, &request.label, &request.room, network);
                });
            }
            self.hub.submit(command);
        }
        if let Some(submitted) = requests.settings {
            self.apply_settings(ui.ctx(), submitted);
        }
        if let Some(appearance) = requests.appearance {
            // Shown already. Saved at once, as Settings has no Save for it.
            self.appearance = appearance;
            self.save();
        }
        if !requests.edits.is_empty() {
            self.change_state(|state| {
                for edit in requests.edits {
                    state.edit(edit);
                }
            });
            // A name or a room is the person's own work: saved at once.
            self.save();
        }

        self.save_if_due();
        ui.ctx().request_repaint_after(REDRAW_EVERY);
    }

    /// What is behind the panes: the window color. eframe's own default is a
    /// translucent near-black, which would show through any gap between them.
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        theme::Palette::for_dark(visuals.dark_mode)
            .window
            .to_normalized_gamma_f32()
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.drain_events();
        let edits = self.screens.end_typing();
        self.change_state(|state| {
            for edit in edits {
                state.edit(edit);
            }
        });
        self.save();
    }
}
