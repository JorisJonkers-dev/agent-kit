import { access, readFile } from 'node:fs/promises'

import type { ModelGatewayFilesPort } from '../../application/ports.js'

const ABSENT = new Set(['ENOENT', 'ENOTDIR'])

export class NodeModelGatewayFiles implements ModelGatewayFilesPort {
  async readText(path: string): Promise<string> {
    return readFile(path, 'utf8')
  }

  /** False only when the file is not there; any other failure to look is thrown. */
  async exists(path: string): Promise<boolean> {
    try {
      await access(path)
      return true
    } catch (error) {
      if (ABSENT.has((error as NodeJS.ErrnoException).code ?? '')) return false
      throw error
    }
  }
}
