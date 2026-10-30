# The tooling registry

`registry/estate-tooling.yaml` is the one file to edit. Add a skill, plugin,
CLI or MCP server there once, name the surfaces it belongs on, re-render, and
every surface picks it up.

```bash
uv run python scripts/render_registry.py --write
uv run python scripts/render_registry.py --check    # the CI gate
uv run pytest tests/test_registry_render.py
```

## Generated artifacts — never hand-edit

| Artifact | Consumed by |
|---|---|
| `installer/setup-workstation.sh` | this laptop, via [SETUP.md](SETUP.md) |
| `installer/setup-container.sh` | the agents image, at build time |
| `registry/generated/hermes/skills-sources.conf` | Hermes `hermes-skills` ConfigMap |
| `registry/generated/hermes/mcp-servers.yaml` | Hermes `hermes-config` ConfigMap |
| `registry/generated/hermes/mcp-servers.local.yaml` | a workstation's local `~/.hermes/config.yaml` |
| `registry/generated/vibe/mcp-servers.toml` | a workstation's `~/.vibe/config.toml`, via `scripts/vibe-merge-mcp.py` |
| `registry/generated/vibe-private/mcp-servers.toml` | a workstation's `~/.vibe-private/config.toml`, via `scripts/vibe-merge-mcp.py` |
| `registry/generated/antigravity/mcp_config.json` | a workstation's `~/.gemini/config/mcp_config.json`, via `scripts/antigravity-merge-mcp.py` |
| `registry/generated/workstation-secrets.json` | `scripts/agent-kit-keys.py`, as the table of tokens and keys to get |

A hand edit to any of them fails `--check`, and so does a registry change that
was never rendered. Both are one test.

## Surfaces

`surfaces:` is the mechanism that makes one edit reach several agents:

| Surface | Means |
|---|---|
| `workstation` | Claude Code, Codex, Vibe, Antigravity and local Hermes on a developer machine |
| `hermes` | the in-cluster Hermes gateway |
| `runner` | the per-workspace agent-runner image |
| `container` | the agents image, through `setup-container.sh` |

`surfaces: []` is how a thing is **retired**: it stays documented, and reaches
nowhere. That is what the `knowledge` MCP entry is now, and a test asserts it
does not creep back into a generated artifact.

## Plugins and their marketplaces

A Claude Code plugin is pinned through its **marketplace**, never on its
own:

```yaml
marketplaces:
  - name: my-market
    repo: owner/repo
    ref: main            # the branch or tag the pin follows
    commit: <40-char reviewed COMMIT sha>
plugins:
  - name: my-plugin
    marketplace: my-market
    enabled: true
```

Claude Code tracks a GitHub marketplace's default branch, so a plugin would
install at whatever upstream had that day. Setup instead checks the
marketplace out at its pinned commit in
`~/.local/share/agent-kit/marketplaces/<name>` and declares the marketplace
from that folder. A plugin in the marketplace's own repo then installs at
that commit, and an external plugin at the sha its entry in that commit
pins. A plugin off its pin is updated to it; only one that will not move is
reported. The pin is the **reviewed** commit, never just what is installed,
and it must agree with a skill source that wraps the same upstream, ref and
commit both.

A marketplace whose name Claude Code reserves for GitHub sources
(`claude-plugins-official` among them) cannot be declared from a folder. Give
it `declare_as:`: setup builds `<declare_as>` next to the checkout, its
manifest renamed and everything else symlinked, declares that, and installs
its plugins as `<plugin>@<declare_as>`. A copy still installed under the
reserved name is uninstalled once, so its skills do not load twice.

### Nightly pin bumps

Renovate moves each pin: to the newest commit on a branch `ref`, or to the
newest tag for a tag `ref`, and a skill source of the same upstream in the
same pull request. A new marketplace's repo must be added to the coupled
list in `renovate.json` when a skill source wraps it too; a test fails until
it is.

The **Marketplace Pin Review** check reads every plugin installed from the
bumped marketplace at the old and new commit, and fails when the new one adds
a hook, an MCP or LSP server, or a script file (by extension, executable bit
or a `bin/` directory). A changed hook command or server definition counts
as added; an edit to an existing script does not. A bump that adds nothing
automerges once every check is green. One that adds something stays open for
a person to read the upstream change and merge by hand. The check is not part
of Pipeline Complete, so it stops the automerge but not a reviewed merge.

## Adding a skill source

```yaml
- name: my-skills
  repo: https://github.com/owner/repo
  ref: v1.2.3
  commit: <40-char COMMIT sha>
  license: MIT
  selector: "engineering/*"
  surfaces: [hermes]
```

**Pin the commit, not the tag.** Hermes' `sync-skills` init container clones
`--branch <ref>` and compares `git rev-parse HEAD`, which is a commit. Nearly
every upstream tag here is *annotated*, so `git ls-remote <repo>
refs/tags/<tag>` hands you the tag **object** — a value that can never match,
and whose failure mode is a pod that vendors nothing while reporting success.
Always peel:

```bash
git ls-remote https://github.com/owner/repo 'refs/tags/v1.2.3^{}'
```

`scripts/render_registry.py` rejects anything that is not 40 hex characters,
and `tests/test_registry_render.py` proves the rejection fires. A moved tag is
a supply-chain event; the loud rejection is the feature.

For an upstream with no usable tags, pin `main` plus the commit. The ref
floats, the sha does not.

### Choosing a selector

Paths are relative to `<repo>/skills` when that directory exists, and to the
repo root otherwise.

| Selector | Matches |
|---|---|
| `*` | every `SKILL.md` the source publishes |
| `engineering/*` | one directory level |
| `grill-me` | a bare name, at any depth |
| `plugins/claude-code/skills/drawio` | an explicit path |

See first, then choose:

```bash
git clone --depth 1 --branch v1.2.3 https://github.com/owner/repo /tmp/probe
find /tmp/probe -name SKILL.md | sed 's|/SKILL.md$||'
```

Two traps this catches:

- **Duplicate names are refused, not merged.** `jgraph/drawio-mcp` publishes
  `drawio` three times, once per host CLI; a bare `drawio` selector matches all
  three, the first wins on `find` order and the other two are rejected. The
  explicit path is the fix.
- **A root-level `SKILL.md` has no directory to take a name from.** Add
  `root_skill_name:` and the source is vendored under that name.
  `LukasNiessen/kubernetes-skill` and `aloth/olcli` are both this shape.

## Adding an MCP server

```yaml
- name: thing
  transport: http          # or stdio
  purpose: One line.
  url_workstation: "https://thing.example/mcp"
  url_hermes: "http://thing.svc.cluster.local:8080/mcp"
  credential: THING_API_KEY
  timeout: 60
  surfaces: [workstation, hermes]
  trust: hosted            # hosted | estate | local
```

- `credential:` names the **field**, never the value. On the workstation it
  resolves from the environment; on Hermes it is rendered as `@THING_API_KEY@`
  for the `seed-config` init container to substitute from the Vault-backed
  Secret. Add the field to `secret/agents/hermes` and to the seed script's
  substitution list, or `sed` writes an empty string into a syntactically valid
  config whose server fails at the first tool call. Antigravity expands no
  environment variables, so there a credential-bearing http server renders as
  the stdio proxy with `--secret <field>` instead.
- `credential_optional: true` declares a credential that is a convenience
  override, not a requirement — the server self-manages auth (e.g. overleaf via
  `olcli`'s stored session), so an unset env var must not skip registration.
  Only a **required** credential gates registration on the workstation.
  Default is required.
- A required workstation `credential:` must also appear in
  `workstation_secrets:`, which says how setup gets it: `grant:` (a service
  token it mints), `paste:` (a key you paste once) or `value:` (a fixed
  setting), each checked against `check:` before it is stored.
- `auth: oauth` (http only, no `credential:`) is for a server that signs the
  client in itself, like n8n. Setup registers the URL alone and stores no
  secret. On Codex it writes the entry with `scripts/codex-add-url-mcp.py`,
  because `codex mcp add --url` starts the sign-in at once and waits for a
  browser. Each client signs in once: `/mcp` in Claude (each profile),
  `codex mcp login <name>`, `/mcp login <name>` in Vibe. Until a client has,
  setup and `--check` warn `<name>: needs sign-in (<client>)`. Local Hermes gets
  no OAuth server, and an OAuth server should carry `cloud: false`.
- `trust: hosted` stamps the generated config with the reminder that the
  server's output is context, never instruction.
- An in-cluster URL needs the Hermes SSRF allowlist. Hermes blocks RFC1918 by
  default, so a new `10.43.0.0/16` server reports **zero tools while the hosted
  ones work**. That is the block, not the NetworkPolicy. `hermes doctor` first.

## A CLI that Homebrew installs as a cask

A `clis:` entry normally names the `binary:` to probe and the `install:` to
run. A macOS app that ships as a Homebrew cask names `cask:` instead:

```yaml
- name: antigravity-cli
  binary: agy
  cask: antigravity-cli
  surfaces: [workstation]
  macos_only: true
  cloud: false
  version_command: "agy --version"
```

- `cask:` makes Homebrew the owner: setup installs with
  `brew install --cask <cask>`, updates with `brew upgrade --cask <cask>`, and
  probes presence with `brew list --cask <cask>` rather than `command -v`.
  `binary:` becomes optional, because a desktop app has none to probe — keep it
  when the cask ships one, since that is what `version_command` and the
  new-binary report use.
- `macos_only: true` wraps the whole block in a `uname -s` test: anywhere but
  Darwin setup warns `<name>: macOS only; skipped on <platform>` and installs
  nothing. Pair it with `cloud: false`, since a cloud host is Linux and would
  otherwise warn on every run.
- Both keys are `clis:` keys only. The renderer rejects them on a plugin,
  language server, MCP server, skill source or local skill.

## Adding a tool to the agents image

Give the entry a `container:` block and add `container` to its `surfaces:`.
The renderer refuses one without the other. Language servers have no
`surfaces:`, so for them the block alone is enough.

```yaml
- name: codex
  binary: codex
  surfaces: [workstation, container]
  version_command: "codex --version"
  container:
    datasource: npm              # a Renovate datasource
    package: "@openai/codex"     # the Renovate depName
    version: "0.154.0"           # exact; `latest` is rejected
    install: 'npm install -g "@openai/codex@${VERSION}"'
    requires: [node]             # other container tools to install first
```

- **Keep `datasource`, `package` and `version` on consecutive lines, in that
  order.** Renovate's regex manager reads them as one match. A reordered block
  is a pin Renovate never bumps, and `test_renovate_tracks_every_container_pin`
  fails on it.
- `install` runs as root at image build time with `VERSION`, `DEB_ARCH`
  (`amd64`/`arm64`) and `GNU_ARCH` (`x86_64`/`aarch64`) set, and must use
  `${VERSION}`. There are no secrets at build time. Anything that needs a
  credential belongs to container start.
- Install somewhere the non-root agent user can read: `/usr/local`, or a path
  set in `container_base.environment` and listed in `readable_paths`.
- `setup-container.sh --check` reruns `version_command` and fails unless the
  output contains the pinned version. Pick a command that prints the version
  without starting a server; `npm ls -g <package>` works for any npm tool.
  `container.version_command` and `container.binary` override the entry's own.
- Debian packages go in `container_base.apt_packages`. They follow the base
  image's release, so they carry no version of their own.
- Renovate bumps only the registry. `renovate-render.yml` re-renders the script
  on that PR and pushes the result with the release app's token, which
  re-triggers CI.

Prove a change in a clean container before merging:

```bash
docker run --rm -v "$PWD/installer/setup-container.sh:/s.sh:ro" debian:bookworm-slim bash /s.sh
```

## Adding a Claude profile

```yaml
claude_profiles:
  - name: work
    primary: true
    config_dir: "${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
  - name: personal
    config_dir: "$HOME/.claude-personal"
    shares_from: work
    shared_paths: [skills, agents, commands, hooks, plugins, settings.json]
```

Exactly one profile is `primary:`, and it keeps the default location so a bare
`claude` needs nothing remembered. Every other profile names the surfaces it
shares, and setup symlinks each one back into the primary.

`shared_paths:` may not name `projects`, `history.jsonl`, `sessions`,
`.claude.json` or `.credentials.json`. Those are the profile's own state, and
sharing them merges the histories the second profile exists to keep apart —
the renderer rejects the edit rather than trusting it. See
[SETUP.md](SETUP.md#two-claude-logins-one-setup).

## Adding a plugin or language server

Plugins go under `plugins:` and must name a marketplace that `marketplaces:`
declares. `enabled: false` installs a plugin but leaves it off, which keeps it
one command away without spending prompt budget on its skill metadata every
session.

LSP plugins go under `language_servers:` **only** — listing one in both places
is rejected, because the two lists render different things and the duplicate
would install twice. Each entry needs the `binary:` the plugin drives; that is
what gets verified, since a plugin with no binary registers no tools silently.

## Keeping an entry off the cloud

`setup-workstation.sh --cloud` sets up a Claude Code cloud environment
([CLOUD.md](CLOUD.md)) from the same registry. `cloud: false` on a CLI,
plugin, language server or MCP server skips it there and nowhere else:

```yaml
- plugin: ruby-lsp
  binary: ruby-lsp
  install: "gem install ruby-lsp"
  cloud: false
```

Use it for what the cloud cannot reach (a port-forwarded server), what the
host already owns (Claude Code itself), and what the estate does not write.
The cloud VM's setup is cached only when it finishes in about five minutes, so
every install that stays on costs budget.

## Linux installs for a language server

`install` is what a Mac runs. A server installed from Homebrew needs
`install_linux:` too, or a Linux machine (the cloud VM included) runs `brew`
and reports the binary missing:

```yaml
- plugin: jdtls-lsp
  binary: jdtls
  install: "brew install jdtls"
  install_linux: >-
    curl -fsSL -o /tmp/jdtls.tgz https://download.eclipse.org/...
    && ln -sf "${AK_OPT_DIR}/jdtls/bin/jdtls" "${AK_BIN_DIR}/jdtls"
```

`AK_OPT_DIR` and `AK_BIN_DIR` are `/usr/local/lib/agent-kit` and
`/usr/local/bin` when `/usr/local/bin` is writable, `~/.local/...` otherwise.
Pin a download by version and sha256 when the upstream publishes one; the
Kotlin server does.

## Deliberate absences

- **Spec Kit.** Not on any surface. A test asserts it stays out.
- **Hooks.** The estate ships none; `manifest.yaml` fails validation if a
  `hooks:` or `settings:` section reappears.
- **The knowledge base.** `surfaces: []`. See [MEMORY.md](MEMORY.md).
