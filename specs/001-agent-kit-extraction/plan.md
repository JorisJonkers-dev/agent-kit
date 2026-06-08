# Technical Plan: Agent Kit Extraction

## Implementation Context

The repository will become the canonical source for the pinned agent kit. The
read-only compatibility baseline is the personal-stack kit renderer, manifest,
installer template partials, generated installer resource, Spec Kit seed files,
and council bundle.

This implementation keeps the artifact source self-contained at repository
root:

- `render-agent-kit.py`: Python renderer/doctor CLI preserving `--check`,
  `--write`, `--output`, and `--doctor`.
- `manifest.yaml`: parity, provenance, path, and checksum ledger.
- `templates/repo/`: source templates for generated repo-local `.claude`,
  `.codex`, `.agents`, and `.specify` surfaces.
- `templates/installer/`: source template and partials used to generate the
  serving artifact.
- `installer/install.sh`: generated shell artifact served by the KB service as
  `ClassPathResource("installer/install.sh")`; contains `@VERSION@` and
  `@KB_URL@` placeholders for runtime substitution.
- `council/`: first-class council driver, prompts, schemas, and default config.
- `.specify/memory/constitution.md`: project seed source copied into project
  scope installs only when absent.

## Chosen Technology

- Python 3 stdlib for renderer behavior, deterministic file rendering, drift
  checks, shell heredoc expansion, and read-only doctor checks. This preserves
  the existing renderer contract without introducing runtime dependencies.
- A small Python manifest validator using PyYAML in development/CI. YAML is the
  existing manifest format and the validator checks schema shape, checksums,
  managed-path coverage, parity gaps, installer placeholders, and secret-like
  markers.
- GitHub Actions for CI with real gating jobs for Python linting, renderer
  checks, doctor diagnostics, manifest validation, and release configuration.
  The terminal job is named exactly `Pipeline Complete`.
- Release Please `simple` release type for version tags. This repo is a tool
  kit; no Maven or npm artifact is published in this change.

## Architecture

The renderer treats the kit repository root as the default destination for
`--check`, `--write`, and `--doctor`. `--output` renders the same managed
surface set into a caller-selected directory. Template expansion supports:

- literal files under `templates/repo/`;
- `# @agent-kit-include ...` installer partials;
- generated council heredocs from `council/`;
- generated Spec Kit command and skill heredocs from `templates/repo/`;
- generated Spec Kit project seed heredocs from root `.specify` and
  `templates/repo/.specify`.

Doctor checks are self-contained:

- static render drift;
- manifest version presence;
- surface parity for shared skills and Spec Kit command/skill pairs;
- installer placeholder and secret-pattern checks;
- optional live KB reachability through `KB_URL` and `KB_BEARER_TOKEN`.

The manifest remains the compatibility ledger. Every generated managed path is
listed under `renderer.managed_paths` and pinned in a typed section with a
SHA-256 checksum. Intentional single-surface gaps are represented with
`unsupported` reasons.

## File Layout

```text
.
├── .github/workflows/ci.yml
├── .github/workflows/release.yml
├── .release-please-manifest.json
├── .specify/memory/constitution.md
├── council/
├── installer/install.sh
├── manifest.yaml
├── render-agent-kit.py
├── release-please-config.json
├── requirements-dev.txt
├── scripts/validate_manifest.py
├── templates/installer/
├── templates/repo/
└── specs/001-agent-kit-extraction/
```

## FR Traceability

| Requirement | Design Element |
| --- | --- |
| FR-1 | Release Please tags plus `manifest.yaml` artifact metadata define the versioned ExtraToast/agent-kit source. |
| FR-2 | Manifest short coordinate is `github:ExtraToast/agent-kit`; validator rejects repeated `agent-kit` coordinate segments. |
| FR-3 | Renderer emits all supported agent surfaces from checked-in templates. |
| FR-4 | Manifest records artifact metadata, managed paths, and checksums; installer writes version manifests. |
| FR-5 | Renderer CLI keeps `--check`, `--write`, `--output`, and `--doctor`. |
| FR-6 | `--check` compares rendered bytes and modes without writing. |
| FR-7 | `--write` updates managed surfaces in the destination root. |
| FR-8 | `--output DIR` writes the full managed surface set into `DIR`. |
| FR-9 | `--doctor` reports render drift, manifest version, parity, installer state, and KB reachability. |
| FR-10 | Manifest sections cover skills, hooks, settings, commands, council, KB-oriented surfaces, Spec Kit seeds, and installer. |
| FR-11 | Manifest pins SHA-256 checksums for managed files. |
| FR-12 | Manifest and validator enforce supported-surface parity or explicit unsupported reasons. |
| FR-13 | `council/` plus manifest `council` section define the first-class bundle. |
| FR-14 | KB hooks and KB-oriented skills are included in repo templates, installer partials, and manifest entries. |
| FR-15 | CI renderer check and manifest validation fail non-zero on drift. |
| FR-16 | CI passes when rendered surfaces, manifest entries, and checksums agree. |
| FR-17 | `installer/install.sh` remains the script body for the `/install.sh` service route. |
| FR-18 | Artifact path matches the classpath resource name `installer/install.sh`. |
| FR-19 | Installer source contains `@VERSION@` and `@KB_URL@` placeholders. |
| FR-20 | Validator scans installer source for secret-like markers. |
| FR-21 | Installer keeps `--scope user` and `--scope project`. |
| FR-22 | Installer keeps `--dry-run` preview and `--uninstall` removal. |
| FR-23 | This repo publishes tags only; personal-stack remains a consumer and is not versioned here. |
| FR-24 | `--output` and the short GitHub release coordinate allow website adoption without a separate distribution path. |

## Deviations From Reference

- Personal-stack-only doctor MCP profile checks are replaced with
  self-contained manifest/parity checks because this repo does not own the
  runner entrypoint or cluster ConfigMap.
- The generated installer serving artifact moves from
  `services/knowledge-api/src/main/resources/installer/install.sh` to
  `installer/install.sh`; consumers can vendor or copy that path into the KB
  service classpath without changing the serving contract.
- No package publish workflow is added because the initial artifact is a
  versioned repository tag consumed by pinned source vendoring.

## Validation Plan

- `python3 render-agent-kit.py --check`
- `python3 render-agent-kit.py --doctor`
- `python3 render-agent-kit.py --output <tmpdir>` and compare no worktree drift
- `python3 scripts/validate_manifest.py`
- `python3 -m compileall render-agent-kit.py scripts council`
- `ruff check .`
- `bash -n installer/install.sh` after rendering
- CI jobs for lint, render/doctor, manifest validation, and release config
  with final `Pipeline Complete`.
