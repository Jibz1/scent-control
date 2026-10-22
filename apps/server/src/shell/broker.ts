// The connection to the broker, over MQTT.js. Everything the listener sends goes through
// `publish`, which refuses while disconnected so that nothing is queued and flushed later.
import { Context, Effect, Queue, Redacted, Schema, Stream } from "effect";
import { connect } from "mqtt";

import type { Publish } from "../listener/gates.ts";
import { WILL_BODY } from "../listener/payloads.ts";
import { randomHex8 } from "../probe.ts";
import { LISTENER_TOPIC } from "../protocol/topics.ts";
import { ListenerConfig } from "./config.ts";

export type BrokerEvent =
  | { readonly _tag: "ConnAck" }
  | { readonly _tag: "Message"; readonly topic: string; readonly body: Uint8Array }
  | { readonly _tag: "Closed" };

/** Why the broker didn't take something: a fixed phrase, never the library's own message. */
export class BrokerError extends Schema.TaggedError<BrokerError>()("BrokerError", {
  reason: Schema.String,
}) {}

export interface BrokerContract {
  readonly events: Stream.Stream<BrokerEvent>;
  readonly isConnected: Effect.Effect<boolean>;
  readonly subscribe: (filters: readonly string[]) => Effect.Effect<void, BrokerError>;
  readonly publish: (publish: Publish) => Effect.Effect<void, BrokerError>;
}

const RECONNECT_MS = 5000;

const make = Effect.gen(function* () {
  const settings = yield* ListenerConfig;
  const events = yield* Queue.unbounded<BrokerEvent>();
  const client = yield* Effect.acquireRelease(
    Effect.sync(() => {
      const created = connect(settings.brokerUrl, {
        protocolVersion: 4,
        clean: true,
        clientId: `scent-control-listener-${randomHex8()}`,
        username: settings.username,
        password: Redacted.value(settings.password),
        reconnectPeriod: RECONNECT_MS,
        // MQTT.js's Node path needs ws.createWebSocketStream, which Bun's built-in `ws`
        // lacks; Bun's own WebSocket works.
        forceNativeWebSocket: true,
        will: { topic: LISTENER_TOPIC, payload: WILL_BODY, qos: 1, retain: true },
      });
      // The listeners go on before the first connect can happen, so no event is missed.
      created.on("connect", () => Queue.offerUnsafe(events, { _tag: "ConnAck" }));
      created.on("message", (topic, payload) =>
        Queue.offerUnsafe(events, { _tag: "Message", topic, body: new Uint8Array(payload) }),
      );
      created.on("close", () => Queue.offerUnsafe(events, { _tag: "Closed" }));
      created.on("error", () => {
        // A refused or failed connect is reported here, then closes; the close is what counts.
      });
      return created;
    }),
    // A clean DISCONNECT, so the broker sends no will: a listener replacing this one during a
    // deploy has already published its own heartbeat.
    (created) => Effect.promise(() => created.endAsync(false)),
  );

  const subscribe = (filters: readonly string[]): Effect.Effect<void, BrokerError> =>
    Effect.tryPromise({
      try: () => client.subscribeAsync([...filters], { qos: 1 }),
      catch: () => new BrokerError({ reason: "subscribe failed" }),
    }).pipe(Effect.asVoid);

  const publish = (message: Publish): Effect.Effect<void, BrokerError> =>
    Effect.suspend(() =>
      client.connected
        ? Effect.tryPromise({
            try: () =>
              client.publishAsync(message.topic, message.body, {
                qos: message.qos,
                retain: message.retain,
              }),
            catch: () => new BrokerError({ reason: "publish failed" }),
          }).pipe(Effect.asVoid)
        : Effect.fail(new BrokerError({ reason: "not connected" })),
    );

  const broker: BrokerContract = {
    events: Stream.fromQueue(events),
    isConnected: Effect.sync(() => client.connected),
    subscribe,
    publish,
  };
  return broker;
});

export class Broker extends Context.Service<Broker, BrokerContract>()("Broker", { make }) {}
