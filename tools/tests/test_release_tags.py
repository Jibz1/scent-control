"""Exercise release tagging against temporary repositories and a local bare remote."""

import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest


SPEC = importlib.util.spec_from_file_location(
    "release_tag", Path(__file__).resolve().parents[2] / "scripts/release_tag.py"
)
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ReleaseTagTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "repo"
        self.remote = Path(self.temp.name) / "remote.git"
        subprocess.run(["git", "init", "--bare", str(self.remote)], check=True, capture_output=True)
        subprocess.run(["git", "init", "-b", "main", str(self.root)], check=True, capture_output=True)
        self.git("config", "user.name", "Release Test")
        self.git("config", "user.email", "release@example.com")
        # Keep tests independent of a developer's signing and hook configuration.
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "tag.gpgsign", "false")
        self.git("config", "core.hooksPath", "/dev/null")
        self.git("remote", "add", "origin", str(self.remote))
        self.write("apps/desktop/Cargo.toml", '[package]\nname = "desktop"\nversion = "0.1.0"\n')
        self.write("packages/scent-core/Cargo.toml", '[package]\nversion = "0.1.0"\n')
        self.write("apps/home-assistant/Cargo.toml", '[package]\nversion = "0.1.0"\n')
        self.write("tools/pyproject.toml", '[project]\nversion = "0.0.1"\n')
        self.commit()
        self.git("push", "origin", "main")

    def git(self, *args):
        return release.git(self.root, *args)

    def write(self, name, content):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)

    def commit(self):
        self.git("add", ".")
        self.git("commit", "-m", "test: update release fixture")

    def publish(self, component="desktop", version="0.1.0"):
        return release.release_tag(self.root, component, version, push=True)

    def test_preview_creates_no_tags(self):
        tag, commit = release.release_tag(self.root, "desktop", "0.1.0")
        self.assertEqual(tag, "desktop/v0.1.0")
        self.assertEqual(commit, self.git("rev-parse", "HEAD"))
        self.assertEqual(self.git("tag", "--list"), "")
        self.assertEqual(self.git("ls-remote", "--tags", "origin"), "")

    def test_publish_is_annotated_scoped_and_retryable(self):
        self.git("tag", "unrelated-local-tag")
        tag, commit = self.publish()
        original = self.git("rev-parse", f"refs/tags/{tag}")
        self.publish()
        self.assertEqual(original, self.git("rev-parse", f"refs/tags/{tag}"))
        self.assertEqual(self.git("cat-file", "-t", f"refs/tags/{tag}"), "tag")
        self.assertIn(f"{commit}\trefs/tags/{tag}^{{}}", self.git("ls-remote", "--tags", "origin"))
        self.assertNotIn("unrelated", self.git("ls-remote", "--tags", "origin"))

    def test_components_have_independent_namespaces(self):
        self.publish()
        self.publish("scent-core")
        self.publish("home-assistant")
        self.publish("tools", "0.0.1")
        self.assertEqual(self.git("tag", "--list").splitlines(), ["desktop/v0.1.0", "home-assistant/v0.1.0", "scent-core/v0.1.0", "tools/v0.0.1"])

    def test_rejected_push_can_be_retried_without_recreating_tag(self):
        hook = self.remote / "hooks/pre-receive"
        hook.write_text("#!/bin/sh\nexit 1\n")
        hook.chmod(0o755)
        # The bare remote must use its own hooks, regardless of developer defaults.
        release.git(self.remote, "config", "core.hooksPath", str(hook.parent))
        with self.assertRaises(ValueError):
            self.publish()
        original = self.git("rev-parse", "refs/tags/desktop/v0.1.0")
        self.assertEqual(self.git("ls-remote", "--tags", "origin"), "")
        hook.unlink()
        self.publish()
        self.assertEqual(original, self.git("rev-parse", "refs/tags/desktop/v0.1.0"))

    def test_future_apps_require_committed_manifests(self):
        for component in ("server", "mobile"):
            with self.subTest(component=component), self.assertRaisesRegex(ValueError, "not committed yet"):
                self.publish(component)
        self.assertEqual(self.git("tag", "--list"), "")

    def test_future_apps_use_package_versions(self):
        for component in ("server", "mobile"):
            self.write(f"apps/{component}/package.json", '{"version": "1.2.0-rc.1"}')
        self.commit()
        self.git("push", "origin", "main")
        for component in ("server", "mobile"):
            self.publish(component, "1.2.0-rc.1")

    def test_invalid_versions_and_components_are_rejected(self):
        for version in ("v0.1.0", "01.1.0", "1.0", "1.0.0-01", "1.0.0-", "1.0.0+", "1.0.0\n", "$(touch nope)"):
            with self.subTest(version=version), self.assertRaisesRegex(ValueError, "SemVer"):
                self.publish(version=version)
        with self.assertRaisesRegex(ValueError, "Unknown component"):
            self.publish("../unknown")

    def test_version_must_match_manifest(self):
        with self.assertRaisesRegex(ValueError, "declares"):
            self.publish(version="0.2.0")

    def test_dirty_checkout_is_rejected(self):
        self.write("untracked", "changes")
        with self.assertRaisesRegex(ValueError, "working-tree changes"):
            self.publish()
        self.git("add", ".")
        with self.assertRaisesRegex(ValueError, "working-tree changes"):
            self.publish()

    def test_unmerged_commit_is_rejected(self):
        self.git("switch", "-c", "feature")
        self.write("feature", "not merged")
        self.commit()
        with self.assertRaisesRegex(ValueError, "merged into origin/main"):
            self.publish()
        self.assertEqual(self.git("tag", "--list"), "")

    def test_existing_tag_cannot_be_moved(self):
        tag, original = self.publish()
        self.write("change", "next commit")
        self.commit()
        self.git("push", "origin", "main")
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.publish()
        self.assertEqual(self.git("rev-parse", f"refs/tags/{tag}^{{commit}}"), original)

    def test_lightweight_tag_is_rejected(self):
        self.git("tag", "desktop/v0.1.0")
        with self.assertRaisesRegex(ValueError, "annotated tag"):
            self.publish()

    def test_remote_tag_is_fetched_before_duplicate_check(self):
        self.publish()
        self.git("tag", "-d", "desktop/v0.1.0")
        self.write("change", "next commit")
        self.commit()
        self.git("push", "origin", "main")
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.publish()


if __name__ == "__main__":
    unittest.main()
