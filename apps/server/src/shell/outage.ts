// What the log says about the connection (spec §6.6). MQTT.js retries every 5 s, so an
// outage is logged once: its first disconnect, and each kind of error once. Every connect
// is logged, and starts a new outage.
import type { BrokerEvent } from "./broker.ts";

export interface Outage {
  /** Whether this outage's disconnect has been logged. */
  readonly closed: boolean;
  /** The kinds of error this outage has logged. */
  readonly kinds: readonly string[];
}

export interface LogLine {
  readonly level: "info" | "warning";
  readonly text: string;
}

/** Before the first connect: nothing logged yet. */
export const STARTING: Outage = { closed: false, kinds: [] };

export const noteEvent = (
  outage: Outage,
  event: BrokerEvent,
): { readonly outage: Outage; readonly line?: LogLine } => {
  switch (event._tag) {
    case "ConnAck": {
      return { outage: STARTING, line: { level: "info", text: "Connected to the broker." } };
    }
    case "Closed": {
      return outage.closed
        ? { outage }
        : {
            outage: { ...outage, closed: true },
            line: { level: "warning", text: "Disconnected from the broker; reconnecting." },
          };
    }
    case "Error": {
      return outage.kinds.includes(event.kind)
        ? { outage }
        : {
            outage: { ...outage, kinds: [...outage.kinds, event.kind] },
            line: { level: "warning", text: `Connecting failed: ${event.kind}.` },
          };
    }
    case "Message": {
      return { outage };
    }
  }
};
