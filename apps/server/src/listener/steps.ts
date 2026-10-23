// What the listener does on each thing that happens: given what it knows and the time, the
// new memory and the exact publishes to make. The shell only carries these out.
import { parseMqtt } from "../protocol/message.ts";
import {
  APP_PREFIX,
  LISTENER_TOPIC,
  isValidDeviceId,
  scheduleTopicDevice,
  stateTopic,
  stateTopicDevice,
} from "../protocol/topics.ts";
import { commandPublish, retainedPublish, type Publish } from "./gates.ts";
import { parseState, publishedBy, renderHeartbeat, renderState } from "./payloads.ts";
import {
  EMPTY_RECORD,
  hearMessage,
  mergeRecords,
  needsStatus,
  samePublished,
  type DeviceRecord,
  type Memory,
} from "./record.ts";

export interface Step {
  readonly memory: Memory;
  readonly publishes: readonly Publish[];
}

const defined = <T>(items: readonly (T | undefined)[]): T[] =>
  items.filter((item): item is T => item !== undefined);

const byId = (memory: Memory): [string, DeviceRecord][] =>
  [...memory].toSorted(([a], [b]) => (a < b ? -1 : 1));

const withRecord = (memory: Memory, id: string, record: DeviceRecord): Memory =>
  new Map(memory).set(id, record);

const heartbeat = (now: number, startedAt: number): Publish | undefined =>
  retainedPublish(LISTENER_TOPIC, renderHeartbeat(now, startedAt));

const statePublish = (
  id: string,
  record: DeviceRecord,
  now: number,
  self: string,
): Publish | undefined => {
  const topic = stateTopic(id);
  return topic === undefined ? undefined : retainedPublish(topic, renderState(record, now, self));
};

/**
 * On every connect: the heartbeat, then every record, in case the broker lost them. `self`
 * is this listener's instance tag, put on every record it publishes.
 */
export const onConnAck = (
  memory: Memory,
  now: number,
  startedAt: number,
  self: string,
): readonly Publish[] =>
  defined([
    heartbeat(now, startedAt),
    ...byId(memory).map(([id, r]) => statePublish(id, r, now, self)),
  ]);

/**
 * A message under our own prefix: a record or a stored schedule. Our own record coming back
 * changes nothing: what we hold is never older than what we published, and an older copy of
 * it, heard after a newer report from the same second, must not undo that report. Another
 * listener's record (during a deploy) is merged in, and the merge is published only when it
 * differs from what arrived.
 */
const onOwnMessage = (
  memory: Memory,
  topic: string,
  body: Uint8Array,
  now: number,
  self: string,
): Step => {
  const stateId = stateTopicDevice(topic);
  if (stateId !== undefined) {
    if (publishedBy(body) === self) {
      return { memory, publishes: [] };
    }
    const arrived = parseState(body, now);
    if (arrived === undefined) {
      return { memory, publishes: [] };
    }
    const known = memory.get(stateId);
    const merged = known === undefined ? arrived : mergeRecords(known, arrived);
    return {
      memory: withRecord(memory, stateId, merged),
      publishes: samePublished(merged, arrived)
        ? []
        : defined([statePublish(stateId, merged, now, self)]),
    };
  }
  const scheduleId = scheduleTopicDevice(topic);
  if (scheduleId !== undefined && body.length > 0 && !memory.has(scheduleId)) {
    return { memory: withRecord(memory, scheduleId, EMPTY_RECORD), publishes: [] };
  }
  return { memory, publishes: [] };
};

/** One message from the broker, at `now`. */
export const onMessage = (
  memory: Memory,
  topic: string,
  body: Uint8Array,
  now: number,
  self: string,
): Step => {
  if (topic.startsWith(APP_PREFIX)) {
    return onOwnMessage(memory, topic, body, now, self);
  }
  const message = parseMqtt(topic, body);
  const id = message.deviceId;
  if (message.direction === "toDevice" || id === undefined || !isValidDeviceId(id)) {
    return { memory, publishes: [] };
  }
  const record = hearMessage(memory.get(id), message, now);
  return {
    memory: withRecord(memory, id, record),
    publishes: defined([statePublish(id, record, now, self)]),
  };
};

/**
 * Every 30 s: the heartbeat, then for each known diffuser a ping, a status request if it
 * needs one (judged before this tick's changes), and its updated record.
 */
export const onTick = (
  memory: Memory,
  now: number,
  routeId: string,
  startedAt: number,
  self: string,
): Step => {
  const publishes = defined([heartbeat(now, startedAt)]);
  const next = new Map(memory);
  for (const [id, record] of byId(memory)) {
    const ask = needsStatus(record, now);
    let updated = record;
    const ping = commandPublish(id, { action: "PING", routeId });
    if (ping !== undefined) {
      publishes.push(ping);
      updated = { ...updated, pingAt: now };
    }
    const status = ask ? commandPublish(id, { action: "GET_STATUS" }) : undefined;
    if (status !== undefined) {
      publishes.push(status);
      updated = { ...updated, askedAt: now };
    }
    next.set(id, updated);
    publishes.push(...defined([statePublish(id, updated, now, self)]));
  }
  return { memory: next, publishes };
};
