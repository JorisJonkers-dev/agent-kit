# A fourth hook guards where clones and worktrees go

Agents on the workstation cloned repositories wherever their working directory
happened to be, and made worktrees in `.worktrees/`, `.claude/worktrees/` or
next to the repository. So one repository had several copies, and an agent
could not find a worktree another agent had made. The owner chose one layout
for every agent:

- a clone at `~/Projects/<owner>/<repo>`, where `<owner>` is the GitHub org or
  user with its exact case;
- a worktree at `~/Projects/<owner>/worktrees/<repo>/<name>`.

A rule in each agent's instructions is advisory. A model that has started a
task in `/tmp` clones there. A hook that runs before the tool call can stop
it. ADR-0002 allows exactly three hooks and says any other hook needs its own
ADR. This is that ADR, for one hook: `projects-layout-guard`.

## The hook

| Name | Event | Matcher | Runs before | Why it exists |
| :- | :- | :- | :- | :- |
| `projects-layout-guard` | `PreToolUse` | `Bash\|EnterWorktree` in Claude Code, `Bash` in Codex | `git clone`, `gh repo clone`, `git worktree add`, and Claude Code's `EnterWorktree` | Denies a clone of a GitHub repository to any path except `~/Projects/<owner>/<repo>`, a worktree outside `~/Projects/<owner>/worktrees/<repo>/`, and `EnterWorktree` with `name` (which creates `.claude/worktrees/`). |

It lives in the `projects-layout` first-party skill. That skill's `SKILL.md`
states the layout, and setup copies it into every agent home. Setup also
writes a short form of the rule as a marked `projects-layout` block into the
global instruction files of Claude Code, Codex, Hermes and OpenCode, the same
way it writes the model-gateway pointer.

## Rules it follows

It follows every rule of ADR-0002, except the opt-in rule:

- **On for every repository.** The critique hooks do nothing outside an
  opted-in repository. This guard applies to every clone and worktree,
  because that is the point of the layout. It does nothing when `~/Projects`
  does not exist (a cloud session, a machine that does not use the layout).
- **No model and no network.** It reads the command, the payload's `cwd` and
  `git rev-parse` / `git remote get-url` of the repository named there. A bare
  `gh repo clone <repo>` would need the signed-in user from the network, so
  the guard allows it.
- **Well under one second.** Each `git` call has a 2-second timeout, and the
  hook is registered with the same 5-second `timeout` as the critique hooks.
- **Fail open on its own errors.** An unparsable command, a repository it
  cannot identify, a missing `python3` or a crash allows the call.
- **Fail closed only on an explicit decision.** It denies by printing the JSON
  `deny` and exiting 0. The reason is one line that names the path the clone
  or worktree must use.
- **Kill switch.** `PROJECTS_LAYOUT_OFF=1` in the environment makes it a no-op.
- **Merged by name**, with the critique hooks, into Claude Code's
  `settings.json` and Codex's `hooks.json`, and removed by `--uninstall`.

## Consequences

- `manifest.yaml` declares `projects-layout-guard` with this ADR, its
  `matcher` and `skill: projects-layout`. `scripts/validate_manifest.py`
  accepts this name with this ADR, and the critique names with ADR-0002.
- A hook entry can name the skill whose `hooks/` holds its script. The
  critique hooks keep the default, `critique`.
- Codex runs the guard only once the owner trusts it in `/hooks`, as with the
  critique hooks.
- OpenCode, Vibe, Hermes and Antigravity get the written rule and no guard,
  until someone checks their hook events.
- Claude Code subagents started with `isolation: worktree` make their worktree
  without a tool call the hook sees. The written rule cannot change that.
