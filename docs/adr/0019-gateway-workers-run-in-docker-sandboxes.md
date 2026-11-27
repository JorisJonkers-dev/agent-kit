# Gateway workers run only in Docker Sandboxes

Decided 2026-11-26. This supersedes ADR-0016 for every client the model gateway launches, once the sbx
backend lands; until then `agent-launcher confine` holds them as ADR-0016 describes. ADR-0013 (the write
layer of interactive agents) is unchanged.

## Why

`agent-launcher confine` holds a delegate with Seatbelt on macOS and Landlock on Linux. Each client needed a
recipe traced by hand (ADR-0016), some still run with gaps (the login keychain for Codex and Antigravity, a
writable OpenCode database), Seatbelt is deprecated by Apple, Landlock cannot hold OpenCode or Antigravity
(`--net-loopback`), and a host allowlist needs a proxy that was never built. Docker Sandboxes (`sbx`) runs
each sandbox as a microVM with its own kernel and Docker engine, mounts host folders at the same path
(`PATH` or `PATH:ro`), and sends all outbound traffic through a host-side proxy that allows by host name.
It runs on macOS 14+ on Apple silicon and on Ubuntu 24.04+ with KVM
(https://docs.docker.com/ai/sandboxes/install.md, read 2026-11-26), so one Linux image per client serves
both operating systems.

## Decision

- **sbx is required.** Every worker the gateway launches runs in a Docker Sandbox. A machine where `sbx` is
  missing, not signed in, or cannot start a microVM (no KVM, an Intel Mac) refuses the launch with
  `sandbox-unavailable` and the reason, before anything is made. There is no fallback to `confine` and never
  an unconfined run, on macOS or Linux.
- **One fresh sandbox per run**, named after the run and destroyed when it ends, on cancel, on a stall kill,
  and by the lost-run recovery. Worktree changes come back only through git: the worktree is mounted, and the
  gateway commits as today. Nothing else leaves the sandbox but the run directory (events, stderr, report).
- **Mounts**: the working directory read-only for a read-only run and written for a writing run, the git
  directories git needs read-only (the writing run's own written), the run directory and the run's home
  written. Never the owner's home, the main checkout, or another run's directory. The client's own install
  comes from its image (`agent-kit/<client>`, built locally by setup), not from the host.
- **Credentials: per run, minimal, read-only.** A launch gets the one key or login its harness and route need,
  as a variable or a read-only file, and nothing else of the home. A login that refreshes itself is a copy in
  the run's home, written back by the gateway outside the sandbox. A client whose login lives only in the
  macOS keychain (Antigravity today) is refused until it has another way to sign in.
- **Network: an allowlist by default.** Allowed: the LiteLLM proxy, the harness's own provider API, GitHub for
  reading, and the package registries; everything else is denied. A run may add hosts by flag, and the added
  hosts are recorded in its event log.
- **Docker inside the sandbox is on by default**, with the sandbox's own daemon. The host's Docker socket is
  never mounted and `DOCKER_HOST` is never passed in.
- **The grant set stays the policy.** What ADR-0016 grants (working directory, git view, run home, ports) is
  computed as today and translated into the sandbox's mounts, variables and network rules. The repository
  policy (public, `nedap`, private) and data scope are decided before the launch and are unchanged; a
  sandbox never widens them. Sandboxes run locally only; the cloud mode is never used.
- **Native harness sandboxes stay off** (Codex `danger-full-access`), as in ADR-0016: the microVM is the
  sandbox.
- **The event log is unchanged** (ADR-0017): the gateway runs the sandbox's command as the child process and
  pumps its output live, as it does today.

## Templates

A sandbox starts from Docker's prebuilt image for its client, `docker.io/sbx/<name>-image` (`pi`, `opencode`, `claude`, `codex`, `vibe` and `hermes-agent`), pinned by digest: the launch's `sbx create` names `docker.io/sbx/<name>-image@sha256:...` and passes `--pull missing`. The gateway builds no image of its own, so there is no Dockerfile or build script here; the images hold the client's own install and an `agent` user, and the gateway drives them with `sbx exec`.

Docker tags a build each day as `YYYYMMDD-<40-hex commit>` (and `latest`, which the gateway never uses). The gateway runs the newest dated tag at least two days old (`COOLDOWN_DAYS` in `sbx_images.rs`), so a build that is hours old never reaches a run, and the run pins that tag's digest. The choice is recorded per client in `sbx-images.json` under the gateway's state directory (`tag`, `digest`, `checked_at`). A launch refreshes a client's record before `sbx create` when the record is missing or older than a day, so a newer qualifying image reaches the machine without a setup run. A refresh that fails keeps the record with a warning; with no record at all the launch is refused (`sandbox-unavailable`) until `model-gateway sandbox refresh` succeeds. `model-gateway sandbox status` prints the records without the network.

A launch's variables reach the client through `sbx exec --env-file`: a file written 0600 into the launch's own directory, removed when the exec ends whether it succeeded or not, so no value appears in an argument list.

## Migration

`AGENT_KIT_SANDBOX=sbx-dry-run` (first slice) prepares a launch, translates its grants into the sandbox plan,
removes what it made and refuses with the plan, so the translation can be checked on any machine. The sbx
backend then lands per client (Pi and OpenCode first, then Vibe, then Claude Code, Codex and Hermes; Antigravity stays on confine), the default switches to sbx with the refusal above,
and the gateway's `confine` path (client recipes, `PiConfinement`, the dry run) is removed. ADR-0016 is then
marked superseded.

## Open

Facts the docs do not state and a machine with `sbx` must settle before the backend lands: whether stdout and
stderr stay separate without a terminal, startup time, git in a linked worktree, single-file mounts, whether
a network policy is per sandbox or global, and how a sandbox reaches a LiteLLM proxy on the host.
