import { access, appendFile, mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname } from 'node:path'

import type { CritiqueFilesPort } from '../../application/ports.js'

export class NodeCritiqueFiles implements CritiqueFilesPort {
  async readText(path: string): Promise<string> {
    return readFile(path, 'utf8')
  }

  async writeText(path: string, content: string): Promise<void> {
    await mkdir(dirname(path), { recursive: true })
    await writeFile(path, content)
  }

  async appendText(path: string, content: string): Promise<void> {
    await mkdir(dirname(path), { recursive: true })
    await appendFile(path, content)
  }

  async exists(path: string): Promise<boolean> {
    return access(path).then(
      () => true,
      () => false,
    )
  }
}
