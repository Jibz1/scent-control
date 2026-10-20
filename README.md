# Scent Control

A self-hosted app and server for Scentiment smart diffusers. It sets them up on your
WiFi, controls them, and runs their schedules without Scentiment's cloud.

> **Status:** design phase. Nothing is ready to use yet.

## Why

Scentiment's official app has to reach Scentiment's servers before it will set up or
control a diffuser. Those servers no longer work, so the diffusers can only be used
with their physical buttons. Scent Control replaces the app and the cloud. It never
contacts Scentiment's servers.

## Supported devices

| Device | Status |
|---|---|
| Scentiment Diffuser Air 2 | Planned |
| Scentiment Diffuser HVAC 2 | Planned |

Other brands may follow later.

## Development

This is a Bun + Turborepo monorepo. The Rust crates are members of the Cargo workspace
at the repo root, and the Python tools live in `tools/`. You need Bun, a Rust toolchain
and Python 3.11 or newer.

```sh
bun install
bunx turbo run build test lint fmt:check
```

That one command builds, tests, lints and format-checks every package, including the
Python tools' tests (it creates `tools/.venv` on first run).

## Desktop app

`apps/desktop` is a native macOS app (Rust, eframe/egui). It is macOS only.

Build and launch it as an app bundle:

```sh
scripts/bundle-macos.sh
open "target/Scent Control.app"
```

Always launch it with `open`, never by running the binary directly. macOS kills any
process that uses Bluetooth unless the app responsible for it declares a Bluetooth
usage description in its Info.plist, and only the bundle has one. The script builds a
release binary, assembles `target/Scent Control.app` and signs it ad hoc.

## Docs

- [docs/findings.md](docs/findings.md): what we know about the devices and their
  protocol, with dates and sources.

## Acknowledgements

[Scent Assistant](https://github.com/mr-sparks/scent-assistant) (MIT) documented the
Scentiment Bluetooth protocol first.

## Disclaimer

This project is not affiliated with or endorsed by Scentiment. "Scentiment" is used
only to name the devices this project works with.

## License

[MIT](LICENSE)
