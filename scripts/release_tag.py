#!/usr/bin/env python3
"""Preview or push an annotated, independently versioned component release tag."""

import argparse
import json
from pathlib import Path
import re
import subprocess
import tomllib


COMPONENTS = {
    "desktop": ("apps/desktop/Cargo.toml", "package"),
    "server": ("apps/server/package.json", None),
    "mobile": ("apps/mobile/package.json", None),
    "scent-core": ("packages/scent-core/Cargo.toml", "package"),
    "tools": ("tools/pyproject.toml", "project"),
}
# SemVer 2.0.0: numeric identifiers cannot have leading zeroes, except in build metadata.
NUMBER = r"(?:0|[1-9][0-9]*)"
PRE = rf"(?:{NUMBER}|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
VERSION = re.compile(
    rf"{NUMBER}\.{NUMBER}\.{NUMBER}(?:-{PRE}(?:\.{PRE})*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
)


def git(root, *args):
    result = subprocess.run(
        ["git", "-C", str(root), *args], text=True, capture_output=True
    )
    if result.returncode:
        raise ValueError(result.stderr.strip() or f"git {args[0]} failed")
    return result.stdout.strip()


def release_tag(root, component, version, *, push=False):
    if component not in COMPONENTS:
        raise ValueError(f"Unknown component: {component}")
    if not VERSION.fullmatch(version):
        raise ValueError("Version must be SemVer without a v prefix (for example 0.1.0).")
    if git(root, "status", "--porcelain"):
        raise ValueError("Commit or remove working-tree changes before tagging.")

    manifest, section = COMPONENTS[component]
    try:
        source = git(root, "show", f"HEAD:{manifest}")
    except ValueError as error:
        raise ValueError(
            f"Cannot release {component}: {manifest} is not committed yet."
        ) from error
    data = tomllib.loads(source) if section else json.loads(source)
    declared = data.get(section, {}).get("version") if section else data.get("version")
    if declared != version:
        raise ValueError(
            f"{manifest} declares {declared!r}, not {version!r}. Update it in a PR first."
        )

    git(root, "fetch", "origin", "+refs/heads/main:refs/remotes/origin/main", "--tags")
    commit = git(root, "rev-parse", "HEAD")
    try:
        git(root, "merge-base", "--is-ancestor", commit, "refs/remotes/origin/main")
    except ValueError as error:
        raise ValueError("Release commits must already be merged into origin/main.") from error

    tag = f"{component}/v{version}"
    ref = f"refs/tags/{tag}"
    existing = git(root, "tag", "--list", tag)
    if existing:
        if (
            git(root, "cat-file", "-t", ref) != "tag"
            or git(root, "rev-parse", f"{ref}^{{commit}}") != commit
        ):
            raise ValueError(
                f"{tag} already exists and is not an annotated tag for this commit."
            )

    if push:
        if not existing:
            git(root, "tag", "-a", tag, "-m", f"{component} {version}", commit)
        # Push only this tag. Never force or move an existing release.
        git(root, "push", "origin", f"{ref}:{ref}")
    return tag, commit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("component", choices=COMPONENTS)
    parser.add_argument("version", help="SemVer, without a leading v")
    parser.add_argument(
        "--push", action="store_true",
        help="Create and push the tag; otherwise preview only",
    )
    args = parser.parse_args()
    try:
        tag, commit = release_tag(
            Path(__file__).resolve().parent.parent,
            args.component,
            args.version,
            push=args.push,
        )
    except (ValueError, OSError) as error:
        parser.exit(1, f"Error: {error}\n")
    print(f"{'Published' if args.push else 'Would publish'} {tag} at {commit}")


if __name__ == "__main__":
    main()
