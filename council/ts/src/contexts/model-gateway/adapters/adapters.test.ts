import { execFile } from 'node:child_process'
import { createHash } from 'node:crypto'
import { chmod, mkdir, mkdtemp, readdir, readFile, realpath, rm, stat, symlink, utimes, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { basename, dirname, join } from 'node:path'

import { afterEach, describe, expect, it, vi } from 'vitest'

import { FakeEnv, FakeProcess } from '../../../../test/model-gateway/harness.js'
import type { KeptWorktree } from '../application/ports.js'
import { newGrant, type Grant } from '../domain/grants.js'
import type { LedgerRecord } from '../domain/ledger.js'
import { endWorkflow, startWorkflow } from '../domain/workflows.js'
import { CodexStatus } from './clients/status.js'
import { NodeModelGatewayFiles, writeFileAtomic } from './fs/index.js'
import { NodeModelGatewayGit } from './git/index.js'
import { NodeModelGatewayGrants, type GrantLockTimings } from './grants/index.js'
import { NodeModelGatewayKept } from './kept/index.js'
import { NodeModelGatewayLedger, type LedgerLockTimings } from './ledger/index.js'
import { NodeModelGatewayProcess } from './process/index.js'
import { NodeModelGatewayProof } from './proof/index.js'
import { NodeModelGatewayWorkflows, type WorkflowLockTimings } from './workflows/index.js'

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

  it('says when a file was last modified, and nothing when it is not there', async () => {
    const dir = await tempDir()
    const path = join(dir, 'model-gateway.json')
    const files = new NodeModelGatewayFiles()

    expect(await files.modifiedMs(path)).toBeUndefined()

    await writeFile(path, '{}')
    expect(await files.modifiedMs(path)).toBe((await stat(path)).mtimeMs)
  })

  it('stamps a file with what makes it itself — its real path, change time, size, inode and content — and nothing when it is not there', async () => {
    const dir = await tempDir()
    const path = join(dir, 'model-gateway.json')
    const linked = join(dir, 'linked.json')
    await writeFile(path, 'the file itself')
    await symlink(path, linked)
    const files = new NodeModelGatewayFiles()

    const stamp = await files.stamp(linked)
    const of = await stat(path)

    expect(stamp).toEqual({
      ctimeMs: of.ctimeMs,
      ino: of.ino,
      realpath: path,
      sha256: createHash('sha256').update('the file itself').digest('hex'),
      size: of.size,
    })
    expect(await files.stamp(join(dir, 'missing.toml'))).toBeUndefined()
  })

  it('writes a file only its owner can read, in a directory only its owner can look in', async () => {
    const dir = await tempDir()
    const path = join(dir, 'model-gateway', 'vibe-available.json')
    const files = new NodeModelGatewayFiles()

    await files.writeTextPrivate(path, '{}')

    expect(await readFile(path, 'utf8')).toBe('{}')
    expect((await stat(path)).mode & 0o777).toBe(0o600)
    expect((await stat(dirname(path))).mode & 0o777).toBe(0o700)
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
    await git(root, '-c', 'user.email=test@example.com', '-c', 'user.name=Test', 'commit', '-q', '--allow-empty', '-m', 'first')
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

  it('names the git directory of a checkout: the repository’s own, and the one a linked worktree keeps under it', async () => {
    const root = await committed()
    const worktree = join(root, '..', `${basename(root)}-worktree`)
    cleanups.push(() => rm(worktree, { force: true, recursive: true }))
    await git(root, 'worktree', 'add', '-q', worktree)

    expect(await repositories.gitDir(root)).toBe(join(await realpath(root), '.git'))
    expect(await repositories.gitDir(worktree)).toBe(join(await realpath(root), '.git', 'worktrees', basename(worktree)))
  })

  it('fails with git’s own message when it cannot name a git directory', async () => {
    const dir = await tempDir()

    await expect(repositories.gitDir(dir)).rejects.toThrow(/not a git repository/)
  })

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

  it('creates a worktree on a new branch off a base, stages everything in it, and lists the staged files', async () => {
    const root = await committed()
    const base = await tempDir()
    const target = join(base, 'worktrees', 'run-1')
    await repositories.createWorktree(root, target, 'gateway/run-1', 'HEAD')

    expect(await readFile(join(target, 'README.md'), 'utf8')).toBe('hello')
    await mkdir(join(target, 'src'), { recursive: true })
    await writeFile(join(target, 'src', 'new file.ts'), 'x')
    await writeFile(join(target, 'README.md'), 'changed')
    expect(await repositories.stagedFiles(target)).toEqual([])
    await repositories.stageAll(target)

    expect([...(await repositories.stagedFiles(target))].sort()).toEqual(['README.md', 'src/new file.ts'])
    expect(await readFile(join(root, 'README.md'), 'utf8')).toBe('hello')
    await repositories.removeWorktree(root, target)
    await repositories.deleteBranch(root, 'gateway/run-1')
    await expect(repositories.deleteBranch(root, 'gateway/run-1')).rejects.toThrow(/gateway\/run-1/)
  })

  it('lists a change the delegate committed on the worktree’s branch beside the staged one, when it is given the base', async () => {
    const root = await committed()
    const target = join(await tempDir(), 'worktrees', 'run-1')
    await repositories.createWorktree(root, target, 'gateway/run-1', 'HEAD')
    await writeFile(join(target, 'committed.ts'), 'x')
    await git(target, 'add', 'committed.ts')
    await git(target, '-c', 'user.email=test@example.com', '-c', 'user.name=Test', 'commit', '-q', '-m', 'the delegate’s own commit')
    await writeFile(join(target, 'staged.ts'), 'y')
    await repositories.stageAll(target)

    expect(await repositories.stagedFiles(target, 'HEAD~1')).toEqual(['committed.ts', 'staged.ts'])
    expect(await repositories.stagedFiles(target)).toEqual(['staged.ts'])
  })

  it('fails with git’s own message when the base it is given is no ref', async () => {
    const root = await committed()
    const target = join(await tempDir(), 'worktrees', 'run-1')
    await repositories.createWorktree(root, target, 'gateway/run-1', 'HEAD')

    await expect(repositories.stagedFiles(target, 'no-such-ref')).rejects.toThrow(/no-such-ref/)
  })

  it('fails with git\'s own message when the base is unknown, the branch exists, or there is nothing to stage in', async () => {
    const root = await committed()
    const target = join(await tempDir(), 'wt')

    await expect(repositories.createWorktree(root, target, 'gateway/a', 'no-such-ref')).rejects.toThrow(/no-such-ref/)
    await repositories.createWorktree(root, target, 'gateway/a', 'HEAD')
    await expect(repositories.createWorktree(root, join(await tempDir(), 'wt'), 'gateway/a', 'HEAD')).rejects.toThrow(/gateway\/a/)
    const outside = await tempDir()
    await expect(repositories.stageAll(outside)).rejects.toThrow(/not a git repository/)
    await expect(repositories.stagedFiles(outside)).rejects.toThrow()
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

  it('reports no remote for a repository without one', async () => {
    const root = await tempDir()
    await git(root, 'init', '-q')

    expect(await repositories.remotes(root)).toEqual([])
  })

  it('names every worktree of a repository, and reads what each holds', async () => {
    const root = await committed()
    const worktree = join(root, '..', `${basename(root)}-worktree`)
    cleanups.push(() => rm(worktree, { force: true, recursive: true }))
    await git(root, 'worktree', 'add', '-q', worktree)

    expect(await repositories.listWorktrees(root)).toEqual([await realpath(root), await realpath(worktree)])
    expect(await repositories.status(worktree)).toBe('')
    await writeFile(join(worktree, 'stepped-over.md'), 'x')
    await writeFile(join(root, 'README.md'), 'edited')
    expect(await repositories.status(worktree)).toBe('?? stepped-over.md\n')
    expect(await repositories.status(root)).toBe(' M README.md\n')
    await expect(repositories.status(await tempDir())).rejects.toThrow(/not a git repository/)
  })

  it('names and reads a worktree that lives inside the common directory, as the gateway’s own do', async () => {
    const root = await committed()
    const nested = join(root, '.git', 'model-gateway', 'worktrees', 'run-1')
    await mkdir(dirname(nested), { recursive: true })
    await git(root, 'worktree', 'add', '-q', nested)

    expect(await repositories.listWorktrees(root)).toContain(await realpath(nested))
    expect(await repositories.status(nested)).toBe('')
    expect(await repositories.status(root)).toBe('')
  })

  it('leaves a worktree that is gone, and so marked prunable, out of the list', async () => {
    const root = await committed()
    const worktree = join(root, '..', `${basename(root)}-worktree`)
    cleanups.push(() => rm(worktree, { force: true, recursive: true }))
    await git(root, 'worktree', 'add', '-q', worktree)
    await rm(worktree, { force: true, recursive: true })

    expect(await repositories.listWorktrees(root)).toEqual([await realpath(root)])
  })

  it('fails with git\'s own message when it cannot name the worktrees of a path', async () => {
    const dir = await tempDir()

    await expect(repositories.listWorktrees(dir)).rejects.toThrow(/not a git repository/)
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

  it('fails, rather than calling it no repository, when git answers with nothing', async () => {
    const dir = await tempDir()
    const bin = await fakeGit(dir, '#!/bin/sh\nexit 0\n')

    await expect(new NodeModelGatewayGit(bin).root(dir)).rejects.toThrow(/git answered with no repository root/)
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

describe('NodeModelGatewayKept', () => {
  function worktree(runId: string, workflow = 'spec-293'): KeptWorktree {
    return { at: '2026-01-01T00:00:00.000Z', branch: `gateway/${runId}`, path: `/repo/.git/model-gateway/worktrees/${runId}`, repo: '/repo', run_id: runId, workflow }
  }

  async function kept(timings = {}): Promise<{ kept: NodeModelGatewayKept; path: string }> {
    const home = await tempDir()
    return { kept: new NodeModelGatewayKept(new FakeEnv({ HOME: home }), timings), path: join(home, '.local', 'state', 'agent-kit', 'model-gateway', 'kept-worktrees.json') }
  }

  it('is empty until a worktree is added, then lists what was added, in order', async () => {
    const { kept: list, path } = await kept()

    expect(await list.list()).toEqual([])
    await list.add(worktree('one'))
    await list.add(worktree('two', 'spec-294'))

    expect((await list.list()).map((entry) => entry.run_id)).toEqual(['one', 'two'])
    expect(JSON.parse(await readFile(path, 'utf8'))).toEqual({ version: 1, worktrees: [worktree('one'), worktree('two', 'spec-294')] })
  })

  it('forgets the worktrees of the runs it is told to, and leaves the rest', async () => {
    const { kept: list } = await kept()
    await list.add(worktree('one'))
    await list.add(worktree('two'))

    await list.remove(['one', 'unknown'])

    expect((await list.list()).map((entry) => entry.run_id)).toEqual(['two'])
  })

  it('names a file that is not the layout it writes, and does not overwrite it', async () => {
    const { kept: list, path } = await kept()
    await mkdir(join(path, '..'), { recursive: true })
    await writeFile(path, '{"version":1,"worktrees":{}}')

    await expect(list.add(worktree('one'))).rejects.toThrow(/must be/)
    await writeFile(path, '[')
    await expect(list.list()).rejects.toThrow(/not JSON/)
    expect(await readFile(path, 'utf8')).toBe('[')
  })

  it('fails with the file system’s own error when the file cannot be read at all', async () => {
    const { kept: list, path } = await kept()
    await mkdir(path, { recursive: true })

    await expect(list.list()).rejects.toThrow(/EISDIR/)
  })

  it('takes turns with another writer: concurrent adds all land', async () => {
    const { kept: list } = await kept()

    await Promise.all(Array.from({ length: 12 }, (_, index) => list.add(worktree(`run-${String(index)}`))))

    expect(await list.list()).toHaveLength(12)
  })

  it('gives up, naming what the lock guards, when another process holds it past the timeout', async () => {
    const { kept: list, path } = await kept({ retryMs: 1, staleMs: 60_000, timeoutMs: 30 })
    await mkdir(join(path, '..'), { recursive: true })
    await writeFile(`${path}.lock`, '')

    await expect(list.add(worktree('one'))).rejects.toThrow(/held the kept-worktrees lock/)
  })

  it('replaces the list in one step, leaving no half-written file and no temporary one beside it', async () => {
    const { kept: list, path } = await kept()
    await list.add(worktree('one'))

    expect(await readdir(join(path, '..'))).toEqual(['kept-worktrees.json'])
    expect(JSON.parse(await readFile(path, 'utf8'))).toMatchObject({ worktrees: [{ run_id: 'one' }] })
  })
})

describe('writeFileAtomic', () => {
  it('replaces the file in one step, leaving no temporary file behind', async () => {
    const dir = await tempDir()
    const path = join(dir, 'kept-worktrees.json')
    await writeFile(path, 'previous')

    await writeFileAtomic(path, 'next')

    expect(await readFile(path, 'utf8')).toBe('next')
    expect(await readdir(dir)).toEqual(['kept-worktrees.json'])
  })

  it('leaves the previous file intact when the new one cannot be written', async () => {
    const dir = await tempDir()
    const path = join(dir, 'kept-worktrees.json')
    await writeFile(path, 'previous')
    await chmod(dir, 0o500)
    cleanups.push(() => chmod(dir, 0o700))

    await expect(writeFileAtomic(path, 'next')).rejects.toThrow(/EACCES|permission denied/i)

    await chmod(dir, 0o700)
    expect(await readFile(path, 'utf8')).toBe('previous')
    expect(await readdir(dir)).toEqual(['kept-worktrees.json'])
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

describe('NodeModelGatewayWorkflows', () => {
  /** The workflows a test writes to, in a state directory of its own. */
  async function store(timings: WorkflowLockTimings = {}): Promise<{ path: string; workflows: NodeModelGatewayWorkflows }> {
    const home = await tempDir()
    const env = new FakeEnv({ HOME: home })
    return {
      path: join(home, '.local', 'state', 'agent-kit', 'model-gateway', 'workflows.json'),
      workflows: new NodeModelGatewayWorkflows(env, timings),
    }
  }

  /** A moment in the test's own clock, as the domain reads it. */
  function at(minutes: number): Date {
    return new Date(Date.UTC(2026, 10, 4, 12, minutes))
  }

  it('holds no workflow before the first one is started', async () => {
    const { workflows } = await store()

    expect(await workflows.read()).toEqual([])
  })

  it('writes a started workflow, creating the state directory, and reads it back', async () => {
    const { path, workflows } = await store()

    const started = await workflows.update((held) => startWorkflow(held, 'spec-293', at(0)))

    expect(started).toEqual({ name: 'spec-293', started_at: '2026-11-04T12:00:00.000Z' })
    expect(await workflows.read()).toEqual([started])
    expect(JSON.parse(await readFile(path, 'utf8'))).toEqual({
      version: 1,
      workflows: [{ name: 'spec-293', started_at: '2026-11-04T12:00:00.000Z' }],
    })
  })

  it('keeps the workflows it already held when it writes another', async () => {
    const { workflows } = await store()
    await workflows.update((held) => startWorkflow(held, 'spec-293', at(0)))

    await workflows.update((held) => startWorkflow(held, 'review', at(1)))

    expect((await workflows.read()).map((workflow) => workflow.name)).toEqual(['spec-293', 'review'])
  })

  it('writes an ended workflow with the moment it ended', async () => {
    const { workflows } = await store()
    await workflows.update((held) => startWorkflow(held, 'spec-293', at(0)))

    const ended = await workflows.update((held) => endWorkflow(held, 'spec-293', at(5)))

    expect(ended).toEqual({ ended_at: '2026-11-04T12:05:00.000Z', name: 'spec-293', started_at: '2026-11-04T12:00:00.000Z' })
    expect(await workflows.read()).toEqual([ended])
  })

  it('names the file when it is not JSON', async () => {
    const { path, workflows } = await store()
    await mkdir(join(path, '..'), { recursive: true })
    await writeFile(path, 'not json\n')

    await expect(workflows.read()).rejects.toThrow(new RegExp(`${path}: not JSON`))
  })

  it('names the file when it holds another version', async () => {
    const { path, workflows } = await store()
    await mkdir(join(path, '..'), { recursive: true })
    await writeFile(path, '{"version":2,"workflows":[]}\n')

    await expect(workflows.read()).rejects.toThrow(new RegExp(`${path}: version must be 1`))
  })

  it('fails with the file system\'s own error when the file cannot be read at all', async () => {
    const { path, workflows } = await store()
    await mkdir(path, { recursive: true })

    await expect(workflows.read()).rejects.toThrow(new RegExp(`${path}: cannot read the workflows`))
  })

  it('never loses a change when two updates run at once', async () => {
    const { workflows } = await store({ retryMs: 1 })
    const names = Array.from({ length: 8 }, (_, index) => `workflow-${String(index)}`)

    await Promise.all(names.map((name) => workflows.update((held) => startWorkflow(held, name, at(0)))))

    expect((await workflows.read()).map((workflow) => workflow.name).sort()).toEqual(names)
  })

  it('waits for a lock another process holds, then writes', async () => {
    const { path, workflows } = await store({ retryMs: 5 })
    const lock = `${path}.lock`
    await mkdir(join(path, '..'), { recursive: true })
    await writeFile(lock, '')
    const release = setTimeout(() => {
      void rm(lock, { force: true })
    }, 50)
    try {
      await workflows.update((held) => startWorkflow(held, 'spec-293', at(0)))
    } finally {
      clearTimeout(release)
    }

    expect(await readFile(path, 'utf8')).toContain('"name": "spec-293"')
  })

  it('breaks a lock left behind by a process that died holding it', async () => {
    const { path, workflows } = await store({ retryMs: 5, staleMs: 20 })
    const lock = `${path}.lock`
    await mkdir(join(path, '..'), { recursive: true })
    await writeFile(lock, '')
    const old = new Date(Date.now() - 60_000)
    await utimes(lock, old, old)

    await workflows.update((held) => startWorkflow(held, 'spec-293', at(0)))

    expect(await readFile(path, 'utf8')).toContain('"name": "spec-293"')
    await expect(stat(lock)).rejects.toThrow()
  })

  it('gives up when another process holds the lock past its timeout', async () => {
    const { path, workflows } = await store({ retryMs: 5, timeoutMs: 30 })
    const lock = `${path}.lock`
    await mkdir(join(path, '..'), { recursive: true })
    await writeFile(lock, '')

    await expect(workflows.update((held) => startWorkflow(held, 'spec-293', at(0)))).rejects.toThrow(
      /held the workflows lock/,
    )
    await expect(readFile(path, 'utf8')).rejects.toThrow()
  })

  it('leaves no temporary file behind', async () => {
    const { path, workflows } = await store()

    await workflows.update((held) => startWorkflow(held, 'spec-293', at(0)))

    expect(await readdir(join(path, '..'))).toEqual(['workflows.json'])
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

describe('NodeModelGatewayGrants', () => {
  /** The grants a test writes to, in a machine configuration directory of its own. */
  async function store(
    timings: GrantLockTimings = {},
  ): Promise<{ path: string; grants: NodeModelGatewayGrants; env: FakeEnv }> {
    const home = await tempDir()
    const env = new FakeEnv({ HOME: home })
    return {
      env,
      grants: new NodeModelGatewayGrants(env, new NodeModelGatewayGit(), timings),
      path: join(home, '.config', 'agent-kit', 'model-gateway-grants.json'),
    }
  }

  /** A grant the owner gave for `spec-293`, as `grant write` would build it. */
  function grant(overrides: Partial<Grant> = {}): Grant {
    return {
      ...newGrant({
        answer: 'yes, up to five dollars',
        cap_usd: 5,
        client: 'codex',
        id: 'grant-20261104T105215Z-ebd578',
        question: 'may I spend up to $5 of nedap money on spec-293?',
        workflow: 'spec-293',
        workflowRecord: undefined,
        written_at: new Date('2026-11-04T10:52:15.000Z'),
      }),
      ...overrides,
    }
  }

  it('holds no grant before the first one is written', async () => {
    const { grants } = await store()

    expect(await grants.read()).toEqual([])
  })

  it('writes a grant into the machine configuration, creating the directory, and reads it back', async () => {
    const { grants, path } = await store()
    const written = grant()

    const changed = await grants.update((held) => ({ grant: written, grants: [...held, written] }))

    expect(changed).toEqual(written)
    expect(await grants.read()).toEqual([written])
    expect(JSON.parse(await readFile(path, 'utf8'))).toEqual({ grants: [written], version: 1 })
  })

  it('keeps the grants it already held when it writes another', async () => {
    const { grants } = await store()
    const first = grant()
    const second = grant({ id: 'grant-20261104T105215Z-aaaaaa', workflow: 'spec-294' })
    await grants.update((held) => ({ grant: first, grants: [...held, first] }))

    await grants.update((held) => ({ grant: second, grants: [...held, second] }))

    expect((await grants.read()).map((entry) => entry.workflow)).toEqual(['spec-293', 'spec-294'])
  })

  it('writes where XDG_CONFIG_HOME points, not under the home directory', async () => {
    const home = await tempDir()
    const config = await tempDir()
    const grants = new NodeModelGatewayGrants(new FakeEnv({ HOME: home, XDG_CONFIG_HOME: config }), new NodeModelGatewayGit())
    const written = grant()

    await grants.update((held) => ({ grant: written, grants: [...held, written] }))

    expect(JSON.parse(await readFile(join(config, 'agent-kit', 'model-gateway-grants.json'), 'utf8'))).toEqual({
      grants: [written],
      version: 1,
    })
  })

  it('refuses to write a grant inside a git repository, naming the repository', async () => {
    const root = await tempDir()
    await git(root, 'init', '-q')
    const config = join(root, 'config')
    const grants = new NodeModelGatewayGrants(new FakeEnv({ HOME: root, XDG_CONFIG_HOME: config }), new NodeModelGatewayGit())
    const written = grant()
    // Resolved before the call starts: awaiting inside the expect() arguments would leave the
    // already-rejecting promise without a handler, which vitest reports as an unhandled rejection.
    const expected = new RegExp(
      `${join(config, 'agent-kit', 'model-gateway-grants.json')} is inside the git repository ${await realpath(root)}: a grant lives in the machine configuration, never in a repository`,
    )

    await expect(grants.update((held) => ({ grant: written, grants: [...held, written] }))).rejects.toThrow(expected)
    await expect(readFile(join(config, 'agent-kit', 'model-gateway-grants.json'), 'utf8')).rejects.toThrow()
  })

  it('refuses, naming the file, when git cannot answer whether the grants file is in a repository', async () => {
    const dir = await tempDir()
    const home = join(dir, 'home')
    const path = join(home, '.config', 'agent-kit', 'model-gateway-grants.json')
    const dubious = await fakeGit(dir, '#!/bin/sh\necho "fatal: detected dubious ownership in repository" >&2\nexit 128\n')
    const gits = {
      'git refuses the directory': new NodeModelGatewayGit(dubious),
      'git cannot be started': new NodeModelGatewayGit(''),
    }

    for (const git of Object.values(gits)) {
      const grants = new NodeModelGatewayGrants(new FakeEnv({ HOME: home }), git)
      const written = grant()
      const read = grants.read()
      await expect(read).rejects.toMatchObject({ code: 'invalid-config' })
      await expect(read).rejects.toThrow(new RegExp(`${path}: cannot tell whether .* is inside a git repository`))
      await expect(grants.update((held) => ({ grant: written, grants: [...held, written] }))).rejects.toMatchObject({
        code: 'invalid-config',
      })
      await expect(readFile(path, 'utf8')).rejects.toThrow()
    }
  })

  it('refuses when git answers with no repository root at all, rather than calling it outside one', async () => {
    const dir = await tempDir()
    const home = join(dir, 'home')
    const silent = await fakeGit(dir, '#!/bin/sh\nexit 0\n')
    const grants = new NodeModelGatewayGrants(new FakeEnv({ HOME: home }), new NodeModelGatewayGit(silent))

    await expect(grants.read()).rejects.toMatchObject({ code: 'invalid-config' })
  })

  it('wraps what is thrown that is not an Error too', async () => {
    const throwing = {
      root: vi.fn().mockRejectedValue('boom'),
    } as unknown as NodeModelGatewayGit
    const grants = new NodeModelGatewayGrants(new FakeEnv({ HOME: await tempDir() }), throwing)

    await expect(grants.read()).rejects.toThrow(/cannot tell whether .*boom/)
  })

  it('refuses to read a grants file inside a git repository too', async () => {
    const root = await tempDir()
    await git(root, 'init', '-q')
    const grants = new NodeModelGatewayGrants(new FakeEnv({ HOME: root }), new NodeModelGatewayGit())

    const expected = new RegExp(`is inside the git repository ${await realpath(root)}`)

    await expect(grants.read()).rejects.toThrow(expected)
  })

  it('names the file when it is not JSON', async () => {
    const { grants, path } = await store()
    await mkdir(join(path, '..'), { recursive: true })
    await writeFile(path, 'not json\n')

    await expect(grants.read()).rejects.toThrow(new RegExp(`${path}: not JSON`))
  })

  it('names the file when it holds another version', async () => {
    const { grants, path } = await store()
    await mkdir(join(path, '..'), { recursive: true })
    await writeFile(path, '{"version":2,"grants":[]}\n')

    await expect(grants.read()).rejects.toThrow(new RegExp(`${path}: version must be 1`))
  })

  it('fails with the file system\'s own error when the file cannot be read at all', async () => {
    const { grants, path } = await store()
    await mkdir(path, { recursive: true })

    await expect(grants.read()).rejects.toThrow(new RegExp(`${path}: cannot read the grants`))
  })

  it('never loses a grant when two updates run at once', async () => {
    const { grants } = await store({ retryMs: 1 })
    const written = Array.from({ length: 8 }, (_, index) =>
      grant({ id: `grant-20261104T105215Z-00000${String(index)}`, workflow: `workflow-${String(index)}` }),
    )

    await Promise.all(written.map((entry) => grants.update((held) => ({ grant: entry, grants: [...held, entry] }))))

    expect((await grants.read()).map((entry) => entry.workflow).sort()).toEqual(written.map((entry) => entry.workflow))
  })

  it('leaves no temporary file behind', async () => {
    const { grants, path } = await store()
    const written = grant()

    await grants.update((held) => ({ grant: written, grants: [...held, written] }))

    expect(await readdir(join(path, '..'))).toEqual(['model-gateway-grants.json'])
  })
})
