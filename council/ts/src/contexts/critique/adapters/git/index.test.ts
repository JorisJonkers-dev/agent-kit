import { describe, expect, it } from 'vitest'

import type { ProcessCommand, ProcessPort, ProcessResult } from '../../../../ports/index.js'
import { ProcessCritiqueGit } from './index.js'

class ScriptedProcess implements ProcessPort {
  readonly commands: ProcessCommand[] = []

  constructor(private readonly reply: (args: readonly string[]) => ProcessResult) {}

  async exec(command: ProcessCommand): Promise<ProcessResult> {
    this.commands.push(command)
    return Promise.resolve(this.reply(command.args))
  }
}

const ok = (stdout: string): ProcessResult => ({ exitCode: 0, stderr: '', stdout })

describe('ProcessCritiqueGit', () => {
  it('reads the top level and the absolute common directory', async () => {
    const process = new ScriptedProcess((args) =>
      ok(args.includes('--show-toplevel') ? '/work/repo\n' : '/work/repo/.git\n'),
    )
    const git = new ProcessCritiqueGit(process)

    expect(await git.root('/work/repo/sub')).toBe('/work/repo')
    expect(await git.commonDir('/work/repo/sub')).toBe('/work/repo/.git')
    expect(process.commands.map((c) => [c.command, ...c.args])).toEqual([
      ['git', '-C', '/work/repo/sub', 'rev-parse', '--show-toplevel'],
      ['git', '-C', '/work/repo/sub', 'rev-parse', '--path-format=absolute', '--git-common-dir'],
    ])
  })

  it('lists every remote with its URL', async () => {
    const git = new ProcessCritiqueGit(
      new ScriptedProcess(() =>
        ok('remote.origin.url git@github.com:a/b.git\nremote.my.fork.url https://github.com/c/b\n'),
      ),
    )

    expect(await git.remotes('/r')).toEqual([
      { name: 'origin', url: 'git@github.com:a/b.git' },
      { name: 'my.fork', url: 'https://github.com/c/b' },
    ])
  })

  it('reports no remotes when git config finds none', async () => {
    const git = new ProcessCritiqueGit(new ScriptedProcess(() => ({ exitCode: 1, stderr: '', stdout: '' })))

    expect(await git.remotes('/r')).toEqual([])
  })

  it('throws with git stderr on any other failure', async () => {
    const git = new ProcessCritiqueGit(
      new ScriptedProcess(() => ({ exitCode: 128, stderr: 'fatal: not a git repository', stdout: '' })),
    )

    await expect(git.root('/tmp')).rejects.toThrow(/not a git repository/)
    await expect(git.remotes('/tmp')).rejects.toThrow(/not a git repository/)
  })
})
