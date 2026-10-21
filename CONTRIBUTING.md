# Contributing

Thanks for helping. The project is in its design phase, so the most useful
contributions right now are device reports.

## Reporting a device

Open an issue with:

- The exact model, from the label on the device.
- Its Bluetooth name and, if you can, a GATT dump.
- What you observed, with the date.

Use synthetic values in anything you post: never your WiFi password, account email,
or other personal data.

## Rules

- Never contact Scentiment's servers, and never use credentials found in vendor apps.
- Protocol code is pure and tested against real captured messages saved as
  fixtures.
- Record every observed device behavior in `docs/findings.md` with a date and a
  source.

## Pull request labels

PRs receive component labels from the files they change: `desktop`, `server`,
`mobile`, `scent-core`, `tools`, `infra`, and `ci`. Markdown changes also receive
`documentation`. A PR can have several labels; for example, a desktop feature
with updated docs receives both `desktop` and `documentation`.

The rules live in `.github/labeler.yml`. After the workflow is merged into `main`,
labels update when PRs are opened, updated, reopened, or edited. To label an older
PR immediately, run **Actions → Label pull request → Run workflow** with its PR
number. Only labels listed in the rules are synchronized; labels such as `bug`
and `enhancement` are managed manually.

Create new label names in the repository's **Issues → Labels** settings before
adding rules. The workflow uses the base repository's configuration and does not
check out or execute PR code.

PR labels describe the scope of a change. Versioned Git tags such as
`desktop/v0.1.0` are created separately through the manual
[release workflow](docs/releases.md).
