import { expect, test } from "bun:test";

import type { ListenerCommand } from "../protocol/commands.ts";
import { commandPublish, retainedPublish } from "./gates.ts";

test("nothing is stored retained outside our own prefix", () => {
  // The Mac's list (apps/desktop/src/mqtt.rs, nothing_is_stored_retained_outside_our_own_prefix).
  for (const foreign of [
    "scentiment-mqtt-prod/device/TESTDEVICE01",
    "scentiment-mqtt-prod/user/example-route",
    "scentiment-mqtt-prod/#",
    "",
    "scent-control/v1",
    "scent-control/v1/",
    "scent-control/v2/schedule/TESTDEVICE01",
    "Scent-Control/v1/schedule/TESTDEVICE01",
    "/scent-control/v1/schedule/TESTDEVICE01",
    "scent-control/v1/#",
    "scent-control/v1/schedule/+",
    "scent-control/v1/schedule/TESTDEVICE01/#",
  ]) {
    expect(retainedPublish(foreign, "{}")).toBeUndefined();
  }
});

test("a state topic is stored retained at QoS 1", () => {
  expect(retainedPublish("scent-control/v1/state/TESTDEVICE01", "{}")).toEqual({
    topic: "scent-control/v1/state/TESTDEVICE01",
    body: "{}",
    qos: 1,
    retain: true,
  });
});

test("commands go only to valid device topics, never retained", () => {
  expect(commandPublish("TESTDEVICE01", { action: "GET_STATUS" })).toEqual({
    topic: "scentiment-mqtt-prod/device/TESTDEVICE01",
    body: '{"action":"GET_STATUS","payload":{}}',
    qos: 1,
    retain: false,
  });
  for (const bad of ["", "testdevice01", "TESTDEVICE0", "TESTDEVIC/#", "+", "TESTDEVICE01/x"]) {
    expect(commandPublish(bad, { action: "GET_STATUS" })).toBeUndefined();
  }
});

test("PING needs a route ID", () => {
  expect(commandPublish("TESTDEVICE01", { action: "PING", routeId: "" })).toBeUndefined();
  expect(commandPublish("TESTDEVICE01", { action: "PING", routeId: "  " })).toBeUndefined();
  expect(commandPublish("TESTDEVICE01", { action: "PING", routeId: "example-route" })?.body).toBe(
    '{"action":"PING","payload":{"userid":"example-route"}}',
  );
});

test("the only bodies a command can carry are PING and GET_STATUS", () => {
  const allowed = [
    '{"action":"PING","payload":{"userid":"example-route"}}',
    '{"action":"GET_STATUS","payload":{}}',
  ];
  const commands: ListenerCommand[] = [
    { action: "PING", routeId: "example-route" },
    { action: "GET_STATUS" },
  ];
  for (const command of commands) {
    const publish = commandPublish("TESTDEVICE01", command);
    expect(allowed).toContain(publish?.body ?? "");
    expect(publish?.retain).toBe(false);
  }
});
