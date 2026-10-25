import { describe, expect, test } from "bun:test";

import type { DeviceRecord, Memory } from "./record.ts";
import { onConnAck, onMessage, onTick } from "./steps.ts";

const NOW = 1_792_510_200;
const STARTED = NOW - 1000;
const USER_TOPIC = "scentiment-mqtt-prod/user/example-route";
const STATE_01 = "scent-control/v1/state/TESTDEVICE01";
const STATE_02 = "scent-control/v1/state/TESTDEVICE02";
/** This listener's instance tag, and another listener's. */
const SELF = "0a1b2c3d";
const OTHER = "4e5f6a7b";

const bytes = (text: string): Uint8Array => new TextEncoder().encode(text);
const memoryOf = (entries: [string, DeviceRecord][]): Memory => new Map(entries);

const report = bytes(
  '{"action":"update_status","payload":{"deviceId":"TESTDEVICE01","data":{"data":[85,170,11,255,1,2,200,50,255,2,3,255,255,0,92,106]},"rssi":-66}}',
);

const fresh: DeviceRecord = {
  heardAt: NOW - 10,
  fields: { power: { value: 1, at: NOW - 60 }, intensity: { value: 2, at: NOW - 60 } },
};

describe("onConnAck", () => {
  test("on ConnAck the heartbeat comes first, then every record", () => {
    const memory = memoryOf([
      ["TESTDEVICE02", { fields: {} }],
      ["TESTDEVICE01", fresh],
    ]);
    const publishes = onConnAck(memory, NOW, STARTED, SELF);
    expect(publishes.map((p) => [p.topic, p.qos, p.retain])).toEqual([
      ["scent-control/v1/listener", 1, true],
      [STATE_01, 1, true],
      [STATE_02, 1, true],
    ]);
    expect(JSON.parse(publishes[0]?.body ?? "")).toEqual({
      v: 1,
      running: true,
      at: NOW,
      started_at: STARTED,
    });
    expect(JSON.parse(publishes[2]?.body ?? "")).toEqual({
      v: 1,
      by: SELF,
      updated_at: NOW,
      fields: {},
    });
  });
});

describe("onMessage", () => {
  test("a report publishes that diffuser's state, retained", () => {
    const step = onMessage(new Map(), USER_TOPIC, report, NOW, SELF);
    expect(step.publishes.map((p) => [p.topic, p.qos, p.retain])).toEqual([[STATE_01, 1, true]]);
    expect(JSON.parse(step.publishes[0]?.body ?? "")).toEqual({
      v: 1,
      by: SELF,
      updated_at: NOW,
      heard_at: NOW,
      fields: {
        power: { value: 1, at: NOW },
        intensity: { value: 2, at: NOW },
        charging: { value: 2, at: NOW },
        battery: { value: 200, at: NOW },
        fan: { value: 255, at: NOW },
        gas_pump: { value: 255, at: NOW },
        light: { value: 255, at: NOW },
        rssi: { value: -66, at: NOW },
      },
    });
    expect(step.memory.get("TESTDEVICE01")?.heardAt).toBe(NOW);
  });

  test("a command to a diffuser changes nothing", () => {
    const memory = memoryOf([["TESTDEVICE01", fresh]]);
    const step = onMessage(
      memory,
      "scentiment-mqtt-prod/device/TESTDEVICE01",
      bytes('{"action":"TURN_ON","payload":{"on":1}}'),
      NOW,
      SELF,
    );
    expect(step.publishes).toEqual([]);
    expect(step.memory).toEqual(memory);
  });

  test("an invalid device ID changes nothing", () => {
    const step = onMessage(
      new Map(),
      USER_TOPIC,
      bytes('{"action":"update_status","payload":{"deviceId":"testdevice01","data":"enable:1"}}'),
      NOW,
      SELF,
    );
    expect(step.publishes).toEqual([]);
    expect(step.memory.size).toBe(0);
  });

  test("a stored schedule makes its diffuser known without a publish", () => {
    const topic = "scent-control/v1/schedule/TESTDEVICE01";
    const step = onMessage(new Map(), topic, bytes('{"v":1,"profile":{},"sent_at":1}'), NOW, SELF);
    expect(step.publishes).toEqual([]);
    expect(step.memory.get("TESTDEVICE01")).toEqual({ fields: {} });
    // An empty retained message takes a stored schedule off the broker: it says nothing.
    expect(onMessage(new Map(), topic, new Uint8Array(), NOW, SELF).memory.size).toBe(0);
    // A known diffuser stays as it was.
    const known = memoryOf([["TESTDEVICE01", fresh]]);
    expect(onMessage(known, topic, bytes("{}"), NOW, SELF).memory.get("TESTDEVICE01")).toEqual(
      fresh,
    );
  });

  test("our own record coming back publishes nothing", () => {
    const heard = onMessage(new Map(), USER_TOPIC, report, NOW, SELF);
    const [published] = heard.publishes;
    expect(published).toBeDefined();
    const echo = onMessage(
      heard.memory,
      published?.topic ?? "",
      bytes(published?.body ?? ""),
      NOW + 1,
      SELF,
    );
    expect(echo.publishes).toEqual([]);
    expect(echo.memory.get("TESTDEVICE01")).toEqual(heard.memory.get("TESTDEVICE01"));
  });

  test("an echo of our earlier record, heard after a newer report from the same second, changes nothing", () => {
    const enable = (on: 0 | 1, rssi: number): Uint8Array =>
      bytes(
        `{"action":"notification","payload":{"deviceId":"TESTDEVICE01","data":enable:${on},"rssi":${rssi}}}`,
      );
    const turnedOn = onMessage(new Map(), USER_TOPIC, enable(1, -46), NOW, SELF);
    const turnedOff = onMessage(turnedOn.memory, USER_TOPIC, enable(0, -66), NOW, SELF);
    const [earlier] = turnedOn.publishes;
    const [later] = turnedOff.publishes;
    const lastReported = {
      power: { value: 0, at: NOW },
      rssi: { value: -66, at: NOW },
    };
    const earlierBack = onMessage(
      turnedOff.memory,
      STATE_01,
      bytes(earlier?.body ?? ""),
      NOW,
      SELF,
    );
    expect(earlierBack.publishes).toEqual([]);
    expect(earlierBack.memory.get("TESTDEVICE01")?.fields).toEqual(lastReported);
    const laterBack = onMessage(earlierBack.memory, STATE_01, bytes(later?.body ?? ""), NOW, SELF);
    expect(laterBack.publishes).toEqual([]);
    expect(laterBack.memory.get("TESTDEVICE01")?.fields).toEqual(lastReported);
  });

  test("another listener's record from the same second can't turn a diffuser back on", () => {
    const ours = memoryOf([["TESTDEVICE01", { fields: { power: { value: 0, at: NOW } } }]]);
    const theirs = memoryOf([["TESTDEVICE01", { fields: { power: { value: 1, at: NOW } } }]]);
    const [theirRecord] = onConnAck(theirs, NOW, STARTED, OTHER).filter(
      (p) => p.topic === STATE_01,
    );
    const step = onMessage(ours, STATE_01, bytes(theirRecord?.body ?? ""), NOW, SELF);
    expect(step.memory.get("TESTDEVICE01")?.fields.power).toEqual({ value: 0, at: NOW });
    expect(step.publishes.map((p) => JSON.parse(p.body).fields.power.value)).toEqual([0]);
  });

  test("two listeners converge in one exchange", () => {
    const a = memoryOf([
      ["TESTDEVICE01", { heardAt: NOW - 10, fields: { power: { value: 1, at: NOW - 10 } } }],
    ]);
    const b = memoryOf([
      ["TESTDEVICE01", { heardAt: NOW - 5, fields: { intensity: { value: 2, at: NOW - 5 } } }],
    ]);
    const aPublishes = onConnAck(a, NOW, STARTED, OTHER).filter((p) => p.topic === STATE_01);
    const atB = onMessage(b, STATE_01, bytes(aPublishes[0]?.body ?? ""), NOW, SELF);
    expect(atB.publishes).toHaveLength(1);
    const atA = onMessage(a, STATE_01, bytes(atB.publishes[0]?.body ?? ""), NOW, OTHER);
    expect(atA.publishes).toEqual([]);
    expect(atA.memory.get("TESTDEVICE01")).toEqual(atB.memory.get("TESTDEVICE01"));
    expect(atA.memory.get("TESTDEVICE01")).toEqual({
      heardAt: NOW - 5,
      fields: { power: { value: 1, at: NOW - 10 }, intensity: { value: 2, at: NOW - 5 } },
    });
  });

  test("an unusable record changes nothing", () => {
    const memory = memoryOf([["TESTDEVICE01", fresh]]);
    const step = onMessage(memory, STATE_01, bytes('{"v":2}'), NOW, SELF);
    expect(step.publishes).toEqual([]);
    expect(step.memory).toEqual(memory);
  });
});

describe("onTick", () => {
  const memory = memoryOf([
    ["TESTDEVICE02", { heardAt: NOW - 10, fields: { power: { value: 1, at: NOW - 60 } } }],
    ["TESTDEVICE01", fresh],
  ]);

  test("a tick pings every known diffuser and asks only those that need it", () => {
    const step = onTick(memory, NOW, "example-route", STARTED, SELF);
    expect(step.publishes.map((p) => [p.topic, p.body, p.retain])).toEqual([
      [
        "scent-control/v1/listener",
        JSON.stringify({ v: 1, running: true, at: NOW, started_at: STARTED }),
        true,
      ],
      [
        "scentiment-mqtt-prod/device/TESTDEVICE01",
        '{"action":"PING","payload":{"userid":"example-route"}}',
        false,
      ],
      [STATE_01, expect.any(String), true],
      [
        "scentiment-mqtt-prod/device/TESTDEVICE02",
        '{"action":"PING","payload":{"userid":"example-route"}}',
        false,
      ],
      ["scentiment-mqtt-prod/device/TESTDEVICE02", '{"action":"GET_STATUS","payload":{}}', false],
      [STATE_02, expect.any(String), true],
    ]);
    expect(step.memory.get("TESTDEVICE01")).toEqual({ ...fresh, pingAt: NOW });
    expect(step.memory.get("TESTDEVICE02")?.askedAt).toBe(NOW);
    expect(JSON.parse(step.publishes[2]?.body ?? "").ping_at).toBe(NOW);
  });

  test("a tick asks again only after 300 s", () => {
    const asked = onTick(memory, NOW, "example-route", STARTED, SELF).memory;
    const stillHeard = (at: number): Memory =>
      new Map(
        [...asked].map(([id, record]): [string, DeviceRecord] => [id, { ...record, heardAt: at }]),
      );
    // Only TESTDEVICE02 was asked at NOW. (By NOW + 299 TESTDEVICE01's readings are stale, so
    // it is rightly asked too.)
    const asks = (step: { publishes: readonly { topic: string; body: string }[] }): number =>
      step.publishes.filter(
        (p) =>
          p.topic === "scentiment-mqtt-prod/device/TESTDEVICE02" && p.body.includes("GET_STATUS"),
      ).length;
    expect(asks(onTick(stillHeard(NOW + 298), NOW + 299, "example-route", STARTED, SELF))).toBe(0);
    expect(asks(onTick(stillHeard(NOW + 299), NOW + 300, "example-route", STARTED, SELF))).toBe(1);
  });
});
