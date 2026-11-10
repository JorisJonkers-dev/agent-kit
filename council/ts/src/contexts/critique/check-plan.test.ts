import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { harness, type Harness } from '../../../test/critique/harness.js'
import type { CheckPlanResult, Refusal } from './index.js'
import { goalChangesBehaviour, pathsIn } from './domain/refactoring-plan.js'

/**
 * The deterministic refactoring-plan checker (#485): the plan format's
 * rules, each broken by one fixture plan and nothing else.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

type Fields = Readonly<Record<string, string | undefined>>

const DEFAULTS: Record<string, string> = {
  Anchor: '`src/importer/run.ts#run`',
  'Blocked by': 'none',
  'Done when': 'The suite passes',
  Kind: 'tidy',
  'Main stays green because': 'The suite passes after this step alone',
  Rollback: 'Revert the commit',
  'Safety net': 'Unit tests in the suite',
  Technique: 'Extract Function',
  Touches: '`src/importer/run.ts`',
  'Transitional element': 'none',
}

interface Step {
  readonly id: string
  readonly fields?: Fields
}

const SECTIONS: Readonly<Record<string, string>> = {
  Goal: 'Unblock the importer split. Behaviour is unchanged. Evidence: `src/importer/run.ts`.',
  Target: 'The importer is a module with one interface.',
  'Safety net': 'The importer suite.',
  'Not doing': 'Renaming the importer.',
  'Open decisions': 'None.',
}

function stepText({ id, fields = {} }: Step): string {
  const merged = { ...DEFAULTS, ...fields }
  const lines = Object.entries(merged).flatMap(([name, value]) => (value === undefined ? [] : [`- **${name}**: ${value}`]))
  return `### ${id}: Step ${id}\n\n${lines.join('\n')}\n`
}

function plan(options: { steps?: readonly Step[]; omit?: readonly string[]; sections?: Fields; raw?: string } = {}): string {
  const sections = { ...SECTIONS, ...options.sections }
  const out = Object.entries(sections).flatMap(([name, body]) =>
    options.omit?.includes(name) === true || body === undefined ? [] : [`## ${name}\n\n${body}\n`],
  )
  const steps = options.omit?.includes('Steps') === true ? '' : `## Steps\n\n${options.raw ?? (options.steps ?? [{ id: 'S01' }]).map(stepText).join('\n')}`
  return `# Refactoring plan\n\n${[out[0], steps, ...out.slice(1)].join('\n')}`
}

/** A plan that follows the format: every kind, an expand with its migrate, contract and removal, and untested structure behind a characterize step. */
const VALID: readonly Step[] = [
  { fields: { Kind: 'characterize', 'Safety net': 'none' }, id: 'S01' },
  { fields: { 'Blocked by': 'S01', Kind: 'structure', 'Safety net': 'none, covered by S01' }, id: 'S02' },
  { fields: { 'Blocked by': 'S02', Kind: 'expand', 'Transitional element': 'An adapter shim, removed by S06' }, id: 'S03' },
  { fields: { 'Blocked by': 'S03', Kind: 'migrate' }, id: 'S04' },
  { fields: { 'Blocked by': 'S04', Kind: 'contract' }, id: 'S05' },
  { fields: { 'Blocked by': 'S05', Kind: 'remove-transitional' }, id: 'S06' },
]

async function withPlan(text: string, extra: Readonly<Record<string, string>> = {}, options: Parameters<typeof harness>[0] = {}): Promise<Harness> {
  const h = await harness(options)
  open.push(h)
  await h.writeText(join(h.repo, 'refactoring-plan.md'), text)
  for (const [path, body] of Object.entries(extra)) await h.writeText(join(h.repo, path), body)
  return h
}

async function check(h: Harness, input: object = { repo: h.repo, plan: 'refactoring-plan.md' }): Promise<CheckPlanResult> {
  const result: CheckPlanResult | Refusal = await h.commands.checkPlan.run(input)
  if (result.status === 'refused') throw new Error(`check-plan refused: ${result.message}`)
  return result
}

/** The rules a plan breaks, as `rule` or `S01:rule`. */
async function broken(text: string, extra: Readonly<Record<string, string>> = {}, options: Parameters<typeof harness>[0] = {}): Promise<readonly string[]> {
  const result = await check(await withPlan(text, extra, options))
  return result.errors.map((error) => (error.step === undefined ? error.rule : `${error.step}:${error.rule}`))
}

describe('check-plan on a plan that follows the format', () => {
  it('passes with no errors and lists the steps', async () => {
    const result = await check(await withPlan(plan({ steps: VALID })))

    expect(result.errors).toEqual([])
    expect(result.status).toBe('passed')
    expect(result.steps.map((step) => [step.id, step.kind, step.blocked_by])).toEqual([
      ['S01', 'characterize', []],
      ['S02', 'structure', ['S01']],
      ['S03', 'expand', ['S02']],
      ['S04', 'migrate', ['S03']],
      ['S05', 'contract', ['S04']],
      ['S06', 'remove-transitional', ['S05']],
    ])
    expect(result.stored_data_steps).toEqual([])
  })

  it('reads the bold-colon spelling, sub-bullet values, and ignores fenced text', async () => {
    const text = plan({
      raw: [
        '### S01: Split',
        '',
        '- **Kind:** tidy',
        '- **Technique**: Extract Function',
        '- **Anchor**: `src/a.ts`',
        '- **Touches**:',
        '  - `src/a.ts`',
        '  - `src/b.ts`',
        '- **Safety net**: tests',
        '- **Main stays green because**: nothing',
        '  else changes',
        '- **Rollback**: revert',
        '- **Transitional element**: none',
        '- **Blocked by**: none',
        '- **Done when**: green',
        '- **Owner**: someone',
        '',
        '```',
        '## Not a section',
        '```',
        '',
      ].join('\n'),
    })

    expect(await broken(text)).toEqual([])
  })

  it('reads a heading in another case, and flags the same section given twice', async () => {
    expect(await broken(plan({ sections: { goal: undefined } }).replace('## Goal', '## goal'))).toEqual([])
    expect(await broken(`${plan()}\n## goal\n\nBehaviour changes.\n`)).toEqual(['duplicate-section'])
  })
})

describe('check-plan rules, each broken by one fixture', () => {
  it('missing-section', async () => {
    expect(await broken(plan({ omit: ['Not doing'] }))).toEqual(['missing-section'])
  })

  it('no-steps and bad-step-heading and duplicate-step', async () => {
    expect(await broken(plan({ raw: '' }))).toEqual(['no-steps'])
    expect(await broken(plan({ raw: `### Step one\n\n${stepText({ id: 'S01' })}` }))).toEqual(['bad-step-heading'])
    expect(await broken(plan({ steps: [{ id: 'S01' }, { id: 'S01' }] }))).toEqual(['S01:duplicate-step'])
  })

  it('a malformed step heading ends the step before it', async () => {
    const raw = `${stepText({ id: 'S01' })}\n### Step two\n- **Kind**: tidy\n`
    expect(await broken(plan({ raw }))).toEqual(['bad-step-heading'])
  })

  it('duplicate-section and unknown-section keep their content out of the checks', async () => {
    expect(await broken(`${plan()}\n## Steps\n\n${stepText({ id: 'S02' })}`)).toEqual(['duplicate-section'])
    expect(await broken(`${plan()}\n## Appendix\n\nSee src/app/main.ts for more.\n`)).toEqual(['unknown-section'])
  })

  it('missing-field', async () => {
    expect(await broken(plan({ steps: [{ fields: { Rollback: undefined }, id: 'S01' }] }))).toEqual(['S01:missing-field'])
    expect(await broken(plan({ steps: [{ fields: { Rollback: '' }, id: 'S01' }] }))).toEqual(['S01:missing-field'])
    expect(await broken(plan({ steps: [{ fields: { 'Done when': undefined, Touches: undefined }, id: 'S01' }] }))).toEqual(['S01:missing-field', 'S01:missing-field'])
  })

  it('multiple-kinds and unknown-kind', async () => {
    expect(await broken(plan({ steps: [{ fields: { Kind: 'tidy, structure' }, id: 'S01' }] }))).toEqual(['S01:multiple-kinds'])
    expect(await broken(plan({ steps: [{ fields: { Kind: 'polish' }, id: 'S01' }] }))).toEqual(['S01:unknown-kind'])
  })

  it('step-cycle', async () => {
    const steps = [
      { fields: { 'Blocked by': 'S02' }, id: 'S01' },
      { fields: { 'Blocked by': 'S01' }, id: 'S02' },
      { fields: { 'Blocked by': 'S03' }, id: 'S03' },
      { fields: { 'Blocked by': 'S01' }, id: 'S04' },
    ]
    const result = await check(await withPlan(plan({ steps })))

    expect(result.status).toBe('failed')
    expect(result.errors.map((error) => [error.rule, error.message])).toEqual([
      ['step-cycle', 'steps S01, S02 block each other in a cycle'],
      ['step-cycle', 'step S03 blocks itself'],
    ])
  })

  it('blocked-by-unknown', async () => {
    expect(await broken(plan({ steps: [{ fields: { 'Blocked by': 'S09' }, id: 'S01' }] }))).toEqual(['S01:blocked-by-unknown'])
    expect(await broken(plan({ steps: [{ fields: { 'Blocked by': 'the parser work' }, id: 'S01' }] }))).toEqual(['S01:blocked-by-unknown'])
  })

  it('expand-incomplete', async () => {
    const steps = [{ fields: { Kind: 'expand' }, id: 'S01' }, { fields: { Kind: 'migrate' }, id: 'S02' }]

    expect(await broken(plan({ steps }))).toEqual(['S01:expand-incomplete'])
  })

  it('transitional-no-removal', async () => {
    const steps = [{ fields: { 'Transitional element': 'A feature toggle' }, id: 'S01' }]

    expect(await broken(plan({ steps }))).toEqual(['S01:transitional-no-removal'])
    expect(await broken(plan({ steps: [{ fields: { 'Transitional element': 'A toggle, removed by S01' }, id: 'S01' }] }))).toEqual([
      'S01:transitional-no-removal',
    ])
  })

  it('anchor-unresolved and anchor-unknown-step', async () => {
    expect(await broken(plan({ steps: [{ fields: { Anchor: 'it feels messy' }, id: 'S01' }] }))).toEqual(['S01:anchor-unresolved'])
    expect(await broken(plan({ steps: [{ fields: { Anchor: 'prerequisite-of:S09' }, id: 'S01' }] }))).toEqual(['S01:anchor-unknown-step'])
    expect(await broken(plan({ steps: [{ fields: { Anchor: 'prerequisite-of:S02' }, id: 'S01' }, { id: 'S02' }] }))).toEqual([])
  })

  it('behaviour-not-in-goal', async () => {
    const steps = [{ fields: { Kind: 'behaviour' }, id: 'S01' }]

    expect(await broken(plan({ steps }))).toEqual(['S01:behaviour-not-in-goal'])
    expect(await broken(plan({ sections: { Goal: 'Behaviour changes: imports reject empty rows.' }, steps }))).toEqual([])
  })

  it('untested-structure', async () => {
    const untested = { fields: { Kind: 'structure', 'Safety net': 'None' }, id: 'S01' }

    expect(await broken(plan({ steps: [untested] }))).toEqual(['S01:untested-structure'])
    expect(await broken(plan({ steps: [{ fields: { 'Blocked by': 'S01', Kind: 'tidy' }, id: 'S02' }, { ...untested, id: 'S01' }] }))).toEqual([
      'S01:untested-structure',
    ])
    const covered = [
      { fields: { Kind: 'characterize' }, id: 'S01' },
      { fields: { 'Blocked by': 'S01' }, id: 'S02' },
      { fields: { 'Blocked by': 'S02', Kind: 'structure', 'Safety net': 'untested' }, id: 'S03' },
    ]
    expect(await broken(plan({ steps: covered }))).toEqual([])
    const diamond = [
      { id: 'S01' },
      { fields: { 'Blocked by': 'S01' }, id: 'S02' },
      { fields: { 'Blocked by': 'S01' }, id: 'S03' },
      { fields: { 'Blocked by': 'S02, S03', Kind: 'structure', 'Safety net': 'none' }, id: 'S04' },
    ]
    expect(await broken(plan({ steps: diamond }))).toEqual(['S04:untested-structure'])
  })

  it('path-outside-anchor', async () => {
    expect(await broken(plan({ steps: [{ fields: { Rollback: 'Revert src/importer/run.ts' }, id: 'S01' }] }))).toEqual(['S01:path-outside-anchor'])
    expect(await broken(plan({ steps: [{ fields: { Technique: 'Extract run.ts' }, id: 'S01' }] }))).toEqual(['S01:path-outside-anchor'])
    expect(await broken(plan({ sections: { Target: 'The module lives in src/importer/run.ts.' } }))).toEqual(['path-outside-anchor'])
    expect(await broken(plan({ steps: [{ fields: { Rollback: 'Revert, see https://example.com/a/b.html or and/or' }, id: 'S01' }] }))).toEqual([])
  })

  it('too-many-touches and too-many-criteria, from routing ticket_size', async () => {
    const touches = Array.from({ length: 6 }, (_, i) => `\`src/m${String(i)}.ts\``).join(', ')
    const done = Array.from({ length: 9 }, (_, i) => `criterion ${String(i)}`).join('; ')
    const steps = [{ fields: { 'Done when': done, Touches: touches }, id: 'S01' }]

    expect(await broken(plan({ steps }))).toEqual(['S01:too-many-touches', 'S01:too-many-criteria'])

    const small = { routingOverride: { stages: { tickets: { ticket_size: { max_criteria: 1, max_modules: 1 } } } } }
    const two = [{ fields: { 'Done when': 'a; b', Touches: '`src/a.ts`, `src/b.ts`' }, id: 'S01' }]
    expect(await broken(plan({ steps: two }), {}, small)).toEqual(['S01:too-many-touches', 'S01:too-many-criteria'])
    expect(await broken(plan({ steps: two }))).toEqual([])
  })

  it('counts sub-bullets as touches and criteria', async () => {
    const raw = [
      '### S01: Big',
      ...Object.entries(DEFAULTS).flatMap(([name, value]) =>
        name === 'Touches' || name === 'Done when'
          ? [`- **${name}**:`, ...Array.from({ length: 9 }, (_, i) => `  - item ${String(i)}`)]
          : [`- **${name}**: ${value}`],
      ),
      '',
    ].join('\n')

    expect(await broken(plan({ raw }))).toEqual(['S01:too-many-touches', 'S01:too-many-criteria'])
  })
})

describe('check-plan reading of fields and prose', () => {
  it('reads rule-bearing values from sub-bullets', async () => {
    const raw = (fields: string): string => `### S01: A\n\n${fields}\n- **Technique**: Extract Function\n- **Touches**: \`src/a.ts\`\n- **Safety net**: tests\n- **Main stays green because**: x\n- **Rollback**: revert\n- **Done when**: green\n`
    const bulleted = ['- **Kind**:', '  - tidy', '- **Anchor**:', '  - `src/a.ts`', '- **Transitional element**:', '  - none', '- **Blocked by**:', '  - none'].join('\n')
    expect(await broken(plan({ raw: raw(bulleted) }))).toEqual([])
    const two = ['- **Kind**:', '  - tidy', '  - structure', '- **Anchor**:', '  - `src/a.ts`', '- **Transitional element**: none', '- **Blocked by**: none'].join('\n')
    expect(await broken(plan({ raw: raw(two) }))).toEqual(['S01:multiple-kinds'])
  })

  it('keeps a field across a blank line before its sub-bullets', async () => {
    const raw = ['### S01: A', '', ...Object.entries(DEFAULTS).filter(([name]) => name !== 'Touches').map(([name, value]) => `- **${name}**: ${value}`), '- **Touches**:', '', '  - `src/a.ts`', ''].join('\n')

    expect(await broken(plan({ raw }))).toEqual([])
  })

  it('flags a path in free prose under a step', async () => {
    const raw = `${stepText({ id: 'S01' })}\nSee src/importer/run.ts for details.\n`

    expect(await broken(plan({ raw }))).toEqual(['S01:path-outside-anchor'])
  })

  it('does not take product names for files', () => {
    expect(pathsIn('Node.js, Vue.js and Next.js')).toEqual([])
    expect(pathsIn('run.py and node.js')).toEqual(['run.py', 'node.js'])
  })

  it('flags a field given twice', async () => {
    const raw = `${stepText({ id: 'S01' })}- **Rollback**: Revert again\n`

    expect(await broken(plan({ raw }))).toEqual(['S01:duplicate-field'])
  })

  it('reads a Goal that denies behaviour change only at the behaviour clause', () => {
    expect(goalChangesBehaviour('The behaviour changes for the same users.')).toBe(true)
    expect(goalChangesBehaviour('Makes the retry behaviour change, not the cache.')).toBe(true)
    expect(goalChangesBehaviour('The importer does not change behaviour.')).toBe(false)
    expect(goalChangesBehaviour('Without any behaviour change.')).toBe(false)
  })

  it('reads untested safety nets in several wordings', async () => {
    for (const net of ['Not tested', 'No existing tests', 'Nothing covers this module', 'The code is untested']) {
      expect(await broken(plan({ steps: [{ fields: { Kind: 'structure', 'Safety net': net }, id: 'S01' }] }))).toEqual(['S01:untested-structure'])
    }
  })
})

describe('check-plan stored data', () => {
  it('lists the migrate and contract steps that touch stored data', async () => {
    const steps = [
      { fields: { Kind: 'migrate', Touches: 'The orders table' }, id: 'S01' },
      { fields: { Kind: 'contract', Technique: 'Drop the legacy column' }, id: 'S02' },
      { fields: { Kind: 'migrate', Touches: '`src/a.ts`' }, id: 'S03' },
      { fields: { Kind: 'tidy', Touches: 'The orders table' }, id: 'S04' },
      { fields: { Kind: 'migrate', Touches: '' }, id: 'S05' },
    ]
    const result = await check(await withPlan(plan({ steps })))

    expect(result.stored_data_steps).toEqual(['S01', 'S02'])
  })
})

describe('check-plan ADR anchors', () => {
  const log = {
    'docs/adr/0001-first.md': '# First\n\nWe decided.\n',
    'docs/adr/0002-second.md': '# Second\n\nStatus: superseded\n\nWe decided.\n',
    'docs/adr/0003-third.md': '# Third\n\nSupersedes ADR-0001.\n',
    'docs/adr/0004-fourth.md': '# Fourth\n\nStatus: Accepted\n\nWe decided.\n',
    'docs/adr/0005-fifth.md': '# Fifth\n\nStatus: Accepted\n\nSupersedes ADR-0004.\n',
  }
  const anchored = (anchor: string): string => plan({ steps: [{ fields: { Anchor: anchor }, id: 'S01' }] })

  it('flags an ADR id absent from docs/adr and one that is superseded', async () => {
    expect(await broken(anchored('ADR-0009'), log)).toEqual(['S01:adr-absent'])
    expect(await broken(anchored('ADR-0002'), log)).toEqual(['S01:adr-superseded'])
    expect(await broken(anchored('ADR-0001'), log)).toEqual(['S01:adr-superseded'])
    expect(await broken(anchored('ADR-0003'), log)).toEqual([])
    expect(await broken(anchored('ADR-0004'), log)).toEqual(['S01:adr-superseded'])
    expect(await broken(anchored('ADR-0005'), log)).toEqual([])
  })

  it('skips the ADR checks with a note when no repo is given', async () => {
    const h = await withPlan(anchored('ADR-0009'))
    const result = await check(h, { plan: join(h.repo, 'refactoring-plan.md') })

    expect(result.errors).toEqual([])
    expect(result.notes).toEqual(['ADR ids were not checked against docs/adr: no repo was given'])
  })
})

describe('check-plan command', () => {
  it('fails a malformed plan and passes a valid one, with the report fields', async () => {
    const bad = await check(await withPlan(plan({ omit: ['Goal'] })))
    expect(bad).toMatchObject({ errors: [{ rule: 'missing-section' }], status: 'failed', stored_data_steps: [] })

    expect(await check(await withPlan(plan()))).toMatchObject({ errors: [], notes: [], status: 'passed' })
  })

  it('is refused for a repository whose remotes refuse a critique', async () => {
    const h = await harness({ remotes: [] })
    open.push(h)

    expect(await h.commands.checkPlan.run({ plan: 'refactoring-plan.md', repo: h.repo })).toMatchObject({ reason: 'no-remote', status: 'refused' })
  })

  it('resolves an absolute plan path as it is', async () => {
    const h = await withPlan(plan())

    expect(await h.commands.checkPlan.run({ plan: join(h.repo, 'refactoring-plan.md'), repo: h.repo })).toMatchObject({ status: 'passed' })
  })

  it('is an error for a plan that does not exist', async () => {
    const h = await withPlan(plan())

    await expect(h.commands.checkPlan.run({ plan: 'missing.md', repo: h.repo })).rejects.toThrow('does not exist')
  })
})

describe('plan text helpers', () => {
  it('reads whether a Goal says behaviour changes', () => {
    expect(goalChangesBehaviour('Behaviour changes for empty rows.')).toBe(true)
    expect(goalChangesBehaviour('We change the behaviour of imports.')).toBe(true)
    expect(goalChangesBehaviour('No behaviour change.')).toBe(false)
    expect(goalChangesBehaviour('Behaviour is preserved.')).toBe(false)
    expect(goalChangesBehaviour('Split the module.')).toBe(false)
  })

  it('finds paths but not words or URLs', () => {
    expect(pathsIn('see src/a/b.ts and run.py, https://x.io/a/b.md, and/or a/b/c')).toEqual(['src/a/b.ts', 'run.py', 'a/b/c'])
  })
})
