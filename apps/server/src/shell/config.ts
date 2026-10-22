// The listener's four settings, from the environment. A missing or bad one stops it at
// start-up with a message that names the variable and never shows a value.
import { Context, Effect, Redacted, Schema } from "effect";

export interface ListenerSettings {
  /** The broker's `wss://` address. */
  readonly brokerUrl: string;
  readonly username: string;
  readonly password: Redacted.Redacted<string>;
  /** The route ID the diffusers already hold: what every PING carries (spec §4). */
  readonly routeId: string;
}

export class ListenerConfigError extends Schema.TaggedError<ListenerConfigError>()(
  "ListenerConfigError",
  { message: Schema.String },
) {}

type Env = Readonly<Record<string, string | undefined>>;

const required = (env: Env, name: string): Effect.Effect<string, ListenerConfigError> => {
  const value = env[name];
  return value === undefined || value.trim() === ""
    ? Effect.fail(new ListenerConfigError({ message: `${name} is not set.` }))
    : Effect.succeed(value);
};

/** The settings in `env`. Every value is trimmed except the password, which is kept as given. */
export const settingsFrom = (env: Env): Effect.Effect<ListenerSettings, ListenerConfigError> =>
  Effect.gen(function* () {
    const brokerUrl = (yield* required(env, "SCENT_BROKER_URL")).trim();
    if (!brokerUrl.startsWith("wss://")) {
      return yield* new ListenerConfigError({
        message: "SCENT_BROKER_URL must be a wss:// address.",
      });
    }
    const username = (yield* required(env, "SCENT_BROKER_USERNAME")).trim();
    const password = Redacted.make(yield* required(env, "SCENT_BROKER_PASSWORD"));
    const routeId = (yield* required(env, "SCENT_ROUTE_ID")).trim();
    return { brokerUrl, username, password, routeId };
  });

export class ListenerConfig extends Context.Service<ListenerConfig, ListenerSettings>()(
  "ListenerConfig",
  { make: Effect.suspend(() => settingsFrom(process.env)) },
) {}
