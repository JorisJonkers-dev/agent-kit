import { readFile, readdir } from 'node:fs/promises'
import { basename, join } from 'node:path'

import type {
  LiveRunArtifacts,
  LiveRunDirReaderPort,
  NormalizedRunDirectory,
  RunStoreEvent,
} from '../../../../ports/index.js'
import type { RunState } from '../../../../shared-kernel/index.js'
import { RUNSTORE_EVENTS_FILE } from '../../../runstore/index.js'
import {
  RESULT_FILE,
  SUPERVISOR_FILE,
  WORKERS_DIR,
  assertLegacyReport,
  assertRecord,
  assertRunState,
  assertRunStoreEvent,
  assertTasks,
  assertWorkerResult,
  assertWorkerSupervisorSnapshot,
  copyOptionalInteger,
  copyOptionalString,
  isErrno,
  parseJson,
  type LegacyRunReport,
  type WorkerResult,
  type WorkerSupervisorSnapshot,
} from './artifact-codec.js'
import { normalizeLegacyRunDir } from './legacy-run-dir-normalizer.js'

export class FsLiveRunDirReader implements LiveRunDirReaderPort {
  async readRunDir(runDir: string): Promise<LiveRunArtifacts> {
    const workerResults = await readWorkerResults(runDir)
    const normalized = await readNormalizedSummary(runDir, workerResults)
    const events = await readEvents(runDir)
    const workerSupervisorSnapshots = await readWorkerSupervisorSnapshots(runDir)

    return {
      events,
      normalized,
      workerResults,
      workerSupervisorSnapshots,
    }
  }
}

async function readNormalizedSummary(
  runDir: string,
  workerResults: ReadonlyMap<string, WorkerResult>,
): Promise<NormalizedRunDirectory> {
  try {
    const normalized = await normalizeLegacyRunDir(runDir)
    return {
      report: normalized.report,
      runId: normalized.runId,
      state: normalized.state,
      tasks: normalized.tasks,
      workerResults,
    }
  } catch (error) {
    if (isMissingOptionalWorkerResult(error)) return readRequiredSummary(runDir, workerResults)
    throw error
  }
}

async function readRequiredSummary(
  runDir: string,
  workerResults: ReadonlyMap<string, WorkerResult>,
): Promise<NormalizedRunDirectory> {
  return {
    report: await readOptionalLegacyReport(join(runDir, 'report.json')),
    runId: basename(runDir),
    state: normalizeLegacyState(await readJson(join(runDir, 'state.json'))),
    tasks: assertTasks(await readJson(join(runDir, 'tasks.json'))),
    workerResults,
  }
}

async function readEvents(runDir: string): Promise<readonly RunStoreEvent[]> {
  try {
    const text = await readFile(join(runDir, RUNSTORE_EVENTS_FILE), 'utf8')
    return text
      .split('\n')
      .filter((line) => line.length > 0)
      .map((line) => assertRunStoreEvent(parseJson(line)))
  } catch (error) {
    if (isErrno(error, 'ENOENT')) return []
    throw error
  }
}

async function readWorkerResults(runDir: string): Promise<ReadonlyMap<string, WorkerResult>> {
  const results = new Map<string, WorkerResult>()
  for (const taskId of await readWorkerTaskIds(runDir)) {
    const result = await readOptionalWorkerResult(join(runDir, WORKERS_DIR, taskId, RESULT_FILE), taskId)
    if (result !== undefined) results.set(taskId, result)
  }
  return results
}

async function readWorkerSupervisorSnapshots(
  runDir: string,
): Promise<ReadonlyMap<string, WorkerSupervisorSnapshot>> {
  const snapshots = new Map<string, WorkerSupervisorSnapshot>()
  for (const taskId of await readWorkerTaskIds(runDir)) {
    const snapshot = await readOptionalWorkerSupervisorSnapshot(
      join(runDir, WORKERS_DIR, taskId, SUPERVISOR_FILE),
      taskId,
    )
    if (snapshot !== undefined) snapshots.set(taskId, snapshot)
  }
  return snapshots
}

async function readWorkerTaskIds(runDir: string): Promise<readonly string[]> {
  try {
    return (await readdir(join(runDir, WORKERS_DIR))).sort()
  } catch (error) {
    if (isErrno(error, 'ENOENT')) return []
    throw error
  }
}

async function readOptionalWorkerResult(
  path: string,
  taskId: string,
): Promise<WorkerResult | undefined> {
  try {
    return assertWorkerResult(await readJson(path), taskId)
  } catch (error) {
    if (isErrno(error, 'ENOENT')) return undefined
    throw error
  }
}

async function readOptionalWorkerSupervisorSnapshot(
  path: string,
  taskId: string,
): Promise<WorkerSupervisorSnapshot | undefined> {
  try {
    return assertWorkerSupervisorSnapshot(await readJson(path), taskId)
  } catch (error) {
    if (isErrno(error, 'ENOENT')) return undefined
    throw error
  }
}

async function readOptionalLegacyReport(path: string): Promise<LegacyRunReport | undefined> {
  try {
    return assertLegacyReport(await readJson(path))
  } catch (error) {
    if (isErrno(error, 'ENOENT')) return undefined
    /* c8 ignore next */
    throw error
  }
}

function normalizeLegacyState(value: unknown): RunState {
  const record = assertRecord(value, 'state')
  const state: Record<string, unknown> = {}
  copyOptionalString(record, state, 'stage')
  copyOptionalString(record, state, 'intensity')
  copyOptionalInteger(record, state, 'rounds')
  copyOptionalInteger(record, state, 'task_count')
  copyOptionalString(record, state, 'spec_id')
  copyOptionalString(record, state, 'spec_slug')
  copyOptionalString(record, state, 'spec_relpath')
  copyOptionalString(record, state, 'integration_branch')
  return assertRunState(state)
}

function isMissingOptionalWorkerResult(error: unknown): boolean {
  if (!isErrno(error, 'ENOENT') || !hasStringPath(error)) return false
  return isResultPath(error.path)
}

function hasStringPath(error: unknown): error is { readonly path: string } {
  return error instanceof Error && 'path' in error && typeof error.path === 'string'
}

function isResultPath(path: string): boolean {
  return path.includes(`${WORKERS_DIR}/`) && path.endsWith(`/${RESULT_FILE}`)
}

async function readJson(path: string): Promise<unknown> {
  return parseJson(await readFile(path, 'utf8'))
}
