//! Two clean MQTT sessions. Losing either discards both queues and all cached readings.

use std::collections::HashMap;
use std::time::Duration;

use rumqttc::{
    AsyncClient, Event, LastWill, MqttOptions, Packet, Publish, QoS, SubscribeFilter,
    SubscribeReasonCode,
};
use scent_core::Secret;
use tokio::{
    sync::mpsc,
    task::JoinHandle,
    time::{Instant, MissedTickBehavior},
};

use crate::bridge::{Bridge, CONNECTED_TOPIC, HA_STATUS, Publication};
use crate::config::Config;

#[derive(Clone, Copy)]
enum Side {
    Diffuser,
    HomeAssistant,
}

type Incoming = (Side, Result<Event, &'static str>);

struct Link {
    client: AsyncClient,
    worker: JoinHandle<()>,
    subscriptions: Vec<String>,
    ready: bool,
}

impl Drop for Link {
    fn drop(&mut self) {
        // Aborting destroys this event loop, including rumqttc's retry queues.
        // Closing HA's socket also triggers the retained offline last will.
        self.worker.abort();
    }
}

impl Link {
    fn new(
        options: MqttOptions,
        subscriptions: Vec<String>,
        side: Side,
        events: mpsc::Sender<Incoming>,
    ) -> Self {
        let (client, mut eventloop) = AsyncClient::new(options, 256);
        eventloop.network_options.set_connection_timeout(10);
        let worker = tokio::spawn(async move {
            // Poll each connection in its own task: cancelling poll while it is
            // establishing a connection can discard the socket mid-handshake.
            loop {
                let event = eventloop.poll().await.map_err(|_| match side {
                    Side::Diffuser => {
                        "Diffuser broker disconnected; check connectivity and credentials."
                    }
                    Side::HomeAssistant => {
                        "Home Assistant broker disconnected; check connectivity and credentials."
                    }
                });
                let failed = event.is_err();
                if events.send((side, event)).await.is_err() || failed {
                    break;
                }
            }
        });
        Self {
            client,
            worker,
            subscriptions,
            ready: false,
        }
    }

    fn received(&mut self, event: Event) -> Result<Option<Publish>, &'static str> {
        match event {
            Event::Incoming(Packet::ConnAck(_)) => {
                self.client
                    .try_subscribe_many(
                        self.subscriptions
                            .iter()
                            .map(|topic| SubscribeFilter::new(topic.clone(), QoS::AtMostOnce)),
                    )
                    .map_err(|_| "Could not subscribe to the broker.")?;
            }
            Event::Incoming(Packet::SubAck(ack)) => {
                if ack.return_codes.len() != self.subscriptions.len()
                    || ack
                        .return_codes
                        .iter()
                        .any(|code| matches!(code, SubscribeReasonCode::Failure))
                {
                    return Err("Broker refused a required subscription; check its ACL.");
                }
                self.ready = true;
            }
            Event::Incoming(Packet::Publish(publish)) if self.ready => return Ok(Some(publish)),
            _ => {}
        }
        Ok(None)
    }

    fn publish(&self, messages: Vec<Publication>) -> Result<(), &'static str> {
        for message in messages {
            // Commands are best effort, never retried. Retained discovery/state
            // uses QoS 1 and can be reconstructed in a new session.
            let qos = if message.retain {
                QoS::AtLeastOnce
            } else {
                QoS::AtMostOnce
            };
            self.client
                .try_publish(message.topic, qos, message.retain, message.payload)
                .map_err(|_| "Broker output queue is full; discarding this session.")?;
        }
        Ok(())
    }
}

/// Runs until a connection/subscription fails. Both workers are dropped on return;
/// retries start from fresh event loops, with no cached or queued commands.
pub async fn session(
    config: Config,
    upstream_password: &Secret,
    ha_password: &Secret,
) -> Result<(), &'static str> {
    let mut bridge = Bridge::new(config);
    let config = &bridge.config;
    let (events, mut incoming) = mpsc::channel(32);
    let mut upstream = Link::new(
        config.diffuser_broker.options(
            format!("scent-ha-{}-devices", config.instance_id),
            upstream_password,
            None,
        )?,
        vec![config.report_topic(), CONNECTED_TOPIC.to_owned()],
        Side::Diffuser,
        events.clone(),
    );
    let will = LastWill::new(
        bridge.availability_topic(),
        "offline",
        QoS::AtLeastOnce,
        true,
    );
    let mut subscriptions = bridge.command_topics();
    subscriptions.push(HA_STATUS.to_owned());
    let mut ha = Link::new(
        config.home_assistant_broker.options(
            format!("scent-ha-{}-home", config.instance_id),
            ha_password,
            Some(will),
        )?,
        subscriptions,
        Side::HomeAssistant,
        events,
    );
    let started = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut advertised = false;
    let mut snapshot: HashMap<String, Vec<u8>> = HashMap::new();
    loop {
        tokio::select! {
            message = incoming.recv() => {
                let (side, event) = message.ok_or("MQTT workers stopped.")?;
                let event = event?;
                let now = started.elapsed().as_secs() as i64;
                match side {
                    Side::Diffuser => {
                        if let Some(message) = upstream.received(event)? {
                            bridge.report(&message.topic, &message.payload, message.retain, now);
                        }
                    }
                    Side::HomeAssistant => {
                        let connected = matches!(event, Event::Incoming(Packet::ConnAck(_)));
                        if let Some(message) = ha.received(event)? {
                            if message.topic == HA_STATUS {
                                if message.payload.as_ref() == b"online" { advertised = false; }
                            } else if upstream.ready && advertised {
                                match bridge.command(&message.topic, &message.payload, message.retain, now) {
                                    Ok(commands) => {
                                        upstream.publish(commands)?;
                                        eprintln!("Home Assistant command sent, awaiting device report.");
                                    }
                                    Err(reason) => eprintln!("{reason}"),
                                }
                            } else { eprintln!("Bridge is not ready; command discarded."); }
                        }
                        if connected { ha.publish(vec![Publication::state(bridge.availability_topic(), "offline")])?; }
                    }
                }
            }
            _ = tick.tick() => {}
        }
        if ha.ready && upstream.ready {
            let now = started.elapsed().as_secs() as i64;
            if !advertised {
                // Clear retained state from previous runs before advertising online.
                let mut messages = bridge.discovery();
                let readings = bridge.snapshot(now);
                snapshot = readings
                    .iter()
                    .map(|m| (m.topic.clone(), m.payload.clone()))
                    .collect();
                messages.extend(readings);
                messages.push(Publication::state(bridge.availability_topic(), "online"));
                ha.publish(messages)?;
                advertised = true;
            }
            let changed = bridge
                .snapshot(now)
                .into_iter()
                .filter(|message| snapshot.get(&message.topic) != Some(&message.payload))
                .collect::<Vec<_>>();
            for message in &changed {
                snapshot.insert(message.topic.clone(), message.payload.clone());
            }
            ha.publish(changed)?;
            upstream.publish(bridge.pings(now))?;
            upstream.publish(bridge.status_requests(now))?;
        }
    }
}
