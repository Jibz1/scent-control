// The only two commands the listener ever sends a diffuser. Both only ask; neither changes
// anything on it.

export type ListenerCommand =
  | { readonly action: "PING"; readonly routeId: string }
  | { readonly action: "GET_STATUS" };

/** The MQTT body, as scent-core's `Command::mqtt_body` writes it. */
export const commandBody = (command: ListenerCommand): string =>
  command.action === "PING"
    ? JSON.stringify({ action: "PING", payload: { userid: command.routeId } })
    : JSON.stringify({ action: "GET_STATUS", payload: {} });
