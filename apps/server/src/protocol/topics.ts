// Topics, as in scent-core's topics.rs.

/** Commands to a diffuser are published to this prefix plus its device ID. */
export const DEVICE_PREFIX = "scentiment-mqtt-prod/device/";

/** Everything under Scentiment's prefix: what the diffusers say, and what is sent to them. */
export const SUBSCRIBE_ALL = "scentiment-mqtt-prod/#";

/**
 * Where Scent Control keeps what it stores on the broker. Never under Scentiment's prefix:
 * the diffusers subscribe there, and a retained message there would be replayed to a
 * diffuser every time it connects.
 */
export const APP_PREFIX = "scent-control/v1/";

/** Everything under our own prefix. */
export const SUBSCRIBE_APP = `${APP_PREFIX}#`;

/** The retained copy of the last schedule sent to a diffuser, plus its device ID. */
export const SCHEDULE_PREFIX = `${APP_PREFIX}schedule/`;

/** The listener's retained record of a diffuser, plus its device ID. */
export const STATE_PREFIX = `${APP_PREFIX}state/`;

/** The listener's retained heartbeat, and its will. */
export const LISTENER_TOPIC = `${APP_PREFIX}listener`;

const DEVICE_ID = /^[0-9A-Z]{12}$/u;

/** A device ID is 12 characters, each an upper-case letter or a digit. */
export const isValidDeviceId = (id: string): boolean => DEVICE_ID.test(id);

/**
 * The topic to publish a command to. Check the ID with `isValidDeviceId` first; this does
 * not, so an ID with `/`, `+` or `#` would build a different topic.
 */
export const deviceTopic = (id: string): string => `${DEVICE_PREFIX}${id}`;

/** A diffuser's state topic, or `undefined` when `id` is not a device ID. */
export const stateTopic = (id: string): string | undefined =>
  isValidDeviceId(id) ? `${STATE_PREFIX}${id}` : undefined;

const deviceAfter = (prefix: string, topic: string): string | undefined => {
  if (!topic.startsWith(prefix)) {
    return undefined;
  }
  const id = topic.slice(prefix.length);
  return isValidDeviceId(id) ? id : undefined;
};

/** The device ID a state topic is for, or `undefined` when the topic is not one. */
export const stateTopicDevice = (topic: string): string | undefined =>
  deviceAfter(STATE_PREFIX, topic);

/** The device ID a stored schedule's topic is for, or `undefined` when the topic is not one. */
export const scheduleTopicDevice = (topic: string): string | undefined =>
  deviceAfter(SCHEDULE_PREFIX, topic);
