"""setup-workstation.sh --cloud, the kit bundle it re-runs, and the cloud/ files."""

from __future__ import annotations

import copy
import hashlib
import os
import re
import subprocess
import sys
import tarfile
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402

CLOUD_HELPER = KIT_ROOT / "installer" / "cloud-session.sh"
SETUP_SCRIPT = KIT_ROOT / "cloud" / "setup-script.sh"
ENV_TEMPLATE = KIT_ROOT / "cloud" / "environment.env"

STUB_LOGGERS = "\n".join(
    [
        "failures=0",
        "warnings=0",
        'log()  { echo "log $*"; }',
        'ok()   { echo "ok $*"; }',
        'warn() { echo "warn $*"; }',
        'fail() { echo "FAIL $*"; }',
    ],
)


def _registry() -> dict:
    return copy.deepcopy(render_registry.load_registry())


def _script() -> str:
    return render_registry.render_setup_script(_registry())


def _shell_function(name: str) -> str:
    script = _script()
    start = script.index(f"{name}() {{")
    return script[start : script.index("\n}\n", start) + 3]


def _bash(body: str, env: dict[str, str] | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["bash", "-c", body],
        capture_output=True,
        text=True,
        check=False,
        env={"PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "/tmp"), **(env or {})},
    )


def _stub(bin_dir: Path, name: str, body: str) -> None:
    bin_dir.mkdir(parents=True, exist_ok=True)
    path = bin_dir / name
    path.write_text("#!/usr/bin/env bash\n" + body + "\n")
    path.chmod(0o755)


# ---------------------------------------------------------------------------
# secret_ref: a value locally, a ${VAR} reference in the cloud
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    ("cloud", "args", "env", "expected"),
    [
        (0, "TOKEN", {"TOKEN": "smt_abc"}, "smt_abc"),
        (0, "TOKEN", {}, ""),
        (1, "TOKEN", {"TOKEN": "smt_abc"}, "${TOKEN}"),
        (1, "TOKEN optional", {}, "${TOKEN:-}"),
    ],
)
def test_secret_ref_keeps_the_token_out_of_a_cloud_registration(
    cloud: int, args: str, env: dict[str, str], expected: str,
) -> None:
    body = f"set -uo pipefail\nCLOUD={cloud}\n{_shell_function('secret_ref')}\nsecret_ref {args}"
    result = _bash(body, env)
    assert result.returncode == 0, result.stderr
    assert result.stdout == expected


def test_every_claude_credential_goes_through_secret_ref() -> None:
    script = _script()
    for line in script.splitlines():
        if "claude_each_profile claude mcp add" not in line:
            continue
        assert not re.search(r'Bearer \$\{[A-Z_]+\}', line), line
        assert not re.search(r'--env [A-Z_]+="\$\{[A-Z_]+(:-)?\}"', line.replace("OVERLEAF_BASE_URL", "")), line


def test_a_cloud_run_does_not_skip_a_server_for_an_unset_token() -> None:
    """The token arrives with the session, after setup; setup may not see it."""
    script = _script()
    for server in _registry()["mcp_servers"]:
        if "workstation" in server["surfaces"] and server.get("credential") and not server.get("credential_optional"):
            assert f'if [ -z "${{{server["credential"]}:-}}" ] && [ "${{CLOUD}}" != 1 ]; then' in script


# ---------------------------------------------------------------------------
# cloud: false
# ---------------------------------------------------------------------------


def test_cloud_false_entries_are_skipped_in_cloud_mode_only() -> None:
    data = _registry()
    script = _script()
    off = []
    for key in ("clis", "plugins", "language_servers", "mcp_servers"):
        for item in data.get(key) or []:
            if "surfaces" in item and "workstation" not in item["surfaces"]:
                continue
            label = item.get("name") or item.get("plugin")
            marker = f'log "{label}: not set up in cloud mode"'
            if item.get("cloud") is False:
                off.append(label)
                assert marker in script, label
            elif item.get("cloud") is not False:
                assert marker not in script, label
    assert {"claude-code", "kubernetes", "github"} <= set(off)
    assert "kotlin-lsp" not in off


def test_the_cloud_check_does_not_expect_a_server_it_skipped() -> None:
    script = _script()
    assert "expected_servers=(memory-api memory-mcp playwright drawio overleaf)" in script
    assert 'if [ "${CLOUD}" != 1 ]; then\n        expected_servers+=(kubernetes)' in script


def test_cloud_implies_a_single_profile() -> None:
    assert "--cloud) CLOUD=1; DO_PROFILES=0 ;;" in _script()


def test_cloud_must_be_a_boolean() -> None:
    data = _registry()
    data["mcp_servers"][0]["cloud"] = "no"
    with pytest.raises(render_registry.RegistryError, match="cloud: must be true or false"):
        render_registry.validate(data)


# ---------------------------------------------------------------------------
# install_linux
# ---------------------------------------------------------------------------


def test_linux_gets_its_own_install_for_a_brew_only_server() -> None:
    script = _script()
    block = script[script.index("# kotlin-lsp -> kotlin-lsp") : script.index("# jdtls-lsp -> jdtls")]
    assert 'if [ "$(uname -s)" = Linux ]; then' in block
    assert "kotlin-server-${v}.tar.gz" in block
    assert "sha256sum -c -" in block
    assert "lsp_cmd='brew install kotlin-lsp'" in block


def test_install_linux_needs_an_install_beside_it() -> None:
    data = _registry()
    server = next(s for s in data["language_servers"] if s["plugin"] == "kotlin-lsp")
    server["install"] = None
    with pytest.raises(render_registry.RegistryError, match="install_linux"):
        render_registry.validate(data)


def test_the_kotlin_install_command_runs_under_bash(tmp_path: Path) -> None:
    """Prove the rendered command against a local tarball instead of the CDN."""
    server = next(s for s in _registry()["language_servers"] if s["plugin"] == "kotlin-lsp")
    root = tmp_path / "src" / "kotlin-server-0"
    root.mkdir(parents=True)
    (root / "kotlin-lsp.sh").write_text("#!/usr/bin/env bash\necho kotlin-lsp-ran\n")
    (root / "kotlin-lsp.sh").chmod(0o755)
    tarball = tmp_path / "k.tgz"
    with tarfile.open(tarball, "w:gz") as tar:
        tar.add(root, arcname="kotlin-server-0")
    digest = hashlib.sha256(tarball.read_bytes()).hexdigest()

    command = re.sub(r"sum=[0-9a-f]{64}", f"sum={digest}", server["install_linux"])
    command = re.sub(r'curl -fsSL -o (\S+) "[^"]+"', rf"cp {tarball} \1", command)
    bin_stub = tmp_path / "bin"
    _stub(bin_stub, "sha256sum", 'exec shasum -a 256 "$@"')
    env = {
        "PATH": f"{bin_stub}:{os.environ['PATH']}",
        "AK_BIN_DIR": str(tmp_path / "out" / "bin"),
        "AK_OPT_DIR": str(tmp_path / "out" / "opt"),
    }
    result = _bash(command, env)
    assert result.returncode == 0, result.stdout + result.stderr
    launcher = tmp_path / "out" / "bin" / "kotlin-lsp"
    ran = subprocess.run([str(launcher)], capture_output=True, text=True, check=True)
    assert ran.stdout.strip() == "kotlin-lsp-ran"


# ---------------------------------------------------------------------------
# use_kit_bundle: a copy outside a checkout re-runs the bundled one
# ---------------------------------------------------------------------------


def _bundle(tmp_path: Path, checksum: str | None = None) -> Path:
    src = tmp_path / "bundle-src"
    (src / "installer").mkdir(parents=True)
    (src / "registry").mkdir()
    (src / "registry" / "estate-tooling.yaml").write_text("version: 1\n")
    (src / "installer" / "setup-workstation.sh").write_text(
        'echo "bundled-run args=[$*] stage=${AGENT_KIT_BUNDLE_STAGE}"\n',
    )
    bundle = tmp_path / "kit.tar.gz"
    with tarfile.open(bundle, "w:gz") as tar:
        tar.add(src / "installer", arcname="installer")
        tar.add(src / "registry", arcname="registry")
    digest = checksum or hashlib.sha256(bundle.read_bytes()).hexdigest()
    (tmp_path / "kit.tar.gz.sha256").write_text(f"{digest}  agent-kit-skills.tar.gz\n")
    return bundle


def _use_kit_bundle(kit_root: Path, bundle: Path) -> subprocess.CompletedProcess:
    body = "\n".join(
        [
            STUB_LOGGERS,
            "ORIG_ARGS=(--cloud --no-lsp)",
            f'KIT_ROOT="{kit_root}"',
            _shell_function("use_kit_bundle"),
            "use_kit_bundle",
            "echo fell-through",
        ],
    )
    return _bash(body, {"AGENT_KIT_SKILLS_BUNDLE_URL": f"file://{bundle}"})


def test_a_copy_outside_a_checkout_reruns_the_bundled_one_with_its_arguments(tmp_path: Path) -> None:
    bundle = _bundle(tmp_path)
    result = _use_kit_bundle(tmp_path / "not-a-checkout", bundle)
    assert result.returncode == 0, result.stdout + result.stderr
    assert "bundled-run args=[--cloud --no-lsp] stage=" in result.stdout
    assert "fell-through" not in result.stdout


def test_a_bundle_that_fails_its_checksum_never_runs(tmp_path: Path) -> None:
    bundle = _bundle(tmp_path, checksum="0" * 64)
    result = _use_kit_bundle(tmp_path / "not-a-checkout", bundle)
    assert result.returncode == 1
    assert "failed its sha256 check" in result.stdout
    assert "bundled-run" not in result.stdout


def test_a_checkout_runs_itself(tmp_path: Path) -> None:
    result = _use_kit_bundle(KIT_ROOT, tmp_path / "missing.tar.gz")
    assert result.returncode == 0
    assert result.stdout.strip() == "fell-through"


# ---------------------------------------------------------------------------
# installer/cloud-session.sh
# ---------------------------------------------------------------------------


def _cloud_finish(tmp_path: Path, env: dict[str, str]) -> subprocess.CompletedProcess:
    bin_dir = tmp_path / "bin"
    _stub(bin_dir, "hermes", "exit 0")
    _stub(
        bin_dir,
        "codex",
        'state="$HOME/codex-logged-in"\n'
        'case "$1 $2" in\n'
        '  "login status") [ -f "$state" ] ;;\n'
        '  "login --with-api-key") [ "$(cat)" = sk-test ] && touch "$state" ;;\n'
        "  *) exit 2 ;;\n"
        "esac",
    )
    body = "\n".join([STUB_LOGGERS, f". {CLOUD_HELPER}", "cloud_finish"])
    return _bash(body, {"PATH": f"{bin_dir}:/usr/bin:/bin", "HOME": str(tmp_path), **env})


def test_cloud_finish_logs_codex_in_with_the_key(tmp_path: Path) -> None:
    result = _cloud_finish(tmp_path, {"OPENAI_API_KEY": "sk-test"})
    assert "ok cloud: codex logged in with OPENAI_API_KEY" in result.stdout, result.stdout + result.stderr
    assert (tmp_path / "codex-logged-in").exists()


def test_cloud_finish_says_how_to_log_codex_in_without_a_key(tmp_path: Path) -> None:
    result = _cloud_finish(tmp_path, {})
    assert "warn cloud: codex is not logged in" in result.stdout


def test_cloud_finish_sets_a_hermes_model_once(tmp_path: Path) -> None:
    config = tmp_path / ".hermes" / "config.yaml"
    config.parent.mkdir()
    config.write_text("mcp_servers:\n  drawio:\n    command: drawio-mcp")
    for _ in range(2):
        result = _cloud_finish(tmp_path, {})
        assert "FAIL" not in result.stdout, result.stdout
    text = config.read_text()
    assert text.count("model:") == 1
    assert "mcp_servers:\n  drawio:\n    command: drawio-mcp\nmodel:\n" in text
    assert "  provider: openrouter\n" in text


# ---------------------------------------------------------------------------
# cloud/setup-script.sh and cloud/environment.env
# ---------------------------------------------------------------------------


def _paste(tmp_path: Path, curl_body: str) -> subprocess.CompletedProcess:
    bin_dir = tmp_path / "bin"
    _stub(bin_dir, "curl", curl_body)
    return subprocess.run(
        ["bash", str(SETUP_SCRIPT)],
        capture_output=True,
        text=True,
        check=False,
        env={
            "PATH": f"{bin_dir}:/usr/bin:/bin",
            "HOME": str(tmp_path),
            "TMPDIR": str(tmp_path),
            "AGENT_KIT_SETUP_LOG": str(tmp_path / "setup.log"),
        },
    )


def test_the_paste_runs_the_fetched_script_in_cloud_mode(tmp_path: Path) -> None:
    fetched = '#!/usr/bin/env bash\\n# agent-kit v9.9.9\\necho "ran with $*"; exit 3\\n'
    result = _paste(tmp_path, f'while [ "$1" != -o ]; do shift; done; printf \'{fetched}\' > "$2"')
    assert result.returncode == 0
    assert "agent-kit: # agent-kit v9.9.9 from https://assets.jorisjonkers.dev/setup-workstation.sh" in result.stdout
    assert "ran with --cloud" in (tmp_path / "setup.log").read_text()


def test_the_paste_lets_the_session_start_when_the_fetch_fails(tmp_path: Path) -> None:
    result = _paste(tmp_path, "exit 7")
    assert result.returncode == 0
    assert "cannot fetch" in result.stderr


def test_the_env_template_names_every_credential_a_cloud_session_reads() -> None:
    data = _registry()
    names = {
        line.split("=", 1)[0]
        for line in ENV_TEMPLATE.read_text().splitlines()
        if line and not line.startswith("#")
    }
    declared = re.search(r"AK_CLOUD_VARS=\(([^)]*)\)", CLOUD_HELPER.read_text())
    assert declared
    wanted = set(declared.group(1).split())
    for server in data["mcp_servers"]:
        if "workstation" in server["surfaces"] and server.get("cloud", True) and server.get("credential"):
            wanted.add(server["credential"])
    for plugin in data["plugins"]:
        if plugin.get("cloud", True):
            wanted.update(plugin.get("requires_env") or {})
    assert wanted <= names, sorted(wanted - names)
