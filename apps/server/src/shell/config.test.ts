import { expect, test } from "bun:test";
import { Effect, Redacted } from "effect";

import { settingsFrom } from "./config.ts";

const env = {
  SCENT_BROKER_URL: "wss://broker.example.test/mqtt",
  SCENT_BROKER_USERNAME: "someone",
  SCENT_BROKER_PASSWORD: "pw-test-secret",
  SCENT_ROUTE_ID: "example-route",
};

const failure = (values: Record<string, string | undefined>) =>
  Effect.runPromise(Effect.flip(settingsFrom(values)));

test("a missing setting is named, without any value", async () => {
  for (const empty of [undefined, "", "   "]) {
    const error = await failure({ ...env, SCENT_ROUTE_ID: empty });
    expect(error.message).toContain("SCENT_ROUTE_ID");
    expect(JSON.stringify(error)).not.toContain("pw-test-secret");
    expect(String(error)).not.toContain("pw-test-secret");
  }
});

test("the broker address must be wss", async () => {
  const error = await failure({ ...env, SCENT_BROKER_URL: "http://broker.example.test/mqtt" });
  expect(error.message).toContain("SCENT_BROKER_URL");
  expect(error.message).not.toContain("broker.example.test");
});

test("good settings are read, trimmed, with the password kept redacted", async () => {
  const settings = await Effect.runPromise(
    settingsFrom({ ...env, SCENT_ROUTE_ID: " example-route " }),
  );
  expect(settings.brokerUrl).toBe("wss://broker.example.test/mqtt");
  expect(settings.username).toBe("someone");
  expect(settings.routeId).toBe("example-route");
  expect(Redacted.value(settings.password)).toBe("pw-test-secret");
  expect(String(settings.password)).not.toContain("pw-test-secret");
});
