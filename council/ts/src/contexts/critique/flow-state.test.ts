import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  DECISION_CANARY,
  fenced,
  finding,
  harness,
  isolatedExport,
  specIssue,
  ticketIssue,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import { flowStatePath, parseFlowState, type NextStep, type PrepareResult, type Refusal, type StateResult } from './index.js'

/**
 * What `/flow` (#192), `/flow budget` (#193) and the gates (#197) rely on the
 * `state` command for: one feature's flow state, rebuilt from the ledger,
 * the run directories, the review and gate indexes and the tracker, and the
 * single next step, which is a critique stage or the Pocock command the
 * owner types.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const SPEC = '.scratch/widgets/spec.md'
const ISSUES = '.scratch/widgets/issues'
const STORE = `${ISSUES}/01-store.md`
const REPORT = `${ISSUES}/02-report.md`
const LEDGER = '.local/share/agent-kit/critique/ledger.jsonl'

/** A repository on local markdown, opted in for code to Mistral only, so each stage runs at most one Mistral critic. */
async function repository(options: Parameters<typeof harness>[0] & { readonly families?: readonly string[] } = {}): Promise<Harness> {
  const h = await harness(options)
  open.push(h)
  await h.commands.allow.run({ code: true, families: options.families ?? ['mistral'], repo: h.repo })
  return h
}

async function stateOf(h: Harness, input: Record<string, unknown> = {}): Promise<StateResult> {
  const result = await h.commands.state.run({ repo: h.repo, spec: SPEC, ...input })
  if (result.status === 'refused') throw new Error(`state refused: ${result.message}`)
  return result
}

async function next(h: Harness, input: Record<string, unknown> = {}): Promise<NextStep> {
  return (await stateOf(h, input)).next
}

/** A critic that raises `findings`, or says what it checked when it found nothing. */
function critic(findings: readonly Record<string, unknown>[] = []): VibeScript {
  const review = findings.length === 0 ? { findings, no_findings_reason: 'Read every file.' } : { findings }
  return { exitCode: 0, exportJson: { ...isolatedExport(), cost_usd: 0.25 }, stdout: vibeStdout(fenced(review)) }
}

/** Prepares a run a minute after the last one, so run ids and record times keep their order. */
async function prepared(h: Harness, input: Record<string, unknown>): Promise<PrepareResult> {
  h.clock.advance(1)
  const result = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: SPEC, ...input })
  if (result.status !== 'ready') throw new Error(`prepare refused: ${result.message}`)
  return result
}

/** Prepares, fans out and records one run. The first critic raises `findings`; any other finds nothing. */
async function recorded(
  h: Harness,
  input: Record<string, unknown>,
  findings: readonly Record<string, unknown>[] = [],
  verdicts: readonly Record<string, unknown>[] = [],
): Promise<PrepareResult> {
  const run = await prepared(h, input)
  run.critics.forEach((_, index) => { h.process.queue(critic(index === 0 ? findings : [])); })
  await h.commands.fanOut.run({ run_dir: run.run_dir })
  await h.commands.record.run({ run_dir: run.run_dir, verdicts })
  return run
}

/** A critical spec finding at its own location, so findings never merge. */
function critical(n: number): Record<string, unknown> {
  return finding({ id: `F${String(n)}`, location: `User story ${String(n)}`, severity: 'critical' })
}

async function ticket(h: Harness, path: string, heading: string, extra = ''): Promise<void> {
  await h.writeText(join(h.repo, path), `# ${heading}\n\n**Status:** ready-for-agent\n${extra}\n## Acceptance criteria\n\n- [ ] It works.\n`)
}

/** Two tickets, the first blocked by the second, so dependency order is the reverse of tracker order. */
async function twoTickets(h: Harness): Promise<void> {
  await ticket(h, STORE, '01: Store widgets', '\n**Blocked by:** 02\n')
  await ticket(h, REPORT, '02: Report widgets')
}

/** A spec flow through planning: the spec critique and the ticket check recorded, two tickets cut. */
async function planned(options: Parameters<typeof repository>[0] = {}): Promise<Harness> {
  const h = await repository(options)
  await recorded(h, { stage: 'spec' })
  await twoTickets(h)
  await recorded(h, { stage: 'tickets' })
  return h
}

async function close(h: Harness, path: string): Promise<void> {
  const text = await h.readText(join(h.repo, path))
  await h.writeText(join(h.repo, path), text.replace('**Status:** ready-for-agent', '**Status:** closed'))
}

describe('a draft flow, before /to-spec', () => {
  it('starts by naming /grill-with-docs for the owner to type', async () => {
    const h = await repository()

    const result = await stateOf(h, { draft: 'widgets', spec: undefined })

    expect(result.next).toEqual({
      command: '/grill-with-docs',
      kind: 'command',
      reason: expect.stringContaining('grilling') as unknown,
      skill: '/grill-with-docs',
    })
    expect(result.state).toMatchObject({ draft: null, flow: 'draft:widgets', grilled: false, phase: 'planning', spec: null })
  })

  it('runs the decisions critique once the owner says the grilling is done', async () => {
    const h = await repository()

    const step = await next(h, { draft: 'widgets', record: { grilled: true }, spec: undefined })

    expect(step).toMatchObject({ kind: 'stage', stage: 'decisions' })
  })

  it('remembers the grilling in a later session', async () => {
    const h = await repository()
    await stateOf(h, { draft: 'widgets', record: { grilled: true }, spec: undefined })

    const result = await stateOf(h, { draft: 'widgets', spec: undefined })

    expect(result.state.grilled).toBe(true)
    expect(result.next).toMatchObject({ kind: 'stage', stage: 'decisions' })
  })

  it('finishes a decisions run that was prepared but not recorded', async () => {
    const h = await repository()
    const run = await prepared(h, { feature: 'Widgets are frobnicated.', flow: 'draft:widgets', stage: 'decisions' })

    const step = await next(h, { draft: 'widgets', spec: undefined })

    expect(step).toMatchObject({ kind: 'stage', resume_run_id: run.run_id, stage: 'decisions' })
  })

  it('names /to-spec once the decisions critique is recorded', async () => {
    const h = await repository()
    const run = await recorded(h, { feature: 'Widgets are frobnicated.', flow: 'draft:widgets', stage: 'decisions' })

    const result = await stateOf(h, { draft: 'widgets', spec: undefined })

    expect(result.next).toMatchObject({ command: '/to-spec', kind: 'command', skill: '/to-spec' })
    expect(result.state.stages.decisions).toEqual({ latest_run_id: run.run_id, run_ids: [run.run_id], status: 'recorded' })
    expect(result.state.grilled).toBe(true)
  })

  it('skips the decisions critique when critique is skipped on purpose', async () => {
    const h = await repository()

    const result = await stateOf(h, { draft: 'widgets', record: { grilled: true, no_critique: true }, spec: undefined })

    expect(result.next).toMatchObject({ command: '/to-spec' })
    expect(result.state.no_critique).toBe(true)
  })

  it('carries the draft into the spec flow it grew into', async () => {
    const h = await repository()
    const run = await recorded(h, { feature: 'Widgets are frobnicated.', flow: 'draft:widgets', stage: 'decisions' })

    const linked = await stateOf(h, { draft: 'widgets' })
    const later = await stateOf(h)

    expect(linked.state).toMatchObject({ draft: 'widgets', flow: SPEC, spec: SPEC })
    expect(later.state.draft).toBe('widgets')
    expect(later.state.stages.decisions.run_ids).toEqual([run.run_id])
  })
})

describe('planning, once the spec exists', () => {
  it('runs the spec critique first', async () => {
    const h = await repository()

    const result = await stateOf(h)

    expect(result.next).toMatchObject({ kind: 'stage', stage: 'spec' })
    expect(result.state).toMatchObject({ grilled: true, phase: 'planning' })
  })

  it("leaves out another spec's runs", async () => {
    const h = await repository()
    await h.writeText(join(h.repo, '.scratch/gadgets/spec.md'), '# Gadgets\n\n## User Stories\n\n1. As the owner, I want gadgets.\n')
    await recorded(h, { spec: '.scratch/gadgets/spec.md', stage: 'spec' })

    const result = await stateOf(h)

    expect(result.state.stages.spec).toMatchObject({ run_ids: [], status: 'not-run' })
  })

  it('finishes a spec run that was prepared but not recorded', async () => {
    const h = await repository()
    const run = await prepared(h, { stage: 'spec' })

    const result = await stateOf(h)

    expect(result.next).toMatchObject({ kind: 'stage', resume_run_id: run.run_id, stage: 'spec' })
    expect(result.state.stages.spec.status).toBe('in-progress')
  })

  it('runs a second spec round after a first round accepted more than three critical findings', async () => {
    const h = await repository()
    const findings = [1, 2, 3, 4].map(critical)
    const run = await recorded(h, { stage: 'spec' }, findings, findings.map((f) => ({ finding_id: f.id, verdict: 'accepted' })))

    const result = await stateOf(h)

    expect(result.state.stages.spec.status).toBe('second-round-due')
    expect(result.next).toMatchObject({ kind: 'stage', previous_run_id: run.run_id, stage: 'spec' })
  })

  it('moves on once the second round is recorded', async () => {
    const h = await repository()
    const findings = [1, 2, 3, 4].map(critical)
    const first = await recorded(h, { stage: 'spec' }, findings, findings.map((f) => ({ finding_id: f.id, verdict: 'accepted' })))
    const second = await recorded(h, { previous_run_dir: first.run_dir, stage: 'spec' })

    const result = await stateOf(h)

    expect(result.state.stages.spec).toEqual({ latest_run_id: second.run_id, run_ids: [first.run_id, second.run_id], status: 'recorded' })
    expect(result.next).toMatchObject({ command: '/to-tickets' })
  })

  it('names /grill-me while a critical question is unanswered, and moves on once it is answered', async () => {
    const h = await repository()
    const run = await recorded(h, { stage: 'spec' }, [critical(1)], [{ finding_id: 'F1', verdict: 'needs-human' }])

    const asked = await stateOf(h)
    const answered = await stateOf(h, { record: { answered: [{ finding_id: 'F1', run_id: run.run_id }] } })
    const later = await stateOf(h)

    expect(asked.state.open_questions).toEqual([{ finding_id: 'F1', run_id: run.run_id, severity: 'critical', stage: 'spec' }])
    expect(asked.next).toMatchObject({ command: '/grill-me', kind: 'command', reason: expect.stringContaining('F1') as unknown })
    expect(answered.state.open_questions).toEqual([])
    expect(answered.next).toMatchObject({ command: '/to-tickets' })
    expect(later.state.open_questions).toEqual([])
  })

  it('is unchanged by any number of adr-review runs, their questions and their spend', async () => {
    const h = await repository()
    await recorded(h, { stage: 'spec' })
    await h.writeText(join(h.repo, 'docs', 'adr', '0001-nightly-job.md'), '# Nightly job\n\nWidgets are frobnicated nightly.\n')
    const before = (await stateOf(h)).state
    const question = finding({
      evidence: [
        { file: 'artifact/docs/adr/0001-nightly-job.md', kind: 'quote', value: 'Widgets are frobnicated nightly.' },
        { file: 'references/CONTEXT.md', kind: 'quote', value: DECISION_CANARY },
      ],
      location: 'docs/adr/0001-nightly-job.md',
      severity: 'critical',
    })

    for (let n = 0; n < 2; n += 1) {
      h.clock.advance(1)
      const review = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, stage: 'adr-review' })
      if (review.status !== 'ready') throw new Error('adr-review not ready')
      h.process.queue(critic([question]))
      await h.commands.fanOut.run({ run_dir: review.run_dir })
      await h.commands.record.run({ run_dir: review.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'needs-human' }] })
    }
    const after = (await stateOf(h)).state

    const questions = (await h.readText(join(h.home, LEDGER)))
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line) as Record<string, unknown>)
      .filter((line) => line.type === 'finding' && line.stage === 'adr-review' && line.verdict === 'needs-human')
    expect(questions).toHaveLength(2)
    expect({ ...after, updated_at: before.updated_at }).toEqual(before)
  })

  it('counts a question the owner answered before the run was recorded as answered', async () => {
    const h = await repository()
    await recorded(h, { stage: 'spec' }, [critical(1)], [{ answered: true, finding_id: 'F1', verdict: 'needs-human' }])

    const result = await stateOf(h)

    expect(result.state.open_questions).toEqual([])
  })

  it('lists a minor open question without holding the flow on it', async () => {
    const h = await repository()
    await recorded(h, { stage: 'spec' }, [finding()], [{ finding_id: 'F1', verdict: 'needs-human' }])

    const result = await stateOf(h)

    expect(result.state.open_questions).toHaveLength(1)
    expect(result.next).toMatchObject({ command: '/to-tickets' })
  })

  it('names /to-tickets once the spec critique is recorded', async () => {
    const h = await repository()
    await recorded(h, { stage: 'spec' })

    expect(await next(h)).toEqual({
      command: '/to-tickets',
      kind: 'command',
      reason: 'The spec is settled, so it can be cut into tickets.',
      skill: '/to-tickets',
    })
  })

  it('runs the ticket check once the tickets are cut', async () => {
    const h = await repository()
    await recorded(h, { stage: 'spec' })
    await twoTickets(h)

    expect(await next(h)).toMatchObject({ kind: 'stage', stage: 'tickets' })
  })

  it('runs the ticket check again while the tickets hold a dependency cycle', async () => {
    const h = await repository()
    await recorded(h, { stage: 'spec' })
    await ticket(h, STORE, '01: Store widgets', '\n**Blocked by:** 02\n')
    await ticket(h, REPORT, '02: Report widgets', '\n**Blocked by:** 01\n')
    await recorded(h, { stage: 'tickets' })

    const result = await stateOf(h)

    expect(result.state.stages.tickets.status).toBe('check-failed')
    expect(result.next).toMatchObject({ kind: 'stage', reason: expect.stringContaining('cycle') as unknown, stage: 'tickets' })
  })
})

describe('implementation', () => {
  it('lists the tickets in dependency order and names /implement for the first, in a fresh session', async () => {
    const h = await planned()

    const result = await stateOf(h)

    expect(result.state.phase).toBe('implementation')
    expect(result.state.tickets.map(({ id, landed, review }) => ({ id, landed, review }))).toEqual([
      { id: '02', landed: false, review: 'not-reviewed' },
      { id: '01', landed: false, review: 'not-reviewed' },
    ])
    expect(result.next).toEqual({
      command: `/implement ${REPORT}`,
      kind: 'command',
      reason: 'Ticket 02 has not landed and it is the next of 2 tickets left in dependency order; start it in a fresh session.',
      skill: '/implement',
    })
  })

  it('moves to the next ticket once the owner marks one landed', async () => {
    const h = await planned()

    const result = await stateOf(h, { record: { landed: ['02'] } })

    expect(result.state.tickets.map((t) => t.landed)).toEqual([true, false])
    expect(result.next).toMatchObject({
      command: `/implement ${STORE}`,
      reason: 'Ticket 01 has not landed and it is the last ticket left; start it in a fresh session.',
    })
  })

  it('counts a ticket closed on the tracker as landed', async () => {
    const h = await planned()
    await close(h, REPORT)

    expect(await next(h)).toMatchObject({ command: `/implement ${STORE}` })
  })

  it('counts a ticket whose change triage skipped as low-risk as landed', async () => {
    const h = await planned()
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widgets = 1\n')
    await h.git.commit()
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widgets = 2\n')
    await h.git.commit()
    h.process.queue({
      agent: 'critic-mistral-small',
      exitCode: 0,
      exportJson: { ...isolatedExport({ active_model: 'critic-mistral-small' }), cost_usd: 0.002 },
      stdout: vibeStdout('no'),
    })
    h.clock.advance(1)
    const triage = await h.commands.triage.run({ base: 'HEAD~1', repo: h.repo, ticket: REPORT })
    if (triage.status !== 'low-risk') throw new Error(`triage said ${triage.status}`)

    const result = await stateOf(h)

    expect(result.state.tickets[0]).toMatchObject({ id: '02', landed: true, review: 'skipped-low-risk', review_run_ids: [triage.run_id] })
  })

  it("runs the ticket review again while a ticket's review left a critical finding unfixed", async () => {
    const h = await planned({ families: ['mistral', 'zhipu'] })
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widgets = 2\n')
    await h.git.commit()
    const bug = finding({ category: 'bug', evidence: [{ kind: 'failing-test', value: 'expect(widgets).toBe(1)' }], location: 'tree/src/widget.ts:1', severity: 'critical' })
    const verdict = { finding_id: 'F1', reason_code: 'test-fails', regression_test: 'test/widget.test.ts', verdict: 'accepted' }
    const run = await recorded(h, { base: 'HEAD~1', stage: 'ticket-review', ticket: REPORT }, [bug], [verdict])

    const result = await stateOf(h)

    expect(result.state.tickets[0]).toMatchObject({ id: '02', landed: false, review: 'findings-open', review_run_ids: [run.run_id] })
    expect(result.next).toMatchObject({ kind: 'stage', stage: 'ticket-review', ticket: '02' })
  })

  it('takes the latest review of a ticket: one that passed after its finding was fixed', async () => {
    const h = await planned({ families: ['mistral', 'zhipu'] })
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widgets = 2\n')
    await h.git.commit()
    const bug = finding({ category: 'bug', evidence: [{ kind: 'failing-test', value: 'expect(widgets).toBe(1)' }], location: 'tree/src/widget.ts:1', severity: 'critical' })
    const verdict = { finding_id: 'F1', reason_code: 'test-fails', regression_test: 'test/widget.test.ts', verdict: 'accepted' }
    const failed = await recorded(h, { base: 'HEAD~1', stage: 'ticket-review', ticket: REPORT }, [bug], [verdict])
    const passed = await recorded(h, { base: 'HEAD~1', stage: 'ticket-review', ticket: REPORT })

    const result = await stateOf(h)

    expect(result.state.tickets[0]).toMatchObject({ landed: true, review: 'reviewed', review_run_ids: [failed.run_id, passed.run_id] })
  })

  it('names /grill-me before the next /implement while a ticket review left a critical question open', async () => {
    const h = await planned({ families: ['mistral', 'zhipu'] })
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widgets = 2\n')
    await h.git.commit()
    const unclear = finding({
      category: 'bug',
      check: 'bug',
      evidence: [{ kind: 'trace', value: 'tree/src/widget.ts:1 sets two widgets where the ticket may mean one.' }],
      location: 'tree/src/widget.ts:1',
      severity: 'critical',
    })
    await recorded(h, { base: 'HEAD~1', stage: 'ticket-review', ticket: REPORT }, [unclear], [{ finding_id: 'F1', verdict: 'needs-human' }])

    const step = await next(h)

    expect(step).toMatchObject({ command: '/grill-me', reason: expect.stringContaining('before implementing') as unknown })
  })

  it('counts a ticket whose review passed as reviewed and landed', async () => {
    const h = await planned()
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widgets = 2\n')
    await h.git.commit()
    await recorded(h, { base: 'HEAD~1', stage: 'ticket-review', ticket: REPORT })

    const result = await stateOf(h)

    expect(result.state.tickets[0]).toMatchObject({ id: '02', landed: true, review: 'reviewed' })
  })

  it('takes the first waiting ticket when every waiting ticket is blocked', async () => {
    const h = await repository({ families: ['mistral'] })
    await recorded(h, { stage: 'spec' })
    await ticket(h, STORE, '01: Store widgets', '\n**Blocked by:** 02\n')
    await ticket(h, REPORT, '02: Report widgets', '\n**Blocked by:** 01, #999\n')
    await recorded(h, { stage: 'tickets' })
    await stateOf(h, { record: { no_critique: true } })

    const result = await stateOf(h)

    expect(result.state.tickets.map((t) => t.id)).toEqual(['01', '02'])
    expect(result.next).toMatchObject({ command: `/implement ${STORE}` })
  })
})

describe('finishing', () => {
  /** A planned flow whose two tickets have both landed on a feature branch. */
  async function landed(): Promise<Harness> {
    const h = await planned()
    await h.git.commit()
    await h.writeText(join(h.repo, 'src/store.ts'), 'export function store(count: number): string {\n  return String(count)\n}\n')
    await h.writeText(join(h.repo, 'src/report.ts'), "import { store } from './store'\n\nexport const total: number = store(3)\n")
    await h.git.commit()
    await stateOf(h, { record: { landed: ['01', '02'] } })
    return h
  }

  it('runs the feature review once every ticket has landed', async () => {
    const h = await landed()

    const result = await stateOf(h)

    expect(result.state.phase).toBe('finishing')
    expect(result.next).toMatchObject({ kind: 'stage', stage: 'feature-review' })
  })

  it('finishes a feature review that was prepared but not recorded', async () => {
    const h = await landed()
    const run = await prepared(h, { base: 'HEAD~1', stage: 'feature-review' })

    expect(await next(h)).toMatchObject({ kind: 'stage', resume_run_id: run.run_id, stage: 'feature-review' })
  })

  it('is done once the gate passes for the current head', async () => {
    const h = await landed()
    const run = await recorded(h, { base: 'HEAD~1', stage: 'feature-review' })

    const result = await stateOf(h)

    expect(result.state.gate).toEqual({ head_commit: result.state.head_commit, line: 'GATE: PASS', run_id: run.run_id, status: 'pass' })
    expect(result.next).toEqual({ kind: 'done', reason: 'GATE: PASS for the current head: the feature is ready for its pull request.' })
  })

  it('reviews the feature again once the head has moved past a passing gate', async () => {
    const h = await landed()
    await recorded(h, { base: 'HEAD~1', stage: 'feature-review' })
    await h.writeText(join(h.repo, 'src/store.ts'), 'export function store(count: number): number {\n  return count\n}\n')
    await h.git.commit()

    expect(await next(h)).toMatchObject({ kind: 'stage', reason: expect.stringContaining('earlier head') as unknown, stage: 'feature-review' })
  })

  it('implements the follow-up a failing gate created, then reviews the feature again', async () => {
    const h = await landed()
    const followUp = `${ISSUES}/03-fix-the-seam.md`
    await ticket(h, followUp, '03: Fix the seam')
    const seam = finding({
      category: 'integration',
      evidence: [{ kind: 'trace', value: 'tree/src/report.ts:3 assigns store(3) to a number; tree/src/store.ts:2 returns String(count).' }],
      location: 'tree/src/report.ts:3',
      severity: 'critical',
    })
    const accepted = { finding_id: 'F1', follow_up: '03', reason_code: 'cross-ticket-trace', trace_tickets: ['tickets/01-store.md', 'tickets/02-report.md'], verdict: 'accepted' }
    await recorded(h, { base: 'HEAD~1', stage: 'feature-review' }, [seam], [accepted])

    const failing = await stateOf(h)
    await close(h, followUp)
    const fixed = await stateOf(h)

    expect(failing.state.gate).toMatchObject({ line: 'GATE: FAIL (1 critical)', status: 'fail' })
    expect(failing.next).toMatchObject({ command: `/implement ${followUp}` })
    expect(fixed.state.phase).toBe('finishing')
    expect(fixed.next).toMatchObject({ kind: 'stage', reason: expect.stringContaining('GATE: FAIL (1 critical)') as unknown, stage: 'feature-review' })
  })

  it('reviews the feature again when its review left no gate', async () => {
    const h = await landed()
    const run = await recorded(h, { base: 'HEAD~1', stage: 'feature-review' })
    await h.removeFile(join(h.repo, '.git', 'critique', 'gates.jsonl'))

    const result = await stateOf(h)

    expect(result.state.stages['feature-review'].run_ids).toEqual([run.run_id])
    expect(result.next).toMatchObject({ reason: 'The feature review left no gate: review the feature again.', stage: 'feature-review' })
  })
})

describe('no-critique', () => {
  it('steps through the plain Pocock flow and records that critique was skipped on purpose', async () => {
    const h = await repository()

    const choose = await stateOf(h, { record: { no_critique: true } })
    await twoTickets(h)
    const implement = await stateOf(h)
    const done = await stateOf(h, { record: { landed: ['01', '02'] } })

    expect(choose.state.no_critique).toBe(true)
    expect(choose.next).toMatchObject({ command: '/to-tickets' })
    expect(implement.next).toMatchObject({ command: `/implement ${REPORT}` })
    expect(done.next).toEqual({ kind: 'done', reason: 'Every ticket has landed, and critique was skipped on purpose, so there is no feature review to run.' })
  })

  it('can be turned back off', async () => {
    const h = await repository()
    await stateOf(h, { record: { no_critique: true } })

    expect(await next(h, { record: { no_critique: false } })).toMatchObject({ kind: 'stage', stage: 'spec' })
  })
})

describe('the state file', () => {
  it('lives in the critique folder of the git common directory, keyed by the spec', async () => {
    const h = await repository()

    const result = await stateOf(h)

    expect(result.state_path).toBe(flowStatePath(join(h.repo, '.git', 'critique'), SPEC))
    expect(result.state_path).toMatch(/\/\.git\/critique\/flows\/scratch-widgets-spec-md-[0-9a-f]{12}\.json$/)
    expect(parseFlowState(await h.readText(result.state_path))).toEqual(result.state)
    expect(result.source).toBe('rebuilt')
  })

  it('reads back as the state it holds, tickets and open questions included', async () => {
    const h = await repository()
    await recorded(h, { stage: 'spec' }, [finding()], [{ finding_id: 'F1', verdict: 'needs-human' }])
    await twoTickets(h)

    const result = await stateOf(h)

    expect(result.state.tickets).toHaveLength(2)
    expect(result.state.open_questions).toHaveLength(1)
    expect(parseFlowState(await h.readText(result.state_path))).toEqual(result.state)
  })

  it('is rebuilt the same from the ledger, the run directories and the tracker after it is deleted', async () => {
    const h = await planned()
    await stateOf(h, { record: { landed: ['02'] } })
    const before = await stateOf(h)

    await h.removeFile(before.state_path)
    const after = await stateOf(h)

    expect(before.source).toBe('refreshed')
    expect(after.source).toBe('rebuilt')
    expect(after.state).toEqual(before.state)
    expect(after.next).toEqual(before.next)
  })

  it('still counts a recorded run whose directory is gone, from the ledger', async () => {
    const h = await repository()
    const run = await recorded(h, { stage: 'spec' })
    await h.removeFile(join(run.run_dir, 'manifest.json'))

    const result = await stateOf(h)

    expect(result.state.stages.spec).toMatchObject({ run_ids: [run.run_id], status: 'recorded' })
  })

  it('is returned as saved when the caller asks for the cache', async () => {
    const h = await repository()
    const saved = await stateOf(h)
    await recorded(h, { stage: 'spec' })

    const cached = await stateOf(h, { cached: true })

    expect(cached.source).toBe('cache')
    expect(cached.state).toEqual(saved.state)
  })

  it('is rebuilt when the cache asked for does not read as a flow state', async () => {
    const h = await repository()
    const saved = await stateOf(h)
    await h.writeText(saved.state_path, '{"version": 2}')

    const cached = await stateOf(h, { cached: true })

    expect(cached.source).toBe('refreshed')
    expect(cached.state.version).toBe(1)
  })

  it("holds the flow's Vibe spend, its draft's included", async () => {
    const h = await repository()
    await recorded(h, { feature: 'Widgets are frobnicated.', flow: 'draft:widgets', stage: 'decisions' })
    await recorded(h, { stage: 'spec' })

    const result = await stateOf(h, { draft: 'widgets' })

    expect(result.state.vibe_spend_usd).toBeCloseTo(0.5)
  })

  it('keeps the owner’s choices as flow lines in the ledger', async () => {
    const h = await repository()

    await stateOf(h, { record: { no_critique: true } })

    const lines = (await readFile(join(h.home, LEDGER), 'utf8')).trim().split('\n').map((line) => JSON.parse(line) as unknown)
    expect(lines).toEqual([
      { event: 'no-critique', flow: SPEC, recorded_at: '2026-10-30T12:00:00.000Z', repository: 'github.com/jorisjonkers-dev/agent-kit', type: 'flow', value: true },
    ])
  })
})

describe('a spec on GitHub', () => {
  it('keys the flow by the issue and names /implement with the ticket number', async () => {
    const repo = 'github.com/jorisjonkers-dev/agent-kit'
    const h = await repository({
      issues: {
        [`${repo}#167`]: specIssue(),
        [`${repo}#170`]: ticketIssue(170, 'Store widgets', '## Acceptance criteria\n\n- [ ] It works.\n'),
      },
      links: { subIssues: { [`${repo}#167`]: [170] } },
      tracker: 'github',
    })

    const result = await stateOf(h, { record: { no_critique: true }, spec: '#167' })

    expect(result.state.flow).toBe(`${repo}#167`)
    expect(result.next).toMatchObject({ command: '/implement #170' })
  })
})

describe('refusals and errors', () => {
  it('needs a spec or a draft name', async () => {
    const h = await repository()

    await expect(h.commands.state.run({ repo: h.repo })).rejects.toMatchObject({ code: 'invalid-input' })
  })

  it('refuses a draft name that could not be a file name', async () => {
    const h = await repository()

    await expect(h.commands.state.run({ draft: '../x', repo: h.repo })).rejects.toMatchObject({ code: 'invalid-input' })
  })

  it('refuses to mark a ticket landed that is not a ticket of the spec', async () => {
    const h = await planned()

    await expect(stateOf(h, { record: { landed: ['07'] } })).rejects.toMatchObject({ code: 'invalid-input' })
  })

  it('refuses to answer a question that is not open', async () => {
    const h = await repository()

    await expect(stateOf(h, { record: { answered: [{ finding_id: 'F1', run_id: 'run-1' }] } })).rejects.toMatchObject({ code: 'invalid-input' })
  })

  it('refuses a repository owned by nedap, as every command does', async () => {
    const h = await harness({ remotes: [{ name: 'origin', url: 'git@github.com:nedap/widgets.git' }] })
    open.push(h)

    const result = (await h.commands.state.run({ repo: h.repo, spec: SPEC })) as Refusal

    expect(result).toMatchObject({ reason: 'nedap', status: 'refused' })
  })

  it('refuses a tracker it cannot read', async () => {
    const h = await repository({ tracker: 'gitlab' })

    expect(await h.commands.state.run({ repo: h.repo, spec: SPEC })).toMatchObject({ reason: 'unsupported-tracker', status: 'refused' })
  })

  it("refuses a spec whose tickets the tracker cannot read", async () => {
    const repo = 'github.com/jorisjonkers-dev/agent-kit'
    const h = await repository({ issues: { [`${repo}#167`]: specIssue() }, links: { subIssues: { [`${repo}#167`]: 'HTTP 502' } }, tracker: 'github' })

    expect(await h.commands.state.run({ repo: h.repo, spec: '#167' })).toMatchObject({ reason: 'tracker-unreachable', status: 'refused' })
  })

  it('refuses a spec issue gh cannot read', async () => {
    const h = await repository({ tracker: 'github' })

    expect(await h.commands.state.run({ repo: h.repo, spec: '#167' })).toMatchObject({ reason: 'tracker-unreachable', status: 'refused' })
  })

  it('fails on a gate index line it cannot read', async () => {
    const h = await repository()
    await h.writeText(join(h.repo, '.git', 'critique', 'gates.jsonl'), '{not json\n')

    await expect(stateOf(h)).rejects.toMatchObject({ code: 'invalid-run' })
  })

  it('fails on a ledger line it cannot read', async () => {
    const h = await repository()
    await h.writeText(join(h.home, LEDGER), '{not json\n')

    await expect(stateOf(h)).rejects.toMatchObject({ code: 'invalid-ledger' })
  })
})
