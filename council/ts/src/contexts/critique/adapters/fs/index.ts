import { access, appendFile, mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import { dirname, join, relative } from 'node:path'

import type { CritiqueFilesPort } from '../../application/ports.js'

export class NodeCritiqueFiles implements CritiqueFilesPort {
  async readText(path: string): Promise<string> {
    return readFile(path, 'utf8')
  }

  async readBytes(path: string): Promise<Uint8Array> {
    return readFile(path)
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

  async listFiles(path: string): Promise<readonly string[]> {
    const entries = await readdir(path, { withFileTypes: true }).catch(() => [])
    return entries
      .filter((entry) => entry.isFile())
      .map((entry) => entry.name)
      .sort()
  }

  async listDirectories(path: string): Promise<readonly string[]> {
    const entries = await readdir(path, { withFileTypes: true }).catch(() => [])
    return entries
      .filter((entry) => entry.isDirectory())
      .map((entry) => entry.name)
      .sort()
  }

  async listTree(dir: string): Promise<readonly string[]> {
    if (!(await this.exists(dir))) return []
    const entries = await readdir(dir, { recursive: true, withFileTypes: true })
    return entries.filter((entry) => !entry.isDirectory()).map((entry) => relative(dir, join(entry.parentPath, entry.name)))
  }

  async remove(path: string): Promise<void> {
    await rm(path, { force: true })
  }
}
