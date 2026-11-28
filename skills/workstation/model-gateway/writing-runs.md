## Writing runs

`"mode": "write"` hands a code change to a client model. It takes `repo`,
`base` (a ref) and `check` (a command string) besides the usual input; a run
without `repo` is `write-needs-repository`, and one without `base` or `check`
is `invalid-input`. `base` and `check` on a read-only run are `invalid-input`
too. The data policy is the same as for any run that reads a repository: the
entry must allow the model's family and `code`.

```sh
model-gateway run --json '{"workflow":"spec-293","client":"claude","model":"claude-sonnet-5-5","mode":"write","prompt":"Add the --verbose flag","repo":"/path/to/repo","base":"main","check":"npm test"}'
```

1. The gateway creates a worktree off `base` at
   `<git common dir>/model-gateway/worktrees/<run_id>`, on a new branch
   `gateway/<run_id>` (`git worktree add -b`). Whatever the model's read-only
   proof, a writing run works there and never in the repository itself.
2. The delegate runs in the worktree with write permission: Vibe through the
   `delegate-write` agent (`read_file`, `grep`, `write_file` and `edit`, no
   shell, nothing outside the worktree, no subagents; setup installs it beside
   `delegate-readonly`), Claude Code with `--permission-mode acceptEdits` (and
   `--allowedTools Read,Grep,Glob,Edit,Write`), Antigravity with `--mode
   accept-edits`, OpenCode with its edit tools and no shell, Codex with `-s
   workspace-write`. The prompt adds "Edit only
   files inside the working directory. Do not commit." A Codex writing run
   differs: see "Codex worktree workers" below.
3. When the delegate has finished, the gateway stages everything in the
   worktree with `git add -A` and lists what that staged, and only then runs
   `check` there with `sh -c` (held to `time_limit_s`; its output, stdout then
   stderr, goes to `check-output.txt`; a check that cannot start has exit code
   127). It never stages again, so whatever the check writes (coverage, a
   build, a rewritten lockfile) is not part of the delegate's diff and stays an
   unstaged change in the worktree. The gateway never commits, merges or
   pushes itself (a Codex worker may commit inside its worktree, above): the
   worktree and the branch are the caller's to take.

**Codex worktree workers.** A Codex writing run in a worktree can commit. The
worktree's own git directory lies outside the Codex sandbox, so the gateway
adds it with `--add-dir`, together with the repository's `objects`, `refs` and
`logs` directories (never the common directory itself, so `hooks` and `config`
stay read-only). The prompt then tells the worker to commit on its branch with a
Conventional Commits message and never push, and starts with the repository's
own brief when the worktree holds a `codex-brief.md` at its root (cut at 20000
characters). `staged_files` lists everything changed since `base`, committed or
not. When the git directory cannot be resolved the run falls back to the old
behaviour: nothing is committed and the gateway stages what is left.

The result is the read-only result and `worktree` (the path), `branch`,
`staged_files` (relative to the worktree) and `check`
`{"command","exit_code","output_path"}`. `status` is `ok` only when the
delegate finished and the check exited 0; a failing check is `failed` with
`reason: "check-failed"`, and its worktree is kept and staged all the same. A
run whose delegate did not finish (`timeout`, `exit-<n>`, `parse-error`, ...)
leaves its worktree unstaged and kept, runs no check and has no `check` in its
result. If the gateway itself fails after the delegate ran (git cannot stage the
worktree, git cannot list what it staged, or the check's output cannot be
written), the run is `failed` with `reason: "stage-failed"`,
`"stage-list-failed"` or `"check-output-failed"`, the worktree and branch in
the result and git's or the file system's message in `worktree_error`. A run
whose staged files git could not list says so with `staged_files: null` rather
than an empty list, which would read as a delegate that changed nothing.

Every writing worktree is entered in `kept-worktrees.json` in the gateway's
state directory (`$XDG_STATE_HOME/agent-kit/model-gateway/`, else
`~/.local/state/...`), whatever the run came to, and stays until the workflow
that made it ends. `workflow end` ends it; `worktrees clean` then removes what
it kept:

```sh
model-gateway workflow end --json '{"name":"spec-293"}'
model-gateway worktrees clean --json '{"workflow":"spec-293"}'
model-gateway worktrees clean --json '{"all":true}'
```

It removes each kept worktree of that workflow (`git worktree remove --force`)
and its branch, and prints `{"status":"ok","removed":[...],"failed":[...],"branch_kept":[...]}`;
a worktree it cannot remove stays on the list with the reason, for the next
call. Once the worktree is removed the entry is gone, and a branch git would not
delete (checked out elsewhere, or already gone) is named in `branch_kept` with
git's message.
Take what you want from a worktree (commit its staged diff on a branch of your
own) before the workflow ends: cleaning discards it.

