// What kind of failure an MQTT.js `error` event is, as a fixed phrase for the log (spec
// §6.6). Never the library's own message, which can carry an address.

// MQTT 3.1.1 CONNACK return codes, which MQTT.js puts in `code` when the broker refuses.
const REFUSED: Readonly<Record<number, string>> = {
  1: "unacceptable protocol version",
  2: "identifier rejected",
  3: "server unavailable",
  4: "bad username or password",
  5: "not authorized",
};

/** A system error code such as `ECONNREFUSED`, and nothing that only looks like text. */
const SYSTEM_CODE = /^E[A-Z]+$/u;

export const errorKind = (error: unknown): string => {
  if (!(error instanceof Error)) {
    return "other";
  }
  const { code } = error as { code?: unknown };
  if (typeof code === "number") {
    return REFUSED[code] ?? `refused (code ${code})`;
  }
  if (typeof code === "string") {
    return SYSTEM_CODE.test(code) ? `network (${code})` : "network";
  }
  if (error.message === "connack timeout") {
    return "no answer to connect";
  }
  if (error.message === "Keepalive timeout") {
    return "keepalive timeout";
  }
  return "other";
};
