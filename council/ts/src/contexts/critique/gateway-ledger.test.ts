import { chmod } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  fenced,
  finding,
  harness,
  isolatedExport,
  seedGatewayLedger,
  SyntheticLedger,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import { critiqueLedgerRecord, gatewayLedgerLines, type CritiqueLedgerContext, type LedgerRun } from './domain/gateway-ledger.js'
import type { BudgetResult, PrepareResult, StatsResult } from './index.js'

/**
 * The one ledger: every critic run — the fan-out wave, the refute round, the
 * triage model's question — is appended to the model gateway's ledger as one
 * record carrying the gateway's fields beside critique's, and critique's
 * stats and `/flow budget` read the gateway's records and critique's own
 * together, so a run recorded before the move still counts.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const SPEC = '.scratch/widgets/spec.md'
const REPOSITORY = 'github.com/jorisjonkers-dev/agent-kit'

async function opened(options: Parameters<typeof harness>[0] = {}): Promise<Harness> {
  const h = await harness(options)
  open.push(h)
  // Vibe's families only, so no Antigravity critic joins the roster.
  await h.commands.allow.run({ code: true, families: ['mistral', 'zhipu'], repo: h.repo })
  return h
}

const vibeOk = (
  agent: string,
  cost = 0.0123,
  findings: readonly Record<string, unknown>[] = [finding()],
  usage?: Record<string, number>,
): VibeScript => ({
  agent,
  exitCode: 0,
  exportJson: { ...isolatedExport(), cost_usd: cost, ...(usage === undefined ? {} : { usage }) },
  stdout: vibeStdout(fenced({ findings })),
})

async function prepared(h: Harness, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: SPEC, stage: 'spec', ...extra })
  if (run.status !== 'ready') throw new Error(`not ready: ${run.message}`)
  return run
}

/** The gateway's ledger, line by line. */
async function gatewayRecords(h: Harness): Promise<Record<string, unknown>[]> {
  return (await h.gatewayLedger())
    .split('\n')
    .filter((line) => line.length > 0)
    .map((line) => JSON.parse(line) as Record<string, unknown>)
}

async function stats(h: Harness): Promise<StatsResult> {
  const result = await h.commands.stats.run({})
  if (result.status !== 'refused' && 'cells' in result) return result
  throw new Error('stats refused')
}

async function budgetOf(h: Harness): Promise<BudgetResult> {
  const result = await h.commands.budget.run({ repo: h.repo, spec: SPEC })
  if (result.status === 'refused') throw new Error(`budget refused: ${result.message}`)
  return result
}

describe("the gateway's ledger", () => {
  it('holds one record per critic run, with the gateway fields and critique\'s beside them', async () => {
    const h = await opened({ scripts: [vibeOk('critic-mistral-large'), vibeOk('critic-glm')] })
    const run = await prepared(h)

    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const record = (await gatewayRecords(h)).find((line) => line.critic_id === 'c1-vibe-mistral-large')
    expect(record).toMatchObject({
      at: '2026-10-30T12:00:00.000Z',
      charged_usd: 0.0123,
      client: 'vibe',
      cost_usd: 0.0123,
      critic_client: 'vibe-mistral-large',
      critic_id: 'c1-vibe-mistral-large',
      critic_model: 'mistral-large-4-0',
      family: 'mistral',
      finding_count: 1,
      kind: 'vibe',
      mode: 'read-only',
      model: 'mistral-large-4-0-3',
      outcome: 'ok',
      repository: REPOSITORY,
      role: 'spec-redteam',
      round: 1,
      run_id: run.run_id,
      stage: 'spec',
      status: 'ok',
      type: 'invocation',
      workflow: SPEC,
    })
    expect(record?.prompt_hashes).toMatchObject({ contract: expect.any(String) as unknown })
  })

  it('writes the token counts the client reported beside the dollars', async () => {
    const h = await opened({
      scripts: [
        vibeOk('critic-mistral-large', 0.0123, [finding()], { cached_input_tokens: 5, input_tokens: 100, output_tokens: 20 }),
        vibeOk('critic-glm'),
      ],
    })
    const run = await prepared(h)

    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const record = (await gatewayRecords(h)).find((line) => line.critic_id === 'c1-vibe-mistral-large')
    expect(record?.tokens).toEqual({ cached: 5, input: 100, output: 20 })
  })

  it('writes no token count for a run whose client reported none, which the gateway cannot know', async () => {
    const h = await opened({ scripts: [vibeOk('critic-mistral-large'), vibeOk('critic-glm')] })
    const run = await prepared(h)

    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const record = (await gatewayRecords(h)).find((line) => line.critic_id === 'c1-vibe-mistral-large')
    expect(record).not.toHaveProperty('tokens')
  })

  it("records the refute round's run too", async () => {
    const critical = finding({ severity: 'critical' })
    const h = await opened({
      scripts: [vibeOk('critic-mistral-large', 0.0123, [critical]), vibeOk('critic-glm', 0.0123, [])],
    })
    const run = await prepared(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })
    h.process.queue(vibeOk('critic-glm'))

    await h.commands.refute.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })

    const refuter = (await gatewayRecords(h)).find((record) => record.critic_id === 'r-F1-vibe-glm')
    expect(refuter).toMatchObject({ client: 'vibe', critic_client: 'vibe-glm', mode: 'read-only', stage: 'spec', workflow: SPEC })
  })

  it("records the triage model's question as spend only, without prompt hashes", async () => {
    const h = await opened({
      scripts: [{ agent: 'critic-mistral-small', exitCode: 0, exportJson: { ...isolatedExport(), cost_usd: 0.0021 }, stdout: vibeStdout('no') }],
    })
    await h.writeText(`${h.repo}/src/widget.ts`, 'export const widgets = 2\n')
    await h.git.commit()

    await h.commands.triage.run({ base: 'HEAD~1', repo: h.repo, ticket: SPEC })

    const record = (await gatewayRecords(h)).find((line) => line.critic_id === 'triage')
    expect(record).toMatchObject({ client: 'vibe', critic_client: 'vibe-mistral-small', critic_id: 'triage', stage: 'ticket-review' })
    expect(record?.prompt_hashes).toBeUndefined()
  })
})

describe('what critique reads back from both ledgers', () => {
  it("counts a run only the gateway's ledger holds", async () => {
    const writer = await opened({ scripts: [vibeOk('critic-mistral-large'), vibeOk('critic-glm')] })
    const run = await prepared(writer)
    await writer.commands.fanOut.run({ run_dir: run.run_dir })
    const reader = await opened()
    await reader.writeText(reader.gatewayLedgerFile, await writer.gatewayLedger())

    const result = await stats(reader)

    expect(result.cells.map((cell) => [cell.client, cell.stage, cell.runs])).toEqual([
      ['vibe-glm', 'spec', 1],
      ['vibe-mistral-large', 'spec', 1],
    ])
  })

  it("still counts a run critique's own ledger recorded before the move", async () => {
    const h = await opened()
    await new SyntheticLedger().run({ critics: [{ client: 'vibe-mistral-large' }] }).writeTo(h)

    const result = await stats(h)

    expect(result.cells.map((cell) => [cell.client, cell.runs])).toEqual([['vibe-mistral-large', 1]])
  })

  it('counts a run both ledgers hold once', async () => {
    const writer = await opened({ scripts: [vibeOk('critic-mistral-large'), vibeOk('critic-glm')] })
    const run = await prepared(writer)
    await writer.commands.fanOut.run({ run_dir: run.run_dir })
    const reader = await opened()
    await reader.writeText(reader.gatewayLedgerFile, await writer.gatewayLedger())
    // The same run, as critique's own ledger recorded it before the move.
    await new SyntheticLedger()
      .run({ critics: [{ client: 'vibe-mistral-large' }, { client: 'vibe-glm' }], run_id: run.run_id })
      .writeTo(reader)

    const result = await stats(reader)

    expect(result.cells.map((cell) => [cell.client, cell.runs])).toEqual([
      ['vibe-glm', 1],
      ['vibe-mistral-large', 1],
    ])
  })

  it("adds the gateway's records to the flow's spend", async () => {
    const h = await opened()
    await seedGatewayLedger(h, [
      {
        charged_usd: 1.25,
        critic_client: 'vibe-mistral-large',
        critic_id: 'c1-vibe-mistral-large',
        kind: 'vibe',
        recorded_at: '2026-10-30T10:00:00.000Z',
        repository: REPOSITORY,
        round: 1,
        run_id: '20261030T100000Z-spec',
        stage: 'spec',
        type: 'invocation',
        workflow: SPEC,
      },
    ])

    const result = await budgetOf(h)

    expect(result.vibe.spent_usd).toBe(1.25)
  })
})

describe('a gateway ledger that cannot be written', () => {
  it('completes the stage, keeps critique\'s own line, and says so in the summary', async () => {
    const h = await opened({ scripts: [vibeOk('critic-mistral-large'), vibeOk('critic-glm')] })
    const run = await prepared(h)
    // A ledger nobody may append to: every write to it fails, every read still works.
    await h.writeText(h.gatewayLedgerFile, '')
    await chmod(h.gatewayLedgerFile, 0o444)

    const fanOut = await h.commands.fanOut.run({ run_dir: run.run_dir })
    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })

    expect(await h.readText(fanOut.findings_path)).toContain('"status": "reviewed"')
    expect(recorded.summary).toContain('the gateway ledger could not be written: ')
    expect(recorded.summary).toContain('; this run is recorded in critique\'s own ledger only')
    expect(await h.readText(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl'))).toContain(
      '"critic_id":"c1-vibe-mistral-large"',
    )
  })

  it("says so when a refuter's run cannot be recorded", async () => {
    const critical = finding({ severity: 'critical' })
    const h = await opened({
      scripts: [vibeOk('critic-mistral-large', 0.0123, [critical]), vibeOk('critic-glm', 0.0123, [])],
    })
    const run = await prepared(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })
    await chmod(h.gatewayLedgerFile, 0o444)
    h.process.queue(vibeOk('critic-glm'))

    await h.commands.refute.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })
    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })

    expect(recorded.summary).toContain('the gateway ledger could not be written: ')
  })
})

describe('reading a gateway ledger a later version or a cut write left', () => {
  const AT = '2026-10-30T12:00:00.000Z'
  const CONTEXT: CritiqueLedgerContext = { flow: SPEC, repository: REPOSITORY, round: 1, run_id: '20261030T120000Z-spec', stage: 'spec' }
  const RUN: LedgerRun = {
    charged_usd: 0.0123,
    client: 'vibe-glm',
    cost_usd: null,
    critic_id: 'c1',
    exit_code: 0,
    family: 'zhipu',
    model: 'zai-glm-5-3',
    prompt_hashes: { contract: 'a', preamble: 'b', role: 'c', schema: 'd' },
    status: 'ok',
  }
  const line = (criticId: string): string => JSON.stringify(critiqueLedgerRecord(CONTEXT, { ...RUN, critic_id: criticId }, { kind: 'vibe' }, AT))

  it('skips a line it does not recognise and reads the rest', () => {
    const text = [line('c1'), '{"type":"invocation","critic_id":"c1"}', line('c2')].join('\n')

    expect(gatewayLedgerLines(text).runs.map((run) => run.critic_id)).toEqual(['c1', 'c2'])
  })

  it('reads back a record it wrote without a role, lenses or finding count', () => {
    const [run] = gatewayLedgerLines(line('c1')).runs

    expect(run).toMatchObject({ finding_count: 0, lens_hashes: {}, lenses: [], role: '' })
  })
})
