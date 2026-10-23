import { expect, test } from "bun:test";

import { probeTopic, randomHex8 } from "./probe.ts";

test("probe topics stay under our probe prefix", () => {
  expect(probeTopic("0a1b2c3d")).toBe("scent-control/v1/probe/server/0a1b2c3d");
  for (const bad of ["", "0A1B2C3D", "0a1b2c3", "0a1b2c3d/#", "+"]) {
    expect(() => probeTopic(bad)).toThrow();
  }
  expect(randomHex8()).toMatch(/^[0-9a-f]{8}$/u);
});
