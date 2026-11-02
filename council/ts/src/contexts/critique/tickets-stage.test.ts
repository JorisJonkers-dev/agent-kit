import { readdir, readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  ASSETS_ROOT,
  fenced,
  finding,
  harness,
  isolatedExport,
  specIssue,
  ticketIssue,
  vibeStdout,
  type FakeGhLinks,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import { sha256 } from './domain/manifest.js'
import type { PrepareResult, Refusal } from './index.js'

/**
 * The tickets stage that `/check-tickets` (#185) drives: prepare runs the
 * deterministic ticket checker and freezes its report beside the spec and
 * the tickets, one cheap `ticket-linter` critic judges what the checker
 * cannot, a finding must name the ticket and the rule it breaks, and the
 * stage fails while the tickets hold a dependency cycle.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const AGENT_KIT = 'github.com/jorisjonkers-dev/agent-kit'
const EVERY_FAMILY = ['mistral', 'zhipu', 'deepseek', 'google']
const CRITERION = 'The widget report lists every widget frobnicated today.'
const CRITERIA = `## Acceptance criteria\n\n- [ ] ${CRITERION}\n- [ ] The report is fast.\n`

function critic(findings: readonly Record<string, unknown>[], agent?: string): VibeScript {
  return {
    exitCode: 0,
    exportJson: isolatedExport(),
    stdout: vibeStdout(fenced({ findings, ...(findings.length === 0 ? { no_findings_reason: 'Read every ticket.' } : {}) })),
    ...(agent === undefined ? {} : { agent }),
  }
}

/** A spec issue #167 whose tickets are #170, #171 and #172, opted in for `families`. */
async function github(
  options: {
    readonly links?: FakeGhLinks
    readonly scripts?: VibeScript[]
    readonly families?: readonly string[]
    readonly labels?: readonly string[]
    readonly routingOverride?: unknown
    readonly subIssues?: readonly number[]
  } = {},
): Promise<Harness> {
  const h = await harness({
    issues: {
      [`${AGENT_KIT}#167`]: specIssue({ labels: options.labels ?? ['type:feature'] }),
      [`${AGENT_KIT}#170`]: ticketIssue(170, 'Evidence checks', `## Parent\n\n#167\n\n${CRITERIA}`),
      [`${AGENT_KIT}#171`]: ticketIssue(171, 'Parallel fan-out', `## Parent\n\n#167\n\n${CRITERIA}`),
      [`${AGENT_KIT}#172`]: ticketIssue(172, 'Tripwire', `## Parent\n\n#167\n\n${CRITERIA}`),
    },
    links: { subIssues: { [`${AGENT_KIT}#167`]: options.subIssues ?? [170, 171, 172] }, ...options.links },
    ...(options.routingOverride === undefined ? {} : { routingOverride: options.routingOverride }),
    scripts: options.scripts ?? [],
    tracker: 'github',
  })
  open.push(h)
  await h.commands.allow.run({ families: options.families ?? EVERY_FAMILY, repo: h.repo })
  return h
}

async function prepare(h: Harness, spec: string, extra: Record<string, unknown> = {}): Promise<PrepareResult | Refusal> {
  return h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec, stage: 'tickets', ...extra })
}

async function ready(h: Harness, spec = '#167', extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const run = await prepare(h, spec, extra)
  if (run.status !== 'ready') throw new Error(`not ready: ${JSON.stringify(run)}`)
  return run
}

async function report(run: PrepareResult): Promise<Record<string, unknown>> {
  return JSON.parse(await readFile(join(run.packet_dir, 'check', 'tickets.json'), 'utf8')) as Record<string, unknown>
}

describe('the shipped routing of the tickets stage', () => {
  it('runs one DeepSeek Flash ticket linter at base depth, capped at $0.10 and three minutes', async () => {
    const h = await github()

    const run = await ready(h, '#167', { depth: 'base' })

    expect(run.critics).toEqual([
      expect.objectContaining({ client: 'vibe-deepseek-flash', critic_id: 'c1-vibe-deepseek-flash', family: 'deepseek' }),
    ])
    const manifest = JSON.parse(await readFile(run.manifest_path, 'utf8')) as { critics: Record<string, unknown>[] }
    expect(manifest.critics[0]).toMatchObject({ lenses: ['testability'], max_price_usd: 1, role: 'ticket-linter', time_limit_s: 600 })
  })

  it('adds a Mistral Large ticket linter on simplicity at deep depth', async () => {
    const h = await github()

    const run = await ready(h, '#167', { depth: 'deep' })

    const manifest = JSON.parse(await readFile(run.manifest_path, 'utf8')) as { critics: Record<string, unknown>[] }
    expect(manifest.critics.map((c) => [c.client, c.role, c.lenses, c.max_price_usd, c.time_limit_s])).toEqual([
      ['vibe-deepseek-flash', 'ticket-linter', ['testability'], 1, 600],
      ['vibe-mistral-large', 'ticket-linter', ['simplicity'], 1, 600],
    ])
  })

  it('goes deep on its own when the spec carries risk:high', async () => {
    const h = await github({ labels: ['risk:high'] })

    const run = await ready(h)

    expect(run.depth).toBe('deep')
    expect(run.depth_choice.signals).toContainEqual({ kind: 'label', label: 'risk:high' })
  })

  it('goes deep on its own when the spec has more tickets than its threshold', async () => {
    const h = await github({ routingOverride: { stages: { tickets: { depth_signals: { thresholds: { tickets: 2 } } } } } })

    const run = await ready(h)

    expect(run.depth).toBe('deep')
    expect(run.depth_choice.measures).toEqual({ tickets: { threshold: 2, value: 3 } })
  })

  it('stays at base depth for a small ticket set', async () => {
    const h = await github()

    const run = await ready(h)

    expect(run.depth).toBe('base')
    expect(run.depth_choice.measures).toEqual({ tickets: { threshold: 10, value: 3 } })
  })
})

describe('the tickets packet', () => {
  it('holds the spec, every ticket and the checker report, and nothing else at its root', async () => {
    const h = await github()

    const run = await ready(h)

    expect(await readdir(run.packet_dir)).toEqual(['check', 'spec', 'tickets'])
    expect(run.packet_files).toEqual([
      'spec/issue-167.md',
      'tickets/issue-170.md',
      'tickets/issue-171.md',
      'tickets/issue-172.md',
      'check/tickets.json',
    ])
    expect(await readFile(join(run.packet_dir, 'tickets', 'issue-170.md'), 'utf8')).toContain(CRITERION)
    expect(run.artifact).toEqual({ labels: ['type:feature'], reference: `${AGENT_KIT}#167`, tracker: 'github' })
  })

  it('writes what check-tickets reports, with the size limits from routing', async () => {
    const h = await github({
      links: { blockedBy: { [`${AGENT_KIT}#171`]: [`${AGENT_KIT}#170`, `${AGENT_KIT}#175`] } },
      routingOverride: { stages: { tickets: { ticket_size: { max_criteria: 1 } } } },
    })
    const checked = await h.commands.checkTickets.run({ repo: h.repo, spec: '#167' })

    const run = await ready(h)

    expect(await report(run)).toEqual({ ...checked, size: { max_criteria: 1, max_modules: 5, over_criteria: ['#170', '#171', '#172'] } })
  })

  it('ships the size limits of about five modules and about eight criteria', async () => {
    const h = await github()

    const run = await ready(h)

    expect((await report(run)).size).toEqual({ max_criteria: 8, max_modules: 5, over_criteria: [] })
  })

  it('reads local-markdown tickets from the issues folder beside the spec', async () => {
    const h = await harness()
    open.push(h)
    await h.commands.allow.run({ families: EVERY_FAMILY, repo: h.repo })
    await h.writeText(join(h.repo, '.scratch', 'widgets', 'issues', '01-schema.md'), `# 01: Schema\n\n**Blocked by:** 02\n\n${CRITERIA}`)
    await h.writeText(join(h.repo, '.scratch', 'widgets', 'issues', '02-api.md'), `# 02: API\n\n**Blocked by:** 01\n\n${CRITERIA}`)

    const run = await ready(h, '.scratch/widgets/spec.md')

    expect(run.packet_files).toEqual(['spec/spec.md', 'tickets/01-schema.md', 'tickets/02-api.md', 'check/tickets.json'])
    expect(await report(run)).toMatchObject({ cycles: [['01', '02']], status: 'failed' })
  })

  it('counts against the spec’s flow', async () => {
    const h = await github()

    const run = await ready(h)

    const manifest = JSON.parse(await readFile(run.manifest_path, 'utf8')) as { flow: string }
    expect(manifest.flow).toBe(`${AGENT_KIT}#167`)
  })

  it('is refused when the spec has no tickets yet', async () => {
    const h = await github({ subIssues: [] })

    expect(await prepare(h, '#167')).toEqual({
      message: `${AGENT_KIT}#167 has no tickets yet; cut them with /to-tickets first`,
      reason: 'no-tickets',
      status: 'refused',
    })
  })

  it('is refused in one line, before a run exists, when gh cannot read the tickets', async () => {
    const h = await github({ links: { subIssues: { [`${AGENT_KIT}#167`]: 'HTTP 502: Bad Gateway' } } })

    expect(await prepare(h, '#167')).toEqual({
      message: `cannot read the tickets of ${AGENT_KIT}#167 through gh: HTTP 502: Bad Gateway`,
      reason: 'tracker-unreachable',
      status: 'refused',
    })
    expect(await readdir(join(h.repo, '.git'))).not.toContain('critique')
  })

  it('is refused before anything is read when the repository is not opted in', async () => {
    const h = await harness({ tracker: 'github' })
    open.push(h)

    expect(await prepare(h, '#167')).toMatchObject({ reason: 'not-opted-in', status: 'refused' })
    expect(h.gh.linkRequests).toEqual([])
  })
})

describe('the ticket-linter prompt', () => {
  it('names the spec, each ticket and the checker report, and its role hash reaches the ledger', async () => {
    const h = await github({ scripts: [critic([])] })
    const run = await ready(h)

    await h.commands.fanOut.run({ run_dir: run.run_dir })
    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    const prompt = await readFile(join(run.run_dir, 'critics', 'c1-vibe-deepseek-flash', 'prompt.md'), 'utf8')
    expect(prompt).toContain('- `spec/issue-167.md`: the spec the tickets come from')
    expect(prompt).toContain('- `tickets/issue-170.md`: a ticket under review')
    expect(prompt).toContain("- `check/tickets.json`: the deterministic ticket checker's report on these tickets")
    const role = await readFile(join(ASSETS_ROOT, 'roles', 'ticket-linter.md'), 'utf8')
    expect(prompt).toContain(role.trim())
    const ledger = await readFile(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl'), 'utf8')
    const runLine = ledger
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line) as { type: string; role?: string; prompt_hashes?: { role: string } })
      .find((line) => line.type === 'run')
    expect(runLine).toMatchObject({ prompt_hashes: { role: sha256(role) }, role: 'ticket-linter' })
  })

  it('keeps the role under about 400 words', async () => {
    const role = await readFile(join(ASSETS_ROOT, 'roles', 'ticket-linter.md'), 'utf8')

    expect(role.split(/\s+/).filter((word) => word.length > 0).length).toBeLessThanOrEqual(400)
  })
})

describe('the tickets-stage evidence rule', () => {
  const quote = { file: 'tickets/issue-170.md', kind: 'quote', value: 'The report is fast.' }

  it.each<[string, string, string | undefined]>([
    ['names the ticket and the rule it breaks', '#170: untestable-criterion', undefined],
    ['names an uncovered requirement against the spec', 'spec: uncovered-requirement (user story 3)', undefined],
    ['names a rule but no ticket', 'Acceptance criteria: untestable-criterion', 'no-ticket-id'],
    ['names a ticket but no rule', '#170, acceptance criteria', 'no-ticket-rule'],
    ['names a ticket that is not one of the spec', '#999: too-big', 'no-ticket-id'],
  ])('a finding that %s', async (_name, location, hint) => {
    const h = await github({ scripts: [critic([finding({ category: 'untestable', check: 'untestable-criterion', evidence: [quote], location })])] })
    const run = await ready(h)

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(result.findings[0]?.pre_verdict).toEqual(
      hint === undefined ? undefined : expect.objectContaining({ reason_code: hint, verdict: 'hint' }),
    )
  })

  it('still rejects a quote that is not in the packet', async () => {
    const fabricated = { file: 'tickets/issue-170.md', kind: 'quote', value: 'The report must render in a second.' }
    const h = await github({ scripts: [critic([finding({ evidence: [fabricated], location: '#170: untestable-criterion' })])] })
    const run = await ready(h)

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(result.findings[0]?.pre_verdict).toMatchObject({ reason_code: 'fabricated-evidence', verdict: 'rejected' })
  })
})

describe('the tickets-stage summary', () => {
  const CYCLE = { [`${AGENT_KIT}#170`]: [`${AGENT_KIT}#171`], [`${AGENT_KIT}#171`]: [`${AGENT_KIT}#170`] }

  it('fails the stage and says not to start /implement while the tickets hold a cycle', async () => {
    const h = await github({ links: { blockedBy: CYCLE }, scripts: [critic([])] })
    const run = await ready(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    expect(recorded.ticket_check).toBe('failed')
    expect(recorded.summary).toContain(
      'Ticket check: failed: 3 tickets; cycle #170 → #171 → #170; no dangling blocker; user story coverage not checkable (no ticket cites a user story)',
    )
    expect(recorded.summary).toContain('Do not start /implement: the tickets hold a dependency cycle.')
  })

  it('passes the stage when there is no cycle, listing blockers to check and uncited stories', async () => {
    const h = await github({ links: { blockedBy: { [`${AGENT_KIT}#171`]: [`${AGENT_KIT}#175`] } }, scripts: [critic([])] })
    h.gh.serve(`${AGENT_KIT}#170`, ticketIssue(170, 'Evidence checks', `Delivers user story 1.\n\n${CRITERIA}`))
    const run = await ready(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    expect(recorded.ticket_check).toBe('passed')
    expect(recorded.summary).toContain('Ticket check: passed: 3 tickets; no cycle; 1 dangling blocker to check (#171 blocked by #175); user story 3 cited by no ticket')
    expect(recorded.summary).not.toContain('Do not start /implement')
  })

  it('says every story is cited when the tickets cite them all', async () => {
    const h = await github({ scripts: [critic([])] })
    h.gh.serve(`${AGENT_KIT}#170`, ticketIssue(170, 'Evidence checks', `Delivers user stories 1 and 3.\n\n${CRITERIA}`))
    const run = await ready(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const { summary } = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    expect(summary).toContain('Ticket check: passed: 3 tickets; no cycle; no dangling blocker; every user story cited')
  })

  it('says not to start /implement while a critical scope question is unanswered', async () => {
    const quote = { file: 'tickets/issue-170.md', kind: 'quote', value: 'The report is fast.' }
    const h = await github({
      scripts: [critic([finding({ check: 'too-big', evidence: [quote], location: '#170: too-big', severity: 'critical' })])],
    })
    const run = await ready(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const { summary } = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [{ finding_id: 'F1', reason: 'Split #170 into the report and its export?', verdict: 'needs-human' }],
    })

    expect(summary).toContain('Do not start /implement: critical question F1 is unanswered.')
    expect(summary).not.toContain('/to-tickets')
  })

  it('leaves the ticket check out of a spec-stage record', async () => {
    const h = await harness({ scripts: [critic([])] })
    open.push(h)
    await h.commands.allow.run({ families: ['mistral'], repo: h.repo })
    const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })
    if (run.status !== 'ready') throw new Error('not ready')
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    expect(recorded).not.toHaveProperty('ticket_check')
    expect(recorded.summary).not.toContain('Ticket check')
  })
})
