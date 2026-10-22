// What the listener knows about each diffuser, and the rules for combining it. Plain
// functions: no I/O, no clock of their own. Times are whole seconds since 1970.
import type { Connected, MqttMessage, Status } from "../protocol/message.ts";
import { ONLINE_FOR_S, STALE_AFTER_S } from "../protocol/timing.ts";

/** The readings a record can hold, in the order they are published. */
export const FIELD_KEYS = [
  "power",
  "intensity",
  "charging",
  "battery",
  "fan",
  "gas_pump",
  "light",
  "rssi",
] as const;

export type FieldKey = (typeof FIELD_KEYS)[number];

/** One value a diffuser reported, as it sent it, with when the listener heard it. */
export interface FieldReading {
  readonly value: number;
  readonly at: number;
}

/** What a diffuser said about itself when it last joined the broker, and when. */
export interface ConnectedInfo extends Connected {
  readonly at: number;
}

export interface DeviceRecord {
  /** When the listener last heard anything from it. */
  readonly heardAt?: number;
  /** When the listener last pinged it. */
  readonly pingAt?: number;
  /** When the listener last asked it for its status. Kept here only, never published. */
  readonly askedAt?: number;
  readonly fields: Readonly<Partial<Record<FieldKey, FieldReading>>>;
  readonly connected?: ConnectedInfo;
}

/** Every known diffuser's record, by device ID. */
export type Memory = ReadonlyMap<string, DeviceRecord>;

/** A diffuser known, with nothing heard from it yet. */
export const EMPTY_RECORD: DeviceRecord = { fields: {} };

const STATUS_FIELDS: readonly (readonly [keyof Status, FieldKey])[] = [
  ["power", "power"],
  ["intensity", "intensity"],
  ["charging", "charging"],
  ["battery", "battery"],
  ["fan", "fan"],
  ["gasPump", "gas_pump"],
  ["light", "light"],
];

interface RecordParts {
  readonly heardAt?: number | undefined;
  readonly pingAt?: number | undefined;
  readonly askedAt?: number | undefined;
  readonly fields: Readonly<Partial<Record<FieldKey, FieldReading>>>;
  readonly connected?: ConnectedInfo | undefined;
}

/** A record with only the parts that are there. */
const recordOf = (parts: RecordParts): DeviceRecord => {
  const record: { -readonly [K in keyof DeviceRecord]: DeviceRecord[K] } = { fields: parts.fields };
  if (parts.heardAt !== undefined) {
    record.heardAt = parts.heardAt;
  }
  if (parts.pingAt !== undefined) {
    record.pingAt = parts.pingAt;
  }
  if (parts.askedAt !== undefined) {
    record.askedAt = parts.askedAt;
  }
  if (parts.connected !== undefined) {
    record.connected = parts.connected;
  }
  return record;
};

const latest = (a: number | undefined, b: number | undefined): number | undefined => {
  if (a === undefined) {
    return b;
  }
  return b === undefined ? a : Math.max(a, b);
};

// A tie (the same second) must go the same way whichever listener merges, or two of them
// would keep republishing their own copy to each other.
const laterReading = (
  a: FieldReading | undefined,
  b: FieldReading | undefined,
): FieldReading | undefined => {
  if (a === undefined || b === undefined) {
    return a ?? b;
  }
  if (a.at !== b.at) {
    return a.at > b.at ? a : b;
  }
  return a.value >= b.value ? a : b;
};

const connectedKey = (c: ConnectedInfo): string =>
  [c.fw, c.model, c.variant, c.manufacturer].join("\u0000");

const laterConnected = (
  a: ConnectedInfo | undefined,
  b: ConnectedInfo | undefined,
): ConnectedInfo | undefined => {
  if (a === undefined || b === undefined) {
    return a ?? b;
  }
  if (a.at !== b.at) {
    return a.at > b.at ? a : b;
  }
  return connectedKey(a) >= connectedKey(b) ? a : b;
};

/** Newer wins, field by field. The same result in either order, and merging twice changes nothing. */
export const mergeRecords = (a: DeviceRecord, b: DeviceRecord): DeviceRecord => {
  const fields: Partial<Record<FieldKey, FieldReading>> = {};
  for (const key of FIELD_KEYS) {
    const reading = laterReading(a.fields[key], b.fields[key]);
    if (reading !== undefined) {
      fields[key] = reading;
    }
  }
  return recordOf({
    heardAt: latest(a.heardAt, b.heardAt),
    pingAt: latest(a.pingAt, b.pingAt),
    askedAt: latest(a.askedAt, b.askedAt),
    fields,
    connected: laterConnected(a.connected, b.connected),
  });
};

const published = (record: DeviceRecord): string =>
  JSON.stringify([
    record.heardAt ?? null,
    record.pingAt ?? null,
    FIELD_KEYS.map((key) => {
      const reading = record.fields[key];
      return reading === undefined ? null : [reading.value, reading.at];
    }),
    record.connected === undefined ? null : [connectedKey(record.connected), record.connected.at],
  ]);

/** Whether the two would be published the same: everything but `askedAt`. */
export const samePublished = (a: DeviceRecord, b: DeviceRecord): boolean =>
  published(a) === published(b);

/**
 * Whether `key` holds a reading that still counts: one reported no earlier than the
 * diffuser's last reconnect (owner, 2026-10-22). A reading from the same second counts.
 */
export const isCurrent = (record: DeviceRecord, key: FieldKey): boolean => {
  const reading = record.fields[key];
  if (reading === undefined) {
    return false;
  }
  return record.connected === undefined || reading.at >= record.connected.at;
};

/** Takes in one message from the diffuser, heard at `at`. Fields it doesn't carry stay as they were. */
export const hearMessage = (
  record: DeviceRecord | undefined,
  message: MqttMessage,
  at: number,
): DeviceRecord => {
  const known = record ?? EMPTY_RECORD;
  const fields: Partial<Record<FieldKey, FieldReading>> = { ...known.fields };
  for (const [statusKey, fieldKey] of STATUS_FIELDS) {
    const value = message.status?.[statusKey];
    if (value !== undefined) {
      fields[fieldKey] = { value, at };
    }
  }
  if (message.rssi !== undefined) {
    fields.rssi = { value: message.rssi, at };
  }
  return recordOf({
    ...known,
    heardAt: at,
    fields,
    connected: message.connected === undefined ? known.connected : { ...message.connected, at },
  });
};

/**
 * Whether to ask the diffuser for its status now: the Home Assistant adapter's rule. Only a
 * reachable one, at most once every `STALE_AFTER_S`, and only when its power or intensity is
 * not known or its newest report of them is older than `STALE_AFTER_S`.
 */
export const needsStatus = (record: DeviceRecord, now: number): boolean => {
  const reachable = record.heardAt !== undefined && now - record.heardAt <= ONLINE_FOR_S;
  const recently = record.askedAt !== undefined && now - record.askedAt < STALE_AFTER_S;
  if (!reachable || recently) {
    return false;
  }
  const unknown = !isCurrent(record, "power") || !isCurrent(record, "intensity");
  const times = [record.fields.power?.at, record.fields.intensity?.at].filter(
    (at): at is number => at !== undefined,
  );
  const stale = times.length === 0 || now - Math.max(...times) > STALE_AFTER_S;
  return unknown || stale;
};
