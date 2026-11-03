import { readFile, rm } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  agyFindings,
  DECISION_CANARY,
  fenced,
  finding,
  harness,
  isolatedExport,
  SyntheticLedger,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import type { FanOutResult, PrepareResult, ReviewsResult } from './index.js'

/**
 * The first review stage, `adr-review` (#220): a run over a repository's
 * decision records that is its own flow, keyed by its run id, with its own
 * spend, outside every feature's flow (ADR-0003).
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const ADR_ONE =
  '# The nightly job frobnicates widgets\n\nWe frobnicate widgets in the nightly job only.\n'
const ADR_TWO = '# Widgets keep a seven-second window\n\nThe window is seven seconds.\n'

/**
 * The checks an ADR review's critics declare. The shipped roster is
 * `spec-redteam` with the `fidelity` and `integration` lenses until the
 * `adr-reviewer` role ships its own lenses, so these tests declare the ADR
 * checks they exercise on those two lenses in a private copy of the assets.
 */
const ADR_CHECKS: NonNullable<Parameters<typeof harness>[0]>['assets'] = {
  'lenses/fidelity.md': (shipped) =>
    `${shipped}- \`no-decision-sentence\` (critical): No sentence states what was decided.\n- \`no-cost\` (major): No cost is named.\n- \`two-readings\` (major): Wording two readers would build differently.\n- \`no-confirmation\` (minor): Needs \`tree/\`. No check named.\n`,
  'lenses/integration.md': (shipped) =>
    `${shipped}- \`records-contradict\` (major): Two records contradict each other.\n`,
}

async function withAdrs(options: Parameters<typeof harness>[0] = {}): Promise<Harness> {
  const h = await harness({ ...options, assets: { ...ADR_CHECKS, ...options.assets } })
  open.push(h)
  await h.commands.allow.run({ families: ['mistral'], repo: h.repo })
  await h.writeText(join(h.repo, 'docs', 'adr', '0001-nightly-job.md'), ADR_ONE)
  await h.writeText(join(h.repo, 'docs', 'adr', '0002-window.md'), ADR_TWO)
  return h
}

async function prepared(h: Harness, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const run = await h.commands.prepare.run({
    orchestrator_family: 'anthropic',
    repo: h.repo,
    stage: 'adr-review',
    ...extra,
  })
  if (run.status !== 'ready') throw new Error(`not ready: ${JSON.stringify(run)}`)
  return run
}

async function manifestOf(run: PrepareResult): Promise<Record<string, unknown>> {
  return JSON.parse(await readFile(run.manifest_path, 'utf8')) as Record<string, unknown>
}

function noFindings(): VibeScript {
  return { exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings: [] })) }
}

async function ledgerLines(h: Harness): Promise<Record<string, unknown>[]> {
  const text = await readFile(
    join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl'),
    'utf8',
  )
  return text
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line) as Record<string, unknown>)
}

describe('an adr-review prepare on a repository with decision records', () => {
  it('is ready with the ADRs as the artifact and the glossary as a reference', async () => {
    const h = await withAdrs()

    const run = await prepared(h)

    expect(run.packet_files).toEqual([
      'artifact/docs/adr/0001-nightly-job.md',
      'artifact/docs/adr/0002-window.md',
      'references/CONTEXT.md',
      'check/adrs.json',
    ])
    expect(
      await readFile(
        join(run.packet_dir, 'artifact', 'docs', 'adr', '0001-nightly-job.md'),
        'utf8',
      ),
    ).toBe(ADR_ONE)
    expect(await readFile(join(run.packet_dir, 'references', 'CONTEXT.md'), 'utf8')).toContain(
      DECISION_CANARY,
    )
    expect(run.artifact).toEqual({ labels: [], reference: 'adrs', tracker: 'none' })
  })
})

describe('the records an adr-review prepare freezes', () => {
  it('reviews the named records, with the glossary and the other records as references', async () => {
    const h = await withAdrs()

    const run = await prepared(h, { adrs: ['docs/adr/0002-window.md'] })

    expect(run.packet_files).toEqual([
      'artifact/docs/adr/0002-window.md',
      'references/CONTEXT.md',
      'references/docs/adr/0001-nightly-job.md',
      'check/adrs.json',
    ])
  })

  it('takes the references the caller names instead', async () => {
    const h = await withAdrs()
    await h.writeText(join(h.repo, 'docs', 'glossary.md'), '# Words\n')

    const run = await prepared(h, { references: ['docs/glossary.md'] })

    expect(run.packet_files).toEqual([
      'artifact/docs/adr/0001-nightly-job.md',
      'artifact/docs/adr/0002-window.md',
      'references/docs/glossary.md',
      'check/adrs.json',
    ])
  })

  it('sends no glossary when the repository has none', async () => {
    const h = await withAdrs()
    await h.removeFile(join(h.repo, 'CONTEXT.md'))

    const run = await prepared(h)

    expect(run.packet_files).toEqual([
      'artifact/docs/adr/0001-nightly-job.md',
      'artifact/docs/adr/0002-window.md',
      'check/adrs.json',
    ])
  })

  it('reads no tracker, so a repository whose tracker the critique cannot read still runs', async () => {
    const h = await withAdrs({ tracker: 'gitlab' })

    expect((await prepared(h)).status).toBe('ready')
  })

  it('is refused when the repository has no decision records', async () => {
    const h = await harness()
    open.push(h)
    await h.commands.allow.run({ families: ['mistral'], repo: h.repo })

    const result = await h.commands.prepare.run({
      orchestrator_family: 'anthropic',
      repo: h.repo,
      stage: 'adr-review',
    })

    expect(result).toEqual({
      message: 'found no decision records in docs/adr/ to review',
      reason: 'no-adrs',
      status: 'refused',
    })
  })

  it('runs on documents alone while the repository allows no code', async () => {
    const h = await withAdrs()

    const run = await prepared(h)

    expect(run.code).toBeUndefined()
    expect(
      run.packet_files.every(
        (path) =>
          path.startsWith('artifact/') ||
          path.startsWith('references/') ||
          path.startsWith('check/'),
      ),
    ).toBe(true)
  })
})

describe("the decision log's pre-check in an adr-review packet", () => {
  async function report(run: PrepareResult): Promise<unknown> {
    return JSON.parse(await readFile(join(run.packet_dir, 'check', 'adrs.json'), 'utf8')) as unknown
  }

  it('freezes what check-adrs reports under check/', async () => {
    const h = await withAdrs()
    await h.writeText(
      join(h.repo, 'docs', 'adr', '0004-retry.md'),
      '# Retry\n\nStatus: pending\n\nRetries follow ADR-0009.\n',
    )
    const checked = await h.commands.checkAdrs.run({ repo: h.repo })

    const run = await prepared(h)

    expect(await report(run)).toEqual(checked)
    expect(await report(run)).toMatchObject({
      numbering: { gaps: [3] },
      status: 'failed',
      unknown_statuses: [{ record: 'docs/adr/0004-retry.md', status: 'pending' }],
    })
    expect((await manifestOf(run)).packet_files).toContainEqual(
      expect.objectContaining({
        kind: 'check',
        path: 'check/adrs.json',
        source: 'check-adrs docs/adr',
      }),
    )
  })

  it('checks the whole log when only some records are under review', async () => {
    const h = await withAdrs()

    const run = await prepared(h, { adrs: ['docs/adr/0002-window.md'] })

    expect(await report(run)).toMatchObject({
      records: [{ path: 'docs/adr/0001-nightly-job.md' }, { path: 'docs/adr/0002-window.md' }],
      status: 'passed',
    })
  })

  it('tells each critic the report is settled', async () => {
    const h = await withAdrs({ scripts: [noFindings()] })
    const run = await prepared(h)

    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const prompt = await readFile(
      join(run.run_dir, 'critics', 'c1-vibe-mistral-large', 'prompt.md'),
      'utf8',
    )
    expect(prompt).toContain(
      "- `check/adrs.json`: the deterministic decision-log checker's report on every record: numbering, status values, link and supersede targets, and superseded records still cited. Settled: build on it and never recompute it",
    )
  })
})

describe('an adr-review run as a flow of its own', () => {
  it('keys each run by its own run id, so two runs get distinct flows and spend lines', async () => {
    const h = await withAdrs({ scripts: [noFindings(), noFindings()] })

    const first = await prepared(h)
    await h.commands.fanOut.run({ run_dir: first.run_dir })
    const second = await prepared(h)
    await h.commands.fanOut.run({ run_dir: second.run_dir })

    expect(first.run_id).not.toBe(second.run_id)
    expect([(await manifestOf(first)).flow, (await manifestOf(second)).flow]).toEqual([
      first.run_id,
      second.run_id,
    ])
    const spend = (await ledgerLines(h))
      .filter((line) => line.type === 'invocation')
      .map((line) => [line.flow, line.run_id, line.charged_usd])
    expect(spend).toEqual([
      [first.run_id, first.run_id, 0.0123],
      [second.run_id, second.run_id, 0.0123],
    ])
  })

  it("refuses a named flow, so a review never charges a feature's spec or draft", async () => {
    const h = await withAdrs()

    await expect(prepared(h, { flow: '.scratch/widgets/spec.md' })).rejects.toThrow(
      /adr-review is a review stage: each run is a flow of its own, so it takes no flow/,
    )
  })
})

describe('the review runs /flow lists', () => {
  const ADR_QUOTE = {
    file: 'artifact/docs/adr/0001-nightly-job.md',
    kind: 'quote',
    value: 'We frobnicate widgets in the nightly job only.',
  }
  const GLOSSARY_QUOTE = { file: 'references/CONTEXT.md', kind: 'quote', value: DECISION_CANARY }

  function critic(findings: readonly Record<string, unknown>[]): VibeScript {
    return { exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings })) }
  }

  function adrFinding(id: string): Record<string, unknown> {
    return finding({
      evidence: [ADR_QUOTE, GLOSSARY_QUOTE],
      id,
      location: `docs/adr/0001-nightly-job.md ${id}`,
    })
  }

  async function reviews(h: Harness, extra: Record<string, unknown> = {}): Promise<ReviewsResult> {
    const result = await h.commands.reviews.run({ repo: h.repo, ...extra })
    if (result.status !== 'read') throw new Error(`reviews refused: ${result.message}`)
    return result
  }

  it('lists each review run, newest first, with its stage, run id, verdict counts and spend', async () => {
    const h = await withAdrs({
      scripts: [critic([adrFinding('F1'), adrFinding('F2'), adrFinding('F3')]), critic([])],
    })
    const first = await prepared(h)
    await h.commands.fanOut.run({ run_dir: first.run_dir })
    await h.commands.record.run({
      run_dir: first.run_dir,
      verdicts: [
        { finding_id: 'F1', verdict: 'accepted' },
        { finding_id: 'F2', reason: 'Which job?', verdict: 'needs-human' },
        {
          finding_id: 'F3',
          reason: 'The glossary says so.',
          reason_code: 'contradicted-by-docs',
          verdict: 'rejected',
        },
      ],
    })
    h.clock.advance(1)
    const second = await prepared(h)
    await h.commands.fanOut.run({ run_dir: second.run_dir })

    const result = await reviews(h)

    expect(result.runs).toEqual([
      {
        recorded: false,
        round: 1,
        run_id: second.run_id,
        spent_usd: 0.0123,
        stage: 'adr-review',
        verdicts: { accepted: 0, hint: 0, 'needs-human': 0, rejected: 0 },
      },
      {
        recorded: true,
        round: 1,
        run_id: first.run_id,
        spent_usd: 0.0123,
        stage: 'adr-review',
        verdicts: { accepted: 1, hint: 0, 'needs-human': 1, rejected: 1 },
      },
    ])
  })

  it("counts a refuter's spend in the review run's own flow, and in the run's listed spend", async () => {
    const h = await withAdrs()
    await h.commands.allow.run({ families: ['mistral', 'zhipu'], repo: h.repo })
    const run = await prepared(h)
    h.process.queue(
      { ...critic([{ ...adrFinding('F1'), severity: 'critical' }]), agent: 'critic-mistral-large' },
      { ...critic([]), agent: 'critic-glm' },
    )
    await h.commands.fanOut.run({ run_dir: run.run_dir })
    h.process.queue({
      ...critic([]),
      agent: 'critic-glm',
      exportJson: { ...isolatedExport(), cost_usd: 0.042 },
    })

    const refuted = await h.commands.refute.run({
      run_dir: run.run_dir,
      verdicts: [{ finding_id: 'F1', verdict: 'accepted' }],
    })

    expect(refuted.results).toMatchObject([{ finding_id: 'F1', refuter: { client: 'vibe-glm' } }])
    const invocations = (await ledgerLines(h)).filter((line) => line.type === 'invocation')
    expect(invocations.find((line) => line.critic_id === 'r-F1-vibe-glm')).toMatchObject({
      charged_usd: 0.042,
      flow: run.run_id,
      run_id: run.run_id,
      stage: 'adr-review',
    })
    const spent = invocations
      .filter((line) => line.kind === 'vibe')
      .reduce((sum, line) => sum + Number(line.charged_usd), 0)
    expect((await reviews(h)).runs).toMatchObject([{ run_id: run.run_id, spent_usd: spent }])
    expect(spent).toBeGreaterThan(0.042)
  })

  it('reports an open critical question without naming a feature command not to start', async () => {
    const h = await withAdrs({ scripts: [critic([{ ...adrFinding('F1'), severity: 'critical' }])] })
    const run = await prepared(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const recorded = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [{ finding_id: 'F1', reason: 'Which job?', verdict: 'needs-human' }],
    })

    expect(recorded.summary).not.toContain('Do not start')
    expect(await readFile(join(run.run_dir, 'report.md'), 'utf8')).not.toContain('Do not start')
  })

  it('leaves out every run of a feature, and never calls a review due', async () => {
    const h = await withAdrs({ scripts: [critic([])] })
    await h.commands.prepare.run({
      orchestrator_family: 'anthropic',
      repo: h.repo,
      spec: '.scratch/widgets/spec.md',
      stage: 'spec',
    })
    await h.commands.state.run({ repo: h.repo, spec: '.scratch/widgets/spec.md' })

    const result = await reviews(h)

    expect(result).toEqual({ runs: [], status: 'read' })
  })

  it('keeps a recorded run whose directory is gone, from the ledger', async () => {
    const h = await withAdrs({ scripts: [critic([adrFinding('F1')])] })
    const run = await prepared(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })
    await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [{ finding_id: 'F1', verdict: 'accepted' }],
    })
    await rm(run.run_dir, { force: true, recursive: true })

    const result = await reviews(h)

    expect(result.runs).toEqual([
      {
        recorded: true,
        round: 1,
        run_id: run.run_id,
        spent_usd: 0.0123,
        stage: 'adr-review',
        verdicts: { accepted: 1, hint: 0, 'needs-human': 0, rejected: 0 },
      },
    ])
  })

  it('lists at most the number of runs asked for', async () => {
    const h = await withAdrs()
    const runs: PrepareResult[] = []
    for (let n = 0; n < 3; n += 1) {
      h.clock.advance(1)
      runs.push(await prepared(h))
    }

    const result = await reviews(h, { limit: 2 })

    expect(result.runs.map((run) => run.run_id)).toEqual([runs[2]?.run_id, runs[1]?.run_id])
  })

  it("leaves out another repository's runs", async () => {
    const h = await withAdrs()
    await new SyntheticLedger()
      .run({
        critics: [{ client: 'vibe-mistral-large' }],
        repository: 'github.com/jorisjonkers-dev/other',
        spec: 'adrs',
        stage: 'adr-review',
      })
      .writeTo(h)

    expect((await reviews(h)).runs).toEqual([])
  })

  it('is refused for a repository whose remotes refuse a critique', async () => {
    const h = await harness({ remotes: [] })
    open.push(h)

    expect(await h.commands.reviews.run({ repo: h.repo })).toMatchObject({
      reason: 'no-remote',
      status: 'refused',
    })
  })
})

describe('the depth of an adr-review run', () => {
  async function withRecords(count: number, text = 'We decided.'): Promise<Harness> {
    const h = await harness()
    open.push(h)
    await h.commands.allow.run({ families: ['mistral'], repo: h.repo })
    for (let n = 1; n <= count; n += 1) {
      await h.writeText(
        join(h.repo, 'docs', 'adr', `${String(n).padStart(4, '0')}-record.md`),
        `# Record ${String(n)}\n\n${text}\n`,
      )
    }
    return h
  }

  it('goes deep with more than 10 records under review', async () => {
    const run = await prepared(await withRecords(11))

    expect(run.depth).toBe('deep')
    expect(run.depth_choice).toEqual({
      chosen_by: 'signals',
      measures: { adrs: { threshold: 10, value: 11 } },
      signals: [{ kind: 'threshold', measure: 'adrs', threshold: 10, value: 11 }],
    })
  })

  it('stays base with 10 records and no risk term', async () => {
    const run = await prepared(await withRecords(10))

    expect(run.depth).toBe('base')
    expect(run.depth_choice.signals).toEqual([])
  })

  it('counts the records under review, not every record in the repository', async () => {
    const run = await prepared(await withRecords(11), {
      adrs: ['docs/adr/0001-record.md', 'docs/adr/0002-record.md'],
    })

    expect(run.depth).toBe('base')
    expect(run.depth_choice.measures).toEqual({ adrs: { threshold: 10, value: 2 } })
  })

  it("goes deep on one of the spec stage's risk terms", async () => {
    const run = await prepared(
      await withRecords(1, 'We run the schema migration before the deploy.'),
    )

    expect(run.depth).toBe('deep')
    expect(run.depth_choice.signals).toEqual([{ kind: 'risk-term', term: 'migration' }])
  })

  it('takes the depth the caller names over the signals', async () => {
    const run = await prepared(await withRecords(11), { depth: 'base' })

    expect(run.depth).toBe('base')
    expect(run.depth_choice.chosen_by).toBe('argument')
  })
})

describe("the adr-review stage's own evidence rule (#249)", () => {
  const ONE = {
    file: 'artifact/docs/adr/0001-nightly-job.md',
    kind: 'quote',
    value: 'We frobnicate widgets in the nightly job only.',
  }
  const TWO = {
    file: 'artifact/docs/adr/0002-window.md',
    kind: 'quote',
    value: 'The window is seven seconds.',
  }
  const ONE_TITLE = {
    file: 'artifact/docs/adr/0001-nightly-job.md',
    kind: 'quote',
    value: 'The nightly job frobnicates widgets',
  }
  const GLOSSARY = { file: 'references/CONTEXT.md', kind: 'quote', value: DECISION_CANARY }

  async function fannedOut(
    raised: readonly Record<string, unknown>[],
    agy: readonly Record<string, unknown>[] = [],
    options: {
      readonly records?: Readonly<Record<string, string>>
      readonly prepare?: Record<string, unknown>
    } = {},
  ): Promise<FanOutResult> {
    const h = await withAdrs({
      agyScripts: [
        agyFindings(
          agy.length === 0
            ? { findings: [], no_findings_reason: 'Read every record.' }
            : { findings: agy },
        ),
      ],
      scripts: [
        {
          exitCode: 0,
          exportJson: isolatedExport(),
          stdout: vibeStdout(fenced({ findings: raised })),
        },
      ],
    })
    for (const [name, text] of Object.entries(options.records ?? {}))
      await h.writeText(join(h.repo, 'docs', 'adr', name), text)
    await h.commands.allow.run({ families: ['mistral', 'google'], repo: h.repo })
    const run = await prepared(h, options.prepare)
    return h.commands.fanOut.run({ run_dir: run.run_dir })
  }

  it('returns findings with the checks the critics declare, left to the orchestrator', async () => {
    const noCost = finding({
      category: 'omission',
      check: 'no-cost',
      evidence: [ONE],
      id: 'F1',
      location: 'docs/adr/0001-nightly-job.md: no-cost',
    })
    const contradict = finding({
      category: 'contradiction',
      check: 'records-contradict',
      evidence: [ONE, TWO],
      id: 'F1',
      location: 'docs/adr/0001-nightly-job.md, docs/adr/0002-window.md: records-contradict',
    })

    const result = await fannedOut([noCost], [contradict])

    expect(
      result.findings.map((found) => [found.check, found.raised_by, found.pre_verdict]),
    ).toEqual([
      ['no-cost', ['c1-vibe-mistral-large'], undefined],
      ['records-contradict', ['c2-agy-gemini-flash-high'], undefined],
    ])
  })

  it('gives a contradiction whose quotes come from one file a mechanical pre-verdict', async () => {
    const oneFile = finding({
      category: 'contradiction',
      check: 'records-contradict',
      evidence: [ONE, ONE_TITLE],
      location: 'docs/adr/0001-nightly-job.md: records-contradict',
    })

    const result = await fannedOut([], [oneFile])

    expect(result.findings[0]?.pre_verdict).toEqual({
      reason: 'a contradiction quotes one file; it needs a quote from each of two files',
      reason_code: 'one-file-contradiction',
      verdict: 'hint',
    })
  })

  it('takes a record and the glossary as the two files of a contradiction, beside any other evidence', async () => {
    const withGlossary = finding({
      category: 'contradiction',
      check: 'records-contradict',
      evidence: [ONE, { kind: 'input', value: 'A widget frobnicated at noon.' }, GLOSSARY],
      location: 'docs/adr/0001-nightly-job.md: records-contradict',
    })

    const result = await fannedOut([], [withGlossary])

    expect(result.findings[0]?.pre_verdict).toBeUndefined()
  })

  it('gives a finding that quotes no record under review a mechanical pre-verdict', async () => {
    const glossaryOnly = finding({
      category: 'omission',
      check: 'no-cost',
      evidence: [GLOSSARY],
      location: 'docs/adr/0001-nightly-job.md: no-cost',
    })

    const result = await fannedOut([glossaryOnly])

    expect(result.findings[0]?.pre_verdict).toEqual({
      reason: 'no quote from a record under review',
      reason_code: 'no-record-quote',
      verdict: 'hint',
    })
  })

  describe('a contradiction whose two quotes come from one record, one of them repeated in another', () => {
    // 0001 holds both sentences; 0002 repeats the first, so each quote is found in two files, yet only 0001 holds both.
    const records = {
      '0001-nightly-job.md':
        '# The nightly job frobnicates widgets\n\nWe frobnicate widgets in the nightly job only.\n\nThe window is seven seconds.\n',
      '0002-window.md':
        '# Widgets keep a window\n\nWe frobnicate widgets in the nightly job only.\n',
    }
    const sameRecord = finding({
      category: 'contradiction',
      check: 'records-contradict',
      evidence: [
        ONE,
        {
          file: 'artifact/docs/adr/0001-nightly-job.md',
          kind: 'quote',
          value: 'The window is seven seconds.',
        },
      ],
      location: 'docs/adr/0001-nightly-job.md: records-contradict',
    })
    const hint = {
      reason: 'a contradiction quotes one file; it needs a quote from each of two files',
      reason_code: 'one-file-contradiction',
      verdict: 'hint',
    }

    it('is a one-file contradiction when both records are under review', async () => {
      const result = await fannedOut([], [sameRecord], { records })

      expect(result.findings[0]?.pre_verdict).toEqual(hint)
    })

    it('is a one-file contradiction when the other record is a reference', async () => {
      const result = await fannedOut([], [sameRecord], {
        prepare: { adrs: ['docs/adr/0001-nightly-job.md'] },
        records,
      })

      expect(result.findings[0]?.pre_verdict).toEqual(hint)
    })
  })

  it('needs both readings as an input for wording with two readings', async () => {
    const bare = finding({
      category: 'unasked-question',
      check: 'two-readings',
      evidence: [TWO],
      id: 'F1',
      location: 'docs/adr/0002-window.md: two-readings',
    })
    const given = finding({
      category: 'unasked-question',
      check: 'two-readings',
      evidence: [
        ONE,
        {
          kind: 'input',
          value: 'Reading A: only the nightly job; reading B: the nightly job among others.',
        },
      ],
      id: 'F2',
      location: 'docs/adr/0001-nightly-job.md: two-readings',
    })

    const result = await fannedOut([bare, given])

    expect(result.findings.map((found) => found.pre_verdict?.reason_code)).toEqual([
      'no-scenario',
      undefined,
    ])
  })
})

describe('a documents-only adr-review run (#249)', () => {
  const ONE = {
    file: 'artifact/docs/adr/0001-nightly-job.md',
    kind: 'quote',
    value: 'We frobnicate widgets in the nightly job only.',
  }

  async function recordedWith(raised: readonly Record<string, unknown>[], code = false) {
    const h = await withAdrs({
      agyScripts: [agyFindings({ findings: [], no_findings_reason: 'Read every record.' })],
      scripts: [
        {
          exitCode: 0,
          exportJson: isolatedExport(),
          stdout: vibeStdout(fenced({ findings: raised })),
        },
      ],
    })
    await h.commands.allow.run({ code, families: ['mistral', 'google'], repo: h.repo })
    await h.writeText(join(h.repo, 'src', 'index.ts'), 'export const nightly = true\n')
    await h.git.commit()
    const run = await prepared(h)
    const fanned = await h.commands.fanOut.run({ run_dir: run.run_dir })
    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })
    return { fanned, recorded, report: await readFile(recorded.report_path, 'utf8'), run }
  }

  it('makes a finding of a check that needs tree/, or of drift or stale, a hint', async () => {
    const confirmation = finding({
      category: 'untestable',
      check: 'no-confirmation',
      evidence: [ONE],
      id: 'F1',
      location: 'docs/adr/0001-nightly-job.md: no-confirmation',
    })
    const drift = finding({
      category: 'drift',
      check: 'no-cost',
      evidence: [ONE],
      id: 'F2',
      location: 'docs/adr/0001-nightly-job.md: drift',
    })
    const stale = finding({
      category: 'stale',
      check: 'no-cost',
      evidence: [ONE],
      id: 'F3',
      location: 'docs/adr/0001-nightly-job.md: stale',
    })

    const { fanned } = await recordedWith([confirmation, drift, stale])

    const hint = {
      reason: 'the check needs code under tree/, which this packet does not hold',
      reason_code: 'needs-tree',
      verdict: 'hint',
    }
    expect(fanned.findings.map((found) => found.pre_verdict)).toEqual([hint, hint, hint])
  })

  it('names the drift, stale-against-code and confirmation checks as skipped in the report and the summary', async () => {
    const { recorded, report } = await recordedWith([])

    expect(report).toContain(
      [
        '## Skipped checks',
        '',
        'The packet holds no code under `tree/`: an ADR review sends documents only while the code permission is off. These checks need the code, so they did not run:',
        '',
        '- `drift` (category): a decision that names code, config or a command, checked against the code',
        '- `stale` (category): an assumption, revisit condition or promised consequence, checked against the code',
        '- `no-confirmation` (lens fidelity)',
        '',
      ].join('\n'),
    )
    expect(recorded.summary).toContain(
      'Skipped, documents only: `drift`, `stale`, `no-confirmation` (the report says why)',
    )
  })

  it('names a role rule marked as needing tree/ as skipped, as it gives its findings the needs-tree hint', async () => {
    const h = await withAdrs({
      agyScripts: [agyFindings({ findings: [], no_findings_reason: 'Read every record.' })],
      assets: {
        'roles/spec-redteam.md': (shipped) =>
          shipped.replace('- `ignored-failure`: ', '- `ignored-failure`: Needs `tree/`. '),
      },
      scripts: [
        {
          exitCode: 0,
          exportJson: isolatedExport(),
          stdout: vibeStdout(
            fenced({
              findings: [
                finding({
                  category: 'structure',
                  check: 'ignored-failure',
                  evidence: [ONE],
                  location: 'docs/adr/0001-nightly-job.md: ignored-failure',
                }),
              ],
            }),
          ),
        },
      ],
    })
    await h.commands.allow.run({ families: ['mistral', 'google'], repo: h.repo })
    const run = await prepared(h)

    const fanned = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(fanned.findings[0]?.pre_verdict?.reason_code).toBe('needs-tree')
    expect(fanned.skipped_checks).toContainEqual({
      check: 'ignored-failure',
      from: 'role spec-redteam',
    })
  })

  it('skips nothing once the packet holds code', async () => {
    const { recorded, report, run } = await recordedWith([], true)

    expect(run.packet_files).toContain('tree/src/index.ts')
    expect(report).not.toContain('## Skipped checks')
    expect(recorded.summary).not.toContain('Skipped')
  })
})

describe('the second round of an adr-review run (#249)', () => {
  const RECORDS = [
    '0001-nightly-job.md',
    '0002-window.md',
    '0003-retry.md',
    '0004-backoff.md',
  ] as const
  const ACCEPTED = RECORDS.map((_, n) => ({ finding_id: `F${String(n + 1)}`, verdict: 'accepted' }))

  function critic(agent: string, findings: readonly Record<string, unknown>[] = []): VibeScript {
    return {
      agent,
      exitCode: 0,
      exportJson: isolatedExport(),
      stdout: vibeStdout(
        fenced(
          findings.length === 0
            ? { findings, no_findings_reason: 'Read every record.' }
            : { findings },
        ),
      ),
    }
  }

  function undecided(record: string, n: number): Record<string, unknown> {
    return finding({
      category: 'omission',
      check: 'no-decision-sentence',
      evidence: [
        {
          file: `artifact/docs/adr/${record}`,
          kind: 'quote',
          value: `Record ${String(n + 1)} weighs its options.`,
        },
      ],
      id: `F${String(n + 1)}`,
      location: `docs/adr/${record}: no-decision-sentence`,
    })
  }

  /** A first round in which Mistral Large finds four records with no decision sentence, each critical by its check. */
  async function fourCriticals(): Promise<{ h: Harness; run: PrepareResult }> {
    const h = await harness({ assets: ADR_CHECKS })
    open.push(h)
    await h.commands.allow.run({ families: ['mistral', 'zhipu'], repo: h.repo })
    for (const [n, record] of RECORDS.entries()) {
      await h.writeText(
        join(h.repo, 'docs', 'adr', record),
        `# Record ${String(n + 1)}\n\nRecord ${String(n + 1)} weighs its options.\n`,
      )
    }
    const run = await prepared(h)
    h.process.queue(critic('critic-mistral-large', RECORDS.map(undecided)), critic('critic-glm'))
    await h.commands.fanOut.run({ run_dir: run.run_dir })
    return { h, run }
  }

  it('is due after more than three criticals still accepted after the refute round, and runs once more', async () => {
    const { h, run } = await fourCriticals()
    h.process.queue(
      critic('critic-glm'),
      critic('critic-glm'),
      critic('critic-glm'),
      critic('critic-glm'),
    )
    const refuted = await h.commands.refute.run({ run_dir: run.run_dir, verdicts: ACCEPTED })
    expect(refuted.counts).toMatchObject({ 'no-objection': 4 })

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: ACCEPTED })

    expect(recorded).toMatchObject({ counts: { accepted: 4 }, round: 1, second_round_due: true })
    h.clock.advance(1)
    const second = await prepared(h, { previous_run_dir: run.run_dir })
    expect(second.round).toBe(2)
    h.process.queue(critic('critic-mistral-large'), critic('critic-glm'))
    await h.commands.fanOut.run({ run_dir: second.run_dir })
    expect(await h.commands.record.run({ run_dir: second.run_dir, verdicts: [] })).toMatchObject({
      round: 2,
      second_round_due: false,
    })
  })

  it('is not due once the refute round rejects one of four criticals', async () => {
    const { h, run } = await fourCriticals()
    const refutation = finding({
      category: 'omission',
      check: 'refute',
      claim: 'The record states its decision.',
      evidence: [
        {
          file: 'artifact/docs/adr/0001-nightly-job.md',
          kind: 'quote',
          value: 'Record 1 weighs its options.',
        },
      ],
      location: 'docs/adr/0001-nightly-job.md: no-decision-sentence',
    })
    h.process.queue(
      critic('critic-glm', [refutation]),
      critic('critic-glm'),
      critic('critic-glm'),
      critic('critic-glm'),
    )
    const refuted = await h.commands.refute.run({ run_dir: run.run_dir, verdicts: ACCEPTED })
    expect(refuted.counts).toMatchObject({ 'no-objection': 3, refute: 1 })

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: ACCEPTED })

    expect(recorded).toMatchObject({
      counts: { accepted: 3, rejected: 1 },
      second_round_due: false,
    })
  })
})
