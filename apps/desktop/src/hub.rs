//! The hub: where a command from the screens becomes a message to a diffuser.
//!
//! The screens hand it [`UiCommand`]s through [`Hub::submit`] and drain the [`Event`]s
//! it sends back. For each command the hub picks the way to the diffuser (see
//! `route`): Bluetooth first when the diffuser is nearby, WiFi when that fails. It
//! keeps a queue for each diffuser, so commands to one diffuser run one at a time in
//! the order they were made while different diffusers run side by side, and it pings
//! the diffusers that are on WiFi, which is how the screens know they answer.
//!
//! Adding a diffuser is one more command (see `add_flow` for its steps): its info is
//! read over Bluetooth first, and everything after that runs as one job in that
//! diffuser's queue, so no other command to it comes between its steps.
//!
//! The hub runs on its own thread and never touches the screens. It reads the app's
//! shared state to learn a diffuser's Bluetooth ID and when it was last heard, and
//! never holds that lock for longer than a copy: not across an `.await`, and not
//! while it talks to a worker. It changes nothing in the state itself: what it did
//! comes out as events, and the screens fold those in.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use scent_core::{Command, DeviceInfo, PING_EVERY_S, Profile, Secret, Via, valid_device_id};
use tokio::sync::mpsc::error::SendError;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::{Instant, sleep, sleep_until};

use crate::add_flow::{self, AddRequest, AddStep, NO_BROKER_PASSWORD as NO_PASSWORD_TO_GIVE};
use crate::ble::Reply;
use crate::mqtt::BrokerSettings;
use crate::route::{TransportPref, ble_known, route, why_no_route, wifi_available};
use crate::state::{AppState, DeviceView, Event, unix_now};
use crate::transport::{Radios, Transport, lock};

/// How long a Bluetooth scan listens.
const SCAN_SECS: u64 = 10;

/// The `what` of the `Failed` event for a scan that didn't run, which is how the
/// screens know the scan is over.
pub const SCAN_FAILED: &str = "Bluetooth scan";

/// How long after a schedule the diffuser is told the time. The other order did not
/// run on the Air 2 (see `docs/findings.md`).
const SYNC_TIME_AFTER: Duration = Duration::from_secs(5);

/// How often each diffuser on WiFi is pinged.
const PING_EVERY: Duration = Duration::from_secs(PING_EVERY_S.unsigned_abs());

/// How often to look again while the broker isn't connected, so that the first ping
/// goes out soon after it is.
const RECHECK_BROKER: Duration = Duration::from_secs(1);

/// Why the broker isn't connected when no password is saved. The screens show it as a
/// notice, with a way to Settings.
pub const NO_BROKER_PASSWORD: &str = "No broker password saved. Add it in Settings (if you already did, the Keychain couldn't be read).";

const BLUETOOTH_STOPPED: &str = "Bluetooth stopped before it answered.";

/// What a Bluetooth request that hit [`BLUETOOTH_BACKSTOP`] failed with. Whether it
/// went through is unknown, which an add has to tell from a refusal.
const BLUETOOTH_UNANSWERED: &str = "Bluetooth didn't answer in 30 s; it may still go through.";

/// How often the hub looks at the state while it waits for something to show up there.
const STATE_POLL: Duration = Duration::from_millis(250);

/// How long the info read over Bluetooth has to reach the state. The screens fold it in
/// at their next frame.
const INFO_WAIT: Duration = Duration::from_secs(5);

const CREDENTIALS_WAIT: Duration = Duration::from_secs(add_flow::CREDENTIALS_WAIT_S);

const NO_INFO: &str = "The diffuser answered, but its info didn't come through, so its device ID isn't known. Try again.";

const PING_REFUSED: &str = "The diffuser joined your broker, but the ping that finishes adding it couldn't be sent. Try again.";

/// How long the hub waits for Bluetooth to answer before it counts as failed. The
/// worker bounds every step itself (a failed connect takes about 13 s, plus a few
/// seconds of I/O), so this only keeps one stuck request from holding a diffuser's
/// queue for good if that ever stops being true.
const BLUETOOTH_BACKSTOP: Duration = Duration::from_secs(30);

const LOST_JOBS: &str = "An earlier command to this diffuser stopped unexpectedly, so it and any commands queued behind it were dropped. This one is not affected.";

const TIME_SYNC_FAILED: &str = "The schedule went out, but the time sync after it didn't, so the diffuser may not run it yet. Send the schedule again.";

/// What the screens ask the hub to do.
pub enum UiCommand {
    /// Looks for diffusers over Bluetooth. The Bluetooth worker sends the result as
    /// `Event::Scanned`.
    Scan,
    /// Sends one command to a diffuser, the way `pref` says.
    Send {
        device_id: String,
        cmd: Command,
        pref: TransportPref,
    },
    /// Asks a diffuser for its state, the way `pref` says: Bluetooth reads what it
    /// last reported, WiFi sends `GET_STATUS`.
    ReadStatus {
        device_id: String,
        pref: TransportPref,
    },
    /// Replaces the schedule table on a diffuser, then tells it the time. Over WiFi
    /// only: setting a schedule over Bluetooth is unconfirmed (see `docs/findings.md`).
    SendSchedule { device_id: String, profile: Profile },
    /// Puts a diffuser found over Bluetooth on our broker (see `add_flow`). Its progress
    /// comes back as `Event::Adding`.
    Add(AddRequest),
    /// Drops the broker connection and makes a new one, with these settings and the
    /// password just read from (or saved to) the Keychain. This is also how a refused
    /// login is tried again. With no password nothing is started.
    Reconnect {
        settings: BrokerSettings,
        password: Option<Secret>,
    },
}

/// How to give the hub work. Dropping it stops the hub, and with it both workers.
pub struct Hub {
    commands: UnboundedSender<UiCommand>,
    /// For saying that the hub itself isn't running.
    events: Sender<Event>,
    repaint: Arc<dyn Fn() + Send + Sync>,
}

impl Hub {
    /// Starts the hub on its own thread and returns at once, with the channel its
    /// events come out of. Every event is followed by a call to `repaint`.
    ///
    /// The hub starts both workers itself, so the MQTT one (which loads the system's
    /// root certificates when it starts) never runs on the caller's thread. With no
    /// `password` the broker is not connected and the first event says so. `state` is
    /// the app's shared state, which the hub only reads.
    pub fn start(
        settings: BrokerSettings,
        password: Option<Secret>,
        state: Arc<Mutex<AppState>>,
        repaint: impl Fn() + Clone + Send + Sync + 'static,
    ) -> (Hub, Receiver<Event>) {
        let (events, received) = std::sync::mpsc::channel();
        let (commands, queue) = unbounded_channel();
        let repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(repaint);
        let hub = Hub {
            commands,
            events: events.clone(),
            repaint: Arc::clone(&repaint),
        };
        let started = std::thread::Builder::new()
            .name("hub".to_owned())
            .spawn(move || run(settings, password, state, events, repaint, queue));
        if let Err(error) = started {
            hub.report(format!("Couldn't start the hub thread: {error}"));
        }
        (hub, received)
    }

    /// Hands the hub a command and returns at once, whatever the network is doing.
    pub fn submit(&self, command: UiCommand) {
        if self.commands.send(command).is_err() {
            self.report("The hub isn't running, so nothing was sent. Restart the app.".to_owned());
        }
    }

    fn report(&self, error: String) {
        send_event(&self.events, &*self.repaint, hub_failed(error));
    }
}

/// The event for something wrong with the hub itself.
fn hub_failed(error: String) -> Event {
    Event::Failed {
        device_id: None,
        what: "Hub".to_owned(),
        error,
        at: unix_now(),
    }
}

/// Hands the event to the app and wakes it, if the app is still listening.
fn send_event(events: &Sender<Event>, repaint: &(dyn Fn() + Send + Sync), event: Event) {
    if events.send(event).is_ok() {
        repaint();
    }
}

/// The hub thread: starts the workers, then serves until the `Hub` is dropped.
fn run(
    settings: BrokerSettings,
    password: Option<Secret>,
    state: Arc<Mutex<AppState>>,
    events: Sender<Event>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    commands: UnboundedReceiver<UiCommand>,
) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(error) => {
            let error = format!("Couldn't start the hub's runtime: {error}");
            send_event(&events, &*repaint, hub_failed(error));
            return;
        }
    };
    let radios = Radios::new(events.clone(), Arc::clone(&repaint));
    let shared = Arc::new(Shared::new(radios, state, events, repaint));
    shared.reconnect(settings, password);
    runtime.block_on(serve(shared, commands));
}

/// What one job asks of a diffuser.
struct Work {
    /// The command that goes out. For a status read this is `GET_STATUS`, which only
    /// WiFi sends.
    cmd: Command,
    /// Bluetooth reads a status from the diffuser instead of writing to it.
    reads_status: bool,
}

impl Work {
    fn send(cmd: Command) -> Work {
        Work {
            cmd,
            reads_status: false,
        }
    }

    fn read_status() -> Work {
        Work {
            cmd: Command::GetStatus,
            reads_status: true,
        }
    }

    /// What the log calls it: the action's name only, never the frame, which can
    /// hold a password.
    fn name(&self, via: Via) -> String {
        if self.reads_status {
            "read status".to_owned()
        } else {
            self.cmd.action(via).to_owned()
        }
    }
}

/// What the state says about one diffuser that decides how to reach it.
#[derive(Default)]
struct Target {
    ble_id: Option<String>,
    /// When it was last heard over Bluetooth.
    heard_ble: Option<i64>,
    /// Whether it has been heard on our broker (see [`on_our_broker`]).
    on_our_broker: bool,
}

/// The broker login the hub last started the MQTT worker with. An add hands it to the
/// diffuser in `SAVE_MQTT` (the test broker has one shared login). Not `Debug`.
#[derive(Default)]
struct Login {
    settings: BrokerSettings,
    password: Option<Secret>,
}

/// What a diffuser's add needs once its info is read: the rest runs as one job in its
/// queue. Not `Debug`: it can hold a WiFi password.
struct AddJob {
    ble_id: String,
    info: DeviceInfo,
    wifi: Option<(String, Secret)>,
}

/// How a write to a diffuser being added ended.
enum Written {
    Yes,
    /// No answer before the backstop: it may still have gone through.
    Unknown,
    /// It failed, with this error: not written, or not all of it.
    No(String),
}

/// What every task of the hub shares.
struct Shared<T> {
    transport: T,
    state: Arc<Mutex<AppState>>,
    events: Sender<Event>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    /// When Bluetooth last failed to reach each diffuser, by device ID. While WiFi can
    /// take over, Auto stays off Bluetooth for a while after that (see
    /// `route::ble_known`).
    ble_failed: Mutex<HashMap<String, i64>>,
    login: Mutex<Login>,
}

impl<T: Transport> Shared<T> {
    fn new(
        transport: T,
        state: Arc<Mutex<AppState>>,
        events: Sender<Event>,
        repaint: Arc<dyn Fn() + Send + Sync>,
    ) -> Shared<T> {
        Shared {
            transport,
            state,
            events,
            repaint,
            ble_failed: Mutex::new(HashMap::new()),
            login: Mutex::new(Login::default()),
        }
    }

    fn emit(&self, event: Event) {
        send_event(&self.events, &*self.repaint, event);
    }

    fn failed(&self, device_id: Option<&str>, what: String, error: String) {
        self.emit(Event::Failed {
            device_id: device_id.map(str::to_owned),
            what,
            error,
            at: unix_now(),
        });
    }

    /// What the state says about a diffuser that decides the route, copied out so that
    /// the lock is let go at once. A diffuser the state does not know has none of it.
    fn target(&self, device_id: &str) -> Target {
        lock(&self.state)
            .devices
            .get(device_id)
            .map_or_else(Target::default, |device| Target {
                ble_id: device.ble_id.clone(),
                heard_ble: device.heard_ble,
                on_our_broker: on_our_broker(device),
            })
    }

    /// Takes `work` to the diffuser the way `pref` says, trying the next transport
    /// when one fails. Emits `Sent` for the one that worked, and one `Failed` for
    /// whatever failed on the way. True when it got there.
    async fn deliver(&self, device_id: &str, work: &Work, pref: TransportPref) -> bool {
        let Target {
            ble_id,
            heard_ble,
            on_our_broker,
        } = self.target(device_id);
        let failed_at = lock(&self.ble_failed).get(device_id).copied();
        // The broker being up does not mean this diffuser is on it.
        let broker_up = self.transport.broker_route_id().is_some();
        let wifi = wifi_available(pref, broker_up, on_our_broker, ble_id.is_some());
        let bluetooth = ble_known(
            pref,
            ble_id.is_some(),
            failed_at,
            heard_ble,
            wifi,
            unix_now(),
        );
        let ways = route(pref, bluetooth, wifi);
        if ways.is_empty() {
            let what = work.name(pref.first_choice());
            self.failed(Some(device_id), what, why_no_route(pref));
            return false;
        }

        let mut failed: Vec<(Via, String)> = Vec::new();
        loop {
            let tried: Vec<Via> = failed.iter().map(|(via, _)| *via).collect();
            let Some(via) = next_attempt(&ways, &tried) else {
                break;
            };
            let result = match via {
                Via::Bluetooth => {
                    self.over_bluetooth(device_id, ble_id.as_deref(), work)
                        .await
                }
                Via::Wifi => self.transport.wifi_send(device_id, &work.cmd),
            };
            match result {
                Ok(()) => {
                    self.report_failure(device_id, work, &failed, Some(via));
                    self.sent(device_id, work, via);
                    return true;
                }
                Err(error) => failed.push((via, error)),
            }
        }
        self.report_failure(device_id, work, &failed, None);
        false
    }

    /// One Bluetooth attempt, which also keeps the cool-down: a failure starts it, a
    /// success ends it.
    async fn over_bluetooth(
        &self,
        device_id: &str,
        ble_id: Option<&str>,
        work: &Work,
    ) -> Result<(), String> {
        let ble_id = ble_id.ok_or_else(|| "No Bluetooth ID for this diffuser.".to_owned())?;
        let reply = if work.reads_status {
            self.transport.ble_read_status(ble_id)
        } else {
            self.transport.ble_send(ble_id, work.cmd.clone())
        };
        let result = answer(reply).await;
        let mut failed_at = lock(&self.ble_failed);
        match &result {
            Ok(()) => {
                failed_at.remove(device_id);
            }
            Err(_) => {
                failed_at.insert(device_id.to_owned(), unix_now());
            }
        }
        result
    }

    /// The command went out over `via`: say so, and what it asked for.
    fn sent(&self, device_id: &str, work: &Work, via: Via) {
        // Reading a status over Bluetooth writes nothing, so there is nothing to log.
        if via == Via::Bluetooth && work.reads_status {
            return;
        }
        let (power, intensity, profile) = effects(&work.cmd);
        self.emit(Event::Sent {
            device_id: device_id.to_owned(),
            summary: work.cmd.redacted(via),
            via,
            at: unix_now(),
            power,
            intensity,
            profile,
        });
    }

    fn report_failure(
        &self,
        device_id: &str,
        work: &Work,
        failed: &[(Via, String)],
        delivered: Option<Via>,
    ) {
        if let Some((what, error)) = failure_line(|via| work.name(via), failed, delivered) {
            self.failed(Some(device_id), what, error);
        }
    }

    /// Sends the schedule, waits, then tells the diffuser the time. Both over WiFi.
    /// Without the first there is no point in the second, so a failure stops here.
    async fn send_schedule(&self, device_id: &str, profile: Profile) {
        let steps = schedule_steps(profile, unix_now());
        for (index, (wait, cmd)) in steps.into_iter().enumerate() {
            if !wait.is_zero() {
                sleep(wait).await;
            }
            if !self
                .deliver(device_id, &Work::send(cmd), TransportPref::Wifi)
                .await
            {
                if index > 0 {
                    self.failed(
                        Some(device_id),
                        "Schedule".to_owned(),
                        TIME_SYNC_FAILED.to_owned(),
                    );
                }
                return;
            }
        }
    }

    async fn scan(&self) {
        if let Err(error) = answer(self.transport.ble_scan(SCAN_SECS)).await {
            self.failed(None, SCAN_FAILED.to_owned(), error);
        }
    }

    /// Pings every diffuser known to be on WiFi. False, having sent nothing, while the
    /// broker isn't connected.
    fn ping_round(&self) -> bool {
        let Some(route_id) = self.transport.broker_route_id() else {
            return false;
        };
        let targets = ping_targets(&lock(&self.state));
        for device_id in targets {
            let ping = Command::Ping {
                userid: Some(route_id.clone()),
            };
            match self.transport.wifi_send(&device_id, &ping) {
                Ok(()) => self.emit(Event::Pinged {
                    device_id,
                    at: unix_now(),
                }),
                Err(error) => self.failed(Some(&device_id), "WiFi PING".to_owned(), error),
            }
        }
        true
    }

    /// Says where the add of the diffuser at `ble_id` is.
    fn adding(&self, ble_id: &str, step: AddStep) {
        self.emit(Event::Adding {
            ble_id: ble_id.to_owned(),
            step,
        });
    }

    /// The broker login, copied out.
    fn login(&self) -> (BrokerSettings, Option<Secret>) {
        let login = lock(&self.login);
        (login.settings.clone(), login.password.clone())
    }

    /// Looks at the state every [`STATE_POLL`] until `found` finds what it looks for,
    /// for up to `limit`. The lock is held only while `found` looks.
    async fn wait_for<R>(
        &self,
        limit: Duration,
        found: impl Fn(&AppState) -> Option<R>,
    ) -> Option<R> {
        let until = Instant::now() + limit;
        loop {
            if let Some(found) = found(&lock(&self.state)) {
                return Some(found);
            }
            let now = Instant::now();
            if now >= until {
                return None;
            }
            sleep(STATE_POLL.min(until - now)).await;
        }
    }

    /// The first part of an add: there must be a broker password to hand the diffuser,
    /// then its info is read over Bluetooth, by its Bluetooth ID. Returns its device ID
    /// and the job that does the rest, for its queue; `None` when the add stops here,
    /// having said why.
    async fn prepare_add(&self, request: AddRequest) -> Option<(String, AddJob)> {
        let AddRequest {
            ble_id,
            label: _,
            wifi,
        } = request;
        if lock(&self.login).password.is_none() {
            self.adding(&ble_id, AddStep::Failed(NO_PASSWORD_TO_GIVE.to_owned()));
            return None;
        }
        self.adding(&ble_id, AddStep::ReadInfo);
        let started = unix_now();
        if let Err(error) = answer(self.transport.ble_read_info(&ble_id)).await {
            self.adding(&ble_id, AddStep::Failed(error));
            return None;
        }
        let Some(info) = self
            .wait_for(INFO_WAIT, |state| fresh_info(state, &ble_id, started))
            .await
        else {
            self.adding(&ble_id, AddStep::Failed(NO_INFO.to_owned()));
            return None;
        };
        match add_flow::next(&AddStep::ReadInfo, Some(&info), wifi.is_some(), None) {
            AddStep::SaveMqtt => {
                let device_id = info.device_id.clone();
                Some((device_id, AddJob { ble_id, info, wifi }))
            }
            stop => {
                self.adding(&ble_id, stop);
                None
            }
        }
    }

    /// The rest of an add, as one job in the diffuser's queue: the writes over
    /// Bluetooth only, then the wait for it on our broker, and the ping over WiFi. A
    /// write whose result is unknown is never sent again: the flow goes on, and the
    /// wait for `device_connected` tells whether it worked.
    async fn run_add(&self, device_id: &str, job: AddJob) {
        let AddJob { ble_id, info, wifi } = job;
        let wifi_given = wifi.is_some();
        // When the restart was asked for: the diffuser must join the broker after it.
        let mut restarted_at = unix_now();
        let mut step = AddStep::SaveMqtt;
        loop {
            self.adding(&ble_id, step.clone());
            let mut connected = None;
            match &step {
                AddStep::SaveMqtt => {
                    let (settings, password) = self.login();
                    let Some(password) = password else {
                        self.adding(&ble_id, AddStep::Failed(NO_PASSWORD_TO_GIVE.to_owned()));
                        return;
                    };
                    let since = unix_now();
                    let save = Command::SaveMqtt {
                        broker: settings.url,
                        username: settings.username,
                        password,
                    };
                    if let Written::No(error) = self.add_write(device_id, &ble_id, save).await {
                        self.adding(&ble_id, AddStep::Failed(error));
                        return;
                    }
                    // Restarted either way once the wait is over.
                    let _ = self
                        .wait_for(CREDENTIALS_WAIT, |state| {
                            credentials_saved(state, device_id, since)
                        })
                        .await;
                }
                AddStep::WifiConnect => {
                    let Some((ssid, password)) = wifi.clone() else {
                        return;
                    };
                    let join = Command::WifiConnect { ssid, password };
                    if let Written::No(error) = self.add_write(device_id, &ble_id, join).await {
                        self.adding(&ble_id, AddStep::Failed(error));
                        return;
                    }
                }
                AddStep::Restart => {
                    restarted_at = unix_now();
                    // The reboot can cut the write short; whether it went through is
                    // what the wait finds out.
                    let _ = self
                        .add_write(device_id, &ble_id, Command::SoftRestart)
                        .await;
                }
                AddStep::WaitConnected => {
                    match self.wait_connected(device_id, restarted_at).await {
                        AddStep::Ping => connected = Some(device_id),
                        failed => {
                            self.adding(&ble_id, failed);
                            return;
                        }
                    }
                }
                AddStep::Ping => {
                    let userid = Some(self.login().0.route_id);
                    let ping = Work::send(Command::Ping { userid });
                    if !self.deliver(device_id, &ping, TransportPref::Wifi).await {
                        self.adding(&ble_id, AddStep::Failed(PING_REFUSED.to_owned()));
                        return;
                    }
                    self.emit(Event::Pinged {
                        device_id: device_id.to_owned(),
                        at: unix_now(),
                    });
                }
                AddStep::ReadInfo | AddStep::NeedWifi | AddStep::Done(_) | AddStep::Failed(_) => {
                    return;
                }
            }
            step = add_flow::next(&step, Some(&info), wifi_given, connected);
        }
    }

    /// Writes `cmd` to the diffuser being added, over Bluetooth only, and logs it as any
    /// command is.
    async fn add_write(&self, device_id: &str, ble_id: &str, cmd: Command) -> Written {
        let work = Work::send(cmd);
        match self.over_bluetooth(device_id, Some(ble_id), &work).await {
            Ok(()) => {
                self.sent(device_id, &work, Via::Bluetooth);
                Written::Yes
            }
            Err(error) => {
                let failed = [(Via::Bluetooth, error)];
                self.report_failure(device_id, &work, &failed, None);
                let [(_, error)] = failed;
                if error == BLUETOOTH_UNANSWERED {
                    Written::Unknown
                } else {
                    Written::No(error)
                }
            }
        }
    }

    /// Waits for the diffuser to say it joined the broker after `since`, for up to
    /// [`add_flow::WAIT_CONNECTED_S`]: `Ping` once it has, `Failed` when it hasn't.
    async fn wait_connected(&self, device_id: &str, since: i64) -> AddStep {
        let started = Instant::now();
        loop {
            let connected = lock(&self.state)
                .devices
                .get(device_id)
                .and_then(|device| device.connected_at)
                .is_some_and(|at| at > since);
            let waited = i64::try_from(started.elapsed().as_secs()).unwrap_or(i64::MAX);
            match add_flow::wait_outcome(0, waited, connected) {
                AddStep::WaitConnected => sleep(STATE_POLL).await,
                outcome => return outcome,
            }
        }
    }

    fn reconnect(&self, settings: BrokerSettings, password: Option<Secret>) {
        let has_password = password.is_some();
        *lock(&self.login) = Login {
            settings: settings.clone(),
            password: password.clone(),
        };
        self.transport.reconnect(settings, password);
        if !has_password {
            self.emit(Event::BrokerDisconnected(NO_BROKER_PASSWORD.to_owned()));
        }
    }
}

/// How a Bluetooth request ended. A reply that never comes means the worker is gone,
/// and one that takes longer than [`BLUETOOTH_BACKSTOP`] counts as a failure.
async fn answer(reply: Reply) -> Result<(), String> {
    match tokio::time::timeout(BLUETOOTH_BACKSTOP, reply).await {
        Ok(answered) => answered.unwrap_or_else(|_| Err(BLUETOOTH_STOPPED.to_owned())),
        Err(_) => Err(BLUETOOTH_UNANSWERED.to_owned()),
    }
}

/// The transport to try next: the first one on the route that has not failed yet.
fn next_attempt(route: &[Via], failed: &[Via]) -> Option<Via> {
    route.iter().copied().find(|via| !failed.contains(via))
}

fn label(via: Via) -> &'static str {
    match via {
        Via::Bluetooth => "Bluetooth",
        Via::Wifi => "WiFi",
    }
}

/// The one `Failed` line (`what`, `error`) for a job, or `None` when nothing failed.
/// `failed` is what went wrong, in the order it was tried; `delivered` is the
/// transport that worked afterwards, if any. Names the action only, never the frame.
fn failure_line(
    name: impl Fn(Via) -> String,
    failed: &[(Via, String)],
    delivered: Option<Via>,
) -> Option<(String, String)> {
    let (first, first_error) = failed.first()?;
    let line = match (delivered, failed.len()) {
        (Some(went_by), _) => (
            format!("{} {}", label(*first), name(*first)),
            format!(
                "{} couldn't reach the diffuser, so this went over {}. {first_error}",
                label(*first),
                label(went_by)
            ),
        ),
        (None, 1) => (
            format!("{} {}", label(*first), name(*first)),
            first_error.clone(),
        ),
        (None, _) => {
            let told: Vec<String> = failed
                .iter()
                .map(|(via, error)| format!("{}: {error}", label(*via)))
                .collect();
            (
                name(*first),
                format!("Neither route worked. {}", told.join(" ")),
            )
        }
    };
    Some(line)
}

/// What `cmd` asks the diffuser to be, for the state to remember until the diffuser
/// reports: the power, the intensity and the schedule table.
fn effects(cmd: &Command) -> (Option<bool>, Option<u8>, Option<Profile>) {
    match cmd {
        Command::TurnOn { on } => (Some(*on), None, None),
        Command::SetLevel(level) => (None, Some(level.get()), None),
        Command::UpdateProfile(profile) => (None, None, Some(profile.clone())),
        Command::Ping { .. }
        | Command::GetStatus
        | Command::SyncTime { .. }
        | Command::SaveMqtt { .. }
        | Command::WifiConnect { .. }
        | Command::SoftRestart => (None, None, None),
    }
}

/// The two commands that set a schedule, each with how long to wait after the one
/// before it: the table, then the time. `now` is when the schedule is sent. The clock
/// is set to the moment the time step goes out, not the moment it was planned.
fn schedule_steps(profile: Profile, now: i64) -> [(Duration, Command); 2] {
    let server_time = u64::try_from(now).unwrap_or(0) + SYNC_TIME_AFTER.as_secs();
    [
        (Duration::ZERO, Command::UpdateProfile(profile)),
        (SYNC_TIME_AFTER, Command::SyncTime { server_time }),
    ]
}

/// The info of the diffuser at `ble_id`, if it was read at `since` or later.
fn fresh_info(state: &AppState, ble_id: &str, since: i64) -> Option<DeviceInfo> {
    state
        .devices
        .values()
        .find(|device| {
            device.ble_id.as_deref() == Some(ble_id) && device.info_at.is_some_and(|at| at >= since)
        })
        .and_then(|device| device.info.clone())
}

/// Whether the diffuser said "Credentials saved" at `since` or later.
fn credentials_saved(state: &AppState, device_id: &str, since: i64) -> Option<()> {
    state
        .devices
        .get(device_id)
        .and_then(|device| device.credentials_saved_at)
        .is_some_and(|at| at >= since)
        .then_some(())
}

/// The diffusers to ping: those worth pinging (see [`worth_pinging`]).
fn ping_targets(state: &AppState) -> Vec<String> {
    state
        .devices
        .values()
        .filter(|device| valid_device_id(&device.id) && worth_pinging(device))
        .map(|device| device.id.clone())
        .collect()
}

/// Whether to ping a diffuser: it is on our broker, or its info says WiFi is set up.
/// `heard_wifi` alone is not enough, as it is not saved, so after a restart nothing
/// would be pinged until a diffuser spoke of its own accord, which can take its whole
/// 1200 s keepalive. What the saved state does hold is the info string
/// (`wifi_configured`) and the readings with their source. A ping to a diffuser that
/// is on some other broker (it may still be on Scentiment's cloud) is harmless, and it
/// is how a diffuser that has just been moved to ours first gets heard.
fn worth_pinging(device: &DeviceView) -> bool {
    device
        .info
        .as_ref()
        .is_some_and(|info| info.wifi_configured)
        || on_our_broker(device)
}

/// Whether a diffuser has been heard on our broker: a message from it, or a reading
/// that came over WiFi (the saved state keeps the readings' source, not when it was
/// last heard). This, not `wifi_configured`, decides whether WiFi is a way to it,
/// since a diffuser with WiFi set up may still be on Scentiment's cloud, where a
/// publish to our broker would be heard by nobody. The screens use it to say so.
pub fn on_our_broker(device: &DeviceView) -> bool {
    device.heard_wifi.is_some()
        || device
            .readings
            .values()
            .any(|reading| reading.via == Via::Wifi)
}

/// A job for one diffuser's queue.
enum Job {
    Send {
        cmd: Command,
        pref: TransportPref,
    },
    ReadStatus {
        pref: TransportPref,
    },
    Schedule(Profile),
    /// The steps of an add after its info was read.
    Add(AddJob),
}

/// Hands each command to the right task. Commands for one diffuser go through that
/// diffuser's queue, which one task works through a job at a time, so a later command
/// never overtakes an earlier one, however long the earlier one takes.
struct Dispatcher<T> {
    shared: Arc<Shared<T>>,
    queues: HashMap<String, UnboundedSender<Job>>,
    /// Where an add hands back the job for its diffuser's queue once its info is read,
    /// for [`serve`] to queue it.
    requeue: UnboundedSender<(String, Job)>,
}

impl<T: Transport> Dispatcher<T> {
    fn new(shared: Arc<Shared<T>>, requeue: UnboundedSender<(String, Job)>) -> Dispatcher<T> {
        Dispatcher {
            shared,
            queues: HashMap::new(),
            requeue,
        }
    }

    fn dispatch(&mut self, command: UiCommand) {
        match command {
            UiCommand::Scan => {
                let shared = Arc::clone(&self.shared);
                tokio::spawn(async move { shared.scan().await });
            }
            UiCommand::Reconnect { settings, password } => {
                self.shared.reconnect(settings, password);
            }
            UiCommand::Send {
                device_id,
                cmd,
                pref,
            } => self.enqueue(device_id, Job::Send { cmd, pref }),
            UiCommand::ReadStatus { device_id, pref } => {
                self.enqueue(device_id, Job::ReadStatus { pref });
            }
            UiCommand::SendSchedule { device_id, profile } => {
                self.enqueue(device_id, Job::Schedule(profile));
            }
            UiCommand::Add(request) => {
                // The device ID, and so the queue, is known once the info is read.
                let shared = Arc::clone(&self.shared);
                let requeue = self.requeue.clone();
                tokio::spawn(async move {
                    if let Some((device_id, job)) = shared.prepare_add(request).await {
                        let _ = requeue.send((device_id, Job::Add(job)));
                    }
                });
            }
        }
    }

    fn enqueue(&mut self, device_id: String, job: Job) {
        if !valid_device_id(&device_id) {
            let error = format!("{device_id:?} is not a valid device ID.");
            self.shared.failed(None, "Command".to_owned(), error);
            return;
        }
        let job = match self.queue(&device_id).send(job) {
            Ok(()) => return,
            Err(SendError(job)) => job,
        };
        // The queue's task is gone, which only happens if it panicked. It took with it
        // the job it was running and every job queued behind that one, so say so, and
        // start a new queue for this job: a fresh task has not run yet, so it takes it.
        self.shared
            .failed(Some(&device_id), "Hub".to_owned(), LOST_JOBS.to_owned());
        self.queues.remove(&device_id);
        let _ = self.queue(&device_id).send(job);
    }

    /// The diffuser's queue, started on first use.
    fn queue(&mut self, device_id: &str) -> &UnboundedSender<Job> {
        self.queues.entry(device_id.to_owned()).or_insert_with(|| {
            let (queue, jobs) = unbounded_channel();
            tokio::spawn(work_through(
                Arc::clone(&self.shared),
                device_id.to_owned(),
                jobs,
            ));
            queue
        })
    }
}

/// One diffuser's queue: the jobs in the order they came, one at a time.
async fn work_through<T: Transport>(
    shared: Arc<Shared<T>>,
    device_id: String,
    mut jobs: UnboundedReceiver<Job>,
) {
    while let Some(job) = jobs.recv().await {
        match job {
            Job::Send { cmd, pref } => {
                shared.deliver(&device_id, &Work::send(cmd), pref).await;
            }
            Job::ReadStatus { pref } => {
                shared.deliver(&device_id, &Work::read_status(), pref).await;
            }
            Job::Schedule(profile) => shared.send_schedule(&device_id, profile).await,
            Job::Add(job) => shared.run_add(&device_id, job).await,
        }
    }
}

/// Takes commands and runs the ping loop until the `Hub` is dropped.
async fn serve<T: Transport>(shared: Arc<Shared<T>>, mut commands: UnboundedReceiver<UiCommand>) {
    let (requeue, mut requeued) = unbounded_channel();
    let mut dispatcher = Dispatcher::new(Arc::clone(&shared), requeue);
    let mut next_ping = Instant::now();
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                if matches!(command, UiCommand::Reconnect { .. }) {
                    // A new connection: look for it at once, and ping when it is up.
                    next_ping = Instant::now();
                }
                dispatcher.dispatch(command);
            }
            Some((device_id, job)) = requeued.recv() => dispatcher.enqueue(device_id, job),
            () = sleep_until(next_ping) => {
                let wait = if shared.ping_round() { PING_EVERY } else { RECHECK_BROKER };
                next_ping = Instant::now() + wait;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{Receiver, channel};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use scent_core::{AlwaysOn, Command, DeviceInfo, Level, Profile, ScheduleFlag, Secret, Via};
    use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
    use tokio::sync::oneshot;
    use tokio::time::Instant;

    use super::*;
    use crate::add_flow::{AddRequest, AddStep};
    use crate::ble::Reply;
    use crate::mqtt::BrokerSettings;
    use crate::route::TransportPref;
    use crate::state::{AppState, DeviceView, Event, Field, Reading};

    const ONE: &str = "TESTDEVICE01";
    const TWO: &str = "TESTDEVICE02";

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn profile() -> Profile {
        Profile {
            always_on: AlwaysOn {
                enable: false,
                level: Level::new(1).unwrap(),
            },
            entries: vec![],
            timezone_hours: -5.0,
            flag: ScheduleFlag::Replace,
        }
    }

    fn info(id: &str, wifi_configured: bool) -> DeviceInfo {
        DeviceInfo {
            manufacturer_id: "TEST".into(),
            model_name: "Diffuser Air 2".into(),
            variant_name: "TEST".into(),
            device_id: id.into(),
            firmware: "2.30".into(),
            wifi_configured,
            wifi_status: 0,
        }
    }

    // --- pure helpers ---

    #[test]
    fn auto_falls_back_to_wifi_after_ble_failure() {
        let route = [Via::Bluetooth, Via::Wifi];
        assert_eq!(next_attempt(&route, &[]), Some(Via::Bluetooth));
        assert_eq!(next_attempt(&route, &[Via::Bluetooth]), Some(Via::Wifi));
        assert_eq!(next_attempt(&route, &[Via::Bluetooth, Via::Wifi]), None);
        // A forced transport has no fallback, and an empty route has nothing to try.
        assert_eq!(next_attempt(&[Via::Wifi], &[Via::Wifi]), None);
        assert_eq!(next_attempt(&[], &[]), None);
    }

    #[test]
    fn schedule_sends_profile_then_sync_time_5_s_later() {
        let now = 1_790_000_000;
        let steps = schedule_steps(profile(), now);
        assert_eq!(
            steps[0],
            (Duration::ZERO, Command::UpdateProfile(profile()))
        );
        // The clock is set to the moment the step goes out, which is 5 s from now.
        assert_eq!(
            steps[1],
            (
                secs(5),
                Command::SyncTime {
                    server_time: 1_790_000_005
                }
            )
        );
    }

    #[test]
    fn a_command_says_what_it_asked_for() {
        assert_eq!(
            effects(&Command::TurnOn { on: true }),
            (Some(true), None, None)
        );
        assert_eq!(
            effects(&Command::TurnOn { on: false }),
            (Some(false), None, None)
        );
        let level = Command::SetLevel(Level::new(2).unwrap());
        assert_eq!(effects(&level), (None, Some(2), None));
        let update = Command::UpdateProfile(profile());
        assert_eq!(effects(&update), (None, None, Some(profile())));
        for cmd in [
            Command::GetStatus,
            Command::SoftRestart,
            Command::Ping { userid: None },
        ] {
            assert_eq!(effects(&cmd), (None, None, None));
        }
    }

    #[test]
    fn pings_devices_known_to_be_on_wifi() {
        let mut state = AppState::default();
        // WiFi is set up on it (from the info string), but it has said nothing yet:
        // after a restart this is all that is known.
        let mut configured = DeviceView::new(ONE);
        configured.info = Some(info(ONE, true));
        // Heard over WiFi.
        let mut heard = DeviceView::new(TWO);
        heard.heard_wifi = Some(100);
        // A reading that came over WiFi, though `heard_wifi` was not kept.
        let mut read = DeviceView::new("TESTDEVICE03");
        read.readings.insert(
            Field::Power,
            Reading {
                value: 1,
                at: 50,
                via: Via::Wifi,
            },
        );
        // Bluetooth only: no WiFi set up, and everything it said came over Bluetooth.
        let mut bluetooth = DeviceView::new("TESTDEVICE04");
        bluetooth.ble_id = Some("ble-4".into());
        bluetooth.info = Some(info("TESTDEVICE04", false));
        bluetooth.heard_ble = Some(100);
        bluetooth.readings.insert(
            Field::Power,
            Reading {
                value: 1,
                at: 100,
                via: Via::Bluetooth,
            },
        );
        // Known by name only.
        let unknown = DeviceView::new("TESTDEVICE05");
        for view in [configured, heard, read, bluetooth, unknown] {
            state.devices.insert(view.id.clone(), view);
        }
        assert_eq!(
            ping_targets(&state),
            vec![ONE.to_owned(), TWO.to_owned(), "TESTDEVICE03".to_owned()]
        );
    }

    #[test]
    fn only_a_diffuser_heard_on_our_broker_is_reached_over_it() {
        // WiFi set up, and nothing heard here: it may still be on Scentiment's cloud,
        // where a publish to our broker would reach nobody. It is still pinged, which
        // is harmless and lets a diffuser that has just been moved get heard.
        let mut configured = DeviceView::new(ONE);
        configured.info = Some(info(ONE, true));
        assert!(!on_our_broker(&configured));
        assert!(worth_pinging(&configured));

        let mut heard = DeviceView::new(ONE);
        heard.heard_wifi = Some(100);
        assert!(on_our_broker(&heard));
        assert!(worth_pinging(&heard));

        // `heard_wifi` is not saved, but a reading that came over WiFi is.
        let mut read = DeviceView::new(ONE);
        read.readings.insert(
            Field::Battery,
            Reading {
                value: 80,
                at: 50,
                via: Via::Wifi,
            },
        );
        assert!(on_our_broker(&read));
        assert!(worth_pinging(&read));

        // Bluetooth only.
        let mut bluetooth = DeviceView::new(ONE);
        bluetooth.heard_ble = Some(100);
        bluetooth.readings.insert(
            Field::Battery,
            Reading {
                value: 80,
                at: 100,
                via: Via::Bluetooth,
            },
        );
        assert!(!on_our_broker(&bluetooth));
        assert!(!worth_pinging(&bluetooth));
    }

    #[test]
    fn one_failed_line_per_job() {
        let name = |_: Via| "TURN_ON".to_owned();
        let bluetooth = (Via::Bluetooth, "Timed out.".to_owned());
        let wifi = (Via::Wifi, "Not connected to the broker.".to_owned());

        // Nothing failed: nothing to say.
        assert_eq!(failure_line(name, &[], Some(Via::Bluetooth)), None);

        // Bluetooth failed and WiFi then worked.
        assert_eq!(
            failure_line(name, std::slice::from_ref(&bluetooth), Some(Via::Wifi)),
            Some((
                "Bluetooth TURN_ON".to_owned(),
                "Bluetooth couldn't reach the diffuser, so this went over WiFi. Timed out."
                    .to_owned()
            ))
        );
        // The one route there was failed.
        assert_eq!(
            failure_line(name, std::slice::from_ref(&bluetooth), None),
            Some(("Bluetooth TURN_ON".to_owned(), "Timed out.".to_owned()))
        );
        assert_eq!(
            failure_line(name, std::slice::from_ref(&wifi), None),
            Some((
                "WiFi TURN_ON".to_owned(),
                "Not connected to the broker.".to_owned()
            ))
        );
        // Both failed: one line that tells both.
        assert_eq!(
            failure_line(name, &[bluetooth, wifi], None),
            Some((
                "TURN_ON".to_owned(),
                "Neither route worked. Bluetooth: Timed out. WiFi: Not connected to the broker."
                    .to_owned()
            ))
        );
    }

    #[test]
    fn what_a_job_is_called_in_the_log() {
        let turn_on = Work::send(Command::TurnOn { on: true });
        assert_eq!(turn_on.name(Via::Bluetooth), "TURN_ON");
        let restart = Work::send(Command::SoftRestart);
        assert_eq!(restart.name(Via::Bluetooth), "SECRET_COMMAND");
        assert_eq!(restart.name(Via::Wifi), "RESET");
        let status = Work::read_status();
        assert_eq!(status.name(Via::Bluetooth), "read status");
        assert_eq!(status.name(Via::Wifi), "read status");
    }

    // --- the hub, with a fake in place of Bluetooth and the broker ---

    /// How a fake Bluetooth request ends.
    enum Ble {
        After(Duration, Result<(), String>),
        /// The worker went away without answering.
        Dies,
        /// The worker never answers, though it is still there.
        Never,
        /// Whatever handles the request panics.
        Panics,
    }

    /// A transport that touches neither Bluetooth nor the network. It notes what it is
    /// asked, with the (virtual) time, and answers as scripted.
    struct Fake {
        began: Instant,
        calls: Arc<Mutex<Vec<(Duration, String)>>>,
        ble: Mutex<VecDeque<Ble>>,
        wifi: Mutex<VecDeque<Result<(), String>>>,
        wifi_commands: Mutex<Vec<(String, Command)>>,
        /// What went out over Bluetooth, by Bluetooth ID, secrets and all.
        ble_commands: Mutex<Vec<(String, Command)>>,
        /// The info each diffuser gives when it is read, by Bluetooth ID.
        infos: Mutex<HashMap<String, DeviceInfo>>,
        /// The app's state, where a read info lands as the screens would fold it in.
        state: Mutex<Option<Arc<Mutex<AppState>>>>,
        route_id: Mutex<Option<String>>,
        reconnects: Mutex<Vec<(String, bool)>>,
        /// Reply senders kept alive so that their replies stay unanswered.
        held: Mutex<Vec<oneshot::Sender<Result<(), String>>>>,
    }

    impl Fake {
        fn new() -> Fake {
            Fake {
                began: Instant::now(),
                calls: Arc::new(Mutex::new(Vec::new())),
                ble: Mutex::new(VecDeque::new()),
                wifi: Mutex::new(VecDeque::new()),
                wifi_commands: Mutex::new(Vec::new()),
                ble_commands: Mutex::new(Vec::new()),
                infos: Mutex::new(HashMap::new()),
                state: Mutex::new(None),
                route_id: Mutex::new(None),
                reconnects: Mutex::new(Vec::new()),
                held: Mutex::new(Vec::new()),
            }
        }

        fn note(&self, label: String) {
            lock(&self.calls).push((self.began.elapsed(), label));
        }

        fn log(&self) -> Vec<(Duration, String)> {
            lock(&self.calls).clone()
        }

        fn labels(&self) -> Vec<String> {
            self.log().into_iter().map(|(_, label)| label).collect()
        }

        fn clear(&self) {
            lock(&self.calls).clear();
        }

        fn script_ble(&self, steps: impl IntoIterator<Item = Ble>) {
            lock(&self.ble).extend(steps);
        }

        fn script_wifi(&self, steps: impl IntoIterator<Item = Result<(), String>>) {
            lock(&self.wifi).extend(steps);
        }

        /// Starts a Bluetooth request: noted now, answered as scripted.
        fn begin(&self, label: String) -> Reply {
            self.note(label.clone());
            let step = lock(&self.ble).pop_front();
            let (answer, reply) = oneshot::channel();
            let (delay, result) = match step {
                None => (Duration::ZERO, Ok(())),
                Some(Ble::After(delay, result)) => (delay, result),
                Some(Ble::Dies) => return reply,
                Some(Ble::Never) => {
                    lock(&self.held).push(answer);
                    return reply;
                }
                // Unwinds without the panic hook, so nothing is printed.
                Some(Ble::Panics) => std::panic::resume_unwind(Box::new("panicked on purpose")),
            };
            let (calls, began) = (Arc::clone(&self.calls), self.began);
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                lock(&calls).push((began.elapsed(), format!("{label} done")));
                let _ = answer.send(result);
            });
            reply
        }
    }

    impl Transport for Fake {
        fn ble_scan(&self, secs: u64) -> Reply {
            self.begin(format!("scan {secs}"))
        }

        fn ble_send(&self, ble_id: &str, cmd: Command) -> Reply {
            let label = format!("ble {ble_id} {}", cmd.action(Via::Bluetooth));
            lock(&self.ble_commands).push((ble_id.to_owned(), cmd));
            self.begin(label)
        }

        /// As the Bluetooth worker does, the info reaches the state (here at once) when
        /// the read works; the reply only says that it did.
        fn ble_read_info(&self, ble_id: &str) -> Reply {
            let works = matches!(lock(&self.ble).front(), None | Some(Ble::After(_, Ok(()))));
            let info = lock(&self.infos).get(ble_id).cloned();
            let state = lock(&self.state).clone();
            if works && let (Some(info), Some(state)) = (info, state) {
                lock(&state).apply(Event::BleInfo {
                    ble_id: ble_id.to_owned(),
                    info,
                    at: unix_now(),
                });
            }
            self.begin(format!("ble {ble_id} read info"))
        }

        fn ble_read_status(&self, ble_id: &str) -> Reply {
            self.begin(format!("ble {ble_id} read status"))
        }

        fn wifi_send(&self, device_id: &str, cmd: &Command) -> Result<(), String> {
            self.note(format!("wifi {device_id} {}", cmd.action(Via::Wifi)));
            lock(&self.wifi_commands).push((device_id.to_owned(), cmd.clone()));
            lock(&self.wifi).pop_front().unwrap_or(Ok(()))
        }

        fn broker_route_id(&self) -> Option<String> {
            lock(&self.route_id).clone()
        }

        fn reconnect(&self, settings: BrokerSettings, password: Option<Secret>) {
            lock(&self.reconnects).push((settings.url, password.is_some()));
        }
    }

    struct Rig {
        shared: Arc<Shared<Fake>>,
        state: Arc<Mutex<AppState>>,
        received: Receiver<Event>,
        repaints: Arc<AtomicUsize>,
    }

    impl Rig {
        fn new() -> Rig {
            let state = Arc::new(Mutex::new(AppState::default()));
            let (events, received) = channel();
            let repaints = Arc::new(AtomicUsize::new(0));
            let counted = Arc::clone(&repaints);
            let repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            });
            let shared = Arc::new(Shared::new(
                Fake::new(),
                Arc::clone(&state),
                events,
                repaint,
            ));
            *lock(&shared.transport.state) = Some(Arc::clone(&state));
            Rig {
                shared,
                state,
                received,
                repaints,
            }
        }

        fn device(self, id: &str, ble_id: Option<&str>) -> Rig {
            let mut view = DeviceView::new(id);
            view.ble_id = ble_id.map(str::to_owned);
            lock(&self.state).devices.insert(id.to_owned(), view);
            self
        }

        /// Says the diffuser has been heard on our broker, which is what makes it
        /// reachable over WiFi.
        fn heard_on_broker(self, id: &str) -> Rig {
            lock(&self.state).devices.get_mut(id).unwrap().heard_wifi = Some(100);
            self
        }

        fn fake(&self) -> &Fake {
            &self.shared.transport
        }

        /// What reading the info of the diffuser at `ble_id` gives.
        fn info_for(self, ble_id: &str, info: DeviceInfo) -> Rig {
            lock(&self.fake().infos).insert(ble_id.to_owned(), info);
            self
        }

        /// The hub logged in to the broker with this password, as it is at start-up.
        fn logged_in(self, password: &str) -> Rig {
            let password = Secret::new(password.to_owned());
            self.shared
                .reconnect(BrokerSettings::default(), Some(password));
            self
        }

        /// Folds `event` into the state `after` this long, as the screens would.
        fn apply_later(&self, after: Duration, event: Event) {
            let state = Arc::clone(&self.state);
            tokio::spawn(async move {
                tokio::time::sleep(after).await;
                lock(&state).apply(event);
            });
        }

        fn broker_up(&self) {
            *lock(&self.fake().route_id) = Some("route-test-1".to_owned());
        }

        fn events(&self) -> Vec<Event> {
            self.received.try_iter().collect()
        }

        /// Runs the hub's loop, as `Hub::start` does, and returns what the screens send
        /// commands through.
        fn serve(&self) -> UnboundedSender<UiCommand> {
            let (commands, queue) = unbounded_channel();
            tokio::spawn(serve(Arc::clone(&self.shared), queue));
            commands
        }
    }

    /// Lets everything that is waiting finish, in virtual time.
    async fn settle() {
        tokio::time::sleep(secs(60)).await;
    }

    fn turn_on(device_id: &str, on: bool, pref: TransportPref) -> UiCommand {
        UiCommand::Send {
            device_id: device_id.to_owned(),
            cmd: Command::TurnOn { on },
            pref,
        }
    }

    /// The `Sent` events' (via, action text, power).
    fn sent(events: &[Event]) -> Vec<(Via, String, Option<bool>)> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Sent {
                    via,
                    summary,
                    power,
                    ..
                } => Some((*via, summary.clone(), *power)),
                _ => None,
            })
            .collect()
    }

    fn failed(events: &[Event]) -> Vec<(Option<String>, String, String)> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Failed {
                    device_id,
                    what,
                    error,
                    ..
                } => Some((device_id.clone(), what.clone(), error.clone())),
                _ => None,
            })
            .collect()
    }

    const TURN_ON_FRAME: &str = r#"{"action":"TURN_ON","payload":{"on":1}}"#;

    #[tokio::test(start_paused = true)]
    async fn commands_to_one_diffuser_run_one_at_a_time_in_order() {
        let rig = Rig::new().device(ONE, Some("ble-1"));
        // The first write takes 3 s. The second would be instant, but must not start
        // before the first has finished: the person's latest change has to win.
        rig.fake().script_ble([Ble::After(secs(3), Ok(()))]);
        let hub = rig.serve();
        hub.send(turn_on(ONE, true, TransportPref::Auto)).unwrap();
        hub.send(turn_on(ONE, false, TransportPref::Auto)).unwrap();
        settle().await;

        let log = rig.fake().log();
        let labels: Vec<&str> = log.iter().map(|(_, label)| label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "ble ble-1 TURN_ON",
                "ble ble-1 TURN_ON done",
                "ble ble-1 TURN_ON",
                "ble ble-1 TURN_ON done",
            ]
        );
        assert_eq!(log[2].0, secs(3));
        let powers: Vec<_> = sent(&rig.events()).into_iter().map(|s| s.2).collect();
        assert_eq!(powers, [Some(true), Some(false)]);
    }

    #[tokio::test(start_paused = true)]
    async fn different_diffusers_do_not_wait_for_each_other() {
        let rig = Rig::new()
            .device(ONE, Some("ble-1"))
            .device(TWO, Some("ble-2"));
        rig.fake()
            .script_ble([Ble::After(secs(3), Ok(())), Ble::After(secs(3), Ok(()))]);
        let hub = rig.serve();
        hub.send(turn_on(ONE, true, TransportPref::Auto)).unwrap();
        hub.send(turn_on(TWO, true, TransportPref::Auto)).unwrap();
        settle().await;

        let log = rig.fake().log();
        assert_eq!(log[0], (Duration::ZERO, "ble ble-1 TURN_ON".to_owned()));
        assert_eq!(log[1], (Duration::ZERO, "ble ble-2 TURN_ON".to_owned()));
        assert_eq!(log.len(), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn a_schedule_is_followed_by_its_time_sync_before_any_later_command() {
        let rig = Rig::new().device(ONE, None);
        rig.broker_up();
        let hub = rig.serve();
        hub.send(UiCommand::SendSchedule {
            device_id: ONE.to_owned(),
            profile: profile(),
        })
        .unwrap();
        hub.send(turn_on(ONE, true, TransportPref::Auto)).unwrap();
        settle().await;

        assert_eq!(
            rig.fake().log(),
            [
                (
                    Duration::ZERO,
                    "wifi TESTDEVICE01 UPDATE_PROFILE".to_owned()
                ),
                (secs(5), "wifi TESTDEVICE01 SYNC_TIME".to_owned()),
                (secs(5), "wifi TESTDEVICE01 TURN_ON".to_owned()),
            ]
        );
        let events = rig.events();
        let went = sent(&events);
        assert_eq!(went.len(), 3);
        assert!(went.iter().all(|(via, _, _)| *via == Via::Wifi));
        assert!(went[0].1.contains("UPDATE_PROFILE"), "{}", went[0].1);
        assert!(went[1].1.contains("SYNC_TIME"), "{}", went[1].1);
        assert_eq!(went[2].2, Some(true));
        // The schedule that went out is on the event, for the state to remember.
        let Some(Event::Sent {
            profile: sent_profile,
            ..
        }) = events.first()
        else {
            panic!("{events:?}");
        };
        assert_eq!(sent_profile.as_ref(), Some(&profile()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_schedule_goes_over_wifi_only_and_needs_the_broker() {
        // Bluetooth is known, but a schedule never goes over it.
        let rig = Rig::new().device(ONE, Some("ble-1"));
        let before = Instant::now();
        rig.shared.send_schedule(ONE, profile()).await;
        let events = rig.events();
        assert_eq!(
            failed(&events),
            [(
                Some(ONE.to_owned()),
                "UPDATE_PROFILE".to_owned(),
                "The broker isn't connected.".to_owned()
            )]
        );
        assert!(sent(&events).is_empty());
        // The time sync is not attempted, so there was no 5 s wait either.
        assert_eq!(before.elapsed(), Duration::ZERO);
        assert!(rig.fake().labels().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_time_sync_that_fails_says_the_schedule_may_not_run() {
        let rig = Rig::new().device(ONE, None);
        rig.broker_up();
        rig.fake()
            .script_wifi([Ok(()), Err("Not connected to the broker.".to_owned())]);
        rig.shared.send_schedule(ONE, profile()).await;
        let events = rig.events();
        assert_eq!(sent(&events).len(), 1);
        let lines = failed(&events);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[0].1, "WiFi SYNC_TIME");
        assert_eq!(lines[1].1, "Schedule");
        assert!(lines[1].2.contains("may not run"), "{}", lines[1].2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_bluetooth_failure_falls_back_to_wifi_with_one_failed_line() {
        let rig = Rig::new().device(ONE, Some("ble-1")).heard_on_broker(ONE);
        rig.broker_up();
        rig.fake().script_ble([Ble::After(
            secs(13),
            Err("Timed out while connecting.".to_owned()),
        )]);
        let started = Instant::now();

        let done = rig
            .shared
            .deliver(
                ONE,
                &Work::send(Command::TurnOn { on: true }),
                TransportPref::Auto,
            )
            .await;

        assert!(done);
        assert_eq!(started.elapsed(), secs(13));
        assert_eq!(
            rig.fake().labels(),
            [
                "ble ble-1 TURN_ON",
                "ble ble-1 TURN_ON done",
                "wifi TESTDEVICE01 TURN_ON"
            ]
        );
        let events = rig.events();
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(
            failed(&events),
            [(
                Some(ONE.to_owned()),
                "Bluetooth TURN_ON".to_owned(),
                "Bluetooth couldn't reach the diffuser, so this went over WiFi. Timed out while connecting."
                    .to_owned()
            )]
        );
        // The failure comes first, then the send that worked, over WiFi.
        assert!(matches!(events[0], Event::Failed { .. }));
        assert_eq!(
            sent(&events),
            [(Via::Wifi, TURN_ON_FRAME.to_owned(), Some(true))]
        );
        assert!(rig.repaints.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test(start_paused = true)]
    async fn after_a_bluetooth_failure_auto_skips_it_for_a_while_but_forced_bluetooth_does_not() {
        let rig = Rig::new().device(ONE, Some("ble-1")).heard_on_broker(ONE);
        rig.broker_up();
        rig.fake()
            .script_ble([Ble::After(Duration::ZERO, Err("Timed out.".to_owned()))]);
        let on = Work::send(Command::TurnOn { on: true });

        assert!(rig.shared.deliver(ONE, &on, TransportPref::Auto).await);
        rig.fake().clear();
        rig.events();

        // Within the cool-down, Auto goes straight to WiFi: no 13 s wait for nothing.
        assert!(rig.shared.deliver(ONE, &on, TransportPref::Auto).await);
        assert_eq!(rig.fake().labels(), ["wifi TESTDEVICE01 TURN_ON"]);
        assert!(failed(&rig.events()).is_empty());
        rig.fake().clear();

        // A forced Bluetooth ignores the cool-down. It works, so the cool-down ends...
        assert!(rig.shared.deliver(ONE, &on, TransportPref::Bluetooth).await);
        assert_eq!(
            rig.fake().labels(),
            ["ble ble-1 TURN_ON", "ble ble-1 TURN_ON done"]
        );
        rig.fake().clear();
        rig.events();

        // ...and Auto is Bluetooth-first again.
        assert!(rig.shared.deliver(ONE, &on, TransportPref::Auto).await);
        assert_eq!(rig.fake().labels()[0], "ble ble-1 TURN_ON");
        let events = rig.events();
        assert_eq!(sent(&events)[0].0, Via::Bluetooth);
    }

    #[tokio::test(start_paused = true)]
    async fn without_the_broker_auto_still_tries_bluetooth_after_a_failure() {
        // No broker: there is no WiFi to fall back to, so skipping Bluetooth would only
        // turn a slow command into a certain failure.
        let rig = Rig::new().device(ONE, Some("ble-1"));
        rig.fake()
            .script_ble([Ble::After(Duration::ZERO, Err("Timed out.".to_owned()))]);
        let on = Work::send(Command::TurnOn { on: true });

        assert!(!rig.shared.deliver(ONE, &on, TransportPref::Auto).await);
        rig.fake().clear();
        rig.events();

        assert!(rig.shared.deliver(ONE, &on, TransportPref::Auto).await);
        assert_eq!(
            rig.fake().labels(),
            ["ble ble-1 TURN_ON", "ble ble-1 TURN_ON done"]
        );
        assert_eq!(sent(&rig.events())[0].0, Via::Bluetooth);
    }

    #[tokio::test(start_paused = true)]
    async fn a_diffuser_not_heard_on_our_broker_is_never_sent_to_over_it() {
        // The broker is up, but this diffuser is not on it: a publish to its topic would
        // be accepted and heard by nobody, and reported as sent. That holds for one
        // found by a scan, one not on WiFi yet, and one whose WiFi is set up but that
        // may still be on Scentiment's cloud (its info says so, and nothing has been
        // heard from it here).
        for wifi_configured in [None, Some(false), Some(true)] {
            let rig = Rig::new().device(ONE, Some("ble-1"));
            lock(&rig.state).devices.get_mut(ONE).unwrap().info =
                wifi_configured.map(|configured| info(ONE, configured));
            rig.broker_up();
            rig.fake()
                .script_ble([Ble::After(Duration::ZERO, Err("Timed out.".to_owned()))]);
            let on = Work::send(Command::TurnOn { on: true });

            assert!(!rig.shared.deliver(ONE, &on, TransportPref::Auto).await);
            let events = rig.events();
            assert!(sent(&events).is_empty(), "{events:?}");
            assert_eq!(
                failed(&events),
                [(
                    Some(ONE.to_owned()),
                    "Bluetooth TURN_ON".to_owned(),
                    "Timed out.".to_owned()
                )]
            );
            assert_eq!(
                rig.fake().labels(),
                ["ble ble-1 TURN_ON", "ble ble-1 TURN_ON done"]
            );
            rig.fake().clear();

            // Nor does the failure start a cool-down: Bluetooth is tried again at once.
            assert!(rig.shared.deliver(ONE, &on, TransportPref::Auto).await);
            assert_eq!(
                rig.fake().labels(),
                ["ble ble-1 TURN_ON", "ble ble-1 TURN_ON done"]
            );
            assert_eq!(sent(&rig.events())[0].0, Via::Bluetooth);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_diffuser_with_a_wifi_reading_falls_back_to_wifi() {
        // A reading that came over WiFi shows it is on our broker, even when the
        // time it was last heard was not kept (`heard_wifi` is not saved).
        let rig = Rig::new().device(ONE, Some("ble-1"));
        rig.broker_up();
        {
            let mut state = lock(&rig.state);
            let device = state.devices.get_mut(ONE).unwrap();
            assert_eq!(device.heard_wifi, None);
            device.readings.insert(
                Field::Power,
                Reading {
                    value: 1,
                    at: 50,
                    via: Via::Wifi,
                },
            );
        }
        rig.fake()
            .script_ble([Ble::After(Duration::ZERO, Err("Timed out.".to_owned()))]);

        let on = Work::send(Command::TurnOn { on: true });
        assert!(rig.shared.deliver(ONE, &on, TransportPref::Auto).await);

        let events = rig.events();
        assert_eq!(sent(&events)[0].0, Via::Wifi);
        assert_eq!(failed(&events).len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_diffuser_with_no_bluetooth_id_is_sent_to_over_the_broker_even_if_not_known_on_it() {
        // Nothing else could reach it, and the broker is how it first shows up.
        let rig = Rig::new().device(ONE, None);
        rig.broker_up();
        let on = Work::send(Command::TurnOn { on: true });
        assert!(rig.shared.deliver(ONE, &on, TransportPref::Auto).await);
        assert_eq!(sent(&rig.events())[0].0, Via::Wifi);
    }

    #[tokio::test(start_paused = true)]
    async fn a_bluetooth_reply_that_never_comes_is_a_failure_after_30_s() {
        let rig = Rig::new().device(ONE, Some("ble-1")).heard_on_broker(ONE);
        rig.broker_up();
        rig.fake().script_ble([Ble::Never]);
        let started = Instant::now();

        let on = Work::send(Command::TurnOn { on: true });
        // The outer limit only turns a hang into a failed assertion.
        let done =
            tokio::time::timeout(secs(120), rig.shared.deliver(ONE, &on, TransportPref::Auto))
                .await;

        assert_eq!(done, Ok(true), "WiFi took over");
        assert_eq!(started.elapsed(), secs(30));
        let lines = failed(&rig.events());
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].2.contains("didn't answer"), "{}", lines[0].2);
        assert!(
            lines[0].2.contains("it may still go through"),
            "{}",
            lines[0].2
        );
        assert!(lines[0].2.contains("went over WiFi"), "{}", lines[0].2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_scan_that_never_answers_is_reported_after_30_s() {
        let rig = Rig::new();
        rig.fake().script_ble([Ble::Never]);
        let started = Instant::now();

        let scanned = tokio::time::timeout(secs(120), rig.shared.scan()).await;

        assert!(scanned.is_ok(), "the scan was waited for without end");
        assert_eq!(started.elapsed(), secs(30));
        let lines = failed(&rig.events());
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0].0, None);
        assert_eq!(lines[0].1, "Bluetooth scan");
        assert!(lines[0].2.contains("didn't answer"), "{}", lines[0].2);
    }

    #[tokio::test(start_paused = true)]
    async fn commands_lost_with_a_dead_queue_are_reported_and_the_next_one_still_goes() {
        let rig = Rig::new().device(ONE, Some("ble-1"));
        // The first command panics inside the queue's task, taking the one queued
        // behind it down with it.
        rig.fake().script_ble([Ble::Panics]);
        let hub = rig.serve();
        hub.send(turn_on(ONE, true, TransportPref::Auto)).unwrap();
        hub.send(turn_on(ONE, false, TransportPref::Auto)).unwrap();
        settle().await;
        assert!(sent(&rig.events()).is_empty());

        // The next command finds the queue dead. It says what was lost, and goes.
        hub.send(turn_on(ONE, true, TransportPref::Auto)).unwrap();
        settle().await;
        let events = rig.events();
        let lines = failed(&events);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0].0.as_deref(), Some(ONE));
        assert_eq!(lines[0].1, "Hub");
        assert!(lines[0].2.contains("dropped"), "{}", lines[0].2);
        // The Bluetooth frame keeps its `-|` terminator.
        assert_eq!(
            sent(&events),
            [(Via::Bluetooth, format!("{TURN_ON_FRAME}-|"), Some(true))]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn when_both_routes_fail_it_is_one_failed_line() {
        let rig = Rig::new().device(ONE, Some("ble-1")).heard_on_broker(ONE);
        rig.broker_up();
        rig.fake()
            .script_ble([Ble::After(Duration::ZERO, Err("No answer.".to_owned()))]);
        rig.fake()
            .script_wifi([Err("The broker connection is too busy.".to_owned())]);

        let done = rig
            .shared
            .deliver(
                ONE,
                &Work::send(Command::TurnOn { on: true }),
                TransportPref::Auto,
            )
            .await;

        assert!(!done);
        let events = rig.events();
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(
            failed(&events),
            [(
                Some(ONE.to_owned()),
                "TURN_ON".to_owned(),
                "Neither route worked. Bluetooth: No answer. WiFi: The broker connection is too busy."
                    .to_owned()
            )]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_forced_transport_does_not_fall_back() {
        let rig = Rig::new().device(ONE, Some("ble-1"));
        rig.broker_up();
        rig.fake()
            .script_ble([Ble::After(Duration::ZERO, Err("No answer.".to_owned()))]);
        let on = Work::send(Command::TurnOn { on: true });

        assert!(!rig.shared.deliver(ONE, &on, TransportPref::Bluetooth).await);
        assert_eq!(
            failed(&rig.events()),
            [(
                Some(ONE.to_owned()),
                "Bluetooth TURN_ON".to_owned(),
                "No answer.".to_owned()
            )]
        );
        // WiFi was never tried.
        assert!(
            rig.fake()
                .labels()
                .iter()
                .all(|label| !label.starts_with("wifi"))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn with_no_route_it_says_why() {
        let rig = Rig::new().device(ONE, None);
        let on = Work::send(Command::TurnOn { on: true });

        assert!(!rig.shared.deliver(ONE, &on, TransportPref::Auto).await);
        assert!(!rig.shared.deliver(ONE, &on, TransportPref::Wifi).await);
        assert!(!rig.shared.deliver(ONE, &on, TransportPref::Bluetooth).await);

        assert_eq!(
            failed(&rig.events()),
            [
                (
                    Some(ONE.to_owned()),
                    "TURN_ON".to_owned(),
                    "No Bluetooth ID and the broker isn't connected.".to_owned()
                ),
                (
                    Some(ONE.to_owned()),
                    "TURN_ON".to_owned(),
                    "The broker isn't connected.".to_owned()
                ),
                (
                    Some(ONE.to_owned()),
                    "TURN_ON".to_owned(),
                    "No Bluetooth ID for this diffuser.".to_owned()
                ),
            ]
        );
        assert!(rig.fake().labels().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_bluetooth_worker_that_went_away_counts_as_a_failure() {
        let rig = Rig::new().device(ONE, Some("ble-1")).heard_on_broker(ONE);
        rig.broker_up();
        rig.fake().script_ble([Ble::Dies]);

        let done = rig
            .shared
            .deliver(
                ONE,
                &Work::send(Command::TurnOn { on: true }),
                TransportPref::Auto,
            )
            .await;

        assert!(done, "WiFi took over");
        let lines = failed(&rig.events());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].2.contains("went over WiFi"), "{}", lines[0].2);
        assert!(lines[0].2.contains("Bluetooth stopped"), "{}", lines[0].2);
    }

    #[tokio::test(start_paused = true)]
    async fn reading_the_status_follows_the_same_route() {
        let rig = Rig::new().device(ONE, Some("ble-1")).heard_on_broker(ONE);
        rig.broker_up();
        let hub = rig.serve();
        // The diffuser is on WiFi, so it is pinged now and then; leave those out.
        let pings = |label: &String| !label.ends_with(" PING");
        let events = || -> Vec<Event> {
            let mut all = rig.events();
            all.retain(|event| !matches!(event, Event::Pinged { .. }));
            all
        };

        // Bluetooth reads what the diffuser last reported. Nothing is written, so
        // there is no `Sent`.
        hub.send(UiCommand::ReadStatus {
            device_id: ONE.to_owned(),
            pref: TransportPref::Auto,
        })
        .unwrap();
        settle().await;
        let labels: Vec<String> = rig.fake().labels().into_iter().filter(pings).collect();
        assert_eq!(
            labels,
            ["ble ble-1 read status", "ble ble-1 read status done"]
        );
        assert!(events().is_empty());
        rig.fake().clear();

        // When Bluetooth fails, GET_STATUS goes over WiFi.
        rig.fake()
            .script_ble([Ble::After(Duration::ZERO, Err("No answer.".to_owned()))]);
        hub.send(UiCommand::ReadStatus {
            device_id: ONE.to_owned(),
            pref: TransportPref::Auto,
        })
        .unwrap();
        settle().await;
        let events = events();
        assert_eq!(failed(&events)[0].1, "Bluetooth read status");
        let status = sent(&events);
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].0, Via::Wifi);
        assert!(status[0].1.contains("GET_STATUS"), "{status:?}");
        let labels = rig.fake().labels();
        assert!(labels.contains(&"wifi TESTDEVICE01 GET_STATUS".to_owned()));
    }

    #[tokio::test(start_paused = true)]
    async fn reading_the_status_honors_the_transport_choice() {
        let rig = Rig::new().device(ONE, Some("ble-1")).heard_on_broker(ONE);
        rig.broker_up();
        let hub = rig.serve();
        let read = |pref| UiCommand::ReadStatus {
            device_id: ONE.to_owned(),
            pref,
        };
        // The diffuser is on WiFi, so it is pinged now and then; leave those out.
        let labels = || -> Vec<String> {
            let mut labels = rig.fake().labels();
            labels.retain(|label| !label.ends_with(" PING"));
            labels
        };
        let events = || -> Vec<Event> {
            let mut all = rig.events();
            all.retain(|event| !matches!(event, Event::Pinged { .. }));
            all
        };

        // WiFi: GET_STATUS goes out and Bluetooth is left alone, though it is in reach.
        hub.send(read(TransportPref::Wifi)).unwrap();
        settle().await;
        assert_eq!(labels(), ["wifi TESTDEVICE01 GET_STATUS"]);
        let events_wifi = events();
        assert_eq!(
            sent(&events_wifi).iter().map(|s| s.0).collect::<Vec<_>>(),
            [Via::Wifi]
        );
        assert!(failed(&events_wifi).is_empty());
        rig.fake().clear();

        // Bluetooth: a failure is reported and WiFi does not take over.
        rig.fake()
            .script_ble([Ble::After(Duration::ZERO, Err("No answer.".to_owned()))]);
        hub.send(read(TransportPref::Bluetooth)).unwrap();
        settle().await;
        assert_eq!(
            labels(),
            ["ble ble-1 read status", "ble ble-1 read status done"]
        );
        let events_ble = events();
        assert!(sent(&events_ble).is_empty());
        assert_eq!(
            failed(&events_ble),
            [(
                Some(ONE.to_owned()),
                "Bluetooth read status".to_owned(),
                "No answer.".to_owned()
            )]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn sent_events_never_carry_a_password() {
        let rig = Rig::new().device(ONE, Some("ble-1")).heard_on_broker(ONE);
        rig.broker_up();
        rig.fake()
            .script_ble([Ble::After(Duration::ZERO, Err("No answer.".to_owned()))]);
        let secrets = [
            Command::SaveMqtt {
                broker: "wss://broker.example.test/mqtt".to_owned(),
                username: "user".to_owned(),
                password: Secret::new("hunter2".to_owned()),
            },
            Command::WifiConnect {
                ssid: "ExampleWiFi".to_owned(),
                password: Secret::new("hunter2".to_owned()),
            },
        ];
        for cmd in secrets {
            // Once over Bluetooth, which fails, and once over WiFi.
            let _ = rig
                .shared
                .deliver(ONE, &Work::send(cmd.clone()), TransportPref::Auto)
                .await;
            let _ = rig
                .shared
                .deliver(ONE, &Work::send(cmd), TransportPref::Wifi)
                .await;
        }
        let events = rig.events();
        assert!(!sent(&events).is_empty());
        let everything = format!("{events:?}");
        assert!(!everything.contains("hunter2"), "{everything}");
        assert!(
            everything.contains(r#"\"password\":\"***\""#),
            "{everything}"
        );
        // The device itself still gets the real thing.
        let wire = lock(&rig.fake().wifi_commands).clone();
        assert!(wire.iter().any(|(_, cmd)| {
            String::from_utf8(cmd.mqtt_body())
                .unwrap()
                .contains("hunter2")
        }));
    }

    #[tokio::test(start_paused = true)]
    async fn a_bad_device_id_is_refused_before_it_is_queued() {
        let rig = Rig::new();
        rig.broker_up();
        let hub = rig.serve();
        for bad in ["#", "TESTDEVICE01/x", ""] {
            hub.send(turn_on(bad, true, TransportPref::Auto)).unwrap();
        }
        settle().await;
        let lines = failed(&rig.events());
        assert_eq!(lines.len(), 3);
        for (device_id, _, error) in lines {
            assert_eq!(device_id, None);
            assert!(error.contains("not a valid device ID"), "{error}");
        }
        assert!(rig.fake().labels().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_scan_reports_only_its_failure() {
        let rig = Rig::new();
        let hub = rig.serve();
        hub.send(UiCommand::Scan).unwrap();
        settle().await;
        // The Bluetooth worker sends the results (`Scanned`) itself.
        assert_eq!(rig.fake().labels(), ["scan 10", "scan 10 done"]);
        assert!(rig.events().is_empty());

        rig.fake().script_ble([Ble::After(
            Duration::ZERO,
            Err("Bluetooth isn't available.".to_owned()),
        )]);
        hub.send(UiCommand::Scan).unwrap();
        settle().await;
        assert_eq!(
            failed(&rig.events()),
            [(
                None,
                "Bluetooth scan".to_owned(),
                "Bluetooth isn't available.".to_owned()
            )]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn pings_go_to_devices_on_wifi_only_while_the_broker_is_connected() {
        let rig = Rig::new().device(ONE, None).device(TWO, Some("ble-2"));
        {
            let mut state = lock(&rig.state);
            state.devices.get_mut(ONE).unwrap().info = Some(info(ONE, true));
            state.devices.get_mut(TWO).unwrap().info = Some(info(TWO, false));
        }

        // The broker is down: nothing is sent, and the caller is told to look again soon.
        assert!(!rig.shared.ping_round());
        assert!(rig.fake().labels().is_empty());
        assert!(rig.events().is_empty());

        rig.broker_up();
        assert!(rig.shared.ping_round());
        assert_eq!(
            lock(&rig.fake().wifi_commands).clone(),
            [(
                ONE.to_owned(),
                Command::Ping {
                    userid: Some("route-test-1".to_owned())
                }
            )]
        );
        let events = rig.events();
        assert_eq!(events.len(), 1, "{events:?}");
        assert!(matches!(&events[0], Event::Pinged { device_id, .. } if device_id == ONE));
    }

    #[tokio::test(start_paused = true)]
    async fn the_ping_loop_pings_as_soon_as_the_broker_is_up_and_then_every_30_s() {
        let rig = Rig::new().device(ONE, None);
        lock(&rig.state).devices.get_mut(ONE).unwrap().info = Some(info(ONE, true));
        let _hub = rig.serve();

        // The broker connects after 10 s: the first ping follows within a second.
        tokio::time::sleep(secs(10)).await;
        assert!(rig.fake().labels().is_empty());
        rig.broker_up();
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(rig.fake().log().len(), 1);
        let first = rig.fake().log()[0].0;
        assert!(first >= secs(10) && first <= secs(11), "{first:?}");

        // Then once every PING_EVERY_S: at 11 s, 41 s, 71 s and 101 s at the latest.
        tokio::time::sleep(secs(95)).await;
        let log = rig.fake().log();
        assert_eq!(log.len(), 4, "{log:?}");
        assert_eq!(log[1].0 - log[0].0, secs(30));
        assert_eq!(log[2].0 - log[1].0, secs(30));
    }

    #[tokio::test(start_paused = true)]
    async fn reconnecting_without_a_password_says_so_and_starts_nothing() {
        let rig = Rig::new();
        let hub = rig.serve();
        let settings = BrokerSettings::default();

        hub.send(UiCommand::Reconnect {
            settings: settings.clone(),
            password: None,
        })
        .unwrap();
        settle().await;
        assert_eq!(
            rig.events(),
            [Event::BrokerDisconnected(
                "No broker password saved. Add it in Settings (if you already did, the Keychain couldn't be read)."
                    .to_owned()
            )]
        );
        assert_eq!(
            *lock(&rig.fake().reconnects),
            [(settings.url.clone(), false)]
        );

        hub.send(UiCommand::Reconnect {
            settings: settings.clone(),
            password: Some(Secret::new("x".to_owned())),
        })
        .unwrap();
        settle().await;
        assert!(rig.events().is_empty(), "the worker reports for itself");
        assert_eq!(lock(&rig.fake().reconnects).len(), 2);
        assert_eq!(lock(&rig.fake().reconnects)[1], (settings.url, true));
    }

    #[tokio::test(start_paused = true)]
    async fn a_poisoned_state_lock_does_not_stop_the_hub() {
        let rig = Rig::new().device(ONE, Some("ble-1"));
        rig.broker_up();
        let state = Arc::clone(&rig.state);
        let poisoner = std::thread::spawn(move || {
            let _guard = state.lock().unwrap();
            // Unwinds without the panic hook, so nothing is printed.
            std::panic::resume_unwind(Box::new("poisoned on purpose"));
        });
        assert!(poisoner.join().is_err());
        assert!(rig.state.is_poisoned());

        let on = Work::send(Command::TurnOn { on: true });
        assert!(rig.shared.deliver(ONE, &on, TransportPref::Auto).await);
        assert!(rig.shared.ping_round());
        assert_eq!(sent(&rig.events())[0].0, Via::Bluetooth);
    }

    // --- adding a diffuser ---

    fn request(ble_id: &str, wifi: Option<(&str, &str)>) -> AddRequest {
        AddRequest {
            ble_id: ble_id.to_owned(),
            label: "Bedroom".to_owned(),
            wifi: wifi.map(|(ssid, password)| (ssid.to_owned(), Secret::new(password.to_owned()))),
        }
    }

    /// Runs an add the way the hub does: the read, then the rest as the diffuser's job.
    async fn add(rig: &Rig, request: AddRequest) {
        if let Some((device_id, job)) = rig.shared.prepare_add(request).await {
            rig.shared.run_add(&device_id, job).await;
        }
    }

    /// `device_connected` from the diffuser, heard at `at`.
    fn device_connected(id: &str, at: i64) -> Event {
        let body = serde_json::json!({
            "action": "device_connected",
            "payload": { "deviceId": id, "fw": "2.30", "model": "Diffuser", "variant": "Black", "manufacturer": "01" },
        });
        Event::Mqtt {
            topic: "scentiment-mqtt-prod/server-notifications/device".to_owned(),
            body: body.to_string().into_bytes(),
            at,
        }
    }

    /// A moment after any restart the hub has made so far, in the app's clock.
    fn soon() -> i64 {
        unix_now() + 5
    }

    fn steps(events: &[Event]) -> Vec<AddStep> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Adding { step, .. } => Some(step.clone()),
                _ => None,
            })
            .collect()
    }

    /// When the fake was asked for `label`, the first time.
    fn when(rig: &Rig, label: &str) -> Duration {
        rig.fake()
            .log()
            .into_iter()
            .find(|(_, seen)| seen == label)
            .unwrap_or_else(|| panic!("never asked for {label:?}: {:?}", rig.fake().labels()))
            .0
    }

    const CONNECT_ERROR: &str = "Timed out while connecting. A diffuser takes one Bluetooth connection at a time: close the Scentiment app or turn the phone's Bluetooth off, and check that the diffuser is on and within a few meters.";

    #[tokio::test(start_paused = true)]
    async fn add_runs_in_order_over_bluetooth_only() {
        // Heard on our broker, so Auto would use WiFi as well: the add's writes never do.
        let rig = Rig::new()
            .device(ONE, Some("ble-1"))
            .heard_on_broker(ONE)
            .info_for("ble-1", info(ONE, true))
            .logged_in("broker-secret");
        rig.broker_up();
        rig.apply_later(secs(40), device_connected(ONE, soon()));

        add(&rig, request("ble-1", None)).await;

        assert_eq!(
            rig.fake().labels(),
            [
                "ble ble-1 read info",
                "ble ble-1 read info done",
                "ble ble-1 SAVE_MQTT",
                "ble ble-1 SAVE_MQTT done",
                "ble ble-1 SECRET_COMMAND",
                "ble ble-1 SECRET_COMMAND done",
                "wifi TESTDEVICE01 PING",
            ]
        );
        let events = rig.events();
        assert_eq!(
            steps(&events),
            [
                AddStep::ReadInfo,
                AddStep::SaveMqtt,
                AddStep::Restart,
                AddStep::WaitConnected,
                AddStep::Ping,
                AddStep::Done(ONE.to_owned()),
            ]
        );
        // SAVE_MQTT hands over the broker the hub is logged in to, with its password.
        let ble = lock(&rig.fake().ble_commands).clone();
        let Some((
            _,
            Command::SaveMqtt {
                broker,
                username,
                password,
            },
        )) = ble.first()
        else {
            panic!("{ble:?}");
        };
        let settings = BrokerSettings::default();
        assert_eq!((broker, username), (&settings.url, &settings.username));
        assert_eq!(password.expose(), "broker-secret");
        assert_eq!(ble[1].1, Command::SoftRestart);
        // The ping goes over WiFi, with our route ID, and the diffuser counts as pinged.
        assert_eq!(
            lock(&rig.fake().wifi_commands).clone(),
            [(
                ONE.to_owned(),
                Command::Ping {
                    userid: Some(settings.route_id)
                }
            )]
        );
        let vias: Vec<Via> = sent(&events).into_iter().map(|(via, _, _)| via).collect();
        assert_eq!(vias, [Via::Bluetooth, Via::Bluetooth, Via::Wifi]);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Pinged { device_id, .. } if device_id == ONE))
        );
        assert!(failed(&events).is_empty(), "{events:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_save_that_cannot_reach_the_diffuser_fails_without_trying_wifi() {
        let rig = Rig::new()
            .device(ONE, Some("ble-1"))
            .heard_on_broker(ONE)
            .info_for("ble-1", info(ONE, true))
            .logged_in("broker-secret");
        rig.broker_up();
        rig.fake().script_ble([
            Ble::After(Duration::ZERO, Ok(())),
            Ble::After(secs(13), Err(CONNECT_ERROR.to_owned())),
        ]);

        add(&rig, request("ble-1", None)).await;

        let labels = rig.fake().labels();
        assert!(
            labels.iter().all(|label| !label.starts_with("wifi")),
            "{labels:?}"
        );
        assert_eq!(labels.last().unwrap(), "ble ble-1 SAVE_MQTT done");
        let events = rig.events();
        assert_eq!(
            steps(&events),
            [
                AddStep::ReadInfo,
                AddStep::SaveMqtt,
                AddStep::Failed(CONNECT_ERROR.to_owned())
            ]
        );
        assert_eq!(
            failed(&events),
            [(
                Some(ONE.to_owned()),
                "Bluetooth SAVE_MQTT".to_owned(),
                CONNECT_ERROR.to_owned()
            )]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_save_whose_answer_never_comes_goes_on_to_the_restart() {
        // The backstop: it may still have gone through, so it is not sent again.
        let rig = Rig::new()
            .device(ONE, Some("ble-1"))
            .info_for("ble-1", info(ONE, true))
            .logged_in("broker-secret");
        rig.broker_up();
        rig.fake()
            .script_ble([Ble::After(Duration::ZERO, Ok(())), Ble::Never]);
        rig.apply_later(secs(90), device_connected(ONE, soon()));

        add(&rig, request("ble-1", None)).await;

        let labels = rig.fake().labels();
        assert_eq!(
            labels
                .iter()
                .filter(|label| *label == "ble ble-1 SAVE_MQTT")
                .count(),
            1,
            "{labels:?}"
        );
        // 30 s for the backstop, then up to 25 s for "Credentials saved".
        assert_eq!(when(&rig, "ble ble-1 SECRET_COMMAND"), secs(55));
        assert_eq!(
            steps(&rig.events()).last(),
            Some(&AddStep::Done(ONE.to_owned()))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn add_waits_for_credentials_saved_up_to_25_s() {
        let rig = || {
            let rig = Rig::new()
                .device(ONE, Some("ble-1"))
                .info_for("ble-1", info(ONE, true))
                .logged_in("broker-secret");
            rig.broker_up();
            rig
        };
        let saved_to_restart = |rig: &Rig| {
            when(rig, "ble ble-1 SECRET_COMMAND") - when(rig, "ble ble-1 SAVE_MQTT done")
        };

        // Nothing heard: the restart goes 25 s after the save.
        let quiet = rig();
        add(&quiet, request("ble-1", None)).await;
        assert_eq!(saved_to_restart(&quiet), secs(25));

        // "Credentials saved" over Bluetooth after 4 s: the restart follows.
        let over_ble = rig();
        over_ble.apply_later(
            secs(4),
            Event::BleNotification {
                ble_id: "ble-1".to_owned(),
                bytes: b"Credentials saved-|".to_vec(),
                at: soon(),
            },
        );
        add(&over_ble, request("ble-1", None)).await;
        let waited = saved_to_restart(&over_ble);
        assert!(
            waited >= secs(4) && waited <= Duration::from_millis(4_250),
            "{waited:?}"
        );

        // As an acknowledgement on the broker after 6 s: the same.
        let over_wifi = rig();
        let ack = serde_json::json!({
            "action": "acknowledgement",
            "payload": { "deviceId": ONE, "msg": "Credentials saved" },
        });
        over_wifi.apply_later(
            secs(6),
            Event::Mqtt {
                topic: "scentiment-mqtt-prod/user/route-test-1".to_owned(),
                body: ack.to_string().into_bytes(),
                at: soon(),
            },
        );
        add(&over_wifi, request("ble-1", None)).await;
        let waited = saved_to_restart(&over_wifi);
        assert!(
            waited >= secs(6) && waited <= Duration::from_millis(6_250),
            "{waited:?}"
        );

        // One from an earlier save is not this save's.
        let earlier = rig();
        lock(&earlier.state)
            .devices
            .get_mut(ONE)
            .unwrap()
            .credentials_saved_at = Some(unix_now() - 100);
        add(&earlier, request("ble-1", None)).await;
        assert_eq!(saved_to_restart(&earlier), secs(25));
    }

    #[tokio::test(start_paused = true)]
    async fn a_restart_write_error_still_waits_for_connected() {
        // The reboot can drop the link before the write is acknowledged: an error, or no
        // answer until the backstop. Either way `device_connected` is the proof.
        let errors = [
            Ble::After(
                Duration::ZERO,
                Err("Couldn't write to the diffuser: Not connected".to_owned()),
            ),
            Ble::Never,
        ];
        for restart in errors {
            let rig = Rig::new()
                .device(ONE, Some("ble-1"))
                .info_for("ble-1", info(ONE, true))
                .logged_in("broker-secret");
            rig.broker_up();
            rig.fake().script_ble([
                Ble::After(Duration::ZERO, Ok(())),
                Ble::After(Duration::ZERO, Ok(())),
                restart,
            ]);
            // It joins at 40 s: before the restart's backstop (25 s + 30 s) runs out.
            rig.apply_later(secs(40), device_connected(ONE, soon()));

            add(&rig, request("ble-1", None)).await;

            let events = rig.events();
            assert_eq!(
                steps(&events),
                [
                    AddStep::ReadInfo,
                    AddStep::SaveMqtt,
                    AddStep::Restart,
                    AddStep::WaitConnected,
                    AddStep::Ping,
                    AddStep::Done(ONE.to_owned()),
                ]
            );
            let lines = failed(&events);
            assert_eq!(lines.len(), 1, "{lines:?}");
            assert_eq!(lines[0].1, "Bluetooth SECRET_COMMAND");
            // The restart was written once.
            let restarts = lock(&rig.fake().ble_commands)
                .iter()
                .filter(|(_, cmd)| *cmd == Command::SoftRestart)
                .count();
            assert_eq!(restarts, 1);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_diffuser_that_never_joins_fails_after_90_s_with_what_to_check() {
        let rig = Rig::new()
            .device(ONE, Some("ble-1"))
            .info_for("ble-1", info(ONE, true))
            .logged_in("broker-secret");
        rig.broker_up();
        // Joined before the restart: that says nothing about after it.
        lock(&rig.state).devices.get_mut(ONE).unwrap().connected_at = Some(unix_now() - 100);

        add(&rig, request("ble-1", None)).await;

        let restart = when(&rig, "ble ble-1 SECRET_COMMAND done");
        let events = rig.events();
        let Some(AddStep::Failed(why)) = steps(&events).last().cloned() else {
            panic!("{events:?}");
        };
        assert!(
            why.contains("2.4 GHz") && why.contains("WiFi password"),
            "{why}"
        );
        assert_eq!(Instant::now() - rig.fake().began - restart, secs(90));
        // No ping was sent.
        assert!(lock(&rig.fake().wifi_commands).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn add_without_a_broker_password_fails_at_once() {
        let rig = Rig::new().info_for("ble-1", info(ONE, true));
        rig.broker_up();
        let before = Instant::now();

        add(&rig, request("ble-1", None)).await;

        assert_eq!(before.elapsed(), Duration::ZERO);
        assert!(rig.fake().labels().is_empty(), "Bluetooth was not touched");
        let failed = Event::Adding {
            ble_id: "ble-1".to_owned(),
            step: AddStep::Failed("Save the broker password in Settings first.".to_owned()),
        };
        assert_eq!(rig.events(), std::slice::from_ref(&failed));

        // Started with no password (none saved, or the Keychain unreadable): the same.
        rig.shared.reconnect(BrokerSettings::default(), None);
        rig.events();
        add(&rig, request("ble-1", None)).await;
        assert_eq!(rig.events(), [failed]);
        assert!(rig.fake().labels().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn an_unconfigured_diffuser_with_no_network_given_asks_for_one() {
        let rig = Rig::new()
            .info_for("ble-1", info(ONE, false))
            .logged_in("broker-secret");
        rig.broker_up();

        add(&rig, request("ble-1", None)).await;

        assert_eq!(
            rig.fake().labels(),
            ["ble ble-1 read info", "ble ble-1 read info done"]
        );
        assert_eq!(steps(&rig.events()), [AddStep::ReadInfo, AddStep::NeedWifi]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_read_that_fails_or_brings_no_info_stops_the_add() {
        let rig = Rig::new().logged_in("broker-secret");
        rig.fake()
            .script_ble([Ble::After(secs(13), Err(CONNECT_ERROR.to_owned()))]);
        add(&rig, request("ble-1", None)).await;
        assert_eq!(
            steps(&rig.events()),
            [AddStep::ReadInfo, AddStep::Failed(CONNECT_ERROR.to_owned())]
        );

        // The read worked, but no info for a diffuser reached the state.
        let rig = Rig::new().logged_in("broker-secret");
        let before = Instant::now();
        add(&rig, request("ble-1", None)).await;
        assert_eq!(before.elapsed(), secs(5));
        let events = rig.events();
        let Some(AddStep::Failed(why)) = steps(&events).last().cloned() else {
            panic!("{events:?}");
        };
        assert!(why.contains("info"), "{why}");
        assert_eq!(
            rig.fake().labels(),
            ["ble ble-1 read info", "ble ble-1 read info done"]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_ping_the_broker_will_not_take_fails_the_add() {
        // The broker went down after the diffuser joined it.
        let rig = Rig::new()
            .device(ONE, Some("ble-1"))
            .info_for("ble-1", info(ONE, true))
            .logged_in("broker-secret");
        rig.apply_later(secs(30), device_connected(ONE, soon()));

        add(&rig, request("ble-1", None)).await;

        let events = rig.events();
        let Some(AddStep::Failed(why)) = steps(&events).last().cloned() else {
            panic!("{events:?}");
        };
        assert!(why.contains("ping"), "{why}");
        assert!(failed(&events).iter().any(|(_, what, _)| what == "PING"));
    }

    #[tokio::test(start_paused = true)]
    async fn commands_to_that_diffuser_wait_behind_the_add() {
        // Not on WiFi yet, so the ping round leaves it alone until it joins.
        let rig = Rig::new()
            .device(ONE, Some("ble-1"))
            .info_for("ble-1", info(ONE, false))
            .logged_in("broker-secret");
        rig.broker_up();
        // The save takes 3 s; it joins at 40 s.
        rig.fake().script_ble([
            Ble::After(Duration::ZERO, Ok(())),
            Ble::After(secs(3), Ok(())),
        ]);
        rig.apply_later(secs(40), device_connected(ONE, soon()));
        let hub = rig.serve();

        hub.send(UiCommand::Add(request(
            "ble-1",
            Some(("ExampleWiFi", "hunter2")),
        )))
        .unwrap();
        // Made while the save is being written.
        tokio::time::sleep(secs(1)).await;
        hub.send(turn_on(ONE, true, TransportPref::Bluetooth))
            .unwrap();
        tokio::time::sleep(secs(120)).await;

        let labels = rig.fake().labels();
        let turn_on = labels
            .iter()
            .position(|label| label == "ble ble-1 TURN_ON")
            .unwrap_or_else(|| panic!("{labels:?}"));
        assert_eq!(
            labels[..=turn_on],
            [
                "ble ble-1 read info",
                "ble ble-1 read info done",
                "ble ble-1 SAVE_MQTT",
                "ble ble-1 SAVE_MQTT done",
                "ble ble-1 WIFI_CONNECT",
                "ble ble-1 WIFI_CONNECT done",
                "ble ble-1 SECRET_COMMAND",
                "ble ble-1 SECRET_COMMAND done",
                "wifi TESTDEVICE01 PING",
                "ble ble-1 TURN_ON",
            ]
        );
        assert!(when(&rig, "ble ble-1 TURN_ON") >= secs(40));
        assert_eq!(
            steps(&rig.events()).last(),
            Some(&AddStep::Done(ONE.to_owned()))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn wifi_password_never_appears_in_events_or_log() {
        let rig = Rig::new()
            .device(ONE, Some("ble-1"))
            .info_for("ble-1", info(ONE, false))
            .logged_in("broker-hunter3");
        rig.broker_up();
        rig.apply_later(secs(40), device_connected(ONE, soon()));

        add(&rig, request("ble-1", Some(("ExampleWiFi", "hunter2")))).await;

        let events = rig.events();
        assert_eq!(steps(&events).last(), Some(&AddStep::Done(ONE.to_owned())));
        let mut state = lock(&rig.state).clone();
        for event in &events {
            let shown = format!("{event:?}");
            for secret in ["hunter2", "broker-hunter3"] {
                assert!(!shown.contains(secret), "{shown}");
            }
            state.apply(event.clone());
        }
        // The log, and everything else the screens see.
        assert!(state.log.iter().any(|entry| entry.action == "WIFI_CONNECT"));
        let everything = format!("{state:?}");
        for secret in ["hunter2", "broker-hunter3"] {
            assert!(!everything.contains(secret), "{everything}");
        }
        // The diffuser itself got them.
        let ble = lock(&rig.fake().ble_commands).clone();
        let frames: Vec<String> = ble
            .iter()
            .map(|(_, cmd)| String::from_utf8(cmd.ble_frame()).unwrap())
            .collect();
        assert!(frames.iter().any(|frame| frame.contains("broker-hunter3")));
        assert!(
            frames
                .iter()
                .any(|frame| frame.contains("hunter2") && frame.contains("ExampleWiFi"))
        );
    }

    #[test]
    fn a_hub_that_is_not_running_says_so() {
        let (events, received) = channel();
        let (commands, queue) = unbounded_channel();
        drop(queue);
        let repaints = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&repaints);
        let hub = Hub {
            commands,
            events,
            repaint: Arc::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            }),
        };

        hub.submit(UiCommand::Scan);

        let Ok(Event::Failed {
            device_id: None,
            what,
            error,
            ..
        }) = received.try_recv()
        else {
            panic!("expected a Failed event");
        };
        assert_eq!(what, "Hub");
        assert!(error.contains("isn't running"), "{error}");
        assert_eq!(repaints.load(Ordering::SeqCst), 1);
    }

    // Nothing here touches Bluetooth or the network. With no password the hub starts
    // no broker worker. The Bluetooth worker is started, but it creates no
    // `btleplug::Manager` until its first request, and this test makes none.
    #[test]
    fn starting_without_a_password_reports_it_and_runs_the_hub() {
        let (hub, received) = Hub::start(
            BrokerSettings::default(),
            None,
            Arc::new(Mutex::new(AppState::default())),
            || {},
        );
        let event = received.recv_timeout(secs(5)).expect("the hub reports");
        assert!(
            matches!(&event, Event::BrokerDisconnected(why) if why.starts_with("No broker password saved.")),
            "{event:?}"
        );

        // A command it can answer without Bluetooth or the broker: no route.
        hub.submit(turn_on(ONE, true, TransportPref::Auto));
        let event = received.recv_timeout(secs(5)).expect("the hub answers");
        assert!(
            matches!(&event, Event::Failed { device_id: Some(id), .. } if id == ONE),
            "{event:?}"
        );
    }
}
