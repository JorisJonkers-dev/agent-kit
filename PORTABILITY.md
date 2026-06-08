# Agent Kit Portability

The durable source for this kit is a pinned ExtraToast/agent-kit release tag.
Generated client homes are reinstallable state; do not treat them as the source
of truth.

## Install And Uninstall

Install into the current user's client homes:

```bash
curl -fsSL -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
  "${KB_URL}/install.sh" | bash -s -- --agent all --scope user
```

Install into a repository checkout for project-local hooks and skills:

```bash
curl -fsSL -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
  "${KB_URL}/install.sh" | AGENT_KIT_PROJECT_ROOT=/workspace/repo \
  bash -s -- --agent all --scope project
```

`--scope user` writes to `CLAUDE_CONFIG_DIR` or `~/.claude` and `CODEX_HOME`
or `~/.codex`. `--scope project` writes to `.claude` and `.codex` under
`AGENT_KIT_PROJECT_ROOT` or the current directory.

Uninstall uses the same `--agent` and `--scope` values:

```bash
curl -fsSL -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
  "${KB_URL}/install.sh" | bash -s -- --agent all --scope user --uninstall
```

Preview any install or uninstall with `--dry-run`.

## Doctor

Run the local doctor before updating a pinned kit version or debugging drift:

```bash
python3 render-agent-kit.py --doctor
python3 render-agent-kit.py --doctor --require-live-kb
```

The doctor is read-only. It checks generated file drift, manifest version,
surface parity, installer placeholders, installer secret-like markers, and
optional KB reachability. Missing `KB_URL` or `KB_BEARER_TOKEN` is a warning
unless `--require-live-kb` is set.

## Restore Order

1. Check out the desired ExtraToast/agent-kit release tag.
2. Run `python3 render-agent-kit.py --check` and
   `python3 scripts/validate_manifest.py`.
3. Ensure the KB service serves the checked-in `installer/install.sh` as
   `ClassPathResource("installer/install.sh")`.
4. Reinstall client homes with `/install.sh --agent all --scope user` or
   `/install.sh --agent all --scope project`.
5. Run `render-agent-kit.py --doctor --require-live-kb` when live KB credentials
   are available.

## Compatibility Matrix

| Surface | Current contract | Compatibility signal |
| --- | --- | --- |
| Manifest | `manifest.yaml`, version `2` | `scripts/validate_manifest.py` pins managed paths and hashes. |
| Renderer | `render-agent-kit.py` | Public modes: `--check`, `--write`, `--output`, `--doctor`. |
| Installer | `installer/install.sh`, generated from `templates/installer/install.sh.tpl` | Supports `--agent claude|codex|all`, `--scope user|project`, `--dry-run`, and `--uninstall`. |
| Agent surfaces | `.claude`, `.codex`, `.agents` | Renderer `--check` must pass; manifest parity gaps require explicit unsupported reasons. |
| Council bundle | `council/` | Manifest `council.files` pins driver, prompts, schemas, and config. |
| KB hooks | Hook templates and installer partials | Manifest lists canonical `knowledge.*` tool calls and CI runs shell syntax checks. |
| Spec Kit seed | `.specify/memory/constitution.md` plus `templates/repo/.specify` | Project-scope install seeds `.specify` files and preserves an existing constitution. |

Update this matrix in the same branch as any change to renderer modes,
installer flags, managed paths, or manifest version.
