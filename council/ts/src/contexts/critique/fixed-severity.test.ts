import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { fenced, finding, harness, isolatedExport, vibeStdout, type Harness } from '../../../test/critique/harness.js'
import type { FanOutResult, PrepareResult } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

/**
 * One spec-stage critic, the shipped roster's first: Mistral Large with the
 * `spec-redteam` role and the `fidelity` and `testability` lenses. In this
 * fixture the fidelity lens declares `wording-changed` critical.
 */
async function reviewed(findings: readonly unknown[]): Promise<{ h: Harness; run: PrepareResult & { status: 'ready' }; fan: FanOutResult }> {
  const h = await harness({
    assets: { 'lenses/fidelity.md': (text) => text.replace('- `wording-changed`:', '- `wording-changed` (critical):') },
    scripts: [{ exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings })) }],
  })
  open.push(h)
  await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })
  const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })
  if (run.status !== 'ready') throw new Error('not ready')
  return { fan: await h.commands.fanOut.run({ run_dir: run.run_dir }), h, run }
}

async function ledgerFindings(h: Harness): Promise<Record<string, unknown>[]> {
  const text = await readFile(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl'), 'utf8')
  return text
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line) as Record<string, unknown>)
    .filter((line) => line.type === 'finding')
}

describe('a check that declares a severity', () => {
  it("sets its finding's severity, whatever the critic wrote", async () => {
    const { fan } = await reviewed([finding({ check: 'wording-changed', severity: 'minor' })])

    expect(fan.findings[0]).toMatchObject({ declared_severity: 'critical', severity: 'critical' })
  })

  it("keeps the critic's severity on a finding whose check declares none", async () => {
    const { fan } = await reviewed([finding({ check: 'term-meaning', severity: 'minor' })])

    expect(fan.findings[0]?.severity).toBe('minor')
    expect(fan.findings[0]).not.toHaveProperty('declared_severity')
  })

  it('is recorded in the ledger beside an empty override when the verdict keeps it', async () => {
    const { h, run } = await reviewed([finding({ check: 'wording-changed', severity: 'minor' })])

    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })

    expect((await ledgerFindings(h))[0]).toMatchObject({
      declared_severity: 'critical',
      severity: 'critical',
      severity_override: null,
      severity_reason: null,
    })
  })
})

describe('a verdict that overrides a fixed severity', () => {
  it('is an error from record without a reason', async () => {
    const { h, run } = await reviewed([finding({ check: 'wording-changed' })])

    await expect(
      h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', severity: 'major', verdict: 'accepted' }] }),
    ).rejects.toMatchObject({ code: 'invalid-input', message: 'F1: a severity override needs a severity_reason' })
  })

  it('is recorded with the declared severity, the override and the reason', async () => {
    const { h, run } = await reviewed([finding({ check: 'wording-changed' })])

    await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [{ finding_id: 'F1', severity: 'major', severity_reason: 'copy-only-change', verdict: 'accepted' }],
    })

    expect((await ledgerFindings(h))[0]).toMatchObject({
      declared_severity: 'critical',
      severity: 'major',
      severity_override: 'major',
      severity_reason: 'copy-only-change',
    })
  })

  it('names the override in the report', async () => {
    const { h, run } = await reviewed([finding({ check: 'wording-changed' })])

    const result = await h.commands.record.run({
      run_dir: run.run_dir,
      verdicts: [{ finding_id: 'F1', severity: 'minor', severity_reason: 'copy-only-change', verdict: 'accepted' }],
    })

    const report = await readFile(result.report_path, 'utf8')
    expect(report).toMatch(/### F1 \[minor /)
    expect(report).toContain("Severity: minor, overriding the check's critical (copy-only-change)")
  })

  it.each([
    [{ check: 'term-meaning' }, { severity: 'minor', severity_reason: 'copy-only-change' }, 'F1: only a finding whose check fixes its severity can be overridden'],
    [{ check: 'wording-changed' }, { severity: 'critical', severity_reason: 'copy-only-change' }, 'F1: the override repeats the declared severity critical'],
    [{ check: 'wording-changed' }, { severity_reason: 'copy-only-change' }, 'F1: a severity_reason belongs to a severity override'],
  ])('is an error when it does not fit the finding (%j, %j)', async (raised, override, message) => {
    const { h, run } = await reviewed([finding(raised)])

    await expect(
      h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted', ...override }] }),
    ).rejects.toMatchObject({ code: 'invalid-input', message })
  })

  it('takes a severity_reason only as a short slug, since the ledger keeps it', async () => {
    const { h, run } = await reviewed([finding({ check: 'wording-changed' })])

    await expect(
      h.commands.record.run({
        run_dir: run.run_dir,
        verdicts: [{ finding_id: 'F1', severity: 'major', severity_reason: 'Only the copy changed.', verdict: 'accepted' }],
      }),
    ).rejects.toMatchObject({ code: 'invalid-input' })
  })
})
