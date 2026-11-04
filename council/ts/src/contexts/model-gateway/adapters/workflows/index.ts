import { readFile, rename, writeFile } from 'node:fs/promises'

import type { EnvPort } from '../../../../ports/index.js'
import { workflowsPath } from '../../application/locations.js'
import type { ModelGatewayWorkflowsPort } from '../../application/ports.js'
import { ModelGatewayError } from '../../domain/config.js'
import { parseWorkflowsFile, WORKFLOWS_VERSION, type WorkflowChange, type WorkflowRecord } from '../../domain/workflows.js'
import { withFileLock, type FileLockTimings } from '../fs/lock.js'

export type WorkflowLockTimings = FileLockTimings

/**
 * The workflows this machine has started, in one JSON file under the state
 * directory. A change reads the file, applies itself and writes the result
 * under an exclusive lock beside it, so two gateway processes starting or
 * ending workflows at once never lose each other's change. The write goes to a
 * temporary file that is renamed into place, so a reader never sees half a
 * file.
 */
export class NodeModelGatewayWorkflows implements ModelGatewayWorkflowsPort {
  constructor(
    private readonly env: EnvPort,
    private readonly timings: WorkflowLockTimings = {},
  ) {}

  async read(): Promise<readonly WorkflowRecord[]> {
    return this.load()
  }

  async update(change: (workflows: readonly WorkflowRecord[]) => WorkflowChange): Promise<WorkflowRecord> {
    const path = workflowsPath(this.env)
    return withFileLock(path, 'workflows', this.timings, async () => {
      const changed = change(await this.load())
      await this.save(path, changed.workflows)
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

  /** Writes the workflows through a temporary file, so a reader never sees half of one. */
  private async save(path: string, workflows: readonly WorkflowRecord[]): Promise<void> {
    const temporary = `${path}.tmp`
    await writeFile(temporary, `${JSON.stringify({ version: WORKFLOWS_VERSION, workflows }, null, 2)}\n`)
    await rename(temporary, path)
  }
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
