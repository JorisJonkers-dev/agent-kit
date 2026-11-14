"""Split the collected tests into CI shards, so each shard's job runs a part of the suite.

``PYTEST_SHARD=2/4`` keeps the second of four parts. Each test goes to exactly one
shard. The split is by the sorted test id, not by collection order, so every
shard computes the same split even when a parametrization's order depends on
the process's hash seed. Round-robin over the sorted ids spreads each file's
tests across all shards, which keeps the shards about equally long.
"""

from __future__ import annotations

import re
from collections.abc import Sequence
from typing import TypeVar

ENV = "PYTEST_SHARD"

T = TypeVar("T")

_SPEC = re.compile(r"(\d+)/(\d+)")


def parse(spec: str) -> tuple[int, int]:
    """``"2/4"`` as ``(2, 4)``; refuses anything but ``index/count`` with 1 <= index <= count."""
    match = _SPEC.fullmatch(spec.strip())
    if match is None:
        raise ValueError(f"{ENV}={spec!r}: expected <index>/<count>, such as 1/4")
    index, count = int(match.group(1)), int(match.group(2))
    if not 1 <= index <= count:
        raise ValueError(f"{ENV}={spec!r}: the index must be between 1 and the count")
    return index, count


def select(ids: Sequence[str], index: int, count: int) -> list[int]:
    """Positions in ``ids`` that belong to shard ``index`` of ``count``, in their original order."""
    rank = {position: n for n, position in enumerate(sorted(range(len(ids)), key=lambda p: ids[p]))}
    return [position for position in range(len(ids)) if rank[position] % count == index - 1]
