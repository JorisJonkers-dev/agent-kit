import { createHash } from 'node:crypto'
import { readdir, readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  ASSETS_ROOT,
  fenced,
  harness,
  isolatedExport,
  specIssue,
  ticketIssue,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import type { PrepareResult } from './index.js'

/**
 * What `/review-feature` (#189) relies on the runner for: the feature-review
 * roster in shipped routing, the `feature-reviewer` role, the evidence rule
 * for accepting a feature finding, a follow-up ticket per accepted finding,
 * and the gate line with the head it was computed for.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const SPEC = '.scratch/widgets/spec.md'
const ISSUES = '.scratch/widgets/issues'

/**
 * A repository opted in for code with every family, on local markdown, whose
 * feature branch holds two tickets' work: `c2` is the base, `c3` the head.
 */
async function feature(
  options: Parameters<typeof harness>[0] & { readonly families?: readonly string[] } = {},
): Promise<Harness> {
  const h = await harness(options)
  open.push(h)
  await h.commands.allow.run({ code: true, families: options.families ?? ['mistral', 'google', 'zhipu', 'deepseek'], repo: h.repo })
  await h.writeText(join(h.repo, ISSUES, '01-store.md'), '# 01: Store widgets\n\n**Status:** ready-for-agent\n')
  await h.writeText(join(h.repo, ISSUES, '02-report.md'), '# 02: Report widgets\n\n**Status:** ready-for-agent\n\n**Blocked by:** 01\n')
  await h.git.commit()
  await h.writeText(join(h.repo, 'src/store.ts'), 'export function store(count: number): string {\n  return String(count)\n}\n')
  await h.writeText(join(h.repo, 'src/report.ts'), "import { store } from './store'\n\nexport const total: number = store(3)\n")
  await h.git.commit()
  return h
}

async function prepared(h: Harness, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const result = await h.commands.prepare.run({
    base: 'HEAD~1',
    orchestrator_family: 'anthropic',
    repo: h.repo,
    spec: SPEC,
    stage: 'feature-review',
    ...extra,
  })
  if (result.status !== 'ready') throw new Error(`prepare refused: ${result.message}`)
  return result
}

describe('the feature-review roster in shipped routing', () => {
  it('runs Mistral Large on integration and Gemini Flash on failure modes, both as feature reviewers', async () => {
    const h = await feature()

    const run = await prepared(h)

    expect(run.critics).toEqual([
      {
        client: 'vibe-mistral-large',
        critic_id: 'c1-vibe-mistral-large',
        family: 'mistral',
        lenses: ['integration'],
        model: 'mistral-large-4-0',
        role: 'feature-reviewer',
      },
      {
        client: 'agy-gemini-flash-high',
        critic_id: 'c2-agy-gemini-flash-high',
        family: 'google',
        lenses: ['failure-modes'],
        model: 'gemini-3.8-flash-high',
        role: 'feature-reviewer',
      },
    ])
  })

  it('adds GLM on operability and DeepSeek Flash on simplicity at deep depth', async () => {
    const h = await feature()

    const run = await prepared(h, { depth: 'deep' })

    expect(run.critics.slice(2).map(({ client, lenses, role }) => ({ client, lenses, role }))).toEqual([
      { client: 'vibe-glm', lenses: ['operability'], role: 'feature-reviewer' },
      { client: 'vibe-deepseek-flash', lenses: ['simplicity'], role: 'feature-reviewer' },
    ])
  })

  it('caps each critic at $1.00 and 12 minutes', async () => {
    const h = await feature()

    const run = await prepared(h, { depth: 'deep' })

    const manifest = JSON.parse(await readFile(run.manifest_path, 'utf8')) as {
      readonly critics: readonly { readonly max_price_usd: number; readonly time_limit_s: number }[]
    }
    expect(manifest.critics.map(({ max_price_usd, time_limit_s }) => ({ max_price_usd, time_limit_s }))).toEqual(
      Array.from({ length: 4 }, () => ({ max_price_usd: 1, time_limit_s: 720 })),
    )
  })

  it('still runs Gemini once only the Antigravity reserve is left', async () => {
    const h = await feature()
    const launch = { client: 'agy-gemini-flash-high', kind: 'agy', recorded_at: '2026-10-30T08:00:00.000Z', type: 'invocation' }
    await h.writeText(join(h.home, '.local/share/agent-kit/critique/ledger.jsonl'), `${JSON.stringify(launch)}\n`.repeat(10))

    const run = await prepared(h)

    expect(run.critics.map((critic) => critic.client)).toEqual(['vibe-mistral-large', 'agy-gemini-flash-high'])
  })
})

/** A critic that raises `findings`, or says what it checked when it found nothing. */
function critic(findings: readonly Record<string, unknown>[]): VibeScript {
  const review = findings.length === 0 ? { findings, no_findings_reason: 'Followed every seam between the tickets.' } : { findings }
  return { exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced(review)) }
}

/** A repository that allows Mistral only, so a feature review runs the one Mistral Large critic. */
async function mistralOnly(scripts: VibeScript[]): Promise<Harness> {
  return feature({ families: ['mistral'], scripts })
}

function ledgerLines(h: Harness): Promise<readonly Record<string, unknown>[]> {
  return readFile(join(h.home, '.local/share/agent-kit/critique/ledger.jsonl'), 'utf8').then((text) =>
    text
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line) as Record<string, unknown>)
      .filter((line) => line.type !== 'invocation'),
  )
}

describe('the feature-reviewer role', () => {
  it("is the role a feature review's critic is given", async () => {
    const h = await mistralOnly([critic([])])
    const run = await prepared(h)

    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const prompt = await readFile(join(run.run_dir, 'critics', 'c1-vibe-mistral-large', 'prompt.md'), 'utf8')
    expect(prompt).toContain('# Role: feature reviewer')
  })

  it('has its hash recorded in the ledger', async () => {
    const h = await mistralOnly([critic([])])
    const run = await prepared(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    const role = await readFile(join(ASSETS_ROOT, 'roles', 'feature-reviewer.md'), 'utf8')
    const [line] = await ledgerLines(h)
    expect(line).toMatchObject({ prompt_hashes: { role: createHash('sha256').update(role).digest('hex') }, role: 'feature-reviewer' })
  })
})

/** A broken contract between the two tickets: the report treats the stored string as a number. */
function seamFinding(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    category: 'integration',
    claim: 'The report adds up what store returns as a number, but store returns a string.',
    evidence: [
      { kind: 'trace', value: 'tree/src/report.ts:3 assigns store(3) to a number; tree/src/store.ts:2 returns String(count).' },
    ],
    id: 'F1',
    location: 'tree/src/report.ts:3',
    severity: 'critical',
    ...overrides,
  }
}

/** Prepares and fans out a feature review whose one critic raises `findings`. */
async function reviewed(findings: readonly Record<string, unknown>[], given?: Harness): Promise<{ h: Harness; run: PrepareResult }> {
  const h = given ?? (await mistralOnly([]))
  h.process.queue(critic(findings))
  const run = await prepared(h)
  await h.commands.fanOut.run({ run_dir: run.run_dir })
  return { h, run }
}

const TRACE_TICKETS = ['tickets/01-store.md', 'tickets/02-report.md']

describe('accepting a feature finding', () => {
  it('needs a failing test the orchestrator ran or a trace across tickets it walked', async () => {
    const { h, run } = await reviewed([seamFinding()])

    await expect(
      h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', follow_up: '03', verdict: 'accepted' }] }),
    ).rejects.toThrow(/F1: a feature finding is accepted only with reason_code test-fails or cross-ticket-trace/)
  })

  it('needs a trace to cross code from two or more tickets', async () => {
    const { h, run } = await reviewed([seamFinding()])

    await expect(
      h.commands.record.run({
        run_dir: run.run_dir,
        verdicts: [
          { finding_id: 'F1', follow_up: '03', reason_code: 'cross-ticket-trace', trace_tickets: ['tickets/01-store.md'], verdict: 'accepted' },
        ],
      }),
    ).rejects.toThrow(/F1: a cross-ticket-trace names at least two tickets of the packet in trace_tickets/)
  })

  it('needs the tickets a trace crosses to be tickets of the packet', async () => {
    const { h, run } = await reviewed([seamFinding()])

    await expect(
      h.commands.record.run({
        run_dir: run.run_dir,
        verdicts: [
          {
            finding_id: 'F1',
            follow_up: '03',
            reason_code: 'cross-ticket-trace',
            trace_tickets: ['tickets/01-store.md', 'tree/src/store.ts'],
            verdict: 'accepted',
          },
        ],
      }),
    ).rejects.toThrow(/F1: a cross-ticket-trace names at least two tickets of the packet in trace_tickets/)
  })

  it('needs the follow-up ticket it became', async () => {
    const { h, run } = await reviewed([seamFinding()])

    await expect(
      h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', reason_code: 'test-fails', verdict: 'accepted' }] }),
    ).rejects.toThrow(/F1: an accepted feature finding names its follow-up ticket in follow_up/)
  })

  it('takes trace_tickets only with a cross-ticket-trace', async () => {
    const { h, run } = await reviewed([seamFinding()])

    await expect(
      h.commands.record.run({
        run_dir: run.run_dir,
        verdicts: [{ finding_id: 'F1', follow_up: '03', reason_code: 'test-fails', trace_tickets: TRACE_TICKETS, verdict: 'accepted' }],
      }),
    ).rejects.toThrow(/F1: trace_tickets belong to a cross-ticket-trace/)
  })

  it('gives a rejected finding no follow-up', async () => {
    const { h, run } = await reviewed([seamFinding()])

    await expect(
      h.commands.record.run({
        run_dir: run.run_dir,
        verdicts: [{ finding_id: 'F1', follow_up: '03', reason: 'The proposed test passes.', reason_code: 'test-passes', verdict: 'rejected' }],
      }),
    ).rejects.toThrow(/F1: only an accepted feature finding has a follow_up or trace_tickets/)
  })
})

describe('a follow-up at any other stage', () => {
  it('is refused, since only a feature review turns findings into tickets', async () => {
    const h = await harness({ scripts: [critic([seamFinding({ evidence: [], location: 'User story 3' })])] })
    open.push(h)
    await h.commands.allow.run({ families: ['mistral'], repo: h.repo })
    const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: SPEC, stage: 'spec' })
    if (run.status !== 'ready') throw new Error('prepare refused')
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    await expect(
      h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', follow_up: '03', verdict: 'accepted' }] }),
    ).rejects.toThrow(/F1: follow_up and trace_tickets belong to a feature-review verdict/)
  })
})

/** Files the follow-up ticket the orchestrator creates for an accepted finding, as `/to-tickets` would. */
async function followUp(h: Harness, name = '03-fix-the-seam.md', status = 'ready-for-agent'): Promise<void> {
  await h.writeText(join(h.repo, ISSUES, name), `# ${name.slice(0, 2)}: Fix the seam\n\n**Status:** ${status}\n`)
}

const ACCEPTED_SEAM = { finding_id: 'F1', follow_up: '03', reason_code: 'cross-ticket-trace', trace_tickets: TRACE_TICKETS, verdict: 'accepted' }

/** A new commit on the feature branch, so a re-run reviews a new head. */
async function newHead(h: Harness): Promise<void> {
  await h.writeText(join(h.repo, 'src/store.ts'), 'export function store(count: number): number {\n  return count\n}\n')
  await h.git.commit()
}

describe('the gate', () => {
  it("opens the report with GATE: FAIL while an accepted critical finding's follow-up ticket is open", async () => {
    const { h, run } = await reviewed([seamFinding()])
    await followUp(h)

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [ACCEPTED_SEAM] })

    expect((await readFile(recorded.report_path, 'utf8')).split('\n')[0]).toBe('GATE: FAIL (1 critical)')
  })

  it('opens the summary with the same gate line', async () => {
    const { h, run } = await reviewed([seamFinding()])
    await followUp(h)

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [ACCEPTED_SEAM] })

    expect(recorded.summary.split('\n')[0]).toBe('GATE: FAIL (1 critical)')
  })

  it('counts every open critical follow-up', async () => {
    const { h, run } = await reviewed([seamFinding(), seamFinding({ claim: 'Store drops the count it was given.', id: 'F2', location: 'tree/src/store.ts:2' })])
    await followUp(h)
    await followUp(h, '04-fix-the-store.md')

    const recorded = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [ACCEPTED_SEAM, { ...ACCEPTED_SEAM, finding_id: 'F2', follow_up: '04' }],
    })

    expect(recorded.summary.split('\n')[0]).toBe('GATE: FAIL (2 critical)')
  })

  it('passes when no accepted finding is critical', async () => {
    const { h, run } = await reviewed([seamFinding({ severity: 'major' })])
    await followUp(h)

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [ACCEPTED_SEAM] })

    expect(recorded.summary.split('\n')[0]).toBe('GATE: PASS')
  })

  it('passes when a critical finding is rejected', async () => {
    const { h, run } = await reviewed([seamFinding()])

    const recorded = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [{ finding_id: 'F1', reason: 'The proposed test passes.', reason_code: 'test-passes', verdict: 'rejected' }],
    })

    expect(recorded.summary.split('\n')[0]).toBe('GATE: PASS')
  })

  it('refuses a follow-up that is not a ticket of the spec', async () => {
    const { h, run } = await reviewed([seamFinding()])

    await expect(h.commands.record.run({ run_dir: run.run_dir, verdicts: [ACCEPTED_SEAM] })).rejects.toThrow(
      /F1's follow-up 03 is not a ticket of \.scratch\/widgets\/spec\.md; create it as one of the spec's tickets first/,
    )
  })

  it('holds on a re-run at a new head while the follow-up is open, even when no critic raises the finding again', async () => {
    const { h, run } = await reviewed([seamFinding()])
    await followUp(h)
    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [ACCEPTED_SEAM] })
    await newHead(h)

    const again = await reviewed([], h)
    const recorded = await h.commands.record.run({ run_dir: again.run.run_dir, verdicts: [] })

    expect(recorded.summary.split('\n')[0]).toBe('GATE: FAIL (1 critical)')
  })

  it('passes on a re-run at a new head once the follow-up is closed', async () => {
    const { h, run } = await reviewed([seamFinding()])
    await followUp(h)
    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [ACCEPTED_SEAM] })
    await followUp(h, '03-fix-the-seam.md', 'done')
    await newHead(h)

    const again = await reviewed([], h)
    const recorded = await h.commands.record.run({ run_dir: again.run.run_dir, verdicts: [] })

    expect(recorded.summary.split('\n')[0]).toBe('GATE: PASS')
  })

  it('counts a follow-up raised again on a re-run once', async () => {
    const { h, run } = await reviewed([seamFinding()])
    await followUp(h)
    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [ACCEPTED_SEAM] })
    await newHead(h)

    const again = await reviewed([seamFinding()], h)
    const recorded = await h.commands.record.run({ run_dir: again.run.run_dir, verdicts: [ACCEPTED_SEAM] })

    expect(recorded.summary.split('\n')[0]).toBe('GATE: FAIL (1 critical)')
  })

  it('records the head it reviewed and the gate in the run, for a script to read', async () => {
    const { h, run } = await reviewed([seamFinding()])
    await followUp(h)

    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [ACCEPTED_SEAM] })

    expect(JSON.parse(await readFile(join(run.run_dir, 'gate.json'), 'utf8'))).toEqual({
      base_commit: 'c2',
      critical_open: 1,
      follow_ups: [{ finding_id: 'F1', open: true, reference: '.scratch/widgets/issues/03-fix-the-seam.md', severity: 'critical', ticket: '03' }],
      gate: 'fail',
      head_commit: 'c3',
      line: 'GATE: FAIL (1 critical)',
      open_critical: [{ finding_id: 'F1', reference: '.scratch/widgets/issues/03-fix-the-seam.md', run_id: run.run_id, ticket: '03' }],
      recorded_at: '2026-10-30T12:00:00.000Z',
      repository: 'github.com/jorisjonkers-dev/agent-kit',
      run_id: run.run_id,
      spec: SPEC,
      stage: 'feature-review',
      version: 1,
    })
  })

  it("appends each run's gate to one index in the critique directory, so the latest for a head is one read away", async () => {
    const { h, run } = await reviewed([seamFinding()])
    await followUp(h)
    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [ACCEPTED_SEAM] })
    await followUp(h, '03-fix-the-seam.md', 'done')
    await newHead(h)
    const again = await reviewed([], h)

    await h.commands.record.run({ run_dir: again.run.run_dir, verdicts: [] })

    const index = (await readFile(join(h.repo, '.git', 'critique', 'gates.jsonl'), 'utf8'))
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line) as Record<string, unknown>)
    expect(index.map(({ gate, head_commit, run_id }) => ({ gate, head_commit, run_id }))).toEqual([
      { gate: 'fail', head_commit: 'c3', run_id: run.run_id },
      { gate: 'pass', head_commit: 'c4', run_id: again.run.run_id },
    ])
  })

  it('puts the gate in record\'s result', async () => {
    const { h, run } = await reviewed([])

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    expect(recorded.gate).toMatchObject({ critical_open: 0, gate: 'pass', head_commit: 'c3', line: 'GATE: PASS' })
  })

  it('is left out of every other stage', async () => {
    const h = await harness({ scripts: [critic([])] })
    open.push(h)
    await h.commands.allow.run({ families: ['mistral'], repo: h.repo })
    const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: SPEC, stage: 'spec' })
    if (run.status !== 'ready') throw new Error('prepare refused')
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    expect(recorded.summary).not.toContain('GATE:')
    expect(recorded.gate).toBeUndefined()
  })
})

describe('the gate in its report', () => {
  it('names the head it holds for and the follow-up that holds it', async () => {
    const { h, run } = await reviewed([seamFinding()])
    await followUp(h)

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [ACCEPTED_SEAM] })

    const report = await readFile(recorded.report_path, 'utf8')
    expect(report).toContain('GATE: FAIL (1 critical) for head c3 (base c2)')
    expect(report).toContain('- 03 (.scratch/widgets/issues/03-fix-the-seam.md), for F1\n')
    expect(report).toContain('Verdict: accepted (cross-ticket-trace), follow-up 03, trace across tickets/01-store.md, tickets/02-report.md')
  })

  it('names the earlier run of a follow-up that still holds it', async () => {
    const { h, run } = await reviewed([seamFinding()])
    await followUp(h)
    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [ACCEPTED_SEAM] })
    await newHead(h)
    const again = await reviewed([], h)

    const recorded = await h.commands.record.run({ run_dir: again.run.run_dir, verdicts: [] })

    expect(await readFile(recorded.report_path, 'utf8')).toContain(`- 03 (.scratch/widgets/issues/03-fix-the-seam.md), for F1 of run ${run.run_id}\n`)
  })

  it('says none when nothing holds it', async () => {
    const { h, run } = await reviewed([])

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    expect(await readFile(recorded.report_path, 'utf8')).toContain('Open critical follow-ups:\n\nnone\n')
  })
})

describe('the gate when it cannot be computed', () => {
  it('keeps failing for a follow-up that is no longer a ticket of the spec', async () => {
    const { h, run } = await reviewed([seamFinding()])
    await followUp(h)
    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [ACCEPTED_SEAM] })
    await h.removeFile(join(h.repo, ISSUES, '03-fix-the-seam.md'))
    await newHead(h)
    const again = await reviewed([], h)

    const recorded = await h.commands.record.run({ run_dir: again.run.run_dir, verdicts: [] })

    expect(recorded.summary.split('\n')[0]).toBe('GATE: FAIL (1 critical)')
  })

  it('refuses a feature-review run whose manifest lost the commits it reviewed', async () => {
    const { h, run } = await reviewed([])
    const manifest = JSON.parse(await readFile(run.manifest_path, 'utf8')) as Record<string, unknown>
    await h.writeText(run.manifest_path, JSON.stringify({ ...manifest, code: undefined }))

    await expect(h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })).rejects.toMatchObject({
      code: 'invalid-run',
      message: `run ${run.run_id} is a feature review without the commits it reviewed`,
    })
  })

  it('refuses to record over a gate index it cannot read, and writes nothing', async () => {
    const { h, run } = await reviewed([])
    await h.writeText(join(h.repo, '.git', 'critique', 'gates.jsonl'), '{"version": 1}\n')

    await expect(h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })).rejects.toThrow(/gates\.jsonl line 1: /)
    expect(await readdir(run.run_dir)).not.toContain('report.md')
  })
})

describe('the gate on a GitHub tracker', () => {
  const AGENT_KIT = 'github.com/jorisjonkers-dev/agent-kit'
  const ticket = (number: number, title: string) => ({ ...ticketIssue(number, title, 'Build it.'), state: 'open' as const })

  /** Spec #181 with tickets #183 and #185, and the follow-up #301 the orchestrator filed for F1. */
  async function onGithub(): Promise<Harness> {
    return feature({
      families: ['mistral'],
      issues: {
        [`${AGENT_KIT}#181`]: specIssue({ number: 181 }),
        [`${AGENT_KIT}#183`]: ticket(183, 'Store widgets'),
        [`${AGENT_KIT}#185`]: ticket(185, 'Report widgets'),
        [`${AGENT_KIT}#301`]: ticket(301, 'Fix the seam'),
      },
      links: { subIssues: { [`${AGENT_KIT}#181`]: [183, 185, 301] } },
      tracker: 'github',
    })
  }

  async function reviewedOnGithub(h: Harness, findings: readonly Record<string, unknown>[]): Promise<PrepareResult> {
    h.process.queue(critic(findings))
    const run = await prepared(h, { spec: '#181' })
    await h.commands.fanOut.run({ run_dir: run.run_dir })
    return run
  }

  const SEAM_ON_GITHUB = { ...ACCEPTED_SEAM, follow_up: '#301', trace_tickets: ['tickets/issue-183.md', 'tickets/issue-185.md'] }

  it('fails while the follow-up issue is open', async () => {
    const h = await onGithub()
    const run = await reviewedOnGithub(h, [seamFinding()])

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [SEAM_ON_GITHUB] })

    expect(recorded.gate).toMatchObject({ line: 'GATE: FAIL (1 critical)', spec: `${AGENT_KIT}#181` })
    expect(recorded.gate?.follow_ups).toEqual([{ finding_id: 'F1', open: true, reference: `${AGENT_KIT}#301`, severity: 'critical', ticket: '#301' }])
  })

  it('takes the follow-up by its URL', async () => {
    const h = await onGithub()
    const run = await reviewedOnGithub(h, [seamFinding()])

    const recorded = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [{ ...SEAM_ON_GITHUB, follow_up: 'https://github.com/JorisJonkers-dev/agent-kit/issues/301' }],
    })

    expect(recorded.gate?.follow_ups.map((followUp) => followUp.ticket)).toEqual(['#301'])
  })

  it('passes on a re-run once the follow-up issue is closed', async () => {
    const h = await onGithub()
    const run = await reviewedOnGithub(h, [seamFinding()])
    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [SEAM_ON_GITHUB] })
    h.gh.serve(`${AGENT_KIT}#301`, { ...ticket(301, 'Fix the seam'), state: 'closed' })
    await newHead(h)
    const again = await reviewedOnGithub(h, [])

    const recorded = await h.commands.record.run({ run_dir: again.run_dir, verdicts: [] })

    expect(recorded.gate?.line).toBe('GATE: PASS')
  })

  it("cannot record while the spec's tickets cannot be read, and writes nothing", async () => {
    const h = await onGithub()
    const run = await reviewedOnGithub(h, [])
    h.gh.serve(`${AGENT_KIT}#183`, 'gh: To get started with GitHub CLI, please run:  gh auth login')

    await expect(h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })).rejects.toMatchObject({
      code: 'tracker-unreachable',
      message: expect.stringMatching(/^the gate needs the spec's tickets: cannot read the tickets of github\.com\/jorisjonkers-dev\/agent-kit#181/) as unknown,
    })
    expect(await readdir(run.run_dir)).not.toContain('report.md')
  })
})
