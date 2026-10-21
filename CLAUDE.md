# Scent Control

Self-hosted replacement for the Scentiment app and cloud. Scentiment's cloud no longer
works, so their app can't set up or control the diffusers. This project is our own
app and server: they put Scentiment diffusers on the owner's WiFi, control them from
anywhere, and let the diffusers run their schedules, with no dependency on Scentiment
at any point.

- Design (approved): `docs/superpowers/specs/2026-10-20-scent-control-design.md`
- Device facts, with dates and sources: `docs/findings.md`
- Current plan: `docs/superpowers/plans/2026-10-20-phase-1-desktop-app.md` (desktop app built;
  the owner's acceptance session, `docs/phase-1-owner-session.md`, is in progress)

## Devices

- Scentiment **Diffuser Air 2** and **Diffuser HVAC 2** (not the Pro 2). Both have an
  Espressif chip and are never opened.
- Bluetooth: JSON commands over GATT, with one connection at a time. WiFi: 2.4 GHz only.
- The firmware is fixed, so we speak its protocol exactly. That includes keeping
  Scentiment's MQTT topic names. Behavior varies by firmware version. See
  `docs/findings.md`.

## Architecture

```
Mobile app ──HTTPS──▶ Server (API, accounts, homes, schedules, reconciler)
    │                     │
    │ Bluetooth           │ MQTT (server's own login)
    │ (setup, and control │
    │  when nearby)       ▼
    └──────────────▶ MQTT broker ◀──wss (one login per diffuser)── Diffusers on home WiFi
```

- **Apps:** Bluetooth for setup and for control whenever a diffuser is in range
  (owner, Bluetooth-first). Setup sends `SAVE_MQTT` pointing at our broker plus a soft
  restart; `WIFI_CONNECT` is only needed when the diffuser isn't on WiFi yet.
  Otherwise everything goes through the server over HTTPS.
- **Server:** API, accounts and homes, device registry, schedules, per-device broker
  logins, and the desired-state reconciler. The person's most recent change wins,
  wherever it was made. A reboot or reconnect is never a change.
- **MQTT broker:** on Railway. Diffusers connect over `wss://` with a public
  certificate. Each diffuser has one login and may only use its own topics.
- **Protocol package:** pure codec shared by app and server.
- **Drivers and capabilities:** each device family is a driver, so other brands can
  slot in later. The app shows only what a device supports (on firmware 2.30 neither
  model reports oil level or accepts light commands).
- Schedules are stored on and run by the diffuser. The server keeps the master copy,
  re-sends it when needed, and re-sends the time zone offset when daylight saving
  changes.

## Stack

- **Product code:** Bun 1.4 workspaces + Turborepo, **Effect v4 only**, Hono, Drizzle +
  Postgres, Expo SDK 57 (custom dev build for Bluetooth), hosted on Railway. Follow
  the conventions of the HCCNA monorepo (`~/Documents/Dev/Projects/HCCNA`, see its
  `AGENTS.md`). Don't copy its domain code (GraphQL/Relay, Strapi, CCB, Firebase).
- **Phase 0 tools:** Python ≥ 3.11 with `bleak` (Bluetooth) and `paho-mqtt`, under
  `tools/`, tested with `pytest`.
- **Rust crates:** `packages/scent-core` and `apps/desktop` are members of the Cargo
  workspace at the repo root. Each has a `package.json` whose `build`, `test`, `lint`
  and `fmt:check` scripts wrap `cargo`, so Turborepo runs them with everything else:
  `bunx turbo run build test lint fmt:check`. Turbo caching is off for these tasks
  because Cargo has its own incremental cache in `target/`.

## Layout

```
package.json         Bun workspace root (apps/*, packages/*, tools)
turbo.json           Turborepo tasks: build, test, lint, fmt:check
Cargo.toml           Cargo workspace root (the Rust crates)
apps/desktop         Rust desktop app for macOS (Phase 1, on main)
apps/mobile          Expo app (Phase 1+)
apps/server          API, MQTT client, reconciler (Phase 1+)
packages/scent-core  pure Rust crate: codec, parsing, schedules; fixture-tested
infra/               broker and hosting config (test broker in Phase 0)
tools/               Phase 0 CLI tools: scan, GATT dump, session, broker
fixtures/scentiment/ sanitized captured messages, used as test fixtures
docs/                findings, design, plans
```

## Rules

- Never contact Scentiment's servers. Never use, store or print credentials found in
  Scentiment's app.
- Never send `UPDATE_FIRMWARE`, `U_FW`, `RESET` other than `{"mode":"soft"}`, or
  `SECRET_COMMAND` other than `{"command":"restart"}`.
- WiFi passwords and broker passwords never go in the repo, logs, captures,
  fixtures or command-line arguments. Use a `getpass` prompt or an environment
  variable. The server never stores WiFi passwords.
- Protocol code is pure (bytes or JSON in, typed data out) and tested against real
  captured messages saved as sanitized fixtures in `fixtures/scentiment/`.
- Record every observed device behavior in `docs/findings.md` with a date and a
  source tag (**[observed]**, **[prior art]**, **[web]**, or app code).
- Device addresses and names go in config files (`tools/devices.toml`, which is
  gitignored), never hardcoded.
- Raw output goes to `captures/` (gitignored). Only sanitized fixtures are committed.
- Never delete or recreate a Railway broker service, or its domain, while a diffuser
  points at it.
- The repo is private now and will be open source later, so no secrets and no real
  personal data anywhere in it.

## Phases

0. **Prove it on the real devices (done 2026-10-20, gate: go; see `docs/findings.md`):**
   - Bluetooth control.
   - `SAVE_MQTT` over Bluetooth to a `wss://` test broker.
   - Schedules running on the device.
   - Behavior across a power cut.
   - The HVAC 2's protocol.

   If `SAVE_MQTT` or on-device schedules fail, or only plain `mqtt://` works, stop and
   ask the owner.
1. **One diffuser end to end for the owner:** protocol package, server and broker on
   Railway, app setup and manual control. Android first.
   The owner chose a Rust desktop app for macOS first (spec addendum). It is built and on
   `main`: Bluetooth-first control with WiFi fallback, adding a diffuser, schedules
   kept on the Mac, rooms, and Apple's design language.
2. **Schedules, status, resilience:** the reconciler, Bluetooth-first control,
   and the HVAC 2.
3. **Other people:** sign-up with an emailed code, homes and invites, TestFlight and
   APK builds. Needs a domain for email first.

## Environment notes

- Bluetooth work needs this Mac (or another machine) within range of the diffusers,
  with Scentiment's app closed or the phone's Bluetooth off.
- The first Bluetooth scan triggers a macOS permission prompt.
