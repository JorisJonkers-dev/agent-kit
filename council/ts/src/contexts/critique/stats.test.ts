import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  ASSETS_ROOT,
  fenced,
  finding,
  harness,
  isolatedExport,
  SyntheticLedger,
  twoCriticRouting,
  vibeStdout,
  type Harness,
  type SyntheticCritic,
  type SyntheticFinding,
} from '../../../test/critique/harness.js'
import type { StatsCell, StatsResult } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const MISTRAL = 'vibe-mistral-large'
const GLM = 'vibe-glm'
const AGY = 'agy-gemini-flash-high'
const AGENT_KIT = 'github.com/jorisjonkers-dev/agent-kit'

/** `count` findings of each verdict, raised by `clients` together. */
function raised(
  clients: string | readonly string[],
  counts: Partial<Record<SyntheticFinding['verdict'], number>>,
): SyntheticFinding[] {
  const raisedBy = typeof clients === 'string' ? [clients] : clients
  return Object.entries(counts).flatMap(([verdict, count]) =>
    Array.from({ length: count }, () => ({
      raised_by: raisedBy,
      verdict: verdict as SyntheticFinding['verdict'],
    })),
  )
}

interface RunsOptions {
  readonly critics?: readonly SyntheticCritic[]
  readonly stage?: string
  readonly spec?: string | null
  readonly repository?: string
  /** The UTC day every run is recorded on, `YYYY-MM-DD`. */
  readonly day?: string
}

/** `count` runs of one stage, every run with the same critics, the findings dealt over the runs in turn. */
function runs(
  ledger: SyntheticLedger,
  count: number,
  findings: readonly SyntheticFinding[],
  options: RunsOptions = {},
): SyntheticLedger {
  for (let i = 0; i < count; i += 1) {
    ledger.run({
      critics: options.critics ?? [{ client: MISTRAL }, { client: GLM }],
      findings: findings.filter((_, index) => index % count === i),
      ...(options.stage === undefined ? {} : { stage: options.stage }),
      ...(options.spec === undefined ? {} : { spec: options.spec }),
      ...(options.repository === undefined ? {} : { repository: options.repository }),
      ...(options.day === undefined
        ? {}
        : { recorded_at: `${options.day}T10:${String(i).padStart(2, '0')}:00.000Z` }),
    })
  }
  return ledger
}

async function shippedRouting(): Promise<Record<string, unknown>> {
  return JSON.parse(await readFile(join(ASSETS_ROOT, 'routing.json'), 'utf8')) as Record<
    string,
    unknown
  >
}

async function opened(options: Parameters<typeof harness>[0] = {}): Promise<Harness> {
  const h = await harness(options)
  open.push(h)
  return h
}

async function stats(
  ledger: SyntheticLedger,
  input: Record<string, unknown> | ((h: Harness) => Record<string, unknown>) = {},
  options: Parameters<typeof harness>[0] = {},
): Promise<StatsResult> {
  const h = await opened(options)
  await ledger.writeTo(h)
  const result = await h.commands.stats.run(typeof input === 'function' ? input(h) : input)
  if (result.status !== 'reported') throw new Error(`not reported: ${JSON.stringify(result)}`)
  return result
}

function cells(result: StatsResult, client: string, stage = 'spec'): readonly StatsCell[] {
  return result.cells.filter((c) => c.client === client && c.stage === stage)
}

function cell(result: StatsResult, client: string, stage = 'spec'): StatsCell {
  const found = cells(result, client, stage)
  const [only] = found
  if (found.length !== 1 || only === undefined)
    throw new Error(`${String(found.length)} cells for ${client} on ${stage}`)
  return only
}

function fabricated(client: string, count: number): SyntheticFinding[] {
  return Array.from({ length: count }, () => ({
    pre_verdict: 'fabricated-evidence',
    raised_by: [client],
    verdict: 'rejected' as const,
  }))
}

describe('critique stats figures', () => {
  it('reports precision per critic client and stage as accepted over accepted plus rejected, leaving hints out', async () => {
    const ledger = runs(
      new SyntheticLedger(),
      20,
      raised(MISTRAL, { accepted: 15, hint: 6, 'needs-human': 2, rejected: 5 }),
    )

    const result = await stats(ledger)

    expect(cell(result, MISTRAL)).toMatchObject({
      accepted: 15,
      adjudicated: 20,
      findings: 28,
      hints: 6,
      needs_human: 2,
      precision: 0.75,
      rejected: 5,
    })
  })

  it('reports no precision for a cell with nothing adjudicated', async () => {
    const result = await stats(runs(new SyntheticLedger(), 20, raised(GLM, { hint: 20 })))

    expect(cell(result, GLM)).toMatchObject({ adjudicated: 0, precision: null })
  })

  it('counts as unique catches only the accepted findings no other critic client raised', async () => {
    const ledger = runs(new SyntheticLedger(), 20, [
      ...raised(MISTRAL, { accepted: 10, rejected: 4 }),
      ...raised([MISTRAL, GLM], { accepted: 6 }),
      ...raised(GLM, { accepted: 4, rejected: 10 }),
    ])

    const result = await stats(ledger)

    expect(cell(result, MISTRAL)).toMatchObject({ accepted: 16, unique_catches: 10 })
    expect(cell(result, GLM)).toMatchObject({ accepted: 10, unique_catches: 4 })
  })

  it('reports the fabrication rate as findings rejected for fabricated evidence over every finding raised', async () => {
    const ledger = runs(new SyntheticLedger(), 20, [
      ...raised(MISTRAL, { accepted: 15 }),
      ...fabricated(MISTRAL, 5),
      ...Array.from({ length: 5 }, () => ({
        pre_verdict: 'no-evidence',
        raised_by: [MISTRAL],
        verdict: 'hint' as const,
      })),
    ])

    const result = await stats(ledger)

    expect(cell(result, MISTRAL)).toMatchObject({
      fabricated: 5,
      fabrication_rate: 0.2,
      findings: 25,
      rejected: 5,
    })
  })

  it('reports no fabrication rate for a cell that raised nothing', async () => {
    const result = await stats(runs(new SyntheticLedger(), 20, raised(MISTRAL, { accepted: 20 })))

    expect(cell(result, GLM)).toMatchObject({ fabrication_rate: null, findings: 0, runs: 20 })
  })

  it('reports the cost per accepted finding from the cost of every run in the cell', async () => {
    const ledger = runs(new SyntheticLedger(), 20, raised(MISTRAL, { accepted: 12, rejected: 8 }), {
      critics: [{ client: MISTRAL, cost_usd: 0.03 }, { client: GLM }],
    })

    const mistral = cell(await stats(ledger), MISTRAL)

    expect(mistral.cost_usd).toBeCloseTo(0.6, 10)
    expect(mistral.cost_per_accepted_usd).toBeCloseTo(0.05, 10)
    expect(mistral.unpriced_runs).toBe(0)
  })

  it('reports no cost per accepted finding for a client whose runs carry no cost', async () => {
    const ledger = runs(new SyntheticLedger(), 20, raised(AGY, { accepted: 20 }), {
      critics: [{ client: MISTRAL }, { client: AGY, cost_usd: null }],
    })

    expect(cell(await stats(ledger), AGY)).toMatchObject({
      cost_per_accepted_usd: null,
      cost_usd: 0,
      unpriced_runs: 20,
    })
  })

  it('reports no cost per accepted finding for a cell with no accepted finding', async () => {
    const result = await stats(runs(new SyntheticLedger(), 20, raised(GLM, { rejected: 20 })))

    expect(cell(result, GLM)).toMatchObject({ accepted: 0, cost_per_accepted_usd: null })
  })

  it('reports the failure rate over every run, and the timeout rate over the runs a time limit stopped', async () => {
    const ledger = new SyntheticLedger()
    const failed: SyntheticCritic[] = [
      { client: MISTRAL, failure: 'stopped-early', status: 'failed' },
      { client: MISTRAL, failure: 'exit-124', status: 'failed' },
      { client: MISTRAL, failure: 'print-timeout', status: 'failed' },
      { client: MISTRAL, failure: 'unparseable-output', status: 'failed' },
      { client: MISTRAL, status: 'violated-readonly' },
    ]
    for (const critic of failed) ledger.run({ critics: [critic, { client: GLM }] })
    runs(ledger, 15, raised(MISTRAL, { accepted: 20 }))

    expect(cell(await stats(ledger), MISTRAL)).toMatchObject({
      failed_runs: 5,
      failure_rate: 0.25,
      runs: 20,
      timed_out_runs: 3,
      timeout_rate: 0.15,
    })
  })
})

describe('wasted spend', () => {
  /** Runs a price or time limit stopped before they found anything, beside runs that are not waste. */
  function stoppedRuns(): SyntheticLedger {
    const ledger = new SyntheticLedger()
    const critics: SyntheticCritic[] = [
      { client: MISTRAL, cost_usd: 0.2, failure: 'stopped-early', status: 'failed' },
      { client: MISTRAL, cost_usd: 0.3, failure: 'stopped-early', status: 'failed' },
      { client: MISTRAL, cost_usd: null, failure: 'timeout', status: 'failed' },
      { client: MISTRAL, cost_usd: 0.1, failure: 'unparseable-output', status: 'failed' },
      { client: MISTRAL, cost_usd: 0.4 },
    ]
    for (const critic of critics) ledger.run({ critics: [critic, { client: GLM }] })
    return ledger
  }

  it('counts the spend on runs a price or time limit stopped with no findings', async () => {
    expect(cell(await stats(stoppedRuns()), MISTRAL)).toMatchObject({ stopped_runs: 3, wasted_usd: 0.5 })
  })

  it('names a single wasted run in the singular', async () => {
    const ledger = new SyntheticLedger().run({ critics: [{ client: MISTRAL, cost_usd: 0.2, failure: 'stopped-early', status: 'failed' }] })

    expect((await stats(ledger)).report).toContain('Wasted: $0.2000 on 1 run a price or time limit stopped before it found anything.\n')
  })

  it('names no wasted spend when the ledger has no runs', async () => {
    const result = await stats(new SyntheticLedger())
    expect(result.report).not.toContain('Wasted:')
  })

  it('counts no waste for a client whose runs all finished', async () => {
    expect(cell(await stats(stoppedRuns()), GLM)).toMatchObject({ stopped_runs: 0, wasted_usd: 0 })
  })

  it('shows the wasted spend in the report, in total and per row', async () => {
    const result = await stats(stoppedRuns())

    expect(result.report).toContain('Wasted: $0.5000 on 3 runs a price or time limit stopped before they found anything.\n')
    expect(result.report).toContain(`| vibe-mistral-large | ${cell(result, MISTRAL).prompt_hash} | 5 | 0 (too few) | - | 0 | - | - | 80% | 40% | $0.5000 (3) |`)
    expect(result.report).toContain(`| vibe-glm | ${cell(result, GLM).prompt_hash} | 5 | 0 (too few) | - | 0 | - | - | 0% | 0% | - |`)
  })
})

describe('critique stats cells', () => {
  it('splits a client into one cell per prompt when its role changed', async () => {
    const ledger = new SyntheticLedger()
    runs(ledger, 20, raised(MISTRAL, { accepted: 10, rejected: 10 }), {
      critics: [{ client: MISTRAL, role_hash: 'role-1' }, { client: GLM }],
    })
    runs(ledger, 20, raised(MISTRAL, { accepted: 18, rejected: 2 }), {
      critics: [{ client: MISTRAL, role_hash: 'role-2' }, { client: GLM }],
    })

    const split = cells(await stats(ledger), MISTRAL)

    expect(split.map((c) => [c.runs, c.precision])).toEqual([
      [20, 0.5],
      [20, 0.9],
    ])
    expect(new Set(split.map((c) => c.prompt_hash)).size).toBe(2)
  })

  it('splits a client into one cell per prompt when a lens changed', async () => {
    const ledger = new SyntheticLedger()
    runs(ledger, 20, raised(GLM, { accepted: 20 }), {
      critics: [{ client: MISTRAL }, { client: GLM, lens_hash: 'lens-1' }],
    })
    runs(ledger, 20, raised(GLM, { rejected: 20 }), {
      critics: [{ client: MISTRAL }, { client: GLM, lens_hash: 'lens-2' }],
    })

    expect(cells(await stats(ledger), GLM).map((c) => c.precision)).toEqual([1, 0])
  })

  it('keeps the same prompt in one cell across runs', async () => {
    const result = await stats(runs(new SyntheticLedger(), 40, raised(MISTRAL, { accepted: 40 })))

    expect(cells(result, MISTRAL)).toHaveLength(1)
    expect(cell(result, MISTRAL).prompt_hash).toMatch(/^[0-9a-f]{12}$/)
  })

  it('keeps each stage in cells of its own', async () => {
    const ledger = new SyntheticLedger()
    runs(ledger, 20, raised(MISTRAL, { accepted: 20 }), { stage: 'spec' })
    runs(ledger, 20, raised(MISTRAL, { rejected: 20 }), { stage: 'tickets' })

    const result = await stats(ledger)

    expect([
      cell(result, MISTRAL, 'spec').precision,
      cell(result, MISTRAL, 'tickets').precision,
    ]).toEqual([1, 0])
  })
})

describe('critique stats filters', () => {
  function twoRepositories(): SyntheticLedger {
    const ledger = new SyntheticLedger()
    runs(ledger, 20, raised(MISTRAL, { accepted: 20 }), { repository: AGENT_KIT })
    return runs(ledger, 20, raised(MISTRAL, { rejected: 20 }), {
      repository: 'github.com/example/widgets',
    })
  }

  it('limits the figures to one repository, named by its allowlist key in any case', async () => {
    const result = await stats(twoRepositories(), {
      repository: 'github.com/JorisJonkers-dev/agent-kit',
    })

    expect(cell(result, MISTRAL)).toMatchObject({ accepted: 20, rejected: 0, runs: 20 })
  })

  it('limits the figures to the repository of a working directory', async () => {
    const result = await stats(twoRepositories(), (h) => ({ repo: h.repo }))

    expect(result.filter).toEqual({ repository: AGENT_KIT })
    expect(cell(result, MISTRAL)).toMatchObject({ accepted: 20, runs: 20 })
  })

  it('refuses a working directory whose repository has no remote', async () => {
    const h = await opened({ remotes: [] })

    expect(await h.commands.stats.run({ repo: h.repo })).toMatchObject({
      reason: 'no-remote',
      status: 'refused',
    })
  })

  function threeDays(): SyntheticLedger {
    const ledger = new SyntheticLedger()
    runs(ledger, 20, raised(MISTRAL, { accepted: 20 }), { day: '2026-10-01' })
    runs(ledger, 20, raised(MISTRAL, { rejected: 20 }), { day: '2026-10-02' })
    return runs(ledger, 20, raised(MISTRAL, { hint: 20 }), { day: '2026-10-03' })
  }

  it('limits the figures to a date range that includes both of its days', async () => {
    const result = await stats(threeDays(), { since: '2026-10-02', until: '2026-10-02' })

    expect(cell(result, MISTRAL)).toMatchObject({ accepted: 0, hints: 0, rejected: 20, runs: 20 })
  })

  it('limits the figures to the days from a start date on', async () => {
    const result = await stats(threeDays(), { since: '2026-10-02' })

    expect(cell(result, MISTRAL)).toMatchObject({ accepted: 0, hints: 20, rejected: 20, runs: 40 })
  })

  it.each([
    [{ since: '2026-02-30' }],
    [{ until: '1 October' }],
    [{ since: '2026-10-03', until: '2026-10-01' }],
    [{ repo: '/x', repository: AGENT_KIT }],
  ])('rejects the input %j', async (input) => {
    const h = await opened()

    await expect(h.commands.stats.run(input)).rejects.toMatchObject({ code: 'invalid-input' })
  })
})

describe('critique stats suggestions', () => {
  it('suggests a swap when a client off the roster has the higher precision over enough findings', async () => {
    const ledger = runs(new SyntheticLedger(), 20, [
      ...raised(MISTRAL, { accepted: 11, rejected: 9 }),
      ...raised(GLM, { accepted: 16, hint: 3, rejected: 4 }),
    ])

    expect((await stats(ledger)).suggestions).toEqual([
      'vibe-glm has higher precision than vibe-mistral-large on the spec stage over 40 findings (0.80 against 0.55); consider swapping.',
    ])
  })

  it('makes no suggestion for a cell with fewer adjudicated findings than the threshold', async () => {
    const ledger = runs(new SyntheticLedger(), 20, [
      ...raised(MISTRAL, { accepted: 11, rejected: 9 }),
      ...raised(GLM, { accepted: 16, hint: 5, rejected: 3 }),
    ])

    const result = await stats(ledger)

    expect(result.suggestions).toEqual([])
    expect(cell(result, GLM)).toMatchObject({ adjudicated: 19, below_threshold: true })
    expect(result.thresholds).toEqual({ min_adjudicated_findings: 20, removal_features: 3 })
  })

  it('reads the threshold from routing', async () => {
    const routing = await shippedRouting()
    const ledger = runs(new SyntheticLedger(), 20, [
      ...raised(MISTRAL, { accepted: 11, rejected: 9 }),
      ...raised(GLM, { accepted: 16, hint: 5, rejected: 3 }),
    ])

    const result = await stats(
      ledger,
      {},
      { routing: { ...routing, stats: { min_adjudicated_findings: 19, removal_features: 3 } } },
    )

    expect(result.suggestions).toEqual([
      'vibe-glm has higher precision than vibe-mistral-large on the spec stage over 39 findings (0.84 against 0.55); consider swapping.',
    ])
  })

  it("reads the threshold from this machine's routing override", async () => {
    const ledger = runs(new SyntheticLedger(), 20, [
      ...raised(MISTRAL, { accepted: 11, rejected: 9 }),
      ...raised(GLM, { accepted: 16, hint: 5, rejected: 3 }),
    ])

    const result = await stats(ledger, {}, { routingOverride: { stats: { min_adjudicated_findings: 19 } } })

    expect(result.thresholds).toEqual({ min_adjudicated_findings: 19, removal_features: 3 })
    expect(result.suggestions).toHaveLength(1)
  })

  it('makes no suggestion when the routed client has the higher precision', async () => {
    const ledger = runs(new SyntheticLedger(), 20, [
      ...raised(MISTRAL, { accepted: 16, rejected: 4 }),
      ...raised(GLM, { accepted: 11, rejected: 9 }),
    ])

    expect((await stats(ledger)).suggestions).toEqual([])
  })

  it('names the client off the roster with the highest precision', async () => {
    const small = 'vibe-mistral-small'
    const ledger = runs(
      new SyntheticLedger(),
      20,
      [
        ...raised(MISTRAL, { accepted: 11, rejected: 9 }),
        ...raised(small, { accepted: 18, rejected: 2 }),
        ...raised(GLM, { accepted: 16, rejected: 4 }),
      ],
      { critics: [{ client: MISTRAL }, { client: small }, { client: GLM }] },
    )

    expect((await stats(ledger)).suggestions).toEqual([
      'vibe-mistral-small has higher precision than vibe-mistral-large on the spec stage over 40 findings (0.90 against 0.55); consider swapping.',
    ])
  })

  it('makes no suggestion to swap two clients the stage already routes', async () => {
    const ledger = runs(
      new SyntheticLedger(),
      20,
      [
        ...raised(MISTRAL, { accepted: 11, rejected: 9 }),
        ...raised(AGY, { accepted: 16, rejected: 4 }),
      ],
      { critics: [{ client: MISTRAL }, { client: AGY }] },
    )

    expect((await stats(ledger)).suggestions).toEqual([])
  })

  it('judges a client by its current prompt, not by a prompt it no longer runs', async () => {
    const ledger = new SyntheticLedger()
    runs(
      ledger,
      20,
      [
        ...raised(MISTRAL, { accepted: 10, rejected: 10 }),
        ...raised(GLM, { accepted: 16, rejected: 4 }),
      ],
      {
        critics: [{ client: MISTRAL, role_hash: 'role-1' }, { client: GLM }],
      },
    )
    runs(
      ledger,
      20,
      [
        ...raised(MISTRAL, { accepted: 18, rejected: 2 }),
        ...raised(GLM, { accepted: 16, rejected: 4 }),
      ],
      {
        critics: [{ client: MISTRAL, role_hash: 'role-2' }, { client: GLM }],
      },
    )

    expect((await stats(ledger)).suggestions).toEqual([])
  })

  it('makes no suggestion for a stage routing does not route', async () => {
    const ledger = runs(
      new SyntheticLedger(),
      20,
      [
        ...raised(MISTRAL, { accepted: 11, rejected: 9 }),
        ...raised(GLM, { accepted: 16, rejected: 4 }),
      ],
      { stage: 'tickets' },
    )

    expect((await stats(ledger)).suggestions).toEqual([])
  })

  it('refuses a routing without stats thresholds', async () => {
    const h = await opened({ routing: twoCriticRouting() })

    await expect(h.commands.stats.run({})).rejects.toMatchObject({ code: 'invalid-routing' })
  })

  it('refuses a threshold that is not a whole number of at least one', async () => {
    const h = await opened({
      routing: {
        ...(await shippedRouting()),
        stats: { min_adjudicated_findings: 0, removal_features: 3 },
      },
    })

    await expect(h.commands.stats.run({})).rejects.toMatchObject({ code: 'invalid-routing' })
  })
})

describe('critique stats removal candidates', () => {
  /** One feature's spec-stage runs: every accepted finding raised by both clients, so none is unique. */
  function feature(ledger: SyntheticLedger, spec: string, uniqueAccepted = 0): SyntheticLedger {
    return runs(
      ledger,
      7,
      [
        ...raised([MISTRAL, GLM], { accepted: 7 }),
        ...raised(MISTRAL, { accepted: uniqueAccepted, rejected: 7 }),
      ],
      { spec },
    )
  }

  it('flags a stage with no unique accepted catch across its last three features', async () => {
    const ledger = new SyntheticLedger()
    for (const spec of ['#1', '#2', '#3']) feature(ledger, spec)

    const result = await stats(ledger)

    expect(result.removal_candidates).toEqual([{ features: ['#3', '#2', '#1'], stage: 'spec' }])
    expect(cell(result, MISTRAL).runs).toBe(21)
  })

  it('does not flag a stage that made a unique accepted catch in one of its last three features', async () => {
    const ledger = new SyntheticLedger()
    feature(ledger, '#1')
    feature(ledger, '#2', 1)
    feature(ledger, '#3')

    expect((await stats(ledger)).removal_candidates).toEqual([])
  })

  it('looks only at the last three features', async () => {
    const ledger = new SyntheticLedger()
    feature(ledger, '#1', 5)
    for (const spec of ['#2', '#3', '#4']) feature(ledger, spec)

    expect((await stats(ledger)).removal_candidates).toEqual([
      { features: ['#4', '#3', '#2'], stage: 'spec' },
    ])
  })

  it('does not judge a stage that has run on fewer than three features', async () => {
    const ledger = new SyntheticLedger()
    feature(ledger, '#1')
    feature(ledger, '#2')

    expect((await stats(ledger)).removal_candidates).toEqual([])
  })

  it('does not count ledger lines that name no spec as a feature', async () => {
    const ledger = new SyntheticLedger()
    feature(ledger, '#1')
    feature(ledger, '#2')
    runs(ledger, 20, raised([MISTRAL, GLM], { accepted: 20 }), { spec: null })

    expect((await stats(ledger)).removal_candidates).toEqual([])
  })

  it('judges a review stage by its latest runs, since a review belongs to no feature', async () => {
    const ledger = new SyntheticLedger()
    for (let n = 0; n < 3; n += 1) {
      ledger.run({
        critics: [{ client: MISTRAL }, { client: GLM }],
        findings: raised([MISTRAL, GLM], { accepted: 2 }),
        spec: 'adrs',
        stage: 'adr-review',
      })
    }

    const result = await stats(ledger)

    expect(result.removal_candidates).toEqual([{ runs: ['run-3', 'run-2', 'run-1'], stage: 'adr-review' }])
    expect(result.report).toContain('- adr-review: no unique accepted catch across its last 3 runs (run-3, run-2, run-1)\n')
  })

  it('keeps a review stage whose unique accepted catch was in one of its latest runs', async () => {
    const ledger = new SyntheticLedger()
    for (const unique of [0, 1, 0]) {
      ledger.run({
        critics: [{ client: MISTRAL }, { client: GLM }],
        findings: raised(MISTRAL, { accepted: unique, rejected: 1 }),
        spec: 'adrs',
        stage: 'adr-review',
      })
    }

    expect((await stats(ledger)).removal_candidates).toEqual([])
  })

  it('reads the number of features from routing', async () => {
    const ledger = new SyntheticLedger()
    feature(ledger, '#1')
    feature(ledger, '#2')

    const result = await stats(
      ledger,
      {},
      {
        routing: {
          ...(await shippedRouting()),
          stats: { min_adjudicated_findings: 20, removal_features: 2 },
        },
      },
    )

    expect(result.removal_candidates).toEqual([{ features: ['#2', '#1'], stage: 'spec' }])
  })
})

describe('critique stats ledger', () => {
  it('reports nothing from a missing ledger', async () => {
    const h = await opened()

    const result = await h.commands.stats.run({})

    expect(result).toMatchObject({
      cells: [],
      removal_candidates: [],
      status: 'reported',
      suggestions: [],
    })
    if (result.status !== 'reported') throw new Error('not reported')
    expect(result.report).toContain('No critic runs in the ledger match.')
  })

  it('skips blank lines and lines of a type it does not know', async () => {
    const ledger = runs(new SyntheticLedger(), 20, raised(MISTRAL, { accepted: 20 }))
    ledger.raw({ type: 'spend', vibe_usd: 1 })
    const h = await opened()
    await h.writeText(
      join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl'),
      `\n${ledger.text()}\n\n`,
    )

    const result = await h.commands.stats.run({})

    expect(result).toMatchObject({ status: 'reported' })
    if (result.status !== 'reported') throw new Error('not reported')
    expect(cell(result, MISTRAL).accepted).toBe(20)
  })

  it("skips a low-risk triage's run line, which names no critic", async () => {
    const ledger = runs(new SyntheticLedger(), 20, raised(MISTRAL, { accepted: 20 })).raw({
      answer: 'no',
      changed_files: 1,
      changed_lines: 2,
      changed_lines_threshold: 300,
      charged_usd: 0.001,
      client: 'vibe-mistral-small',
      cost_usd: 0.001,
      flow: 'github.com/jorisjonkers-dev/agent-kit#167',
      model: 'mistral-small-latest',
      prompt_hash: 'triage-1',
      recorded_at: '2026-10-30T12:00:00.000Z',
      repository: 'github.com/jorisjonkers-dev/agent-kit',
      risk_signals: [],
      run_id: '20261030T120000Z-triage',
      stage: 'ticket-review',
      status: 'skipped-low-risk',
      ticket: 'github.com/jorisjonkers-dev/agent-kit#170',
      triage_signal: { answer: 'no', kind: 'model' },
      type: 'run',
    })
    const h = await opened()
    await ledger.writeTo(h)

    const result = await h.commands.stats.run({})

    if (result.status !== 'reported') throw new Error('not reported')
    expect(cell(result, MISTRAL).accepted).toBe(20)
  })

  it('refuses a ledger line that is not JSON, naming its line', async () => {
    const h = await opened()
    const path = await runs(new SyntheticLedger(), 1, []).writeTo(h)
    await h.writeText(path, `${await h.readText(path)}{not json\n`)

    await expect(h.commands.stats.run({})).rejects.toMatchObject({
      code: 'invalid-ledger',
      message: expect.stringContaining('line 3 is not JSON') as unknown,
    })
  })

  it('refuses a ledger line without the shape record writes', async () => {
    const h = await opened()
    await new SyntheticLedger()
      .raw({
        recorded_at: 'x',
        repository: AGENT_KIT,
        run_id: 'r',
        stage: 'spec',
        type: 'finding',
        verdict: 'maybe',
      })
      .writeTo(h)

    await expect(h.commands.stats.run({})).rejects.toMatchObject({
      code: 'invalid-ledger',
      message: expect.stringContaining('line 1.') as unknown,
    })
  })

  it('refuses a finding raised by a critic that has no run line', async () => {
    const h = await opened()
    await new SyntheticLedger()
      .run({
        critics: [{ client: MISTRAL }],
        findings: [{ raised_by: [GLM], verdict: 'accepted' }],
      })
      .writeTo(h)

    await expect(h.commands.stats.run({})).rejects.toMatchObject({ code: 'invalid-ledger' })
  })

  it('reads the ledger lines record writes', async () => {
    const h = await opened({
      scripts: [
        {
          exitCode: 0,
          exportJson: isolatedExport(),
          stdout: vibeStdout(fenced({ findings: [finding()] })),
        },
      ],
    })
    await h.commands.allow.run({ families: ['mistral'], repo: h.repo })
    const run = await h.commands.prepare.run({
      orchestrator_family: 'anthropic',
      repo: h.repo,
      spec: '.scratch/widgets/spec.md',
      stage: 'spec',
    })
    if (run.status !== 'ready') throw new Error('not ready')
    await h.commands.fanOut.run({ run_dir: run.run_dir })
    await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [{ finding_id: 'F1', verdict: 'accepted' }],
    })

    const result = await h.commands.stats.run({})

    expect(result).toMatchObject({
      cells: [
        {
          accepted: 1,
          client: MISTRAL,
          cost_usd: 0.0123,
          precision: 1,
          runs: 1,
          stage: 'spec',
          unique_catches: 1,
        },
      ],
      status: 'reported',
    })
  })

  it('writes nothing: neither routing nor the ledger changes', async () => {
    const h = await opened({ routing: await shippedRouting() })
    const path = await runs(new SyntheticLedger(), 20, raised(GLM, { accepted: 20 })).writeTo(h)
    const routingPath = join(h.assetsRoot, 'routing.json')
    const before = [await h.readText(routingPath), await h.readText(path)]

    await h.commands.stats.run({})

    expect([await h.readText(routingPath), await h.readText(path)]).toEqual(before)
  })
})

describe('the critique stats report', () => {
  async function reported(): Promise<StatsResult> {
    const ledger = new SyntheticLedger()
    runs(
      ledger,
      20,
      [
        ...raised(MISTRAL, { accepted: 11, rejected: 9 }),
        ...raised(GLM, { accepted: 16, rejected: 4 }),
      ],
      {
        critics: [
          { client: MISTRAL, cost_usd: 0.022 },
          { client: GLM, cost_usd: null },
        ],
      },
    )
    return stats(ledger, { repository: AGENT_KIT, since: '2026-10-01' })
  }

  it('says what the figures cover', async () => {
    const { report } = await reported()

    expect(report.split('\n').slice(0, 4)).toEqual([
      '# Critique stats',
      '',
      expect.stringMatching(
        /^Ledger: .*ledger\.jsonl\. Repository: github\.com\/jorisjonkers-dev\/agent-kit\. Dates: 2026-10-01 to today\.$/,
      ) as unknown,
      'One row per critic client and prompt. A row with fewer than 20 adjudicated findings gets no suggestion.',
    ])
  })

  it('shows one table row per cell under its stage', async () => {
    const result = await reported()

    const mistral = cell(result, MISTRAL)
    expect(result.report).toContain(
      [
        '## spec',
        '',
        '| client | prompt | runs | adjudicated | precision | unique | fabricated | cost per accepted | failed | timed out | wasted |',
        '|---|---|---|---|---|---|---|---|---|---|---|',
        `| vibe-glm | ${cell(result, GLM).prompt_hash} | 20 | 20 | 0.80 | 16 | 0% | - | 0% | 0% | - |`,
        `| vibe-mistral-large | ${mistral.prompt_hash} | 20 | 20 | 0.55 | 11 | 0% | $0.0400 | 0% | 0% | - |`,
      ].join('\n'),
    )
  })

  it('marks a row with too few adjudicated findings and shows a missing figure as a dash', async () => {
    const result = await stats(runs(new SyntheticLedger(), 20, raised(GLM, { hint: 20 })))

    expect(result.report).toContain(
      `| vibe-glm | ${cell(result, GLM).prompt_hash} | 20 | 0 (too few) | - | 0 | 0% | - | 0% | 0% | - |`,
    )
  })

  it('ends with the removal candidates and the suggestions', async () => {
    const { report } = await reported()

    expect(report.split('## Removal candidates')[1]).toBe(
      [
        '',
        '',
        'none',
        '',
        '## Suggestions',
        '',
        '- vibe-glm has higher precision than vibe-mistral-large on the spec stage over 40 findings (0.80 against 0.55); consider swapping.',
        '',
      ].join('\n'),
    )
  })

  it('lists each removal candidate with the features it looked at', async () => {
    const ledger = new SyntheticLedger()
    for (const spec of ['#1', '#2', '#3'])
      runs(ledger, 7, raised([MISTRAL, GLM], { accepted: 7 }), { spec })

    const { report } = await stats(ledger)

    expect(report).toContain(
      '## Removal candidates\n\n- spec: no unique accepted catch across its last 3 features (#3, #2, #1)\n',
    )
    expect(report).toContain('Dates: all.')
    expect(report).toContain('## Suggestions\n\nnone\n')
  })
})
