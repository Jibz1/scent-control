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
import { parseState, renderHeartbeat, renderState } from "./payloads.ts";
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

const statePublish = (id: string, record: DeviceRecord, now: number): Publish | undefined => {
  const topic = stateTopic(id);
  return topic === undefined ? undefined : retainedPublish(topic, renderState(record, now));
};

/** On every connect: the heartbeat, then every record, in case the broker lost them. */
export const onConnAck = (memory: Memory, now: number, startedAt: number): readonly Publish[] =>
  defined([heartbeat(now, startedAt), ...byId(memory).map(([id, r]) => statePublish(id, r, now))]);

/**
 * A message under our own prefix: a record (ours, or another listener's during a deploy) or
 * a stored schedule. A record is merged in, and the merge is published only when it differs
 * from what arrived, so our own record coming back publishes nothing.
 */
const onOwnMessage = (memory: Memory, topic: string, body: Uint8Array, now: number): Step => {
  const stateId = stateTopicDevice(topic);
  if (stateId !== undefined) {
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
        : defined([statePublish(stateId, merged, now)]),
    };
  }
  const scheduleId = scheduleTopicDevice(topic);
  if (scheduleId !== undefined && body.length > 0 && !memory.has(scheduleId)) {
    return { memory: withRecord(memory, scheduleId, EMPTY_RECORD), publishes: [] };
  }
  return { memory, publishes: [] };
};

/** One message from the broker, at `now`. */
export const onMessage = (memory: Memory, topic: string, body: Uint8Array, now: number): Step => {
  if (topic.startsWith(APP_PREFIX)) {
    return onOwnMessage(memory, topic, body, now);
  }
  const message = parseMqtt(topic, body);
  const id = message.deviceId;
  if (message.direction === "toDevice" || id === undefined || !isValidDeviceId(id)) {
    return { memory, publishes: [] };
  }
  const record = hearMessage(memory.get(id), message, now);
  return {
    memory: withRecord(memory, id, record),
    publishes: defined([statePublish(id, record, now)]),
  };
};

/**
 * Every 30 s: the heartbeat, then for each known diffuser a ping, a status request if it
 * needs one (judged before this tick's changes), and its updated record.
 */
export const onTick = (memory: Memory, now: number, routeId: string, startedAt: number): Step => {
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
    publishes.push(...defined([statePublish(id, updated, now)]));
  }
  return { memory: next, publishes };
};
