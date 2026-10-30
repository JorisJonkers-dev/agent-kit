import { execFileSync } from 'node:child_process'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  ASSETS_ROOT,
  fenced,
  finding,
  harness,
  isolatedExport,
  vibeStdout,
  type Harness,
} from '../../test/critique/harness.js'
import { CRITIQUE_USAGE, defaultCritiqueRuntime, runCritiqueCli, type CritiqueCliRuntime } from './index.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

function cli(h: Harness, stdin = ''): CritiqueCliRuntime {
  return { commands: h.commands, readFile: (path) => readFile(path, 'utf8'), readStdin: () => Promise.resolve(stdin) }
}

async function run(runtime: CritiqueCliRuntime, ...argv: string[]): Promise<Record<string, unknown>> {
  const result = await runCritiqueCli(argv, runtime)
  expect(result.exitCode).toBe(0)
  return JSON.parse(result.stdout) as Record<string, unknown>
}

describe('critique runner command line', () => {
  it('drives allow, prepare, fan-out and record with JSON in and JSON out', async () => {
    const h = await harness({
      scripts: [{ exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings: [finding()] })) }],
    })
    cleanups.push(h.cleanup)
    const runtime = cli(h)

    expect(await run(runtime, 'allow', '--json', JSON.stringify({ families: ['mistral'], repo: h.repo }))).toMatchObject({
      status: 'allowed',
    })
    const inputFile = join(h.home, 'prepare.json')
    await h.writeText(inputFile, JSON.stringify({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' }))
    const prepared = await run(runtime, 'prepare', '--input', inputFile)
    expect(prepared).toMatchObject({ status: 'ready' })
    const fan = await run(cli(h, JSON.stringify({ run_dir: prepared.run_dir })), 'fan-out')
    expect(fan).toMatchObject({ findings: [{ id: 'F1' }] })
    const recorded = await run(
      runtime,
      'record',
      '--json',
      JSON.stringify({ run_dir: prepared.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] }),
    )
    expect(recorded).toMatchObject({ counts: { accepted: 1 }, status: 'recorded' })
  })

  it('prints a refusal as a normal result', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)

    expect(
      await run(cli(h), 'prepare', '--json', JSON.stringify({ orchestrator_family: 'anthropic', repo: h.repo, spec: 'x', stage: 'spec' })),
    ).toMatchObject({ reason: 'not-opted-in', status: 'refused' })
  })

  it('reports input that is not JSON as an invalid-input error with exit code 1', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)

    const result = await runCritiqueCli(['allow', '--json', '{not json'], cli(h))

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toMatchObject({ code: 'invalid-input', status: 'error' })
    expect(result.stderr).toMatch(/^critique allow: /)
  })

  it('reports the code of the error a command raises', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)
    await h.commands.allow({ families: ['mistral'], repo: h.repo })

    const result = await runCritiqueCli(
      ['prepare', '--json', JSON.stringify({ orchestrator_family: 'anthropic', repo: h.repo, spec: '../x.md', stage: 'spec' })],
      cli(h),
    )

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toEqual({ code: 'outside-repository', message: '../x.md is outside the repository', status: 'error' })
  })

  it('reports an error the runner did not expect with the code unexpected', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)

    const result = await runCritiqueCli(['allow', '--input', join(h.home, 'missing.json')], cli(h))

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toMatchObject({ code: 'unexpected', status: 'error' })
  })

  it.each([[[]], [['explode']], [['prepare', '--input']], [['prepare', '--yaml', 'x']], [['prepare', '--json', '{}', 'extra']]])(
    'prints usage for %j',
    async (argv) => {
      const h = await harness()
      cleanups.push(h.cleanup)

      expect(await runCritiqueCli(argv, cli(h))).toEqual({ exitCode: 2, stderr: CRITIQUE_USAGE, stdout: '' })
    },
  )

  it('prints usage on --help', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)

    expect(await runCritiqueCli(['--help'], cli(h))).toEqual({ exitCode: 0, stderr: '', stdout: CRITIQUE_USAGE })
  })
})

describe('the real adapters', () => {
  it('resolve a real repository, keep the allowlist in HOME and freeze the packet in the git common directory', async () => {
    const base = await mkdtemp(join(tmpdir(), 'critique-cli-'))
    cleanups.push(() => rm(base, { force: true, recursive: true }))
    const repo = join(base, 'repo')
    const home = join(base, 'home')
    await mkdir(join(repo, '.scratch', 'widgets'), { recursive: true })
    await writeFile(join(repo, '.scratch', 'widgets', 'spec.md'), '# Widgets\n')
    await writeFile(join(repo, 'CONTEXT.md'), '# Words\n')
    execFileSync('git', ['init', '-q', repo])
    execFileSync('git', ['-C', repo, 'remote', 'add', 'origin', 'https://github.com/example/widgets.git'])
    const runtime = defaultCritiqueRuntime(ASSETS_ROOT, () => Promise.resolve(''), { HOME: home, PATH: process.env.PATH })

    await writeFile(join(base, 'allow.json'), JSON.stringify({ families: ['mistral'], repo }))
    await run(runtime, 'allow', '--input', join(base, 'allow.json'))
    const prepared = await run(
      runtime,
      'prepare',
      '--json',
      JSON.stringify({ orchestrator_family: 'anthropic', repo, spec: '.scratch/widgets/spec.md', stage: 'spec' }),
    )

    expect(JSON.parse(await readFile(join(home, '.config', 'agent-kit', 'critique-allowlist.json'), 'utf8'))).toMatchObject({
      repositories: { 'github.com/example/widgets': { families: ['mistral'] } },
    })
    expect(String(prepared.packet_dir)).toContain(join('.git', 'critique'))
    expect(await readFile(join(String(prepared.packet_dir), 'artifact', 'spec.md'), 'utf8')).toBe('# Widgets\n')
  })

  it('defaults to the process environment', () => {
    expect(defaultCritiqueRuntime(ASSETS_ROOT, () => Promise.resolve('')).commands).toBeDefined()
  })
})
