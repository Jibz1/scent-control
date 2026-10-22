// Reading what the diffusers send: a line-for-line port of scent-core's message.rs, which
// is the specification. The two must read every message the same way.
import { DEVICE_PREFIX } from "./topics.ts";

// Status array positions. Indices 0-3 are a header; 7, 10 and 13 are read by nobody (7 is
// not the oil level, as once assumed).
const POWER = 4;
const CHARGING = 5;
const BATTERY = 6;
const FAN = 8;
const INTENSITY = 9;
const GAS_PUMP = 11;
const LIGHT = 12;

type Mutable<T> = { -readonly [K in keyof T]: T[K] };

/** What a diffuser reported about itself. A field is absent when the message didn't carry it. */
export interface Status {
  readonly power?: number;
  readonly intensity?: number;
  readonly charging?: number;
  readonly battery?: number;
  readonly fan?: number;
  readonly gasPump?: number;
  readonly light?: number;
}

/**
 * The shape of a notification: `S:1,L:3,C:2,Bat:200`; `{"data":[85,170,11,255,...]}`; one
 * `key:value` pair such as `enable:1` or `led:50`; valid JSON that is not a status array.
 */
export type NoteKind = "statusText" | "statusArray" | "value" | "json" | "unknown";

/** A notification as the device sent it, plus what could be read from it. */
export interface Notification {
  /** The text without the `-|` terminator or surrounding whitespace. */
  readonly raw: string;
  readonly kind: NoteKind;
  readonly status: Status;
}

/** Which way an MQTT message was going: `toDevice` when published to a `device/<id>` topic. */
export type Direction = "toDevice" | "fromDevice";

/** The result of a `PING` with a user ID. */
export type Ack =
  | { readonly kind: "added" }
  | { readonly kind: "alreadyAdded" }
  | { readonly kind: "other"; readonly result: string };

/** A diffuser announcing that it joined the broker. */
export interface Connected {
  readonly fw: string;
  readonly model: string;
  readonly variant: string;
  readonly manufacturer: string;
}

/** One MQTT message, read as far as it can be. */
export interface MqttMessage {
  readonly direction: Direction;
  readonly action?: string;
  /** From the payload's `deviceId`, else from a `device/<id>` topic. Not checked. */
  readonly deviceId?: string;
  /** Set when the payload's `data` held something readable. It can be empty. */
  readonly status?: Status;
  readonly rssi?: number;
  /** Only for `acknowledgement` messages. */
  readonly ack?: Ack;
  /** Only for `device_connected` messages. */
  readonly connected?: Connected;
  /** False when the body was not valid JSON as received. It may still have been repaired. */
  readonly validJson: boolean;
  /** The body as text, exactly as received (bad bytes replaced). */
  readonly raw: string;
}

type Json = { readonly ok: true; readonly value: unknown } | { readonly ok: false };

const parseJson = (text: string): Json => {
  try {
    return { ok: true, value: JSON.parse(text) };
  } catch {
    return { ok: false };
  }
};

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const isInteger = (value: unknown): value is number =>
  typeof value === "number" && Number.isInteger(value);

const asciiLower = (text: string): string => text.replaceAll(/[A-Z]/gu, (c) => c.toLowerCase());

/** Rust's `text.trim().parse::<i64>()`: an optional sign and digits, nothing else. */
const numberOf = (text: string): number | undefined => {
  const trimmed = text.trim();
  return /^[+-]?\d+$/u.test(trimmed) ? Number(trimmed) : undefined;
};

/** Reads one notification. Never fails: anything unreadable is `unknown`. */
export const parseNotification = (text: string): Notification => {
  let raw = text.trim();
  if (raw.endsWith("-|")) {
    raw = raw.slice(0, -"-|".length).trim();
  }
  const [kind, status] = classify(raw);
  return { raw, kind, status };
};

const classify = (text: string): [NoteKind, Status] => {
  if (text.startsWith("{") || text.startsWith("[")) {
    const json = parseJson(text);
    if (json.ok) {
      const data = isObject(json.value) ? json.value["data"] : undefined;
      return Array.isArray(data) ? ["statusArray", statusFromArray(data)] : ["json", {}];
    }
  }
  const fromText = statusFromText(text);
  if (fromText !== undefined) {
    return ["statusText", fromText];
  }
  const fromValue = statusFromValue(text);
  if (fromValue !== undefined) {
    return ["value", fromValue];
  }
  return ["unknown", {}];
};

const statusFromArray = (array: readonly unknown[]): Status => {
  const status: Mutable<Status> = {};
  const fields: [keyof Status, number][] = [
    ["power", POWER],
    ["intensity", INTENSITY],
    ["charging", CHARGING],
    ["battery", BATTERY],
    ["fan", FAN],
    ["gasPump", GAS_PUMP],
    ["light", LIGHT],
  ];
  for (const [field, index] of fields) {
    const value = array[index];
    if (isInteger(value)) {
      status[field] = value;
    }
  }
  return status;
};

const TEXT_KEYS = new Map<string, keyof Status>([
  ["s", "power"],
  ["l", "intensity"],
  ["c", "charging"],
  ["bat", "battery"],
]);

/** `S:1,L:3,C:2,Bat:200`: at least two pairs, every key one we know. */
const statusFromText = (text: string): Status | undefined => {
  const status: Mutable<Status> = {};
  let pairs = 0;
  for (const part of text.split(",")) {
    const colon = part.indexOf(":");
    if (colon === -1) {
      return undefined;
    }
    const field = TEXT_KEYS.get(asciiLower(part.slice(0, colon).trim()));
    if (field === undefined) {
      return undefined;
    }
    const value = numberOf(part.slice(colon + 1));
    if (value === undefined) {
      delete status[field];
    } else {
      status[field] = value;
    }
    pairs += 1;
  }
  return pairs > 1 ? status : undefined;
};

const VALUE_KEYS = new Map<string, keyof Status>([
  ["enable", "power"],
  ["level", "intensity"],
  ["battery", "battery"],
]);

/** One `key:value` pair. Keys we don't map (like `led`) still make it a value. */
const statusFromValue = (text: string): Status | undefined => {
  const colon = text.indexOf(":");
  if (colon === -1) {
    return undefined;
  }
  const key = asciiLower(text.slice(0, colon).trim());
  if (key === "") {
    return undefined;
  }
  const field = VALUE_KEYS.get(key);
  const value = numberOf(text.slice(colon + 1));
  return field !== undefined && value !== undefined ? { [field]: value } : {};
};

/**
 * Reads one MQTT message. Never fails: unreadable bodies give a message with nothing in it
 * but `raw` and `validJson: false`.
 *
 * Firmware 2.30 sends notifications with `"data":enable:0` (no quotes). That is repaired by
 * quoting the value and parsing again.
 */
export const parseMqtt = (topic: string, body: Uint8Array): MqttMessage => {
  // Like Rust's from_utf8_lossy: bad bytes become U+FFFD, and a leading BOM is kept.
  const raw = new TextDecoder("utf-8", { ignoreBOM: true }).decode(body);
  const direct = parseJson(raw);
  const message: Mutable<MqttMessage> = {
    direction: topic.startsWith(DEVICE_PREFIX) ? "toDevice" : "fromDevice",
    validJson: direct.ok,
    raw,
  };
  if (topic.startsWith(DEVICE_PREFIX)) {
    message.deviceId = topic.slice(DEVICE_PREFIX.length);
  }
  const document = direct.ok ? direct : parseJson(quoteUnquotedData(raw));
  if (!document.ok || !isObject(document.value)) {
    return message;
  }
  const action = textOf(document.value, "action");
  if (action !== undefined) {
    message.action = action;
  }
  const { payload } = document.value;
  if (!isObject(payload)) {
    return message;
  }
  const id = textOf(payload, "deviceId");
  if (id !== undefined && id !== "") {
    message.deviceId = id;
  }
  const { rssi } = payload;
  if (isInteger(rssi)) {
    message.rssi = rssi;
  }
  const status = statusFromData(payload["data"]);
  if (status !== undefined) {
    message.status = status;
  }
  if (action === "acknowledgement") {
    const ack = ackFrom(payload);
    if (ack !== undefined) {
      message.ack = ack;
    }
  } else if (action === "device_connected") {
    message.connected = connectedFrom(payload);
  }
  return message;
};

const textOf = (object: Record<string, unknown>, key: string): string | undefined => {
  const value = object[key];
  return typeof value === "string" ? value : undefined;
};

/** `data` as notification-style text: the text itself, or, on some firmware, the array's JSON object. */
const dataText = (data: unknown): string | undefined => {
  if (typeof data === "string") {
    return data;
  }
  return isObject(data) ? JSON.stringify(data) : undefined;
};

const statusFromData = (data: unknown): Status | undefined => {
  const text = dataText(data);
  if (text === undefined) {
    return undefined;
  }
  const note = parseNotification(text);
  return note.kind === "unknown" ? undefined : note.status;
};

/** The acknowledgement's `msg` is itself JSON text: `{"user_id":"...","result":"ADDED"}`. */
const ackFrom = (payload: Record<string, unknown>): Ack | undefined => {
  const msg = textOf(payload, "msg");
  if (msg === undefined) {
    return undefined;
  }
  const parsed = parseJson(msg);
  if (!parsed.ok || !isObject(parsed.value)) {
    return undefined;
  }
  const result = textOf(parsed.value, "result");
  switch (result) {
    case undefined: {
      return undefined;
    }
    case "ADDED": {
      return { kind: "added" };
    }
    case "ALREADY_ADDED": {
      return { kind: "alreadyAdded" };
    }
    default: {
      return { kind: "other", result };
    }
  }
};

/** Fields the device left out come back empty: the connect itself is what matters. */
const connectedFrom = (payload: Record<string, unknown>): Connected => ({
  fw: textOf(payload, "fw") ?? "",
  model: textOf(payload, "model") ?? "",
  variant: textOf(payload, "variant") ?? "",
  manufacturer: textOf(payload, "manufacturer") ?? "",
});

const DATA_KEY = '"data":';

/**
 * Puts quotes around every `"data":` value that doesn't start with `"`, `{` or `[`. The value
 * runs to the next `,` or `}`. Meant for text that failed to parse as JSON.
 */
const quoteUnquotedData = (text: string): string => {
  let out = "";
  let rest = text;
  for (let at = rest.indexOf(DATA_KEY); at !== -1; at = rest.indexOf(DATA_KEY)) {
    out += rest.slice(0, at + DATA_KEY.length);
    rest = rest.slice(at + DATA_KEY.length);
    const [first] = rest;
    if (first !== undefined && first !== '"' && first !== "{" && first !== "[") {
      const ends = [rest.indexOf(","), rest.indexOf("}")].filter((i) => i !== -1);
      const end = ends.length > 0 ? Math.min(...ends) : rest.length;
      out += JSON.stringify(rest.slice(0, end).trim());
      rest = rest.slice(end);
    }
  }
  return out + rest;
};
