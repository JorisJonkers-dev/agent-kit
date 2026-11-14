"""CI splits the suite into shards; together the shards must run every test exactly once."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ci_shard

IDS = [f"tests/test_{f}.py::test_{n}" for f in ("b", "a", "c") for n in range(7)]


def test_the_shards_together_select_every_test_exactly_once() -> None:
    count = 4
    selected = [p for index in range(1, count + 1) for p in ci_shard.select(IDS, index, count)]

    assert sorted(selected) == list(range(len(IDS)))


def test_the_split_does_not_depend_on_the_collection_order() -> None:
    shuffled = IDS[7:] + IDS[:7]

    def chosen(ids: list[str]) -> set[str]:
        return {ids[p] for p in ci_shard.select(ids, 2, 3)}

    assert chosen(IDS) == chosen(shuffled)


def test_a_shard_keeps_its_tests_in_collection_order() -> None:
    positions = ci_shard.select(IDS, 1, 3)

    assert positions == sorted(positions)


def test_the_shards_differ_in_size_by_at_most_one_test() -> None:
    sizes = [len(ci_shard.select(IDS, index, 4)) for index in range(1, 5)]

    assert max(sizes) - min(sizes) <= 1


def test_a_shard_spec_reads_as_index_and_count() -> None:
    assert ci_shard.parse("2/4") == (2, 4)


@pytest.mark.parametrize("spec", ["0/4", "5/4", "2", "two/four", "1/0"])
def test_a_shard_spec_outside_one_to_count_is_refused(spec: str) -> None:
    with pytest.raises(ValueError, match="PYTEST_SHARD"):
        ci_shard.parse(spec)
