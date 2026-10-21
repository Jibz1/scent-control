//! The MQTT worker: one connection to the broker over `wss`, the subscription to
//! everything under the Scentiment prefix, and the [`Event`]s that come out of it.
//!
//! The worker runs on its own thread. It reports through the same channel as the
//! other workers and never touches the screens. [`MqttHandle::send`] queues a command
//! without waiting for the network.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use rumqttc::{
    AsyncClient, ConnectReturnCode, ConnectionError, EventLoop, MqttOptions, Packet, QoS,
    Subscribe, Transport,
};
use scent_core::{Command, SUBSCRIBE_ALL, Secret, device_topic, valid_device_id};
use tokio::sync::oneshot;

use crate::state::{Event, unix_now};

/// The broker keeps the connection alive with a ping this often.
const KEEP_ALIVE: Duration = Duration::from_secs(30);

/// How long the worker waits before it tries the broker again.
const RETRY_AFTER: Duration = Duration::from_secs(5);

/// How many commands can wait for the worker.
const QUEUE: usize = 16;

/// What the app is told when the worker's thread dies in a panic. Nothing restarts it.
const STOPPED_UNEXPECTEDLY: &str = "The broker connection stopped unexpectedly. Restart the app.";

/// The largest MQTT packet the connection writes or reads, both ways. rumqttc's default
/// is 10 KiB, and a packet over it is not sent: the connection drops and the message is
/// lost, though `send` has already said it was queued. A full schedule on firmware 2.30
/// (99 times and the always-on entry, about 130 bytes each) is about 13 KB. Incoming has
/// to be as large: we hear our own commands back on the broker, and the diffuser's
/// acknowledgement repeats the whole command as an escaped string. 256 KiB leaves
/// ample room for both and still bounds what a bad message can make us hold.
const MAX_PACKET_SIZE: usize = 256 * 1024;

/// Where the broker is and who we are on it. The password is not part of this: it
/// lives in the Keychain (see `secrets`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerSettings {
    /// `wss://host/path`. Encrypted only.
    pub url: String,
    pub username: String,
    /// The `userid` the app puts in its `PING`s.
    pub route_id: String,
}

impl BrokerSettings {
    /// The settings from the values a build was given: each one trimmed, and empty when
    /// there was none.
    pub fn from_values(
        url: Option<&str>,
        username: Option<&str>,
        route_id: Option<&str>,
    ) -> BrokerSettings {
        let value = |value: Option<&str>| value.unwrap_or_default().trim().to_owned();
        BrokerSettings {
            url: value(url),
            username: value(username),
            route_id: value(route_id),
        }
    }

    /// Whether there is a broker at all. An empty address means none: a build that was
    /// given none, with nothing saved, or a saved file that holds none.
    pub fn is_set(&self) -> bool {
        !self.url.trim().is_empty()
    }

    /// A broker for the tests: an example host, and the synthetic route ID.
    #[cfg(test)]
    pub fn example() -> BrokerSettings {
        BrokerSettings {
            url: "wss://broker.example.test/mqtt".to_owned(),
            username: "example-user".to_owned(),
            route_id: "route-test-1".to_owned(),
        }
    }
}

/// The broker a build was given, for when nothing is saved: `SCENT_BROKER_URL`,
/// `SCENT_BROKER_USERNAME` and `SCENT_ROUTE_ID` at build time, which
/// `scripts/bundle-macos.sh` takes from the repo-root `.env`. Each is empty when the build
/// had none, and then there is no broker until one is added in Settings. Never a password:
/// that is in the Keychain.
impl Default for BrokerSettings {
    fn default() -> Self {
        BrokerSettings::from_values(
            option_env!("SCENT_BROKER_URL"),
            option_env!("SCENT_BROKER_USERNAME"),
            option_env!("SCENT_ROUTE_ID"),
        )
    }
}

/// A broker address taken apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerUrl {
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl BrokerUrl {
    /// Reads `wss://host[:port][/path]`. The port is 443 when left out and the path
    /// is `/`. Any other scheme is an error: the app only talks to the broker
    /// encrypted. So is a user name or password in the address, which would leak
    /// into logs.
    pub fn parse(url: &str) -> Result<BrokerUrl, String> {
        let rest = url.strip_prefix("wss://").ok_or_else(|| {
            format!("The broker address must start with wss:// (encrypted only): {url:?}")
        })?;
        let (authority, path) = match rest.find('/') {
            Some(at) => rest.split_at(at),
            None => (rest, "/"),
        };
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => {
                let port = port
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port != 0)
                    .ok_or_else(|| format!("The broker address has a bad port: {url:?}"))?;
                (host, port)
            }
            None => (authority, 443),
        };
        let host_ok = !host.is_empty()
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.');
        if !host_ok {
            return Err(format!("The broker address has a bad host name: {url:?}"));
        }
        Ok(BrokerUrl {
            host: host.to_owned(),
            port,
            path: path.to_owned(),
        })
    }
}

/// Turns a refusal of the login into the event that says so. `None` for every other
/// code: `Success` is not a refusal, and the rest (such as "service unavailable")
/// say nothing about the credentials, so they are retried like any outage.
pub fn map_connack(code: ConnectReturnCode) -> Option<Event> {
    let reason = match code {
        ConnectReturnCode::NotAuthorized => "The broker says this user is not authorized.",
        ConnectReturnCode::BadUserNamePassword => "The broker refused the username or password.",
        _ => return None,
    };
    Some(Event::BrokerLoginRefused(reason.to_owned()))
}

/// What a failed connection or a lost one becomes. rumqttc reports a refused
/// connect as an error, not as a ConnAck.
fn event_for_error(error: &ConnectionError) -> Event {
    match error {
        ConnectionError::ConnectionRefused(code) => map_connack(*code),
        _ => None,
    }
    .unwrap_or_else(|| Event::BrokerDisconnected(error.to_string()))
}

/// Whether, and when, to try the broker again after an event.
pub struct RetryPolicy;

impl RetryPolicy {
    /// `None` after a refused login (trying again with the same password only
    /// repeats the refusal) and after anything that isn't a disconnect; 5 s after a
    /// disconnect.
    pub fn after(event: &Event) -> Option<Duration> {
        match event {
            Event::BrokerDisconnected(_) => Some(RETRY_AFTER),
            _ => None,
        }
    }
}

/// How to queue commands for the broker. Dropping it stops the worker.
pub struct MqttHandle {
    /// `None` when the worker never started (a bad broker address).
    client: Option<AsyncClient>,
    /// True from the ConnAck until the connection drops or the worker's thread ends
    /// (see [`WorkerExit`]).
    connected: Arc<AtomicBool>,
    /// The worker ends when this goes away.
    _stop: Option<oneshot::Sender<()>>,
}

impl MqttHandle {
    /// Whether the broker is connected right now: true from the ConnAck until the
    /// connection drops or the worker stops, and never true for a worker that did not
    /// start.
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    /// Queues `cmd` for the diffuser, as a QoS 1 message on its device topic, and
    /// returns without waiting for the network. It refuses a bad device ID, and
    /// refuses while the broker isn't connected so that nothing waits in the queue
    /// to go out minutes later, when the user no longer expects it. Once the worker
    /// has stopped, even in a panic, it is never connected again.
    pub fn send(&self, device_id: &str, cmd: &Command) -> Result<(), String> {
        if !valid_device_id(device_id) {
            return Err(format!("{device_id:?} is not a valid device ID."));
        }
        let client = match &self.client {
            Some(client) if self.connected.load(Ordering::SeqCst) => client,
            _ => return Err("Not connected to the broker.".to_owned()),
        };
        client
            .try_publish(
                device_topic(device_id),
                QoS::AtLeastOnce,
                false,
                cmd.mqtt_body(),
            )
            .map_err(|_| "The broker connection is too busy to take this message.".to_owned())
    }
}

/// Starts the worker on its own thread and returns at once. Events (`BrokerConnected`,
/// `BrokerLoginRefused`, `BrokerDisconnected` and one `Mqtt` per message on the
/// broker) go to `events`, each followed by a call to `repaint`.
///
/// The worker subscribes to everything under the Scentiment prefix on every ConnAck.
/// A refused login is reported once and the worker stops; start a new one with the
/// corrected password. Any other failure is reported and retried every 5 s.
pub fn spawn(
    settings: BrokerSettings,
    password: Secret,
    events: Sender<Event>,
    repaint: impl Fn() + Send + 'static,
) -> MqttHandle {
    let connected = Arc::new(AtomicBool::new(false));
    let unstarted = |error: String| {
        // No repaint here: the failure is reported before any worker exists.
        let _ = events.send(Event::BrokerDisconnected(error));
        MqttHandle {
            client: None,
            connected: Arc::clone(&connected),
            _stop: None,
        }
    };

    let options = match build_options(&settings, &password) {
        Ok(options) => options,
        Err(error) => return unstarted(error),
    };
    let (client, eventloop) = AsyncClient::new(options, QUEUE);
    let (stop, stopped) = oneshot::channel();
    let worker = Worker {
        connected: Arc::clone(&connected),
        events: events.clone(),
        repaint: Box::new(repaint),
    };
    let started = std::thread::Builder::new()
        .name("mqtt".to_owned())
        .spawn(move || worker.run(eventloop, stopped));
    if let Err(error) = started {
        return unstarted(format!("Couldn't start the MQTT thread: {error}"));
    }
    MqttHandle {
        client: Some(client),
        connected,
        _stop: Some(stop),
    }
}

fn build_options(settings: &BrokerSettings, password: &Secret) -> Result<MqttOptions, String> {
    let url = BrokerUrl::parse(&settings.url)?;
    // Loading the system's root certificates panics if the platform can't supply them.
    let transport = std::panic::catch_unwind(Transport::wss_with_default_config)
        .map_err(|_| "Couldn't load the system's root certificates.".to_owned())?;
    // For websockets, rumqttc takes the host and path from the URL it is given and
    // ignores the port argument.
    let mut options = MqttOptions::new(random_client_id(), settings.url.clone(), url.port);
    options
        .set_transport(transport)
        .set_keep_alive(KEEP_ALIVE)
        .set_max_packet_size(MAX_PACKET_SIZE, MAX_PACKET_SIZE)
        .set_credentials(settings.username.clone(), password.expose().to_owned());
    Ok(options)
}

/// `scent-desktop-` and 8 hex digits, so two copies of the app don't kick each
/// other off the broker.
fn client_id(n: u32) -> String {
    format!("scent-desktop-{n:08x}")
}

fn random_client_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    // A fresh RandomState is seeded from the system's random source.
    client_id(RandomState::new().build_hasher().finish() as u32)
}

struct Worker {
    connected: Arc<AtomicBool>,
    events: Sender<Event>,
    repaint: Box<dyn Fn() + Send>,
}

/// Held by the worker's thread for as long as it runs. When the thread ends, however it
/// ends, a panic in rumqttc included, the connection flag goes down with it, so the app
/// no longer believes it is connected and [`MqttHandle::send`] refuses. A panic is also
/// told to the app, as nothing else would tell it; an ordinary end has been told already
/// (a refused login) or nobody is listening (the handle was dropped).
struct WorkerExit<'a>(&'a Worker);

impl Drop for WorkerExit<'_> {
    fn drop(&mut self) {
        self.0.connected.store(false, Ordering::SeqCst);
        if std::thread::panicking() {
            self.0
                .emit(Event::BrokerDisconnected(STOPPED_UNEXPECTEDLY.to_owned()));
        }
    }
}

impl Worker {
    /// Runs the connection until the handle is dropped, the login is refused or the
    /// app stops listening.
    fn run(self, eventloop: EventLoop, stopped: oneshot::Receiver<()>) {
        let _exit = WorkerExit(&self);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        match runtime {
            Ok(runtime) => runtime.block_on(self.poll_forever(eventloop, stopped)),
            Err(error) => {
                self.emit(Event::BrokerDisconnected(format!(
                    "Couldn't start the network runtime: {error}"
                )));
            }
        }
    }

    async fn poll_forever(&self, mut eventloop: EventLoop, mut stopped: oneshot::Receiver<()>) {
        loop {
            let polled = tokio::select! {
                _ = &mut stopped => break,
                polled = eventloop.poll() => polled,
            };
            let keep_going = match polled {
                Ok(rumqttc::Event::Incoming(Packet::ConnAck(_))) => {
                    self.on_connected(&mut eventloop)
                }
                Ok(rumqttc::Event::Incoming(Packet::Publish(publish))) => self.emit(Event::Mqtt {
                    topic: publish.topic,
                    body: publish.payload.to_vec(),
                    at: unix_now(),
                }),
                Ok(_) => true,
                Err(error) => {
                    self.connected.store(false, Ordering::SeqCst);
                    let event = event_for_error(&error);
                    let retry = RetryPolicy::after(&event);
                    let heard = self.emit(event);
                    match retry {
                        Some(delay) if heard => {
                            tokio::select! {
                                _ = &mut stopped => false,
                                () = tokio::time::sleep(delay) => true,
                            }
                        }
                        _ => false,
                    }
                }
            };
            if !keep_going {
                break;
            }
        }
    }

    /// A ConnAck means the login was accepted (rumqttc turns any other code into an
    /// error). Subscribes again, as the session starts clean every time. The
    /// subscription goes to the front of the event loop's own queue, so it can't be
    /// held up behind commands and goes out before any of them.
    fn on_connected(&self, eventloop: &mut EventLoop) -> bool {
        eventloop
            .pending
            .push_front(Subscribe::new(SUBSCRIBE_ALL, QoS::AtLeastOnce).into());
        self.connected.store(true, Ordering::SeqCst);
        self.emit(Event::BrokerConnected)
    }

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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use rumqttc::{AsyncClient, ConnectReturnCode, ConnectionError, MqttOptions};
    use scent_core::{Command, Secret};

    use super::*;
    use crate::state::Event;

    /// A handle on a client nobody polls: `send` only queues, so no network is needed.
    fn handle(connected: bool) -> (MqttHandle, rumqttc::EventLoop) {
        let (client, eventloop) = AsyncClient::new(MqttOptions::new("test", "localhost", 1883), 4);
        let handle = MqttHandle {
            client: Some(client),
            connected: Arc::new(AtomicBool::new(connected)),
            _stop: None,
        };
        (handle, eventloop)
    }

    /// A worker that reports into a channel and counts its repaints, sharing its
    /// connection flag with a handle, as `spawn` makes them; nothing polls the client.
    fn worker_and_handle() -> (
        Worker,
        MqttHandle,
        rumqttc::EventLoop,
        std::sync::mpsc::Receiver<Event>,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let (mut handle, eventloop) = handle(true);
        let connected = Arc::new(AtomicBool::new(true));
        handle.connected = Arc::clone(&connected);
        let (events, received) = std::sync::mpsc::channel();
        let repaints = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = Arc::clone(&repaints);
        let worker = Worker {
            connected,
            events,
            repaint: Box::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            }),
        };
        (worker, handle, eventloop, received, repaints)
    }

    #[test]
    fn a_worker_that_panics_says_so_and_takes_the_connection_down_with_it() {
        let (worker, handle, _eventloop, received, repaints) = worker_and_handle();
        assert!(handle.is_connected());
        let ended = std::thread::spawn(move || {
            let _exit = WorkerExit(&worker);
            panic!("rumqttc fell over");
        })
        .join();
        assert!(ended.is_err(), "the thread panicked");
        assert!(!handle.is_connected());
        assert!(handle.send("TESTDEVICE01", &Command::GetStatus).is_err());
        assert_eq!(
            received.try_recv(),
            Ok(Event::BrokerDisconnected(
                "The broker connection stopped unexpectedly. Restart the app.".to_owned()
            ))
        );
        // ...followed by a repaint, so that the footer says so at once.
        assert_eq!(repaints.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_worker_that_ends_on_its_own_takes_the_connection_down_and_says_nothing_more() {
        // Its handle was dropped, or the login was refused, which it has said already.
        let (worker, handle, _eventloop, received, repaints) = worker_and_handle();
        std::thread::spawn(move || {
            let _exit = WorkerExit(&worker);
        })
        .join()
        .unwrap();
        assert!(!handle.is_connected());
        assert!(handle.send("TESTDEVICE01", &Command::GetStatus).is_err());
        assert_eq!(
            received.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        );
        assert_eq!(repaints.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn connack_refusal_is_reported_once() {
        for code in [
            ConnectReturnCode::NotAuthorized,
            ConnectReturnCode::BadUserNamePassword,
        ] {
            let event = map_connack(code).expect("a refusal");
            assert!(matches!(event, Event::BrokerLoginRefused(_)), "{code:?}");
            // ...and a refusal is never retried, so it is reported once.
            assert_eq!(RetryPolicy::after(&event), None, "{code:?}");
        }
        assert_eq!(map_connack(ConnectReturnCode::Success), None);
        // Other refusals are not about the login: they are retried like any outage.
        assert_eq!(map_connack(ConnectReturnCode::ServiceUnavailable), None);
    }

    #[test]
    fn a_disconnect_is_retried_after_5_s() {
        let event = Event::BrokerDisconnected("Network timeout".into());
        assert_eq!(RetryPolicy::after(&event), Some(Duration::from_secs(5)));
        assert_eq!(RetryPolicy::after(&Event::BrokerConnected), None);
    }

    #[test]
    fn connection_errors_become_events() {
        let refused = ConnectionError::ConnectionRefused(ConnectReturnCode::NotAuthorized);
        assert!(matches!(
            event_for_error(&refused),
            Event::BrokerLoginRefused(_)
        ));
        let unavailable = ConnectionError::ConnectionRefused(ConnectReturnCode::ServiceUnavailable);
        for error in [unavailable, ConnectionError::NetworkTimeout] {
            let event = event_for_error(&error);
            assert!(matches!(event, Event::BrokerDisconnected(_)), "{error}");
            assert_eq!(RetryPolicy::after(&event), Some(Duration::from_secs(5)));
        }
    }

    #[test]
    fn send_refuses_bad_device_ids() {
        let (handle, _eventloop) = handle(true);
        for bad in ["#", "TESTDEVICE01/x", ""] {
            let error = handle.send(bad, &Command::GetStatus).unwrap_err();
            assert!(error.contains("device ID"), "{bad:?}: {error}");
        }
        assert_eq!(handle.send("TESTDEVICE01", &Command::GetStatus), Ok(()));
    }

    #[test]
    fn is_connected_follows_the_connection_flag() {
        let (up, _eventloop) = handle(true);
        assert!(up.is_connected());
        let (down, _eventloop) = handle(false);
        assert!(!down.is_connected());
    }

    #[test]
    fn send_refuses_while_the_broker_is_not_connected() {
        let (handle, _eventloop) = handle(false);
        let error = handle
            .send("TESTDEVICE01", &Command::GetStatus)
            .unwrap_err();
        assert!(error.contains("Not connected"), "{error}");
    }

    #[test]
    fn broker_url_parsing() {
        let url = BrokerUrl::parse("wss://broker.example.test/mqtt").unwrap();
        assert_eq!(url.host, "broker.example.test");
        assert_eq!(url.port, 443);
        assert_eq!(url.path, "/mqtt");

        let url = BrokerUrl::parse("wss://broker.example:8884/ws").unwrap();
        assert_eq!((url.host.as_str(), url.port), ("broker.example", 8884));
        assert_eq!(url.path, "/ws");

        let url = BrokerUrl::parse("wss://broker.example").unwrap();
        assert_eq!(url.path, "/");

        for bad in [
            "ws://broker.example/mqtt",
            "mqtt://broker.example",
            "tcp://broker.example:1883",
            "https://broker.example/mqtt",
            "broker.example",
            "",
            "wss://",
            "wss:///mqtt",
            "wss://broker.example:0/mqtt",
            "wss://broker.example:notaport/mqtt",
            "wss://user:secret@broker.example/mqtt",
        ] {
            assert!(BrokerUrl::parse(bad).is_err(), "{bad:?}");
        }
    }

    /// A loopback address nothing listens on: connecting is refused at once.
    fn closed_port_url() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        format!("wss://localhost:{port}/mqtt")
    }

    #[test]
    fn a_failed_connection_is_reported_and_dropping_the_handle_stops_the_worker() {
        let settings = BrokerSettings {
            url: closed_port_url(),
            ..BrokerSettings::default()
        };
        let (events, received) = std::sync::mpsc::channel();
        let handle = spawn(settings, Secret::new("x".into()), events, || {});
        let first = received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(first, Event::BrokerDisconnected(_)), "{first:?}");
        assert!(handle.send("TESTDEVICE01", &Command::GetStatus).is_err());

        // The worker is now waiting out its 5 s retry delay; dropping the handle must
        // end it at once, which closes the event channel.
        drop(handle);
        let closed = received.recv_timeout(Duration::from_secs(2));
        assert_eq!(closed, Err(std::sync::mpsc::RecvTimeoutError::Disconnected));
    }

    #[test]
    fn a_bad_broker_address_is_reported_and_starts_nothing() {
        let settings = BrokerSettings {
            url: "ws://broker.example/mqtt".into(),
            ..BrokerSettings::default()
        };
        let (events, received) = std::sync::mpsc::channel();
        let handle = spawn(settings, Secret::new("x".into()), events, || {});
        assert!(matches!(
            received.try_recv(),
            Ok(Event::BrokerDisconnected(_))
        ));
        assert!(handle.send("TESTDEVICE01", &Command::GetStatus).is_err());
    }

    #[test]
    fn without_a_broker_from_the_build_the_defaults_are_empty() {
        let none = BrokerSettings::from_values(None, None, None);
        assert_eq!(none.url, "");
        assert_eq!(none.username, "");
        assert_eq!(none.route_id, "");
        assert!(!none.is_set());
        // Empty or blank values from the build are none either.
        let blank = BrokerSettings::from_values(Some(""), Some(" "), Some("\t"));
        assert_eq!(blank, none);
    }

    #[test]
    fn a_broker_from_the_build_is_the_default() {
        let built = BrokerSettings::from_values(
            Some(" wss://broker.example.test/mqtt "),
            Some("example-user"),
            Some("route-test-1"),
        );
        assert_eq!(built, BrokerSettings::example());
        assert!(built.is_set());
        // `default()` is what this build was given, and nothing else.
        assert_eq!(
            BrokerSettings::default(),
            BrokerSettings::from_values(
                option_env!("SCENT_BROKER_URL"),
                option_env!("SCENT_BROKER_USERNAME"),
                option_env!("SCENT_ROUTE_ID"),
            )
        );
    }

    #[test]
    fn a_broker_is_set_only_when_it_has_an_address() {
        assert!(BrokerSettings::example().is_set());
        for url in ["", "  "] {
            let settings = BrokerSettings {
                url: url.to_owned(),
                ..BrokerSettings::example()
            };
            assert!(!settings.is_set(), "{url:?}");
        }
    }

    /// The largest schedule the editor lets through on firmware 2.30: 99 times, each on
    /// every day, plus the always-on entry.
    fn full_schedule() -> Command {
        use scent_core::{AlwaysOn, Entry, HhMm, Level, Profile, ScheduleFlag};
        let entry = Entry {
            start: HhMm::parse("23:57").unwrap(),
            end: HhMm::parse("23:59").unwrap(),
            weekdays: (0..7).collect(),
            level: Level::new(3).unwrap(),
            enable: true,
        };
        Command::UpdateProfile(Profile {
            always_on: AlwaysOn {
                enable: true,
                level: Level::new(3).unwrap(),
            },
            entries: vec![entry; 99],
            timezone_hours: -3.5,
            flag: ScheduleFlag::Replace,
        })
    }

    #[test]
    fn a_full_schedule_fits_well_under_the_packet_limit() {
        use rumqttc::{Packet, Publish};
        let body = full_schedule().mqtt_body();
        let publish = Publish::new(device_topic("TESTDEVICE01"), QoS::AtLeastOnce, body);
        // The size rumqttc checks against the limit when it writes the packet.
        let size = Packet::Publish(publish).size();
        // rumqttc's own default (10 KiB) would drop it: that was the bug.
        assert!(size > 10 * 1024, "{size}");
        // Well under ours, with room for the diffuser's acknowledgement, which repeats
        // the whole command as an escaped string, and for firmware that keeps more.
        assert!(size * 8 < MAX_PACKET_SIZE, "{size} vs {MAX_PACKET_SIZE}");
    }

    #[test]
    fn the_connection_takes_packets_up_to_the_limit() {
        let options = build_options(&BrokerSettings::example(), &Secret::new("x".into())).unwrap();
        // rumqttc reads back only the incoming limit; the outgoing one is set in the same
        // call, from the same constant.
        assert_eq!(options.max_packet_size(), MAX_PACKET_SIZE);
        assert_eq!(MAX_PACKET_SIZE, 256 * 1024);
    }

    #[test]
    fn client_ids_are_the_prefix_and_8_hex_digits() {
        assert_eq!(client_id(0xab12_cd34), "scent-desktop-ab12cd34");
        assert_eq!(client_id(0x1), "scent-desktop-00000001");
        let random = random_client_id();
        let hex = random.strip_prefix("scent-desktop-").expect("prefix");
        assert_eq!(hex.len(), 8);
        assert!(hex.bytes().all(|b| b.is_ascii_hexdigit()), "{random}");
    }
}
