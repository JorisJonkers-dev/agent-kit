import { readFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  fenced,
  finding,
  harness,
  isolatedExport,
  searchExport,
  SPEC_CANARY,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../test/critique/harness.js'
import { runCritiqueCli, type CritiqueCliRuntime } from './index.js'

/**
 * A Gold Band template node runs one runner command through the gate shim, so
 * the runner is the only thing that can hold a critic to its rules. These
 * tests drive the commands a template's nodes run, each with `--strict` as the
 * node does, and read the exit status the gate would read.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

function cli(h: Harness, stdin = ''): CritiqueCliRuntime {
  return { commands: h.commands, readFile: (path) => readFile(path, 'utf8'), readStdin: () => Promise.resolve(stdin) }
}

async function node(h: Harness, command: string, input: Record<string, unknown>): Promise<{ exitCode: number; result: Record<string, unknown> }> {
  const out = await runCritiqueCli([command, '--json', JSON.stringify(input), '--strict'], cli(h))
  return { exitCode: out.exitCode, result: JSON.parse(out.stdout) as Record<string, unknown> }
}

async function templateRun(options: Parameters<typeof harness>[0] = {}): Promise<{ h: Harness; runDir: string }> {
  const h = await harness(options)
  open.push(h)
  await h.commands.allow.run({ families: ['mistral'], repo: h.repo })
  const prepared = await node(h, 'prepare', { orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })
  expect(prepared.exitCode).toBe(0)
  return { h, runDir: String(prepared.result.run_dir) }
}

const critic = (overrides: Partial<VibeScript> = {}): VibeScript => ({
  exitCode: 0,
  exportJson: isolatedExport(),
  stdout: vibeStdout(fenced({ findings: [finding()] })),
  ...overrides,
})

describe('the runner behind a Gold Band template node', () => {
  it('refuses a critic that writes outside its packet, through the read-only tripwire', async () => {
    const { h, runDir } = await templateRun()
    h.process.queue(critic({ effect: ({ cwd }) => writeFile(join(cwd, 'notes.md'), 'scratch\n') }))

    const fan = await node(h, 'fan-out', { run_dir: runDir })

    expect(fan.exitCode).toBe(0)
    expect(fan.result).toMatchObject({ critics: [{ status: 'violated-readonly' }], findings: [] })
  })

  it('answers a search outside the packet with nothing', async () => {
    const { h, runDir } = await templateRun({ search: true })
    const answers: string[] = []
    h.process.queue(
      critic({
        effect: async ({ search }) => {
          answers.push(await search({ arguments: { path: '..', pattern: 'canary' }, tool: 'grep' }))
          answers.push(await search({ arguments: { path: 'artifact/spec.md', pattern: 'canary' }, tool: 'grep' }))
        },
        exportJson: searchExport(),
      }),
    )

    const fan = await node(h, 'fan-out', { run_dir: runDir })

    expect(fan.exitCode).toBe(0)
    expect(answers[0]).toBe('Nothing: that path is outside the packet, or is not in it.')
    expect(answers[1]).toContain(SPEC_CANARY)
  })

  it('stops a critic past its time limit', async () => {
    const { h, runDir } = await templateRun()
    h.process.queue({ exitCode: 0, stdout: '', timedOut: true })

    const fan = await node(h, 'fan-out', { run_dir: runDir })

    expect(fan.exitCode).toBe(0)
    expect(fan.result).toMatchObject({ critics: [{ failure: 'timeout', status: 'failed' }] })
    expect(h.process.commands[0]?.timeoutMs).toBeGreaterThan(0)
  })

  it('refuses a repository with no family allowlist entry, so the gate node fails', async () => {
    const h = await harness()
    open.push(h)

    const prepared = await node(h, 'prepare', { orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })

    expect(prepared.exitCode).toBe(4)
    expect(prepared.result).toMatchObject({ reason: 'not-opted-in', status: 'refused' })
  })
})
