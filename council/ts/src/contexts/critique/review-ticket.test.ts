import { createHash } from 'node:crypto'
import { readdir, readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  ASSETS_ROOT,
  FakeGit,
  fenced,
  harness,
  isolatedExport,
  ticketIssue,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import { parseTicketReviewRecord, StagedDiffReader, ticketReviewIndexPath, ticketReviewPath, type PrepareResult, type TriageResult } from './index.js'

/**
 * What `/review-ticket` (#188) relies on the runner for: the ticket-review
 * roster in shipped routing, the `diff-reviewer` role, a review of the
 * staged change with a hash of it a commit gate can match, the evidence rule
 * for accepting a ticket finding, and the summary's "do not commit" while an
 * accepted critical finding is unfixed.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const SPEC = '.scratch/widgets/spec.md'
const TICKET = '.scratch/widgets/issues/01-frobnicate.md'
const WIDGET_BEFORE = 'export function frobnicate(widgets: number[]): number {\n  return widgets.length\n}\n'
const WIDGET_AFTER = 'export function frobnicate(widgets: number[]): number {\n  return widgets.length - 1\n}\n'

/**
 * A repository opted in for code with every family, on local markdown, whose
 * ticket's change is committed: `c2` is the base, `c3` the head.
 */
async function ticketRepository(
  options: Parameters<typeof harness>[0] & { readonly families?: readonly string[] } = {},
): Promise<Harness> {
  const h = await harness(options)
  open.push(h)
  await h.commands.allow.run({ code: true, families: options.families ?? ['mistral', 'google', 'zhipu', 'deepseek'], repo: h.repo })
  await h.writeText(join(h.repo, TICKET), '# 01: Frobnicate widgets\n\n## Acceptance criteria\n\n- [ ] Frobnicate counts every widget.\n')
  await h.writeText(join(h.repo, 'src/widget.ts'), WIDGET_BEFORE)
  await h.git.commit()
  await h.writeText(join(h.repo, 'src/widget.ts'), WIDGET_AFTER)
  await h.git.commit()
  return h
}

async function prepared(h: Harness, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const result = await h.commands.prepare.run({
    base: 'HEAD~1',
    orchestrator_family: 'anthropic',
    repo: h.repo,
    spec: SPEC,
    stage: 'ticket-review',
    ticket: TICKET,
    ...extra,
  })
  if (result.status !== 'ready') throw new Error(`prepare refused: ${result.message}`)
  return result
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

/** A critic that raises `findings`, or says what it checked when it found nothing. */
function critic(findings: readonly Record<string, unknown>[]): VibeScript {
  const review = findings.length === 0 ? { findings, no_findings_reason: 'Read the diff and the code around it.' } : { findings }
  return { exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced(review)) }
}

describe('the ticket-review roster in shipped routing', () => {
  it('runs one Gemini Flash critic as diff reviewer on failure modes and security', async () => {
    const h = await ticketRepository()

    const run = await prepared(h)

    expect(run.critics).toEqual([
      {
        client: 'agy-gemini-flash-high',
        critic_id: 'c1-agy-gemini-flash-high',
        family: 'google',
        lenses: ['failure-modes', 'security'],
        model: 'gemini-3.8-flash-high',
        role: 'diff-reviewer',
      },
    ])
  })

  it('caps the critic at $0.50 and 8 minutes', async () => {
    const h = await ticketRepository()

    const run = await prepared(h)

    const manifest = JSON.parse(await readFile(run.manifest_path, 'utf8')) as {
      readonly critics: readonly { readonly max_price_usd: number; readonly time_limit_s: number }[]
    }
    expect(manifest.critics.map(({ max_price_usd, time_limit_s }) => ({ max_price_usd, time_limit_s }))).toEqual([
      { max_price_usd: 0.5, time_limit_s: 480 },
    ])
  })

  it('falls back to GLM when Gemini cannot run', async () => {
    const h = await ticketRepository({ agyOnPath: false })

    const run = await prepared(h)

    expect(run.critics.map(({ client, lenses, role }) => ({ client, lenses, role }))).toEqual([
      { client: 'vibe-glm', lenses: ['failure-modes', 'security'], role: 'diff-reviewer' },
    ])
    expect(run.dropped).toMatchObject([{ client: 'agy-gemini-flash-high', reason: 'unavailable', replaced_by: 'c1-vibe-glm' }])
  })

  it('falls back to GLM once only the Antigravity reserve is left', async () => {
    const h = await ticketRepository()
    const launch = {
      charged_usd: 0,
      client: 'agy-gemini-flash-high',
      critic_id: 'c1-agy-gemini-flash-high',
      family: 'google',
      flow: 'github.com/jorisjonkers-dev/agent-kit#12',
      kind: 'agy',
      model: 'gemini-3.8-flash-high',
      recorded_at: '2026-10-30T08:00:00.000Z',
      repository: 'github.com/jorisjonkers-dev/agent-kit',
      type: 'invocation',
    }
    await h.writeText(join(h.home, '.local/share/agent-kit/critique/ledger.jsonl'), `${JSON.stringify(launch)}\n`.repeat(10))

    const run = await prepared(h)

    expect(run.critics.map((c) => c.client)).toEqual(['vibe-glm'])
    expect(run.dropped).toMatchObject([{ client: 'agy-gemini-flash-high', reason: 'quota' }])
  })

  it('adds a critic from a further family at deep depth', async () => {
    const h = await ticketRepository()

    const run = await prepared(h, { depth: 'deep' })

    expect(run.critics.map(({ client, family, role }) => ({ client, family, role }))).toEqual([
      { client: 'agy-gemini-flash-high', family: 'google', role: 'diff-reviewer' },
      { client: 'vibe-mistral-large', family: 'mistral', role: 'diff-reviewer' },
    ])
  })

  it('keeps the deep critic in a distinct family when GLM took the base slot', async () => {
    const h = await ticketRepository({ agyOnPath: false })

    const run = await prepared(h, { depth: 'deep' })

    expect(run.critics.map((c) => c.family)).toEqual(['zhipu', 'mistral'])
  })

  it('goes deep when the change touches more risk paths than routing allows', async () => {
    const h = await ticketRepository()
    for (const name of ['session', 'login', 'permissions', 'rbac']) await h.writeText(join(h.repo, 'src/auth', `${name}.ts`), 'export {}\n')
    await h.git.commit()

    const run = await prepared(h)

    expect(run.depth_choice.signals).toEqual([{ kind: 'threshold', measure: 'risk_path_hits', threshold: 3, value: 4 }])
  })

  it('goes deep when the change is larger than routing allows', async () => {
    const h = await ticketRepository()
    await h.writeText(join(h.repo, 'src/big.ts'), Array.from({ length: 601 }, (_, i) => `export const n${String(i)} = ${String(i)}`).join('\n'))
    await h.git.commit()

    const run = await prepared(h)

    expect(run.depth_choice.signals).toEqual([{ kind: 'threshold', measure: 'changed_lines', threshold: 600, value: 601 }])
  })
})

describe('the diff-reviewer role', () => {
  it("is the role a ticket review's critic is given", async () => {
    const h = await ticketRepository({ families: ['mistral', 'zhipu'], scripts: [critic([])] })
    const run = await prepared(h)

    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const prompt = await readFile(join(run.run_dir, 'critics', 'c1-vibe-glm', 'prompt.md'), 'utf8')
    expect(prompt).toContain('# Role: diff reviewer')
  })

  it('has its hash recorded in the ledger', async () => {
    const h = await ticketRepository({ families: ['mistral', 'zhipu'], scripts: [critic([])] })
    const run = await prepared(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    const role = await readFile(join(ASSETS_ROOT, 'roles', 'diff-reviewer.md'), 'utf8')
    const [line] = await ledgerLines(h)
    expect(line).toMatchObject({ prompt_hashes: { role: createHash('sha256').update(role).digest('hex') }, role: 'diff-reviewer' })
  })
})

/**
 * The ticket's change as `/review-ticket` finds it after `/code-review`:
 * `c2` committed, the change staged on top of it, and a file the owner has
 * not staged.
 */
async function stagedRepository(options: Parameters<typeof ticketRepository>[0] = {}): Promise<Harness> {
  const h = await harness(options)
  open.push(h)
  await h.commands.allow.run({ code: true, families: options.families ?? ['mistral', 'google', 'zhipu', 'deepseek'], repo: h.repo })
  await h.writeText(join(h.repo, TICKET), '# 01: Frobnicate widgets\n\n## Acceptance criteria\n\n- [ ] Frobnicate counts every widget.\n')
  await h.writeText(join(h.repo, 'src/widget.ts'), WIDGET_BEFORE)
  await h.git.commit()
  await h.writeText(join(h.repo, 'src/widget.ts'), WIDGET_AFTER)
  await h.git.stage()
  await h.writeText(join(h.repo, 'src/notes.ts'), '// not staged\n')
  return h
}

const STAGED = { base: 'HEAD', staged: true }

interface StagedManifest {
  readonly code: { readonly base_commit: string; readonly head_commit: string; readonly staged_diff?: string }
}

async function stagedHash(run: PrepareResult): Promise<string | undefined> {
  return (JSON.parse(await readFile(run.manifest_path, 'utf8')) as StagedManifest).code.staged_diff
}

describe('reviewing the staged change', () => {
  it('freezes the staged files, not the unstaged ones', async () => {
    const h = await stagedRepository()

    const run = await prepared(h, STAGED)

    expect(await readFile(join(run.packet_dir, 'tree/src/widget.ts'), 'utf8')).toBe(WIDGET_AFTER)
    expect(run.code).toMatchObject({ base_commit: 'c2', changed_files: ['tree/src/widget.ts'] })
    expect(await readFile(join(run.packet_dir, 'tree/src/notes.ts'), 'utf8').catch(() => 'absent')).toBe('absent')
  })

  it('reviews a commit made of the index, and moves no HEAD', async () => {
    const h = await stagedRepository()

    const run = await prepared(h, STAGED)

    expect(run.code?.head_commit).toBe('s1')
    expect(await h.git.resolveCommit(h.repo, 'HEAD')).toBe('c2')
  })

  it('records a hash of the staged change in the manifest', async () => {
    const h = await stagedRepository()

    const run = await prepared(h, STAGED)

    expect(await stagedHash(run)).toMatch(/^[0-9a-f]{64}$/)
  })

  it('gives the same staged change the same hash, and a restaged one another', async () => {
    const h = await stagedRepository()
    const first = await prepared(h, STAGED)
    const again = await prepared(h, STAGED)
    await h.writeText(join(h.repo, 'src/widget.ts'), WIDGET_BEFORE.replace('length', 'length + 0'))
    await h.git.stage()

    const restaged = await prepared(h, STAGED)

    expect(await stagedHash(again)).toBe(await stagedHash(first))
    expect(await stagedHash(restaged)).not.toBe(await stagedHash(first))
  })

  it('records no staged hash when it reviews a commit', async () => {
    const h = await ticketRepository()

    const run = await prepared(h)

    expect(await stagedHash(run)).toBeUndefined()
  })

  it('takes staged or head, not both', async () => {
    const h = await stagedRepository()

    await expect(prepared(h, { ...STAGED, head: 'HEAD' })).rejects.toMatchObject({
      code: 'invalid-input',
      message: 'staged reviews the index: leave out head',
    })
  })

  it('reviews a staged change at ticket review only', async () => {
    const h = await stagedRepository()

    await expect(prepared(h, { ...STAGED, stage: 'feature-review' })).rejects.toMatchObject({
      code: 'invalid-input',
      message: 'only ticket-review takes staged; feature-review reviews commits',
    })
  })
})

/** The triage model's answer, for a change no risk rule fires on. */
function triageAnswer(text: string): VibeScript {
  return {
    agent: 'critic-mistral-small',
    exitCode: 0,
    exportJson: { ...isolatedExport({ active_model: 'critic-mistral-small' }), cost_usd: 0.0021 },
    stdout: vibeStdout(text),
  }
}

/** Triages the ticket's change; when no risk rule fires, the triage model answers no. */
async function triaged(h: Harness, input: Record<string, unknown>): Promise<TriageResult> {
  h.process.queue(triageAnswer('no'))
  const result = await h.commands.triage.run({ repo: h.repo, ticket: TICKET, ...input })
  if (result.status === 'refused') throw new Error(`triage refused: ${result.message}`)
  return result
}

describe('triage of the staged change', () => {
  it('ends a low-risk ticket with one line naming the signal, and launches no critic', async () => {
    const h = await stagedRepository()

    const result = await triaged(h, STAGED)

    expect(result.summary).toBe('Triage: low-risk: no risk rule fired, and the triage model (vibe-mistral-small) answered no, so the review is skipped.')
    expect(h.process.commands.map((command) => command.args[command.args.indexOf('--agent') + 1])).toEqual(['critic-mistral-small'])
    expect(h.agy.launches).toEqual([])
  })

  it('reads the staged change, not the unstaged one', async () => {
    const h = await stagedRepository()
    await h.writeText(join(h.repo, 'src/auth/session.ts'), 'export const unstaged = true\n')

    const result = await triaged(h, STAGED)

    expect(result.change).toMatchObject({ base_commit: 'c2', changed_files: 1, head_commit: 's1' })
    expect(result.signals.map((signal) => signal.kind)).not.toContain('risk-path')
  })

  it('calls a staged change risky when it touches a risk path', async () => {
    const h = await stagedRepository()
    await h.writeText(join(h.repo, 'src/auth/session.ts'), 'export const staged = true\n')
    await h.git.stage()

    const result = await triaged(h, STAGED)

    expect(result).toMatchObject({ signal: { kind: 'risk-path', path: 'src/auth/session.ts' }, status: 'risky' })
  })

  it('takes staged or head, not both', async () => {
    const h = await stagedRepository()

    await expect(triaged(h, { ...STAGED, head: 'HEAD' })).rejects.toMatchObject({
      code: 'invalid-input',
      message: 'staged reviews the index: leave out head',
    })
  })
})

describe("a ticket review's flow", () => {
  it("is the spec's by default, so its spend counts with the spec's other stages", async () => {
    const h = await stagedRepository()

    const run = await prepared(h, STAGED)

    const manifest = JSON.parse(await readFile(run.manifest_path, 'utf8')) as { readonly flow: string }
    expect(manifest.flow).toBe(SPEC)
  })

  it("is the spec's for triage when triage names the spec", async () => {
    const h = await stagedRepository()

    const result = await triaged(h, { ...STAGED, spec: SPEC })

    expect(result.flow).toBe(SPEC)
  })

  it('is still the one the caller names', async () => {
    const h = await stagedRepository()

    const result = await triaged(h, { ...STAGED, flow: 'flow-of-my-own', spec: SPEC })

    expect(result.flow).toBe('flow-of-my-own')
  })

  it('is refused for triage, before anything is written, when the spec cannot be read', async () => {
    const repo = 'github.com/jorisjonkers-dev/agent-kit'
    const h = await stagedRepository({
      issues: { [`${repo}#181`]: 'gh: To get started with GitHub CLI, please run:  gh auth login', [`${repo}#186`]: ticketIssue(186, 'Staged', 'Build it.') },
      tracker: 'github',
    })

    const result = await h.commands.triage.run({ ...STAGED, repo: h.repo, spec: '#181', ticket: '#186' })

    expect(result).toMatchObject({ reason: 'tracker-unreachable', status: 'refused' })
    expect(await readdir(join(h.repo, '.git'))).toEqual([])
  })
})

describe('reading a staged change', () => {
  it('needs a commit at HEAD to diff the index against', async () => {
    const reader = new StagedDiffReader({ git: new FakeGit('/nowhere', []) })

    await expect(reader.read('/nowhere')).rejects.toMatchObject({
      code: 'invalid-input',
      message: expect.stringMatching(/^staged needs a commit at HEAD to diff the index against: fatal: ambiguous argument 'HEAD'/) as unknown,
    })
  })
})

const PROPOSED_TEST = [
  "import { frobnicate } from '../src/widget'",
  "it('counts every widget', () => expect(frobnicate([1, 2, 3])).toBe(3))",
].join('\n')

/** The off-by-one the staged change plants, with the failing test that shows it. */
function offByOne(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    category: 'bug',
    claim: 'frobnicate counts one widget fewer than it is given.',
    evidence: [{ kind: 'failing-test', value: PROPOSED_TEST }],
    id: 'F1',
    location: 'tree/src/widget.ts:2',
    severity: 'critical',
    ...overrides,
  }
}

/** Prepares and fans out a review of the staged change whose one critic, GLM, raises `findings`. */
async function reviewed(findings: readonly Record<string, unknown>[]): Promise<{ h: Harness; run: PrepareResult }> {
  const h = await stagedRepository({ families: ['mistral', 'zhipu'] })
  h.process.queue(critic(findings))
  const run = await prepared(h, STAGED)
  await h.commands.fanOut.run({ run_dir: run.run_dir })
  return { h, run }
}

const REGRESSION_TEST = 'test/widget.test.ts'
const TEST_FAILS = { finding_id: 'F1', reason_code: 'test-fails', regression_test: REGRESSION_TEST, verdict: 'accepted' }

describe('accepting a ticket finding', () => {
  it('needs a failing test the orchestrator ran or a trace it walked', async () => {
    const { h, run } = await reviewed([offByOne()])

    await expect(h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })).rejects.toThrow(
      /F1: a ticket finding is accepted only with reason_code test-fails or trace-walked/,
    )
  })

  it('names the regression test it kept when its test failed as predicted', async () => {
    const { h, run } = await reviewed([offByOne()])

    await expect(
      h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', reason_code: 'test-fails', verdict: 'accepted' }] }),
    ).rejects.toThrow(/F1: a finding accepted by test-fails names the test kept in the ticket's change in regression_test/)
  })

  it('takes a regression test only when a test decided it', async () => {
    const { h, run } = await reviewed([offByOne()])

    await expect(h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ ...TEST_FAILS, reason_code: 'trace-walked' }] })).rejects.toThrow(
      /F1: regression_test belongs to a finding accepted by test-fails/,
    )
  })

  it('accepts a walked trace without a regression test', async () => {
    const { h, run } = await reviewed([offByOne()])

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', reason_code: 'trace-walked', verdict: 'accepted' }] })

    expect(recorded.counts.accepted).toBe(1)
  })

  it('gives a rejected finding no regression test and no fix', async () => {
    const { h, run } = await reviewed([offByOne()])

    await expect(
      h.commands.record.run({
        run_dir: run.run_dir,
        verdicts: [{ finding_id: 'F1', fixed: true, reason: 'The proposed test passes.', reason_code: 'test-passes', verdict: 'rejected' }],
      }),
    ).rejects.toThrow(/F1: only an accepted ticket finding has a regression_test or is fixed/)
  })

  it('takes neither at any other stage', async () => {
    const h = await harness({ scripts: [critic([offByOne({ evidence: [], location: 'User story 3' })])] })
    open.push(h)
    await h.commands.allow.run({ families: ['mistral'], repo: h.repo })
    const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: SPEC, stage: 'spec' })
    if (run.status !== 'ready') throw new Error('prepare refused')
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    await expect(h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ ...TEST_FAILS, fixed: true }] })).rejects.toThrow(
      /F1: regression_test and fixed belong to a ticket-review verdict/,
    )
  })

  it('keeps test-fails and test-passes in the ledger, and no test code', async () => {
    const { h, run } = await reviewed([offByOne(), offByOne({ claim: 'frobnicate fails on an empty list.', id: 'F2', severity: 'major' })])

    await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [TEST_FAILS, { finding_id: 'F2', reason: 'The proposed test passes.', reason_code: 'test-passes', verdict: 'rejected' }],
    })

    const findings = (await ledgerLines(h)).filter((line) => line.type === 'finding')
    expect(findings.map(({ reason_code, verdict }) => ({ reason_code, verdict }))).toEqual([
      { reason_code: 'test-fails', verdict: 'accepted' },
      { reason_code: 'test-passes', verdict: 'rejected' },
    ])
    const ledger = await readFile(join(h.home, '.local/share/agent-kit/critique/ledger.jsonl'), 'utf8')
    // The ledger names the ticket's path, so look for the claims themselves.
    expect(ledger).not.toContain('frobnicate counts')
    expect(ledger).not.toContain('frobnicate fails')
    expect(ledger).not.toContain(REGRESSION_TEST)
  })

  it('shows the regression test and the fix in the report', async () => {
    const { h, run } = await reviewed([offByOne()])

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ ...TEST_FAILS, fixed: true }] })

    expect(await readFile(recorded.report_path, 'utf8')).toContain(`Verdict: accepted (test-fails), regression test ${REGRESSION_TEST}, fixed`)
  })
})

describe('do not commit', () => {
  const lastLine = (summary: string) => summary.trimEnd().split('\n').at(-1)

  it('ends the summary while an accepted critical finding is unfixed', async () => {
    const { h, run } = await reviewed([offByOne()])

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [TEST_FAILS] })

    expect(lastLine(recorded.summary)).toBe('Critical finding F1 is accepted and not fixed: do not commit.')
  })

  it('names every unfixed critical finding', async () => {
    const { h, run } = await reviewed([offByOne(), offByOne({ claim: 'frobnicate fails on an empty list.', id: 'F2' })])

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [TEST_FAILS, { ...TEST_FAILS, finding_id: 'F2' }] })

    expect(lastLine(recorded.summary)).toBe('Critical findings F1, F2 are accepted and not fixed: do not commit.')
  })

  it('is gone once the finding is fixed', async () => {
    const { h, run } = await reviewed([offByOne()])

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ ...TEST_FAILS, fixed: true }] })

    expect(recorded.summary).not.toContain('do not commit')
    expect(recorded.summary).toContain(`[critical bug] tree/src/widget.ts:2: frobnicate counts one widget fewer than it is given. (test-fails, fixed)`)
  })

  it('is not said for an unfixed major finding', async () => {
    const { h, run } = await reviewed([offByOne({ severity: 'major' })])

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [TEST_FAILS] })

    expect(recorded.summary).not.toContain('do not commit')
  })
})

/** The orchestrator fixes the off-by-one and keeps the failing test, both staged in the ticket's change. */
async function fixAndKeepTest(h: Harness): Promise<void> {
  await h.writeText(join(h.repo, 'src/widget.ts'), WIDGET_BEFORE)
  await h.writeText(join(h.repo, REGRESSION_TEST), PROPOSED_TEST)
  await h.git.stage()
}

function reviewIndex(h: Harness): Promise<readonly Record<string, unknown>[]> {
  return readFile(join(h.repo, '.git', 'critique', 'ticket-reviews.jsonl'), 'utf8').then((text) =>
    text
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line) as Record<string, unknown>),
  )
}

describe('the review record a commit gate reads', () => {
  it('records the review, the commits and both staged hashes in the run', async () => {
    const { h, run } = await reviewed([offByOne()])
    const reviewed_staged_diff = await stagedHash(run)
    await fixAndKeepTest(h)

    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ ...TEST_FAILS, fixed: true }] })

    const now = await new StagedDiffReader({ git: h.git }).read(h.repo)
    expect(JSON.parse(await readFile(join(run.run_dir, 'review.json'), 'utf8'))).toEqual({
      base_commit: 'c2',
      degraded: false,
      head_commit: 's1',
      recorded_at: '2026-10-30T12:00:00.000Z',
      repository: 'github.com/jorisjonkers-dev/agent-kit',
      review: 'pass',
      reviewed_staged_diff,
      run_id: run.run_id,
      spec: SPEC,
      stage: 'ticket-review',
      staged_diff: now.sha256,
      ticket: TICKET,
      unfixed_critical: [],
      version: 1,
    })
    expect(now.sha256).not.toBe(reviewed_staged_diff)
  })

  it('fails while an accepted critical finding is unfixed', async () => {
    const { h, run } = await reviewed([offByOne()])

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [TEST_FAILS] })

    expect(recorded.ticket_review).toMatchObject({ review: 'fail', unfixed_critical: ['F1'] })
  })

  it('passes a run no critic reviewed, and says it was degraded', async () => {
    const h = await stagedRepository({ families: ['mistral', 'zhipu'] })
    h.process.queue({ exitCode: 1, stdout: '' })
    const run = await prepared(h, STAGED)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    expect(recorded.ticket_review).toMatchObject({ degraded: true, review: 'pass' })
  })

  it("appends each run's record to one index in the critique directory, oldest first", async () => {
    const { h, run } = await reviewed([offByOne()])
    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [TEST_FAILS] })
    await fixAndKeepTest(h)
    h.process.queue(critic([]))
    const again = await prepared(h, STAGED)
    await h.commands.fanOut.run({ run_dir: again.run_dir })

    await h.commands.record.run({ run_dir: again.run_dir, verdicts: [] })

    const index = await reviewIndex(h)
    expect(index.map(({ review, run_id }) => ({ review, run_id }))).toEqual([
      { review: 'fail', run_id: run.run_id },
      { review: 'pass', run_id: again.run_id },
    ])
  })

  it('reads back with parseTicketReviewRecord, for a gate that makes no model call', async () => {
    const { h, run } = await reviewed([offByOne()])
    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [TEST_FAILS] })

    const text = await readFile(ticketReviewIndexPath(join(h.repo, '.git', 'critique')), 'utf8')

    expect(parseTicketReviewRecord(text.trim())).toEqual(recorded.ticket_review)
    expect(parseTicketReviewRecord(await readFile(ticketReviewPath(run.run_dir), 'utf8'))).toEqual(recorded.ticket_review)
  })

  it('rejects a line that is not a review record', () => {
    expect(() => parseTicketReviewRecord('{"version": 1, "stage": "feature-review"}')).toThrow()
  })

  it('has no staged hashes for a review of a commit', async () => {
    const h = await ticketRepository({ families: ['mistral', 'zhipu'], scripts: [critic([])] })
    const run = await prepared(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    expect(recorded.ticket_review).toMatchObject({ head_commit: 'c3', reviewed_staged_diff: null, staged_diff: null })
  })

  it('is written by no other stage', async () => {
    const h = await harness({ scripts: [critic([])] })
    open.push(h)
    await h.commands.allow.run({ families: ['mistral'], repo: h.repo })
    const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: SPEC, stage: 'spec' })
    if (run.status !== 'ready') throw new Error('prepare refused')
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    expect(recorded.ticket_review).toBeUndefined()
    expect(await readFile(join(run.run_dir, 'review.json'), 'utf8').catch(() => 'absent')).toBe('absent')
  })

  it('refuses a ticket-review run whose manifest lost the commits it reviewed', async () => {
    const { h, run } = await reviewed([])
    const manifest = JSON.parse(await readFile(run.manifest_path, 'utf8')) as Record<string, unknown>
    await h.writeText(run.manifest_path, JSON.stringify({ ...manifest, code: undefined }))

    await expect(h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })).rejects.toMatchObject({
      code: 'invalid-run',
      message: `run ${run.run_id} is a ticket review without the commits it reviewed`,
    })
  })
})
