# The operating-system write limit has one backend per OS

A reach level is enforced by two layers: tool configuration, and an
operating-system limit on where an agent may write. The second layer exists
because some agents cannot be restricted through their tools (Hermes's
toolset is fixed over ACP, Pi has no permission prompts, and no tool list
stops a shell command from writing). The earlier design used only
`sandbox-exec`, which exists only on macOS. Gold Band now also runs on the
Linux workstation (ADR-0011).

So the launcher has one write-limit interface with two backends: Landlock on
Linux and `sandbox-exec` on macOS. Landlock is in the kernel, needs no
privileges and no extra binary, and Rust can call it directly, which fits the
Rust-only rule for new code. When the backend for the current OS is not
available, the launcher refuses to start the agent. It never runs the agent
without the limit.

## What each reach level may write

Every level may write the agent's own state directories and a temporary
directory for its run. Beyond that:

| Reach level | May also write |
| :- | :- |
| `read`, `read-web` | nothing |
| `research`, `write-scoped` | only its declared output directory |
| `write` | the repository's working tree (its worktree), but not `.git/hooks` or `.git/config` |
| `run-one-command` | only its gate-output directory |
| `full` | anything, and only ever in a throwaway repository |

Keeping `.git/hooks` and `.git/config` out of `write` stops an agent from
switching off the commit and push hooks that guard every repository.

## What the layer does not limit

The limit is a write limit. It does not limit reads, except one: the personal
instance on the work Mac. There the account boundary (ADR-0011, #396) is the
first defence and the launcher adds a second, an operating-system **read**
limit (#666).

- **What.** Every launch of a personal-instance agent (any agent but
  `claude-work` and `codex`) on the work Mac starts under a `sandbox-exec`
  profile that denies `file-read*` and `file-write*` under each employer root:
  a read, a listing, a stat, a hard link or a move out of the root fails with
  the operating system's error, from a shell command or a read tool alike. It
  applies at every reach level, `full` included (where the profile holds only
  this denial). The agent's own repository, the toolchain and its state
  directories stay readable. A run whose own directory (worktree, temporary or
  granted directory) lies inside an employer root is refused.
- **Where the roots come from.** One declared place, the work overlay
  (`~/.config/agent-kit/overlay.yaml`): `instances.gold-band.employer_roots`,
  absolute or `~/` paths (`~/Projects/nedap` is the owner's value, not built
  into the launcher). An overlay that names the work instance without roots,
  or that cannot be read, refuses the launch instead of reading as "no roots".
- **Refusal.** When the backend cannot apply the read limit, the launcher
  refuses to start the agent, and setup runs `agent-launcher read-limit-check`
  and stops before converging the personal instance, saying why.
- **Linux.** The workstation holds no employer clones (ADR-0011) and needs no
  read limit. Landlock only grants access and cannot carve a read out of what
  the agent already has, so a declared root there is refused, not ignored.
- **Proof.** `tests/personal_read_limit.rs` runs on the `macos` CI job.

The work instance is not limited. Neither backend limits the network
either.

Neither backend limits the network, because every agent needs the network to
reach its model. Web limits stay tool-configuration and MCP-list limits only.

## Considered options

- **bubblewrap on Linux.** Stronger isolation through mount namespaces, but an
  extra setuid or user-namespace binary that has to be packaged on NixOS, and
  more than a write limit needs.
- **Containers on both OSes.** Uniform, but heavy, and on macOS it means a
  virtual machine for every agent launch.
- **Keep `sandbox-exec` only.** It leaves Linux with no second layer. Apple
  has also deprecated `sandbox-exec`, so the macOS backend may need
  replacing later.
