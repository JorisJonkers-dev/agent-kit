import { execFile } from 'node:child_process'
import { chmod, mkdir, mkdtemp, readdir, readFile, realpath, rm, stat, symlink, utimes, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { basename, join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { FakeEnv, FakeProcess } from '../../../../test/model-gateway/harness.js'
import type { LedgerRecord } from '../domain/ledger.js'
import { CodexStatus } from './clients/status.js'
import { NodeModelGatewayFiles } from './fs/index.js'
import { NodeModelGatewayGit } from './git/index.js'
import { NodeModelGatewayLedger, type LedgerLockTimings } from './ledger/index.js'
import { NodeModelGatewayProcess } from './process/index.js'
import { NodeModelGatewayProof } from './proof/index.js'

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

  it('writes a file, creating the directories above it', async () => {
    const dir = await tempDir()
    const path = join(dir, 'runs', 'one', 'prompt.md')

    await new NodeModelGatewayFiles().writeText(path, 'read this')

    expect(await readFile(path, 'utf8')).toBe('read this')
  })

  it('makes a directory, and leaves an existing one as it is', async () => {
    const dir = await tempDir()
    const path = join(dir, 'runs', 'one')
    const files = new NodeModelGatewayFiles()

    await files.mkdir(path)
    await files.mkdir(path)

    expect(await files.exists(path)).toBe(true)
  })

  it('makes a fresh empty temporary directory named after its prefix', async () => {
    const dir = await new NodeModelGatewayFiles().tempDir('model-gateway-adapter-')
    cleanups.push(() => rm(dir, { force: true, recursive: true }))

    expect(dir).toContain('model-gateway-adapter-')
    expect(await readdir(dir)).toEqual([])
  })
})

describe('NodeModelGatewayFiles.remove', () => {
  it('removes a directory with everything in it, and leaves a path that is not there as it is', async () => {
    const dir = await tempDir()
    const folder = join(dir, 'worktree')
    await mkdir(join(folder, 'src'), { recursive: true })
    await writeFile(join(folder, 'src', 'a.ts'), 'a')
    const files = new NodeModelGatewayFiles()

    await files.remove(folder)
    await files.remove(folder)

    expect(await files.exists(folder)).toBe(false)
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

  it('finds the common directory a worktree shares with its repository', async () => {
    const root = await tempDir()
    await git(root, 'init', '-q')
    await git(root, 'commit', '-q', '--allow-empty', '-m', 'first')
    const worktree = join(root, '..', `${basename(root)}-worktree`)
    cleanups.push(() => rm(worktree, { force: true, recursive: true }))
    await git(root, 'worktree', 'add', '-q', worktree)

    expect(await repositories.commonDir(worktree)).toBe(join(await realpath(root), '.git'))
  })

  /** A repository whose HEAD holds a README. */
  async function committed(): Promise<string> {
    const root = await tempDir()
    await git(root, 'init', '-q')
    await git(root, '-c', 'user.email=test@example.com', '-c', 'user.name=Test', 'commit', '-q', '--allow-empty', '-m', 'first')
    await writeFile(join(root, 'README.md'), 'hello')
    await git(root, 'add', 'README.md')
    await git(root, '-c', 'user.email=test@example.com', '-c', 'user.name=Test', 'commit', '-q', '-m', 'readme')
    return root
  }

  it('checks HEAD out detached into an empty directory, and removes it with git, directory and all', async () => {
    const root = await committed()
    const target = await tempDir()
    await writeFile(join(root, 'README.md'), 'edited and not committed')

    await repositories.addWorktree(root, target)

    expect(await readFile(join(target, 'README.md'), 'utf8')).toBe('hello')
    await repositories.removeWorktree(root, target)
    expect(await new NodeModelGatewayFiles().exists(target)).toBe(false)
    expect(await new NodeModelGatewayGit().commonDir(root)).toBe(join(await realpath(root), '.git'))
    expect(await readdir(join(root, '.git', 'worktrees')).catch(() => [])).toEqual([])
  })

  it('removes a worktree whose directory git no longer knows, by hand, and prunes it', async () => {
    const root = await committed()
    const target = await tempDir()
    await repositories.addWorktree(root, target)
    await rm(join(target, '.git'))

    await repositories.removeWorktree(root, target)

    expect(await new NodeModelGatewayFiles().exists(target)).toBe(false)
    expect(await readdir(join(root, '.git', 'worktrees')).catch(() => [])).toEqual([])
  })

  it('fails with git\'s own message when it cannot add a worktree', async () => {
    const root = await tempDir()
    await git(root, 'init', '-q')

    await expect(repositories.addWorktree(root, await tempDir())).rejects.toThrow(/HEAD/)
  })

  it('fails when git cannot prune what it could not remove', async () => {
    const dir = await tempDir()
    const target = await tempDir()

    await expect(repositories.removeWorktree(dir, target)).rejects.toThrow(/not a git repository/)
    expect(await new NodeModelGatewayFiles().exists(target)).toBe(false)
  })

  it('fails with git\'s own message when it cannot find a common directory', async () => {
    const dir = await tempDir()

    await expect(repositories.commonDir(dir)).rejects.toThrow(/not a git repository/)
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

describe('NodeModelGatewayLedger', () => {
  /** A record the ledger can hold, with the fields the test cares about filled in. */
  function record(runId: string): LedgerRecord {
    return {
      at: '2026-01-01T00:00:00.000Z',
      client: 'vibe',
      cost_usd: 0.5,
      duration_s: 12,
      family: 'mistral',
      mode: 'read-only',
      model: 'mistral-medium-3.5',
      outcome: 'ok',
      repository: 'github.com/jorisjonkers-dev/agent-kit',
      run_id: runId,
      tokens: { cached: 0, input: 10, output: 20 },
      workflow: 'review',
    }
  }

  /** The ledger a test writes to, in a home of its own. */
  async function ledger(timings: LedgerLockTimings = {}): Promise<{ ledger: NodeModelGatewayLedger; path: string }> {
    const home = await tempDir()
    const env = new FakeEnv({ HOME: home })
    return { ledger: new NodeModelGatewayLedger(env, timings), path: join(home, '.local', 'state', 'agent-kit', 'model-gateway', 'ledger.jsonl') }
  }

  it('appends one JSON line per run, creating the state directory', async () => {
    const { ledger: writer, path } = await ledger()

    await writer.append(record('one'))
    await writer.append(record('two'))

    const lines = (await readFile(path, 'utf8')).split('\n').filter((line) => line !== '')
    expect(lines.map((line) => (JSON.parse(line) as LedgerRecord).run_id)).toEqual(['one', 'two'])
  })

  it('waits for a lock another process holds, then writes', async () => {
    const { ledger: writer, path } = await ledger({ retryMs: 5 })
    const lock = `${path}.lock`
    await mkdir(join(path, '..'), { recursive: true })
    await writeFile(lock, '')
    const release = setTimeout(() => {
      void rm(lock, { force: true })
    }, 50)
    try {
      await writer.append(record('one'))
    } finally {
      clearTimeout(release)
    }

    expect(await readFile(path, 'utf8')).toContain('"run_id":"one"')
  })

  it('breaks a lock left behind by a process that died holding it', async () => {
    const { ledger: writer, path } = await ledger({ retryMs: 5, staleMs: 20 })
    const lock = `${path}.lock`
    await mkdir(join(path, '..'), { recursive: true })
    await writeFile(lock, '')
    const old = new Date(Date.now() - 60_000)
    await utimes(lock, old, old)

    await writer.append(record('one'))

    expect(await readFile(path, 'utf8')).toContain('"run_id":"one"')
    await expect(stat(lock)).rejects.toThrow()
  })

  it('breaks, with its shipped timings, a lock left behind a few seconds ago, instead of giving up first', async () => {
    const { ledger: writer, path } = await ledger()
    const lock = `${path}.lock`
    await mkdir(join(path, '..'), { recursive: true })
    await writeFile(lock, '')
    const old = new Date(Date.now() - 6_000)
    await utimes(lock, old, old)

    await writer.append(record('one'))

    expect(await readFile(path, 'utf8')).toContain('"run_id":"one"')
  }, 15_000)

  it('gives up when another process holds the lock past its timeout', async () => {
    const { ledger: writer, path } = await ledger({ retryMs: 5, timeoutMs: 30 })
    const lock = `${path}.lock`
    await mkdir(join(path, '..'), { recursive: true })
    await writeFile(lock, '')

    await expect(writer.append(record('one'))).rejects.toThrow(/held the ledger lock/)
    await expect(readFile(path, 'utf8')).rejects.toThrow()
  })

  it('fails with the file system\'s own error when the lock cannot be created at all', async () => {
    const { ledger: writer, path } = await ledger({ retryMs: 5 })
    const dir = join(path, '..')
    await mkdir(dir, { recursive: true })
    await chmod(dir, 0o500)
    cleanups.push(() => chmod(dir, 0o700))

    await expect(writer.append(record('one'))).rejects.toThrow(/EACCES|permission denied/i)
  })

  it('treats a lock that vanished before it could be read as held, and waits for it', async () => {
    const { ledger: writer, path } = await ledger({ retryMs: 5, timeoutMs: 30 })
    await mkdir(join(path, '..'), { recursive: true })
    await symlink(join(path, '..', 'gone'), `${path}.lock`)

    await expect(writer.append(record('one'))).rejects.toThrow(/held the ledger lock/)
  })

  it('never interleaves two concurrent appends', async () => {
    const { ledger: writer, path } = await ledger({ retryMs: 1 })
    const ids = Array.from({ length: 8 }, (_, index) => `run-${String(index)}`)

    await Promise.all(ids.map((id) => writer.append(record(id))))

    const lines = (await readFile(path, 'utf8')).split('\n').filter((line) => line !== '')
    expect(lines).toHaveLength(ids.length)
    expect(lines.map((line) => (JSON.parse(line) as LedgerRecord).run_id).sort()).toEqual(ids)
  })

  it('reads every record back, oldest first', async () => {
    const { ledger: writer } = await ledger()
    await writer.append(record('one'))
    await writer.append(record('two'))

    expect((await writer.read()).map((entry) => entry.run_id)).toEqual(['one', 'two'])
  })

  it('reads no record from a ledger that is not there yet', async () => {
    const { ledger: reader } = await ledger()

    expect(await reader.read()).toEqual([])
  })

  it('skips a line a writer left half-written', async () => {
    const { ledger: reader, path } = await ledger()
    await mkdir(join(path, '..'), { recursive: true })
    await writeFile(path, `${JSON.stringify(record('one'))}\n{"run_id":`)

    expect((await reader.read()).map((entry) => entry.run_id)).toEqual(['one'])
  })

  it('fails with the file system’s own error when the ledger cannot be read at all', async () => {
    const { ledger: reader, path } = await ledger()
    await mkdir(path, { recursive: true })

    await expect(reader.read()).rejects.toThrow(/EISDIR|illegal operation/i)
  })
})

describe('the clients’ own status commands', () => {
  it('gives a status command the time limit it is handed, so a hung client cannot hang usage', async () => {
    const process = new FakeProcess(['codex'], { codex: { exitCode: 0, stderr: '', stdout: 'Logged in using ChatGPT\n' } })

    expect(await new CodexStatus(process, 50).read()).toEqual({
      command: 'codex login status',
      detail: 'Logged in using ChatGPT',
      signed_in: 'yes',
    })
    expect(process.ran[0]?.timeoutMs).toBe(50)
  })

  it('reads a login status that says it is not logged in as signed out', async () => {
    const process = new FakeProcess(['codex'], { codex: { exitCode: 1, stderr: 'Not logged in\n', stdout: '' } })

    expect(await new CodexStatus(process).read()).toEqual({
      command: 'codex login status',
      detail: 'Not logged in',
      signed_in: 'no',
    })
  })

  it('reads a login status that failed for another reason as unknown, not signed out', async () => {
    const process = new FakeProcess(['codex'], {
      codex: { exitCode: 2, stderr: "error: unrecognized subcommand 'login'\n", stdout: '' },
    })

    expect(await new CodexStatus(process).read()).toEqual({
      command: 'codex login status',
      detail: "could not be read: exited 2: error: unrecognized subcommand 'login'",
      signed_in: 'unknown',
    })
  })
})

describe('NodeModelGatewayProof', () => {
  it('reads the shipped routing, this machine\'s override and setup\'s results', async () => {
    const home = await tempDir()
    const routing = join(home, 'routing.json')
    await writeFile(routing, '{"clients":{}}')
    await mkdir(join(home, '.config', 'agent-kit'), { recursive: true })
    await writeFile(join(home, '.config', 'agent-kit', 'critique-routing.json'), '{"vibe":{}}')
    await mkdir(join(home, '.local', 'share', 'agent-kit', 'critique'), { recursive: true })
    await writeFile(join(home, '.local', 'share', 'agent-kit', 'critique', 'machine-setup.json'), '{"checks":{}}')
    const proof = new NodeModelGatewayProof({ env: new FakeEnv({ HOME: home }), files: new NodeModelGatewayFiles(), routingPath: routing })

    expect(await proof.read()).toEqual({
      routing: '{"clients":{}}',
      routingOverride: '{"vibe":{}}',
      setupResults: '{"checks":{}}',
    })
  })

  it('reports a file that is not there as undefined', async () => {
    const home = await tempDir()
    const proof = new NodeModelGatewayProof({
      env: new FakeEnv({ HOME: home }),
      files: new NodeModelGatewayFiles(),
      routingPath: join(home, 'routing.json'),
    })

    expect(await proof.read()).toEqual({ routing: undefined, routingOverride: undefined, setupResults: undefined })
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
