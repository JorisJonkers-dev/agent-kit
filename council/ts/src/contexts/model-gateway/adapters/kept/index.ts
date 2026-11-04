import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname } from 'node:path'

import type { EnvPort } from '../../../../ports/index.js'
import { keptWorktreesPath } from '../../application/locations.js'
import type { KeptWorktree, ModelGatewayKeptPort } from '../../application/ports.js'
import { isObject, ModelGatewayError } from '../../domain/config.js'
import { FileLock, type LockTimings } from '../lock/index.js'

/** The version of the file's layout. */
const KEPT_VERSION = 1

/**
 * The kept-worktrees file: `{"version": 1, "worktrees": [...]}` in the
 * gateway's state directory. Every operation reads and rewrites it under an
 * exclusive lock beside it, so two gateway processes never lose each other's
 * entries. A file that is not there is an empty list; one that cannot be read
 * as the layout above is an `invalid-config` error naming the file, never a
 * silently empty list that would leave worktrees behind.
 */
export class NodeModelGatewayKept implements ModelGatewayKeptPort {
  private readonly lock: FileLock

  constructor(
    private readonly env: EnvPort,
    timings: LockTimings = {},
  ) {
    this.lock = new FileLock('kept-worktrees', timings)
  }

  async add(worktree: KeptWorktree): Promise<void> {
    await this.update((worktrees) => [...worktrees, worktree])
  }

  async list(): Promise<readonly KeptWorktree[]> {
    return this.update((worktrees) => worktrees, false)
  }

  async remove(runIds: readonly string[]): Promise<void> {
    await this.update((worktrees) => worktrees.filter((worktree) => !runIds.includes(worktree.run_id)))
  }

  /** Reads the list under the lock, and writes back what `change` makes of it unless the change is only a look. */
  private async update(change: (worktrees: readonly KeptWorktree[]) => readonly KeptWorktree[], write = true): Promise<readonly KeptWorktree[]> {
    const path = keptWorktreesPath(this.env)
    await mkdir(dirname(path), { recursive: true })
    return this.lock.run(`${path}.lock`, async () => {
      const changed = change(await this.read(path))
      if (write) await writeFile(path, `${JSON.stringify({ version: KEPT_VERSION, worktrees: changed }, null, 2)}\n`)
      return changed
    })
  }

  private async read(path: string): Promise<readonly KeptWorktree[]> {
    let text: string
    try {
      text = await readFile(path, 'utf8')
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return []
      throw error
    }
    let parsed: unknown
    try {
      parsed = JSON.parse(text)
    } catch (error) {
      throw new ModelGatewayError('invalid-config', `${path}: not JSON: ${error instanceof Error ? error.message : String(error)}`)
    }
    if (!isObject(parsed) || !Array.isArray(parsed.worktrees)) {
      throw new ModelGatewayError('invalid-config', `${path}: must be {"version": ${String(KEPT_VERSION)}, "worktrees": [...]}`)
    }
    return parsed.worktrees as KeptWorktree[]
  }
}
