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

## Deploy

```bash
railway up infra/test-broker --path-as-root --service test-broker
```

Then generate a Railway domain for the service with target port **8080**.

## Watch

```bash
cd tools
.venv/bin/python broker.py watch --url wss://<domain>/mqtt --username scent-test
```

The password comes from `SCENT_BROKER_PASSWORD`, from `tools/.env.local`
(gitignored), or from a prompt.
