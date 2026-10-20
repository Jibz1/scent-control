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
