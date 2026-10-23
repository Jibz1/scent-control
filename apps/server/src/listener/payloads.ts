// The messages the listener publishes, exactly as the spec (§6.4, §6.5) and
// fixtures/scent-control/ define them.
import { isInteger, isObject, parseJson } from "../protocol/json.ts";
import { DAY_AHEAD_S } from "../protocol/timing.ts";
import {
  FIELD_KEYS,
  type ConnectedInfo,
  type DeviceRecord,
  type FieldKey,
  type FieldReading,
} from "./record.ts";

const STATE_VERSION = 1;

/** The listener's last will: the broker publishes it, retained, if the listener drops off. */
export const WILL_BODY = JSON.stringify({ v: 1, running: false });

/** The heartbeat, published retained on every ConnAck and then every 30 s. */
export const renderHeartbeat = (at: number, startedAt: number): string =>
  JSON.stringify({ v: 1, running: true, at, started_at: startedAt });

/** A diffuser's record as published. What was never known is left out; `askedAt` always is. */
export const renderState = (record: DeviceRecord, updatedAt: number): string => {
  const fields: Partial<Record<FieldKey, FieldReading>> = {};
  for (const key of FIELD_KEYS) {
    const reading = record.fields[key];
    if (reading !== undefined) {
      fields[key] = { value: reading.value, at: reading.at };
    }
  }
  const { connected } = record;
  // JSON.stringify leaves out the keys whose value is undefined.
  return JSON.stringify({
    v: STATE_VERSION,
    updated_at: updatedAt,
    heard_at: record.heardAt,
    ping_at: record.pingAt,
    fields,
    connected:
      connected === undefined
        ? undefined
        : {
            fw: connected.fw,
            model: connected.model,
            variant: connected.variant,
            manufacturer: connected.manufacturer,
            at: connected.at,
          },
  });
};

const readFields = (
  raw: Record<string, unknown>,
  usable: (time: unknown) => time is number,
): Partial<Record<FieldKey, FieldReading>> | undefined => {
  const fields: Partial<Record<FieldKey, FieldReading>> = {};
  for (const key of FIELD_KEYS) {
    const entry = raw[key];
    if (entry !== undefined) {
      if (!isObject(entry) || !isInteger(entry["value"]) || !usable(entry["at"])) {
        return undefined;
      }
      fields[key] = { value: entry["value"], at: entry["at"] };
    }
  }
  return fields;
};

const readConnected = (
  raw: unknown,
  usable: (time: unknown) => time is number,
): ConnectedInfo | undefined => {
  if (!isObject(raw)) {
    return undefined;
  }
  const { fw, model, variant, manufacturer, at } = raw;
  const texts = [fw, model, variant, manufacturer];
  if (!texts.every((text) => typeof text === "string") || !usable(at)) {
    return undefined;
  }
  return {
    fw: String(fw),
    model: String(model),
    variant: String(variant),
    manufacturer: String(manufacturer),
    at,
  };
};

/**
 * A published record read back, or `undefined` when it can't be used: empty, not JSON,
 * another version, the wrong shape, or with any time more than a day ahead of `now`. Field
 * keys it doesn't know are dropped.
 */
export const parseState = (body: Uint8Array, now: number): DeviceRecord | undefined => {
  const json = parseJson(new TextDecoder().decode(body));
  if (!json.ok || !isObject(json.value)) {
    return undefined;
  }
  const state = json.value;
  const usable = (time: unknown): time is number => isInteger(time) && time <= now + DAY_AHEAD_S;
  const absentOrUsable = (time: unknown): boolean => time === undefined || usable(time);
  if (
    state["v"] !== STATE_VERSION ||
    !usable(state["updated_at"]) ||
    !absentOrUsable(state["heard_at"]) ||
    !absentOrUsable(state["ping_at"]) ||
    !isObject(state["fields"])
  ) {
    return undefined;
  }
  const fields = readFields(state["fields"], usable);
  if (fields === undefined) {
    return undefined;
  }
  const record: { -readonly [K in keyof DeviceRecord]: DeviceRecord[K] } = { fields };
  if (isInteger(state["heard_at"])) {
    record.heardAt = state["heard_at"];
  }
  if (isInteger(state["ping_at"])) {
    record.pingAt = state["ping_at"];
  }
  if (state["connected"] !== undefined) {
    const connected = readConnected(state["connected"], usable);
    if (connected === undefined) {
      return undefined;
    }
    record.connected = connected;
  }
  return record;
};
