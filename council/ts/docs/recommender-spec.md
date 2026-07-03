# Lens Recommendation Engine Implementation Spec

**Repository Note**
The package currently has `src/contexts/triage`, while the locked architecture doc says the target bounded context is `routing`. Implement this feature in a new `src/contexts/routing` context and leave existing triage behavior unchanged.

**Goal**
Add a pure routing-domain service:

```ts
export function recommendLenses(
  problemProfile?: LensProblemProfile,
  catalog?: LensCatalog,
): LensRecommendation
```

It scores the lens catalog against a problem profile, negotiates a balanced roster, and returns selected lenses, worker count, rounds, rationale, and negotiation audit data.

**File Layout**
```text
src/contexts/routing/
  index.ts
  domain/
    index.ts
    lens-catalog.json
    lens-catalog.schema.json
    index.test.ts
```

Copy `./lens-catalog.merged.json` to `src/contexts/routing/domain/lens-catalog.json`. Treat it as matrix-style domain data, like `routing-matrix.json`. Do not generate TypeScript constants from it.

**Public Types**
```ts
export type LensSize = 'small' | 'medium' | 'large' | 'program'
export type LensRisk = 'low' | 'medium' | 'high' | 'critical'
export type LensTier = 'codex-medium' | 'codex-high' | 'codex-xhigh'

export type LensKind =
  | 'api' | 'architecture' | 'data' | 'feature' | 'infra' | 'library'
  | 'migration' | 'performance' | 'program' | 'refactor' | 'security'
  | 'test' | 'ui'

export interface LensProblemProfile {
  readonly size?: 'trivial' | LensSize
  readonly kind?: string
  readonly risk?: LensRisk
  readonly landscape?: 'brownfield' | 'greenfield'
  readonly clarity?: 'clear' | 'needs-questions' | 'unclear'
  readonly parallelism?: 'none' | 'some' | 'high'
  readonly signals?: readonly string[]
}

export interface LensDefinition {
  readonly id: string
  readonly name: string
  readonly category: string
  readonly focus: string
  readonly concerns: readonly string[]
  readonly whenBeneficial: {
    readonly signals: readonly string[]
    readonly kinds: readonly LensKind[]
    readonly sizes: readonly LensSize[]
    readonly risk: readonly LensRisk[]
  }
  readonly tension: string
  readonly pairsWith: readonly string[]
  readonly conflictsWith: readonly string[]
  readonly suggestedTier: LensTier
  readonly typicalRounds: 1 | 2 | 3
}

export interface LensCatalog {
  readonly schemaVersion: 1
  readonly source: string
  readonly count: number
  readonly lenses: readonly LensDefinition[]
}

export interface LensRef {
  readonly id: string
  readonly name: string
  readonly category: string
  readonly focus: string
  readonly suggestedTier: LensTier
  readonly score: number
}

export interface LensRecommendation {
  readonly lenses: readonly LensRef[]
  readonly workerCount: number
  readonly rounds: number
  readonly rationale: readonly string[]
  readonly negotiation: {
    readonly considered: readonly LensRef[]
    readonly 'dropped-with-reasons': readonly {
      readonly id: string
      readonly name: string
      readonly score: number
      readonly reason: string
      readonly blockedBy?: string
    }[]
    readonly 'tensions-balanced': readonly {
      readonly lensIds: readonly string[]
      readonly reason: string
    }[]
  }
}
```

**Domain API**
```ts
export const lensCatalog: LensCatalog

export function loadLensCatalog(value?: unknown): LensCatalog
export function parseLensCatalog(value: unknown): LensCatalog
export function recommendLenses(
  problemProfile?: LensProblemProfile,
  catalog?: LensCatalog,
): LensRecommendation
```

`parseLensCatalog` validates schema version, `count === lenses.length`, required arrays, unique IDs, valid `suggestedTier`, valid `typicalRounds`, and that every `pairsWith` / `conflictsWith` reference exists.

**Scoring**
Normalize `trivial -> small`. Normalize triage kinds into catalog kinds:

```ts
ui-tweak -> ui, feature
bugfix -> feature, test, infra
hotfix -> infra, feature, test
cross-cutting -> infra, refactor, architecture
maintenance -> refactor, infra
design-system -> ui, library
prototype -> feature, ui, api
```

If the incoming kind is already a `LensKind`, include it directly.

Use deterministic additive scoring:

```text
size exact match: +30
kind exact direct match: +24
kind alias match: +16
risk exact match: +20
risk adjacent ordinal match: +8
each signal exact phrase match: +14, capped at +28
each signal token overlap match: +8, capped at +16
brownfield + migration/refactor lens: +4
greenfield + architecture/feature/api lens: +4
parallelism high + program/infra lens: +4
```

Token matching lowercases, splits on non-alphanumeric boundaries, drops words shorter than 3 and common stopwords. Sort scored candidates by score descending, then signal match count descending, dimension match count descending, category ascending, id ascending.

Empty profile returns no lenses, `workerCount: 1`, `rounds: 1`, and rationale explaining that no profile dimensions were provided.

**Negotiation**
Roster caps:

```text
trivial/small: 3
medium: 5
large: 7
program: 9
critical risk: +1
high parallelism: +1
absolute max: 10
```

Greedy selection:

```text
while roster is below cap:
  recompute negotiation score for remaining candidates:
    base score
    +8 if pairsWith an already selected lens in either direction
    +4 if category not yet represented
    -6 per existing selected lens in same category

  reject if category limit exceeded:
    cap <= 3: max 1 per category
    cap > 3: max 2 per category

  reject conflictsWith unless override applies:
    override only when risk is critical,
    candidate has a signal match,
    and candidate score is at least 15 higher than the conflicting selected lens

  select highest negotiated score using the same deterministic tie-breaker
```

Record pair boosts, category balancing, conflict drops, and conflict overrides in `negotiation`.

**Worker Count And Rounds**
`workerCount` equals selected lens count, except empty roster returns `1`.

Rounds:

```text
empty roster: 1
max selected typicalRounds
at least 2 when risk is critical, size is program, parallelism is high, or roster size >= 6
at least 3 when size is program, risk is critical, and roster size >= 8
cap at 3
```

**Exports**
`src/contexts/routing/domain/index.ts` exports all domain functions and types.  
`src/contexts/routing/index.ts` re-exports `./domain/index.js`.

No domain import may use Node, adapters, app, CLI, or sibling context internals.

**App And CLI**
Add:

```ts
export interface RecommendInput {
  readonly profile?: LensProblemProfile
}

CouncilApp.recommend(input: RecommendInput = {}): Promise<LensRecommendation>
```

Add CLI command:

```text
council recommend --input '<json profile>'
```

It outputs the full `LensRecommendation` JSON. Do not change existing `triage`, `plan`, or `design` output. This keeps existing command behavior byte-stable.

**TDD Test Plan**
Add `src/contexts/routing/domain/index.test.ts` first.

Cover:

- Default catalog loads 179 lenses.
- Parser rejects wrong schema version, missing lenses, count mismatch, duplicate IDs, invalid tier, invalid rounds, unknown pair/conflict reference.
- Empty profile returns empty roster, worker count 1, rounds 1.
- Exact size/kind/risk/signal scoring selects expected high-scoring lenses.
- `trivial` maps to `small`.
- Triage kind aliases map deterministically.
- Program + high parallelism yields a larger roster and at least 2 rounds.
- Program + critical + large roster yields 3 rounds.
- Conflicting lenses are not co-selected without override.
- Critical-risk signal override can keep a conflicting higher-scored lens and records the reason.
- `pairsWith` complement beats an otherwise close non-complement.
- Category limits spread the roster.
- Equal-score tie breaks by category then id.
- Result and input catalog are not mutated.

CLI/app tests:

- `commandRegistry()` includes `recommend`.
- `runCli(['recommend', '--input', json])` calls `app.recommend({ profile })`.
- Missing `--input` fails with exit code 2.
- Invalid JSON fails with exit code 2.
- Existing `triage` tests remain unchanged.
- `CouncilApp.recommend()` does not touch filesystem or GitHub adapters.

Run gates:

```text
npm test
npm run typecheck
npm run lint
npm run depcruise
npm run build
```