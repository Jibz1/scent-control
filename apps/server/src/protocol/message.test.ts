// The same cases as scent-core: its fixture contract (tests/fixtures.rs) and the parser's
// own tests (message.rs). The two parsers must read every message the same way.
import { describe, expect, test } from "bun:test";

import { fixture, onChannel } from "../test-support/fixtures.ts";
import { isValidDeviceId } from "./topics.ts";
import { parseMqtt, parseNotification, type MqttMessage, type NoteKind } from "./message.ts";

const USER_TOPIC = "scentiment-mqtt-prod/user/route-test-1";
const SYNTHETIC_IDS = ["TESTDEVICE01", "TESTDEVICE02"];

const bytes = (text: string): Uint8Array => new TextEncoder().encode(text);

/** What a ble-notify fixture's name says it holds. */
const expectedKind = (name: string): NoteKind => {
  if (name.includes("status-array")) {
    return "statusArray";
  }
  return name.includes("status-text") ? "statusText" : "value";
};

const parseFixture = (name: string): MqttMessage => {
  const f = fixture(name);
  if (f.topic === undefined) {
    throw new Error(`${name}: no topic`);
  }
  return parseMqtt(f.topic, bytes(f.text));
};

describe("the captured fixtures", () => {
  test("every mqtt fixture parses with an action and a synthetic device ID", () => {
    const mqtt = onChannel("mqtt");
    expect(mqtt.length).toBeGreaterThanOrEqual(7);
    for (const f of mqtt) {
      const m = parseMqtt(f.topic ?? "", bytes(f.text));
      expect(m.action).toBeDefined();
      expect(SYNTHETIC_IDS).toContain(m.deviceId ?? "");
      expect(isValidDeviceId(m.deviceId ?? "")).toBe(true);
      expect(m.direction).toBe("fromDevice");
      expect(m.raw).toBe(f.text);
      // The device's own invalid JSON is the only message that needs repair.
      expect(m.validJson).toBe(f.name !== "air2-mqtt-notification-invalid-json");
    }
  });

  test("the invalid JSON notification is repaired", () => {
    const m = parseFixture("air2-mqtt-notification-invalid-json");
    expect(m.validJson).toBe(false);
    expect(m.action).toBe("notification");
    expect(m.status?.power).toBe(0);
    expect(m.rssi).toBe(-46);
  });

  test("the status object fixture gives a status", () => {
    const m = parseFixture("air2-mqtt-update-status-object");
    expect([m.status?.power, m.status?.intensity, m.status?.battery]).toEqual([1, 2, 200]);
    expect(m.rssi).toBe(-66);
  });

  test("the ack fixtures carry their result", () => {
    expect(parseFixture("air2-mqtt-ack-added").ack).toEqual({ kind: "added" });
    expect(parseFixture("air2-mqtt-ack-already-added").ack).toEqual({ kind: "alreadyAdded" });
  });

  test("the device connected fixtures carry model information", () => {
    for (const name of [
      "air2-mqtt-device-connected-server-notifications",
      "air2-mqtt-device-connected-user",
      "hvac-mqtt-device-connected-user",
    ]) {
      const { connected } = parseFixture(name);
      expect(connected?.fw).toBe("2.30");
      expect(connected?.variant).toBe("Black");
    }
    expect(parseFixture("hvac-mqtt-device-connected-user").connected?.model).toBe("Hvac");
  });

  test("ble notify fixtures are recognised", () => {
    const notify = onChannel("ble-notify");
    expect(notify.length).toBeGreaterThanOrEqual(7);
    for (const f of notify) {
      expect(parseNotification(f.text).kind).toBe(expectedKind(f.name));
    }
  });
});

describe("parseNotification", () => {
  test("status text reads the four keys", () => {
    const n = parseNotification("S:1,L:3,C:2,Bat:200-|");
    expect(n.kind).toBe("statusText");
    expect(n.raw).toBe("S:1,L:3,C:2,Bat:200");
    expect(n.status).toEqual({ power: 1, intensity: 3, charging: 2, battery: 200 });
  });

  test("a status array from Scent Assistant issue 42", () => {
    const n = parseNotification('{"data":[85,170,11,255,0,2,200,100,255,0,0,255,255,0,48,106]}');
    expect(n.kind).toBe("statusArray");
    expect(n.status).toEqual({
      power: 0,
      charging: 2,
      battery: 200,
      fan: 255,
      intensity: 0,
      gasPump: 255,
      light: 255,
    });
  });

  test("status index 7 is not read as oil", () => {
    const n = parseNotification('{"data":[85,170,11,255,1,2,200,50,255,2,3,255,255,0,92,106]}');
    expect(n.status).toEqual({
      power: 1,
      charging: 2,
      battery: 200,
      fan: 255,
      intensity: 2,
      gasPump: 255,
      light: 255,
    });
  });

  test("a short array reads only the indices present", () => {
    const n = parseNotification('{"data":[85,170,11,255,1,0,90]}');
    expect(n.kind).toBe("statusArray");
    expect(n.status).toEqual({ power: 1, charging: 0, battery: 90 });
  });

  test("array entries that are not integers are skipped", () => {
    const n = parseNotification('{"data":[85,170,11,255,"x",1.5,90]}');
    expect(n.kind).toBe("statusArray");
    expect(n.status).toEqual({ battery: 90 });
  });

  test("single values map to status fields", () => {
    const power = parseNotification("enable:1");
    expect(power.kind).toBe("value");
    expect(power.status.power).toBe(1);
    expect(parseNotification("level:2").status.intensity).toBe(2);
    expect(parseNotification(" battery:80 ").status.battery).toBe(80);
    expect(parseNotification("ENABLE:0").status.power).toBe(0);
  });

  test("values without a status field are still values", () => {
    const n = parseNotification("led:50");
    expect(n.kind).toBe("value");
    expect(n.status).toEqual({});
    expect(parseNotification("enable:soon").status).toEqual({});
  });

  test("JSON without a data array is JSON", () => {
    expect(parseNotification('{"other":1}')).toMatchObject({ kind: "json", status: {} });
    expect(parseNotification("[1,2]").kind).toBe("json");
    expect(parseNotification('{"data":"x"}').kind).toBe("json");
  });

  test("anything else is unknown, never an error", () => {
    const cases: [string, NoteKind][] = [
      [new TextDecoder().decode(new Uint8Array([255, 254])), "unknown"],
      ["", "unknown"],
      ["-|", "unknown"],
      ["hello", "unknown"],
      ["{not json", "unknown"],
      ["S:1,", "value"],
      [":5", "unknown"],
    ];
    for (const [text, kind] of cases) {
      expect(parseNotification(text).kind).toBe(kind);
    }
    expect(parseNotification("hello").raw).toBe("hello");
  });
});

describe("parseMqtt", () => {
  test("repairs the unquoted notification data", () => {
    // Exactly what an Air 2 (firmware 2.30) sent on 2026-10-20 in reply to GET_STATUS.
    const raw =
      '{"action":"notification","payload":{"deviceId":"TESTDEVICE01","data":enable:0,"rssi":-46}}';
    const m = parseMqtt(USER_TOPIC, bytes(raw));
    expect(m.action).toBe("notification");
    expect(m.deviceId).toBe("TESTDEVICE01");
    expect(m.validJson).toBe(false);
    expect(m.rssi).toBe(-46);
    expect(m.status?.power).toBe(0);
    expect(m.raw).toBe(raw);
  });

  test("reads a status array inside update_status", () => {
    const raw = String.raw`{"action":"update_status","payload":{"deviceId":"TESTDEVICE01","data":"{\"data\":[85,170,11,255,1,2,200,100,255,3,3,255,255,0,41,106]}"}}`;
    const m = parseMqtt(USER_TOPIC, bytes(raw));
    expect(m.validJson).toBe(true);
    expect([m.status?.power, m.status?.intensity]).toEqual([1, 3]);
  });

  test("reads a status array given as an object", () => {
    // Firmware 2.30 over MQTT sends the array as an object, not a string (observed 2026-10-20).
    const raw =
      '{"action":"update_status","payload":{"deviceId":"TESTDEVICE01","data":{"data":[85,170,11,255,1,2,200,50,255,2,3,255,255,0,92,106]},"rssi":-66}}';
    const m = parseMqtt(USER_TOPIC, bytes(raw));
    expect(m.validJson).toBe(true);
    expect([m.status?.power, m.status?.intensity]).toEqual([1, 2]);
    expect(m.rssi).toBe(-66);
  });

  test("messages on a device topic are going to the device", () => {
    const m = parseMqtt(
      "scentiment-mqtt-prod/device/TESTDEVICE01",
      bytes('{"action":"PING","payload":{"userid":"route-test-1"}}'),
    );
    expect(m.direction).toBe("toDevice");
    expect(m.deviceId).toBe("TESTDEVICE01");
    expect(m.action).toBe("PING");
    expect(m.status).toBeUndefined();
  });

  test("the payload's device ID wins over the topic's, unless it is empty", () => {
    const topic = "scentiment-mqtt-prod/device/TESTDEVICE01";
    expect(
      parseMqtt(topic, bytes('{"action":"X","payload":{"deviceId":"TESTDEVICE02"}}')).deviceId,
    ).toBe("TESTDEVICE02");
    expect(parseMqtt(topic, bytes('{"action":"X","payload":{"deviceId":""}}')).deviceId).toBe(
      "TESTDEVICE01",
    );
  });

  test("reads device_connected", () => {
    const m = parseMqtt(
      "scentiment-mqtt-prod/server-notifications/device",
      bytes(
        '{"action":"device_connected","payload":{"deviceId":"TESTDEVICE01","fw":"2.30","model":"Diffuser","variant":"Black","manufacturer":"01"}}',
      ),
    );
    expect(m.direction).toBe("fromDevice");
    expect(m.deviceId).toBe("TESTDEVICE01");
    expect(m.connected).toEqual({
      fw: "2.30",
      model: "Diffuser",
      variant: "Black",
      manufacturer: "01",
    });
  });

  test("reads acknowledgement results", () => {
    const ack = (result: string) => {
      const msg = JSON.stringify({ user_id: "route-test-1", result });
      const body = JSON.stringify({
        action: "acknowledgement",
        payload: { deviceId: "TESTDEVICE01", msg },
      });
      return parseMqtt(USER_TOPIC, bytes(body)).ack;
    };
    expect(ack("ADDED")).toEqual({ kind: "added" });
    expect(ack("ALREADY_ADDED")).toEqual({ kind: "alreadyAdded" });
    expect(ack("NOPE")).toEqual({ kind: "other", result: "NOPE" });
    for (const payload of [
      '{"msg":"not json"}',
      String.raw`{"msg":"{\"user_id\":\"x\"}"}`,
      '{"msg":5}',
      "{}",
    ]) {
      const body = `{"action":"acknowledgement","payload":${payload}}`;
      expect(parseMqtt(USER_TOPIC, bytes(body)).ack).toBeUndefined();
    }
    const elsewhere = parseMqtt(
      USER_TOPIC,
      bytes(String.raw`{"action":"other","payload":{"msg":"{\"result\":\"ADDED\"}"}}`),
    );
    expect(elsewhere.ack).toBeUndefined();
  });

  test("bodies that are not messages give raw text only", () => {
    for (const body of [
      new Uint8Array([255, 254]),
      bytes(""),
      bytes("{"),
      bytes("null"),
      bytes('{"action":5}'),
      bytes('{"payload":{"data":{"data":"x"}}}'),
    ]) {
      const m = parseMqtt(USER_TOPIC, body);
      expect(m.raw.length > 0 || body.length === 0).toBe(true);
    }
    const m = parseMqtt("anything", new Uint8Array([255, ...bytes(" not json at all")]));
    expect([m.action, m.status, m.deviceId]).toEqual([undefined, undefined, undefined]);
    expect(m.validJson).toBe(false);
    expect(m.raw.length).toBeGreaterThan(0);
  });

  test("a 256 KiB unreadable body gives raw text only", () => {
    const body = new Uint8Array(256 * 1024).fill(123); // "{{{{…"
    const m = parseMqtt(USER_TOPIC, body);
    expect(m.validJson).toBe(false);
    expect([m.action, m.deviceId, m.status]).toEqual([undefined, undefined, undefined]);
    expect(m.raw.length).toBe(256 * 1024);
  });
});
