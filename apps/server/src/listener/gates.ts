// The two ways the listener can make a publish. Every message it sends goes through one of
// them, so these two functions are what keep it to its rules (spec §4, §6.3, §6.4).
import { commandBody, type ListenerCommand } from "../protocol/commands.ts";
import { APP_PREFIX, deviceTopic, isValidDeviceId } from "../protocol/topics.ts";

export interface Publish {
  readonly topic: string;
  readonly body: string;
  readonly qos: 1;
  readonly retain: boolean;
}

/**
 * A retained publish, only under our own prefix, with something after it and no wildcard.
 * A retained message on a diffuser's topic would be replayed to it on every connect.
 */
export const retainedPublish = (topic: string, body: string): Publish | undefined => {
  const ours =
    topic.startsWith(APP_PREFIX) &&
    topic.length > APP_PREFIX.length &&
    !topic.includes("+") &&
    !topic.includes("#");
  return ours ? { topic, body, qos: 1, retain: true } : undefined;
};

/**
 * A command to one diffuser: only `PING` (with a route ID) or `GET_STATUS`, never retained,
 * and only to a valid device ID's topic.
 */
export const commandPublish = (deviceId: string, command: ListenerCommand): Publish | undefined => {
  if (!isValidDeviceId(deviceId)) {
    return undefined;
  }
  if (command.action === "PING" && command.routeId.trim() === "") {
    return undefined;
  }
  return { topic: deviceTopic(deviceId), body: commandBody(command), qos: 1, retain: false };
};
