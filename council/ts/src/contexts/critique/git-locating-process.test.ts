import { chmod, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import type { EnvPort, ProcessCommand, ProcessPort, ProcessResult } from '../../ports/index.js'
import { gitSearchDirs, GitLocatingProcess } from './adapters/git/index.js'

const dirs: string[] = []

afterEach(async () => {
  await Promise.all(dirs.splice(0).map((dir) => rm(dir, { force: true, recursive: true })))
})

async function scratch(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'git-locate-'))
  dirs.push(dir)
  return dir
}

async function fakeGit(dir: string): Promise<string> {
  await mkdir(dir, { recursive: true })
  const path = join(dir, 'git')
  await writeFile(path, '#!/bin/sh\n')
  await chmod(path, 0o755)
  return path
}

function envOf(values: Record<string, string>): EnvPort {
  return {
    all: () => values,
    get: (name) => values[name],
    require: (name) => values[name] ?? '',
  }
}

class Recorder implements ProcessPort {
  readonly seen: ProcessCommand[] = []
  exec(command: ProcessCommand): Promise<ProcessResult> {
    this.seen.push(command)
    return Promise.resolve({ exitCode: 0, stderr: '', stdout: 'ok' })
  }
}

describe('GitLocatingProcess', () => {
  it('runs another command untouched', async () => {
    const inner = new Recorder()
    const process = new GitLocatingProcess(inner, envOf({}))

    await process.exec({ args: ['x'], command: 'echo' })

    expect(inner.seen).toEqual([{ args: ['x'], command: 'echo' }])
  })

  it('runs the git that PATH names, by its full path', async () => {
    const root = await scratch()
    const git = await fakeGit(join(root, 'bin'))
    const inner = new Recorder()

    await new GitLocatingProcess(inner, envOf({ HOME: root, PATH: `${join(root, 'none')}:${join(root, 'bin')}` })).exec({ args: ['status'], command: 'git', cwd: root })

    expect(inner.seen).toEqual([{ args: ['status'], command: git, cwd: root }])
  })

  it('finds git in the usual places when the hook environment has no PATH', async () => {
    const root = await scratch()
    const git = await fakeGit(join(root, '.nix-profile', 'bin'))
    const inner = new Recorder()

    await new GitLocatingProcess(inner, envOf({ HOME: root }), [join(root, 'missing'), join(root, '.nix-profile', 'bin')]).exec({ args: [], command: 'git' })

    expect(inner.seen.map((seen) => seen.command)).toEqual([git])
  })

  it('names the places it looked when there is no git', async () => {
    const root = await scratch()
    const inner = new Recorder()
    const process = new GitLocatingProcess(inner, envOf({ HOME: root, PATH: join(root, 'empty') }), [join(root, 'also-empty')])

    await expect(process.exec({ args: [], command: 'git' })).rejects.toThrow(`git was not found on PATH (${join(root, 'empty')}) or in ${join(root, 'also-empty')}`)
    expect(inner.seen).toEqual([])
  })

  it('says a missing working directory is the cause, not git', async () => {
    const root = await scratch()
    const git = await fakeGit(join(root, 'bin'))
    const inner = new Recorder()
    const gone = join(root, 'removed-worktree')

    await expect(new GitLocatingProcess(inner, envOf({ PATH: join(root, 'bin') })).exec({ args: [], command: 'git', cwd: gone })).rejects.toThrow(
      `git cannot run in ${gone}: the directory does not exist`,
    )
    expect(git).toContain('bin')
    expect(inner.seen).toEqual([])
  })

  it('lists the usual places for the user and home', () => {
    expect(gitSearchDirs(envOf({ HOME: '/home/a', USER: 'a' }))).toEqual([
      '/run/current-system/sw/bin',
      '/etc/profiles/per-user/a/bin',
      '/home/a/.nix-profile/bin',
      '/nix/var/nix/profiles/default/bin',
      '/usr/local/bin',
      '/usr/bin',
      '/bin',
      '/opt/homebrew/bin',
    ])
    expect(gitSearchDirs(envOf({}))).toEqual(['/run/current-system/sw/bin', '/nix/var/nix/profiles/default/bin', '/usr/local/bin', '/usr/bin', '/bin', '/opt/homebrew/bin'])
  })
})
