# Security

Scent Control holds people's accounts and controls devices in their homes, so
security reports matter.

## Reporting a vulnerability

Please don't open a public issue. Report it privately through GitHub:
**Security → Report a vulnerability** on this repository. Include what you found, how
to reproduce it, and what an attacker could do with it.

## Ground rules for this project

- Never commit secrets: API keys, broker passwords, certificates, `.env` files.
- Never commit captures or logs that may contain WiFi passwords or account data.
- Never use, test against, or publish credentials belonging to Scentiment or any
  other vendor.

## Repository examples

Use `broker.example.test`, `example-user`, and `example-route` for broker examples.
Use `TESTDEVICE01` and `TESTDEVICE02` for diffuser IDs, and `ExampleWiFi` for WiFi names.
Generate screenshots from the sample-data renderers, with no real account or device settings.
Keep actual settings in the ignored local configuration files.

Before committing, configure this checkout with the noreply email shown in your
GitHub email settings:

```sh
git config --local user.email '<your GitHub noreply address>'
```
