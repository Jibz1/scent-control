# Server: the listener

The first piece of the Scent Control server. It runs all the time on Railway, hears
everything the diffusers say on our broker, keeps each one's latest reported state, pings
each one every 30 s, asks for its state when that isn't known, and shares all of that as
retained messages under `scent-control/v1/`. The design is
`docs/superpowers/specs/2026-10-22-listener-design.md`.

## Settings

All four come from the environment, never from the command line:

| Variable                | What it is                                                                           |
| ----------------------- | ------------------------------------------------------------------------------------ |
| `SCENT_BROKER_URL`      | the broker's `wss://` address                                                        |
| `SCENT_BROKER_USERNAME` | the broker login                                                                     |
| `SCENT_BROKER_PASSWORD` | its password: in `tools/.env.local` locally, a Railway reference variable on Railway |
| `SCENT_ROUTE_ID`        | the route ID the diffusers already hold (the Mac app's, in Settings)                 |

## Running it here

```bash
cd apps/server
bun --env-file=../../.env --env-file=../../tools/.env.local run probe   # round trip through the broker
bun --env-file=../../.env --env-file=../../tools/.env.local run start   # the listener itself
```

`probe` also takes `store <hex>`, `check <hex>` and `clear <hex>`, for checking that the
broker keeps retained messages across a restart.
