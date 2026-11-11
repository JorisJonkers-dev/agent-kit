import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  fenced,
  finding,
  harness,
  isolatedExport,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import { planStepIds, planTarget } from './domain/refactoring-plan.js'
import type { FanOutResult, PrepareResult } from './index.js'

/**
 * The plan stage: a critique stage of the feature flow that runs for a
 * refactor draft. It freezes the draft's refactoring plan, the checker's
 * report on it, the ADRs in force, the facts and the tree the plan targets,
 * and its critics check the plan's steps, their order and their anchors
 * before the spec is written.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const ORDER = [
  "import { db } from '../adapters/db'",
  '',
  'export interface Order {',
  '  readonly id: string',
  '}',
  '',
  'export const order = (id: string): Order => ({ id })',
  '',
  'export const find = (id: string) => db.find(id)',
  '',
  'export const save = (order: Order) => db.save(order)',
  'export const remove = (id: string) => db.remove(id)',
  '',
].join('\n')
const DB = 'export const db = { find: (id: string) => id, remove: (id: string) => id, save: (order: unknown) => order }\n'
const ADR = '# The domain never imports an adapter\n\nStatus: accepted\n\nThe domain layer declares its ports; adapters implement them.\n'
const SUPERSEDED = '# The domain imports its adapter directly\n\nStatus: superseded\n\nThe domain imported the adapter it needed.\n'
const README = '# Widgets\n\nA small service. The domain holds the rules and the adapters hold the I/O.\n'
const PACKAGE = '{\n  "name": "widgets",\n  "private": true\n}\n'

/** The plan the fixture drafts: two steps, a target, a safety net and no open decision. */
const PLAN = [
  '# Refactoring plan',
  '',
  '## Goal',
  '',
  'Unblock the importer split. Behaviour is unchanged. Evidence: `src/domain/order.ts`.',
  '',
  '## Target',
  '',
  'The importer is a module with one interface.',
  '',
  '## Safety net',
  '',
  'The importer suite.',
  '',
  '## Steps',
  '',
  '### S01: Characterize the order module',
  '',
  '- **Kind**: characterize',
  '- **Technique**: Characterization Test',
  '- **Anchor**: `src/domain/order.ts#order`',
  '- **Touches**: `src/domain/order.ts`',
  '- **Safety net**: none',
  '- **Main stays green because**: Only tests are added',
  '- **Rollback**: Revert the commit',
  '- **Transitional element**: none',
  '- **Blocked by**: none',
  '- **Done when**: The suite passes',
  '',
  '### S02: Extract the order port',
  '',
  '- **Kind**: structure',
  '- **Technique**: Extract Interface',
  '- **Anchor**: ADR-0001',
  '- **Touches**: `src/domain/order.ts`',
  '- **Safety net**: none, covered by S01',
  '- **Main stays green because**: The suite passes after this step alone',
  '- **Rollback**: Revert the commit',
  '- **Transitional element**: none',
  '- **Blocked by**: S01',
  '- **Done when**: The suite passes',
  '',
  '## Not doing',
  '',
  'Renaming the importer.',
  '',
  '## Open decisions',
  '',
  'None.',
  '',
].join('\n')

/** A step block of the ten fields the format check wants, in the fixture's order. */
function tidyStep(id: string, overrides: Record<string, string> = {}): string {
  const fields: Record<string, string> = {
    Kind: 'tidy',
    Technique: 'Rename',
    Anchor: '`src/domain/order.ts`',
    Touches: '`src/domain/order.ts`',
    'Safety net': 'none',
    'Main stays green because': 'Only names change',
    Rollback: 'Revert the commit',
    'Transitional element': 'none',
    'Blocked by': 'none',
    'Done when': 'The suite passes',
    ...overrides,
  }
  return [`### ${id}: Tidy the importer`, '', ...Object.entries(fields).map(([name, value]) => `- **${name}**: ${value}`), ''].join('\n')
}

/** The fixture plan with `count` more tidy steps after S02, for the step threshold. */
function planWithSteps(count: number): string {
  const extra = Array.from({ length: count }, (_, n) => tidyStep(`S${String(n + 3).padStart(2, '0')}`))
  return PLAN.replace('\n## Not doing', `\n${extra.join('\n')}\n## Not doing`)
}

/** The fixture plan with one migrate step that touches the orders table. */
function planWithMigrate(): string {
  const migrate = tidyStep('S03', {
    Kind: 'migrate',
    Technique: 'Expand and contract the orders table',
    Touches: 'the orders table',
  })
  return PLAN.replace('\n## Not doing', `\n${migrate}\n## Not doing`)
}

interface Options {
  readonly adr?: boolean
  readonly code?: boolean
  readonly plan?: string | null
  readonly routingOverride?: unknown
  readonly tracker?: 'github' | 'local-markdown'
}

/** Where the tracker convention puts a draft's plan, as the fake git lays it out. */
function planPath(h: Harness, tracker: 'github' | 'local-markdown', draft = 'widgets'): string {
  return tracker === 'local-markdown'
    ? join(h.repo, '.scratch', draft, 'refactoring-plan.md')
    : join(h.repo, '.git', 'critique', 'plans', draft.replaceAll('/', '~'), 'refactoring-plan.md')
}

/**
 * A repository with an ADR in force, a superseded one, a README, a
 * `package.json`, the order module and its adapter, and the draft's plan at
 * the tracker convention's path. The allowlist lets Mistral, DeepSeek and
 * Zhipu read it, so no critic of the plan stage is dropped.
 */
async function fixture(options: Options = {}): Promise<Harness> {
  const h = await harness({
    ...(options.routingOverride === undefined ? {} : { routingOverride: options.routingOverride }),
    ...(options.tracker === undefined ? {} : { tracker: options.tracker }),
  })
  open.push(h)
  await h.commands.allow.run({
    code: options.code ?? true,
    families: ['mistral', 'deepseek', 'zhipu'],
    history: false,
    repo: h.repo,
  })
  if (options.adr ?? true) {
    await h.writeText(join(h.repo, 'docs', 'adr', '0001-layers.md'), ADR)
    await h.writeText(join(h.repo, 'docs', 'adr', '0002-direct.md'), SUPERSEDED)
  }
  await h.writeText(join(h.repo, 'README.md'), README)
  await h.writeText(join(h.repo, 'package.json'), PACKAGE)
  await h.writeText(join(h.repo, 'src', 'domain', 'order.ts'), ORDER)
  await h.writeText(join(h.repo, 'src', 'adapters', 'db.ts'), DB)
  const plan = options.plan === undefined ? PLAN : options.plan
  if (plan !== null) await h.writeText(planPath(h, options.tracker ?? 'local-markdown'), plan)
  await h.git.commit()
  return h
}

async function prepared(h: Harness, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const run = await h.commands.prepare.run({
    flow: 'draft:widgets',
    orchestrator_family: 'anthropic',
    repo: h.repo,
    stage: 'plan',
    ...extra,
  })
  if (run.status !== 'ready') throw new Error(`not ready: ${JSON.stringify(run)}`)
  return run
}

function critic(agent: string, findings: readonly Record<string, unknown>[]): VibeScript {
  const reply = findings.length === 0 ? { findings, no_findings_reason: 'Read the plan, the records and the tree.' } : { findings }
  return { agent, exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced(reply)) }
}

/** What each critic of the run reports: the plan critics, and the deep run's coupling critic. */
interface Replies {
  readonly coupling?: readonly Record<string, unknown>[]
  readonly plan?: readonly Record<string, unknown>[]
}

async function reviewed(h: Harness, replies: Replies, extra: Record<string, unknown> = {}): Promise<{ result: FanOutResult; run: PrepareResult }> {
  const run = await prepared(h, extra)
  h.process.queue(
    critic('critic-deepseek-flash', replies.plan ?? []),
    critic('critic-mistral-large', []),
    critic('critic-glm', replies.coupling ?? []),
  )
  const result = await h.commands.fanOut.run({ run_dir: run.run_dir })
  return { result, run }
}

/** The client and role of each critic the run launches, in the runner's order. */
function roster(run: PrepareResult): readonly (readonly [string, string])[] {
  return run.critics.map((critic) => [critic.client, critic.role] as const)
}

const STEP_QUOTE = { file: 'artifact/refactoring-plan.md', kind: 'quote', value: 'Extract the order port' }
const DECISION_QUOTE = {
  file: 'references/docs/adr/0001-layers.md',
  kind: 'quote',
  value: 'The domain layer declares its ports; adapters implement them.',
}
const FACT_QUOTE = { file: 'facts/INDEX.md', kind: 'quote', value: 'Facts are settled: cite them, never recompute them.' }
const TRACE = { kind: 'trace', value: 'tree/src/domain/order.ts:1 imports ../adapters/db' }

/** A finding of the plan critic: one step, one rule, the step's line and the record beside it. */
function planFinding(id: string, overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return finding({
    category: 'structure',
    check: 'no-rollback',
    claim: 'S02 extracts the port before the suite covers the importer.',
    evidence: [STEP_QUOTE, DECISION_QUOTE],
    id,
    location: 'S02: no-rollback',
    severity: 'major',
    suggestion: 'Add the characterization test before the extraction.',
    ...overrides,
  })
}

/** The pre-verdict fan-out gave each finding, in the order the critics raised them. */
async function preVerdicts(options: Options, findings: readonly Record<string, unknown>[]): Promise<readonly unknown[]> {
  const h = await fixture(options)
  const { result } = await reviewed(h, { plan: findings })
  return result.findings.map((raised) => raised.pre_verdict)
}

/** The flow state a draft reports, narrowed to the fields these tests read. */
interface FlowView {
  readonly next: {
    readonly kind: string
    readonly reason?: string
    readonly resume_run_id?: string
    readonly skill?: string
    readonly stage?: string
  }
  readonly state: {
    readonly grilled?: boolean
    readonly plan_path?: string | null
    readonly refactor?: boolean
    readonly stages: Record<string, { readonly run_ids?: readonly string[]; readonly status?: string } | undefined>
  }
}

async function flowState(h: Harness, record?: Record<string, unknown>, draft = 'widgets'): Promise<FlowView> {
  const result = await h.commands.state.run(record === undefined ? { draft, repo: h.repo } : { draft, record, repo: h.repo })
  return result as unknown as FlowView
}

describe('a plan prepare', () => {
  it('is refused while the code permission is off, and names how to turn it on', async () => {
    const h = await fixture({ code: false })

    const result = await h.commands.prepare.run({ flow: 'draft:widgets', orchestrator_family: 'anthropic', repo: h.repo, stage: 'plan' })

    expect(result).toMatchObject({
      message: expect.stringContaining('allows documents but not code') as unknown,
      reason: 'code-not-allowed',
      status: 'refused',
    })
  })

  it('freezes the plan, the checker report, the ADRs in force, the manifest, the facts and the tree, the plan files first', async () => {
    const h = await fixture()

    const run = await prepared(h)

    expect(run.artifact).toMatchObject({ labels: [], reference: '.scratch/widgets/refactoring-plan.md', tracker: 'none' })
    expect(run.packet_files.slice(0, 4)).toEqual([
      'artifact/refactoring-plan.md',
      'check/plan.json',
      'references/CONTEXT.md',
      'references/docs/adr/0001-layers.md',
    ])
    expect(run.packet_files).toContain('tree/MANIFEST.tsv')
    expect(run.packet_files).toContain('facts/INDEX.md')
    // A record that is superseded is no decision the plan can be checked against.
    expect(run.packet_files).not.toContain('references/docs/adr/0002-direct.md')
    // The files the plan names in its Anchor and Touches are the first the tree sample takes.
    expect(run.packet_files).toContain('tree/src/domain/order.ts')
    const index = await h.readText(join(run.packet_dir, 'facts', 'INDEX.md'))
    expect(index).toContain('- `src/domain/order.ts` (named, 12 lines): sent in full')
    expect(run.repository?.tree_files).toBe(1)
  })

  it('freezes the checker report with the steps and the stored-data steps it found', async () => {
    const h = await fixture({ plan: planWithMigrate() })

    const run = await prepared(h)
    const report = JSON.parse(await h.readText(join(run.packet_dir, 'check', 'plan.json'))) as {
      readonly errors: readonly unknown[]
      readonly steps: readonly { readonly id: string }[]
      readonly stored_data_steps: readonly unknown[]
    }

    expect(report.errors).toEqual([])
    expect(report.steps.map((step) => step.id)).toEqual(['S01', 'S02', 'S03'])
    expect(report.stored_data_steps).toHaveLength(1)
  })

  it('is refused, before any packet exists, when the plan and its references alone pass the packet cap', async () => {
    const h = await fixture({ routingOverride: { packet: { cap_bytes: 100 } } })

    expect(await h.commands.prepare.run({ flow: 'draft:widgets', orchestrator_family: 'anthropic', repo: h.repo, stage: 'plan' })).toMatchObject({
      reason: 'packet-over-cap',
      status: 'refused',
    })
  })

  it('is refused when the repository tracks its specs somewhere the runner cannot read, since the plan has no convention there', async () => {
    const h = await harness({ tracker: 'gitlab' })
    open.push(h)
    await h.commands.allow.run({ code: true, families: ['mistral', 'deepseek', 'zhipu'], repo: h.repo })

    expect(await h.commands.prepare.run({ flow: 'draft:widgets', orchestrator_family: 'anthropic', repo: h.repo, stage: 'plan' })).toMatchObject({
      reason: 'unsupported-tracker',
      status: 'refused',
    })
  })

  it('refuses a plan file that is not there', async () => {
    const h = await fixture({ plan: null })

    const result = await h.commands.prepare.run({ flow: 'draft:widgets', orchestrator_family: 'anthropic', repo: h.repo, stage: 'plan' })

    expect(result).toMatchObject({ reason: 'no-plan', status: 'refused' })
  })

  it('refuses a flow that is not a draft when no plan is named', async () => {
    const h = await fixture({ plan: null })

    await expect(
      h.commands.prepare.run({ flow: 'spec:182', orchestrator_family: 'anthropic', repo: h.repo, stage: 'plan' }),
    ).rejects.toThrow(/plan needs plan/)
    await expect(h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, stage: 'plan' })).rejects.toThrow(/plan needs plan/)
  })

  it('refuses a draft name with a dot segment', async () => {
    const h = await fixture({ plan: null })

    await expect(
      h.commands.prepare.run({ flow: 'draft:a/../b', orchestrator_family: 'anthropic', repo: h.repo, stage: 'plan' }),
    ).rejects.toThrow(/has an empty or dot segment/)
  })

  it('refuses a plan outside the repository and its git directory', async () => {
    const h = await fixture({ plan: null })

    await expect(prepared(h, { plan: join(h.repo, '..', 'elsewhere', 'refactoring-plan.md') })).rejects.toThrow(/is outside the repository/)
  })

  it('reads a plan the caller names, absolute or relative to the repository root', async () => {
    const h = await fixture({ plan: null })
    await h.writeText(join(h.repo, 'plans', 'importer.md'), PLAN)

    const relative = await prepared(h, { plan: 'plans/importer.md' })
    const absolute = await prepared(h, { plan: join(h.repo, 'plans', 'importer.md') })

    expect(relative.artifact).toMatchObject({ reference: 'plans/importer.md' })
    expect(absolute.artifact).toMatchObject({ reference: 'plans/importer.md' })
  })

  it('reads a plan that lies in the git common directory', async () => {
    const h = await fixture({ plan: null, tracker: 'github' })
    const path = planPath(h, 'github')
    await h.writeText(path, PLAN)

    const run = await prepared(h, { plan: path })

    expect(run.artifact).toMatchObject({ reference: '.git/critique/plans/widgets/refactoring-plan.md' })
  })

  it('stores a draft name with a slash under the git directory with ~ in its place', async () => {
    const h = await fixture({ plan: null, tracker: 'github' })
    await h.writeText(planPath(h, 'github', 'team/widgets'), PLAN)

    const run = await prepared(h, { flow: 'draft:team/widgets' })

    expect(run.artifact).toMatchObject({ reference: '.git/critique/plans/team~widgets/refactoring-plan.md' })
  })

  it('charges the run to the draft flow the plan belongs to', async () => {
    const h = await fixture()

    const run = await prepared(h)
    const manifest = JSON.parse(await h.readText(run.manifest_path)) as { readonly flow?: string }

    expect(manifest.flow).toBe('draft:widgets')
  })
})

describe('the plan depth signals', () => {
  it('stays at base for a two-step plan, and says what it measured', async () => {
    const h = await fixture()

    const run = await prepared(h)

    expect(run.depth).toBe('base')
    expect(run.depth_choice.measures.steps).toMatchObject({ threshold: 8, value: 2 })
    expect(run.depth_choice.measures.stored_data_steps).toMatchObject({ threshold: 0, value: 0 })
    expect(run.depth_choice.signals).toEqual([])
    expect(roster(run)).toEqual([
      ['vibe-deepseek-flash', 'plan-critic'],
      ['vibe-mistral-large', 'plan-critic'],
    ])
    expect(JSON.stringify(run.critics)).toContain('incremental-safety')
    expect(JSON.stringify(run.critics)).toContain('evidence-anchoring')
    // Every critic's family is allowed, so none is dropped.
    expect(run.dropped).toEqual([])
  })

  it('goes deep above eight steps, and adds the coupling critic', async () => {
    const h = await fixture({ plan: planWithSteps(7) })

    const run = await prepared(h)

    expect(run.depth).toBe('deep')
    expect(run.depth_choice.measures.steps?.value).toBe(9)
    expect(run.depth_choice.signals.map((signal) => (signal.kind === 'threshold' ? signal.measure : signal.kind))).toEqual(['steps'])
    expect(roster(run)).toEqual([
      ['vibe-deepseek-flash', 'plan-critic'],
      ['vibe-mistral-large', 'plan-critic'],
      ['vibe-glm', 'plan-critic'],
    ])
    expect(JSON.stringify(run.critics)).toContain('coupling')
    expect(JSON.stringify(run.critics)).toContain('test-seams')
  })

  it('goes deep when a migrate step touches stored data', async () => {
    const h = await fixture({ plan: planWithMigrate() })

    const run = await prepared(h)

    expect(run.depth).toBe('deep')
    expect(run.depth_choice.measures.stored_data_steps?.value).toBe(1)
    expect(run.depth_choice.signals.map((signal) => (signal.kind === 'threshold' ? signal.measure : signal.kind))).toEqual(['stored_data_steps'])
  })

  it('goes deep when the plan names a risk term', async () => {
    const h = await fixture({ plan: PLAN.replace('Unblock the importer split.', 'Unblock the importer split without touching authentication.') })

    const run = await prepared(h)

    expect(run.depth).toBe('deep')
    expect(JSON.stringify(run.depth_choice.signals)).toContain('authentication')
  })
})

describe('the plan evidence rule', () => {
  it('hints a location that names no step the plan has', async () => {
    const verdicts = await preVerdicts({}, [
      planFinding('F1', { location: 'no-rollback' }),
      planFinding('F2', { location: 'S99: no-rollback' }),
    ])

    expect(verdicts[0]).toMatchObject({ reason_code: 'no-plan-step', verdict: 'hint' })
    expect(verdicts[1]).toMatchObject({ reason_code: 'no-plan-step', verdict: 'hint' })
  })

  it('hints a location that names a step but no known rule', async () => {
    const verdicts = await preVerdicts({}, [planFinding('F1', { location: 'S01' })])

    expect(verdicts[0]).toMatchObject({ reason_code: 'no-plan-rule', verdict: 'hint' })
  })

  it('hints a finding whose evidence quotes no line of the plan', async () => {
    const verdicts = await preVerdicts({}, [planFinding('F1', { evidence: [DECISION_QUOTE] })])

    expect(verdicts[0]).toMatchObject({ reason_code: 'no-step-quote', verdict: 'hint' })
  })

  it('hints a finding with a step quote and nothing beside it', async () => {
    const verdicts = await preVerdicts({}, [planFinding('F1', { evidence: [STEP_QUOTE] })])

    expect(verdicts[0]).toMatchObject({ reason_code: 'no-plan-anchor', verdict: 'hint' })
  })

  it('passes a finding with a step quote and an ADR quote, a fact quote or a trace beside it', async () => {
    const verdicts = await preVerdicts({}, [
      planFinding('F1', { evidence: [STEP_QUOTE, DECISION_QUOTE] }),
      planFinding('F2', { evidence: [STEP_QUOTE, FACT_QUOTE], location: 'S01: no-rollback' }),
      planFinding('F3', { check: 'mixed-change', evidence: [STEP_QUOTE, TRACE], location: 'S02: mixed-change' }),
    ])

    expect(verdicts[0]).toBeUndefined()
    expect(verdicts[1]).toBeUndefined()
    expect(verdicts[2]).toBeUndefined()
  })

  it('hints a check the critic lens does not declare', async () => {
    // `stale-hotspot` is the evidence-anchoring lens's check, and this critic carries the incremental-safety lens.
    const verdicts = await preVerdicts({}, [planFinding('F1', { check: 'stale-hotspot', location: 'S01: stale-hotspot' })])

    expect(verdicts[0]).toMatchObject({ verdict: 'hint' })
    expect(JSON.stringify(verdicts[0])).toContain('unknown-check')
  })
})

describe('the deep roster of a plan run', () => {
  it('judges a coupling or test-seams finding by the same rule, naming its lens check as the rule', async () => {
    const h = await fixture()
    const { result } = await reviewed(
      h,
      { coupling: [planFinding('F1', { check: 'no-fake', location: 'S02: no-fake' })] },
      { depth: 'deep' },
    )

    expect(result.findings.map((raised) => raised.pre_verdict?.reason_code)).toEqual([undefined])
  })
})

describe('the fixed severity of a plan finding', () => {
  it('raises a no-rollback finding to critical, whatever the critic wrote', async () => {
    const h = await fixture()

    const { result } = await reviewed(h, { plan: [planFinding('F1', { severity: 'minor' })] })

    expect(result.findings[0]).toMatchObject({ declared_severity: 'critical', severity: 'critical' })
  })

  it('raises a mixed-change finding to major', async () => {
    const h = await fixture()

    const { result } = await reviewed(h, {
      plan: [planFinding('F1', { check: 'mixed-change', location: 'S01: mixed-change', severity: 'minor' })],
    })

    expect(result.findings[0]).toMatchObject({ severity: 'major' })
  })

  it('lowers a stale-hotspot finding the mistral critic raised to minor', async () => {
    const h = await fixture()
    const run = await prepared(h)
    h.process.queue(
      critic('critic-deepseek-flash', []),
      critic('critic-mistral-large', [planFinding('F1', { check: 'stale-hotspot', location: 'S01: stale-hotspot', severity: 'critical' })]),
    )

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(result.findings[0]).toMatchObject({ severity: 'minor' })
  })
})

describe('a plan run over the fixture repository', () => {
  it('yields a finding that quotes the step and the ADR, and records it as accepted', async () => {
    const h = await fixture()

    const { result, run } = await reviewed(h, { plan: [planFinding('F1')] })

    expect(result.findings).toHaveLength(1)
    expect(result.findings[0]?.pre_verdict).toBeUndefined()

    const recorded = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [{ finding_id: 'F1', reason: 'The step has no rollback.', verdict: 'accepted' }],
    })

    expect(recorded.counts.accepted).toBe(1)
    const report = await readFile(recorded.report_path, 'utf8')
    expect(report).toMatch(/### F1 \[critical structure\]/)
    expect(report).toContain('S02: no-rollback')
    expect(report).toContain('- quote (artifact/refactoring-plan.md): Extract the order port')
    expect(report).toContain('- quote (references/docs/adr/0001-layers.md): The domain layer declares its ports; adapters implement them.')
    // The stage reads no tracker: nothing was viewed, linked or created through gh.
    expect(h.gh.requests).toEqual([])
    expect(h.gh.linkRequests).toEqual([])
  })

  it('opens a second round after a run accepted more than three critical findings', async () => {
    const h = await fixture({ plan: planWithSteps(2) })
    const first = await prepared(h)
    h.process.queue(
      critic(
        'critic-deepseek-flash',
        ['S01', 'S02', 'S03', 'S04'].map((step, n) => planFinding(`F${String(n + 1)}`, { location: `${step}: no-rollback` })),
      ),
      critic('critic-mistral-large', []),
    )
    const result = await h.commands.fanOut.run({ run_dir: first.run_dir })
    await h.commands.record.run({
      run_dir: first.run_dir,
      verdicts: result.findings.map((raised) => ({ finding_id: raised.id, reason: 'The step has no rollback.', verdict: 'accepted' })),
    })

    const second = await prepared(h, { previous_run_dir: first.run_dir })

    expect(second.round).toBe(2)
  })
})

describe('the flow state of a draft', () => {
  it('asks for the decisions stage when the draft has no plan file', async () => {
    const h = await fixture({ plan: null })

    const view = await flowState(h, { grilled: true })

    expect(view.state.refactor).toBe(false)
    expect(view.next).toMatchObject({ kind: 'stage', stage: 'decisions' })
  })

  it('counts a draft with a plan file as grilled, and asks for the plan stage', async () => {
    const h = await fixture()

    const view = await flowState(h)

    expect(view.state.refactor).toBe(true)
    expect(view.state.grilled).toBe(true)
    expect(view.state.plan_path).toBe(join(h.repo, '.scratch', 'widgets', 'refactoring-plan.md'))
    expect(view.next).toMatchObject({ kind: 'stage', stage: 'plan' })
    expect(view.next.reason).toMatch(/refactoring plan/)
  })

  it('moves to /to-spec once the plan run is recorded', async () => {
    const h = await fixture()
    const run = await prepared(h)
    h.process.queue(critic('critic-deepseek-flash', [planFinding('F1')]), critic('critic-mistral-large', []))
    await h.commands.fanOut.run({ run_dir: run.run_dir })
    await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [{ finding_id: 'F1', reason: 'The step has no rollback.', verdict: 'accepted' }],
    })

    const view = await flowState(h)

    expect(view.next).toMatchObject({ kind: 'command', skill: '/to-spec' })
    expect(view.state.stages.plan?.run_ids).toContain(run.run_id)
  })

  it('resumes a prepared run that is not recorded yet', async () => {
    const h = await fixture()
    const run = await prepared(h)

    const view = await flowState(h)

    expect(view.next).toMatchObject({ kind: 'stage', resume_run_id: run.run_id, stage: 'plan' })
  })

  it('goes straight to /to-spec when critique is skipped on purpose', async () => {
    const h = await fixture()

    const view = await flowState(h, { no_critique: true })

    expect(view.next).toMatchObject({ kind: 'command', skill: '/to-spec' })
  })

  it('keeps a GitHub repository plan beside the flow state in the git directory', async () => {
    const h = await fixture({ tracker: 'github' })

    const view = await flowState(h)

    expect(view.state.plan_path).toBe(join(h.repo, '.git', 'critique', 'plans', 'widgets', 'refactoring-plan.md'))
    expect(view.state.refactor).toBe(true)
  })

  it('reports no plan path for a draft name that names no folder', async () => {
    const h = await fixture({ plan: null })

    const view = await flowState(h, undefined, 'a/./b')

    expect(view.state.plan_path).toBeNull()
    expect(view.state.refactor).toBe(false)
  })
})

describe('the plan helpers', () => {
  it('reads the step and the rule a location names, and neither when it names none', () => {
    expect(planTarget('S03: mixed-change', ['S01', 'S02', 'S03'])).toEqual({ rule: 'mixed-change', step: 'S03' })
    expect(planTarget('no-rollback', ['S01'])).toEqual({ rule: 'no-rollback' })
    expect(planTarget('S01', ['S01'])).toEqual({ step: 'S01' })
    expect(planTarget('S99: no-rollback', ['S01'])).toEqual({ rule: 'no-rollback' })
    expect(planTarget('nothing here', ['S01'])).toEqual({})
    // A rule inside a longer word, and a step id inside a path, name neither.
    expect(planTarget('S01: no-rollbackish', ['S01'])).toEqual({ step: 'S01' })
    expect(planTarget('tree/src/domain/order.ts#S01', ['S01'])).toEqual({})
  })

  it('reads the step ids a checker report holds, and none from text that is not one', () => {
    expect(planStepIds(JSON.stringify({ steps: [{ id: 'S01' }, { id: 'S02' }, { name: 'no id' }, { id: 3 }, null] }))).toEqual(['S01', 'S02'])
    expect(planStepIds('not json')).toEqual([])
    expect(planStepIds(JSON.stringify({ steps: 'none' }))).toEqual([])
    expect(planStepIds(JSON.stringify(null))).toEqual([])
  })
})
