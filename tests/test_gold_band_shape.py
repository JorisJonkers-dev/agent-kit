"""The shape rules of ``scripts/gold_band_shape.py`` (the setup-side twin of the ``workflow-lint`` crate's ``shape``
group, #780): every fixture breaks its own rule, and, where the crate's binary is available
(``WORKFLOW_LINT_BIN`` or ``scripts/target/debug/workflow-lint``), both implementations report the same
``(node, rule)`` pairs over the fixtures, the shipped templates and a set of odd definitions."""

from __future__ import annotations

import copy
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import gold_band_shape as shape  # noqa: E402

FIXTURES = KIT_ROOT / "scripts" / "crates" / "workflow-lint" / "tests" / "fixtures"
TEMPLATES = KIT_ROOT / "installer" / "gold-band" / "workflows"
NON_SHAPE = {"banned-binding", "opus-outside-judgment", "memory-write-outside-judgment", "loop-not-unrolled"}

REFUSED = {
    "multiple-entries": {"entry-multiple"},
    "validated-output-without-success-condition": {"success-expression-required"},
    "no-entry": {"entry-missing"},
    "no-end": {"end-required", "dangling-node"},
    "no-artifact": {"output-artifact-required"},
    "unparseable-success-condition": {"success-expression-invalid"},
    "schema-missing-path": {"schema-missing-path"},
    "legacy-json-schema": {"legacy-schema"},
    "manual-check-with-validation": {"manual-check-exclusive"},
    "dangling-node": {"dangling-node"},
    "edge-source-missing": {"edge-source-missing"},
    "edge-target-missing": {"edge-target-missing"},
    "edge-outcome-invalid": {"edge-outcome-invalid"},
    "failure-edge-without-result-decision": {"failure-edge-unsupported"},
    "duplicate-edge-outcome": {"duplicate-edge-outcome"},
    "reserved-node-id": {"reserved-node-id"},
    "duplicate-node-id": {"duplicate-node-id"},
    "max-attempts-not-positive": {"max-attempts-positive"},
    "max-rounds-not-positive": {"max-rounds-positive"},
    "slot-missing": {"slot-required"},
    "slot-duplicate": {"slot-duplicate"},
    "profile-missing": {"profile-required"},
    "terminal-edge-source": {"terminal-edge-source", "edge-source-missing"},
    "success-edge-to-new-round": {"success-new-round"},
    "new-round-without-entry": {"new-round-entry-required"},
    "continue-into-end": {"continue-terminal-target"},
}


def _definition(path: Path) -> dict[str, Any]:
    body = json.loads(path.read_text())
    return body.get("workflow", body)  # type: ignore[no-any-return]


def _pairs(definition: dict[str, Any]) -> list[tuple[str, str]]:
    return sorted((node, rule) for node, rule, _ in shape.shape_findings(definition))


def test_the_valid_fixture_passes() -> None:
    assert shape.shape_findings(_definition(FIXTURES / "shape-valid" / "check-round-loop.json")) == []


@pytest.mark.parametrize("name", sorted(REFUSED))
def test_each_refused_fixture_breaks_exactly_its_rule(name: str) -> None:
    found = shape.shape_findings(_definition(FIXTURES / "shape-refused" / f"{name}.json"))
    assert found
    assert {rule for _, rule, _ in found} <= REFUSED[name]
    assert all(node for node, _, _ in found)


def test_a_failure_target_is_not_an_entry_but_a_real_second_entry_is() -> None:
    valid = shape.shape_findings(_definition(FIXTURES / "shape-valid" / "failure-target.json"))
    assert valid == []
    cases = _odd_definitions()
    assert shape.shape_findings(cases["failure-target-sorted-first"]) == []
    assert [(n, r) for n, r, _ in shape.shape_findings(cases["real-second-entry"])] == [("g", "entry-multiple")]
    assert shape.shape_findings(cases["failure-loop-long"]) == []
    assert [r for _, r, _ in shape.shape_findings(cases["cycle"])] == ["entry-missing"]


def test_the_two_owner_reported_messages() -> None:
    found = shape.shape_findings(_definition(FIXTURES / "shape-refused" / "multiple-entries.json"))
    assert found[0][2].startswith("Workflow has multiple entry nodes: gate, check.")
    found = shape.shape_findings(
        _definition(FIXTURES / "shape-refused" / "validated-output-without-success-condition.json")
    )
    assert [m for _, _, m in found] == ["write requires a success expression when output validation is enabled."]


def _worker(ident: str, **extra: Any) -> dict[str, Any]:
    return {"type": "worker", "id": ident, "executionSlotId": f"s-{ident}", "profile": "p", **extra}


def _validated(ident: str, schema: Any, condition: Any) -> dict[str, Any]:
    return _worker(ident, output={"kind": "json", "artifact": "x", "schema": schema}, success_condition=condition)


def _wf(nodes: list[Any], edges: list[Any], **top: Any) -> dict[str, Any]:
    return {"id": "w", "nodes": nodes, "edges": edges, "control": {"max_attempts": 1, "max_rounds": 1}, **top}


END = [{"from": "a", "to": "$end", "on": "success"}]
SCHEMA = {"a": [{"b": "s"}, 1], "q": [[1, 2]], "e": [], "n": [None]}


def _odd_definitions() -> dict[str, dict[str, Any]]:
    cases: dict[str, dict[str, Any]] = {
        "empty": {"nodes": [], "edges": [], "control": {"max_attempts": 0, "max_rounds": "0"}},
        "no-control": {"id": "w", "nodes": [_worker("a")], "edges": END},
    }
    for value in (2, "3", " 4 ", True, False, "", "x", [1], -1, 0.5, None, "Infinity"):
        cases[f"limit-{value!r}"] = _wf([_worker("a")], END, control={"max_rounds": value})
    for index, path in enumerate(
        [
            "$.a[1]",
            "a",
            "$a",
            "$.a[0].b",
            "$",
            "",
            "$.a..b",
            ".a",
            "$.a[",
            "$.a[x]",
            "$.a[]",
            "$.a[0]b",
            "$[0]",
            "$.q[0][1]",
            "$.e[0]",
            "$.n[0]",
            "$.n[3]",
            "$.zz",
            "$.a.b",
            "$.a[99999999999999999999999]",
        ]
    ):
        cases[f"path-{index}"] = _wf([_validated("a", SCHEMA, {"path": path})], END)
    for index, condition in enumerate(
        [
            {"expression": "$.a >= 1"},
            {"expression": "$.a"},
            {"expression": "a == 1"},
            {"expression": 5},
            {"expression": "$ == 1"},
            {},
            {"path": None},
            {"path": 3},
            "text",
            True,
            0,
            None,
            [],
        ]
    ):
        cases[f"condition-{index}"] = _wf([_validated("a", SCHEMA, condition)], END)
    for index, schema in enumerate([{"type": "object"}, {"required": ["a"]}, ["a"], None, 0, False, "", {"a": 1}]):
        cases[f"schema-{index}"] = _wf([_validated("a", schema, {"path": "$.a"})], END)
    for index, artifact in enumerate(["  ", 3, None]):
        node = _validated("a", SCHEMA, {"path": "$.a"})
        node["output"]["artifact"] = artifact
        cases[f"artifact-{index}"] = _wf([node], END)
    both = _validated("a", SCHEMA, {"path": "$.a"})
    both["manual_check"] = True
    cases["manual-and-validation"] = _wf([both], END)
    cases["ai-dynamic"] = _wf([{"type": "ai-dynamic", "id": "a"}], END)
    cases["no-type"] = _wf([{"id": "a"}], [*END, {"from": "a", "to": "$end", "on": "failure"}])
    cases["non-object-node"] = _wf([7, _worker("a")], END)
    cases["falsy-failure-support"] = _wf(
        [_worker("a", output=0, manual_check=0)], [{"from": "a", "to": "$end", "on": "failure"}]
    )
    cases["dup-ids-slots"] = _wf(
        [
            _worker("a"),
            _worker("a", executionSlotId="s-b"),
            _worker("b", executionSlotId=" "),
            _worker("c", profile=""),
        ],
        END,
    )
    cases["reserved"] = _wf([_worker(i) for i in ("$end", "$entry", "$new-round", "")], END)
    cases["slot-dup"] = _wf([_worker("a", executionSlotId="d"), _worker("b", executionSlotId="d")], END)
    check = _worker("c", manual_check=True)
    base_nodes = [_worker("a"), check]
    for name, edges in {
        "endpoints": [
            {"from": "", "to": "$end", "on": "success"},
            {"from": "ghost", "to": "$end", "on": "success"},
            {"from": "a", "to": "", "on": "success"},
            {"from": "a", "to": "ghost", "on": "success"},
        ],
        "outcomes": [
            {"from": "a", "to": "c", "on": "success"},
            {"from": "a", "to": "$end", "on": "maybe"},
            {"from": "a", "to": "c", "on": "failure"},
            {"from": "c", "to": "$end", "on": "success"},
            {"from": "c", "to": "a", "on": "failure"},
            {"from": "c", "to": "$end", "on": "failure"},
        ],
        "dup-triple": [{"from": "a", "to": "c", "on": "success"}] * 3 + [{"from": "c", "to": "$end", "on": "success"}],
        "new-round": [
            {"from": "a", "to": "c", "on": "success"},
            {"from": "c", "to": "$end", "on": "success"},
            {"from": "c", "to": "$new-round", "on": "failure", "new_round_entry": "$entry"},
            {"from": "a", "to": "$new-round", "on": "success"},
            {"from": "c", "to": "$new-round", "on": "failure", "new_round_entry": "ghost"},
            {"from": "c", "to": "$new-round", "on": "failure", "new_round_entry": "a"},
            {"from": "c", "to": "$new-round", "on": "failure"},
        ],
        "terminals": [
            {"from": "a", "to": "c", "on": "success"},
            {"from": "c", "to": "$end", "on": "success", "session": "continue"},
            {"from": "$end", "to": "a", "on": "success"},
            {"from": "$new-round", "to": "a", "on": "success"},
        ],
        "cycle": [
            {"from": "a", "to": "c", "on": "success"},
            {"from": "c", "to": "a", "on": "success"},
            {"from": "c", "to": "$end", "on": "failure"},
        ],
        "backward-failure": [
            {"from": "a", "to": "c", "on": "success"},
            {"from": "c", "to": "a", "on": "failure"},
            {"from": "c", "to": "$end", "on": "success"},
        ],
    }.items():
        cases[name] = _wf(copy.deepcopy(base_nodes), edges)
    # Failure targets and entries (the build patch): "a" sorts first and is reached by a failure edge only.
    chain = [
        {"from": "g", "to": "c", "on": "success"},
        {"from": "c", "to": "$end", "on": "success"},
        {"from": "a", "to": "$end", "on": "success"},
    ]
    nodes = [_worker("a"), _worker("g"), check]
    cases["failure-target-sorted-first"] = _wf(
        copy.deepcopy(nodes), [*chain, {"from": "c", "to": "a", "on": "failure"}]
    )
    cases["real-second-entry"] = _wf(copy.deepcopy(nodes), chain)
    cases["failure-loop-long"] = _wf(
        copy.deepcopy([_worker("a"), _worker("b"), check]),
        [
            {"from": "a", "to": "b", "on": "success"},
            {"from": "b", "to": "c", "on": "success"},
            {"from": "c", "to": "a", "on": "failure"},
            {"from": "c", "to": "$end", "on": "success"},
        ],
    )
    return cases


def _binary() -> Path | None:
    named = os.environ.get("WORKFLOW_LINT_BIN")
    for candidate in (Path(named) if named else None, KIT_ROOT / "scripts" / "target" / "debug" / "workflow-lint"):
        if candidate is not None and candidate.is_file():
            return candidate
    return None


def _crate_pairs(binary: Path, path: Path) -> list[tuple[str, str]]:
    result = subprocess.run(
        [str(binary), "--shape", "--instance", "gold-band", str(path)], capture_output=True, text=True, check=False
    )
    assert result.returncode in (0, 1), result.stderr
    pairs = []
    for line in result.stderr.splitlines():
        subject, rule, _ = line.removeprefix(f"workflow-lint: {path}: ").split(": ", 2)
        if rule not in NON_SHAPE:
            pairs.append((subject, rule))
    return sorted(pairs)


@pytest.mark.skipif(_binary() is None, reason="no workflow-lint binary (set WORKFLOW_LINT_BIN)")
def test_the_setup_twin_and_the_crate_report_the_same_findings(tmp_path: Path) -> None:
    binary = _binary()
    assert binary is not None
    files = sorted(FIXTURES.glob("shape-*/*.json")) + sorted(TEMPLATES.glob("*.json"))
    for name, definition in _odd_definitions().items():
        path = tmp_path / f"{name}.json"
        path.write_text(json.dumps(definition))
        files.append(path)
    for path in files:
        assert _pairs(_definition(path)) == _crate_pairs(binary, path), path.name
