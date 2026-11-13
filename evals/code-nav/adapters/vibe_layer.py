#!/usr/bin/env python3
"""A code-nav-eval generic adapter: one question through Vibe, at one layer.

usage (as the runner's --command template): python3 -I vibe_layer.py {model}
{model} is "<vibe model alias>@<layer>". The question arrives in
CODE_NAV_PROMPT; the runner's working directory is the snapshot root.
Prints the one JSON object the runner's generic adapter reads.

Layers are cumulative:
  L0  baseline: file reading and the shell over the repository
  L1  L0 plus the repository's AGENTS.md put in front of the question
  L2  L1 plus code-nav search, outline and expand (cn.py, through the shell)
  L3  L2 plus code-nav map
  L4  L3 plus code-nav symbols and related_repos
Environment: LAYER_WORK (scratch dir), ONLY_REPO (the one repository the agent
may see; its snapshot is copied under LAYER_WORK/scope), CN_BIN, AGENTS_FILE.
"""

import json
import os
import shutil
import subprocess
import sys

NAV = {
    "L2": ["search", "outline", "expand"],
    "L3": ["search", "outline", "expand", "map"],
    "L4": ["search", "outline", "expand", "map", "symbols", "related_repos"],
}
HELP = {
    "search": "search <query> [path]: lines containing the query, ranked",
    "outline": "outline <path>: the symbols a file declares, with line ranges",
    "expand": "expand <path> <symbol>: one symbol's source without the rest of the file",
    "map": "map [path]: files and symbols ranked by how often they are referenced",
    "symbols": "symbols <name>: where a symbol is declared across repositories",
    "related_repos": "related_repos <repo>: what a repository depends on and what depends on it",
}


def main(model_arg):
    alias, _, layer = model_arg.partition("@")
    layer = layer or "L0"
    repo = os.environ["ONLY_REPO"]
    work = os.environ["LAYER_WORK"]
    scope = os.path.join(work, "scope")
    if not os.path.isdir(os.path.join(scope, repo)):
        shutil.copytree(os.path.join(os.getcwd(), repo), os.path.join(scope, repo))
    prompt = os.environ["CODE_NAV_PROMPT"]
    n = int(layer[1:])
    if n >= 1:
        with open(os.environ["AGENTS_FILE"], encoding="utf-8") as f:
            prompt = f"The repository's AGENTS.md:\n\n{f.read()}\n\n---\n\n{prompt}"
    # Vibe's file_system tools (read, grep, shell) come as one group, so every
    # layer has a shell; a layer differs by what it is told and given.
    tools = ["read_file", "bash"]
    env = dict(os.environ)
    prompt = f"Work only inside {scope}; do not read anything outside it.\n\n" + prompt
    if layer in NAV:
        env.update(CN_ROOT=scope, CN_ALLOW=",".join(NAV[layer]))
        cn = os.path.join(os.path.dirname(os.path.abspath(__file__)), "cn.py")
        lines = "\n".join(f"  python3 -I {cn} {HELP[t]}" for t in NAV[layer])
        prompt = (
            "These read-only code-navigation commands are also available through the shell "
            f"(paths are relative to {scope}):\n{lines}\n\n" + prompt
        )
    out_dir = os.path.join(work, "export", f"{os.getpid()}")
    cmd = [
        "vibe",
        "-p",
        prompt,
        "--model",
        alias,
        "--output",
        "json",
        "--max-turns",
        "15",
        "--workdir",
        scope,
        "--trust",
        "--output-dir",
        out_dir,
    ]
    for t in tools:
        cmd += ["--enabled-tools", t]
    try:
        p = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=600)
    except subprocess.TimeoutExpired:
        print("vibe timed out after 600 s", file=sys.stderr)
        return 1
    # A run that hits the turn limit exits non-zero but still prints its journal:
    # it is graded wrong, not dropped as an error. Output with no journal at all
    # is an error.
    if "[" not in p.stdout:
        print(p.stderr.strip()[-400:], file=sys.stderr)
        return p.returncode or 1
    start = p.stdout.index("[")
    entries = json.loads(p.stdout[start:])
    answer = ""
    calls = 0
    opened = []
    for e in entries:
        if e.get("type") == "message" and e.get("role") == "assistant":
            answer = "".join(c.get("text", "") for c in e.get("content", []))
        elif e.get("type") == "effect":
            calls += 1
            inp = (e.get("detail") or {}).get("input") or {}
            for key in ("filePath", "path", "command"):
                v = inp.get(key)
                if isinstance(v, str):
                    opened.append(v.replace(scope + "/", "").lstrip("/"))
    with open(os.path.join(out_dir, "export.json"), encoding="utf-8") as f:
        export = json.load(f)
    usage = export.get("usage") or {}
    with open(os.path.join(work, "cost.jsonl"), "a", encoding="utf-8") as f:
        f.write(json.dumps({"model": model_arg, "cost_usd": export.get("cost_usd"), "usage": usage}) + "\n")
    print(
        json.dumps(
            {
                "answer": answer,
                "input_tokens": usage.get("input_tokens", 0),
                "output_tokens": usage.get("output_tokens", 0),
                "cache_read_tokens": usage.get("cached_input_tokens", 0),
                "tool_calls": calls,
                "files_opened": opened,
            }
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
