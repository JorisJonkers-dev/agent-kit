# `agent-launcher confine` holds a read allowlist and a network limit on macOS

This extends ADR-0013, which gave macOS a write limit and no read allowlist.
`agent-launcher confine` (#737) limits a whole process to the paths it is
given, and until now only Landlock could hold that, so on a Mac every
confined launch (every Pi launch) was refused with exit 125 (#904). The Seatbelt
backend now holds it too. `sandbox-exec` is still deprecated by Apple and
still shipped; ADR-0013 already chose it, and nothing below needed more.

## The profile

A confinement profile starts from `(deny default)`, never `(allow default)`.
Everything it allows is one of these, and every path reaches it as a
`-D name=value` parameter (`(param "name")`), never as text, so a path cannot
add a rule.

- **Read**: `file-read*` for each `--read` path and for the system files below.
- **Run**: `file-read*`, `file-map-executable` and `process-exec` for each
  `--run` and `--run-from` path and for the system runtime. Nothing else may
  be executed, so a program the caller did not grant fails with "Operation
  not permitted".
- **Write**: `file-read*` and `file-write*` for each `--write` directory and
  `/dev/null`; `file-link` is denied, as in the write limit.
- **Ancestors**: `file-read-metadata` (stat, never list or read) on every
  ancestor of every grant. Node's `realpath` and the dynamic loader `lstat`
  each one, and without this Pi aborts at start.
- **Standard streams**: `file-read-metadata` and `file-ioctl` on the path of
  stdin, stdout and stderr when they are files or terminals (`fcntl
  F_GETPATH`), since an `fstat` on a descriptor is checked against its path.
  Node aborts at start when its stdout is a file it cannot stat; a pipe has
  no path and needs nothing.
- **Network**: see below.

The launcher resolves the command to an absolute path with its links
resolved before the profile is on: `sandbox-exec` looks the name up after the
profile is applied, when the directories of `PATH` and a symlink in them
(`/opt/homebrew/bin/opencode`) can no longer be stat'ed.

The environment and the descriptors are the same as on Linux: the command
keeps `KEEP_ENV`, `LC_*` and each `--env` name, and every descriptor above
the standard three is closed before the exec (the loop in `close_range`, since
macOS has no such syscall). `tests/confine.rs` proves both on macOS.

Reviewed and checked on macOS 27:

- **The Data volume prefix.** `/System` is granted whole and the user's volume
  is mounted at `/System/Volumes/Data`. Reading `$HOME`, a sibling directory
  or `/Users/Shared` through that prefix is refused anyway, since Seatbelt
  resolves the firmlink before it matches (`tests/confine.rs` proves it, and
  proves the same path reads unconfined). A denial of
  `/System/Volumes/Data` and `/System/Volumes/VM` for read, map and exec
  comes after the grants regardless, in case that resolution changes; every
  traced harness (Pi, Node, curl, Claude Code) still runs with it.
- **A port limit without a confinement.** Only a confinement's profile has
  network rules. A policy with `--net-connect` ports and no read allowlist
  is refused (exit 125) rather than run with the limit dropped.
- **TIOCSTI.** The stdio paths get `file-ioctl` so Node can query a terminal.
  Tried on this Mac with a pty: unconfined, `TIOCSTI` types into the
  terminal; confined, it is refused (`Operation not permitted`) while
  `TIOCGWINSZ` and `TIOCGETA` work. The test `a_terminal_can_be_queried_but_not_typed_into`
  pins that. If a macOS release lets it through, restrict the grant with an
  `ioctl-command` filter.
- **Descriptors.** Closing lists the process's descriptors with
  `proc_pidinfo(PROC_PIDLISTFDS)`, so one above 65535 is closed too; a
  failure to list refuses the launch.

### The macOS runtime set, as traced

Traced by running `curl`, `node`, Pi (`pi --version` and a full `pi -p` run
against a local OpenAI-compatible server), Claude Code, Codex, Vibe,
OpenCode, `nscurl` and `/bin/sh`, `cat`, `ls` under `(deny default)` and
reading the `Sandbox` denials with `log stream --predicate 'sender=="Sandbox"'`,
on macOS 27 (Darwin 27.0.0), then removing each rule to see whether the run
still worked. What a run needs, and nothing else:

| Rule | Why (what failed without it) |
| :- | :- |
| `file-read*` on `/System`, mapped and run | dyld, the shared cache (in the Cryptex under `/System`), frameworks. Aborts at exec without it. |
| `file-read*`, `process-exec` on `/usr`, `/bin`, `/sbin` | the system programs and `/usr/lib`, `/usr/share`; the same set Linux grants. |
| `file-read-data` on `/` itself | dyld reads the root directory; aborts with exit 134 without it. |
| `file-read-metadata` on `/etc`, `/var`, `/tmp` | the symlinks into `/private`; DNS and TLS fail without them. |
| `file-read-metadata` on `/private/etc/localtime`, `/private/var/select/sh` | stat of the two symlinks the time zone and `/bin/sh` follow. |
| `file-read*` on `/private/etc/ssl` | the CA bundle LibreSSL (`curl`) reads; "configuration file routines" error without it. |
| `file-read*` on `/private/var/db/timezone` | the time zone database behind `/etc/localtime`, and ICU's `icutz` that Node reads. |
| `file-read*` on `/etc/resolv.conf`, `hosts`, `services`, `protocols`, `localtime`, `/dev/urandom`, `/dev/random` | as on Linux (links resolved). |
| `file-read*` on `/Library/Preferences/.GlobalPreferences.plist` | the locale defaults; every program asks, none fails without it. |
| `sysctl-read` | dyld and the language runtimes ask the kernel (`hw.optional.*`, `kern.osproductversion`); Node aborts without it. |
| `process-fork`, `signal (target self)` | a shell starts children. |
| `mach-lookup com.apple.trustd.agent` | certificate trust for programs that use the system trust store (Security.framework: `nscurl`, native TLS clients). Without it a valid certificate is rejected. |
| `network-outbound` to `/private/var/run/mDNSResponder` | name resolution (below). |

Denied and harmless, so left denied: `/dev/dtracehelper`, `vfs.disk-space`,
`kern.bootargs`, the `com.apple.diagnosticd`, `logd`, `notification_center`,
`opendirectoryd` Mach services, `/private/etc/passwd` (user-name lookups;
`HOME` is kept in the environment), and a `network-bind` the Node runtime
tries once at start. `/Users`, `/Volumes`, the home directory, other users'
files and every dot-directory under `~` are never readable.

Not in the set, and why a recipe has to add it: a Homebrew or `uv` install
reaches its own prefix through symlinks (`/opt/homebrew/bin/x` to a Cellar
directory; a `uv` tool's `bin/python` to `uv/python/cpython-3.x`, itself
reached through `uv/python/cpython-3.11-macos-aarch64-none`). `--run-from`
grants the real install of the program and of its `#!` interpreter, which is
what Pi and Claude Code need. A launcher script that `exec`s another
interpreter (Vibe, Hermes: `/bin/sh` then the tool's Python; `agy`: a wrapper
in `.homebrew-command-wrappers` that execs the binary above it) needs that
interpreter's install named with `--run` (for `uv` the whole `uv/python`
directory, since the minor-version link is read too).

## The network

`(deny default)` already denies the network. With `--net-connect PORT` the
profile adds `(allow network-outbound (remote tcp "*:PORT"))` for each port,
and the mDNSResponder socket for name resolution; a bind is never allowed. A
confinement with no `--net-connect` leaves the network alone (`(allow
network*)`), as on Linux.

This is the same promise Landlock's `ConnectTcp` makes (a port, any host),
rather than `localhost:<port>`: the Pi launch passes `--net-connect 443`
because the LiteLLM proxy is a remote host, so a `localhost`-only rule would
refuse the one thing Pi has to reach. A direct UDP send is refused on macOS
(Landlock lets port 53 through), since names resolve through the resolver
socket and need no UDP of the process's own; `tests/confine.rs` proves that.
The `*:PORT` form was verified for a loopback server (the Pi run against a
mock) and for an internet host (`curl`, Node `fetch` and `nscurl` to
`example.com:443`, while `:8080` is refused).

### Open point: harnesses that reach the internet directly

OpenCode (Zen), `agy`, Claude Code and Codex talk to a model host of their
own, not to the LiteLLM proxy. A port rule lets them reach any host on 443,
which is what Landlock allows too, and Seatbelt filters by address and port
and not by name. A host allowlist needs a local forward proxy that a
confined harness is pointed at (`HTTPS_PROXY=http://127.0.0.1:PORT`, with
`--net-connect PORT` for the proxy alone and the proxy holding the host
list). That proxy is **not built here**. The decision to take, per client,
when #903 wires them: (1) accept "443 to any host" where the client's own
tools are the limit and the read and write confinement is the point; (2)
build the allow-listing proxy for clients whose network reach matters; or (3)
for a client that cannot be confined (below), use its own sandbox. If a
runtime path cannot be proven on a Mac, the fallback the issue names is a
lightweight VM or container (Apple's Containerization framework, or a Linux
VM where Landlock holds); it is a fallback, not the first choice.

## Refusal

A profile that cannot be applied still exits 125 with nothing started, never
an unconfined run: the launcher builds the profile, runs it once under
`sandbox-exec` against `/usr/bin/true` (the probe profile has one extra rule
that lets `true` run, so the probe also proves the runtime set holds), and
refuses when that fails (`sandbox-exec` missing, already inside a Seatbelt
sandbox, which cannot nest, a path that cannot be resolved). One difference
from Linux: a program the profile does not let run is refused by
`sandbox-exec` itself, with its exit 71 and "Operation not permitted", not
125.

## What the launcher gained for the clients (#903)

Wiring every client showed three things a recipe needs that the first cut did not have. Each is an option of
`confine`, and none is on unless a recipe asks for it.

- **`--run-from` follows a launcher script.** A uv tool (Vibe, Hermes) and the Homebrew `agy` wrapper are
  `/bin/sh` scripts that `exec` another program (a venv's `python3` after `exec`, or `exec "<Caskroom>/antigravity"`).
  The launcher now reads the first lines of a script for the absolute program after `exec` and grants its
  install: the directory above its `bin` as written (a virtual environment keeps its libraries beside `bin`) and
  as it resolves (the Python it links to), and the `python` store directory of a uv-managed Python, which also
  holds the links to its versions. Nothing is guessed from a name: a script that names no absolute program grants
  what it did before.
- **Grants are resolved, and a missing one is refused.** Every `--read`, `--run` and `--write` is the path it
  resolves to, never the path it was written as, and it must exist: a missing `--write` file is not turned into a
  grant of a directory that may be made there later. A grant that goes through a link that sits in a directory the
  command may write is refused, since the command could have planted it; a link in the run's own home therefore
  cannot move a grant (to `~/.ssh`, say) on a later launch. `--run-from` grants the resolved program's own install
  (`Cellar/<pkg>/<version>`, a uv tool's directory), never a shared prefix: `/usr`, `/usr/local`, `/opt/homebrew`,
  `~`, `~/.local`, or the `bin` of one of them is narrowed to the program's file. A script's `exec` target counts
  only on a real command line (first word, after `NAME=value` assignments; not a comment, a string or an
  argument), and its venv is granted as written only when it has a `pyvenv.cfg`.
- **`--write FILE` is a file.** A path that is a file is granted as that file alone (a `literal` rule, not a
  subpath). The recipes below do not use it for a login any more (see Codex).
- **`--net-loopback` (macOS).** Binds, accepts and connects on loopback at any port (`localhost:*`), beside the
  `--net-connect` ports. A client that serves itself on a port chosen at run time needs it. It never opens the
  rest of the network. Landlock cannot name a port chosen at run time, so it refuses the flag: OpenCode and
  Antigravity are `launch-failed` on Linux until a backend can hold it.
- **`--keychain` (macOS).** Claude Code, Antigravity, Vibe and Codex keep their sign-in in the login keychain and
  ask it through `/usr/bin/security` or the Security framework. `--keychain` allows the Mach lookup of
  `com.apple.SecurityServer` and reads the login keychain file and the System keychain, not the directory of
  every keychain and not the rest of `~/Library`. **Residual risk, proved:** a confined process with it can read
  any item of those keychains whose access list lets the caller read it. In a throwaway keychain, an item made
  with `security add-generic-password -A` was read by a confined `security find-generic-password -w` with
  `--keychain` and not found without it; the keychain was deleted afterwards. An item whose access list names
  other applications still prompts or fails. Which clients need it: Codex (its cloud-config bundle fails without
  it: `Failed to load cloud config bundle`; tried), Claude Code (its subscription login is only in the keychain;
  `CLAUDE_CODE_OAUTH_TOKEN` would need the gateway to read and pass the token), Antigravity and Vibe (their
  keys; Vibe reads `MISTRAL_API_KEY` from the environment first, which would need the gateway to read the key).
  None could be moved off it without the gateway handling the credential itself, so the flag stays for these
  four and is the accepted risk of this ADR. The keychain is found through `HOME`, so these clients keep the real
  `HOME`.
- **`--preferences` (macOS).** Allows `cfprefsd` and the managed preferences, which Codex asks before it
  starts (`Failed to synchronize managed preferences` without it). There is no `user-preference-read`: traced,
  Codex asks only for `kCFPreferencesAnyApplication` and `com.apple.security` and starts and answers without
  them, so other applications' defaults stay unreadable.

## Per-client recipes (wired by #903: `client_run`, `run_home`, `client_confine`)

Every client but Pi starts as `agent-launcher confine <grants> -- <client> ...`. A client that cannot be
confined (no launcher, a home that cannot be made, a login that is not there) is refused with `launch-failed` and
the reason, never started bare. The launch passes by name every variable it sets (the launcher clears the rest),
and a key a wrapper reads from the secret store (`LITELLM_*_KEY`) by its name. Common to all: **read** the work
directory (or **write** it, and the git directories a writing run's client writes, for `mode: "write"`), **write**
the run directory (which holds the client's home and a `tmp` directory named by `TMPDIR`), **run** the client's
own install (`--run-from`) and the system runtime. What each adds, as traced and run live on macOS 27:

| Client | Home it runs in | Also reads / writes | Ports | macOS services | Notes |
| :- | :- | :- | :- | :- | :- |
| Pi | `pi-agent` in the run directory (as before) | | the proxy | | `pi_grants`, unchanged |
| OpenCode (LiteLLM route) | `<run>/client-home`: `HOME`, `XDG_*`, `OPENCODE_*`, one provider, tools per the mode | | 443 and loopback | | key from the secret store by the wrapper |
| OpenCode (own providers, `opencode/...` free models) | the same, with `OPENCODE_DB` at the owner's database | **writes** the owner's `~/.local/share/opencode` directory (the database and its journal) | 443 and loopback | | the sign-in is in the database; a fresh one answers 429 |
| Hermes (LiteLLM route) | `<run>/client-home`: `HERMES_HOME`, `HOME`, `XDG_*`, one provider | | the provider's (443) | | no literal key copied |
| Hermes (other provider) | the same, a `config.yaml` with no MCP server and a copy of the owner's `.env` | nothing of the owner's | 443 | | `--safe-mode` |
| Claude Code | `tmp` only (`CLAUDE_CODE_TMPDIR`), real `HOME` | nothing of `~/.claude`, `~/.claude.json` or `~/.claude-personal` | 443 | keychain; `USER`, `LOGNAME` kept | no global `CLAUDE.md`, skills or plugins load |
| Codex | `CODEX_HOME=<run>/client-home` holding a copy of the owner's `auth.json`; after the launch only the `tokens` and `last_refresh` members that changed are written back to the owner's file | nothing of the owner's | 443 | keychain, preferences | runs with its own sandbox off, below |
| Antigravity | `tmp` only, real `HOME` | **writes** `~/.gemini/antigravity-cli` and `~/.gemini/config` | 443 and loopback | keychain | its `--sandbox` is left out: its own Seatbelt cannot nest |
| Vibe | `VIBE_HOME=<run>/client-home/vibe`: a copy of the owner's `config.toml` and `agents/`, real `HOME` | **reads** `~/.agents/skills` (Vibe walks it at start and fails on a directory it cannot even stat; the delegate agent enables no skill) | 443 | keychain | no session history, trusted folders or key file |

**Codex.** Its own sandbox is a Seatbelt profile, and Seatbelt does not nest (`sandbox_apply: Operation not
permitted`, reproduced under `confine`). The two choices were to use Codex's sandbox and verify it, or to turn it
off and let ours be the sandbox. The first leaves its read-only mode reading the whole disk (Seatbelt's read-only
profile allows every read) and is the one sandbox the gateway cannot check. The second holds the reads to the work
directory and the writes to the worktree (a writing run) or the run directory, with the same proof every other
client has. **Decided: a confined Codex runs `-s danger-full-access`** (and no `--add-dir`, which only its own
workspace sandbox reads). The shell commands it runs are children of the confined process and inherit the
profile. It stays `--ignore-user-config`, and its home is the run's own, so the owner's `AGENTS.md`, skills and
plugins are no longer loaded.

**The network.** Pi, Hermes, Claude Code, Codex and Vibe get `--net-connect 443` (or the proxy's port). OpenCode
(`run --standalone`) and Antigravity serve themselves on a loopback port chosen at run time: they get
`--net-connect 443 --net-loopback`, so they reach loopback at any port and the rest of the network at 443 only.
Live on macOS, a confined `curl` to `example.com:8080` was refused and to `example.com:443` was answered under
that grant, and OpenCode answered with it (`tests/confine.rs` pins the refusal of an ungranted remote port).
No client runs with an unlimited network. **A host allowlist is not built:** `443` is any host, and the
allow-listing proxy of the open point above is still the open item for every client with web tools.

## Gaps left, per client

- An OpenCode run on its own providers can write the owner's OpenCode data directory. It holds the sign-in and
  the session history in one database, which the launcher cannot split.
- Antigravity can read the owner's `~/.gemini/config`, where its skills are, and its own state beside it.
- Vibe can list `~/.agents/skills`.
- Claude Code, Antigravity, Vibe and Codex ask the keychain for credentials; a prompt-injected client can still
  use the credential it was signed in with, as it always could.
- Codex runs on a copy of its login and only the `tokens` and `last_refresh` members are written back after the launch; a login refreshed by the run and then lost to a crash is not written back, and the owner signs in again.
- Linux: `--keychain` and `--preferences` do nothing, the recipes were run on macOS only, and OpenCode and Antigravity are refused (`--net-loopback` is not held by Landlock).
- The matrix's escape probe passes live for Claude Code, Antigravity, OpenCode and Hermes; Codex and Vibe have
  no profile model on this machine, so their cells are blocked and were run by hand under the same grants.

## Consequences

- The Pi launch, and every client wired to `confine` after it, runs on a
  Mac the way it runs on Linux, with the read allowlist and the network
  limit held by the kernel.
- A runtime set is macOS-version specific. The profile is proved at
  every launch by the probe, and `tests/confine.rs` runs on the macOS CI job,
  so a macOS release that needs more fails the check and the launch rather
  than running with less.
- The read limit of the personal instance on the work Mac (ADR-0013, #666)
  is unchanged: it is still a subtraction from `(allow default)` for a
  reach-level launch, not a confinement.
