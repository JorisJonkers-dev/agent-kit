import { createHash } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  ASSETS_ROOT,
  DECISION_CANARY,
  fenced,
  finding,
  harness,
  isolatedExport,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import type { PrepareResult } from './index.js'

/**
 * What `/critique-decisions` (#183) relies on the runner for: a packet of the
 * decisions grilling recorded (the decision docs, or the orchestrator's
 * summary after `/grill-me`) with the feature they bear on, the shipped
 * decisions roster, and the stage's evidence and verdict rules.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const FEATURE = 'Widgets get a frobnication window the owner can set per widget.'

async function optedIn(...args: Parameters<typeof harness>): Promise<Harness> {
  const h = await harness(...args)
  open.push(h)
  await h.commands.allow.run({ families: ['mistral'], repo: h.repo })
  return h
}

async function prepared(h: Harness, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const run = await h.commands.prepare.run({ feature: FEATURE, orchestrator_family: 'anthropic', repo: h.repo, stage: 'decisions', ...extra })
  if (run.status !== 'ready') throw new Error(`not ready: ${JSON.stringify(run)}`)
  return run
}

describe('the decisions packet after /grill-with-docs', () => {
  it('holds the decision docs as the artifact, beside the feature statement', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, 'docs', 'adr', '0001-nightly-job.md'), '# The nightly job frobnicates\n')

    const run = await prepared(h)

    expect(run.packet_files).toEqual(['feature/feature.md', 'artifact/CONTEXT.md', 'artifact/docs/adr/0001-nightly-job.md'])
    expect(await readFile(join(run.packet_dir, 'feature', 'feature.md'), 'utf8')).toBe(`# Feature\n\n${FEATURE}\n`)
    expect(await readFile(join(run.packet_dir, 'artifact', 'CONTEXT.md'), 'utf8')).toContain(DECISION_CANARY)
    expect(run.artifact).toEqual({ labels: [], reference: 'decision-docs', tracker: 'none' })
  })

  it('reads no tracker, so a repository whose tracker the critique cannot read still runs', async () => {
    const h = await optedIn({ tracker: 'gitlab' })

    const run = await prepared(h)

    expect(run.status).toBe('ready')
  })

  it('is refused when the repository has no decision docs to review', async () => {
    const h = await optedIn()
    await h.removeFile(join(h.repo, 'CONTEXT.md'))

    const result = await h.commands.prepare.run({ feature: FEATURE, orchestrator_family: 'anthropic', repo: h.repo, stage: 'decisions' })

    expect(result).toEqual({
      message:
        'found no decision docs (docs/agents/domain.md, GLOSSARY.md, CONTEXT.md or docs/adr/); after /grill-me, pass the decisions as decision_summary',
      reason: 'no-decision-docs',
      status: 'refused',
    })
  })

  it('reviews the decision docs the caller names instead', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, 'docs', 'decisions.md'), '# Decisions\n')

    const run = await prepared(h, { references: ['docs/decisions.md'] })

    expect(run.packet_files).toEqual(['feature/feature.md', 'artifact/docs/decisions.md'])
  })

  it('needs a feature statement', async () => {
    const h = await optedIn()

    await expect(h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, stage: 'decisions' })).rejects.toThrow(
      /decisions needs feature: a short statement of the feature the decisions are for/,
    )
  })

  it('keeps the feature statement short', async () => {
    const h = await optedIn()

    await expect(prepared(h, { feature: Array.from({ length: 6 }, (_, n) => `Line ${String(n + 1)}.`).join('\n') })).rejects.toThrow(
      /feature must be at most 5 lines; it has 6/,
    )
  })
})

describe('the decisions packet after /grill-me', () => {
  const SUMMARY = '- Widgets are frobnicated by the nightly job only.\n- The window is seven seconds.\n'

  it('holds the decision summary as the artifact, with the decision docs as reference material', async () => {
    const h = await optedIn()

    const run = await prepared(h, { decision_summary: SUMMARY })

    expect(run.packet_files).toEqual(['feature/feature.md', 'artifact/decision-summary.md', 'references/CONTEXT.md'])
    expect(await readFile(join(run.packet_dir, 'artifact', 'decision-summary.md'), 'utf8')).toBe(SUMMARY)
    expect(run.artifact).toEqual({ labels: [], reference: 'decision-summary', tracker: 'none' })
  })

  it('runs without any decision docs', async () => {
    const h = await optedIn()
    await h.removeFile(join(h.repo, 'CONTEXT.md'))

    const run = await prepared(h, { decision_summary: SUMMARY })

    expect(run.packet_files).toEqual(['feature/feature.md', 'artifact/decision-summary.md'])
  })

  it('keeps the summary to at most 40 lines', async () => {
    const h = await optedIn()
    const long = Array.from({ length: 41 }, (_, n) => `- Decision ${String(n + 1)}.`).join('\n')

    await expect(prepared(h, { decision_summary: long })).rejects.toThrow(/decision_summary must be at most 40 lines; it has 41/)
  })
})

async function manifestOf(run: PrepareResult): Promise<{ critics: Record<string, unknown>[]; flow: string }> {
  return JSON.parse(await readFile(run.manifest_path, 'utf8')) as { critics: Record<string, unknown>[]; flow: string }
}

async function allowing(families: readonly string[], ...args: Parameters<typeof harness>): Promise<Harness> {
  const h = await harness(...args)
  open.push(h)
  await h.commands.allow.run({ families, repo: h.repo })
  return h
}

const slots = (manifest: { critics: Record<string, unknown>[] }) =>
  manifest.critics.map((critic) => [critic.critic_id, critic.role, critic.lenses, critic.max_price_usd, critic.time_limit_s])

describe('the shipped decisions roster', () => {
  it('runs Mistral Large and Gemini Flash as gap critics at base depth, each capped at $0.30 and 4 minutes', async () => {
    const h = await allowing(['mistral', 'google', 'zhipu'])

    const run = await prepared(h)

    expect(run.depth).toBe('base')
    expect(slots(await manifestOf(run))).toEqual([
      ['c1-vibe-mistral-large', 'gap-critic', ['unasked-questions'], 0.3, 240],
      ['c2-agy-gemini-flash-high', 'gap-critic', ['unasked-questions', 'failure-modes'], 0.3, 240],
    ])
  })

  it('adds GLM on the integration lens at deep depth', async () => {
    const h = await allowing(['mistral', 'google', 'zhipu'])

    const run = await prepared(h, { depth: 'deep' })

    expect(slots(await manifestOf(run))).toEqual([
      ['c1-vibe-mistral-large', 'gap-critic', ['unasked-questions'], 0.3, 240],
      ['c2-agy-gemini-flash-high', 'gap-critic', ['unasked-questions', 'failure-modes'], 0.3, 240],
      ['c3-vibe-glm', 'gap-critic', ['integration'], 0.3, 240],
    ])
  })

  it('counts each run as its own flow unless the caller names one', async () => {
    const h = await optedIn()

    const own = await prepared(h)
    const named = await prepared(h, { flow: 'widgets-window' })

    expect((await manifestOf(own)).flow).toBe(own.run_id)
    expect((await manifestOf(named)).flow).toBe('widgets-window')
  })
})

const EVERY_FAMILY = ['mistral', 'google', 'zhipu', 'deepseek'] as const
const ADR_PATH = join('docs', 'adr', '0001-nightly-job.md')
const ADR_DECISION = 'We frobnicate widgets in the nightly job only, because daytime load is already at its limit.'
const ADR_CRITIC = ['c3-vibe-deepseek-flash', 'adr-reviewer', ['adr-record'], 0.3, 240]

async function withAdr(h: Harness): Promise<Harness> {
  await h.writeText(join(h.repo, ADR_PATH), `# The nightly job frobnicates\n\n${ADR_DECISION}\n`)
  return h
}

describe('an ADR among the decisions under review', () => {
  it('adds DeepSeek Flash as an adr-reviewer on the adr-record lens, at the stage’s caps', async () => {
    const h = await withAdr(await allowing(EVERY_FAMILY))

    const run = await prepared(h)

    expect(slots(await manifestOf(run))).toEqual([
      ['c1-vibe-mistral-large', 'gap-critic', ['unasked-questions'], 0.3, 240],
      ['c2-agy-gemini-flash-high', 'gap-critic', ['unasked-questions', 'failure-modes'], 0.3, 240],
      ADR_CRITIC,
    ])
    expect(run.critics.map((critic) => [critic.critic_id, critic.role, critic.lenses])).toContainEqual([
      'c3-vibe-deepseek-flash',
      'adr-reviewer',
      ['adr-record'],
    ])
  })

  it('adds no adr-record critic when the decisions under review hold no ADR', async () => {
    const h = await allowing(EVERY_FAMILY)

    const run = await prepared(h)

    expect(run.critics.map((critic) => critic.role)).toEqual(['gap-critic', 'gap-critic'])
  })

  it('adds none after /grill-me, whose ADRs are reference material rather than the artifact', async () => {
    const h = await withAdr(await allowing(EVERY_FAMILY))

    const run = await prepared(h, { decision_summary: '- The window is seven seconds.\n' })

    expect(run.packet_files).toContain('references/docs/adr/0001-nightly-job.md')
    expect(run.critics.map((critic) => critic.role)).toEqual(['gap-critic', 'gap-critic'])
  })

  it('counts a record of a context’s own ADR directory, but not another doc the domain doc names', async () => {
    const h = await allowing(EVERY_FAMILY)
    await h.writeText(join(h.repo, 'docs', 'agents', 'domain.md'), '# Domain docs\n\nSee `docs/decisions.md`.\n')
    await h.writeText(join(h.repo, 'docs', 'decisions.md'), '# Decisions\n')
    const plain = await prepared(h)
    await h.writeText(join(h.repo, 'docs', 'agents', 'domain.md'), '# Domain docs\n\nSee `src/billing/docs/adr/`.\n')
    await h.writeText(join(h.repo, 'src', 'billing', 'docs', 'adr', '0001-invoices.md'), '# Invoices are monthly\n')
    const context = await prepared(h)

    expect(plain.critics.map((critic) => critic.role)).toEqual(['gap-critic', 'gap-critic'])
    expect(context.critics.map((critic) => critic.role)).toEqual(['gap-critic', 'gap-critic', 'adr-reviewer'])
  })

  it('runs it before the deep critics, so a deep run keeps it', async () => {
    const h = await withAdr(await allowing(EVERY_FAMILY))

    const run = await prepared(h, { depth: 'deep' })

    expect(slots(await manifestOf(run)).map(([id, role]) => [id, role])).toEqual([
      ['c1-vibe-mistral-large', 'gap-critic'],
      ['c2-agy-gemini-flash-high', 'gap-critic'],
      ['c3-vibe-deepseek-flash', 'adr-reviewer'],
      ['c4-vibe-glm', 'gap-critic'],
    ])
  })
})

describe('the adr-record critic’s family rules', () => {
  it('drops it when DeepSeek is the orchestrator’s family', async () => {
    const h = await withAdr(await allowing(EVERY_FAMILY))

    const run = await prepared(h, { orchestrator_family: 'deepseek' })

    expect(run.critics.map((critic) => critic.role)).toEqual(['gap-critic', 'gap-critic'])
    expect(run.dropped).toEqual([{ client: 'vibe-deepseek-flash', family: 'deepseek', reason: 'orchestrator-family' }])
  })

  it('drops it when the allowlist does not permit DeepSeek', async () => {
    const h = await withAdr(await allowing(['mistral', 'google']))

    const run = await prepared(h)

    expect(run.dropped).toEqual([{ client: 'vibe-deepseek-flash', family: 'deepseek', reason: 'family-not-allowed' }])
  })

  it('never runs it beside a critic of its own family, taking a fallback of another instead', async () => {
    const h = await withAdr(
      await allowing(EVERY_FAMILY, {
        routingOverride: {
          stages: {
            decisions: {
              adr: [{ client: 'vibe-mistral-large', lenses: ['adr-record'], max_price_usd: 0.3, role: 'adr-reviewer', time_limit_s: 240 }],
            },
          },
        },
      }),
    )

    const run = await prepared(h)

    expect(slots(await manifestOf(run)).map(([id, role]) => [id, role])).toEqual([
      ['c1-vibe-mistral-large', 'gap-critic'],
      ['c2-agy-gemini-flash-high', 'gap-critic'],
      ['c3-vibe-glm', 'adr-reviewer'],
    ])
    expect(run.dropped).toEqual([{ client: 'vibe-mistral-large', family: 'mistral', reason: 'same-family', replaced_by: 'c3-vibe-glm' }])
  })
})

describe('the decisions depth signals', () => {
  it('stays at base for a small set of decisions, and says what it measured', async () => {
    const h = await optedIn()

    const run = await prepared(h)

    expect(run.depth_choice).toEqual({
      chosen_by: 'signals',
      measures: { decisions: { threshold: 40, value: 0 }, words: { threshold: 4000, value: 11 } },
      signals: [],
    })
  })

  it('goes deep when the decisions under review number more than the threshold', async () => {
    const h = await optedIn()
    const terms = Array.from({ length: 41 }, (_, n) => `**Term ${String(n + 1)}**:\nWhat it means.\n`).join('\n')
    await h.writeText(join(h.repo, 'CONTEXT.md'), `# Glossary\n\n${terms}`)

    const run = await prepared(h)

    expect(run.depth).toBe('deep')
    expect(run.depth_choice.signals).toEqual([{ kind: 'threshold', measure: 'decisions', threshold: 40, value: 41 }])
  })

  it('counts glossary terms and top-level list items of a decision summary as decisions', async () => {
    const h = await optedIn()

    const run = await prepared(h, { decision_summary: '- One.\n  - A detail, not a decision.\n- Two.\n1. Three.\n**Window**: seven seconds.\n' })

    expect(run.depth_choice.measures.decisions).toEqual({ threshold: 40, value: 4 })
  })

  it('goes deep on a risk term in the decisions, but not on one in the feature statement alone', async () => {
    const h = await optedIn()

    const quiet = await prepared(h, { feature: 'Widgets get an authentication window.' })
    const risky = await prepared(h, { decision_summary: '- Widgets need authentication.\n' })

    expect(quiet.depth).toBe('base')
    expect(risky.depth_choice.signals).toEqual([{ kind: 'risk-term', term: 'authentication' }])
  })
})

const ROLE = 'roles/gap-critic.md'
const SCENARIO = 'The nightly job fails at 02:00 and a widget is created at 02:05: nothing says who frobnicates it before the next night.'

/** A gap a critic of the decisions stage reports, quoting the frozen glossary. */
function gap(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return finding({
    category: 'unasked-question',
    claim: 'When the nightly job is down, who frobnicates a new widget?',
    evidence: [
      { file: 'artifact/CONTEXT.md', kind: 'quote', value: DECISION_CANARY },
      { kind: 'input', value: SCENARIO },
    ],
    location: 'CONTEXT.md, the nightly job',
    suggestion: 'Either the next nightly run, or an immediate retry.',
    ...overrides,
  })
}

function critic(findings: readonly Record<string, unknown>[]): VibeScript {
  return { exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings })) }
}

async function fannedOut(findings: readonly Record<string, unknown>[], extra: Record<string, unknown> = {}) {
  const h = await optedIn({ scripts: [critic(findings)] })
  const run = await prepared(h, extra)
  const result = await h.commands.fanOut.run({ run_dir: run.run_dir })
  return { h, result, run }
}

function asset(path: string): Promise<string> {
  return readFile(join(ASSETS_ROOT, path), 'utf8')
}

async function ledger(h: Harness): Promise<Record<string, unknown>[]> {
  const text = await readFile(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl'), 'utf8')
  return text
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line) as Record<string, unknown>)
    .filter((line) => line.type !== 'invocation')
}

describe('the gap-critic role', () => {
  it('stays under about 400 words and names no other critic or model', async () => {
    const role = await asset(ROLE)

    expect(role.split(/\s+/).filter((word) => word.length > 0).length).toBeLessThan(400)
    expect(role).not.toMatch(/\b(critics?|models?|reviewers?|mistral|gemini|glm|deepseek|claude|gpt|codex)\b/i)
  })

  it('is what the critic is told, with its lens and the feature statement in the file list', async () => {
    const { h } = await fannedOut([gap()])

    const args = h.process.commands[0]?.args ?? []
    const prompt = await readFile(args[args.indexOf('--prompt-file') + 1] ?? '', 'utf8')
    expect(prompt).toContain((await asset(ROLE)).trim())
    expect(prompt).toContain((await asset('lenses/unasked-questions.md')).trim())
    expect(prompt).toContain('- `feature/feature.md`: the feature the decisions are for\n- `artifact/CONTEXT.md`: the artifact under review')
  })

  it('reaches the ledger as the role hash of every line the run records', async () => {
    const { h, run } = await fannedOut([gap()])
    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ answered: true, finding_id: 'F1', verdict: 'accepted' }] })

    const hash = createHash('sha256').update(await asset(ROLE)).digest('hex')
    const lines = await ledger(h)
    expect(lines.map((line) => [line.type, line.role, (line.prompt_hashes as { role: string }).role])).toEqual([
      ['run', 'gap-critic', hash],
      ['finding', 'gap-critic', hash],
    ])
  })
})

describe('the decisions evidence rule', () => {
  it('leaves a gap with a decision quote and a scenario for the orchestrator to check', async () => {
    const { result } = await fannedOut([gap()])

    expect(result.findings[0]?.pre_verdict).toBeUndefined()
  })

  it('makes a gap without a concrete scenario a hint', async () => {
    const { result } = await fannedOut([gap({ evidence: [{ file: 'artifact/CONTEXT.md', kind: 'quote', value: DECISION_CANARY }] })])

    expect(result.findings[0]?.pre_verdict).toEqual({ reason: 'no concrete scenario', reason_code: 'no-scenario', verdict: 'hint' })
  })

  it('makes a gap that quotes only the feature statement a hint', async () => {
    const { result } = await fannedOut([
      gap({ evidence: [{ file: 'feature/feature.md', kind: 'quote', value: FEATURE }, { kind: 'input', value: SCENARIO }] }),
    ])

    expect(result.findings[0]?.pre_verdict).toMatchObject({ reason_code: 'no-decision-quote', verdict: 'hint' })
  })

  it('counts a quote from the decision summary as a decision quote', async () => {
    const { result } = await fannedOut(
      [gap({ evidence: [{ kind: 'quote', value: 'The window is seven seconds.' }, { kind: 'input', value: SCENARIO }] })],
      { decision_summary: '- The window is seven seconds.\n' },
    )

    expect(result.findings[0]?.pre_verdict).toBeUndefined()
  })

  it('makes a finding of any category but unasked-question or contradiction a hint', async () => {
    const { result } = await fannedOut([gap({ category: 'omission' }), gap({ category: 'contradiction', id: 'F2', location: 'docs/adr' })])

    expect(result.findings.map((f) => f.pre_verdict?.reason_code)).toEqual(['not-a-decision-gap', undefined])
  })
})

describe('recording the owner’s answers', () => {
  it('marks an accepted gap the owner answered, and one the owner skipped', async () => {
    const { h, run } = await fannedOut([gap(), gap({ id: 'F2', location: 'docs/adr', severity: 'critical' })])

    const recorded = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [
        { answered: true, finding_id: 'F1', verdict: 'accepted' },
        { finding_id: 'F2', reason: 'The owner left it open for now.', reason_code: 'skipped-by-owner', verdict: 'accepted' },
      ],
    })

    expect(recorded.summary).toContain(
      '- F1 [major unasked-question] CONTEXT.md, the nightly job: When the nightly job is down, who frobnicates a new widget? (answered)',
    )
    expect(recorded.summary).toContain(
      '- F2 [critical unasked-question] docs/adr: When the nightly job is down, who frobnicates a new widget? (skipped-by-owner)',
    )
    expect(recorded.summary).not.toContain('Do not start')
    const findings = (await ledger(h)).filter((line) => line.type === 'finding')
    expect(findings.map((line) => [line.finding_id, line.verdict, line.reason_code])).toEqual([
      ['F1', 'accepted', null],
      ['F2', 'accepted', 'skipped-by-owner'],
    ])
  })

  it('refuses a needs-human verdict, since every accepted gap is already a question', async () => {
    const { h, run } = await fannedOut([gap()])

    await expect(
      h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', reason: 'Who frobnicates?', verdict: 'needs-human' }] }),
    ).rejects.toThrow(/F1: the decisions stage asks every accepted gap as a question; give it accepted, not needs-human/)
  })

  it('takes an answer only on an accepted gap', async () => {
    const { h, run } = await fannedOut([gap()])

    await expect(
      h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ answered: true, finding_id: 'F1', reason: 'Settled by ADR 1.', verdict: 'rejected' }] }),
    ).rejects.toThrow(/F1: only an accepted verdict can be answered at the decisions stage/)
  })

  it('never calls for a second round, since accepted gaps change nothing', async () => {
    const gaps = Array.from({ length: 4 }, (_, n) => gap({ id: `F${String(n + 1)}`, location: `Term ${String(n + 1)}`, severity: 'critical' }))
    const { h, run } = await fannedOut(gaps)

    const recorded = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: gaps.map((_, n) => ({ answered: true, finding_id: `F${String(n + 1)}`, verdict: 'accepted' })),
    })

    expect(recorded.second_round_due).toBe(false)
  })
})

const ADR_FILE = 'artifact/docs/adr/0001-nightly-job.md'
const NO_GAPS = { findings: [], no_findings_reason: 'Every branch the feature meets is decided.' }

/** A finding the adr-record critic reports on the frozen ADR. */
function recordFinding(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return finding({
    category: 'omission',
    check: 'no-cost',
    claim: 'The record names no cost of frobnicating only at night.',
    evidence: [{ file: ADR_FILE, kind: 'quote', value: ADR_DECISION }],
    location: 'docs/adr/0001-nightly-job.md: no-cost',
    severity: 'minor',
    suggestion: 'Ask the owner what frobnicating only at night costs.',
    ...overrides,
  })
}

/**
 * A decisions run over the glossary and one ADR, with Mistral Large as the
 * gap critic and DeepSeek Flash as the adr-record critic (`c2`), since the
 * allowlist permits only their two families.
 */
async function recordsReviewed(records: readonly Record<string, unknown>[], gaps: unknown = NO_GAPS) {
  const h = await withAdr(
    await allowing(['mistral', 'deepseek'], {
      scripts: [
        { agent: 'critic-mistral-large', exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced(gaps)) },
        { agent: 'critic-deepseek-flash', ...critic(records) },
      ],
    }),
  )
  const run = await prepared(h)
  const result = await h.commands.fanOut.run({ run_dir: run.run_dir })
  return { h, result, run }
}

describe('the adr-record critic at the decisions stage', () => {
  it('is told the adr-reviewer role and the adr-record lens', async () => {
    const { h } = await recordsReviewed([recordFinding()])

    const launch = h.process.commands.find((command) => command.args.includes('critic-deepseek-flash'))
    const args = launch?.args ?? []
    const prompt = await readFile(args[args.indexOf('--prompt-file') + 1] ?? '', 'utf8')
    expect(prompt).toContain((await asset('roles/adr-reviewer.md')).trim())
    expect(prompt).toContain((await asset('lenses/adr-record.md')).trim())
    expect(prompt).toContain(`- \`${ADR_FILE}\`: the artifact under review`)
  })

  it('is told by its role to keep to the records the feature bears on, in a packet with no ADR check report', async () => {
    const role = (await asset('roles/adr-reviewer.md')).replace(/\s+/g, ' ')

    expect(role).toContain('When the packet holds `feature/feature.md`')
    expect(role).toContain('When the packet holds `check/adrs.json`')
  })

  it('leaves a finding that quotes its record for the orchestrator, at the severity its check fixes', async () => {
    const { result } = await recordsReviewed([recordFinding()])

    expect(result.findings).toHaveLength(1)
    expect(result.findings[0]).toMatchObject({ declared_severity: 'major', raised_by: ['c2-vibe-deepseek-flash'], severity: 'major' })
    expect(result.findings[0]?.pre_verdict).toBeUndefined()
  })

  it('makes a finding that quotes none of the decisions under review a hint', async () => {
    const { result } = await recordsReviewed([recordFinding({ evidence: [{ file: 'feature/feature.md', kind: 'quote', value: FEATURE }] })])

    expect(result.findings[0]?.pre_verdict).toEqual({
      reason: 'no quote from a record under review',
      reason_code: 'no-record-quote',
      verdict: 'hint',
    })
  })

  it('still holds the gap critic’s findings to the gap rule in the same run', async () => {
    const { result } = await recordsReviewed([recordFinding()], { findings: [gap({ category: 'omission', id: 'G1' })] })

    expect(result.findings.map((f) => [f.raised_by, f.pre_verdict?.reason_code])).toEqual([
      [['c1-vibe-mistral-large'], 'not-a-decision-gap'],
      [['c2-vibe-deepseek-flash'], undefined],
    ])
  })

  it('is ruled on with the stage’s verdicts: an accepted question the owner answered, and a rejection', async () => {
    const { h, run } = await recordsReviewed([recordFinding(), recordFinding({ check: 'undated-claim', id: 'F2', location: 'docs/adr/0001-nightly-job.md: undated-claim' })])

    const recorded = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [
        { answered: true, finding_id: 'F1', verdict: 'accepted' },
        { finding_id: 'F2', reason: 'The record bears on no part of the feature.', reason_code: 'outside-feature', verdict: 'rejected' },
      ],
    })

    expect(recorded.summary).toContain(
      '- F1 [major omission] docs/adr/0001-nightly-job.md: no-cost: The record names no cost of frobnicating only at night. (answered)',
    )
    const findings = (await ledger(h)).filter((line) => line.type === 'finding')
    expect(findings.map((line) => [line.finding_id, line.role, line.check, line.verdict, line.reason_code])).toEqual([
      ['F1', 'adr-reviewer', 'no-cost', 'accepted', null],
      ['F2', 'adr-reviewer', 'undated-claim', 'rejected', 'outside-feature'],
    ])
  })

  it('takes a severity override with its reason, as the ledger records it', async () => {
    const { h, run } = await recordsReviewed([recordFinding()])

    await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [{ answered: true, finding_id: 'F1', severity: 'minor', severity_reason: 'cost-in-context', verdict: 'accepted' }],
    })

    const [line] = (await ledger(h)).filter((entry) => entry.type === 'finding')
    expect(line).toMatchObject({ declared_severity: 'major', severity_override: 'minor', severity_reason: 'cost-in-context' })
  })

  it('has an accepted critical finding refuted by a quote of its record', async () => {
    const { h, run } = await recordsReviewed([
      recordFinding({ check: 'no-decision-sentence', claim: 'No sentence says what was decided.', location: 'docs/adr/0001-nightly-job.md: no-decision-sentence' }),
    ])
    h.process.queue({
      agent: 'critic-mistral-large',
      ...critic([
        finding({
          check: 'refute',
          claim: 'The record’s second line states the decision.',
          evidence: [{ file: ADR_FILE, kind: 'quote', value: ADR_DECISION }],
        }),
      ]),
    })

    const result = await h.commands.refute.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })

    expect(result.results[0]).toMatchObject({ answer: 'refute', finding_id: 'F1', outcome: 'refute' })
  })
})

describe('every other stage', () => {
  it('still needs a spec', async () => {
    const h = await optedIn()

    await expect(h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, stage: 'spec' })).rejects.toThrow(
      /spec needs spec: the spec, named the way its tracker names it/,
    )
  })
})

describe('an answer at the spec stage', () => {
  it('still belongs only to a needs-human verdict', async () => {
    const h = await optedIn({ scripts: [critic([finding()])] })
    const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })
    if (run.status !== 'ready') throw new Error('not ready')
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    await expect(
      h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ answered: true, finding_id: 'F1', verdict: 'accepted' }] }),
    ).rejects.toThrow(/F1: only a needs-human verdict can be answered/)
  })
})
