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
    })
  })
})
