import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import type { RunState, Task } from '../../../../shared-kernel/index.js'
import { FsLiveRunDirReader, type WorkerResult, type WorkerSupervisorSnapshot } from './index.js'

const tempRoots: string[] = []

afterEach(async () => {
  await Promise.all(tempRoots.splice(0).map((root) => rm(root, { force: true, recursive: true })))
})

describe('FsLiveRunDirReader', () => {
  it('loads a complete run directory with deterministic worker artifacts and raw event order', async () => {
    const runDir = await writeRunDir('complete')
    const reader = new FsLiveRunDirReader()
    const secondEvent = {
      payload: { id: 'A1', summary: 'amended' },
      type: 'amendment',
    } as const
    const firstEvent = {
      payload: {
        byte_count: 3,
        offset: 0,
        stream: 'stdout',
        task_id: 'T2',
        worker_id: 'worker-T2',
      },
      type: 'worker_output',
    } as const
    await writeJsonl(runDir, 'events.jsonl', [firstEvent, secondEvent])
    await writeWorkerResult(runDir, 'T2', fullWorkerResult('T2'))
    await writeWorkerResult(runDir, 'T1', fullWorkerResult('T1'))
    await writeSupervisorSnapshot(runDir, 'T2', fullSupervisorSnapshot('T2'))
    await writeSupervisorSnapshot(runDir, 'T1', fullSupervisorSnapshot('T1'))

    const artifacts = await reader.readRunDir(runDir)

    expect(artifacts.normalized.runId).toBe('complete')
    expect(artifacts.normalized.state).toEqual(normalizedState())
    expect(artifacts.normalized.tasks).toEqual(fullTasks())
    expect(artifacts.normalized.report).toEqual({
      run: 'complete',
      tasks: [{ status: 'ok', task_id: 'T1' }],
      waves: [['T1']],
    })
    expect(artifacts.events).toEqual([firstEvent, secondEvent])
    expect([...artifacts.workerResults.keys()]).toEqual(['T1', 'T2'])
    expect([...artifacts.workerSupervisorSnapshots.keys()]).toEqual(['T1', 'T2'])
    expect(artifacts.normalized.workerResults).toBe(artifacts.workerResults)
  })

  it('treats a missing events file as an empty partial event stream', async () => {
    const runDir = await writeRunDir('missing-events')
    const reader = new FsLiveRunDirReader()

    await expect(reader.readRunDir(runDir)).resolves.toMatchObject({
      events: [],
    })
  })

  it('treats a missing workers directory as empty partial worker artifacts', async () => {
    const runDir = await writeRunDir('missing-workers')
    const reader = new FsLiveRunDirReader()

    const artifacts = await reader.readRunDir(runDir)

    expect(artifacts.workerResults.size).toBe(0)
    expect(artifacts.workerSupervisorSnapshots.size).toBe(0)
    expect(artifacts.normalized.workerResults.size).toBe(0)
  })

  it('skips a missing individual supervisor snapshot without dropping the worker result', async () => {
    const runDir = await writeRunDir('missing-supervisor')
    const reader = new FsLiveRunDirReader()
    await writeWorkerResult(runDir, 'T1', fullWorkerResult('T1'))

    const artifacts = await reader.readRunDir(runDir)

    expect([...artifacts.workerResults.keys()]).toEqual(['T1'])
    expect(artifacts.workerSupervisorSnapshots.size).toBe(0)
  })

  it('skips a missing individual worker result without dropping the supervisor snapshot', async () => {
    const runDir = await writeRunDir('missing-result')
    const reader = new FsLiveRunDirReader()
    await writeSupervisorSnapshot(runDir, 'T1', fullSupervisorSnapshot('T1'))
    await rm(join(runDir, 'report.json'), { force: true })

    const artifacts = await reader.readRunDir(runDir)

    expect(artifacts.normalized.report).toBeUndefined()
    expect(artifacts.workerResults.size).toBe(0)
    expect([...artifacts.workerSupervisorSnapshots.keys()]).toEqual(['T1'])
    expect(artifacts.normalized.workerResults.size).toBe(0)
  })

  it('fails malformed required state and tasks artifacts', async () => {
    const malformedState = await writeRunDir('bad-state')
    const malformedTasks = await writeRunDir('bad-tasks')
    const reader = new FsLiveRunDirReader()
    await writeJson(malformedState, 'state.json', [])
    await writeJson(malformedTasks, 'tasks.json', [])

    await expect(reader.readRunDir(malformedState)).rejects.toThrow('state must be an object')
    await expect(reader.readRunDir(malformedTasks)).rejects.toThrow('consolidator returned no tasks')
  })

  it('surfaces malformed optional artifacts when the file exists', async () => {
    const badEvent = await writeRunDir('bad-event')
    const badSupervisor = await writeRunDir('bad-supervisor')
    const badResult = await writeRunDir('bad-result')
    const reader = new FsLiveRunDirReader()
    await writeJsonl(badEvent, 'events.jsonl', [{ type: 'unknown' }])
    await writeJson(join(badSupervisor, 'workers', 'T1'), 'supervisor.json', fullSupervisorSnapshot('T2'))
    await writeJson(join(badResult, 'workers', 'T1'), 'result.json', { status: 'ok' })

    await expect(reader.readRunDir(badEvent)).rejects.toThrow('unsupported run store event type: unknown')
    await expect(reader.readRunDir(badSupervisor)).rejects.toThrow(
      'worker supervisor snapshot task_id must match path task id: T1',
    )
    await expect(reader.readRunDir(badResult)).rejects.toThrow('worker result.task_id must be a string')
  })

  it('surfaces unexpected worker directory read errors', async () => {
    const runDir = await writeRunDir('workers-file')
    const reportDir = await writeRunDir('report-dir')
    const reader = new FsLiveRunDirReader()
    await writeFile(join(runDir, 'workers'), '', 'utf8')
    await writeSupervisorSnapshot(reportDir, 'T1', fullSupervisorSnapshot('T1'))
    await rm(join(reportDir, 'report.json'), { force: true })
    await mkdir(join(reportDir, 'report.json'))

    await expect(reader.readRunDir(runDir)).rejects.toMatchObject({ code: 'ENOTDIR' })
    await expect(reader.readRunDir(reportDir)).rejects.toMatchObject({ code: 'EISDIR' })
  })
})

async function writeRunDir(runId: string): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), 'council-live-run-'))
  tempRoots.push(root)
  const runDir = join(root, runId)
  await writeJson(runDir, 'state.json', fullState())
  await writeJson(runDir, 'tasks.json', fullTasks())
  await writeJson(runDir, 'report.json', {
    run: runId,
    tasks: [{ status: 'ok', task_id: 'T1' }],
    waves: [['T1']],
  })
  return runDir
}

async function writeWorkerResult(runDir: string, taskId: string, result: WorkerResult): Promise<void> {
  await writeJson(join(runDir, 'workers', taskId), 'result.json', result)
}

async function writeSupervisorSnapshot(
  runDir: string,
  taskId: string,
  snapshot: WorkerSupervisorSnapshot,
): Promise<void> {
  await writeJson(join(runDir, 'workers', taskId), 'supervisor.json', snapshot)
}

async function writeJson(runDir: string, file: string, value: unknown): Promise<void> {
  await mkdir(runDir, { recursive: true })
  await writeFile(join(runDir, file), `${JSON.stringify(value, null, 2)}\n`, 'utf8')
}

async function writeJsonl(runDir: string, file: string, values: readonly unknown[]): Promise<void> {
  await writeFile(join(runDir, file), `${values.map((value) => JSON.stringify(value)).join('\n')}\n`, 'utf8')
}

function fullState(): RunState {
  return {
    agents: ['planner-a'],
    content_hash: 'sha256:state',
    engine: { cli: 'codex', model: 'gpt-5' },
    integration_branch: 'council/run-a/integration',
    intensity: 'quick',
    model_tier: 'frontier',
    rounds: 1,
    spec_id: '001-run-a',
    spec_relpath: 'specs/001-run-a',
    spec_slug: 'run-a',
    stage: 'planned',
    task_count: 1,
  }
}

function normalizedState(): RunState {
  return {
    integration_branch: 'council/run-a/integration',
    intensity: 'quick',
    rounds: 1,
    spec_id: '001-run-a',
    spec_relpath: 'specs/001-run-a',
    spec_slug: 'run-a',
    stage: 'planned',
    task_count: 1,
  }
}

function fullTasks(): readonly Task[] {
  return [
    {
      acceptance_criteria: ['Criterion'],
      archetype: 'implementation',
      boundaries: 'Only src/example.ts.',
      content_hash: 'sha256:task',
      context_profile: 'code',
      context_refs: ['kb://task'],
      depends_on: [],
      difficulty: 'moderate',
      id: 'T1',
      model: 'sonnet',
      model_tier: 'standard',
      objective: 'Implement the example behavior.',
      output_format: 'Patch',
      paths: ['src/example.ts'],
      supersedes: [],
      title: 'Example task',
      verify: 'npm test',
    },
  ]
}

function fullWorkerResult(taskId: string): WorkerResult {
  return {
    files_changed: ['src/example.ts'],
    status: 'ok',
    task_id: taskId,
    verify_rc: 0,
  }
}

function fullSupervisorSnapshot(taskId: string): WorkerSupervisorSnapshot {
  return {
    attempt_id: 1,
    logs: {
      stderr: `workers/${taskId}/logs/stderr.log`,
      stdout: `workers/${taskId}/logs/stdout.log`,
    },
    offsets: {
      stderr: 0,
      stdout: 0,
    },
    restart_count: 0,
    status: 'running',
    task_id: taskId,
    watchdog: {
      handling_detection: false,
      loop: {
        actions: [],
      },
      progress: {
        attemptStartedAtMs: 0,
        lastActionAtMs: 0,
        lastOutputAtMs: 0,
        lastProgressAtMs: 0,
        outputBytes: 0,
        startedAtMs: 0,
      },
      retry: {
        attempts: 0,
        failureFingerprints: [],
      },
    },
  }
}
