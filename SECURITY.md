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
