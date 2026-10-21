# Component release tags

Each component has an independent version and annotated Git tag. A tag identifies
the whole repository commit used for that component's release, including shared
dependencies. Releasing one component does not bump or tag the others.

| Component | Tag example | Version source |
|---|---|---|
| Desktop app | `desktop/v0.1.0` | `apps/desktop/Cargo.toml` → `package.version` |
| Server (planned) | `server/v0.1.0` | `apps/server/package.json` → `version` |
| Mobile app (planned) | `mobile/v0.1.0` | `apps/mobile/package.json` → `version` |
| Shared Rust core | `scent-core/v0.1.0` | `packages/scent-core/Cargo.toml` → `package.version` |
| Device tools | `tools/v0.0.1` | `tools/pyproject.toml` → `project.version` |

Server and mobile names are reserved; tagging them fails until their versioned
manifests exist. Add future components to `COMPONENTS` in `scripts/release_tag.py`
and to the workflow's component choices.

## Release from GitHub

1. Choose the component's next SemVer version. Update its version source and any
   associated app metadata in a PR, run the component's checks, and merge it into
   `main`. For Rust versions, commit the updated `Cargo.lock`. The desktop app shows
   its version in Settings → About, and `scripts/bundle-macos.sh` writes it to the
   bundle's `CFBundleShortVersionString` and `CFBundleVersion`, so there is nothing
   else to keep in sync. The tagger does not edit versions.
2. Open **Actions → Tag component release → Run workflow**. Select `main`, choose
   the component, and enter the exact manifest version without `v`.
3. The workflow tags the `main` commit selected when the run was dispatched. It
   checks the version, requires a commit reachable from `origin/main`, and pushes
   only that component's annotated tag. Dispatches on other branches are skipped.

Versions such as `0.2.0-rc.1` are accepted when they match the manifest. Existing
tags are never moved or overwritten. Rerunning for the same annotated tag and
commit is safe; using the same version at another commit fails. Publish a new
version to correct a release.

Tags are explicit release markers. Merges do not automatically create them, and
this workflow does not build binaries, create GitHub Releases, publish packages,
or deploy services. Run the relevant build and checks before tagging.

The workflow uses `GITHUB_TOKEN` with `contents: write`; it needs no extra secret.
Tags pushed using that token do not start other workflows listening for tag pushes
([GitHub's event rules](https://docs.github.com/en/actions/concepts/security/github_token)).
Future deployment or packaging jobs should run in this workflow or be explicitly
dispatched. Local tag pushes using your own Git credentials follow normal push
event rules.

## Release from a local checkout

Use Python 3.11 or newer and a clean checkout of the intended, already merged
commit. The command fetches `origin/main` and tags before validating the target.

```sh
# Preview: validates everything, creates no tag.
python3 scripts/release_tag.py desktop 0.1.0

# Create and push only desktop/v0.1.0.
python3 scripts/release_tag.py desktop 0.1.0 --push
```

A failed push can leave the annotated tag locally; rerun the same command from
the same commit after fixing access. The script never uses force pushes.

The release tests use temporary repositories and a local bare remote. They run
with the tools' normal test task and in the **Check release tooling** PR workflow,
or independently without installing dependencies:

```sh
python3 -m unittest discover -s tools/tests -p test_release_tags.py -v
```
