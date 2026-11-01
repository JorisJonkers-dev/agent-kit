import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  agyFindings,
  fenced,
  finding,
  harness,
  isolatedExport,
  specIssue,
  twoCriticRouting,
  vibeStdout,
  type Harness,
} from '../../../test/critique/harness.js'
import type { PrepareResult } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const EVERY_FAMILY = ['mistral', 'google', 'zhipu', 'deepseek']
const AGENT_KIT = 'github.com/jorisjonkers-dev/agent-kit'
const SPEC = '.scratch/widgets/spec.md'

async function optedIn(families: readonly string[] = EVERY_FAMILY, ...args: Parameters<typeof harness>): Promise<Harness> {
  const h = await harness(...args)
  open.push(h)
  await h.commands.allow.run({ families, repo: h.repo })
  return h
}

async function ready(h: Harness, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const result = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: SPEC, stage: 'spec', ...extra })
  if (result.status !== 'ready') throw new Error(`prepare refused: ${result.message}`)
  return result
}

/** A spec with `count` numbered user stories under its User Stories heading, then a numbered list after it. */
function specWithStories(count: number): string {
  const stories = Array.from({ length: count }, (_, i) => `${String(i + 1)}. As the owner, I want widget ${String(i + 1)}.`)
  return `# Widgets\n\n## User Stories\n\n${stories.join('\n')}\n\n## Out of Scope\n\n1. Gadgets.\n`
}

function clients(result: PrepareResult): readonly string[] {
  return result.critics.map((critic) => critic.critic_id)
}

/** The lines record appends: fan-out's invocation lines, which the budget reads, are left out. */
function ledger(h: Harness): Promise<Record<string, unknown>[]> {
  return readFile(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl'), 'utf8').then((text) =>
    text
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line) as Record<string, unknown>)
      .filter((line) => line.type !== 'invocation'),
  )
}

/**
 * Runs the prepared Mistral-only run through fan-out and record, with one
 * finding that has no evidence (so it needs no verdict), and returns the
 * summary and the report.
 */
async function recorded(h: Harness, run: PrepareResult): Promise<{ summary: string; report: string }> {
  h.process.queue({ exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings: [finding({ evidence: [] })] })) })
  await h.commands.fanOut.run({ run_dir: run.run_dir })
  const result = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })
  return { report: await readFile(result.report_path, 'utf8'), summary: result.summary }
}

describe('depth chosen from the spec', () => {
  it('runs the base roster when no signal fires, and says what it measured', async () => {
    const h = await optedIn()

    const result = await ready(h)

    expect(result.depth).toBe('base')
    expect(clients(result)).toEqual(['c1-vibe-mistral-large', 'c2-agy-gemini-flash-high'])
    expect(result.depth_choice).toEqual({
      chosen_by: 'signals',
      measures: { user_stories: { threshold: 30, value: 2 }, words: { threshold: 5000, value: 25 } },
      signals: [],
    })
  })

  it('adds GLM and DeepSeek Flash with their lenses when the spec has more user stories than the threshold', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, SPEC), specWithStories(31))

    const result = await ready(h)

    expect(result.depth).toBe('deep')
    expect(result.critics.map((critic) => [critic.critic_id, critic.family, critic.lenses])).toEqual([
      ['c1-vibe-mistral-large', 'mistral', ['fidelity', 'testability']],
      ['c2-agy-gemini-flash-high', 'google', ['failure-modes', 'security']],
      ['c3-vibe-glm', 'zhipu', ['integration', 'operability']],
      ['c4-vibe-deepseek-flash', 'deepseek', ['simplicity', 'fidelity']],
    ])
    expect(result.depth_choice).toEqual({
      chosen_by: 'signals',
      measures: { user_stories: { threshold: 30, value: 31 }, words: { threshold: 5000, value: 259 } },
      signals: [{ kind: 'threshold', measure: 'user_stories', threshold: 30, value: 31 }],
    })
  })

  it('stays at base with exactly the threshold of user stories', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, SPEC), specWithStories(30))

    expect((await ready(h)).depth).toBe('base')
  })

  it('counts the stories under subheadings of the User Stories section', async () => {
    const h = await optedIn()
    const grouped = specWithStories(31).replace('16. As', '### Later\n\n16. As')
    await h.writeText(join(h.repo, SPEC), grouped)

    expect((await ready(h)).depth_choice.measures.user_stories).toEqual({ threshold: 30, value: 31 })
  })

  it('goes deep when the spec has more words than the threshold', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, SPEC), `# Widgets\n\n${'word '.repeat(5000)}\n`)

    const result = await ready(h)

    expect(result.depth).toBe('deep')
    expect(result.depth_choice.signals).toEqual([{ kind: 'threshold', measure: 'words', threshold: 5000, value: 5002 }])
  })

  it('goes deep when the spec contains a configured risk term in any case', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, SPEC), '# Widgets\n\nWidgets need a schema Migration before launch.\n')

    const result = await ready(h)

    expect(result.depth).toBe('deep')
    expect(result.depth_choice.signals).toEqual([{ kind: 'risk-term', term: 'migration' }])
  })

  it('matches a risk phrase across a line break', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, SPEC), '# Widgets\n\nWidgets get a public\nAPI for partners.\n')

    expect((await ready(h)).depth_choice.signals).toEqual([{ kind: 'risk-term', term: 'public API' }])
  })

  it('ignores a risk term that is only part of a longer word', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, SPEC), '# Widgets\n\nThe author of a widget may rename it.\n')

    expect((await ready(h)).depth).toBe('base')
  })

  it('goes deep when the spec issue carries the risk:high label', async () => {
    const issue = specIssue({ labels: ['type:feature', 'risk:high'] })
    const h = await optedIn(EVERY_FAMILY, { issues: { [`${AGENT_KIT}#167`]: issue }, tracker: 'github' })

    const result = await ready(h, { spec: '#167' })

    expect(result.depth).toBe('deep')
    expect(result.depth_choice.signals).toEqual([{ kind: 'label', label: 'risk:high' }])
  })
})

describe('an explicit depth', () => {
  it('keeps the base roster when the caller names base, though the signals would go deep', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, SPEC), '# Widgets\n\nWidgets need auth.\n')

    const result = await ready(h, { depth: 'base' })

    expect(result.depth).toBe('base')
    expect(clients(result)).toEqual(['c1-vibe-mistral-large', 'c2-agy-gemini-flash-high'])
    expect(result.depth_choice).toMatchObject({ chosen_by: 'argument', signals: [{ kind: 'risk-term', term: 'auth' }] })
  })

  it('runs the deep roster when the caller names deep, though no signal fires', async () => {
    const h = await optedIn()

    const result = await ready(h, { depth: 'deep' })

    expect(result.depth).toBe('deep')
    expect(clients(result)).toEqual(['c1-vibe-mistral-large', 'c2-agy-gemini-flash-high', 'c3-vibe-glm', 'c4-vibe-deepseek-flash'])
    expect(result.depth_choice).toMatchObject({ chosen_by: 'argument', signals: [] })
  })

  it('rejects a depth it does not know', async () => {
    const h = await optedIn()

    await expect(ready(h, { depth: 'fast' })).rejects.toThrow(/depth must be one of base, deep/)
  })
})

describe('the deep roster and the family rules', () => {
  it('leaves out a deep critic whose family the allowlist does not permit', async () => {
    const h = await optedIn(['mistral', 'google', 'zhipu'])

    const result = await ready(h, { depth: 'deep' })

    expect(clients(result)).toEqual(['c1-vibe-mistral-large', 'c2-agy-gemini-flash-high', 'c3-vibe-glm'])
    expect(result.dropped).toEqual([{ client: 'vibe-deepseek-flash', deep: true, family: 'deepseek', reason: 'family-not-allowed' }])
  })

  it('says in the summary that a deep critic was left out', async () => {
    const h = await optedIn(['mistral'])
    const run = await ready(h, { depth: 'deep' })

    const { summary } = await recorded(h, run)

    expect(summary).toContain('- Not run: vibe-glm (zhipu): family-not-allowed; deep critic left out')
    expect(summary).toContain('- Not run: vibe-deepseek-flash (deepseek): family-not-allowed; deep critic left out')
  })

  it('leaves out the deep GLM critic when GLM already took a base slot', async () => {
    const h = await optedIn(['mistral', 'zhipu', 'deepseek'])

    const result = await ready(h, { depth: 'deep' })

    expect(clients(result)).toEqual(['c1-vibe-mistral-large', 'c2-vibe-glm', 'c3-vibe-deepseek-flash'])
    expect(result.dropped).toContainEqual({ client: 'vibe-glm', deep: true, family: 'zhipu', reason: 'same-family' })
  })

  it('keeps a stage with no deep roster at its base critics', async () => {
    const h = await optedIn(EVERY_FAMILY, { routing: twoCriticRouting() })

    const result = await ready(h, { depth: 'deep' })

    expect(clients(result)).toEqual(['c1-vibe-mistral-large', 'c2-vibe-glm'])
  })
})

describe('the chosen depth in the summary and the ledger', () => {
  it('shows the depth and the signals that chose it', async () => {
    const h = await optedIn(['mistral'])
    await h.writeText(join(h.repo, SPEC), '# Widgets\n\nWidgets need auth.\n')
    const run = await ready(h)

    const { report, summary } = await recorded(h, run)

    expect(summary).toContain('Depth: deep, chosen by signals: risk term "auth"\n')
    expect(report).toContain('Depth: deep, chosen by signals: risk term "auth"\n')
  })

  it('shows what was measured when no signal fired', async () => {
    const h = await optedIn(['mistral'])
    const run = await ready(h)

    const { summary } = await recorded(h, run)

    expect(summary).toContain('Depth: base, chosen by signals: none fired: user_stories 2 of 30, words 25 of 5000\n')
  })

  it('shows what the signals alone would have chosen when the caller named the depth', async () => {
    const h = await optedIn(['mistral'])
    await h.writeText(join(h.repo, SPEC), specWithStories(31))
    const run = await ready(h, { depth: 'base' })

    const { summary } = await recorded(h, run)

    expect(summary).toContain('Depth: base, chosen by argument; the signals alone would choose deep (user_stories 31 > 30)\n')
  })

  it('names a fired label in the summary', async () => {
    const issue = specIssue({ labels: ['risk:high'] })
    const h = await optedIn(['mistral'], { issues: { [`${AGENT_KIT}#167`]: issue }, tracker: 'github' })
    const run = await ready(h, { spec: '#167' })

    const { summary } = await recorded(h, run)

    expect(summary).toContain('Depth: deep, chosen by signals: label risk:high\n')
  })

  it('logs the depth, how it was chosen and the signals on every ledger line', async () => {
    const h = await optedIn(['mistral'])
    await h.writeText(join(h.repo, SPEC), '# Widgets\n\nWidgets need auth.\n')
    const run = await ready(h)
    await recorded(h, run)

    const lines = await ledger(h)

    expect(lines.map((line) => line.type)).toEqual(['run', 'finding'])
    for (const line of lines) {
      expect(line).toMatchObject({
        depth: 'deep',
        depth_chosen_by: 'signals',
        depth_measures: { user_stories: { threshold: 30, value: 0 }, words: { threshold: 5000, value: 5 } },
        depth_signals: [{ kind: 'risk-term', term: 'auth' }],
      })
    }
  })
})

describe('the routing override on this machine', () => {
  it('changes only the keys it names', async () => {
    const override = { stages: { spec: { depth_signals: { thresholds: { user_stories: 1 } } } } }
    const h = await optedIn(EVERY_FAMILY, { routingOverride: override })

    const result = await ready(h)

    expect(result.depth_choice.measures).toEqual({ user_stories: { threshold: 1, value: 2 }, words: { threshold: 5000, value: 25 } })
    expect(result.depth).toBe('deep')
    expect(clients(result)).toEqual(['c1-vibe-mistral-large', 'c2-agy-gemini-flash-high', 'c3-vibe-glm', 'c4-vibe-deepseek-flash'])
  })

  it("changes a client's model without touching the other clients", async () => {
    const h = await optedIn(EVERY_FAMILY, { routingOverride: { clients: { 'vibe-mistral-large': { model: 'mistral-large-5' } } } })

    const result = await ready(h)

    expect(result.critics.map((critic) => [critic.client, critic.model])).toEqual([
      ['vibe-mistral-large', 'mistral-large-5'],
      ['agy-gemini-flash-high', 'gemini-3.8-flash-high'],
    ])
  })

  it('removes a key it sets to null', async () => {
    const h = await optedIn(['mistral', 'google', 'zhipu'], { routingOverride: { fallbacks: { 'agy-gemini-flash-high': null } } })

    const result = await ready(h, { orchestrator_family: 'google' })

    expect(result.dropped).toEqual([{ client: 'agy-gemini-flash-high', family: 'google', reason: 'orchestrator-family' }])
  })

  it('lets a prepared run fan out through the overridden client', async () => {
    const h = await optedIn(['mistral', 'google'], {
      agyScripts: [agyFindings({ findings: [], no_findings_reason: 'Checked every failure path.' })],
      routingOverride: { clients: { 'vibe-mistral-large': { agent: 'critic-mistral-small' } } },
      scripts: [{ exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings: [finding()] })) }],
    })
    const run = await ready(h)

    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const args = h.process.commands[0]?.args ?? []
    expect(args[args.indexOf('--agent') + 1]).toBe('critic-mistral-small')
  })

  it('rejects an override that leaves the routing malformed, naming the override', async () => {
    const h = await optedIn(EVERY_FAMILY, { routingOverride: { clients: { 'vibe-glm': { family: 'openai' } } } })

    await expect(ready(h)).rejects.toThrow(/with the override .*critique-routing\.json: clients\.vibe-glm\.family must be one of/)
  })

  it('rejects an override that is not a JSON object', async () => {
    const h = await optedIn(EVERY_FAMILY, { routingOverride: [] })

    await expect(ready(h)).rejects.toThrow(/critique-routing\.json: the override must be a JSON object/)
  })

  it('rejects an override that is not JSON', async () => {
    const h = await optedIn(EVERY_FAMILY, { routingOverride: '{ "stages": ' })

    await expect(ready(h)).rejects.toThrow(/critique-routing\.json: .*JSON/)
  })

  it('rejects a threshold on a measure the stage does not have', async () => {
    const override = { stages: { spec: { depth_signals: { thresholds: { tickets: 3 } } } } }
    const h = await optedIn(EVERY_FAMILY, { routingOverride: override })

    await expect(ready(h)).rejects.toThrow(
      /stages\.spec\.depth_signals\.thresholds\.tickets is not a measure of stage spec; it knows user_stories, words/,
    )
  })

  it('rejects depth signals on a stage that has no measures yet', async () => {
    const override = { stages: { tickets: { base: [], depth_signals: { thresholds: { tickets: 3 } }, distinct_families: false } } }
    const h = await optedIn(EVERY_FAMILY, { routingOverride: override })

    await expect(ready(h)).rejects.toThrow(/thresholds\.tickets is not a measure of stage tickets; it knows none/)
  })
})
