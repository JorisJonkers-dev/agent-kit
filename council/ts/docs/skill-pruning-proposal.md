# Skill Pruning Proposal

Status: checkpoint-2 proposal only. This document flags candidate pruning and
merge work for human sign-off; it does not authorize silent deletion.

## Audit Scope

Audited rendered and source surfaces:

- `templates/repo/.agents/skills/*/SKILL.md` rendered to
  `.agents/skills/*/SKILL.md`
- `templates/repo/.claude/skills/*/SKILL.md` rendered to
  `.claude/skills/*/SKILL.md`
- `templates/repo/.claude/commands/speckit.*.md` rendered to
  `.claude/commands/speckit.*.md`
- `templates/installer/partials/skills/*.md` embedded into
  `installer/install.sh`
- `council/` embedded into the installer council skill bundle
- `manifest.yaml` and `render-agent-kit.py` surface/parity declarations

Out of scope for this proposal: hooks, MCP server profiles, runtime container
files, and Spec Kit project seed files except where they explain command or
skill visibility.

## Current Surface

| Surface | Count | Notes |
| --- | ---: | --- |
| Codex repo skills | 17 | 8 shared skills plus 9 `speckit-*` phase skills. |
| Claude repo skills | 8 | Shared skills only. |
| Claude repo commands | 9 | `speckit.*` phase commands. |
| Installer partial skills | 5 | `topics`, `audit`, `kb-first`, `token-economy`, `agent-session-bootstrap`. |
| Council bundle | 1 skill name, 12 toolkit files | Installed for both Claude and Codex from `council/`. |

Validation snapshot:

- `python3 render-agent-kit.py --check` passed.
- `python3 render-agent-kit.py --doctor` passed render, manifest, parity, and
  installer checks. The only warning was the expected skipped live KB probe
  because `KB_URL` was unset.
- `render-agent-kit.py --doctor` reports `8 shared skills; 9 Spec Kit
  command/skill pairs`.
- `python3 scripts/validate_manifest.py` could not run in this sandbox because
  `PyYAML` is unavailable.

## Findings

### F1. Spec Kit phases are duplicated by design, but not by source

Evidence:

- Claude exposes 9 commands under `.claude/commands/speckit.*.md`.
- Codex exposes the same 9 phases under `.agents/skills/speckit-*/SKILL.md`.
- `manifest.yaml` pairs each Claude command with the corresponding Codex skill
  through `unsupported` notes.
- `render-agent-kit.py` checks only name parity between those two surfaces.

Risk:

- Users see both scoped and unscoped naming conventions depending on agent:
  `/speckit.plan` for Claude and `$speckit-plan` for Codex.
- Phase logic can drift because command bodies and skill bodies are maintained
  as separate Markdown files.

Proposal:

- Keep both rendered surfaces for now. They target different agent invocation
  models and current validation expects both.
- Merge the source of truth behind the 9 phases before pruning anything. A
  shared phase registry can render both command and skill wrappers.
- Mark the rendered command and skill as aliases of the same logical phase in
  `manifest.yaml`.

Human sign-off needed:

- Approve source merge while preserving both rendered aliases.
- Separately approve any later deletion after both agents support the same
  invocation path.

### F2. User-scope and project-scope installs can expose duplicate skills

Evidence:

- `install.sh` can install into user homes or project homes through
  `--scope user|project`.
- The installer writes `kb-first`, `token-economy`, `agent-session-bootstrap`,
  `topics`, `audit`, and `council` into client homes.
- The repository also contains local `.agents` and `.claude` surfaces, including
  `kb-first`, `token-economy`, `agent-session-bootstrap`, and `council`.

Risk:

- In a repository with both user-level and project-level agent-kit installed,
  skill discovery can show duplicate names from different roots.
- The user cannot tell which copy wins when local and user copies differ.

Proposal:

- Add explicit surface classes to `manifest.yaml`: `user-install`,
  `project-install`, `repo-template`, `installer-only`, and `alias`.
- Document discovery precedence: project-local skill wins over user-global
  skill when names collide.
- Keep uninstall behavior path-based. Do not remove user files simply because a
  project copy exists.

Human sign-off needed:

- Approve the precedence rule.
- Decide whether project-scope install should continue writing the shared
  user-style skills into `.claude/skills` and `.codex/skills`, or only write
  project hooks plus Spec Kit seed files.

### F3. `topics` and `audit` are active installer skills but look stale in repo audit

Evidence:

- `templates/installer/partials/skills/topics.md` and
  `templates/installer/partials/skills/audit.md` are written by
  `installer/install.sh`.
- `manifest.yaml` lists both skills with installer targets.
- Neither skill has a repo-local target under `templates/repo/.agents/skills` or
  `templates/repo/.claude/skills`.

Risk:

- A surface audit that starts from repo-local skills can incorrectly classify
  `topics` and `audit` as stale manifest entries.
- A surface audit that starts from installed user homes can incorrectly classify
  them as missing repo templates.

Proposal:

- Keep both skills if KB maintenance remains part of the default user install.
- Mark them explicitly as `installer-only` in the manifest.
- Add checksums for their partial source files so they have the same provenance
  quality as repo-local skills.

Human sign-off needed:

- Approve `topics` and `audit` as installer-only skills, or approve moving them
  into `templates/repo` so repo-local and installer surfaces match.

### F4. Personal-stack skills are mixed into the portable repo template surface

Evidence:

- `add-public-service`, `fleet-change`, `open-pr`, and `run-tests` are present
  in both `.agents/skills` and `.claude/skills`.
- Their descriptions and bodies reference personal-stack-specific paths and
  workflows such as `platform/inventory/fleet.yaml`, `services/auth-api`,
  `services/app-ui`, and Platform Validate.
- `kb-first` defaults to `scope=project:personal-stack` in the Codex template.

Risk:

- The portable agent-kit repository exposes project-specific skills by default.
- Installing or rendering these templates outside personal-stack can produce a
  stale or misleading skill picker.

Proposal:

- Split the repo template surface into a default portable profile and a
  `personal-stack` profile.
- Keep these skills in the `personal-stack` profile unless the kit is intended
  to remain personal-stack-specific.
- Change `kb-first` examples to avoid a hard-coded default personal-stack scope,
  or profile-gate that variant.

Human sign-off needed:

- Decide whether the default rendered repo surface should be portable or
  personal-stack-specific.
- Approve moving personal-stack-only skills behind a profile before any removal
  from the default surface.

### F5. Council is correctly bundled, but stale path text remains

Evidence:

- `council/` is the source path in this repository.
- `.agents/skills/council/SKILL.md`, `.claude/skills/council/SKILL.md`, and
  their templates still say the personal-stack copy lives at
  `platform/agents/council/council.py`.
- `council/README.md` and `council/council.py` also contain
  `platform/agents/council/council.py` examples or self-test paths.
- The installer writes a full multi-file `council` skill bundle for both agents.

Risk:

- The skill name is not stale, but the documentation points at a path that does
  not exist in this repository.
- Users can see both repo-local and installed `council` skills, with slightly
  different path assumptions.

Proposal:

- Keep the council skill and multi-file bundle.
- Update council path references in a later cleanup to prefer installed paths
  and this repository's `council/council.py` source path.
- Treat repo-local and user-installed `council` as the same logical skill in the
  manifest surface classes.

Human sign-off needed:

- Approve council path-text cleanup as non-behavioral.
- Do not delete the council bundle unless the orchestrator itself is removed.

### F6. BMAD and upstream lock files are not skill-surface entries

Evidence:

- `bmad-source.lock`, `skills-source.lock`, and `spec-kit-source.lock` exist.
- They do not render commands or skills directly.

Risk:

- Lock files can be mistaken for stale command or skill surface leftovers during
  broad cleanup.

Proposal:

- Do not prune lock files as part of skill-surface cleanup.
- Review them only in a source-vendoring audit.

Human sign-off needed:

- None for this pruning checkpoint, unless the checkpoint scope is expanded
  beyond rendered skills and commands.

## Proposed Pruning Map

| Candidate | Recommended action | Delete now? |
| --- | --- | --- |
| `speckit.*` Claude commands | Keep rendered aliases; merge source registry. | No |
| `speckit-*` Codex skills | Keep rendered aliases; merge source registry. | No |
| Duplicate user/project shared skills | Add surface classes and precedence. | No |
| `topics`, `audit` | Mark `installer-only` or promote to repo templates. | No |
| `add-public-service`, `fleet-change`, `open-pr`, `run-tests` | Move behind `personal-stack` profile if portability is desired. | No |
| `council` skill bundle | Keep; fix stale path text. | No |
| `bmad-source.lock`, `skills-source.lock`, `spec-kit-source.lock` | Exclude from skill pruning. | No |

## Checkpoint-2 Decisions Requested

1. Approve preserving both Spec Kit rendered alias surfaces while merging their
   source definitions.
2. Approve adding manifest surface classes and a project-over-user precedence
   rule.
3. Decide whether `topics` and `audit` are installer-only skills or should
   become repo-template skills.
4. Decide whether personal-stack skills stay in the default portable surface or
   move behind a profile.
5. Approve council stale path-text cleanup without deleting the council bundle.

## Suggested Follow-up Order

1. Add manifest surface classes and validator coverage.
2. Create a shared Spec Kit phase registry, then render current Claude commands
   and Codex skills from it.
3. Mark or promote installer-only KB maintenance skills.
4. Introduce a `personal-stack` profile for stack-specific skills, if approved.
5. Fix council path text and examples.
6. After one release with aliases and profile metadata, revisit actual deletions
   with a migration note and uninstall preview.
