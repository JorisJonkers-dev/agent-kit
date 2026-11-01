import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  agyFindings,
  DECISION_CANARY,
  fenced,
  finding,
  harness,
  isolatedExport,
  SPEC_CANARY,
  vibeStdout,
  type AgyScript,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import type { FanOutResult, OrchestratorFamily, PrepareResult } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

interface Stage {
  readonly families: readonly string[]
  readonly orchestrator?: OrchestratorFamily
  readonly vibe?: VibeScript[]
  readonly agy?: AgyScript[]
}

async function prepared(stage: Stage): Promise<{ h: Harness; run: PrepareResult }> {
  const h = await harness({ agyScripts: stage.agy ?? [], scripts: stage.vibe ?? [] })
  open.push(h)
  await h.commands.allow.run({ families: stage.families, repo: h.repo })
  const run = await h.commands.prepare.run({
    orchestrator_family: stage.orchestrator ?? 'anthropic',
    repo: h.repo,
    spec: '.scratch/widgets/spec.md',
    stage: 'spec',
  })
  if (run.status !== 'ready') throw new Error('not ready')
  return { h, run }
}

/** The shipped base roster, Mistral Large and Gemini Flash, each returning the given findings. */
async function bothFamilies(
  vibeFindings: readonly unknown[],
  agyFound: readonly unknown[],
  delays: { readonly vibe?: number; readonly agy?: number } = {},
): Promise<{ h: Harness; run: PrepareResult; result: FanOutResult }> {
  const { h, run } = await prepared({
    agy: [{ ...agyFindings({ findings: agyFound, no_findings_reason: 'Walked every failure path.' }), delayMs: delays.agy ?? 0 }],
    families: ['mistral', 'google'],
    vibe: [
      {
        delayMs: delays.vibe ?? 0,
        exitCode: 0,
        exportJson: isolatedExport(),
        stdout: vibeStdout(fenced({ findings: vibeFindings, no_findings_reason: 'Compared every story.' })),
      },
    ],
  })
  const result = await h.commands.fanOut.run({ run_dir: run.run_dir })
  return { h, result, run }
}

const VIBE = 'c1-vibe-mistral-large'
const AGY = 'c2-agy-gemini-flash-high'

describe('the spec roster across families', () => {
  it('runs Mistral Large and Gemini Flash at base depth', async () => {
    const { run } = await prepared({ families: ['mistral', 'google'] })

    expect(run.critics.map((critic) => [critic.critic_id, critic.family, critic.lenses])).toEqual([
      [VIBE, 'mistral', ['fidelity', 'testability']],
      [AGY, 'google', ['failure-modes', 'security']],
    ])
    expect(run.dropped).toEqual([])
  })

  it("replaces a critic from the orchestrator's family with an allowed fallback from another family", async () => {
    const { run } = await prepared({ families: ['mistral', 'google', 'zhipu'], orchestrator: 'google' })

    expect(run.critics.map((critic) => [critic.critic_id, critic.family, critic.lenses])).toEqual([
      [VIBE, 'mistral', ['fidelity', 'testability']],
      ['c2-vibe-glm', 'zhipu', ['failure-modes', 'security']],
    ])
    expect(run.dropped).toEqual([
      { client: 'agy-gemini-flash-high', family: 'google', reason: 'orchestrator-family', replaced_by: 'c2-vibe-glm' },
    ])
  })

  it("drops a critic from the orchestrator's family and reports it when no fallback is left", async () => {
    const { h, run } = await prepared({
      families: ['mistral', 'google'],
      orchestrator: 'google',
      vibe: [{ exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings: [finding()] })) }],
    })
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const { summary } = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })

    expect(run.critics.map((critic) => critic.client)).toEqual(['vibe-mistral-large'])
    expect(summary).toContain(
      '- Not run: agy-gemini-flash-high (google): orchestrator-family; no fallback left, so the stage runs with fewer critics',
    )
  })

  it('never runs two critics of one family, even through a fallback', async () => {
    const { run } = await prepared({ families: ['mistral', 'google'], orchestrator: 'mistral' })

    expect(run.critics.map((critic) => [critic.critic_id, critic.lenses])).toEqual([
      ['c1-agy-gemini-flash-high', ['fidelity', 'testability']],
    ])
    expect(run.dropped).toEqual([
      { client: 'vibe-mistral-large', family: 'mistral', reason: 'orchestrator-family', replaced_by: 'c1-agy-gemini-flash-high' },
      { client: 'agy-gemini-flash-high', family: 'google', reason: 'same-family' },
    ])
  })
})

describe('fan-out across families', () => {
  it('starts both critics before either finishes', async () => {
    const { h } = await bothFamilies([finding()], [], { agy: 40, vibe: 40 })

    expect(h.launches.slice(0, 2).sort()).toEqual(['start agy', 'start vibe'])
  })

  it("never shows a critic another critic's output, the critics' identities or an earlier session", async () => {
    const vibeClaim = 'VIBE-ONLY: the nightly job is never named in the spec.'
    const { h, run } = await bothFamilies([finding({ claim: vibeClaim })], [])

    const vibePrompt = await readFile(join(run.run_dir, 'critics', VIBE, 'prompt.md'), 'utf8')
    const agyArgs = h.agy.launches[0]?.args ?? []
    const agyPrompt = agyArgs[agyArgs.indexOf('-p') + 1] ?? ''
    for (const prompt of [vibePrompt, agyPrompt]) {
      expect(prompt).not.toContain(vibeClaim)
      expect(prompt).not.toMatch(/mistral|gemini|glm|deepseek|vibe-|agy-|c1-|c2-/i)
    }
    const vibeArgs = h.process.commands[0]?.args ?? []
    for (const flag of ['-c', '--continue', '--resume']) expect(vibeArgs).not.toContain(flag)
  })

  it('merges a finding both families raised into one that keeps both critics, every quote and the highest severity', async () => {
    const fromVibe = finding({
      evidence: [{ file: 'artifact/spec.md', kind: 'quote', value: 'within the canary window of seven seconds' }],
      id: 'V1',
      location: 'User story 3',
      severity: 'major',
      suggestion: undefined,
    })
    const fromAgy = finding({ id: 'G7', location: 'user story  3.', severity: 'critical' })

    const { result } = await bothFamilies([fromVibe], [fromAgy])

    expect(result.findings).toHaveLength(1)
    expect(result.findings[0]).toMatchObject({
      critic_finding_id: 'V1',
      id: 'F1',
      location: 'User story 3',
      raised_by: [VIBE, AGY],
      severity: 'critical',
      suggestion: 'Say that only the nightly job frobnicates widgets.',
    })
    expect(result.findings[0]?.evidence.map((e) => e.value)).toEqual([
      'within the canary window of seven seconds',
      SPEC_CANARY,
      DECISION_CANARY,
    ])
  })

  it('keeps two defects at one location apart', async () => {
    const { result } = await bothFamilies([finding()], [finding({ category: 'failure-mode', id: 'G1' })])

    expect(result.findings.map((f) => [f.id, f.category, f.raised_by])).toEqual([
      ['F1', 'contradiction', [VIBE]],
      ['F2', 'failure-mode', [AGY]],
    ])
  })

  it('merges findings without quotes when their claims match', async () => {
    const scenario = [{ kind: 'input', value: 'A widget created at 23:59 during the nightly job.' }]
    const { result } = await bothFamilies(
      [finding({ category: 'failure-mode', claim: 'Widgets created mid-run are never frobnicated.', evidence: scenario })],
      [finding({ category: 'failure-mode', claim: 'widgets created  mid-run are never frobnicated', evidence: scenario, id: 'G1' })],
    )

    expect(result.findings.map((f) => f.raised_by)).toEqual([[VIBE, AGY]])
    expect(result.findings[0]?.evidence).toHaveLength(1)
  })

  it('keeps findings without quotes apart when their claims differ', async () => {
    const scenario = [{ kind: 'input', value: 'A widget created at 23:59.' }]
    const { result } = await bothFamilies(
      [finding({ category: 'failure-mode', claim: 'Widgets created mid-run are never frobnicated.', evidence: scenario })],
      [finding({ category: 'failure-mode', claim: 'The nightly job can run twice.', evidence: scenario, id: 'G1' })],
    )

    expect(result.findings.map((f) => f.raised_by)).toEqual([[VIBE], [AGY]])
  })

  it('merges a duplicate one critic reported twice', async () => {
    const { result } = await bothFamilies([finding({ id: 'V1' }), finding({ id: 'V2', suggestion: 'Another fix.' })], [])

    expect(result.findings.map((f) => [f.critic_finding_id, f.raised_by])).toEqual([['V1', [VIBE]]])
  })
})

describe('dissent across families', () => {
  it('lists a rejected finding two families raised under Dissent, with the reason', async () => {
    const { h, run } = await bothFamilies([finding({ severity: 'major' })], [finding({ id: 'G1', severity: 'major' })])

    const { report_path, summary } = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [{ finding_id: 'F1', reason: 'CONTEXT.md allows a manual run too.', verdict: 'rejected' }],
    })

    const dissent = summary.slice(summary.indexOf('## Dissent'), summary.indexOf('## Report'))
    expect(dissent).toContain('F1 [major contradiction] User story 3')
    expect(dissent).toContain('(raised by mistral and google) Rejected: CONTEXT.md allows a manual run too.')
    const report = await readFile(report_path, 'utf8')
    expect(report).toContain('- F1: CONTEXT.md allows a manual run too.')
    expect(report).toContain(`Raised by: ${VIBE}, ${AGY} (mistral, google)`)
  })

  it('names the families that agree on an accepted finding', async () => {
    const { h, run } = await bothFamilies([finding()], [finding({ id: 'G1' })])

    const { summary } = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })

    expect(summary).toContain('(raised by mistral and google)')
    expect(summary.slice(summary.indexOf('## Dissent'))).toContain('none')
  })
})
