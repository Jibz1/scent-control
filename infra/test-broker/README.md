# Phase 0 test broker

A throwaway Mosquitto broker for Phase 0 experiments (see
`docs/superpowers/plans/2026-10-20-phase-0-device-verification.md`). Phase 1 replaces
it with the real broker: one login per diffuser, with topic rules managed by the
server.

- **Transport:** MQTT over WebSocket on port 8080. Railway's HTTPS edge terminates
  TLS, so clients and diffusers connect to `wss://<domain>/mqtt` and see a public
  certificate.
- **Login:** one shared username and password (`TEST_BROKER_USERNAME`,
  `TEST_BROKER_PASSWORD`, set as Railway variables). Every diffuser in Phase 0
  uses it.
- **Logs:** `log_type all`, so Railway's logs show every connect, subscribe and
  publish, including which topics a diffuser subscribes to.
- **Persistence:** on (`persistence true`, saved to `/mosquitto/data/` every 60 s
  and on shutdown). It keeps the retained messages, which is where the desktop app
  stores each diffuser's last schedule for its other copies
  (`scent-control/v1/schedule/<deviceId>`, interim until the Phase 2 server).
  Persistence also keeps the sessions and queues of `clean_session=false` clients
  (bounded in `mosquitto.conf`). It also keeps any retained message under
  `scentiment-mqtt-prod/`, so a stray retained publish to a device topic would be
  replayed to the diffuser across restarts; the cure is an empty retained publish to
  that topic.

> **A Railway volume must be mounted at `/mosquitto/data`.** Without one, the
> container's disk is thrown away on every restart or redeploy, and every stored
> schedule goes with it: the diffusers keep running their schedules, but the app
> no longer knows what they are until each is sent again. The entrypoint makes the
> directory and gives it to the `mosquitto` user, so a fresh volume needs nothing
> else.

> **Never delete or recreate this service, or its domain, while a diffuser points at
> it.** A diffuser keeps the broker address it was given. Before removing the
> service, move every diffuser off it with `SAVE_MQTT`.

## Where it runs

- Railway project `scent-control`, service `test-broker`, in the owner's personal
  workspace (to be transferred to a dedicated workspace later).
- Address: `wss://test-broker-production.up.railway.app/mqtt` (port 443, Railway
  certificate), routed to container port 8080.
- Deployed and checked with a wss round trip on 2026-10-20.

## Deploy

```bash
railway up infra/test-broker --path-as-root --service test-broker
```

Then generate a Railway domain for the service with target port **8080**, and
attach a volume to the service with mount path **`/mosquitto/data`** (see
Persistence above).

## Watch

```bash
cd tools
.venv/bin/python broker.py watch --url wss://<domain>/mqtt --username scent-test
```

The password comes from `SCENT_BROKER_PASSWORD`, from `tools/.env.local`
(gitignored), or from a prompt.
