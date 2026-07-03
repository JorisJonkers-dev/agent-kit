# council/ts rearchitecture — locked design

Consolidated (Opus 4.8) from an all-codex council: 9 investigators, two
cross-critique rounds (convergence). This is a **behavior-preserving refactor**
to DDD / hexagonal / OOP with TDD. The 273 vitest tests, the CLI contract, and
the run-dir/observation contracts are the characterization safety net and stay
green after every step.

## Target structure

```
src/
  shared-kernel/            # pure stable value/wire types ONLY — no services, policies, adapters, node:*
    engine-spec.ts          # EngineRef wire interface + EngineSpec.parse (claude|codex:<model>)
    model-tier.ts           # cheap|standard|strong|max + aliases (haiku->cheap, sonnet->strong, opus->max)
    role-class.ts           # investigate | cross-critique | implement | consolidate-lock (kebab canonical)
    value-object.ts  ids.ts  json.ts  run-artifacts.ts  task-contracts.ts  verdicts.ts  index.ts

  contexts/                 # each owns domain/ [application/] [ports/] [adapters/] + index.ts barrel
    config/  routing/  planning/  execution/  runs/  engines/
    watchdog/  review/  design/  github/  context-packs/  prompts/  grill/

  workflows/                # use-cases: plan / fanout / fleet / status / review-pack
  composition/              # council-app.ts + root.ts wire adapters -> ports
  cli/                      # imports composition only
```

### Context ownership (contested points, resolved)
- **planning** owns task artifacts: schema, markdown<->json codec, amendment validator, plan artifact.
- **execution** owns the dispatch graph: `TaskGraph`, waves, ready-set policy, assignment, bounds gate, fanout/fleet application.
- **routing** owns triage + the model policy (below). Triage emits route + stage tiers + stage **role classes** only — never engine pins.

## Model policy (routing/domain/model-policy.ts)

Ordered policy chain (no giant switch), table-backed role policies. Precedence:

```
explicit-engine
  > strict-json-claude                       # strict JSON is a capability constraint, always Claude
  > program-consolidate-lock -> claude:opus  # program scale: the one protected stage
  > program-codex-override                   # program scale: all other roles -> codex
  > role-class-policy                         # default scale (below)
  > legacy-workload                          # existing single/bulk/strict-json fallback
```

Role classes → engines (default scale):
- **investigate** → codex · **cross-critique** → codex · **implement** → claude, capped at `strong`/sonnet (never opus unless explicit) · **consolidate-lock** → claude strong (opus at max).

Program scale: every role → codex **except** `consolidate-lock` → `claude:opus`; strict-json still overrides to Claude.

`RoleClass`/`ModelPrecedence` types live in shared-kernel + routing. Kebab-case
canonical serialization; underscore aliases accepted only at the parse boundary.

## Dependency rules (dependency-cruiser)

no cycles · shared-kernel imports nothing (no contexts/shell/adapters/node:*) ·
context domain imports own domain or shared-kernel only · application imports
own domain/ports not adapters · ports import own domain/shared-kernel only ·
contexts never import sibling internals · workflows/composition/cli import
contexts only through their `index.ts` · composition may wire all adapters.
Generated `dist/**` excluded from source dep-cruiser + coverage.

## Packaging

Replace the esbuild single bundle with checked-in **`tsc` multi-file `dist/`** +
a thin stable `council.mjs` launcher + a runtime `package.json` + a
`toolkit-files.json` inventory consumed by renderer/manifest/validator/
runtime-package. `dist/**` excluded from dep-cruiser/coverage; covered by build +
launcher smoke tests. Delete only `test/fixtures/gen_python_runs.py` (freeze the
fixtures with a README; repo-level render/validate Python tooling stays for now).

## Always-green migration order (locked)

1. Baseline gates; record passing state.
2. Add role-class/model-policy tests behind existing `resolveModelMatrix`.
3. Split `domain/config/index.ts` (984) in place.
4. Split `domain/context/model-matrix.ts`; enforce precedence.
5. Split `domain/tasks` (markdown codec / validator / schema).
6. Split `domain/graph` (factory / waves / ready-set / bounds gate).
7. Split `adapters/fs` (run store / atomic writer / event log / artifact codec / legacy normalizer).
8. Split `adapters/process` (supervisor / session / process-group / disk usage).
9. Split `adapters/engines` (runner / drivers / result extraction / prompt delivery).
10. Split `adapters/github` (client / labels / milestones / issues / PRs / comments).
11. Move one bounded context at a time; old paths become export-only barrels.
12. Migrate imports by consumer group (app, cli, adapters, tests).
13. Enable dep-cruiser context rules with a temporary barrel allowlist.
14. Remove old barrels only after `rg` shows no imports and full gates pass.
15. Refactor the CLI registry last (highest byte-stability risk).
16. Packaging (`dist` + launcher + inventory) as a separate gated track.

## Frozen contracts (must not change)

CLI flags/stdout/stderr/exit codes (`runCli(argv, runtime)` + parity test) ·
run-dir files `state.json`/`tasks.json`/`events.jsonl`/`workers/*/result.json`
+ legacy `report.json` · `stageTiers:"skip"` never enters model resolution ·
`plan` never runs workers or GitHub · top-level role config + CLI flags always
override derived defaults · no invalid `export final class`.
