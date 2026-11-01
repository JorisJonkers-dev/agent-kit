import { createHash } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  ASSETS_ROOT,
  fenced,
  harness,
  isolatedExport,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import type { PrepareResult } from './index.js'

/**
 * What `/review-feature` (#189) relies on the runner for: the feature-review
 * roster in shipped routing, the `feature-reviewer` role, the evidence rule
 * for accepting a feature finding, a follow-up ticket per accepted finding,
 * and the gate line with the head it was computed for.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const SPEC = '.scratch/widgets/spec.md'
const ISSUES = '.scratch/widgets/issues'

/**
 * A repository opted in for code with every family, on local markdown, whose
 * feature branch holds two tickets' work: `c2` is the base, `c3` the head.
 */
async function feature(
  options: Parameters<typeof harness>[0] & { readonly families?: readonly string[] } = {},
): Promise<Harness> {
  const h = await harness(options)
  open.push(h)
  await h.commands.allow.run({ code: true, families: options.families ?? ['mistral', 'google', 'zhipu', 'deepseek'], repo: h.repo })
  await h.writeText(join(h.repo, ISSUES, '01-store.md'), '# 01: Store widgets\n\n**Status:** ready-for-agent\n')
  await h.writeText(join(h.repo, ISSUES, '02-report.md'), '# 02: Report widgets\n\n**Status:** ready-for-agent\n\n**Blocked by:** 01\n')
  await h.git.commit()
  await h.writeText(join(h.repo, 'src/store.ts'), 'export function store(count: number): string {\n  return String(count)\n}\n')
  await h.writeText(join(h.repo, 'src/report.ts'), "import { store } from './store'\n\nexport const total: number = store(3)\n")
  await h.git.commit()
  return h
}

async function prepared(h: Harness, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const result = await h.commands.prepare.run({
    base: 'HEAD~1',
    orchestrator_family: 'anthropic',
    repo: h.repo,
    spec: SPEC,
    stage: 'feature-review',
    ...extra,
  })
  if (result.status !== 'ready') throw new Error(`prepare refused: ${result.message}`)
  return result
}

describe('the feature-review roster in shipped routing', () => {
  it('runs Mistral Large on integration and Gemini Flash on failure modes, both as feature reviewers', async () => {
    const h = await feature()

    const run = await prepared(h)

    expect(run.critics).toEqual([
      {
        client: 'vibe-mistral-large',
        critic_id: 'c1-vibe-mistral-large',
        family: 'mistral',
        lenses: ['integration'],
        model: 'mistral-large-4-0-3',
        role: 'feature-reviewer',
      },
      {
        client: 'agy-gemini-flash-high',
        critic_id: 'c2-agy-gemini-flash-high',
        family: 'google',
        lenses: ['failure-modes'],
        model: 'gemini-3.8-flash-high',
        role: 'feature-reviewer',
      },
    ])
  })

  it('adds GLM on operability and DeepSeek Flash on simplicity at deep depth', async () => {
    const h = await feature()

    const run = await prepared(h, { depth: 'deep' })

    expect(run.critics.slice(2).map(({ client, lenses, role }) => ({ client, lenses, role }))).toEqual([
      { client: 'vibe-glm', lenses: ['operability'], role: 'feature-reviewer' },
      { client: 'vibe-deepseek-flash', lenses: ['simplicity'], role: 'feature-reviewer' },
    ])
  })

  it('caps each critic at $1.00 and 12 minutes', async () => {
    const h = await feature()

    const run = await prepared(h, { depth: 'deep' })

    const manifest = JSON.parse(await readFile(run.manifest_path, 'utf8')) as {
      readonly critics: readonly { readonly max_price_usd: number; readonly time_limit_s: number }[]
    }
    expect(manifest.critics.map(({ max_price_usd, time_limit_s }) => ({ max_price_usd, time_limit_s }))).toEqual(
      Array.from({ length: 4 }, () => ({ max_price_usd: 1, time_limit_s: 720 })),
    )
  })

  it('still runs Gemini once only the Antigravity reserve is left', async () => {
    const h = await feature()
    const launch = { client: 'agy-gemini-flash-high', kind: 'agy', recorded_at: '2026-10-30T08:00:00.000Z', type: 'invocation' }
    await h.writeText(join(h.home, '.local/share/agent-kit/critique/ledger.jsonl'), `${JSON.stringify(launch)}\n`.repeat(10))

    const run = await prepared(h)

    expect(run.critics.map((critic) => critic.client)).toEqual(['vibe-mistral-large', 'agy-gemini-flash-high'])
  })
})

function critic(findings: readonly Record<string, unknown>[]): VibeScript {
  return { exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings })) }
}

/** A repository that allows Mistral only, so a feature review runs the one Mistral Large critic. */
async function mistralOnly(scripts: VibeScript[]): Promise<Harness> {
  return feature({ families: ['mistral'], scripts })
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

describe('the feature-reviewer role', () => {
  it("is the role a feature review's critic is given", async () => {
    const h = await mistralOnly([critic([])])
    const run = await prepared(h)

    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const prompt = await readFile(join(run.run_dir, 'critics', 'c1-vibe-mistral-large', 'prompt.md'), 'utf8')
    expect(prompt).toContain('# Role: feature reviewer')
  })

  it('has its hash recorded in the ledger', async () => {
    const h = await mistralOnly([{ ...critic([]), stdout: vibeStdout(fenced({ findings: [], no_findings_reason: 'Read both tickets.' })) }])
    const run = await prepared(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    const role = await readFile(join(ASSETS_ROOT, 'roles', 'feature-reviewer.md'), 'utf8')
    const [line] = await ledgerLines(h)
    expect(line).toMatchObject({ prompt_hashes: { role: createHash('sha256').update(role).digest('hex') }, role: 'feature-reviewer' })
  })
})
