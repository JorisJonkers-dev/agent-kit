import { existsSync } from 'node:fs'
import { mkdir, readFile } from 'node:fs/promises'
import { dirname } from 'node:path'

import type { EnvPort } from '../../../../ports/index.js'
import { grantsPath } from '../../application/locations.js'
import type { ModelGatewayGitPort, ModelGatewayGrantsPort } from '../../application/ports.js'
import { ModelGatewayError } from '../../domain/config.js'
import { GRANTS_VERSION, parseGrantsFile, type Grant, type GrantChange } from '../../domain/grants.js'
import { writeFileAtomic } from '../fs/index.js'
import { FileLock, type LockTimings } from '../lock/index.js'

export type GrantLockTimings = LockTimings

/**
 * The grants the owner gave, in one JSON file in the machine configuration
 * (`$XDG_CONFIG_HOME/agent-kit/model-gateway-grants.json`, else
 * `~/.config/agent-kit/...`). A change reads the file, applies itself and
 * writes the result under an exclusive lock beside it, so two gateway
 * processes never lose each other's grant, and the write goes to a temporary
 * file that is renamed into place.
 *
 * A grant is never written inside a repository, so every read and every write
 * first asks git whether the file's directory lies in one, and refuses when it
 * does. The directory need not exist yet: the nearest one that does is the one
 * git is asked about. When git cannot answer (it is missing, refuses the
 * directory or does not reply) the file is refused as well: only git saying
 * "not a repository" lets a grant be read or written.
 */
export class NodeModelGatewayGrants implements ModelGatewayGrantsPort {
  private readonly lock: FileLock

  constructor(
    private readonly env: EnvPort,
    private readonly git: ModelGatewayGitPort,
    timings: GrantLockTimings = {},
  ) {
    this.lock = new FileLock('grants', timings)
  }

  async read(): Promise<readonly Grant[]> {
    await this.requireOutsideRepository()
    return this.load()
  }

  async update(change: (grants: readonly Grant[]) => GrantChange): Promise<Grant> {
    await this.requireOutsideRepository()
    const path = grantsPath(this.env)
    await mkdir(dirname(path), { recursive: true })
    return this.lock.run(`${path}.lock`, async () => {
      const changed = change(await this.load())
      await writeFileAtomic(path, `${JSON.stringify({ grants: changed.grants, version: GRANTS_VERSION }, null, 2)}\n`)
      return changed.grant
    })
  }

  /** Refuses a grants file that lies inside a git repository, naming the repository. */
  private async requireOutsideRepository(): Promise<void> {
    const path = grantsPath(this.env)
    const directory = nearestExisting(dirname(path))
    let root: string | undefined
    try {
      root = await this.git.root(directory)
    } catch (error) {
      // git could not answer, which is not the same as git saying there is no repository: refuse.
      throw new ModelGatewayError(
        'invalid-config',
        `${path}: cannot tell whether ${directory} is inside a git repository, so no grant is read or written: ${messageOf(error)}`,
      )
    }
    if (root === undefined) return
    throw new ModelGatewayError(
      'grants-in-repository',
      `${path} is inside the git repository ${root}: a grant lives in the machine configuration, never in a repository`,
    )
  }

  /** The grants the file holds; a file that is not there yet holds none. */
  private async load(): Promise<readonly Grant[]> {
    const path = grantsPath(this.env)
    let text: string
    try {
      text = await readFile(path, 'utf8')
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return []
      throw new ModelGatewayError('invalid-config', `${path}: cannot read the grants: ${messageOf(error)}`)
    }
    try {
      return parseGrantsFile(text)
    } catch (error) {
      throw new ModelGatewayError('invalid-config', `${path}: ${messageOf(error)}`)
    }
  }
}

/** The nearest directory at or above `path` that exists, which is the one git can be asked about. */
function nearestExisting(path: string): string {
  let current = path
  while (!existsSync(current) && dirname(current) !== current) current = dirname(current)
  return current
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
