# Agent Kit Portability

The durable source for this kit is a pinned JorisJonkers-dev/agent-kit release tag.
Generated client homes are reinstallable state; do not treat them as the source
of truth.

## Install And Uninstall

`install.sh` and `install-agents.sh` are retired (agent-kit#40); the KB
service that served them is being decommissioned (fleet-infra#231).
`setup-workstation.sh`, generated from `registry/estate-tooling.yaml`, is the
one installer now. From a checkout:

```bash
./installer/setup-workstation.sh
./installer/setup-workstation.sh --check   # report only, change nothing
```

Fetched over HTTP, once agent-kit#35 publishes the skills bundle:

```bash
curl -fsSL https://assets.jorisjonkers.dev/setup-workstation.sh | bash
```

Writes to `CLAUDE_CONFIG_DIR` or `~/.claude` and `CODEX_HOME` or `~/.codex`;
there is no project-scope install (that was an `install.sh`-only mode and did
not carry over).

Remove what a machine set up by the retired `install.sh` / `install-agents.sh`
wrote -- base skills, Spec Kit commands/skills, their MCP entries -- and purge
the retired knowledge hooks:

```bash
./installer/setup-workstation.sh --uninstall
./installer/setup-workstation.sh --uninstall --check   # preview
```

## Doctor

Run the local doctor before updating a pinned kit version or debugging drift:

```bash
python3 render-agent-kit.py --doctor
python3 render-agent-kit.py --doctor --require-live-kb
```

The doctor is read-only. It checks generated file drift, manifest version,
surface parity, and optional KB reachability. Missing `KB_URL` or
`KB_BEARER_TOKEN` is a warning unless `--require-live-kb` is set -- expect
that warning going forward, since nothing publishes those any more.

## Restore Order

1. Check out the desired JorisJonkers-dev/agent-kit release tag.
2. Run `uv run python render-agent-kit.py --check`,
   `uv run python scripts/validate_manifest.py`, and
   `uv run python scripts/render_registry.py --check`.
3. Reinstall client homes with `./installer/setup-workstation.sh`.
4. Run `render-agent-kit.py --doctor` (add `--require-live-kb` only while the
   KB still exists; expect it to warn once knowledge-api is decommissioned).

## Compatibility Matrix

| Surface | Current contract | Compatibility signal |
| --- | --- | --- |
| Manifest | `manifest.yaml`, version `2` | `scripts/validate_manifest.py` pins managed paths and hashes. |
| Renderer | `render-agent-kit.py` | Public modes: `--check`, `--write`, `--output`, `--doctor`. Renders repo skills/commands and the runtime package only; `setup-workstation.sh` is a separate artifact (below). |
| Installer | `installer/setup-workstation.sh`, generated from `registry/estate-tooling.yaml` by `scripts/render_registry.py` | Supports `--check`, `--no-lsp`, `--no-mcp`, `--no-profiles`, `--uninstall`. User scope only; no `--agent`/`--scope project` (those were `install.sh`-only and are retired with it). |
| Agent surfaces | `.claude`, `.codex`, `.agents` | Renderer `--check` must pass; manifest parity gaps require explicit unsupported reasons. |
| Council bundle | `council/` (repo tooling), `skills/council/` (workstation copy) | Manifest `council.files` pins the repo copy; `registry/estate-tooling.yaml`'s `local_skills` pins the workstation copy. |
| KB hooks | None. Retired estate-wide; `setup-workstation.sh` purges any still wired into a client's settings, on every run and on `--uninstall`. | `scripts/validate_manifest.py` fails if a `hooks:` or `settings:` section reappears in `manifest.yaml`. |

Update this matrix in the same branch as any change to renderer modes,
installer flags, managed paths, or manifest version.
