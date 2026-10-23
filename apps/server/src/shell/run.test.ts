import { expect, test } from "bun:test";
import { Effect, Fiber, Layer, Redacted, Stream } from "effect";

import { ListenerConfig } from "./config.ts";
import { fakeBroker } from "./fake-broker.ts";
import { runListener, runListenerWith } from "./run.ts";

const settings = Layer.succeed(ListenerConfig, {
  brokerUrl: "wss://broker.example.test/mqtt",
  username: "someone",
  password: Redacted.make("pw-test-secret"),
  routeId: "route-test-1",
});

const bytes = (text: string): Uint8Array => new TextEncoder().encode(text);

const report = {
  _tag: "Message",
  topic: "scentiment-mqtt-prod/user/route-test-1",
  body: bytes(
    '{"action":"update_status","payload":{"deviceId":"TESTDEVICE01","data":{"data":[85,170,11,255,1,2,200,50,255,2,3,255,255,0,92,106]},"rssi":-66}}',
  ),
} as const;

/** Runs the listener against `broker` for a moment (its first tick runs at once), then stops it. */
const runAgainst = async (broker: ReturnType<typeof fakeBroker>): Promise<void> => {
  const fiber = Effect.runFork(
    runListener.pipe(Effect.provide(Layer.merge(settings, broker.layer))),
  );
  await Bun.sleep(50);
  await Effect.runPromise(Fiber.interrupt(fiber));
};

test("on ConnAck it subscribes to both filters, then publishes the heartbeat", async () => {
  const broker = fakeBroker({ connected: true, events: [{ _tag: "ConnAck" }] });
  await runAgainst(broker);
  const subscribed = broker.ops.findIndex((op) => op.op === "subscribe");
  expect(broker.ops[subscribed]).toEqual({
    op: "subscribe",
    filters: ["scentiment-mqtt-prod/#", "scent-control/v1/#"],
  });
  const heartbeatAfter = broker.ops
    .slice(subscribed + 1)
    .some((op) => op.op === "publish" && op.publish.topic === "scent-control/v1/listener");
  expect(heartbeatAfter).toBe(true);
});

test("a diffuser's report publishes its record", async () => {
  const broker = fakeBroker({ connected: true, events: [{ _tag: "ConnAck" }, report] });
  await runAgainst(broker);
  const record = broker.published().find((p) => p.topic === "scent-control/v1/state/TESTDEVICE01");
  expect(record?.retain).toBe(true);
  expect(JSON.parse(record?.body ?? "{}").fields?.power?.value).toBe(1);
});

test("a tick while disconnected publishes nothing", async () => {
  const offline = fakeBroker({ connected: false, events: [] });
  await runAgainst(offline);
  expect(offline.ops).toEqual([]);
  // The same tick while connected publishes the heartbeat: the tick did run.
  const online = fakeBroker({ connected: true, events: [] });
  await runAgainst(online);
  expect(online.published().map((p) => p.topic)).toEqual(["scent-control/v1/listener"]);
});

test("ticks are handled in turn with the broker's messages, so the last record is the newest", async () => {
  // The tick's ping is slow to be acknowledged, and the diffuser's next report (off) arrives
  // meanwhile. Were they handled side by side, the tick's older record would go out last and
  // the broker would keep "on".
  const off = {
    _tag: "Message",
    topic: "scentiment-mqtt-prod/user/route-test-1",
    body: bytes(
      '{"action":"notification","payload":{"deviceId":"TESTDEVICE01","data":enable:0,"rssi":-66}}',
    ),
  } as const;
  const broker = fakeBroker({
    connected: true,
    events: Stream.concat(
      Stream.make({ _tag: "ConnAck" } as const, report),
      Stream.fromEffect(Effect.as(Effect.sleep("15 millis"), off)),
    ),
    slowPublish: (publish) => (publish.body.includes('"PING"') ? "30 millis" : undefined),
  });
  const fiber = Effect.runFork(
    runListenerWith(Stream.fromEffect(Effect.sleep("5 millis"))).pipe(
      Effect.provide(Layer.merge(settings, broker.layer)),
    ),
  );
  await Bun.sleep(120);
  await Effect.runPromise(Fiber.interrupt(fiber));
  const powers = broker
    .published()
    .filter((p) => p.topic === "scent-control/v1/state/TESTDEVICE01")
    .map((p) => JSON.parse(p.body).fields.power.value);
  expect(powers).toEqual([1, 1, 0]);
});

test("a failed subscribe reconnects, and claims nothing while it can't hear", async () => {
  const broker = fakeBroker({
    connected: true,
    refuseSubscribe: true,
    events: [{ _tag: "ConnAck" }],
  });
  const fiber = Effect.runFork(
    runListenerWith(Stream.empty).pipe(Effect.provide(Layer.merge(settings, broker.layer))),
  );
  await Bun.sleep(50);
  await Effect.runPromise(Fiber.interrupt(fiber));
  expect(broker.ops.map((op) => op.op)).toEqual(["subscribe", "reconnect"]);
});

test("a failed publish does not stop the listener", async () => {
  const broker = fakeBroker({
    connected: true,
    refusePublishes: true,
    events: [{ _tag: "ConnAck" }, report],
  });
  await runAgainst(broker);
  // The heartbeat was refused, and still the report after it was taken in and published.
  const attempted = broker.ops.flatMap((op) => (op.op === "publish" ? [op.publish.topic] : []));
  expect(attempted).toContain("scent-control/v1/listener");
  expect(attempted).toContain("scent-control/v1/state/TESTDEVICE01");
});
