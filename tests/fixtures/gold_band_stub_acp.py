#!/usr/bin/env python3
"""A stub ACP agent for the Gold Band headless proof (issue #348).

Python 3 stdlib only. JSON-RPC 2.0 over newline-delimited JSON on stdio,
the framing Gold Band uses to drive an ACP agent during ``gold-band run``:
it answers ``initialize``, ``session/new``, ``session/list``,
``session/set_config_option``, ``session/set_mode``, ``session/delete``,
``session/close`` and ``session/prompt``. Before answering
``session/prompt`` it sends one ``agent_message_chunk`` whose text is a
fenced JSON findings document valid against the critique pipeline's finding
schema, then answers ``{"stopReason": "end_turn"}``.

The verdict comes from the environment: ``STUB_ACP_VERDICT=needs-human``
makes every findings document carry one needs-human case (a major finding a
human must decide), so the workflow's success condition fails and the run
routes to its manual-check node and pauses with exit code 2. Any other
value reports an empty findings document, so a clean run ends with exit 0.

Run as ``gold_band_stub_acp.py`` (the test launches it with
``sys.executable``); it needs no arguments and talks only on stdio.
"""

from __future__ import annotations

import json
import os
import sys
import uuid

# The findings document every session/prompt answers with on the clean
# path: no finding, and the reason that says the stub actually looked.
CLEAN = {
    "findings": [],
    "no_findings_reason": (
        "The stub critic read the requirement through its lenses and raised nothing; "
        "this is the empty verdict the headless proof expects."
    ),
}

# The findings document on the needs-human path: one finding that states a
# person must decide before the spec stage can close.
NEEDS_HUMAN = {
    "findings": [
        {
            "id": "stub-needs-human-1",
            "severity": "major",
            "category": "undocumented-decision",
            "check": "spec-redteam-needs-human",
            "location": "requirement.md",
            "claim": (
                "The spec stage's verdict is needs-human: this finding is the marker "
                "the manual-check node exists to put in front of a person."
            ),
            "evidence": [{"kind": "input", "value": "STUB_ACP_VERDICT=needs-human"}],
            "suggestion": "Answer the manual check in Gold Band: the human rules on this finding.",
        }
    ]
}

VERDICT_ENV = "STUB_ACP_VERDICT"
NEEDS_HUMAN_VERDICT = "needs-human"


def findings_document() -> dict:
    """The findings document this run answers every prompt with."""
    verdict = os.environ.get(VERDICT_ENV, "").strip() or ""
    return NEEDS_HUMAN if verdict == NEEDS_HUMAN_VERDICT else CLEAN


def fenced_findings() -> str:
    """The one agent_message_chunk text: the findings document as fenced JSON."""
    return "```json\n" + json.dumps(findings_document(), indent=2) + "\n```"


def emit(message: dict) -> None:
    """One NDJSON frame, flushed, so Gold Band sees it immediately."""
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def reply(request: dict, result: dict) -> None:
    """One JSON-RPC response to a request."""
    emit({"jsonrpc": "2.0", "id": request.get("id"), "result": result})


def notify(method: str, params: dict) -> None:
    """One JSON-RPC notification from the agent to the client."""
    emit({"jsonrpc": "2.0", "method": method, "params": params})


def agent_chunk(content: dict) -> dict:
    """One session/update notification carrying an agent_message_chunk, in
    ACP's shape: params.update.sessionUpdate names the kind."""
    return {
        "sessionId": content["sessionId"],
        "update": {
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": content["text"]},
        },
    }


MODELS_ENV = "STUB_ACP_MODELS"


def model_option(current: str | None = None) -> list[dict]:
    """The model picker Gold Band reads from session/new to check a workflow's
    model bindings: one config option of category "model" whose options are
    the comma-separated ids in STUB_ACP_MODELS."""
    models = [m.strip() for m in os.environ.get(MODELS_ENV, "").split(",") if m.strip()]
    if not models:
        return []
    return [{
        "id": "model",
        "name": "Model",
        "category": "model",
        "type": "select",
        "currentValue": current or models[0],
        "options": [{"value": m, "name": m} for m in models],
    }]


def handle(request: dict, sessions: set[str]) -> None:
    """Answer one request in place, notifying before session/prompt answers."""
    method = request.get("method", "")
    params = request.get("params") or {}
    if method == "initialize":
        reply(
            request,
            {
                "protocolVersion": 1,
                "agentCapabilities": {"loadSession": False},
                "authMethods": [],
            },
        )
    elif method == "session/new":
        session_id = str(uuid.uuid4())
        sessions.add(session_id)
        reply(request, {"sessionId": session_id, "configOptions": model_option()})
    elif method == "session/list":
        reply(request, {"sessions": []})
    elif method == "session/set_config_option":
        value = params.get("value") if params.get("configId") == "model" else None
        reply(request, {"configOptions": model_option(value)})
    elif method == "session/set_mode":
        reply(request, {})
    elif method in ("session/delete", "session/close"):
        sessions.discard(params.get("sessionId"))
        reply(request, {})
    elif method == "session/prompt":
        session_id = params.get("sessionId") or next(iter(sessions), "")
        notify("session/update", agent_chunk({"sessionId": session_id, "text": fenced_findings()}))
        reply(request, {"stopReason": "end_turn"})
    elif request.get("id") is not None:
        # A method the proof does not expect: answer as the protocol says, so
        # a surprise surfaces as a protocol error instead of a hang.
        emit(
            {
                "jsonrpc": "2.0",
                "id": request["id"],
                "error": {"code": -32601, "message": "Method not found"},
            }
        )


def main() -> int:
    sessions: set[str] = set()
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(request, dict):
            handle(request, sessions)
    return 0


if __name__ == "__main__":
    sys.exit(main())
