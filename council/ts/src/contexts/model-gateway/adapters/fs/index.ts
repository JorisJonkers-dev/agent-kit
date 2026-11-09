import { createHash, randomBytes } from 'node:crypto'
import { access, mkdir, mkdtemp, readFile, realpath, rename, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'

import type { FileStamp, ModelGatewayFilesPort } from '../../application/ports.js'

const ABSENT = new Set(['ENOENT', 'ENOTDIR'])

/**
 * Writes `content` to `path` in one step: a temporary file beside it, then a
 * rename over it. A write that fails leaves the file as it was, so a reader
 * never sees half of one, and a process that dies mid-write leaves the old
 * file rather than a truncated one. A `mode` makes the file — and the
 * temporary one it is written through — one the platform takes for the new
 * file's permissions.
 */
export async function writeFileAtomic(path: string, content: string, mode?: number): Promise<void> {
  const temp = `${path}.${randomBytes(6).toString('hex')}.tmp`
  try {
    await writeFile(temp, content, mode === undefined ? undefined : { mode })
    await rename(temp, path)
  } catch (error) {
    await rm(temp, { force: true })
    throw error
  }
}

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

  async writeText(path: string, content: string): Promise<void> {
    await mkdir(dirname(path), { recursive: true })
    await writeFile(path, content)
  }

  async mkdir(path: string): Promise<void> {
    await mkdir(path, { recursive: true })
  }

  async tempDir(prefix: string): Promise<string> {
    return mkdtemp(join(tmpdir(), prefix))
  }

  async remove(path: string): Promise<void> {
    await rm(path, { force: true, recursive: true })
  }

  async modifiedMs(path: string): Promise<number | undefined> {
    try {
      return (await stat(path)).mtimeMs
    } catch {
      return undefined
    }
  }

  async stamp(path: string): Promise<FileStamp | undefined> {
    try {
      const real = await realpath(path)
      const of = await stat(real)
      return {
        ctimeMs: of.ctimeMs,
        ino: of.ino,
        realpath: real,
        sha256: createHash('sha256').update(await readFile(real)).digest('hex'),
        size: of.size,
      }
    } catch {
      return undefined
    }
  }

  async writeTextPrivate(path: string, content: string): Promise<void> {
    await mkdir(dirname(path), { recursive: true, mode: 0o700 })
    await writeFileAtomic(path, content, 0o600)
  }
}
