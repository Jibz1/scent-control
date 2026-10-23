// A broker for the shell's tests: it plays scripted events and records what the listener
// asks of it, in order. Like the real one, it refuses publishes while not connected.
import { Effect, Layer, Stream, type Duration } from "effect";

import type { Publish } from "../listener/gates.ts";
import { Broker, BrokerError, type BrokerEvent } from "./broker.ts";

export type BrokerOp =
  | { readonly op: "subscribe"; readonly filters: readonly string[] }
  | { readonly op: "publish"; readonly publish: Publish; readonly refused: boolean }
  | { readonly op: "reconnect" };

export interface FakeBrokerScript {
  readonly connected: boolean;
  /** The events, in order; a stream when they need to arrive over time. */
  readonly events: readonly BrokerEvent[] | Stream.Stream<BrokerEvent>;
  /** Refuse every publish, as a broker that has just gone away would. */
  readonly refusePublishes?: boolean;
  /** Refuse every subscribe. */
  readonly refuseSubscribe?: boolean;
  /** How long the broker takes to acknowledge a publish, when not at once. */
  readonly slowPublish?: (publish: Publish) => Duration.Input | undefined;
}

export const fakeBroker = (script: FakeBrokerScript) => {
  const ops: BrokerOp[] = [];
  const layer = Layer.succeed(
    Broker,
    Broker.of({
      events: Stream.isStream(script.events) ? script.events : Stream.fromIterable(script.events),
      isConnected: Effect.sync(() => script.connected),
      subscribe: (filters) =>
        Effect.suspend(() => {
          ops.push({ op: "subscribe", filters: [...filters] });
          return script.refuseSubscribe === true
            ? Effect.fail(new BrokerError({ reason: "subscribe failed" }))
            : Effect.void;
        }),
      publish: (publish) =>
        Effect.suspend(() => {
          // Recorded when it is sent; acknowledged after `slowPublish`, if any.
          const refused = !script.connected || script.refusePublishes === true;
          ops.push({ op: "publish", publish, refused });
          if (refused) {
            return Effect.fail(new BrokerError({ reason: "not connected" }));
          }
          const delay = script.slowPublish?.(publish);
          return delay === undefined ? Effect.void : Effect.sleep(delay);
        }),
      reconnect: Effect.sync(() => {
        ops.push({ op: "reconnect" });
      }),
    }),
  );
  const published = (): Publish[] =>
    ops.flatMap((entry) => (entry.op === "publish" && !entry.refused ? [entry.publish] : []));
  return { layer, ops, published };
};
