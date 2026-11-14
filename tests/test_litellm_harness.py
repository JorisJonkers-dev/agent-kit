"""One tested way to run each harness on a LiteLLM route (agent-kit#737 step 5).

``scripts/render_registry.py`` renders the route table, ``scripts/litellm-harness.py`` writes
each harness's provider entries from it and launches a harness with the route's key from the
secret store. The key is only ever in the child's environment: never argv, a config file or output.
"""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import os
import stat
import sys
from pathlib import Path
from typing import Any

import pytest
import tomlkit
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402

SCRIPT = KIT_ROOT / "scripts" / "litellm-harness.py"
TABLE_PATH = KIT_ROOT / render_registry.LITELLM_HARNESSES
KEY = "sk-test-route-key-0123456789"

spec = importlib.util.spec_from_file_location("litellm_harness", SCRIPT)
assert spec and spec.loader
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)


def _table() -> dict[str, Any]:
    return {
        "base_url": "https://litellm.example/v1",
        "routes": [
            {
                "agent": "vibe",
                "provider": "litellm-vibe",
                "key_env": "LITELLM_VIBE_KEY",
                "data_scope": "any",
                "models": [{"id": "glm-5-3"}, {"id": "codestral", "contextWindow": 256000}],
            },
            {
                "agent": "public",
                "provider": "litellm-public",
                "key_env": "LITELLM_PUBLIC_KEY",
                "data_scope": "public",
                "models": [{"id": "gemini-flash"}],
            },
        ],
    }


@pytest.fixture
def table_file(tmp_path: Path) -> Path:
    path = tmp_path / "harnesses.json"
    path.write_text(json.dumps(_table()))
    return path


def _main(*argv: str, exec_fn: Any = None) -> tuple[int, str, str]:
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = harness.main(list(argv), exec_fn=exec_fn or (lambda *a: None))
    return code, out.getvalue(), err.getvalue()


def _helper(tmp_path: Path, body: str) -> str:
    path = tmp_path / "agent-kit-secret"
    path.write_text(f"#!{sys.executable}\nimport sys\n{body}\n")
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return str(path)


# ----- the rendered table -----------------------------------------------------------


def test_the_table_is_rendered_from_the_registry_and_holds_no_key():
    table = json.loads(TABLE_PATH.read_text())
    data = render_registry.load_registry()
    agents = [route["agent"] for route in data["litellm"]["routes"]]
    assert [route["agent"] for route in table["routes"]] == agents
    assert table["base_url"] == data["litellm"]["base_url"]
    for route in table["routes"]:
        assert route["provider"] == f"litellm-{route['agent']}"
        assert route["key_env"].startswith("LITELLM_") and route["models"]
    assert "sk-" not in TABLE_PATH.read_text()
    assert {r["agent"]: r["data_scope"] for r in table["routes"]}["public"] == "public"


# ----- configure --------------------------------------------------------------------


def test_codex_gets_a_model_provider_per_route_and_keeps_the_owners_config(tmp_path, table_file):
    config = tmp_path / "codex" / "config.toml"
    config.parent.mkdir()
    config.write_text(
        'model = "gpt-5"\n[model_providers.mine]\nname = "mine"\n[model_providers.litellm-old]\nname="x"\n'
    )
    code, out, _ = _main("--table", str(table_file), "configure", "codex", "--config", str(config))
    assert code == 0 and "written" in out
    doc = tomlkit.parse(config.read_text()).unwrap()
    assert doc["model"] == "gpt-5" and doc["model_providers"]["mine"] == {"name": "mine"}
    assert "litellm-old" not in doc["model_providers"]
    assert doc["model_providers"]["litellm-vibe"] == {
        "name": "LiteLLM vibe",
        "base_url": "https://litellm.example/v1",
        "env_key": "LITELLM_VIBE_KEY",
        "wire_api": "responses",
    }
    assert "litellm-public" in doc["model_providers"]
    assert (tmp_path / "codex" / "config.toml.bak").exists()
    assert _main("--table", str(table_file), "configure", "codex", "--config", str(config))[1].endswith("in place\n")


def test_codex_config_without_providers_gets_the_table(tmp_path, table_file):
    config = tmp_path / "config.toml"
    assert _main("--table", str(table_file), "configure", "codex", "--config", str(config))[0] == 0
    assert "litellm-vibe" in tomlkit.parse(config.read_text()).unwrap()["model_providers"]


def test_hermes_gets_a_custom_provider_with_a_base_url_and_a_key_variable(tmp_path, table_file):
    config = tmp_path / "config.yaml"
    config.write_text("model: x\nproviders:\n  mine: {base_url: http://a}\n  litellm-old: {}\n")
    assert _main("--table", str(table_file), "configure", "hermes", "--config", str(config))[0] == 0
    data = yaml.safe_load(config.read_text())
    assert data["model"] == "x" and data["providers"]["mine"] == {"base_url": "http://a"}
    entry = data["providers"]["litellm-vibe"]
    assert entry["base_url"] == "https://litellm.example/v1" and entry["key_env"] == "LITELLM_VIBE_KEY"
    assert entry["default_model"] == "glm-5-3" and "litellm-old" not in data["providers"]


def test_hermes_refuses_a_config_that_is_not_a_mapping(tmp_path, table_file):
    config = tmp_path / "config.yaml"
    config.write_text("- a\n")
    code, _, err = _main("--table", str(table_file), "configure", "hermes", "--config", str(config))
    assert code == 2 and "not a mapping" in err and config.read_text() == "- a\n"
    config.write_text("providers: [1]\n")
    assert _main("--table", str(table_file), "configure", "hermes", "--config", str(config))[0] == 2


def test_opencode_gets_a_provider_that_reads_its_key_from_the_environment(tmp_path, table_file):
    config = tmp_path / "opencode.json"
    config.write_text(json.dumps({"mcp": {"servers": {}}, "provider": {"mine": {"npm": "x"}}}))
    assert _main("--table", str(table_file), "configure", "opencode", "--config", str(config))[0] == 0
    data = json.loads(config.read_text())
    assert data["mcp"] == {"servers": {}} and data["provider"]["mine"] == {"npm": "x"}
    entry = data["provider"]["litellm-public"]
    assert entry["options"] == {"baseURL": "https://litellm.example/v1", "apiKey": "{env:LITELLM_PUBLIC_KEY}"}
    assert list(entry["models"]) == ["gemini-flash"]


def test_vibe_gets_providers_and_models_in_a_non_private_home(tmp_path, table_file, monkeypatch):
    monkeypatch.setenv("HOME", str(tmp_path))
    config = tmp_path / ".config" / "agent-kit" / "vibe-litellm" / "config.toml"
    config.parent.mkdir(parents=True)
    config.write_text(
        'active_model = "m"\n[[providers]]\nname = "mistral"\napi_base = "https://a"\n'
        '[[models]]\nname = "x"\nprovider = "mistral"\nalias = "x"\n'
        '[[models]]\nname = "old"\nprovider = "litellm-old"\nalias = "o"\n'
    )
    assert _main("--table", str(table_file), "configure", "vibe", "--config", str(config))[0] == 0
    doc = tomlkit.parse(config.read_text()).unwrap()
    assert doc["active_model"] == "m"
    assert [p["name"] for p in doc["providers"]] == ["mistral", "litellm-vibe", "litellm-public"]
    assert doc["providers"][1] == {
        "name": "litellm-vibe",
        "api_base": "https://litellm.example/v1",
        "api_key_env_var": "LITELLM_VIBE_KEY",
        "api_style": "openai",
        "backend": "generic",
    }
    assert [m["alias"] for m in doc["models"]] == [
        "x",
        "litellm-vibe/glm-5-3",
        "litellm-vibe/codestral",
        "litellm-public/gemini-flash",
    ]


@pytest.mark.parametrize("home_name", ["." + "vibe", "." + "vibe-private"])
def test_vibe_providers_are_never_written_into_the_default_or_private_home(
    tmp_path, table_file, monkeypatch, home_name
):
    monkeypatch.setenv("HOME", str(tmp_path))
    config = tmp_path / home_name / "config.toml"
    code, _, err = _main("--table", str(table_file), "configure", "vibe", "--config", str(config))
    assert code == 2 and "never writes" in err and not config.exists()


def test_a_home_named_private_is_refused_too(tmp_path, table_file):
    config = tmp_path / "my-private-home" / "config.toml"
    code, _, err = _main("--table", str(table_file), "configure", "vibe", "--config", str(config))
    assert code == 2 and "private" in err


def test_check_changes_nothing_and_a_foreign_file_is_refused(tmp_path, table_file):
    config = tmp_path / "opencode.json"
    code, out, _ = _main("--table", str(table_file), "configure", "opencode", "--config", str(config), "--check")
    assert code == 0 and "would be written" in out and not config.exists()
    config.write_text("[1, 2]")
    code, _, err = _main("--table", str(table_file), "configure", "opencode", "--config", str(config))
    assert code == 2 and config.read_text() == "[1, 2]" and "left as it is" in err


def test_no_config_file_holds_a_key(tmp_path, table_file):
    for name, ext in (("codex", "toml"), ("hermes", "yaml"), ("opencode", "json")):
        config = tmp_path / f"{name}.{ext}"
        _main("--table", str(table_file), "configure", name, "--config", str(config))
        assert "sk-" not in config.read_text()


def test_a_write_that_fails_leaves_no_temporary_file(tmp_path, monkeypatch):
    target = tmp_path / "out.json"

    def boom(*_args: Any) -> None:
        raise OSError("disk")

    monkeypatch.setattr(harness.os, "replace", boom)
    with pytest.raises(OSError):
        harness.write_atomically(target, "x")
    assert list(tmp_path.iterdir()) == []


# ----- run --------------------------------------------------------------------------


def _run(tmp_path, table_file, *argv):
    helper = _helper(tmp_path, f'assert sys.argv[1:] == ["get", "LITELLM_VIBE_KEY"]\nprint("{KEY}")')
    calls: list[tuple[str, list[str], dict[str, str]]] = []
    code, out, err = _main(
        "--table",
        str(table_file),
        "run",
        *argv[:2],
        "--secret-helper",
        helper,
        *argv[2:],
        exec_fn=lambda *call: calls.append(call),
    )
    return code, out, err, calls


def test_claude_code_gets_the_base_url_and_token_for_that_process_only(tmp_path, table_file, monkeypatch):
    monkeypatch.setenv("ANTHROPIC_API_KEY", "other")
    code, _, _, calls = _run(tmp_path, table_file, "claude", "vibe", "--model", "glm-5-3", "--", "-p", "hi")
    ((binary, argv, env),) = calls
    assert code == 0 and binary == "claude" and argv == ["claude", "--model", "glm-5-3", "-p", "hi"]
    assert env["ANTHROPIC_BASE_URL"] == "https://litellm.example" and env["ANTHROPIC_AUTH_TOKEN"] == KEY
    assert "ANTHROPIC_API_KEY" not in env
    # The launcher's own environment, the one a default profile would inherit, is untouched.
    assert os.environ.get("ANTHROPIC_AUTH_TOKEN") != KEY
    assert os.environ.get("ANTHROPIC_BASE_URL") != "https://litellm.example"


@pytest.mark.parametrize(
    ("name", "argv"),
    [
        ("codex", ["codex", "-c", 'model_provider="litellm-vibe"', "-m", "glm-5-3"]),
        ("hermes", ["hermes", "chat", "--provider", "litellm-vibe", "-m", "glm-5-3"]),
        ("opencode", ["opencode", "-m", "litellm-vibe/glm-5-3"]),
        ("vibe", ["vibe"]),
    ],
)
def test_each_other_harness_is_started_on_its_route_with_the_key_variable_set(tmp_path, table_file, name, argv):
    code, _, _, calls = _run(tmp_path, table_file, name, "vibe", "--model", "glm-5-3")
    ((binary, got, env),) = calls
    assert code == 0 and got == argv and binary == argv[0]
    assert env["LITELLM_VIBE_KEY"] == KEY
    if name == "vibe":
        assert env["VIBE_ACTIVE_MODEL"] == "litellm-vibe/glm-5-3"
        assert env["VIBE_HOME"].endswith("agent-kit/vibe-litellm")


def test_without_a_model_no_model_flag_is_passed(tmp_path, table_file):
    for name in harness.HARNESSES:
        _, _, _, calls = _run(tmp_path, table_file, name, "vibe")
        assert "-m" not in calls[0][1] and "--model" not in calls[0][1]
        assert "VIBE_ACTIVE_MODEL" not in calls[0][2]


def test_the_key_is_never_in_argv_or_output(tmp_path, table_file):
    for name in harness.HARNESSES:
        _, out, err, calls = _run(tmp_path, table_file, name, "vibe", "--model", "glm-5-3", "--", "extra")
        assert KEY not in out + err
        assert not any(KEY in arg for arg in calls[0][1])


def test_a_missing_key_a_missing_route_and_an_unlisted_model_are_refused_without_launching(tmp_path, table_file):
    launched: list[Any] = []
    failing = _helper(tmp_path, "sys.exit(1)")
    code, _, err = _main(
        "--table",
        str(table_file),
        "run",
        "codex",
        "vibe",
        "--secret-helper",
        failing,
        exec_fn=lambda *a: launched.append(a),
    )
    assert code == 2 and "LITELLM_VIBE_KEY" in err
    code, _, err = _main("--table", str(table_file), "run", "codex", "nope", exec_fn=lambda *a: launched.append(a))
    assert code == 2 and "no LiteLLM route" in err
    code, _, err = _main(
        "--table", str(table_file), "run", "codex", "vibe", "--model", "nope", exec_fn=lambda *a: launched.append(a)
    )
    assert code == 2 and "does not list" in err
    code, _, err = _main(
        "--table",
        str(table_file),
        "run",
        "codex",
        "vibe",
        "--secret-helper",
        str(tmp_path / "gone"),
        exec_fn=lambda *a: launched.append(a),
    )
    assert code == 2 and "did not run" in err
    assert launched == []


def test_vibe_never_runs_in_the_default_or_private_home(tmp_path, table_file, monkeypatch):
    monkeypatch.setenv("HOME", str(tmp_path))
    helper = _helper(tmp_path, f'print("{KEY}")')
    for home_name in ("." + "vibe", "." + "vibe-private"):
        code, _, err = _main(
            "--table",
            str(table_file),
            "run",
            "vibe",
            "vibe",
            "--secret-helper",
            helper,
            "--vibe-home",
            str(tmp_path / home_name),
        )
        assert code == 2 and "never writes" in err


def test_a_bad_table_is_refused(tmp_path):
    bad = tmp_path / "t.json"
    bad.write_text("{}")
    assert _main("--table", str(bad), "run", "codex", "vibe")[0] == 2
    assert _main("--table", str(tmp_path / "missing.json"), "run", "codex", "vibe")[0] == 2


def test_the_default_table_is_the_kits_then_the_installed_copy(tmp_path, monkeypatch):
    assert harness.default_table() == TABLE_PATH
    assert harness.default_helper().endswith(".local/bin/agent-kit-secret")
    monkeypatch.setattr(harness, "__file__", str(tmp_path / "bin" / "agent-kit-litellm"))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path))
    installed = tmp_path / "agent-kit" / "litellm-harnesses.json"
    installed.parent.mkdir()
    installed.write_text("{}")
    assert harness.default_table() == installed


def test_setup_installs_the_launcher_and_configures_each_harness():
    setup = (KIT_ROOT / "installer" / "setup-workstation.sh").read_text()
    for harness_name in ("codex", "hermes", "opencode", "vibe"):
        assert f"configure {harness_name} --config" in setup
    assert "agent-kit-litellm" in setup and "vibe-litellm/config.toml" in setup
    assert "sk-" not in setup.split("litellm_launcher=")[1].split("converge_mcp")[0]
