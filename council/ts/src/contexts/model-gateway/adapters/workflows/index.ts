import { mkdir, readFile } from 'node:fs/promises'
import { dirname } from 'node:path'

import type { EnvPort } from '../../../../ports/index.js'
import { workflowsPath } from '../../application/locations.js'
import type { ModelGatewayWorkflowsPort } from '../../application/ports.js'
import { ModelGatewayError } from '../../domain/config.js'
import { parseWorkflowsFile, WORKFLOWS_VERSION, type WorkflowChange, type WorkflowRecord } from '../../domain/workflows.js'
import { writeFileAtomic } from '../fs/index.js'
import { FileLock, type LockTimings } from '../lock/index.js'

export type WorkflowLockTimings = LockTimings

/**
 * The workflows this machine has started, in one JSON file under the state
 * directory. A change reads the file, applies itself and writes the result
 * under an exclusive lock beside it, so two gateway processes starting or
 * ending workflows at once never lose each other's change. The write goes to a
 * temporary file that is renamed into place, so a reader never sees half a
 * file.
 */
export class NodeModelGatewayWorkflows implements ModelGatewayWorkflowsPort {
  private readonly lock: FileLock

  constructor(
    private readonly env: EnvPort,
    timings: WorkflowLockTimings = {},
  ) {
    this.lock = new FileLock('workflows', timings)
  }

  async read(): Promise<readonly WorkflowRecord[]> {
    return this.load()
  }

  async update(change: (workflows: readonly WorkflowRecord[]) => WorkflowChange): Promise<WorkflowRecord> {
    const path = workflowsPath(this.env)
    await mkdir(dirname(path), { recursive: true })
    return this.lock.run(`${path}.lock`, async () => {
      const changed = change(await this.load())
      await writeFileAtomic(path, `${JSON.stringify({ version: WORKFLOWS_VERSION, workflows: changed.workflows }, null, 2)}\n`)
      return changed.workflow
    })
  }

  /** The workflows the file holds; a file that is not there yet holds none. */
  private async load(): Promise<readonly WorkflowRecord[]> {
    const path = workflowsPath(this.env)
    let text: string
    try {
      text = await readFile(path, 'utf8')
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return []
      throw new ModelGatewayError('invalid-config', `${path}: cannot read the workflows: ${messageOf(error)}`)
    }
    try {
      return parseWorkflowsFile(text)
    } catch (error) {
      throw new ModelGatewayError('invalid-config', `${path}: ${messageOf(error)}`)
    }
  }
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
