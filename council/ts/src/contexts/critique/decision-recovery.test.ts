import { readFile, readdir } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  fenced,
  finding,
  harness,
  isolatedExport,
  ticketIssue,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import type { FanOutResult, PrepareResult } from './index.js'

/**
 * The decision-recovery stage (#480): a review stage that samples a
 * repository's tree beside its recorded decisions, and, only where the
 * allowlist entry allows history, its commit messages and issues. A critic
 * proposes each decision the code embodies as a candidate; the runner rejects
 * mechanically any that is not traced to two sites or invents its reason, and
 * the rest are questions for the owner, ten at most.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const AGENT_KIT = 'github.com/jorisjonkers-dev/agent-kit'
const DB = 'export const db = openSqlite("local.db")\n'
const CACHE = 'export const cache = openSqlite("cache.db")\n'
const ADR = '# Widgets are frobnicated nightly\n\nStatus: accepted\n\nThe nightly job frobnicates widgets, not the request path.\n'
const SUPERSEDED = '# Widgets are frobnicated live\n\nStatus: superseded\n\nThe request path frobnicates widgets.\n'
const REASON = 'Use sqlite for storage because the box has no network'

interface Options {
  readonly code?: boolean
  readonly history?: boolean
  readonly scripts?: VibeScript[]
  readonly routingOverride?: unknown
  readonly tracker?: Parameters<typeof harness>[0] extends infer O ? (O extends { tracker?: infer T } ? T : never) : never
  readonly issues?: Parameters<typeof harness>[0] extends infer O ? (O extends { issues?: infer T } ? T : never) : never
}

/** A repository with two sampled source files, a recorded ADR in force, a superseded one, and a commit that gives a reason. */
async function fixture(options: Options = {}): Promise<Harness> {
  const h = await harness({
    ...(options.issues === undefined ? {} : { issues: options.issues }),
    ...(options.scripts === undefined ? {} : { scripts: options.scripts }),
    ...(options.tracker === undefined ? {} : { tracker: options.tracker }),
    ...(options.routingOverride === undefined ? {} : { routingOverride: options.routingOverride }),
  })
  open.push(h)
  await h.commands.allow.run({
    code: options.code ?? true,
    families: ['mistral', 'deepseek'],
    history: options.history ?? false,
    repo: h.repo,
  })
  await h.writeText(join(h.repo, 'docs', 'adr', '0001-nightly.md'), ADR)
  await h.writeText(join(h.repo, 'docs', 'adr', '0002-live.md'), SUPERSEDED)
  await h.writeText(join(h.repo, 'src', 'db.ts'), DB)
  await h.writeText(join(h.repo, 'src', 'cache.ts'), CACHE)
  await h.git.commit('2026-02-01T00:00:00+00:00', `${REASON} (#12)\n\nThe deploy target is offline.`)
  return h
}

async function prepared(h: Harness, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, stage: 'decision-recovery', ...extra })
  if (run.status !== 'ready') throw new Error(`not ready: ${JSON.stringify(run)}`)
  return run
}

async function files(dir: string): Promise<string[]> {
  const entries = await readdir(dir, { recursive: true, withFileTypes: true })
  return entries
    .filter((entry) => !entry.isDirectory())
    .map((entry) => join(entry.parentPath, entry.name).slice(dir.length + 1))
    .sort()
}

function critic(agent: string, findings: readonly Record<string, unknown>[]): VibeScript {
  const reply = findings.length === 0 ? { findings, no_findings_reason: 'Read the tree and the records.' } : { findings }
  return { agent, exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced(reply)) }
}

const TRACE_DB = { kind: 'trace', value: 'tree/src/db.ts:1 opens sqlite directly' }
const TRACE_CACHE = { kind: 'trace', value: 'tree/src/cache.ts:1 opens sqlite again' }
const NO_RATIONALE = 'In the context of storage, facing no network (inferred), we decided for sqlite (observed), because no rationale found.'

/** Candidates with one location and claim are one finding to the runner, so each is its own by id. */
function candidate(id: string, overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return finding({
    location: `tree/src/db.ts:1 candidate ${id}`,
    category: 'undocumented-decision',
    check: 'undocumented-decision',
    claim: NO_RATIONALE,
    evidence: [TRACE_DB, TRACE_CACHE],
    id,
    ...overrides,
  })
}

async function reviewed(h: Harness, findings: readonly Record<string, unknown>[], extra: Record<string, unknown> = {}): Promise<{ run: PrepareResult; result: FanOutResult }> {
  const run = await prepared(h, extra)
  h.process.queue(critic('critic-deepseek-flash', findings), critic('critic-mistral-large', []))
  const result = await h.commands.fanOut.run({ run_dir: run.run_dir })
  return { result, run }
}

describe('a decision-recovery prepare', () => {
  it('is refused while the code permission is off, and names how to turn it on', async () => {
    const h = await fixture({ code: false })

    const result = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, stage: 'decision-recovery' })

    expect(result).toEqual({
      message: `${AGENT_KIT} allows documents but not code; record the code permission with allow and "code": true`,
      reason: 'code-not-allowed',
      status: 'refused',
    })
  })

  it('freezes the tree, the facts, the ADRs in force and the glossary, and no history while the history permission is off', async () => {
    const h = await fixture()

    const run = await prepared(h)

    expect(await files(run.packet_dir)).toEqual([
      'facts/INDEX.md',
      'references/CONTEXT.md',
      'references/docs/adr/0001-nightly.md',
      'tree/MANIFEST.tsv',
      'tree/src/cache.ts',
      'tree/src/db.ts',
    ])
    expect(run.history).toBe(false)
    expect(run.artifact).toEqual({ labels: [], reference: 'recovery', tracker: 'none' })
    expect(run.repository?.tree_files).toBe(2)
  })

  it('adds the commit log of the sampled files and the issues it cites once the history permission is on', async () => {
    const h = await fixture({
      history: true,
      issues: { [`${AGENT_KIT}#12`]: ticketIssue(12, 'Storage', 'Local only: no network on the box.') },
      tracker: 'github',
    })

    const run = await prepared(h)

    expect(run.history).toBe(true)
    expect(await files(run.packet_dir)).toEqual([
      'facts/INDEX.md',
      'history/issues/issue-12.md',
      'history/log.txt',
      'references/CONTEXT.md',
      'references/docs/adr/0001-nightly.md',
      'tree/MANIFEST.tsv',
      'tree/src/cache.ts',
      'tree/src/db.ts',
    ])
    const log = await readFile(join(run.packet_dir, 'history', 'log.txt'), 'utf8')
    expect(log).toContain(`${REASON} (#12)`)
    expect(log).toContain('The deploy target is offline.')
    expect(log).toContain('Files: src/cache.ts, src/db.ts')
    expect(await readFile(join(run.packet_dir, 'history', 'issues', 'issue-12.md'), 'utf8')).toContain('Local only: no network on the box.')
  })

  it('leaves out an issue the tracker cannot read, and an unsupported tracker, and still sends the log', async () => {
    const missing = await fixture({ history: true, tracker: 'github' })
    const gitlab = await fixture({ history: true, tracker: 'gitlab' })

    expect(await files((await prepared(missing)).packet_dir)).toContain('history/log.txt')
    expect(await files((await prepared(missing)).packet_dir)).not.toContain('history/issues/issue-12.md')
    expect(await files((await prepared(gitlab)).packet_dir)).toContain('history/log.txt')
  })

  it('is refused, before any packet exists, when the records and the manifest alone pass the packet cap', async () => {
    const h = await fixture({ routingOverride: { packet: { cap_bytes: 100 } } })

    expect(await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, stage: 'decision-recovery' })).toMatchObject({
      reason: 'packet-over-cap',
      status: 'refused',
    })
  })

  it('runs on a repository with no glossary and no ADR, sampling its source', async () => {
    const h = await fixture()
    await h.removeFile(join(h.repo, 'CONTEXT.md'))
    await h.removeFile(join(h.repo, 'docs', 'adr', '0001-nightly.md'))
    await h.removeFile(join(h.repo, 'docs', 'adr', '0002-live.md'))
    await h.git.commit()

    const run = await prepared(h)

    expect(run.packet_files.filter((path) => path.startsWith('references/'))).toEqual([])
    expect(run.packet_files).toContain('tree/src/db.ts')
  })

  it('sends a context map beside the glossary', async () => {
    const h = await fixture()
    await h.writeText(join(h.repo, 'CONTEXT-MAP.md'), '# Map\n')
    await h.git.commit()

    const run = await prepared(h)

    expect(run.packet_files).toContain('references/CONTEXT-MAP.md')
  })

  it('takes DeepSeek and Mistral Large as decision-recoverers, and never a flow', async () => {
    const h = await fixture()

    const run = await prepared(h)

    expect(run.critics.map((c) => [c.client, c.role])).toEqual([
      ['vibe-deepseek-flash', 'decision-recoverer'],
      ['vibe-mistral-large', 'decision-recoverer'],
    ])
    await expect(h.commands.prepare.run({ flow: 'draft:x', orchestrator_family: 'anthropic', repo: h.repo, stage: 'decision-recovery' })).rejects.toThrow(
      /decision-recovery is a review stage: each run is a flow of its own, so it takes no flow/,
    )
  })
})

describe('the decision-recovery depth signals', () => {
  it('stays at base for a small repository, and says what it measured', async () => {
    const h = await fixture()

    const run = await prepared(h)

    expect(run.depth).toBe('base')
    expect(run.depth_choice.measures).toEqual({ context_maps: { threshold: 0, value: 0 }, source_lines: { threshold: 20000, value: 2 } })
    expect(run.depth_choice.signals).toEqual([])
  })

  it('goes deep above 20k tracked source lines, counting source only', async () => {
    const h = await fixture()
    await h.writeText(join(h.repo, 'src', 'big.ts'), 'export {}\n'.repeat(20_000))
    await h.writeText(join(h.repo, 'docs', 'long.md'), 'text\n'.repeat(5_000))
    await h.git.commit()

    const run = await prepared(h)

    expect(run.depth).toBe('deep')
    expect(run.depth_choice.signals).toEqual([{ kind: 'threshold', measure: 'source_lines', threshold: 20000, value: 20_002 }])
  })

  it('goes deep when the repository has a context map', async () => {
    const h = await fixture()
    await h.writeText(join(h.repo, 'CONTEXT-MAP.md'), '# Map\n')
    await h.git.commit()

    const run = await prepared(h)

    expect(run.depth).toBe('deep')
    expect(run.depth_choice.signals).toEqual([{ kind: 'threshold', measure: 'context_maps', threshold: 0, value: 1 }])
  })
})

describe('the decision-recovery evidence rule', () => {
  async function preVerdict(findings: Record<string, unknown>): Promise<unknown> {
    const h = await fixture({ history: true })
    const { result } = await reviewed(h, [findings])
    return result.findings[0]?.pre_verdict
  }

  it('rejects a candidate that cites no file of tree/', async () => {
    expect(await preVerdict(candidate('F1', { evidence: [{ kind: 'input', value: 'a deploy with no network' }] }))).toMatchObject({
      reason_code: 'no-code-trace',
      verdict: 'rejected',
    })
  })

  it('rejects a candidate with no evidence at all as untraced', async () => {
    expect(await preVerdict(candidate('F1', { evidence: [] }))).toMatchObject({ reason_code: 'no-code-trace', verdict: 'rejected' })
  })

  it('rejects a candidate traced to one site only, even at two lines of it', async () => {
    const lines = [TRACE_DB, { kind: 'trace', value: 'tree/src/db.ts:1-1 and again' }]
    expect(await preVerdict(candidate('F1', { evidence: lines }))).toMatchObject({ reason_code: 'single-site', verdict: 'rejected' })
  })

  it('does not count a cited record or history file as a site of the code', async () => {
    const records = [TRACE_DB, { kind: 'trace', value: 'references/docs/adr/0001-nightly.md:3 says so' }]
    expect(await preVerdict(candidate('F1', { evidence: records }))).toMatchObject({ reason_code: 'single-site', verdict: 'rejected' })
  })

  it('rejects a candidate whose because has neither a history quote nor "no rationale found"', async () => {
    const claim = 'In the context of storage, we decided for sqlite (observed), because the team likes it (inferred).'
    expect(await preVerdict(candidate('F1', { claim }))).toMatchObject({ reason_code: 'invented-rationale', verdict: 'rejected' })
  })

  it('passes a candidate with two traces and "no rationale found"', async () => {
    expect(await preVerdict(candidate('F1'))).toBeUndefined()
  })

  it('passes a candidate with two traces and a quote from history/ for its reason', async () => {
    const claim = 'In the context of storage, we decided for sqlite (observed), because the box has no network (observed).'
    const quote = { file: 'history/log.txt', kind: 'quote', value: REASON }

    expect(await preVerdict(candidate('F1', { claim, evidence: [TRACE_DB, TRACE_CACHE, quote] }))).toBeUndefined()
  })

  it('rejects a quote of history/ that is not in the packet as fabricated, before any rule', async () => {
    const quote = { file: 'history/log.txt', kind: 'quote', value: 'Use postgres because the DBA asked' }

    expect(await preVerdict(candidate('F1', { evidence: [TRACE_DB, TRACE_CACHE, quote] }))).toMatchObject({
      reason_code: 'fabricated-evidence',
      verdict: 'rejected',
    })
  })

  it('rejects a trace that cites a line the file does not have as fabricated', async () => {
    const trace = { kind: 'trace', value: 'tree/src/cache.ts:40 opens sqlite again' }

    expect(await preVerdict(candidate('F1', { evidence: [TRACE_DB, trace] }))).toMatchObject({ reason_code: 'fabricated-evidence' })
  })

  it('hints at a finding that is not a candidate', async () => {
    expect(await preVerdict(candidate('F1', { category: 'omission' }))).toMatchObject({ reason_code: 'not-a-candidate', verdict: 'hint' })
  })

  it('leaves a mechanical rejection standing against any verdict, and out of the dissent', async () => {
    const h = await fixture()
    const { run } = await reviewed(h, [candidate('F1', { evidence: [TRACE_DB], severity: 'critical' })])

    await expect(h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'needs-human', reason: 'ask' }] })).rejects.toThrow(
      /F1 was rejected as single-site, which a verdict cannot overturn/,
    )
    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })
    expect(recorded.counts).toEqual({ accepted: 0, hint: 0, 'needs-human': 0, rejected: 1 })
    expect(await readFile(join(run.run_dir, 'report.md'), 'utf8')).toContain('Verdict: rejected (single-site)')
    expect(recorded.summary).toContain('## Dissent\n\nnone')
  })
})

describe('a decision-recovery run over the fixture repository', () => {
  it('yields a recovered decision with its evidence, held for the owner', async () => {
    const h = await fixture({ history: true })
    const { result, run } = await reviewed(h, [candidate('F1', { severity: 'critical' })])

    const recorded = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [{ finding_id: 'F1', reason: 'Is this a decision you made, and why? What did you reject?', verdict: 'needs-human' }],
    })

    expect(result.findings).toHaveLength(1)
    expect(recorded.counts['needs-human']).toBe(1)
    expect(recorded.questions).toEqual(['F1'])
    expect(recorded.deferred).toEqual([])
    expect(recorded.second_round_due).toBe(false)
    const report = await readFile(recorded.report_path, 'utf8')
    expect(report).toContain('[critical undocumented-decision] tree/src/db.ts:1 candidate F1')
    expect(report).toContain('- trace: tree/src/db.ts:1 opens sqlite directly')
    expect(report).toContain('- trace: tree/src/cache.ts:1 opens sqlite again')
    expect(recorded.summary).toContain('## Questions for you\n\n- F1 [critical undocumented-decision] tree/src/db.ts:1 candidate F1')
    expect(recorded.summary).toContain('Question: Is this a decision you made')
    expect(recorded.summary).not.toContain('Do not start')
  })

  it('writes nothing under the decision log: the run directory holds the candidates', async () => {
    const h = await fixture()
    const { run } = await reviewed(h, [candidate('F1')])
    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', reason: 'ask', verdict: 'needs-human' }] })

    expect(await files(join(h.repo, 'docs', 'adr'))).toEqual(['0001-nightly.md', '0002-live.md'])
    expect(await readFile(join(run.run_dir, 'report.md'), 'utf8')).toContain('F1')
  })

  it('never accepts a recovered decision', async () => {
    const h = await fixture()
    const { run } = await reviewed(h, [candidate('F1')])

    await expect(h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })).rejects.toThrow(
      /F1: a recovered decision is never accepted/,
    )
  })

  it('rejects a candidate for a reason the orchestrator names, such as not-significant', async () => {
    const h = await fixture()
    const { run } = await reviewed(h, [candidate('F1'), candidate('F2'), candidate('F3')])

    const recorded = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [
        { finding_id: 'F1', reason: 'Easy to reverse.', reason_code: 'not-significant', verdict: 'rejected' },
        { finding_id: 'F2', reason: 'ADR-0001 states it.', reason_code: 'duplicates-adr', verdict: 'rejected' },
        { finding_id: 'F3', reason: 'No history gives it.', reason_code: 'invented-rationale', verdict: 'rejected' },
      ],
    })

    expect(recorded.counts.rejected).toBe(3)
    expect(recorded.questions).toEqual([])
  })

  it('refuses an answer for a candidate past the question cap', async () => {
    const h = await fixture()
    const { run } = await reviewed(h, Array.from({ length: 11 }, (_, n) => candidate(`F${String(n + 1)}`)))
    const verdicts = Array.from({ length: 11 }, (_, n) => ({ answered: n === 10, finding_id: `F${String(n + 1)}`, reason: 'ask', verdict: 'needs-human' }))

    await expect(h.commands.record.run({ run_dir: run.run_dir, verdicts })).rejects.toThrow(/F11 is past the 10 questions a run asks/)
  })

  it('asks ten of twelve candidates, ranked by confidence, and defers two in the report', async () => {
    const h = await fixture()
    // F1..F5 minor (low), F6..F9 major, F10..F12 critical (high): the two lowest, last in order, are deferred.
    const severities = ['minor', 'minor', 'minor', 'minor', 'minor', 'major', 'major', 'major', 'major', 'critical', 'critical', 'critical']
    const { run } = await reviewed(
      h,
      severities.map((severity, n) => candidate(`F${String(n + 1)}`, { severity })),
    )

    const recorded = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: severities.map((_, n) => ({ finding_id: `F${String(n + 1)}`, reason: `Question ${String(n + 1)}?`, verdict: 'needs-human' })),
    })

    expect(recorded.questions).toEqual(['F10', 'F11', 'F12', 'F6', 'F7', 'F8', 'F9', 'F1', 'F2', 'F3'])
    expect(recorded.deferred).toEqual(['F4', 'F5'])
    const report = await readFile(recorded.report_path, 'utf8')
    expect(report).toContain('## Deferred candidates\n\n- F4 [minor] tree/src/db.ts:1 candidate F4')
    expect(report).toContain('- F5 [minor] tree/src/db.ts:1 candidate F5')
    expect(recorded.summary).toContain('## Deferred\n\n- F4 [minor undocumented-decision]')
    expect(recorded.summary.match(/^- F\d+ .*Question:/gm)).toHaveLength(10)
  })

  it('says none when nothing is deferred, and trims a summary that is too long', async () => {
    const h = await fixture()
    const { run } = await reviewed(h, Array.from({ length: 12 }, (_, n) => candidate(`F${String(n + 1)}`, { claim: `${NO_RATIONALE} ${'x'.repeat(200)}` })))
    const verdicts = Array.from({ length: 12 }, (_, n) => ({ finding_id: `F${String(n + 1)}`, reason: 'q'.repeat(900), verdict: 'needs-human' }))

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts })

    expect(recorded.summary.length).toBeLessThanOrEqual(6000)
    expect(recorded.summary).toContain('more in the report')
    expect(await readFile(recorded.report_path, 'utf8')).toContain('## Deferred candidates')
  })

  it('reports no deferred candidate when every candidate was asked', async () => {
    const h = await fixture()
    const { run } = await reviewed(h, [candidate('F1', { severity: 'minor' })])

    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', reason: 'ask', verdict: 'needs-human' }] })

    expect(await readFile(recorded.report_path, 'utf8')).toContain('## Deferred candidates\n\nnone')
    expect(recorded.summary).toContain('## Deferred\n\nnone')
  })
})

describe('the record /recover-decisions writes from an owner’s answer', () => {
  const SKILL = join(import.meta.dirname, '..', '..', '..', '..', '..', 'skills', 'workstation', 'recover-decisions', 'SKILL.md')

  /** The house format the skill shows, with the owner's answers in place of its placeholders. */
  async function written(): Promise<string> {
    const skill = await readFile(SKILL, 'utf8')
    const template = /```markdown\n([\s\S]*?)```/.exec(skill)?.[1] ?? ''
    return template
      .split('\n')
      .map((line) => line.replace(/^ {2}/, ''))
      .join('\n')
      .replace('<the decision, as a sentence>', 'Storage is a local SQLite file')
      .replace(/<Context and decision:[\s\S]*?>\n/, 'In the context of an offline deploy target, we decided for a local SQLite file, because the box has no network.\n')
      .replace(/<the alternative the owner named>: <why it lost, in the owner's\nwords>/, 'a hosted Postgres: it needs a network the box does not have')
      .replace('<the cost the owner named>', 'Every service opens its own file, so a second writer needs a migration.')
  }

  it('holds a decision sentence, a rejected alternative with its reason and a cost, and no status line', async () => {
    const record = await written()

    expect(record).toMatch(/^# Storage is a local SQLite file$/m)
    expect(record).toContain('we decided for a local SQLite file, because the box has no network.')
    expect(record).toContain('We rejected a hosted Postgres: it needs a network the box does not have.')
    expect(record).toMatch(/## Consequences\n\n- Every service opens its own file/)
    expect(record).not.toMatch(/^status\b/im)
    expect(record).not.toMatch(/[<>]/)
  })

  it('passes the decision-log check beside the records already there, and is read as a record in force', async () => {
    const h = await fixture()
    await h.writeText(join(h.repo, 'docs', 'adr', '0003-storage.md'), await written())

    const checked = await h.commands.checkAdrs.run({ repo: h.repo })

    expect(checked).toMatchObject({ status: 'passed' })
    const run = await prepared(h)
    expect(run.packet_files).toContain('references/docs/adr/0003-storage.md')
  })
})
