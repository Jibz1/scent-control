import { expect, test } from "bun:test";

import { shortId } from "./log.ts";

test("device IDs are shortened to their last four characters", () => {
  expect(shortId("TESTDEVICE01")).toBe("…CE01");
});
