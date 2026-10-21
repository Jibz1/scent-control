"""The broker the desktop build is given from the repo-root .env (scripts/broker-env.sh).

Each test runs the loader in a fresh bash with a bare environment and a .env in a
temporary folder, then lists what it exported. Nothing is built.
"""

from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
LOADER = ROOT / "scripts" / "broker-env.sh"
BUNDLE = ROOT / "scripts" / "bundle-macos.sh"

SECRET = "hunter2-not-for-the-build"


def load(env_file: Path, environment: dict[str, str] | None = None) -> subprocess.CompletedProcess:
    """Sources the loader under `set -euo pipefail`, as the bundle script does, loads
    `env_file`, and prints the environment the build would get."""
    script = 'set -euo pipefail; source "$1"; load_broker_env "$2"; env'
    env = {"PATH": "/usr/bin:/bin"}
    env.update(environment or {})
    return subprocess.run(
        ["bash", "-c", script, "bash", str(LOADER), str(env_file)],
        env=env,
        capture_output=True,
        text=True,
        check=True,
    )


def exported(result: subprocess.CompletedProcess) -> dict[str, str]:
    pairs = (line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)
    return {key: value for key, value in pairs}


def test_only_the_three_broker_settings_are_exported(tmp_path):
    env_file = tmp_path / ".env"
    env_file.write_text(
        "# The broker the app starts with.\n"
        "SCENT_BROKER_URL=wss://broker.example.test/mqtt\n"
        'export SCENT_BROKER_USERNAME="example-user"\n'
        "  SCENT_ROUTE_ID = 'route-test-1'  \n"
        "\n"
        "OTHER_SETTING=not-for-the-build\n"
    )
    result = load(env_file)
    got = exported(result)
    assert got["SCENT_BROKER_URL"] == "wss://broker.example.test/mqtt"
    assert got["SCENT_BROKER_USERNAME"] == "example-user"
    assert got["SCENT_ROUTE_ID"] == "route-test-1"
    assert "OTHER_SETTING" not in got
    assert result.stderr == ""


def test_a_password_in_env_is_ignored_with_one_line_that_never_shows_it(tmp_path):
    env_file = tmp_path / ".env"
    env_file.write_text(
        "SCENT_BROKER_URL=wss://broker.example.test/mqtt\n"
        f"SCENT_BROKER_PASSWORD={SECRET}\n"
    )
    result = load(env_file)
    got = exported(result)
    assert got["SCENT_BROKER_URL"] == "wss://broker.example.test/mqtt"
    assert not any("PASSWORD" in key.upper() for key in got)
    assert SECRET not in result.stdout + result.stderr
    warning = result.stderr.splitlines()
    assert len(warning) == 1, result.stderr
    assert "SCENT_BROKER_PASSWORD" in warning[0]
    assert "Keychain" in warning[0]


def test_any_key_with_password_in_its_name_is_ignored(tmp_path):
    env_file = tmp_path / ".env"
    env_file.write_text(
        f"broker_password={SECRET}\n"
        f"export MY_PASSWORD_TOO='{SECRET}'\n"
        f"SCENT_ROUTE_ID=route-test-1\n"
    )
    result = load(env_file)
    got = exported(result)
    assert got["SCENT_ROUTE_ID"] == "route-test-1"
    assert not any("PASSWORD" in key.upper() for key in got)
    assert SECRET not in result.stdout + result.stderr
    assert len(result.stderr.splitlines()) == 2, result.stderr


def test_no_env_file_exports_nothing_and_says_nothing(tmp_path):
    result = load(tmp_path / ".env")
    got = exported(result)
    assert not any(key.startswith("SCENT_") for key in got)
    assert result.stderr == ""


def test_the_environment_wins_over_the_file(tmp_path):
    env_file = tmp_path / ".env"
    env_file.write_text("SCENT_BROKER_URL=wss://from-the-file.example.test/mqtt\n")
    result = load(env_file, {"SCENT_BROKER_URL": "wss://from-the-shell.example.test/mqtt"})
    assert exported(result)["SCENT_BROKER_URL"] == "wss://from-the-shell.example.test/mqtt"


def test_the_scripts_parse():
    for script in (LOADER, BUNDLE):
        subprocess.run(["bash", "-n", str(script)], env={"PATH": "/usr/bin:/bin"}, check=True)


def test_the_bundle_script_loads_the_repo_root_env_before_it_builds():
    text = BUNDLE.read_text()
    loads = text.index('load_broker_env "$root/.env"')
    builds = text.index("cargo build")
    assert loads < builds
    # A password never goes to the build, from .env or from the shell.
    assert "unset_passwords" in text[:builds]


def test_passwords_in_the_shell_do_not_reach_the_build(tmp_path):
    script = 'set -euo pipefail; source "$1"; unset_passwords; env'
    result = subprocess.run(
        ["bash", "-c", script, "bash", str(LOADER)],
        env={
            "PATH": "/usr/bin:/bin",
            "SCENT_BROKER_PASSWORD": SECRET,
            "some_password": SECRET,
            "SCENT_BROKER_URL": "wss://broker.example.test/mqtt",
        },
        capture_output=True,
        text=True,
        check=True,
    )
    got = exported(result)
    assert got["SCENT_BROKER_URL"] == "wss://broker.example.test/mqtt"
    assert SECRET not in result.stdout + result.stderr
