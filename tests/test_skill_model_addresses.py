"""The delegate and review skills name models through addresses only.

An address is `free/<model>`, `paid/<model>`, `auto/<model>`,
`<harness>:<pricing>/<model>` or a `tier-*` name. A provider's own model id (a Claude,
GPT, Gemini, GLM, Kimi or DeepSeek version) belongs in the registry and the gateway's
defaults, never in these skills, so a model change never edits a skill.
Family names (`deepseek`, `zhipu`, `mistral`, `google`) are routing keys, not ids.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
WORKSTATION = KIT_ROOT / "skills" / "workstation"
SKILL_DIRS = sorted(
    [WORKSTATION / "delegate", WORKSTATION / "critique", WORKSTATION / "critique-adrs", WORKSTATION / "critique-plan"]
    + list(WORKSTATION.glob("review-*"))
)
# critique/ also holds the runner's own README and prompts, which document the routed
# provider ids by design (routing.json is their source); only its SKILL.md is a skill
# body that an orchestrator reads.
FILES = [p for d in SKILL_DIRS for p in sorted(d.rglob("*.md")) if d.name != "critique" or p.name == "SKILL.md"]

# A provider's own model id: a versioned family name.
PROVIDER_ID = re.compile(
    r"\b(?:claude-(?:opus|sonnet|haiku)-[\w.-]+|gpt-\d[\w.-]*|gemini-\d[\w.-]*|glm-\d[\w.-]*|kimi-[kv]?\d[\w.-]*"
    r"|deepseek-v?\d[\w.-]*|mistral-(?:small|large|medium)[\w.-]*|codestral[\w.-]*|qwen\d[\w.-]*|minimax-m?\d[\w.-]*)",
    re.I,
)
# Inside an address the id is the address's `<model>` part.
ADDRESS = re.compile(r"(?:[a-z-]+:)?(?:free|paid|auto)/\S+", re.I)
# A client:model pair, `<client>:<model>` with a provider id after the colon.
CLIENT_MODEL = re.compile(r"\b(?:claude|codex|vibe|antigravity|opencode|pi|pi-vibe|hermes):(?!free/|paid/|auto/)[\w.-]*\d", re.I)

# What must stay, and why. Each entry is (file name, matched text).
ALLOWED: dict[tuple[str, str], str] = {}


def _hits(text: str) -> list[str]:
    stripped = ADDRESS.sub("", text)
    return PROVIDER_ID.findall(stripped) + CLIENT_MODEL.findall(stripped)


@pytest.mark.parametrize("path", FILES, ids=lambda p: f"{p.parent.name}/{p.name}")
def test_the_skill_names_models_only_through_addresses(path: Path) -> None:
    found = [h for h in _hits(path.read_text()) if (path.name, h) not in ALLOWED]
    assert not found, f"{path.relative_to(KIT_ROOT)} names a provider model id {found}; use free/, paid/, auto/, <harness>:<pricing>/ or tier-*"


def test_the_check_would_catch_a_provider_id() -> None:
    assert _hits('"model": "claude-sonnet-5-5"')
    assert _hits('"client": "pi-vibe", "model": "glm-5.3"')
    assert _hits("use codex:gpt-5.5 here")
    assert not _hits("use `pi:free/glm-4.7-flash` or `paid/glm-5.3` or `tier-vibe`")
