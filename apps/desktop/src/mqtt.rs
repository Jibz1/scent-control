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

impl Default for BrokerSettings {
    fn default() -> Self {
        BrokerSettings {
            url: "wss://test-broker-production.up.railway.app/mqtt".to_owned(),
            username: "scent-test".to_owned(),
            route_id: "route-test-1".to_owned(),
        }
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
    /// True from the ConnAck until the connection drops.
    connected: Arc<AtomicBool>,
    /// The worker ends when this goes away.
    _stop: Option<oneshot::Sender<()>>,
}

impl MqttHandle {
    /// Whether the broker is connected right now: true from the ConnAck until the
    /// connection drops, and never true for a worker that did not start.
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    /// Queues `cmd` for the diffuser, as a QoS 1 message on its device topic, and
    /// returns without waiting for the network. It refuses a bad device ID, and
    /// refuses while the broker isn't connected so that nothing waits in the queue
    /// to go out minutes later, when the user no longer expects it.
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

impl Worker {
    /// Runs the connection until the handle is dropped, the login is refused or the
    /// app stops listening.
    fn run(self, eventloop: EventLoop, stopped: oneshot::Receiver<()>) {
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

    async fn poll_forever(self, mut eventloop: EventLoop, mut stopped: oneshot::Receiver<()>) {
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
        self.connected.store(false, Ordering::SeqCst);
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
        let url = BrokerUrl::parse("wss://test-broker-production.up.railway.app/mqtt").unwrap();
        assert_eq!(url.host, "test-broker-production.up.railway.app");
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
    fn the_defaults_are_the_test_broker() {
        let settings = BrokerSettings::default();
        assert_eq!(
            settings.url,
            "wss://test-broker-production.up.railway.app/mqtt"
        );
        assert_eq!(settings.username, "scent-test");
        assert_eq!(settings.route_id, "route-test-1");
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
