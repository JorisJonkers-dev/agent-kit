import { mkdtemp, realpath, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { NodeProcessAdapter } from './index.js'

const tempRoots: string[] = []

afterEach(async () => {
  await Promise.all(tempRoots.splice(0).map((root) => rm(root, { force: true, recursive: true })))
})

async function tempRoot(prefix: string): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), prefix))
  tempRoots.push(root)
  return root
}

describe('NodeProcessAdapter', () => {
  it('executes commands asynchronously with cwd and environment overrides', async () => {
    const root = await tempRoot('node-process-adapter-')
    const resolvedRoot = await realpath(root)
    const adapter = new NodeProcessAdapter()

    await expect(
      adapter.exec({
        args: ['-e', 'process.stdout.write(`${process.cwd()} ${process.env.NODE_PROCESS_ADAPTER_TEST ?? ""}`)'],
        command: process.execPath,
        cwd: root,
        env: { NODE_PROCESS_ADAPTER_TEST: 'ok' },
      }),
    ).resolves.toEqual({
      exitCode: 0,
      stderr: '',
      stdout: `${resolvedRoot} ok`,
    })
  })

  it('captures stderr output from commands', async () => {
    const adapter = new NodeProcessAdapter()

    await expect(
      adapter.exec({
        args: ['-e', 'process.stderr.write("problem\\n")'],
        command: process.execPath,
      }),
    ).resolves.toEqual({
      exitCode: 0,
      stderr: 'problem\n',
      stdout: '',
    })
  })

  it('terminates commands that exceed their timeout', async () => {
    const adapter = new NodeProcessAdapter()

    const result = await adapter.exec({
      args: ['-e', 'setTimeout(() => undefined, 1000)'],
      command: process.execPath,
      timeoutMs: 1,
    })

    expect(result).toMatchObject({
      exitCode: 124,
      stderr: '',
      stdout: '',
      timedOut: true,
    })
  })

  it('kills a command that ignores the termination signal once the grace period ends', async () => {
    const adapter = new NodeProcessAdapter({ killGraceMs: 50 })

    const result = await adapter.exec({
      args: ['-e', "process.on('SIGTERM', () => undefined); setTimeout(() => undefined, 5000)"],
      command: process.execPath,
      timeoutMs: 200,
    })

    expect(result).toMatchObject({ exitCode: 124, timedOut: true })
  })

  it('does not call a command that finishes in time timed out', async () => {
    const adapter = new NodeProcessAdapter()

    const result = await adapter.exec({ args: ['-e', ''], command: process.execPath, timeoutMs: 5000 })

    expect(result.timedOut).toBeUndefined()
  })
})
