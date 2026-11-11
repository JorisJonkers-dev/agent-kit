# Agents

Guidance for any coding agent working in this repository. The repository's
agent contract lives in [CLAUDE.md](CLAUDE.md); it points at the estate-wide
conventions and holds the repository-specific rules. Read it first.

- `CONTEXT.md` defines the words this repository uses: declared, retired and
  undeclared entries, the keep file, and what it means for setup to converge.
- `docs/adr/` holds the architecture decisions.
- `registry/estate-tooling.yaml` is the source of what setup installs;
  `installer/setup-workstation.sh` is generated from it, so edit the registry
  and re-render with `scripts/render_registry.py`, never the generated file.
- `scripts/check_agents_md.py` is the lint that keeps this file honest: it fails
  when a path or symbol named here no longer exists.
