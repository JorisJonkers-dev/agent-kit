"""setup-workstation.sh walks you through what it can fix, and stops nagging about what it cannot.

Tokens and keys come first: in a terminal, setup runs agent-kit-keys to get
them, then loads env.sh so the same run registers the servers that need them.
Everything they gate reports once, under the tokens step. Missing toolchains,
hand-added servers and a Homebrew uv are facts, not to-dos.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
import yaml  # noqa: E402

TABLE = Path("registry/generated/workstation-secrets.json")


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _script() -> str:
    return render_registry.render_setup_script(_registry())


def _fn(script: str, name: str) -> str:
    return f"{name}() {{" + script.split(f"\n{name}() {{")[1].split("\n}\n")[0] + "\n}\n"


def _helpers(script: str) -> str:
    start = script.index("\nlog()  {")
    return script[start : script.index("\n\n", start)] + "\n"


# --- the secrets table ---


def test_the_table_is_generated_from_the_registry_and_shipped() -> None:
    data = _registry()
    rendered = json.loads(render_registry.artifacts(data)[TABLE][0])
    assert rendered == data["workstation_secrets"]
    names = {e["env"] for e in rendered}
    assert {"HINDSIGHT_API_URL", "HINDSIGHT_API_TOKEN", "MEMORY_MCP_TOKEN", "MEMORY_PRIVATE_TOKEN",
            "MISTRAL_API_KEY"} <= names
    private = next(e for e in rendered if e["env"] == "MEMORY_PRIVATE_TOKEN")
    assert private["export"] is False and private["grant"] == "MEMORY_PRIVATE"
    publish = (KIT_ROOT / "scripts" / "publish-installer-artifacts.sh").read_text()
    paths = publish.split("KIT_PATHS=(")[1].split(")")[0].split()
    assert str(TABLE) in paths and "scripts/agent-kit-keys.py" in paths


def test_every_credential_a_workstation_server_needs_is_in_the_table() -> None:
    data = _registry()
    names = {e["env"] for e in data["workstation_secrets"]}
    for server in data["mcp_servers"]:
        if "workstation" in server["surfaces"] and server.get("credential") and not server.get("credential_optional"):
            assert server["credential"] in names, server["name"]


# --- the tokens step, run against a fake helper ---


def _step(tmp_path: Path, status: str, *, interactive: bool, cloud: bool = False,
          check_only: bool = False) -> tuple[subprocess.CompletedProcess, list[str]]:
    script = _script()
    kit = tmp_path / "kit"
    (kit / "scripts").mkdir(parents=True)
    calls = tmp_path / "calls"
    (kit / "scripts" / "agent-kit-keys.py").write_text(
        "import sys, os, pathlib\n"
        f"open({str(calls)!r}, 'a').write(sys.argv[-1] + '\\n')\n"
        f"if sys.argv[-1] == 'status': print({status!r}, end='')\n"
        "if sys.argv[-1] in ('ensure', 'write-env'):\n"
        "    p = pathlib.Path(os.environ['HOME'], '.config/agent-kit/env.sh')\n"
        "    p.parent.mkdir(parents=True, exist_ok=True)\n"
        "    p.write_text('export AK_LOADED=1\\nvibe() { :; }\\n')\n"
    )
    home = tmp_path / "home"
    home.mkdir()
    body = (
        f"set -u\nKIT_ROOT={kit}\nCLOUD={int(cloud)}\nCHECK_ONLY={int(check_only)}\nPROMPT=1\nPROMPT_TTY=/dev/null\n"
        "failures=0\nwarnings=0\ntodo_failures=()\ntodo_warnings=()\n"
        + _helpers(script)
        + f"interactive() {{ {'true' if interactive else 'false'}; }}\n"
        + _fn(script, "tokens_and_keys")
        + 'tokens_and_keys </dev/null\nprintf "LOADED=%s\\n" "${AK_LOADED:-no}"\n'
        + 'printf "VIBEFN=%s\\n" "$(declare -F vibe || echo none)"\n'
    )
    env = {**os.environ, "HOME": str(home), "PATH": f"{Path(sys.executable).parent}:/usr/bin:/bin"}
    env["PATH"] = f"{tmp_path}/py:{env['PATH']}"
    (tmp_path / "py").mkdir()
    (tmp_path / "py" / "python3").symlink_to(sys.executable)
    result = subprocess.run(["/bin/bash", "-c", body], capture_output=True, text=True, check=False, env=env)
    return result, calls.read_text().split() if calls.exists() else []


def test_in_a_terminal_setup_gets_the_tokens_then_loads_them(tmp_path: Path) -> None:
    result, calls = _step(tmp_path, "HINDSIGHT_API_TOKEN ok\nshell ok\n", interactive=True)
    assert result.returncode == 0, result.stderr
    assert calls == ["ensure", "status"]
    assert "LOADED=1" in result.stdout
    # Setup's own `command -v vibe` must find the binary, not env.sh's wrapper.
    assert "VIBEFN=none" in result.stdout
    assert "warn" not in result.stderr


def test_without_a_terminal_it_reports_what_is_missing_and_how_to_fix_it(tmp_path: Path) -> None:
    status = "HINDSIGHT_API_TOKEN missing\nMEMORY_MCP_TOKEN invalid\nMISTRAL_API_KEY ok\nshell missing\n"
    result, calls = _step(tmp_path, status, interactive=False)
    assert calls == ["write-env", "status"]
    warns = [line for line in result.stderr.splitlines() if "warn" in line]
    assert len(warns) == 3
    assert all("run setup in a terminal" in w for w in warns)
    assert any("HINDSIGHT_API_TOKEN" in w and "missing" in w for w in warns)
    assert any("MEMORY_MCP_TOKEN" in w and "refused" in w for w in warns)
    assert any("env.sh" in w for w in warns)


def test_a_cloud_session_brings_its_own_tokens(tmp_path: Path) -> None:
    result, calls = _step(tmp_path, "HINDSIGHT_API_TOKEN missing\n", interactive=True, cloud=True)
    assert calls == []
    assert "warn" not in result.stderr


def test_check_mode_only_reports(tmp_path: Path) -> None:
    result, calls = _step(tmp_path, "HINDSIGHT_API_TOKEN missing\nshell ok\n", interactive=False, check_only=True)
    assert calls == ["status"]
    assert "HINDSIGHT_API_TOKEN" in result.stderr


def test_the_tokens_step_runs_before_anything_that_needs_them() -> None:
    script = _script()
    main = script.split(_fn(script, "tokens_and_keys"))[1]
    assert main.index("\ntokens_and_keys\n") < main.index('log "command-line tools"')


def test_interactive_means_a_terminal_and_no_opt_out() -> None:
    fn = _fn(_script(), "interactive")
    for part in ('"${PROMPT}" = 1', '"${CHECK_ONLY}" != 1', '"${CLOUD}" != 1', "[ -t 1 ]", "/dev/tty"):
        assert part in fn
    assert "--no-prompt) PROMPT=0 ;;" in _script()


# --- one report per cause ---


def test_a_managed_token_is_not_reported_again_by_what_it_gates() -> None:
    script = _script()
    for var in ("HINDSIGHT_API_TOKEN", "MEMORY_MCP_TOKEN", "MISTRAL_API_KEY", "HINDSIGHT_API_URL"):
        lines = [line.strip() for line in script.splitlines() if "warn " in line and f"{var} is not set" in line]
        for line in lines:
            idx = script.index(line)
            guard = script[script.rindex("\n", 0, script.rindex("\n", 0, idx)) : idx]
            assert 'if [ "${CLOUD}" = 1 ]; then' in guard, f"{var} warned outside the cloud: {line}"


def test_a_server_skipped_for_its_token_is_not_reported_as_missing_too() -> None:
    script = _script()
    assert "skipped_mcp_names+=(memory-api)" in script
    assert 'case " ${skipped_mcp_names[*]-} " in *" ${want} "*) continue ;; esac' in script


def test_hand_added_servers_are_noted_and_counted_in_the_current_shell() -> None:
    script = _script()
    assert "| while read -r found" not in script
    assert 'log "hand-added MCP server ${found} in ${profile_dir}: kept"' in script


def test_a_language_without_its_toolchain_is_not_a_to_do() -> None:
    script = _script()
    block = script[script.index("# rust-analyzer-lsp -> rust-analyzer") : script.index("# lua-lsp ->")]
    assert "if ! command -v rustup >/dev/null 2>&1; then" in block
    assert 'log "rust-analyzer-lsp: no rustup on this machine; plugin left off"' in block
    csharp = script[script.index("# csharp-lsp -> csharp-ls") : script.index("# swift-lsp ->")]
    assert "if ! command -v dotnet >/dev/null 2>&1; then" in csharp


def test_liquid_says_why_it_is_on() -> None:
    assert "liquid-lsp: shopify on PATH, so plugin enabled (off by default)" in _script()


def test_a_homebrew_uv_is_updated_by_homebrew() -> None:
    uv = next(c for c in _registry()["clis"] if c["name"] == "uv")
    assert uv["update"] == "uv self update 2>/dev/null || brew upgrade uv 2>/dev/null || true"


# --- curl | bash: no child reads the rest of the script ---


def test_a_piped_run_drains_stdin_and_detaches_it_before_running_the_bundle() -> None:
    fn = _fn(_script(), "use_kit_bundle")
    drain = fn.index("[ -t 0 ] || cat >/dev/null")
    run = fn.index('exec bash "${stage}/installer/setup-workstation.sh"')
    assert drain < run
    assert "</dev/null" in fn[run:]


def test_drain_then_detach_keeps_the_pipe_writer_happy(tmp_path: Path) -> None:
    # A pipe writer must not see EPIPE: the drained reader takes all of it.
    big = tmp_path / "big"
    big.write_text("x\n" * 200_000)
    result = subprocess.run(
        ["/bin/bash", "-c", f'cat {big} | /bin/bash -c \'[ -t 0 ] || cat >/dev/null; exec /bin/cat </dev/null\'; '
         'echo "pipe=${PIPESTATUS[0]}"'],
        capture_output=True, text=True, check=False,
    )
    assert "pipe=0" in result.stdout
