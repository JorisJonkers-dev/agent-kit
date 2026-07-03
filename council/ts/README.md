# Council TypeScript

Standalone TypeScript package scaffold for the council orchestrator.

The build script emits the Node entrypoint to `../council.mjs` as a single-file
esbuild bundle with a Node shebang.

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
