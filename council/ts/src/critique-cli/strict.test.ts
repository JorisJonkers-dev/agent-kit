import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { harness, type Harness } from '../../test/critique/harness.js'
import { CRITIQUE_USAGE, REFUSED_EXIT_CODE, runCritiqueCli, type CritiqueCliRuntime } from './index.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

function cli(h: Harness, stdin = ''): CritiqueCliRuntime {
  return { commands: h.commands, readFile: (path) => readFile(path, 'utf8'), readStdin: () => Promise.resolve(stdin) }
}

const unopened = (h: Harness): string => JSON.stringify({ orchestrator_family: 'anthropic', repo: h.repo, spec: 'x', stage: 'spec' })

async function opened(): Promise<Harness> {
  const h = await harness()
  cleanups.push(h.cleanup)
  return h
}

describe('--strict, the exit status a Gold Band gate node reads', () => {
  it('exits 4 on a refusal and still prints the refusal', async () => {
    const h = await opened()

    const result = await runCritiqueCli(['prepare', '--json', unopened(h), '--strict'], cli(h))

    expect(REFUSED_EXIT_CODE).toBe(4)
    expect(result.exitCode).toBe(4)
    expect(JSON.parse(result.stdout)).toMatchObject({ reason: 'not-opted-in', status: 'refused' })
  })

  it('reads its input from stdin when the command has no input flag', async () => {
    const h = await opened()

    const result = await runCritiqueCli(['prepare', '--strict'], cli(h, unopened(h)))

    expect(result.exitCode).toBe(4)
  })

  it('leaves a refusal at exit 0 without it', async () => {
    const h = await opened()

    expect((await runCritiqueCli(['prepare', '--json', unopened(h)], cli(h))).exitCode).toBe(0)
  })

  it('exits 0 for a result that is not a refusal', async () => {
    const h = await opened()

    const result = await runCritiqueCli(['check-tickets', '--json', JSON.stringify({ repo: h.repo, spec: '.scratch/widgets/spec.md' }), '--strict'], cli(h))

    expect(result.exitCode).toBe(0)
  })

  it('keeps exit 1 for an error and exit 3 for a failed check', async () => {
    const h = await opened()
    await h.writeText(join(h.repo, '.scratch', 'widgets', 'issues', '01-schema.md'), '# 01: Schema\n\n**Blocked by:** 01\n')

    const error = await runCritiqueCli(['prepare', '--json', 'not json', '--strict'], cli(h))
    const failed = await runCritiqueCli(['check-tickets', '--json', JSON.stringify({ repo: h.repo, spec: '.scratch/widgets/spec.md' }), '--strict'], cli(h))

    expect(error.exitCode).toBe(1)
    expect(failed.exitCode).toBe(3)
  })

  it.each([
    ['alone', ['--strict']],
    ['twice', ['prepare', '--strict', '--strict']],
    ['before the input flag', ['prepare', '--strict', '--json', '{}']],
    ['with an unknown command', ['nothing', '--strict']],
  ])('is a usage error %s', async (_name, argv) => {
    const h = await opened()

    expect(await runCritiqueCli(argv, cli(h))).toEqual({ exitCode: 2, stderr: CRITIQUE_USAGE, stdout: '' })
  })

  it('is not a hook argument', async () => {
    const h = await opened()

    const result = await runCritiqueCli(['hook', 'critique-pr-gate', '--strict'], cli(h))

    expect(result).toEqual({ exitCode: 1, stderr: CRITIQUE_USAGE, stdout: '' })
  })
})
