import { expect, test } from "bun:test";

import { commandBody } from "./commands.ts";

test("PING carries the route ID", () => {
  expect(commandBody({ action: "PING", routeId: "example-route" })).toBe(
    '{"action":"PING","payload":{"userid":"example-route"}}',
  );
});

test("GET_STATUS has an empty payload", () => {
  expect(commandBody({ action: "GET_STATUS" })).toBe('{"action":"GET_STATUS","payload":{}}');
});
