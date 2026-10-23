# Scent Control fixtures

Messages Scent Control itself publishes: the contract between the listener and its
readers. The listener (`apps/server`) renders exactly these, and `scent-core` (the Mac
app) decodes them; both test against these files. See
`docs/superpowers/specs/2026-10-22-listener-design.md`, §6.4 and §6.5.

- `state-v1.json`: a diffuser's record, retained at `scent-control/v1/state/<deviceId>`.
  The example from the spec, exactly.
- `heartbeat-v1.json`: the listener's heartbeat, retained at `scent-control/v1/listener`.
- `will-v1.json`: its last will, on the same topic.

The values are synthetic. No device ID appears in a body: it is in the topic.
`by` (`0a1b2c3d`) is a synthetic listener instance tag; readers ignore it.
