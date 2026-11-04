"""The model gateway's shipped assets: its skill in every agent home, its
runner and defaults pinned in manifest.yaml, and the defaults validated
against the schema beside them.

The setup assertions run the checked-in setup script against a temp HOME with
every external command stubbed (tests/setup_harness.py).
"""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import re
import sys
import tomllib
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
from setup_harness import Run, setup_run  # noqa: E402

SKILL = KIT_ROOT / "skills" / "workstation" / "model-gateway"
RUNNER = SKILL / "model-gateway.mjs"
DEFAULTS = SKILL / "defaults.json"
SCHEMA = SKILL / "defaults.schema.json"
HOMES = (".claude", ".codex", ".vibe", ".vibe-private", ".gemini/config", ".gemini/antigravity-cli")
ROUTING = KIT_ROOT / "skills" / "workstation" / "critique" / "routing.json"
DELEGATE_AGENTS_DIR = render_registry.VIBE_DELEGATE_AGENTS_DIR
DELEGATE_AGENT = "delegate-readonly"
WRITE_DELEGATE_AGENT = "delegate-write"
# Every key a rendered delegate agent file may hold: Vibe's own settings, the
# same set the critic agent files are held to (#168 loaded them under
# VIBE_STRICT_CONFIG=1, which refuses any setting Vibe does not use).
DELEGATE_PROFILE_KEYS = {"display_name", "description", "safety"}
DELEGATE_SETTING_KEYS = {
    "active_model",
    "enabled_tools",
    "enabled_skills",
    "outside_workdir",
    "enable_mcp",
    "enable_subagents",
    "include_project_context",
    "session_logging",
    "tools",
    "models",
}


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _manifest() -> dict:
    return yaml.safe_load((KIT_ROOT / "manifest.yaml").read_text())


def _validator():
    spec = importlib.util.spec_from_file_location("validate_manifest", KIT_ROOT / "scripts" / "validate_manifest.py")
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.fixture(scope="module")
def macos_run(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("model-gateway-macos"), "Darwin")


# ---------------------------------------------------------------------------
# The shipped defaults (#294)
# ---------------------------------------------------------------------------


def test_the_shipped_defaults_name_every_client_with_its_cost_class_and_source() -> None:
    clients = json.loads(DEFAULTS.read_text())["clients"]
    assert {name: (c["binary"], c["cost_class"], c["model_source"]) for name, c in clients.items()} == {
        "vibe": ("vibe", "subsidised", "config"),
        "agy": ("agy", "subsidised", "live"),
        "claude": ("claude", "subsidised", "declared"),
        "hermes": ("hermes", "metered-personal", "config"),
        "opencode": ("opencode", "metered-personal", "live"),
        "codex": ("codex", "metered-employer", "declared"),
    }
    assert clients["agy"]["families"] == ["google"]
    assert clients["opencode"]["provider_cost_classes"] == {"opencode-go": "subsidised"}


def test_the_shipped_defaults_declare_every_client_model_with_its_tier() -> None:
    models = json.loads(DEFAULTS.read_text())["models"]
    assert [(m["client"], m["model"], m["family"], m["tier"]) for m in models] == [
        ("claude", "claude-opus-5-5", "anthropic", "frontier"),
        ("claude", "claude-sonnet-5-5", "anthropic", "hard"),
        ("claude", "claude-haiku-5-5", "anthropic", "routine"),
        ("codex", "gpt-5.5", "openai", "frontier"),
        ("vibe", "deepseek-v4-1-flash-1017", "deepseek", "routine"),
        ("vibe", "glm-5-3", "zhipu", "hard"),
        ("vibe", "mistral-large-latest", "mistral", "hard"),
        ("vibe", "mistral-small-latest", "mistral", "trivial"),
        ("agy", "gemini-3.8-flash-high", "google", "hard"),
    ]
    assert all(m["notes"].strip() for m in models)


def test_the_shipped_vibe_models_are_named_by_alias_and_record_their_config_names() -> None:
    models = json.loads(DEFAULTS.read_text())["models"]
    assert {m["model"]: m.get("names") for m in models if m["client"] == "vibe"} == {
        "deepseek-v4-1-flash-1017": None,
        "glm-5-3": ["zai-glm-5-3"],
        "mistral-large-latest": ["mistral-large-4-0"],
        "mistral-small-latest": ["mistral-small-latest"],
    }


def test_the_shipped_defaults_carry_the_settings_a_delegation_is_held_to() -> None:
    assert json.loads(DEFAULTS.read_text())["settings"] == {
        "plan_warning_ratio": 0.8,
        "summary_max_chars": 1500,
        "max_parallel_per_client": 3,
        "availability_window_minutes": 60,
        "metered_personal_caps": {"per_workflow_usd": 2, "per_day_usd": 5},
        "output_allowance_tokens": {"trivial": 500, "routine": 2000, "hard": 6000, "frontier": 16000},
    }


def test_the_schema_allows_an_output_allowance_per_tier() -> None:
    """A metered run's estimate counts its tier's allowance, so every tier has one."""
    allowances = json.loads(SCHEMA.read_text())["$defs"]["settings"]["properties"]["output_allowance_tokens"]
    assert allowances["required"] == ["trivial", "routine", "hard", "frontier"]
    assert allowances["additionalProperties"] is False
    assert all(allowances["properties"][tier]["minimum"] == 1 for tier in allowances["required"])


def test_the_shipped_defaults_price_every_vibe_model_from_critiques_routing() -> None:
    """A run whose client prices nothing is counted at these, so they are the
    prices critique already pays for the same models (routing.json)."""
    routing = json.loads(ROUTING.read_text())["clients"]
    priced = {
        client["model"]: client["prices"]
        for client in routing.values()
        if client["kind"] == "vibe"
    }
    models = json.loads(DEFAULTS.read_text())["models"]
    # The catalogue names a Vibe model by its alias and records the config's own
    # names (critique's routing uses those) beside it.
    by_name = {
        name: m["prices"]
        for m in models
        if m["client"] == "vibe"
        for name in [m["model"], *m.get("names", [])]
    }
    assert {name: by_name[name] for name in priced} == priced
    assert all(p["input"] > 0 and p["output"] > 0 and p["cached_input"] >= 0 for p in priced.values())


def test_the_schema_prices_a_model_above_zero() -> None:
    """0 turns every price cap off, so the schema refuses it."""
    prices = json.loads(SCHEMA.read_text())["$defs"]["model"]["properties"]["prices"]
    assert prices["required"] == ["input", "cached_input", "output"]
    assert prices["additionalProperties"] is False
    assert prices["properties"]["input"]["exclusiveMinimum"] == 0
    assert prices["properties"]["output"]["exclusiveMinimum"] == 0
    assert prices["properties"]["cached_input"]["minimum"] == 0


def test_the_schema_allows_exactly_the_contracts_enums() -> None:
    schema = json.loads(SCHEMA.read_text())
    client = schema["$defs"]["client"]["properties"]
    assert client["cost_class"]["enum"] == ["subsidised", "metered-personal", "metered-employer"]
    assert client["model_source"]["enum"] == ["live", "config", "declared"]
    assert schema["$defs"]["model"]["properties"]["tier"]["enum"] == ["trivial", "routine", "hard", "frontier"]


def test_the_schema_lets_a_client_set_a_cost_class_per_provider() -> None:
    schema = json.loads(SCHEMA.read_text())
    provider_cost_classes = schema["$defs"]["client"]["properties"]["provider_cost_classes"]
    assert provider_cost_classes["type"] == "object"
    assert provider_cost_classes["additionalProperties"]["enum"] == [
        "subsidised",
        "metered-personal",
        "metered-employer",
    ]
    assert "opencode" in schema["properties"]["clients"]["required"]
    assert "opencode" in schema["$defs"]["model"]["properties"]["client"]["enum"]


def test_the_schema_lets_a_model_record_the_names_its_client_configuration_gives_it() -> None:
    schema = json.loads(SCHEMA.read_text())
    model = schema["$defs"]["model"]
    assert "names" not in model["required"]
    assert model["properties"]["names"]["type"] == "array"
    assert model["properties"]["names"]["items"] == {"type": "string", "minLength": 1}


def test_the_schema_requires_every_key_and_allows_no_others() -> None:
    schema = json.loads(SCHEMA.read_text())
    assert schema["required"] == ["version", "clients", "models", "settings"]
    assert schema["additionalProperties"] is False
    assert schema["$defs"]["client"]["additionalProperties"] is False
    assert schema["$defs"]["model"]["additionalProperties"] is False
    assert schema["$defs"]["settings"]["additionalProperties"] is False


# ---------------------------------------------------------------------------
# The manifest pins and the Python validators
# ---------------------------------------------------------------------------


def test_the_runner_and_defaults_are_pinned_in_the_manifest() -> None:
    files = {f["path"]: f for f in _manifest()["model_gateway"]["files"]}
    assert set(files) == {
        "skills/workstation/model-gateway/model-gateway.mjs",
        "skills/workstation/model-gateway/defaults.json",
        "skills/workstation/model-gateway/defaults.schema.json",
    }
    assert files["skills/workstation/model-gateway/model-gateway.mjs"]["mode"] == "0755"
    for path in ("skills/workstation/model-gateway/defaults.json",
                 "skills/workstation/model-gateway/defaults.schema.json"):
        assert files[path]["mode"] == "0644", path
    for path, entry in files.items():
        digest = hashlib.sha256((KIT_ROOT / path).read_bytes()).hexdigest()
        assert entry["sha256"] == digest, f"{path}: rebuild council/ts and update manifest.yaml model_gateway.files"


def test_the_manifest_pins_every_file_the_skill_ships_but_its_skill_md() -> None:
    _validator().validate_model_gateway(_manifest())


def test_a_file_missing_from_the_pin_fails_validation() -> None:
    manifest = copy.deepcopy(_manifest())
    manifest["model_gateway"]["files"] = []
    with pytest.raises(AssertionError, match=re.escape("model_gateway.files mismatch")):
        _validator().validate_model_gateway(manifest)


def test_the_shipped_defaults_validate_against_the_schema() -> None:
    _validator().validate_model_gateway_defaults()


def test_a_defaults_file_that_breaks_the_schema_fails_validation(tmp_path: Path) -> None:
    broken = tmp_path / "defaults.json"
    defaults = json.loads(DEFAULTS.read_text())
    defaults["clients"]["codex"]["cost_class"] = "free"
    broken.write_text(json.dumps(defaults))
    with pytest.raises(AssertionError, match=re.escape("clients.codex.cost_class")):
        _validator().validate_model_gateway_defaults(broken, SCHEMA)


def test_a_defaults_file_that_is_not_json_fails_validation(tmp_path: Path) -> None:
    broken = tmp_path / "defaults.json"
    broken.write_text('{"version": 1,')
    with pytest.raises(AssertionError, match=re.escape("defaults.json")):
        _validator().validate_model_gateway_defaults(broken, SCHEMA)


def test_a_defaults_file_with_an_extra_key_fails_validation(tmp_path: Path) -> None:
    extra = tmp_path / "defaults.json"
    defaults = json.loads(DEFAULTS.read_text())
    defaults["settings"]["vibe_flow_cap_usd"] = 30
    extra.write_text(json.dumps(defaults))
    with pytest.raises(AssertionError, match=re.escape("settings.vibe_flow_cap_usd")):
        _validator().validate_model_gateway_defaults(extra, SCHEMA)


def test_a_defaults_file_with_a_provider_cost_class_the_schema_forbids_fails_validation(tmp_path: Path) -> None:
    broken = tmp_path / "defaults.json"
    defaults = json.loads(DEFAULTS.read_text())
    defaults["clients"]["opencode"]["provider_cost_classes"] = {"opencode-go": "free"}
    broken.write_text(json.dumps(defaults))
    with pytest.raises(AssertionError, match=re.escape("clients.opencode.provider_cost_classes.opencode-go")):
        _validator().validate_model_gateway_defaults(broken, SCHEMA)


def test_a_defaults_file_with_a_zero_input_price_fails_validation(tmp_path: Path) -> None:
    broken = tmp_path / "defaults.json"
    defaults = json.loads(DEFAULTS.read_text())
    defaults["models"][0]["prices"] = {"input": 0, "cached_input": 0, "output": 1.2}
    broken.write_text(json.dumps(defaults))
    with pytest.raises(AssertionError, match=re.escape("models[0].prices.input")):
        _validator().validate_model_gateway_defaults(broken, SCHEMA)


def test_a_defaults_file_missing_a_required_key_fails_validation(tmp_path: Path) -> None:
    missing = tmp_path / "defaults.json"
    defaults = json.loads(DEFAULTS.read_text())
    del defaults["models"][0]["tier"]
    missing.write_text(json.dumps(defaults))
    with pytest.raises(AssertionError, match=re.escape("models[0].tier")):
        _validator().validate_model_gateway_defaults(missing, SCHEMA)


def test_a_plan_limit_with_neither_dollars_nor_runs_fails_validation(tmp_path: Path) -> None:
    broken = tmp_path / "defaults.json"
    defaults = json.loads(DEFAULTS.read_text())
    defaults["clients"]["opencode"]["plan_limits"] = [{"window": "5h"}]
    broken.write_text(json.dumps(defaults))
    with pytest.raises(AssertionError, match=re.escape("clients.opencode.plan_limits[0] must match exactly one")):
        _validator().validate_model_gateway_defaults(broken, SCHEMA)


def test_a_plan_limit_with_both_dollars_and_runs_fails_validation(tmp_path: Path) -> None:
    broken = tmp_path / "defaults.json"
    defaults = json.loads(DEFAULTS.read_text())
    defaults["clients"]["opencode"]["plan_limits"] = [{"limit_runs": 5, "limit_usd": 12, "window": "5h"}]
    broken.write_text(json.dumps(defaults))
    with pytest.raises(AssertionError, match=re.escape("clients.opencode.plan_limits[0] must match exactly one")):
        _validator().validate_model_gateway_defaults(broken, SCHEMA)


def test_a_plan_limit_with_one_limit_passes_validation(tmp_path: Path) -> None:
    good = tmp_path / "defaults.json"
    defaults = json.loads(DEFAULTS.read_text())
    defaults["clients"]["opencode"]["plan_limits"] = [
        {"limit_usd": 12, "window": "5h"},
        {"limit_runs": 10, "window": "day"},
    ]
    good.write_text(json.dumps(defaults))
    _validator().validate_model_gateway_defaults(good, SCHEMA)


def test_a_plan_limit_with_a_window_the_schema_does_not_know_fails_validation(tmp_path: Path) -> None:
    broken = tmp_path / "defaults.json"
    defaults = json.loads(DEFAULTS.read_text())
    defaults["clients"]["opencode"]["plan_limits"] = [{"limit_usd": 12, "window": "fortnight"}]
    broken.write_text(json.dumps(defaults))
    with pytest.raises(AssertionError, match=re.escape("clients.opencode.plan_limits[0].window")):
        _validator().validate_model_gateway_defaults(broken, SCHEMA)


# ---------------------------------------------------------------------------
# Setup installs the skill folder in every agent home
# ---------------------------------------------------------------------------


def test_the_model_gateway_skill_is_declared_for_claude_codex_and_the_other_agents() -> None:
    skill = next(s for s in _registry()["local_skills"] if s["name"] == "model-gateway")
    assert skill["path"] == "skills/workstation/model-gateway"
    assert set(skill["agents"]) == {"claude", "codex", "vibe", "antigravity"}
    assert skill["surfaces"] == ["workstation"]
    assert "pending" not in skill


def test_setup_installs_the_whole_skill_into_every_agent_home(macos_run: Run) -> None:
    want = sorted(p.relative_to(SKILL).as_posix() for p in SKILL.rglob("*") if p.is_file())
    assert want == ["SKILL.md", "defaults.json", "defaults.schema.json", "model-gateway.mjs"]
    for home in HOMES:
        dest = macos_run.home / home / "skills" / "model-gateway"
        got = sorted(p.relative_to(dest).as_posix() for p in dest.rglob("*") if p.is_file())
        assert got == want, home
        assert (dest / "defaults.json").read_bytes() == DEFAULTS.read_bytes(), home
        assert (dest / "model-gateway.mjs").read_bytes() == RUNNER.read_bytes(), home


def test_a_second_run_replaces_an_older_copy_of_the_folder(tmp_path: Path) -> None:
    """The destination is replaced, not merged: a stale script from an older
    version is worse than a missing one."""

    def older_copy(home: Path) -> None:
        dest = home / ".claude" / "skills" / "model-gateway"
        dest.mkdir(parents=True)
        (dest / "stale.mjs").write_text("// from an older version\n")

    run = setup_run(tmp_path, "Darwin", prepare_home=older_copy)
    dest = run.home / ".claude" / "skills" / "model-gateway"
    got = sorted(p.relative_to(dest).as_posix() for p in dest.rglob("*") if p.is_file())
    assert got == ["SKILL.md", "defaults.json", "defaults.schema.json", "model-gateway.mjs"]
    assert (dest / "defaults.json").read_bytes() == DEFAULTS.read_bytes()
    assert f"ok    model-gateway -> {dest}" in run.stdout


def test_the_shipped_runner_is_executable_and_starts_with_a_node_shebang() -> None:
    assert RUNNER.stat().st_mode & 0o777 == 0o755
    assert RUNNER.read_text().splitlines()[0] == "#!/usr/bin/env node"


# ---------------------------------------------------------------------------
# The read-only delegate agent (#296)
# ---------------------------------------------------------------------------


def _delegate_file() -> dict:
    return tomllib.loads((KIT_ROOT / DELEGATE_AGENTS_DIR / f"{DELEGATE_AGENT}.toml").read_text())


def test_the_delegate_agents_are_declared_for_the_workstation() -> None:
    agents = _registry()["vibe_delegate_agents"]
    assert [(a["name"], a["surfaces"]) for a in agents] == [
        (DELEGATE_AGENT, ["workstation"]),
        (WRITE_DELEGATE_AGENT, ["workstation"]),
    ]


def test_the_delegate_agent_can_only_read_and_search_inside_its_working_directory() -> None:
    """Vibe 3.0.4 has no listing tool, so `grep` is how a delegate finds files.

    No bash, no writes, no skills, no subagents, no MCP: the gateway's
    read-only proof rests on this file, so it is the renderer's and no
    registry entry can widen it.
    """
    doc = _delegate_file()
    assert doc["enabled_tools"] == ["read_file", "grep"]
    assert "tools" not in doc
    assert doc["outside_workdir"] == "deny"
    assert "enable_mcp" not in doc
    assert "mcp_servers" not in doc
    assert doc["enable_subagents"] is False
    assert doc["include_project_context"] is False
    assert doc["enabled_skills"] == ["re:^$"]
    assert doc["session_logging"] == {"enabled": False}
    assert doc["safety"] == "safe"


def _write_delegate_file() -> dict:
    return tomllib.loads((KIT_ROOT / DELEGATE_AGENTS_DIR / f"{WRITE_DELEGATE_AGENT}.toml").read_text())


def test_the_write_delegate_reads_searches_writes_and_edits_inside_its_working_directory_and_runs_no_shell() -> None:
    """A writing delegation (#300) runs in a worktree the gateway made, so the agent may change files.

    It still has no bash (the gateway runs the caller's check itself), no
    subagents, no skills, no MCP, nothing outside the working directory and no
    project context. Its two write tools are always allowed, because a
    headless run has nobody to ask.
    """
    doc = _write_delegate_file()
    assert doc["enabled_tools"] == ["read_file", "grep", "write_file", "edit"]
    assert not {"bash", "task"} & set(doc["enabled_tools"])
    assert doc["tools"] == {"write_file": {"permission": "always"}, "edit": {"permission": "always"}}
    assert doc["outside_workdir"] == "deny"
    assert "enable_mcp" not in doc
    assert "mcp_servers" not in doc
    assert doc["enable_subagents"] is False
    assert doc["include_project_context"] is False
    assert doc["enabled_skills"] == ["re:^$"]
    assert doc["session_logging"] == {"enabled": False}
    assert doc["safety"] == "safe"
    assert "active_model" not in doc
    assert "models" not in doc
    assert "default_agent" not in doc
    assert not set(doc) - DELEGATE_PROFILE_KEYS - DELEGATE_SETTING_KEYS


def test_the_read_only_delegate_still_has_no_write_tool() -> None:
    assert not {"write_file", "edit", "bash"} & set(_delegate_file()["enabled_tools"])


def test_the_delegate_agent_pins_no_model_and_is_nobody_s_default() -> None:
    """The gateway passes `--model <alias>`; the file must not override it."""
    doc = _delegate_file()
    assert "active_model" not in doc
    assert "models" not in doc
    assert "default_agent" not in doc
    assert "default_agent" not in render_registry.render_setup_script(_registry())


def test_every_key_in_the_delegate_agent_file_is_a_vibe_setting() -> None:
    doc = _delegate_file()
    unknown = set(doc) - DELEGATE_PROFILE_KEYS - DELEGATE_SETTING_KEYS
    assert not unknown, unknown


def test_a_delegate_agent_entry_may_not_widen_the_file() -> None:
    data = copy.deepcopy(_registry())
    data["vibe_delegate_agents"][0]["enabled_tools"] = ["read_file", "bash"]
    with pytest.raises(render_registry.RegistryError, match="read-only settings are the same"):
        render_registry.validate(data)


def test_a_delegate_agent_must_be_named_delegate_something() -> None:
    data = copy.deepcopy(_registry())
    data["vibe_delegate_agents"][0]["name"] = "helper"
    with pytest.raises(render_registry.RegistryError, match="must be named delegate-"):
        render_registry.validate(data)


def test_a_file_left_over_from_a_retired_delegate_is_a_stale_artifact(tmp_path: Path) -> None:
    data = _registry()
    for relative, (content, _mode) in render_registry.artifacts(data).items():
        (tmp_path / relative).parent.mkdir(parents=True, exist_ok=True)
        (tmp_path / relative).write_text(content)
    assert render_registry.stale_artifacts(data, tmp_path) == []
    (tmp_path / DELEGATE_AGENTS_DIR / "delegate-gone.toml").write_text("safety = 'safe'\n")
    assert render_registry.stale_artifacts(data, tmp_path) == [str(DELEGATE_AGENTS_DIR / "delegate-gone.toml")]


@pytest.mark.parametrize("agent", [DELEGATE_AGENT, WRITE_DELEGATE_AGENT])
def test_setup_installs_the_delegate_agents_into_the_shared_vibe_home(macos_run: Run, agent: str) -> None:
    installed = macos_run.home / ".vibe" / "agents" / f"{agent}.toml"
    assert installed.read_text() == (KIT_ROOT / DELEGATE_AGENTS_DIR / f"{agent}.toml").read_text()
    assert f"Vibe delegate agent {agent} -> " in macos_run.stdout
    assert not (macos_run.home / ".vibe-private" / "agents" / f"{agent}.toml").exists()


# ---------------------------------------------------------------------------
# The skill text
# ---------------------------------------------------------------------------


def _skill_text() -> str:
    return (SKILL / "SKILL.md").read_text()


def _front() -> dict:
    return yaml.safe_load(_skill_text().split("---")[1])


def test_the_skill_says_it_is_not_a_workflow_of_its_own() -> None:
    front = _front()
    assert set(front) == {"name", "description"}
    assert front["name"] == "model-gateway"
    assert "not a workflow of its own" in front["description"]
    assert "delegate" in front["description"]


def test_the_skill_names_its_files_and_the_models_command() -> None:
    text = _skill_text()
    for name in ("model-gateway.mjs", "defaults.json", "defaults.schema.json"):
        assert name in text, name
    assert """model-gateway.mjs models --json '{"repo": "/path/to/repo"}'""" in text
    assert "model-gateway.mjs models --input <file.json>" in text


def test_the_skill_documents_the_run_command() -> None:
    text = _skill_text()
    for word in (
        "model-gateway.mjs run --json",
        "delegate-readonly",
        "throwaway worktree",
        "worktree-failed",
        "cleanup_warning",
        "delegate-write",
        "write-needs-repository",
        "check-failed",
        "worktrees clean",
        "kept-worktrees.json",
        "AGENT_KIT_DELEGATION",
        "ledger.jsonl",
        "SUMMARY:",
        "not-supported-yet",
    ):
        assert word in text, word


def test_the_skill_warns_that_the_check_runs_the_delegate_s_code() -> None:
    text = _skill_text()
    assert "The check runs the delegate's code" in text
    for word in ("sh -c", "trust", "staged diff"):
        assert word in text, word


def test_the_skill_says_the_data_policy_filters_what_models_offers() -> None:
    text = _skill_text()
    for word in ("critique-allowlist.json", "may_send", "inline_only", "nedap"):
        assert word in text, word


def test_the_skill_says_where_the_machine_override_lives() -> None:
    text = _skill_text()
    assert "$XDG_CONFIG_HOME/agent-kit/model-gateway.json" in text
    assert "~/.config/agent-kit/model-gateway.json" in text
    assert "disabled" in text


def test_the_skill_carries_no_assistant_attribution() -> None:
    text = _skill_text().lower()
    for mark in ("co-authored-by", "generated with", "noreply@anthropic.com", "\U0001f916"):
        assert mark not in text, mark
