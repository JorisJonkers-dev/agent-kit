"""Plugins install at the commit the registry pins, not whatever upstream has now.

Claude Code tracks a GitHub marketplace's default branch, so an installed
plugin was whatever upstream had at install time and never matched a pin. Setup
now checks each marketplace out at its pinned commit in a local folder and
declares the marketplace from that folder; a plugin's expected commit follows
from that checkout (the marketplace commit, or the sha its entry pins).
"""

from __future__ import annotations

import copy
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
import yaml  # noqa: E402

SHA = "^[0-9a-f]{40}$"


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _script() -> str:
    return render_registry.render_setup_script(_registry())


def _fn(script: str, name: str) -> str:
    start = script.index(f"\n{name}() {{") + 1
    end = script.index("\n}\n", script.index("# end " + name, start))
    return script[start : end + 3]


# --- the registry ---


def test_every_marketplace_is_pinned_and_plugins_follow_it() -> None:
    data = _registry()
    import re

    for market in data["marketplaces"]:
        assert re.match(SHA, str(market.get("commit", ""))), market["name"]
    for plugin in data["plugins"]:
        assert "commit" not in plugin, f"{plugin['name']} is pinned through its marketplace"


def test_a_marketplace_and_a_skill_source_of_one_upstream_agree() -> None:
    data = _registry()
    caveman = next(m for m in data["marketplaces"] if m["name"] == "caveman")
    skill = next(s for s in data["skill_sources"] if s["name"] == "caveman")
    assert caveman["commit"] == skill["commit"]
    broken = copy.deepcopy(data)
    next(m for m in broken["marketplaces"] if m["name"] == "caveman")["commit"] = "0" * 40
    with pytest.raises(render_registry.RegistryError, match="skill source"):
        render_registry.validate(broken)


def test_validation_refuses_an_unpinned_marketplace_or_a_plugin_pin() -> None:
    data = copy.deepcopy(_registry())
    del data["marketplaces"][0]["commit"]
    with pytest.raises(render_registry.RegistryError, match="commit"):
        render_registry.validate(data)
    data = copy.deepcopy(_registry())
    data["plugins"][0]["commit"] = "a" * 40
    with pytest.raises(render_registry.RegistryError, match="marketplace"):
        render_registry.validate(data)


# --- ensure_marketplace, against a local upstream and a stub claude ---

STUB_CLAUDE = r'''#!/usr/bin/env python3
import json, os, sys
state = os.environ["STUB_STATE"]
log = os.environ["STUB_LOG"]
args = sys.argv[1:]
open(log, "a").write(" ".join(args) + "\n")
markets = json.load(open(state)) if os.path.exists(state) else []
def save(): json.dump(markets, open(state, "w"))
if args[:3] == ["plugin", "marketplace", "list"]:
    print(json.dumps(markets)); sys.exit(0)
if args[:3] == ["plugin", "marketplace", "remove"]:
    markets[:] = [m for m in markets if m["name"] != args[3]]; save(); sys.exit(0)
if args[:3] == ["plugin", "marketplace", "add"]:
    path = args[3]
    name = json.load(open(os.path.join(path, ".claude-plugin", "marketplace.json")))["name"]
    markets.append({"name": name, "source": "directory", "path": path}); save(); sys.exit(0)
sys.exit(0)
'''


def _upstream(tmp_path: Path) -> tuple[Path, list[str]]:
    repo = tmp_path / "upstream" / "owner" / "tools"
    repo.mkdir(parents=True)
    git = ["git", "-C", str(repo), "-c", "user.email=t@t", "-c", "user.name=t"]
    subprocess.run(["git", "init", "-q", str(repo)], check=True)
    commits = []
    for version in ("1", "2"):
        manifest = repo / ".claude-plugin" / "marketplace.json"
        manifest.parent.mkdir(exist_ok=True)
        manifest.write_text(json.dumps({"name": "tools", "plugins": [
            {"name": "inrepo", "source": "./plugins/inrepo"},
            {"name": "external", "source": {"source": "url", "url": "https://x/y.git", "sha": "e" * 39 + version}},
        ]}))
        subprocess.run([*git, "add", "-A"], check=True)
        subprocess.run([*git, "commit", "-q", "-m", version], check=True)
        commits.append(subprocess.run([*git, "rev-parse", "HEAD"], capture_output=True, text=True,
                                      check=True).stdout.strip())
    # Allow fetching any commit by sha, as GitHub does.
    subprocess.run([*git, "config", "uploadpack.allowAnySHA1InWant", "true"], check=True)
    return tmp_path / "upstream", commits


def _ensure(
    tmp_path: Path, commit: str, *, github_first: bool = False,
) -> tuple[subprocess.CompletedProcess, list[str]]:
    base, _ = _upstream(tmp_path) if not (tmp_path / "upstream").exists() else (tmp_path / "upstream", [])
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir(exist_ok=True)
    (bin_dir / "claude").write_text(STUB_CLAUDE.replace("/usr/bin/env python3", sys.executable))
    (bin_dir / "claude").chmod(0o755)
    state, log = tmp_path / "markets.json", tmp_path / "claude.log"
    if github_first and not state.exists():
        state.write_text(json.dumps([{"name": "tools", "source": "github", "repo": "owner/tools"}]))
    log.write_text("")
    body = (
        "CHECK_ONLY=0\nlog() { :; }\nok() { echo \"ok $*\"; }\nwarn() { echo \"warn $*\" >&2; }\n"
        "fail() { echo \"fail $*\" >&2; }\nrun() { \"$@\"; }\n"
        + "unpinned_markets=()\n"
        + _fn(_script(), "_marketplace_from_folder")
        + _fn(_script(), "ensure_marketplace") + f"ensure_marketplace tools owner/tools {commit}\n"
        + 'echo "UNPINNED=${unpinned_markets[*]-}"\n'
    )
    env = {**os.environ, "PATH": f"{bin_dir}:{os.environ['PATH']}", "STUB_STATE": str(state),
           "STUB_LOG": str(log), "AGENT_KIT_GIT_BASE": f"file://{base}",
           "AGENT_KIT_MARKETPLACES": str(tmp_path / "markets"), "GIT_CONFIG_GLOBAL": "/dev/null"}
    result = subprocess.run(["/bin/bash", "-c", body], capture_output=True, text=True, check=False, env=env)
    return result, [line for line in log.read_text().splitlines() if line]


def _head(tmp_path: Path) -> str:
    return subprocess.run(["git", "-C", str(tmp_path / "markets" / "tools"), "rev-parse", "HEAD"],
                          capture_output=True, text=True, check=True).stdout.strip()


def test_a_github_marketplace_moves_to_the_pinned_checkout(tmp_path: Path) -> None:
    _, commits = _upstream(tmp_path)
    result, calls = _ensure(tmp_path, commits[0], github_first=True)
    assert result.returncode == 0 and "warn" not in result.stderr, result.stderr
    assert _head(tmp_path) == commits[0]
    assert "plugin marketplace remove tools" in calls
    assert f"plugin marketplace add {tmp_path / 'markets' / 'tools'}" in calls


def test_a_second_run_at_the_same_pin_touches_nothing(tmp_path: Path) -> None:
    _, commits = _upstream(tmp_path)
    _ensure(tmp_path, commits[0], github_first=True)
    _, calls = _ensure(tmp_path, commits[0])
    assert calls == ["plugin marketplace list --json"]


def test_a_new_pin_moves_the_checkout_and_refreshes_the_marketplace(tmp_path: Path) -> None:
    _, commits = _upstream(tmp_path)
    _ensure(tmp_path, commits[0], github_first=True)
    _, calls = _ensure(tmp_path, commits[1])
    assert _head(tmp_path) == commits[1]
    assert "plugin marketplace update tools" in calls
    assert not any(c.startswith("plugin marketplace remove") for c in calls)


def test_an_unreachable_pin_changes_nothing(tmp_path: Path) -> None:
    _, commits = _upstream(tmp_path)
    _ensure(tmp_path, commits[0], github_first=True)
    result, calls = _ensure(tmp_path, "f" * 40)
    assert "could not fetch" in result.stderr
    assert _head(tmp_path) == commits[0]
    assert calls == ["plugin marketplace list --json"], "it looks, and changes nothing"
    assert "still at its previous pin" in result.stderr


# --- the commit a plugin is expected at ---


def _expected(tmp_path: Path, plugin: str, commit: str) -> str:
    folder = tmp_path / "markets" / "tools"
    body = _fn(_script(), "expected_plugin_commit") + f'expected_plugin_commit "{folder}" {plugin} {commit}\n'
    return subprocess.run(["/bin/bash", "-c", body], capture_output=True, text=True, check=True).stdout.strip()


def test_a_plugin_in_the_marketplace_repo_is_at_the_marketplace_commit(tmp_path: Path) -> None:
    _, commits = _upstream(tmp_path)
    _ensure(tmp_path, commits[0], github_first=True)
    assert _expected(tmp_path, "inrepo", commits[0]) == commits[0]


def test_an_external_plugin_is_at_the_sha_its_entry_pins(tmp_path: Path) -> None:
    _, commits = _upstream(tmp_path)
    _ensure(tmp_path, commits[0], github_first=True)
    assert _expected(tmp_path, "external", commits[0]) == "e" * 39 + "1"


# --- drift: a plugin behind or ahead of its pin is moved, not just reported ---


def test_a_plugin_off_its_pin_is_updated_to_it() -> None:
    script = _script()
    drift = script[script.index('log "checking plugin commit drift"') :]
    assert "run claude plugin update caveman@caveman" in drift
    assert 'expected_plugin_commit "${AGENT_KIT_MARKETPLACES' in drift


# --- fail closed: an unpinned marketplace installs nothing ---


def test_an_unreachable_pin_on_a_github_marketplace_marks_it_unpinned(tmp_path: Path) -> None:
    _upstream(tmp_path)
    result, calls = _ensure(tmp_path, "f" * 40, github_first=True)
    assert "could not fetch" in result.stderr
    assert "UNPINNED=tools" in result.stdout
    assert not any(c.startswith("plugin marketplace") and "list" not in c for c in calls)


def test_plugins_of_an_unpinned_marketplace_are_not_installed() -> None:
    script = _script()
    block = script[script.index("# caveman: ") : script.index("# drawio: ")]
    assert 'case " ${unpinned_markets[*]-} " in *" caveman "*)' in block
    assert "not installed: its marketplace is not at its pinned commit" in block


def test_a_plugin_that_cannot_be_pinned_is_a_warning_not_a_log() -> None:
    drift = _script()[_script().index('log "checking plugin commit drift"') :]
    assert 'warn "caveman: its marketplace entry pins no commit' in drift


def test_language_servers_of_an_unpinned_marketplace_are_not_installed() -> None:
    script = _script()
    lsp = script[script.index('log "language servers"') : script.index('log "language servers skipped')]
    server = next(s for s in _registry()["language_servers"])["plugin"]
    block = lsp[lsp.index(f"# {server} -> ") :]
    assert 'case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)' in block
    assert f"{server}: not installed: its marketplace is not at its pinned commit" in block
