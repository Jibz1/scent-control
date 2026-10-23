import { describe, expect, test } from "bun:test";

import { parseMqtt } from "../protocol/message.ts";
import { hearMessage, isCurrent, mergeRecords, needsStatus, type DeviceRecord } from "./record.ts";

const NOW = 1_792_510_200;
const USER_TOPIC = "scentiment-mqtt-prod/user/route-test-1";
const CONNECTED = { fw: "2.30", model: "Diffuser", variant: "Black", manufacturer: "01" };

const bytes = (text: string): Uint8Array => new TextEncoder().encode(text);

const alreadyAdded = parseMqtt(
  USER_TOPIC,
  bytes(
    JSON.stringify({
      action: "acknowledgement",
      payload: {
        deviceId: "TESTDEVICE01",
        msg: JSON.stringify({ user_id: "route-test-1", result: "ALREADY_ADDED" }),
      },
    }),
  ),
);

describe("mergeRecords", () => {
  const a: DeviceRecord = {
    heardAt: NOW - 50,
    pingAt: NOW - 30,
    fields: { power: { value: 1, at: NOW - 100 }, intensity: { value: 2, at: NOW - 10 } },
    connected: { ...CONNECTED, at: NOW - 500 },
  };
  const b: DeviceRecord = {
    heardAt: NOW - 20,
    pingAt: NOW - 60,
    askedAt: NOW - 5,
    fields: { power: { value: 0, at: NOW - 40 }, battery: { value: 90, at: NOW - 70 } },
    connected: { ...CONNECTED, fw: "2.31", at: NOW - 200 },
  };

  test("merging keeps the later reading of each field, and the later times", () => {
    expect(mergeRecords(a, b)).toEqual({
      heardAt: NOW - 20,
      pingAt: NOW - 30,
      askedAt: NOW - 5,
      fields: {
        power: { value: 0, at: NOW - 40 },
        intensity: { value: 2, at: NOW - 10 },
        battery: { value: 90, at: NOW - 70 },
      },
      connected: { ...CONNECTED, fw: "2.31", at: NOW - 200 },
    });
  });

  test("merging is the same in either order, and merging twice changes nothing", () => {
    expect(mergeRecords(b, a)).toEqual(mergeRecords(a, b));
    expect(mergeRecords(mergeRecords(a, b), b)).toEqual(mergeRecords(a, b));
    // Two listeners can hold different values from the same second. Unless a tie resolves
    // the same way on both, each would keep republishing its own.
    const one: DeviceRecord = {
      fields: { power: { value: 1, at: NOW } },
      connected: { ...CONNECTED, at: NOW },
    };
    const other: DeviceRecord = {
      fields: { power: { value: 0, at: NOW } },
      connected: { ...CONNECTED, fw: "2.31", at: NOW },
    };
    expect(mergeRecords(one, other)).toEqual(mergeRecords(other, one));
  });

  test("a same-second tie goes to the lower value, so a power tie reads as off", () => {
    const on: DeviceRecord = {
      fields: { power: { value: 1, at: NOW }, intensity: { value: 3, at: NOW } },
    };
    const off: DeviceRecord = {
      fields: { power: { value: 0, at: NOW }, intensity: { value: 1, at: NOW } },
    };
    const expected = { power: { value: 0, at: NOW }, intensity: { value: 1, at: NOW } };
    expect(mergeRecords(on, off).fields).toEqual(expected);
    expect(mergeRecords(off, on).fields).toEqual(expected);
  });
});

describe("hearMessage", () => {
  test("a message with no status moves heardAt and leaves every field as it was", () => {
    const known: DeviceRecord = {
      heardAt: NOW - 100,
      fields: { power: { value: 1, at: NOW - 100 } },
    };
    const acked = hearMessage(known, alreadyAdded, NOW);
    expect(acked.heardAt).toBe(NOW);
    expect(acked.fields).toEqual({ power: { value: 1, at: NOW - 100 } });
    const led = parseMqtt(
      USER_TOPIC,
      bytes('{"action":"notification","payload":{"deviceId":"TESTDEVICE01","data":"led:50"}}'),
    );
    expect(hearMessage(known, led, NOW).fields).toEqual({ power: { value: 1, at: NOW - 100 } });
  });

  test("a report stores each field it carries, and the signal, at the time heard", () => {
    const report = parseMqtt(
      USER_TOPIC,
      bytes(
        '{"action":"update_status","payload":{"deviceId":"TESTDEVICE01","data":{"data":[85,170,11,255,1,2,200,50,255,2,3,255,255,0,92,106]},"rssi":-66}}',
      ),
    );
    expect(hearMessage(undefined, report, NOW)).toEqual({
      heardAt: NOW,
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
  });

  test("a device_connected message is stored with the time heard", () => {
    const connected = parseMqtt(
      "scentiment-mqtt-prod/server-notifications/device",
      bytes(
        '{"action":"device_connected","payload":{"deviceId":"TESTDEVICE01","fw":"2.30","model":"Diffuser","variant":"Black","manufacturer":"01"}}',
      ),
    );
    expect(hearMessage(undefined, connected, NOW).connected).toEqual({ ...CONNECTED, at: NOW });
  });
});

test("a reading from before the last reconnect is not current, one in the same second is", () => {
  const reconnected: DeviceRecord = {
    fields: { power: { value: 1, at: NOW - 10 }, intensity: { value: 2, at: NOW } },
    connected: { ...CONNECTED, at: NOW },
  };
  expect(isCurrent(reconnected, "power")).toBe(false);
  expect(isCurrent(reconnected, "intensity")).toBe(true);
  expect(isCurrent(reconnected, "battery")).toBe(false);
  expect(isCurrent({ fields: reconnected.fields }, "power")).toBe(true);
});

describe("needsStatus", () => {
  const fresh: DeviceRecord = {
    heardAt: NOW - 10,
    fields: { power: { value: 1, at: NOW - 60 }, intensity: { value: 2, at: NOW - 60 } },
  };

  test("an unreachable diffuser is never asked", () => {
    expect(needsStatus({ heardAt: NOW - 76, fields: {} }, NOW)).toBe(false);
    expect(needsStatus({ fields: {} }, NOW)).toBe(false);
  });

  test("a reachable diffuser whose state is current and fresh is not asked", () => {
    expect(needsStatus(fresh, NOW)).toBe(false);
    expect(needsStatus({ ...fresh, heardAt: NOW - 75 }, NOW)).toBe(false);
  });

  test("a reachable diffuser whose power is from before its reconnect is asked", () => {
    expect(needsStatus({ ...fresh, connected: { ...CONNECTED, at: NOW - 30 } }, NOW)).toBe(true);
  });

  test("a reachable diffuser that never reported its intensity is asked", () => {
    expect(
      needsStatus({ heardAt: NOW - 10, fields: { power: { value: 1, at: NOW - 60 } } }, NOW),
    ).toBe(true);
  });

  test("a reachable diffuser whose newest report is more than 300 s old is asked", () => {
    const old = (age: number): DeviceRecord => ({
      heardAt: NOW - 10,
      fields: { power: { value: 1, at: NOW - age }, intensity: { value: 2, at: NOW - age } },
    });
    expect(needsStatus(old(301), NOW)).toBe(true);
    expect(needsStatus(old(300), NOW)).toBe(false);
  });

  test("a diffuser is asked again only 300 s after the last time", () => {
    const unknown: DeviceRecord = { heardAt: NOW - 10, fields: {} };
    expect(needsStatus({ ...unknown, askedAt: NOW - 299 }, NOW)).toBe(false);
    expect(needsStatus({ ...unknown, askedAt: NOW - 300 }, NOW)).toBe(true);
  });
});
