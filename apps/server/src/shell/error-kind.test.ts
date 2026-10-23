import { expect, test } from "bun:test";

import { errorKind } from "./error-kind.ts";

const withCode = (message: string, code: unknown): Error =>
  Object.assign(new Error(message), { code });

test("a refused connect is named by the broker's return code", () => {
  expect(errorKind(withCode("Connection refused: Unacceptable protocol version", 1))).toBe(
    "unacceptable protocol version",
  );
  expect(errorKind(withCode("Connection refused: Identifier rejected", 2))).toBe(
    "identifier rejected",
  );
  expect(errorKind(withCode("Connection refused: Server unavailable", 3))).toBe(
    "server unavailable",
  );
  expect(errorKind(withCode("Connection refused: Bad username or password", 4))).toBe(
    "bad username or password",
  );
  expect(errorKind(withCode("Connection refused: Not authorized", 5))).toBe("not authorized");
  expect(errorKind(withCode("Connection refused: Banned", 138))).toBe("refused (code 138)");
});

test("a network failure is named by its system code, and only a code that looks like one", () => {
  expect(errorKind(withCode("connect ECONNREFUSED 10.0.0.1:443", "ECONNREFUSED"))).toBe(
    "network (ECONNREFUSED)",
  );
  expect(errorKind(withCode("getaddrinfo ENOTFOUND broker.example.test", "ENOTFOUND"))).toBe(
    "network (ENOTFOUND)",
  );
  expect(errorKind(withCode("odd", "wss://broker.example.test"))).toBe("network");
});

test("timeouts are named, and anything else is 'other', never the library's own message", () => {
  expect(errorKind(new Error("connack timeout"))).toBe("no answer to connect");
  expect(errorKind(new Error("Keepalive timeout"))).toBe("keepalive timeout");
  expect(errorKind(new Error("failed for pw-test-secret at wss://broker.example.test"))).toBe(
    "other",
  );
  expect(errorKind("not an error")).toBe("other");
});
