import { constants } from 'node:fs'
import { access } from 'node:fs/promises'
import { delimiter, join } from 'node:path'

import type { ModelGatewayProcessPort } from '../../application/ports.js'

/**
 * Finds out whether a program is installed by looking for an executable file
 * of that name in each folder of PATH. It starts no program at all: neither
 * the client's own command nor a lookup helper such as `which`.
 */
export class NodeModelGatewayProcess implements ModelGatewayProcessPort {
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
}
