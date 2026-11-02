import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { ASSETS_ROOT, fenced, finding, harness, isolatedExport, vibeStdout, type Harness } from '../../../test/critique/harness.js'
import { CATEGORIES } from './domain/findings.js'
import type { FanOutResult, PrepareResult } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

/**
 * One spec-stage critic, the shipped roster's first: Mistral Large with the
 * `spec-redteam` role and the `fidelity` and `testability` lenses.
 */
async function reviewed(findings: readonly unknown[]): Promise<{ h: Harness; run: PrepareResult; result: FanOutResult }> {
  const h = await harness({ scripts: [{ exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings })) }] })
  open.push(h)
  await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })
  const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })
  if (run.status !== 'ready') throw new Error('not ready')
  return { h, result: await h.commands.fanOut.run({ run_dir: run.run_dir }), run }
}

async function fanOut(findings: readonly unknown[]): Promise<FanOutResult> {
  return (await reviewed(findings)).result
}

function asset(path: string): Promise<string> {
  return readFile(join(ASSETS_ROOT, path), 'utf8')
}

async function schemaCategories(): Promise<readonly string[]> {
  const schema = JSON.parse(await asset('schemas/finding.schema.json')) as {
    $defs: { finding: { properties: { category: { enum: string[] } } } }
  }
  return schema.$defs.finding.properties.category.enum
}

/** The backticked names in the output contract's `category` bullet, which runs until the next field's bullet. */
async function contractCategories(): Promise<readonly string[]> {
  const contract = await asset('prompts/output-contract.md')
  const bullet = /^- `category`:([\s\S]*?)^- `/m.exec(contract)?.[1] ?? ''
  return [...bullet.matchAll(/`([^`]+)`/g)].map((match) => match[1] ?? '')
}

describe('the finding categories', () => {
  it('are the same in the runner, the finding schema and the output contract', async () => {
    const runner = [...CATEGORIES].sort()

    expect([...(await schemaCategories())].sort()).toEqual(runner)
    expect([...(await contractCategories())].sort()).toEqual(runner)
  })

  it('include the three that the ADR and architecture stages need', () => {
    expect(CATEGORIES).toEqual(expect.arrayContaining(['drift', 'stale', 'undocumented-decision']))
  })
})

describe("a finding's check", () => {
  it('is kept when the role of its critic declares it', async () => {
    const result = await fanOut([finding({ check: 'contradiction' })])

    expect(result.findings[0]).toMatchObject({ check: 'contradiction' })
    expect(result.findings[0]?.pre_verdict).toBeUndefined()
  })

  it("may name a check of one of its critic's lenses", async () => {
    const result = await fanOut([finding({ check: 'term-meaning' })])

    expect(result.findings[0]?.pre_verdict).toBeUndefined()
  })

  // A key set to undefined is left out of the critic's JSON reply.
  it.each([undefined, '', '  '])('is a hint when the finding names none (%j)', async (check) => {
    const result = await fanOut([finding({ check })])

    expect(result.findings[0]?.pre_verdict).toEqual({ reason: 'names no check', reason_code: 'no-check', verdict: 'hint' })
  })

  it("is a hint when no lens or role in its critic's prompt declares it", async () => {
    // `data-egress` is a check of the security lens, which this critic was not given.
    const result = await fanOut([finding({ check: 'data-egress' })])

    expect(result.findings[0]?.pre_verdict).toEqual({
      reason: "names the check data-egress, which no lens or role in its critic's prompt declares",
      reason_code: 'unknown-check',
      verdict: 'hint',
    })
  })

  it('is written to the ledger line of its finding, and a missing or free-text one as null', async () => {
    const unnamed = finding({ check: undefined, id: 'F2', location: 'User story 1' })
    // The ledger keeps identifiers only, so a check that is not an id stays in the report.
    const prose = finding({ check: 'Every widget must be frobnicated', id: 'F3', location: 'User story 2' })
    const { h, run } = await reviewed([finding({ check: 'wording-changed' }), unnamed, prose])

    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })

    const ledger = await readFile(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl'), 'utf8')
    const findings = ledger
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line) as Record<string, unknown>)
      .filter((line) => line.type === 'finding')
    expect(findings.map((line) => [line.finding_id, line.check])).toEqual([
      ['F1', 'wording-changed'],
      ['F2', null],
      ['F3', null],
    ])
  })

  it('gives way to the stage evidence rule, which is checked first', async () => {
    const result = await fanOut([finding({ check: 'data-egress', evidence: [] })])

    expect(result.findings[0]?.pre_verdict).toMatchObject({ reason_code: 'no-evidence', verdict: 'hint' })
  })
})
