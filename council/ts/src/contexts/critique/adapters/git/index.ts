import { constants } from 'node:fs'
import { access, stat } from 'node:fs/promises'
import { join } from 'node:path'

import type { EnvPort, ProcessCommand, ProcessPort, ProcessResult } from '../../../../ports/index.js'

/**
 * Where git usually is when a hook runs with a `PATH` that lacks it: a Nix
 * system or user profile, the Nix default profile, and the Homebrew and
 * system directories. An agent's hook environment is not always the
 * login shell's, so `PATH` alone cannot be trusted to name git.
 */
export function gitSearchDirs(env: EnvPort): readonly string[] {
  const user = env.get('USER')
  const home = env.get('HOME')
  return [
    '/run/current-system/sw/bin',
    ...(user === undefined ? [] : [`/etc/profiles/per-user/${user}/bin`]),
    ...(home === undefined ? [] : [join(home, '.nix-profile', 'bin')]),
    '/nix/var/nix/profiles/default/bin',
    '/usr/local/bin',
    '/usr/bin',
    '/bin',
    '/opt/homebrew/bin',
  ]
}

/**
 * Runs `git` by the full path of the first executable git on `PATH`, then in
 * the usual places, so a hook whose `PATH` lacks git still finds it. Node
 * reports both a missing git and a missing working directory as `spawn git
 * ENOENT`; this names which one it is, before anything is spawned. Any other
 * command passes through untouched.
 */
export class GitLocatingProcess implements ProcessPort {
  constructor(
    private readonly inner: ProcessPort,
    private readonly env: EnvPort,
    private readonly fallbackDirs: readonly string[] = gitSearchDirs(env),
  ) {}

  async exec(command: ProcessCommand): Promise<ProcessResult> {
    if (command.command !== 'git') return this.inner.exec(command)
    if (command.cwd !== undefined && !(await isDirectory(command.cwd))) {
      throw new Error(`git cannot run in ${command.cwd}: the directory does not exist`)
    }
    const path = (this.env.get('PATH') ?? '').split(':').filter((entry) => entry.length > 0)
    for (const dir of [...path, ...this.fallbackDirs]) {
      const git = join(dir, 'git')
      if (await isExecutable(git)) return this.inner.exec({ ...command, command: git })
    }
    throw new Error(`git was not found on PATH (${path.join(':')}) or in ${this.fallbackDirs.join(':')}`)
  }
}

async function isDirectory(path: string): Promise<boolean> {
  return stat(path).then(
    (found) => found.isDirectory(),
    () => false,
  )
}

async function isExecutable(path: string): Promise<boolean> {
  return access(path, constants.X_OK).then(
    () => true,
    () => false,
  )
}
