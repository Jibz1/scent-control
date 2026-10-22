// The listener's entry point. SIGTERM (a Railway redeploy) interrupts it, which runs the
// broker's finalizer: a clean disconnect, so no will is sent.
import { BunRuntime } from "@effect/platform-bun";
import { Effect, Layer } from "effect";

import { Broker } from "./shell/broker.ts";
import { ListenerConfig } from "./shell/config.ts";
import { runListener } from "./shell/run.ts";

const ConfigLive = Layer.effect(ListenerConfig, ListenerConfig.make);
const Live = Layer.effect(Broker, Broker.make).pipe(Layer.provideMerge(ConfigLive));

BunRuntime.runMain(runListener.pipe(Effect.provide(Live)));
