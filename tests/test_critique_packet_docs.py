"""What the docs say a code packet holds.

Since agent-kit#221 and #254 a code packet no longer freezes every tracked
file at the commit under review: it sends the filtered diff, the repo map,
the context slices and as much of the tree as the packet cap lets in, with
the facts index recording every drop. A document that still says a code
packet holds every file of the tree tells the owner, and an orchestrator
reading a stage skill, something the runner no longer does.

The old wording may stay as history only inside a section whose heading says
it is superseded (``## What #186 settled (superseded 2026-11-02)``). Such a
section runs to the next heading of the same or a higher level.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
SEARCHED = [KIT_ROOT / "docs", KIT_ROOT / "skills"]
INTEGRATION = KIT_ROOT / "docs" / "critique-pipeline" / "integration.md"

HEADING = re.compile(r"^(#{1,6})\s+(.*)$")

# Ways a document has said that a code packet holds the whole tree. The text
# is matched with its line breaks and runs of spaces folded to one space.
WHOLE_TREE_CLAIMS = [
    re.compile(r"every (?:regular |tracked |text )?file (?:of|at|in) (?:the |`head`'s )?(?:staged |whole )?(?:tree|head)\b", re.I),
    re.compile(r"every tracked file", re.I),
    re.compile(r"every tree file is a packet file", re.I),
    re.compile(r"\b(?:holds|freezes|froze|sends) the whole tree\b", re.I),
]


def outside_superseded(text: str) -> str:
    """The text without any section whose heading says it is superseded, fenced code included."""
    kept: list[str] = []
    skipping_below: int | None = None
    fenced = False
    for line in text.splitlines():
        if line.lstrip().startswith("```"):
            fenced = not fenced
        heading = None if fenced else HEADING.match(line)
        if heading is not None:
            level = len(heading.group(1))
            if skipping_below is not None and level <= skipping_below:
                skipping_below = None
            if skipping_below is None and "superseded" in heading.group(2).lower():
                skipping_below = level
        if skipping_below is None:
            kept.append(line)
    return "\n".join(kept)


def whole_tree_claims(text: str) -> list[str]:
    """Each phrase in the text that says a code packet holds every file of the tree."""
    folded = re.sub(r"\s+", " ", text)
    return [match.group(0) for pattern in WHOLE_TREE_CLAIMS for match in pattern.finditer(folded)]


def searched_docs() -> list[Path]:
    return sorted(path for root in SEARCHED for path in root.rglob("*.md"))


def test_the_check_reads_docs_from_both_trees() -> None:
    docs = searched_docs()
    assert any(path.is_relative_to(KIT_ROOT / "docs") for path in docs)
    assert any(path.is_relative_to(KIT_ROOT / "skills") for path in docs)


@pytest.mark.parametrize("path", searched_docs(), ids=lambda path: str(path.relative_to(KIT_ROOT)))
def test_no_doc_says_a_code_packet_holds_every_tracked_file(path: Path) -> None:
    claims = whole_tree_claims(outside_superseded(path.read_text()))
    assert claims == [], f"{path.relative_to(KIT_ROOT)} still says a code packet holds the whole tree: {claims}"


def test_the_superseded_186_notes_are_still_caught_by_the_check() -> None:
    """The #186 notes stay as history, and they are what the check is for: it must still see them."""
    text = INTEGRATION.read_text()
    heading = re.search(r"^## What #186 settled \(superseded 2026-11-02\)$", text, re.M)
    assert heading is not None, "the #186 section is kept and marked superseded with its date"
    assert whole_tree_claims(text) != []
    assert whole_tree_claims(outside_superseded(text)) == []


def test_a_superseded_section_ends_at_the_next_heading_of_its_level() -> None:
    text = "\n".join(
        [
            "## Old (superseded 2026-11-02)",
            "A code packet holds every file of the tree.",
            "### Detail",
            "It froze the whole tree.",
            "## New",
            "A code packet holds every file of the tree.",
        ]
    )
    assert whole_tree_claims(outside_superseded(text)) == ["every file of the tree"]


def test_a_heading_inside_a_fence_does_not_open_or_close_a_section() -> None:
    text = "\n".join(
        [
            "## Old (superseded 2026-11-02)",
            "```",
            "## not a heading",
            "```",
            "It sends the whole tree.",
            "## New",
        ]
    )
    assert whole_tree_claims(outside_superseded(text)) == []


@pytest.mark.parametrize(
    "claim",
    [
        "`tree/<repository path>` for every regular file of\n  `head`'s tree",
        "the spec, the diff and every file of the staged tree",
        "every ticket, the diff against the branch base and every file at the head",
        "every tree file is a packet file, so each critic's copy holds the whole tree",
    ],
)
def test_the_check_catches_the_wording_the_docs_used(claim: str) -> None:
    assert whole_tree_claims(claim) != []


@pytest.mark.parametrize(
    "fine",
    [
        "`tree/MANIFEST.tsv` listing every tracked path with its line count",
        "`facts/INDEX.md` records every file the cap dropped",
        "every file for a document packet",
        "as much of the rest of the tree as the packet cap lets in",
    ],
)
def test_the_check_leaves_true_statements_alone(fine: str) -> None:
    assert whole_tree_claims(fine) == []
