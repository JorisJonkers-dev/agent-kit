import { randomBytes } from 'node:crypto'
import { mkdir, readdir, readFile, rename, rm, writeFile } from 'node:fs/promises'
import { basename, dirname, join } from 'node:path'

import type { BuildTrackerFilesPort } from '../../application/ports.js'

export class NodeBuildTrackerFiles implements BuildTrackerFilesPort {
  async readText(path: string): Promise<string> {
    return readFile(path, 'utf8')
  }

  async listFiles(dir: string): Promise<readonly string[]> {
    const entries = await readdir(dir, { withFileTypes: true })
    return entries.filter((entry) => entry.isFile()).map((entry) => entry.name)
  }

  /**
   * Writes a temporary file in the same directory, then renames it over the
   * target, so a reader sees the old file or the new one and never half of
   * either. A failed write removes its temporary file.
   */
  async writeAtomic(path: string, content: string): Promise<void> {
    const dir = dirname(path)
    await mkdir(dir, { recursive: true })
    const temporary = join(dir, `.${basename(path)}.${randomBytes(6).toString('hex')}.tmp`)
    try {
      await writeFile(temporary, content)
      await rename(temporary, path)
    } catch (error) {
      await rm(temporary, { force: true })
      throw error
    }
  }
}
