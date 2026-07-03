# Council TypeScript

Standalone TypeScript package scaffold for the council orchestrator.

The build script emits the Node entrypoint to `../council.mjs` as a single-file
esbuild bundle with a Node shebang.

## Native DAG Execution Contract

`fanout` and `fleet` are plan-only by default. Without `--execute`, they load the
run or task list, apply the static pre-fanout gate, compute waves, and optionally
plan GitHub output, but they do not create worktrees, start workers, run verify
commands, merge branches, or run eval.

Use `--execute` to opt into the native DAG executor:

```sh
council fanout --run /runs/run-42 --execute \
  --base-ref origin/integration \
  --integration-branch integration/run-42 \
  --concurrency 2
```

```sh
council fleet --tasks /runs/run-42/tasks.json --agents codex:gpt-5,claude:sonnet \
  --execute --base-ref origin/integration --integration-branch integration/run-42
```

Execution creates one `worker/<task-id>` branch and worktree per dispatched task,
supervises the assigned engine in that worktree, runs the task `verify` command,
commits changed in-bounds files, and reconciles successful task branches back into
the requested integration branch. `--base-ref` is the starting point used when the
integration branch must be created; it defaults to `HEAD`. `--concurrency` limits
how many ready DAG nodes can run at once and defaults to `1`.

`--dry-run` is still honored when `--execute` is present. In execute dry-run mode,
the executor returns planned branches and skipped task results with
`skipped_reason: "dry-run"`; it does not provision worktrees, start workers, run
verify commands, commit, or merge.

`--eval` requests an eval pass after executor completion and attaches the result
to `execution.eval` in the JSON output. The optional eval command label is
reported as metadata for callers; scoring is performed from the normalized run
artifacts for the same run directory. When `require_clean_boundaries` is enabled
by the caller, warning-level eval output is treated as a failed execution eval
result.

## Worktree Dependency Provisioning

`src/adapters/worktree-provisioning/index.ts` exposes an injectable
`WorktreeDependencyProvisionerPort`. The default factory returns the copy
strategy, which copies the repository root `node_modules` directory into a
worktree. Callers can also select the explicit no-op strategy or inject a custom
provisioner for tests and future runtime composition.

Copy is the default because a worktree-local `node_modules` symlink is easy to
make self-referential when a worker operates inside the same repository tree.
It also interacts poorly with `.gitignore` rules that use a trailing slash, such
as `node_modules/`: Git treats that pattern as a directory match, so a symlinked
`node_modules` entry can fail to match as expected and appear as an untracked
path. The adapter therefore copies dependencies and never creates a top-level
`node_modules` symlink.

If the repository root has no `node_modules`, the copy strategy returns an
explicit skipped result with `reason: "source-missing"`. That makes missing
dependency provisioning visible to the caller without hiding it behind implicit
filesystem behavior.

During native execution, dependency provisioning runs after each worker worktree
is created and before the worker process is supervised. The default production
provisioner copies the repository root `node_modules` into the worker worktree so
verify commands and local CLIs can run without reinstalling dependencies. Tests
and specialized callers can inject a fake or no-op provisioner through the app
composition boundary; execution results remain explicit about provisioning
behavior through the provisioner return value.
