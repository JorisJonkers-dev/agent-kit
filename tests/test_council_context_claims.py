from __future__ import annotations

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTEXT = ROOT / "council" / "context"


def test_the_repo_map_does_not_describe_retired_render_sources() -> None:
    # agent-kit#280
    text = " ".join((CONTEXT / "repo-map.md").read_text().split())

    assert "repo templates" not in text
    assert "installer templates" not in text


def test_pack_refs_cover_every_fragment_source() -> None:
    # agent-kit#281
    refs = set(json.loads((CONTEXT / "pack.json").read_text())["refs"])
    for fragment in sorted(CONTEXT.glob("*.md")):
        match = re.search(r"^source: (.+)$", fragment.read_text(), re.MULTILINE)
        assert match, fragment.name
        sources = {part.strip() for part in match.group(1).split(";")}
        assert sources <= refs, (fragment.name, sorted(sources - refs))


def test_every_pack_ref_exists() -> None:
    refs = json.loads((CONTEXT / "pack.json").read_text())["refs"]

    assert [ref for ref in refs if not (ROOT / ref).is_file()] == []
