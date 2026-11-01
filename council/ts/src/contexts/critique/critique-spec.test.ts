import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { fenced, finding, harness, isolatedExport, vibeStdout, type Harness, type VibeScript } from '../../../test/critique/harness.js'
import type { PrepareResult } from './index.js'

/**
 * What `/critique-spec` (#177) relies on the runner for: the warning not to
 * start `/to-tickets` while a critical question is unanswered, and the rule
 * that more than three accepted critical findings call for one second round
 * and never a third.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

function critic(findings: readonly Record<string, unknown>[]): VibeScript {
  return { exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings })) }
}

async function opened(scripts: VibeScript[]): Promise<Harness> {
  const h = await harness({ scripts })
  open.push(h)
  await h.commands.allow.run({ families: ['mistral'], repo: h.repo })
  return h
}

async function prepared(h: Harness, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const run = await h.commands.prepare.run({
    orchestrator_family: 'anthropic',
    repo: h.repo,
    spec: '.scratch/widgets/spec.md',
    stage: 'spec',
    ...extra,
  })
  if (run.status !== 'ready') throw new Error(`not ready: ${JSON.stringify(run)}`)
  return run
}

/** Critical findings at distinct locations, so none of them merge. */
function criticals(count: number): Record<string, unknown>[] {
  return Array.from({ length: count }, (_, n) => finding({ id: `F${String(n + 1)}`, location: `User story ${String(n + 1)}`, severity: 'critical' }))
}

function accepting(count: number): { finding_id: string; verdict: string }[] {
  return Array.from({ length: count }, (_, n) => ({ finding_id: `F${String(n + 1)}`, verdict: 'accepted' }))
}

const WARNING = 'Do not start /to-tickets'

describe('the warning not to start /to-tickets', () => {
  it('is in the summary while a critical question is unanswered', async () => {
    const h = await opened([critic([finding({ id: 'F1', severity: 'critical' }), finding({ id: 'F2', location: 'User story 9' })])])
    const run = await prepared(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const { summary } = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [
        { finding_id: 'F1', reason: 'Should the nightly job stay the only frobnicator?', verdict: 'needs-human' },
        { finding_id: 'F2', reason: 'Which window applies?', verdict: 'needs-human' },
      ],
    })

    expect(summary).toContain(`${WARNING}: critical question F1 is unanswered.`)
  })

  it('is left out once the owner has answered every critical question', async () => {
    const h = await opened([critic([finding({ id: 'F1', severity: 'critical' })])])
    const run = await prepared(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const { report_path, summary } = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [{ answered: true, finding_id: 'F1', reason: 'Should the nightly job stay the only frobnicator?', verdict: 'needs-human' }],
    })

    expect(summary).not.toContain(WARNING)
    expect(summary).toContain('Question: Should the nightly job stay the only frobnicator? (answered)')
    expect(await readFile(report_path, 'utf8')).toContain('Verdict: needs-human, answered: Should the nightly job stay the only frobnicator?')
  })

  it('is left out when the only open questions are not critical', async () => {
    const h = await opened([critic([finding({ id: 'F1', severity: 'major' })])])
    const run = await prepared(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const { summary } = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [{ finding_id: 'F1', reason: 'Which window applies?', verdict: 'needs-human' }],
    })

    expect(summary).not.toContain(WARNING)
  })

  it('refuses an answer on a finding that is not a question', async () => {
    const h = await opened([critic([finding({ id: 'F1', severity: 'critical' })])])
    const run = await prepared(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    await expect(
      h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ answered: true, finding_id: 'F1', verdict: 'accepted' }] }),
    ).rejects.toThrow(/F1: only a needs-human verdict can be answered/)
  })
})

/** Prepares, fans out and records one round that accepts every finding. */
async function round(h: Harness, findings: number, extra: Record<string, unknown> = {}) {
  const run = await prepared(h, extra)
  await h.commands.fanOut.run({ run_dir: run.run_dir })
  const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: accepting(findings) })
  return { recorded, run }
}

describe('a second round', () => {
  it('is due when more than three critical findings are accepted', async () => {
    const h = await opened([critic(criticals(4))])

    const { recorded, run } = await round(h, 4)

    expect(run.round).toBe(1)
    expect(recorded).toMatchObject({ round: 1, second_round_due: true })
  })

  it('is not due when three critical findings are accepted', async () => {
    const h = await opened([critic([...criticals(3), finding({ id: 'F4', location: 'Problem Statement', severity: 'major' })])])

    const { recorded } = await round(h, 4)

    expect(recorded.second_round_due).toBe(false)
  })

  it('counts critical findings that were accepted, not those rejected or asked about', async () => {
    const h = await opened([critic(criticals(5))])
    const run = await prepared(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const recorded = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [
        ...accepting(3),
        { finding_id: 'F4', reason: 'The decision doc settles it.', verdict: 'rejected' },
        { finding_id: 'F5', reason: 'Which job owns it?', verdict: 'needs-human' },
      ],
    })

    expect(recorded.second_round_due).toBe(false)
  })

  it('runs on the revised spec as round 2 of the same stage, and says so', async () => {
    const h = await opened([critic(criticals(4)), critic(criticals(4))])
    const first = await round(h, 4)
    const spec = join(h.repo, '.scratch/widgets/spec.md')
    await h.writeText(spec, `${await h.readText(spec)}\nRevised after the first round.\n`)

    const second = await round(h, 4, { previous_run_dir: first.run.run_dir })

    expect(second.run.round).toBe(2)
    expect(second.run.run_id).not.toBe(first.run.run_id)
    expect(await readFile(join(second.run.packet_dir, 'artifact', 'spec.md'), 'utf8')).toContain('Revised after the first round.')
    expect(second.recorded).toMatchObject({ round: 2, second_round_due: false })
    expect(second.recorded.summary).toContain(`Round 2, after run ${first.run.run_id}`)
    const manifest = JSON.parse(await readFile(second.run.manifest_path, 'utf8')) as Record<string, unknown>
    expect(manifest).toMatchObject({ previous_run_id: first.run.run_id, round: 2 })
  })

  it('writes the round into every ledger line', async () => {
    const h = await opened([critic(criticals(4)), critic(criticals(4))])
    const first = await round(h, 4)
    await round(h, 4, { previous_run_dir: first.run.run_dir })

    const text = await readFile(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl'), 'utf8')
    const rounds = text
      .trim()
      .split('\n')
      .map((line) => (JSON.parse(line) as { round: number }).round)
    // Per round: fan-out's invocation line, then record's run line and four finding lines.
    expect(rounds).toEqual([1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2])
  })

  it('is never followed by a third round', async () => {
    const h = await opened([critic(criticals(4)), critic(criticals(4))])
    const first = await round(h, 4)
    const second = await round(h, 4, { previous_run_dir: first.run.run_dir })

    const third = await h.commands.prepare.run({
      orchestrator_family: 'anthropic',
      previous_run_dir: second.run.run_dir,
      repo: h.repo,
      spec: '.scratch/widgets/spec.md',
      stage: 'spec',
    })

    expect(third).toEqual({
      message: `run ${second.run.run_id} was already the second round; a stage runs at most two rounds`,
      reason: 'round-limit',
      status: 'refused',
    })
  })

  it('needs the first round recorded before it starts', async () => {
    const h = await opened([critic(criticals(4))])
    const first = await prepared(h)

    await expect(prepared(h, { previous_run_dir: first.run_dir })).rejects.toThrow(/is not recorded yet/)
  })

  it('must critique the same artifact at the same stage', async () => {
    const h = await opened([critic(criticals(4))])
    const first = await round(h, 4)
    await h.writeText(join(h.repo, '.scratch/gadgets/spec.md'), '# Gadgets\n')

    await expect(prepared(h, { previous_run_dir: first.run.run_dir, spec: '.scratch/gadgets/spec.md' })).rejects.toThrow(
      /critiqued .scratch\/widgets\/spec.md at the spec stage, not .scratch\/gadgets\/spec.md/,
    )
  })
})
