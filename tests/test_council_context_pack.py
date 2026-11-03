from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "council" / "context"
PACK = SOURCE / "pack.json"
# The pack names files where scripts/render_agent_kit.py --write puts them.
RENDERED = ".council/context/"


def entries() -> list[dict[str, str]]:
    pack = json.loads(PACK.read_text(encoding="utf-8"))
    return [entry for value in pack.values() if isinstance(value, list) for entry in value if isinstance(entry, dict)]


def test_every_pack_entry_points_at_a_file_that_exists() -> None:
    paths = [entry["path"] for entry in entries() if "path" in entry]

    assert all(path.startswith(RENDERED) for path in paths)
    missing = [path for path in paths if not (SOURCE / path.removeprefix(RENDERED)).is_file()]

    assert missing == []


def test_the_constitution_digest_is_not_described_as_a_checked_in_seed() -> None:
    # The Spec Kit constitution seed is retired (agent-kit#253); the digest is the only copy.
    digest = next(entry for entry in entries() if entry.get("ref") == "constitution-digest")

    assert "checked-in" not in digest["text"]
