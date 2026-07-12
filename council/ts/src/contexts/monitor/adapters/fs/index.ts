import { mkdir, open, readdir, readFile, rename } from 'node:fs/promises'
import { constants } from 'node:fs'
import { basename, dirname, join } from 'node:path'

import type { MonitorState } from '../../domain/index.js'

const MONITORS_DIR = 'monitors'

export interface MonitorFsAdapter {
  readState(execDir: string, name: string): Promise<MonitorState>
  writeState(execDir: string, state: MonitorState): Promise<void>
  listStates(execDir: string): Promise<readonly MonitorState[]>
}

export function createMonitorFsAdapter(): MonitorFsAdapter {
  return {
    readState,
    writeState,
    listStates,
  }
}

async function readState(execDir: string, name: string): Promise<MonitorState> {
  const filePath = monitorFilePath(execDir, name)
  const text = await readFile(filePath, 'utf8')
  return parseMonitorState(text)
}

async function writeState(execDir: string, state: MonitorState): Promise<void> {
  const finalPath = monitorFilePath(execDir, state.name)
  const tempPath = join(dirname(finalPath), `.${basename(finalPath)}.tmp`)
  await mkdir(dirname(finalPath), { recursive: true })
  const bytes = `${JSON.stringify(state, null, 2)}\n`
  const handle = await open(tempPath, constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY)
  try {
    await handle.write(bytes, 0, 'utf8')
    await handle.sync()
  } finally {
    await handle.close()
  }
  await rename(tempPath, finalPath)
}

async function listStates(execDir: string): Promise<readonly MonitorState[]> {
  const dir = join(execDir, MONITORS_DIR)
  let entries: string[]
  try {
    entries = await readdir(dir)
  } catch (error) {
    if (isErrno(error, 'ENOENT')) return []
    /* c8 ignore next -- defensive guard for unexpected OS errors */
    throw error
  }
  const states: MonitorState[] = []
  for (const entry of entries) {
    if (!entry.endsWith('.json')) continue
    try {
      const text = await readFile(join(dir, entry), 'utf8')
      states.push(parseMonitorState(text))
    } catch {
      // skip unreadable files
    }
  }
  return states
}

function monitorFilePath(execDir: string, name: string): string {
  return join(execDir, MONITORS_DIR, `${name}.json`)
}

function parseMonitorState(text: string): MonitorState {
  const value: unknown = JSON.parse(text)
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error('invalid monitor state: not an object')
  }
  const obj = value as Record<string, unknown>
  assertString(obj, 'name')
  assertString(obj, 'status')
  assertString(obj, 'startedAt')
  assertString(obj, 'deadline')
  assertString(obj, 'lastTickAt')
  assertString(obj, 'lastOutput')
  assertNumber(obj, 'intervalMs')
  assertString(obj, 'cmd')
  assertString(obj, 'until')
  assertString(obj, 'then')
  return obj as unknown as MonitorState
}

function assertString(obj: Record<string, unknown>, key: string): void {
  if (typeof obj[key] !== 'string') throw new Error(`invalid monitor state: ${key} must be a string`)
}

function assertNumber(obj: Record<string, unknown>, key: string): void {
  if (typeof obj[key] !== 'number') throw new Error(`invalid monitor state: ${key} must be a number`)
}

function isErrno(error: unknown, code: string): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    (error as { readonly code?: unknown }).code === code
  )
}
