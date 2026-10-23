// A broker for the shell's tests: it plays scripted events and records what the listener
// asks of it, in order. Like the real one, it refuses publishes while not connected.
import { Effect, Layer, Stream } from "effect";

import type { Publish } from "../listener/gates.ts";
import { Broker, BrokerError, type BrokerEvent } from "./broker.ts";

export type BrokerOp =
  | { readonly op: "subscribe"; readonly filters: readonly string[] }
  | { readonly op: "publish"; readonly publish: Publish; readonly refused: boolean };

export interface FakeBrokerScript {
  readonly connected: boolean;
  readonly events: readonly BrokerEvent[];
  /** Refuse every publish, as a broker that has just gone away would. */
  readonly refusePublishes?: boolean;
}

export const fakeBroker = (script: FakeBrokerScript) => {
  const ops: BrokerOp[] = [];
  const layer = Layer.succeed(
    Broker,
    Broker.of({
      events: Stream.fromIterable(script.events),
      isConnected: Effect.sync(() => script.connected),
      subscribe: (filters) =>
        Effect.sync(() => {
          ops.push({ op: "subscribe", filters: [...filters] });
        }),
      publish: (publish) => {
        const refused = !script.connected || script.refusePublishes === true;
        ops.push({ op: "publish", publish, refused });
        return refused ? Effect.fail(new BrokerError({ reason: "not connected" })) : Effect.void;
      },
    }),
  );
  const published = (): Publish[] =>
    ops.flatMap((entry) => (entry.op === "publish" && !entry.refused ? [entry.publish] : []));
  return { layer, ops, published };
};
