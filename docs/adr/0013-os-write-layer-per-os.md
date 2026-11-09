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
