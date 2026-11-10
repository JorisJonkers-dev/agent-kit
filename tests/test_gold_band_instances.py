"""The Gold Band instances and the per-machine overlay are validated by the crate.

The flattener is plain Python and is tested directly. The rules live in the
gold-band-registry crate, so the refusals are driven through the real crate
(the binary named by GOLD_BAND_REGISTRY_BIN, else `cargo run`), and skipped
only on a machine that has neither.
"""

from __future__ import annotations

import copy
import os
import shutil
import sys
from pathlib import Path
from typing import Any

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import gold_band_instances as gbi  # noqa: E402
import render_registry  # noqa: E402

needs_crate = pytest.mark.skipif(
    shutil.which("cargo") is None and not os.environ.get(gbi.BIN_ENV),
    reason="neither cargo nor GOLD_BAND_REGISTRY_BIN is available",
)


def _registry() -> dict[str, Any]:
    return yaml.safe_load(render_registry.REGISTRY_PATH.read_text())  # type: ignore[no-any-return]


def _overlay(tmp_path: Path, body: str) -> Path:
    path = tmp_path / "overlay.yaml"
    path.write_text(body)
    return path


def test_flatten_numbers_lists_and_dots_nested_mappings() -> None:
    text = gbi.flatten(
        {gbi.REGISTRY_KEY: {"gold-band": {"agents": ["codex"], "repository_rule": {"remotes": "nedap-only"}}}}, None
    )

    assert text.splitlines() == [
        "source\tregistry",
        "instance\tgold-band",
        "field\tagents.0\tcodex",
        "field\trepository_rule.remotes\tnedap-only",
    ]


def test_flatten_adds_the_overlay_section_and_renders_booleans_and_numbers() -> None:
    overlay = {
        gbi.OVERLAY_KEY: {"gold-band": {"settings": {"on": True, "off": False, "n": 3}}, "gold-band-personal": None}
    }

    lines = gbi.flatten({gbi.REGISTRY_KEY: {}}, overlay).splitlines()

    assert lines[1:] == [
        "source\toverlay",
        "instance\tgold-band",
        "field\tsettings.on\ttrue",
        "field\tsettings.off\tfalse",
        "field\tsettings.n\t3",
        "instance\tgold-band-personal",
    ]


@pytest.mark.parametrize(
    "registry,overlay,message",
    [
        ({}, None, "no gold_band_instances section"),
        ({gbi.REGISTRY_KEY: []}, None, "must be a mapping"),
        ({gbi.REGISTRY_KEY: {"a": "x"}}, None, "must be a mapping"),
        ({gbi.REGISTRY_KEY: {"a\tb": {}}}, None, "tab or newline"),
        ({gbi.REGISTRY_KEY: {"a": {"b.c": "x"}}}, None, "empty or holds a dot"),
        ({gbi.REGISTRY_KEY: {"a": {"b": "x\ny"}}}, None, "tab or newline"),
        ({gbi.REGISTRY_KEY: {"a": {"b": None}}}, None, "must be a string"),
        ({gbi.REGISTRY_KEY: {}}, {gbi.OVERLAY_KEY: []}, "must be a mapping"),
    ],
)
def test_flatten_refuses_what_it_cannot_express(
    registry: dict[str, Any], overlay: dict[str, Any] | None, message: str
) -> None:
    with pytest.raises(gbi.InstanceError, match=message):
        gbi.flatten(registry, overlay)


def test_load_overlay_reads_a_missing_empty_and_odd_file(tmp_path: Path) -> None:
    assert gbi.load_overlay(tmp_path / "none.yaml") is None
    assert gbi.load_overlay(_overlay(tmp_path, "")) == {}
    with pytest.raises(gbi.InstanceError, match="must be a mapping"):
        gbi.load_overlay(_overlay(tmp_path, "- x\n"))
    with pytest.raises(gbi.InstanceError, match="unknown top-level keys"):
        gbi.load_overlay(_overlay(tmp_path, "keep: []\n"))


def test_a_binary_that_cannot_run_is_an_error_not_a_pass(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(gbi.BIN_ENV, "/nonexistent/gold-band-registry")
    with pytest.raises(gbi.InstanceError, match="could not run"):
        gbi.validate(_registry())


def test_no_cargo_and_no_binary_is_an_error_not_a_pass(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv(gbi.BIN_ENV, raising=False)
    monkeypatch.setenv("PATH", "")
    with pytest.raises(gbi.InstanceError, match="cargo is not on PATH"):
        gbi.validate(_registry())


@needs_crate
def test_the_shipped_registry_declares_both_instances_cleanly() -> None:
    assert gbi.validate(_registry()) == []


@needs_crate
@pytest.mark.parametrize(
    "mutate,expected",
    [
        (
            lambda r: r["gold-band-personal"]["agents"].append("codex"),
            "agent codex is not allowed on the personal instance",
        ),
        (
            lambda r: r["gold-band-personal"]["agents"].append("claude-work"),
            "agent claude-work is not allowed on the personal instance",
        ),
        (lambda r: r["gold-band"]["agents"].append("vibe"), "agent vibe is not allowed on the work instance"),
        (lambda r: r["gold-band"]["repository_rule"].pop("remotes"), "repository_rule.remotes is missing"),
        (lambda r: r["gold-band-personal"].pop("repository_rule"), "repository_rule.remotes is missing"),
        (lambda r: r["gold-band"].update(extra="x"), "unknown field extra"),
        (lambda r: r["gold-band"]["agents"].append("Bad_Id"), "bad agent id"),
        (lambda r: r["gold-band"]["hosts"].append("sk-aaaaaaaaaaaaaaaaaaaaaaaa"), "literal secret"),
        (lambda r: r.pop("gold-band-personal"), "registry gold-band-personal: instance is not declared"),
    ],
)
def test_registry_declarations_that_cross_an_instance_boundary_are_refused(mutate: Any, expected: str) -> None:
    registry = copy.deepcopy(_registry())
    mutate(registry[gbi.REGISTRY_KEY])

    refused = gbi.validate(registry)

    assert any(expected in line for line in refused), refused
    assert not any("sk-aaaa" in line for line in refused)


@needs_crate
def test_the_overlay_may_add_servers_and_settings_to_the_work_instance(tmp_path: Path) -> None:
    overlay = _overlay(
        tmp_path,
        "instances:\n  gold-band:\n    mcp_servers:\n"
        "      jira: {command: npx, args: [-y, jira], env: {JIRA_TOKEN: $JIRA_TOKEN}}\n"
        "    settings: {editor: {theme: dark}}\n",
    )

    assert gbi.validate(_registry(), overlay) == []


@needs_crate
@pytest.mark.parametrize(
    "body,expected",
    [
        ("gold-band-personal: {agents: [codex]}", "overlay gold-band-personal: agent codex is not allowed"),
        ("gold-band-personal: {agents: [claude-work]}", "overlay gold-band-personal: agent claude-work is not allowed"),
        ("gold-band-personal: {mcp_servers: {x: {command: npx}}}", "mcp_servers to the work instance gold-band only"),
        ("gold-band-personal: {settings: {a: b}}", "settings to the work instance gold-band only"),
        ("gold-band: {agents: [vibe]}", "overlay gold-band: agent vibe is not allowed on the work instance"),
        ("gold-band: {hosts: [linux]}", "unknown field hosts.0"),
        ("gold-band: {mcp_servers: {x: {command: npx, token: abc}}}", "unknown field mcp_servers.x.token"),
        ("gold-band: {mcp_servers: {x: {command: npx, env: {API_TOKEN: not a ref}}}}", "secret-named"),
        ("gold-band: {settings: {a: sk-aaaaaaaaaaaaaaaaaaaaaaaa}}", "literal secret"),
        ("other: {agents: [vibe]}", "unknown instance"),
    ],
)
def test_overlay_declarations_that_cross_a_boundary_are_refused(tmp_path: Path, body: str, expected: str) -> None:
    refused = gbi.validate(_registry(), _overlay(tmp_path, f"instances:\n  {body}\n"))

    assert any(expected in line for line in refused), refused
    assert not any("sk-aaaa" in line or "not a ref" in line for line in refused)


@needs_crate
def test_render_registry_runs_the_instance_validation_in_check_and_write(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    bad = _overlay(tmp_path, "instances:\n  gold-band-personal: {agents: [codex]}\n")

    assert render_registry.main(["--check", "--overlay", str(bad)]) == 1
    assert "the Gold Band instances are refused" in capsys.readouterr().err
    assert render_registry.main(["--check", "--overlay", str(tmp_path / "none.yaml")]) == 0


def test_render_registry_reports_an_unrunnable_validator(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setenv(gbi.BIN_ENV, "/nonexistent/gold-band-registry")

    assert render_registry.main(["--check"]) == 1
    assert "registry invalid: could not run" in capsys.readouterr().err
