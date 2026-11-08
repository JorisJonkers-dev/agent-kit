"""The Gold Band registry: a bad declaration is refused before anything is written.

The registry's ``gold_band.agents`` are flattened into the gold-band-agents
crate's line protocol and validated by its binary first thing in a setup run,
so a declaration that breaks an owner rule (#332) -- no Codex, no literal
secret, a well-formed id declared once, only known fields -- stops the run
before any file could be written. Every OpenRouter and every OpenCode model
(an ``opencode/`` one included) is allowed (owner decision, 2026-11-08): the
per-key LiteLLM budgets are the guard, not this registry. A failure that is no verdict -- a ``uv`` that cannot run, or
converge.py's own failure (exit 3) -- stops the run with its own message,
never the refusal's. The flattening cases call converge.py's function
directly; each refusal case runs the whole checked-in setup script against a
temporary home (tests/setup_harness.py), with a ``uv`` stub that runs
converge.py for real
against a registry the test wrote, so the crate's verdict -- not the stub's --
decides the run. The crate builds the way the canary tests build it, and the
run skips with setup's own reason where no Rust toolchain is there.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

import converge  # noqa: E402
from setup_harness import Run, setup_run  # noqa: E402


# ---------------------------------------------------------------------------
# The flattening: every field kind into the crate's line protocol
# ---------------------------------------------------------------------------


def test_every_field_kind_flattens_into_the_line_protocol() -> None:
    agent = {
        "id": "claude-code",
        "adapter": {
            "command": "/usr/local/bin/claude",
            "args": ["--continue", "--dangerously-accept"],
            "displayName": "Claude Code",
            "env": {"CLAUDE_CONFIG_DIR": "/Users/me/.claude", "ANTHROPIC_API_KEY": "${ANTHROPIC_API_KEY}"},
        },
        "icon": "sparkles",
        "primaryAgentDir": "/Users/me/.claude",
        "projectPrimaryAgentDir": "/Users/me/.claude/projects",
        "compatibleAgentDirs": ["/Users/me/.claude/agents", "/Users/me/.claude/commands"],
        "systemPromptDelivery": "meta-append",
        "externalSessionSyncSupported": True,
        "externalSessionSyncEnabled": False,
        "useLocalClaude": True,
        "reach": "read",
        "outputDir": "/Users/me/gold-band/out",
        "provider": "anthropic",
        "model": "claude-sonnet-4",
        "throwawayOnly": False,
    }
    assert converge.gold_band_line_protocol([agent]) == "\n".join([
        "agent\tclaude-code",
        "field\tadapter.command\t/usr/local/bin/claude",
        "field\tadapter.args.0\t--continue",
        "field\tadapter.args.1\t--dangerously-accept",
        "field\tadapter.displayName\tClaude Code",
        "field\tadapter.env.CLAUDE_CONFIG_DIR\t/Users/me/.claude",
        "field\tadapter.env.ANTHROPIC_API_KEY\t${ANTHROPIC_API_KEY}",
        "field\ticon\tsparkles",
        "field\tprimaryAgentDir\t/Users/me/.claude",
        "field\tprojectPrimaryAgentDir\t/Users/me/.claude/projects",
        "field\tcompatibleAgentDirs.0\t/Users/me/.claude/agents",
        "field\tcompatibleAgentDirs.1\t/Users/me/.claude/commands",
        "field\tsystemPromptDelivery\tmeta-append",
        "field\texternalSessionSyncSupported\ttrue",
        "field\texternalSessionSyncEnabled\tfalse",
        "field\tuseLocalClaude\ttrue",
        "field\treach\tread",
        "field\toutputDir\t/Users/me/gold-band/out",
        "field\tprovider\tanthropic",
        "field\tmodel\tclaude-sonnet-4",
        "field\tthrowawayOnly\tfalse",
        "",
    ])


def test_an_unknown_key_flattens_as_a_field_path_so_the_crate_refuses_it() -> None:
    agent = {"id": "zeta", "adapter": {"oops": 1}, "reach": "read", "favouriteColour": "blue"}
    assert converge.gold_band_line_protocol([agent]) == (
        "agent\tzeta\n"
        "field\tadapter.oops\t1\n"
        "field\treach\tread\n"
        "field\tfavouriteColour\tblue\n"
    )


def test_an_agent_without_an_id_opens_an_empty_declaration() -> None:
    assert converge.gold_band_line_protocol([{"reach": "read"}]) == "agent\t\nfield\treach\tread\n"


def test_no_agents_flatten_to_nothing() -> None:
    assert converge.gold_band_line_protocol([]) == ""


# ---------------------------------------------------------------------------
# Through setup: each refusal stops the run before anything is written
# ---------------------------------------------------------------------------


CLEAN_AGENT: dict[str, Any] = {
    "id": "claude-code",
    "adapter": {
        "command": "/usr/local/bin/claude",
        "args": ["--continue"],
        "displayName": "Claude Code",
        "env": {"ANTHROPIC_API_KEY": "${ANTHROPIC_API_KEY}"},
    },
    "icon": "sparkles",
    "primaryAgentDir": "/Users/me/.claude",
    "projectPrimaryAgentDir": "/Users/me/.claude/projects",
    "compatibleAgentDirs": ["/Users/me/.claude/agents"],
    "systemPromptDelivery": "meta-append",
    "externalSessionSyncSupported": True,
    "externalSessionSyncEnabled": False,
    "useLocalClaude": True,
    "reach": "read",
    "provider": "anthropic",
    "model": "claude-sonnet-4",
    "throwawayOnly": False,
}


@pytest.fixture(scope="module")
def built_crate() -> None:
    """Build the workspace's crates, the way setup's own skip line asks for (docs/RUST.md)."""
    if shutil.which("cargo") is None:
        pytest.skip("Rust toolchain (cargo) is absent: gold-band-agents cannot be built and run")
    build = subprocess.run(
        ["cargo", "build", "--release", "--manifest-path", str(KIT_ROOT / "scripts" / "Cargo.toml")],
        capture_output=True,
        text=True,
        check=False,
    )
    assert build.returncode == 0, build.stderr


def _setup_run(root: Path, agents: list[dict[str, Any]]) -> Run:
    """One setup run whose gold-band-validate call reads a registry the test wrote.

    The harness's default ``uv`` stub only records the call; this one runs
    converge.py for real, with the test's registry as its ``--registry``, so
    the crate's verdict decides the run the way it does on a workstation.
    """
    registry = root / "gold-band-agents.yaml"
    registry.write_text(yaml.safe_dump({"gold_band": {"agents": agents}}, sort_keys=False))
    stub = (
        'case "$*" in *gold-band-validate*)\n'
        f'  exec "{sys.executable}" "{KIT_ROOT / "scripts" / "converge.py"}" '
        f'gold-band-validate --registry "{registry}" ;;\n'
        "esac\n"
        f'echo "uv $*" >> "{root / "calls.log"}"\n'
    )
    return setup_run(root, "Linux", stubs={"uv": stub})


def _refused(run: Run, refusal: str) -> None:
    """The run stopped on the crate's refusal line, and nothing was written into the home."""
    assert run.returncode != 0, run.stdout
    assert refusal in run.stdout
    # The refusal is the run's first step, so nothing that later steps write
    # is there yet: no env.sh from tokens_and_keys, no Gold Band file at all.
    assert not (run.home / ".config" / "agent-kit" / "env.sh").exists(), "tokens_and_keys already ran"
    assert not (run.home / ".gold-band").exists(), "a Gold Band file was already written"


def test_a_refusal_writes_nothing_into_the_home(tmp_path: Path, built_crate: None) -> None:
    """The refusal happens before setup writes anything, not just before ~/.gold-band."""
    run = _setup_run(tmp_path, [{
        "id": "my-codex",
        "adapter": {"command": "/usr/local/bin/codex"},
        "reach": "read",
    }])
    assert run.returncode != 0
    assert list(run.home.iterdir()) == []


def test_a_codex_agent_is_refused(tmp_path: Path, built_crate: None) -> None:
    run = _setup_run(tmp_path, [{
        "id": "my-codex",
        "adapter": {"command": "/usr/local/bin/codex"},
        "reach": "read",
    }])
    _refused(run, "my-codex: id must not name codex")


def test_an_openrouter_provider_or_model_passes(tmp_path: Path, built_crate: None) -> None:
    run = _setup_run(tmp_path, [{
        "id": "mixer",
        "reach": "read",
        "provider": "openrouter",
        "model": "openrouter/anthropic/claude-3",
    }])
    assert "gold-band-agents: ok (1 agents)" in run.stdout
    assert "the Gold Band registry is refused" not in run.stdout + run.stderr
    # The validation stopped nothing: the run reached its next step.
    assert "tokens and keys" in run.stdout


def test_an_opencode_model_passes(tmp_path: Path, built_crate: None) -> None:
    run = _setup_run(tmp_path, [{"id": "sidekick", "reach": "read", "model": "opencode/zeta"}])
    assert "gold-band-agents: ok (1 agents)" in run.stdout
    assert "the Gold Band registry is refused" not in run.stdout + run.stderr
    # The validation stopped nothing: the run reached its next step.
    assert "tokens and keys" in run.stdout


def test_a_literal_secret_in_adapter_env_is_refused(tmp_path: Path, built_crate: None) -> None:
    run = _setup_run(tmp_path, [{
        "id": "zeta",
        "reach": "read",
        "adapter": {"env": {"OPENAI_API_KEY": "sk-AbCdEf1234567890Xy"}},
    }])
    _refused(run, "zeta: adapter.env.OPENAI_API_KEY holds a literal secret (openai key)")
    # The refusal names the field and the shape, never the secret itself.
    assert "sk-AbCdEf" not in run.stdout + run.stderr


def test_a_duplicate_id_is_refused(tmp_path: Path, built_crate: None) -> None:
    run = _setup_run(tmp_path, [
        {"id": "jules", "reach": "read"},
        {"id": "jules", "reach": "read"},
    ])
    _refused(run, "jules: duplicate id")


def test_a_malformed_id_is_refused(tmp_path: Path, built_crate: None) -> None:
    run = _setup_run(tmp_path, [{"id": "Bad_Id", "reach": "read"}])
    _refused(run, "Bad_Id: id must be lowercase letters and digits in dash-separated groups")


def test_an_unknown_agent_field_is_refused(tmp_path: Path, built_crate: None) -> None:
    run = _setup_run(tmp_path, [{"id": "zeta", "reach": "read", "favouriteColour": "blue"}])
    _refused(run, "zeta: unknown agent field favouriteColour")


def test_a_uv_that_cannot_run_is_not_reported_as_a_gold_band_refusal(tmp_path: Path) -> None:
    """A uv that fails is a failed validation step, not the crate refusing a declaration (#332)."""
    run = setup_run(tmp_path, "Linux", stubs={"uv": "exit 1"})
    assert "the Gold Band registry is refused" not in run.stdout + run.stderr
    assert run.returncode != 0, run.stdout
    assert "the Gold Band declarations could not be validated (exit 1; see above)" in run.stdout + run.stderr
    # Fail closed: no verdict arrived, so the run stops before the next step.
    assert "tokens and keys" not in run.stdout + run.stderr


def test_a_clean_declaration_passes_and_the_run_carries_on(tmp_path: Path, built_crate: None) -> None:
    run = _setup_run(tmp_path, [CLEAN_AGENT])
    assert "gold-band-agents: ok (1 agents)" in run.stdout
    assert "the Gold Band registry is refused" not in run.stdout + run.stderr
    # The validation came first and stopped nothing: the run reached its next step.
    assert "tokens and keys" in run.stdout
    assert not (run.home / ".gold-band").exists()
    assert not (run.home / ".gold-band").exists()


def test_regression_a_registry_that_is_not_a_mapping_cannot_be_validated(tmp_path: Path) -> None:
    """A registry whose document is a list is the wrong shape, not zero agents (#332 review)."""
    registry = tmp_path / "estate-tooling.yaml"
    registry.write_text("- just\n- a\n- list\n")
    assert converge.gold_band_validate(converge.KIT_ROOT, registry, cargo=True) == converge.GOLD_BAND_CANNOT_VALIDATE_EXIT
