/** Where the server's probes go: our own prefix, never Scentiment's. */
const PROBE_PREFIX = "scent-control/v1/probe/server/";

const HEX8 = /^[0-9a-f]{8}$/u;

/** The probe topic for `hex`, which must be 8 lower-case hex characters. */
export const probeTopic = (hex: string): string => {
  if (!HEX8.test(hex)) {
    throw new Error("A probe name is 8 lower-case hex characters.");
  }
  return `${PROBE_PREFIX}${hex}`;
};

/** 8 random lower-case hex characters, for probe names and client IDs. */
export const randomHex8 = (): string =>
  Array.from(crypto.getRandomValues(new Uint8Array(4)), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
