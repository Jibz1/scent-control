//! The Bluetooth worker: finds diffusers, and keeps one link to each diffuser it is
//! asked to talk to.
//!
//! The worker runs on its own thread and reports through the same channel as the other
//! workers. It never touches the screens. Every [`BleHandle`] method queues its work
//! without waiting for Bluetooth and returns a [`Reply`] that says how the work ended;
//! what comes back from a diffuser (scan results, the info string, notifications) goes
//! to the event channel.
//!
//! Bluetooth is not touched until the first request that needs it, so macOS asks for
//! permission when the user first scans or connects, not when the app starts. macOS
//! ends any process that uses Bluetooth outside an app bundle that declares a
//! Bluetooth usage description, so nothing here may run from a plain `cargo run` or
//! `cargo test`; see `scripts/bundle-macos.sh`.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::fmt::Display;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use btleplug::api::{
    Central, CentralEvent, CentralState, Characteristic, Manager as _, Peripheral as _,
    PeripheralProperties, RetrievePeripheralsOptions, ScanFilter, ValueNotification, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral, PeripheralId};
use futures::stream::{Stream, StreamExt};
use scent_core::{Command, Via, chunks, is_scentiment_name, parse_info};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{OnceCell, mpsc, oneshot, watch};
use tokio::task::{JoinError, JoinSet, LocalSet};
use uuid::{Uuid, uuid};

use crate::state::{Event, ScanHit, unix_now};

/// The diffuser's GATT service, and the characteristics we use in it.
const SERVICE: Uuid = uuid!("00000180-0000-1000-8000-00805f9b34fb");
/// Frames are written here.
const WRITE_CHAR: Uuid = uuid!("0000dead-0000-1000-8000-00805f9b34fb");
/// Notifies when the state changes. Reading it returns the last value it sent.
const STATUS_CHAR: Uuid = uuid!("0000fef3-0000-1000-8000-00805f9b34fb");
/// Reads the info string.
const INFO_CHAR: Uuid = uuid!("0000fef4-0000-1000-8000-00805f9b34fb");

/// How long it may take to find a diffuser and open a link to it.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to keep listening to a diffuser after a command went out.
pub const LISTEN_AFTER_SEND: Duration = Duration::from_secs(6);

/// How long a link may sit unused before it is closed.
const IDLE_AFTER: Duration = Duration::from_secs(10);

/// How long one read, or the writes of one command, may take on an open link.
const IO_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to wait for CoreBluetooth to confirm a disconnect. It confirms only through
/// the callback that says the peripheral disconnected, and sends none when the
/// peripheral was not connected, so the wait must end on its own.
const DISCONNECT_LIMIT: Duration = Duration::from_secs(3);

/// How often to look again for a diffuser while scanning for it.
const LOOKUP_POLL: Duration = Duration::from_millis(200);

/// How many requests can wait for the worker, and for one diffuser's link.
const QUEUE: usize = 16;

/// What to try when a diffuser can't be reached. A diffuser takes one Bluetooth
/// connection at a time, so the usual cause is another app holding it.
const CONNECT_HINT: &str = "A diffuser takes one Bluetooth connection at a time: close the \
     Scentiment app or turn the phone's Bluetooth off, and check that the diffuser is on and \
     within a few meters.";

/// When a link is due to be closed, as long as nothing uses it.
#[derive(Debug, Clone, Copy)]
pub struct LinkTimer {
    until: Instant,
}

impl LinkTimer {
    /// A link opened at `now`.
    pub fn new(now: Instant) -> LinkTimer {
        LinkTimer {
            until: now + IDLE_AFTER,
        }
    }

    /// The link was used at `now`: it stays open 10 s from here.
    pub fn touch(&mut self, now: Instant) {
        self.hold_until(now + IDLE_AFTER);
    }

    /// A command went out at `now`: the link stays open at least long enough to hear
    /// the diffuser's answer.
    pub fn sent(&mut self, now: Instant) {
        self.hold_until(now + IDLE_AFTER.max(LISTEN_AFTER_SEND));
    }

    pub fn should_disconnect(&self, now: Instant) -> bool {
        now >= self.until
    }

    /// When the link is due to close if nothing uses it before then.
    pub fn deadline(&self) -> Instant {
        self.until
    }

    /// Never brings the deadline forward.
    fn hold_until(&mut self, until: Instant) {
        self.until = self.until.max(until);
    }
}

/// The writes that carry `cmd` to the diffuser, in order.
fn writes_for(cmd: &Command) -> Vec<Vec<u8>> {
    chunks(&cmd.ble_frame()).map(<[u8]>::to_vec).collect()
}

/// How a `ble_id` is written. This is the one place a [`PeripheralId`] becomes text:
/// scan results, info and notifications all use it, and lookups parse it back. On
/// macOS it is the peripheral's UUID, in lower case with hyphens.
fn ble_id_of(id: &PeripheralId) -> String {
    id.to_string()
}

/// Reads a stored `ble_id`, in any case.
fn parse_ble_id(text: &str) -> Result<Uuid, String> {
    Uuid::parse_str(text.trim()).map_err(|_| format!("{text:?} is not a Bluetooth ID."))
}

/// The scan result for a peripheral, when it is a Scentiment diffuser.
fn hit_for(ble_id: String, properties: &PeripheralProperties) -> Option<ScanHit> {
    let name = properties
        .local_name
        .as_deref()
        .or(properties.advertisement_name.as_deref())?;
    is_scentiment_name(name).then(|| ScanHit {
        ble_id,
        name: name.to_owned(),
        rssi: properties.rssi,
    })
}

/// What a failed command is called in the log. The action's name only: the frame can
/// hold a password.
fn what_for(cmd: &Command) -> String {
    format!("Bluetooth send {}", cmd.action(Via::Bluetooth))
}

/// What setting up the adapter failed with. macOS refuses once the user has said no to
/// the permission prompt, and only System Settings can undo that.
fn unavailable(error: &btleplug::Error) -> String {
    match error {
        btleplug::Error::PermissionDenied => "macOS has not allowed Scent Control to use \
             Bluetooth. Allow it in System Settings, Privacy & Security, Bluetooth, then try \
             again."
            .to_owned(),
        other => format!("Bluetooth isn't available: {other}"),
    }
}

/// The characteristics of a diffuser that we use.
#[derive(Debug)]
struct Chars {
    write: Characteristic,
    notify: Characteristic,
    info: Characteristic,
}

impl Chars {
    /// Picks them out of what service discovery found. The error names the first one
    /// that is missing.
    fn find(found: &BTreeSet<Characteristic>) -> Result<Chars, String> {
        let pick = |uuid: Uuid, name: &str| {
            found
                .iter()
                .find(|c| c.service_uuid == SERVICE && c.uuid == uuid)
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "The device has no Bluetooth characteristic {name}, so it is not a \
                         diffuser we can talk to."
                    )
                })
        };
        Ok(Chars {
            write: pick(WRITE_CHAR, "0xDEAD")?,
            notify: pick(STATUS_CHAR, "0xFEF3")?,
            info: pick(INFO_CHAR, "0xFEF4")?,
        })
    }
}

/// How a request ended: `Ok` when it was done, `Err` with what went wrong. The reply
/// resolves with a receive error, not an answer, only if the worker stopped first or
/// the task that held the request panicked. Dropping the reply withdraws a request
/// that has not started yet: the worker skips it, without a word.
pub type Reply = oneshot::Receiver<Result<(), String>>;

/// A reply that is already answered.
fn answered(result: Result<(), String>) -> Reply {
    let (answer, reply) = oneshot::channel();
    let _ = answer.send(result);
    reply
}

/// The worker's side of a [`Reply`].
struct Done {
    answer: oneshot::Sender<Result<(), String>>,
    /// What was attempted, for the `Failed` event.
    what: String,
}

impl Done {
    fn new(what: impl Into<String>) -> (Done, Reply) {
        let (answer, reply) = oneshot::channel();
        let done = Done {
            answer,
            what: what.into(),
        };
        (done, reply)
    }

    /// Whether the caller has stopped waiting: it dropped the [`Reply`]. The hub does
    /// that when it gives up on a request and sends the command another way, so a job
    /// in this state must not run: it could land after something newer and undo it.
    fn is_abandoned(&self) -> bool {
        self.answer.is_closed()
    }

    /// Answers the caller of a job that ran. A failure that nobody is waiting to hear
    /// (the reply was dropped while the job ran) becomes an `Event::Failed` so that it
    /// is not lost. It carries no device ID: only the hub knows which device the
    /// Bluetooth ID belongs to.
    fn finish(self, emitter: &Emitter, result: Result<(), String>) {
        match result {
            Ok(()) => {
                let _ = self.answer.send(Ok(()));
            }
            Err(error) => {
                if let Err(Err(error)) = self.answer.send(Err(error)) {
                    emitter.emit(Event::Failed {
                        device_id: None,
                        what: self.what,
                        error,
                        at: unix_now(),
                    });
                }
            }
        }
    }

    /// Answers the caller of a job that never ran, unless the caller has stopped
    /// waiting: then it is dropped without a word, as a job skipped for that reason is,
    /// since the caller has reported its own timeout.
    fn finish_unless_abandoned(self, emitter: &Emitter, result: Result<(), String>) {
        if !self.is_abandoned() {
            self.finish(emitter, result);
        }
    }
}

/// What a connected diffuser can be asked to do.
enum Op {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the add-a-diffuser sheet (Task 10) is the first to ask"
        )
    )]
    ReadInfo,
    Send(Command),
    ReadStatus,
}

enum Task {
    Scan { secs: u64 },
    Link { ble_id: String, op: Op },
}

struct Job {
    task: Task,
    done: Done,
}

/// A job for one diffuser's link.
struct LinkJob {
    op: Op,
    done: Done,
}

/// How to queue Bluetooth work. Dropping it stops the worker, which closes every link.
pub struct BleHandle {
    jobs: mpsc::Sender<Job>,
}

impl BleHandle {
    /// Scans for `secs` seconds, then sends `Event::Scanned` with the Scentiment
    /// diffusers known by then. A diffuser seen earlier that has since gone out of
    /// range can still be in the list: connecting to it fails with a clear message.
    #[must_use = "a refusal is reported only through the Reply"]
    pub fn scan(&self, secs: u64) -> Reply {
        self.queue("Bluetooth scan", Task::Scan { secs })
    }

    /// Reads the diffuser's info string and sends it as `Event::BleInfo`. `ble_id` is
    /// what an earlier scan or info event reported.
    #[must_use = "a refusal is reported only through the Reply"]
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the add-a-diffuser sheet (Task 10) is the first to ask"
        )
    )]
    pub fn read_info(&self, ble_id: &str) -> Reply {
        self.link("Bluetooth read info", ble_id, Op::ReadInfo)
    }

    /// Writes `cmd` to the diffuser, opening a link to it first if there is none. The
    /// reply is `Ok` once the last write has been acknowledged. The link then stays
    /// open for at least [`LISTEN_AFTER_SEND`], and the notifications that come in
    /// while it is open are sent as `Event::BleNotification`.
    #[must_use = "a refusal is reported only through the Reply"]
    pub fn send(&self, ble_id: &str, cmd: Command) -> Reply {
        self.link(&what_for(&cmd), ble_id, Op::Send(cmd))
    }

    /// Reads `0xFEF3`, which holds the last state the diffuser reported, and sends it
    /// as `Event::BleNotification`.
    #[must_use = "a refusal is reported only through the Reply"]
    pub fn read_status(&self, ble_id: &str) -> Reply {
        self.link("Bluetooth read status", ble_id, Op::ReadStatus)
    }

    fn link(&self, what: &str, ble_id: &str, op: Op) -> Reply {
        let ble_id = ble_id.to_owned();
        self.queue(what, Task::Link { ble_id, op })
    }

    fn queue(&self, what: &str, task: Task) -> Reply {
        let (done, reply) = Done::new(what);
        let refused = match self.jobs.try_send(Job { task, done }) {
            Ok(()) => return reply,
            Err(TrySendError::Full(_)) => "Bluetooth is busy. Try again in a moment.",
            Err(TrySendError::Closed(_)) => "Bluetooth isn't running.",
        };
        answered(Err(refused.to_owned()))
    }
}

/// Starts the worker on its own thread and returns at once. Every event it sends to
/// `events` is followed by a call to `repaint`.
///
/// Failures reach the caller through the [`Reply`]. A job whose caller dropped the
/// reply before it ran is dropped without a word, as are the jobs behind a link that
/// couldn't be opened whose callers have stopped waiting. Only a job that fails while
/// it runs, with nobody left to hear, becomes an `Event::Failed` (with no device ID).
pub fn spawn(events: Sender<Event>, repaint: impl Fn() + Send + 'static) -> BleHandle {
    let (jobs, queue) = mpsc::channel(QUEUE);
    let reporter = events.clone();
    let emitter = Emitter {
        events,
        repaint: Box::new(repaint),
    };
    let started = std::thread::Builder::new()
        .name("ble".to_owned())
        .spawn(move || run(emitter, queue));
    if let Err(error) = started {
        // No repaint here: the failure is reported before any worker exists.
        let _ = reporter.send(Event::Failed {
            device_id: None,
            what: "Bluetooth".to_owned(),
            error: format!("Couldn't start the Bluetooth thread: {error}"),
            at: unix_now(),
        });
    }
    BleHandle { jobs }
}

struct Emitter {
    events: Sender<Event>,
    repaint: Box<dyn Fn() + Send>,
}

impl Emitter {
    /// Hands the event to the app and wakes it. False when the app has stopped
    /// listening.
    fn emit(&self, event: Event) -> bool {
        let heard = self.events.send(event).is_ok();
        if heard {
            (self.repaint)();
        }
        heard
    }
}

/// Runs the worker until the handle is dropped. All the tasks share this thread, so
/// the state they share needs no lock.
fn run(emitter: Emitter, jobs: mpsc::Receiver<Job>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    match runtime {
        Ok(runtime) => LocalSet::new().block_on(&runtime, serve(emitter, jobs)),
        Err(error) => {
            emitter.emit(Event::Failed {
                device_id: None,
                what: "Bluetooth".to_owned(),
                error: format!("Couldn't start the Bluetooth runtime: {error}"),
                at: unix_now(),
            });
        }
    }
}

async fn serve(emitter: Emitter, mut jobs: mpsc::Receiver<Job>) {
    let (stop_sender, stop) = watch::channel(());
    let shared = Rc::new(Shared {
        emitter,
        adapter_cell: OnceCell::new(),
        scanners: Cell::new(0),
        links: RefCell::new(HashMap::new()),
        stop,
    });
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            job = jobs.recv() => match job {
                Some(job) => shared.dispatch(&mut tasks, job),
                None => break,
            },
            Some(joined) = tasks.join_next(), if !tasks.is_empty() => shared.note_exit(joined),
        }
    }
    // The handle is gone. Closing the stop channel tells every task to wind down; the
    // open links disconnect on the way.
    drop(stop_sender);
    while let Some(joined) = tasks.join_next().await {
        shared.note_exit(joined);
    }
}

/// What all the worker's tasks share.
struct Shared {
    emitter: Emitter,
    /// Set up by the first request that needs Bluetooth.
    adapter_cell: OnceCell<Adapter>,
    /// How many tasks want the adapter to be scanning right now.
    scanners: Cell<usize>,
    /// The queue of each diffuser's link task, by peripheral ID.
    links: RefCell<HashMap<Uuid, mpsc::Sender<LinkJob>>>,
    /// Closes when the worker is told to end.
    stop: watch::Receiver<()>,
}

impl Shared {
    fn dispatch(self: &Rc<Self>, tasks: &mut JoinSet<()>, job: Job) {
        let Job { task, done } = job;
        match task {
            Task::Scan { secs } => {
                let shared = Rc::clone(self);
                tasks.spawn_local(async move {
                    let result = shared.scan(secs).await;
                    done.finish(&shared.emitter, result);
                });
            }
            Task::Link { ble_id, op } => match parse_ble_id(&ble_id) {
                Ok(id) => self.link(tasks, id, LinkJob { op, done }),
                Err(error) => done.finish(&self.emitter, Err(error)),
            },
        }
    }

    /// Hands the job to the diffuser's link, starting one if there is none.
    fn link(self: &Rc<Self>, tasks: &mut JoinSet<()>, id: Uuid, job: LinkJob) {
        let queue = self.links.borrow().get(&id).cloned();
        let job = match queue {
            Some(queue) => match queue.try_send(job) {
                Ok(()) => return,
                Err(TrySendError::Full(job)) => {
                    let error = "Bluetooth is busy with this diffuser. Try again in a moment.";
                    job.done.finish(&self.emitter, Err(error.to_owned()));
                    return;
                }
                // The link task is gone without having removed itself: start a new one.
                Err(TrySendError::Closed(job)) => job,
            },
            None => job,
        };
        let (queue, jobs) = mpsc::channel(QUEUE);
        self.links.borrow_mut().insert(id, queue);
        let link = Link {
            shared: Rc::clone(self),
            id,
            ble_id: ble_id_of(&PeripheralId::from(id)),
            open: None,
        };
        tasks.spawn_local(link.run(job, jobs));
    }

    /// A task panicked (inside btleplug, say). Its reply was dropped unanswered, so
    /// say so here.
    fn note_exit(&self, joined: Result<(), JoinError>) {
        if let Err(error) = joined
            && error.is_panic()
        {
            self.emitter.emit(Event::Failed {
                device_id: None,
                what: "Bluetooth".to_owned(),
                error: "A Bluetooth task stopped unexpectedly.".to_owned(),
                at: unix_now(),
            });
        }
    }

    /// Resolves when the worker is told to end.
    async fn stopped(&self) {
        let mut stop = self.stop.clone();
        // Nothing is ever sent: the channel only closes.
        let _ = stop.changed().await;
    }

    /// The Bluetooth adapter, set up on first use. An error says what the user can do
    /// about it: Bluetooth is off, macOS has not allowed this app to use it, or there
    /// is no adapter.
    async fn adapter(&self) -> Result<Adapter, String> {
        let adapter = self
            .adapter_cell
            .get_or_try_init(|| async {
                let manager = Manager::new().await.map_err(|e| unavailable(&e))?;
                let adapters = manager.adapters().await.map_err(|e| unavailable(&e))?;
                adapters
                    .into_iter()
                    .next()
                    .ok_or_else(|| "This Mac has no Bluetooth adapter.".to_owned())
            })
            .await?
            .clone();
        match adapter.adapter_state().await {
            Ok(CentralState::PoweredOn) => Ok(adapter),
            Ok(CentralState::PoweredOff) => Err("Bluetooth is turned off on this Mac. Turn it \
                on in System Settings, then try again."
                .to_owned()),
            Ok(CentralState::Unknown) => Err("Bluetooth isn't ready on this Mac. If macOS \
                asked, allow Scent Control to use Bluetooth (System Settings, Privacy & \
                Security, Bluetooth)."
                .to_owned()),
            Err(error) => Err(format!("Couldn't check whether Bluetooth is on: {error}")),
        }
    }

    /// Runs `work` with the adapter scanning. Scans can overlap (a scan for the user and
    /// a lookup for a connection), so the adapter stops only when the last one is done.
    async fn scanning<T>(
        &self,
        adapter: &Adapter,
        work: impl Future<Output = T>,
    ) -> Result<T, String> {
        let first = self.scanners.get() == 0;
        self.scanners.set(self.scanners.get() + 1);
        if first && let Err(error) = adapter.start_scan(ScanFilter::default()).await {
            self.scanners.set(self.scanners.get() - 1);
            return Err(format!("Couldn't start scanning for diffusers: {error}"));
        }
        let output = work.await;
        self.scanners.set(self.scanners.get() - 1);
        if self.scanners.get() == 0 {
            let _ = adapter.stop_scan().await;
        }
        Ok(output)
    }

    /// Scans for `secs` seconds and reports the diffusers known by then.
    async fn scan(&self, secs: u64) -> Result<(), String> {
        let adapter = self.adapter().await?;
        let listen = async {
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(secs)) => {}
                () = self.stopped() => {}
            }
        };
        self.scanning(&adapter, listen).await?;

        let known = adapter
            .peripherals()
            .await
            .map_err(|error| format!("Couldn't list the Bluetooth devices: {error}"))?;
        let mut hits = Vec::new();
        for peripheral in known {
            if let Ok(Some(properties)) = peripheral.properties().await
                && let Some(hit) = hit_for(ble_id_of(&peripheral.id()), &properties)
            {
                hits.push(hit);
            }
        }
        hits.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.ble_id.cmp(&b.ble_id)));
        self.emitter.emit(Event::Scanned(hits));
        Ok(())
    }

    /// Finds the peripheral with this ID before `deadline`.
    ///
    /// CoreBluetooth hands out a peripheral only once this process knows it, and
    /// forgets it again when it disconnects. So: the ones CoreBluetooth remembers (by
    /// ID, with no scan) first, then the peripherals already known, and last a scan
    /// that runs until the diffuser advertises.
    ///
    /// Remembered ones come first because a disconnect takes the peripheral out of
    /// CoreBluetooth's list at once but out of btleplug's own list only a moment later.
    /// In that gap the known list holds a dead handle; the retrieval is answered after
    /// the disconnect has been processed, so it never does.
    async fn find(
        &self,
        adapter: &Adapter,
        id: Uuid,
        deadline: tokio::time::Instant,
    ) -> Result<Peripheral, String> {
        let wanted = PeripheralId::from(id);
        let lookup_failed = |error: btleplug::Error| {
            format!("Couldn't look for the diffuser over Bluetooth: {error}")
        };

        let options = RetrievePeripheralsOptions {
            identifiers: Some(vec![wanted.clone()]),
            services: None,
        };
        if let Ok(remembered) = adapter.retrieve_peripherals(options).await
            && let Some(found) = remembered.into_iter().find(|p| p.id() == wanted)
        {
            return Ok(found);
        }

        if let Some(found) = known(adapter, &wanted).await.map_err(lookup_failed)? {
            return Ok(found);
        }

        let appears = async {
            loop {
                tokio::time::sleep(LOOKUP_POLL).await;
                if let Some(found) = known(adapter, &wanted).await? {
                    return Ok::<_, btleplug::Error>(found);
                }
            }
        };
        self.scanning(
            adapter,
            within(deadline, "looking for the diffuser", appears),
        )
        .await?
    }
}

/// The peripheral with this ID, if the adapter knows it.
async fn known(adapter: &Adapter, wanted: &PeripheralId) -> btleplug::Result<Option<Peripheral>> {
    let peripherals = adapter.peripherals().await?;
    Ok(peripherals.into_iter().find(|p| p.id() == *wanted))
}

/// Runs `work`, and gives up on it after `limit`: `None` then.
async fn give_up_after<F: Future>(limit: Duration, work: F) -> Option<F::Output> {
    tokio::time::timeout(limit, work).await.ok()
}

/// Disconnects, for as long as CoreBluetooth takes up to [`DISCONNECT_LIMIT`]. The
/// disconnect is a courtesy that nothing depends on, so it neither fails nor hangs: an
/// unconfirmed one would otherwise hold a link's task, and every job behind it, forever.
async fn disconnect_quietly(peripheral: &Peripheral) {
    let _ = give_up_after(DISCONNECT_LIMIT, peripheral.disconnect()).await;
}

/// Runs a Bluetooth step that must finish before `deadline`. The error says what the
/// step was.
async fn within<T, E: Display>(
    deadline: tokio::time::Instant,
    activity: &str,
    work: impl Future<Output = Result<T, E>>,
) -> Result<T, String> {
    match tokio::time::timeout_at(deadline, work).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(format!("Bluetooth failed while {activity}: {error}")),
        Err(_) => Err(format!("Timed out while {activity}.")),
    }
}

type Notes = Pin<Box<dyn Stream<Item = ValueNotification> + Send>>;
type Events = Pin<Box<dyn Stream<Item = CentralEvent> + Send>>;

/// An open link to a diffuser.
struct Open {
    peripheral: Peripheral,
    chars: Chars,
    /// Everything the diffuser notifies while the link is open.
    notes: Notes,
    /// What the adapter reports from the moment the link was made. Every subscriber
    /// gets every event, so this is how the link learns that the diffuser dropped it.
    events: Events,
    timer: LinkTimer,
}

/// What an open link has to tell us.
#[derive(Debug, PartialEq)]
enum Heard {
    Note(ValueNotification),
    /// The diffuser dropped the link, or a stream ended and with it the means to hear.
    Gone,
}

/// Waits until the diffuser notifies, or its link is gone. Events about other
/// peripherals, and other events about this one, are skipped.
async fn next_heard(notes: &mut Notes, events: &mut Events, me: &PeripheralId) -> Heard {
    loop {
        tokio::select! {
            note = notes.next() => return note.map_or(Heard::Gone, Heard::Note),
            event = events.next() => match event {
                Some(CentralEvent::DeviceDisconnected(gone)) if gone == *me => {
                    return Heard::Gone;
                }
                Some(_) => {}
                None => return Heard::Gone,
            },
        }
    }
}

/// [`next_heard`] for the link in `open`; never ready when there is no link.
async fn next_heard_on(open: &mut Option<Open>, me: &PeripheralId) -> Heard {
    match open {
        Some(open) => next_heard(&mut open.notes, &mut open.events, me).await,
        None => std::future::pending().await,
    }
}

/// Why a link could not be opened.
#[derive(Debug, PartialEq)]
enum OpenError {
    /// Not found, or it would not connect in time. Another app holding the diffuser is
    /// the usual cause.
    Unreachable(String),
    /// Connected, but it is not a diffuser we can talk to.
    NotADiffuser(String),
}

impl OpenError {
    /// The text for the user. Only an unreachable diffuser gets the hint about the
    /// other app; for a device that isn't a diffuser it would be wrong.
    fn message(self) -> String {
        match self {
            OpenError::Unreachable(error) => format!("{error} {CONNECT_HINT}"),
            OpenError::NotADiffuser(error) => error,
        }
    }
}

impl Open {
    /// False once the diffuser has dropped the link (it restarts after some commands).
    async fn is_alive(&self) -> bool {
        self.peripheral.is_connected().await.unwrap_or(false)
    }

    /// Does `op` on the link. Everything it sends to the app is tagged with `ble_id`.
    async fn perform(&self, op: &Op, ble_id: &str, emitter: &Emitter) -> Result<(), String> {
        match op {
            Op::ReadInfo => {
                let bytes = self
                    .peripheral
                    .read(&self.chars.info)
                    .await
                    .map_err(|error| format!("Couldn't read the diffuser's info: {error}"))?;
                let info = parse_info(&String::from_utf8_lossy(&bytes)).map_err(|error| {
                    format!("The diffuser's info wasn't what we expected: {error}")
                })?;
                emitter.emit(Event::BleInfo {
                    ble_id: ble_id.to_owned(),
                    info,
                    at: unix_now(),
                });
            }
            Op::ReadStatus => {
                let bytes = self
                    .peripheral
                    .read(&self.chars.notify)
                    .await
                    .map_err(|error| format!("Couldn't read the diffuser's status: {error}"))?;
                emitter.emit(Event::BleNotification {
                    ble_id: ble_id.to_owned(),
                    bytes,
                    at: unix_now(),
                });
            }
            Op::Send(cmd) => {
                // One write at a time, each acknowledged, so the chunks arrive in order.
                for part in writes_for(cmd) {
                    self.peripheral
                        .write(&self.chars.write, &part, WriteType::WithResponse)
                        .await
                        .map_err(|error| format!("Couldn't write to the diffuser: {error}"))?;
                }
            }
        }
        Ok(())
    }

    async fn close(self) {
        // Disconnecting a peripheral that is not connected is never confirmed, so only
        // ask when it is connected: this is always answered, and tells a link that the
        // diffuser dropped from one that is still up.
        if self.is_alive().await {
            disconnect_quietly(&self.peripheral).await;
        }
    }
}

/// Finds the diffuser and opens a link to it. The clock starts once the adapter is
/// ready, because the adapter waits for the user to answer the permission prompt.
async fn connect(shared: &Shared, id: Uuid) -> Result<Open, String> {
    let adapter = shared.adapter().await?;
    let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;
    open_link(shared, &adapter, id, deadline)
        .await
        .map_err(OpenError::message)
}

async fn open_link(
    shared: &Shared,
    adapter: &Adapter,
    id: Uuid,
    deadline: tokio::time::Instant,
) -> Result<Open, OpenError> {
    let peripheral = shared
        .find(adapter, id, deadline)
        .await
        .map_err(OpenError::Unreachable)?;
    let events = adapter.events().await.map_err(|error| {
        OpenError::Unreachable(format!("Bluetooth failed while listening: {error}"))
    })?;
    match handshake(&peripheral, deadline).await {
        Ok((chars, notes)) => Ok(Open {
            peripheral,
            chars,
            notes,
            events,
            timer: LinkTimer::new(Instant::now()),
        }),
        Err(error) => {
            // Also cancels a connection that is still being made.
            disconnect_quietly(&peripheral).await;
            Err(error)
        }
    }
}

/// Connects, finds the characteristics and subscribes to the notifications.
async fn handshake(
    peripheral: &Peripheral,
    deadline: tokio::time::Instant,
) -> Result<(Chars, Notes), OpenError> {
    within(deadline, "connecting", peripheral.connect())
        .await
        .map_err(OpenError::Unreachable)?;
    within(
        deadline,
        "reading the services",
        peripheral.discover_services(),
    )
    .await
    .map_err(OpenError::Unreachable)?;
    let chars = Chars::find(&peripheral.characteristics()).map_err(OpenError::NotADiffuser)?;
    // Listen before subscribing, so that the first notification is not missed.
    let notes = peripheral.notifications().await.map_err(|error| {
        OpenError::Unreachable(format!("Bluetooth failed while listening: {error}"))
    })?;
    within(
        deadline,
        "turning on notifications",
        peripheral.subscribe(&chars.notify),
    )
    .await
    .map_err(OpenError::Unreachable)?;
    Ok((chars, notes))
}

/// The open link in `slot`, or a new one if there is none or the diffuser dropped it.
async fn ensure_open<'a>(
    slot: &'a mut Option<Open>,
    shared: &Shared,
    id: Uuid,
) -> Result<&'a mut Open, String> {
    let open = match slot.take() {
        Some(open) if open.is_alive().await => open,
        _ => connect(shared, id).await?,
    };
    Ok(slot.insert(open))
}

/// The task that owns one diffuser's connection. It takes that diffuser's jobs one at
/// a time, so writes are never interleaved, and closes the link when it has sat idle.
struct Link {
    shared: Rc<Shared>,
    id: Uuid,
    /// `id` as the app knows it.
    ble_id: String,
    open: Option<Open>,
}

impl Link {
    async fn run(mut self, first: LinkJob, mut jobs: mpsc::Receiver<LinkJob>) {
        let mut next = Some(first);
        loop {
            let job = match next.take() {
                Some(job) => job,
                None => match self.wait(&mut jobs).await {
                    Some(job) => job,
                    None => break,
                },
            };
            if !self.serve(job, &mut jobs).await {
                break;
            }
        }
        // Nothing is awaited between the last look at the queue and here, so no job
        // can be queued to a link that has ended.
        self.shared.links.borrow_mut().remove(&self.id);
    }

    /// Does one job. False when the link could not be opened, which also fails the
    /// jobs queued behind this one.
    async fn serve(&mut self, job: LinkJob, jobs: &mut mpsc::Receiver<LinkJob>) -> bool {
        let LinkJob { op, done } = job;
        // Skipped quietly if the caller gave up already: it has reported its own
        // timeout, and doing this now could undo something newer.
        if done.is_abandoned() {
            return true;
        }
        let open = match ensure_open(&mut self.open, &self.shared, self.id).await {
            Ok(open) => open,
            Err(error) => {
                // None of these ran. Those whose callers gave up say nothing.
                done.finish_unless_abandoned(&self.shared.emitter, Err(error.clone()));
                while let Ok(queued) = jobs.try_recv() {
                    queued
                        .done
                        .finish_unless_abandoned(&self.shared.emitter, Err(error.clone()));
                }
                return false;
            }
        };

        // Connecting can take long enough for the caller to give up. Look again before
        // anything is written or read.
        if done.is_abandoned() {
            return true;
        }
        let work = open.perform(&op, &self.ble_id, &self.shared.emitter);
        let result = match tokio::time::timeout(IO_TIMEOUT, work).await {
            Ok(result) => result,
            Err(_) => Err(format!(
                "The diffuser didn't answer within {} s.",
                IO_TIMEOUT.as_secs()
            )),
        };
        if result.is_ok() {
            let now = Instant::now();
            match op {
                Op::Send(_) => open.timer.sent(now),
                Op::ReadInfo | Op::ReadStatus => open.timer.touch(now),
            }
        }
        let failed = result.is_err();
        done.finish(&self.shared.emitter, result);
        if failed {
            // Where the link stands is unknown; the next job opens a fresh one.
            self.close().await;
        }
        true
    }

    /// Waits for the next job while forwarding the diffuser's notifications. Closes the
    /// link when it has been idle for too long or the worker is ending, and returns
    /// `None` then unless a job turned up in the meantime.
    async fn wait(&mut self, jobs: &mut mpsc::Receiver<LinkJob>) -> Option<LinkJob> {
        loop {
            let Some(open) = &self.open else {
                // The link is closed and nothing is queued: this task is done.
                return jobs.try_recv().ok();
            };
            let due = tokio::time::Instant::from_std(open.timer.deadline());
            let me = PeripheralId::from(self.id);
            // Biased, so that once the worker is told to end no job can start a new
            // connection.
            tokio::select! {
                biased;
                () = self.shared.stopped() => {
                    self.close().await;
                    return None;
                }
                job = jobs.recv() => {
                    if job.is_none() {
                        self.close().await;
                    }
                    return job;
                }
                heard = next_heard_on(&mut self.open, &me) => match heard {
                    Heard::Note(note) => self.forward(&note),
                    // The diffuser dropped the link (a restart, say). CoreBluetooth has
                    // let go of it, so closing finds it disconnected and asks for nothing.
                    Heard::Gone => self.close().await,
                },
                () = tokio::time::sleep_until(due) => {
                    let idle = self.open.as_ref().is_some_and(|open| open.timer.should_disconnect(Instant::now()));
                    if idle {
                        self.close().await;
                        // Anything queued while it closed gets a fresh link.
                        return jobs.try_recv().ok();
                    }
                }
            }
        }
    }

    fn forward(&self, note: &ValueNotification) {
        if note.uuid == STATUS_CHAR {
            self.shared.emitter.emit(Event::BleNotification {
                ble_id: self.ble_id.clone(),
                bytes: note.value.clone(),
                at: unix_now(),
            });
        }
    }

    async fn close(&mut self) {
        if let Some(open) = self.open.take() {
            open.close().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use btleplug::api::{CentralEvent, CharPropFlags, Characteristic, PeripheralProperties};
    use btleplug::platform::PeripheralId;
    use scent_core::{Command, Level, Secret};
    use tokio::sync::{mpsc, oneshot};
    use uuid::Uuid;

    use super::*;
    use crate::state::Event;

    #[test]
    fn idle_link_is_dropped_after_10_s() {
        let t0 = Instant::now();
        let mut timer = LinkTimer::new(t0);
        timer.touch(t0);
        assert!(!timer.should_disconnect(t0 + Duration::from_secs(9)));
        assert!(timer.should_disconnect(t0 + Duration::from_secs(10)));

        // Touching again at t0 + 9 s defers the disconnect by another 10 s.
        timer.touch(t0 + Duration::from_secs(9));
        assert!(!timer.should_disconnect(t0 + Duration::from_secs(10)));
        assert!(!timer.should_disconnect(t0 + Duration::from_secs(18)));
        assert!(timer.should_disconnect(t0 + Duration::from_secs(19)));
    }

    #[test]
    fn a_send_keeps_the_link_open_for_the_listen_window() {
        let t0 = Instant::now();
        let mut timer = LinkTimer::new(t0);
        timer.sent(t0);
        assert!(!timer.should_disconnect(t0 + LISTEN_AFTER_SEND));
        assert_eq!(timer.deadline(), t0 + IDLE_AFTER.max(LISTEN_AFTER_SEND));

        // Neither a late-arriving older touch nor a send can bring the deadline forward.
        let later = t0 + Duration::from_secs(9);
        timer.touch(later);
        timer.sent(t0);
        assert_eq!(timer.deadline(), later + IDLE_AFTER);
    }

    #[test]
    fn frames_are_written_in_order_in_177_byte_chunks() {
        let command = Command::SaveMqtt {
            broker: format!("wss://{}/mqtt", "a".repeat(300)),
            username: "u".into(),
            password: Secret::new("p".into()),
        };
        let writes = writes_for(&command);
        assert!(writes.len() > 1, "a long frame is split");
        assert!(writes.iter().all(|part| part.len() <= 177));
        assert_eq!(writes.concat(), command.ble_frame());

        // A short frame goes out in one write.
        let short = Command::SetLevel(Level::new(2).unwrap());
        assert_eq!(writes_for(&short), vec![short.ble_frame()]);
    }

    #[test]
    fn the_gatt_ids_are_the_scentiment_ones() {
        assert_eq!(SERVICE.to_string(), "00000180-0000-1000-8000-00805f9b34fb");
        assert_eq!(
            WRITE_CHAR.to_string(),
            "0000dead-0000-1000-8000-00805f9b34fb"
        );
        assert_eq!(
            STATUS_CHAR.to_string(),
            "0000fef3-0000-1000-8000-00805f9b34fb"
        );
        assert_eq!(
            INFO_CHAR.to_string(),
            "0000fef4-0000-1000-8000-00805f9b34fb"
        );
    }

    #[test]
    fn ble_ids_are_lowercase_hyphenated_uuids_and_match_in_any_case() {
        let lower = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";
        let id = parse_ble_id(lower).unwrap();
        assert_eq!(ble_id_of(&PeripheralId::from(id)), lower);
        // What the worker reports is what it accepts back, in whatever case it is typed.
        assert_eq!(parse_ble_id(&lower.to_uppercase()), Ok(id));
        assert_eq!(parse_ble_id(&format!(" {lower}\n")), Ok(id));
        for bad in ["", "Scentiment-D B 1A2B", "TESTDEVICE01", "0f1e2d3c"] {
            let error = parse_ble_id(bad).unwrap_err();
            assert!(error.contains("Bluetooth ID"), "{bad:?}: {error}");
        }
    }

    fn characteristic(service: Uuid, uuid: Uuid) -> Characteristic {
        Characteristic {
            uuid,
            service_uuid: service,
            properties: CharPropFlags::READ,
            descriptors: BTreeSet::new(),
        }
    }

    #[test]
    fn characteristics_are_found_by_service_and_uuid() {
        let other_service = Uuid::from_u128(7);
        let mut found = BTreeSet::from([
            characteristic(SERVICE, WRITE_CHAR),
            characteristic(SERVICE, STATUS_CHAR),
            // The same UUID in another service is not ours.
            characteristic(other_service, INFO_CHAR),
        ]);
        let error = Chars::find(&found).unwrap_err();
        assert!(error.contains("0xFEF4"), "{error}");
        assert!(!error.contains("0xFEF3"), "{error}");

        found.insert(characteristic(SERVICE, INFO_CHAR));
        let chars = Chars::find(&found).unwrap();
        assert_eq!(chars.write.uuid, WRITE_CHAR);
        assert_eq!(chars.notify.uuid, STATUS_CHAR);
        assert_eq!(chars.info.uuid, INFO_CHAR);
        assert_eq!(chars.info.service_uuid, SERVICE);

        let error = Chars::find(&BTreeSet::new()).unwrap_err();
        assert!(error.contains("0xDEAD"), "{error}");
    }

    #[test]
    fn only_scentiment_names_become_scan_hits() {
        let named = |local: Option<&str>, advertised: Option<&str>| PeripheralProperties {
            local_name: local.map(str::to_owned),
            advertisement_name: advertised.map(str::to_owned),
            rssi: Some(-61),
            ..PeripheralProperties::default()
        };
        let hit = hit_for("id-1".into(), &named(Some("Scentiment-D B 1A2B"), None)).unwrap();
        assert_eq!(hit.ble_id, "id-1");
        assert_eq!(hit.name, "Scentiment-D B 1A2B");
        assert_eq!(hit.rssi, Some(-61));

        // The advertised name is used when the other one is missing.
        let hit = hit_for("id-2".into(), &named(None, Some("scentiment-h w"))).unwrap();
        assert_eq!(hit.name, "scentiment-h w");

        assert_eq!(
            hit_for("id-3".into(), &named(Some("JBL Flip 6"), None)),
            None
        );
        assert_eq!(hit_for("id-4".into(), &named(None, None)), None);
    }

    #[test]
    fn failures_name_the_action_and_never_the_payload() {
        let wifi = Command::WifiConnect {
            ssid: "ExampleWiFi".into(),
            password: Secret::new("hunter2!".into()),
        };
        let what = what_for(&wifi);
        assert_eq!(what, "Bluetooth send WIFI_CONNECT");
        assert!(!what.contains("hunter2!") && !what.contains("ExampleWiFi"));
        assert_eq!(
            what_for(&Command::SetLevel(Level::new(2).unwrap())),
            "Bluetooth send SET_LEVEL"
        );
        // A restart over Bluetooth is a SECRET_COMMAND, and that is the name it gets.
        assert_eq!(
            what_for(&Command::SoftRestart),
            "Bluetooth send SECRET_COMMAND"
        );
    }

    fn emitter(events: std::sync::mpsc::Sender<Event>, repaints: &Arc<AtomicUsize>) -> Emitter {
        let repaints = Arc::clone(repaints);
        Emitter {
            events,
            repaint: Box::new(move || {
                repaints.fetch_add(1, Ordering::SeqCst);
            }),
        }
    }

    #[test]
    fn a_failure_goes_to_the_reply_and_only_to_the_reply_when_someone_waits() {
        let (events, received) = std::sync::mpsc::channel();
        let repaints = Arc::new(AtomicUsize::new(0));
        let emitter = emitter(events, &repaints);

        let (done, mut reply) = Done::new("Bluetooth scan");
        done.finish(&emitter, Err("no adapter".into()));
        assert_eq!(reply.try_recv().unwrap(), Err("no adapter".to_owned()));

        let (done, mut reply) = Done::new("Bluetooth scan");
        done.finish(&emitter, Ok(()));
        assert_eq!(reply.try_recv().unwrap(), Ok(()));

        assert!(
            received.try_recv().is_err(),
            "nothing for the event channel"
        );
        assert_eq!(repaints.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_failure_nobody_waits_for_becomes_a_failed_event() {
        let (events, received) = std::sync::mpsc::channel();
        let repaints = Arc::new(AtomicUsize::new(0));
        let emitter = emitter(events, &repaints);

        let (done, reply) = Done::new("Bluetooth send SET_LEVEL");
        drop(reply);
        done.finish(&emitter, Err("The diffuser didn't answer.".into()));
        match received.try_recv().unwrap() {
            Event::Failed {
                device_id,
                what,
                error,
                at,
            } => {
                assert_eq!(device_id, None);
                assert_eq!(what, "Bluetooth send SET_LEVEL");
                assert_eq!(error, "The diffuser didn't answer.");
                assert!(at > 0);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(repaints.load(Ordering::SeqCst), 1);

        // A success nobody waits for says nothing.
        let (done, reply) = Done::new("Bluetooth scan");
        drop(reply);
        done.finish(&emitter, Ok(()));
        assert!(received.try_recv().is_err());
    }

    // A job that never ran, because its link couldn't be opened (the job itself, or one
    // queued behind it), says nothing when its caller has stopped waiting: the caller
    // reported its own timeout already. `serve` has no seam to run against without a
    // peripheral, so this is the call it makes for those jobs.
    #[test]
    fn an_abandoned_job_that_never_ran_is_dropped_without_a_word() {
        let (events, received) = std::sync::mpsc::channel();
        let repaints = Arc::new(AtomicUsize::new(0));
        let emitter = emitter(events, &repaints);

        let (done, reply) = Done::new("Bluetooth send SAVE_MQTT");
        drop(reply);
        done.finish_unless_abandoned(&emitter, Err("Timed out while connecting.".into()));
        assert!(received.try_recv().is_err(), "no Failed event");
        assert_eq!(repaints.load(Ordering::SeqCst), 0);

        // A caller that still waits hears why.
        let (done, mut reply) = Done::new("Bluetooth send SAVE_MQTT");
        done.finish_unless_abandoned(&emitter, Err("Timed out while connecting.".into()));
        assert_eq!(
            reply.try_recv().unwrap(),
            Err("Timed out while connecting.".to_owned())
        );
        assert!(received.try_recv().is_err());
    }

    // `serve` has no seam to run against without a peripheral, so the decision is a
    // predicate on the job's `Done`, and this is what it turns on.
    #[test]
    fn a_job_whose_caller_stopped_waiting_is_not_wanted() {
        let (done, reply) = Done::new("Bluetooth send TURN_ON");
        assert!(!done.is_abandoned(), "someone is waiting for the answer");
        drop(reply);
        assert!(done.is_abandoned(), "the caller gave up on it");
    }

    fn handle_with_room_for(n: usize) -> (BleHandle, mpsc::Receiver<Job>) {
        let (jobs, queue) = mpsc::channel(n);
        (BleHandle { jobs }, queue)
    }

    #[test]
    fn a_worker_that_is_gone_answers_with_an_error() {
        let (handle, queue) = handle_with_room_for(4);
        drop(queue);
        for mut reply in [
            handle.scan(1),
            handle.read_info("0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0"),
            handle.send("0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0", Command::GetStatus),
            handle.read_status("0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0"),
        ] {
            let error = reply.try_recv().unwrap().unwrap_err();
            assert!(error.contains("isn't running"), "{error}");
        }
    }

    #[test]
    fn a_full_queue_answers_busy_instead_of_waiting() {
        let (handle, _queue) = handle_with_room_for(1);
        let mut first = handle.scan(1);
        assert!(first.try_recv().is_err(), "queued, not answered yet");
        let mut second = handle.scan(1);
        let error = second.try_recv().unwrap().unwrap_err();
        assert!(error.contains("busy"), "{error}");
    }

    /// Waits for a reply the way the hub would, but gives up after 5 s.
    fn answer(mut reply: Reply) -> Result<(), String> {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            match reply.try_recv() {
                Ok(result) => return result,
                Err(oneshot::error::TryRecvError::Empty) if Instant::now() < until => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("no answer: {error}"),
            }
        }
    }

    #[test]
    fn a_bad_id_is_answered_by_the_worker_without_touching_bluetooth() {
        let (events, received) = std::sync::mpsc::channel();
        let handle = spawn(events, || {});
        // None of these may reach CoreBluetooth: it would kill a test run outside the
        // app bundle.
        for reply in [
            handle.read_info("TESTDEVICE01"),
            handle.read_status("not an id"),
            handle.send("", Command::GetStatus),
        ] {
            let error = answer(reply).unwrap_err();
            assert!(error.contains("Bluetooth ID"), "{error}");
        }
        // The caller heard about each one, so the event channel stays quiet.
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn dropping_the_handle_stops_the_worker() {
        let (events, received) = std::sync::mpsc::channel();
        let handle = spawn(events, || {});
        drop(handle);
        // The worker ends at once, which closes the event channel. It never asked for
        // the adapter, so Bluetooth was not touched.
        let closed = received.recv_timeout(Duration::from_secs(2));
        assert_eq!(closed, Err(std::sync::mpsc::RecvTimeoutError::Disconnected));
    }

    #[test]
    fn steps_that_fail_or_time_out_say_what_they_were_doing() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let soon = tokio::time::Instant::now() + Duration::from_millis(20);
            let ok = within(soon, "connecting", async { Ok::<_, btleplug::Error>(7) }).await;
            assert_eq!(ok, Ok(7));

            let failed = within(soon, "connecting", async {
                Err::<(), _>(btleplug::Error::NotConnected)
            })
            .await
            .unwrap_err();
            assert!(
                failed.contains("connecting") && failed.contains("Not connected"),
                "{failed}"
            );

            let timed_out = within(
                soon,
                "reading its services",
                std::future::pending::<btleplug::Result<()>>(),
            )
            .await
            .unwrap_err();
            // It states no number: the time allowed depends on the deadline it was given.
            assert_eq!(timed_out, "Timed out while reading its services.");
        });
    }

    #[tokio::test(start_paused = true)]
    async fn a_step_that_never_finishes_is_given_up_on_at_the_limit() {
        let started = tokio::time::Instant::now();
        let outcome = give_up_after(Duration::from_secs(3), std::future::pending::<()>()).await;
        assert_eq!(outcome, None);
        assert_eq!(started.elapsed(), Duration::from_secs(3));
    }

    #[tokio::test(start_paused = true)]
    async fn a_step_that_finishes_is_not_cut_short() {
        let started = tokio::time::Instant::now();
        let outcome = give_up_after(Duration::from_secs(3), async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            5
        })
        .await;
        assert_eq!(outcome, Some(5));
        assert_eq!(started.elapsed(), Duration::from_secs(1));
    }

    #[test]
    fn a_disconnect_is_waited_for_3_s_at_most() {
        assert_eq!(DISCONNECT_LIMIT, Duration::from_secs(3));
    }

    #[test]
    fn only_errors_a_busy_diffuser_can_cause_carry_the_hint() {
        let hinted = OpenError::Unreachable("Timed out while connecting.".into()).message();
        assert!(
            hinted.starts_with("Timed out while connecting. "),
            "{hinted}"
        );
        assert!(hinted.contains("Scentiment app"), "{hinted}");

        // "Close the Scentiment app" is no help when the device is not a diffuser.
        let plain = OpenError::NotADiffuser("The device has no characteristic.".into()).message();
        assert_eq!(plain, "The device has no characteristic.");
    }

    #[test]
    fn a_refused_permission_says_where_to_allow_it() {
        let denied = unavailable(&btleplug::Error::PermissionDenied);
        assert!(
            denied.contains("System Settings") && denied.contains("Privacy & Security"),
            "{denied}"
        );
        let other = unavailable(&btleplug::Error::NoAdapterAvailable);
        assert!(other.starts_with("Bluetooth isn't available"), "{other}");
        assert!(!other.contains("System Settings"), "{other}");
    }

    fn me() -> PeripheralId {
        PeripheralId::from(Uuid::from_u128(1))
    }

    fn note(value: &[u8]) -> ValueNotification {
        ValueNotification {
            uuid: STATUS_CHAR,
            service_uuid: SERVICE,
            value: value.to_vec(),
        }
    }

    /// Streams that say `items` and then go quiet, like a link with nothing more to say.
    fn then_quiet<T: Send + 'static>(items: Vec<T>) -> Pin<Box<dyn Stream<Item = T> + Send>> {
        Box::pin(futures::stream::iter(items).chain(futures::stream::pending()))
    }

    fn ended<T: Send + 'static>() -> Pin<Box<dyn Stream<Item = T> + Send>> {
        Box::pin(futures::stream::empty())
    }

    #[tokio::test]
    async fn a_notification_is_heard() {
        let mut notes = then_quiet(vec![note(b"led:50")]);
        let mut events = then_quiet(vec![]);
        let heard = next_heard(&mut notes, &mut events, &me()).await;
        assert_eq!(heard, Heard::Note(note(b"led:50")));
    }

    #[tokio::test]
    async fn the_diffusers_own_disconnect_is_heard_and_other_events_are_not() {
        let other = PeripheralId::from(Uuid::from_u128(2));
        let mut notes = then_quiet(vec![]);
        let mut events = then_quiet(vec![
            CentralEvent::DeviceUpdated(me()),
            CentralEvent::DeviceDisconnected(other.clone()),
            CentralEvent::DeviceDisconnected(me()),
        ]);
        assert_eq!(
            next_heard(&mut notes, &mut events, &me()).await,
            Heard::Gone
        );

        // Somebody else's disconnect is skipped, and a notification still comes through.
        let mut notes = then_quiet(vec![note(b"S:0")]);
        let mut events = then_quiet(vec![CentralEvent::DeviceDisconnected(other)]);
        assert_eq!(
            next_heard(&mut notes, &mut events, &me()).await,
            Heard::Note(note(b"S:0"))
        );
    }

    #[tokio::test]
    async fn a_stream_that_ends_means_the_link_is_gone() {
        let mut events = then_quiet(vec![]);
        assert_eq!(
            next_heard(&mut ended(), &mut events, &me()).await,
            Heard::Gone
        );
        let mut notes = then_quiet(vec![]);
        assert_eq!(
            next_heard(&mut notes, &mut ended(), &me()).await,
            Heard::Gone
        );
    }
}
