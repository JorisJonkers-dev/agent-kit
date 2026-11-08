---
name: projects-layout
description: Where clones and git worktrees live on a workstation, ~/Projects/<owner>/<repo> and ~/Projects/<owner>/worktrees/<repo>/<name>. Use before cloning a repository or creating a worktree.
---

# projects-layout

Every agent on a workstation (Claude Code, Codex, OpenCode, Vibe, Hermes,
Antigravity) keeps repositories in one layout, so any agent finds a clone where
another left it.

## Clones

A GitHub repository lives at `~/Projects/<owner>/<repo>`. `<owner>` is the org
or user name with its exact case: `~/Projects/JorisJonkers-dev/agent-kit`.

- Look for `~/Projects/<owner>/<repo>` first. If it exists, `git fetch` there.
  Do not clone a second copy, not even into `/tmp`.
- Clone over SSH: `git clone git@github.com:<owner>/<repo>.git ~/Projects/<owner>/<repo>`.
- Never clone directly into `~/Projects/`, and never inside another repository.

## Worktrees

A worktree lives at `~/Projects/<owner>/worktrees/<repo>/<name>`. `<name>` is
the branch with `/` replaced by `-`, or a short task name.

```sh
git -C ~/Projects/<owner>/<repo> worktree add ~/Projects/<owner>/worktrees/<repo>/<name> -b <branch>
```

- Never inside the repository: no `.worktrees/`, no `.claude/worktrees/`, no
  `../<repo>-feature`.
- Claude Code: do not call `EnterWorktree` with `name`, which creates
  `.claude/worktrees/`. Create the worktree with the command above, then call
  `EnterWorktree` with `path`.
- Remove it with `git -C ~/Projects/<owner>/<repo> worktree remove <path>`.
- After moving a repository or a worktree, run `git worktree repair` in the
  main repository.

## The guard

`hooks/projects-layout-guard.sh` runs before every shell command in Claude
Code and Codex ([ADR-0009](https://github.com/JorisJonkers-dev/agent-kit/blob/main/docs/adr/0009-projects-layout-guard.md)).
It denies `git clone`, `gh repo clone` and `git worktree add` with a target
outside this layout, and Claude Code's `EnterWorktree` with `name`. It does
nothing when `~/Projects` does not exist or `PROJECTS_LAYOUT_OFF=1` is set.
