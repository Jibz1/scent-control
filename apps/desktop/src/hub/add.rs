//! Adding a diffuser, as the hub runs it (see `add_flow` for the order of the steps):
//! its info is read over Bluetooth first, and everything after that runs as one job in
//! that diffuser's queue, so no other command to it comes between its steps. One add
//! at a time for each diffuser: another asked for while one is under way is refused.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use scent_core::{Command, DeviceInfo, Secret, Via};
use tokio::time::{Instant, sleep};

use super::{BleError, Shared, Work, answer};
use crate::add_flow::{self, AddRequest, AddStep, NO_BROKER_PASSWORD as NO_PASSWORD_TO_GIVE};
use crate::mqtt::BrokerSettings;
use crate::route::TransportPref;
use crate::state::{AppState, BrokerState, Event, unix_now};
use crate::transport::{Transport, lock};

/// How often the hub looks at the state while it waits for something to show up there.
const STATE_POLL: Duration = Duration::from_millis(250);

/// How long the info read over Bluetooth has to reach the state. The screens fold it in
/// at their next frame.
const INFO_WAIT: Duration = Duration::from_secs(5);

const CREDENTIALS_WAIT: Duration = Duration::from_secs(add_flow::CREDENTIALS_WAIT_S);

/// What the log calls an add the hub turned down.
const ADD: &str = "Add";

const NO_INFO: &str = "The diffuser answered, but its info didn't come through, so its device ID isn't known. Try again.";

const LOGIN_REFUSED: &str = "The broker refused this app's login, so the diffuser would be refused too. Fix it in Settings first.";

/// Why an add stops before it starts when there is no broker to put the diffuser on.
const NO_BROKER_TO_GIVE: &str = "Add the broker in Settings first.";

const ALREADY_ADDING: &str =
    "This diffuser is already being added. Wait for that to finish, then try again.";

/// A step the add never runs. Only a mistake in the app gets here.
const LOST_STEP: &str = "The add stopped at a step it doesn't run. Try again.";

/// The broker login the hub last started the MQTT worker with. An add hands it to the
/// diffuser in `SAVE_MQTT` (the test broker has one shared login). Not `Debug`.
#[derive(Default)]
pub(super) struct Login {
    pub(super) settings: BrokerSettings,
    pub(super) password: Option<Secret>,
}

/// The diffusers being added, by Bluetooth ID.
pub(super) type Adds = Arc<Mutex<HashSet<String>>>;

/// An add under way. It is over when this is dropped, however the add ended: done,
/// failed, or its job lost.
struct InProgress {
    adds: Adds,
    ble_id: String,
}

impl Drop for InProgress {
    fn drop(&mut self) {
        lock(&self.adds).remove(&self.ble_id);
    }
}

/// What a diffuser's add needs once its info is read: the rest runs as one job in its
/// queue. Not `Debug`: it can hold a WiFi password.
pub(super) struct AddJob {
    ble_id: String,
    info: DeviceInfo,
    wifi: Option<(String, Secret)>,
    _in_progress: InProgress,
}

/// How a write to a diffuser being added ended.
enum Written {
    Yes,
    /// No answer before the backstop: it may still have gone through.
    Unknown,
    /// It failed, with this error: not written, or not all of it.
    No(String),
}

impl<T: Transport> Shared<T> {
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

    /// Marks an add of the diffuser at `ble_id` as under way, unless one already is.
    fn start_add(&self, ble_id: &str) -> Option<InProgress> {
        lock(&self.adds)
            .insert(ble_id.to_owned())
            .then(|| InProgress {
                adds: Arc::clone(&self.adds),
                ble_id: ble_id.to_owned(),
            })
    }

    /// The first part of an add: no other add of it may be under way, and the broker
    /// must have taken our login, which is what the diffuser is handed. Then its info is
    /// read over Bluetooth, by its Bluetooth ID. Returns its device ID and the job that
    /// does the rest, for its queue; `None` when the add stops here, having said why.
    pub(super) async fn prepare_add(&self, request: AddRequest) -> Option<(String, AddJob)> {
        let AddRequest {
            ble_id,
            label: _,
            room: _,
            wifi,
        } = request;
        let Some(in_progress) = self.start_add(&ble_id) else {
            // Said in the log, not as this add's step: the one under way goes on.
            let device_id = lock(&self.state)
                .devices
                .values()
                .find(|device| device.ble_id.as_deref() == Some(ble_id.as_str()))
                .map(|device| device.id.clone());
            self.failed(
                device_id.as_deref(),
                ADD.to_owned(),
                ALREADY_ADDING.to_owned(),
            );
            return None;
        };
        let refused = matches!(lock(&self.state).broker, BrokerState::LoginRefused(_));
        let (broker_set, has_password) = {
            let login = lock(&self.login);
            (login.settings.is_set(), login.password.is_some())
        };
        let stop = if !broker_set {
            Some(NO_BROKER_TO_GIVE)
        } else if refused {
            Some(LOGIN_REFUSED)
        } else if !has_password {
            Some(NO_PASSWORD_TO_GIVE)
        } else {
            None
        };
        if let Some(why) = stop {
            self.adding(&ble_id, AddStep::Failed(why.to_owned()));
            return None;
        }
        self.adding(&ble_id, AddStep::ReadInfo);
        let started = unix_now();
        if let Err(error) = answer(self.transport.ble_read_info(&ble_id)).await {
            self.adding(&ble_id, AddStep::Failed(error.to_string()));
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
                let job = AddJob {
                    ble_id,
                    info,
                    wifi,
                    _in_progress: in_progress,
                };
                Some((device_id, job))
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
    /// wait for `device_connected` tells whether it worked. Every way out says how the
    /// add ended.
    pub(super) async fn run_add(&self, device_id: &str, job: AddJob) {
        let AddJob {
            ble_id, info, wifi, ..
        } = job;
        let wifi_given = wifi.is_some();
        // When the restart was asked for, by the app's clock (the diffuser must join
        // the broker after it) and by the hub's (the 90 s count from it).
        let mut restarted = (unix_now(), Instant::now());
        let mut step = AddStep::SaveMqtt;
        loop {
            self.adding(&ble_id, step.clone());
            let mut connected = None;
            let outcome = match &step {
                AddStep::SaveMqtt => {
                    let (settings, password) = self.login();
                    let written = match password {
                        Some(password) => {
                            let save = Command::SaveMqtt {
                                broker: settings.url,
                                username: settings.username,
                                password,
                            };
                            self.add_write(device_id, &ble_id, save).await
                        }
                        None => Written::No(NO_PASSWORD_TO_GIVE.to_owned()),
                    };
                    if !matches!(written, Written::No(_)) {
                        // Always the whole wait, whatever the diffuser says meanwhile
                        // (see `add_flow::CREDENTIALS_WAIT_S`).
                        sleep(CREDENTIALS_WAIT).await;
                    }
                    written
                }
                AddStep::WifiConnect => match wifi.clone() {
                    Some((ssid, password)) => {
                        let join = Command::WifiConnect { ssid, password };
                        self.add_write(device_id, &ble_id, join).await
                    }
                    None => Written::No(LOST_STEP.to_owned()),
                },
                AddStep::Restart => {
                    restarted = (unix_now(), Instant::now());
                    // The reboot can cut the write short; whether it went through is
                    // what the wait finds out.
                    let _ = self
                        .add_write(device_id, &ble_id, Command::SoftRestart)
                        .await;
                    Written::Yes
                }
                AddStep::WaitConnected => match self.wait_connected(device_id, restarted).await {
                    AddStep::Ping => {
                        connected = Some(device_id);
                        Written::Yes
                    }
                    AddStep::Failed(why) => Written::No(why),
                    _ => Written::No(LOST_STEP.to_owned()),
                },
                AddStep::Ping => {
                    // Joining proved the move. If the broker won't take the ping now
                    // (the failure is logged), the ping round registers it later.
                    let userid = Some(self.login().0.route_id);
                    let ping = Work::send(Command::Ping { userid });
                    if self.deliver(device_id, &ping, TransportPref::Wifi).await {
                        self.emit(Event::Pinged {
                            device_id: device_id.to_owned(),
                            at: unix_now(),
                        });
                    }
                    Written::Yes
                }
                AddStep::Done(_) | AddStep::Failed(_) => return,
                AddStep::ReadInfo | AddStep::NeedWifi => Written::No(LOST_STEP.to_owned()),
            };
            if let Written::No(why) = outcome {
                self.adding(&ble_id, AddStep::Failed(why));
                return;
            }
            // Written::Unknown goes on, as Written::Yes does: never sent again.
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
                let failed = [(Via::Bluetooth, error.to_string())];
                self.report_failure(device_id, &work, &failed, None);
                match error {
                    BleError::Unanswered => Written::Unknown,
                    BleError::Failed(why) => Written::No(why),
                }
            }
        }
    }

    /// Waits for the diffuser to say it joined the broker after the restart, for up to
    /// [`add_flow::WAIT_CONNECTED_S`] from it: `restarted` is when it was asked for, by
    /// the app's clock and by the hub's. `Ping` once it has, `Failed` when it hasn't.
    async fn wait_connected(&self, device_id: &str, restarted: (i64, Instant)) -> AddStep {
        let (since, at) = restarted;
        loop {
            let connected = lock(&self.state)
                .devices
                .get(device_id)
                .and_then(|device| device.connected_at)
                .is_some_and(|connected| connected > since);
            let waited = i64::try_from(at.elapsed().as_secs()).unwrap_or(i64::MAX);
            match add_flow::wait_outcome(0, waited, connected) {
                AddStep::WaitConnected => sleep(STATE_POLL).await,
                outcome => return outcome,
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use scent_core::{Command, Secret, Via};
    use tokio::time::Instant;

    use super::super::UiCommand;
    use super::super::tests::{Ble, ONE, Rig, failed, info, secs, sent, turn_on};
    use crate::add_flow::{AddRequest, AddStep};
    use crate::mqtt::BrokerSettings;
    use crate::route::TransportPref;
    use crate::state::{BrokerState, Event, unix_now};
    use crate::transport::lock;

    // --- adding a diffuser ---

    fn request(ble_id: &str, wifi: Option<(&str, &str)>) -> AddRequest {
        AddRequest {
            ble_id: ble_id.to_owned(),
            label: "Bedroom".to_owned(),
            room: String::new(),
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
        let settings = BrokerSettings::example();
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
    async fn add_waits_the_full_25_s_after_the_save() {
        // Whatever the diffuser says meanwhile: the test login it makes with the new
        // credentials (about 20 s) could otherwise announce it on the broker after an
        // early restart, as though the restart had worked.
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

        let quiet = rig();
        add(&quiet, request("ble-1", None)).await;
        assert_eq!(saved_to_restart(&quiet), secs(25));

        // "Credentials saved" over Bluetooth after 4 s: still 25 s.
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
        assert_eq!(saved_to_restart(&over_ble), secs(25));

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
        assert_eq!(saved_to_restart(&over_wifi), secs(25));
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
        // A broker, and no password for it.
        lock(&rig.shared.login).settings = BrokerSettings::example();
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
        rig.shared.reconnect(BrokerSettings::example(), None);
        rig.events();
        add(&rig, request("ble-1", None)).await;
        assert_eq!(rig.events(), [failed]);
        assert!(rig.fake().labels().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn add_with_no_broker_set_fails_at_once_and_says_to_set_one() {
        let rig = Rig::new().info_for("ble-1", info(ONE, true));
        // A build given no broker, and nothing saved yet: whatever the Keychain held.
        let none = BrokerSettings {
            url: String::new(),
            ..BrokerSettings::example()
        };
        rig.shared
            .reconnect(none, Some(Secret::new("broker-secret".to_owned())));
        rig.events();
        let before = Instant::now();

        add(&rig, request("ble-1", None)).await;

        assert_eq!(before.elapsed(), Duration::ZERO);
        assert!(rig.fake().labels().is_empty(), "Bluetooth was not touched");
        let failed = Event::Adding {
            ble_id: "ble-1".to_owned(),
            step: AddStep::Failed("Add the broker in Settings first.".to_owned()),
        };
        assert_eq!(rig.events(), [failed]);
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
    async fn a_ping_the_broker_will_not_take_still_ends_done() {
        // The broker went down after the diffuser joined it. Joining proved the move,
        // and the ping round registers it once the broker is back.
        let rig = Rig::new()
            .device(ONE, Some("ble-1"))
            .info_for("ble-1", info(ONE, true))
            .logged_in("broker-secret");
        rig.apply_later(secs(30), device_connected(ONE, soon()));

        add(&rig, request("ble-1", None)).await;

        let events = rig.events();
        assert_eq!(steps(&events).last(), Some(&AddStep::Done(ONE.to_owned())));
        // The ping that didn't go is logged, and the diffuser doesn't count as pinged.
        assert!(failed(&events).iter().any(|(_, what, _)| what == "PING"));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::Pinged { .. }))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_wait_for_it_to_join_counts_from_the_restart() {
        // The restart's write is never answered: the backstop takes 30 s of the 90.
        let rig = Rig::new()
            .device(ONE, Some("ble-1"))
            .info_for("ble-1", info(ONE, true))
            .logged_in("broker-secret");
        rig.broker_up();
        rig.fake().script_ble([
            Ble::After(Duration::ZERO, Ok(())),
            Ble::After(Duration::ZERO, Ok(())),
            Ble::Never,
        ]);

        add(&rig, request("ble-1", None)).await;

        let restart = when(&rig, "ble ble-1 SECRET_COMMAND");
        assert_eq!(Instant::now() - rig.fake().began - restart, secs(90));
        let events = rig.events();
        assert!(
            matches!(steps(&events).last(), Some(AddStep::Failed(_))),
            "{events:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn add_is_refused_while_the_broker_refuses_our_login() {
        // The diffuser would be handed a login the broker turns down, and be stranded.
        let rig = Rig::new()
            .info_for("ble-1", info(ONE, true))
            .logged_in("broker-secret");
        lock(&rig.state).broker = BrokerState::LoginRefused("Not authorized.".to_owned());

        add(&rig, request("ble-1", None)).await;

        assert!(rig.fake().labels().is_empty(), "Bluetooth was not touched");
        let events = rig.events();
        let [
            Event::Adding {
                step: AddStep::Failed(why),
                ..
            },
        ] = &events[..]
        else {
            panic!("{events:?}");
        };
        assert!(why.contains("refused") && why.contains("Settings"), "{why}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_second_add_for_a_diffuser_being_added_is_refused() {
        let rig = Rig::new()
            .device(ONE, Some("ble-1"))
            .info_for("ble-1", info(ONE, true))
            .logged_in("broker-secret");
        rig.broker_up();
        rig.apply_later(secs(40), device_connected(ONE, soon()));
        let hub = rig.serve();
        let count = |label: &str| {
            rig.fake()
                .labels()
                .iter()
                .filter(|seen| *seen == label)
                .count()
        };

        hub.send(UiCommand::Add(request("ble-1", None))).unwrap();
        // While the first is waiting after the save, and then while it waits for the
        // diffuser to join.
        tokio::time::sleep(secs(5)).await;
        hub.send(UiCommand::Add(request("ble-1", None))).unwrap();
        tokio::time::sleep(secs(30)).await;
        hub.send(UiCommand::Add(request("ble-1", None))).unwrap();
        tokio::time::sleep(secs(30)).await;

        assert_eq!(count("ble ble-1 read info"), 1);
        assert_eq!(count("ble ble-1 SAVE_MQTT"), 1);
        assert_eq!(count("ble ble-1 SECRET_COMMAND"), 1);
        let events = rig.events();
        let refused: Vec<_> = failed(&events)
            .into_iter()
            .filter(|(_, what, _)| what == "Add")
            .collect();
        assert_eq!(refused.len(), 2, "{events:?}");
        assert!(refused[0].2.contains("already being added"), "{refused:?}");
        // The first carried on, untouched.
        assert_eq!(steps(&events).last(), Some(&AddStep::Done(ONE.to_owned())));

        // Once it is over, it can be added again.
        hub.send(UiCommand::Add(request("ble-1", None))).unwrap();
        tokio::time::sleep(secs(1)).await;
        assert_eq!(count("ble ble-1 read info"), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_wifi_connect_that_cannot_reach_the_diffuser_fails_the_add() {
        let rig = Rig::new()
            .device(ONE, Some("ble-1"))
            .info_for("ble-1", info(ONE, false))
            .logged_in("broker-secret");
        rig.broker_up();
        rig.fake().script_ble([
            Ble::After(Duration::ZERO, Ok(())),
            Ble::After(Duration::ZERO, Ok(())),
            Ble::After(secs(13), Err(CONNECT_ERROR.to_owned())),
        ]);

        add(&rig, request("ble-1", Some(("ExampleWiFi", "hunter2")))).await;

        assert_eq!(
            steps(&rig.events()),
            [
                AddStep::ReadInfo,
                AddStep::SaveMqtt,
                AddStep::WifiConnect,
                AddStep::Failed(CONNECT_ERROR.to_owned())
            ]
        );
        // Not restarted, and nothing went over WiFi.
        let labels = rig.fake().labels();
        assert!(!labels.iter().any(|label| label.contains("SECRET_COMMAND")));
        assert!(!labels.iter().any(|label| label.starts_with("wifi")));
    }

    #[tokio::test(start_paused = true)]
    async fn a_wifi_connect_whose_answer_never_comes_goes_on_to_the_restart() {
        let rig = Rig::new()
            .device(ONE, Some("ble-1"))
            .info_for("ble-1", info(ONE, false))
            .logged_in("broker-secret");
        rig.broker_up();
        rig.fake().script_ble([
            Ble::After(Duration::ZERO, Ok(())),
            Ble::After(Duration::ZERO, Ok(())),
            Ble::Never,
        ]);
        rig.apply_later(secs(90), device_connected(ONE, soon()));

        add(&rig, request("ble-1", Some(("ExampleWiFi", "hunter2")))).await;

        // 25 s after the save, then 30 s for the backstop: sent once, never again.
        assert_eq!(when(&rig, "ble ble-1 SECRET_COMMAND"), secs(55));
        let joins = lock(&rig.fake().ble_commands)
            .iter()
            .filter(|(_, cmd)| matches!(cmd, Command::WifiConnect { .. }))
            .count();
        assert_eq!(joins, 1);
        assert_eq!(
            steps(&rig.events()).last(),
            Some(&AddStep::Done(ONE.to_owned()))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn info_that_reaches_the_state_a_moment_after_the_read_is_used() {
        // The screens fold the info in at their next frame, after the reply.
        let rig = Rig::new()
            .info_for("ble-1", info(ONE, false))
            .info_after(secs(1))
            .logged_in("broker-secret");

        add(&rig, request("ble-1", None)).await;

        assert_eq!(steps(&rig.events()), [AddStep::ReadInfo, AddStep::NeedWifi]);
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
}
