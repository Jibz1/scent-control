import { expect, test } from "bun:test";

import { contractFixture } from "../test-support/fixtures.ts";
import { WILL_BODY, parseState, publishedBy, renderHeartbeat, renderState } from "./payloads.ts";
import type { DeviceRecord } from "./record.ts";

const NOW = 1_792_510_230;

const bytes = (text: string): Uint8Array => new TextEncoder().encode(text);

// The spec's §6.4 example, as the record it renders from.
const example: DeviceRecord = {
  heardAt: 1_792_510_229,
  pingAt: 1_792_510_200,
  fields: {
    power: { value: 1, at: 1_792_510_100 },
    intensity: { value: 2, at: 1_792_510_100 },
    charging: { value: 2, at: 1_792_510_100 },
    battery: { value: 200, at: 1_792_510_100 },
    rssi: { value: -66, at: 1_792_510_100 },
  },
  connected: { fw: "2.30", model: "D", variant: "B", manufacturer: "01", at: 1_792_509_000 },
};

test("the spec's example renders exactly", () => {
  expect(JSON.parse(renderState(example, NOW, "0a1b2c3d"))).toEqual(
    JSON.parse(contractFixture("state-v1.json")),
  );
});

test("a record names the listener that published it, and reading it back ignores the name", () => {
  const body = renderState({ fields: {} }, NOW, "0a1b2c3d");
  expect(JSON.parse(body).by).toBe("0a1b2c3d");
  expect(publishedBy(bytes(body))).toBe("0a1b2c3d");
  expect(parseState(bytes(body), NOW)).toEqual({ fields: {} });
  expect(publishedBy(bytes(`{"v":1,"updated_at":${NOW},"fields":{}}`))).toBeUndefined();
  expect(publishedBy(bytes("not json"))).toBeUndefined();
});

test("what is never known is left out, and when it asked is never published", () => {
  expect(JSON.parse(renderState({ askedAt: NOW, fields: {} }, NOW, "0a1b2c3d"))).toEqual({
    v: 1,
    by: "0a1b2c3d",
    updated_at: NOW,
    fields: {},
  });
});

test("the state fixture parses back to the same record", () => {
  expect(parseState(bytes(contractFixture("state-v1.json")), NOW)).toEqual(example);
});

test("the heartbeat and will match their fixtures", () => {
  expect(JSON.parse(renderHeartbeat(1_792_510_230, 1_792_500_000))).toEqual(
    JSON.parse(contractFixture("heartbeat-v1.json")),
  );
  expect(JSON.parse(WILL_BODY)).toEqual(JSON.parse(contractFixture("will-v1.json")));
});

test("state of another version, from more than a day ahead, empty, or misshapen is not usable", () => {
  const state = (overrides: Record<string, unknown>): Uint8Array =>
    bytes(JSON.stringify({ v: 1, updated_at: NOW, fields: {}, ...overrides }));
  expect(parseState(state({}), NOW)).toEqual({ fields: {} });
  for (const unusable of [
    state({ v: 2 }),
    state({ updated_at: NOW + 86_401 }),
    state({ heard_at: NOW + 86_401 }),
    state({ fields: { power: { value: 1, at: NOW + 86_401 } } }),
    state({ connected: { fw: "", model: "", variant: "", manufacturer: "", at: NOW + 86_401 } }),
    bytes(""),
    bytes("not json"),
    bytes("[]"),
    state({ fields: undefined }),
    state({ updated_at: "now" }),
    state({ heard_at: 1.5 }),
    state({ fields: { power: { value: "1", at: NOW } } }),
    state({ fields: { power: 1 } }),
    state({ connected: { fw: 2.3, model: "", variant: "", manufacturer: "", at: NOW } }),
  ]) {
    expect(parseState(unusable, NOW)).toBeUndefined();
  }
  expect(parseState(state({ updated_at: NOW + 86_400 }), NOW)).toEqual({ fields: {} });
});

test("field keys it does not know are dropped", () => {
  const body = JSON.stringify({
    v: 1,
    updated_at: NOW,
    fields: { power: { value: 1, at: NOW }, smell: { value: 3, at: NOW }, odd: "x" },
  });
  expect(parseState(bytes(body), NOW)).toEqual({ fields: { power: { value: 1, at: NOW } } });
});
