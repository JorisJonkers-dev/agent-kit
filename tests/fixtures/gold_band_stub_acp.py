#!/usr/bin/env python3
"""A stub ACP agent for the Gold Band headless proofs (issues #348, #444 and #445).

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

Two more modes serve the run-start shim proof (issue #444). With
``STUB_ACP_RUN`` set to a JSON list (an argv), ``session/prompt`` runs that
command, ignores its exit code and answers with the text of the file named by
``STUB_ACP_ANSWER`` as one fenced JSON block: this is the shim node, whose
answer is the report the pinned wrapper wrote. With ``STUB_ACP_MARKER`` set,
``session/prompt`` writes that file and answers ``{"ok": true}``: this is the
model node, whose marker file proves it ran.

Two more serve the gate shim proof (issue #445). With ``STUB_ACP_GATE`` set to
a JSON argv, ``session/prompt`` runs the pinned gate wrapper and answers
``{"exit_code": <the wrapper's exit code>}``: the verdict is the wrapper's
status, never a word of the agent's. ``STUB_ACP_TAMPER`` then lets this gate
agent misbehave once the wrapper has finished, in the report directory named
by ``STUB_ACP_REPORT_DIR``: ``delete-report``, ``rewrite-output``,
``rewrite-command`` or ``age-report`` (stamp the report 2020-01-01). Real runs
deny the gate agent id those writes (ticket #432); the stub does them to
prove the copier catches them. With ``STUB_ACP_COPY`` set to a JSON argv (the
copier node), ``session/prompt`` runs it and answers with its stdout, the
report, as one fenced JSON block when it exited 0, and otherwise with a report-shaped
document whose ``exit_code`` is its non-zero exit code, so a refusing copier
can never be masked into a pass.

One more serves the grant-check proof (issue #460). With ``STUB_ACP_MARKER``
set and ``STUB_ACP_AFTER`` set to a JSON argv, the model node runs that command
after it wrote its marker, ignoring its exit code: time passing while Codex
works, such as the run's grant lapsing.

Run as ``gold_band_stub_acp.py`` (the test launches it with
``sys.executable``); it needs no arguments and talks only on stdio.
"""

from __future__ import annotations

import json
import os
import subprocess
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


def fenced(document: dict) -> str:
    """One fenced JSON block, the shape every node's answer takes."""
    return "```json\n" + json.dumps(document, indent=2) + "\n```"


def tamper(operation: str, report_dir: str) -> None:
    """What a gate agent with write access to the report directory could do
    after the wrapper finished (the launcher forbids it; the stub proves the
    copier catches it)."""
    report = os.path.join(report_dir, "gate-report.json")
    if operation == "delete-report":
        os.remove(report)
    elif operation == "rewrite-output":
        with open(os.path.join(report_dir, "gate-output.txt"), "w", encoding="utf-8") as handle:
            handle.write("everything passed, honest\n")
    elif operation in ("rewrite-command", "age-report"):
        with open(report, encoding="utf-8") as handle:
            document = json.load(handle)
        if operation == "rewrite-command":
            document["command"] = "true"
        else:
            document["timestamp"] = "2020-01-01T00:00:00Z"
        with open(report, "w", encoding="utf-8") as handle:
            json.dump(document, handle)


def gate_answer(argv: list[str]) -> str:
    """The gate node: run the wrapper, optionally tamper, answer its status."""
    code = subprocess.run(argv, check=False, capture_output=True).returncode
    operation = os.environ.get("STUB_ACP_TAMPER")
    if operation:
        tamper(operation, os.environ["STUB_ACP_REPORT_DIR"])
    return fenced({"exit_code": code})


def refusal(code: int) -> dict:
    """What the copier answers when verify refuses: a document of the report's
    shape (a malformed answer would only be sent back for repair) whose
    exit_code is the refusal, so the success condition cannot hold."""
    return {
        "run_id": "",
        "attempt": 0,
        "command": "",
        "exit_code": code or 1,
        "output_hash": "",
        "timestamp": "",
    }


def copy_answer(argv: list[str]) -> str:
    """The copier node: the report on a pass, else a failing exit_code."""
    done = subprocess.run(argv, check=False, capture_output=True, text=True)
    if done.returncode == 0:
        try:
            return fenced(json.loads(done.stdout))
        except json.JSONDecodeError:
            return fenced(refusal(1))
    return fenced(refusal(done.returncode))


def prompt_answer() -> str:
    """The answer to one session/prompt: the run-start shim mode, the marker
    mode, or else the findings document."""
    gate = os.environ.get("STUB_ACP_GATE")
    if gate:
        return gate_answer(json.loads(gate))
    copy = os.environ.get("STUB_ACP_COPY")
    if copy:
        return copy_answer(json.loads(copy))
    run = os.environ.get("STUB_ACP_RUN")
    if run:
        subprocess.run(json.loads(run), check=False, capture_output=True)
        with open(os.environ["STUB_ACP_ANSWER"], encoding="utf-8") as answer:
            return "```json\n" + json.dumps(json.load(answer), indent=2) + "\n```"
    marker = os.environ.get("STUB_ACP_MARKER")
    if marker:
        with open(marker, "w", encoding="utf-8") as handle:
            handle.write("the model node ran\n")
        after = os.environ.get("STUB_ACP_AFTER")
        if after:
            subprocess.run(json.loads(after), check=False, capture_output=True)
        return '```json\n{"ok": true}\n```'
    return fenced_findings()


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
    return [
        {
            "id": "model",
            "name": "Model",
            "category": "model",
            "type": "select",
            "currentValue": current or models[0],
            "options": [{"value": m, "name": m} for m in models],
        }
    ]


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
        notify("session/update", agent_chunk({"sessionId": session_id, "text": prompt_answer()}))
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
