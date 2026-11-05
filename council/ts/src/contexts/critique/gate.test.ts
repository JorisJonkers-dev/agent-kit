import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { fenced, harness, isolatedExport, vibeStdout, writeFlowState, type Harness, type VibeScript } from '../../../test/critique/harness.js'
import type { GateResult, PrepareResult } from './index.js'

/**
 * The commit and PR gates (#197): the runner's `gate` command, which the
 * critique hooks call before a `git commit`, a `gh pr create` or a
 * `git push`. It decides from local files and git alone, with no model and
 * no network, blocks only on an explicit decision, and allows (and logs)
 * whenever it cannot decide.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const SPEC = '.scratch/widgets/spec.md'
const TICKET = '.scratch/widgets/issues/01-session.md'
const RISKY_PATH = 'src/auth/session.ts'

/** A repository opted in for code with Mistral and GLM, with its ticket committed. */
async function optedIn(options: Parameters<typeof harness>[0] & { readonly code?: boolean; readonly flow?: boolean } = {}): Promise<Harness> {
  const h = await harness(options)
  open.push(h)
  if (options.flow !== false) await writeFlowState(h)
  await h.commands.allow.run({ code: options.code ?? true, families: ['mistral', 'zhipu'], repo: h.repo })
  await h.writeText(join(h.repo, TICKET), '# 01: Sessions expire\n\n## Acceptance criteria\n\n- [ ] A session expires after a minute.\n')
  await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widgets = 1\n')
  await h.git.commit()
  return h
}

/** Stages a change to a risk path. */
async function stageRisky(h: Harness, content = 'export const ttl = 60\n'): Promise<void> {
  await h.writeText(join(h.repo, RISKY_PATH), content)
  await h.git.stage()
}

/** The payload a `PreToolUse` hook on `Bash` passes, for `command` run in `cwd`. */
function payload(h: Harness, command: string, cwd = h.repo): Record<string, unknown> {
  return { cwd, hook_event_name: 'PreToolUse', tool_input: { command }, tool_name: 'Bash' }
}

function gate(h: Harness, command: string, extra: Record<string, unknown> = {}): Promise<GateResult> {
  return h.commands.gate.run({ ...payload(h, command), ...extra })
}

/** A critic with nothing to report. */
function quietCritic(): VibeScript {
  return { exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings: [], no_findings_reason: 'Read the diff.' })) }
}

/** Runs `/review-implementation` on the staged change as the skill does, with a critic that finds nothing, so the review passes. */
async function reviewStaged(h: Harness): Promise<PrepareResult> {
  h.process.queue(quietCritic())
  const run = await h.commands.prepare.run({
    base: 'HEAD',
    orchestrator_family: 'anthropic',
    repo: h.repo,
    spec: SPEC,
    stage: 'ticket-review',
    staged: true,
    ticket: TICKET,
  })
  if (run.status !== 'ready') throw new Error(`prepare refused: ${run.message}`)
  await h.commands.fanOut.run({ run_dir: run.run_dir })
  await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })
  return run
}

function deny(line: string): unknown {
  return { hookSpecificOutput: { hookEventName: 'PreToolUse', permissionDecision: 'deny', permissionDecisionReason: line } }
}

describe('the commit gate', () => {
  it('blocks a risky staged change that no /review-implementation run has passed, in one line that names the skill', async () => {
    const h = await optedIn()
    await stageRisky(h)

    const result = await gate(h, 'git commit -m "Expire sessions"')

    const line =
      'Commit blocked: the staged change is risky (src/auth/session.ts is a risk path) and no passing /review-implementation run matches it. Run /review-implementation, then commit.'
    expect(result).toEqual({ gate: 'commit', hook_output: deny(line), line, reason: 'unreviewed', status: 'blocked' })
  })

  it('allows a low-risk staged change', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widgets = 2\n')
    await h.git.stage()

    expect(await gate(h, 'git commit -m "More widgets"')).toEqual({ gate: 'commit', line: null, reason: 'low-risk', status: 'allowed' })
  })

  it('allows the risky change once a /review-implementation run passed for it', async () => {
    const h = await optedIn()
    await stageRisky(h)
    await reviewStaged(h)

    expect(await gate(h, 'git commit -m "Expire sessions"')).toMatchObject({ reason: 'reviewed', status: 'allowed' })
  })

  it('blocks again once the staged change differs from the reviewed one', async () => {
    const h = await optedIn()
    await stageRisky(h)
    await reviewStaged(h)
    await stageRisky(h, 'export const ttl = 6000\n')

    expect(await gate(h, 'git commit -m "Expire sessions"')).toMatchObject({ reason: 'unreviewed', status: 'blocked' })
  })

  it('blocks an amend that stages more risky work after the reviewed commit', async () => {
    const h = await optedIn()
    await stageRisky(h)
    await reviewStaged(h)
    await h.git.commit()
    await stageRisky(h, 'export const ttl = 120\n')

    expect(await gate(h, 'git commit --amend --no-edit')).toMatchObject({ reason: 'unreviewed', status: 'blocked' })
  })

  it('counts a change above the size threshold as risky', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, 'src/widget.ts'), Array.from({ length: 301 }, (_, n) => `export const w${String(n)} = ${String(n)}`).join('\n'))
    await h.git.stage()

    expect(await gate(h, 'git commit -m big')).toMatchObject({
      line: 'Commit blocked: the staged change is risky (302 changed lines, above 300) and no passing /review-implementation run matches it. Run /review-implementation, then commit.',
      status: 'blocked',
    })
  })

  it('counts every risk signal in the line', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, 'src/auth/token.ts'), 'export const token = 1\n')
    await stageRisky(h)

    expect((await gate(h, 'git commit -m auth')).line).toContain('(src/auth/session.ts is a risk path, and 1 more risk signal)')
  })

  it('says how many more risk signals fired', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, 'src/auth/token.ts'), 'export const token = 1\n')
    await h.writeText(join(h.repo, 'src/auth/keys.ts'), 'export const keys = 1\n')
    await stageRisky(h)

    expect((await gate(h, 'git commit -m auth')).line).toContain('(src/auth/keys.ts is a risk path, and 2 more risk signals)')
  })
})

/** Rewrites the one line of the ticket-review index with `changes`, as a later review of the same change would leave it. */
async function rewriteReview(h: Harness, changes: Record<string, unknown>, keepOriginal = false): Promise<void> {
  const index = join(h.repo, '.git', 'critique', 'ticket-reviews.jsonl')
  const original = (await readFile(index, 'utf8')).trim()
  const changed = JSON.stringify({ ...(JSON.parse(original) as object), ...changes })
  await h.writeText(index, keepOriginal ? `${changed}\n${original}\n` : `${changed}\n`)
}

describe('a ticket review that failed', () => {
  it('blocks while its accepted critical finding is unfixed, naming the run and the finding', async () => {
    const h = await optedIn()
    await stageRisky(h)
    const run = await reviewStaged(h)
    await rewriteReview(h, { review: 'fail', unfixed_critical: ['F1'] })

    expect(await gate(h, 'git commit -m x')).toMatchObject({
      line: `Commit blocked: /review-implementation run ${run.run_id} failed for this staged change, with critical F1 accepted and not fixed. Fix and stage it, then run /review-implementation again.`,
      reason: 'review-failed',
    })
  })

  it('names every unfixed finding', async () => {
    const h = await optedIn()
    await stageRisky(h)
    await reviewStaged(h)
    await rewriteReview(h, { review: 'fail', unfixed_critical: ['F1', 'F2'] })

    expect((await gate(h, 'git commit -m x')).line).toContain('with critical F1, F2 accepted and not fixed. Fix and stage them,')
  })

  it('lets the latest review of the change decide, so a passing re-run opens the gate', async () => {
    const h = await optedIn()
    await stageRisky(h)
    await reviewStaged(h)
    await rewriteReview(h, { review: 'fail', unfixed_critical: ['F1'] }, true)

    expect(await gate(h, 'git commit -m x')).toMatchObject({ reason: 'reviewed' })
  })

  it("ignores another repository's review of the same staged change", async () => {
    const h = await optedIn()
    await stageRisky(h)
    await reviewStaged(h)
    await rewriteReview(h, { repository: 'github.com/someone/else' })

    expect(await gate(h, 'git commit -m x')).toMatchObject({ reason: 'unreviewed' })
  })
})

describe('a ticket review no critic reported on', () => {
  /** Runs `/review-implementation` on the staged change with a critic that fails, so the run is degraded. */
  async function reviewDegraded(h: Harness): Promise<PrepareResult> {
    h.process.queue({ exitCode: 1, stdout: '' })
    const run = await h.commands.prepare.run({
      base: 'HEAD',
      orchestrator_family: 'anthropic',
      repo: h.repo,
      spec: SPEC,
      stage: 'ticket-review',
      staged: true,
      ticket: TICKET,
    })
    if (run.status !== 'ready') throw new Error(`prepare refused: ${run.message}`)
    await h.commands.fanOut.run({ run_dir: run.run_dir })
    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })
    return run
  }

  it('blocks the commit, naming the degraded run and why its critics did not report', async () => {
    const h = await optedIn()
    await stageRisky(h)
    const run = await reviewDegraded(h)

    const line = `Commit blocked: /review-implementation run ${run.run_id} is degraded: no critic reported on this staged change (c1-vibe-glm: config-error). Run /review-implementation again; the commit waits for a run in which a critic reports.`
    expect(await gate(h, 'git commit -m x')).toEqual({ gate: 'commit', hook_output: deny(line), line, reason: 'review-degraded', status: 'blocked' })
  })

  it('blocks on a record written before degraded runs had their own verdict', async () => {
    const h = await optedIn()
    await stageRisky(h)
    const run = await reviewStaged(h)
    const index = join(h.repo, '.git', 'critique', 'ticket-reviews.jsonl')
    const legacy = JSON.parse(await readFile(index, 'utf8')) as Record<string, unknown>
    delete legacy.critic_failures
    await h.writeText(index, `${JSON.stringify({ ...legacy, degraded: true, review: 'pass' })}\n`)

    expect(await gate(h, 'git commit -m x')).toMatchObject({
      line: `Commit blocked: /review-implementation run ${run.run_id} is degraded: no critic reported on this staged change. Run /review-implementation again; the commit waits for a run in which a critic reports.`,
      reason: 'review-degraded',
    })
  })

  it('opens the gate once a later run of the same change has a critic that reported', async () => {
    const h = await optedIn()
    await stageRisky(h)
    await reviewDegraded(h)
    await reviewStaged(h)

    expect(await gate(h, 'git commit -m x')).toMatchObject({ reason: 'reviewed', status: 'allowed' })
  })
})

describe("the ticket's risk label", () => {
  const triageLine = (stagedOn: string) =>
    `${JSON.stringify({
      labels: ['risk:high'],
      recorded_at: '2026-10-30T12:00:00.000Z',
      repository: 'github.com/jorisjonkers-dev/agent-kit',
      run_id: 'r1',
      staged_diff: 'x',
      staged_on: stagedOn,
      status: 'risky',
      ticket: '.scratch/widgets/issues/02-copy.md',
      version: 1,
    })}\n`

  async function lowRiskStaged(): Promise<Harness> {
    const h = await optedIn()
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widgets = 2\n')
    await h.git.stage()
    return h
  }

  it('makes a commit risky when triage read it for the change staged on this HEAD', async () => {
    const h = await lowRiskStaged()
    await h.writeText(join(h.repo, '.git', 'critique', 'triages.jsonl'), triageLine('c2'))

    expect((await gate(h, 'git commit -m x')).line).toContain('(the ticket carries risk:high)')
  })

  it('is not read from a triage staged on another commit', async () => {
    const h = await lowRiskStaged()
    await h.writeText(join(h.repo, '.git', 'critique', 'triages.jsonl'), triageLine('c1'))

    expect(await gate(h, 'git commit -m x')).toMatchObject({ reason: 'low-risk' })
  })

  it('is recorded by a triage of the staged change, with the commit it is staged on', async () => {
    const issue = { body: 'Copy.', comments: [], labels: ['Risk:High'], number: 5, title: 'Copy', url: 'https://github.com/JorisJonkers-dev/agent-kit/issues/5' }
    const h = await harness({ issues: { 'github.com/jorisjonkers-dev/agent-kit#5': issue }, tracker: 'github' })
    open.push(h)
    await writeFlowState(h)
    await h.commands.allow.run({ code: true, families: ['mistral', 'zhipu'], repo: h.repo })
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widgets = 2\n')
    await h.git.stage()

    expect(await h.commands.triage.run({ base: 'HEAD', repo: h.repo, staged: true, ticket: '#5' })).toMatchObject({ status: 'risky' })

    const line = JSON.parse(await readFile(join(h.repo, '.git', 'critique', 'triages.jsonl'), 'utf8')) as Record<string, unknown>
    expect(line).toMatchObject({ labels: ['Risk:High'], staged_on: 'c1', status: 'risky', ticket: 'github.com/jorisjonkers-dev/agent-kit#5' })
    expect(await gate(h, 'git commit -m x')).toMatchObject({ reason: 'unreviewed', status: 'blocked' })
  })

  it('is not recorded for a triage of a commit', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widgets = 2\n')
    await h.git.commit()
    await h.writeText(join(h.repo, 'src/auth/session.ts'), 'export const ttl = 1\n')
    await h.git.commit()

    await h.commands.triage.run({ base: 'HEAD~1', repo: h.repo, ticket: TICKET })

    expect(await readFile(join(h.repo, '.git', 'critique', 'triages.jsonl'), 'utf8').catch(() => 'absent')).toBe('absent')
  })
})

describe('where the gates do nothing', () => {
  it.each([
    ['no remote', [], 'no-remote'],
    ['a nedap remote', [{ name: 'upstream', url: 'git@github-work:nedap/widgets.git' }], 'nedap'],
  ] as const)('allows every command in a repository with %s', async (_name, remotes, reason) => {
    const h = await harness({ remotes })
    open.push(h)
    await stageRisky(h)

    expect(await gate(h, 'git commit -m x && gh pr create --fill')).toEqual({ gate: 'commit', line: null, reason, status: 'allowed' })
  })

  it('gives a repository with no allowlist entry the personal default, so it looks for a flow', async () => {
    const h = await harness()
    open.push(h)
    await stageRisky(h)

    expect(await gate(h, 'git commit -m x')).toMatchObject({ reason: 'no-active-flow', status: 'allowed' })
    expect(await gate(h, 'gh pr create')).toMatchObject({ gate: 'pr', reason: 'no-active-flow', status: 'allowed' })
  })

  it('allows every command where the entry allows documents only, since no code stage could pass the gate', async () => {
    const h = await optedIn({ code: false })
    await stageRisky(h)

    expect(await gate(h, 'git commit -m x')).toMatchObject({ reason: 'code-not-allowed', status: 'allowed' })
  })

  it('allows everything under CRITIQUE_OFF=1', async () => {
    const h = await optedIn({ env: { CRITIQUE_OFF: '1' } })
    await stageRisky(h)

    expect(await gate(h, 'git commit -m x && gh pr create')).toEqual({ gate: null, line: null, reason: 'off', status: 'allowed' })
  })

  it("allows everything when this machine's critique configuration is disabled", async () => {
    const h = await optedIn()
    await stageRisky(h)
    await h.writeText(join(h.home, '.config', 'agent-kit', 'critique.json'), '{"disabled": true}\n')

    expect(await gate(h, 'git commit -m x')).toEqual({ gate: null, line: null, reason: 'disabled', status: 'allowed' })
  })

  it('gates as usual when the configuration does not disable critique', async () => {
    const h = await optedIn()
    await stageRisky(h)
    await h.writeText(join(h.home, '.config', 'agent-kit', 'critique.json'), '{}\n')

    expect(await gate(h, 'git commit -m x')).toMatchObject({ status: 'blocked' })
  })

  it.each([
    ['git status && git diff --cached'],
    ['echo "git commit -m x"'],
    ["grep -n 'git push' notes.md"],
    ['git commit --dry-run'],
    ['git push --dry-run origin main'],
    ['git push -n origin main'],
    ['git push origin --tags'],
    ['git push origin v1.2.0:refs/tags/v1.2.0'],
    ['git push origin refs/tags/v1.2.0'],
    ['git push origin --delete old-branch'],
    ['git push origin :old-branch'],
    ['gh pr view 12'],
    ['# git commit -m x'],
    [''],
  ])('does not gate %j', async (command) => {
    const h = await optedIn()
    await stageRisky(h)

    expect(await gate(h, command)).toEqual({ gate: null, line: null, reason: 'not-gated', status: 'allowed' })
  })

  it('does not gate a tool other than Bash', async () => {
    const h = await optedIn()
    await stageRisky(h)

    expect(await gate(h, 'git commit -m x', { tool_name: 'Write' })).toMatchObject({ reason: 'not-gated' })
  })

  it('runs only the gate the hook names', async () => {
    const h = await optedIn()
    await stageRisky(h)

    expect(await gate(h, 'git commit -m x', { gate: 'pr' })).toMatchObject({ reason: 'not-gated' })
    expect(await gate(h, 'git commit -m x', { gate: 'commit' })).toMatchObject({ reason: 'unreviewed' })
  })
})

/** Runs `/review-pr` on HEAD as the skill does, with critics that find nothing, so its gate passes. */
async function reviewFeature(h: Harness): Promise<void> {
  h.process.queue(quietCritic())
  h.process.queue(quietCritic())
  const run = await h.commands.prepare.run({ base: 'HEAD~1', orchestrator_family: 'anthropic', repo: h.repo, spec: SPEC, stage: 'feature-review' })
  if (run.status !== 'ready') throw new Error(`prepare refused: ${run.message}`)
  await h.commands.fanOut.run({ run_dir: run.run_dir })
  const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })
  expect(recorded.gate).toMatchObject({ gate: 'pass', head_commit: 'c2' })
}

/** Rewrites the one line of the gate index with `changes`. */
async function rewriteGate(h: Harness, changes: Record<string, unknown>): Promise<void> {
  const index = join(h.repo, '.git', 'critique', 'gates.jsonl')
  const original = JSON.parse(await readFile(index, 'utf8')) as object
  await h.writeText(index, `${JSON.stringify({ ...original, ...changes })}\n`)
}

describe('the PR gate', () => {
  it('blocks gh pr create while no /review-pr run has recorded a gate for HEAD', async () => {
    const h = await optedIn()

    const line = 'PR blocked: no /review-pr run has recorded a gate for c2. Run /review-pr, then open the PR.'
    expect(await gate(h, 'gh pr create --fill')).toEqual({ gate: 'pr', hook_output: deny(line), line, reason: 'no-feature-review', status: 'blocked' })
  })

  it('allows it once the latest /review-pr run for HEAD says GATE: PASS', async () => {
    const h = await optedIn()
    await reviewFeature(h)

    expect(await gate(h, 'gh pr create --fill')).toEqual({ gate: 'pr', line: null, reason: 'gate-pass', status: 'allowed' })
  })

  it('blocks on GATE: FAIL, quoting the gate line', async () => {
    const h = await optedIn()
    await reviewFeature(h)
    await rewriteGate(h, { critical_open: 1, gate: 'fail', line: 'GATE: FAIL (1 critical)' })

    expect(await gate(h, 'gh pr create --fill')).toMatchObject({
      line: 'PR blocked: /review-pr says GATE: FAIL (1 critical) for c2. Fix and close the critical follow-ups, run /review-pr on the new head, then open the PR.',
      reason: 'gate-fail',
    })
  })

  it('holds a pass for the head it reviewed only', async () => {
    const h = await optedIn()
    await reviewFeature(h)
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widgets = 3\n')
    await h.git.commit()

    expect(await gate(h, 'gh pr create --fill')).toMatchObject({ reason: 'no-feature-review' })
  })

  it("ignores another repository's gate for the same head", async () => {
    const h = await optedIn()
    await reviewFeature(h)
    await rewriteGate(h, { repository: 'github.com/someone/else' })

    expect(await gate(h, 'gh pr create')).toMatchObject({ reason: 'no-feature-review' })
  })

  it('reads the gate of the branch --head names', async () => {
    const h = await optedIn()
    await reviewFeature(h)
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widgets = 3\n')
    await h.git.commit()

    expect(await gate(h, 'gh pr create --base main --head c2')).toMatchObject({ reason: 'gate-pass' })
    expect(await gate(h, 'gh pr create --head=c2')).toMatchObject({ reason: 'gate-pass' })
  })

  it('blocks a PR opened after a commit in the same command, since what it sends is not committed yet', async () => {
    const h = await optedIn()
    await reviewFeature(h)

    expect(await gate(h, 'git commit -m wip && gh pr create --fill')).toMatchObject({
      line: 'PR blocked: this command moves HEAD before it opens the PR, so no /review-pr gate can hold for what it sends. Run that part in a command of its own after /review-pr.',
      reason: 'head-unknown',
    })
  })
})

describe('the PR gate on a push', () => {
  it.each([['git push origin feat/widgets'], ['git push -u origin impl/197-gate'], ['git push origin HEAD:refs/heads/fix/typo'], ['git push origin main:wip/try']])(
    'lets %j through, since it updates a feature branch',
    async (command) => {
      const h = await optedIn()

      expect(await gate(h, command)).toEqual({ gate: 'pr', line: null, reason: 'feature-branch', status: 'allowed' })
    },
  )

  it.each([['git push -o ci.skip origin main'], ['git push --repo origin origin main'], ['git push origin -- main']])(
    'finds the branch %j updates past options and their values',
    async (command) => {
      const h = await optedIn()

      expect((await gate(h, command)).line).toMatch(/^Push to main blocked: /)
    },
  )

  it('lets a push of the current branch through when it is a feature branch', async () => {
    const h = await optedIn()
    h.git.branch = 'feat/widgets'

    expect(await gate(h, 'git push')).toMatchObject({ reason: 'feature-branch' })
    expect(await gate(h, 'git push origin HEAD')).toMatchObject({ reason: 'feature-branch' })
  })

  it('blocks a push to a branch outside the feature branch patterns without a passing feature review', async () => {
    const h = await optedIn()

    expect(await gate(h, 'git push origin main')).toMatchObject({
      line: 'Push to main blocked: no /review-pr run has recorded a gate for c2. Run /review-pr, then push.',
      reason: 'no-feature-review',
    })
  })

  it('blocks a push of the current branch when it is not a feature branch', async () => {
    const h = await optedIn()

    expect((await gate(h, 'git push')).line).toMatch(/^Push to main blocked: /)
  })

  it('blocks a push from a detached HEAD, whose branch it cannot name', async () => {
    const h = await optedIn()
    h.git.branch = ''

    expect((await gate(h, 'git push origin HEAD')).line).toMatch(/^Push to every branch blocked: /)
  })

  it('allows the push once the commit it sends passed /review-pr', async () => {
    const h = await optedIn()
    await reviewFeature(h)

    expect(await gate(h, 'git push origin main')).toMatchObject({ reason: 'gate-pass' })
    expect(await gate(h, 'git push origin c2:main')).toMatchObject({ reason: 'gate-pass' })
  })

  it('reads the gate of the commit the refspec sends, not HEAD', async () => {
    const h = await optedIn()
    await reviewFeature(h)

    expect((await gate(h, 'git push origin c1:main')).line).toBe(
      'Push to main blocked: no /review-pr run has recorded a gate for c1. Run /review-pr, then push.',
    )
  })

  it('gates --all and --mirror as a push to every branch', async () => {
    const h = await optedIn()

    expect((await gate(h, 'git push --all origin')).line).toMatch(/^Push to every branch blocked: /)
    expect((await gate(h, 'git push --mirror origin')).line).toMatch(/^Push to every branch blocked: /)
  })

  it('blocks a push after a commit in the same command', async () => {
    const h = await optedIn()
    await reviewFeature(h)

    expect((await gate(h, 'git commit -m wip && git push origin main')).line).toBe(
      'Push to main blocked: this command moves HEAD before it pushes, so no /review-pr gate can hold for what it sends. Run that part in a command of its own after /review-pr.',
    )
  })

  it('gates every branch when routing names no feature branches', async () => {
    const h = await optedIn({ routingOverride: { gate: null } })
    h.git.branch = 'feat/widgets'

    expect(await gate(h, 'git push origin feat/widgets')).toMatchObject({ reason: 'no-feature-review' })
  })
})

describe('reading the command line', () => {
  it.each([
    ['git commit -m "$(cat <<\'EOF\'\nDon\'t expire (yet) "sessions"\n\ngit push origin main\nEOF\n)"'],
    ["git commit -F - <<'EOF'\nDon't push: git push origin main\nEOF"],
    ['git commit -m "fix `git push` docs"'],
    ['git commit -m "a \\"git push origin main\\" note"'],
    ['git commit -m \'git push origin main\''],
    ['GIT_EDITOR=true git --no-pager -c core.hooksPath=/dev/null commit -m x'],
    ['env GIT_EDITOR=true command git commit -m x 2>&1 | tail -n 3'],
    ['git commit -m x > /tmp/out.txt'],
    ['git commit -m x\\\n  --no-verify'],
    ['echo start; git commit -qm x'],
    ['(git commit -m x)'],
    ['git commit -m x # and then git push origin main'],
    ['git commit -m "$(printf \'%s\' \'it)s; git push origin main\' "a) b" \\) $(echo "c") `whoami` (d))"'],
    ['git commit -m "$(cat <<- \'EOF\'\n\tgit push origin main)\n\tEOF\n)"'],
    ['git commit -F - << \'EOF\'\ngit push origin main\nEOF\necho done'],
    ['git commit -F - <<-EOF\n\tgit push origin main\n\tEOF'],
    ['git commit -F - <<<"git push origin main"'],
    ['git commit -m $(cat msg.txt) -m `cat more.txt`'],
    ['git commit -m "$(echo "a \\" ) $(date) b; git push origin main")"'],
    ["git commit -m it\\'s"],
    ['git commit -m "unterminated && git push origin main'],
    ["git commit -m 'unterminated && git push origin main"],
    ['FOO=1; env -i PATH=/usr/bin git commit -m x'],
  ])('finds the one commit in %j', async (command) => {
    const h = await optedIn()
    await stageRisky(h)

    expect(await gate(h, command)).toMatchObject({ gate: 'commit', reason: 'unreviewed' })
  })

  it('follows cd and git -C to the repository the command runs in', async () => {
    const h = await optedIn()
    await stageRisky(h)
    const elsewhere = join(h.home, 'elsewhere')

    expect(await h.commands.gate.run(payload(h, `cd ${h.repo} && git commit -m x`, elsewhere))).toMatchObject({ reason: 'unreviewed' })
    expect(await h.commands.gate.run(payload(h, `git -C ${h.repo} commit -m x`, elsewhere))).toMatchObject({ reason: 'unreviewed' })
    expect(await h.commands.gate.run(payload(h, 'cd repo && git commit -m x', join(h.repo, '..')))).toMatchObject({ reason: 'unreviewed' })
    expect(await h.commands.gate.run(payload(h, 'git -C src commit -m x'))).toMatchObject({ reason: 'unreviewed' })
  })

  it('fails open when it cannot tell which directory the command runs in', async () => {
    const h = await optedIn()
    await stageRisky(h)

    for (const command of ['cd - && git commit -m x', 'cd - && cd src && git commit -m x', 'cd "$REPO" && git commit -m x', 'git -C $REPO commit -m x']) {
      expect(await gate(h, command)).toMatchObject({ failure: 'error', reason: 'failed-open', status: 'allowed' })
    }
  })

  it('expands ~ in cd from HOME', async () => {
    const h = await optedIn()
    await stageRisky(h)
    await h.writeText(join(h.home, 'marker'), '')

    // The fake answers for any directory; what matters is that ~ is read, not left unclear.
    expect(await gate(h, 'cd ~ && git commit -m x')).toMatchObject({ reason: 'unreviewed' })
    expect(await gate(h, 'cd ~/projects && git commit -m x')).toMatchObject({ reason: 'unreviewed' })
    expect(await gate(h, 'cd && git commit -m x')).toMatchObject({ reason: 'unreviewed' })
  })

  it('blocks on the commit and the push of one command line', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widgets = 2\n')
    await h.git.stage()

    expect(await gate(h, 'git commit -m x && git push origin main')).toMatchObject({ gate: 'pr', reason: 'head-unknown' })
  })

  it('reports the first decision when nothing blocks', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widgets = 2\n')
    await h.git.stage()

    expect(await gate(h, 'git commit -m x; git push origin feat/widgets')).toMatchObject({ gate: 'commit', reason: 'low-risk' })
  })
})

describe('a commit that records more than is staged', () => {
  it.each([['git commit -am x'], ['git commit -a -m x'], ['git commit --all -m x'], ['git commit -m x src/auth/session.ts'], ['git commit -m x -- src/auth/session.ts'], ['git add src/auth && git commit -m x']])(
    'reads the tracked files on disk for %j',
    async (command) => {
      const h = await optedIn()
      await h.writeText(join(h.repo, RISKY_PATH), 'export const ttl = 60\n')
      await h.git.stage()
      await h.git.commit()
      await h.writeText(join(h.repo, RISKY_PATH), 'export const ttl = 90\n')

      expect(await gate(h, command)).toMatchObject({
        line: 'Commit blocked: the change is risky (src/auth/session.ts is a risk path) and this command commits changes that are not staged yet, so no /review-implementation run can vouch for them. Stage them in a command of their own, run /review-implementation, then commit.',
        reason: 'unstaged',
      })
    },
  )

  it('sees a new file a git add in the same command would stage', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, RISKY_PATH), 'export const ttl = 60\n')

    expect(await gate(h, 'git add -A && git commit -m x')).toMatchObject({ reason: 'unstaged' })
    expect(await gate(h, 'git commit -m x')).toMatchObject({ reason: 'low-risk' })
  })

  it('allows it when nothing on disk is left to stage and the staged change passed review', async () => {
    const h = await optedIn()
    await stageRisky(h)
    await reviewStaged(h)

    expect(await gate(h, 'git commit -am x')).toMatchObject({ reason: 'reviewed' })
    expect(await gate(h, 'git add -A && git commit -m x')).toMatchObject({ reason: 'reviewed' })
  })

  it('allows a low-risk one', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widgets = 2\n')

    expect(await gate(h, 'git commit -am x')).toMatchObject({ reason: 'low-risk' })
  })

  it('reads a cluster of short options and the values they take', async () => {
    const h = await optedIn()
    await stageRisky(h)
    await reviewStaged(h)

    expect(await gate(h, 'git commit -qm x --author "A <a@example.com>" -S --cleanup=strip')).toMatchObject({ reason: 'reviewed' })
    expect(await gate(h, 'git commit --message x -C HEAD')).toMatchObject({ reason: 'reviewed' })
    expect(await gate(h, 'git commit -mx -- ')).toMatchObject({ reason: 'reviewed' })
    expect(await gate(h, 'git commit --include -m x')).toMatchObject({ reason: 'reviewed' })
  })
})

describe('where there is no active flow', () => {
  it('lets a risky unreviewed commit through, a PR and a push to a non-feature branch too', async () => {
    const h = await optedIn({ flow: false })
    await stageRisky(h)

    expect(await gate(h, 'git commit -m x')).toEqual({ gate: 'commit', line: null, reason: 'no-active-flow', status: 'allowed' })
    expect(await gate(h, 'gh pr create --fill')).toEqual({ gate: 'pr', line: null, reason: 'no-active-flow', status: 'allowed' })
    expect(await gate(h, 'git push origin main')).toMatchObject({ gate: 'pr', reason: 'no-active-flow', status: 'allowed' })
  })

  it('counts a flow still in planning, or of another repository, as no flow', async () => {
    const h = await optedIn({ flow: false })
    await stageRisky(h)
    await writeFlowState(h, { name: 'planning', phase: 'planning' })
    await writeFlowState(h, { name: 'other', repository: 'github.com/example/other' })
    await h.writeText(join(h.repo, '.git', 'critique', 'flows', 'notes.txt'), 'not a flow state')

    expect(await gate(h, 'git commit -m x')).toMatchObject({ reason: 'no-active-flow', status: 'allowed' })
  })

  it.each(['implementation', 'finishing'] as const)('still blocks the risky unreviewed commit and the PR while a flow is in %s', async (phase) => {
    const h = await optedIn({ flow: false })
    await stageRisky(h)
    await writeFlowState(h, { phase })

    expect(await gate(h, 'git commit -m x')).toMatchObject({ reason: 'unreviewed', status: 'blocked' })
    expect(await gate(h, 'gh pr create --fill')).toMatchObject({ reason: 'no-feature-review', status: 'blocked' })
  })

  it('allows and logs when a flow state does not read, since it cannot tell whether a flow is active', async () => {
    const h = await optedIn({ flow: false })
    await stageRisky(h)
    await h.writeText(join(h.repo, '.git', 'critique', 'flows', 'broken.json'), '{not json')

    expect(await gate(h, 'git commit -m x')).toMatchObject({ failure: 'unreadable-state', gate: 'commit', reason: 'failed-open', status: 'allowed' })
  })

  it('lets an active flow of this repository win over a state file that does not read', async () => {
    const h = await optedIn()
    await stageRisky(h)
    await h.writeText(join(h.repo, '.git', 'critique', 'flows', 'a-torn.json'), '{not json')

    expect(await gate(h, 'git commit -m x')).toMatchObject({ reason: 'unreviewed', status: 'blocked' })
  })

  it('still fails open when a state file does not read and no readable one shows an active flow', async () => {
    const h = await optedIn({ flow: false })
    await stageRisky(h)
    await h.writeText(join(h.repo, '.git', 'critique', 'flows', 'a-torn.json'), '{not json')
    await writeFlowState(h, { name: 'planning', phase: 'planning' })

    expect(await gate(h, 'git commit -m x')).toMatchObject({ failure: 'unreadable-state', reason: 'failed-open', status: 'allowed' })
  })

  it('holds a repository with no allowlist entry to the personal default, so the flow decides', async () => {
    const h = await harness()
    open.push(h)
    await stageRisky(h)

    expect(await gate(h, 'git commit -m x')).toMatchObject({ reason: 'no-active-flow' })
  })
})

describe('failing open', () => {
  async function logLines(h: Harness): Promise<readonly Record<string, unknown>[]> {
    const text = await readFile(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'gate.log.jsonl'), 'utf8')
    return text
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line) as Record<string, unknown>)
  }

  it('allows and logs when its deadline passes before it decides', async () => {
    const h = await optedIn()
    await stageRisky(h)
    h.git.remotesBehaviour = 'hang'
    h.clock.wake()

    expect(await gate(h, 'git commit -m x')).toEqual({
      detail: 'the gate did not decide within 700 ms',
      failure: 'deadline',
      gate: null,
      line: null,
      reason: 'failed-open',
      status: 'allowed',
    })
    expect(await logLines(h)).toEqual([
      { cwd: h.repo, detail: 'the gate did not decide within 700 ms', failure: 'deadline', gate: null, recorded_at: '2026-10-30T12:00:00.000Z' },
    ])
  })

  it('allows and logs when it crashes', async () => {
    const h = await optedIn()
    await stageRisky(h)
    h.git.remotesBehaviour = new Error('git: segmentation fault')

    expect(await gate(h, 'git commit -m x')).toMatchObject({ detail: 'git: segmentation fault', failure: 'error', gate: 'commit', status: 'allowed' })
    expect(await logLines(h)).toMatchObject([{ failure: 'error', gate: 'commit' }])
  })

  it('still blocks on an explicit decision beside a check that crashed', async () => {
    const h = await optedIn()
    await stageRisky(h)

    expect(await gate(h, 'git -C $ELSEWHERE commit -m x; git commit -m y')).toMatchObject({ reason: 'unreviewed', status: 'blocked' })
  })

  it.each([
    ['the ticket-review index', 'ticket-reviews.jsonl'],
    ['the triage index', 'triages.jsonl'],
  ])('allows and logs when a line of %s does not parse', async (_name, file) => {
    const h = await optedIn()
    await stageRisky(h)
    await h.writeText(join(h.repo, '.git', 'critique', file), '{"version": 1, "stage": "feature-review"}\n\n')

    expect(await gate(h, 'git commit -m x')).toMatchObject({ failure: 'unreadable-state', reason: 'failed-open', status: 'allowed' })
  })

  it('allows and logs when a line of the gate index does not parse', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, '.git', 'critique', 'gates.jsonl'), 'not json\n')

    expect(await gate(h, 'gh pr create')).toMatchObject({ failure: 'unreadable-state', gate: 'pr', status: 'allowed' })
  })

  it('allows when the allowlist cannot be read', async () => {
    const h = await optedIn()
    await stageRisky(h)
    await h.writeText(join(h.home, '.config', 'agent-kit', 'critique-allowlist.json'), '{')

    expect(await gate(h, 'git commit -m x')).toMatchObject({ failure: 'unreadable-state', status: 'allowed' })
  })

  it("allows when this machine's critique configuration cannot be read", async () => {
    const h = await optedIn()
    await stageRisky(h)
    await h.writeText(join(h.home, '.config', 'agent-kit', 'critique.json'), '{"disabled": "yes"}')

    expect(await gate(h, 'git commit -m x')).toMatchObject({ failure: 'unreadable-state', gate: null, status: 'allowed' })
  })

  it('allows when routing is broken or has no triage', async () => {
    const broken = await optedIn({ routingOverride: '{' })
    await stageRisky(broken)
    expect(await gate(broken, 'git commit -m x')).toMatchObject({ failure: 'error', status: 'allowed' })

    const untriaged = await optedIn({ routingOverride: { triage: null } })
    await stageRisky(untriaged)
    expect(await gate(untriaged, 'git commit -m x')).toMatchObject({
      detail: 'routing has no triage, so the commit gate has no risk rules',
      failure: 'error',
    })
  })

  it('allows a payload that is not a hook payload', async () => {
    const h = await optedIn()

    for (const raw of [null, 'git commit', { tool_input: { command: 'git commit' } }, { cwd: h.repo }]) {
      expect(await h.commands.gate.run(raw)).toMatchObject({ failure: 'error', reason: 'failed-open', status: 'allowed' })
    }
    expect((await logLines(h)).map((line) => line.cwd)).toEqual([null, null, null, h.repo])
  })

  it('allows a payload with no command', async () => {
    const h = await optedIn()

    expect(await h.commands.gate.run({ cwd: h.repo, tool_input: {} })).toMatchObject({ reason: 'not-gated' })
  })

  it('still allows when the log cannot be written', async () => {
    const h = await optedIn()
    await h.writeText(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'gate.log.jsonl', 'in-the-way'), '')

    expect(await h.commands.gate.run(null)).toMatchObject({ reason: 'failed-open', status: 'allowed' })
  })
})
