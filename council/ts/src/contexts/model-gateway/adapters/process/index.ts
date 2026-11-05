import { constants } from 'node:fs'
import { access } from 'node:fs/promises'
import { delimiter, join } from 'node:path'

import { NodeProcessAdapter } from '../../../../adapters/process/node-process.js'
import type { ProcessCommand, ProcessResult } from '../../../../ports/index.js'
import type { ModelGatewayProcessPort } from '../../application/ports.js'

/**
 * Finds out whether a program is installed by looking for an executable file
 * of that name in each folder of PATH. It starts no program at all: neither
 * the client's own command nor a lookup helper such as `which`. The one
 * program it does run is a client's own listing command, through `exec`.
 */
export class NodeModelGatewayProcess implements ModelGatewayProcessPort {
  private readonly adapter = new NodeProcessAdapter()

  constructor(private readonly path: string = process.env.PATH ?? '') {}

  async which(binary: string): Promise<boolean> {
    for (const folder of this.path.split(delimiter)) {
      if (folder === '') continue
      try {
        await access(join(folder, binary), constants.X_OK)
        return true
      } catch {
        // Not here, or not executable here: PATH lookup moves on to the next folder.
      }
    }
    return false
  }

  async exec(command: ProcessCommand): Promise<ProcessResult> {
    return this.adapter.exec(command)
  }
}
