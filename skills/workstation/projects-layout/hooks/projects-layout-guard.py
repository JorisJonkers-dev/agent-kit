#!/usr/bin/env python3
"""projects-layout-guard: the PreToolUse hook of docs/adr/0009-projects-layout-guard.md.

Denies `git clone`, `gh repo clone` and `git worktree add` when the target breaks the
workstation layout in ../SKILL.md:

  clones:    ~/Projects/<owner>/<repo>
  worktrees: ~/Projects/<owner>/worktrees/<repo>/<name>

Also denies Claude Code's EnterWorktree with `name`, which creates .claude/worktrees/.

It follows ADR-0002's rules for every hook: it denies by printing the JSON `deny` and
exiting 0, reads local state only (no network), and fails open: an unparsable command,
an unknown repository or any error of its own allows the call. It does nothing when
~/Projects does not exist or PROJECTS_LAYOUT_OFF=1 is set.
"""
import json
import os
import re
import shlex
import subprocess
import sys

HOME = os.path.expanduser("~")
PROJECTS = os.path.join(HOME, "Projects")
RULES = "See the projects-layout skill."

GITHUB_URL = re.compile(
    r"^(?:(?:https?|ssh|git)://(?:[^@/]+@)?github\.com/|(?:[^@\s]+@)?github\.com:)"
    r"(?P<owner>[^/\s]+)/(?P<repo>[^/\s]+?)(?:\.git)?/?$"
)
SHORT_SPEC = re.compile(r"^(?P<owner>[A-Za-z0-9_.-]+)/(?P<repo>[A-Za-z0-9_.-]+)$")


class Deny(Exception):
    pass


def block(msg):
    raise Deny(f"{msg} {RULES}")


def norm(path, cwd):
    path = os.path.expanduser(path)
    if not os.path.isabs(path):
        path = os.path.join(cwd, path)
    return os.path.normpath(path)



def segments(command):
    """Split a shell command into simple commands, tracking `cd` for the working dir."""
    try:
        lex = shlex.shlex(command, posix=True, punctuation_chars=";&|()")
        lex.whitespace_split = True
        tokens = list(lex)
    except ValueError:
        return []
    out, cur = [], []
    for t in tokens:
        if t and set(t) <= set(";&|()"):
            if cur:
                out.append(cur)
            cur = []
        else:
            cur.append(t)
    if cur:
        out.append(cur)
    return out


def strip_env(argv):
    while argv and re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", argv[0]):
        argv = argv[1:]
    while argv and argv[0] in ("command", "exec", "env", "nice", "time"):
        argv = argv[1:]
    return argv


def git_global_opts(argv, cwd):
    """Consume git's global options (-C, -c, ...). Return (rest, cwd)."""
    i = 1
    while i < len(argv):
        a = argv[i]
        if a == "-C" and i + 1 < len(argv):
            cwd = norm(argv[i + 1], cwd)
            i += 2
        elif a in ("-c", "--git-dir", "--work-tree", "--namespace") and i + 1 < len(argv):
            i += 2
        elif a.startswith("-"):
            i += 1
        else:
            break
    return argv[i:], cwd


def positional(args, opts_with_value):
    pos, i = [], 0
    while i < len(args):
        a = args[i]
        if a == "--":
            pos.extend(args[i + 1:])
            break
        if a.startswith("-"):
            name = a.split("=", 1)[0]
            if name in opts_with_value and "=" not in a:
                i += 1
        else:
            pos.append(a)
        i += 1
    return pos


def check_clone_target(owner, repo, target):
    want = os.path.join(PROJECTS, owner, repo)
    if os.path.normcase(target) != os.path.normcase(want):
        block(f"clone {owner}/{repo} into {want}, not {target}. "
              f"If {want} already exists, use that clone.")


def check_git_clone(args, cwd):
    pos = positional(args, {"-o", "--origin", "-b", "--branch", "-u", "--upload-pack",
                            "--reference", "--reference-if-able", "--separate-git-dir",
                            "--depth", "--shallow-since", "--shallow-exclude", "-c",
                            "--config", "-j", "--jobs", "--template", "--filter",
                            "--server-option", "--bundle-uri"})
    if not pos:
        return
    m = GITHUB_URL.match(pos[0])
    if not m:
        return
    owner, repo = m["owner"], m["repo"]
    target = norm(pos[1], cwd) if len(pos) > 1 else os.path.join(cwd, repo)
    check_clone_target(owner, repo, target)


def check_gh_clone(args, cwd):
    if "--" in args:
        args = args[:args.index("--")]
    pos = positional(args, {"-u", "--upstream-remote-name"})
    if not pos:
        return
    spec = pos[0]
    m = GITHUB_URL.match(spec) or SHORT_SPEC.match(spec)
    if m:
        owner, repo = m["owner"], m["repo"]
    else:
        # A bare repo name means the signed-in user's repo: finding that user
        # takes the network, so the guard lets it through.
        return
    repo = re.sub(r"\.git$", "", repo)
    target = norm(pos[1], cwd) if len(pos) > 1 else os.path.join(cwd, repo)
    check_clone_target(owner, repo, target)


def repo_identity(cwd):
    """Return (owner, repo) of the repository that contains cwd, from its main worktree path."""
    try:
        common = subprocess.run(["git", "-C", cwd, "rev-parse", "--path-format=absolute",
                                 "--git-common-dir"], capture_output=True, text=True,
                                timeout=2).stdout.strip()
    except Exception:
        return None
    if not common:
        return None
    main = os.path.dirname(common) if os.path.basename(common) == ".git" else common
    rel = os.path.relpath(main, PROJECTS)
    parts = rel.split(os.sep)
    if len(parts) == 2 and not rel.startswith(".."):
        return parts[0], parts[1]
    # Repo outside the layout: fall back to the origin URL.
    try:
        url = subprocess.run(["git", "-C", cwd, "remote", "get-url", "origin"],
                             capture_output=True, text=True, timeout=2).stdout.strip()
    except Exception:
        return None
    m = GITHUB_URL.match(url)
    return (m["owner"], m["repo"]) if m else None


def check_worktree_add(args, cwd):
    pos = positional(args, {"-b", "-B", "--reason", "--orphan"})
    if not pos:
        return
    ident = repo_identity(cwd)
    if not ident:
        return
    owner, repo = ident
    target = norm(pos[0], cwd)
    base = os.path.join(PROJECTS, owner, "worktrees", repo)
    if os.path.dirname(target) != base:
        name = os.path.basename(target) or "<name>"
        block(f"worktrees of {owner}/{repo} go in {base}/<name> "
              f"(for example {base}/{name}), not {target}.")


def check_bash(command, cwd):
    for argv in segments(command):
        argv = strip_env(argv)
        if not argv:
            continue
        if argv[0] == "cd":
            if len(argv) > 1 and argv[1] != "-":
                cwd = norm(argv[1], cwd)
            elif len(argv) == 1:
                cwd = HOME
            continue
        if os.path.basename(argv[0]) in ("bash", "sh", "zsh") and len(argv) > 2 \
                and argv[1] in ("-c", "-lc", "-ic"):
            check_bash(argv[2], cwd)
            continue
        if os.path.basename(argv[0]) == "git":
            rest, gcwd = git_global_opts(argv, cwd)
            if rest[:1] == ["clone"]:
                check_git_clone(rest[1:], gcwd)
            elif rest[:2] == ["worktree", "add"]:
                check_worktree_add(rest[2:], gcwd)
        elif os.path.basename(argv[0]) == "gh" and argv[1:3] == ["repo", "clone"]:
            check_gh_clone(argv[3:], cwd)


def decide(data):
    tool = data.get("tool_name", "")
    tin = data.get("tool_input") or {}
    cwd = data.get("cwd") or os.getcwd()
    if tool == "EnterWorktree":
        if not tin.get("path"):
            block("EnterWorktree with `name` creates .claude/worktrees/ inside the repo. Run "
                  "`git worktree add ~/Projects/<owner>/worktrees/<repo>/<name> -b <branch>` "
                  "from the main repository, then call EnterWorktree with `path`.")
        return
    command = tin.get("command")
    if isinstance(command, list):
        command = " ".join(shlex.quote(c) for c in command)
    if isinstance(command, str):
        check_bash(command, cwd)


def main():
    if os.environ.get("PROJECTS_LAYOUT_OFF") == "1" or not os.path.isdir(PROJECTS):
        return
    try:
        decide(json.load(sys.stdin))
    except Deny as deny:
        print(json.dumps({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": f"projects-layout-guard: {deny}",
        }}))
    except Exception:
        # Fail open: a bug in the guard never blocks work.
        return


if __name__ == "__main__":
    main()
