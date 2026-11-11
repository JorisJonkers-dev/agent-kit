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
import type { FanOutResult, PrepareResult } from './index.js'

/**
 * The architecture-review stage (#483): a review stage that samples a
 * repository's tree beside its recorded decisions, its README and its root
 * build configuration, and never its history. A critic judges the design
 * against what the project declared; a deep run adds a decision recoverer,
 * whose candidates follow the recovery rules and are never accepted.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const AGENT_KIT = 'github.com/jorisjonkers-dev/agent-kit'
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
/** The layer rule the fixture's config declares, quoted as a finding quotes it. */
const RULE = "from: { path: '^src/domain' }"
const CRUISER = `module.exports = {\n  forbidden: [\n    {\n      name: 'domain-may-not-import-adapters',\n      severity: 'error',\n      ${RULE},\n      to: { path: '^src/adapters' },\n    },\n  ],\n}\n`

interface Options {
  readonly adr?: boolean
  readonly code?: boolean
  readonly cruiser?: boolean
  readonly glossary?: boolean
  readonly history?: boolean
  readonly readme?: boolean
  readonly routingOverride?: unknown
}

/**
 * A repository with a layer rule and one violation of it, an ADR in force, a
 * superseded one, a README and a `package.json`. The allowlist lets Mistral,
 * DeepSeek and Zhipu read it, so the roster drops the Gemini critic and its
 * slot falls back to `vibe-glm`.
 */
async function fixture(options: Options = {}): Promise<Harness> {
  const h = await harness(options.routingOverride === undefined ? {} : { routingOverride: options.routingOverride })
  open.push(h)
  await h.commands.allow.run({
    code: options.code ?? true,
    families: ['mistral', 'deepseek', 'zhipu'],
    history: options.history ?? false,
    repo: h.repo,
  })
  if (options.glossary === false) await h.removeFile(join(h.repo, 'CONTEXT.md'))
  if (options.adr ?? true) {
    await h.writeText(join(h.repo, 'docs', 'adr', '0001-layers.md'), ADR)
    await h.writeText(join(h.repo, 'docs', 'adr', '0002-direct.md'), SUPERSEDED)
  }
  if (options.readme ?? true) await h.writeText(join(h.repo, 'README.md'), README)
  if (options.cruiser ?? true) await h.writeText(join(h.repo, '.dependency-cruiser.cjs'), CRUISER)
  await h.writeText(join(h.repo, 'package.json'), PACKAGE)
  await h.writeText(join(h.repo, 'src', 'domain', 'order.ts'), ORDER)
  await h.writeText(join(h.repo, 'src', 'adapters', 'db.ts'), DB)
  await h.git.commit()
  return h
}

async function prepared(h: Harness, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, stage: 'architecture-review', ...extra })
  if (run.status !== 'ready') throw new Error(`not ready: ${JSON.stringify(run)}`)
  return run
}

function critic(agent: string, findings: readonly Record<string, unknown>[]): VibeScript {
  const reply = findings.length === 0 ? { findings, no_findings_reason: 'Read the tree, the records and the configuration.' } : { findings }
  return { agent, exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced(reply)) }
}

/** What each critic of the run reports: the architecture critics, and the deep run's decision recoverer. */
interface Replies {
  readonly architecture?: readonly Record<string, unknown>[]
  readonly recovered?: readonly Record<string, unknown>[]
}

async function reviewed(h: Harness, replies: Replies, extra: Record<string, unknown> = {}): Promise<{ result: FanOutResult; run: PrepareResult }> {
  const run = await prepared(h, extra)
  h.process.queue(
    critic('critic-glm', replies.architecture ?? []),
    critic('critic-mistral-large', []),
    critic('critic-deepseek-flash', replies.recovered ?? []),
  )
  const result = await h.commands.fanOut.run({ run_dir: run.run_dir })
  return { result, run }
}

const TRACE_ORDER = { kind: 'trace', value: 'tree/src/domain/order.ts:1 imports ../adapters/db' }
const TRACE_DB = { kind: 'trace', value: 'tree/src/adapters/db.ts:1 opens the database' }
const RULE_QUOTE = { file: 'references/.dependency-cruiser.cjs', kind: 'quote', value: RULE }
const DECISION_QUOTE = {
  file: 'references/docs/adr/0001-layers.md',
  kind: 'quote',
  value: 'The domain layer declares its ports; adapters implement them.',
}
const FACT_QUOTE = { file: 'facts/INDEX.md', kind: 'quote', value: 'Facts are settled: cite them, never recompute them.' }
const OBSERVABLE = 'Observable: the dependency-cruiser rule passes with no violation.'
const NO_RATIONALE =
  'In the context of layering, facing a shared database (inferred), we decided for a direct import (observed), because no rationale found.'

/** A finding of the architecture critic: the declared rule, the line that breaks it, and a fix with its observable. */
function architectureFinding(id: string, overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return finding({
    category: 'structure',
    check: 'inward-import',
    claim: 'The domain imports the adapter that implements its port.',
    evidence: [RULE_QUOTE, TRACE_ORDER],
    id,
    location: 'tree/src/domain/order.ts:1',
    severity: 'major',
    suggestion: OBSERVABLE,
    ...overrides,
  })
}

/** A candidate the decision recoverer proposes: one decision, traced to two sites, with no reason in the packet. */
function candidate(id: string, overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return finding({
    category: 'undocumented-decision',
    check: 'undocumented-decision',
    claim: NO_RATIONALE,
    evidence: [TRACE_ORDER, TRACE_DB],
    id,
    location: 'tree/src/domain/order.ts:9',
    severity: 'minor',
    suggestion: 'Ask the owner why the domain imports the adapter.',
    ...overrides,
  })
}

/** The pre-verdict fan-out gave each finding, in the order the critics raised them. */
async function preVerdicts(options: Options, findings: readonly Record<string, unknown>[]): Promise<readonly unknown[]> {
  const h = await fixture(options)
  const { result } = await reviewed(h, { architecture: findings })
  return result.findings.map((raised) => raised.pre_verdict)
}

describe('an architecture-review prepare', () => {
  it('is refused while the code permission is off, and names how to turn it on', async () => {
    const h = await fixture({ code: false })

    const result = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, stage: 'architecture-review' })

    expect(result).toEqual({
      message: `${AGENT_KIT} allows documents but not code; record the code permission with allow and "code": true`,
      reason: 'code-not-allowed',
      status: 'refused',
    })
  })

  it('freezes the tree, the facts, the ADRs in force, the glossary, the README and the root build configuration, and reads no history', async () => {
    const h = await fixture({ history: true })

    const run = await prepared(h)

    expect(run.history).toBe(true)
    expect(run.artifact).toEqual({ labels: [], reference: 'architecture', tracker: 'none' })
    expect(run.packet_files.slice(0, 5)).toEqual([
      'references/CONTEXT.md',
      'references/docs/adr/0001-layers.md',
      'references/README.md',
      'references/.dependency-cruiser.cjs',
      'references/package.json',
    ])
    expect(run.packet_files).toContain('tree/MANIFEST.tsv')
    expect(run.packet_files).toContain('facts/INDEX.md')
    expect(run.packet_files).toContain('tree/src/domain/order.ts')
    expect(run.packet_files).toContain('tree/src/adapters/db.ts')
    // A record that is superseded is no decision the code can be checked against.
    expect(run.packet_files).not.toContain('references/docs/adr/0002-direct.md')
    // The history permission is on, and the stage reads no commit message or issue.
    expect(run.packet_files.filter((path) => path.startsWith('history/'))).toEqual([])
    expect(run.repository?.tree_files).toBe(2)
  })

  it('sends no README when the repository has none, and cuts a long one to the byte cap', async () => {
    const without = await fixture({ readme: false })
    const long = await fixture()
    await long.writeText(join(long.repo, 'README.md'), `# Widgets\n\n${'x'.repeat(10_000)}\n`)
    await long.git.commit()

    expect((await prepared(without)).packet_files).not.toContain('references/README.md')
    const run = await prepared(long)
    expect(run.packet_files).toContain('references/README.md')
    expect((await readFile(join(run.packet_dir, 'references', 'README.md'), 'utf8')).length).toBe(8192)
  })

  it('keeps twelve build configurations, the layer config first', async () => {
    const h = await fixture()
    const others = [
      'Cargo.toml',
      'Makefile',
      'build.gradle',
      'docker-compose.yml',
      'go.mod',
      'justfile',
      'pom.xml',
      'pyproject.toml',
      'settings.gradle',
      'tsconfig.build.json',
      'tsconfig.json',
    ]
    for (const name of others) await h.writeText(join(h.repo, name), '# configuration\n')
    await h.git.commit()

    const run = await prepared(h)
    const configs = run.packet_files.filter((path) => path.startsWith('references/') && !path.endsWith('.md'))

    expect(configs).toHaveLength(12)
    expect(configs).toContain('references/.dependency-cruiser.cjs')
    expect(configs).toContain('references/tsconfig.build.json')
    // The layer config is read first, so the twelfth other config is the one the cap leaves out.
    expect(configs).not.toContain('references/tsconfig.json')
  })

  it('runs on a repository with no glossary and no ADR', async () => {
    const h = await fixture({ adr: false, glossary: false })

    const run = await prepared(h)

    expect(run.packet_files.filter((path) => path.startsWith('references/'))).toEqual([
      'references/README.md',
      'references/.dependency-cruiser.cjs',
      'references/package.json',
    ])
    expect(run.packet_files).toContain('tree/src/domain/order.ts')
  })

  it('is refused, before any packet exists, when the documents alone pass the packet cap', async () => {
    const h = await fixture({ routingOverride: { packet: { cap_bytes: 100 } } })

    expect(await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, stage: 'architecture-review' })).toMatchObject({
      reason: 'packet-over-cap',
      status: 'refused',
    })
  })
})

describe('the architecture-review depth signals', () => {
  it('stays at base for a small repository, and says what it measured', async () => {
    const h = await fixture()

    const run = await prepared(h)

    expect(run.depth).toBe('base')
    expect(run.depth_choice.measures.context_maps).toEqual({ threshold: 0, value: 0 })
    expect(run.depth_choice.measures.source_lines?.threshold).toBe(20000)
    expect(run.depth_choice.signals).toEqual([])
    expect(run.critics.map((critic) => [critic.client, critic.role])).toEqual([
      ['vibe-glm', 'architecture-critic'],
      ['vibe-mistral-large', 'architecture-critic'],
    ])
    expect(run.dropped.map((critic) => [critic.client, critic.reason])).toEqual([['agy-gemini-flash-high', 'family-not-allowed']])
  })

  it('goes deep above 20k tracked source lines, and adds the deep critics', async () => {
    const h = await fixture()
    await h.writeText(join(h.repo, 'src', 'big.ts'), 'export {}\n'.repeat(20_000))
    await h.git.commit()

    const run = await prepared(h)

    expect(run.depth).toBe('deep')
    expect(run.depth_choice.signals.map((signal) => (signal.kind === 'threshold' ? signal.measure : signal.kind))).toEqual(['source_lines'])
    expect(run.depth_choice.measures.source_lines?.value).toBeGreaterThan(20000)
    expect(run.critics.map((critic) => [critic.client, critic.role])).toEqual([
      ['vibe-glm', 'architecture-critic'],
      ['vibe-mistral-large', 'architecture-critic'],
      ['vibe-deepseek-flash', 'decision-recoverer'],
    ])
    expect(run.dropped.map((critic) => [critic.client, critic.reason])).toEqual([
      ['agy-gemini-flash-high', 'family-not-allowed'],
      ['vibe-glm', 'same-family'],
    ])
  })

  it('goes deep when the repository has a context map', async () => {
    const h = await fixture()
    await h.writeText(join(h.repo, 'CONTEXT-MAP.md'), '# Map\n')
    await h.git.commit()

    const run = await prepared(h)

    expect(run.depth).toBe('deep')
    expect(run.depth_choice.signals).toEqual([{ kind: 'threshold', measure: 'context_maps', threshold: 0, value: 1 }])
    expect(run.packet_files).toContain('references/CONTEXT-MAP.md')
    expect(run.critics.map((critic) => critic.client)).toContain('vibe-glm')
    expect(run.critics.map((critic) => critic.client)).toContain('vibe-deepseek-flash')
  })
})

describe('the architecture-review evidence rule', () => {
  it('hints a dependency-direction finding with no layer config, and asks an unasked question only with a trace', async () => {
    const verdicts = await preVerdicts({ cruiser: false }, [
      architectureFinding('F1', { evidence: [TRACE_ORDER] }),
      architectureFinding('F2', { category: 'unasked-question', evidence: [TRACE_ORDER], location: 'tree/src/domain/order.ts:2' }),
      architectureFinding('F3', {
        category: 'unasked-question',
        evidence: [{ kind: 'input', value: 'a deploy with no network' }],
        location: 'tree/src/domain/order.ts:3',
      }),
    ])

    expect(verdicts[0]).toMatchObject({ reason_code: 'no-layer-config', verdict: 'hint' })
    expect(verdicts[1]).toBeUndefined()
    expect(verdicts[2]).toMatchObject({ reason_code: 'no-trace', verdict: 'hint' })
  })

  it('hints a verdict word that rests on no fact and no quoted decision, and passes one that quotes a decision or a fact', async () => {
    const claim = 'The domain and the adapters are tightly coupled.'
    const verdicts = await preVerdicts({}, [
      architectureFinding('F1', { check: 'decision-spread', claim, evidence: [TRACE_ORDER] }),
      architectureFinding('F2', {
        check: 'decision-spread',
        claim,
        evidence: [DECISION_QUOTE, TRACE_ORDER],
        location: 'tree/src/domain/order.ts:2',
      }),
      architectureFinding('F3', {
        check: 'decision-spread',
        claim,
        evidence: [FACT_QUOTE, TRACE_ORDER],
        location: 'tree/src/domain/order.ts:3',
      }),
    ])

    expect(verdicts[0]).toMatchObject({ reason_code: 'verdict-word', verdict: 'hint' })
    expect(verdicts[1]).toBeUndefined()
    expect(verdicts[2]).toBeUndefined()
  })

  it('hints a finding with only a quote and no rule, fact or trace, and one whose fix names no observable', async () => {
    const verdicts = await preVerdicts({}, [
      architectureFinding('F1', {
        check: 'decision-spread',
        claim: 'The domain reaches into the adapter query builder.',
        evidence: [{ file: 'tree/src/domain/order.ts', kind: 'quote', value: 'export const find = (id: string) => db.find(id)' }],
      }),
      architectureFinding('F2', {
        check: 'decision-spread',
        evidence: [DECISION_QUOTE, TRACE_ORDER],
        location: 'tree/src/domain/order.ts:2',
        suggestion: 'Move the query behind the port.',
      }),
      architectureFinding('F3', { check: 'decision-spread', evidence: [DECISION_QUOTE, TRACE_ORDER], location: 'tree/src/domain/order.ts:3' }),
      architectureFinding('F4', {
        check: undefined,
        evidence: [DECISION_QUOTE, TRACE_ORDER],
        location: 'tree/src/domain/order.ts:4',
        suggestion: undefined,
      }),
    ])

    expect(verdicts[0]).toMatchObject({ reason_code: 'no-rule-or-consequence', verdict: 'hint' })
    expect(verdicts[1]).toMatchObject({ reason_code: 'no-observable', verdict: 'hint' })
    expect(verdicts[2]).toBeUndefined()
    expect(verdicts[3]).toMatchObject({ reason_code: 'no-observable', verdict: 'hint' })
  })
})

describe('an architecture-review run over the fixture repository', () => {
  it('yields a finding that cites the declared rule and the line, and records it as accepted', async () => {
    const h = await fixture()

    const { result, run } = await reviewed(h, { architecture: [architectureFinding('F1')] })

    expect(result.findings).toHaveLength(1)
    expect(result.findings[0]?.pre_verdict).toBeUndefined()

    const recorded = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [{ finding_id: 'F1', reason: 'The rule is declared and the import breaks it.', verdict: 'accepted' }],
    })

    expect(recorded.counts.accepted).toBe(1)
    const report = await readFile(recorded.report_path, 'utf8')
    expect(report).toMatch(/### F1 \[(?:critical|major|minor) structure\] tree\/src\/domain\/order\.ts:1/)
    expect(report).toContain("- quote (references/.dependency-cruiser.cjs): from: { path: '^src/domain' }")
    expect(report).toContain('- trace: tree/src/domain/order.ts:1 imports ../adapters/db')
    expect(report).toContain(`Suggestion: ${OBSERVABLE}`)
    // The accepted finding is a cited candidate: its lines, its evidence and its observable.
    const candidates = report.slice(report.indexOf('## Architecture candidates'))
    expect(candidates).toContain('### F1')
    expect(candidates).toContain('- trace: tree/src/domain/order.ts:1 imports ../adapters/db')
    expect(candidates).toContain(`- ${OBSERVABLE}`)
    // The stage reads no tracker: nothing was viewed, linked or created through gh.
    expect(h.gh.requests).toEqual([])
    expect(h.gh.linkRequests).toEqual([])
    expect(h.process.commands.filter((command) => command.args.includes('issue'))).toEqual([])
  })
})

describe('a deep architecture-review run with a decision recoverer', () => {
  it('judges the recoverer by the recovery rule, and never accepts a recovered decision', async () => {
    const h = await fixture()

    const { result, run } = await reviewed(
      h,
      {
        architecture: [architectureFinding('F1')],
        recovered: [candidate('F2', { evidence: [TRACE_ORDER] }), candidate('F3', { location: 'tree/src/domain/order.ts:10' })],
      },
      { depth: 'deep' },
    )

    expect(result.findings.map((raised) => raised.pre_verdict?.reason_code)).toEqual([undefined, 'single-site', undefined])

    await expect(h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F3', verdict: 'accepted' }] })).rejects.toThrow(
      /F3: a recovered decision is never accepted/,
    )

    const recorded = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [
        { finding_id: 'F1', reason: 'The rule is declared and the import breaks it.', verdict: 'accepted' },
        { finding_id: 'F3', reason: 'Is this a decision you made, and why?', verdict: 'needs-human' },
      ],
    })

    expect(recorded.counts).toEqual({ accepted: 1, hint: 0, 'needs-human': 1, rejected: 1 })
    expect(recorded.questions).toEqual(['F3'])
    expect(recorded.deferred).toEqual([])
  })

  it('asks ten questions across both kinds, the architecture findings first, and defers two', async () => {
    const h = await fixture()
    const architecture = Array.from({ length: 8 }, (_, n) =>
      architectureFinding(`A${String(n + 1)}`, { location: `tree/src/domain/order.ts:${String(n + 1)}`, severity: 'critical' }),
    )
    const recovered = Array.from({ length: 4 }, (_, n) => candidate(`C${String(n + 1)}`, { location: `tree/src/domain/order.ts:${String(n + 9)}` }))

    const { result, run } = await reviewed(h, { architecture, recovered }, { depth: 'deep' })

    expect(result.findings).toHaveLength(12)
    const recorded = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: result.findings.map((raised) => ({ finding_id: raised.id, reason: `Question ${raised.id}?`, verdict: 'needs-human' })),
    })

    expect(recorded.questions).toEqual(['F1', 'F2', 'F3', 'F4', 'F5', 'F6', 'F7', 'F8', 'F9', 'F10'])
    expect(recorded.deferred).toEqual(['F11', 'F12'])
    expect(recorded.counts['needs-human']).toBe(12)
  })
})
