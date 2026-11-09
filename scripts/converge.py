#!/usr/bin/env python3
"""Converge agent surfaces to the registry and the keep file.

``mcp``: the MCP servers of each Claude profile's ``.claude.json``, Codex's
``config.toml`` and the ``config.toml`` of each Vibe home (the shared one and
vibe-private's), and the ``mcp.servers`` of OpenCode's ``opencode.json``.
``plugins``: Claude's marketplaces and user-scope plugins, which every
profile shares. The setup script renders
the declared and retired names from the registry. A retired entry is removed;
any other entry the surface's declared set does not name is removed unless the
keep file names it. A ``--private`` entry (Private Memory) is removed from every
surface that does not declare it, whatever the keep file says.

Every run first calls the Rust workspace's canary crate for its version, when
its binary is built, proving the workspace path end to end from setup itself;
without a build it says so and carries on (see docs/RUST.md).

``gold-band-validate`` flattens the registry's ``gold_band.agents`` into the
gold-band-registry crate's line protocol and asks the crate's binary to rule
on it. The crate's output is printed and its verdict is the exit code, so
setup refuses a bad declaration before anything is written; without a built
binary the reason is printed and nothing is refused, the canary's pattern.
A failure that is no verdict -- the registry unreadable or malformed, the
crate binary not answering, the canary blocking the run -- exits 3 with the
reason printed, so setup never reports it as the crate's refusal (exit 1).

``gold-band`` converges the ``agents`` map of ``~/.gold-band/settings.json``
onto the registry's ``gold_band.agents`` (#333): each declared agent is
written with only the fields Gold Band knows, without
``adapter.command``/``adapter.args`` for a built-in id -- the settings map
REPLACES the app's runtime agent list, it is not merged with the built-in
catalog (#328). An existing agent survives only when the keep file names
it, never when its id names codex, and a retired id (registry
``gold_band.retired``) goes even when kept; every other top-level key of the
document is preserved. Nothing is written while Gold Band runs: a writing
run is refused and the caller told to quit it and re-run, and the crate
re-checks a guard command immediately before its rename (#334), so an app
that starts mid-run replaces nothing either -- the step then reports each
group it did not write (``not written: settings (Gold Band started)``) and
a second run finishes. The report names the installed version and build
commit from the app's Info.plist, and one line per group says what was
written; after one, the data directory is closed to its owner (0700, the
settings file 0600). ``--check`` writes nothing, so a running app never
refuses it. The step reuses
gold-band-validate's flattener and crate,
passes the crate's exit code through, and exits 3 for its own failures,
like gold-band-validate. Where the app is not installed, setup skips the
step whole and the run carries on.

``gold-band-roles`` converges ``~/.gold-band/context/profiles`` onto the
six critique roles rendered as Gold Band profiles (#345): Gold Band reads
its profiles there, never from presets/profiles. The gold-band-roles
crate renders the six from the critique skill's own files -- the ones
setup installs as the critique skill -- and keeps the directory at them:
a hand-made profile (no front matter ``id``) and a Gold Band built-in are
never touched, a retired or stale generated one goes, and the keep file
is read for both its ``keep`` and its ``retired`` lists, so one file
names the kept and the retired roles. Nothing is created where the app
is not installed: the profiles directory, owner-only (0700), is created
only with Gold Band there, and a running app refuses the write the way
the agent map's converge does; ``--check`` writes nothing, so it is never
refused. The crate's exit code is passed through, and this step's own
failures carry ``GOLD_BAND_CANNOT_VALIDATE_EXIT``, like gold-band's.

``--check`` prints each removal and changes nothing. A normal run removes the
entries, reads every surface again and fails if a removed name is still there.

Usage::

    uv run --no-project --with pyyaml --with tomlkit python scripts/converge.py mcp \\
        --keep PATH --claude-home DIR [--claude DIR ...] [--codex PATH] [--vibe PATH] [--vibe-private PATH] \\
        [--opencode PATH] --declared SURFACE=NAME,... --retired NAME,... [--private NAME,...] [--check]
    uv run --no-project --with pyyaml --with tomlkit python scripts/converge.py plugins \\
        --keep PATH --declared-markets NAME,... --declared-plugins ID,... \\
        --retired-markets NAME,... --retired-plugins ID,... [--check]
    uv run --no-project --with pyyaml --with tomlkit python scripts/converge.py gold-band-validate \\
        [--registry PATH]
    uv run --no-project --with pyyaml --with tomlkit python scripts/converge.py gold-band \\
        --keep PATH [--settings PATH] [--registry PATH] [--check]
    uv run --no-project --with pyyaml --with tomlkit python scripts/converge.py gold-band-roles \\
        --critique DIR --keep PATH [--profiles PATH] [--check]
    uv run --no-project --with pyyaml --with tomlkit python scripts/converge.py gold-band-running

``gold-band-running`` is the guard the crate re-checks with immediately
before the rename (#334): exit 0 means Gold Band is not running, exit 1
means it is; it takes no flag and asks no crate.
"""

from __future__ import annotations

import argparse
import json
import os
import plistlib
import shlex
import shutil
import subprocess
import sys
import tempfile
import tomllib
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any
from xml.parsers.expat import ExpatError

import tomlkit
import yaml

DECLARED_SOURCES = ("plugin:",)

# The one workspace (`scripts/Cargo.toml`) and its one std-only member
# crate, whose built binary marks a machine where the Rust path works.
KIT_ROOT = Path(__file__).resolve().parent.parent
CRATE_NAME = "agent-kit-setup"
CRATE_TIMEOUT_S = 30.0

# The Gold Band registry crate, whose `validate` refuses a declaration that
# breaks an owner rule (agent-kit#332).
GOLD_BAND_CRATE_NAME = "gold-band-registry"
GOLD_BAND_TIMEOUT_S = 30.0

# The Gold Band roles crate, whose `converge` keeps the profiles directory
# at the six critique roles (#345).
GOLD_BAND_ROLES_CRATE_NAME = "gold-band-roles"

# gold-band-validate and gold-band exit 3 for their own failures -- the
# registry unreadable or malformed, the crate binary not answering, the
# canary blocking the run, Gold Band running under a converge -- a code
# apart from the crate's refusal (exit 1), so setup never reports a broken
# step as "the Gold Band registry is refused" (agent-kit#332).
GOLD_BAND_CANNOT_VALIDATE_EXIT = 3

# How the tests answer "is Gold Band running" (1: yes, anything else: no).
GOLD_BAND_RUNNING_ENV = "AGENT_KIT_GOLD_BAND_RUNNING"

# The Gold Band app the steps look at: the real /Applications one, with the
# path overridable so the harness points every check and report at a stub.
GOLD_BAND_APP_ENV = "AGENT_KIT_GOLD_BAND_APP"
GOLD_BAND_APP = "/Applications/Gold Band.app"

# The guard command the crate re-checks with immediately before the rename
# (#334); a test override stands in for an app that starts mid-run.
GOLD_BAND_GUARD_ENV = "AGENT_KIT_GOLD_BAND_GUARD"

# The groups a Gold Band converge writes, one report line each (#334):
# settings today; later tickets add roles, skill directories and launchers.
GOLD_BAND_GROUPS: tuple[str, ...] = ("settings",)

# What the crate's refusal line says when the guard vetoed the write, which
# is how the step knows to report "not written" rather than "broken".
GOLD_BAND_GUARD_REFUSED = "the guard reports Gold Band runs"


def crate_binary(kit_root: Path, crate: str = CRATE_NAME) -> Path | None:
    """A workspace crate's built binary, preferring the release build over a debug one.

    The binary is only ever there after `cargo build --manifest-path
    scripts/Cargo.toml`; a machine without a Rust toolchain never has it, and
    setup skips the crate call rather than pretending it happened.
    """
    for profile in ("release", "debug"):
        candidate = kit_root / "scripts" / "target" / profile / crate
        if candidate.is_file():
            return candidate
    return None


def crate_report(kit_root: Path, cargo: bool) -> tuple[str, int]:
    """One line from the workspace's canary crate, and whether the run may carry on.

    Setup runs converge on every run, so the canary crate proves the Rust
    workspace path end to end from setup itself: when the binary is built,
    converge asks it for its version. A binary that is not there skips the
    call with the reason said out loud, because nothing can be proven about a
    machine that never built it; a built binary that cannot answer blocks the
    run, closing the gate instead of assuming the crate is fine.
    """
    binary = crate_binary(kit_root)
    if binary is None:
        if cargo:
            return (f"{CRATE_NAME}: skipped: not built yet; "
                    "run cargo build --manifest-path scripts/Cargo.toml"), 0
        return f"{CRATE_NAME}: skipped: no Rust toolchain (cargo) on PATH", 0
    try:
        result = subprocess.run(
            [str(binary), "--version"], capture_output=True, text=True, check=False, timeout=CRATE_TIMEOUT_S
        )
    except subprocess.TimeoutExpired:
        return f"{CRATE_NAME}: the built binary at {binary} did not answer within {CRATE_TIMEOUT_S:g} seconds", 1
    version = result.stdout.strip().splitlines()[0] if result.stdout.strip() else ""
    if version and result.returncode == 0:
        return f"{CRATE_NAME}: {version} ({binary})", 0
    return f"{CRATE_NAME}: the built binary at {binary} answered exit {result.returncode}, not a version", 1


def gold_band_line_protocol(agents: list[dict[str, Any]]) -> str:
    """``agents`` in the gold-band-registry crate's line protocol.

    One ``agent<TAB>id`` line opens each declaration, then one
    ``field<TAB>path<TAB>value`` line per field in declaration order: dotted
    paths for the nested ones (``adapter.command``, ``adapter.args.<N>``,
    ``adapter.env.<NAME>``, ``compatibleAgentDirs.<N>``) and booleans as
    true/false. A key the crate does not know flattens the same way, so the
    crate refuses it as an unknown field: this file decides the paths, the
    crate the rules.
    """
    lines: list[str] = []
    for agent in agents:
        lines.append(f"agent\t{agent.get('id', '')}")
        for key, value in agent.items():
            if key != "id":
                lines.extend(_gold_band_field(key, value))
    return "".join(f"{line}\n" for line in lines)


def _gold_band_field(path: str, value: Any) -> list[str]:
    """One field's lines: a nested key flattens into a dotted path, a leaf into one line."""
    if isinstance(value, dict):
        return [line for name, item in value.items() for line in _gold_band_field(f"{path}.{name}", item)]
    if isinstance(value, list):
        return [line for index, item in enumerate(value) for line in _gold_band_field(f"{path}.{index}", item)]
    if isinstance(value, bool):
        return [f"field\t{path}\t{'true' if value else 'false'}"]
    return [f"field\t{path}\t{'' if value is None else value}"]


def _gold_band_agents(registry: Path, step: str) -> tuple[list[dict[str, Any]], list[str]] | int:
    """The registry's ``gold_band`` section, read as its declared agents and retired ids.

    Returns ``GOLD_BAND_CANNOT_VALIDATE_EXIT``, with the reason printed, when
    the registry is unreadable or the section is the wrong shape, so ``step``
    (validate or converge) stops there having written nothing; a missing
    section is an empty list on both sides.
    """
    try:
        data = yaml.safe_load(registry.read_text()) or {}
    except (OSError, UnicodeDecodeError, yaml.YAMLError) as exc:
        print(f"{GOLD_BAND_CRATE_NAME}: cannot {step}: cannot read {registry}: {exc}")
        return GOLD_BAND_CANNOT_VALIDATE_EXIT
    if not isinstance(data, dict):
        print(f"{GOLD_BAND_CRATE_NAME}: cannot {step}: {registry} is not a mapping")
        return GOLD_BAND_CANNOT_VALIDATE_EXIT
    section = data.get("gold_band")
    if section is None:
        return ([], [])
    if not isinstance(section, dict):
        print(f"{GOLD_BAND_CRATE_NAME}: cannot {step}: {registry}: gold_band must be a mapping")
        return GOLD_BAND_CANNOT_VALIDATE_EXIT
    declared = section.get("agents")
    if declared is not None and (
        not isinstance(declared, list) or not all(isinstance(a, dict) for a in declared)
    ):
        print(f"{GOLD_BAND_CRATE_NAME}: cannot {step}: {registry}:"
              " gold_band.agents must be a list of mappings")
        return GOLD_BAND_CANNOT_VALIDATE_EXIT
    retired = section.get("retired")
    if retired is None:
        retired = []
    if not isinstance(retired, list) or not all(isinstance(r, str) for r in retired):
        print(f"{GOLD_BAND_CRATE_NAME}: cannot {step}: {registry}:"
              " gold_band.retired must be a list of agent ids")
        return GOLD_BAND_CANNOT_VALIDATE_EXIT
    return (declared or [], retired)


def gold_band_validate(kit_root: Path, registry: Path, cargo: bool) -> int:
    """The registry's Gold Band agents through the crate: 0 clean or skipped, 1 a refusal, 3 its own failures.

    The declared ``gold_band.agents`` flatten into the crate's line protocol
    in a temporary file and the crate's built binary rules on it; the crate's
    own output is printed and its exit code returned, so setup stops before
    anything is written when the verdict is a refusal. A binary that is not
    built can prove nothing either way, so the reason is said out loud and the
    run carries on, like the canary. A failure that is no verdict -- the
    registry unreadable, ``gold_band`` not the expected shape, the crate
    binary not answering -- exits ``GOLD_BAND_CANNOT_VALIDATE_EXIT``, so
    setup tells a broken step from the crate's refusal.
    """
    # The registry's shape is checked first: it needs no crate, so a broken
    # registry fails the same way whether or not the crate is built.
    read = _gold_band_agents(registry, "validate")
    if isinstance(read, int):
        return read
    binary = crate_binary(kit_root, GOLD_BAND_CRATE_NAME)
    if binary is None:
        reason = ("not built yet; run cargo build --manifest-path scripts/Cargo.toml"
                  if cargo else "no Rust toolchain (cargo) on PATH")
        print(f"{GOLD_BAND_CRATE_NAME}: skipped: {reason}")
        return 0
    agents, _retired = read
    descriptor, staged = tempfile.mkstemp(prefix=f"{GOLD_BAND_CRATE_NAME}-", suffix=".txt")
    try:
        with os.fdopen(descriptor, "w") as handle:
            handle.write(gold_band_line_protocol(agents))
        try:
            result = subprocess.run(
                [str(binary), "validate", staged],
                capture_output=True, text=True, check=False, timeout=GOLD_BAND_TIMEOUT_S,
            )
        except subprocess.TimeoutExpired:
            print(f"{GOLD_BAND_CRATE_NAME}: cannot validate: the built binary at {binary}"
                  f" did not answer within {GOLD_BAND_TIMEOUT_S:g} seconds")
            return GOLD_BAND_CANNOT_VALIDATE_EXIT
    finally:
        os.unlink(staged)
    for output in (result.stdout, result.stderr):
        if output.strip():
            print(output.strip())
    return result.returncode


def _gold_band_running() -> bool:
    """Whether Gold Band runs right now, so setup never writes under it.

    ``pgrep -f '/Applications/Gold Band.app/'`` hears the real app; the tests,
    and any caller that cannot trust pgrep, answer through the env var
    instead (``1`` always means running; any other value means not running
    only in test mode, with ``AGENT_KIT_GOLD_BAND_APP`` set). The full running-app
    guard is #334's; this is the rule this ticket always enforces.
    """
    forced = os.environ.get(GOLD_BAND_RUNNING_ENV)
    if forced == "1":
        return True
    # "Not running" may be forced only in test mode, where a fake app path
    # stands in for /Applications/Gold Band.app; otherwise pgrep decides, so
    # the environment can never hide a running app.
    if forced is not None and os.environ.get(GOLD_BAND_APP_ENV):
        return False
    try:
        return subprocess.run(
            ["pgrep", "-f", f"{GOLD_BAND_APP}/"], capture_output=True, check=False
        ).returncode == 0
    except OSError:
        return False


def _gold_band_app() -> Path:
    """The Gold Band app the steps look at: the real one, or the test's stub."""
    return Path(os.environ.get(GOLD_BAND_APP_ENV, GOLD_BAND_APP))


def _gold_band_build() -> tuple[str, str]:
    """The installed Gold Band's version and build commit, from its Info.plist.

    ``CFBundleShortVersionString`` is the version; the commit comes from any
    key whose name names one when the plist stamps it, because no plist key
    is standard for it. Either answer is ``unknown`` when the app (or its
    plist) cannot be read, which is the report saying so, not a failure
    (#334).
    """
    try:
        with (_gold_band_app() / "Contents" / "Info.plist").open("rb") as handle:
            info = plistlib.load(handle)
    except (OSError, ValueError, ExpatError):
        return ("unknown", "unknown")
    if not isinstance(info, dict):
        return ("unknown", "unknown")
    version = info.get("CFBundleShortVersionString")
    commit: str | None = next((body for name, body in info.items()
                               if isinstance(body, str) and "commit" in name.lower()), None)
    return (str(version) if version is not None else "unknown",
            commit if commit is not None else "unknown")


def _gold_band_guard() -> str:
    """The command the crate runs immediately before the rename (#334),
    re-asking this script's own running-check answer, so the re-check sees
    the same pgrep and the same test overrides as the check before the crate
    ran. The command allows the write by exiting 0 and vetoes with anything
    else, so a Gold Band that starts mid-run replaces nothing; a test
    override stands in for exactly that mid-run start.
    """
    if override := os.environ.get(GOLD_BAND_GUARD_ENV):
        return override
    return " ".join((shlex.quote(sys.executable),
                    shlex.quote(str(KIT_ROOT / "scripts" / "converge.py")),
                    "gold-band-running"))


def gold_band_running() -> int:
    """The guard's answer, asked again right before the crate's rename (#334):
    0 Gold Band is not running, so the write may happen; 1 it is, so the
    write is vetoed and the step reports it."""
    return 1 if _gold_band_running() else 0


def gold_band(args: argparse.Namespace, cargo: bool) -> int:
    """Converge ``~/.gold-band/settings.json``'s agent map onto the registry (#333).

    The declared agents and the retired ids come from the registry's
    ``gold_band`` section, the keep ids from the same keep file the other
    surfaces use; each goes to a temporary file, and the crate's `converge`
    command makes the settings' ``agents`` map hold exactly the declared
    agents plus the kept ones (#328: the map REPLACES the app's runtime list,
    it is not merged with the built-in catalog). A run that would write never
    happens while Gold Band runs: the caller is told to quit it and re-run;
    the crate re-checks a guard command immediately before its rename (#334),
    so an app that starts mid-run replaces nothing either and the step
    reports it. The report also names the installed version and build commit
    from the app's Info.plist, and one line per group says what was written;
    after a write the data directory is closed to its owner. The crate's exit
    code is passed through, and this step's own failures carry
    ``GOLD_BAND_CANNOT_VALIDATE_EXIT``, like gold-band-validate's, so a
    broken step is never reported as the crate's refusal (exit 1).
    """
    if not args.check and _gold_band_running():
        print(f"{GOLD_BAND_CRATE_NAME}: Gold Band is running;"
              " quit it and re-run setup (nothing was written)")
        return GOLD_BAND_CANNOT_VALIDATE_EXIT
    version, commit = _gold_band_build()
    print(f"{GOLD_BAND_CRATE_NAME}: Gold Band {version} (commit {commit})")
    binary = crate_binary(KIT_ROOT, GOLD_BAND_CRATE_NAME)
    if binary is None:
        reason = ("not built yet; run cargo build --manifest-path scripts/Cargo.toml"
                  if cargo else "no Rust toolchain (cargo) on PATH")
        print(f"{GOLD_BAND_CRATE_NAME}: skipped: {reason}")
        return 0
    read = _gold_band_agents(args.registry, "converge")
    if isinstance(read, int):
        return read
    agents, retired = read
    keep = _keep(args.keep)
    staged: list[str] = []
    try:
        for kind, lines in (
            ("declared", gold_band_line_protocol(agents)),
            ("keep", "".join(f"{name}\n" for name in keep)),
            ("retired", "".join(f"{name}\n" for name in retired)),
        ):
            descriptor, path = tempfile.mkstemp(
                prefix=f"{GOLD_BAND_CRATE_NAME}-{kind}-", suffix=".txt")
            staged.append(path)
            with os.fdopen(descriptor, "w") as handle:
                handle.write(lines)
        command = [str(binary), "converge",
                   "--declared", staged[0], "--keep", staged[1], "--retired", staged[2],
                   "--settings", str(args.settings),
                   # The re-check the crate runs immediately before the
                   # rename (#334): a guard that vetoes replaces nothing.
                   "--guard", _gold_band_guard()]
        if args.check:
            command.append("--check")
        try:
            result = subprocess.run(command, capture_output=True, text=True, check=False,
                                     timeout=GOLD_BAND_TIMEOUT_S)
        except subprocess.TimeoutExpired:
            print(f"{GOLD_BAND_CRATE_NAME}: cannot converge: the built binary at {binary}"
                  f" did not answer within {GOLD_BAND_TIMEOUT_S:g} seconds")
            return GOLD_BAND_CANNOT_VALIDATE_EXIT
    finally:
        for path in staged:
            os.unlink(path)
    for output in (result.stdout, result.stderr):
        if output.strip():
            print(output.strip())
    return _gold_band_groups(result, args.check, args.settings)


def _gold_band_groups(result: subprocess.CompletedProcess[str], check: bool,
                      settings: Path) -> int:
    """One report line per group the run wrote (``checked`` under ``--check``),
    then, with a write, the owner-only hardening of the data directory (#334).

    The groups are ``GOLD_BAND_GROUPS`` -- settings today, roles, skill
    directories and launchers later -- so the report grows one line per group
    without a change of shape. A run the guard vetoed writes none of them
    and says so per group, the crate's refusal listed above it, so a second
    run after quitting the app finishes. Returns the step's exit code.
    """
    if result.returncode == 0:
        unchanged = "settings unchanged" in (result.stdout or "")
        for group in GOLD_BAND_GROUPS:
            state = "checked" if check else ("unchanged" if unchanged else "written")
            print(f"{GOLD_BAND_CRATE_NAME}: {state}: {group}")
        if check or unchanged:
            return 0
        return 0 if _gold_band_owner_only(settings) else GOLD_BAND_CANNOT_VALIDATE_EXIT
    refused = GOLD_BAND_GUARD_REFUSED in "".join(
        part for part in (result.stdout, result.stderr) if part)
    if result.returncode == 1 and refused:
        for group in GOLD_BAND_GROUPS:
            print(f"{GOLD_BAND_CRATE_NAME}: not written: {group} (Gold Band started)")
    return result.returncode


def _gold_band_owner_only(settings: Path) -> bool:
    """The data directory and the settings file, owner-only after a write
    (#334): Gold Band keeps tokens in there, so closing them to the owner is
    the converge's last act. Prints the reason and answers False when either
    chmod could not happen."""
    try:
        os.chmod(settings.parent, 0o700)
        os.chmod(settings, 0o600)
    except OSError as exc:
        print(f"{GOLD_BAND_CRATE_NAME}: cannot make {settings.parent} and {settings} owner-only: {exc}")
        return False
    print(f"{GOLD_BAND_CRATE_NAME}: owner-only: {settings.parent} is 0700 and {settings.name} is 0600")
    return True


def gold_band_roles(args: argparse.Namespace, cargo: bool) -> int:
    """Converge ``~/.gold-band/context/profiles`` onto the six critique roles (#345).

    Gold Band reads its profiles from ``context/profiles`` under its data
    directory (v0.19.1, src/app/profiles.rs), never from presets/profiles.
    The crate renders the six roles from the critique skill's own files and
    converges the directory onto them, never touching a hand-made profile
    (no front matter ``id``) or a Gold Band built-in; the keep file is given
    to the crate as both its keep and its retired file -- each list is read
    under its own key -- so one file names the kept and the retired roles.
    Nothing is created on a machine without an installed Gold Band (the
    setup step's own check, asked again here); with the app there, the
    profiles directory is created owner-only (0700) and a running app
    refuses the write, and only a writing run can be refused, because
    ``--check`` writes nothing. The crate's lines are printed and its exit
    code returned; a binary that is not there skips the call with the
    reason said out loud, like the canary, and this step's own failures
    carry ``GOLD_BAND_CANNOT_VALIDATE_EXIT``, like gold-band's.
    """
    if not _gold_band_app().exists():
        print(f"{GOLD_BAND_ROLES_CRATE_NAME}: Gold Band is not installed"
              f" ({_gold_band_app()}); its profiles are left alone")
        return 0
    if not args.check and _gold_band_running():
        print(f"{GOLD_BAND_ROLES_CRATE_NAME}: Gold Band is running;"
              " quit it and re-run setup (nothing was written)")
        return GOLD_BAND_CANNOT_VALIDATE_EXIT
    binary = crate_binary(KIT_ROOT, GOLD_BAND_ROLES_CRATE_NAME)
    if binary is None:
        reason = ("not built yet; run cargo build --manifest-path scripts/Cargo.toml"
                  if cargo else "no Rust toolchain (cargo) on PATH")
        print(f"{GOLD_BAND_ROLES_CRATE_NAME}: skipped: {reason}")
        return 0
    if not args.check:
        try:
            args.profiles.mkdir(parents=True, exist_ok=True)
            os.chmod(args.profiles, 0o700)
        except OSError as exc:
            print(f"{GOLD_BAND_ROLES_CRATE_NAME}: cannot make {args.profiles} owner-only: {exc}")
            return GOLD_BAND_CANNOT_VALIDATE_EXIT
    command = [str(binary), "converge",
               str(args.critique), str(args.profiles),
               "--keep", str(args.keep), "--retired", str(args.keep)]
    if args.check:
        command.append("--check")
    try:
        result = subprocess.run(command, capture_output=True, text=True, check=False,
                                 timeout=GOLD_BAND_TIMEOUT_S)
    except subprocess.TimeoutExpired:
        print(f"{GOLD_BAND_ROLES_CRATE_NAME}: cannot converge: the built binary at {binary}"
              f" did not answer within {GOLD_BAND_TIMEOUT_S:g} seconds")
        return GOLD_BAND_CANNOT_VALIDATE_EXIT
    for output in (result.stdout, result.stderr):
        if output.strip():
            print(output.strip())
    return result.returncode


@dataclass
class Surface:
    label: str
    kind: str
    path: Path | None = None
    env: dict[str, str] = field(default_factory=dict)

    def names(self) -> set[str]:
        if self.kind == "marketplace":
            return {str(m["name"]) for m in _claude_json("plugin", "marketplace", "list", "--json")}
        if self.kind == "plugin":
            return {str(p["id"]) for p in _claude_json("plugin", "list", "--json") if p.get("scope") == "user"}
        assert self.path is not None
        if not self.path.is_file():
            return set()
        if self.kind == "claude":
            return set(json.loads(self.path.read_text()).get("mcpServers") or {})
        if self.kind == "opencode":
            return set(_opencode_servers(json.loads(self.path.read_text())))
        data = tomllib.loads(self.path.read_text())
        if self.kind == "codex":
            return set(data.get("mcp_servers") or {})
        return {str(s.get("name")) for s in data.get("mcp_servers") or []}

    def remove(self, name: str) -> None:
        commands = {
            "claude": ["claude", "mcp", "remove", "--scope", "user", name],
            "codex": ["codex", "mcp", "remove", name],
            "plugin": ["claude", "plugin", "uninstall", name, "--scope", "user"],
            "marketplace": ["claude", "plugin", "marketplace", "remove", name],
        }
        if self.kind in commands:
            subprocess.run(commands[self.kind], env=self.env or None, capture_output=True, check=False)
            return
        assert self.path is not None
        if self.kind == "opencode":
            config = json.loads(self.path.read_text())
            _opencode_servers(config).pop(name, None)
            staged = self.path.with_name(self.path.name + ".agent-kit-tmp")
            staged.write_text(json.dumps(config, indent=2) + "\n")
            os.replace(staged, self.path)
            return
        doc = tomlkit.parse(self.path.read_text())
        kept = tomlkit.aot()
        for server in doc.get("mcp_servers") or []:
            if str(server.get("name")) != name:
                kept.append(server)
        del doc["mcp_servers"]
        if len(kept):
            doc["mcp_servers"] = kept
        self.path.write_text(tomlkit.dumps(doc))


def _opencode_servers(config: dict[str, Any]) -> dict[str, Any]:
    """OpenCode 2's ``mcp.servers`` table, the live object, or an empty one."""
    mcp = config.get("mcp")
    servers = mcp.get("servers") if isinstance(mcp, dict) else None
    return servers if isinstance(servers, dict) else {}


def _plain_json(path: Path) -> bool:
    """True when ``path`` is missing or parses as JSON."""
    if not path.is_file():
        return True
    try:
        json.loads(path.read_text())
    except json.JSONDecodeError:
        return False
    return True


def _claude_json(*args: str) -> list[dict[str, Any]]:
    out = subprocess.run(["claude", *args], capture_output=True, text=True, check=True).stdout
    entries: list[dict[str, Any]] = json.loads(out)
    return entries


def _tilde(path: Path) -> str:
    home = Path.home()
    return f"~/{path.relative_to(home)}" if path.is_relative_to(home) else str(path)


def _claude(profile: Path, claude_home: Path) -> Surface:
    env = dict(os.environ)
    # The primary profile without CLAUDE_CONFIG_DIR keeps .claude.json in HOME.
    if profile == claude_home and not env.get("CLAUDE_CONFIG_DIR"):
        env.pop("CLAUDE_CONFIG_DIR", None)
        path = Path.home() / ".claude.json"
    else:
        env["CLAUDE_CONFIG_DIR"] = str(profile)
        path = profile / ".claude.json"
    return Surface(f"claude {_tilde(profile)}", "claude", path, env)


def _keep(path: Path) -> set[str]:
    if not path.is_file():
        return set()
    data = yaml.safe_load(path.read_text()) or {}
    keep = data.get("keep") if isinstance(data, dict) else None
    if keep is None:
        return set()
    if not isinstance(keep, list) or not all(isinstance(n, str) for n in keep):
        raise SystemExit(f"converge: {path} must hold a top-level `keep:` list of names; nothing removed")
    return set(keep)


def _names(value: str) -> set[str]:
    return {n for n in value.split(",") if n}


def converge(surfaces: list[Surface], retired: Callable[[Surface, str], bool],
             spared: Callable[[Surface, str], bool], check: bool,
             private: Callable[[Surface, str], bool] = lambda _s, _n: False) -> int:
    """Remove each retired or misplaced private entry, and each other one not spared; then read every surface back."""
    removals: list[tuple[Surface, str, str]] = []
    for surface in surfaces:
        for name in sorted(surface.names()):
            if private(surface, name):
                removals.append((surface, name, "private"))
            elif retired(surface, name):
                removals.append((surface, name, "retired"))
            elif not spared(surface, name):
                removals.append((surface, name, "undeclared"))

    for surface, name, reason in removals:
        if check:
            print(f"would remove {name} ({surface.label}, {reason})")
        else:
            surface.remove(name)
            print(f"removed {name} ({surface.label}, {reason})")

    lingering = [(s, n) for s, n, _ in removals if not check and n in s.names()]
    for surface, name in lingering:
        print(f"{name} is still present ({surface.label})")
    return 1 if lingering else 0


def mcp(args: argparse.Namespace) -> int:
    keep = _keep(args.keep)
    retired = _names(args.retired)
    declared = {kind: _names(names) for kind, _, names in (d.partition("=") for d in args.declared)}
    private = _names(args.private)
    surfaces = [_claude(p, args.claude_home) for p in args.claude]
    if args.codex:
        surfaces.append(Surface("codex", "codex", args.codex))
    if args.vibe:
        surfaces.append(Surface("vibe", "vibe", args.vibe))
    if args.vibe_private:
        surfaces.append(Surface("vibe-private", "vibe-private", args.vibe_private))
    unreadable = 0
    if args.opencode:
        if _plain_json(args.opencode):
            surfaces.append(Surface("opencode", "opencode", args.opencode))
        else:
            # OpenCode accepts comments; rewriting would drop them, and one bad file
            # must not stop every other surface from converging.
            print(f"{_tilde(args.opencode)} is not plain JSON; opencode not converged")
            unreadable = 1
    return converge(
        surfaces,
        lambda _, name: name in retired,
        lambda s, name: name in declared.get(s.kind, set()) or name.startswith(DECLARED_SOURCES) or name in keep,
        args.check,
        lambda s, name: name in private and name not in declared.get(s.kind, set()),
    ) or unreadable


def plugins(args: argparse.Namespace) -> int:
    keep = _keep(args.keep)
    retired = _names(args.retired_plugins) | _names(args.retired_markets)
    declared = _names(args.declared_plugins) | _names(args.declared_markets)

    def spared(_: Surface, name: str) -> bool:
        plugin, _sep, market = name.partition("@")
        return name in declared or bool({name, plugin, market} & keep)

    # Plugins first: removing a marketplace uninstalls its plugins without saying which.
    surfaces = [Surface("claude plugin", "plugin"), Surface("claude marketplace", "marketplace")]
    return converge(surfaces, lambda _, name: name in retired, spared, args.check)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Converge agent surfaces to the registry and the keep file")
    sub = parser.add_subparsers(dest="surface", required=True)
    m = sub.add_parser("mcp")
    m.add_argument("--claude-home", type=Path, required=True)
    m.add_argument("--claude", type=Path, action="append", default=[])
    m.add_argument("--codex", type=Path)
    m.add_argument("--vibe", type=Path)
    m.add_argument("--vibe-private", type=Path)
    m.add_argument("--opencode", type=Path)
    m.add_argument("--declared", action="append", default=[], metavar="SURFACE=NAME,...")
    m.add_argument("--retired", default="")
    m.add_argument("--private", default="", metavar="NAME,...")
    p = sub.add_parser("plugins")
    for flag in ("--declared-markets", "--declared-plugins", "--retired-markets", "--retired-plugins"):
        p.add_argument(flag, default="")
    for each in (m, p):
        each.add_argument("--keep", type=Path, required=True)
        each.add_argument("--check", action="store_true")
    g = sub.add_parser("gold-band-validate")
    g.add_argument("--registry", type=Path, default=None)
    b = sub.add_parser("gold-band")
    b.add_argument("--keep", type=Path, required=True)
    b.add_argument("--settings", type=Path, default=None)
    b.add_argument("--registry", type=Path, default=None)
    b.add_argument("--check", action="store_true")
    r = sub.add_parser("gold-band-roles")
    r.add_argument("--critique", type=Path, required=True)
    r.add_argument("--profiles", type=Path, default=None)
    r.add_argument("--keep", type=Path, required=True)
    r.add_argument("--check", action="store_true")
    sub.add_parser("gold-band-running")
    args = parser.parse_args(argv)
    # The guard's answer needs no canary, no crate and no timeout: it is the
    # one command that must answer fast, because the crate runs it
    # immediately before its rename (#334).
    if args.surface == "gold-band-running":
        return gold_band_running()
    # The workspace's canary crate first: one line proving the Rust path works
    # on this machine, or saying why it could not be called (docs/RUST.md).
    line, crate_code = crate_report(KIT_ROOT, shutil.which("cargo") is not None)
    print(line)
    if crate_code:
        if args.surface == "gold-band-validate":
            # The canary blocking the run is no verdict on the registry, so it
            # takes the cannot-validate code -- never the refusal's exit 1.
            print(f"{GOLD_BAND_CRATE_NAME}: cannot validate: the canary crate blocked the run")
            return GOLD_BAND_CANNOT_VALIDATE_EXIT
        return crate_code
    if args.surface == "gold-band-validate":
        return gold_band_validate(
            KIT_ROOT,
            args.registry or KIT_ROOT / "registry" / "estate-tooling.yaml",
            shutil.which("cargo") is not None,
        )
    if args.surface == "gold-band":
        args.settings = args.settings or Path.home() / ".gold-band" / "settings.json"
        args.registry = args.registry or KIT_ROOT / "registry" / "estate-tooling.yaml"
        return gold_band(args, shutil.which("cargo") is not None)
    if args.surface == "gold-band-roles":
        args.profiles = args.profiles or Path.home() / ".gold-band" / "context" / "profiles"
        return gold_band_roles(args, shutil.which("cargo") is not None)
    return mcp(args) if args.surface == "mcp" else plugins(args)


if __name__ == "__main__":
    sys.exit(main())
