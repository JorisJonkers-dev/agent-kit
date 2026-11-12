#!/usr/bin/env python3
"""Print a LiteLLM virtual key's budget window and reset, as the real proxy reports them.

One GET /key/info with the agent id's key: LiteLLM answers with the key's
``max_budget``, ``budget_duration``, ``budget_reset_at`` and ``spend``. It is
not a model call, so it costs nothing. The output is those four fields and the
request that asked, with both credentials redacted; nothing else of the reply
and no key value is ever printed.

The key and the edge token come from the environment or, when unset, from the
one secret store (``installer/agent-kit-secret get``), the place setup put them.
The edge wants the auth-api service token as the bearer and the virtual key in
``x-litellm-api-key`` (the proxy's own header, see docs/LITELLM.md).

Usage::

    python3 scripts/measure_litellm_budget.py [--agent vibe] [--url https://litellm.jorisjonkers.dev]

Exit status: 0 the fields printed, 1 the proxy refused or answered unusably,
2 bad usage or a missing credential. Stdlib only.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Mapping, Sequence
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ROUTES = ROOT / "registry" / "generated" / "litellm" / "routes.json"
SECRET = ROOT / "installer" / "agent-kit-secret"
FIELDS = ("max_budget", "budget_duration", "budget_reset_at", "spend")
SAFE = re.compile(r"^[A-Za-z0-9._~+/=-]+$")
AGENT = "agent-kit-measure-litellm-budget/1"
TIMEOUT_S = 20


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    """Never follow a redirect: it would carry both credentials to another host."""

    def redirect_request(self, *args: object, **kwargs: object) -> None:
        return None  # pragma: no cover


class Failure(Exception):
    def __init__(self, message: str, code: int) -> None:
        super().__init__(message)
        self.code = code


def route_for(agent: str, routes: Path = ROUTES) -> dict[str, object]:
    table = json.loads(routes.read_text())
    for route in table["routes"]:
        if route["agent"] == agent:
            return {"key_env": route["key_env"], "edge_env": table["edge_token_env"],
                    "header": table["key_header"], "base_url": table["base_url"]}
    raise Failure(f"no route for agent {agent!r}; known: {', '.join(r['agent'] for r in table['routes'])}", 2)


def credential(name: str, env: Mapping[str, str], secret: Path = SECRET) -> str:
    """The value from the environment, else the secret store; never printed."""
    value = env.get(name, "")
    if not value:
        try:
            done = subprocess.run([sys.executable, str(secret), "get", name],
                                  capture_output=True, text=True, check=False, timeout=30)
            value = done.stdout.strip() if done.returncode == 0 else ""
        except subprocess.TimeoutExpired:
            value = ""
    if not value:
        raise Failure(f"{name} is not set and not in the secret store; run setup and paste it", 2)
    if not SAFE.match(value):
        raise Failure(f"{name} has characters a header cannot carry safely", 2)
    return value


def endpoint(url: str) -> str:
    """The proxy root's /key/info, https only (plain http on loopback, for a stub)."""
    parts = urllib.parse.urlsplit(url)
    if parts.username or parts.password:
        raise Failure("the proxy url must not carry credentials", 2)
    local = parts.hostname in ("127.0.0.1", "localhost", "::1")
    if parts.scheme != "https" and not (parts.scheme == "http" and local):
        raise Failure("the proxy url must be https", 2)
    path = parts.path.rstrip("/")
    path = path.removesuffix("/v1")
    return urllib.parse.urlunsplit((parts.scheme, parts.netloc, path + "/key/info", "", ""))


def measure(agent: str, url: str | None, env: Mapping[str, str], routes: Path = ROUTES,
            secret: Path = SECRET) -> tuple[str, dict[str, object]]:
    route = route_for(agent, routes)
    target = endpoint(url or str(route["base_url"]))
    key = credential(str(route["key_env"]), env, secret)
    edge = credential(str(route["edge_env"]), env, secret)
    request = urllib.request.Request(target, method="GET", headers={
        "Authorization": f"Bearer {edge}", str(route["header"]): f"Bearer {key}", "User-Agent": AGENT,
    })
    shown = (f"GET {target}\n  Authorization: Bearer <{route['edge_env']}, redacted>\n"
             f"  {route['header']}: Bearer <{route['key_env']}, redacted>")
    try:
        with urllib.request.build_opener(_NoRedirect).open(request, timeout=TIMEOUT_S) as response:
            body = json.loads(response.read())
    except urllib.error.HTTPError as e:
        raise Failure(f"{shown}\nthe proxy answered HTTP {e.code}", 1) from None
    except (urllib.error.URLError, OSError, ValueError) as e:
        raise Failure(f"{shown}\nno usable answer from the proxy ({type(e).__name__})", 1) from None
    info = body.get("info") if isinstance(body, dict) else None
    if not isinstance(info, dict):
        raise Failure(f"{shown}\nthe proxy's reply has no key info", 1)
    return shown, {name: info.get(name) for name in FIELDS}


def main(argv: Sequence[str] | None = None, env: Mapping[str, str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--agent", default="vibe", help="the agent id whose key to measure (default vibe)")
    parser.add_argument("--url", help="the proxy base url (default: the registry's route table)")
    args = parser.parse_args(argv)
    try:
        shown, fields = measure(args.agent, args.url, os.environ if env is None else env)
    except Failure as e:
        print(f"measure-litellm-budget: {e}", file=sys.stderr)
        return e.code
    print(f"request:\n  {shown}")
    print("budget:")
    for name, value in fields.items():
        print(f"  {name}: {json.dumps(value)}")
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
