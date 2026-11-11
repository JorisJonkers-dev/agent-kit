import { chmod, readdir, readFile } from 'node:fs/promises'
import { join, relative } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  fenced,
  harness,
  isolatedExport,
  seedGatewayLedger,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import { coldReadPath, type ColdReadResult, type PrepareResult } from './index.js'

/**
 * The cold read (#476): after `/critique-adrs` rewrote records and
 * check-rewrite confirmed the decisions survived, a client that never saw the
 * review reads each rewritten record on its own and says what it decided,
 * what it rejected and why, and when to revisit.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const ADR_ONE =
  '# The nightly job frobnicates widgets\n\nWe frobnicate widgets in the nightly job only, because a widget frobnicated twice is counted twice. We rejected frobnicating on read, since a read would then cost a write. Revisit this when a widget can be read by more than one job.\n'
const ADR_TWO = '# Widgets keep a seven-second window\n\nThe window is seven seconds. We rejected a longer window because the queue drains in seven. Revisit this when the queue grows.\n'

/** The rewritten first record: the same decision, in the owner's plainer words. */
const REWRITTEN_ONE =
  '# The nightly job frobnicates widgets\n\nOnly the nightly job frobnicates a widget, so no widget is counted twice. Frobnicating on read was rejected: a read would then cost a write. Revisit this when more than one job may read a widget.\n'

/** This machine's gateway file: Vibe's plan allows $5 a day. */
const VIBE_PLAN = { clients: { vibe: { plan_limits: [{ limit_usd: 5, window: 'day' }] } } }

const slot = (client: string) => ({ client, lenses: ['adr-record'], max_price_usd: 0.5, role: 'adr-reviewer', time_limit_s: 600 })

/** One adr-review critic, and the Mistral Small client the cold read runs through. */
const ROUTING = {
  clients: {
    'agy-gemini-flash-high': { binary: 'agy', family: 'google', kind: 'agy', model: 'gemini-3.8-flash-high' },
    'vibe-mistral-large': { agent: 'critic-mistral-large', binary: 'vibe', family: 'mistral', kind: 'vibe', model: 'mistral-large-4-0' },
    'vibe-mistral-small': { agent: 'critic-mistral-small', binary: 'vibe', family: 'mistral', kind: 'vibe', model: 'mistral-small-latest' },
  },
  cold_read: { client: 'vibe-mistral-small', max_price_usd: 0.03, time_limit_s: 60 },
  stages: { 'adr-review': { base: [slot('vibe-mistral-large')], distinct_families: true } },
  version: 1,
}

const ANSWERS = {
  decided: 'Widgets are frobnicated by the nightly job only.',
  rejected: 'Frobnicating on read, because a read would then cost a write.',
  revisit: 'When a widget can be read by more than one job.',
}

/** The cold reader's reply: its three answers in the fenced block the role asks for. */
function answer(value: unknown, overrides: Partial<VibeScript> = {}): VibeScript {
  return {
    agent: 'critic-mistral-small',
    exitCode: 0,
    exportJson: { ...isolatedExport(), cost_usd: 0.0021 },
    stdout: vibeStdout(fenced(value)),
    ...overrides,
  }
}

interface Reviewed {
  readonly h: Harness
  readonly run: PrepareResult
}

/** An adr-review run over two records, prepared and not yet fanned out. */
async function reviewed(options: Parameters<typeof harness>[0] = {}): Promise<Reviewed> {
  const h = await harness({ routing: ROUTING, ...options })
  open.push(h)
  await h.commands.allow.run({ families: ['mistral'], repo: h.repo })
  await h.writeText(join(h.repo, 'docs', 'adr', '0001-nightly-job.md'), ADR_ONE)
  await h.writeText(join(h.repo, 'docs', 'adr', '0002-window.md'), ADR_TWO)
  const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, stage: 'adr-review' })
  if (run.status !== 'ready') throw new Error(`not ready: ${JSON.stringify(run)}`)
  return { h, run }
}

async function coldRead(r: Reviewed, files: readonly string[] = ['docs/adr/0001-nightly-job.md']): Promise<ColdReadResult> {
  return r.h.commands.coldRead.run({ files, run_dir: r.run.run_dir })
}

function ledgerLines(h: Harness): Promise<Record<string, unknown>[]> {
  return readFile(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl'), 'utf8').then((text) =>
    text
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line) as Record<string, unknown>),
  )
}

/** Every file under a directory, at any depth, relative to it. */
function tree(dir: string): Promise<string[]> {
  return readdir(dir, { recursive: true, withFileTypes: true }).then((entries) =>
    entries.filter((entry) => entry.isFile()).map((entry) => relative(dir, join(entry.parentPath, entry.name))).sort(),
  )
}

/** This machine's routing override, written after the run was prepared, so the run itself is unaffected. */
async function override(h: Harness, value: unknown): Promise<void> {
  await h.writeText(join(h.home, '.config', 'agent-kit', 'critique-routing.json'), JSON.stringify(value))
}

describe('cold-read: the reader answers', () => {
  it('reports what was decided, what was rejected and when to revisit', async () => {
    const r = await reviewed()
    r.h.process.queue(answer(ANSWERS))

    const result = await coldRead(r)

    expect(result).toEqual({
      files: [{ answers: ANSWERS, missing: [], path: 'docs/adr/0001-nightly-job.md', status: 'answered' }],
      run_id: r.run.run_id,
      status: 'checked',
    })
  })

  it('lists an empty answer as missing, so an omission is never a pass', async () => {
    const r = await reviewed()
    r.h.process.queue(answer({ ...ANSWERS, rejected: '' }))

    const result = await coldRead(r)

    expect(result.files[0]).toMatchObject({ missing: ['rejected'], status: 'answered' })
    expect(result.files[0]?.answers?.rejected).toBe('')
  })

  it('lists a key the reader left out as missing too', async () => {
    const r = await reviewed()
    r.h.process.queue(answer({ decided: ANSWERS.decided }))

    const result = await coldRead(r)

    expect(result.files[0]).toMatchObject({ missing: ['rejected', 'revisit'], status: 'answered' })
  })

  it('cold-reads each record on its own, in the order the caller named them', async () => {
    const r = await reviewed()
    r.h.process.queue(answer(ANSWERS), answer({ ...ANSWERS, decided: 'The window is seven seconds.' }))

    const result = await coldRead(r, ['docs/adr/0001-nightly-job.md', 'docs/adr/0002-window.md'])

    expect(result.files.map((file) => [file.path, file.answers?.decided])).toEqual([
      ['docs/adr/0001-nightly-job.md', ANSWERS.decided],
      ['docs/adr/0002-window.md', 'The window is seven seconds.'],
    ])
    expect(r.h.process.commands).toHaveLength(2)
    expect(await tree(join(r.run.run_dir, 'cold-read', '2', 'packet'))).toEqual(['artifact/0002-window.md'])
  })
})

describe('cold-read: the packet', () => {
  it('holds only the rewritten record, and the prompt names only it', async () => {
    const r = await reviewed()
    await r.h.writeText(join(r.h.repo, 'docs', 'adr', '0001-nightly-job.md'), REWRITTEN_ONE)
    r.h.process.queue(answer(ANSWERS))

    await coldRead(r)

    const dir = join(r.run.run_dir, 'cold-read', '1')
    expect(await tree(join(dir, 'packet'))).toEqual(['artifact/0001-nightly-job.md'])
    expect(await r.h.readText(join(dir, 'packet', 'artifact', '0001-nightly-job.md'))).toBe(REWRITTEN_ONE)
    const prompt = await r.h.readText(join(dir, 'prompt.md'))
    expect(prompt).toContain('# Role: cold-reader')
    expect(prompt).toContain('- `artifact/0001-nightly-job.md`: the decision record you answer about')
    expect(prompt).not.toContain('CONTEXT.md')
    expect(prompt).not.toContain('0002-window.md')
    expect(r.h.process.commands[0]?.cwd).toBe(join(dir, 'packet'))
  })
})

describe('cold-read: unavailable', () => {
  it.each([
    ['no json block', 'I could not read the record.'],
    ['a block that is not JSON', '```json\n{ decided: widgets }\n```'],
    ['a block that is not an object', '```json\n["widgets"]\n```'],
    ['an answer that is not a string', '```json\n{ "decided": 7 }\n```'],
  ])('reports the record unavailable on %s, and still logs the launch', async (_name, text) => {
    const r = await reviewed()
    r.h.process.queue(answer(undefined, { stdout: vibeStdout(text) }))

    const result = await coldRead(r)

    expect(result.files[0]).toEqual({
      missing: ['decided', 'rejected', 'revisit'],
      path: 'docs/adr/0001-nightly-job.md',
      reason: 'the cold reader gave no readable answer',
      status: 'unavailable',
    })
    expect((await ledgerLines(r.h)).find((line) => line.type === 'invocation')).toMatchObject({ charged_usd: 0.0021, status: 'failed' })
  })

  it('reports the record unavailable when the client fails, with the failure', async () => {
    const r = await reviewed()
    r.h.process.queue(answer(ANSWERS, { exitCode: 1, stderr: 'Error: no API key\n' }))

    const result = await coldRead(r)

    expect(result.files[0]).toMatchObject({
      reason: 'config-error: vibe exited 1: outcome finished',
      status: 'unavailable',
    })
    expect(result.files[0]?.answers).toBeUndefined()
  })

  it('reports every record unavailable when the client is not ready, and launches nothing', async () => {
    const r = await reviewed({ vibeOnPath: false })

    const result = await coldRead(r, ['docs/adr/0001-nightly-job.md', 'docs/adr/0002-window.md'])

    expect(result.files.map((file) => [file.status, file.reason])).toEqual([
      ['unavailable', 'vibe: vibe is not on PATH'],
      ['unavailable', 'vibe: vibe is not on PATH'],
    ])
    expect(r.h.process.commands).toEqual([])
  })

  it("reports every record unavailable when the allowlist does not permit the reader's family", async () => {
    const h = await harness({ routing: ROUTING })
    open.push(h)
    await h.commands.allow.run({ families: ['zhipu'], repo: h.repo })
    await h.writeText(join(h.repo, 'docs', 'adr', '0001-nightly-job.md'), ADR_ONE)
    const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, stage: 'adr-review' })
    if (run.status !== 'ready') throw new Error('not ready')

    const result = await coldRead({ h, run })

    expect(result.files[0]).toMatchObject({ reason: 'the allowlist does not permit the mistral family', status: 'unavailable' })
    expect(h.process.commands).toEqual([])
  })

  it('reports every record unavailable once Vibe is at 100% of its plan', async () => {
    const r = await reviewed({ gatewayOverride: VIBE_PLAN })
    await seedGatewayLedger(r.h, [{ cost_usd: 5 }])

    const result = await coldRead(r)

    expect(result.files[0]).toMatchObject({ status: 'unavailable' })
    expect(result.files[0]?.reason).toContain('at 100% of its day plan limit')
    expect(r.h.process.commands).toEqual([])
  })
})

describe('cold-read: spend', () => {
  it('launches Mistral Small under the $0.03 cap and a one-minute limit', async () => {
    const r = await reviewed()
    r.h.process.queue(answer(ANSWERS))

    await coldRead(r)

    const [launch] = r.h.process.commands
    const arg = (flag: string) => launch?.args[launch.args.indexOf(flag) + 1]
    expect([arg('--agent'), arg('--max-price'), arg('--time-limit'), arg('--enabled-tools')]).toEqual([
      'critic-mistral-small',
      '0.03',
      '60',
      'read_file',
    ])
  })

  it("logs each launch to both ledgers, so its spend counts in the run's flow", async () => {
    const r = await reviewed()
    r.h.process.queue(answer(ANSWERS))

    await coldRead(r)

    const invocation = (await ledgerLines(r.h)).find((line) => line.type === 'invocation')
    expect(invocation).toMatchObject({
      charged_usd: 0.0021,
      client: 'vibe-mistral-small',
      critic_id: 'cold-read-1',
      family: 'mistral',
      flow: r.run.run_id,
      kind: 'vibe',
      model: 'mistral-small-latest',
      run_id: r.run.run_id,
      status: 'ok',
    })
    const gateway = (await r.h.gatewayLedger())
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line) as Record<string, unknown>)
    expect(gateway).toEqual([
      expect.objectContaining({ cost_usd: 0.0021, critic_id: 'cold-read-1', role: 'cold-reader', workflow: r.run.run_id }),
    ])
  })
})

describe('cold-read: once', () => {
  it('writes its result beside the run, and refuses a second cold read', async () => {
    const r = await reviewed()
    r.h.process.queue(answer(ANSWERS))

    const result = await coldRead(r)

    expect(coldReadPath(r.run.run_dir)).toBe(join(r.run.run_dir, 'cold-read.json'))
    expect(JSON.parse(await readFile(coldReadPath(r.run.run_dir), 'utf8'))).toEqual(JSON.parse(JSON.stringify(result)))
    await expect(coldRead(r)).rejects.toMatchObject({ code: 'already-cold-read' })
  })
})

describe('cold-read: input', () => {
  it('refuses a record that is not in the repository', async () => {
    const r = await reviewed()

    await expect(coldRead(r, ['../elsewhere.md'])).rejects.toMatchObject({ code: 'outside-repository' })
  })

  it('refuses a record the working tree does not hold', async () => {
    const r = await reviewed()

    await expect(coldRead(r, ['docs/adr/0009-missing.md'])).rejects.toMatchObject({
      code: 'invalid-input',
      message: 'cold-read input: docs/adr/0009-missing.md does not exist in the repository',
    })
  })

  it('refuses routing without a cold_read section', async () => {
    const r = await reviewed()
    await override(r.h, { cold_read: null })

    await expect(coldRead(r)).rejects.toMatchObject({
      code: 'invalid-routing',
      message: 'routing has no cold_read, so a rewrite cannot be cold-read',
    })
  })

  it('refuses a cold_read client that is not a Vibe client', async () => {
    const r = await reviewed()
    await override(r.h, { cold_read: { client: 'agy-gemini-flash-high' } })

    await expect(coldRead(r)).rejects.toMatchObject({
      code: 'invalid-routing',
      message: expect.stringContaining('cold_read.client must name a vibe client of clients; agy-gemini-flash-high is an agy client') as unknown,
    })
  })
})

describe('cold-read: failures that must not lose the run', () => {
  it('reports the record unavailable when its launch throws, and keeps the records already read', async () => {
    const r = await reviewed()
    r.h.process.queue(answer(ANSWERS), answer(ANSWERS, { effect: () => Promise.reject(new Error('spawn crash')) }))

    const result = await coldRead(r, ['docs/adr/0001-nightly-job.md', 'docs/adr/0002-window.md'])

    expect(result.files.map((file) => file.status)).toEqual(['answered', 'unavailable'])
    expect(result.files[1]?.reason).toContain('the launch failed: ')
    expect(result.files[1]?.reason).toContain('spawn crash')
  })

  it("says so when the gateway's ledger cannot take a launch's record", async () => {
    const r = await reviewed()
    r.h.process.queue(answer(ANSWERS))
    await r.h.writeText(r.h.gatewayLedgerFile, '')
    await chmod(r.h.gatewayLedgerFile, 0o444)

    const result = await coldRead(r)

    expect(result.files[0]?.status).toBe('answered')
    expect(result.ledger_warnings).toHaveLength(1)
  })

  it('refuses an empty list of records', async () => {
    const r = await reviewed()

    await expect(coldRead(r, [])).rejects.toMatchObject({ code: 'invalid-input' })
  })
})
