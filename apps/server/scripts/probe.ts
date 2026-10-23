// Checks the broker from this machine with MQTT.js, the client the listener uses.
//
//   bun run probe                 publish a probe and wait for it to come back
//   bun run probe store <hex>     leave a retained probe on the broker
//   bun run probe check <hex>     look for that retained probe
//   bun run probe clear <hex>     take it off again
//
// The broker settings come from the environment (see README.md). Nothing it prints
// carries a setting, a topic or an error's own text: only what happened.
import { connectAsync, type MqttClient } from "mqtt";

import { probeTopic, randomHex8 } from "../src/probe.ts";

const WAIT_MS = 10_000;
const BODY = '{"v":1,"probe":true}';

const fail = (kind: string): never => {
  console.log(`error: ${kind}`);
  process.exit(1);
};

const setting = (name: string): string => {
  const value = process.env[name]?.trim();
  return value || fail(`missing ${name}`);
};

/** The first message on `topic` that `accept` takes, or `null` after `WAIT_MS`. */
const firstMessage = (
  client: MqttClient,
  topic: string,
  accept: (body: Buffer) => boolean,
): Promise<Buffer | null> =>
  new Promise((resolve) => {
    const timer = setTimeout(() => resolve(null), WAIT_MS);
    client.on("message", (arrived, body) => {
      if (arrived === topic && accept(body)) {
        clearTimeout(timer);
        resolve(body);
      }
    });
  });

const [mode = "round-trip", name] = process.argv.slice(2);
if (!["round-trip", "store", "check", "clear"].includes(mode)) {
  fail("unknown mode");
}
if (mode !== "round-trip" && name === undefined) {
  fail("missing probe name");
}
const topic = probeTopic(name ?? randomHex8());

const client = await connectAsync(setting("SCENT_BROKER_URL"), {
  protocolVersion: 4,
  clean: true,
  clientId: `scent-control-probe-${randomHex8()}`,
  username: setting("SCENT_BROKER_USERNAME"),
  password: setting("SCENT_BROKER_PASSWORD"),
  connectTimeout: WAIT_MS,
  reconnectPeriod: 0,
  // MQTT.js's Node path wraps the `ws` package with createWebSocketStream, which Bun's
  // built-in `ws` doesn't support ("Not supported yet in Bun"). Bun's own WebSocket works.
  forceNativeWebSocket: true,
}).catch(() => fail("connect failed"));
console.log("connected");

try {
  switch (mode) {
    case "round-trip": {
      const back = firstMessage(client, topic, (body) => body.toString() === BODY);
      await client.subscribeAsync(topic, { qos: 1 });
      const sent = Date.now();
      await client.publishAsync(topic, BODY, { qos: 1, retain: false });
      if ((await back) === null) {
        fail("timed out");
      }
      console.log(`came back in ${Date.now() - sent} ms`);
      break;
    }
    case "store": {
      await client.publishAsync(topic, BODY, { qos: 1, retain: true });
      console.log("stored");
      break;
    }
    case "check": {
      const found = firstMessage(client, topic, (body) => body.length > 0);
      await client.subscribeAsync(topic, { qos: 1 });
      console.log((await found) === null ? "not found" : "found");
      break;
    }
    case "clear": {
      await client.publishAsync(topic, "", { qos: 1, retain: true });
      console.log("cleared");
      break;
    }
  }
} catch {
  fail("broker refused");
} finally {
  await client.endAsync();
}
