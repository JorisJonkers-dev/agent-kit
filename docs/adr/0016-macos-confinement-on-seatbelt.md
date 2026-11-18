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

## Per-client recipes (to be wired by #903)

Where the launcher's confinement wraps the client, the recipe is the
arguments after `agent-launcher confine`. None of these is wired yet: #903
does that, and `docs/model-gateway/harness-matrix.md` gets its macOS column
then. The "traced" column says what was run under the confinement here.

| Client | Wrapped by `confine`? | Reads | Runs | Writes | Ports | Env | Traced here |
| :- | :- | :- | :- | :- | :- | :- | :- |
| Pi | yes (wired: `pi_grants`) | the work directory | `--run-from pi` (its package and `node`) | its agent directory (also `TMPDIR`) | the proxy (443) | `PI_CODING_AGENT_DIR`, `TMPDIR` | full `pi -p` run, local mock and the real proxy through `model-gateway run` (`pi-opencode-go`) |
| OpenCode | yes | the work directory | `--run-from opencode` (the Homebrew Cellar directory) | `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME` and `TMPDIR`, all under one run directory (it creates `$TMPDIR/opencode` and a `log` directory at start) | the proxy; Zen is a direct host (open point) | the five variables | `opencode --version` |
| Hermes | yes | the work directory, its state directory `~/.hermes` (it reads `~/.hermes/.env`) | the `uv` tool directory and the whole `uv/python` directory | its state directory | the proxy | as the client needs | start and import reach `~/.hermes/.env` and stop there; `--version` not run to the end |
| Claude Code | yes, `claude --version` and a Mach-O binary | the work directory | `--run-from claude` | its config directory | `api.anthropic.com` is a direct host (open point); the trust store goes through `trustd` | `ANTHROPIC_*`, config directory | `claude --version` |
| Codex | **no**: it uses its own Seatbelt, which cannot nest (`sandbox_apply: Operation not permitted` was reproduced under `confine`) | | | | | | `codex --version` runs confined; the real run uses its sandbox flags and is checked, not wrapped |
| `agy` (Antigravity) | yes, unless `agy --sandbox` is used (it may use Seatbelt too; not checked) | the work directory | `--run` the Caskroom directory that holds `antigravity` (the `agy` wrapper sits one level below it) | its state directory | direct host (open point) | as the client needs | `antigravity --version` |
| Vibe | yes | the work directory | `--run` the `uv` tool directory and the Python it runs on (here `/Library/Frameworks/Python.framework`) | `VIBE_HOME` | the proxy or Mistral (direct host) | `VIBE_HOME`, key | `vibe --version` |

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
