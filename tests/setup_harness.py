"""A whole run of the checked-in setup script, with every external command stubbed.

HOME is a temp dir and nothing real is on PATH, so a run never touches this
machine's agent homes. Tests assert on the tree the run leaves behind.
"""

from __future__ import annotations

import fcntl
import json
import os
import pwd
import subprocess
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
SETUP = KIT_ROOT / "installer" / "setup-workstation.sh"


_MERGE_BINARY: list[str] = []


def merge_binary() -> str:
    """The agent-kit-merge binary the setup script's ``agent_kit_merge`` function runs.

    Built once per machine from the kit's cargo workspace (a debug build, under a lock
    so parallel test workers wait for one build), with the real home's cargo and rustup
    directories since a test's HOME is a temporary one. ``AGENT_KIT_MERGE`` names one
    explicitly, such as a prebuilt release binary.
    """
    if _MERGE_BINARY:
        return _MERGE_BINARY[0]
    named = os.environ.get("AGENT_KIT_MERGE")
    if named:
        _MERGE_BINARY.append(named)
        return named
    real_home = pwd.getpwuid(os.getuid()).pw_dir
    target = Path(os.environ.get("CARGO_TARGET_DIR") or KIT_ROOT / "scripts" / "target")
    target.mkdir(parents=True, exist_ok=True)
    env = {
        **os.environ,
        "CARGO_HOME": os.environ.get("CARGO_HOME") or f"{real_home}/.cargo",
        "RUSTUP_HOME": os.environ.get("RUSTUP_HOME") or f"{real_home}/.rustup",
    }
    with open(target / ".merge-binary.lock", "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        subprocess.run(
            ["cargo", "build", "--quiet", "--locked", "-p", "agent-kit-merge",
             "--manifest-path", str(KIT_ROOT / "scripts" / "Cargo.toml")],
            check=True,
            env=env,
            capture_output=True,
        )
    _MERGE_BINARY.append(str(target / "debug" / "agent-kit-merge"))
    return _MERGE_BINARY[0]


@dataclass(frozen=True)
class Run:
    home: Path
    stdout: str
    stderr: str
    calls: list[str]
    returncode: int = 0


def stub(bin_dir: Path, name: str, body: str) -> None:
    path = bin_dir / name
    path.write_text("#!/usr/bin/env bash\n" + body + "\n")
    path.chmod(0o755)


def setup_run(
    root: Path,
    platform: str,
    seed: dict | None = None,
    *,
    args: tuple[str, ...] = (),
    prepare_home: Callable[[Path], None] | None = None,
    stubs: dict[str, str] | None = None,
    extra_env: dict[str, str] | None = None,
    without: tuple[str, ...] = (),
) -> Run:
    """Run the checked-in setup script with HOME in a temp dir and nothing real on PATH.

    ``agy`` is on PATH on both platforms: the skip has to come from the platform
    gate, not from a binary that happens to be missing. ``seed`` is written to
    Antigravity's MCP config before the run, standing in for a config that was
    already there. ``prepare_home`` writes anything else the home should hold
    before the run, and ``args`` are passed to the script (``--check``).
    ``stubs`` replaces or adds stub bodies by command name, written after the
    defaults, for a test that needs one command to do real work. ``extra_env``
    adds variables to the run's environment, such as XDG_CONFIG_HOME. ``without``
    names default stubs to leave off PATH, for a machine that lacks them.
    """
    bin_dir = root / "bin"
    bin_dir.mkdir(parents=True)
    home = root / "home"
    home.mkdir()
    if seed is not None:
        config = home / ".gemini" / "config" / "mcp_config.json"
        config.parent.mkdir(parents=True)
        config.write_text(json.dumps(seed, indent=2) + "\n")
    if prepare_home is not None:
        prepare_home(home)
    # A setup test builds no Rust gateway: the pin file skips that step and keeps the node runner.
    pin = home / ".config" / "agent-kit" / "model-gateway-node"
    pin.parent.mkdir(parents=True, exist_ok=True)
    pin.write_text("")
    calls = root / "calls.log"
    for name in ("uv", "npm", "node", "npx", "curl", "git", "codex", "vibe", "gh", "jq", "docker"):
        stub(bin_dir, name, f'echo "{name} $*" >> "{calls}"')
    # Converge reads Claude's plugin and marketplace lists as JSON: a machine with none prints an empty list.
    stub(bin_dir, "claude", f'echo "claude $*" >> "{calls}"\ncase "$*" in *"list --json"*) echo "[]" ;; esac')
    stub(bin_dir, "uname", f"echo {platform}")
    stub(bin_dir, "security", f'echo "security $*" >> "{calls}"')
    stub(bin_dir, "agy", 'echo "agy 1.0.0"')
    # Homebrew answers `list --cask` for what `install --cask` recorded, so a
    # fresh machine takes the install branch and a second run the upgrade one.
    stub(
        bin_dir,
        "brew",
        f'echo "brew $*" >> "{calls}"\n'
        'case "$1 $2" in\n'
        f'  "list --cask") [ -f "{root}/casks/$3" ] ;;\n'
        f'  "install --cask") mkdir -p "{root}/casks" && : > "{root}/casks/$3" ;;\n'
        "esac",
    )
    for name, body in (stubs or {}).items():
        stub(bin_dir, name, body)
    for name in without:
        (bin_dir / name).unlink()
    result = subprocess.run(
        ["bash", str(SETUP), *args],
        capture_output=True,
        text=True,
        check=False,
        # Not NixOS unless a test says so, whatever machine runs the suite.
        env={
            "HOME": str(home),
            "PATH": f"{bin_dir}:/usr/bin:/bin",
            "AK_NIXOS_MARKER": str(root / "not-nixos"),
            # Not the machine's /usr/local: where it is writable, as on a CI runner, setup would
            # install there and a cloud run would put the runner's real npm ahead of the stubs.
            "AK_SYSTEM_PREFIX": str(root / "no-system-prefix"),
            "AGENT_KIT_MERGE": merge_binary(),
            **(extra_env or {}),
        },
    )
    return Run(
        home=home,
        stdout=result.stdout,
        stderr=result.stderr,
        calls=calls.read_text().splitlines() if calls.exists() else [],
        returncode=result.returncode,
    )


def skill_text(skill_md: Path) -> str:
    """A skill's whole text: its SKILL.md, then each part file it links, in link order.

    A skill body stays under 10 KB, so the rest of its rules live in part files in
    the same folder; a test that checks a rule reads the skill through this.
    """
    import re as _re

    text = skill_md.read_text()
    parts: list[str] = []
    for name in _re.findall(r"\]\(([A-Za-z0-9_.-]+\.md)\)", text):
        path = skill_md.parent / name
        if name not in parts and path.is_file() and name != "SKILL.md":
            parts.append(name)
    return "\n".join([text, *(skill_md.parent.joinpath(n).read_text() for n in parts)])
