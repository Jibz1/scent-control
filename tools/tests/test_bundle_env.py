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


def test_inline_comments_and_crlf_line_ends_are_not_part_of_a_value(tmp_path):
    env_file = tmp_path / ".env"
    env_file.write_bytes(
        b"SCENT_BROKER_URL=wss://broker.example.test/mqtt # mine\r\n"
        b'SCENT_BROKER_USERNAME="example # user" # the comment, not the hash inside\r\n'
        b"SCENT_ROUTE_ID=route#1\r\n"
    )
    result = load(env_file)
    got = exported(result)
    assert got["SCENT_BROKER_URL"] == "wss://broker.example.test/mqtt"
    assert got["SCENT_BROKER_USERNAME"] == "example # user"
    # A # with no space before it is part of the value.
    assert got["SCENT_ROUTE_ID"] == "route#1"
    assert "\r" not in result.stdout
    assert result.stderr == ""


def test_an_empty_variable_in_the_environment_does_not_hide_the_file(tmp_path):
    # As in Python's tools: empty is the same as not set.
    env_file = tmp_path / ".env"
    env_file.write_text("SCENT_BROKER_URL=wss://broker.example.test/mqtt\n")
    result = load(env_file, {"SCENT_BROKER_URL": ""})
    assert exported(result)["SCENT_BROKER_URL"] == "wss://broker.example.test/mqtt"


def test_a_malformed_password_line_is_told_by_its_number_never_its_text(tmp_path):
    env_file = tmp_path / ".env"
    env_file.write_text(
        "SCENT_ROUTE_ID=route-test-1\n"
        f"PASSWORD: {SECRET}=x\n"
    )
    result = load(env_file)
    assert SECRET not in result.stdout + result.stderr
    warning = result.stderr.splitlines()
    assert len(warning) == 1, result.stderr
    assert "line 2" in warning[0]
    assert exported(result)["SCENT_ROUTE_ID"] == "route-test-1"


def check_url(url: str) -> subprocess.CompletedProcess:
    """Runs the bundle script's check of the broker address on `url`, and prints the
    environment the build would get."""
    script = 'set -euo pipefail; source "$1"; check_broker_url; env'
    return subprocess.run(
        ["bash", "-c", script, "bash", str(LOADER)],
        env={"PATH": "/usr/bin:/bin", "SCENT_BROKER_URL": url},
        capture_output=True,
        text=True,
        check=True,
    )


def test_a_broker_url_the_app_would_turn_down_is_left_out_with_a_warning():
    for bad in [
        "mqtt://broker.example.test",
        "broker.example.test/mqtt",
        f"wss://someone:{SECRET}@broker.example.test/mqtt",
    ]:
        result = check_url(bad)
        assert "SCENT_BROKER_URL" not in exported(result), bad
        warning = result.stderr.splitlines()
        assert len(warning) == 1, result.stderr
        assert "wss://" in warning[0] and "Settings" in warning[0]
        # The address itself is never shown: it may hold a password.
        assert bad not in result.stderr and SECRET not in result.stdout + result.stderr


def test_a_good_or_missing_broker_url_passes_the_check_quietly():
    good = check_url("wss://broker.example.test/mqtt")
    assert exported(good)["SCENT_BROKER_URL"] == "wss://broker.example.test/mqtt"
    assert good.stderr == ""
    empty = check_url("")
    assert empty.stderr == ""


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
    checks = text.index("check_broker_url")
    builds = text.index("cargo build")
    assert loads < checks < builds
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
