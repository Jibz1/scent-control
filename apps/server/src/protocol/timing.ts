// Whether a diffuser is reachable, and how old a reading may get: scent-core's status.rs.
// Times are whole seconds since 1970.

/** How often to ping each diffuser. */
export const PING_EVERY_S = 30;

/** How long after a ping to wait for the answer before calling it offline. */
export const ANSWER_WITHIN_S = 15;

/** How long after hearing from a diffuser to still call it online. */
export const ONLINE_FOR_S = 75;

/** How old a reading may get before it counts as stale, and how often to ask for status. */
export const STALE_AFTER_S = 300;

/** Times further ahead of the reader's clock than this make a shared record unusable. */
export const DAY_AHEAD_S = 86_400;
