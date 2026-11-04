import { execFile } from 'node:child_process'
import { chmod, mkdir, mkdtemp, realpath, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { NodeModelGatewayFiles } from './fs/index.js'
import { NodeModelGatewayGit } from './git/index.js'
import { NodeModelGatewayProcess } from './process/index.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

async function tempDir(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'model-gateway-adapter-'))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  return dir
}

/** Runs git in a temporary repository, the way the adapter does. */
function git(dir: string, ...args: string[]): Promise<void> {
  return new Promise((resolve, reject) => {
    execFile('git', ['-C', dir, ...args], (error) => {
      if (error === null) resolve()
      else reject(new Error(error.message))
    })
  })
}

describe('NodeModelGatewayFiles', () => {
  it('reads a file as text', async () => {
    const dir = await tempDir()
    const path = join(dir, 'defaults.json')
    await writeFile(path, '{"version":1}\n')

    expect(await new NodeModelGatewayFiles().readText(path)).toBe('{"version":1}\n')
  })

  it('says whether a file exists', async () => {
    const dir = await tempDir()
    const path = join(dir, 'model-gateway.json')
    const files = new NodeModelGatewayFiles()

    expect(await files.exists(path)).toBe(false)
    await writeFile(path, '{}')
    expect(await files.exists(path)).toBe(true)
  })

  it('throws, rather than saying no, when it may not look', async () => {
    const dir = await tempDir()
    const locked = join(dir, 'locked')
    await mkdir(locked)
    await writeFile(join(locked, 'model-gateway.json'), '{}')
    await chmod(locked, 0o000)
    cleanups.unshift(() => chmod(locked, 0o755))

    await expect(new NodeModelGatewayFiles().exists(join(locked, 'model-gateway.json'))).rejects.toThrow()
  })
})

describe('NodeModelGatewayProcess', () => {
  it('finds a program that is on PATH', async () => {
    expect(await new NodeModelGatewayProcess().which('node')).toBe(true)
  })

  it('does not find a program that is not on PATH', async () => {
    expect(await new NodeModelGatewayProcess().which('agent-kit-no-such-binary')).toBe(false)
  })

  it('runs a listing command and reads what it printed', async () => {
    const result = await new NodeModelGatewayProcess().exec({
      args: ['-e', 'process.stdout.write("gemini-4-pro\\tGemini 4 Pro")'],
      command: process.execPath,
    })

    expect(result).toMatchObject({ exitCode: 0, stdout: 'gemini-4-pro\tGemini 4 Pro' })
  })

  it('reports the exit code of a listing command that fails', async () => {
    const result = await new NodeModelGatewayProcess().exec({ args: ['-e', 'process.exit(3)'], command: process.execPath })

    expect(result.exitCode).toBe(3)
  })

  it('looks the program up on the PATH it is given, without running any other program', async () => {
    const dir = await tempDir()
    const bin = join(dir, 'bin')
    await mkdir(bin)
    await writeFile(join(bin, 'runnable'), '#!/bin/sh\n')
    await chmod(join(bin, 'runnable'), 0o755)
    await writeFile(join(bin, 'plain'), 'not a program\n')
    const lookup = new NodeModelGatewayProcess(`${join(dir, 'missing')}:${bin}`)

    expect(await lookup.which('runnable')).toBe(true)
    expect(await lookup.which('plain')).toBe(false)
    expect(await new NodeModelGatewayProcess('').which('runnable')).toBe(false)
  })
})

describe('NodeModelGatewayGit', () => {
  const repositories = new NodeModelGatewayGit()

  it("finds a directory's root and its remotes", async () => {
    const root = await tempDir()
    await git(root, 'init', '-q')
    await git(root, 'remote', 'add', 'origin', 'git@github.com:Acme/Widgets.git')
    await git(root, 'remote', 'add', 'upstream', 'https://example.com/other/thing.git')
    await mkdir(join(root, 'docs'), { recursive: true })

    expect(await repositories.root(join(root, 'docs'))).toBe(await realpath(root))
    expect(await repositories.remotes(root)).toEqual([
      { name: 'origin', url: 'git@github.com:Acme/Widgets.git' },
      { name: 'upstream', url: 'https://example.com/other/thing.git' },
    ])
  })

  it('reports no remote for a repository without one', async () => {
    const root = await tempDir()
    await git(root, 'init', '-q')

    expect(await repositories.remotes(root)).toEqual([])
  })

  it('says a directory outside any repository has no root', async () => {
    const dir = await tempDir()

    expect(await repositories.root(dir)).toBeUndefined()
  })

  it('fails loudly when git cannot be started at all', async () => {
    const dir = await tempDir()

    await expect(new NodeModelGatewayGit('').root(dir)).rejects.toThrow(/cannot run git/)
  })

  it('fails with git\'s own message for a path it cannot read, rather than calling it no repository', async () => {
    const dir = await tempDir()
    const missing = join(dir, 'nowhere')

    await expect(repositories.root(missing)).rejects.toThrow(/cannot change to .*nowhere/)
  })

  it('fails when git does not answer within its timeout', async () => {
    const dir = await tempDir()
    // A git that never answers: it waits on a stdin nobody writes to, so the
    // test needs no command from the PATH the adapter hands it.
    const bin = await fakeGit(dir, '#!/bin/sh\nread line\n')

    await expect(new NodeModelGatewayGit(bin, 50).root(dir)).rejects.toThrow(/did not answer within 50 ms/)
  })

  it('says a path has no root when git answers with nothing', async () => {
    const dir = await tempDir()
    const bin = await fakeGit(dir, '#!/bin/sh\nexit 0\n')

    expect(await new NodeModelGatewayGit(bin).root(dir)).toBeUndefined()
  })

  it('falls back to git\'s exit code when git says nothing at all', async () => {
    const dir = await tempDir()
    const bin = await fakeGit(dir, '#!/bin/sh\nexit 2\n')
    const silent = new NodeModelGatewayGit(bin)

    await expect(silent.root(dir)).rejects.toThrow(/git exited 2/)
    await expect(silent.remotes(dir)).rejects.toThrow(/git exited 2/)
  })

  it('looks git up on the PATH it inherits when it is given none', async () => {
    const dir = await tempDir()
    const path = process.env.PATH
    delete process.env.PATH
    try {
      await expect(new NodeModelGatewayGit().root(dir)).rejects.toThrow(/cannot run git/)
    } finally {
      process.env.PATH = path
    }
  })
})

/** A git of the test's own making, on a PATH of its own. */
async function fakeGit(dir: string, script: string): Promise<string> {
  const bin = join(dir, 'bin')
  await mkdir(bin)
  await writeFile(join(bin, 'git'), script)
  await chmod(join(bin, 'git'), 0o755)
  return bin
}
