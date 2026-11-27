"""Setup clears dangling uv tool links before installing a uv tool.

uv's tool installer refuses to overwrite an executable already in its bin
directory, even a symlink whose tool environment is gone. After the tool
directory is wiped or migrated, `~/.local/bin/hermes` still points at the old
environment, `command -v hermes` fails, and every setup run reports
"hermes still not on PATH after install". The helper removes only links that
dangle and point into uv's own tool directory.
"""

from __future__ import annotations

import os
import re
import subprocess
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
SETUP = KIT_ROOT / "installer" / "setup-workstation.sh"


def _helper() -> str:
    text = SETUP.read_text()
    match = re.search(r"^clear_dangling_uv_links\(\) \{\n.*?^\}\n", text, re.S | re.M)
    assert match, "setup-workstation.sh defines clear_dangling_uv_links"
    return match.group(0)


def _run(tmp_path: Path, check_only: str) -> tuple[Path, Path]:
    tool_dir = tmp_path / "tools"
    bin_dir = tmp_path / "bin"
    stubs = tmp_path / "stubs"
    for d in (tool_dir / "live" / "bin", bin_dir, stubs):
        d.mkdir(parents=True)
    (tool_dir / "live" / "bin" / "live").write_text("")
    os.symlink(tool_dir / "gone" / "bin" / "gone", bin_dir / "gone")
    os.symlink(tool_dir / "gone" / "bin" / "gone-acp", bin_dir / "gone-acp")
    os.symlink(tool_dir / "live" / "bin" / "live", bin_dir / "live")
    os.symlink(tmp_path / "elsewhere" / "other", bin_dir / "other")
    (bin_dir / "plain").write_text("")
    uv = stubs / "uv"
    uv.write_text(
        "#!/bin/sh\n"
        f'if [ "$3" = --bin ]; then echo {bin_dir}; else echo {tool_dir}; fi\n',
    )
    uv.chmod(0o755)
    script = f'log() {{ :; }}\nCHECK_ONLY={check_only}\n{_helper()}clear_dangling_uv_links\n'
    env = {**os.environ, "PATH": f"{stubs}:{os.environ['PATH']}"}
    subprocess.run(["bash", "-euo", "pipefail", "-c", script], check=True, env=env)
    return bin_dir, tool_dir


def test_removes_only_dangling_links_into_the_uv_tool_dir(tmp_path: Path) -> None:
    bin_dir, _ = _run(tmp_path, "0")
    assert sorted(p.name for p in bin_dir.iterdir()) == ["live", "other", "plain"]


def test_check_mode_changes_nothing(tmp_path: Path) -> None:
    bin_dir, _ = _run(tmp_path, "1")
    assert sorted(p.name for p in bin_dir.iterdir()) == ["gone", "gone-acp", "live", "other", "plain"]


def test_every_fresh_uv_tool_install_in_setup_clears_dangling_links_first() -> None:
    lines = [line for line in SETUP.read_text().splitlines() if "run_sh 'uv tool " + "install" in line]
    fresh = [line for line in lines if "--reinstall" not in line]
    assert fresh
    for line in fresh:
        assert line.strip().startswith("clear_dangling_uv_links; run_sh"), line
