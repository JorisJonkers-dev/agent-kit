"""Gold Band's workflow editor validation rules, as ``converge.py gold-band-workflows`` applies them (#780).

This is the Python twin of the ``shape`` rule group of the ``workflow-lint`` crate
(``scripts/crates/workflow-lint/src/shape.rs``), which CI runs over every shipped
template. Setup needs the rules before the kit's cargo tools exist on a fresh
machine (``workspace_tools_setup`` runs after the workflow templates are
converged), so it cannot call the crate; ``tests/test_gold_band_shape.py`` runs both over the same
fixtures and the shipped templates and fails when their findings differ.

Ported from Gold Band v0.19.1, commit 987fc3e298b15d725a7f0e9b609b0f77dba63a74:
``web/src/components/WorkflowEditor.tsx`` (``validateWorkflowForSave``, lines
3395-3680, ``deriveWorkflowEntryCandidateIds`` :2904, ``nodeSupportsFailureOutcome``
:264) and ``web/src/components/workflowGraph.ts`` (``isBackwardEdge`` :39,
``workflowSuccessTopologyOrder`` :404). Not ported: the provider, model, permission-mode
and config-option availability checks, the role-visibility check and the duplicate
workflow id check (they need the running app's bindings and stores), and the
``ai-dynamic`` node rules.

Each finding is ``(subject, rule, message)``; ``subject`` is a node id or ``workflow``.
"""

from __future__ import annotations

import re
from collections import deque
from typing import Any

END_NODE = "$end"
ENTRY_NODE = "$entry"
NEW_ROUND_NODE = "$new-round"
WORKFLOW = "workflow"

Finding = tuple[str, str, str]


def _text(value: Any, key: str) -> str:
    found = value.get(key) if isinstance(value, dict) else None
    return found if isinstance(found, str) else ""


def _truthy(value: Any) -> bool:
    """JavaScript ``Boolean(x)`` of a JSON value."""
    if value is None or value is False:
        return False
    if isinstance(value, (str, int, float)):
        return bool(value)
    return True


def _supports_failure(node: Any) -> bool:
    return (
        isinstance(node, dict)
        and _text(node, "type") == "worker"
        and any(_truthy(node.get(k)) for k in ("manual_check", "output", "success_condition"))
    )


def _topology_order(ids: list[str], edges: list[Any]) -> dict[str, int]:
    known = set(ids)
    adjacency: dict[str, list[str]] = {i: [] for i in ids}
    indegree: dict[str, int] = dict.fromkeys(ids, 0)
    for edge in edges:
        source, target = _text(edge, "from"), _text(edge, "to")
        if _text(edge, "on") == "success" and source in known and target in known:
            adjacency[source].append(target)
            indegree[target] += 1
    queued: set[str] = set()
    queue: deque[str] = deque()
    for i in ids:
        if indegree[i] == 0 and i not in queued:
            queued.add(i)
            queue.append(i)
    ordered: list[str] = []
    while queue:
        current = queue.popleft()
        ordered.append(current)
        for nxt in adjacency[current]:
            indegree[nxt] = max(indegree[nxt] - 1, 0)
            if indegree[nxt] == 0 and nxt not in queued:
                queued.add(nxt)
                queue.append(nxt)
    ordered += [i for i in ids if i not in queued]
    return {i: index for index, i in enumerate(ordered)}


def _entry_candidates(nodes: list[Any], edges: list[Any]) -> list[str]:
    ids = [i for i in (_text(n, "id") for n in nodes) if i]
    unique = list(dict.fromkeys(ids))
    order = _topology_order(unique, edges)
    incoming: set[str] = set()
    for edge in edges:
        source, target = _text(edge, "from"), _text(edge, "to")
        if source not in order or target not in order:
            continue
        if _text(edge, "on") != "success" and order[target] < order[source]:
            continue
        incoming.add(target)
    return [i for i in ids if i not in incoming]


def _parse_json_path(path: str) -> list[tuple[str, Any]] | None:
    """The segments of a ``$.a.b[0]`` path, or None where the editor throws."""
    value = path.strip()
    if value.startswith("$."):
        value = value[2:]
    elif value == "$":
        return None
    elif value.startswith("$"):
        value = value[1:]
    if not value:
        return None
    segments: list[tuple[str, Any]] = []
    key = ""
    index = 0
    while index < len(value):
        char = value[index]
        if char == ".":
            if not key:
                if not segments or segments[-1][0] != "index":
                    return None
            else:
                segments.append(("key", key))
                key = ""
            index += 1
        elif char == "[":
            if key:
                segments.append(("key", key))
                key = ""
            close = value.find("]", index + 1)
            if close < 0:
                return None
            raw = value[index + 1 : close]
            if not re.fullmatch(r"[0-9]+", raw):
                return None
            segments.append(("index", int(raw)))
            index = close + 1
            if index < len(value) and value[index] not in ".[":
                return None
        else:
            key += char
            index += 1
    if key:
        segments.append(("key", key))
    return segments


def _success_path(condition: Any) -> list[tuple[str, Any]] | None:
    if not isinstance(condition, dict):
        return None
    if "expression" in condition:
        expression = condition["expression"]
        if not isinstance(expression, str):
            expression = ""
        operator = next((op for op in (">=", "<=", "!=", "==", ">", "<") if op in expression), None)
        if operator is None:
            return None
        left = expression.split(operator)[0].strip()
        return _parse_json_path(left) if left.startswith("$") else None
    path = condition.get("path")
    if path is None:
        return _parse_json_path("")
    return _parse_json_path(path) if isinstance(path, str) else None


def _looks_like_json_schema(schema: Any) -> bool:
    keys = ("type", "properties", "required", "additionalProperties", "items")
    return isinstance(schema, dict) and any(k in schema for k in keys)


def _schema_contains_path(schema: Any, path: list[tuple[str, Any]]) -> bool:
    cursor = schema
    for kind, segment in path:
        if kind == "key":
            if not isinstance(cursor, dict) or segment not in cursor:
                return False
            cursor = cursor[segment]
            continue
        if not isinstance(cursor, list):
            return False
        item = cursor[segment] if segment < len(cursor) else None
        if item is None:
            if not cursor:
                return False
            item = cursor[0]
        cursor = item
    return True


def _control_limit(value: Any) -> int:
    if isinstance(value, bool):
        return int(value)
    if isinstance(value, (int, float)):
        number = float(value)
    elif isinstance(value, str):
        try:
            number = float(value.strip()) if value.strip() else 0.0
        except ValueError:
            number = 0.0
    else:
        number = 0.0
    return int(number) if number == number and abs(number) != float("inf") else 0


def _worker_findings(node: dict[str, Any], slot_counts: dict[str, int], add: Any) -> None:
    ident = _text(node, "id")
    name = ident or "Unnamed node"
    slot = _text(node, "executionSlotId").strip()
    if not slot:
        add(ident, "slot-required", f"{name} node is missing an execution slot.")
    elif slot_counts.get(slot, 0) > 1:
        add(ident, "slot-duplicate", f"{name} node has a duplicate execution slot.")
    if not _text(node, "profile").strip():
        add(ident, "profile-required", f"{name} node is not associated with a role.")
    validation = _truthy(node.get("output")) or _truthy(node.get("success_condition"))
    if validation and _truthy(node.get("manual_check")):
        add(
            ident,
            "manual-check-exclusive",
            f"{name} cannot enable AI output validation and manual check at the same time.",
        )
    if not validation:
        return
    output = node.get("output")
    artifact = output.get("artifact") if isinstance(output, dict) else None
    if not isinstance(artifact, str) or not artifact.strip():
        add(ident, "output-artifact-required", f"{name} output artifact Key is required.")
    condition = node.get("success_condition")
    path = None
    if not _truthy(condition):
        add(
            ident,
            "success-expression-required",
            f"{name} requires a success expression when output validation is enabled.",
        )
    else:
        path = _success_path(condition)
        if path is None:
            add(ident, "success-expression-invalid", f"{name} has an invalid success expression.")
    schema = output.get("schema") if isinstance(output, dict) else None
    if _truthy(schema):
        if _looks_like_json_schema(schema):
            add(
                ident,
                "legacy-schema",
                f"{name} still uses legacy JSON Schema. Use the simplified output shape instead.",
            )
        elif path is not None and not _schema_contains_path(schema, path):
            add(
                ident,
                "schema-missing-path",
                f"{name} success expression path is not declared in the JSON output constraint.",
            )


def shape_findings(definition: dict[str, Any]) -> list[Finding]:
    """Every shape finding of one workflow definition (the ``workflow`` object)."""
    out: list[Finding] = []

    def add(subject: str, rule: str, message: str) -> None:
        out.append((subject or WORKFLOW, rule, message))

    raw_nodes, raw_edges = definition.get("nodes"), definition.get("edges")
    nodes: list[Any] = raw_nodes if isinstance(raw_nodes, list) else []
    edges: list[Any] = raw_edges if isinstance(raw_edges, list) else []
    if not _text(definition, "id").strip():
        add(WORKFLOW, "workflow-id-required", "Workflow ID is required.")
    if not nodes:
        add(WORKFLOW, "nodes-required", "Workflow requires at least one node.")
    else:
        entries = _entry_candidates(nodes, edges)
        if not entries:
            add(WORKFLOW, "entry-missing", "Workflow must have exactly one entry node with no incoming edge.")
        for extra in entries[1:]:
            add(
                extra,
                "entry-multiple",
                f"Workflow has multiple entry nodes: {', '.join(entries)}. "
                "Connect the graph so there is exactly one entry.",
            )
    if not any(_text(e, "to") == END_NODE for e in edges):
        add(WORKFLOW, "end-required", "Workflow must include an end node.")
    control = definition.get("control")
    for key, rule, label in (
        ("max_attempts", "max-attempts-positive", "Max attempts"),
        ("max_rounds", "max-rounds-positive", "Max rounds"),
    ):
        limit = control.get(key) if isinstance(control, dict) else None
        if limit is not None and _control_limit(limit) <= 0:
            add(WORKFLOW, rule, f"{label} must be greater than 0.")

    id_counts: dict[str, int] = {}
    slot_counts: dict[str, int] = {}
    for node in nodes:
        id_counts[_text(node, "id")] = id_counts.get(_text(node, "id"), 0) + 1
        slot = _text(node, "executionSlotId")
        if _text(node, "type") == "worker" and slot.strip():
            slot_counts[slot] = slot_counts.get(slot, 0) + 1
    for node in nodes:
        ident = _text(node, "id")
        name = ident or "Unnamed node"
        if not ident.strip():
            add(ident, "node-id-required", f"{name} node ID is required.")
        if ident in (END_NODE, ENTRY_NODE, NEW_ROUND_NODE):
            add(ident, "reserved-node-id", f"{name} uses a reserved node ID.")
        if id_counts[ident] > 1:
            add(ident, "duplicate-node-id", f"{name} node ID is duplicated.")
        if not ident.strip() or not any(_text(e, "from") == ident for e in edges):
            add(ident, "dangling-node", f"{name} has no outgoing edge and cannot continue or end the workflow.")
        if _text(node, "type") != "ai-dynamic":
            _worker_findings(node if isinstance(node, dict) else {}, slot_counts, add)

    ids = {i for i in (_text(n, "id") for n in nodes) if i}
    by_id = {_text(n, "id"): n for n in nodes}
    counts: dict[tuple[str, str], int] = {}
    for edge in edges:
        source, on = _text(edge, "from"), _text(edge, "on")
        if source.strip() and on in ("success", "failure"):
            counts[(source, on)] = counts.get((source, on), 0) + 1
    reported: set[tuple[str, str]] = set()
    for index, edge in enumerate(edges):
        source, target, on = _text(edge, "from"), _text(edge, "to"), _text(edge, "on")
        n = index + 1
        if not source.strip():
            add(source, "edge-source-required", f"Edge {n} is missing a source node.")
        elif source not in ids:
            add(source, "edge-source-missing", f"Edge source node {source} does not exist.")
        if not target.strip():
            add(source, "edge-target-required", f"Edge {n} is missing a target node.")
        elif target not in (END_NODE, NEW_ROUND_NODE) and target not in ids:
            add(source, "edge-target-missing", f"Edge target node {target} does not exist.")
        if target == NEW_ROUND_NODE:
            entry = _text(edge, "new_round_entry").strip()
            if not entry:
                add(
                    source,
                    "new-round-entry-required",
                    f"{source} edge targeting new round must choose the next round start.",
                )
            elif entry != ENTRY_NODE and entry not in ids:
                add(
                    source,
                    "new-round-entry-missing",
                    f"{source} edge targeting new round selected missing start node {entry}.",
                )
        if source in (END_NODE, NEW_ROUND_NODE):
            add(source, "terminal-edge-source", f"End node {source} cannot be an edge source.")
        if _text(edge, "session") == "continue" and target in (END_NODE, NEW_ROUND_NODE):
            add(source, "continue-terminal-target", f"Edge {n} continue session cannot target an end node.")
        if on not in ("success", "failure"):
            add(source, "edge-outcome-invalid", f"Edge {n} has an invalid type.")
        elif on == "failure" and not _supports_failure(by_id.get(source)):
            add(
                source,
                "failure-edge-unsupported",
                f"{source} can only have a failure branch when AI output validation or manual check is enabled.",
            )
        elif on == "success" and target == NEW_ROUND_NODE:
            add(source, "success-new-round", f"{source} success edge cannot target new round.")
        elif source.strip() and counts[(source, on)] > 1 and (source, on) not in reported:
            reported.add((source, on))
            add(
                source,
                "duplicate-edge-outcome",
                f"{source} has {counts[(source, on)]} {on} edges; each outcome type can only have one edge.",
            )
    return out
