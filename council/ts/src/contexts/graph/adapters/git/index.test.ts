import { describe, expect, it } from 'vitest'

import type { ProcessCommand, ProcessPort, ProcessResult } from '../../../../ports/index.js'
import { GitCliAdapter as BarrelGitCliAdapter } from '../../index.js'
import type { GitReconcileRequest as BarrelGitReconcileRequest } from '../../index.js'
import { GitCliAdapter, GitCommandError } from './index.js'

type Handler = (command: ProcessCommand) => ProcessResult | Promise<ProcessResult>

class RecordingProcess implements ProcessPort {
  readonly commands: ProcessCommand[] = []
  private readonly handler: Handler

  constructor(handler: Handler) {
    this.handler = handler
  }

  async exec(command: ProcessCommand): Promise<ProcessResult> {
    this.commands.push(command)
    return this.handler(command)
  }
}

function success(stdout = ''): ProcessResult {
  return {
    exitCode: 0,
    stderr: '',
    stdout,
  }
}

function failure(exitCode: number, stderr = '', stdout = ''): ProcessResult {
  return {
    exitCode,
    stderr,
    stdout,
  }
}

describe('GitCliAdapter', () => {
  it('runs root, branch, worktree, status, diff, and push commands', async () => {
    const process = new RecordingProcess((command) => {
      const args = command.args.join(' ')
      if (args === 'rev-parse --show-toplevel') {
        return success('/repo\n')
      }
      if (args === 'branch --show-current') {
        return success('main\n')
      }
      if (args === 'status --porcelain=v1') {
        return success(' M changed.ts\nR  old name.ts -> new name.ts\n?? fresh.ts\n')
      }
      if (args === 'diff --staged main..feature -- src/index.ts') {
        return success('patch')
      }
      if (args === 'show-ref --verify --quiet refs/heads/worker-a') {
        return failure(1)
      }
      return success()
    })
    const adapter = new GitCliAdapter(process)

    await expect(adapter.root('/repo/sub')).resolves.toBe('/repo')
    await expect(adapter.currentBranch('/repo')).resolves.toBe('main')
    await expect(adapter.createWorktree('/repo', 'worker-a', '/tmp/worker-a')).resolves.toEqual({
      branch: 'worker-a',
      path: '/tmp/worker-a',
    })
    await expect(adapter.removeWorktree('/repo', '/tmp/worker-a')).resolves.toBeUndefined()
    await expect(adapter.changedFiles('/repo')).resolves.toEqual([
      'changed.ts',
      'new name.ts',
      'fresh.ts',
    ])
    await expect(
      adapter.diff('/repo', {
        base: 'main',
        head: 'feature',
        paths: ['src/index.ts'],
        staged: true,
      }),
    ).resolves.toBe('patch')
    await expect(
      adapter.pushBranch('/repo', {
        branch: 'feature',
        forceWithLease: true,
        remote: 'upstream',
        setUpstream: false,
      }),
    ).resolves.toBeUndefined()

    expect(process.commands).toEqual([
      { args: ['rev-parse', '--show-toplevel'], command: 'git', cwd: '/repo/sub' },
      { args: ['branch', '--show-current'], command: 'git', cwd: '/repo' },
      {
        args: ['show-ref', '--verify', '--quiet', 'refs/heads/worker-a'],
        command: 'git',
        cwd: '/repo',
      },
      {
        args: ['worktree', 'add', '-b', 'worker-a', '/tmp/worker-a', 'HEAD'],
        command: 'git',
        cwd: '/repo',
      },
      {
        args: ['worktree', 'remove', '--force', '/tmp/worker-a'],
        command: 'git',
        cwd: '/repo',
      },
      { args: ['status', '--porcelain=v1'], command: 'git', cwd: '/repo' },
      {
        args: ['diff', '--staged', 'main..feature', '--', 'src/index.ts'],
        command: 'git',
        cwd: '/repo',
      },
      {
        args: ['push', '--force-with-lease', 'upstream', 'feature'],
        command: 'git',
        cwd: '/repo',
      },
    ])
  })

  it('adds a worktree for an existing branch without recreating the branch', async () => {
    const process = new RecordingProcess(() => success())
    const adapter = new GitCliAdapter(process)

    await expect(adapter.createWorktree('/repo', 'worker-a', '/tmp/worker-a')).resolves.toEqual({
      branch: 'worker-a',
      path: '/tmp/worker-a',
    })

    expect(process.commands).toEqual([
      {
        args: ['show-ref', '--verify', '--quiet', 'refs/heads/worker-a'],
        command: 'git',
        cwd: '/repo',
      },
      {
        args: ['worktree', 'add', '/tmp/worker-a', 'worker-a'],
        command: 'git',
        cwd: '/repo',
      },
    ])
  })

  it('handles clean status, single-ended diffs, and default push upstream', async () => {
    const process = new RecordingProcess((command) => {
      if (command.args[0] === 'status') {
        return success('\n')
      }
      return success('diff')
    })
    const adapter = new GitCliAdapter(process)

    await expect(adapter.status('/repo')).resolves.toEqual([])
    await expect(adapter.diff('/repo', { base: 'main' })).resolves.toBe('diff')
    await expect(adapter.diff('/repo', { head: 'feature' })).resolves.toBe('diff')
    await expect(adapter.pushBranch('/repo', { branch: 'feature' })).resolves.toBeUndefined()

    expect(process.commands).toEqual([
      { args: ['status', '--porcelain=v1'], command: 'git', cwd: '/repo' },
      { args: ['diff', 'main'], command: 'git', cwd: '/repo' },
      { args: ['diff', 'feature'], command: 'git', cwd: '/repo' },
      { args: ['push', '--set-upstream', 'origin', 'feature'], command: 'git', cwd: '/repo' },
    ])
  })

  it('commits all current changes and returns typed commit metadata', async () => {
    const process = new RecordingProcess((command) => {
      const args = command.args.join(' ')
      if (args === 'status --porcelain=v1') {
        return success(' M changed.ts\n?? fresh.ts\n')
      }
      if (args === 'rev-parse HEAD') {
        return success('abc123\n')
      }
      if (args === 'branch --show-current') {
        return success('worker-a\n')
      }
      return success()
    })
    const adapter = new GitCliAdapter(process)

    await expect(
      adapter.commitAll('/repo', {
        allow_empty: true,
        author: 'Agent <agent@example.test>',
        message: 'T1 implement native execution',
      }),
    ).resolves.toEqual({
      branch: 'worker-a',
      commit: 'abc123',
      files_changed: ['changed.ts', 'fresh.ts'],
      message: 'T1 implement native execution',
    })

    expect(process.commands).toEqual([
      { args: ['status', '--porcelain=v1'], command: 'git', cwd: '/repo' },
      { args: ['add', '-A'], command: 'git', cwd: '/repo' },
      {
        args: [
          'commit',
          '-m',
          'T1 implement native execution',
          '--author',
          'Agent <agent@example.test>',
          '--allow-empty',
        ],
        command: 'git',
        cwd: '/repo',
      },
      { args: ['rev-parse', 'HEAD'], command: 'git', cwd: '/repo' },
      { args: ['branch', '--show-current'], command: 'git', cwd: '/repo' },
    ])
  })

  it('commits all changes without optional commit flags', async () => {
    const process = new RecordingProcess((command) => {
      if (command.args[0] === 'status') return success(' M changed.ts\n')
      if (command.args.join(' ') === 'rev-parse HEAD') return success('def456\n')
      if (command.args[0] === 'branch') return success('worker-b\n')
      return success()
    })
    const adapter = new GitCliAdapter(process)

    await expect(
      adapter.commitAll('/repo', {
        message: 'T2 minimal commit',
      }),
    ).resolves.toEqual({
      branch: 'worker-b',
      commit: 'def456',
      files_changed: ['changed.ts'],
      message: 'T2 minimal commit',
    })

    expect(process.commands.map((command) => command.args)).toEqual([
      ['status', '--porcelain=v1'],
      ['add', '-A'],
      ['commit', '-m', 'T2 minimal commit'],
      ['rev-parse', 'HEAD'],
      ['branch', '--show-current'],
    ])
  })

  it('exports the git adapter and reconcile request type from the graph barrel', () => {
    const request: BarrelGitReconcileRequest = {
      integrationBranch: 'integration/native',
      sourceBranch: 'worker-a',
    }

    expect(BarrelGitCliAdapter).toBe(GitCliAdapter)
    expect(request).toEqual({
      integrationBranch: 'integration/native',
      sourceBranch: 'worker-a',
    })
  })

  it('creates an integration branch when missing and merges the completed branch', async () => {
    const process = new RecordingProcess((command) => {
      if (command.args[0] === 'show-ref') {
        return failure(1)
      }
      if (command.args.join(' ') === 'rev-parse HEAD') {
        return success('abc123\n')
      }
      return success()
    })
    const adapter = new GitCliAdapter(process)

    await expect(
      adapter.reconcileIntegrationBranch('/repo', {
        baseBranch: 'main',
        integrationBranch: 'council/run/integration',
        sourceBranch: 'worker-a',
      }),
    ).resolves.toEqual({
      head: 'abc123',
      integrationBranch: 'council/run/integration',
      sourceBranch: 'worker-a',
    })

    expect(process.commands).toEqual([
      {
        args: ['show-ref', '--verify', '--quiet', 'refs/heads/council/run/integration'],
        command: 'git',
        cwd: '/repo',
      },
      {
        args: ['checkout', '-B', 'council/run/integration', 'main'],
        command: 'git',
        cwd: '/repo',
      },
      {
        args: ['merge', '--no-ff', '--no-edit', 'worker-a'],
        command: 'git',
        cwd: '/repo',
      },
      { args: ['rev-parse', 'HEAD'], command: 'git', cwd: '/repo' },
    ])
  })

  it('serializes integration merges in caller completion order', async () => {
    let releaseFirstMerge: (() => void) | undefined
    const mergeStarts: string[] = []
    const process = new RecordingProcess((command) => {
      if (command.args[0] === 'merge' && command.args[3] === 'worker-a') {
        mergeStarts.push(command.args[3])
        return new Promise<ProcessResult>((resolve) => {
          releaseFirstMerge = () => {
            resolve(success())
          }
        })
      }
      if (command.args[0] === 'merge') {
        mergeStarts.push(command.args[3] ?? 'missing')
      }
      if (command.args.join(' ') === 'rev-parse HEAD') {
        return success(`head-${String(mergeStarts.length)}\n`)
      }
      return success()
    })
    const adapter = new GitCliAdapter(process)

    const first = adapter.reconcileIntegrationBranch('/repo', {
      integrationBranch: 'integration',
      sourceBranch: 'worker-a',
    })
    await expect.poll(() => mergeStarts).toEqual(['worker-a'])

    const second = adapter.reconcileIntegrationBranch('/repo', {
      integrationBranch: 'integration',
      sourceBranch: 'worker-b',
    })
    await Promise.resolve()
    expect(mergeStarts).toEqual(['worker-a'])

    releaseFirstMerge?.()
    await expect(first).resolves.toEqual({
      head: 'head-1',
      integrationBranch: 'integration',
      sourceBranch: 'worker-a',
    })
    await expect(second).resolves.toEqual({
      head: 'head-2',
      integrationBranch: 'integration',
      sourceBranch: 'worker-b',
    })
    expect(mergeStarts).toEqual(['worker-a', 'worker-b'])
  })

  it('aborts a conflicted merge and keeps the reconcile queue usable', async () => {
    let shouldFailMerge = true
    const process = new RecordingProcess((command) => {
      if (command.args[0] === 'merge' && command.args[1] === '--no-ff' && shouldFailMerge) {
        shouldFailMerge = false
        return failure(1, 'conflict')
      }
      if (command.args.join(' ') === 'rev-parse HEAD') {
        return success('recovered\n')
      }
      return success()
    })
    const adapter = new GitCliAdapter(process)

    const first = adapter.reconcileIntegrationBranch('/repo', {
      integrationBranch: 'integration',
      sourceBranch: 'worker-a',
    })
    const second = adapter.reconcileIntegrationBranch('/repo', {
      integrationBranch: 'integration',
      sourceBranch: 'worker-b',
    })

    await expect(first).rejects.toThrow(GitCommandError)
    await expect(second).resolves.toEqual({
      head: 'recovered',
      integrationBranch: 'integration',
      sourceBranch: 'worker-b',
    })

    expect(process.commands.map((command) => command.args)).toEqual([
      ['show-ref', '--verify', '--quiet', 'refs/heads/integration'],
      ['checkout', 'integration'],
      ['merge', '--no-ff', '--no-edit', 'worker-a'],
      ['merge', '--abort'],
      ['show-ref', '--verify', '--quiet', 'refs/heads/integration'],
      ['checkout', 'integration'],
      ['merge', '--no-ff', '--no-edit', 'worker-b'],
      ['rev-parse', 'HEAD'],
    ])
  })

  it('reconciles after a previously rejected integration job', async () => {
    let shouldFailMerge = true
    const process = new RecordingProcess((command) => {
      if (command.args[0] === 'merge' && shouldFailMerge) {
        shouldFailMerge = false
        return failure(1, 'conflict')
      }
      if (command.args.join(' ') === 'rev-parse HEAD') {
        return success('after-failure\n')
      }
      return success()
    })
    const adapter = new GitCliAdapter(process)

    await expect(
      adapter.reconcileIntegrationBranch('/repo', {
        integrationBranch: 'integration',
        sourceBranch: 'worker-a',
      }),
    ).rejects.toThrow(GitCommandError)
    await expect(
      adapter.reconcileIntegrationBranch('/repo', {
        integrationBranch: 'integration',
        sourceBranch: 'worker-b',
      }),
    ).resolves.toEqual({
      head: 'after-failure',
      integrationBranch: 'integration',
      sourceBranch: 'worker-b',
    })

    expect(process.commands.map((command) => command.args)).toEqual([
      ['show-ref', '--verify', '--quiet', 'refs/heads/integration'],
      ['checkout', 'integration'],
      ['merge', '--no-ff', '--no-edit', 'worker-a'],
      ['merge', '--abort'],
      ['show-ref', '--verify', '--quiet', 'refs/heads/integration'],
      ['checkout', 'integration'],
      ['merge', '--no-ff', '--no-edit', 'worker-b'],
      ['rev-parse', 'HEAD'],
    ])
  })

  it('throws git command errors for unexpected failures', async () => {
    const process = new RecordingProcess((command) => {
      if (command.args[0] === 'show-ref') {
        return failure(2, '', 'broken')
      }
      return failure(128, 'fatal')
    })
    const adapter = new GitCliAdapter(process)

    await expect(adapter.root('/repo')).rejects.toMatchObject({
      args: ['rev-parse', '--show-toplevel'],
      cwd: '/repo',
      message: 'git rev-parse --show-toplevel failed in /repo: fatal',
    })
    await expect(
      adapter.reconcileIntegrationBranch('/repo', {
        integrationBranch: 'integration',
        sourceBranch: 'worker-a',
      }),
    ).rejects.toMatchObject({
      message: 'git show-ref --verify --quiet refs/heads/integration failed in /repo: broken',
    })
  })
})

describe('GitCliAdapter repository facts', () => {
  it('reads the absolute git common directory, which every worktree shares', async () => {
    const process = new RecordingProcess(() => success('/work/repo/.git\n'))

    expect(await new GitCliAdapter(process).commonDir('/work/repo/sub')).toBe('/work/repo/.git')
    expect(process.commands).toEqual([
      { args: ['rev-parse', '--path-format=absolute', '--git-common-dir'], command: 'git', cwd: '/work/repo/sub' },
    ])
  })

  it('lists every remote with its URL, including a remote whose name has a dot', async () => {
    const process = new RecordingProcess(() =>
      success('remote.origin.url git@github.com:a/b.git\nremote.my.fork.url https://github.com/c/b\n'),
    )

    expect(await new GitCliAdapter(process).remotes('/r')).toEqual([
      { name: 'origin', url: 'git@github.com:a/b.git' },
      { name: 'my.fork', url: 'https://github.com/c/b' },
    ])
  })

  it('reports no remotes when git config finds none', async () => {
    const process = new RecordingProcess(() => failure(1))

    expect(await new GitCliAdapter(process).remotes('/r')).toEqual([])
  })

  it('reads the porcelain status with every untracked file and NUL-separated paths', async () => {
    const process = new RecordingProcess(() =>
      success(' M changed.ts\0R  new name.ts\0old name.ts\0?? dir/with "quote".md\0'),
    )

    expect(await new GitCliAdapter(process).porcelainStatus('/r')).toEqual([
      { code: ' M', path: 'changed.ts' },
      { code: 'R ', path: 'new name.ts' },
      { code: '??', path: 'dir/with "quote".md' },
    ])
    expect(process.commands).toEqual([
      { args: ['status', '--porcelain=v1', '-z', '--untracked-files=all'], command: 'git', cwd: '/r' },
    ])
  })

  it('reads a clean working tree as no porcelain entries', async () => {
    const process = new RecordingProcess(() => success(''))

    expect(await new GitCliAdapter(process).porcelainStatus('/r')).toEqual([])
  })

  it('throws with the git error when listing remotes fails otherwise', async () => {
    const process = new RecordingProcess(() => failure(128, 'fatal: not a git repository'))

    await expect(new GitCliAdapter(process).remotes('/tmp')).rejects.toThrow(/not a git repository/)
  })
})

describe('GitCliAdapter commits', () => {
  it('resolves a revision to the full id of the commit it names', async () => {
    const process = new RecordingProcess(() => success('0123456789abcdef0123456789abcdef01234567\n'))

    expect(await new GitCliAdapter(process).resolveCommit('/r', 'main')).toBe('0123456789abcdef0123456789abcdef01234567')
    expect(process.commands).toEqual([
      { args: ['rev-parse', '--verify', '--quiet', '--end-of-options', 'main^{commit}'], command: 'git', cwd: '/r' },
    ])
  })

  it('throws when a revision names no commit', async () => {
    const process = new RecordingProcess(() => failure(1))

    await expect(new GitCliAdapter(process).resolveCommit('/r', 'nope')).rejects.toThrow(/rev-parse/)
  })

  it('reads the merge base of two commits', async () => {
    const process = new RecordingProcess(() => success('abc\n'))

    expect(await new GitCliAdapter(process).mergeBase('/r', 'a1', 'b2')).toBe('abc')
    expect(process.commands[0]?.args).toEqual(['merge-base', '--end-of-options', 'a1', 'b2'])
  })

  it('lists the regular files of a commit and leaves out symbolic links and submodules', async () => {
    const process = new RecordingProcess(() =>
      success(
        [
          '100644 blob aaa\tsrc/a b.ts',
          '100755 blob bbb\tbin/run',
          '120000 blob ccc\tlink',
          '160000 commit ddd\tvendor/lib',
          '',
        ].join('\0'),
      ),
    )

    expect(await new GitCliAdapter(process).treeFiles('/r', 'abc')).toEqual(['src/a b.ts', 'bin/run'])
    expect(process.commands[0]?.args).toEqual(['ls-tree', '-r', '-z', '--full-tree', 'abc'])
  })

  it('reads a file as it is in a commit', async () => {
    const process = new RecordingProcess(() => success('line one\n'))

    expect(await new GitCliAdapter(process).fileAt('/r', 'abc', 'src/a.ts')).toBe('line one\n')
    expect(process.commands[0]?.args).toEqual(['cat-file', 'blob', 'abc:src/a.ts'])
  })

  it('reads the diff between two commits without diff drivers, text conversion or colour', async () => {
    const process = new RecordingProcess(() => success('diff --git a/x b/x\n'))

    expect(await new GitCliAdapter(process).diffCommits('/r', 'a1', 'b2')).toBe('diff --git a/x b/x\n')
    expect(process.commands[0]?.args).toEqual([
      '-c',
      'core.quotePath=false',
      'diff',
      '--no-ext-diff',
      '--no-textconv',
      '--no-color',
      'a1',
      'b2',
      '--',
    ])
  })

  it('lists the paths a diff between two commits touches, a rename as both paths', async () => {
    const process = new RecordingProcess(() => success('src/a.ts\0old name.ts\0new name.ts\0'))

    expect(await new GitCliAdapter(process).changedPaths('/r', 'a1', 'b2')).toEqual(['src/a.ts', 'old name.ts', 'new name.ts'])
    expect(process.commands[0]?.args).toEqual(['diff', '--name-only', '-z', '--no-renames', 'a1', 'b2', '--'])
  })
})
