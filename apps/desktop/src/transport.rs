//! How the hub reaches the diffusers: the [`Transport`] it talks to, and [`Radios`],
//! the real one, made of the Bluetooth worker and the MQTT worker.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use scent_core::{Command, Secret};

use crate::ble::{self, BleHandle, Reply};
use crate::mqtt::{self, BrokerSettings, MqttHandle};
use crate::state::Event;

/// A repaint the workers can own.
fn forward(repaint: &Arc<dyn Fn() + Send + Sync>) -> impl Fn() + Send + 'static + use<> {
    let repaint = Arc::clone(repaint);
    move || repaint()
}

/// What the hub uses to reach the diffusers. [`Radios`] is the real thing; the tests
/// use a fake, since nothing they run may touch Bluetooth or the network.
pub trait Transport: Send + Sync + 'static {
    /// Scans for `secs` seconds.
    fn ble_scan(&self, secs: u64) -> Reply;
    /// Writes `cmd` to the diffuser at `ble_id`. The reply is `Ok` once it is written.
    fn ble_send(&self, ble_id: &str, cmd: Command) -> Reply;
    /// Reads the last state the diffuser at `ble_id` reported.
    fn ble_read_status(&self, ble_id: &str) -> Reply;
    /// Queues `cmd` for the diffuser on the broker. Refuses while it isn't connected.
    fn wifi_send(&self, device_id: &str, cmd: &Command) -> Result<(), String>;
    /// The route ID to ping with, while the broker is connected; `None` otherwise.
    fn broker_route_id(&self) -> Option<String>;
    /// Stops the broker worker, if there is one, and starts a new one with these
    /// settings unless there is no password.
    fn reconnect(&self, settings: BrokerSettings, password: Option<Secret>);
}

/// The real transport: the Bluetooth worker and the MQTT worker.
pub struct Radios {
    ble: BleHandle,
    /// `None` until a worker is started, and while there is no password.
    broker: Mutex<Option<Arc<Broker>>>,
    events: Sender<Event>,
    repaint: Arc<dyn Fn() + Send + Sync>,
}

struct Broker {
    handle: MqttHandle,
    /// The `userid` for pings.
    route_id: String,
}

impl Radios {
    pub fn new(events: Sender<Event>, repaint: Arc<dyn Fn() + Send + Sync>) -> Radios {
        let ble = ble::spawn(events.clone(), forward(&repaint));
        Radios {
            ble,
            broker: Mutex::new(None),
            events,
            repaint,
        }
    }
}

impl Transport for Radios {
    fn ble_scan(&self, secs: u64) -> Reply {
        self.ble.scan(secs)
    }

    fn ble_send(&self, ble_id: &str, cmd: Command) -> Reply {
        self.ble.send(ble_id, cmd)
    }

    fn ble_read_status(&self, ble_id: &str) -> Reply {
        self.ble.read_status(ble_id)
    }

    fn wifi_send(&self, device_id: &str, cmd: &Command) -> Result<(), String> {
        let broker = lock(&self.broker).clone();
        match broker {
            Some(broker) => broker.handle.send(device_id, cmd),
            None => Err("Not connected to the broker.".to_owned()),
        }
    }

    fn broker_route_id(&self) -> Option<String> {
        lock(&self.broker)
            .as_ref()
            .filter(|broker| broker.handle.is_connected())
            .map(|broker| broker.route_id.clone())
    }

    fn reconnect(&self, settings: BrokerSettings, password: Option<Secret>) {
        // The old worker is told to stop first, so it is not left logged in beside the new one.
        let old = lock(&self.broker).take();
        drop(old);
        let Some(password) = password else { return };
        let route_id = settings.route_id.clone();
        let handle = mqtt::spawn(
            settings,
            password,
            self.events.clone(),
            forward(&self.repaint),
        );
        *lock(&self.broker) = Some(Arc::new(Broker { handle, route_id }));
    }
}

/// Locks, and carries on if another thread panicked while holding the lock: what
/// these locks guard stays usable, and the hub must not die of it.
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use scent_core::Secret;

    use super::*;

    const ID: &str = "TESTDEVICE01";

    /// An address `mqtt::spawn` turns down before it loads any certificate, so it
    /// starts no worker and touches no network.
    fn unparseable() -> BrokerSettings {
        BrokerSettings {
            url: "ws://not-encrypted.example.test/mqtt".to_owned(),
            ..BrokerSettings::default()
        }
    }

    fn refused(radios: &Radios) -> String {
        radios.wifi_send(ID, &Command::GetStatus).unwrap_err()
    }

    // The Bluetooth worker `Radios::new` starts is idle: it creates no
    // `btleplug::Manager` until its first request, and these tests make none.
    #[test]
    fn a_broker_that_never_connected_is_not_up_and_refuses_sends() {
        let (events, received) = channel();
        let radios = Radios::new(events, Arc::new(|| {}));

        // Nothing started yet.
        assert_eq!(radios.broker_route_id(), None);
        assert!(refused(&radios).contains("Not connected"));

        // A worker slot exists, but its handle never started. The route ID is offered
        // only while the broker is connected, so it is still `None`: this is all that
        // separates "a worker exists" from "the broker is up".
        radios.reconnect(unparseable(), Some(Secret::new("x".to_owned())));
        assert!(lock(&radios.broker).is_some());
        assert_eq!(radios.broker_route_id(), None);
        assert!(refused(&radios).contains("Not connected"));
        assert!(matches!(
            received.try_recv(),
            Ok(Event::BrokerDisconnected(_))
        ));
    }

    #[test]
    fn reconnecting_without_a_password_drops_the_old_worker_and_starts_none() {
        let (events, _received) = channel();
        let radios = Radios::new(events, Arc::new(|| {}));
        radios.reconnect(unparseable(), Some(Secret::new("x".to_owned())));
        assert!(lock(&radios.broker).is_some());

        radios.reconnect(unparseable(), None);

        assert!(lock(&radios.broker).is_none());
        assert_eq!(radios.broker_route_id(), None);
        assert!(refused(&radios).contains("Not connected"));
    }

    #[test]
    fn a_bad_device_id_is_refused_by_the_broker_handle() {
        let (events, _received) = channel();
        let radios = Radios::new(events, Arc::new(|| {}));
        radios.reconnect(unparseable(), Some(Secret::new("x".to_owned())));
        let error = radios.wifi_send("#", &Command::GetStatus).unwrap_err();
        assert!(error.contains("device ID"), "{error}");
    }
}
