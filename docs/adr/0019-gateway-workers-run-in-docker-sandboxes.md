# Gateway workers run only in Docker Sandboxes

Decided 2026-11-26. Accepted and in force (#961): this supersedes ADR-0016 for every client the model gateway
launches, and the gateway no longer starts `agent-launcher confine` for any of them. ADR-0013 (the write layer
of interactive agents) is unchanged.

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
- **Network: a reach per run, from its profile** (section below). The LiteLLM route, the estate package proxy,
  and what the run's delegation profile adds; everything else is denied. A run may add hosts (`extras.network_allow`),
  and the reach it got is in its event log.
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

## Cleanup

A run's sandbox is removed by `sbx rm --force` however the run ended (success, failure, cancel, stall kill), and removing it frees everything the sandbox held: measured on sbx 0.54.1, `sbx df` reads the same before and after a run, with no containers and no volumes left. A gateway killed before it could remove its sandbox leaves an `mg-<pid>-<time>` sandbox, which the next launch removes when that pid is gone.

What does accumulate is the images. Each client's pinned digest changes about daily, and the images of earlier digests stay in sbx's store at roughly 0.8 GB each (more once layers are counted: `sbx df` showed 15 GB for ten images). So the gateway removes them:

- When a refresh (`sandbox refresh`, or the one a launch makes) records a digest, it runs `sbx template rm --force` on every image of the gateway's own repositories (`docker.io/sbx/<name>-image`, and the `sandbox-kit-assembled` copy sbx makes of the same image) whose digest no client pins. An image a running sandbox still mentions is kept. A failure is a warning, never a failed launch.
- `model-gateway sandbox prune [--json] [--dry-run] [--all]` does the same on demand, and also removes the `mg-*` sandboxes of ended gateways. It reports `freed_bytes` as the fall in `sbx df`'s image total (the listed sizes on a dry run). `--all` also removes images that are not the gateway's (earlier test images, `sbx/shell`) but never a pinned image, an image a sandbox uses, or a sandbox. With no readable records file it removes no image.
- Setup runs `model-gateway sandbox prune` after the image pre-pull.

Docker Desktop's own volumes and images (the VM disk that grew to 145 GB) are not sbx's and are not touched here.

## Network reach

Decided 2026-11-27 (#960). A sandbox's network reach is set per run from its delegation profile, not one allowlist for
every run. The table is the profile field `network` in `registry/delegation-profiles.yaml` (rendered into the shipped
defaults); a profile that states none, a tier profile and a run with no profile get `proxy-only`.

| `network` | Reaches | Profiles |
|---|---|---|
| `open-web` | any public host, plus the route and the package proxy | `research`, `cheap-research` |
| `packages` | the route, the package proxy, npm, PyPI, crates.io, the Go proxy, Docker Hub, ghcr, GitHub read, and the hosts the task names (`extras.network_allow`) | `implement`, `implement-cheap` |
| `proxy-only` | the route (the LiteLLM proxy) and the package proxy | `critique`, `red-team`, `refute`, `triage`, `cross-critic`, `public-gemini`, every tier profile |

Pi and OpenCode talk to their provider through the LiteLLM route, so the route's host is the client's own provider
for them; a client that talks to its provider directly adds that host when it joins the sbx backend. A repository
that is not public (no remote, `nedap`, private) keeps its policy: `open-web` is cut to `packages` for it, and the
run records the cut. `extras.network_allow` takes `host` or `host:port` (443 when none); a pattern is refused.

### What sbx 0.54.1 enforces (measured, not read from the docs)

Throwaway sandboxes on this machine, `sbx policy ls` holding the default `local-policy` (195 global allows) and the
default fallback deny; no global rule was changed.

- A per-sandbox `sbx policy allow network --sandbox <name> <hosts>` adds a host: `example.com:443` and
  `packages.jorisjonkers.dev:443` went from 403 to 200 and 401 (reached) for that sandbox only. `**` allows every
  host, per sandbox, as the help says (a bare `*` is rejected).
- A per-sandbox `sbx policy deny network --sandbox <name> <host>` beats a global allow: `github.com:443`, allowed
  globally, returned 403 inside the sandbox and stayed reachable elsewhere.
- A per-sandbox deny of `**` beats the sandbox's own allows too (every host, including the allowed ones, gave 403), so
  "deny everything, allow the route" cannot be written. A narrow reach is therefore made by denying, in one
  `sbx policy deny network --sandbox <name> <comma list>`, every global allow the reach does not need: all 195 went
  in one call (0.6 s), `registry.npmjs.org` left out stayed reachable, GitHub, crates.io and `example.com` gave 403,
  and `packages.jorisjonkers.dev` (allowed per sandbox) stayed reachable. The list is read from
  `sbx policy ls --json` at launch. Sandbox rules and secrets go with `sbx rm` (checked: nothing is left in
  `sbx secret ls` or the policy afterwards).
- What this cannot do: a global allow that is a wildcard over a host the run needs (`**.github.com` for GitHub read)
  cannot be partly denied, so it is kept and the run's recorded reach lists it as a wider global allow kept; a global
  allow of every host (`**`, after `sbx policy init allow-all`) cannot be narrowed at all, and a launch whose reach
  is narrower than the open web refuses with `sandbox-unsafe` rather than claim a restriction. A global allow the
  owner adds between the listing and the run is not denied for that run.
- Nothing in the owner's global sbx settings changes: setup does not edit `local-policy`, and the gateway only adds
  rules scoped to its own `mg-*` sandboxes.
- `sbx create --deny-network` exists but takes one rule per flag; the gateway uses the policy call, which takes a list.

### The estate package proxy, in every sandbox

Every sandbox, whatever its reach, allows `packages.jorisjonkers.dev:443` and gets the package managers pointed at it
the way estate CI is, through the run's env file: `GOPROXY`, `GOPRIVATE` (first-party modules stay direct),
`PIP_INDEX_URL`, `UV_DEFAULT_INDEX`, `NPM_CONFIG_REGISTRY` with `NPM_CONFIG_USERCONFIG` naming the run's own
`packages.npmrc` (a file in the run's directory, never the owner's), and the Cargo registry
`jorisjonkers-packages` (`CARGO_REGISTRIES_JORISJONKERS_PACKAGES_INDEX`, source replacement for crates.io).

Its credential is the workstation key setup already stores (`PACKAGES_PROXY_TOKEN`, else
`${XDG_CONFIG_HOME:-~/.config}/jorisjonkers/packages-token`). It reaches the sandbox as an sbx custom secret
(`sbx secret set-custom --sandbox <name> --host packages.jorisjonkers.dev --placeholder <p> --value-stdin`, the key on
stdin). Measured: the sandbox sends the placeholder (in `Authorization: Bearer <p>` or in a Basic pair
`packages:<p>`), and the host proxy replaces it with the secret on the way out over HTTPS, base64 included (a test
server behind HTTPS saw `Bearer <secret>` and `Basic base64(packages:<secret>)`); over plain HTTP nothing is
replaced, which the package proxy never uses. The key is therefore in no file, argument or variable inside the
sandbox: only the placeholder, derived from the sandbox's name and worthless outside its secret. The Go and pip URLs
carry `packages:<placeholder>@`, npm and Cargo send the placeholder as their token. A machine with no key skips the
credential and the run records that it reaches the proxy anonymously. Cargo's environment settings follow the
documented names and are not exercised by a live build in the sandbox image, which carries no Rust toolchain.

### No stored service secrets

sbx puts a proxy-managed placeholder for every service secret it stores (`GH_TOKEN` for `github`) in every sandbox,
so a stored global service secret would reach every run whatever its reach. The launch reads
`sbx secret ls -g --json` before it makes anything and refuses with `sandbox-unsafe` while a service secret is stored,
unless the run's profile names it in `sbx_secrets`. A secret scoped to one sandbox, a registry credential and a custom
secret are not service secrets; a listing that cannot be read refuses too.

## Migration

Done (#961). `sbx` is the default backend: `AGENT_KIT_SANDBOX` unset or empty is `sbx`, and a machine without a
working sbx refuses each launch with `sandbox-unavailable` and the install command (`brew trust docker/tap && brew
install docker/tap/sbx`, then `sbx login`), before anything is made. `AGENT_KIT_SANDBOX=confine` is refused
(`sandbox-unknown`) because the backend was retired; `sbx-dry-run` stays as a diagnostic that prints the plan.
Pi, OpenCode, Vibe, Claude Code, Codex and Hermes run in sandboxes; Antigravity is refused
(`sandbox-unsupported`) until it has a sign-in a sandbox can hold, and is no longer started at all. The
gateway starts no `agent-launcher confine`: the grants are still computed per client recipe, but they are the
plan's input, carried by a plan command that is read and never run. ADR-0016 is marked superseded for gateway
workers; `agent-launcher confine` stays as a tool of its own (ADR-0013).

## Open

Facts the docs do not state and a machine with `sbx` must settle before the backend lands: whether stdout and
stderr stay separate without a terminal, startup time, git in a linked worktree, single-file mounts, and how a sandbox reaches a LiteLLM proxy on the host.
