// The listener: it carries out the core's steps (src/listener/steps.ts) as the broker's
// events arrive and every 30 seconds.
import { Clock, Effect, Ref, Schedule, Stream } from "effect";

import type { Publish } from "../listener/gates.ts";
import type { Memory } from "../listener/record.ts";
import { onConnAck, onMessage, onTick, type Step } from "../listener/steps.ts";
import { PING_EVERY_S } from "../protocol/timing.ts";
import { SUBSCRIBE_ALL, SUBSCRIBE_APP } from "../protocol/topics.ts";
import { Broker, type BrokerEvent } from "./broker.ts";
import { ListenerConfig } from "./config.ts";
import { shortId } from "./log.ts";

const nowSeconds = Effect.map(Clock.currentTimeMillis, (ms) => Math.floor(ms / 1000));

const knows = (count: number): string => `Knows ${count} ${count === 1 ? "diffuser" : "diffusers"}`;

export const runListener: Effect.Effect<never, never, Broker | ListenerConfig> = Effect.gen(
  function* () {
    const broker = yield* Broker;
    const { routeId } = yield* ListenerConfig;
    const startedAt = yield* nowSeconds;
    const memory = yield* Ref.make<Memory>(new Map());

    // A refused publish is logged, by its reason only, and the listener carries on.
    const carryOut = (publishes: readonly Publish[]) =>
      Effect.forEach(
        publishes,
        (publish) =>
          broker
            .publish(publish)
            .pipe(
              Effect.catchTag("BrokerError", (error) =>
                Effect.logWarning(`A publish was not sent: ${error.reason}.`),
              ),
            ),
        { discard: true },
      );

    // One step of the core, applied atomically, so a tick and a message never lose each
    // other's changes. Diffusers it learns of are logged by their last four characters.
    const apply = (step: (known: Memory) => Step) =>
      Effect.gen(function* () {
        const [publishes, added, count] = yield* Ref.modify(memory, (known) => {
          const next = step(known);
          const newIds = [...next.memory.keys()].filter((id) => !known.has(id));
          return [[next.publishes, newIds, next.memory.size] as const, next.memory];
        });
        if (added.length > 0) {
          yield* Effect.logInfo(`${knows(count)} (new: ${added.map(shortId).join(", ")}).`);
        }
        yield* carryOut(publishes);
      });

    const handle = (event: BrokerEvent) =>
      Effect.gen(function* () {
        const now = yield* nowSeconds;
        switch (event._tag) {
          case "ConnAck": {
            yield* Effect.logInfo("Connected to the broker.");
            yield* broker
              .subscribe([SUBSCRIBE_ALL, SUBSCRIBE_APP])
              .pipe(
                Effect.catchTag("BrokerError", (error) =>
                  Effect.logWarning(`Subscribing failed: ${error.reason}.`),
                ),
              );
            const known = yield* Ref.get(memory);
            yield* Effect.logInfo(`${knows(known.size)}.`);
            yield* carryOut(onConnAck(known, now, startedAt));
            return;
          }
          case "Message": {
            yield* apply((known) => onMessage(known, event.topic, event.body, now));
            return;
          }
          case "Closed": {
            yield* Effect.logWarning("Disconnected from the broker; reconnecting.");
          }
        }
      });

    // Nothing is published while disconnected (spec Review Focus 1).
    const tick = Effect.gen(function* () {
      if (!(yield* broker.isConnected)) {
        return;
      }
      const now = yield* nowSeconds;
      yield* apply((known) => onTick(known, now, routeId, startedAt));
    });

    yield* Effect.all(
      [
        Stream.runForEach(broker.events, handle),
        tick.pipe(Effect.repeat(Schedule.spaced(`${PING_EVERY_S} seconds`))),
      ],
      { concurrency: "unbounded" },
    );
    return yield* Effect.never;
  },
);
