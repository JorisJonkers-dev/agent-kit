import { readFile, rm } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
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
import type { PrepareResult, ReviewsResult } from './index.js'

/**
 * The first review stage, `adr-review` (#220): a run over a repository's
 * decision records that is its own flow, keyed by its run id, with its own
 * spend, outside every feature's flow (ADR-0003).
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const ADR_ONE = '# The nightly job frobnicates widgets\n\nWe frobnicate widgets in the nightly job only.\n'
const ADR_TWO = '# Widgets keep a seven-second window\n\nThe window is seven seconds.\n'

async function withAdrs(options: Parameters<typeof harness>[0] = {}): Promise<Harness> {
  const h = await harness(options)
  open.push(h)
  await h.commands.allow.run({ families: ['mistral'], repo: h.repo })
  await h.writeText(join(h.repo, 'docs', 'adr', '0001-nightly-job.md'), ADR_ONE)
  await h.writeText(join(h.repo, 'docs', 'adr', '0002-window.md'), ADR_TWO)
  return h
}

async function prepared(h: Harness, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, stage: 'adr-review', ...extra })
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
  const text = await readFile(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl'), 'utf8')
  return text
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line) as Record<string, unknown>)
}

describe('an adr-review prepare on a repository with decision records', () => {
  it('is ready with the ADRs as the artifact and the glossary as a reference', async () => {
    const h = await withAdrs()

    const run = await prepared(h)

    expect(run.packet_files).toEqual(['artifact/docs/adr/0001-nightly-job.md', 'artifact/docs/adr/0002-window.md', 'references/CONTEXT.md'])
    expect(await readFile(join(run.packet_dir, 'artifact', 'docs', 'adr', '0001-nightly-job.md'), 'utf8')).toBe(ADR_ONE)
    expect(await readFile(join(run.packet_dir, 'references', 'CONTEXT.md'), 'utf8')).toContain(DECISION_CANARY)
    expect(run.artifact).toEqual({ labels: [], reference: 'adrs', tracker: 'none' })
  })
})

describe('the records an adr-review prepare freezes', () => {
  it('reviews the named records, with the glossary and the other records as references', async () => {
    const h = await withAdrs()

    const run = await prepared(h, { adrs: ['docs/adr/0002-window.md'] })

    expect(run.packet_files).toEqual(['artifact/docs/adr/0002-window.md', 'references/CONTEXT.md', 'references/docs/adr/0001-nightly-job.md'])
  })

  it('takes the references the caller names instead', async () => {
    const h = await withAdrs()
    await h.writeText(join(h.repo, 'docs', 'glossary.md'), '# Words\n')

    const run = await prepared(h, { references: ['docs/glossary.md'] })

    expect(run.packet_files).toEqual(['artifact/docs/adr/0001-nightly-job.md', 'artifact/docs/adr/0002-window.md', 'references/docs/glossary.md'])
  })

  it('sends no glossary when the repository has none', async () => {
    const h = await withAdrs()
    await h.removeFile(join(h.repo, 'CONTEXT.md'))

    const run = await prepared(h)

    expect(run.packet_files).toEqual(['artifact/docs/adr/0001-nightly-job.md', 'artifact/docs/adr/0002-window.md'])
  })

  it('reads no tracker, so a repository whose tracker the critique cannot read still runs', async () => {
    const h = await withAdrs({ tracker: 'gitlab' })

    expect((await prepared(h)).status).toBe('ready')
  })

  it('is refused when the repository has no decision records', async () => {
    const h = await harness()
    open.push(h)
    await h.commands.allow.run({ families: ['mistral'], repo: h.repo })

    const result = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, stage: 'adr-review' })

    expect(result).toEqual({ message: 'found no decision records in docs/adr/ to review', reason: 'no-adrs', status: 'refused' })
  })

  it('runs on documents alone while the repository allows no code', async () => {
    const h = await withAdrs()

    const run = await prepared(h)

    expect(run.code).toBeUndefined()
    expect(run.packet_files.every((path) => path.startsWith('artifact/') || path.startsWith('references/'))).toBe(true)
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
    expect([(await manifestOf(first)).flow, (await manifestOf(second)).flow]).toEqual([first.run_id, second.run_id])
    const spend = (await ledgerLines(h)).filter((line) => line.type === 'invocation').map((line) => [line.flow, line.run_id, line.charged_usd])
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
  const ADR_QUOTE = { file: 'artifact/docs/adr/0001-nightly-job.md', kind: 'quote', value: 'We frobnicate widgets in the nightly job only.' }
  const GLOSSARY_QUOTE = { file: 'references/CONTEXT.md', kind: 'quote', value: DECISION_CANARY }

  function critic(findings: readonly Record<string, unknown>[]): VibeScript {
    return { exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings })) }
  }

  function adrFinding(id: string): Record<string, unknown> {
    return finding({ evidence: [ADR_QUOTE, GLOSSARY_QUOTE], id, location: `docs/adr/0001-nightly-job.md ${id}` })
  }

  async function reviews(h: Harness, extra: Record<string, unknown> = {}): Promise<ReviewsResult> {
    const result = await h.commands.reviews.run({ repo: h.repo, ...extra })
    if (result.status !== 'read') throw new Error(`reviews refused: ${result.message}`)
    return result
  }

  it('lists each review run, newest first, with its stage, run id, verdict counts and spend', async () => {
    const h = await withAdrs({ scripts: [critic([adrFinding('F1'), adrFinding('F2'), adrFinding('F3')]), critic([])] })
    const first = await prepared(h)
    await h.commands.fanOut.run({ run_dir: first.run_dir })
    await h.commands.record.run({
      run_dir: first.run_dir,
      verdicts: [
        { finding_id: 'F1', verdict: 'accepted' },
        { finding_id: 'F2', reason: 'Which job?', verdict: 'needs-human' },
        { finding_id: 'F3', reason: 'The glossary says so.', reason_code: 'contradicted-by-docs', verdict: 'rejected' },
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

  it('reports an open critical question without naming a feature command not to start', async () => {
    const h = await withAdrs({ scripts: [critic([{ ...adrFinding('F1'), severity: 'critical' }])] })
    const run = await prepared(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', reason: 'Which job?', verdict: 'needs-human' }] })

    expect(recorded.summary).not.toContain('Do not start')
    expect(await readFile(join(run.run_dir, 'report.md'), 'utf8')).not.toContain('Do not start')
  })

  it('leaves out every run of a feature, and never calls a review due', async () => {
    const h = await withAdrs({ scripts: [critic([])] })
    await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })
    await h.commands.state.run({ repo: h.repo, spec: '.scratch/widgets/spec.md' })

    const result = await reviews(h)

    expect(result).toEqual({ runs: [], status: 'read' })
  })

  it('keeps a recorded run whose directory is gone, from the ledger', async () => {
    const h = await withAdrs({ scripts: [critic([adrFinding('F1')])] })
    const run = await prepared(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })
    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })
    await rm(run.run_dir, { force: true, recursive: true })

    const result = await reviews(h)

    expect(result.runs).toEqual([
      { recorded: true, round: 1, run_id: run.run_id, spent_usd: 0.0123, stage: 'adr-review', verdicts: { accepted: 1, hint: 0, 'needs-human': 0, rejected: 0 } },
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
    await new SyntheticLedger().run({ critics: [{ client: 'vibe-mistral-large' }], repository: 'github.com/jorisjonkers-dev/other', spec: 'adrs', stage: 'adr-review' }).writeTo(h)

    expect((await reviews(h)).runs).toEqual([])
  })

  it('is refused for a repository whose remotes refuse a critique', async () => {
    const h = await harness({ remotes: [] })
    open.push(h)

    expect(await h.commands.reviews.run({ repo: h.repo })).toMatchObject({ reason: 'no-remote', status: 'refused' })
  })
})

describe('the depth of an adr-review run', () => {
  async function withRecords(count: number, text = 'We decided.'): Promise<Harness> {
    const h = await harness()
    open.push(h)
    await h.commands.allow.run({ families: ['mistral'], repo: h.repo })
    for (let n = 1; n <= count; n += 1) {
      await h.writeText(join(h.repo, 'docs', 'adr', `${String(n).padStart(4, '0')}-record.md`), `# Record ${String(n)}\n\n${text}\n`)
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
    const run = await prepared(await withRecords(11), { adrs: ['docs/adr/0001-record.md', 'docs/adr/0002-record.md'] })

    expect(run.depth).toBe('base')
    expect(run.depth_choice.measures).toEqual({ adrs: { threshold: 10, value: 2 } })
  })

  it("goes deep on one of the spec stage's risk terms", async () => {
    const run = await prepared(await withRecords(1, 'We run the schema migration before the deploy.'))

    expect(run.depth).toBe('deep')
    expect(run.depth_choice.signals).toEqual([{ kind: 'risk-term', term: 'migration' }])
  })

  it('takes the depth the caller names over the signals', async () => {
    const run = await prepared(await withRecords(11), { depth: 'base' })

    expect(run.depth).toBe('base')
    expect(run.depth_choice.chosen_by).toBe('argument')
  })
})
