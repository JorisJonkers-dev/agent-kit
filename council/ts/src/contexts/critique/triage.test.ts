import { readdir, readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  ASSETS_ROOT,
  codeStageRouting,
  harness,
  isolatedExport,
  ticketIssue,
  twoCriticRouting,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import type { PrepareResult, Refusal, TriageResult } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const TICKET = '.scratch/widgets/issues/01-frobnicate.md'
const AUTH_GLOB = '**/{auth,authn,authz,oauth,oidc,sso,login,session,sessions,permissions,rbac,acl}/**'
/** A small change no risk rule fires on. */
const HARMLESS = { 'src/widget.ts': 'export const widgets = 2\n' }

/**
 * A repository whose last commit is the change under review: `files` are
 * written on top of a first commit that holds the ticket and a widget, so
 * `HEAD~1` is the change's base.
 */
async function changed(files: Readonly<Record<string, string>>, options: Parameters<typeof harness>[0] = {}): Promise<Harness> {
  const h = await harness(options)
  open.push(h)
  await h.writeText(join(h.repo, TICKET), '# 01: Frobnicate widgets\n\n## Acceptance criteria\n\n- [ ] Frobnicate counts every widget.\n')
  await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widgets = 1\n')
  await h.git.commit()
  for (const [path, text] of Object.entries(files)) await h.writeText(join(h.repo, path), text)
  await h.git.commit()
  return h
}

/** A repository opted in for code to the families named, with a harmless change. */
async function optedIn(
  scripts: VibeScript[],
  allow: Record<string, unknown> = {},
  options: Parameters<typeof harness>[0] = {},
): Promise<Harness> {
  const h = await changed(HARMLESS, { scripts, ...options })
  await h.commands.allow.run({ code: true, families: ['mistral'], repo: h.repo, ...allow })
  return h
}

function answers(text: string, overrides: Partial<VibeScript> = {}): VibeScript {
  return {
    agent: 'critic-mistral-small',
    exitCode: 0,
    exportJson: { ...isolatedExport({ active_model: 'critic-mistral-small' }), cost_usd: 0.0021 },
    stdout: vibeStdout(text),
    ...overrides,
  }
}

async function triage(h: Harness, input: Record<string, unknown> = {}): Promise<TriageResult | Refusal> {
  return h.commands.triage.run({ base: 'HEAD~1', repo: h.repo, ticket: TICKET, ...input })
}

async function triaged(h: Harness, input: Record<string, unknown> = {}): Promise<TriageResult> {
  const result = await triage(h, input)
  if (result.status === 'refused') throw new Error(`triage refused: ${result.message}`)
  return result
}

async function ledger(h: Harness): Promise<readonly Record<string, unknown>[]> {
  const text = await readFile(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl'), 'utf8')
  return text
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line) as Record<string, unknown>)
}

/** The ledger lines triage wrote, without fan-out's invocation lines. */
async function triageLines(h: Harness): Promise<readonly Record<string, unknown>[]> {
  return (await ledger(h)).filter((line) => line.type !== 'invocation')
}

function lines(count: number): string {
  return Array.from({ length: count }, (_, i) => `export const line${String(i)} = ${String(i)}`).join('\n') + '\n'
}

describe('risk triage from the risk rules', () => {
  it('calls a change risky when it touches a risk path, naming the path and the glob', async () => {
    const h = await changed({ 'src/auth/session.ts': 'export const ttl = 60\n' })

    const result = await triaged(h)

    expect(result.status).toBe('risky')
    expect(result.signal).toEqual({ glob: AUTH_GLOB, kind: 'risk-path', path: 'src/auth/session.ts' })
    expect(result.summary).toBe(`Triage: risky: src/auth/session.ts matches the risk path ${AUTH_GLOB}.`)
  })

  it('counts a deleted file in a risk path as touching it', async () => {
    const h = await harness()
    open.push(h)
    await h.writeText(join(h.repo, TICKET), '# 01: Drop the old migration\n')
    await h.writeText(join(h.repo, 'db/migrations/001_init.sql'), 'create table widgets (id int);\n')
    await h.git.commit()
    await h.removeFile(join(h.repo, 'db/migrations/001_init.sql'))
    await h.git.commit()

    const result = await triaged(h)

    expect(result.signal).toMatchObject({ glob: '**/{migrations,migration,migrate}/**', path: 'db/migrations/001_init.sql' })
  })

  it('calls a change risky when it changes more lines than the size threshold', async () => {
    const h = await changed({ 'src/report.ts': lines(301) })

    const result = await triaged(h)

    expect(result.signal).toEqual({ changed_lines: 301, kind: 'size', threshold: 300 })
    expect(result.change).toMatchObject({ changed_files: 1, changed_lines: 301, changed_lines_threshold: 300 })
    expect(result.summary).toBe('Triage: risky: 301 changed lines, above the threshold of 300.')
  })

  it('does not call a change risky by size at exactly the threshold', async () => {
    const h = await changed({ 'src/report.ts': lines(300) })

    const result = await triaged(h)

    expect(result.signals).toEqual([])
    expect(result.signal.kind).toBe('model-unavailable')
  })

  it('counts added and removed lines, and no diff header, as changed lines', async () => {
    const h = await changed({ 'src/widget.ts': '-- a line that reads like a diff header\n' })

    const result = await triaged(h)

    expect(result.change.changed_lines).toBe(2)
  })

  it('counts only the lines of files that are not generated, vendored or lock files', async () => {
    const bundle = `#!/usr/bin/env node\n// Generated by scripts/build.mjs. Do not edit.\n${lines(698)}`
    const h = await changed({
      'uv.lock': lines(50),
      'skills/critique/critique.mjs': bundle,
      'src/report.ts': lines(200),
      'vendor/lib/pad.ts': lines(40),
    })

    const result = await triaged(h)

    expect(result.signals).toEqual([])
    expect(result.change).toMatchObject({ changed_files: 4, changed_lines: 200 })
  })

  it('calls a 200-line change beside a 700-line bundle rebuild low-risk on size, so the model decides', async () => {
    const h = await changed(
      { 'skills/critique/critique.mjs': `// Generated by scripts/build.mjs. Do not edit.\n${lines(699)}`, 'src/report.ts': lines(200) },
      { scripts: [answers('no')] },
    )
    await h.commands.allow.run({ code: true, families: ['mistral'], repo: h.repo })

    const result = await triaged(h)

    expect(result.status).toBe('low-risk')
    expect(result.signal).toEqual({ answer: 'no', kind: 'model' })
    const diff = await readFile(join(h.repo, '.git', 'critique', result.run_id, 'packet', 'diff', 'change.diff'), 'utf8')
    expect(diff).not.toContain('Do not edit')
    expect(diff).toContain('+export const line199 = 199\n')
    expect(diff).toContain('skills/critique/critique.mjs')
  })

  it('calls a change risky when its ticket carries risk:high', async () => {
    const h = await changed(HARMLESS, {
      issues: { 'github.com/jorisjonkers-dev/agent-kit#190': { ...ticketIssue(190, 'Frobnicate', 'Body.'), labels: ['type:feature', 'Risk:High'] } },
      tracker: 'github',
    })

    const result = await triaged(h, { ticket: '#190' })

    expect(result.signal).toEqual({ kind: 'label', label: 'risk:high' })
    expect(result.ticket).toEqual({ labels: ['type:feature', 'Risk:High'], reference: 'github.com/jorisjonkers-dev/agent-kit#190' })
    expect(result.summary).toBe('Triage: risky: the ticket carries the label risk:high.')
  })

  it('lists every rule that fired, the deciding one first', async () => {
    const h = await changed({ 'infra/main.tf': lines(400), 'src/auth/session.ts': 'x\n' })

    const result = await triaged(h)

    expect(result.signals.map((signal) => signal.kind)).toEqual(['risk-path', 'risk-path', 'size'])
    expect(result.signal).toEqual(result.signals[0])
  })

  it('decides from the rules without any allowlist entry, model launch or readiness check', async () => {
    const h = await changed({ 'src/auth/session.ts': 'x\n' })

    await triaged(h)

    expect(h.process.commands).toEqual([])
    expect(h.process.checks).toEqual([])
  })

  it('logs a risky decision with the glob that fired but not the path', async () => {
    const h = await changed({ 'src/auth/session.ts': 'x\n' })

    const result = await triaged(h)

    expect(await triageLines(h)).toEqual([
      {
        answer: null,
        changed_files: 1,
        changed_lines: 1,
        changed_lines_threshold: 300,
        charged_usd: 0,
        client: null,
        cost_usd: null,
        flow: TICKET,
        model: null,
        prompt_hash: null,
        recorded_at: '2026-10-30T12:00:00.000Z',
        repository: 'github.com/jorisjonkers-dev/agent-kit',
        risk_signals: [{ glob: AUTH_GLOB, kind: 'risk-path' }],
        run_id: result.run_id,
        stage: 'ticket-review',
        status: 'risky',
        ticket: TICKET,
        triage_signal: { glob: AUTH_GLOB, kind: 'risk-path' },
        type: 'triage',
      },
    ])
  })

  it('gives each triage in the same second its own run id', async () => {
    const h = await changed({ 'src/auth/session.ts': 'x\n' })

    const first = await triaged(h)
    const second = await triaged(h)

    expect([first.run_id, second.run_id]).toEqual(['20261030T120000Z-triage', '20261030T120000Z-triage-2'])
  })
})

describe('the risk rules in routing', () => {
  it('applies this machine override to the size threshold', async () => {
    const h = await changed(HARMLESS, { routingOverride: { triage: { changed_lines_threshold: 1 } } })

    const result = await triaged(h)

    expect(result.signal).toEqual({ changed_lines: 2, kind: 'size', threshold: 1 })
  })

  it('applies this machine override to the risk paths', async () => {
    const h = await changed(HARMLESS, { routingOverride: { triage: { risk_paths: ['src/widget.ts'] } } })

    const result = await triaged(h)

    expect(result.signal).toEqual({ glob: 'src/widget.ts', kind: 'risk-path', path: 'src/widget.ts' })
  })

  it.each([
    ['**/auth/**', 'auth/login.ts', true],
    ['**/auth/**', 'src/auth/deep/login.ts', true],
    ['**/auth/**', 'src/author/login.ts', false],
    ['src/*.ts', 'src/widget.ts', true],
    ['src/*.ts', 'src/a/widget.ts', false],
    ['src/widget.?s', 'src/widget.ts', true],
    ['src**.ts', 'src/a/b/widget.ts', true],
    ['**/*.{tf,sql}', 'db/schema.sql', true],
    ['**/*.{tf,sql}', 'db/schema.sqlite', false],
    ['src/{widget}.ts}', 'src/widget.ts}', true],
    ['**/Dockerfile*', 'Dockerfile.dev', true],
    ['.github/workflows/**', 'github/workflows/ci.yml', false],
  ])('treats the glob %s as matching %s: %s', async (glob, path, matches) => {
    const h = await changed({ [path]: 'x\n' }, { routingOverride: { triage: { changed_lines_threshold: 1000, risk_paths: [glob] } } })

    const result = await triaged(h)

    expect(result.signals.some((signal) => signal.kind === 'risk-path' && signal.path === path)).toBe(matches)
  })

  it('refuses routing that has no triage', async () => {
    const h = await changed(HARMLESS, { routing: twoCriticRouting() })

    const failing = triage(h)

    await expect(failing).rejects.toMatchObject({ code: 'invalid-routing' })
    await expect(failing).rejects.toThrow(/routing has no triage/)
  })

  it('refuses a triage model that is not a Vibe client', async () => {
    const h = await changed(HARMLESS, { routingOverride: { triage: { model: { client: 'agy-gemini-flash-high' } } } })

    const failing = triage(h)

    await expect(failing).rejects.toMatchObject({ code: 'invalid-routing' })
    await expect(failing).rejects.toThrow('triage.model.client must name a vibe client of clients; agy-gemini-flash-high is an agy client')
  })

  it('refuses a triage model that names no client', async () => {
    const h = await changed(HARMLESS, { routingOverride: { triage: { model: { client: 'vibe-nobody' } } } })

    await expect(triage(h)).rejects.toThrow('vibe-nobody is not defined')
  })
})

describe('the triage model', () => {
  it('skips the review of a change the model calls low-risk, and logs the skip as a run', async () => {
    const h = await optedIn([answers('no')])

    const result = await triaged(h)

    expect(result.status).toBe('low-risk')
    expect(result.signal).toEqual({ answer: 'no', kind: 'model' })
    expect(result.model).toEqual({ answer: 'no', charged_usd: 0.0021, client: 'vibe-mistral-small', cost_usd: 0.0021, model: 'mistral-small-latest' })
    expect(result.summary).toBe('Triage: low-risk: no risk rule fired, and the triage model (vibe-mistral-small) answered no, so the review is skipped.')
    expect(await triageLines(h)).toEqual([
      expect.objectContaining({
        answer: 'no',
        client: 'vibe-mistral-small',
        model: 'mistral-small-latest',
        stage: 'ticket-review',
        status: 'skipped-low-risk',
        triage_signal: { answer: 'no', kind: 'model' },
        type: 'run',
      }),
    ])
  })

  it('launches Mistral Small under a $0.03 cap and a one-minute limit', async () => {
    const h = await optedIn([answers('no')])

    await triaged(h)

    const [launch] = h.process.commands
    const arg = (flag: string) => launch?.args[launch.args.indexOf(flag) + 1]
    expect([arg('--agent'), arg('--max-price'), arg('--time-limit'), arg('--enabled-tools')]).toEqual([
      'critic-mistral-small',
      '0.03',
      '60',
      'read_file',
    ])
    expect(launch?.args.slice(0, 4)).toEqual(['-u', 'MISTRAL_API_KEY', '-u', 'VIBE_HOME'])
  })

  it('asks the model in a working directory holding only the diff, with the triage prompt', async () => {
    const h = await optedIn([answers('no')])

    const result = await triaged(h)

    const runDir = join(h.repo, '.git', 'critique', result.run_id)
    const [launch] = h.process.commands
    expect(launch?.cwd).toBe(join(runDir, 'packet'))
    expect(await readdir(join(runDir, 'packet'), { recursive: true })).toEqual(['diff', join('diff', 'change.diff')])
    expect(await readFile(join(runDir, 'packet', 'diff', 'change.diff'), 'utf8')).toContain('+export const widgets = 2')
    const prompt = await readFile(join(runDir, 'prompt.md'), 'utf8')
    expect(prompt).toContain((await readFile(join(ASSETS_ROOT, 'prompts', 'triage.md'), 'utf8')).trim())
    expect(prompt).toContain('- `diff/change.diff`: the change under review')
    expect(launch?.args[launch.args.indexOf('--prompt-file') + 1]).toBe(join(runDir, 'prompt.md'))
  })

  it('logs the prompt hash and charges the launch to the flow it names', async () => {
    const flow = 'github.com/jorisjonkers-dev/agent-kit#181'
    const h = await optedIn([answers('no')])

    await triaged(h, { flow })

    const [invocation] = (await ledger(h)).filter((line) => line.type === 'invocation')
    expect(invocation).toMatchObject({ charged_usd: 0.0021, client: 'vibe-mistral-small', critic_id: 'triage', flow, kind: 'vibe', status: 'ok' })
    const [line] = await triageLines(h)
    expect(line?.flow).toBe(flow)
    expect(line?.prompt_hash).toMatch(/^[0-9a-f]{64}$/)
  })

  it('calls a change risky when the model answers yes', async () => {
    const h = await optedIn([answers('**Yes.**')])

    const result = await triaged(h)

    expect(result.status).toBe('risky')
    expect(result.signal).toEqual({ answer: 'yes', kind: 'model' })
    expect(result.summary).toBe('Triage: risky: no risk rule fired, and the triage model (vibe-mistral-small) answered yes.')
    expect((await triageLines(h))[0]).toMatchObject({ answer: 'yes', status: 'risky', type: 'triage' })
  })

  it('calls a change risky when the model answers neither yes nor no', async () => {
    const h = await optedIn([answers('Probably not, but the cache key changed.')])

    const result = await triaged(h)

    expect(result.status).toBe('risky')
    expect(result.signal).toEqual({ detail: 'the triage model answered neither yes nor no', kind: 'model-unavailable', reason: 'unclear-answer' })
    expect(result.model).not.toHaveProperty('answer')
  })

  it('calls a change risky when the model run fails, and charges what it reported', async () => {
    const h = await optedIn([answers('', { exitCode: 3, exportJson: { ...isolatedExport(), cost_usd: 0.01, outcome: 'max_price' } })])

    const result = await triaged(h)

    expect(result.signal).toEqual({ detail: 'stopped-early: vibe exited 3: outcome max_price', kind: 'model-unavailable', reason: 'failed' })
    expect(result.summary).toBe(
      'Triage: risky: no risk rule fired, and the triage model (vibe-mistral-small) could not answer: stopped-early: vibe exited 3: outcome max_price.',
    )
    expect((await ledger(h)).find((line) => line.type === 'invocation')).toMatchObject({ charged_usd: 0.01, status: 'failed' })
  })

  it('calls a change risky when the model run has an unsafe config', async () => {
    const h = await optedIn([answers('no', { exportJson: isolatedExport({ enable_mcp: true }) })])

    const result = await triaged(h)

    expect(result.signal).toMatchObject({ kind: 'model-unavailable', reason: 'failed' })
  })
})

describe('when the triage model may not run', () => {
  it('calls a change risky in a repository with no allowlist entry, and asks nothing', async () => {
    const h = await changed(HARMLESS, { scripts: [answers('no')] })

    const result = await triaged(h)

    expect(result.signal).toEqual({
      detail: 'github.com/jorisjonkers-dev/agent-kit has no family allowlist entry on this machine',
      kind: 'model-unavailable',
      reason: 'not-opted-in',
    })
    expect(h.process.commands).toEqual([])
  })

  it('calls a change risky while the entry does not allow code', async () => {
    const h = await optedIn([answers('no')], { code: false })

    const result = await triaged(h)

    expect(result.signal).toMatchObject({ kind: 'model-unavailable', reason: 'code-not-allowed' })
    expect(result.summary).toBe(
      'Triage: risky: no risk rule fired, and the triage model (vibe-mistral-small) could not answer: github.com/jorisjonkers-dev/agent-kit allows documents but not code.',
    )
    expect(h.process.commands).toEqual([])
  })

  it('calls a change risky when the entry does not allow the Mistral family', async () => {
    const h = await optedIn([answers('no')], { families: ['google'] })

    const result = await triaged(h)

    expect(result.signal).toMatchObject({ kind: 'model-unavailable', reason: 'family-not-allowed' })
    expect(h.process.commands).toEqual([])
  })

  it('calls a change risky once the flow has spent its Vibe cap', async () => {
    const flow = 'github.com/jorisjonkers-dev/agent-kit#181'
    const h = await optedIn([answers('no')])
    const spent = { charged_usd: 5, flow, kind: 'vibe', recorded_at: '2026-10-30T11:00:00.000Z', repository: 'github.com/jorisjonkers-dev/agent-kit', type: 'invocation' }
    await h.writeText(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl'), `${JSON.stringify(spent)}\n`)

    const result = await triaged(h, { flow })

    expect(result.signal).toEqual({
      detail: "the flow's Vibe spend $5.0000 has reached its $5.00 cap",
      kind: 'model-unavailable',
      reason: 'budget',
    })
    expect(h.process.commands).toEqual([])
  })

  it('calls a change risky when the Mistral Small agent is not installed', async () => {
    const h = await optedIn([answers('no')], {}, { vibeAgents: ['critic-mistral-large'] })

    const result = await triaged(h)

    expect(result.signal).toMatchObject({
      detail: 'vibe: the critic agent critic-mistral-small is not installed in ~/.vibe/agents; re-run setup',
      reason: 'unavailable',
    })
    expect(h.process.commands).toEqual([])
    expect((await triageLines(h))[0]).toMatchObject({ triage_signal: { kind: 'model-unavailable', reason: 'unavailable' } })
  })
})

describe('what triage refuses', () => {
  it('refuses a repository with a nedap remote before reading anything', async () => {
    const h = await changed({ 'src/auth/session.ts': 'x\n' }, { remotes: [{ name: 'origin', url: 'git@github.com:nedap/widgets.git' }] })

    const result = await triage(h)

    expect(result).toMatchObject({ reason: 'nedap', status: 'refused' })
  })

  it('refuses a tracker it cannot read', async () => {
    const h = await changed(HARMLESS, { tracker: 'gitlab' })

    expect(await triage(h)).toMatchObject({ reason: 'unsupported-tracker', status: 'refused' })
  })

  it('refuses a ticket gh cannot read', async () => {
    const h = await changed(HARMLESS, { tracker: 'github' })

    expect(await triage(h, { ticket: '#404' })).toMatchObject({ reason: 'tracker-unreachable', status: 'refused' })
  })

  it('rejects a base that names no commit', async () => {
    const h = await changed(HARMLESS)

    const failing = triage(h, { base: 'nowhere' })

    await expect(failing).rejects.toMatchObject({ code: 'invalid-input' })
    await expect(failing).rejects.toThrow('base nowhere names no commit')
  })
})

describe('ticket-review depth from the change', () => {
  /** Ticket review routed with depth thresholds on the change's measures. */
  function depthRouting(thresholds: Readonly<Record<string, number>>): unknown {
    const routing = codeStageRouting() as { clients: Record<string, unknown>; stages: Record<string, Record<string, unknown>> }
    const shipped = { risk_paths: ['**/auth/**'], changed_lines_threshold: 300, labels: ['risk:high'] }
    return {
      ...routing,
      clients: {
        ...routing.clients,
        'vibe-mistral-small': { agent: 'critic-mistral-small', binary: 'vibe', family: 'mistral', kind: 'vibe', model: 'mistral-small-latest' },
      },
      stages: { ...routing.stages, 'ticket-review': { ...routing.stages['ticket-review'], depth_signals: { thresholds } } },
      triage: { ...shipped, model: { client: 'vibe-mistral-small', max_price_usd: 0.03, time_limit_s: 60 } },
    }
  }

  async function prepared(h: Harness): Promise<PrepareResult> {
    await h.commands.allow.run({ code: true, families: ['mistral'], repo: h.repo })
    const result = await h.commands.prepare.run({
      base: 'HEAD~1',
      orchestrator_family: 'anthropic',
      repo: h.repo,
      spec: '.scratch/widgets/spec.md',
      stage: 'ticket-review',
      ticket: TICKET,
    })
    if (result.status !== 'ready') throw new Error(`prepare refused: ${result.message}`)
    return result
  }

  it('goes deep when the change touches more risk paths than the threshold', async () => {
    const h = await changed({ 'src/auth/session.ts': 'x\n' }, { routing: depthRouting({ risk_path_hits: 0 }) })

    const result = await prepared(h)

    expect(result.depth).toBe('deep')
    expect(result.depth_choice.signals).toEqual([{ kind: 'threshold', measure: 'risk_path_hits', threshold: 0, value: 1 }])
  })

  it('goes deep when the change has more changed lines than the threshold', async () => {
    const h = await changed({ 'src/report.ts': lines(11) }, { routing: depthRouting({ changed_lines: 10, risk_path_hits: 0 }) })

    const result = await prepared(h)

    expect(result.depth_choice.measures).toEqual({ changed_lines: { threshold: 10, value: 11 }, risk_path_hits: { threshold: 0, value: 0 } })
    expect(result.depth).toBe('deep')
  })

  it('counts no risk path hits when routing has no triage', async () => {
    const routing = { ...(depthRouting({ risk_path_hits: 0 }) as Record<string, unknown>) }
    delete routing.triage
    const h = await changed({ 'src/auth/session.ts': 'x\n' }, { routing })

    const result = await prepared(h)

    expect(result.depth).toBe('base')
  })
})
