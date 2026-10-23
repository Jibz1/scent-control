import { expect, test } from "bun:test";

import type { BrokerEvent } from "./broker.ts";
import { STARTING, noteEvent, type LogLine, type Outage } from "./outage.ts";

const lines = (events: readonly BrokerEvent[]): LogLine[] => {
  let outage: Outage = STARTING;
  const logged: LogLine[] = [];
  for (const event of events) {
    const noted = noteEvent(outage, event);
    outage = noted.outage;
    if (noted.line !== undefined) {
      logged.push(noted.line);
    }
  }
  return logged;
};

const badLogin: BrokerEvent = { _tag: "Error", kind: "bad username or password" };
const refused: BrokerEvent = { _tag: "Error", kind: "network (ECONNREFUSED)" };
const closed: BrokerEvent = { _tag: "Closed" };
const connAck: BrokerEvent = { _tag: "ConnAck" };

test("an outage is logged once: its first disconnect, and each kind of error once", () => {
  // A wrong password retried every 5 s, then the network going away, then a connect.
  expect(lines([badLogin, closed, badLogin, closed, refused, closed, badLogin, closed])).toEqual([
    { level: "warning", text: "Connecting failed: bad username or password." },
    { level: "warning", text: "Disconnected from the broker; reconnecting." },
    { level: "warning", text: "Connecting failed: network (ECONNREFUSED)." },
  ]);
});

test("every connect is logged, and starts a new outage", () => {
  expect(lines([connAck, closed, badLogin, closed, connAck, closed, badLogin])).toEqual([
    { level: "info", text: "Connected to the broker." },
    { level: "warning", text: "Disconnected from the broker; reconnecting." },
    { level: "warning", text: "Connecting failed: bad username or password." },
    { level: "info", text: "Connected to the broker." },
    { level: "warning", text: "Disconnected from the broker; reconnecting." },
    { level: "warning", text: "Connecting failed: bad username or password." },
  ]);
});

test("messages log nothing", () => {
  expect(lines([connAck, { _tag: "Message", topic: "t", body: new Uint8Array() }])).toEqual([
    { level: "info", text: "Connected to the broker." },
  ]);
});
