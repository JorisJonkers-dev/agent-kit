import { basename, dirname } from 'node:path'

import type { BuildTrackerRepositoryPort, RepositoryLocation } from '../../application/ports.js'
import type { RepositoryName } from '../../domain/spec.js'
import type { BuildTrackerProcess } from '../process/index.js'

/** Finds a directory's repository with git: its root, the directory's place under it, and its remote. */
export class GitBuildTrackerRepositories implements BuildTrackerRepositoryPort {
  constructor(private readonly process: BuildTrackerProcess) {}

  async locate(dir: string): Promise<RepositoryLocation | undefined> {
    const paths = await this.git(dir, ['rev-parse', '--path-format=absolute', '--show-toplevel', '--git-common-dir', '--show-prefix'])
    if (paths === undefined) return undefined
    const [root = '', commonDir = '', prefix = ''] = paths.split('\n')
    const remotes = (await this.git(dir, ['config', '--get-regexp', String.raw`^remote\..*\.url$`])) ?? ''
    return {
      mainRoot: basename(commonDir) === '.git' ? dirname(commonDir) : commonDir,
      prefix,
      remote: primaryRemote(remotes),
      root,
    }
  }

  /** The command's output, or undefined when git exits non-zero. */
  private async git(dir: string, args: readonly string[]): Promise<string | undefined> {
    const result = await this.process.exec({ args: ['-C', dir, ...args], command: 'git' })
    return result.exitCode === 0 ? result.stdout : undefined
  }
}

/** `origin`'s location, or the first remote's when there is no `origin`. */
function primaryRemote(config: string): RepositoryName | undefined {
  const remotes = config.split('\n').flatMap((line) => {
    const match = /^remote\.(.+)\.url\s+(.+)$/.exec(line.trim())
    const location = match === null ? undefined : parseRemote(String(match[2]))
    return location === undefined ? [] : [{ location, name: String(match?.[1]) }]
  })
  return (remotes.find((remote) => remote.name === 'origin') ?? remotes[0])?.location
}

/** Reads `git@host:owner/repo.git`, `https://host/owner/repo.git` or `ssh://git@host/owner/repo`. */
export function parseRemote(url: string): RepositoryName | undefined {
  const match = /^[^@/\s]+@([^:/\s]+):(.+)$/.exec(url) ?? /^[a-z+]+:\/\/(?:[^@/]+@)?([^/:]+)(?::\d+)?\/(.+)$/i.exec(url)
  if (match === null) return undefined
  const segments = String(match[2])
    .replace(/\/+$/, '')
    .replace(/\.git$/, '')
    .split('/')
    .filter((segment) => segment.length > 0)
  const repo = segments.pop()
  const owner = segments.join('/')
  if (repo === undefined || owner.length === 0) return undefined
  return { host: String(match[1]).toLowerCase(), owner, repo }
}
