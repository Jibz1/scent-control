//! Exercise real rumqttc sockets against two loopback MQTT peers. No live broker,
//! credentials, Bluetooth, or diffusers are touched by these tests.

use std::time::Duration;

use scent_core::Secret;
use scent_home_assistant::{
    config::{Config, Protocol},
    runtime,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};

const POWER: &str = "scent-control/example-home/TESTDEVICE01/power/set";
const LEVEL: &str = "scent-control/example-home/TESTDEVICE01/level/set";
const DEVICE: &str = "scentiment-mqtt-prod/device/TESTDEVICE01";
const REPORT: &str = "scentiment-mqtt-prod/user/route-test-1";
const AVAILABILITY: &str = "scent-control/example-home/availability";
const DEVICE_AVAILABILITY: &str = "scent-control/example-home/TESTDEVICE01/availability";

struct Peer(TcpStream);

impl Peer {
    async fn packet(&mut self) -> (u8, Vec<u8>) {
        let header = self.0.read_u8().await.unwrap();
        let mut length = 0;
        let mut multiplier = 1;
        loop {
            let byte = self.0.read_u8().await.unwrap();
            length += usize::from(byte & 127) * multiplier;
            if byte & 128 == 0 {
                break;
            }
            multiplier *= 128;
            assert!(multiplier <= 128 * 128 * 128);
        }
        assert!(length < 256 * 1024);
        let mut body = vec![0; length];
        self.0.read_exact(&mut body).await.unwrap();
        (header, body)
    }

    async fn send(&mut self, header: u8, body: &[u8]) {
        let mut bytes = vec![header];
        let mut length = body.len();
        loop {
            let digit = (length % 128) as u8;
            length /= 128;
            bytes.push(digit | if length > 0 { 128 } else { 0 });
            if length == 0 {
                break;
            }
        }
        bytes.extend(body);
        self.0.write_all(&bytes).await.unwrap();
    }

    async fn connect(&mut self, has_will: bool, accept_subscriptions: bool) -> Vec<String> {
        let (header, body) = self.packet().await;
        assert_eq!(header, 0x10);
        assert_ne!(body[7] & 2, 0, "must request a clean session");
        assert_eq!(body[7] & 4 != 0, has_will);
        if has_will {
            assert_ne!(body[7] & 32, 0, "will must be retained");
            assert!(body.windows(7).any(|w| w == b"offline"));
        }
        self.send(0x20, &[0, 0]).await;
        let (header, body) = self.packet().await;
        assert_eq!(header, 0x82);
        let mut at = 2;
        let mut topics = Vec::new();
        while at < body.len() {
            let length = u16::from_be_bytes([body[at], body[at + 1]]) as usize;
            at += 2;
            topics.push(String::from_utf8(body[at..at + length].to_vec()).unwrap());
            at += length;
            assert_eq!(
                body[at], 0,
                "commands/reports must not be queued at the broker"
            );
            at += 1;
        }
        let mut ack = body[..2].to_vec();
        ack.extend(vec![
            if accept_subscriptions { 0 } else { 128 };
            topics.len()
        ]);
        self.send(0x90, &ack).await;
        topics
    }

    async fn publish(&mut self, topic: &str, payload: &[u8], retain: bool) {
        let mut body = (topic.len() as u16).to_be_bytes().to_vec();
        body.extend(topic.as_bytes());
        body.extend(payload);
        self.send(if retain { 0x31 } else { 0x30 }, &body).await;
    }

    async fn next_publish(&mut self) -> (u8, String, Vec<u8>) {
        loop {
            let (header, body) = self.packet().await;
            match header >> 4 {
                3 => {
                    let length = u16::from_be_bytes([body[0], body[1]]) as usize;
                    let topic = String::from_utf8(body[2..2 + length].to_vec()).unwrap();
                    let mut offset = 2 + length;
                    if (header >> 1) & 3 == 1 {
                        self.send(0x40, &body[offset..offset + 2]).await;
                        offset += 2;
                    }
                    return (header, topic, body[offset..].to_vec());
                }
                12 => self.send(0xd0, &[]).await,
                _ => panic!("unexpected MQTT packet type {}", header >> 4),
            }
        }
    }

    async fn until(&mut self, expected_topic: &str, expected_body: &[u8]) {
        timeout(Duration::from_secs(5), async {
            loop {
                let (_, topic, payload) = self.next_publish().await;
                if topic == expected_topic && payload == expected_body {
                    break;
                }
            }
        })
        .await
        .expect("expected publish did not arrive");
    }
}

struct Harness {
    upstream: Peer,
    home: Peer,
    task: tokio::task::JoinHandle<Result<(), &'static str>>,
    config: Config,
    upstream_listener: TcpListener,
    home_listener: TcpListener,
}

impl Harness {
    async fn start() -> Self {
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let home_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = Config::parse(include_bytes!("../config.example.json")).unwrap();
        for (broker, listener) in [
            (&mut config.diffuser_broker, &upstream_listener),
            (&mut config.home_assistant_broker, &home_listener),
        ] {
            broker.host = "127.0.0.1".to_owned();
            broker.port = listener.local_addr().unwrap().port();
            broker.transport = Protocol::Tcp;
            broker.allow_insecure = true;
        }
        config.validate().unwrap();
        let task = spawn(config.clone());
        let (upstream, home) = tokio::join!(upstream_listener.accept(), home_listener.accept());
        Self {
            upstream: Peer(upstream.unwrap().0),
            home: Peer(home.unwrap().0),
            task,
            config,
            upstream_listener,
            home_listener,
        }
    }

    async fn ready(&mut self) {
        let subscriptions = self.upstream.connect(false, true).await;
        assert_eq!(
            subscriptions,
            vec![REPORT, "scentiment-mqtt-prod/server-notifications/device"]
        );
        assert_eq!(
            self.home.connect(true, true).await,
            vec![POWER, LEVEL, "homeassistant/status"]
        );
        self.home.until(AVAILABILITY, b"online").await;
        let (flags, topic, body) = self.upstream.next_publish().await;
        assert_eq!(flags, 0x30, "PING must be QoS 0 and unretained");
        assert_eq!(topic, DEVICE);
        assert_eq!(
            body,
            br#"{"action":"PING","payload":{"userid":"route-test-1"}}"#
        );
    }

    async fn report(&mut self) {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/scentiment/air2-mqtt-update-status-object.json"
        ))
        .unwrap();
        self.upstream
            .publish(REPORT, fixture["text"].as_str().unwrap().as_bytes(), false)
            .await;
        self.home.until(DEVICE_AVAILABILITY, b"online").await;
    }
}

fn spawn(config: Config) -> tokio::task::JoinHandle<Result<(), &'static str>> {
    tokio::spawn(async move {
        runtime::session(
            config,
            &Secret::new("synthetic".into()),
            &Secret::new("synthetic".into()),
        )
        .await
    })
}

#[tokio::test]
async fn discovery_control_ha_restart_and_disconnect_over_real_sockets() {
    timeout(Duration::from_secs(15), async {
        let mut h = Harness::start().await;
        h.ready().await;
        // Neither a broker connection nor our own PING is evidence the diffuser is online.
        h.home.publish(POWER, b"ON", false).await;
        assert!(
            timeout(Duration::from_millis(100), h.upstream.next_publish())
                .await
                .is_err()
        );
        h.report().await;
        h.home.publish(LEVEL, b"3", true).await;
        assert!(
            timeout(Duration::from_millis(100), h.upstream.next_publish())
                .await
                .is_err()
        );
        h.home.publish(LEVEL, b"3", false).await;
        let (flags, topic, body) = h.upstream.next_publish().await;
        assert_eq!((flags, topic.as_str()), (0x30, DEVICE));
        assert_eq!(body, br#"{"action":"SET_LEVEL","payload":{"intensity":3}}"#);
        assert_eq!(
            h.upstream.next_publish().await.2,
            br#"{"action":"TURN_ON","payload":{"on":1}}"#
        );
        // HA birth republishes discovery and the last *reported* level, not 3.
        h.home
            .publish("homeassistant/status", b"online", false)
            .await;
        h.home
            .until("scent-control/example-home/TESTDEVICE01/level/state", b"2")
            .await;
        h.home.until(AVAILABILITY, b"online").await;
        drop(h.upstream);
        assert!(h.task.await.unwrap().is_err());
        // The other session must close too, causing the HA broker to send its LWT.
        let mut byte = [0];
        assert_eq!(
            timeout(Duration::from_secs(2), h.home.0.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );

        // A fresh pair of connections advertises unknown state and sends only PING.
        let task = spawn(h.config);
        let (upstream, home) = tokio::join!(h.upstream_listener.accept(), h.home_listener.accept());
        let mut upstream = Peer(upstream.unwrap().0);
        let mut home = Peer(home.unwrap().0);
        upstream.connect(false, true).await;
        home.connect(true, true).await;
        home.until(
            "scent-control/example-home/TESTDEVICE01/power/state",
            b"None",
        )
        .await;
        home.until(AVAILABILITY, b"online").await;
        let body = upstream.next_publish().await.2;
        assert!(String::from_utf8_lossy(&body).contains("\"PING\""));
        assert!(
            timeout(Duration::from_millis(100), upstream.next_publish())
                .await
                .is_err()
        );
        task.abort();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn subscription_denial_stops_the_session_before_device_commands() {
    timeout(Duration::from_secs(5), async {
        let mut h = Harness::start().await;
        h.upstream.connect(false, false).await;
        assert_eq!(
            h.task.await.unwrap(),
            Err("Broker refused a required subscription; check its ACL.")
        );
    })
    .await
    .unwrap();
}
