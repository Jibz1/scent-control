/** A device ID as the logs show it: its last four characters, never the whole ID. */
export const shortId = (id: string): string => `…${id.slice(-4)}`;
