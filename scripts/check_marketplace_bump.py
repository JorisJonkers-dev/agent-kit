#!/usr/bin/env python3
"""Fail a marketplace pin bump that adds anything able to run code.

For every marketplace whose pinned commit changed, each plugin the registry
installs from it is read at the old and the new commit. A hook, an MCP or LSP
server, or a script that only the new commit has fails the check, and so does
a plugin that can no longer be read. Edits to what was already there pass, so
a bump that adds nothing automerges and one that adds something waits for a
person. The upstream code is only read, never run.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import yaml

MANIFEST = Path(".claude-plugin") / "marketplace.json"
PLUGIN_MANIFEST = Path(".claude-plugin") / "plugin.json"
SCRIPT_SUFFIXES = {".sh", ".bash", ".zsh", ".fish", ".py", ".js", ".mjs", ".cjs", ".ts", ".mts", ".cts",
                   ".rb", ".pl", ".php", ".ps1", ".bat", ".cmd", ".exe", ".jar", ".wasm"}
# Manifest fields known to carry nothing that runs, or to be read below. Any other is a finding.
KNOWN_FIELDS = {"$schema", "name", "version", "description", "author", "homepage", "repository", "license",
                "keywords", "category", "tags", "strict", "source", "hooks", "mcpServers", "lspServers",
                "commands", "agents", "skills", "outputStyles", "displayName", "documentationUrl", "supportUrl",
                "privacyPolicyUrl", "termsOfServiceUrl"}
SHELL_LINE = re.compile(r"!`([^`\n]+)`")
SHA = re.compile(r"[0-9a-f]{40}")
SLUG = re.compile(r"[A-Za-z0-9][\w.-]*/[\w.-]+")
HTTPS = re.compile(r"https://[^\s'\"\\]+")
OFFICIAL = "claude-plugins-official"
COSMETIC_HOOK_FIELDS = {"timeout", "statusMessage"}
SCRIPTS_SHOWN = 10
GITHUB = "https://github.com/"


class CheckError(Exception):
    pass


@dataclass
class Surface:
    """What a plugin can run, keyed by identity, with a label for the report."""

    items: dict[str, str] = field(default_factory=dict)

    def add(self, key: str, label: str) -> None:
        self.items[key] = label


class Fetcher:
    def __init__(self, git_base: str, root: Path) -> None:
        self.git_base = git_base.rstrip("/")
        self.root = root
        self.cache: dict[tuple[str, str], Path] = {}

    def url(self, repo_or_url: str) -> str:
        """Only an https URL or an owner/repo slug is fetched: upstream writes these."""
        slug = repo_or_url.removeprefix(GITHUB)
        if SLUG.fullmatch(slug):
            return f"{self.git_base}/{slug.removesuffix('.git')}"
        if HTTPS.fullmatch(repo_or_url):
            return repo_or_url
        raise CheckError(f"refuses to fetch {repo_or_url!r}: only https URLs and owner/repo are read")

    def checkout(self, repo_or_url: str, commit: str) -> Path:
        url = self.url(repo_or_url)
        if not SHA.fullmatch(commit):
            raise CheckError(f"refuses to fetch {url} at {commit!r}: not a full commit sha")
        if (url, commit) in self.cache:
            return self.cache[(url, commit)]
        dest = self.root / str(len(self.cache))
        git = ["git", "-c", "core.symlinks=false", "-c", "advice.detachedHead=false", "-C", str(dest)]
        env = {**os.environ, "GIT_TERMINAL_PROMPT": "0"}
        dest.mkdir(parents=True)
        steps = [["git", "init", "-q", str(dest)], [*git, "fetch", "-q", "--depth", "1", "--", url, commit],
                 [*git, "checkout", "-q", "FETCH_HEAD"]]
        for step in steps:
            if subprocess.run(step, capture_output=True, env=env, check=False).returncode != 0:
                raise CheckError(f"could not fetch {url} at {commit[:12]}")
        self.cache[(url, commit)] = dest
        return dest


def _no_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    keys = [key for key, _ in pairs]
    duplicate = next((key for key in keys if keys.count(key) > 1), None)
    if duplicate is not None:
        raise ValueError(f"duplicate key {duplicate!r}")
    return dict(pairs)


def _json(path: Path, label: str) -> Any:
    """Parse strictly: a file Claude Code might read differently is a finding, not an empty config."""
    try:
        return json.loads(path.read_text(), object_pairs_hook=_no_duplicates)
    except (OSError, UnicodeDecodeError, ValueError) as error:
        raise CheckError(f"{label} cannot be read: {error}") from error


def _inside(root: Path, rel: str, label: str) -> Path:
    path = (root / rel).resolve()
    if not path.is_relative_to(root.resolve()):
        raise CheckError(f"{label}: {rel} leaves the plugin")
    return path


def _configs(root: Path, value: Any, default: str | None, label: str) -> list[dict[str, Any]]:
    """A manifest field that is inline, a path, or a list of paths; plus its default file."""
    found: list[Any] = []
    for item in (value if isinstance(value, list) else [] if value is None else [value]):
        if isinstance(item, str):
            found.append(_json(_inside(root, item, label), f"{label} {item}"))
        else:
            found.append(item)
    if default and (root / default).is_file():
        found.append(_json(_inside(root, default, label), f"{label} {default}"))
    if not all(isinstance(item, dict) for item in found):
        raise CheckError(f"{label} cannot be read: not an object")
    return found


def _hooks(surface: Surface, config: dict[str, Any], label: str) -> None:
    events = config.get("hooks", config)
    bad = CheckError(f"{label} hooks cannot be read: unexpected shape")
    if not isinstance(events, dict):
        raise bad
    for event, groups in events.items():
        if not isinstance(groups, list) or not all(isinstance(g, dict) for g in groups):
            raise bad
        for group in groups:
            matcher = str(group.get("matcher") or "")
            hooks = group.get("hooks") or []
            if not isinstance(hooks, list) or not all(isinstance(h, dict) for h in hooks):
                raise bad
            for hook in hooks:
                action = {k: v for k, v in hook.items() if k not in COSMETIC_HOOK_FIELDS}
                key = json.dumps({"event": event, "matcher": matcher, **action}, sort_keys=True)
                what = hook.get("command") or hook.get("url") or hook.get("prompt") or ""
                surface.add(f"hook:{key}", f"hook {event}{' ' + matcher if matcher else ''}: {str(what)[:160]}")


def _servers(surface: Surface, kind: str, config: dict[str, Any], wrapper: str, label: str) -> None:
    servers = config.get(wrapper, config)
    if not isinstance(servers, dict) or not all(isinstance(spec, dict) for spec in servers.values()):
        raise CheckError(f"{label} {kind} servers cannot be read: unexpected shape")
    for name, spec in servers.items():
        command = [spec.get("command") or "", *(spec.get("args") or [])]
        target = spec.get("url") or " ".join(str(part) for part in command)
        surface.add(f"{kind}:{name}:{json.dumps(spec, sort_keys=True)}", f"{kind} server {name}: {str(target)[:160]}")


def _scripts_in(folder: Path, root: Path) -> list[tuple[str, str]]:
    """Script files, and `!` shell lines in markdown, which run when a command or skill is used."""
    found = []
    for path in sorted(folder.rglob("*")):
        rel = path.relative_to(root)
        if ".git" in rel.parts or not path.is_file():
            continue
        if path.suffix.lower() in SCRIPT_SUFFIXES or os.access(path, os.X_OK) or "bin" in rel.parts[:-1]:
            found.append((f"script:{rel.as_posix()}", f"script {rel.as_posix()}"))
        elif path.suffix.lower() == ".md":
            for line in dict.fromkeys(SHELL_LINE.findall(path.read_text(errors="replace"))):
                found.append((f"shell:{rel.as_posix()}:{line}", f"shell line {rel.as_posix()}: {line[:160]}"))
    return found


def _fields(surface: Surface, manifest: dict[str, Any], label: str) -> None:
    for key, value in manifest.items():
        if key not in KNOWN_FIELDS:
            surface.add(f"field:{label}:{key}:{json.dumps(value, sort_keys=True)}", f"manifest field {key}")


def surface_of(root: Path, entry: dict[str, Any]) -> Surface:
    surface = Surface()
    name = str(entry.get("name"))
    manifest: Any = {}
    if (root / PLUGIN_MANIFEST).is_file():
        manifest = _json(root / PLUGIN_MANIFEST, f"{name}: {PLUGIN_MANIFEST}")
    if not isinstance(manifest, dict):
        raise CheckError(f"{name}: {PLUGIN_MANIFEST} cannot be read: not an object")
    _fields(surface, manifest, "plugin")
    _fields(surface, entry, "entry")
    for where, fields in ((f"{name}: plugin.json", manifest), (f"{name}: marketplace entry", entry)):
        defaults = fields is manifest
        for config in _configs(root, fields.get("hooks"), "hooks/hooks.json" if defaults else None, where):
            _hooks(surface, config, where)
        for config in _configs(root, fields.get("mcpServers"), ".mcp.json" if defaults else None, where):
            _servers(surface, "MCP", config, "mcpServers", where)
        for config in _configs(root, fields.get("lspServers"), ".lsp.json" if defaults else None, where):
            _servers(surface, "LSP", config, "lspServers", where)
    for key, label in _scripts_in(root, root):
        surface.add(key, label)
    return surface


def plugin_root(fetcher: Fetcher, checkout: Path, manifest: dict[str, Any], name: str) -> tuple[Path, dict[str, Any]]:
    entry = next((p for p in manifest.get("plugins") or [] if isinstance(p, dict) and p.get("name") == name), None)
    if entry is None:
        raise CheckError(f"{name}: not listed in the marketplace")
    source = entry.get("source")
    if isinstance(source, str):
        base = str((manifest.get("metadata") or {}).get("pluginRoot") or ".")
        rel = source if source.startswith("./") or base == "." else f"{base}/{source}"
        root = (checkout / rel).resolve()
        if not root.is_relative_to(checkout.resolve()):
            raise CheckError(f"{name}: its source {source} leaves the marketplace")
        if not root.is_dir():
            raise CheckError(f"{name}: its source {source} does not exist")
        return root, entry
    if not isinstance(source, dict):
        raise CheckError(f"{name}: its source cannot be read")
    sha = str(source.get("sha") or "")
    where = str(source.get("url") or source.get("repo") or "")
    if not sha:
        raise CheckError(f"{name}: pins no commit, so it cannot be reviewed")
    if not where:
        raise CheckError(f"{name}: its source names no repository")
    external = fetcher.checkout(where, sha).resolve()
    sub = str(source.get("path") or "")
    root = (external / sub).resolve()
    if not root.is_relative_to(external):
        raise CheckError(f"{name}: its path {sub} leaves its repository")
    return root, entry


def _installed(registry: dict[str, Any], market: str) -> list[str]:
    names = [str(p["name"]) for p in registry.get("plugins") or [] if p.get("marketplace") == market]
    if market == OFFICIAL:
        names += [str(s["plugin"]) for s in registry.get("language_servers") or []]
    return names


def _surface_at(fetcher: Fetcher, repo: str, commit: str, plugin: str) -> Surface:
    checkout = fetcher.checkout(repo, commit)
    manifest = _json(checkout / MANIFEST, f"{repo} at {commit[:12]}: {MANIFEST}")
    if not isinstance(manifest, dict) or not isinstance(manifest.get("plugins"), list):
        raise CheckError(f"{repo} at {commit[:12]}: {MANIFEST} cannot be read: no plugin list")
    root, entry = plugin_root(fetcher, checkout, manifest, plugin)
    return surface_of(root, entry)


def _skill_dirs(root: Path, selector: str) -> list[Path]:
    """The directories a skill source selector publishes from."""
    if selector in ("", "*"):
        return [root]
    if "/" in selector:
        return [root / selector.removesuffix("/*")]
    return [path for path in root.rglob(selector) if path.is_dir()]


def _skill_scripts(fetcher: Fetcher, source: dict[str, Any], commit: str) -> dict[str, str]:
    checkout = fetcher.checkout(str(source["repo"]), commit)
    root = checkout / "skills" if (checkout / "skills").is_dir() else checkout
    surface = Surface()
    for folder in _skill_dirs(root, str(source.get("selector") or "*")):
        if folder.resolve().is_relative_to(root.resolve()) and folder.is_dir():
            for key, label in _scripts_in(folder, root):
                surface.add(key, label)
    return surface.items


def _additions(name: str, others: list[str], scripts: list[str]) -> list[str]:
    """Report lines for what a bump adds; past a few scripts, only their count."""
    lines = [f"  {name}: adds {label}" for label in [*others, *scripts[:SCRIPTS_SHOWN]]]
    if len(scripts) > SCRIPTS_SHOWN:
        lines.append(f"  {name}: adds {len(scripts) - SCRIPTS_SHOWN} more scripts")
    return lines


def _bumped(base: dict[str, Any], head: dict[str, Any], section: str) -> list[tuple[dict[str, Any], str]]:
    old = {e["name"]: str(e.get("commit")) for e in base.get(section) or []}
    return [(e, old[e["name"]]) for e in head.get(section) or []
            if e["name"] in old and old[e["name"]] != str(e.get("commit"))]


def check(base: dict[str, Any], head: dict[str, Any], fetcher: Fetcher) -> tuple[list[str], int]:
    """Return the report lines and how many findings block the bump."""
    lines: list[str] = []
    findings = 0
    bumped = _bumped(base, head, "marketplaces")
    skills = _bumped(base, head, "skill_sources")
    if not bumped and not skills:
        return ["no marketplace pin changed, and no skill source pin"], 0
    for source, old in skills:
        new = str(source["commit"])
        lines.append(f"skill source {source['name']} ({source['repo']}): {old[:12]} -> {new[:12]}")
        try:
            new_items = _skill_scripts(fetcher, source, new)
        except CheckError as error:
            lines.append(f"  {error}")
            findings += 1
            continue
        try:
            old_items = _skill_scripts(fetcher, source, old)
        except CheckError:
            old_items = {}
        new_scripts = [label for key, label in new_items.items() if key not in old_items]
        lines += _additions(str(source["name"]), [], new_scripts) or ["  adds nothing that runs code"]
        findings += len(new_scripts)
    for market, old_pin in bumped:
        name, repo, old, new = market["name"], market["repo"], old_pin, str(market["commit"])
        lines.append(f"marketplace {name} ({repo}): {old[:12]} -> {new[:12]}")
        count = 0
        for plugin in _installed(head, name):
            try:
                new_surface = _surface_at(fetcher, repo, new, plugin)
            except CheckError as error:
                lines.append(f"  {error}")
                count += 1
                continue
            try:
                old_items = _surface_at(fetcher, repo, old, plugin).items
            except CheckError:
                old_items = {}
            added = {key: label for key, label in new_surface.items.items() if key not in old_items}
            scripts = [label for key, label in added.items() if key.startswith("script:")]
            others = dict.fromkeys(label for key, label in added.items() if not key.startswith("script:"))
            lines += _additions(plugin, list(others), scripts)
            count += len(added)
        if not count:
            lines.append("  adds nothing that runs code")
        findings += count
    return lines, findings


def _registry(text: str) -> dict[str, Any]:
    data = yaml.safe_load(text)
    if not isinstance(data, dict):
        raise CheckError("the registry is not a mapping")
    return data


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Fail a marketplace pin bump that adds anything able to run code.")
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--base-ref", help="git ref holding the registry before the bump")
    source.add_argument("--base-registry", type=Path, help="registry file before the bump")
    parser.add_argument("--registry", type=Path, default=Path("registry/estate-tooling.yaml"))
    parser.add_argument("--git-base", default=os.environ.get("AGENT_KIT_GIT_BASE", "https://github.com"))
    args = parser.parse_args(argv)
    if args.base_ref:
        shown = subprocess.run(["git", "show", f"{args.base_ref}:{args.registry.as_posix()}"],
                               capture_output=True, text=True, check=True)
        base_text = shown.stdout
    else:
        base_text = args.base_registry.read_text()
    with tempfile.TemporaryDirectory() as tmp:
        lines, findings = check(_registry(base_text), _registry(args.registry.read_text()),
                                Fetcher(args.git_base, Path(tmp)))
    if findings:
        lines.append(f"{findings} finding(s): review the upstream change before merging this bump")
    report = "\n".join(lines)
    print(report)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as handle:
            # A fence longer than any backtick run in upstream text, which cannot then close it.
            fence = "`" * max(3, 1 + max((len(run) for run in re.findall(r"`+", report)), default=0))
            handle.write(f"## Marketplace pin review\n\n{fence}\n{report}\n{fence}\n")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
