import { describe, expect, it } from 'vitest'

import {
  lensCatalog,
  loadLensCatalog,
  parseLensCatalog,
  recommendLenses,
  type LensCatalog,
  type LensDefinition,
  type LensKind,
  type LensRisk,
  type LensSize,
} from './index.js'

interface LensFixture {
  readonly category?: string
  readonly conflictsWith?: readonly string[]
  readonly id: string
  readonly kinds?: readonly LensKind[]
  readonly pairsWith?: readonly string[]
  readonly risk?: readonly LensRisk[]
  readonly signals?: readonly string[]
  readonly sizes?: readonly LensSize[]
  readonly typicalRounds?: 1 | 2 | 3
}

function lens(fixture: LensFixture): LensDefinition {
  return {
    category: fixture.category ?? 'general',
    concerns: [`${fixture.id} concern`],
    conflictsWith: fixture.conflictsWith ?? [],
    focus: `${fixture.id} focus`,
    id: fixture.id,
    name: `${fixture.id} name`,
    pairsWith: fixture.pairsWith ?? [],
    suggestedTier: 'codex-high',
    tension: `${fixture.id} tension`,
    typicalRounds: fixture.typicalRounds ?? 1,
    whenBeneficial: {
      kinds: fixture.kinds ?? ['api'],
      risk: fixture.risk ?? ['medium'],
      signals: fixture.signals ?? [],
      sizes: fixture.sizes ?? ['medium'],
    },
  }
}

function catalog(lenses: readonly LensDefinition[]): LensCatalog {
  return { count: lenses.length, lenses, schemaVersion: 1 }
}

function rawLens(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    category: 'general',
    concerns: ['concern'],
    conflictsWith: [],
    focus: 'focus',
    id: 'base',
    name: 'Base',
    pairsWith: [],
    suggestedTier: 'codex-high',
    tension: 'tension',
    typicalRounds: 1,
    whenBeneficial: {
      kinds: ['api'],
      risk: ['medium'],
      signals: ['signal'],
      sizes: ['medium'],
    },
    ...overrides,
  }
}

function rawCatalog(lenses: readonly unknown[]): Record<string, unknown> {
  return { count: lenses.length, lenses, schemaVersion: 1 }
}

describe('lens catalog loading and parsing', () => {
  it('loads the bundled 177-lens catalog and normalizes staged omissions', () => {
    const loaded = loadLensCatalog()
    const normalizedMissingEntry = loaded.lenses.find((candidate) => candidate.id === 'privacy-data-minimization')

    expect(loaded.count).toBe(177)
    expect(loaded.lenses).toHaveLength(177)
    expect(lensCatalog.count).toBe(177)
    expect(normalizedMissingEntry?.whenBeneficial).toEqual({
      kinds: [],
      risk: [],
      signals: [],
      sizes: [],
    })
    expect(loaded.lenses.find((candidate) => candidate.id === 'authorization-surface-contracts')?.pairsWith).not.toContain(
      'consumer-contract-verification',
    )
  })

  it('normalizes incomplete loaded catalog records without relaxing parseLensCatalog', () => {
    const loaded = loadLensCatalog(
      rawCatalog([
        rawLens({
          conflictsWith: ['missing'],
          pairsWith: ['missing'],
          tension: undefined,
          typicalRounds: undefined,
          whenBeneficial: undefined,
        }),
      ]),
    )

    expect(loaded.lenses[0]).toMatchObject({
      conflictsWith: [],
      pairsWith: [],
      tension: 'No catalog tension provided.',
      typicalRounds: 1,
      whenBeneficial: {
        kinds: [],
        risk: [],
        signals: [],
        sizes: [],
      },
    })
    expect(() => loadLensCatalog({ count: 0, schemaVersion: 1 })).toThrow('lenses array')
  })

  it('keeps parseLensCatalog strict for malformed input', () => {
    expect(() => parseLensCatalog({ schemaVersion: 2 })).toThrow('schema version 1')
    expect(() => parseLensCatalog({ count: 0, schemaVersion: 1 })).toThrow('lenses array')
    expect(() => parseLensCatalog({ count: 2, lenses: [rawLens()], schemaVersion: 1 })).toThrow('count')
    expect(() => parseLensCatalog(rawCatalog([rawLens({ id: 'dup' }), rawLens({ id: 'dup' })]))).toThrow(
      'duplicate id',
    )
    expect(() => parseLensCatalog(rawCatalog([rawLens({ suggestedTier: 'unknown' })]))).toThrow('tier')
    expect(() => parseLensCatalog(rawCatalog([rawLens({ typicalRounds: 4 })]))).toThrow('typicalRounds')
    expect(() => parseLensCatalog(rawCatalog([rawLens({ pairsWith: ['missing'] })]))).toThrow('unknown lens')
    expect(() => parseLensCatalog(rawCatalog([rawLens({ conflictsWith: ['missing'] })]))).toThrow('unknown lens')
  })

  it('rejects invalid lens shapes and required arrays', () => {
    expect(() => loadLensCatalog(rawCatalog([null]))).toThrow('index 0')
    expect(() => parseLensCatalog(rawCatalog([rawLens({ id: '' })]))).toThrow('id')
    expect(() => parseLensCatalog(rawCatalog([rawLens({ concerns: [1] })]))).toThrow('only strings')
    expect(() => parseLensCatalog(rawCatalog([rawLens({ whenBeneficial: undefined })]))).toThrow('whenBeneficial')
    expect(() =>
      parseLensCatalog(rawCatalog([rawLens({ whenBeneficial: { kinds: ['api'], risk: ['medium'], sizes: ['medium'] } })])),
    ).toThrow('signals')
    expect(() =>
      parseLensCatalog(
        rawCatalog([rawLens({ whenBeneficial: { kinds: ['unknown'], risk: ['medium'], signals: [], sizes: ['medium'] } })]),
      ),
    ).toThrow('kind')
    expect(() =>
      parseLensCatalog(
        rawCatalog([rawLens({ whenBeneficial: { kinds: ['api'], risk: ['unknown'], signals: [], sizes: ['medium'] } })]),
      ),
    ).toThrow('risk')
    expect(() =>
      parseLensCatalog(
        rawCatalog([rawLens({ whenBeneficial: { kinds: ['api'], risk: ['medium'], signals: [], sizes: ['unknown'] } })]),
      ),
    ).toThrow('size')
  })
})

describe('lens recommendation scoring', () => {
  it('returns an empty recommendation when no scoring dimensions are provided', () => {
    expect(recommendLenses({}, catalog([lens({ id: 'candidate' })]))).toEqual({
      lenses: [],
      negotiation: {
        considered: [],
        dropped: [],
        tensionsBalanced: [],
      },
      rationale: ['No profile dimensions were provided; no lenses were recommended.'],
      rounds: 1,
      workerCount: 1,
    })
    expect(recommendLenses(undefined, catalog([lens({ id: 'candidate' })])).rounds).toBe(1)
    expect(recommendLenses({ kind: 'unknown' }, catalog([lens({ id: 'candidate' })])).lenses).toEqual([])
  })

  it('scores exact size, direct kind, risk, exact signals, and token-overlap signals deterministically', () => {
    const recommendation = recommendLenses(
      {
        kind: 'api',
        risk: 'high',
        signals: ['gateway timeout', 'retry webhook', 'progress callbacks', 'external system'],
        size: 'medium',
      },
      catalog([
        lens({
          id: 'best',
          risk: ['high'],
          signals: ['gateway timeout', 'retry webhook', 'progress callbacks needed', 'external system available'],
        }),
        lens({ id: 'adjacent-risk', risk: ['medium'], signals: ['gateway timeout'] }),
      ]),
    )

    expect(recommendation.lenses[0]).toMatchObject({ id: 'best', score: 118 })
    expect(recommendation.negotiation.considered.map((candidate) => candidate.id)).toEqual(['best', 'adjacent-risk'])
  })

  it('normalizes trivial size and triage kind aliases', () => {
    const recommendation = recommendLenses(
      { kind: 'hotfix', risk: 'low', size: 'trivial' },
      catalog([
        lens({ id: 'small-infra', kinds: ['infra'], risk: ['low'], sizes: ['small'] }),
        lens({ id: 'medium-api', kinds: ['api'], risk: ['low'], sizes: ['medium'] }),
      ]),
    )

    expect(recommendation.lenses[0]).toMatchObject({ id: 'small-infra', score: 66 })
  })

  it('uses landscape and high-parallelism boosts', () => {
    const recommendation = recommendLenses(
      { landscape: 'greenfield', parallelism: 'high' },
      catalog([
        lens({ id: 'architecture', kinds: ['architecture'] }),
        lens({ id: 'program-infra', kinds: ['program', 'infra'] }),
        lens({ id: 'security', kinds: ['security'] }),
      ]),
    )

    expect(recommendation.negotiation.considered.map((candidate) => [candidate.id, candidate.score])).toEqual([
      ['architecture', 4],
      ['program-infra', 4],
    ])

    const brownfield = recommendLenses(
      { landscape: 'brownfield' },
      catalog([
        lens({ id: 'migration', kinds: ['migration'] }),
        lens({ id: 'presentation', kinds: ['ui'] }),
      ]),
    )
    expect(brownfield.negotiation.considered.map((candidate) => [candidate.id, candidate.score])).toEqual([
      ['migration', 4],
    ])
  })

  it('returns one worker and one round when dimensions match no candidate', () => {
    const recommendation = recommendLenses(
      { risk: 'low' },
      catalog([lens({ id: 'critical-only', risk: ['critical'] })]),
    )

    expect(recommendation.lenses).toEqual([])
    expect(recommendation.workerCount).toBe(1)
    expect(recommendation.rounds).toBe(1)
  })

  it('expands program/high-parallel rosters and derives two-round recommendations', () => {
    const recommendation = recommendLenses({
      kind: 'infra',
      parallelism: 'high',
      risk: 'high',
      size: 'program',
    })

    expect(recommendation.workerCount).toBe(10)
    expect(recommendation.rounds).toBeGreaterThanOrEqual(2)
  })

  it('derives three rounds for program-scale critical recommendations with a large roster', () => {
    const recommendation = recommendLenses({
      kind: 'feature',
      parallelism: 'high',
      risk: 'critical',
      signals: ['critical rollout compatibility'],
      size: 'program',
    })

    expect(recommendation.workerCount).toBeGreaterThanOrEqual(8)
    expect(recommendation.rounds).toBe(3)
  })
})

describe('lens recommendation negotiation', () => {
  it('does not co-select conflicting lenses without a critical override', () => {
    const recommendation = recommendLenses(
      { kind: 'api', risk: 'high', size: 'medium' },
      catalog([
        lens({ category: 'alpha', conflictsWith: ['beta'], id: 'alpha', risk: ['high'] }),
        lens({ category: 'beta', id: 'beta', risk: ['high'] }),
      ]),
    )

    expect(recommendation.lenses.map((candidate) => candidate.id)).toEqual(['alpha'])
    expect(recommendation.negotiation.dropped).toEqual([
      {
        blockedBy: 'alpha',
        id: 'beta',
        name: 'beta name',
        reason: 'Conflicts with selected lens alpha.',
        score: 74,
      },
    ])
    expect(recommendation.negotiation.tensionsBalanced).toContainEqual({
      lensIds: ['alpha', 'beta'],
      reason: 'Dropped beta because it conflicts with alpha.',
    })
  })

  it('keeps a higher-scored conflicting lens when the critical-risk signal override applies', () => {
    const recommendation = recommendLenses(
      {
        kind: 'api',
        risk: 'critical',
        signals: ['alpha exact', 'beta exact', 'shared token one', 'shared token two'],
        size: 'medium',
      },
      catalog([
        lens({
          category: 'alpha',
          id: 'a',
          risk: ['critical'],
          signals: ['alpha exact', 'beta exact', 'shared token value', 'another shared token'],
        }),
        lens({
          category: 'alpha',
          conflictsWith: ['c'],
          id: 'b',
          risk: ['critical'],
          signals: ['alpha exact', 'beta exact', 'shared token value', 'another shared token'],
        }),
        lens({
          category: 'beta',
          id: 'c',
          pairsWith: ['a'],
          risk: ['critical'],
          signals: ['alpha exact', 'beta exact'],
        }),
      ]),
    )

    expect(recommendation.lenses.map((candidate) => candidate.id)).toEqual(['a', 'c', 'b'])
    expect(recommendation.negotiation.tensionsBalanced).toContainEqual({
      lensIds: ['c', 'b'],
      reason: 'Critical-risk override kept b despite conflict with c.',
    })
  })

  it('lets a pairsWith complement beat a close non-complement', () => {
    const recommendation = recommendLenses(
      {
        kind: 'api',
        risk: 'high',
        signals: ['first exact', 'token shared'],
        size: 'medium',
      },
      catalog([
        lens({ category: 'alpha', id: 'a', risk: ['high'], signals: ['first exact', 'token shared'] }),
        lens({ category: 'beta', id: 'b', pairsWith: ['a'], risk: ['high'], signals: ['token shared elsewhere'] }),
        lens({ category: 'gamma', id: 'c', risk: ['high'], signals: ['first exact'] }),
      ]),
    )

    expect(recommendation.lenses.map((candidate) => candidate.id)).toEqual(['a', 'b', 'c'])
    expect(recommendation.negotiation.tensionsBalanced).toContainEqual({
      lensIds: ['a', 'b'],
      reason: 'Pair boost balanced complementary lenses a and b.',
    })
  })

  it('enforces category limits so small rosters spread categories', () => {
    const recommendation = recommendLenses(
      { kind: 'api', risk: 'high', signals: ['boost'], size: 'small' },
      catalog([
        lens({ category: 'alpha', id: 'a', risk: ['high'], signals: ['boost'], sizes: ['small'] }),
        lens({ category: 'alpha', id: 'b', risk: ['high'], signals: ['boost'], sizes: ['small'] }),
        lens({ category: 'beta', id: 'c', risk: ['high'], sizes: ['small'] }),
        lens({ category: 'gamma', id: 'd', risk: ['high'], sizes: ['small'] }),
      ]),
    )

    expect(recommendation.lenses.map((candidate) => candidate.id)).toEqual(['a', 'c', 'd'])
    expect(recommendation.negotiation.dropped[0]?.id).toBe('b')
    expect(recommendation.negotiation.dropped[0]?.reason).toContain('category')
  })

  it('breaks equal scores by category and then id', () => {
    const recommendation = recommendLenses(
      { kind: 'api', size: 'medium' },
      catalog([
        lens({ category: 'beta', id: 'a' }),
        lens({ category: 'alpha', id: 'z' }),
        lens({ category: 'alpha', id: 'y' }),
      ]),
    )

    expect(recommendation.lenses[0]?.id).toBe('y')
    expect(recommendation.lenses.map((candidate) => candidate.id)).toEqual(['y', 'a', 'z'])
  })

  it('does not mutate the input catalog while recommending', () => {
    const input = catalog([
      lens({ category: 'alpha', id: 'a', pairsWith: ['b'], risk: ['high'] }),
      lens({ category: 'beta', id: 'b', risk: ['high'] }),
    ])
    const before = JSON.stringify(input)

    const recommendation = recommendLenses({ kind: 'api', risk: 'high', size: 'medium' }, input)

    expect(recommendation.rationale).toEqual([
      'Scored 2 lenses against size=medium, kind=api, risk=high.',
      'Selected 2 lenses after negotiation with roster cap 5.',
      'Worker count follows the selected lens count; rounds derive from lens depth, risk, scale, and parallelism.',
    ])
    expect(JSON.stringify(input)).toBe(before)
  })
})
