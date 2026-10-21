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

> **Never delete or recreate this service, or its domain, while a diffuser points at
> it.** A diffuser keeps the broker address it was given. Before removing the
> service, move every diffuser off it with `SAVE_MQTT`.

## Where it runs

- Railway project `scent-control`, service `test-broker`, in the owner's personal
  workspace (to be transferred to a dedicated workspace later).
- Address: `wss://<your-broker-host>/mqtt`, the Railway domain generated for the
  service (port 443, Railway certificate), routed to container port 8080. Put it in
  the repo-root `.env` as `SCENT_BROKER_URL` (see `.env.example`); it is not in the repo.
- Deployed and checked with a wss round trip on 2026-10-20.

## Deploy

```bash
railway up infra/test-broker --path-as-root --service test-broker
```

Then generate a Railway domain for the service with target port **8080**.

## Watch

```bash
cd tools
.venv/bin/python broker.py watch --url wss://<your-broker-host>/mqtt --username <username>
```

`--url` and `--username` can be left out once `SCENT_BROKER_URL` and
`SCENT_BROKER_USERNAME` are in the repo-root `.env`. The password comes from
`SCENT_BROKER_PASSWORD`, from `tools/.env.local` (gitignored), or from a prompt.
