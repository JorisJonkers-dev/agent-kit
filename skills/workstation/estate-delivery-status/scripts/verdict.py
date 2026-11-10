#!/usr/bin/env python3
"""Apply the estate delivery verdict to signals gathered by SKILL.md.

Reads one JSON object from stdin, or from --signals FILE. Prints a
`VERDICT:` line, then one line per signal. Exits 0 for any well-formed
input and 2 for malformed input (message on stderr).
"""

import argparse
import json
import re
import sys

PUBLISH_STATES = {"success", "failure", "missing"}
COMPOSE_STATES = {"clean", "errors", "missing"}
STATUS_STATES = {"success", "failure", "pending", "missing"}
DEPLOYMENT_STATES = {"success", "failure", "in_progress", "missing"}


FULL_SHA = re.compile(r"[0-9a-fA-F]{40}")


class Malformed(Exception):
    pass


def require(condition, message):
    if not condition:
        raise Malformed(message)


def read_text(obj, key, where):
    value = obj.get(key)
    require(isinstance(value, str) and value, f"{where}.{key} must be a non-empty string")
    return value


def read_signal(signals, name, states):
    signal = signals.get(name)
    require(isinstance(signal, dict), f"signals.{name} must be an object")
    state = signal.get("state")
    require(isinstance(state, str) and state in states, f"signals.{name}.state must be one of {sorted(states)}")
    require(isinstance(signal.get("url", ""), str), f"signals.{name}.url must be a string")
    head = signal.get("headSha")
    require(
        head is None or head == "" or (isinstance(head, str) and FULL_SHA.fullmatch(head)),
        f"signals.{name}.headSha must be a 40-hex commit, an empty string or null",
    )
    return signal


def read_issues(signals):
    issues = signals.get("issues", [])
    require(isinstance(issues, list), "signals.issues must be a list")
    for issue in issues:
        require(
            isinstance(issue, dict) and isinstance(issue.get("title"), str) and isinstance(issue.get("url"), str),
            "each signals.issues entry needs string title and url",
        )
    return issues


def effective_state(signal, sha):
    """A signal for another commit than the release counts as missing."""
    head = signal.get("headSha")
    if head and head.lower() != sha.lower():
        return "missing", f"headSha {head} differs from {sha}"
    return signal["state"], None


def verdict(states):
    """landed: both good; not landed: neither good; mixed: exactly one.

    The Deployment and the publish run are reported but never decide.
    """
    compose_good = states["compose"] == "clean"
    status_good = states["status"] == "success"
    if compose_good and status_good:
        return "landed"
    if not (compose_good or status_good):
        return "not landed"
    return "mixed"


def evaluate(data):
    require(isinstance(data, dict), "input must be a JSON object")
    read_text(data, "project", "input")
    read_text(data, "release", "input")
    sha = read_text(data, "sha", "input")
    require(FULL_SHA.fullmatch(sha) is not None, "input.sha must be a 40-hex commit")
    signals = data.get("signals")
    require(isinstance(signals, dict), "signals must be an object")

    publish = read_signal(signals, "publish", PUBLISH_STATES)
    compose = read_signal(signals, "compose", COMPOSE_STATES)
    codes = compose.get("codes", [])
    require(
        isinstance(codes, list) and all(isinstance(c, str) for c in codes),
        "signals.compose.codes must be a list of strings",
    )
    status = read_signal(signals, "status", STATUS_STATES)
    deployment = read_signal(signals, "deployment", DEPLOYMENT_STATES)
    issues = read_issues(signals)

    rows = [("publish", publish), ("compose", compose), ("status", status), ("deployment", deployment)]
    states, notes = {}, {}
    for name, signal in rows:
        states[name], notes[name] = effective_state(signal, sha)
    return rows, states, notes, codes, issues


def render(rows, states, notes, codes, issues, result):
    lines = [f"VERDICT: {result}"]
    for name, signal in rows:
        line = f"{name}: {states[name]}  {signal.get('url', '')}"
        if notes[name]:
            line += f"  (counted as missing: {notes[name]})"
        lines.append(line)
        if name == "compose":
            lines.append(f"  codes: {', '.join(codes) if codes else 'none'}")
    if issues:
        lines.append(f"issues: {len(issues)} open")
        lines.extend(f"  {issue['title']}  {issue['url']}" for issue in issues)
    else:
        lines.append("issues: none open")
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--signals", metavar="FILE", help="read the JSON from FILE instead of stdin")
    args = parser.parse_args()
    try:
        if args.signals:
            with open(args.signals, encoding="utf-8") as handle:
                data = json.load(handle)
        else:
            data = json.load(sys.stdin)
        rows, states, notes, codes, issues = evaluate(data)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError, Malformed) as error:
        print(f"verdict.py: malformed input: {error}", file=sys.stderr)
        return 2
    print(render(rows, states, notes, codes, issues, verdict(states)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
