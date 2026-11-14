"""The setup guide's Gold Band sections and the lost-guarantee list (B1-22).

The guide may only name commands the setup harness runs, and the lost-guarantee
list may only cite sources and tests that exist.
"""

from __future__ import annotations

import re
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
GUIDE = (KIT_ROOT / "docs" / "SETUP.md").read_text()
BAND = (KIT_ROOT / "docs" / "GOLD-BAND.md").read_text()
INSTALLER = (KIT_ROOT / "installer" / "setup-workstation.sh").read_text()

SECTIONS = (
    "### Personal instance, Linux workstation",
    "### Personal instance, work Mac (second macOS user)",
    "### Work instance, work Mac",
    "### Work instance, Linux",
)


def _section(text: str, heading: str) -> str:
    start = text.index(heading)
    level = heading.split(" ")[0]
    rest = text[start + len(heading):]
    ends = [m.start() for m in re.finditer(rf"^#{{1,{len(level)}}} ", rest, re.MULTILINE)]
    return rest[: ends[0]] if ends else rest


def test_the_guide_has_a_section_per_instance_per_os() -> None:
    block = _section(GUIDE, "## Gold Band instances on each OS")
    for heading in SECTIONS:
        assert heading in block, heading


def test_every_command_the_guide_names_is_one_the_setup_harness_runs() -> None:
    block = _section(GUIDE, "## Gold Band instances on each OS")
    commands = [
        line.strip()
        for fence in re.findall(r"```bash\n(.*?)```", block, re.DOTALL)
        for line in fence.splitlines()
        if line.strip()
    ]
    assert commands
    for command in commands:
        words = command.split()
        if words[0].startswith("./installer/"):
            assert (KIT_ROOT / words[0]).is_file(), command
            continue
        script = next((w.strip('"') for w in words if "/" in w and not w.startswith("--")), words[0])
        name = Path(script.replace("${KIT_ROOT}", "")).name
        assert name in INSTALLER, command
        subcommand = [w for w in words[words.index(next(w for w in words if name in w)) + 1:] if not w.startswith("--")]
        for word in subcommand[:1]:
            assert word in INSTALLER, command
        for flag in (w for w in words if w.startswith("--")):
            assert flag in INSTALLER, command


def test_the_lost_guarantee_list_cites_sources_that_exist() -> None:
    block = _section(BAND, "## What a bare Gold Band node does not guarantee")
    rows = [line for line in block.splitlines() if line.startswith("| **")]
    assert len(rows) >= 10
    sources = {Path(p).name for p in re.findall(r"`([\w./-]+\.(?:ts|md))`", block)}
    assert sources
    present = {path.name for path in (KIT_ROOT / "council" / "ts" / "src").rglob("*.ts")}
    present |= {path.name for path in (KIT_ROOT / "docs").rglob("*.md")}
    assert sources <= present, sorted(sources - present)
