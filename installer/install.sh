#!/usr/bin/env bash
# knowledge-system installer for Claude Code and Codex clients.
#
# Writes the local hooks + skills that pair with the knowledge-api
# MCP server. Idempotent: re-running picks up any updates the server
# ships in subsequent versions. Use `--agent claude|codex|all` and
# `--scope user|project` to choose the client homes to manage. Use
# `--dry-run` to preview the changes before they land. Use `--uninstall`
# to remove them.
#
# Released versions are pinned by the @VERSION@ token below, which
# the knowledge-api templates at request time from
# `SERVICE_VERSION` (or `unknown` for local builds).

set -euo pipefail

readonly INSTALLER_VERSION='@VERSION@'
readonly KB_URL='@KB_URL@'

DRY_RUN=0
UNINSTALL=0
AGENT=claude
SCOPE=user

usage() {
  cat <<USAGE
knowledge-system installer ${INSTALLER_VERSION}

Writes Claude Code and/or Codex hooks + skills that pair with the MCP server at
${KB_URL}.

Usage:
  curl -fsSL -H "Authorization: Bearer \$KB_BEARER_TOKEN" \\
    ${KB_URL}/install.sh | bash [-s -- [--agent claude|codex|all] [--scope user|project] [--dry-run|--uninstall]]

Options:
  --agent      Client home to manage. Defaults to "claude" for backwards compatibility.
  --scope      Install scope. "user" writes to client config homes; "project"
               writes to .claude/.codex under AGENT_KIT_PROJECT_ROOT or $PWD.
               Defaults to "user".
  --dry-run     Print every change without modifying the filesystem.
  --uninstall   Remove every file this installer would write.
  --help        Show this help and exit.

Environment:
  CLAUDE_CONFIG_DIR   Override the Claude Code config root (default ~/.claude).
  CODEX_HOME          Override the Codex config root (default ~/.codex).
  AGENT_KIT_PROJECT_ROOT
                      Project root for --scope project (default current directory).
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --agent)
      shift
      [ "$#" -gt 0 ] || { echo "--agent requires claude, codex, or all" >&2; exit 64; }
      AGENT="$1"
      ;;
    --agent=*) AGENT="${1#--agent=}" ;;
    --scope)
      shift
      [ "$#" -gt 0 ] || { echo "--scope requires user or project" >&2; exit 64; }
      SCOPE="$1"
      ;;
    --scope=*) SCOPE="${1#--scope=}" ;;
    --dry-run) DRY_RUN=1 ;;
    --uninstall) UNINSTALL=1 ;;
    --help|-h) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage >&2; exit 64 ;;
  esac
  shift
done

case "${AGENT}" in
  claude)
    INSTALL_CLAUDE=1
    INSTALL_CODEX=0
    ;;
  codex)
    INSTALL_CLAUDE=0
    INSTALL_CODEX=1
    ;;
  all)
    INSTALL_CLAUDE=1
    INSTALL_CODEX=1
    ;;
  *)
    echo "--agent must be claude, codex, or all" >&2
    usage >&2
    exit 64
    ;;
esac

case "${SCOPE}" in
  user|project) ;;
  *)
    echo "--scope must be user or project" >&2
    usage >&2
    exit 64
    ;;
esac

if [ "${SCOPE}" = "project" ]; then
  PROJECT_ROOT="${AGENT_KIT_PROJECT_ROOT:-$PWD}"
  [ -d "${PROJECT_ROOT}" ] || { echo "project root does not exist: ${PROJECT_ROOT}" >&2; exit 64; }
  readonly PROJECT_ROOT
  readonly CLAUDE_HOME="$(cd "${PROJECT_ROOT}" && pwd)/.claude"
  readonly CODEX_HOME="$(cd "${PROJECT_ROOT}" && pwd)/.codex"
else
  readonly CLAUDE_HOME="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
  readonly CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"
fi
readonly HOOKS_DIR="${CLAUDE_HOME}/hooks"
readonly AGENTS_DIR="${CLAUDE_HOME}/agents"
readonly COMMANDS_DIR="${CLAUDE_HOME}/commands"
readonly SKILLS_DIR="${CLAUDE_HOME}/skills"
readonly MANIFEST="${CLAUDE_HOME}/.knowledge-system-version"
readonly CODEX_HOOKS_DIR="${CODEX_HOME}/hooks"
readonly CODEX_SKILLS_DIR="${CODEX_HOME}/skills"
readonly CODEX_HOOKS_CONFIG="${CODEX_HOME}/hooks.json"
readonly CODEX_MANIFEST="${CODEX_HOME}/.knowledge-system-version"
readonly CODEX_ALLOWLIST="${CODEX_HOME}/.knowledge-system-allowlist"

log() { printf 'knowledge-system installer: %s\n' "$*"; }

write_file() {
  local path="$1"
  local mode="$2"
  local content="$3"
  if [ "${DRY_RUN}" = 1 ]; then
    log "would write ${path} (mode ${mode}, $(printf '%s' "$content" | wc -c | tr -d ' ') bytes)"
    return
  fi
  mkdir -p "$(dirname "$path")"
  printf '%s' "$content" > "$path"
  chmod "$mode" "$path"
  log "wrote ${path}"
}

remove_file() {
  local path="$1"
  if [ ! -e "$path" ]; then return; fi
  if [ "${DRY_RUN}" = 1 ]; then
    log "would remove ${path}"
    return
  fi
  rm -f "$path"
  log "removed ${path}"
}

readonly STATE_DIR="${CLAUDE_HOME}/state"
readonly ALLOWLIST="${CLAUDE_HOME}/.knowledge-system-allowlist"

claude_agent_managed_paths=(
)

claude_managed_paths=(
  "${HOOKS_DIR}/pre-tool-use-edit-recall.sh"
  "${HOOKS_DIR}/pre-tool-use-git-commit-capture.sh"
  "${HOOKS_DIR}/stop-session-digest.sh"
  "${claude_agent_managed_paths[@]}"
  "${COMMANDS_DIR}/speckit.analyze.md"
  "${COMMANDS_DIR}/speckit.checklist.md"
  "${COMMANDS_DIR}/speckit.clarify.md"
  "${COMMANDS_DIR}/speckit.constitution.md"
  "${COMMANDS_DIR}/speckit.implement.md"
  "${COMMANDS_DIR}/speckit.plan.md"
  "${COMMANDS_DIR}/speckit.specify.md"
  "${COMMANDS_DIR}/speckit.tasks.md"
  "${COMMANDS_DIR}/speckit.taskstoissues.md"
  "${SKILLS_DIR}/topics/SKILL.md"
  "${SKILLS_DIR}/audit/SKILL.md"
  "${SKILLS_DIR}/kb-first/SKILL.md"
  "${SKILLS_DIR}/token-economy/SKILL.md"
  "${SKILLS_DIR}/agent-session-bootstrap/SKILL.md"
  "${SKILLS_DIR}/council/SKILL.md"
  "${SKILLS_DIR}/council/council.mjs"
  "${SKILLS_DIR}/council/council.toml"
  "${SKILLS_DIR}/council/prompts/_baseline.md"
  "${SKILLS_DIR}/council/prompts/consolidator.md"
  "${SKILLS_DIR}/council/prompts/correct_course.md"
  "${SKILLS_DIR}/council/prompts/critic.md"
  "${SKILLS_DIR}/council/prompts/design_consolidator.md"
  "${SKILLS_DIR}/council/prompts/design_lens.md"
  "${SKILLS_DIR}/council/prompts/design_vote.md"
  "${SKILLS_DIR}/council/prompts/designer.md"
  "${SKILLS_DIR}/council/prompts/grill.md"
  "${SKILLS_DIR}/council/prompts/planner.md"
  "${SKILLS_DIR}/council/prompts/review_triage.md"
  "${SKILLS_DIR}/council/prompts/reviewer_acceptance.md"
  "${SKILLS_DIR}/council/prompts/reviewer_adversarial.md"
  "${SKILLS_DIR}/council/prompts/reviewer_edgecase.md"
  "${SKILLS_DIR}/council/prompts/reviewpack/checkpoint-1.html"
  "${SKILLS_DIR}/council/prompts/reviewpack/checkpoint-1.md"
  "${SKILLS_DIR}/council/prompts/reviewpack/checkpoint-2.html"
  "${SKILLS_DIR}/council/prompts/reviewpack/checkpoint-2.md"
  "${SKILLS_DIR}/council/prompts/reviewpack/design-checkpoint.html"
  "${SKILLS_DIR}/council/prompts/reviewpack/design-checkpoint.md"
  "${SKILLS_DIR}/council/prompts/reviser.md"
  "${SKILLS_DIR}/council/prompts/story_author.md"
  "${SKILLS_DIR}/council/prompts/story_check.md"
  "${SKILLS_DIR}/council/prompts/story_template.md"
  "${SKILLS_DIR}/council/prompts/survey.md"
  "${SKILLS_DIR}/council/prompts/triage_judge.md"
  "${SKILLS_DIR}/council/prompts/verifier.md"
  "${SKILLS_DIR}/council/prompts/worker.md"
  "${SKILLS_DIR}/council/schemas/amendment.schema.json"
  "${SKILLS_DIR}/council/schemas/consolidated.schema.json"
  "${SKILLS_DIR}/council/schemas/context-pack.schema.json"
  "${SKILLS_DIR}/council/schemas/design-ledger.schema.json"
  "${SKILLS_DIR}/council/schemas/events.schema.json"
  "${SKILLS_DIR}/council/schemas/plan.schema.json"
  "${SKILLS_DIR}/council/schemas/review-verdict.schema.json"
  "${SKILLS_DIR}/council/schemas/routing-verdict.schema.json"
  "${SKILLS_DIR}/council/schemas/run-state.schema.json"
  "${SKILLS_DIR}/council/schemas/verdict.schema.json"
  "${SKILLS_DIR}/council/schemas/worker-result.schema.json"
  "${ALLOWLIST}"
)

codex_managed_paths=(
  "${CODEX_HOOKS_DIR}/pre-tool-use-edit-recall.sh"
  "${CODEX_HOOKS_DIR}/pre-tool-use-git-commit-capture.sh"
  "${CODEX_HOOKS_DIR}/kb-stop-digest.sh"
  "${CODEX_SKILLS_DIR}/speckit-analyze/SKILL.md"
  "${CODEX_SKILLS_DIR}/speckit-checklist/SKILL.md"
  "${CODEX_SKILLS_DIR}/speckit-clarify/SKILL.md"
  "${CODEX_SKILLS_DIR}/speckit-constitution/SKILL.md"
  "${CODEX_SKILLS_DIR}/speckit-implement/SKILL.md"
  "${CODEX_SKILLS_DIR}/speckit-plan/SKILL.md"
  "${CODEX_SKILLS_DIR}/speckit-specify/SKILL.md"
  "${CODEX_SKILLS_DIR}/speckit-tasks/SKILL.md"
  "${CODEX_SKILLS_DIR}/speckit-taskstoissues/SKILL.md"
  "${CODEX_SKILLS_DIR}/topics/SKILL.md"
  "${CODEX_SKILLS_DIR}/audit/SKILL.md"
  "${CODEX_SKILLS_DIR}/kb-first/SKILL.md"
  "${CODEX_SKILLS_DIR}/token-economy/SKILL.md"
  "${CODEX_SKILLS_DIR}/agent-session-bootstrap/SKILL.md"
  "${CODEX_SKILLS_DIR}/council/SKILL.md"
  "${CODEX_SKILLS_DIR}/council/council.mjs"
  "${CODEX_SKILLS_DIR}/council/council.toml"
  "${CODEX_SKILLS_DIR}/council/prompts/_baseline.md"
  "${CODEX_SKILLS_DIR}/council/prompts/consolidator.md"
  "${CODEX_SKILLS_DIR}/council/prompts/correct_course.md"
  "${CODEX_SKILLS_DIR}/council/prompts/critic.md"
  "${CODEX_SKILLS_DIR}/council/prompts/design_consolidator.md"
  "${CODEX_SKILLS_DIR}/council/prompts/design_lens.md"
  "${CODEX_SKILLS_DIR}/council/prompts/design_vote.md"
  "${CODEX_SKILLS_DIR}/council/prompts/designer.md"
  "${CODEX_SKILLS_DIR}/council/prompts/grill.md"
  "${CODEX_SKILLS_DIR}/council/prompts/planner.md"
  "${CODEX_SKILLS_DIR}/council/prompts/review_triage.md"
  "${CODEX_SKILLS_DIR}/council/prompts/reviewer_acceptance.md"
  "${CODEX_SKILLS_DIR}/council/prompts/reviewer_adversarial.md"
  "${CODEX_SKILLS_DIR}/council/prompts/reviewer_edgecase.md"
  "${CODEX_SKILLS_DIR}/council/prompts/reviewpack/checkpoint-1.html"
  "${CODEX_SKILLS_DIR}/council/prompts/reviewpack/checkpoint-1.md"
  "${CODEX_SKILLS_DIR}/council/prompts/reviewpack/checkpoint-2.html"
  "${CODEX_SKILLS_DIR}/council/prompts/reviewpack/checkpoint-2.md"
  "${CODEX_SKILLS_DIR}/council/prompts/reviewpack/design-checkpoint.html"
  "${CODEX_SKILLS_DIR}/council/prompts/reviewpack/design-checkpoint.md"
  "${CODEX_SKILLS_DIR}/council/prompts/reviser.md"
  "${CODEX_SKILLS_DIR}/council/prompts/story_author.md"
  "${CODEX_SKILLS_DIR}/council/prompts/story_check.md"
  "${CODEX_SKILLS_DIR}/council/prompts/story_template.md"
  "${CODEX_SKILLS_DIR}/council/prompts/survey.md"
  "${CODEX_SKILLS_DIR}/council/prompts/triage_judge.md"
  "${CODEX_SKILLS_DIR}/council/prompts/verifier.md"
  "${CODEX_SKILLS_DIR}/council/prompts/worker.md"
  "${CODEX_SKILLS_DIR}/council/schemas/amendment.schema.json"
  "${CODEX_SKILLS_DIR}/council/schemas/consolidated.schema.json"
  "${CODEX_SKILLS_DIR}/council/schemas/context-pack.schema.json"
  "${CODEX_SKILLS_DIR}/council/schemas/design-ledger.schema.json"
  "${CODEX_SKILLS_DIR}/council/schemas/events.schema.json"
  "${CODEX_SKILLS_DIR}/council/schemas/plan.schema.json"
  "${CODEX_SKILLS_DIR}/council/schemas/review-verdict.schema.json"
  "${CODEX_SKILLS_DIR}/council/schemas/routing-verdict.schema.json"
  "${CODEX_SKILLS_DIR}/council/schemas/run-state.schema.json"
  "${CODEX_SKILLS_DIR}/council/schemas/verdict.schema.json"
  "${CODEX_SKILLS_DIR}/council/schemas/worker-result.schema.json"
  "${CODEX_ALLOWLIST}"
  "${CODEX_HOOKS_CONFIG}"
)

managed_paths=()
if [ "${INSTALL_CLAUDE}" = 1 ]; then
  managed_paths+=("${claude_managed_paths[@]}")
fi
if [ "${INSTALL_CODEX}" = 1 ]; then
  managed_paths+=("${codex_managed_paths[@]}")
fi

if [ "${UNINSTALL}" = 1 ]; then
  log "uninstalling knowledge-system files (${INSTALLER_VERSION}, agent=${AGENT}, scope=${SCOPE})"
  for path in "${managed_paths[@]}"; do
    remove_file "$path"
  done
  if [ "${INSTALL_CLAUDE}" = 1 ]; then
    remove_file "${MANIFEST}"
  fi
  if [ "${INSTALL_CODEX}" = 1 ]; then
    remove_file "${CODEX_MANIFEST}"
  fi
  log "done"
  exit 0
fi

# -----------------------------------------------------------------
# Spec Kit Claude commands
# -----------------------------------------------------------------
# Spec Kit Claude commands — generated by render-agent-kit.py from repo templates.
read -r -d '' SPECKIT_COMMAND_speckit_analyze_md <<'SPECKIT_COMMAND_speckit_analyze_md_EOF' || true
---
description: Analyze consistency across the active spec, plan, and tasks.
---

## User Input

```text
$ARGUMENTS
```

Use the input as analysis focus if provided.

## Bootstrap

If `.specify` is absent, scaffold `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add a minimal
`.specify/scripts/bash/check-prerequisites.sh` if missing. It must support
`--json --require-tasks --include-tasks` and return `FEATURE_DIR` plus available
docs. Do not overwrite existing scripts or templates.

## Procedure

1. Run `.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks`
   from the repo root. Stop if `spec.md`, `plan.md`, or `tasks.md` is missing.
2. Load `FEATURE_DIR/spec.md`, `FEATURE_DIR/plan.md`,
   `FEATURE_DIR/tasks.md`, `.specify/memory/constitution.md` if present, and
   referenced supporting docs.
3. Perform a read-only analysis. Do not modify files.
4. Report only actionable findings:
   - Missing task coverage for requirements or success criteria.
   - Plan decisions that conflict with the spec or constitution.
   - Tasks that introduce scope not present in the spec.
   - Duplicated, contradictory, vague, or unordered tasks.
   - Acceptance criteria that cannot be verified by the task list.
5. Order findings by severity and include file/section references. If no issues
   are found, say so and note residual risks.
6. Recommend the next command: refine with `/speckit.specify`,
   `/speckit.plan`, `/speckit.tasks`, or proceed to `/speckit.implement`.
SPECKIT_COMMAND_speckit_analyze_md_EOF
read -r -d '' SPECKIT_COMMAND_speckit_checklist_md <<'SPECKIT_COMMAND_speckit_checklist_md_EOF' || true
---
description: Generate a focused quality checklist for the active specification.
---

## User Input

```text
$ARGUMENTS
```

Use the input as the checklist domain or focus. If empty, infer the most useful
quality checklist from the spec.

## Bootstrap

If `.specify` is absent, scaffold `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add
`.specify/templates/checklist-template.md` and
`.specify/scripts/bash/check-prerequisites.sh` only when missing. The script
must locate the active feature from `.specify/feature.json` and print JSON with
`FEATURE_DIR`. Do not overwrite existing Spec Kit scripts or templates.

## Procedure

1. Run `.specify/scripts/bash/check-prerequisites.sh --json` from the repo root.
   Stop if no active feature or spec exists.
2. Load `FEATURE_DIR/spec.md`, `FEATURE_DIR/plan.md` if present, and
   `.specify/memory/constitution.md` if present.
3. Create `FEATURE_DIR/checklists/<focus>.md` using
   `.specify/templates/checklist-template.md` when available. Choose a concise
   file name from the focus, such as `security.md`, `accessibility.md`, or
   `requirements.md`.
4. Write checklist items as validation questions for requirements completeness,
   clarity, consistency, and acceptance readiness. Do not turn the checklist
   into implementation tasks.
5. If the user asked to validate, mark items complete only when the current
   artifacts satisfy them; otherwise leave them unchecked for review.
6. Report the checklist path and any high-risk gaps found while generating it.
SPECKIT_COMMAND_speckit_checklist_md_EOF
read -r -d '' SPECKIT_COMMAND_speckit_clarify_md <<'SPECKIT_COMMAND_speckit_clarify_md_EOF' || true
---
description: Clarify underspecified feature requirements before technical planning.
---

## User Input

```text
$ARGUMENTS
```

Use the input as clarification focus if provided.

## Bootstrap

If `.specify` is absent, scaffold `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add a minimal
`.specify/scripts/bash/check-prerequisites.sh` if missing. It must read
`.specify/feature.json`, print JSON with `FEATURE_DIR` and available docs, and
honor required-file checks. Do not overwrite existing scripts or templates.

## Procedure

1. Run `.specify/scripts/bash/check-prerequisites.sh --json` from the repo root.
   Stop if no active feature or `spec.md` is found; tell the user to run
   `/speckit.specify` first.
2. Load `FEATURE_DIR/spec.md` and `.specify/memory/constitution.md` if present.
3. Identify ambiguities that block planning. Prioritize scope, actor and
   permission boundaries, data lifecycle, compliance, failure behavior, and user
   experience. Ignore implementation choices unless the spec already leaked
   them into requirements.
4. Ask up to five concise questions. Prefer multiple choice when the tradeoff is
   bounded; allow custom answers when needed. Ask only questions whose answers
   would materially change the spec or plan.
5. Update `FEATURE_DIR/spec.md` with a `## Clarifications` section dated today.
   Record each question and answer, then revise affected requirements so the
   body is unambiguous.
6. Report how many clarifications were added and whether the feature is ready
   for `/speckit.plan`.
SPECKIT_COMMAND_speckit_clarify_md_EOF
read -r -d '' SPECKIT_COMMAND_speckit_constitution_md <<'SPECKIT_COMMAND_speckit_constitution_md_EOF' || true
---
description: Create or update the Spec Kit project constitution.
---

## User Input

```text
$ARGUMENTS
```

You MUST consider the user input before proceeding.

## Bootstrap

If `.specify` is absent, scaffold the minimal Spec Kit runtime before changing
the constitution:

- Create `.specify/memory`, `.specify/templates`, `.specify/scripts/bash`, and
  `specs`.
- Add missing template files only when absent:
  `.specify/templates/constitution-template.md`,
  `.specify/templates/spec-template.md`,
  `.specify/templates/plan-template.md`,
  `.specify/templates/tasks-template.md`, and
  `.specify/templates/checklist-template.md`.
- Add missing helper scripts only when absent:
  `.specify/scripts/bash/check-prerequisites.sh`,
  `.specify/scripts/bash/create-new-feature.sh`,
  `.specify/scripts/bash/setup-plan.sh`,
  `.specify/scripts/bash/setup-tasks.sh`, and
  `.specify/scripts/bash/update-agent-context.sh`.
- The minimal scripts must support the core paths used by these commands:
  active feature lookup from `.specify/feature.json`, JSON output, and required
  file checks. Do not overwrite existing Spec Kit scripts or templates.

## Procedure

1. Load `.specify/templates/constitution-template.md` if present and load the
   current `.specify/memory/constitution.md` if it already exists.
2. Extract governing principles, testing expectations, delivery workflow, and
   review rules from the user input. If updating an existing constitution,
   preserve valid principles unless the user explicitly replaces them.
3. Write `.specify/memory/constitution.md` with concrete principles, rationale,
   governance, amendment rules, and version/date metadata. Do not leave template
   placeholders unresolved.
4. Review `.specify/templates/spec-template.md`,
   `.specify/templates/plan-template.md`, and
   `.specify/templates/tasks-template.md` for obvious constitution references.
   Note any template updates that should be made by a separate command if they
   are outside this command's requested scope.
5. Report the constitution path, whether it was created or updated, and the
   next recommended phase (`/speckit.specify`).
SPECKIT_COMMAND_speckit_constitution_md_EOF
read -r -d '' SPECKIT_COMMAND_speckit_implement_md <<'SPECKIT_COMMAND_speckit_implement_md_EOF' || true
---
description: Implement the active Spec Kit task list.
---

## User Input

```text
$ARGUMENTS
```

Use the input as implementation focus if provided.

## Bootstrap

If `.specify` is absent, scaffold `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add a minimal
`.specify/scripts/bash/check-prerequisites.sh` if missing. It must support
`--json --require-tasks --include-tasks` and return `FEATURE_DIR`,
`SPEC_FILE`, `PLAN_FILE`, and `TASKS_FILE`. Do not overwrite existing Spec Kit
scripts or templates.

## Procedure

1. Run `.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks`
   from the repo root. Stop if `spec.md`, `plan.md`, or `tasks.md` is missing.
2. Load the active spec, plan, tasks, constitution, and supporting docs. Treat
   `tasks.md` as the execution source of truth.
3. Parse incomplete tasks in order. Respect dependencies and phase boundaries.
   `[P]` tasks may be run in parallel only when they touch independent files and
   the environment allows it.
4. Implement one coherent task or small dependency group at a time. Keep scope
   limited to the task list and do not add new product behavior absent from the
   spec.
5. Mark each task complete in `tasks.md` after its implementation and relevant
   validation pass.
6. Run the smallest meaningful tests described by the plan, task, or repository
   conventions. If a test cannot run, report the exact blocker.
7. Stop and report if the plan/spec/tasks conflict, a task is not actionable, or
   implementation would require a decision not captured in the spec.
8. Finish with completed task ids, files changed, validation results, and any
   remaining incomplete tasks.
SPECKIT_COMMAND_speckit_implement_md_EOF
read -r -d '' SPECKIT_COMMAND_speckit_plan_md <<'SPECKIT_COMMAND_speckit_plan_md_EOF' || true
---
description: Create a technical implementation plan for the active specification.
---

## User Input

```text
$ARGUMENTS
```

Treat the input as the user's technical preferences and constraints.

## Bootstrap

If `.specify` is absent, scaffold `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add missing
`.specify/scripts/bash/check-prerequisites.sh`,
`.specify/scripts/bash/setup-plan.sh`, and
`.specify/scripts/bash/update-agent-context.sh` only when absent. The minimal
`setup-plan.sh` must locate the active feature, copy
`.specify/templates/plan-template.md` to `FEATURE_DIR/plan.md` if needed, and
print JSON with `FEATURE_DIR`, `SPEC_FILE`, and `PLAN_FILE`. Do not overwrite
existing Spec Kit scripts or templates.

## Procedure

1. Run `.specify/scripts/bash/setup-plan.sh --json` from the repo root. Stop if
   the active feature or spec is missing; tell the user to run
   `/speckit.specify` first.
2. Load `SPEC_FILE`, `PLAN_FILE`, `.specify/memory/constitution.md`, and any
   existing docs under `FEATURE_DIR`.
3. Fill `FEATURE_DIR/plan.md` with implementation context, constraints,
   constitution compliance, project structure, and phase gates. Use the user's
   input for stack and architecture choices.
4. Produce supporting docs when relevant:
   `FEATURE_DIR/research.md`, `FEATURE_DIR/data-model.md`,
   `FEATURE_DIR/contracts/`, and `FEATURE_DIR/quickstart.md`.
5. Keep the plan consistent with the spec. Do not add feature scope that the
   spec did not request; send scope changes back to `/speckit.specify`.
6. Run `.specify/scripts/bash/update-agent-context.sh` if present so agent
   context reflects the chosen stack.
7. Report created/updated files, unresolved risks, and readiness for
   `/speckit.tasks`.
SPECKIT_COMMAND_speckit_plan_md_EOF
read -r -d '' SPECKIT_COMMAND_speckit_specify_md <<'SPECKIT_COMMAND_speckit_specify_md_EOF' || true
---
description: Create or update a feature specification from a natural language description.
---

## User Input

```text
$ARGUMENTS
```

You MUST consider the user input before proceeding. If it is empty, stop and ask
for the feature description.

## Bootstrap

If `.specify` is absent, scaffold the minimal Spec Kit runtime before creating
the spec:

- Create `.specify/memory`, `.specify/templates`, `.specify/scripts/bash`, and
  `specs`.
- Add missing templates only when absent, especially
  `.specify/templates/spec-template.md` and
  `.specify/templates/checklist-template.md`.
- Add missing helper scripts only when absent:
  `.specify/scripts/bash/create-new-feature.sh` must create a single
  `specs/<number>-<short-name>/spec.md`, copy the spec template, write
  `.specify/feature.json`, and print JSON with `FEATURE_DIR` and `SPEC_FILE`.
  `.specify/scripts/bash/check-prerequisites.sh` must locate the active feature
  from `.specify/feature.json`.
- Do not overwrite existing Spec Kit scripts or templates.

## Procedure

1. Generate a concise 2-4 word feature short name from the user description.
   Prefer action-noun names such as `add-user-auth` or `analytics-dashboard`.
2. Run `.specify/scripts/bash/create-new-feature.sh --json "$ARGUMENTS"` if the
   script supports it. Otherwise use the minimal script behavior described in
   Bootstrap. Capture `FEATURE_DIR` and `SPEC_FILE`.
3. Load `.specify/templates/spec-template.md` and, if present,
   `.specify/memory/constitution.md`.
4. Write `FEATURE_DIR/spec.md` from the user's description:
   - Focus on what users need and why.
   - Avoid implementation details, tech stacks, APIs, libraries, and code
     structure.
   - Include user scenarios, functional requirements, success criteria,
     assumptions, edge cases, and key entities when relevant.
   - Use at most three `[NEEDS CLARIFICATION: ...]` markers, only for decisions
     that materially affect scope, security/privacy, or user experience and have
     no reasonable default.
5. Create `FEATURE_DIR/checklists/requirements.md` from the checklist template.
   Validate the spec against completeness, testability, measurable success
   criteria, bounded scope, and absence of implementation details. Iterate the
   spec up to three times for fixable failures.
6. If clarification markers remain, present all questions together with options
   and wait for the user's answers before finalizing.
7. Report `FEATURE_DIR`, `SPEC_FILE`, checklist status, and readiness for
   `/speckit.clarify` or `/speckit.plan`.
SPECKIT_COMMAND_speckit_specify_md_EOF
read -r -d '' SPECKIT_COMMAND_speckit_tasks_md <<'SPECKIT_COMMAND_speckit_tasks_md_EOF' || true
---
description: Generate an actionable task list from the active implementation plan.
---

## User Input

```text
$ARGUMENTS
```

Use the input as task-generation focus if provided.

## Bootstrap

If `.specify` is absent, scaffold `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add missing
`.specify/scripts/bash/check-prerequisites.sh` and
`.specify/scripts/bash/setup-tasks.sh` only when absent. The minimal
`setup-tasks.sh` must require `spec.md` and `plan.md`, create
`FEATURE_DIR/tasks.md` from `.specify/templates/tasks-template.md` if needed,
and print JSON with `FEATURE_DIR`, `PLAN_FILE`, and `TASKS_FILE`. Do not
overwrite existing Spec Kit scripts or templates.

## Procedure

1. Run `.specify/scripts/bash/setup-tasks.sh --json` from the repo root. Stop if
   `spec.md` or `plan.md` is missing; tell the user which Speckit command to run
   first.
2. Load `FEATURE_DIR/spec.md`, `FEATURE_DIR/plan.md`, and available supporting
   docs (`research.md`, `data-model.md`, `contracts/`, `quickstart.md`).
3. Generate `FEATURE_DIR/tasks.md` as executable Markdown:
   - Number tasks `T001`, `T002`, and so on.
   - Group by setup, foundations, user story phases, polish, and validation.
   - Mark independent tasks with `[P]`.
   - Include exact file paths where practical.
   - If tests are required by the spec, constitution, or plan, put test tasks
     before implementation tasks for the same behavior.
   - Keep each user story independently implementable and testable.
4. Add dependency notes and parallel execution examples when they help execute
   the plan.
5. Report task count, parallelizable count, and readiness for
   `/speckit.analyze`, `/speckit.taskstoissues`, or `/speckit.implement`.
SPECKIT_COMMAND_speckit_tasks_md_EOF
read -r -d '' SPECKIT_COMMAND_speckit_taskstoissues_md <<'SPECKIT_COMMAND_speckit_taskstoissues_md_EOF' || true
---
description: Create GitHub issues from the active Spec Kit tasks list.
---

## User Input

```text
$ARGUMENTS
```

Recognize `--dry-run`. In dry-run mode, print the `gh issue create` commands
that would run and do not create issues.

## Bootstrap

If `.specify` is absent, scaffold `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add a minimal
`.specify/scripts/bash/check-prerequisites.sh` if missing. It must support
`--json --require-tasks --include-tasks` and return `FEATURE_DIR` plus available
docs. Do not overwrite existing scripts or templates.

## Procedure

1. Run `.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks`
   from the repo root. Stop if `FEATURE_DIR/tasks.md` is missing.
2. Verify `gh` is available and authenticated unless this is `--dry-run`.
3. Read `FEATURE_DIR/tasks.md` and create one issue per task line that starts
   with a Markdown task checkbox and contains a `T###` task id.
4. Use titles in this format: `[SpecKit] <feature-dir-name>: <task-id> <task>`.
5. Before creating each issue, check all repository issues by exact title. If an
   issue with the same title already exists, warn and skip it.
6. Assign every created issue with `--assignee "$assignee"` from
   `AGENT_KIT_GH_ASSIGNEE`.
7. Pick exactly one best-fit label from the existing repository labels:
   `bug`, `documentation`, or `enhancement`.
   - Use `bug` for tasks about bugs, fixes, regressions, failures, or defects.
   - Use `documentation` for docs, README, runbook, guide, or content tasks.
   - Otherwise use `enhancement`.
   - Never invent labels. If the chosen label does not exist in the repository,
     warn and omit `--label`.
8. Use this helper shape inline; do not add scripts outside this command:

```bash
dry_run=false
case " $ARGUMENTS " in *" --dry-run "*) dry_run=true ;; esac
assignee="${AGENT_KIT_GH_ASSIGNEE:?set AGENT_KIT_GH_ASSIGNEE to the GitHub login that should own generated issues}"

feature_dir="<FEATURE_DIR from check-prerequisites>"
tasks_file="$feature_dir/tasks.md"
feature_name="$(basename "$feature_dir")"

label_exists() {
  gh label list --limit 1000 --json name --jq '.[].name' | grep -Fxq "$1"
}

issue_exists() {
  gh issue list --state all --search "$1 in:title" --json title --jq '.[].title' | grep -Fxq "$1"
}

while IFS= read -r task_line; do
  task_id="$(printf '%s\n' "$task_line" | grep -Eo 'T[0-9]{3,}' | head -n 1)"
  task_text="$(printf '%s\n' "$task_line" |
    sed -E 's/^- \[[ xX]\][[:space:]]*//; s/\[P\][[:space:]]*//g; s/T[0-9]{3,}[[:space:]]*//; s/^[[:space:]]+//')"
  [ -n "$task_id" ] || continue
  [ -n "$task_text" ] || continue

  title="[SpecKit] ${feature_name}: ${task_id} ${task_text}"
  body="$(cat <<EOF
## Task

${task_line}

## Source

- Feature: ${feature_dir}
- Tasks: ${tasks_file}
EOF
)"

  lower="$(printf '%s\n' "$task_text" | tr '[:upper:]' '[:lower:]')"
  label="enhancement"
  case "$lower" in
    *bug*|*fix*|*regression*|*failure*|*defect*) label="bug" ;;
    *doc*|*docs*|*documentation*|*readme*|*runbook*|*guide*) label="documentation" ;;
  esac

  label_args=()
  if label_exists "$label"; then
    label_args=(--label "$label")
  else
    printf 'WARN: label %s does not exist; omitting --label for %s\n' "$label" "$title" >&2
  fi

  if issue_exists "$title"; then
    printf 'WARN: issue already exists, skipping: %s\n' "$title" >&2
    continue
  fi

  if [ "$dry_run" = true ]; then
    printf 'gh issue create --title %q --body %q --assignee %q' "$title" "$body" "$assignee"
    [ "${#label_args[@]}" -eq 0 ] || printf ' --label %q' "$label"
    printf '\n'
  else
    gh issue create --title "$title" --body "$body" --assignee "$assignee" "${label_args[@]}"
  fi
done < <(grep -E '^- \[[ xX]\].*T[0-9]{3,}' "$tasks_file")
```

9. Report created, skipped, and dry-run counts.
SPECKIT_COMMAND_speckit_taskstoissues_md_EOF
if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${COMMANDS_DIR}/speckit.analyze.md" 0644 "${SPECKIT_COMMAND_speckit_analyze_md}"
  write_file "${COMMANDS_DIR}/speckit.checklist.md" 0644 "${SPECKIT_COMMAND_speckit_checklist_md}"
  write_file "${COMMANDS_DIR}/speckit.clarify.md" 0644 "${SPECKIT_COMMAND_speckit_clarify_md}"
  write_file "${COMMANDS_DIR}/speckit.constitution.md" 0644 "${SPECKIT_COMMAND_speckit_constitution_md}"
  write_file "${COMMANDS_DIR}/speckit.implement.md" 0644 "${SPECKIT_COMMAND_speckit_implement_md}"
  write_file "${COMMANDS_DIR}/speckit.plan.md" 0644 "${SPECKIT_COMMAND_speckit_plan_md}"
  write_file "${COMMANDS_DIR}/speckit.specify.md" 0644 "${SPECKIT_COMMAND_speckit_specify_md}"
  write_file "${COMMANDS_DIR}/speckit.tasks.md" 0644 "${SPECKIT_COMMAND_speckit_tasks_md}"
  write_file "${COMMANDS_DIR}/speckit.taskstoissues.md" 0644 "${SPECKIT_COMMAND_speckit_taskstoissues_md}"
fi

# -----------------------------------------------------------------
# Skill: topics
# -----------------------------------------------------------------
read -r -d '' TOPICS_SKILL <<'SKILL' || true
---
name: topics
description: Inspect the knowledge-base topic vocabulary before capturing or recalling. Use proactively when about to assign a scope or pick a tag — the closed-vocabulary slugs change over time and an incorrect slug routes captures to _inbox/_needs-review/.
---

# Topics + tags discovery

Three MCP tools surface what the knowledge base already knows:

- `knowledge.list_topics` — every topic slug in use, with note count
  + last-captured-at. Sort by note_count desc by default. Use before
  picking a `topic:<slug>` scope so a new capture lands on the
  existing vocabulary instead of forking a near-duplicate.
- `knowledge.topic_stats(slug)` — per-topic aggregate: count,
  capture window, type breakdown, top tags. Use to decide whether a
  topic is well-populated enough to capture into or whether to merge
  it with a more active neighbour.
- `knowledge.list_tags(scope?)` — tag frequency, optional scope
  filter. Use before tagging a new capture so the spelling matches
  existing tags (`kotlin` vs `Kotlin` vs `kt`).

When in doubt about which slug to use, prefer the one with the
highest note_count among plausible candidates. If none fit, capture
without scope — the curator's classifier will assign one against
the closed vocabulary.
SKILL

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${SKILLS_DIR}/topics/SKILL.md" 0644 "${TOPICS_SKILL}"
fi

# -----------------------------------------------------------------
# Skill: audit
# -----------------------------------------------------------------
read -r -d '' AUDIT_SKILL <<'SKILL' || true
---
name: audit
description: Audit the knowledge base for drift — pending inbox notes, near-duplicate tags, near-duplicate topic slugs. Use periodically (weekly is plenty) or when capture quality feels off.
---

# Knowledge-base audit

Three checks, each a single MCP call:

1. `knowledge.list_inbox(limit=20)` — notes the worker captured but
   the curator hasn't classified yet. A persistent backlog signals
   that Ollama is wedged or the classifier is rejecting too much.
2. `knowledge.list_tags(limit=100)` — scan for near-duplicate
   spellings (`kotlin` / `Kotlin` / `kt`, `ci` / `CI` / `ci-cd`).
   Propose `knowledge.rename_tag(from, to)` for the cleanups, but
   don't run them — those are admin-only mutations.
3. `knowledge.list_topics(limit=100)` — flag topics with note_count
   of 1 or 2 (thin) and pairs of slugs that look like duplicates.
   Propose `knowledge.merge_topics(from_slug, into_slug)` for the
   candidates.

Report findings in three short sections. Don't mutate anything —
the operator runs the proposed merges / renames manually after
review.
SKILL

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${SKILLS_DIR}/audit/SKILL.md" 0644 "${AUDIT_SKILL}"
fi

# -----------------------------------------------------------------
# Skill: kb-first
# -----------------------------------------------------------------
read -r -d '' KB_FIRST_SKILL <<'SKILL' || true
---
name: kb-first
description: Use before designing or changing behavior that may depend on prior knowledge-base captures, repo history, architecture decisions, cluster state, agent conventions, or remembered lessons. Also use near task completion to capture durable lessons or decisions without dumping large KB context.
---

# KB First

Use the KB as a small retrieval layer, not as a large context dump.

1. Distill the task into a short recall query: nouns, service names,
   file names, and the decision being made.
2. Call `knowledge.recall` with `limit <= 5`. Prefer
   `scope=project:personal-stack` for repo behavior, `topic:<slug>`
   for general framework/tool facts, or omit scope for the curated
   default.
3. Choose the right mode:
   - `fast` — short/trivial lookups or latency-sensitive (< 80 chars).
   - `hybrid` — normal work; FTS + vector + RRF.
   - `deep` — only after fast or hybrid misses something important.
4. Read only what is needed. Usually snippets are enough. If a hit
   matters, call `knowledge.relations(id, depth=1)` before fetching
   the full note.
5. Filter mentally: hits with scores below 0.01 are rarely useful —
   treat as no match and continue from repo/source inspection.
6. If the KB has no useful context, continue from repo/source
   inspection and say the KB had no relevant hits.

Capture at the end only when the information is durable and reusable:
implementation pitfalls, verified behavior, operational runbooks,
architecture/process choices, or ambiguity that needs operator judgment.
Keep captures compact. Do not capture secrets, raw logs, full diffs, or
entire transcripts.

Never run broad `scope=all` recall as a first step. Use it only after
targeted recall fails and the task genuinely needs cross-scope context.
SKILL

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${SKILLS_DIR}/kb-first/SKILL.md" 0644 "${KB_FIRST_SKILL}"
fi

# -----------------------------------------------------------------
# Skill: token-economy
# -----------------------------------------------------------------
read -r -d '' TOKEN_ECONOMY_SKILL <<'SKILL' || true
---
name: token-economy
description: Use when the user asks to reduce token usage, agent cost, context bloat, prompt-caching misses, RAG/LightRAG behavior, memory policies, or durable instructions. Also use when installing many skills or designing automatic KB recall so retrieval stays bounded.
---

# Token Economy

- Keep stable instructions in `CLAUDE.md` or skills; keep volatile facts
  in the KB and retrieve them on demand.
- Prefer progressive disclosure: list/search first, open small file
  ranges next, fetch full files or notes only when needed.
- Keep recall bounded: default to `limit=3` for hook-injected context and
  `limit <= 5` for manual task setup.
- Use adaptive recall mode: `fast` for prompts under 80 chars, `hybrid`
  for normal work, `deep` only after a miss or non-obvious cross-topic
  dependency.
- Keep runner MCP profiles narrow: `minimal` by default, wider profiles
  only when the task needs those extra tools.
- Do not install or enable low-fit skills just to grow the list. Skill
  metadata itself consumes prompt budget and very large skill sets can
  hide useful skills.
- Preserve prompt-cache-friendly ordering when writing durable
  instructions: stable policy first, dynamic task-specific context later.

When reporting command results, summarize only the lines needed to
support the decision. Session digests should capture only reusable
lessons above a confidence floor and should dedupe against existing KB
hits before writing.

## Tunable env vars

| Variable | Default | Effect |
| --- | --- | --- |
| `KB_RECALL_MIN_SCORE` | `0.004` | Minimum hit score injected into context; raise to tighten relevance. |
| `KB_RECALL_HOOK_LIMIT` | `3` | Max recall hits per hook invocation. |
| `KB_RECALL_HOOK_MODE` | auto | Override adaptive mode (`fast`/`hybrid`/`deep`). |
| `KB_DIGEST_MAX_CHARS` | `30000` | Transcript chars fed to the stop-digest hook; lower = cheaper. |
| `KB_DIGEST_MAX_CAPTURES` | `4` | Per-session capture cap for the stop hook. |
| `KB_AUTO_MCP_DISABLED` | `0` | Set to `1` to disable all automatic KB calls (panic switch). |
SKILL

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${SKILLS_DIR}/token-economy/SKILL.md" 0644 "${TOKEN_ECONOMY_SKILL}"
fi

# -----------------------------------------------------------------
# Skill: agent-session-bootstrap
# -----------------------------------------------------------------
read -r -d '' AGENT_SESSION_BOOTSTRAP_SKILL <<'SKILL' || true
---
name: agent-session-bootstrap
description: Use when configuring Claude Code or Codex sessions, hooks, skills, MCP servers, durable instructions, agent runners, or future-session defaults. Ensures KB recall/capture and token-efficient behavior are installed without relying on user reminders.
---

# Agent Session Bootstrap

Checklist:

1. Locate the active user and project config layers:
   `~/.claude/settings.json`, `~/.claude/CLAUDE.md`, project
   `CLAUDE.md`, project `.claude/settings.json`, project `.claude/hooks`,
   `~/.claude/skills`, Codex `~/.codex/config.toml`, `~/.codex/hooks.json`,
   repo `AGENTS.md`, and `.agents/skills`.
2. Ensure the `knowledge` MCP server is configured and uses
   `KB_BEARER_TOKEN` rather than an inline secret where possible.
3. Keep runner MCP profiles narrow:
   `minimal` for routine work, and `frontend`, `cluster`, `code-intel`,
   or `full-diagnostic` only when the task needs those tools. Prefer
   `AGENT_MCP_PROFILE` for one runner and
   `AGENT_RUNTIME_DEFAULT_MCP_PROFILE` only for fleet-wide default changes.
4. Register bounded automatic hooks:
   `PreToolUse` edit recall deduped per session and `Stop` transcript
   digest with a per-session capture cap. Prompt-level KB recall stays
   on demand through the recall skill or `knowledge.recall` MCP tool.
5. Keep hooks silent on KB failure and add `KB_AUTO_MCP_DISABLED=1` as
   a panic switch.
6. Add or update memory files so future sessions know to consult and
   update the KB without user reminders.
7. Validate with dry-run hook payloads and at least one `tools/list` or
   `knowledge.recall` MCP call.

Every Codex project skill, hook, or durable instruction must have an
equivalent Claude implementation in the same branch. Treat Codex-only
`.agents`/`.codex` files as incomplete until `.claude`/`CLAUDE.md`/
installer parity exists.

Do not put bearer tokens, secrets, or full transcripts into committed
files.
SKILL

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${SKILLS_DIR}/agent-session-bootstrap/SKILL.md" 0644 "${AGENT_SESSION_BOOTSTRAP_SKILL}"
fi

# -----------------------------------------------------------------
# Spec Kit Codex skills
# -----------------------------------------------------------------
# Spec Kit Codex skills — generated by render-agent-kit.py from repo templates.
read -r -d '' CODEX_SPECKIT_speckit_analyze_SKILL_md <<'CODEX_SPECKIT_speckit_analyze_SKILL_md_EOF' || true
---
name: speckit-analyze
description: Analyze consistency across the active spec, plan, and tasks.
---

# Speckit Analyze

Use this skill for the `$speckit-analyze` phase.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add a minimal
`.specify/scripts/bash/check-prerequisites.sh` only when absent. It must support
`--json --require-tasks --include-tasks`.

## Workflow

1. Run `.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks`.
   Stop if `spec.md`, `plan.md`, or `tasks.md` is missing.
2. Read the spec, plan, tasks, constitution if present, and referenced
   supporting docs.
3. Perform read-only analysis. Do not modify files.
4. Report missing requirement coverage, spec-plan conflicts, task scope creep,
   duplicated or vague tasks, ordering problems, and unverifiable acceptance
   criteria.
5. Order findings by severity with file or section references. If no issues are
   found, say so and identify residual risk.
6. Recommend the next Speckit command.
CODEX_SPECKIT_speckit_analyze_SKILL_md_EOF
read -r -d '' CODEX_SPECKIT_speckit_checklist_SKILL_md <<'CODEX_SPECKIT_speckit_checklist_SKILL_md_EOF' || true
---
name: speckit-checklist
description: Generate a focused quality checklist for the active specification.
---

# Speckit Checklist

Use this skill for the `$speckit-checklist` phase. Treat the user's prompt as
the checklist focus; infer a useful focus if none is provided.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add
`.specify/templates/checklist-template.md` and
`.specify/scripts/bash/check-prerequisites.sh` only when missing.

## Workflow

1. Run `.specify/scripts/bash/check-prerequisites.sh --json`. Stop if no active
   feature or spec exists.
2. Read the active spec, plan if present, and constitution if present.
3. Create `FEATURE_DIR/checklists/<focus>.md` from the checklist template when
   available.
4. Write validation questions for requirements completeness, clarity,
   consistency, and acceptance readiness. Do not write implementation tasks.
5. Mark items complete only when the user asked for validation and the current
   artifacts satisfy the item.
6. Report the checklist path and high-risk gaps.
CODEX_SPECKIT_speckit_checklist_SKILL_md_EOF
read -r -d '' CODEX_SPECKIT_speckit_clarify_SKILL_md <<'CODEX_SPECKIT_speckit_clarify_SKILL_md_EOF' || true
---
name: speckit-clarify
description: Clarify underspecified feature requirements before technical planning.
---

# Speckit Clarify

Use this skill for the `$speckit-clarify` phase.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add a minimal
`.specify/scripts/bash/check-prerequisites.sh` only when absent. It must read
`.specify/feature.json`, print JSON with `FEATURE_DIR`, and honor required-file
checks.

## Workflow

1. Run `.specify/scripts/bash/check-prerequisites.sh --json`. Stop if no active
   feature or `spec.md` exists.
2. Read `FEATURE_DIR/spec.md` and the constitution if present.
3. Identify planning-blocking ambiguities in scope, actors, permissions, data,
   compliance, failure behavior, and user experience.
4. Ask up to five concise questions, using multiple choice where possible.
5. Add a dated `## Clarifications` section to the spec, record answers, and
   revise affected requirements.
6. Report clarification count and readiness for `$speckit-plan`.
CODEX_SPECKIT_speckit_clarify_SKILL_md_EOF
read -r -d '' CODEX_SPECKIT_speckit_constitution_SKILL_md <<'CODEX_SPECKIT_speckit_constitution_SKILL_md_EOF' || true
---
name: speckit-constitution
description: Create or update the Spec Kit project constitution.
---

# Speckit Constitution

Use this skill for the `$speckit-constitution` phase.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add missing templates and minimal helper
scripts under `.specify/scripts/bash/` only when absent:
`check-prerequisites.sh`, `create-new-feature.sh`, `setup-plan.sh`,
`setup-tasks.sh`, and `update-agent-context.sh`. The helpers must support
active feature lookup from `.specify/feature.json`, JSON output, and required
file checks. Do not overwrite existing Spec Kit files.

## Workflow

1. Read `.specify/templates/constitution-template.md` and any existing
   `.specify/memory/constitution.md`.
2. Derive concrete project principles, testing standards, delivery workflow,
   review rules, governance, and amendment rules from the user request.
3. Write `.specify/memory/constitution.md` with no unresolved placeholders.
4. Check the spec, plan, and tasks templates for obvious constitution alignment
   notes, but do not make unrelated edits.
5. Report the constitution path and readiness for `$speckit-specify`.
CODEX_SPECKIT_speckit_constitution_SKILL_md_EOF
read -r -d '' CODEX_SPECKIT_speckit_implement_SKILL_md <<'CODEX_SPECKIT_speckit_implement_SKILL_md_EOF' || true
---
name: speckit-implement
description: Implement the active Spec Kit task list.
---

# Speckit Implement

Use this skill for the `$speckit-implement` phase.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add a minimal
`.specify/scripts/bash/check-prerequisites.sh` only when absent. It must support
`--json --require-tasks --include-tasks` and return `FEATURE_DIR`,
`SPEC_FILE`, `PLAN_FILE`, and `TASKS_FILE`.

## Workflow

1. Run `.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks`.
   Stop if the spec, plan, or tasks file is missing.
2. Read the active spec, plan, tasks, constitution, and supporting docs.
3. Execute incomplete tasks from `tasks.md` in dependency order. Run `[P]` tasks
   in parallel only when they touch independent files and the environment allows
   it.
4. Keep implementation scope limited to the task list and active spec.
5. Mark tasks complete in `tasks.md` after implementation and relevant
   validation.
6. Run the smallest meaningful tests required by the task, plan, or repository
   conventions. Report exact blockers for checks that cannot run.
7. Stop for conflicts, unactionable tasks, or decisions missing from the spec.
8. Finish with completed task ids, changed files, validation results, and
   remaining incomplete tasks.
CODEX_SPECKIT_speckit_implement_SKILL_md_EOF
read -r -d '' CODEX_SPECKIT_speckit_plan_SKILL_md <<'CODEX_SPECKIT_speckit_plan_SKILL_md_EOF' || true
---
name: speckit-plan
description: Create a technical implementation plan for the active specification.
---

# Speckit Plan

Use this skill for the `$speckit-plan` phase. Treat the user's prompt as
technical preferences and constraints.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add missing
`.specify/scripts/bash/check-prerequisites.sh`,
`.specify/scripts/bash/setup-plan.sh`, and
`.specify/scripts/bash/update-agent-context.sh` only when absent. The minimal
`setup-plan.sh` must locate the active feature, copy
`.specify/templates/plan-template.md` to `FEATURE_DIR/plan.md` if needed, and
print JSON with `FEATURE_DIR`, `SPEC_FILE`, and `PLAN_FILE`.

## Workflow

1. Run `.specify/scripts/bash/setup-plan.sh --json`. Stop if no active spec
   exists.
2. Read the spec, plan template or existing plan, constitution, and supporting
   docs in `FEATURE_DIR`.
3. Fill `FEATURE_DIR/plan.md` with implementation context, constraints,
   constitution compliance, project structure, and phase gates.
4. Create supporting docs when relevant: `research.md`, `data-model.md`,
   `contracts/`, and `quickstart.md`.
5. Keep scope aligned to the spec; send product-scope changes back to
   `$speckit-specify`.
6. Run `.specify/scripts/bash/update-agent-context.sh` if present.
7. Report updated files, unresolved risks, and readiness for `$speckit-tasks`.
CODEX_SPECKIT_speckit_plan_SKILL_md_EOF
read -r -d '' CODEX_SPECKIT_speckit_specify_SKILL_md <<'CODEX_SPECKIT_speckit_specify_SKILL_md_EOF' || true
---
name: speckit-specify
description: Create or update a feature specification from a natural language description.
---

# Speckit Specify

Use this skill for the `$speckit-specify` phase. The user's prompt is the
feature description; if it is empty, ask for the description before proceeding.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add missing
`.specify/templates/spec-template.md`,
`.specify/templates/checklist-template.md`,
`.specify/scripts/bash/create-new-feature.sh`, and
`.specify/scripts/bash/check-prerequisites.sh` only when absent. The feature
script must create one `specs/<number>-<short-name>/spec.md`, copy the spec
template, write `.specify/feature.json`, and print JSON with `FEATURE_DIR` and
`SPEC_FILE`.

## Workflow

1. Generate a concise 2-4 word feature short name.
2. Run `.specify/scripts/bash/create-new-feature.sh --json "<description>"` or
   the minimal equivalent from Bootstrap.
3. Read `.specify/templates/spec-template.md` and
   `.specify/memory/constitution.md` if present.
4. Write `FEATURE_DIR/spec.md` focused on what users need and why. Avoid
   implementation details. Include user scenarios, functional requirements,
   success criteria, assumptions, edge cases, and key entities when relevant.
5. Use at most three `[NEEDS CLARIFICATION: ...]` markers for critical unknowns
   with no reasonable default.
6. Create `FEATURE_DIR/checklists/requirements.md`, validate the spec, and
   iterate up to three times for fixable gaps.
7. Report `FEATURE_DIR`, `SPEC_FILE`, checklist status, and readiness for
   `$speckit-clarify` or `$speckit-plan`.
CODEX_SPECKIT_speckit_specify_SKILL_md_EOF
read -r -d '' CODEX_SPECKIT_speckit_tasks_SKILL_md <<'CODEX_SPECKIT_speckit_tasks_SKILL_md_EOF' || true
---
name: speckit-tasks
description: Generate an actionable task list from the active implementation plan.
---

# Speckit Tasks

Use this skill for the `$speckit-tasks` phase.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add missing
`.specify/scripts/bash/check-prerequisites.sh` and
`.specify/scripts/bash/setup-tasks.sh` only when absent. The minimal
`setup-tasks.sh` must require `spec.md` and `plan.md`, create
`FEATURE_DIR/tasks.md` from `.specify/templates/tasks-template.md` if needed,
and print JSON with `FEATURE_DIR`, `PLAN_FILE`, and `TASKS_FILE`.

## Workflow

1. Run `.specify/scripts/bash/setup-tasks.sh --json`. Stop if the spec or plan
   is missing.
2. Read the spec, plan, and supporting docs.
3. Generate `FEATURE_DIR/tasks.md` with `T001` style task ids, user-story
   phases, dependency notes, and `[P]` markers for independent parallel tasks.
4. Include exact file paths where practical. Put test tasks before
   implementation tasks whenever tests are required by the spec, constitution,
   or plan.
5. Keep each user story independently implementable and testable.
6. Report task count, parallelizable count, and readiness for
   `$speckit-analyze`, `$speckit-taskstoissues`, or `$speckit-implement`.
CODEX_SPECKIT_speckit_tasks_SKILL_md_EOF
read -r -d '' CODEX_SPECKIT_speckit_taskstoissues_SKILL_md <<'CODEX_SPECKIT_speckit_taskstoissues_SKILL_md_EOF' || true
---
name: speckit-taskstoissues
description: Create GitHub issues from the active Spec Kit tasks list.
---

# Speckit Tasks To Issues

Use this skill for the `$speckit-taskstoissues` phase. Support `--dry-run` by
printing the `gh issue create` commands without creating issues.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add a minimal
`.specify/scripts/bash/check-prerequisites.sh` only when absent. It must support
`--json --require-tasks --include-tasks` and return `FEATURE_DIR`.

## Workflow

1. Run `.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks`.
   Stop if `FEATURE_DIR/tasks.md` is missing.
2. Verify `gh` is installed and authenticated unless running `--dry-run`.
3. Parse task lines from `tasks.md` that start with a Markdown task checkbox and
   contain a `T###` task id.
4. Create one issue per task with title
   `[SpecKit] <feature-dir-name>: <task-id> <task>`.
5. Before creating, check all repository issues by exact title; warn and skip
   already-created issues.
6. Always include `--assignee "$assignee"` from `AGENT_KIT_GH_ASSIGNEE`.
7. Pick exactly one best-fit existing repo label from
   `enhancement`, `bug`, or `documentation`. Never invent labels; if the chosen
   label does not exist, warn and omit `--label`.
8. Use this inline helper shape; do not add a script file:

```bash
dry_run=false
case " $ARGUMENTS " in *" --dry-run "*) dry_run=true ;; esac
assignee="${AGENT_KIT_GH_ASSIGNEE:?set AGENT_KIT_GH_ASSIGNEE to the GitHub login that should own generated issues}"

feature_dir="<FEATURE_DIR from check-prerequisites>"
tasks_file="$feature_dir/tasks.md"
feature_name="$(basename "$feature_dir")"

label_exists() {
  gh label list --limit 1000 --json name --jq '.[].name' | grep -Fxq "$1"
}

issue_exists() {
  gh issue list --state all --search "$1 in:title" --json title --jq '.[].title' | grep -Fxq "$1"
}

while IFS= read -r task_line; do
  task_id="$(printf '%s\n' "$task_line" | grep -Eo 'T[0-9]{3,}' | head -n 1)"
  task_text="$(printf '%s\n' "$task_line" |
    sed -E 's/^- \[[ xX]\][[:space:]]*//; s/\[P\][[:space:]]*//g; s/T[0-9]{3,}[[:space:]]*//; s/^[[:space:]]+//')"
  [ -n "$task_id" ] || continue
  [ -n "$task_text" ] || continue

  title="[SpecKit] ${feature_name}: ${task_id} ${task_text}"
  body="$(cat <<EOF
## Task

${task_line}

## Source

- Feature: ${feature_dir}
- Tasks: ${tasks_file}
EOF
)"

  lower="$(printf '%s\n' "$task_text" | tr '[:upper:]' '[:lower:]')"
  label="enhancement"
  case "$lower" in
    *bug*|*fix*|*regression*|*failure*|*defect*) label="bug" ;;
    *doc*|*docs*|*documentation*|*readme*|*runbook*|*guide*) label="documentation" ;;
  esac

  label_args=()
  if label_exists "$label"; then
    label_args=(--label "$label")
  else
    printf 'WARN: label %s does not exist; omitting --label for %s\n' "$label" "$title" >&2
  fi

  if issue_exists "$title"; then
    printf 'WARN: issue already exists, skipping: %s\n' "$title" >&2
    continue
  fi

  if [ "$dry_run" = true ]; then
    printf 'gh issue create --title %q --body %q --assignee %q' "$title" "$body" "$assignee"
    [ "${#label_args[@]}" -eq 0 ] || printf ' --label %q' "$label"
    printf '\n'
  else
    gh issue create --title "$title" --body "$body" --assignee "$assignee" "${label_args[@]}"
  fi
done < <(grep -E '^- \[[ xX]\].*T[0-9]{3,}' "$tasks_file")
```

9. Report created, skipped, and dry-run counts.
CODEX_SPECKIT_speckit_taskstoissues_SKILL_md_EOF
if [ "${INSTALL_CODEX}" = 1 ]; then
  write_file "${CODEX_SKILLS_DIR}/speckit-analyze/SKILL.md" 0644 "${CODEX_SPECKIT_speckit_analyze_SKILL_md}"
  write_file "${CODEX_SKILLS_DIR}/speckit-checklist/SKILL.md" 0644 "${CODEX_SPECKIT_speckit_checklist_SKILL_md}"
  write_file "${CODEX_SKILLS_DIR}/speckit-clarify/SKILL.md" 0644 "${CODEX_SPECKIT_speckit_clarify_SKILL_md}"
  write_file "${CODEX_SKILLS_DIR}/speckit-constitution/SKILL.md" 0644 "${CODEX_SPECKIT_speckit_constitution_SKILL_md}"
  write_file "${CODEX_SKILLS_DIR}/speckit-implement/SKILL.md" 0644 "${CODEX_SPECKIT_speckit_implement_SKILL_md}"
  write_file "${CODEX_SKILLS_DIR}/speckit-plan/SKILL.md" 0644 "${CODEX_SPECKIT_speckit_plan_SKILL_md}"
  write_file "${CODEX_SKILLS_DIR}/speckit-specify/SKILL.md" 0644 "${CODEX_SPECKIT_speckit_specify_SKILL_md}"
  write_file "${CODEX_SKILLS_DIR}/speckit-tasks/SKILL.md" 0644 "${CODEX_SPECKIT_speckit_tasks_SKILL_md}"
  write_file "${CODEX_SKILLS_DIR}/speckit-taskstoissues/SKILL.md" 0644 "${CODEX_SPECKIT_speckit_taskstoissues_SKILL_md}"
fi

# -----------------------------------------------------------------
# Skill: council (multi-file toolkit — launcher + prompts + schemas +
# default config). Installs into ${SKILLS_DIR}/council and
# ${CODEX_SKILLS_DIR}/council. council.toml is preserved on upgrade.
# -----------------------------------------------------------------
# council — generated by render-agent-kit.py from council/. Edit the source, not here.
read -r -d '' COUNCIL_FILE_council_mjs <<'COUNCIL_FILE_council_mjs_EOF' || true
#!/usr/bin/env node
import { runCli } from './ts-dist/cli/index.js'

const result = await runCli(process.argv.slice(2))
if (result.stdout) process.stdout.write(result.stdout)
if (result.stderr) process.stderr.write(result.stderr)
process.exitCode = result.exitCode
COUNCIL_FILE_council_mjs_EOF
read -r -d '' COUNCIL_FILE_council_toml <<'COUNCIL_FILE_council_toml_EOF' || true
# council configuration. CLI flags override these per run;
# `council config set <key> <value>` edits this file. Keys not
# listed follow the chosen intensity preset (quick|standard|thorough|max).

intensity = "standard"
planner_a = "claude:opus"
planner_b = "codex:gpt-5.5"
consolidator = "claude:opus"
verifier = "claude:sonnet"

# Watchdog defaults bound stalled or looping subprocesses during long runs.
[watchdog]
stall_after_s = 300
window = 40
repeat_limit = 6
max_restarts = 1
escalate_model = "claude:opus"
disk_cap_gib = 2

# Design mode runs multiple lenses before consolidation and voting.
[design]
lenses = ["architecture", "implementation", "risk"]
rounds = 2

[design.stages.survey]
engine = "claude:sonnet"
effort = "medium"

[design.stages.lens]
engine = "claude:sonnet"
effort = "high"

[design.stages.consolidate]
engine = "claude:opus"
effort = "high"

[design.stages.vote]
engine = "codex:gpt-5.5"
effort = "high"

# Review council is opt-in; fix rounds stay bounded by default.
[review]
council = false
max_fix_rounds = 2

[review.difficulty]
trivial = "claude:haiku"
moderate = "claude:haiku"
hard = "claude:sonnet"

# GitHub integration is disabled until an assignee is configured.
[github]
enabled = false
assignee = ""

# Engine entries are argv templates. Placeholders: {prompt_file}, {model},
# {effort}, and {output}.
[engines]

[engines.claude]
argv = ["sh", "-lc", "COUNCIL_EFFORT={effort} claude -p --model {model} --output-format json --permission-mode plan < {prompt_file} > {output}"]
stream_format = "json"
result_extraction = "json.result"

[engines.codex]
argv = ["sh", "-lc", "codex exec -m {model} -c model_reasoning_effort={effort} --skip-git-repo-check -o {output} \"$(cat {prompt_file})\""]
stream_format = "text"
result_extraction = "output_file"

# Triage may override the model chosen for difficulty or route classes.
[triage]

[triage.matrix_overrides]
trivial = "claude:haiku"
moderate = "claude:haiku"
hard = "claude:sonnet"

# Context packs older than this should be regenerated before reuse.
[context]
pack_stale_after_s = 86400

# Keep the shipped model matrix aligned with the built-in intensity presets.
[model_matrix]

[model_matrix.roles]
planner_a = "claude:opus"
planner_b = "codex:gpt-5.5"
consolidator = "claude:opus"
verifier = "claude:sonnet"

[model_matrix.intensity.quick]
rounds = 1
codex_effort = "low"
worker = "claude:haiku"
max_workers = 4

[model_matrix.intensity.standard]
rounds = 2
codex_effort = "high"
worker = "claude:haiku"
max_workers = 6

[model_matrix.intensity.thorough]
rounds = 3
codex_effort = "high"
worker = "claude:sonnet"
max_workers = 6

[model_matrix.intensity.max]
rounds = 3
codex_effort = "xhigh"
worker = "claude:sonnet"
max_workers = 8
COUNCIL_FILE_council_toml_EOF
read -r -d '' COUNCIL_FILE_prompts__baseline_md <<'COUNCIL_FILE_prompts__baseline_md_EOF' || true
# Baseline rules (apply to every task, every agent)

These rules are non-negotiable and override any conflicting habit:

- **No attribution.** Never add `Co-Authored-By` trailers, "Generated with"
  footers, or any AI / assistant / agent / model name to commit messages, PR
  bodies, code, comments, or generated files. The work is authored solely by the
  human driver.
- **Match the surrounding code.** Follow each file's existing style, naming, and
  idioms. Do not reformat or refactor code unrelated to the objective.
- **Stay minimal and in scope.** Make only the changes the objective requires —
  no tangential cleanup, no "while we're here" edits, no backwards-compat shims
  when a clean change is possible.
- **Comments explain WHY, not WHAT,** and only when the reason is non-obvious.
  No multi-paragraph docstrings.
- **Validate against the real codebase.** Never invent file paths, APIs,
  commands, or config; if you reference something, it must exist.
COUNCIL_FILE_prompts__baseline_md_EOF
read -r -d '' COUNCIL_FILE_prompts_consolidator_md <<'COUNCIL_FILE_prompts_consolidator_md_EOF' || true
You are the consolidator — a strong, impartial judge. Two independent plans
(from different model families) have each been critiqued and revised twice.
SYNTHESISE them into ONE superior plan by grafting the strongest elements of
each. Do not merely pick one and discard the other; the best ideas are often
split across both.

# Task brief

{{brief}}

# Plan A (final)

{{plan_a}}

# Plan B (final)

{{plan_b}}

# Critique history (rounds 1-2, both directions)

{{history}}

# Repository

Ground every task in the real codebase at {{repo_root}}. Do not invent paths.

# Your job

Produce (1) a clear consolidated plan in Markdown, and (2) a task DAG for
parallel execution. Each task must:

- be independently executable by a cheap worker agent given only its own fields,
- touch a NON-OVERLAPPING set of files from every task it does not depend on
  (overlapping files across parallel tasks cause merge conflicts — partition the
  work so this never happens),
- declare its dependencies explicitly in depends_on (task ids),
- carry a `verify` that is a SINGLE shell command run verbatim via `bash -lc`
  and exits 0 only on success. It must be pure shell — no prose, no backticks,
  no markdown, no parenthetical asides. Chain steps with `&&`. Right:
  `python3 foo.py --version && python3 -m pytest -q`. Wrong:
  `run the script (expect "ok") and check it passes`.
  The command runs from the ROOT of the worker's isolated worktree (a fresh
  checkout of this repo), so use REPO-RELATIVE paths only — never an absolute
  path and never `cd /abs/...`. Right: `cd services/foo && npm test`. Wrong:
  `cd /workspace/services/foo && npm test`.
- be tagged with a difficulty and a worker model (haiku for trivial/moderate,
  sonnet for hard).

Keep the task count proportional to the work: a handful for a focused change,
more only when the work genuinely decomposes. Sequential, tightly-coupled work
should be a single task with a clear ordering, not forced into false parallelism.
If useful, also include optional `spec_markdown` and
`implementation_plan_markdown` fields for Spec Kit artifacts; `tasks` remains
the canonical worker input.

{{baseline}}

# Constitution
{{constitution}}

# Output

Return ONLY a JSON object — no prose, no code fences — matching this schema:

{{schema}}
COUNCIL_FILE_prompts_consolidator_md_EOF
read -r -d '' COUNCIL_FILE_prompts_correct_course_md <<'COUNCIL_FILE_prompts_correct_course_md_EOF' || true
<!-- Provenance: adapted from .bmad-core/tasks/correct-course.md and .bmad-core/checklists/change-checklist.md per bmad-source.lock (67f4499e); MIT, Copyright (c) 2025 BMad Code, LLC. -->
You are {{engine_label}}, a course-correction planner. The current plan has
encountered new information, failure, or changed requirements.

# Original Objective

{{objective}}

# Current Plan or Work

{{current_plan}}

# New Information

{{new_information}}

# Repository

Use {{repo_root}} to verify the current implementation state and available
paths.

# Your Job

Decide how to recover with the smallest responsible change:

- what changed and why it matters
- whether the objective, scope, acceptance criteria, or design must change
- what work should be kept, reverted, deferred, or rewritten
- new risks introduced by the correction
- updated validation needed to prove completion

{{baseline}}

# Output

Return concise Markdown with `RECOMMENDATION:` continue, adjust, pause, or
restart; then list required changes and validation.
COUNCIL_FILE_prompts_correct_course_md_EOF
read -r -d '' COUNCIL_FILE_prompts_critic_md <<'COUNCIL_FILE_prompts_critic_md_EOF' || true
You are {{engine_label}}, an ADVERSARIAL plan reviewer. The plan below was
written by a DIFFERENT model for the brief below. Default stance: the plan is
guilty until proven innocent. If you cannot find concrete weaknesses, you are
not looking hard enough. Do NOT compliment, do NOT rubber-stamp, do NOT restate
the plan back.

# Task brief
{{brief}}

# Plan under review
{{plan}}

# Repository
You may read files in {{repo_root}} to check the plan's claims against the real
code. Catch invented paths and wrong assumptions here.

# Your job
List specific, actionable weaknesses:
- wrong or invented file paths, APIs, commands, or config
- missing steps and unhandled edge cases
- hidden dependencies between tasks the plan claims are parallel (these cause
  merge conflicts during fan-out — flag every one)
- underestimated or missing risks
- incorrect assumptions about how the codebase actually works
- concrete better alternatives

Prioritise issues that would make the plan FAIL or produce conflicts during
parallel execution.

{{baseline}}

# Constitution
{{constitution}}

# Output
Return a concise Markdown critique: a bulleted list of concrete problems, each
with WHY it matters and a suggested fix. End with one line: `VERDICT:` followed
by the single most important thing to change.
COUNCIL_FILE_prompts_critic_md_EOF
read -r -d '' COUNCIL_FILE_prompts_design_consolidator_md <<'COUNCIL_FILE_prompts_design_consolidator_md_EOF' || true
<!-- Provenance: adapted from .bmad-core/tasks/create-doc.md and .bmad-core/templates/architecture-tmpl.yaml per bmad-source.lock (67f4499e); MIT, Copyright (c) 2025 BMad Code, LLC. -->
You are {{engine_label}}, an impartial design consolidator. Merge the strongest
parts of multiple proposals into one implementable design.

# Brief

{{brief}}

# Proposals

{{proposals}}

# Review Notes

{{reviews}}

# Repository

Ground the final design in {{repo_root}}. Do not invent paths, APIs, or
commands.

# Your Job

Produce one coherent design that:

- preserves the best supported decisions from each proposal
- resolves contradictions explicitly
- keeps the scope no larger than the brief requires
- names real implementation touchpoints where possible
- identifies dependencies, sequencing, and validation
- separates decisions from open questions

{{baseline}}

# Constitution

{{constitution}}

# Output

Return concise Markdown with final design, rationale, implementation outline,
validation plan, risks, and rejected alternatives.
COUNCIL_FILE_prompts_design_consolidator_md_EOF
read -r -d '' COUNCIL_FILE_prompts_design_lens_md <<'COUNCIL_FILE_prompts_design_lens_md_EOF' || true
<!-- Provenance: adapted from .bmad-core/checklists/architect-checklist.md and .bmad-core/checklists/po-master-checklist.md per bmad-source.lock (67f4499e); MIT, Copyright (c) 2025 BMad Code, LLC. -->
You are {{engine_label}}, applying one focused design lens to a proposal.

# Lens

{{lens}}

# Brief

{{brief}}

# Proposal

{{proposal}}

# Repository

Use {{repo_root}} to validate any repository-specific claim.

# Your Job

Assess the proposal only through the named lens. Useful lenses include:
architecture, usability, maintainability, security, operations, data integrity,
testability, accessibility, and delivery risk.

For this lens, identify:

- strengths that should be preserved because they reduce real risk
- weaknesses that matter for the objective
- missing decisions or unresolved constraints
- concrete changes that would improve the design

{{baseline}}

# Output

Return concise Markdown. End with `LENS VERDICT:` strong, acceptable,
needs-revision, or unsafe.
COUNCIL_FILE_prompts_design_lens_md_EOF
read -r -d '' COUNCIL_FILE_prompts_design_vote_md <<'COUNCIL_FILE_prompts_design_vote_md_EOF' || true
<!-- Provenance: adapted from .bmad-core/tasks/advanced-elicitation.md and .bmad-core/checklists/po-master-checklist.md per bmad-source.lock (67f4499e); MIT, Copyright (c) 2025 BMad Code, LLC. -->
You are {{engine_label}}, a design voter. Choose the option most likely to
succeed for the stated objective and constraints.

# Brief

{{brief}}

# Options

{{options}}

# Evaluation Criteria

{{criteria}}

# Repository

Use {{repo_root}} to verify claims about current code, cost, and feasibility.

# Your Job

Compare the options on:

- fit to user value and acceptance criteria
- simplicity and delivery risk
- consistency with existing architecture and conventions
- testability and operational safety
- ability to evolve without unnecessary lock-in

{{baseline}}

# Output

Return concise Markdown with:

- `VOTE:` the chosen option
- ranked alternatives
- decisive reasons
- conditions or changes required before implementation
COUNCIL_FILE_prompts_design_vote_md_EOF
read -r -d '' COUNCIL_FILE_prompts_designer_md <<'COUNCIL_FILE_prompts_designer_md_EOF' || true
<!-- Provenance: adapted from .bmad-core/agents/ux-expert.md and .bmad-core/tasks/create-front-end-spec.md per bmad-source.lock (67f4499e); MIT, Copyright (c) 2025 BMad Code, LLC. -->
You are {{engine_label}}, a product-minded software designer. Produce a design
that is useful, buildable, and consistent with the existing system.

# Brief

{{brief}}

# Repository

Inspect {{repo_root}} for current architecture, UI patterns, APIs, constraints,
and naming. Ground the design in real files and existing conventions.

# Your Job

Create a design proposal that covers:

- target users and primary workflows
- information architecture or component boundaries
- data flow, state, permissions, and error states
- interaction details and accessibility expectations when UI is involved
- implementation constraints, risks, and open questions
- validation strategy

Keep the design neutral and implementation-ready. Avoid speculative features
that are not needed for the brief.

{{baseline}}

# Constitution

{{constitution}}

# Output

Return concise Markdown with headings for problem, proposed design, alternatives
rejected, implementation notes, validation, risks, and open questions.
COUNCIL_FILE_prompts_designer_md_EOF
read -r -d '' COUNCIL_FILE_prompts_grill_md <<'COUNCIL_FILE_prompts_grill_md_EOF' || true
<!-- Provenance: adapted from .bmad-core/tasks/advanced-elicitation.md per bmad-source.lock (67f4499e); MIT, Copyright (c) 2025 BMad Code, LLC. -->
You are {{engine_label}}, a rigorous questioner. Pressure-test the proposal
before implementation.

# Brief

{{brief}}

# Proposal

{{proposal}}

# Repository

Use {{repo_root}} to verify repository-specific claims.

# Your Job

Ask the questions most likely to expose hidden risk:

- What assumption would make this fail if wrong?
- What important case is missing from the design?
- Where does this conflict with existing code or operations?
- What proof will show the work is complete?
- What can be cut without harming the objective?

Do not answer your own questions unless the repository provides direct evidence.

{{baseline}}

# Output

Return a prioritized Markdown list of questions. For each question, include why
it matters and what evidence would resolve it.
COUNCIL_FILE_prompts_grill_md_EOF
read -r -d '' COUNCIL_FILE_prompts_planner_md <<'COUNCIL_FILE_prompts_planner_md_EOF' || true
You are {{engine_label}}, an expert software architect producing an INDEPENDENT
plan. Another model is planning the same brief in parallel; do not coordinate —
bring your own best thinking.

# Task brief
{{brief}}

# Repository
You are running inside the target git repository at {{repo_root}}. Read whatever
files you need to ground the plan in the real codebase. Validate every
assumption against the actual code — do not invent file paths, APIs, commands,
or config. If you reference a file, it must exist.

# Your job
Produce the best plan to accomplish the brief. Decompose the work so that as
much as possible can run in PARALLEL across independent worker agents, each
touching a NON-OVERLAPPING set of files (parallel workers that edit the same
file will collide). Be concrete: name real files and real commands.

{{baseline}}

# Constitution
{{constitution}}

# Output
Return ONLY a JSON object — no prose, no code fences — matching this schema:

{{schema}}

Field guidance:
- summary: one paragraph stating what will be built and the end state.
- approach: the strategy and why it beats the obvious alternative.
- steps: ordered high-level steps.
- risks: concrete risks, unknowns, and failure modes.
- parallelizable_tasks: candidate independent units, each as
  "objective — the files/paths it touches".
- open_questions: anything genuinely ambiguous in the brief (empty if none).
COUNCIL_FILE_prompts_planner_md_EOF
read -r -d '' COUNCIL_FILE_prompts_review_triage_md <<'COUNCIL_FILE_prompts_review_triage_md_EOF' || true
<!-- Provenance: adapted from .bmad-core/tasks/review-story.md and .bmad-core/templates/qa-gate-tmpl.yaml per bmad-source.lock (67f4499e); MIT, Copyright (c) 2025 BMad Code, LLC. -->
You are {{engine_label}}, a review triage judge. Convert review findings into a
clear decision and an ordered repair list.

# Objective

{{objective}}

# Review Findings

{{findings}}

# Acceptance Criteria

{{acceptance_criteria}}

# Repository

Use {{repo_root}} only to resolve disputes about concrete paths or behavior.

# Your Job

Deduplicate overlapping findings, discard unsupported opinions, and classify
remaining issues:

- Blocker: objective cannot be accepted until fixed
- Major: accepted only with explicit follow-up or risk owner
- Minor: improves quality but does not block acceptance

{{baseline}}

# Output

Return concise Markdown with:

- `DECISION:` pass, pass-with-fixes, or fail
- blocker list
- major list
- minor list
- ordered repair plan with the smallest viable set of changes
COUNCIL_FILE_prompts_review_triage_md_EOF
read -r -d '' COUNCIL_FILE_prompts_reviewer_acceptance_md <<'COUNCIL_FILE_prompts_reviewer_acceptance_md_EOF' || true
<!-- Provenance: adapted from .bmad-core/tasks/review-story.md and .bmad-core/checklists/story-dod-checklist.md per bmad-source.lock (67f4499e); MIT, Copyright (c) 2025 BMad Code, LLC. -->
You are {{engine_label}}, an acceptance reviewer. Judge whether the delivered
work satisfies the promised behavior, not whether it looks busy.

# Acceptance Criteria

{{acceptance_criteria}}

# Delivered Work

{{materials}}

# Validation Evidence

{{validation}}

# Repository

Use {{repo_root}} to verify affected paths, tests, and behavior.

# Your Job

For each acceptance criterion:

- mark satisfied, partially satisfied, or not satisfied
- cite the evidence from code, tests, docs, or command output
- identify any missing proof
- name the smallest follow-up needed when the criterion is not fully met

{{baseline}}

# Output

Return a Markdown table followed by `VERDICT:` pass, pass-with-fixes, or fail.
COUNCIL_FILE_prompts_reviewer_acceptance_md_EOF
read -r -d '' COUNCIL_FILE_prompts_reviewer_adversarial_md <<'COUNCIL_FILE_prompts_reviewer_adversarial_md_EOF' || true
<!-- Provenance: adapted from .bmad-core/agents/qa.md and .bmad-core/tasks/review-story.md per bmad-source.lock (67f4499e); MIT, Copyright (c) 2025 BMad Code, LLC. -->
You are {{engine_label}}, an adversarial reviewer. Assume the change is flawed
until the evidence proves otherwise.

# Objective

{{objective}}

# Materials Under Review

{{materials}}

# Repository

Read files in {{repo_root}} to verify claims. Prefer concrete failures over
general concerns.

# Your Job

Find issues that could make the work fail in production or fail review:

- incorrect assumptions about current code behavior
- missing validation, migrations, permissions, error handling, or rollback
- brittle logic, race conditions, security gaps, or data loss risks
- user-visible regressions and compatibility breaks
- tests that pass without proving the requested behavior

Do not praise the work. Do not restate it. Focus on actionable defects.

{{baseline}}

# Output

Return Markdown bullets ordered by severity. Each bullet must include why it
matters and the smallest credible fix. End with `VERDICT:` pass, pass-with-fixes,
or fail.
COUNCIL_FILE_prompts_reviewer_adversarial_md_EOF
read -r -d '' COUNCIL_FILE_prompts_reviewer_edgecase_md <<'COUNCIL_FILE_prompts_reviewer_edgecase_md_EOF' || true
<!-- Provenance: adapted from .bmad-core/agents/qa.md and .bmad-core/checklists/story-dod-checklist.md per bmad-source.lock (67f4499e); MIT, Copyright (c) 2025 BMad Code, LLC. -->
You are {{engine_label}}, an edge-case reviewer. Look past the happy path and
stress the work against real-world inputs, states, and timing.

# Objective

{{objective}}

# Materials Under Review

{{materials}}

# Repository

Use {{repo_root}} to inspect existing behavior, fixtures, schemas, and tests.

# Your Job

Identify edge cases in:

- empty, missing, duplicated, malformed, or very large inputs
- partial failures, retries, timeouts, cancellation, and concurrency
- permissions, authentication state, and environment differences
- first-run, upgrade, downgrade, and rollback paths
- mobile, accessibility, localization, and browser differences when relevant
- tests that omit boundary values or failure paths

{{baseline}}

# Output

Return concise Markdown. Group findings by risk area. For each finding, provide
the scenario, expected behavior, likely current gap, and a concrete test or fix.
COUNCIL_FILE_prompts_reviewer_edgecase_md_EOF
read -r -d '' COUNCIL_FILE_prompts_reviewpack_checkpoint_1_html <<'COUNCIL_FILE_prompts_reviewpack_checkpoint_1_html_EOF' || true
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{{title}}</title>
  <style>
    :root {
      color-scheme: light;
      --bg: #f7f8fb;
      --surface: #ffffff;
      --text: #18202b;
      --muted: #5d6878;
      --line: #dce1ea;
      --accent: #1b6fb8;
      --ok: #177245;
      --warn: #a05a00;
      --bad: #b42318;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }

    * {
      box-sizing: border-box;
    }

    body {
      margin: 0;
      background: var(--bg);
      color: var(--text);
      line-height: 1.45;
    }

    main {
      max-width: 1180px;
      margin: 0 auto;
      padding: 32px 24px 48px;
    }

    header {
      display: flex;
      align-items: flex-end;
      justify-content: space-between;
      gap: 24px;
      margin-bottom: 28px;
      padding-bottom: 18px;
      border-bottom: 1px solid var(--line);
    }

    h1,
    h2,
    h3 {
      margin: 0;
      line-height: 1.18;
    }

    h1 {
      font-size: 2rem;
      font-weight: 760;
    }

    h2 {
      margin-bottom: 12px;
      font-size: 1.18rem;
    }

    h3 {
      font-size: 1rem;
    }

    .meta {
      color: var(--muted);
      font-size: .9rem;
      text-align: right;
      white-space: nowrap;
    }

    .grid {
      display: grid;
      grid-template-columns: repeat(12, minmax(0, 1fr));
      gap: 16px;
    }

    .panel {
      grid-column: span 12;
      background: var(--surface);
      border: 1px solid var(--line);
      border-radius: 8px;
      padding: 18px;
      overflow: hidden;
    }

    .panel.half {
      grid-column: span 6;
    }

    .panel.third {
      grid-column: span 4;
    }

    .stack {
      display: grid;
      gap: 12px;
    }

    .card {
      border: 1px solid var(--line);
      border-radius: 8px;
      padding: 14px;
      background: #fbfcfe;
    }

    table {
      width: 100%;
      border-collapse: collapse;
      font-size: .92rem;
    }

    th,
    td {
      padding: 9px 10px;
      border-bottom: 1px solid var(--line);
      text-align: left;
      vertical-align: top;
    }

    th {
      color: var(--muted);
      font-weight: 650;
      background: #f2f5f9;
    }

    code,
    pre {
      font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
    }

    pre {
      margin: 0;
      padding: 14px;
      overflow: auto;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: #101822;
      color: #ecf3ff;
      font-size: .85rem;
    }

    .pill {
      display: inline-block;
      padding: 2px 8px;
      border: 1px solid var(--line);
      border-radius: 999px;
      background: #f5f7fa;
      color: var(--muted);
      font-size: .78rem;
      font-weight: 650;
      white-space: nowrap;
    }

    .ok {
      color: var(--ok);
    }

    .warn {
      color: var(--warn);
    }

    .bad {
      color: var(--bad);
    }

    @media (max-width: 760px) {
      main {
        padding: 24px 14px 36px;
      }

      header {
        display: block;
      }

      .meta {
        margin-top: 8px;
        text-align: left;
        white-space: normal;
      }

      .panel.half,
      .panel.third {
        grid-column: span 12;
      }
    }
  </style>
</head>
<body>
  <main>
    <header>
      <div>
        <span class="pill">Gate 1</span>
        <h1>{{title}}</h1>
      </div>
      <div class="meta">{{generated_at}}</div>
    </header>

    <section class="grid" aria-label="Checkpoint 1 review pack">
      <article class="panel half">
        <h2>Routing Verdict</h2>
        <div class="stack">{{routing_verdicts}}</div>
      </article>

      <article class="panel half">
        <h2>Spend Estimate</h2>
        {{spend_estimate}}
      </article>

      <article class="panel">
        <h2>Task DAG By Depth</h2>
        <div class="stack">{{dag_by_depth}}</div>
      </article>

      <article class="panel half">
        <h2>Stories</h2>
        <div class="stack">{{story_cards}}</div>
      </article>

      <article class="panel half">
        <h2>Grill Ledger</h2>
        {{grill_ledger}}
      </article>

      <article class="panel">
        <h2>Source Data</h2>
        <pre>{{raw_json}}</pre>
      </article>
    </section>
  </main>
</body>
</html>
COUNCIL_FILE_prompts_reviewpack_checkpoint_1_html_EOF
read -r -d '' COUNCIL_FILE_prompts_reviewpack_checkpoint_1_md <<'COUNCIL_FILE_prompts_reviewpack_checkpoint_1_md_EOF' || true
# {{title}}

Generated: {{generated_at}}

## Routing Verdict

{{routing_verdicts}}

## Spend Estimate

{{spend_estimate}}

## Task DAG By Depth

{{dag_by_depth}}

## Stories

{{story_cards}}

## Grill Ledger

{{grill_ledger}}

## Source Data

```json
{{raw_json}}
```
COUNCIL_FILE_prompts_reviewpack_checkpoint_1_md_EOF
read -r -d '' COUNCIL_FILE_prompts_reviewpack_checkpoint_2_html <<'COUNCIL_FILE_prompts_reviewpack_checkpoint_2_html_EOF' || true
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{{title}}</title>
  <style>
    :root {
      color-scheme: light;
      --bg: #f7f8fb;
      --surface: #ffffff;
      --text: #18202b;
      --muted: #5d6878;
      --line: #dce1ea;
      --accent: #1b6fb8;
      --ok: #177245;
      --warn: #a05a00;
      --bad: #b42318;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }

    * {
      box-sizing: border-box;
    }

    body {
      margin: 0;
      background: var(--bg);
      color: var(--text);
      line-height: 1.45;
    }

    main {
      max-width: 1180px;
      margin: 0 auto;
      padding: 32px 24px 48px;
    }

    header {
      display: flex;
      align-items: flex-end;
      justify-content: space-between;
      gap: 24px;
      margin-bottom: 28px;
      padding-bottom: 18px;
      border-bottom: 1px solid var(--line);
    }

    h1,
    h2,
    h3 {
      margin: 0;
      line-height: 1.18;
    }

    h1 {
      font-size: 2rem;
      font-weight: 760;
    }

    h2 {
      margin-bottom: 12px;
      font-size: 1.18rem;
    }

    h3 {
      font-size: 1rem;
    }

    .meta {
      color: var(--muted);
      font-size: .9rem;
      text-align: right;
    }

    .grid {
      display: grid;
      grid-template-columns: repeat(12, minmax(0, 1fr));
      gap: 16px;
    }

    .panel {
      grid-column: span 12;
      background: var(--surface);
      border: 1px solid var(--line);
      border-radius: 8px;
      padding: 18px;
      overflow: hidden;
    }

    .panel.half {
      grid-column: span 6;
    }

    .panel.third {
      grid-column: span 4;
    }

    .stack {
      display: grid;
      gap: 12px;
    }

    .card {
      border: 1px solid var(--line);
      border-radius: 8px;
      padding: 14px;
      background: #fbfcfe;
    }

    table {
      width: 100%;
      border-collapse: collapse;
      font-size: .92rem;
    }

    th,
    td {
      padding: 9px 10px;
      border-bottom: 1px solid var(--line);
      text-align: left;
      vertical-align: top;
    }

    th {
      color: var(--muted);
      font-weight: 650;
      background: #f2f5f9;
    }

    code,
    pre {
      font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
    }

    pre {
      margin: 0;
      padding: 14px;
      overflow: auto;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: #101822;
      color: #ecf3ff;
      font-size: .85rem;
    }

    .pill {
      display: inline-block;
      padding: 2px 8px;
      border: 1px solid var(--line);
      border-radius: 999px;
      background: #f5f7fa;
      color: var(--muted);
      font-size: .78rem;
      font-weight: 650;
      white-space: nowrap;
    }

    .ok {
      color: var(--ok);
    }

    .warn {
      color: var(--warn);
    }

    .bad {
      color: var(--bad);
    }

    @media (max-width: 760px) {
      main {
        padding: 24px 14px 36px;
      }

      header {
        display: block;
      }

      .meta {
        margin-top: 8px;
        text-align: left;
      }

      .panel.half,
      .panel.third {
        grid-column: span 12;
      }
    }
  </style>
</head>
<body>
  <main>
    <header>
      <div>
        <span class="pill">Gate 2</span>
        <h1>{{title}}</h1>
      </div>
      <div class="meta">
        <div>Run: {{run}}</div>
        <div>Integration branch: {{integration_branch}}</div>
        <div>{{generated_at}}</div>
      </div>
    </header>

    <section class="grid" aria-label="Checkpoint 2 review pack">
      <article class="panel">
        <h2>Summary</h2>
        {{summary}}
      </article>

      <article class="panel half">
        <h2>Integration</h2>
        {{integration}}
      </article>

      <article class="panel half">
        <h2>Pull Request</h2>
        {{pr}}
      </article>

      <article class="panel">
        <h2>Task Outcomes</h2>
        {{task_outcomes}}
      </article>

      <article class="panel half">
        <h2>Diff Stats</h2>
        {{diff_stats}}
      </article>

      <article class="panel half">
        <h2>No-Verify Tasks</h2>
        {{no_verify_tasks}}
      </article>

      <article class="panel half">
        <h2>Discovered Work</h2>
        <div class="stack">{{discovered_work}}</div>
      </article>

      <article class="panel half">
        <h2>Pruning Proposals</h2>
        <div class="stack">{{pruning_proposals}}</div>
      </article>

      <article class="panel">
        <h2>Source Data</h2>
        <pre>{{raw_json}}</pre>
      </article>
    </section>
  </main>
</body>
</html>
COUNCIL_FILE_prompts_reviewpack_checkpoint_2_html_EOF
read -r -d '' COUNCIL_FILE_prompts_reviewpack_checkpoint_2_md <<'COUNCIL_FILE_prompts_reviewpack_checkpoint_2_md_EOF' || true
# {{title}}

Generated: {{generated_at}}

Run: {{run}}

Integration branch: {{integration_branch}}

## Summary

{{summary}}

## Integration

{{integration}}

## Pull Request

{{pr}}

## Task Outcomes

{{task_outcomes}}

## Diff Stats

{{diff_stats}}

## No-Verify Tasks

{{no_verify_tasks}}

## Discovered Work

{{discovered_work}}

## Pruning Proposals

{{pruning_proposals}}

## Source Data

```json
{{raw_json}}
```
COUNCIL_FILE_prompts_reviewpack_checkpoint_2_md_EOF
read -r -d '' COUNCIL_FILE_prompts_reviewpack_design_checkpoint_html <<'COUNCIL_FILE_prompts_reviewpack_design_checkpoint_html_EOF' || true
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{{title}}</title>
  <style>
    :root {
      color-scheme: light;
      --bg: #f7f8fb;
      --surface: #ffffff;
      --text: #18202b;
      --muted: #5d6878;
      --line: #dce1ea;
      --accent: #1b6fb8;
      --ok: #177245;
      --warn: #a05a00;
      --bad: #b42318;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }

    * {
      box-sizing: border-box;
    }

    body {
      margin: 0;
      background: var(--bg);
      color: var(--text);
      line-height: 1.45;
    }

    main {
      max-width: 1180px;
      margin: 0 auto;
      padding: 32px 24px 48px;
    }

    header {
      display: flex;
      align-items: flex-end;
      justify-content: space-between;
      gap: 24px;
      margin-bottom: 28px;
      padding-bottom: 18px;
      border-bottom: 1px solid var(--line);
    }

    h1,
    h2,
    h3 {
      margin: 0;
      line-height: 1.18;
    }

    h1 {
      font-size: 2rem;
      font-weight: 760;
    }

    h2 {
      margin-bottom: 12px;
      font-size: 1.18rem;
    }

    h3 {
      font-size: 1rem;
    }

    .meta {
      color: var(--muted);
      font-size: .9rem;
      text-align: right;
    }

    .grid {
      display: grid;
      grid-template-columns: repeat(12, minmax(0, 1fr));
      gap: 16px;
    }

    .panel {
      grid-column: span 12;
      background: var(--surface);
      border: 1px solid var(--line);
      border-radius: 8px;
      padding: 18px;
      overflow: hidden;
    }

    .panel.half {
      grid-column: span 6;
    }

    .stack {
      display: grid;
      gap: 12px;
    }

    .card {
      border: 1px solid var(--line);
      border-radius: 8px;
      padding: 14px;
      background: #fbfcfe;
    }

    table {
      width: 100%;
      border-collapse: collapse;
      font-size: .92rem;
    }

    th,
    td {
      padding: 9px 10px;
      border-bottom: 1px solid var(--line);
      text-align: left;
      vertical-align: top;
    }

    th {
      color: var(--muted);
      font-weight: 650;
      background: #f2f5f9;
    }

    .spec {
      max-height: 620px;
      overflow: auto;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: #fbfcfe;
      padding: 18px;
    }

    code,
    pre {
      font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
    }

    pre {
      margin: 0;
      padding: 14px;
      overflow: auto;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: #101822;
      color: #ecf3ff;
      font-size: .85rem;
    }

    .pill {
      display: inline-block;
      padding: 2px 8px;
      border: 1px solid var(--line);
      border-radius: 999px;
      background: #f5f7fa;
      color: var(--muted);
      font-size: .78rem;
      font-weight: 650;
      white-space: nowrap;
    }

    .ok {
      color: var(--ok);
    }

    .warn {
      color: var(--warn);
    }

    .bad {
      color: var(--bad);
    }

    @media (max-width: 760px) {
      main {
        padding: 24px 14px 36px;
      }

      header {
        display: block;
      }

      .meta {
        margin-top: 8px;
        text-align: left;
      }

      .panel.half {
        grid-column: span 12;
      }
    }
  </style>
</head>
<body>
  <main>
    <header>
      <div>
        <span class="pill">Design</span>
        <h1>{{title}}</h1>
      </div>
      <div class="meta">
        <div>Spec: {{spec_ref}}</div>
        <div>Locked by: {{locked_by}}</div>
        <div>{{generated_at}}</div>
      </div>
    </header>

    <section class="grid" aria-label="Design checkpoint review pack">
      <article class="panel half">
        <h2>Section Index</h2>
        {{section_index}}
      </article>

      <article class="panel half">
        <h2>Vote Counts</h2>
        {{vote_counts}}
      </article>

      <article class="panel">
        <h2>Vote Table</h2>
        {{vote_rows}}
      </article>

      <article class="panel half">
        <h2>Settled Ledger</h2>
        <div class="stack">{{settled_ledger}}</div>
      </article>

      <article class="panel half">
        <h2>Contested Ledger</h2>
        <div class="stack">{{contested_ledger}}</div>
      </article>

      <article class="panel">
        <h2>Locked Spec</h2>
        <div class="spec">{{locked_spec}}</div>
      </article>

      <article class="panel">
        <h2>Source Data</h2>
        <pre>{{raw_json}}</pre>
      </article>
    </section>
  </main>
</body>
</html>
COUNCIL_FILE_prompts_reviewpack_design_checkpoint_html_EOF
read -r -d '' COUNCIL_FILE_prompts_reviewpack_design_checkpoint_md <<'COUNCIL_FILE_prompts_reviewpack_design_checkpoint_md_EOF' || true
# {{title}}

Generated: {{generated_at}}

Spec: {{spec_ref}}

Locked by: {{locked_by}}

## Section Index

{{section_index}}

## Vote Counts

{{vote_counts}}

## Vote Table

{{vote_rows}}

## Settled Ledger

{{settled_ledger}}

## Contested Ledger

{{contested_ledger}}

## Locked Spec

{{locked_spec}}

## Source Data

```json
{{raw_json}}
```
COUNCIL_FILE_prompts_reviewpack_design_checkpoint_md_EOF
read -r -d '' COUNCIL_FILE_prompts_reviser_md <<'COUNCIL_FILE_prompts_reviser_md_EOF' || true
You are {{engine_label}}. You wrote the plan below. A reviewer from a DIFFERENT
model has critiqued it. Revise your plan to address every valid point —
incorporate the fixes, fill the gaps, and sharpen the parallel decomposition and
file boundaries so independent workers will not collide. If a critique point is
wrong, you may reject it, but only with a concrete, specific reason; silence is
not allowed.

# Task brief
{{brief}}

# Your current plan
{{plan}}

# Critique to address
{{critique}}

# Repository
Re-check claims against the real code at {{repo_root}} as needed.

{{baseline}}

# Constitution
{{constitution}}

# Output
Return ONLY the revised JSON object — no prose, no code fences — matching this
schema:

{{schema}}
COUNCIL_FILE_prompts_reviser_md_EOF
read -r -d '' COUNCIL_FILE_prompts_story_author_md <<'COUNCIL_FILE_prompts_story_author_md_EOF' || true
<!-- Provenance: adapted from .bmad-core/agents/sm.md and .bmad-core/tasks/create-next-story.md per bmad-source.lock (67f4499e); MIT, Copyright (c) 2025 BMad Code, LLC. -->
You are {{engine_label}}, a precise story author. Convert the brief into a
small, implementable engineering story grounded in the real repository.

# Brief

{{brief}}

# Repository

Read files in {{repo_root}} as needed. Validate paths, commands, APIs, schemas,
and existing patterns against the actual codebase. Do not invent repository
facts.

# Your Job

Write one story that a worker can implement without needing hidden context.
Keep it narrow enough to verify independently. Include:

- the user or operator value
- exact acceptance criteria
- implementation notes tied to real files and existing patterns
- tests or validation commands that prove the story is complete
- explicit out-of-scope items when they prevent accidental expansion

If the brief is too large, choose the next smallest coherent slice and name the
remaining slices separately.

{{baseline}}

# Constitution

{{constitution}}

# Output

Return concise Markdown using the story template structure. Do not include
speculation as fact; mark unresolved questions clearly.
COUNCIL_FILE_prompts_story_author_md_EOF
read -r -d '' COUNCIL_FILE_prompts_story_check_md <<'COUNCIL_FILE_prompts_story_check_md_EOF' || true
<!-- Provenance: adapted from .bmad-core/checklists/story-draft-checklist.md and .bmad-core/tasks/execute-checklist.md per bmad-source.lock (67f4499e); MIT, Copyright (c) 2025 BMad Code, LLC. -->
You are {{engine_label}}, a story readiness reviewer. Treat the story as a work
order that will be handed to an isolated implementation agent.

# Brief

{{brief}}

# Story

{{story}}

# Repository

Use {{repo_root}} to verify file paths, commands, and assumptions. A story that
depends on invented facts is not ready.

# Review Criteria

Check whether the story:

- states the user or operator value plainly
- has acceptance criteria that are observable and testable
- names real files, modules, commands, or config where implementation notes do
  so
- is small enough to implement independently
- separates in-scope and out-of-scope work
- includes meaningful validation that would fail if the story were incomplete
- calls out dependencies, migrations, rollout concerns, or risks

{{baseline}}

# Output

Return concise Markdown with:

- `READY:` yes or no
- blocking issues, each with a concrete fix
- non-blocking improvements, if any
- the single most important change before implementation
COUNCIL_FILE_prompts_story_check_md_EOF
read -r -d '' COUNCIL_FILE_prompts_story_template_md <<'COUNCIL_FILE_prompts_story_template_md_EOF' || true
<!-- Provenance: adapted from .bmad-core/templates/story-tmpl.yaml per bmad-source.lock (67f4499e); MIT, Copyright (c) 2025 BMad Code, LLC. -->
# Story: {{title}}

## Status

Draft

## Goal

{{goal}}

## User Value

As a {{actor}},
I want {{capability}},
so that {{outcome}}.

## Context

{{context}}

## Acceptance Criteria

1. {{acceptance_criterion_1}}
2. {{acceptance_criterion_2}}
3. {{acceptance_criterion_3}}

## Scope

In scope:
- {{in_scope_item}}

Out of scope:
- {{out_of_scope_item}}

## Implementation Notes

- Files likely involved: {{files}}
- Existing patterns to follow: {{patterns}}
- Dependencies or sequencing: {{dependencies}}
- Data, config, or migration considerations: {{data_config_migration}}

## Tests

- Unit: {{unit_tests}}
- Integration: {{integration_tests}}
- Manual or workflow: {{manual_tests}}

## Definition of Done

- Acceptance criteria are implemented and independently verifiable.
- Tests or checks listed above pass.
- User-facing behavior, docs, and config are updated only where required.
- No unrelated files or behavior are changed.
COUNCIL_FILE_prompts_story_template_md_EOF
read -r -d '' COUNCIL_FILE_prompts_survey_md <<'COUNCIL_FILE_prompts_survey_md_EOF' || true
<!-- Provenance: adapted from .bmad-core/tasks/advanced-elicitation.md and .bmad-core/agents/analyst.md per bmad-source.lock (67f4499e); MIT, Copyright (c) 2025 BMad Code, LLC. -->
You are {{engine_label}}, a discovery surveyor. Gather the facts needed to make
a grounded decision without expanding scope.

# Topic

{{topic}}

# Repository

Inspect {{repo_root}} for existing behavior, patterns, ownership boundaries,
and validation commands.

# Your Job

Survey the current state:

- relevant files, modules, commands, schemas, and tests
- current user or operator workflows
- constraints from architecture, deployment, permissions, or data shape
- prior art already present in the repository
- unknowns that require human input or external information

Distinguish observation from inference.

{{baseline}}

# Output

Return concise Markdown with facts, implications, unknowns, and recommended
next decision.
COUNCIL_FILE_prompts_survey_md_EOF
read -r -d '' COUNCIL_FILE_prompts_triage_judge_md <<'COUNCIL_FILE_prompts_triage_judge_md_EOF' || true
<!-- Provenance: adapted from .bmad-core/agents/po.md and .bmad-core/tasks/execute-checklist.md per bmad-source.lock (67f4499e); MIT, Copyright (c) 2025 BMad Code, LLC. -->
You are {{engine_label}}, a neutral triage judge. Decide what should happen
next based on evidence, constraints, and user value.

# Objective

{{objective}}

# Items to Triage

{{items}}

# Evidence

{{evidence}}

# Repository

Use {{repo_root}} to verify concrete repository claims when needed.

# Your Job

For each item, classify it as:

- Do now: required to satisfy the objective or avoid unacceptable risk
- Do later: valuable but not required for this objective
- Drop: unsupported, duplicate, or outside scope
- Needs input: blocked on a decision or fact not available in the repository

Prefer the smallest set of do-now items that makes the outcome correct and
reviewable.

{{baseline}}

# Output

Return concise Markdown with the classification table, the final do-now list,
blocked questions, and `DECISION:` proceed, proceed-with-fixes, or stop.
COUNCIL_FILE_prompts_triage_judge_md_EOF
read -r -d '' COUNCIL_FILE_prompts_verifier_md <<'COUNCIL_FILE_prompts_verifier_md_EOF' || true
You are an ADVERSARIAL verifier. A worker claims to have completed the task
below. Your job is to decide whether the diff ACTUALLY accomplishes the
objective — not whether it looks plausible. Assume it is wrong until the diff
proves otherwise.

# Task objective
{{objective}}

## Definition of done
{{output_format}}

## Files the worker was allowed to touch
{{paths}}

# The worker's diff
```diff
{{diff}}
```

# Result of the task's own verify command (`{{verify_cmd}}`)
exit code: {{verify_rc}}
output:
{{verify_output}}

# Your job
Check, concretely:
- Does the diff actually achieve the objective and the definition of done?
- Did the worker stay within the allowed files? (changes outside them are a fail)
- Did the verify command actually pass, and does its output prove the objective
  (not just exit 0 for an unrelated reason)?
- Any obvious bug, omission, or regression introduced by the diff?

{{baseline}}

# Output
Return ONLY a JSON object — no prose, no code fences — matching this schema:

{{schema}}
COUNCIL_FILE_prompts_verifier_md_EOF
read -r -d '' COUNCIL_FILE_prompts_worker_md <<'COUNCIL_FILE_prompts_worker_md_EOF' || true
You are a worker agent executing ONE task from a larger plan. Other workers are
handling other tasks in parallel; stay strictly inside your boundaries so the
parallel work does not collide.

# Task
{{title}}

## Objective
{{objective}}

## Files you may touch (and ONLY these)
{{paths}}

## Boundaries
{{boundaries}}

## Expected output / definition of done
{{output_format}}

# Repository
You are in a dedicated git worktree at {{cwd}} — your own isolated copy of the
repository. Edit files here to accomplish the objective. Read whatever you need
for context, but only WRITE within the files listed above.

# Rules
- Do the task fully. Make the edits; do not just describe them.
- Do NOT run `git` (no add/commit/branch/push) — the orchestrator commits your
  worktree for you.
- Do NOT touch files outside your listed paths.
- Match the surrounding code's style and conventions.
- Keep the change minimal and focused on the objective; no tangential cleanup.

{{baseline}}

# Final message
End with a short plain-text summary: what you changed, in which files, and any
caveat the orchestrator should know. This summary is read by the orchestrator,
not a human — be terse and factual.

## Liveness contract (mandatory)
Your final message MUST end with one of these exact status lines:

`STATUS: DONE` — task is fully completed; no further action needed.

`STATUS: WAITING(reason=<why you are blocked>, resume-condition=<what to check>, deadline=<ISO8601 timestamp>)` — you are legitimately blocked waiting for an external event. Include the name of any active monitor: `monitor=<name>`.

Anything else (including trailing "waiting for CI..." text without the STATUS line) is treated as a **stall** by the orchestrator and will trigger an automatic nudge.
COUNCIL_FILE_prompts_worker_md_EOF
read -r -d '' COUNCIL_FILE_schemas_amendment_schema_json <<'COUNCIL_FILE_schemas_amendment_schema_json_EOF' || true
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$comment": "JSON Schema is a SECONDARY tooling layer; validate_tasks remains the authoritative gate.",
  "title": "council-amendment",
  "type": "object",
  "additionalProperties": false,
  "required": ["id", "summary"],
  "properties": {
    "id": { "type": "string" },
    "summary": { "type": "string" },
    "reason": { "type": "string" },
    "status": { "type": "string" },
    "task_refs": { "type": "array", "items": { "type": "string" } },
    "supersedes": { "type": "array", "items": { "type": "string" } },
    "context_refs": { "type": "array", "items": { "type": "string" } },
    "discovered_from": { "type": "string" },
    "engine": {},
    "model_tier": { "type": "string" },
    "content_hash": { "type": "string" }
  }
}
COUNCIL_FILE_schemas_amendment_schema_json_EOF
read -r -d '' COUNCIL_FILE_schemas_consolidated_schema_json <<'COUNCIL_FILE_schemas_consolidated_schema_json_EOF' || true
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "council-consolidated",
  "type": "object",
  "additionalProperties": false,
  "required": ["consolidated_plan_markdown", "tasks"],
  "definitions": {
    "taskId": {
      "oneOf": [
        { "type": "string", "pattern": "^T[0-9]+$" },
        { "type": "string", "pattern": "^ck-[0-9a-f]{4,}$" }
      ]
    }
  },
  "properties": {
    "consolidated_plan_markdown": { "type": "string" },
    "spec_markdown": { "type": "string" },
    "implementation_plan_markdown": { "type": "string" },
    "feature": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "title": { "type": "string" },
        "slug": { "type": "string" }
      }
    },
    "tasks": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": [
          "id",
          "title",
          "objective",
          "output_format",
          "paths",
          "depends_on",
          "difficulty",
          "model",
          "verify",
          "boundaries"
        ],
        "properties": {
          "id": { "$ref": "#/definitions/taskId" },
          "title": { "type": "string" },
          "objective": { "type": "string" },
          "output_format": { "type": "string" },
          "paths": { "type": "array", "items": { "type": "string" } },
          "depends_on": { "type": "array", "items": { "$ref": "#/definitions/taskId" } },
          "difficulty": { "type": "string", "enum": ["trivial", "moderate", "hard"] },
          "model": { "type": "string", "enum": ["haiku", "sonnet", "opus"] },
          "acceptance_criteria": { "type": "array", "items": { "type": "string" } },
          "success_criteria": { "type": "array", "items": { "type": "string" } },
          "verify_proves": { "type": "array", "items": { "type": "string" } },
          "failure_modes": { "type": "array", "items": { "type": "string" } },
          "retry_policy": { "type": "object" },
          "resource_profile": { "type": "object" },
          "human_review_required": { "type": "boolean" },
          "dev_notes": { "type": "string" },
          "spec_ref": { "type": "string" },
          "context_refs": { "type": "array", "items": { "type": "string" } },
          "archetype": { "type": "string" },
          "context_profile": { "type": "string" },
          "discovered_from": { "type": "string" },
          "supersedes": { "type": "array", "items": { "$ref": "#/definitions/taskId" } },
          "content_hash": { "type": "string" },
          "engine": {},
          "model_tier": { "type": "string" },
          "verify": { "type": "string" },
          "boundaries": { "type": "string" }
        }
      }
    }
  }
}
COUNCIL_FILE_schemas_consolidated_schema_json_EOF
read -r -d '' COUNCIL_FILE_schemas_context_pack_schema_json <<'COUNCIL_FILE_schemas_context_pack_schema_json_EOF' || true
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$comment": "JSON Schema is a SECONDARY tooling layer; validate_tasks remains the authoritative gate.",
  "title": "council-context-pack",
  "type": "object",
  "additionalProperties": false,
  "required": ["summary"],
  "properties": {
    "summary": { "type": "string" },
    "refs": { "type": "array", "items": { "type": "string" } },
    "files": { "type": "array", "items": { "type": "string" } },
    "snippets": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["ref", "text"],
        "properties": {
          "ref": { "type": "string" },
          "path": { "type": "string" },
          "text": { "type": "string" },
          "content_hash": { "type": "string" }
        }
      }
    },
    "profile": { "type": "string" },
    "engine": {},
    "model_tier": { "type": "string" },
    "content_hash": { "type": "string" }
  }
}
COUNCIL_FILE_schemas_context_pack_schema_json_EOF
read -r -d '' COUNCIL_FILE_schemas_design_ledger_schema_json <<'COUNCIL_FILE_schemas_design_ledger_schema_json_EOF' || true
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$comment": "JSON Schema is a SECONDARY tooling layer; validate_tasks remains the authoritative gate.",
  "title": "council-design-ledger",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "entries": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "decision"],
        "properties": {
          "id": { "type": "string" },
          "decision": { "type": "string" },
          "rationale": { "type": "string" },
          "status": { "type": "string" },
          "task_refs": { "type": "array", "items": { "type": "string" } },
          "context_refs": { "type": "array", "items": { "type": "string" } },
          "supersedes": { "type": "array", "items": { "type": "string" } },
          "content_hash": { "type": "string" }
        }
      }
    },
    "content_hash": { "type": "string" }
  }
}
COUNCIL_FILE_schemas_design_ledger_schema_json_EOF
read -r -d '' COUNCIL_FILE_schemas_events_schema_json <<'COUNCIL_FILE_schemas_events_schema_json_EOF' || true
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$comment": "JSON Schema is a SECONDARY tooling layer; validate_tasks remains the authoritative gate.",
  "title": "council-events",
  "type": "array",
  "items": {
    "type": "object",
    "additionalProperties": false,
    "required": ["type"],
    "properties": {
      "type": { "type": "string" },
      "run": { "type": "string" },
      "task_id": { "type": "string" },
      "stage": { "type": "string" },
      "timestamp": { "type": "string" },
      "message": { "type": "string" },
      "data": {},
      "engine": {},
      "model_tier": { "type": "string" },
      "content_hash": { "type": "string" }
    }
  }
}
COUNCIL_FILE_schemas_events_schema_json_EOF
read -r -d '' COUNCIL_FILE_schemas_plan_schema_json <<'COUNCIL_FILE_schemas_plan_schema_json_EOF' || true
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "council-plan",
  "type": "object",
  "additionalProperties": false,
  "required": [
    "summary",
    "approach",
    "steps",
    "risks",
    "parallelizable_tasks",
    "open_questions"
  ],
  "properties": {
    "summary": { "type": "string" },
    "approach": { "type": "string" },
    "steps": { "type": "array", "items": { "type": "string" } },
    "risks": { "type": "array", "items": { "type": "string" } },
    "parallelizable_tasks": { "type": "array", "items": { "type": "string" } },
    "open_questions": { "type": "array", "items": { "type": "string" } },
    "dev_notes": { "type": "string" },
    "spec_ref": { "type": "string" },
    "context_refs": { "type": "array", "items": { "type": "string" } },
    "archetype": { "type": "string" },
    "context_profile": { "type": "string" },
    "discovered_from": { "type": "string" },
    "supersedes": { "type": "array", "items": { "type": "string" } },
    "content_hash": { "type": "string" },
    "engine": {},
    "model_tier": { "type": "string" }
  }
}
COUNCIL_FILE_schemas_plan_schema_json_EOF
read -r -d '' COUNCIL_FILE_schemas_review_verdict_schema_json <<'COUNCIL_FILE_schemas_review_verdict_schema_json_EOF' || true
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$comment": "JSON Schema is a SECONDARY tooling layer; validate_tasks remains the authoritative gate.",
  "title": "council-review-verdict",
  "type": "object",
  "additionalProperties": false,
  "required": ["satisfied", "reasons", "issues"],
  "properties": {
    "satisfied": { "type": "boolean" },
    "reasons": { "type": "string" },
    "issues": { "type": "array", "items": { "type": "string" } },
    "task_id": { "type": "string" },
    "reviewer": { "type": "string" },
    "engine": {},
    "model_tier": { "type": "string" },
    "content_hash": { "type": "string" }
  }
}
COUNCIL_FILE_schemas_review_verdict_schema_json_EOF
read -r -d '' COUNCIL_FILE_schemas_routing_verdict_schema_json <<'COUNCIL_FILE_schemas_routing_verdict_schema_json_EOF' || true
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$comment": "JSON Schema is a SECONDARY tooling layer; validate_tasks remains the authoritative gate.",
  "title": "council-routing-verdict",
  "type": "object",
  "additionalProperties": false,
  "required": ["route", "reasons"],
  "properties": {
    "route": { "type": "string" },
    "reasons": { "type": "string" },
    "task_id": { "type": "string" },
    "candidate_routes": { "type": "array", "items": { "type": "string" } },
    "engine": {},
    "model_tier": { "type": "string" },
    "context_refs": { "type": "array", "items": { "type": "string" } },
    "content_hash": { "type": "string" }
  }
}
COUNCIL_FILE_schemas_routing_verdict_schema_json_EOF
read -r -d '' COUNCIL_FILE_schemas_run_state_schema_json <<'COUNCIL_FILE_schemas_run_state_schema_json_EOF' || true
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$comment": "JSON Schema is a SECONDARY tooling layer; validate_tasks remains the authoritative gate.",
  "title": "council-run-state",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "stage": { "type": "string" },
    "intensity": { "type": "string" },
    "rounds": { "type": "integer", "minimum": 0 },
    "task_count": { "type": "integer", "minimum": 0 },
    "spec_id": { "type": "string" },
    "spec_slug": { "type": "string" },
    "spec_relpath": { "type": "string" },
    "agents": { "type": "array", "items": { "type": "string" } },
    "integration_branch": { "type": "string" },
    "engine": {},
    "model_tier": { "type": "string" },
    "content_hash": { "type": "string" }
  }
}
COUNCIL_FILE_schemas_run_state_schema_json_EOF
read -r -d '' COUNCIL_FILE_schemas_verdict_schema_json <<'COUNCIL_FILE_schemas_verdict_schema_json_EOF' || true
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "council-verdict",
  "type": "object",
  "additionalProperties": false,
  "required": ["satisfied", "reasons", "issues"],
  "properties": {
    "satisfied": { "type": "boolean" },
    "reasons": { "type": "string" },
    "issues": { "type": "array", "items": { "type": "string" } },
    "task_id": { "type": "string" },
    "dev_notes": { "type": "string" },
    "spec_ref": { "type": "string" },
    "context_refs": { "type": "array", "items": { "type": "string" } },
    "content_hash": { "type": "string" },
    "engine": {},
    "model_tier": { "type": "string" }
  }
}
COUNCIL_FILE_schemas_verdict_schema_json_EOF
read -r -d '' COUNCIL_FILE_schemas_worker_result_schema_json <<'COUNCIL_FILE_schemas_worker_result_schema_json_EOF' || true
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$comment": "JSON Schema is a SECONDARY tooling layer; validate_tasks remains the authoritative gate.",
  "title": "council-worker-result",
  "type": "object",
  "additionalProperties": false,
  "required": ["task_id", "status"],
  "properties": {
    "task_id": { "type": "string" },
    "title": { "type": "string" },
    "model": { "type": "string" },
    "suggested_model": { "type": "string", "enum": ["haiku", "sonnet", "opus"] },
    "engine": {},
    "model_tier": { "type": "string" },
    "branch": { "type": "string" },
    "worktree": { "type": "string" },
    "committed": { "type": "boolean" },
    "summary": { "type": "string" },
    "files_changed": { "type": "array", "items": { "type": "string" } },
    "out_of_bounds": { "type": "array", "items": { "type": "string" } },
    "verify_rc": { "type": ["integer", "null"] },
    "verify_output": { "type": "string" },
    "verdict": {
      "type": ["object", "null"],
      "additionalProperties": false,
      "properties": {
        "satisfied": { "type": "boolean" },
        "reasons": { "type": "string" },
        "issues": { "type": "array", "items": { "type": "string" } },
        "task_id": { "type": "string" },
        "dev_notes": { "type": "string" },
        "spec_ref": { "type": "string" },
        "context_refs": { "type": "array", "items": { "type": "string" } },
        "content_hash": { "type": "string" },
        "engine": {},
        "model_tier": { "type": "string" }
      }
    },
    "merge": { "type": "string" },
    "status": { "type": "string" },
    "error": { "type": "string" },
    "content_hash": { "type": "string" }
  }
}
COUNCIL_FILE_schemas_worker_result_schema_json_EOF
read -r -d '' COUNCIL_SKILL_claude <<'COUNCIL_SKILL_claude_EOF' || true
---
name: council
description: Orchestrate a large, decomposable problem across Claude and Codex with triage, cross-model planning, grill, design checkpoint, guarded fan-out, review council, and resumable status/tail. Use for big parallelizable work; NOT for small or tightly-coupled/sequential changes.
---

# council

A cross-model orchestrator for problems too big for one pass. The current driver
is the installed TypeScript CLI, invoked through the `council` launcher or, if
the launcher is not on `PATH`, the skill-local CLI under
`~/.claude/skills/council`. It shells out to `claude -p` and `codex exec` and
targets the repository of the current working directory.

Your job around it is the human-facing part: clarify the brief, route small work
away from council, present checkpoints, and keep the user in control of
expensive fan-out.

Run it from the project you want council to work on. It installs into
`~/.claude/skills/council` (and `~/.codex/skills/council`) via the agent-kit
installer (`curl .../install.sh | bash`); re-run that to upgrade.

## When To Use

- Use it when the work is large, decomposes into independent parallel pieces,
  and is worth the spend.
- Do not use it for small changes, or for tightly-coupled / sequential work
  where every step depends on the last. Handle those in a normal session.

## The Loop

1. Clarify. If scope, constraints, definition of done, or repo area are
   ambiguous, ask 2-3 targeted questions with `AskUserQuestion`; otherwise
   skip. Write the result to `brief.md`.
2. Start council with the TypeScript CLI:
   ```bash
   council run --brief brief.md
   ```
   If the shell shim is unavailable, use the installed skill-local CLI:
   ```bash
   node ~/.claude/skills/council/council.mjs run --brief brief.md
   ```
   Use `--estimate` before a run when the user wants the call count and rough
   cost envelope.
3. Triage routing. Council first classifies the brief as `normal-session`,
   `plan-only`, `fleet`, or `full-council`. If it routes to `normal-session`,
   stop the run and handle the task in the current chat. If it routes to
   `plan-only`, present the design without fan-out unless the user asks to
   continue.
4. Context packs. Before model calls, council builds bounded context packs for
   each role: brief, relevant files, repo rules, prior artifacts, task
   boundaries, and review-pack requirements. Treat those packs as the source of
   truth for what each sub-agent was allowed to use.
5. Plan, critique, and grill. Independent Claude and Codex planners produce
   plans, cross-critique rounds refine them, and the grill step stress-tests the
   design for missing dependencies, unsafe parallelism, unclear ownership,
   validation gaps, and rollback/recovery risk.
6. Design checkpoint. Council stops before fan-out with `design.md`,
   `grill.md`, `consolidated_plan.md`, and `tasks.json`. Show the user the
   recommended route, risks, task DAG, affected areas, expected spend, and the
   command printed by the CLI to resume. Do not fan out without approval.
7. Fan out on approval. Workers execute DAG tasks in isolated worktrees. The
   watchdog monitors stalled workers, missing heartbeats, out-of-bounds edits,
   and verification timeouts, then marks tasks for retry or review instead of
   silently continuing.
8. Monitor progress:
   ```bash
   council status --run <run-id>
   council tail --run <run-id> --follow
   ```
   `status` is for a compact run summary; `tail` is for live worker, watchdog,
   review, and GitHub-mirror events.
9. Review-pack gate. Every completed task must produce a review pack with the
   objective, allowed files, changed files, diff summary, verification output,
   residual risks, and follow-up notes. Tasks without a valid review pack do not
   pass the gate.
10. Review council. Reviewers inspect the review packs and diffs, then return
    `accept`, `fix`, or `reject`. Accepted work is reconciled onto the
    integration branch; rejected or fix-required tasks stay out and are reported
    with their reason.
11. GitHub mirroring. When enabled, council mirrors run state, task status,
    review packs, and review-council outcomes to GitHub issues, PR comments, or
    checks. The local run directory remains canonical; GitHub is a visibility
    mirror, not the source of truth.
12. Final checkpoint. Present `report.md`: what merged, what failed, which
    review packs were accepted, which tasks need another pass, and the
    integration branch to inspect. Re-loop into design, retry selected tasks, or
    close the council run.

## Commands

```bash
council run --brief brief.md --preset standard
council run --brief brief.md --estimate
council resume --run <run-id>
council status --run <run-id>
council tail --run <run-id> --follow
council config show
council config set preset thorough
```

`run` owns the normal triage -> design checkpoint -> fan-out -> review council
loop. `resume` continues from the latest checkpoint or interrupted stage. Use
the exact resume command printed by the CLI when a run stops at a gate.

## Config Knobs

Council is driven by a preset plus optional per-role and per-gate overrides in
`council.toml` and project-local `./.council.toml`. Precedence is:
preset defaults < user `council.toml` < project `./.council.toml` < CLI flags.

| preset | rounds | codex effort | worker | max workers | review council |
|---|---:|---|---|---:|---|
| `quick` | 1 | low | claude:haiku | 4 | single reviewer |
| `standard` (default) | 2 | high | claude:haiku | 6 | two reviewers |
| `thorough` | 3 | high | claude:sonnet | 6 | two reviewers + grill retry |
| `max` | 3 | xhigh | claude:sonnet | 8 | three reviewers + grill retry |

Core role knobs:
`triager`, `planner_a`, `planner_b`, `critic_a`, `critic_b`, `griller`,
`designer`, `worker`, `watchdog`, `reviewer`, `verifier`.

Execution knobs:
`preset`, `rounds`, `codex_effort`, `max_workers`, `worker_timeout_s`,
`watchdog_interval_s`, `retry_limit`, `status_tail_lines`.

Gate and visibility knobs:
`design_checkpoint`, `require_context_pack`, `require_review_pack`,
`review_council`, `github_mirror`, `github_mirror_target`.

Context-pack knobs:
`context_pack_budget`, `context_pack_max_files`, `context_pack_include_tests`,
`review_pack_max_bytes`.

Unset role and gate keys inherit from the selected preset. Keep GitHub mirroring
off unless the user asks for it or the repository convention requires it.

## Fleet

Use `fleet` when the task DAG already exists and no plan/grill/design phase is
needed:

```bash
council fleet --tasks tasks.json --agents 'codex:gpt-5.5*3,claude:haiku*2'
council fleet --tasks tasks.json --agents 'claude:haiku*4' --estimate
```

The same context-pack, watchdog, review-pack, review-council, and GitHub mirror
gates apply.

## split

`split` carves a path out of the current repo into a new GitHub repo with that
path's history preserved, on a throwaway branch; the working branch is
untouched.

```bash
council split --path services/foo --dest myorg/foo --dry-run
council split --path services/foo --dest myorg/foo
```

`--no-push` stops at the local branch; `--visibility public` creates a public
repo. The GitHub App must be installed on the destination owner for the push.

## Phase 2 Roadmap

- Council board: persistent board view for runs, lanes, task state, blockers,
  review decisions, and integration status.
- ACP adapter: expose the same run, status, tail, checkpoint, and review-pack
  workflow through the Agent Client Protocol.
- Self-hosted engine profile: preset family for local or privately hosted model
  endpoints, with the same role, watchdog, and gate semantics.

## Notes

- Both `claude` and `codex` CLIs must be installed and authenticated.
- Runs are resumable; use `council status` to find the latest gate and `council
  resume` to continue.
- Do not edit worker worktrees by hand. Feed fixes back through retry, review
  council, or a new council run.
COUNCIL_SKILL_claude_EOF
read -r -d '' COUNCIL_SKILL_codex <<'COUNCIL_SKILL_codex_EOF' || true
---
name: council
description: Orchestrate a large, decomposable problem across Codex and Claude with triage, cross-model planning, grill, design checkpoint, guarded fan-out, review council, and resumable status/tail. Use for big parallelizable work, not for small or tightly-coupled sequential changes.
---

# council

A cross-model orchestrator for problems too big for one pass. The current driver
is the installed TypeScript CLI, invoked through the `council` launcher or, if
the launcher is not on `PATH`, the skill-local CLI under
`~/.codex/skills/council`. It shells out to `codex exec` and `claude -p` and
targets the repository of the current working directory.

Your job around it is still the human-facing part: clarify the brief, route
small work away from council, present checkpoints, and keep the user in control
of expensive fan-out.

Run it from the project you want council to work on. It installs into
`~/.codex/skills/council` (and `~/.claude/skills/council`) via the agent-kit
installer (`curl .../install.sh | bash`); re-run that to upgrade.

Use it for large work that decomposes into independent parallel pieces and is
worth the spend. Do not use it for small changes, tightly-coupled changes, or
work where every step depends on the last.

## The Loop

1. Clarify. If scope, constraints, definition of done, or repo area are
   ambiguous, ask the user 2-3 targeted questions; otherwise skip. Write the
   result to `brief.md`.
2. Start council with the TypeScript CLI:
   ```bash
   council run --brief brief.md
   ```
   If the shell shim is unavailable, use the installed skill-local CLI:
   ```bash
   node ~/.codex/skills/council/council.mjs run --brief brief.md
   ```
   Use `--estimate` before a run when the user wants the call count and rough
   cost envelope.
3. Triage routing. Council first classifies the brief as `normal-session`,
   `plan-only`, `fleet`, or `full-council`. If it routes to `normal-session`,
   stop the run and handle the task in the current chat. If it routes to
   `plan-only`, present the design without fan-out unless the user asks to
   continue.
4. Context packs. Before model calls, council builds bounded context packs for
   each role: brief, relevant files, repo rules, prior artifacts, task
   boundaries, and review-pack requirements. Treat those packs as the source of
   truth for what each sub-agent was allowed to use.
5. Plan, critique, and grill. Independent Codex and Claude planners produce
   plans, cross-critique rounds refine them, and the grill step stress-tests the
   design for missing dependencies, unsafe parallelism, unclear ownership,
   validation gaps, and rollback/recovery risk.
6. Design checkpoint. Council stops before fan-out with `design.md`,
   `grill.md`, `consolidated_plan.md`, and `tasks.json`. Show the user the
   recommended route, risks, task DAG, affected areas, expected spend, and the
   command printed by the CLI to resume. Do not fan out without approval.
7. Fan out on approval. Workers execute DAG tasks in isolated worktrees. The
   watchdog monitors stalled workers, missing heartbeats, out-of-bounds edits,
   and verification timeouts, then marks tasks for retry or review instead of
   silently continuing.
8. Monitor progress:
   ```bash
   council status --run <run-id>
   council tail --run <run-id> --follow
   ```
   `status` is for a compact run summary; `tail` is for live worker, watchdog,
   review, and GitHub-mirror events.
9. Review-pack gate. Every completed task must produce a review pack with the
   objective, allowed files, changed files, diff summary, verification output,
   residual risks, and follow-up notes. Tasks without a valid review pack do not
   pass the gate.
10. Review council. Reviewers inspect the review packs and diffs, then return
    `accept`, `fix`, or `reject`. Accepted work is reconciled onto the
    integration branch; rejected or fix-required tasks stay out and are reported
    with their reason.
11. GitHub mirroring. When enabled, council mirrors run state, task status,
    review packs, and review-council outcomes to GitHub issues, PR comments, or
    checks. The local run directory remains canonical; GitHub is a visibility
    mirror, not the source of truth.
12. Final checkpoint. Present `report.md`: what merged, what failed, which
    review packs were accepted, which tasks need another pass, and the
    integration branch to inspect. Re-loop into design, retry selected tasks, or
    close the council run.

## Commands

```bash
council run --brief brief.md --preset standard
council run --brief brief.md --estimate
council resume --run <run-id>
council status --run <run-id>
council tail --run <run-id> --follow
council config show
council config set preset thorough
```

`run` owns the normal triage -> design checkpoint -> fan-out -> review council
loop. `resume` continues from the latest checkpoint or interrupted stage. Use
the exact resume command printed by the CLI when a run stops at a gate.

## Config Knobs

Council is driven by a preset plus optional per-role and per-gate overrides in
`council.toml` and project-local `./.council.toml`. Precedence is:
preset defaults < user `council.toml` < project `./.council.toml` < CLI flags.

| preset | rounds | codex effort | worker | max workers | review council |
|---|---:|---|---|---:|---|
| `quick` | 1 | low | claude:haiku | 4 | single reviewer |
| `standard` (default) | 2 | high | claude:haiku | 6 | two reviewers |
| `thorough` | 3 | high | claude:sonnet | 6 | two reviewers + grill retry |
| `max` | 3 | xhigh | claude:sonnet | 8 | three reviewers + grill retry |

Core role knobs:
`triager`, `planner_a`, `planner_b`, `critic_a`, `critic_b`, `griller`,
`designer`, `worker`, `watchdog`, `reviewer`, `verifier`.

Execution knobs:
`preset`, `rounds`, `codex_effort`, `max_workers`, `worker_timeout_s`,
`watchdog_interval_s`, `retry_limit`, `status_tail_lines`.

Gate and visibility knobs:
`design_checkpoint`, `require_context_pack`, `require_review_pack`,
`review_council`, `github_mirror`, `github_mirror_target`.

Context-pack knobs:
`context_pack_budget`, `context_pack_max_files`, `context_pack_include_tests`,
`review_pack_max_bytes`.

Unset role and gate keys inherit from the selected preset. Keep GitHub mirroring
off unless the user asks for it or the repository convention requires it.

## Fleet

Use `fleet` when the task DAG already exists and no plan/grill/design phase is
needed:

```bash
council fleet --tasks tasks.json --agents 'codex:gpt-5.5*3,claude:haiku*2'
council fleet --tasks tasks.json --agents 'codex:gpt-5.5*4' --estimate
```

The same context-pack, watchdog, review-pack, review-council, and GitHub mirror
gates apply.

## split

`split` carves a path out of the current repo into a new GitHub repo with that
path's history preserved, on a throwaway branch; the working branch is
untouched.

```bash
council split --path services/foo --dest myorg/foo --dry-run
council split --path services/foo --dest myorg/foo
```

`--no-push` stops at the local branch; `--visibility public` creates a public
repo. The GitHub App must be installed on the destination owner for the push.

## Phase 2 Roadmap

- Council board: persistent board view for runs, lanes, task state, blockers,
  review decisions, and integration status.
- ACP adapter: expose the same run, status, tail, checkpoint, and review-pack
  workflow through the Agent Client Protocol.
- Self-hosted engine profile: preset family for local or privately hosted model
  endpoints, with the same role, watchdog, and gate semantics.

## Notes

- Both `codex` and `claude` CLIs must be installed and authenticated.
- Runs are resumable; use `council status` to find the latest gate and `council
  resume` to continue.
- Do not edit worker worktrees by hand. Feed fixes back through retry, review
  council, or a new council run.
COUNCIL_SKILL_codex_EOF
install_council() {
  local dir="$1" skill="$2"
  write_file "${dir}/council/council.mjs" 0755 "${COUNCIL_FILE_council_mjs}"
  write_file "${dir}/council/prompts/_baseline.md" 0644 "${COUNCIL_FILE_prompts__baseline_md}"
  write_file "${dir}/council/prompts/consolidator.md" 0644 "${COUNCIL_FILE_prompts_consolidator_md}"
  write_file "${dir}/council/prompts/correct_course.md" 0644 "${COUNCIL_FILE_prompts_correct_course_md}"
  write_file "${dir}/council/prompts/critic.md" 0644 "${COUNCIL_FILE_prompts_critic_md}"
  write_file "${dir}/council/prompts/design_consolidator.md" 0644 "${COUNCIL_FILE_prompts_design_consolidator_md}"
  write_file "${dir}/council/prompts/design_lens.md" 0644 "${COUNCIL_FILE_prompts_design_lens_md}"
  write_file "${dir}/council/prompts/design_vote.md" 0644 "${COUNCIL_FILE_prompts_design_vote_md}"
  write_file "${dir}/council/prompts/designer.md" 0644 "${COUNCIL_FILE_prompts_designer_md}"
  write_file "${dir}/council/prompts/grill.md" 0644 "${COUNCIL_FILE_prompts_grill_md}"
  write_file "${dir}/council/prompts/planner.md" 0644 "${COUNCIL_FILE_prompts_planner_md}"
  write_file "${dir}/council/prompts/review_triage.md" 0644 "${COUNCIL_FILE_prompts_review_triage_md}"
  write_file "${dir}/council/prompts/reviewer_acceptance.md" 0644 "${COUNCIL_FILE_prompts_reviewer_acceptance_md}"
  write_file "${dir}/council/prompts/reviewer_adversarial.md" 0644 "${COUNCIL_FILE_prompts_reviewer_adversarial_md}"
  write_file "${dir}/council/prompts/reviewer_edgecase.md" 0644 "${COUNCIL_FILE_prompts_reviewer_edgecase_md}"
  write_file "${dir}/council/prompts/reviewpack/checkpoint-1.html" 0644 "${COUNCIL_FILE_prompts_reviewpack_checkpoint_1_html}"
  write_file "${dir}/council/prompts/reviewpack/checkpoint-1.md" 0644 "${COUNCIL_FILE_prompts_reviewpack_checkpoint_1_md}"
  write_file "${dir}/council/prompts/reviewpack/checkpoint-2.html" 0644 "${COUNCIL_FILE_prompts_reviewpack_checkpoint_2_html}"
  write_file "${dir}/council/prompts/reviewpack/checkpoint-2.md" 0644 "${COUNCIL_FILE_prompts_reviewpack_checkpoint_2_md}"
  write_file "${dir}/council/prompts/reviewpack/design-checkpoint.html" 0644 "${COUNCIL_FILE_prompts_reviewpack_design_checkpoint_html}"
  write_file "${dir}/council/prompts/reviewpack/design-checkpoint.md" 0644 "${COUNCIL_FILE_prompts_reviewpack_design_checkpoint_md}"
  write_file "${dir}/council/prompts/reviser.md" 0644 "${COUNCIL_FILE_prompts_reviser_md}"
  write_file "${dir}/council/prompts/story_author.md" 0644 "${COUNCIL_FILE_prompts_story_author_md}"
  write_file "${dir}/council/prompts/story_check.md" 0644 "${COUNCIL_FILE_prompts_story_check_md}"
  write_file "${dir}/council/prompts/story_template.md" 0644 "${COUNCIL_FILE_prompts_story_template_md}"
  write_file "${dir}/council/prompts/survey.md" 0644 "${COUNCIL_FILE_prompts_survey_md}"
  write_file "${dir}/council/prompts/triage_judge.md" 0644 "${COUNCIL_FILE_prompts_triage_judge_md}"
  write_file "${dir}/council/prompts/verifier.md" 0644 "${COUNCIL_FILE_prompts_verifier_md}"
  write_file "${dir}/council/prompts/worker.md" 0644 "${COUNCIL_FILE_prompts_worker_md}"
  write_file "${dir}/council/schemas/amendment.schema.json" 0644 "${COUNCIL_FILE_schemas_amendment_schema_json}"
  write_file "${dir}/council/schemas/consolidated.schema.json" 0644 "${COUNCIL_FILE_schemas_consolidated_schema_json}"
  write_file "${dir}/council/schemas/context-pack.schema.json" 0644 "${COUNCIL_FILE_schemas_context_pack_schema_json}"
  write_file "${dir}/council/schemas/design-ledger.schema.json" 0644 "${COUNCIL_FILE_schemas_design_ledger_schema_json}"
  write_file "${dir}/council/schemas/events.schema.json" 0644 "${COUNCIL_FILE_schemas_events_schema_json}"
  write_file "${dir}/council/schemas/plan.schema.json" 0644 "${COUNCIL_FILE_schemas_plan_schema_json}"
  write_file "${dir}/council/schemas/review-verdict.schema.json" 0644 "${COUNCIL_FILE_schemas_review_verdict_schema_json}"
  write_file "${dir}/council/schemas/routing-verdict.schema.json" 0644 "${COUNCIL_FILE_schemas_routing_verdict_schema_json}"
  write_file "${dir}/council/schemas/run-state.schema.json" 0644 "${COUNCIL_FILE_schemas_run_state_schema_json}"
  write_file "${dir}/council/schemas/verdict.schema.json" 0644 "${COUNCIL_FILE_schemas_verdict_schema_json}"
  write_file "${dir}/council/schemas/worker-result.schema.json" 0644 "${COUNCIL_FILE_schemas_worker_result_schema_json}"
  write_file "${dir}/council/SKILL.md" 0644 "$skill"
  if [ ! -e "${dir}/council/council.toml" ]; then
    write_file "${dir}/council/council.toml" 0644 "${COUNCIL_FILE_council_toml}"
  else
    log "preserving existing ${dir}/council/council.toml"
  fi
}
if [ "${INSTALL_CLAUDE}" = 1 ]; then install_council "${SKILLS_DIR}" "${COUNCIL_SKILL_claude}"; fi
if [ "${INSTALL_CODEX}" = 1 ]; then install_council "${CODEX_SKILLS_DIR}" "${COUNCIL_SKILL_codex}"; fi

# -----------------------------------------------------------------
# Path allowlist (gitignore-style). Hooks below skip any tool input
# whose target matches a pattern here. Defaults exclude paths that
# typically carry secrets so an Edit on `.env` does not exfiltrate
# the path to the KB recall query.
# -----------------------------------------------------------------
if [ "${INSTALL_CLAUDE}" = 1 ] || [ "${INSTALL_CODEX}" = 1 ]; then
  read -r -d '' ALLOWLIST_DEFAULTS <<'ALLOW' || true
# knowledge-system auto-MCP path allowlist (gitignore-style).
# Lines starting with `#` are comments. Patterns match against the
# full target path the hook is about to act on. Hooks SKIP any
# match, so adding a line here disables auto-MCP for that path.
#
# Re-running the installer never overwrites this file once you've
# customised it — only the initial install seeds these defaults.

# SDD paths; git-commit-capture has no allowlist and stop-session-digest digests transcripts, so neither is path-suppressible.
**/.specify/**
**/specs/**

# Secrets-bearing files
*.env
.env*
*.secret
*.key
*.pem
*.p12
*.pfx
*.jks
secrets/**
credentials*
**/credentials/**
id_rsa
id_ed25519
**/.ssh/**

# Vault / KMS / cloud auth state
.aws/**
.config/gcloud/**
.kube/config
.kube/cache/**

# Browser / OS keychains
**/Library/Keychains/**
**/Mozilla/Firefox/**
**/Google/Chrome/Default/Login Data*
ALLOW
fi

if [ "${INSTALL_CLAUDE}" = 1 ] && [ ! -e "${ALLOWLIST}" ]; then
  write_file "${ALLOWLIST}" 0644 "${ALLOWLIST_DEFAULTS}"
elif [ "${INSTALL_CLAUDE}" = 1 ]; then
  log "preserving existing ${ALLOWLIST}"
fi

if [ "${INSTALL_CODEX}" = 1 ] && [ ! -e "${CODEX_ALLOWLIST}" ]; then
  write_file "${CODEX_ALLOWLIST}" 0644 "${ALLOWLIST_DEFAULTS}"
elif [ "${INSTALL_CODEX}" = 1 ]; then
  log "preserving existing ${CODEX_ALLOWLIST}"
fi

# -----------------------------------------------------------------
# Hook: PreToolUse — Edit/Write/MultiEdit/apply_patch recall
# -----------------------------------------------------------------
read -r -d '' PRE_TOOL_USE_EDIT_HOOK <<'HOOK' || true
#!/usr/bin/env bash
# PreToolUse hook for Edit/Write/MultiEdit/apply_patch. Looks at the file path
# the agent is about to touch and runs a `knowledge.recall` against
# it so prior captures referencing that file (or its module) surface
# before the edit lands.
#
# Safety:
#   - Honours KB_AUTO_MCP_DISABLED=1 (panic switch).
#   - Honours the client allowlist (skip if match).
#   - Per-session dedupe: only fires once per (session, file_path)
#     so an N-Edit sequence on the same file does not stutter.
#   - Silent on failure — the KB being unreachable must never block
#     an edit.

set -u

[ "${KB_AUTO_MCP_DISABLED:-0}" = 1 ] && exit 0
[ -z "${KB_BEARER_TOKEN:-}" ] && exit 0

KB_URL="${KB_URL:-@KB_URL@}"
case "${KB_URL}" in
  */mcp) KB_MCP_URL="${KB_URL}" ;;
  *) KB_MCP_URL="${KB_URL%/}/mcp" ;;
esac
CLIENT_HOME="${KB_AUTO_MCP_HOME:-${CLAUDE_CONFIG_DIR:-$HOME/.claude}}"
STATE_DIR="${KB_AUTO_MCP_STATE_DIR:-${CLIENT_HOME}/state}"
ALLOWLIST="${KB_AUTO_MCP_ALLOWLIST:-${CLIENT_HOME}/.knowledge-system-allowlist}"

input=$(cat 2>/dev/null || true)
file_path=$(printf '%s' "${input}" | python3 -c '
import json, re, sys
try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(0)

inputs = []
for key in ("tool_input", "input", "arguments", "params"):
    value = data.get(key)
    if isinstance(value, dict):
        inputs.append(value)
tool = data.get("tool")
if isinstance(tool, dict):
    for key in ("input", "arguments", "params"):
        value = tool.get(key)
        if isinstance(value, dict):
            inputs.append(value)

for source in inputs + [data]:
    for key in ("file_path", "filePath", "path", "target_file", "targetFile"):
        value = source.get(key) if isinstance(source, dict) else None
        if isinstance(value, str) and value.strip():
            print(value.strip(), end="")
            sys.exit(0)

patch_parts = []
for source in inputs + [data]:
    if not isinstance(source, dict):
        continue
    for key in ("patch", "content", "body", "diff", "input"):
        value = source.get(key)
        if isinstance(value, str):
            patch_parts.append(value)

for line in "\n".join(patch_parts).splitlines():
    match = re.match(r"^\*\*\* (?:Update|Add|Delete) File: (.+)$", line)
    if match:
        print(match.group(1).strip(), end="")
        sys.exit(0)
' 2>/dev/null || true)
[ -z "${file_path}" ] && exit 0

# Allowlist match — git-style globbing via a python helper. fnmatch
# does not span `/`, so we walk pattern + path together.
if [ -r "${ALLOWLIST}" ]; then
  if python3 - "${ALLOWLIST}" "${file_path}" <<'PY' 2>/dev/null
import fnmatch, sys, os
allowlist, path = sys.argv[1], sys.argv[2]
with open(allowlist) as f:
    for line in f:
        pat = line.strip()
        if not pat or pat.startswith("#"): continue
        # Translate `**` to a recursive match by relying on fnmatch's
        # `*` against the basename and the full path.
        if fnmatch.fnmatch(path, pat) or fnmatch.fnmatch(os.path.basename(path), pat):
            sys.exit(0)
    sys.exit(1)
PY
  then
    exit 0
  fi
fi

# Per-session dedupe: state/sessions/<session>/edit-<sha1-of-path>
session="${CLAUDE_SESSION_ID:-${CODEX_THREAD_ID:-${CODEX_SESSION_ID:-unknown}}}"
mkdir -p "${STATE_DIR}/sessions/${session}"
marker="${STATE_DIR}/sessions/${session}/edit-$(printf '%s' "${file_path}" | shasum -a 1 | cut -c1-12)"
[ -e "${marker}" ] && exit 0
: > "${marker}"

canonical_project_scope_from_origin() {
  remote="$(git remote get-url origin 2>/dev/null || true)"
  case "${remote}" in
    git@github.com:*) path="${remote#git@github.com:}" ;;
    https://github.com/*) path="${remote#https://github.com/}" ;;
    ssh://git@github.com/*) path="${remote#ssh://git@github.com/}" ;;
    *) return 0 ;;
  esac
  path="${path%.git}"
  owner="${path%%/*}"
  repo="${path#*/}"
  [ "${repo}" != "${path}" ] || return 0
  [ -n "${owner}" ] && [ -n "${repo}" ] || return 0
  case "${repo}" in */*) return 0 ;; esac
  printf 'project:%s/%s' \
    "$(printf '%s' "${owner}" | tr '[:upper:]' '[:lower:]')" \
    "$(printf '%s' "${repo}" | tr '[:upper:]' '[:lower:]')"
}

# Scope to a canonical GitHub repo and query by filename + parent + path; this
# keeps automatic edit recall focused while still giving FTS enough terms.
repo_scope="$(canonical_project_scope_from_origin)"
scope="${KB_RECALL_SCOPE:-${repo_scope}}"

basename=$(basename "${file_path}")
parent=$(basename "$(dirname "${file_path}")")
query="${basename} ${parent} ${file_path}"
mode="${KB_RECALL_HOOK_MODE:-hybrid}"
limit="${KB_RECALL_EDIT_LIMIT:-2}"

recall_payload() {
  python3 -c 'import json,sys
args = {"query": sys.argv[1], "limit": int(sys.argv[2]), "mode": sys.argv[3]}
if sys.argv[4]:
    args["scope"] = sys.argv[4]
print(json.dumps({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge.recall","arguments":args}}))' \
    "$1" "$2" "$3" "$4"
}

call_recall() {
  payload=$(recall_payload "$1" "$2" "$3" "$4") || return 1
  curl -sS --connect-timeout 3 --max-time 5 \
    -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "${payload}" \
    "${KB_MCP_URL}" 2>/dev/null
}

response=$(call_recall "${query}" "${limit}" "${mode}" "${scope}") || response=""
if [ -z "${response}" ] && [ "${mode}" != "fast" ]; then
  response=$(call_recall "${query}" "${limit}" fast "${scope}") || exit 0
fi
[ -n "${response}" ] || exit 0

printf '%s' "${response}" | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
    hits = data["result"]["structuredContent"]["hits"]
    if not hits: sys.exit(0)
    print()
    print(f"## Related captures for this file")
    for h in hits:
        title = h.get("title", "")
        scope = h.get("scope", "")
        note_id = h.get("id", "")
        print(f"- **{title}** (`{scope}`) — id `{note_id}`")
        snip = h.get("snippet","").replace("\n"," ").strip()
        if snip: print(f"  > {snip[:160]}")
except Exception:
    sys.exit(0)' 2>/dev/null || true
HOOK

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${HOOKS_DIR}/pre-tool-use-edit-recall.sh" 0755 "${PRE_TOOL_USE_EDIT_HOOK}"
fi

# -----------------------------------------------------------------
# Hook: PreToolUse — Bash matching `git commit` capture
# -----------------------------------------------------------------
read -r -d '' PRE_TOOL_USE_GIT_COMMIT_HOOK <<'HOOK' || true
#!/usr/bin/env bash
# PreToolUse hook for Bash. Fires only when the command looks like a
# `git commit -m "..."` (or heredoc variant). Captures the commit
# message as a `decision` note scoped to the current repo, with a
# distinct source so the operator can bulk-revoke if needed.
#
# Skips merge / fixup / WIP commits — too noisy to capture.

set -u

[ "${KB_AUTO_MCP_DISABLED:-0}" = 1 ] && exit 0
[ -z "${KB_BEARER_TOKEN:-}" ] && exit 0

KB_URL="${KB_URL:-@KB_URL@}"
case "${KB_URL}" in
  */mcp) KB_MCP_URL="${KB_URL}" ;;
  *) KB_MCP_URL="${KB_URL%/}/mcp" ;;
esac

input=$(cat 2>/dev/null || true)
parsed=$(printf '%s' "${input}" | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(0)

tool = data.get("tool_name") or data.get("name") or ""
raw_tool = data.get("tool")
if isinstance(raw_tool, str):
    tool = tool or raw_tool
elif isinstance(raw_tool, dict):
    tool = tool or raw_tool.get("name") or ""

inputs = []
for key in ("tool_input", "input", "arguments", "params"):
    value = data.get(key)
    if isinstance(value, dict):
        inputs.append(value)
if isinstance(raw_tool, dict):
    for key in ("input", "arguments", "params"):
        value = raw_tool.get(key)
        if isinstance(value, dict):
            inputs.append(value)

command = ""
for source in inputs + [data]:
    if not isinstance(source, dict):
        continue
    for key in ("command", "cmd", "script", "shell_command", "shellCommand"):
        value = source.get(key)
        if isinstance(value, str) and value.strip():
            command = value.strip()
            break
    if command:
        break

print(f"{tool}\x1f{command}", end="")' 2>/dev/null || true)
tool="${parsed%%$'\x1f'*}"
command="${parsed#*$'\x1f'}"
[ "${tool}" = "${parsed}" ] && command=""
if [ -n "${tool}" ] && [ "${tool}" != "Bash" ] && [ "${tool}" != "bash" ]; then
  exit 0
fi

# Match `git commit -m "..."` shape; both single and double quotes.
case "${command}" in
  *"git commit"*"-m"*) : ;;
  *) exit 0 ;;
esac
# Skip noise: merge, fixup, WIP.
case "${command}" in
  *"fixup!"*|*"WIP"*|*"wip"*|*"Merge "*) exit 0 ;;
esac

# Extract the message: everything between the first matching quote
# pair after `-m`. Defer the regex to python for sanity.
title=$(printf '%s' "${command}" | python3 -c '
import re, sys
cmd = sys.stdin.read()
m = re.search(r"-m\s+(\x27([^\x27]+)\x27|\"([^\"]+)\")", cmd)
if not m: sys.exit(0)
print(m.group(2) or m.group(3) or "", end="")' 2>/dev/null)
[ -z "${title}" ] && exit 0

canonical_project_scope_from_origin() {
  remote="$(git remote get-url origin 2>/dev/null || true)"
  case "${remote}" in
    git@github.com:*) path="${remote#git@github.com:}" ;;
    https://github.com/*) path="${remote#https://github.com/}" ;;
    ssh://git@github.com/*) path="${remote#ssh://git@github.com/}" ;;
    *) return 0 ;;
  esac
  path="${path%.git}"
  owner="${path%%/*}"
  repo="${path#*/}"
  [ "${repo}" != "${path}" ] || return 0
  [ -n "${owner}" ] && [ -n "${repo}" ] || return 0
  case "${repo}" in */*) return 0 ;; esac
  printf 'project:%s/%s' \
    "$(printf '%s' "${owner}" | tr '[:upper:]' '[:lower:]')" \
    "$(printf '%s' "${repo}" | tr '[:upper:]' '[:lower:]')"
}

scope="$(canonical_project_scope_from_origin)"

body=$(cat <<BODY
Commit message: ${title}

Captured automatically by ${KB_AUTO_MCP_CLIENT_NAME:-the Claude PreToolUse} \`git commit\` hook. The
diff and surrounding context live in git history.
BODY
)

source="${KB_AUTO_MCP_SOURCE:-claude-code:auto-capture:git-commit}"
payload=$(python3 -c 'import json,sys
args = {
  "title": sys.argv[1],
  "body": sys.argv[2],
  "source": sys.argv[4],
  "tags": ["auto-capture","git-commit"]
}
if sys.argv[3]:
    args["scope"] = sys.argv[3]
print(json.dumps({
  "jsonrpc":"2.0","id":1,"method":"tools/call","params":{
    "name":"knowledge.capture_decision","arguments":args}}))' "${title}" "${body}" "${scope}" "${source}")

curl -sS --connect-timeout 3 --max-time 5 \
  -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
  -H "Content-Type: application/json" \
  -d "${payload}" \
  "${KB_MCP_URL}" >/dev/null 2>&1 || true
HOOK

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${HOOKS_DIR}/pre-tool-use-git-commit-capture.sh" 0755 "${PRE_TOOL_USE_GIT_COMMIT_HOOK}"
fi

# -----------------------------------------------------------------
# Hook: Stop — session-digest auto-capture
# -----------------------------------------------------------------
read -r -d '' STOP_SESSION_DIGEST_HOOK <<'HOOK' || true
#!/usr/bin/env bash
# Claude Stop hook: summarize reusable lessons from the transcript and
# capture a capped set into the KB. Silent on failure.

set -u

[ "${KB_AUTO_MCP_DISABLED:-0}" = 1 ] && exit 0
[ -z "${KB_BEARER_TOKEN:-}" ] && exit 0

KB_URL="${KB_URL:-@KB_URL@}"
case "${KB_URL}" in
  */mcp) KB_MCP_URL="${KB_URL}" ;;
  *) KB_MCP_URL="${KB_URL%/}/mcp" ;;
esac

CLAUDE_STATE="${CLAUDE_CONFIG_DIR:-${HOME}/.claude}"
STATE_DIR="${CLAUDE_STATE}/state"
LOG="${STATE_DIR}/auto-kb.log"
mkdir -p "${STATE_DIR}"

input="$(cat 2>/dev/null || true)"
read -r session transcript_path < <(printf '%s' "${input}" | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
except Exception:
    print("unknown")
    sys.exit(0)
session = data.get("session_id") or data.get("conversation_id") or data.get("thread_id") or "unknown"
path = data.get("transcript_path") or data.get("transcriptPath") or data.get("log_path") or ""
print(session, path)
' 2>/dev/null)

[ -n "${transcript_path:-}" ] && [ -r "${transcript_path}" ] || exit 0

session_dir="${STATE_DIR}/sessions/${session}"
mkdir -p "${session_dir}"
remaining_file="${session_dir}/digest-budget"
if [ -r "${remaining_file}" ]; then
  remaining="$(cat "${remaining_file}")"
else
  remaining="${KB_DIGEST_MAX_CAPTURES:-4}"
fi
[ "${remaining}" -gt 0 ] 2>/dev/null || exit 0

transcript="$(python3 - "${transcript_path}" "${KB_DIGEST_MAX_CHARS:-30000}" <<'PY' 2>/dev/null
import json, sys
path, max_chars = sys.argv[1], int(sys.argv[2])
rows = []

def text(value):
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return " ".join(text(v) for v in value)
    if isinstance(value, dict):
        if isinstance(value.get("text"), str):
            return value["text"]
        if "content" in value:
            return text(value["content"])
    return ""

with open(path, errors="ignore") as f:
    for line in f:
        line = line.strip()
        if not line:
            continue
        try:
            row = json.loads(line)
        except Exception:
            continue
        role = row.get("role") or row.get("type") or row.get("source") or "?"
        content = text(row.get("content") or row.get("text") or row.get("message") or row)
        if content:
            rows.append(f"[{role}] {content}")
out = "\n".join(rows)
print(out[-max_chars:])
PY
)" || exit 0

[ -n "${transcript}" ] || exit 0

payload="$(python3 -c 'import json,sys; print(json.dumps({
  "jsonrpc":"2.0","id":1,"method":"tools/call","params":{
    "name":"knowledge.digest_transcript",
    "arguments":{"transcript":sys.argv[1],"max_candidates":int(sys.argv[2])}}}))' \
  "${transcript}" "${remaining}")"

response="$(curl -sS --connect-timeout 5 --max-time 60 \
  -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
  -H "Content-Type: application/json" \
  -d "${payload}" \
  "${KB_MCP_URL}" 2>/dev/null)" || exit 0

candidates="$(printf '%s' "${response}" | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
    print(json.dumps(data["result"]["structuredContent"]["candidates"]))
except Exception:
    print("[]")
' 2>/dev/null || echo "[]")"

canonical_project_scope_from_origin() {
  remote="$(git remote get-url origin 2>/dev/null || true)"
  case "${remote}" in
    git@github.com:*) path="${remote#git@github.com:}" ;;
    https://github.com/*) path="${remote#https://github.com/}" ;;
    ssh://git@github.com/*) path="${remote#ssh://git@github.com/}" ;;
    *) return 0 ;;
  esac
  path="${path%.git}"
  owner="${path%%/*}"
  repo="${path#*/}"
  [ "${repo}" != "${path}" ] || return 0
  [ -n "${owner}" ] && [ -n "${repo}" ] || return 0
  case "${repo}" in */*) return 0 ;; esac
  printf 'project:%s/%s' \
    "$(printf '%s' "${owner}" | tr '[:upper:]' '[:lower:]')" \
    "$(printf '%s' "${repo}" | tr '[:upper:]' '[:lower:]')"
}

fallback_scope="$(canonical_project_scope_from_origin)"
emitted=0

while IFS= read -r line; do
  [ -n "${line}" ] || continue
  [ "${remaining}" -gt 0 ] 2>/dev/null || break

  title="$(printf '%s' "${line}" | python3 -c 'import json,sys; print(json.loads(sys.stdin.read()).get("title",""), end="")')"
  body="$(printf '%s' "${line}" | python3 -c 'import json,sys; print(json.loads(sys.stdin.read()).get("body",""), end="")')"
  topic="$(printf '%s' "${line}" | python3 -c 'import json,sys; print((json.loads(sys.stdin.read()).get("suggested_topic") or ""), end="")')"
  tags_json="$(printf '%s' "${line}" | python3 -c 'import json,sys; print(json.dumps(json.loads(sys.stdin.read()).get("suggested_tags") or []), end="")')"
  [ -n "${title}" ] && [ -n "${body}" ] || continue

  dedupe_payload="$(python3 -c 'import json,sys; print(json.dumps({
    "jsonrpc":"2.0","id":1,"method":"tools/call","params":{
      "name":"knowledge.recall","arguments":{
        "query": sys.argv[1], "limit": 1, "mode": "hybrid"}}}))' "${title} ${body}")"
  duplicate_count="$(curl -sS --connect-timeout 3 --max-time 5 \
    -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "${dedupe_payload}" \
    "${KB_MCP_URL}" 2>/dev/null | python3 -c '
import json, sys
try:
    hits = json.load(sys.stdin)["result"]["structuredContent"]["hits"]
    print(1 if hits and float(hits[0].get("score", 0)) >= float("'${KB_DIGEST_DEDUPE_SCORE:-0.86}'") else 0)
except Exception:
    print(0)
' 2>/dev/null || echo 0)"
  [ "${duplicate_count}" = 1 ] && continue

  scope="${fallback_scope}"
  [ -n "${topic}" ] && scope="topic:${topic}"
  capture_payload="$(python3 -c 'import json,sys
args = {
    "title": sys.argv[1],
    "body": sys.argv[2],
    "source": "claude-code:auto-digest:" + sys.argv[4],
    "session_id": sys.argv[4],
    "tags": json.loads(sys.argv[5])
}
if sys.argv[3]:
    args["scope"] = sys.argv[3]
print(json.dumps({
    "jsonrpc":"2.0","id":1,"method":"tools/call","params":{
      "name":"knowledge.capture_lesson","arguments":args}}))' \
    "${title}" "${body}" "${scope}" "${session}" "${tags_json}")"
  curl -sS --connect-timeout 3 --max-time 10 \
    -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "${capture_payload}" \
    "${KB_MCP_URL}" >/dev/null 2>&1 || continue
  remaining=$((remaining - 1))
  emitted=$((emitted + 1))
done < <(printf '%s' "${candidates}" | python3 -c '
import json, sys
try:
    for row in json.load(sys.stdin):
        print(json.dumps(row))
except Exception:
    pass
' 2>/dev/null)

echo "${remaining}" > "${remaining_file}"
echo "$(date -u +%FT%TZ) claude-stop-digest session=${session} emitted=${emitted}" >>"${LOG}" 2>/dev/null
HOOK

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${HOOKS_DIR}/stop-session-digest.sh" 0755 "${STOP_SESSION_DIGEST_HOOK}"
fi

# -----------------------------------------------------------------
# Codex project hook mirror
# -----------------------------------------------------------------
read -r -d '' CODEX_STOP_DIGEST_HOOK <<'HOOK' || true
#!/usr/bin/env bash
# Codex Stop hook: summarize reusable lessons from the transcript and
# capture a capped set into the KB. Silent on failure.

set -u

[ "${KB_AUTO_MCP_DISABLED:-0}" = 1 ] && exit 0
[ -z "${KB_BEARER_TOKEN:-}" ] && exit 0

KB_URL="${KB_URL:-@KB_URL@}"
case "${KB_URL}" in
  */mcp) KB_MCP_URL="${KB_URL}" ;;
  *) KB_MCP_URL="${KB_URL%/}/mcp" ;;
esac

CODEX_STATE="${CODEX_HOME:-${HOME}/.codex}"
STATE_DIR="${CODEX_STATE}/state"
LOG="${STATE_DIR}/auto-kb.log"
mkdir -p "${STATE_DIR}"

input="$(cat 2>/dev/null || true)"
read -r session transcript_path < <(printf '%s' "${input}" | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
except Exception:
    print("unknown")
    sys.exit(0)
session = data.get("session_id") or data.get("conversation_id") or data.get("thread_id") or "unknown"
path = data.get("transcript_path") or data.get("transcriptPath") or data.get("log_path") or ""
print(session, path)
' 2>/dev/null)

[ -n "${transcript_path:-}" ] && [ -r "${transcript_path}" ] || exit 0

session_dir="${STATE_DIR}/sessions/${session}"
mkdir -p "${session_dir}"
remaining_file="${session_dir}/digest-budget"
if [ -r "${remaining_file}" ]; then
  remaining="$(cat "${remaining_file}")"
else
  remaining="${KB_DIGEST_MAX_CAPTURES:-4}"
fi
[ "${remaining}" -gt 0 ] 2>/dev/null || exit 0

transcript="$(python3 - "${transcript_path}" "${KB_DIGEST_MAX_CHARS:-30000}" <<'PY' 2>/dev/null
import json, sys
path, max_chars = sys.argv[1], int(sys.argv[2])
rows = []

def text(value):
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return " ".join(text(v) for v in value)
    if isinstance(value, dict):
        if isinstance(value.get("text"), str):
            return value["text"]
        if "content" in value:
            return text(value["content"])
    return ""

with open(path, errors="ignore") as f:
    for line in f:
        line = line.strip()
        if not line:
            continue
        try:
            row = json.loads(line)
        except Exception:
            continue
        role = row.get("role") or row.get("type") or row.get("source") or "?"
        content = text(row.get("content") or row.get("text") or row.get("message") or row)
        if content:
            rows.append(f"[{role}] {content}")
out = "\n".join(rows)
print(out[-max_chars:])
PY
)" || exit 0

[ -n "${transcript}" ] || exit 0

payload="$(python3 -c 'import json,sys; print(json.dumps({
  "jsonrpc":"2.0","id":1,"method":"tools/call","params":{
    "name":"knowledge.digest_transcript",
    "arguments":{"transcript":sys.argv[1],"max_candidates":int(sys.argv[2])}}}))' \
  "${transcript}" "${remaining}")"

response="$(curl -sS --connect-timeout 5 --max-time 60 \
  -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
  -H "Content-Type: application/json" \
  -d "${payload}" \
  "${KB_MCP_URL}" 2>/dev/null)" || exit 0

candidates="$(printf '%s' "${response}" | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
    print(json.dumps(data["result"]["structuredContent"]["candidates"]))
except Exception:
    print("[]")
' 2>/dev/null || echo "[]")"

canonical_project_scope_from_origin() {
  remote="$(git remote get-url origin 2>/dev/null || true)"
  case "${remote}" in
    git@github.com:*) path="${remote#git@github.com:}" ;;
    https://github.com/*) path="${remote#https://github.com/}" ;;
    ssh://git@github.com/*) path="${remote#ssh://git@github.com/}" ;;
    *) return 0 ;;
  esac
  path="${path%.git}"
  owner="${path%%/*}"
  repo="${path#*/}"
  [ "${repo}" != "${path}" ] || return 0
  [ -n "${owner}" ] && [ -n "${repo}" ] || return 0
  case "${repo}" in */*) return 0 ;; esac
  printf 'project:%s/%s' \
    "$(printf '%s' "${owner}" | tr '[:upper:]' '[:lower:]')" \
    "$(printf '%s' "${repo}" | tr '[:upper:]' '[:lower:]')"
}

fallback_scope="$(canonical_project_scope_from_origin)"
emitted=0

while IFS= read -r line; do
  [ -n "${line}" ] || continue
  [ "${remaining}" -gt 0 ] 2>/dev/null || break

  title="$(printf '%s' "${line}" | python3 -c 'import json,sys; print(json.loads(sys.stdin.read()).get("title",""), end="")')"
  body="$(printf '%s' "${line}" | python3 -c 'import json,sys; print(json.loads(sys.stdin.read()).get("body",""), end="")')"
  topic="$(printf '%s' "${line}" | python3 -c 'import json,sys; print((json.loads(sys.stdin.read()).get("suggested_topic") or ""), end="")')"
  tags_json="$(printf '%s' "${line}" | python3 -c 'import json,sys; print(json.dumps(json.loads(sys.stdin.read()).get("suggested_tags") or []), end="")')"
  [ -n "${title}" ] && [ -n "${body}" ] || continue

  dedupe_payload="$(python3 -c 'import json,sys; print(json.dumps({
    "jsonrpc":"2.0","id":1,"method":"tools/call","params":{
      "name":"knowledge.recall","arguments":{
        "query": sys.argv[1], "limit": 1, "mode": "hybrid"}}}))' "${title} ${body}")"
  duplicate_count="$(curl -sS --connect-timeout 3 --max-time 5 \
    -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "${dedupe_payload}" \
    "${KB_MCP_URL}" 2>/dev/null | python3 -c '
import json, sys
try:
    hits = json.load(sys.stdin)["result"]["structuredContent"]["hits"]
    print(1 if hits and float(hits[0].get("score", 0)) >= float("'${KB_DIGEST_DEDUPE_SCORE:-0.86}'") else 0)
except Exception:
    print(0)
' 2>/dev/null || echo 0)"
  [ "${duplicate_count}" = 1 ] && continue

  scope="${fallback_scope}"
  [ -n "${topic}" ] && scope="topic:${topic}"
  capture_payload="$(python3 -c 'import json,sys
args = {
    "title": sys.argv[1],
    "body": sys.argv[2],
    "source": "codex:auto-digest:" + sys.argv[4],
    "session_id": sys.argv[4],
    "tags": json.loads(sys.argv[5])
}
if sys.argv[3]:
    args["scope"] = sys.argv[3]
print(json.dumps({
    "jsonrpc":"2.0","id":1,"method":"tools/call","params":{
      "name":"knowledge.capture_lesson","arguments":args}}))' \
    "${title}" "${body}" "${scope}" "${session}" "${tags_json}")"
  curl -sS --connect-timeout 3 --max-time 10 \
    -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "${capture_payload}" \
    "${KB_MCP_URL}" >/dev/null 2>&1 || continue
  remaining=$((remaining - 1))
  emitted=$((emitted + 1))
done < <(printf '%s' "${candidates}" | python3 -c '
import json, sys
try:
    for row in json.load(sys.stdin):
        print(json.dumps(row))
except Exception:
    pass
' 2>/dev/null)

echo "${remaining}" > "${remaining_file}"
echo "$(date -u +%FT%TZ) codex-stop-digest session=${session} emitted=${emitted}" >>"${LOG}" 2>/dev/null
HOOK

read -r -d '' CODEX_HOOKS_JSON <<HOOKS || true
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Edit|Write|apply_patch",
        "hooks": [
          {
            "type": "command",
            "command": "env KB_AUTO_MCP_HOME=${CODEX_HOME} ${CODEX_HOOKS_DIR}/pre-tool-use-edit-recall.sh",
            "timeout": 5,
            "statusMessage": "Loading file KB context"
          }
        ]
      },
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "env KB_AUTO_MCP_HOME=${CODEX_HOME} KB_AUTO_MCP_SOURCE=codex:auto-capture:git-commit KB_AUTO_MCP_CLIENT_NAME=Codex ${CODEX_HOOKS_DIR}/pre-tool-use-git-commit-capture.sh",
            "timeout": 5,
            "statusMessage": "Capturing commit decision"
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "${CODEX_HOOKS_DIR}/kb-stop-digest.sh",
            "timeout": 60,
            "statusMessage": "Capturing KB lessons"
          }
        ]
      }
    ]
  }
}
HOOKS

if [ "${INSTALL_CODEX}" = 1 ]; then
  write_file "${CODEX_HOOKS_DIR}/pre-tool-use-edit-recall.sh" 0755 "${PRE_TOOL_USE_EDIT_HOOK}"
  write_file "${CODEX_HOOKS_DIR}/pre-tool-use-git-commit-capture.sh" 0755 "${PRE_TOOL_USE_GIT_COMMIT_HOOK}"
  write_file "${CODEX_HOOKS_DIR}/kb-stop-digest.sh" 0755 "${CODEX_STOP_DIGEST_HOOK}"
  write_file "${CODEX_SKILLS_DIR}/topics/SKILL.md" 0644 "${TOPICS_SKILL}"
  write_file "${CODEX_SKILLS_DIR}/audit/SKILL.md" 0644 "${AUDIT_SKILL}"
  write_file "${CODEX_SKILLS_DIR}/kb-first/SKILL.md" 0644 "${KB_FIRST_SKILL}"
  write_file "${CODEX_SKILLS_DIR}/token-economy/SKILL.md" 0644 "${TOKEN_ECONOMY_SKILL}"
  write_file "${CODEX_SKILLS_DIR}/agent-session-bootstrap/SKILL.md" 0644 "${AGENT_SESSION_BOOTSTRAP_SKILL}"
  write_file "${CODEX_HOOKS_CONFIG}" 0644 "${CODEX_HOOKS_JSON}"
fi

# -----------------------------------------------------------------
# Spec Kit project scaffold seed
# -----------------------------------------------------------------
# Spec Kit project scaffold seed — generated by render-agent-kit.py from repo templates.
read -r -d '' SPECIFY_SEED__specify_memory_constitution_md <<'SPECIFY_SEED__specify_memory_constitution_md_EOF' || true
# personal-stack Constitution

## Core Principles

### I. Human Authorship and No Attribution

All repository work is authored solely by the human driver. Do not add
`Co-Authored-By` trailers, generated-by footers, assistant names, model names,
or automation-attribution text to commits, PRs, code comments, docs, generated
files, or templates.

### II. Validate Against Reality

Claims about paths, APIs, config, cluster state, or tooling must be checked
against the real codebase and, where relevant, live state. If a fact is unknown,
search the repo, inspect the source, or run the narrowest safe command before
designing around it. Do not invent secret paths, resource names, commands, or
contracts.

### III. Claude/Codex Parity

Agent-facing behavior must stay equivalent across Claude and Codex surfaces.
Any skill, hook, memory rule, installer behavior, command, or project guidance
added for one agent must get the matching surface for the other in the same
branch, unless an explicit unsupported reason is recorded.

### IV. Render and Validate Discipline

Render-managed files are edited only at their source templates or inventory.
After touching a render source, run the owning renderer and commit the rendered
output with the source change. Run the smallest meaningful validation command
for the touched area, and state exactly what remains unverified if a check
cannot run.

### V. Small Stacked PRs

Every change should be reviewable, revertable, and scoped to one objective.
Prefer small stacked PRs over broad bundles. Avoid tangential cleanup,
speculative abstractions, unrelated refactors, and compatibility shims when a
direct local-pattern change is available.

## Required Workflow

1. Start from a spec for user-visible or cross-cutting changes. The spec must
   describe outcomes, acceptance criteria, non-goals, and open questions.
2. Plan against existing repo patterns and real paths. Surface architectural
   limitations before implementation begins.
3. Break work into tasks that preserve small PR boundaries and parallel safety.
4. Implement only the task scope. Never revert or overwrite unrelated parallel
   edits.
5. Validate with the smallest meaningful command for the touched area:
   `./gradlew :services:<service>:test` for Kotlin services,
   `./gradlew :platform:tooling:test` for platform tooling, and
   `npm run typecheck && npm run lint && npm run test` inside Vue UIs.
6. Capture durable lessons or decisions in the knowledge base when they affect
   future repo behavior, without storing secrets, raw transcripts, or full
   diffs.

## Render-Managed Boundaries

- `platform/inventory/fleet.yaml` is the source of truth for public service
  routing, catalog, placement, exposure, and access intent.
- Generated Traefik routes, catalog ConfigMaps, agent-kit mirrors, and installer
  artifacts must not be hand-edited.
- `.specify/memory/constitution.md` is committed and hand-edited for this repo.
  The generic `.specify/templates/constitution-template.md` is only the
  render-managed starter for future seed installs.

## Governance

This constitution overrides ad-hoc agent behavior. Amend it deliberately when
the governing workflow changes, and update `AGENTS.md`, `CLAUDE.md`, skills, or
templates in the same branch when parity requires it.

**Version**: 1.0.0
**Ratified**: 2026-06-08
**Last Amended**: 2026-06-08
SPECIFY_SEED__specify_memory_constitution_md_EOF
read -r -d '' SPECIFY_SEED__specify_scripts_bash_check_prerequisites_sh <<'SPECIFY_SEED__specify_scripts_bash_check_prerequisites_sh_EOF' || true
#!/usr/bin/env bash

set -euo pipefail

SPECIFY_SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
. "${SPECIFY_SCRIPT_DIR}/common.sh"

json=false
paths_only=false
require_tasks=false
include_tasks=false

while [ "$#" -gt 0 ]; do
  case "$1" in
    --json)
      json=true
      ;;
    --paths-only)
      paths_only=true
      ;;
    --require-tasks)
      require_tasks=true
      ;;
    --include-tasks)
      include_tasks=true
      ;;
    --help|-h)
      printf 'Usage: %s [--json] [--paths-only] [--require-tasks] [--include-tasks]\n' "$(basename "$0")"
      exit 0
      ;;
    --*)
      specify_die "Unknown option: $1"
      ;;
  esac
  shift
done

branch=$(specify_require_feature_branch)
feature_dir=$(specify_feature_dir "${branch}")
spec_file=$(specify_spec_file "${branch}")
plan_file=$(specify_plan_file "${branch}")
tasks_file=$(specify_tasks_file "${branch}")

if [ "${paths_only}" = true ]; then
  if [ "${json}" = true ]; then
    specify_paths_json "${branch}"
  else
    printf 'REPO_ROOT: %s\n' "${REPO_ROOT}"
    printf 'BRANCH: %s\n' "${branch}"
    printf 'FEATURE_DIR: %s\n' "${feature_dir}"
    printf 'FEATURE_SPEC: %s\n' "${spec_file}"
    printf 'IMPL_PLAN: %s\n' "${plan_file}"
    printf 'TASKS: %s\n' "${tasks_file}"
  fi
  exit 0
fi

[ -d "${feature_dir}" ] || specify_die "Feature directory not found: ${feature_dir}"
[ -f "${spec_file}" ] || specify_die "Feature spec not found: ${spec_file}"
[ -f "${plan_file}" ] || specify_die "Implementation plan not found: ${plan_file}"

if [ "${require_tasks}" = true ] && [ ! -f "${tasks_file}" ]; then
  specify_die "Tasks file not found: ${tasks_file}"
fi

docs=""
append_doc() {
  if [ -n "${docs}" ]; then
    docs="${docs},"
  fi
  docs="${docs}\"$1\""
}

[ -f "${feature_dir}/research.md" ] && append_doc "research.md"
[ -f "${feature_dir}/data-model.md" ] && append_doc "data-model.md"
[ -d "${feature_dir}/contracts" ] && append_doc "contracts/"
[ -f "${feature_dir}/quickstart.md" ] && append_doc "quickstart.md"
if [ "${include_tasks}" = true ] && [ -f "${tasks_file}" ]; then
  append_doc "tasks.md"
fi

if [ "${json}" = true ]; then
  printf '{"FEATURE_DIR":"%s","AVAILABLE_DOCS":[%s]}\n' "$(specify_json_escape "${feature_dir}")" "${docs}"
else
  printf 'FEATURE_DIR: %s\n' "${feature_dir}"
  printf 'AVAILABLE_DOCS:\n'
  printf '%s\n' "${docs}" | tr ',' '\n' | sed 's/^/  /; s/"//g'
fi
SPECIFY_SEED__specify_scripts_bash_check_prerequisites_sh_EOF
read -r -d '' SPECIFY_SEED__specify_scripts_bash_common_sh <<'SPECIFY_SEED__specify_scripts_bash_common_sh_EOF' || true
#!/usr/bin/env bash

set -euo pipefail

if [ -z "${SPECIFY_SCRIPT_DIR:-}" ]; then
  SPECIFY_SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
fi

specify_repo_root() {
  if command -v git >/dev/null 2>&1 && git rev-parse --show-toplevel >/dev/null 2>&1; then
    git rev-parse --show-toplevel
    return 0
  fi

  CDPATH= cd -- "${SPECIFY_SCRIPT_DIR}/../../.." && pwd -P
}

REPO_ROOT=$(specify_repo_root)
SPECIFY_DIR="${REPO_ROOT}/.specify"
SPECS_DIR="${REPO_ROOT}/specs"

specify_has_git() {
  command -v git >/dev/null 2>&1 && git -C "${REPO_ROOT}" rev-parse --is-inside-work-tree >/dev/null 2>&1
}

specify_current_branch() {
  if specify_has_git; then
    git -C "${REPO_ROOT}" rev-parse --abbrev-ref HEAD 2>/dev/null
  else
    basename "$(pwd)"
  fi
}

specify_is_feature_branch() {
  case "$1" in
    [0-9][0-9][0-9]-*) return 0 ;;
    *) return 1 ;;
  esac
}

specify_feature_dir() {
  printf '%s/specs/%s\n' "${REPO_ROOT}" "$1"
}

specify_spec_file() {
  printf '%s/specs/%s/spec.md\n' "${REPO_ROOT}" "$1"
}

specify_plan_file() {
  printf '%s/specs/%s/plan.md\n' "${REPO_ROOT}" "$1"
}

specify_tasks_file() {
  printf '%s/specs/%s/tasks.md\n' "${REPO_ROOT}" "$1"
}

specify_template_file() {
  printf '%s/templates/%s\n' "${SPECIFY_DIR}" "$1"
}

specify_json_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

specify_error() {
  printf 'ERROR: %s\n' "$*" >&2
}

specify_die() {
  specify_error "$*"
  exit 1
}

specify_require_feature_branch() {
  current_branch=$(specify_current_branch)
  if [ "${current_branch}" = "main" ] || [ "${current_branch}" = "master" ]; then
    specify_die "Current branch '${current_branch}' is not a feature branch. Run create-new-feature.sh first."
  fi
  if ! specify_is_feature_branch "${current_branch}"; then
    specify_die "Current branch '${current_branch}' must start with a three-digit feature number, e.g. 001-my-feature."
  fi
  printf '%s\n' "${current_branch}"
}

specify_copy_template() {
  template_name=$1
  destination=$2
  feature_name=$3
  source_file=$(specify_template_file "${template_name}")

  [ -f "${source_file}" ] || specify_die "Template not found: ${source_file}"

  today=$(date +%F)
  sed \
    -e "s/{{FEATURE_NAME}}/${feature_name}/g" \
    -e "s/{{feature_name}}/${feature_name}/g" \
    -e "s/{{DATE}}/${today}/g" \
    "${source_file}" > "${destination}"
}

specify_paths_json() {
  branch=$1
  feature_dir=$(specify_feature_dir "${branch}")
  spec_file=$(specify_spec_file "${branch}")
  plan_file=$(specify_plan_file "${branch}")
  tasks_file=$(specify_tasks_file "${branch}")
  has_git=false
  if specify_has_git; then
    has_git=true
  fi

  printf '{"REPO_ROOT":"%s","BRANCH":"%s","HAS_GIT":"%s","FEATURE_DIR":"%s","FEATURE_SPEC":"%s","IMPL_PLAN":"%s","TASKS":"%s"}\n' \
    "$(specify_json_escape "${REPO_ROOT}")" \
    "$(specify_json_escape "${branch}")" \
    "${has_git}" \
    "$(specify_json_escape "${feature_dir}")" \
    "$(specify_json_escape "${spec_file}")" \
    "$(specify_json_escape "${plan_file}")" \
    "$(specify_json_escape "${tasks_file}")"
}
SPECIFY_SEED__specify_scripts_bash_common_sh_EOF
read -r -d '' SPECIFY_SEED__specify_scripts_bash_create_new_feature_sh <<'SPECIFY_SEED__specify_scripts_bash_create_new_feature_sh_EOF' || true
#!/usr/bin/env bash

set -euo pipefail

SPECIFY_SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
. "${SPECIFY_SCRIPT_DIR}/common.sh"

json=false
feature_number=""
description=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --json)
      json=true
      ;;
    --number)
      shift
      [ "$#" -gt 0 ] || specify_die "--number requires a value"
      feature_number=$1
      ;;
    --help|-h)
      printf 'Usage: %s [--json] [--number N] <feature description>\n' "$(basename "$0")"
      exit 0
      ;;
    --*)
      specify_die "Unknown option: $1"
      ;;
    *)
      if [ -z "${description}" ]; then
        description=$1
      else
        description="${description} $1"
      fi
      ;;
  esac
  shift
done

[ -n "${description}" ] || specify_die "Feature description is required"

slug=$(printf '%s\n' "${description}" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9]/ /g' | awk '
{
  count = 0
  for (i = 1; i <= NF; i++) {
    word = $i
    if (word == "the" || word == "and" || word == "for" || word == "with" || word == "from" || word == "that" || word == "this" || word == "into" || word == "onto") {
      continue
    }
    if (length(word) < 2) {
      continue
    }
    words[++count] = word
    if (count == 5) {
      break
    }
  }
  if (count == 0) {
    print "feature"
  } else {
    for (i = 1; i <= count; i++) {
      printf "%s%s", (i == 1 ? "" : "-"), words[i]
    }
    printf "\n"
  }
}')

highest=0
if specify_has_git; then
  branches=$(git -C "${REPO_ROOT}" branch --all --format='%(refname:short)' 2>/dev/null || true)
  for ref in ${branches}; do
    ref=${ref#origin/}
    number=${ref%%-*}
    case "${number}" in
      [0-9][0-9][0-9])
        if [ "${number}" -gt "${highest}" ]; then
          highest=${number}
        fi
        ;;
    esac
  done
fi

if [ -d "${SPECS_DIR}" ]; then
  for path in "${SPECS_DIR}"/[0-9][0-9][0-9]-*; do
    [ -d "${path}" ] || continue
    name=$(basename "${path}")
    number=${name%%-*}
    case "${number}" in
      [0-9][0-9][0-9])
        if [ "${number}" -gt "${highest}" ]; then
          highest=${number}
        fi
        ;;
    esac
  done
fi

if [ -n "${feature_number}" ]; then
  case "${feature_number}" in
    *[!0-9]*) specify_die "--number must be numeric" ;;
  esac
  number=$(printf '%03d' "${feature_number}")
else
  number=$(printf '%03d' $((highest + 1)))
fi

branch_name="${number}-${slug}"
feature_dir=$(specify_feature_dir "${branch_name}")
spec_file=$(specify_spec_file "${branch_name}")

if specify_has_git; then
  if ! git -C "${REPO_ROOT}" rev-parse --verify --quiet "${branch_name}" >/dev/null; then
    git -C "${REPO_ROOT}" checkout -b "${branch_name}" >/dev/null 2>&1
  else
    git -C "${REPO_ROOT}" checkout "${branch_name}" >/dev/null 2>&1
  fi
fi

mkdir -p "${feature_dir}"
if [ ! -f "${spec_file}" ]; then
  specify_copy_template "spec-template.md" "${spec_file}" "${branch_name}"
fi

if [ "${json}" = true ]; then
  printf '{"BRANCH_NAME":"%s","SPEC_FILE":"%s","FEATURE_DIR":"%s","FEATURE_NUMBER":"%s"}\n' \
    "$(specify_json_escape "${branch_name}")" \
    "$(specify_json_escape "${spec_file}")" \
    "$(specify_json_escape "${feature_dir}")" \
    "$(specify_json_escape "${number}")"
else
  printf 'BRANCH_NAME: %s\n' "${branch_name}"
  printf 'SPEC_FILE: %s\n' "${spec_file}"
fi
SPECIFY_SEED__specify_scripts_bash_create_new_feature_sh_EOF
read -r -d '' SPECIFY_SEED__specify_scripts_bash_setup_plan_sh <<'SPECIFY_SEED__specify_scripts_bash_setup_plan_sh_EOF' || true
#!/usr/bin/env bash

set -euo pipefail

SPECIFY_SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
. "${SPECIFY_SCRIPT_DIR}/common.sh"

json=false
while [ "$#" -gt 0 ]; do
  case "$1" in
    --json)
      json=true
      ;;
    --help|-h)
      printf 'Usage: %s [--json]\n' "$(basename "$0")"
      exit 0
      ;;
    --*)
      specify_die "Unknown option: $1"
      ;;
  esac
  shift
done

branch=$(specify_require_feature_branch)
feature_dir=$(specify_feature_dir "${branch}")
spec_file=$(specify_spec_file "${branch}")
plan_file=$(specify_plan_file "${branch}")

mkdir -p "${feature_dir}" "${feature_dir}/contracts"
[ -f "${spec_file}" ] || specify_copy_template "spec-template.md" "${spec_file}" "${branch}"
[ -f "${plan_file}" ] || specify_copy_template "plan-template.md" "${plan_file}" "${branch}"

touch "${feature_dir}/research.md" "${feature_dir}/data-model.md" "${feature_dir}/quickstart.md"

if [ "${json}" = true ]; then
  specify_paths_json "${branch}"
else
  printf 'FEATURE_SPEC: %s\n' "${spec_file}"
  printf 'IMPL_PLAN: %s\n' "${plan_file}"
  printf 'SPECS_DIR: %s\n' "${feature_dir}"
  printf 'BRANCH: %s\n' "${branch}"
fi
SPECIFY_SEED__specify_scripts_bash_setup_plan_sh_EOF
read -r -d '' SPECIFY_SEED__specify_scripts_bash_setup_tasks_sh <<'SPECIFY_SEED__specify_scripts_bash_setup_tasks_sh_EOF' || true
#!/usr/bin/env bash

set -euo pipefail

SPECIFY_SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
. "${SPECIFY_SCRIPT_DIR}/common.sh"

json=false
while [ "$#" -gt 0 ]; do
  case "$1" in
    --json)
      json=true
      ;;
    --help|-h)
      printf 'Usage: %s [--json]\n' "$(basename "$0")"
      exit 0
      ;;
    --*)
      specify_die "Unknown option: $1"
      ;;
  esac
  shift
done

branch=$(specify_require_feature_branch)
feature_dir=$(specify_feature_dir "${branch}")
plan_file=$(specify_plan_file "${branch}")
tasks_file=$(specify_tasks_file "${branch}")

[ -d "${feature_dir}" ] || specify_die "Feature directory not found: ${feature_dir}"
[ -f "${plan_file}" ] || specify_die "Implementation plan not found: ${plan_file}"

if [ ! -f "${tasks_file}" ]; then
  specify_copy_template "tasks-template.md" "${tasks_file}" "${branch}"
fi

if [ "${json}" = true ]; then
  specify_paths_json "${branch}"
else
  printf 'TASKS: %s\n' "${tasks_file}"
  printf 'IMPL_PLAN: %s\n' "${plan_file}"
  printf 'SPECS_DIR: %s\n' "${feature_dir}"
  printf 'BRANCH: %s\n' "${branch}"
fi
SPECIFY_SEED__specify_scripts_bash_setup_tasks_sh_EOF
read -r -d '' SPECIFY_SEED__specify_templates_constitution_template_md <<'SPECIFY_SEED__specify_templates_constitution_template_md_EOF' || true
# Project Constitution

## Core Principles

### I. Outcome-First Specifications

Every feature begins with a specification that describes user-visible outcomes,
acceptance scenarios, non-goals, and success criteria before implementation
details. Ambiguity must be marked explicitly with `NEEDS CLARIFICATION`.

### II. Plan Before Implementation

Implementation work starts only after the plan identifies real project paths,
dependencies, validation commands, rollback considerations, and risks. Plans
must prefer established local patterns over new abstractions.

### III. Tests and Validation Are Mandatory

Each feature defines the smallest meaningful verification command before work
begins. Changes are not complete until those checks pass or the remaining gap is
documented with the exact reason validation could not run.

### IV. Small, Reviewable Changes

Tasks and PRs must be independently reviewable, revertable, and scoped to one
behavioral objective. Unrelated cleanup, broad refactors, and speculative
flexibility are not allowed inside feature work.

### V. Durable Context Stays Current

Specifications, plans, tasks, and durable project memory must reflect decisions
that affect future work. Do not leave important behavior only in chat logs,
temporary notes, or uncommitted local state.

## Workflow

1. `/speckit.specify` creates or updates `specs/<feature>/spec.md`.
2. `/speckit.plan` creates `plan.md` and supporting design artifacts.
3. `/speckit.tasks` creates `tasks.md` from the approved plan.
4. Implementation follows tasks in dependency order, with tests close to the
   behavior being changed.
5. Completion requires validation evidence and any relevant documentation
   updates.

## Governance

This constitution overrides informal conventions. Changes to these principles
must be reviewed deliberately, with downstream templates and instructions
updated in the same change.

**Version**: 1.0.0
**Ratified**: {{DATE}}
**Last Amended**: {{DATE}}
SPECIFY_SEED__specify_templates_constitution_template_md_EOF
read -r -d '' SPECIFY_SEED__specify_templates_plan_template_md <<'SPECIFY_SEED__specify_templates_plan_template_md_EOF' || true
# Implementation Plan: {{FEATURE_NAME}}

**Branch**: `{{FEATURE_NAME}}` | **Date**: {{DATE}} | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/{{FEATURE_NAME}}/spec.md`

## Summary

[Extract from feature spec: primary requirement + technical approach]

## Technical Context

**Language/Version**: [e.g. Kotlin 2.x, TypeScript 5.x or NEEDS CLARIFICATION]
**Primary Dependencies**: [e.g. Spring Boot, Vue, Postgres or NEEDS CLARIFICATION]
**Storage**: [if applicable, e.g. PostgreSQL, Redis, files or N/A]
**Testing**: [e.g. Gradle unit tests, Vitest, Playwright or NEEDS CLARIFICATION]
**Target Platform**: [e.g. k3s, browser, JVM service or NEEDS CLARIFICATION]
**Project Type**: [service/ui/platform/mixed]
**Performance Goals**: [domain-specific target or N/A]
**Constraints**: [domain-specific constraints or N/A]
**Scale/Scope**: [domain-specific scale or N/A]

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- [ ] No attribution is introduced in files, comments, commit text, or PR text
- [ ] Claude/Codex parity is preserved for any agent-facing behavior
- [ ] Rendered artifacts are updated by the owning renderer when source changes require it
- [ ] Small stacked PR boundary is clear and unrelated cleanup is excluded
- [ ] Verification command is identified for each touched area

## Project Structure

### Documentation

```text
specs/{{FEATURE_NAME}}/
|-- plan.md
|-- research.md
|-- data-model.md
|-- quickstart.md
|-- contracts/
`-- tasks.md
```

### Source Code

```text
# Fill with the actual paths this feature will touch.
```

**Structure Decision**: [Document the chosen source layout and real paths]

## Phase 0: Outline & Research

1. Extract unknowns from Technical Context into research tasks.
2. Capture existing repo patterns for touched paths.
3. Resolve all NEEDS CLARIFICATION items before design.

**Output**: `research.md`

## Phase 1: Design & Contracts

1. Derive entities from the feature spec and document them in `data-model.md`.
2. Produce or update API/CLI/config contracts in `contracts/`.
3. Write `quickstart.md` with validation steps for the feature.
4. Re-run Constitution Check.

**Output**: `data-model.md`, `contracts/*`, `quickstart.md`

## Phase 2: Task Planning Approach

Describe how `/speckit.tasks` should convert this plan into ordered, independently executable tasks. Do not create `tasks.md` manually during `/speckit.plan`.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
| --- | --- | --- |
| [Only if a constitution gate is intentionally violated] | [reason] | [why simpler option does not work] |

## Progress Tracking

**Phase Status**:

- [ ] Phase 0: Research complete
- [ ] Phase 1: Design complete
- [ ] Phase 2: Task planning approach complete

**Gate Status**:

- [ ] Initial Constitution Check: PASS
- [ ] Post-Design Constitution Check: PASS
- [ ] All NEEDS CLARIFICATION resolved
SPECIFY_SEED__specify_templates_plan_template_md_EOF
read -r -d '' SPECIFY_SEED__specify_templates_spec_template_md <<'SPECIFY_SEED__specify_templates_spec_template_md_EOF' || true
# Feature Specification: {{FEATURE_NAME}}

**Feature Branch**: `{{FEATURE_NAME}}`
**Created**: {{DATE}}
**Status**: Draft
**Input**: User description: "$ARGUMENTS"

## User Scenarios & Testing *(mandatory)*

<!--
  Prioritize user journeys by business value. Each journey must be independently
  testable: if only one journey ships, it should still provide useful value.
-->

### User Story 1 - [Short Title] (Priority: P1)

[Describe the user journey in plain language]

**Why this priority**: [Explain the value and why it comes first]

**Independent Test**: [Describe how to verify this story independently]

**Acceptance Scenarios**:

1. **Given** [initial state], **When** [action], **Then** [observable outcome]
2. **Given** [initial state], **When** [action], **Then** [observable outcome]

---

### User Story 2 - [Short Title] (Priority: P2)

[Describe the user journey in plain language]

**Why this priority**: [Explain the value]

**Independent Test**: [Describe how to verify this story independently]

**Acceptance Scenarios**:

1. **Given** [initial state], **When** [action], **Then** [observable outcome]

---

### User Story 3 - [Short Title] (Priority: P3)

[Describe the user journey in plain language]

**Why this priority**: [Explain the value]

**Independent Test**: [Describe how to verify this story independently]

**Acceptance Scenarios**:

1. **Given** [initial state], **When** [action], **Then** [observable outcome]

### Edge Cases

- What happens when [boundary condition]?
- How does the system handle [error condition]?

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST [specific capability]
- **FR-002**: System MUST [specific capability]
- **FR-003**: Users MUST be able to [key interaction]
- **FR-004**: System MUST [data requirement]
- **FR-005**: System MUST [observable behavior]

*Example of marking unclear requirements:*

- **FR-006**: System MUST authenticate users via [NEEDS CLARIFICATION: auth method not specified]
- **FR-007**: System MUST retain [NEEDS CLARIFICATION: retention period not specified]

### Key Entities *(include if feature involves data)*

- **[Entity 1]**: [What it represents, key attributes without implementation details]
- **[Entity 2]**: [What it represents, relationships to other entities]

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: [Metric, e.g. "Users can complete primary task in under 2 minutes"]
- **SC-002**: [Metric, e.g. "System supports 1,000 concurrent users"]
- **SC-003**: [Metric, e.g. "95% of users complete the task without support"]
- **SC-004**: [Metric, e.g. "Reduce support tickets about X by 50%"]
SPECIFY_SEED__specify_templates_spec_template_md_EOF
read -r -d '' SPECIFY_SEED__specify_templates_tasks_template_md <<'SPECIFY_SEED__specify_templates_tasks_template_md_EOF' || true
# Tasks: {{FEATURE_NAME}}

**Input**: Design documents from `/specs/{{FEATURE_NAME}}/`
**Prerequisites**: plan.md (required), research.md, data-model.md, contracts/

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel with other tasks because it touches different files
- **[Story]**: User story label, for example US1, US2, US3
- Include exact file paths in descriptions

## Phase 1: Setup

- [ ] T001 Create or verify project structure for this feature
- [ ] T002 Identify the smallest validation command for touched area

## Phase 2: Foundational

- [ ] T003 Implement shared models/configuration needed by all stories
- [ ] T004 Add or update base tests for cross-story behavior

## Phase 3: User Story 1 (Priority: P1)

**Goal**: [Brief value delivered by this story]

**Independent Test**: [How to verify only this story]

- [ ] T005 [US1] Implement [specific behavior] in [path]
- [ ] T006 [US1] Add focused tests in [path]

## Phase 4: User Story 2 (Priority: P2)

**Goal**: [Brief value delivered by this story]

**Independent Test**: [How to verify only this story]

- [ ] T007 [P] [US2] Implement [specific behavior] in [path]
- [ ] T008 [P] [US2] Add focused tests in [path]

## Phase 5: User Story 3 (Priority: P3)

**Goal**: [Brief value delivered by this story]

**Independent Test**: [How to verify only this story]

- [ ] T009 [P] [US3] Implement [specific behavior] in [path]
- [ ] T010 [P] [US3] Add focused tests in [path]

## Phase 6: Polish

- [ ] T011 Run the validation command identified in plan.md
- [ ] T012 Update docs or runbooks affected by this feature

## Dependencies

- Setup before foundational work
- Foundational work before user stories
- User stories may proceed in priority order, unless marked independent and parallel
- Polish after desired stories are complete

## Parallel Example

```text
T007 [P] [US2] ...
T009 [P] [US3] ...
```
SPECIFY_SEED__specify_templates_tasks_template_md_EOF
if [ "${SCOPE}" = "project" ]; then
  if [ ! -e "${PROJECT_ROOT}/.specify/memory/constitution.md" ]; then
    write_file "${PROJECT_ROOT}/.specify/memory/constitution.md" 0644 "${SPECIFY_SEED__specify_memory_constitution_md}"
  else
    log "preserving existing ${PROJECT_ROOT}/.specify/memory/constitution.md"
  fi
  write_file "${PROJECT_ROOT}/.specify/scripts/bash/check-prerequisites.sh" 0755 "${SPECIFY_SEED__specify_scripts_bash_check_prerequisites_sh}"
  write_file "${PROJECT_ROOT}/.specify/scripts/bash/common.sh" 0755 "${SPECIFY_SEED__specify_scripts_bash_common_sh}"
  write_file "${PROJECT_ROOT}/.specify/scripts/bash/create-new-feature.sh" 0755 "${SPECIFY_SEED__specify_scripts_bash_create_new_feature_sh}"
  write_file "${PROJECT_ROOT}/.specify/scripts/bash/setup-plan.sh" 0755 "${SPECIFY_SEED__specify_scripts_bash_setup_plan_sh}"
  write_file "${PROJECT_ROOT}/.specify/scripts/bash/setup-tasks.sh" 0755 "${SPECIFY_SEED__specify_scripts_bash_setup_tasks_sh}"
  write_file "${PROJECT_ROOT}/.specify/templates/constitution-template.md" 0644 "${SPECIFY_SEED__specify_templates_constitution_template_md}"
  write_file "${PROJECT_ROOT}/.specify/templates/plan-template.md" 0644 "${SPECIFY_SEED__specify_templates_plan_template_md}"
  write_file "${PROJECT_ROOT}/.specify/templates/spec-template.md" 0644 "${SPECIFY_SEED__specify_templates_spec_template_md}"
  write_file "${PROJECT_ROOT}/.specify/templates/tasks-template.md" 0644 "${SPECIFY_SEED__specify_templates_tasks_template_md}"
fi

# -----------------------------------------------------------------
# Manifest
# -----------------------------------------------------------------
if [ "${DRY_RUN}" != 1 ] && [ "${INSTALL_CLAUDE}" = 1 ]; then
  cat > "${MANIFEST}" <<MANIFEST
# Managed by the knowledge-system installer (${KB_URL}/install.sh).
# Re-run that command to update. Use --uninstall to remove every
# file listed below.
version=${INSTALLER_VERSION}
installed_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
scope=${SCOPE}
managed:
$(printf '  - %s\n' "${claude_managed_paths[@]}")
MANIFEST
  log "wrote ${MANIFEST}"
fi

if [ "${DRY_RUN}" != 1 ] && [ "${INSTALL_CODEX}" = 1 ]; then
  cat > "${CODEX_MANIFEST}" <<MANIFEST
# Managed by the knowledge-system installer (${KB_URL}/install.sh).
# Re-run that command to update. Use --agent codex --uninstall to remove every
# Codex file listed below.
version=${INSTALLER_VERSION}
installed_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
agent=codex
scope=${SCOPE}
managed:
$(printf '  - %s\n' "${codex_managed_paths[@]}")
MANIFEST
  log "wrote ${CODEX_MANIFEST}"
fi

cat <<EOF
knowledge-system installer complete (${INSTALLER_VERSION}, agent=${AGENT}, scope=${SCOPE}).
EOF

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  cat <<EOF

Claude next steps:

  1. Register the PreToolUse and Stop hooks in ${CLAUDE_HOME}/settings.json under the
     matching "hooks.<event>" arrays. Suggested config:

     "PreToolUse": [
       { "matcher": "Edit|Write|MultiEdit", "hooks": [
         { "type": "command",
           "command": "${HOOKS_DIR}/pre-tool-use-edit-recall.sh",
           "timeout": 5 } ] },
       { "matcher": "Bash", "hooks": [
         { "type": "command",
           "command": "${HOOKS_DIR}/pre-tool-use-git-commit-capture.sh",
           "timeout": 5 } ] } ],
     "Stop": [
       { "matcher": ".*", "hooks": [
         { "type": "command",
           "command": "${HOOKS_DIR}/stop-session-digest.sh",
           "async": true,
           "timeout": 60 } ] } ]

  2. Make sure KB_BEARER_TOKEN is set in the Claude Code environment:
       export KB_BEARER_TOKEN="<your-token>"
EOF
fi

if [ "${INSTALL_CODEX}" = 1 ]; then
  cat <<EOF

Codex next steps:

  1. ${CODEX_HOOKS_CONFIG} has been written with PreToolUse and Stop hooks.
  2. Make sure KB_BEARER_TOKEN is set in the Codex environment:
       export KB_BEARER_TOKEN="<your-token>"
EOF
fi

cat <<EOF

Verify with:  curl -sS -H "Authorization: Bearer \$KB_BEARER_TOKEN" \\
                     ${KB_URL}/mcp -d '{"jsonrpc":"2.0","id":1,"method":"ping"}'

Safety controls:
  - Panic switch:   export KB_AUTO_MCP_DISABLED=1   (turns every hook into a no-op).
EOF
if [ "${INSTALL_CLAUDE}" = 1 ]; then
  cat <<EOF
  - Claude allowlist: edit ${ALLOWLIST}  (gitignore-style patterns).
  - Claude state:     ${STATE_DIR}/auto-mcp.log + per-session dedupe under ${STATE_DIR}/sessions/.
EOF
fi
if [ "${INSTALL_CODEX}" = 1 ]; then
  cat <<EOF
  - Codex allowlist:  edit ${CODEX_ALLOWLIST}  (gitignore-style patterns).
  - Codex state:      ${CODEX_HOME}/state/auto-mcp.log + per-session dedupe under ${CODEX_HOME}/state/sessions/.
EOF
fi
cat <<EOF
  - Provenance:     every auto-capture lands with source = "<agent>:auto-capture:<hook>"
                    or "claude-code:auto-digest:<session>" so a bulk revoke is one SQL query.

Run with --agent ${AGENT} --scope ${SCOPE} --uninstall to remove every selected file this installer wrote.
EOF
