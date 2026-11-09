"""The packages proxy workstation step: key resolution, verify, the token file,
and the config writers -- netrc, go env, npmrc, uv, pip, cargo, gradle and
maven.

Every test runs the checked-in setup script end to end against a scratch HOME
with go, curl and vault stubbed (tests/setup_harness.py). The curl stub accepts
only the fake key, which reaches it on stdin (`-K -`) and never on argv; the
npmrc token and the cargo credentials token arrive on stdin as well, so the
key cannot appear in the run's output or the recorded calls. The uv, pip and
cargo configs are parsed with tomllib and configparser, the maven settings
with ElementTree, not eyeballed: the
value the tool would read is what is asserted on.
"""

from __future__ import annotations

import configparser
import stat
import tomllib
import xml.etree.ElementTree as ET
from pathlib import Path

from setup_harness import Run, setup_run

KEY = "fake-key-123"
WRONG_KEY = "rejected-key-456"
VERIFY_URL = "https://packages.jorisjonkers.dev/npm/typescript"
VERIFY_CALL = f"curl -sf -o /dev/null -K - {VERIFY_URL}"
TOKEN_REL = Path(".config") / "jorisjonkers" / "packages-token"
# NixOS has no /usr/bin or /bin: bash, mktemp and the rest live in the system
# profile. Appending it is a no-op on machines that do have /usr/bin.
SYSTEM_BIN = "/run/current-system/sw/bin" if Path("/run/current-system/sw/bin/bash").exists() else "/usr/bin"


def _curl_stub(root: Path) -> str:
    # Records the argv (never the key -- it rides the stdin curl config) and
    # accepts only the fake key.
    calls = root / "calls.log"
    return (
        'input="$(cat)"\n'
        f'echo "curl $*" >> "{calls}"\n'
        'case "${input}" in\n'
        f'  *"Authorization: Bearer {KEY}"*) exit 0 ;;\n'
        "  *) exit 22 ;;\n"
        "esac\n"
    )


def _go_stub(root: Path) -> str:
    # Records the argv and GOENV (the key never rides go's argv).
    calls = root / "calls.log"
    return f'echo "go $*" >> "{calls}"\necho "GOENV=$GOENV" >> "{calls}"\nexit 0\n'


def _vault_stub(*, ok: bool) -> str:
    if not ok:
        return "exit 1\n"
    return f'case "$1" in\n  token) exit 0 ;;\n  kv) echo {KEY} ;;\n  *) exit 1 ;;\nesac\n'


def _env(scratch: Path, **overrides: str) -> dict[str, str]:
    env = {
        "HOME": str(scratch),
        "XDG_CONFIG_HOME": str(scratch / ".config"),
        "XDG_CACHE_HOME": str(scratch / ".cache"),
        "CARGO_HOME": str(scratch / ".cargo"),
        "GRADLE_USER_HOME": str(scratch / ".gradle"),
        "GOENV": str(scratch / "goenv"),
    }
    env.update(overrides)
    return env


def _run(
    root: Path,
    *,
    scratch: Path | None = None,
    vault_ok: bool = False,
    env: dict[str, str] | None = None,
    preseed_key: str | None = None,
    args: tuple[str, ...] = (),
    without: tuple[str, ...] = (),
) -> Run:
    scratch = scratch if scratch is not None else root / "scratch"
    if preseed_key is not None:
        token = scratch / TOKEN_REL
        token.parent.mkdir(parents=True, exist_ok=True)
        token.write_text(preseed_key)
    extra = _env(scratch, PATH=f"{root}/bin:/usr/bin:/bin:{SYSTEM_BIN}", **(env or {}))
    stubs = {
        "go": _go_stub(root),
        "curl": _curl_stub(root),
        "vault": _vault_stub(ok=vault_ok),
    }
    return setup_run(root, "Linux", stubs=stubs, extra_env=extra, args=args, without=without)


def _token(scratch: Path) -> Path:
    return scratch / TOKEN_REL


def _assert_no_key_leak(run: Run, *keys: str) -> None:
    recorded = "\n".join(run.calls)
    for key in (KEY, WRONG_KEY, *keys):
        assert key not in run.stdout
        assert key not in run.stderr
        assert key not in recorded


def _assert_verified(run: Run) -> None:
    assert any(line == VERIFY_CALL for line in run.calls)


def _no_proxy_failure(run: Run) -> bool:
    # A run's exit code counts unrelated failures too (hermes, marketplaces,
    # clones), so this is what "the run still succeeds" can mean here: the
    # packages proxy step fails nothing itself.
    return not any("FAIL" in line and "packages proxy" in line for line in run.stderr.splitlines())


# 1. A home to write into, or nothing at all.


def test_without_a_home_it_warns_and_neither_crashes_nor_writes(tmp_path: Path) -> None:
    for home in ("", "/"):
        scratch = tmp_path / f"scratch-{home or 'empty'}"
        run = _run(tmp_path / f"run-{home or 'empty'}", scratch=scratch, env={"HOME": home})
        assert "packages proxy: no home directory to configure" in run.stderr
        assert "unbound variable" not in run.stderr
        assert not list(tmp_path.rglob("packages-token"))


# 2. PACKAGES_PROXY_TOKEN, accepted.


def test_a_key_from_the_environment_is_verified_and_stored(tmp_path: Path) -> None:
    run = _run(tmp_path, env={"PACKAGES_PROXY_TOKEN": KEY})
    token = _token(tmp_path / "scratch")
    assert token.read_text() == KEY
    assert stat.S_IMODE(token.stat().st_mode) == 0o600
    assert "the key is verified and stored" in run.stdout
    _assert_verified(run)
    _assert_no_key_leak(run)


# 3. PACKAGES_PROXY_TOKEN, refused.


def test_a_key_the_proxy_refuses_is_never_written(tmp_path: Path) -> None:
    run = _run(tmp_path, env={"PACKAGES_PROXY_TOKEN": WRONG_KEY})
    assert not _token(tmp_path / "scratch").exists()
    assert f"the key was refused by {VERIFY_URL}" in run.stderr
    _assert_no_key_leak(run)


# 4. The stored token file, reused.


def test_a_stored_key_is_reused_and_verified(tmp_path: Path) -> None:
    scratch = tmp_path / "scratch"
    token = _token(scratch)
    token.parent.mkdir(parents=True)
    token.write_text(KEY)
    token.chmod(0o644)
    run = _run(tmp_path, preseed_key=KEY)
    assert token.read_text() == KEY
    assert stat.S_IMODE(token.stat().st_mode) == 0o600
    _assert_verified(run)
    _assert_no_key_leak(run)


# 5. Vault.


def test_a_vault_key_is_used_and_stored(tmp_path: Path) -> None:
    run = _run(tmp_path, vault_ok=True)
    token = _token(tmp_path / "scratch")
    assert token.read_text() == KEY
    assert stat.S_IMODE(token.stat().st_mode) == 0o600
    _assert_verified(run)
    _assert_no_key_leak(run)


# 6. No key anywhere, nobody to ask.


def test_with_no_key_anywhere_it_warns_and_skips(tmp_path: Path) -> None:
    run = _run(tmp_path)
    assert "packages proxy: no key found" in run.stderr
    assert not _token(tmp_path / "scratch").exists()
    assert VERIFY_URL not in "\n".join(run.calls)
    _assert_no_key_leak(run)


# 7. Idempotence.


def test_a_second_run_leaves_the_token_file_byte_identical(tmp_path: Path) -> None:
    scratch = tmp_path / "shared-scratch"
    first = _run(tmp_path / "run-1", scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    token = _token(scratch)
    assert token.read_text() == KEY
    before = token.read_bytes()
    _run(tmp_path / "run-2", scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    assert token.read_bytes() == before
    _assert_no_key_leak(first)


# 8. A refused key never touches a stored one.


def test_a_failing_verify_does_not_overwrite_a_stored_key(tmp_path: Path) -> None:
    run = _run(tmp_path, preseed_key=KEY, env={"PACKAGES_PROXY_TOKEN": WRONG_KEY})
    token = _token(tmp_path / "scratch")
    assert token.read_text() == KEY
    assert f"the key was refused by {VERIFY_URL}" in run.stderr
    _assert_no_key_leak(run)


# The guards: the config root, --check and --cloud.


def test_a_config_root_outside_the_home_refuses_to_store(tmp_path: Path) -> None:
    elsewhere = tmp_path / "elsewhere"
    run = _run(tmp_path, env={"PACKAGES_PROXY_TOKEN": KEY, "XDG_CONFIG_HOME": str(elsewhere)})
    assert "the config root is not under" in run.stderr
    assert not (elsewhere / "jorisjonkers" / "packages-token").exists()
    _assert_no_key_leak(run)


def test_check_mode_neither_verifies_nor_writes(tmp_path: Path) -> None:
    run = _run(tmp_path, env={"PACKAGES_PROXY_TOKEN": KEY}, args=("--check",))
    assert "would look up the packages proxy key" in run.stdout
    assert "would store it under jorisjonkers/packages-token" in run.stdout
    assert VERIFY_URL not in "\n".join(run.calls)
    assert not _token(tmp_path / "scratch").exists()
    scratch = tmp_path / "scratch"
    assert not _uv_toml(scratch).exists()
    assert not _pip_conf(scratch).exists()
    assert not _cargo_config(scratch).exists()
    assert not _cargo_credentials(scratch).exists()
    assert _no_proxy_failure(run)
    _assert_no_key_leak(run)


def test_cloud_mode_skips_the_step(tmp_path: Path) -> None:
    run = _run(tmp_path, env={"PACKAGES_PROXY_TOKEN": KEY}, args=("--cloud",))
    assert "cloud session brings its own" in run.stdout
    assert VERIFY_URL not in "\n".join(run.calls)
    assert not _token(tmp_path / "scratch").exists()
    _assert_no_key_leak(run)


# 9. The writers: netrc, go env, npmrc.

NETRC_ENTRY = f"machine packages.jorisjonkers.dev login packages password {KEY}"
NPMRC_REGISTRY = "registry=https://packages.jorisjonkers.dev/npm/"
NPMRC_BLOCK = (
    "# agent-kit packages-proxy begin",
    NPMRC_REGISTRY,
    f"//packages.jorisjonkers.dev/npm/:_authToken={KEY}",
    "# agent-kit packages-proxy end",
)
GO_CALL = "go env -w GOPROXY=https://packages.jorisjonkers.dev/go GOPRIVATE=github.com/JorisJonkers-dev/*"
PYPI_INDEX = "https://packages.jorisjonkers.dev/pypi/simple/"
CARGO_INDEX = "sparse+https://packages.jorisjonkers.dev/cargo/"
CARGO_REGISTRY = "jorisjonkers-packages"


def _uv_toml(scratch: Path) -> Path:
    return scratch / ".config" / "uv" / "uv.toml"


def _pip_conf(scratch: Path) -> Path:
    return scratch / ".config" / "pip" / "pip.conf"


def _cargo_config(scratch: Path) -> Path:
    return scratch / ".cargo" / "config.toml"


def _cargo_credentials(scratch: Path) -> Path:
    return scratch / ".cargo" / "credentials.toml"


def _parse_pip(path: Path) -> configparser.ConfigParser:
    parser = configparser.ConfigParser(interpolation=None, strict=True)
    parser.read_string(path.read_text())
    return parser


def _assert_mode_600(path: Path) -> None:
    assert stat.S_IMODE(path.stat().st_mode) == 0o600


def test_a_fresh_root_gets_netrc_and_npmrc_at_0600(tmp_path: Path) -> None:
    run = _run(tmp_path, env={"PACKAGES_PROXY_TOKEN": KEY})
    scratch = tmp_path / "scratch"
    netrc = scratch / ".netrc"
    assert netrc.read_text() == NETRC_ENTRY + "\n"
    _assert_mode_600(netrc)
    npmrc = scratch / ".npmrc"
    assert npmrc.read_text().splitlines() == list(NPMRC_BLOCK)
    _assert_mode_600(npmrc)
    _assert_no_key_leak(run)


def test_npmrc_lines_outside_the_block_survive_byte_for_byte(tmp_path: Path) -> None:
    scratch = tmp_path / "scratch"
    original = (
        "@jorisjonkers-dev:registry=https://npm.pkg.github.com\n"
        "//npm.pkg.github.com/:_authToken=ghp_x\n"
        "save-exact=true\n"
    )
    npmrc = scratch / ".npmrc"
    npmrc.parent.mkdir(parents=True)
    npmrc.write_text(original)
    run = _run(tmp_path, scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    assert npmrc.read_text() == original + "\n" + "".join(line + "\n" for line in NPMRC_BLOCK)
    backup = scratch / ".npmrc.bak-agent-kit"
    assert backup.read_text() == original
    _assert_no_key_leak(run, "ghp_x")


def test_an_old_netrc_proxy_entry_is_replaced_in_place(tmp_path: Path) -> None:
    scratch = tmp_path / "scratch"
    netrc = scratch / ".netrc"
    netrc.parent.mkdir(parents=True)
    netrc.write_text(
        "machine github.com login gh password ghp_github\n"
        "machine packages.jorisjonkers.dev\n"
        "login packages\n"
        "password old-proxy-key\n"
        "default login anonymous\n"
    )
    run = _run(tmp_path, scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    lines = netrc.read_text().splitlines()
    assert lines[0] == "machine github.com login gh password ghp_github"
    assert "default login anonymous" in lines
    assert lines.count(NETRC_ENTRY) == 1
    assert "old-proxy-key" not in netrc.read_text()
    backup = scratch / ".netrc.bak-agent-kit"
    assert "old-proxy-key" in backup.read_text()
    _assert_no_key_leak(run)


def test_an_npmrc_registry_outside_the_block_refuses_the_writer(tmp_path: Path) -> None:
    scratch = tmp_path / "scratch"
    original = "registry=https://example.com/\nsave-exact=true\n"
    npmrc = scratch / ".npmrc"
    npmrc.parent.mkdir(parents=True)
    npmrc.write_text(original)
    run = _run(tmp_path, scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    assert npmrc.read_text() == original
    assert not (scratch / ".npmrc.bak-agent-kit").exists()
    assert "has its own registry= line, nothing written" in run.stderr
    # The other writers still ran, and the step fails nothing.
    assert (scratch / ".netrc").read_text() == NETRC_ENTRY + "\n"
    assert GO_CALL in "\n".join(run.calls)
    assert _no_proxy_failure(run)
    _assert_no_key_leak(run)


def test_the_go_env_call_and_the_case_go_is_missing(tmp_path: Path) -> None:
    run = _run(tmp_path, env={"PACKAGES_PROXY_TOKEN": KEY})
    goenv = tmp_path / "scratch" / "goenv"
    assert GO_CALL in run.calls
    assert f"GOENV={goenv}" in run.calls
    _assert_no_key_leak(run)
    run = _run(tmp_path / "without-go", without=("go",), env={"PACKAGES_PROXY_TOKEN": KEY})
    assert not any(line.startswith("go ") for line in run.calls)
    assert (tmp_path / "without-go" / "scratch" / ".netrc").read_text() == NETRC_ENTRY + "\n"
    assert not any(line.startswith("GOENV=") for line in run.calls)
    assert _no_proxy_failure(run)
    _assert_no_key_leak(run)


def test_a_goenv_outside_the_root_is_refused(tmp_path: Path) -> None:
    elsewhere = tmp_path / "elsewhere"
    run = _run(tmp_path, env={"PACKAGES_PROXY_TOKEN": KEY, "GOENV": str(elsewhere / "go" / "env")})
    assert "GOENV is not under" in run.stderr
    assert not any(line.startswith("go ") for line in run.calls)
    assert not elsewhere.exists()
    _assert_no_key_leak(run)


def test_a_second_run_writes_the_same_bytes_and_no_new_backups(tmp_path: Path) -> None:
    scratch = tmp_path / "shared-scratch"
    first = _run(tmp_path / "run-1", scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    netrc, npmrc, token = scratch / ".netrc", scratch / ".npmrc", _token(scratch)
    before = {path: path.read_bytes() for path in (netrc, npmrc, token)}
    baks_before = {path.name for path in scratch.rglob("*.bak-agent-kit")}
    _run(tmp_path / "run-2", scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    assert netrc.read_bytes() == before[netrc]
    assert npmrc.read_bytes() == before[npmrc]
    assert token.read_bytes() == before[token]
    assert {path.name for path in scratch.rglob("*.bak-agent-kit")} == baks_before
    _assert_no_key_leak(first)


# 10. The uv, pip and cargo writers.

UV_INDEX = [{"url": PYPI_INDEX, "default": True, "exclude-newer": "7 days"}]


def test_a_fresh_root_gets_the_uv_pip_and_cargo_configs(tmp_path: Path) -> None:
    run = _run(tmp_path, env={"PACKAGES_PROXY_TOKEN": KEY})
    scratch = tmp_path / "scratch"
    assert tomllib.loads(_uv_toml(scratch).read_text())["index"] == UV_INDEX
    assert _parse_pip(_pip_conf(scratch))["global"]["index-url"] == PYPI_INDEX
    cargo = tomllib.loads(_cargo_config(scratch).read_text())
    assert cargo["source"]["crates-io"]["replace-with"] == CARGO_REGISTRY
    assert cargo["source"][CARGO_REGISTRY]["registry"] == CARGO_INDEX
    assert cargo["registries"][CARGO_REGISTRY]["index"] == CARGO_INDEX
    assert cargo["registry"]["global-credential-providers"] == ["cargo:token"]
    assert cargo["registry"]["global-min-publish-age"] == "7 days"
    credentials = tomllib.loads(_cargo_credentials(scratch).read_text())
    assert credentials["registries"][CARGO_REGISTRY]["token"] == f"Bearer {KEY}"
    _assert_mode_600(_cargo_credentials(scratch))
    _assert_no_key_leak(run)


def test_an_unrelated_uv_table_survives_the_merge(tmp_path: Path) -> None:
    scratch = tmp_path / "scratch"
    original = "[tool.foo]\nbar = 1\n"
    uv_path = _uv_toml(scratch)
    uv_path.parent.mkdir(parents=True)
    uv_path.write_text(original)
    run = _run(tmp_path, scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    uv = tomllib.loads(uv_path.read_text())
    assert uv["tool"]["foo"]["bar"] == 1
    assert uv["index"] == UV_INDEX
    assert uv_path.with_name("uv.toml.bak-agent-kit").read_text() == original
    _assert_no_key_leak(run)


def test_a_preexisting_uv_exclude_newer_refuses_the_writer(tmp_path: Path) -> None:
    scratch = tmp_path / "scratch"
    original = '[tool.foo]\nbar = 1\nexclude-newer = "30 days"\n'
    uv_path = _uv_toml(scratch)
    uv_path.parent.mkdir(parents=True)
    uv_path.write_text(original)
    run = _run(tmp_path, scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    assert uv_path.read_text() == original
    assert not uv_path.with_name("uv.toml.bak-agent-kit").exists()
    assert "has its own exclude-newer, nothing written" in run.stderr
    # The other writers still ran, and the step fails nothing.
    assert (scratch / ".netrc").read_text() == NETRC_ENTRY + "\n"
    assert _parse_pip(_pip_conf(scratch))["global"]["index-url"] == PYPI_INDEX
    assert _no_proxy_failure(run)
    _assert_no_key_leak(run)


def test_an_existing_pip_global_is_not_repeated_and_keeps_its_lines(tmp_path: Path) -> None:
    scratch = tmp_path / "scratch"
    original = "[global]\ntimeout = 30\n"
    pip_path = _pip_conf(scratch)
    pip_path.parent.mkdir(parents=True)
    pip_path.write_text(original)
    run = _run(tmp_path, scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    pip = _parse_pip(pip_path)
    assert pip["global"]["timeout"] == "30"
    assert pip["global"]["index-url"] == PYPI_INDEX
    assert pip_path.read_text().count("[global]") == 1
    assert pip_path.with_name("pip.conf.bak-agent-kit").read_text() == original
    _assert_no_key_leak(run)


def test_a_pip_index_url_outside_the_block_refuses_the_writer(tmp_path: Path) -> None:
    scratch = tmp_path / "scratch"
    original = "[global]\nindex-url = https://pypi.example.com/simple\n"
    pip_path = _pip_conf(scratch)
    pip_path.parent.mkdir(parents=True)
    pip_path.write_text(original)
    run = _run(tmp_path, scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    assert pip_path.read_text() == original
    assert not pip_path.with_name("pip.conf.bak-agent-kit").exists()
    assert "has its own index-url, nothing written" in run.stderr
    # The other writers still ran, and the step fails nothing.
    assert (scratch / ".netrc").read_text() == NETRC_ENTRY + "\n"
    assert tomllib.loads(_uv_toml(scratch).read_text())["index"] == UV_INDEX
    assert _no_proxy_failure(run)
    _assert_no_key_leak(run)


def test_a_cargo_table_outside_the_block_refuses_config_but_not_credentials(tmp_path: Path) -> None:
    scratch = tmp_path / "scratch"
    original = '[source.crates-io]\nreplace-with = "my-mirror"\n'
    config = _cargo_config(scratch)
    config.parent.mkdir(parents=True)
    config.write_text(original)
    run = _run(tmp_path, scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    assert config.read_text() == original
    assert not config.with_name("config.toml.bak-agent-kit").exists()
    assert "has its own cargo registry tables, nothing written" in run.stderr
    # The credentials file is written anyway: its own header is clear.
    credentials = tomllib.loads(_cargo_credentials(scratch).read_text())
    assert credentials["registries"][CARGO_REGISTRY]["token"] == f"Bearer {KEY}"
    _assert_mode_600(_cargo_credentials(scratch))
    assert _parse_pip(_pip_conf(scratch))["global"]["index-url"] == PYPI_INDEX
    assert _no_proxy_failure(run)
    _assert_no_key_leak(run)


def test_a_cargo_home_outside_the_root_falls_back_to_the_root(tmp_path: Path) -> None:
    elsewhere = tmp_path / "elsewhere-cargo"
    run = _run(tmp_path, env={"PACKAGES_PROXY_TOKEN": KEY, "CARGO_HOME": str(elsewhere)})
    scratch = tmp_path / "scratch"
    cargo = tomllib.loads(_cargo_config(scratch).read_text())
    assert cargo["source"]["crates-io"]["replace-with"] == CARGO_REGISTRY
    credentials = tomllib.loads(_cargo_credentials(scratch).read_text())
    assert credentials["registries"][CARGO_REGISTRY]["token"] == f"Bearer {KEY}"
    _assert_mode_600(_cargo_credentials(scratch))
    assert not (elsewhere / "config.toml").exists()
    assert not (elsewhere / "credentials.toml").exists()
    _assert_no_key_leak(run)


def test_a_second_run_leaves_the_uv_pip_and_cargo_files_identical(tmp_path: Path) -> None:
    scratch = tmp_path / "shared-scratch"
    first = _run(tmp_path / "run-1", scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    paths = (
        _uv_toml(scratch),
        _pip_conf(scratch),
        _cargo_config(scratch),
        _cargo_credentials(scratch),
    )
    before = {path: path.read_bytes() for path in paths}
    baks_before = {path.name for path in scratch.rglob("*.bak-agent-kit")}
    _run(tmp_path / "run-2", scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    for path in paths:
        assert path.read_bytes() == before[path]
    assert {path.name for path in scratch.rglob("*.bak-agent-kit")} == baks_before
    _assert_no_key_leak(first)


# 11. The gradle and maven writers.

MAVEN_MIRROR_URL = "https://packages.jorisjonkers.dev/maven/"
GRADLE_BLOCK = (
    "# agent-kit packages-proxy begin",
    f"packagesProxyToken={KEY}",
    "# agent-kit packages-proxy end",
)


def _gradle_init(scratch: Path) -> Path:
    return scratch / ".gradle" / "init.d" / "packages-proxy.init.gradle.kts"


def _gradle_props(scratch: Path) -> Path:
    return scratch / ".gradle" / "gradle.properties"


def _maven_settings(scratch: Path) -> Path:
    return scratch / ".m2" / "settings.xml"


def _assert_block(path: Path, block: tuple[str, ...]) -> None:
    # The marked block holds exactly the given lines, in order.
    lines = path.read_text().splitlines()
    begin = lines.index(block[0])
    end = lines.index(block[-1])
    assert lines[begin : end + 1] == list(block)


def test_a_fresh_root_gets_the_gradle_init_script_without_the_key(tmp_path: Path) -> None:
    run = _run(tmp_path, env={"PACKAGES_PROXY_TOKEN": KEY})
    init = _gradle_init(tmp_path / "scratch")
    text = init.read_text()
    assert "maven.pkg.github.com" in text
    assert "packages.jorisjonkers.dev/maven/" in text
    assert "PACKAGES_PROXY_TOKEN" in text
    assert "packagesProxyToken" in text
    # Repositories are rewritten in place: none is added or removed.
    assert "fun proxyUrlFor" in text
    assert "repo.setUrl(target)" in text
    assert "remove(this)" not in text
    assert "addPackagesProxy" not in text
    # The init script is world-readable and so holds no key: Gradle reads the
    # token from the property or the environment at build time.
    assert KEY not in text
    assert stat.S_IMODE(init.stat().st_mode) == 0o644
    _assert_no_key_leak(run)


def test_the_gradle_properties_get_the_token_in_a_marked_block(tmp_path: Path) -> None:
    run = _run(tmp_path, env={"PACKAGES_PROXY_TOKEN": KEY})
    props = _gradle_props(tmp_path / "scratch")
    _assert_mode_600(props)
    _assert_block(props, GRADLE_BLOCK)
    _assert_no_key_leak(run)


def test_a_fresh_root_gets_maven_settings_with_the_mirror_and_server(tmp_path: Path) -> None:
    run = _run(tmp_path, env={"PACKAGES_PROXY_TOKEN": KEY})
    settings = _maven_settings(tmp_path / "scratch")
    _assert_mode_600(settings)
    settings_xml = ET.fromstring(settings.read_text())
    mirror = settings_xml.find("mirrors/mirror")
    assert mirror is not None and mirror.findtext("id") == "packages-proxy"
    assert mirror.findtext("url") == MAVEN_MIRROR_URL
    assert mirror.findtext("mirrorOf") == "external:*,!github"
    server = settings_xml.find("servers/server")
    assert server is not None and server.findtext("id") == "packages-proxy"
    assert server.findtext("username") == "packages"
    assert server.findtext("password") == KEY
    _assert_no_key_leak(run)


def test_a_preexisting_settings_xml_is_never_edited(tmp_path: Path) -> None:
    scratch = tmp_path / "scratch"
    original = "<settings><mirrors/></settings>\n"
    settings = _maven_settings(scratch)
    settings.parent.mkdir(parents=True)
    settings.write_text(original)
    run = _run(tmp_path, scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    # The writer is create-only: the snippet it logs carries the password as
    # a placeholder, so neither the file nor the output holds the key.
    assert settings.read_text() == original
    assert not settings.with_name("settings.xml.bak-agent-kit").exists()
    assert "PACKAGES_PROXY_KEY" in run.stdout
    _assert_no_key_leak(run)


def test_gradle_properties_lines_outside_the_block_survive(tmp_path: Path) -> None:
    scratch = tmp_path / "scratch"
    props = _gradle_props(scratch)
    props.parent.mkdir(parents=True)
    props.write_text("org.gradle.caching=true\n")
    run = _run(tmp_path, scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    assert "org.gradle.caching=true" in props.read_text().splitlines()
    _assert_block(props, GRADLE_BLOCK)
    _assert_mode_600(props)
    _assert_no_key_leak(run)


def test_a_gradle_proxy_token_outside_the_block_refuses_the_writer(tmp_path: Path) -> None:
    scratch = tmp_path / "scratch"
    original = "packagesProxyToken=other\n"
    props = _gradle_props(scratch)
    props.parent.mkdir(parents=True)
    props.write_text(original)
    run = _run(tmp_path, scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    assert props.read_text() == original
    assert not props.with_name("gradle.properties.bak-agent-kit").exists()
    assert "has its own packagesProxyToken, nothing written" in run.stderr
    assert _no_proxy_failure(run)
    _assert_no_key_leak(run)


def test_a_second_run_leaves_the_gradle_and_maven_files_identical(tmp_path: Path) -> None:
    scratch = tmp_path / "shared-scratch"
    first = _run(tmp_path / "run-1", scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    paths = (
        _gradle_init(scratch),
        _gradle_props(scratch),
        _maven_settings(scratch),
    )
    before = {path: path.read_bytes() for path in paths}
    _run(tmp_path / "run-2", scratch=scratch, env={"PACKAGES_PROXY_TOKEN": KEY})
    for path in paths:
        assert path.read_bytes() == before[path]
    assert not _maven_settings(scratch).with_name("settings.xml.bak-agent-kit").exists()
    _assert_no_key_leak(first)


def test_check_mode_writes_none_of_the_gradle_or_maven_files(tmp_path: Path) -> None:
    run = _run(tmp_path, env={"PACKAGES_PROXY_TOKEN": KEY}, args=("--check",))
    scratch = tmp_path / "scratch"
    assert not _gradle_init(scratch).exists()
    assert not _gradle_props(scratch).exists()
    assert not _maven_settings(scratch).exists()
    assert _no_proxy_failure(run)
    _assert_no_key_leak(run)
