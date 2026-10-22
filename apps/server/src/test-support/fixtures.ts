// The captured messages in `fixtures/scentiment/`, as `packages/scent-core/tests/fixtures.rs`
// reads them.
import { readdirSync, readFileSync } from "node:fs";

export interface Fixture {
  readonly name: string;
  readonly device: string;
  readonly channel: string;
  readonly topic: string | undefined;
  readonly text: string;
}

const SCENTIMENT = new URL("../../../../fixtures/scentiment/", import.meta.url);

/** Every captured fixture, sorted by name. */
export const fixtures = (): Fixture[] =>
  readdirSync(SCENTIMENT)
    .filter((file) => file.endsWith(".json"))
    .toSorted()
    .map((file) => {
      const value = JSON.parse(readFileSync(new URL(file, SCENTIMENT), "utf8"));
      return {
        name: file.slice(0, -".json".length),
        device: value.device,
        channel: value.channel,
        text: value.text,
        topic: typeof value.topic === "string" ? value.topic : undefined,
      };
    });

export const fixture = (name: string): Fixture => {
  const found = fixtures().find((f) => f.name === name);
  if (found === undefined) {
    throw new Error(`no fixture named ${name}`);
  }
  return found;
};

export const onChannel = (channel: string): Fixture[] =>
  fixtures().filter((f) => f.channel === channel);
