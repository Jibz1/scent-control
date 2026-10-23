// The listener: it carries out the core's steps (src/listener/steps.ts) as the broker's
// events arrive and every 30 seconds.
import { Clock, Effect, Ref, Stream } from "effect";

import type { Publish } from "../listener/gates.ts";
import type { Memory } from "../listener/record.ts";
import { onConnAck, onMessage, onTick, type Step } from "../listener/steps.ts";
import { randomHex8 } from "../probe.ts";
import { PING_EVERY_S } from "../protocol/timing.ts";
import { SUBSCRIBE_ALL, SUBSCRIBE_APP } from "../protocol/topics.ts";
import { Broker, type BrokerEvent } from "./broker.ts";
import { ListenerConfig } from "./config.ts";
import { shortId } from "./log.ts";
import { STARTING, noteEvent, type LogLine } from "./outage.ts";

type ListenerEvent = BrokerEvent | { readonly _tag: "Tick" };

const TICK: ListenerEvent = { _tag: "Tick" };

const nowSeconds = Effect.map(Clock.currentTimeMillis, (ms) => Math.floor(ms / 1000));

const knows = (count: number): string => `Knows ${count} ${count === 1 ? "diffuser" : "diffusers"}`;

const log = (line: LogLine | undefined) => {
  if (line === undefined) {
    return Effect.void;
  }
  return line.level === "info" ? Effect.logInfo(line.text) : Effect.logWarning(line.text);
};

/**
 * The listener, ticking whenever `ticks` emits. Ticks and the broker's events are handled
 * one at a time, in order, each with its publishes, so the last record published is always
 * the newest one: the listener never needs its own records back to put that right.
 */
export const runListenerWith = (
  ticks: Stream.Stream<unknown>,
): Effect.Effect<never, never, Broker | ListenerConfig> =>
  Effect.gen(function* () {
    const broker = yield* Broker;
    const { routeId } = yield* ListenerConfig;
    // This instance's tag on every record it publishes, so it can tell them when they come back.
    const self = yield* Effect.sync(randomHex8);
    const startedAt = yield* nowSeconds;
    const memory = yield* Ref.make<Memory>(new Map());
    const outage = yield* Ref.make(STARTING);

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

    // One step of the core. Diffusers it learns of are logged by their last four characters.
    const apply = (step: (known: Memory) => Step) =>
      Effect.gen(function* () {
        const [publishes, added, count] = yield* Ref.modify(memory, (known) => {
          const next = step(known);
          const newIds = [...next.memory.keys()].filter((id) => !known.has(id));
          return [[next.publishes, newIds, next.memory.size] as const, next.memory];
        });
        if (added.length > 0) {
          yield* Effect.logInfo(
            `${knows(count)} (new: ${added.map((id) => shortId(id)).join(", ")}).`,
          );
        }
        yield* carryOut(publishes);
      });

    // Once subscribed, the heartbeat and every record. A listener that couldn't subscribe
    // hears nothing, so it reconnects rather than say it is running.
    const onConnected = (now: number) =>
      broker.subscribe([SUBSCRIBE_ALL, SUBSCRIBE_APP]).pipe(
        Effect.andThen(
          Effect.gen(function* () {
            const known = yield* Ref.get(memory);
            yield* Effect.logInfo(`${knows(known.size)}.`);
            yield* carryOut(onConnAck(known, now, startedAt, self));
          }),
        ),
        Effect.catchTag("BrokerError", (error) =>
          Effect.logWarning(`Subscribing failed: ${error.reason}; reconnecting.`).pipe(
            Effect.andThen(broker.reconnect),
          ),
        ),
      );

    const handle = (event: ListenerEvent) =>
      Effect.gen(function* () {
        const now = yield* nowSeconds;
        if (event._tag === "Tick") {
          // Nothing is published while disconnected (spec Review Focus 1).
          if (yield* broker.isConnected) {
            yield* apply((known) => onTick(known, now, routeId, startedAt, self));
          }
          return;
        }
        yield* log(
          yield* Ref.modify(outage, (o) => {
            const noted = noteEvent(o, event);
            return [noted.line, noted.outage];
          }),
        );
        if (event._tag === "ConnAck") {
          yield* onConnected(now);
        } else if (event._tag === "Message") {
          yield* apply((known) => onMessage(known, event.topic, event.body, now, self));
        }
      });

    const tickEvents = ticks.pipe(Stream.map(() => TICK));
    yield* Stream.runForEach(Stream.merge(broker.events, tickEvents), handle);
    return yield* Effect.never;
  });

/** The listener, ticking at once and then every 30 seconds. */
export const runListener: Effect.Effect<never, never, Broker | ListenerConfig> = runListenerWith(
  Stream.tick(`${PING_EVERY_S} seconds`),
);
