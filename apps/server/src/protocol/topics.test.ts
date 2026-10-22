import { expect, test } from "bun:test";

import {
  APP_PREFIX,
  DEVICE_PREFIX,
  LISTENER_TOPIC,
  SCHEDULE_PREFIX,
  STATE_PREFIX,
  SUBSCRIBE_ALL,
  SUBSCRIBE_APP,
  deviceTopic,
  isValidDeviceId,
  scheduleTopicDevice,
  stateTopic,
  stateTopicDevice,
} from "./topics.ts";

// The same values as scent-core's topics.rs: the listener and the Mac must agree on them.
test("topics match scent-core's constants", () => {
  expect(DEVICE_PREFIX).toBe("scentiment-mqtt-prod/device/");
  expect(SUBSCRIBE_ALL).toBe("scentiment-mqtt-prod/#");
  expect(APP_PREFIX).toBe("scent-control/v1/");
  expect(SUBSCRIBE_APP).toBe("scent-control/v1/#");
  expect(SCHEDULE_PREFIX).toBe("scent-control/v1/schedule/");
  expect(STATE_PREFIX).toBe("scent-control/v1/state/");
  expect(LISTENER_TOPIC).toBe("scent-control/v1/listener");
  expect(deviceTopic("TESTDEVICE01")).toBe("scentiment-mqtt-prod/device/TESTDEVICE01");
});

test("only 12 upper-case letters and digits are device IDs", () => {
  for (const id of ["TESTDEVICE01", "0A1B2C3D4E5F"]) {
    expect(isValidDeviceId(id)).toBe(true);
  }
  for (const id of [
    "",
    "TESTDEVICE0",
    "TESTDEVICE001",
    "testdevice01",
    "TESTDEVICE+1",
    "TESTDEVIC/#",
  ]) {
    expect(isValidDeviceId(id)).toBe(false);
  }
});

test("state topics are built only for valid IDs and read back only exactly", () => {
  expect(stateTopic("TESTDEVICE01")).toBe("scent-control/v1/state/TESTDEVICE01");
  expect(stateTopic("TESTDEVIC/#")).toBeUndefined();
  expect(stateTopicDevice("scent-control/v1/state/TESTDEVICE01")).toBe("TESTDEVICE01");
  for (const topic of [
    "scent-control/v1/state/TESTDEVICE01/x",
    "scent-control/v1/state/",
    "scent-control/v1/schedule/TESTDEVICE01",
  ]) {
    expect(stateTopicDevice(topic)).toBeUndefined();
  }
  expect(scheduleTopicDevice("scent-control/v1/schedule/TESTDEVICE01")).toBe("TESTDEVICE01");
  for (const topic of [
    "scent-control/v1/schedule/TESTDEVICE01/x",
    "scent-control/v1/schedule/",
    "scent-control/v1/state/TESTDEVICE01",
  ]) {
    expect(scheduleTopicDevice(topic)).toBeUndefined();
  }
});
