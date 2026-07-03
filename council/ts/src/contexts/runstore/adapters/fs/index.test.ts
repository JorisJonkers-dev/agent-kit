import { open, readFile, readdir, unlink, appendFile, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'

import { afterEach, describe, expect, it } from 'vitest'

import {
  workerDetectedEvent,
  workerExitedEvent,
  workerFinishedEvent,
  workerOutputEvent,
  workerRestartedEvent,
  workerStartedEvent,
} from '../../../runstore/index.js'
import type { ClockPort } from '../../../../ports/index.js'
import type { DesignLedger, RunState, Story, Task } from '../../../../shared-kernel/index.js'
import { FsRunStoreAdapter, normalizeLegacyRunDir, type WorkerResult } from './index.js'
import type { WorkerSupervisorSnapshot } from './artifact-codec.js'

const tempRoots: string[] = []

afterEach(async () => {
  await Promise.all(tempRoots.splice(0).map((root) => rm(root, { force: true, recursive: true })))
})

class TimeoutClock implements ClockPort {
  private elapsed = 0

  now(): Date {
    return new Date(0)
  }

  monotonicMs(): number {
    this.elapsed += 10
    return this.elapsed
  }

  async sleep(): Promise<void> {
    await Promise.resolve()
  }
}

describe('FsRunStoreAdapter', () => {
  it('atomically writes and validates state, tasks, story, design ledger, and worker results', async () => {
    const root = await tempRoot()
    const store = new FsRunStoreAdapter(root)
    const state = fullState()
    const tasks = fullTasks()
    const story = fullStory()
    const ledger = fullLedger()
    const result = fullWorkerResult('T1')
    const snapshot = fullSupervisorSnapshot('T1')

    await store.writeState('run-a', state)
    await store.writeTasks('run-a', tasks)
    await store.writeStory('run-a', story)
    await store.writeDesignLedger('run-a', ledger)
    await store.writeWorkerResult('run-a', 'T1', result)
    await store.writeWorkerSupervisorSnapshot('run-a', 'T1', snapshot)

    await expect(store.readState('run-a')).resolves.toEqual(state)
    await expect(store.readTasks('run-a')).resolves.toEqual(tasks)
    await expect(store.readStory('run-a')).resolves.toEqual(story)
    await expect(store.readDesignLedger('run-a')).resolves.toEqual(ledger)
    await expect(store.readWorkerResult('run-a', 'T1')).resolves.toEqual(result)
    await expect(store.readWorkerSupervisorSnapshot('run-a', 'T1')).resolves.toEqual(snapshot)
    await expect(readFile(join(root, 'run-a', 'state.json'), 'utf8')).resolves.toBe(
      `${JSON.stringify(state, null, 2)}\n`,
    )
    await expect(readFile(join(root, 'run-a', 'workers', 'T1', 'supervisor.json'), 'utf8')).resolves.toBe(
      `${JSON.stringify(snapshot, null, 2)}\n`,
    )
    const runFiles = await readdir(join(root, 'run-a'))
    expect(runFiles.filter((file) => file.includes('.tmp'))).toEqual([])
    await expect(readdir(join(root, 'run-a', 'workers', 'T1'))).resolves.toEqual([
      'result.json',
      'supervisor.json',
    ])
  })

  it('appends events through the lock and reads them back in order', async () => {
    const root = await tempRoot()
    const store = new FsRunStoreAdapter(root)
    const review = {
      content_hash: 'sha256:review',
      engine: { cli: 'codex', model: 'gpt-5' },
      issues: [],
      model_tier: 'frontier',
      reasons: 'ok',
      reviewer: 'verifier',
      satisfied: true,
      task_id: 'T1',
    } as const
    const routing = {
      candidate_routes: ['done'],
      content_hash: 'sha256:routing',
      context_refs: ['kb://route'],
      engine: { cli: 'claude', model: 'sonnet' },
      model_tier: 'standard',
      reasons: 'complete',
      route: 'done',
      task_id: 'T1',
    } as const
    const amendment = {
      content_hash: 'sha256:amend',
      context_refs: ['kb://amend'],
      discovered_from: 'T1',
      engine: { cli: 'codex', model: 'gpt-5' },
      id: 'A1',
      model_tier: 'frontier',
      reason: 'extra work',
      status: 'accepted',
      summary: 'Add follow-up',
      supersedes: ['A0'],
      task_refs: ['T1'],
    } as const

    await store.appendReviewVerdict('run-a', review)
    await store.appendRoutingVerdict('run-a', routing)
    await store.appendAmendment('run-a', amendment)

    await expect(store.readEvents('run-a')).resolves.toEqual([
      { payload: review, type: 'review_verdict' },
      { payload: routing, type: 'routing_verdict' },
      { payload: amendment, type: 'amendment' },
    ])
    await expect(readdir(join(root, 'run-a'))).resolves.not.toContain('events.jsonl.lock')
  })

  it('appends worker lifecycle events through the generic worker event port', async () => {
    const root = await tempRoot()
    const store = new FsRunStoreAdapter(root)
    const events = [
      workerStartedEvent({
        attempt: 1,
        command: ['npm', 'test'],
        content_hash: 'sha256:started',
        cwd: '/work/run-a',
        engine: { cli: 'codex', model: 'gpt-5' },
        model_tier: 'frontier',
        pid: 101,
        started_at: '2026-07-03T10:00:00.000Z',
        task_id: 'T1',
        worker_id: 'worker-T1',
      }),
      workerOutputEvent({
        byte_count: 128,
        content_hash: 'sha256:output-event',
        log_path: 'workers/T1/logs/stdout.log',
        offset: 256,
        sha256: 'sha256:chunk',
        stream: 'stdout',
        tail: 'last line',
        tail_bytes: 9,
        task_id: 'T1',
        worker_id: 'worker-T1',
      } as never),
      workerDetectedEvent({
        content_hash: 'sha256:detected',
        detected_at: '2026-07-03T10:01:00.000Z',
        pid: 101,
        status: 'running',
        task_id: 'T1',
        worker_id: 'worker-T1',
      }),
      workerRestartedEvent({
        attempt: 2,
        content_hash: 'sha256:restarted',
        pid: 202,
        previous_pid: 101,
        reason: 'stale heartbeat',
        restarted_at: '2026-07-03T10:02:00.000Z',
        task_id: 'T1',
        worker_id: 'worker-T1',
      }),
      workerExitedEvent({
        content_hash: 'sha256:exited',
        duration_ms: 3000,
        exit_code: 0,
        exited_at: '2026-07-03T10:03:00.000Z',
        pid: 202,
        signal: null,
        task_id: 'T1',
        worker_id: 'worker-T1',
      }),
      workerFinishedEvent({
        content_hash: 'sha256:finished',
        duration_ms: 3100,
        finished_at: '2026-07-03T10:03:01.000Z',
        result_path: 'workers/T1/result.json',
        status: 'ok',
        task_id: 'T1',
        worker_id: 'worker-T1',
      }),
    ] as const

    for (const event of events) await store.appendWorkerEvent('run-a', event)

    await expect(store.readEvents('run-a')).resolves.toEqual(events)
    await expect(readFile(join(root, 'run-a', 'events.jsonl'), 'utf8')).resolves.toBe(
      events.map((event) => JSON.stringify(event)).join('\n') + '\n',
    )
    await expect(readdir(join(root, 'run-a'))).resolves.not.toContain('events.jsonl.lock')
  })

  it('waits for an existing event lock and times out stale locks', async () => {
    const root = await tempRoot()
    const store = new FsRunStoreAdapter(root, { lockRetryMs: 1, lockTimeoutMs: 250 })
    await mkdir(join(root, 'run-a'), { recursive: true })
    const lock = await open(join(root, 'run-a', 'events.jsonl.lock'), 'wx')
    await lock.close()

    const append = store.appendAmendment('run-a', { id: 'A1', summary: 'after lock' })
    setTimeout(() => {
      void unlink(join(root, 'run-a', 'events.jsonl.lock'))
    }, 5)
    await expect(append).resolves.toBeUndefined()

    const timeoutStore = new FsRunStoreAdapter(root, {
      clock: new TimeoutClock(),
      lockRetryMs: 1,
      lockTimeoutMs: 0,
    })
    const stale = await open(join(root, 'run-a', 'events.jsonl.lock'), 'wx')
    await stale.close()
    await expect(timeoutStore.appendAmendment('run-a', { id: 'A2', summary: 'blocked' })).rejects.toMatchObject({
      code: 'EEXIST',
    })
    await unlink(join(root, 'run-a', 'events.jsonl.lock'))
  })

  it('rejects unsafe paths and invalid schema shapes', async () => {
    const root = await tempRoot()
    const store = new FsRunStoreAdapter(root)

    await expect(store.writeState('../run', fullState())).rejects.toThrow(
      'runId must be a single path segment',
    )
    await expect(store.writeState('', fullState())).rejects.toThrow('runId must not be empty')
    await expect(store.writeStory('', fullStory())).rejects.toThrow('runId must not be empty')
    await expect(store.writeWorkerResult('run-a', 'T/1', fullWorkerResult('T/1'))).rejects.toThrow(
      'taskId must be a single path segment',
    )
    await expect(store.writeState('run-a', { stage: 1 } as unknown as RunState)).rejects.toThrow(
      'state.stage must be a string',
    )
    await expect(
      store.writeState('run-a', { ...fullState(), rounds: '2' } as unknown as RunState),
    ).rejects.toThrow('state.rounds must be an integer')
    await expect(
      store.writeState('run-a', { planner_a: 'claude:opus' } as unknown as RunState),
    ).rejects.toThrow('state.planner_a is not allowed')
    await expect(store.writeTasks('run-a', [])).rejects.toThrow(
      'consolidator returned no tasks',
    )
    await expect(
      store.writeStory('run-a', { ...fullStory(), acceptance_criteria: [1] } as unknown as Story),
    ).rejects.toThrow('story.acceptance_criteria must be an array of strings')
    await expect(
      store.writeStory('run-a', { ...fullStory(), definition_of_done: 'done' } as unknown as Story),
    ).rejects.toThrow('story.definition_of_done must be an array')
    await expect(store.writeDesignLedger('run-a', [] as unknown as DesignLedger)).rejects.toThrow(
      'design ledger must be an object',
    )
    await expect(
      store.writeWorkerResult('run-a', 'T1', { task_id: 'T2', status: 'ok' }),
    ).rejects.toThrow('worker result task_id must match path task id: T1')
    await expect(
      store.writeWorkerResult('run-a', 'T1', { status: 'ok' } as unknown as WorkerResult),
    ).rejects.toThrow('worker result.task_id must be a string')
    await expect(
      store.writeWorkerResult('run-a', 'T1', {
        status: 'ok',
        suggested_model: 'bad',
        task_id: 'T1',
      } as unknown as WorkerResult),
    ).rejects.toThrow('worker result.suggested_model must be one of: haiku, sonnet, opus')
    await expect(
      store.writeWorkerResult('run-a', 'T1', {
        status: 'ok',
        task_id: 'T1',
        verify_rc: 'bad',
      } as unknown as WorkerResult),
    ).rejects.toThrow('worker result.verify_rc must be an integer or null')
    await expect(
      store.writeWorkerResult('run-a', 'T1', {
        status: 'ok',
        task_id: 'T1',
        verdict: { issues: [], reasons: 'bad', satisfied: 'yes' },
      } as unknown as WorkerResult),
    ).rejects.toThrow('review verdict.satisfied must be a boolean')
    await expect(
      store.writeWorkerResult('run-a', 'T1', {
        status: 'ok',
        stdout_tail: 'x'.repeat(4097),
        task_id: 'T1',
      }),
    ).rejects.toThrow('worker result.stdout_tail must be at most 4096 characters')
    await expect(
      store.writeWorkerResult('run-a', 'T1', {
        status: 'ok',
        stdout_bytes: -1,
        task_id: 'T1',
      }),
    ).rejects.toThrow('worker result.stdout_bytes must be a non-negative integer')
    await expect(
      store.writeWorkerResult('run-a', 'T1', {
        status: 'ok',
        stdout_log_path: 1,
        task_id: 'T1',
      } as unknown as WorkerResult),
    ).rejects.toThrow('worker result.stdout_log_path must be a string')
    await expect(
      store.writeWorkerResult('run-a', 'T1', {
        committed: 'yes',
        status: 'ok',
        task_id: 'T1',
      } as unknown as WorkerResult),
    ).rejects.toThrow('worker result.committed must be a boolean')
    await expect(
      store.writeWorkerSupervisorSnapshot('run-a', 'T/1', fullSupervisorSnapshot('T/1')),
    ).rejects.toThrow('taskId must be a single path segment')
    await expect(
      store.writeWorkerSupervisorSnapshot('run-a', 'T1', fullSupervisorSnapshot('T2')),
    ).rejects.toThrow('worker supervisor snapshot task_id must match path task id: T1')
    await expect(
      store.writeWorkerSupervisorSnapshot('run-a', 'T1', {
        ...fullSupervisorSnapshot('T1'),
        attempt_id: -1,
      }),
    ).rejects.toThrow('worker supervisor snapshot.attempt_id must be a non-negative integer')
    await expect(
      store.writeWorkerSupervisorSnapshot('run-a', 'T1', {
        ...fullSupervisorSnapshot('T1'),
        offsets: { stderr: 0, stdout: -1 },
      }),
    ).rejects.toThrow('worker supervisor snapshot.offsets.stdout must be a non-negative integer')
    await expect(
      store.writeWorkerSupervisorSnapshot('run-a', 'T1', {
        ...fullSupervisorSnapshot('T1'),
        status: 'unknown',
      } as unknown as WorkerSupervisorSnapshot),
    ).rejects.toThrow('worker supervisor snapshot.status must be one of:')
    await expect(
      store.writeWorkerSupervisorSnapshot('run-a', 'T1', {
        ...fullSupervisorSnapshot('T1'),
        watchdog: {
          ...fullSupervisorSnapshot('T1').watchdog,
          progress: { outputBytes: 'bad' },
        },
      } as unknown as WorkerSupervisorSnapshot),
    ).rejects.toThrow('worker supervisor snapshot.watchdog.progress.attemptStartedAtMs must be a non-negative integer')
    await expect(
      store.appendWorkerEvent('run-a', {
        payload: { status: 'ok', worker_id: 'worker-T1' },
        type: 'worker_finished',
      } as never),
    ).rejects.toThrow('worker finished.task_id must be a string')
    await expect(
      store.appendWorkerEvent('run-a', {
        payload: { byte_count: 1, offset: 0, stream: 'stdin', worker_id: 'worker-T1' },
        type: 'worker_output',
      } as never),
    ).rejects.toThrow('worker output.stream must be one of: stdout, stderr')
    await expect(
      store.appendWorkerEvent('run-a', {
        payload: { byte_count: -1, offset: 0, stream: 'stdout', worker_id: 'worker-T1' },
        type: 'worker_output',
      } as never),
    ).rejects.toThrow('worker output.byte_count must be a non-negative integer')
    await expect(
      store.appendWorkerEvent('run-a', {
        payload: {
          byte_count: 4097,
          offset: 0,
          stream: 'stdout',
          tail: 'x'.repeat(4097),
          tail_bytes: 4097,
          worker_id: 'worker-T1',
        },
        type: 'worker_output',
      } as never),
    ).rejects.toThrow('worker output.tail must be at most 4096 characters')
    await expect(
      store.appendWorkerEvent('run-a', {
        payload: { attempt: -1, worker_id: 'worker-T1' },
        type: 'worker_started',
      } as never),
    ).rejects.toThrow('worker started.attempt must be a non-negative integer')
    await expect(
      store.appendWorkerEvent('run-a', {
        payload: { exit_code: 'ok', worker_id: 'worker-T1' },
        type: 'worker_exited',
      } as never),
    ).rejects.toThrow('worker exited.exit_code must be an integer or null')
    await expect(
      store.appendWorkerEvent('run-a', {
        payload: { exit_code: 0, signal: 9, worker_id: 'worker-T1' },
        type: 'worker_exited',
      } as never),
    ).rejects.toThrow('worker exited.signal must be a string or null')
    await expect(
      store.appendWorkerEvent('run-a', {
        payload: {
          byte_count: 1,
          offset: 0,
          stream: 'stdout',
          text: 'full output',
          worker_id: 'worker-T1',
        },
        type: 'worker_output',
      } as never),
    ).rejects.toThrow('worker output.text is not allowed')
    await expect(
      store.appendWorkerEvent('run-a', { payload: { id: 'A1', summary: 'not worker' }, type: 'amendment' } as never),
    ).rejects.toThrow('worker event type is required')

    await mkdir(join(root, 'run-b'), { recursive: true })
    await appendFile(join(root, 'run-b', 'events.jsonl'), '{"type":"unknown"}\n')
    await expect(store.readEvents('run-b')).rejects.toThrow('unsupported run store event type: unknown')
    await mkdir(join(root, 'run-c'), { recursive: true })
    await appendFile(join(root, 'run-c', 'events.jsonl'), '{}\n')
    await expect(store.readEvents('run-c')).rejects.toThrow('unsupported run store event type: undefined')
  })

  it('surfaces unexpected legacy run directory read errors', async () => {
    const reportRoot = await tempRoot()
    const reportStore = new FsRunStoreAdapter(reportRoot)
    await reportStore.writeState('run-a', fullState())
    await reportStore.writeTasks('run-a', fullTasks())
    await mkdir(join(reportRoot, 'run-a', 'report.json'))

    await expect(normalizeLegacyRunDir(join(reportRoot, 'run-a'))).rejects.toMatchObject({
      code: 'EISDIR',
    })

    const workersRoot = await tempRoot()
    const workersStore = new FsRunStoreAdapter(workersRoot)
    await workersStore.writeState('run-a', fullState())
    await workersStore.writeTasks('run-a', fullTasks())
    await appendFile(join(workersRoot, 'run-a', 'workers'), '')

    await expect(normalizeLegacyRunDir(join(workersRoot, 'run-a'))).rejects.toMatchObject({
      code: 'ENOTDIR',
    })
  })
})

describe('normalizeLegacyRunDir', () => {
  it('normalizes legacy-shaped run dirs into typed v2 task graphs without renumbering ids', async () => {
    const fixtures = await writeLegacyPythonRuns()

    const legacy = await normalizeLegacyRunDir(join(fixtures, 'legacy-ordinal-ids'))
    expect(legacy.runId).toBe('legacy-ordinal-ids')
    expect(legacy.state).toEqual({
      integration_branch: 'council/legacy-ordinal-ids/integration',
      intensity: 'quick',
      rounds: 1,
      spec_id: '001-legacy-ordinal-ids',
      spec_relpath: 'specs/001-legacy-ordinal-ids',
      spec_slug: 'legacy-ordinal-ids',
      stage: 'fanned-out',
      task_count: 2,
    })
    expect(legacy.tasks.map((task) => task.id)).toEqual(['T1', 'T2'])
    expect(legacy.tasks[1]?.depends_on).toEqual(['T1'])
    expect(legacy.graph.idStrategy).toBe('legacy-ordinal')
    expect(legacy.graph.edges).toEqual([{ from: 'T1', kind: 'depends_on', to: 'T2' }])
    expect(legacy.report?.waves).toEqual([['T1'], ['T2']])
    expect([...legacy.workerResults.keys()]).toEqual(['T1', 'T2'])

    const watchdog = await normalizeLegacyRunDir(join(fixtures, 'watchdog-table-config'))
    expect(watchdog.tasks[0]?.dev_notes).toContain('[watchdog]')
    expect(watchdog.graph.nodes.get('ck-a100')?.task.id).toBe('ck-a100')

    const grown = await normalizeLegacyRunDir(join(fixtures, 'grown-schema-task'))
    expect(grown.graph.idStrategy).toBe('content-hash')
    expect(grown.tasks[0]).toMatchObject({
      archetype: 'schema-maintenance',
      context_profile: 'contracts',
      context_refs: ['kb://council/tasks/grown-schema'],
      id: 'ck-b200',
      supersedes: [],
    })
    expect(grown.workerResults.get('ck-b200')?.verdict).toEqual({
      issues: [],
      reasons: 'fixture',
      satisfied: true,
    })
  })

  it('accepts v2-shaped run dirs with missing legacy report and workers', async () => {
    const root = await tempRoot()
    const store = new FsRunStoreAdapter(root)
    await store.writeState('run-a', fullState())
    await store.writeTasks('run-a', fullTasks())

    const normalized = await normalizeLegacyRunDir(join(root, 'run-a'))

    expect(normalized.report).toBeUndefined()
    expect(normalized.workerResults.size).toBe(0)
    expect(normalized.graph.nodes.get('T1')?.task.id).toBe('T1')
  })

  it('normalizes optional legacy state fields from standalone JSON', async () => {
    const root = await tempRoot()
    const runDir = join(root, 'legacy-state')
    await mkdir(runDir, { recursive: true })
    await appendFile(
      join(runDir, 'state.json'),
      `${JSON.stringify({
        integration_branch: 'integration',
        intensity: 'quick',
        rounds: 1,
        spec_id: '001-legacy-state',
        spec_relpath: 'specs/001-legacy-state',
        spec_slug: 'legacy-state',
        stage: 'planned',
        task_count: 1,
      })}\n`,
    )
    await appendFile(join(runDir, 'tasks.json'), `${JSON.stringify(fullTasks())}\n`)

    const normalized = await normalizeLegacyRunDir(runDir)

    expect(normalized.state).toMatchObject({
      integration_branch: 'integration',
      rounds: 1,
      stage: 'planned',
      task_count: 1,
    })
  })
})

async function tempRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), 'council-fs-store-'))
  tempRoots.push(root)
  return root
}

async function writeLegacyPythonRuns(): Promise<string> {
  const root = await tempRoot()
  await writeLegacyOrdinalRun(join(root, 'legacy-ordinal-ids'))
  await writeWatchdogRun(join(root, 'watchdog-table-config'))
  await writeGrownSchemaRun(join(root, 'grown-schema-task'))
  return root
}

async function writeLegacyOrdinalRun(runDir: string): Promise<void> {
  const tasks = legacyOrdinalTasks()
  await writeRunJson(runDir, 'state.json', {
    integration_branch: 'council/legacy-ordinal-ids/integration',
    intensity: 'quick',
    rounds: 1,
    spec_id: '001-legacy-ordinal-ids',
    spec_relpath: 'specs/001-legacy-ordinal-ids',
    spec_slug: 'legacy-ordinal-ids',
    stage: 'fanned-out',
    task_count: 2,
  })
  await writeRunJson(runDir, 'tasks.json', tasks)
  await writeRunJson(runDir, 'report.json', {
    integration_branch: 'council/legacy-ordinal-ids/integration',
    run: 'legacy-ordinal-ids',
    tasks: tasks.map((task) => ({ status: 'ok', task_id: task.id })),
    waves: [['T1'], ['T2']],
  })
  await writeWorkerResult(runDir, 'T1', {
    files_changed: ['legacy/t1.txt'],
    status: 'ok',
    task_id: 'T1',
  })
  await writeWorkerResult(runDir, 'T2', {
    files_changed: ['legacy/t2.txt'],
    status: 'ok',
    task_id: 'T2',
  })
}

async function writeWatchdogRun(runDir: string): Promise<void> {
  const tasks = [
    {
      acceptance_criteria: ['The [watchdog] table remains present.'],
      boundaries: 'Only touch config/service.toml.',
      depends_on: [],
      dev_notes: '[watchdog]\ninterval = "30s"',
      difficulty: 'trivial',
      id: 'ck-a100',
      model: 'haiku',
      objective: 'Exercise a task that mentions a [watchdog] TOML table.',
      output_format: 'Patch',
      paths: ['config/service.toml'],
      title: 'Touch watchdog table config',
      verify: "rg '\\[watchdog\\]' config/service.toml",
    },
  ]
  await writeRunJson(runDir, 'state.json', {
    integration_branch: 'council/watchdog-table-config/integration',
    intensity: 'quick',
    rounds: 1,
    spec_id: '001-watchdog-table-config',
    spec_relpath: 'specs/001-watchdog-table-config',
    spec_slug: 'watchdog-table-config',
    stage: 'fanned-out',
    task_count: 1,
  })
  await writeRunJson(runDir, 'tasks.json', tasks)
  await writeRunJson(runDir, 'report.json', {
    run: 'watchdog-table-config',
    tasks: [{ status: 'ok', task_id: 'ck-a100' }],
    waves: [['ck-a100']],
  })
  await writeWorkerResult(runDir, 'ck-a100', {
    status: 'ok',
    task_id: 'ck-a100',
  })
}

async function writeGrownSchemaRun(runDir: string): Promise<void> {
  const tasks = [
    {
      archetype: 'schema-maintenance',
      boundaries: 'Only touch schema/grown.json.',
      content_hash: 'sha256:fixture-grown-schema-task',
      context_profile: 'contracts',
      context_refs: ['kb://council/tasks/grown-schema'],
      depends_on: [],
      difficulty: 'moderate',
      engine: { cli: 'codex', model: 'gpt-5.5' },
      id: 'ck-b200',
      model: 'sonnet',
      model_tier: 'expensive',
      objective: 'Exercise a task with the grown task schema.',
      output_format: 'Patch',
      paths: ['schema/grown.json'],
      supersedes: [],
      title: 'Maintain grown schema',
      verify: 'test -f schema/grown.json',
    },
  ]
  await writeRunJson(runDir, 'state.json', {
    intensity: 'quick',
    rounds: 1,
    spec_id: '001-grown-schema-task',
    spec_relpath: 'specs/001-grown-schema-task',
    spec_slug: 'grown-schema-task',
    stage: 'fanned-out',
    task_count: 1,
  })
  await writeRunJson(runDir, 'tasks.json', tasks)
  await writeRunJson(runDir, 'report.json', {
    run: 'grown-schema-task',
    tasks: [{ status: 'ok', task_id: 'ck-b200' }],
    waves: [['ck-b200']],
  })
  await writeWorkerResult(runDir, 'ck-b200', {
    files_changed: ['schema/grown.json'],
    status: 'ok',
    task_id: 'ck-b200',
    verdict: {
      issues: [],
      reasons: 'fixture',
      satisfied: true,
    },
  })
}

function legacyOrdinalTasks(): readonly Task[] {
  return [
    {
      boundaries: 'Only touch legacy/t1.txt.',
      depends_on: [],
      difficulty: 'trivial',
      id: 'T1',
      model: 'haiku',
      objective: 'Exercise a legacy T1 task id.',
      output_format: 'Patch',
      paths: ['legacy/t1.txt'],
      title: 'Legacy T1',
      verify: 'test -f legacy/t1.txt',
    },
    {
      boundaries: 'Only touch legacy/t2.txt.',
      depends_on: ['T1'],
      difficulty: 'trivial',
      id: 'T2',
      model: 'haiku',
      objective: 'Exercise a legacy T2 task id that depends on T1.',
      output_format: 'Patch',
      paths: ['legacy/t2.txt'],
      title: 'Legacy T2',
      verify: 'test -f legacy/t2.txt',
    },
  ]
}

async function writeWorkerResult(runDir: string, taskId: string, result: WorkerResult): Promise<void> {
  await writeRunJson(join(runDir, 'workers', taskId), 'result.json', result)
}

async function writeRunJson(runDir: string, file: string, value: unknown): Promise<void> {
  await mkdir(runDir, { recursive: true })
  await writeFile(join(runDir, file), `${JSON.stringify(value, null, 2)}\n`, 'utf8')
}

function fullState(): RunState {
  return {
    agents: ['planner-a', 'planner-b'],
    content_hash: 'sha256:state',
    engine: { cli: 'codex', model: 'gpt-5' },
    integration_branch: 'council/run-a/integration',
    intensity: 'quick',
    model_tier: 'frontier',
    rounds: 2,
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
      discovered_from: 'brief',
      engine: { cli: 'claude', model: 'sonnet' },
      id: 'T1',
      model: 'sonnet',
      model_tier: 'standard',
      objective: 'Implement the example behavior.',
      output_format: 'Patch',
      paths: ['src/example.ts'],
      spec_ref: 'specs/001-run-a',
      supersedes: [],
      title: 'Example task',
      verify: 'npm test',
    },
  ]
}

function fullStory(): Story {
  return {
    acceptance_criteria: ['Works'],
    context: 'Context',
    definition_of_done: ['Tested'],
    goal: 'Goal',
    implementation_notes: {
      data_config_migration: ['None'],
      dependencies: ['None'],
      files: ['src/example.ts'],
      patterns: ['Existing style'],
    },
    scope: {
      in_scope: ['Adapter'],
      out_of_scope: ['CLI'],
    },
    status: 'ready',
    tests: {
      integration: ['Run adapter test'],
      manual_or_workflow: ['Not required'],
      unit: ['Run unit test'],
    },
    title: 'Story',
    user_value: {
      actor: 'operator',
      capability: 'resume runs',
      outcome: 'state is durable',
    },
  }
}

function fullLedger(): DesignLedger {
  return {
    content_hash: 'sha256:ledger',
    entries: [
      {
        content_hash: 'sha256:entry',
        context_refs: ['kb://decision'],
        decision: 'Use locked append',
        id: 'D1',
        rationale: 'Multiple workers can append events.',
        status: 'accepted',
        supersedes: ['D0'],
        task_refs: ['T1'],
      },
    ],
  }
}

function fullWorkerResult(taskId: string): WorkerResult {
  return {
    branch: `council/run-a/${taskId}`,
    committed: true,
    content_hash: 'sha256:worker-result',
    engine: { cli: 'claude', model: 'haiku' },
    error: 'none',
    files_changed: ['src/example.ts'],
    merge: 'ok',
    model: 'claude:haiku',
    model_tier: 'cheap',
    out_of_bounds: [],
    status: 'ok',
    stderr_bytes: 8,
    stderr_log_path: `workers/${taskId}/logs/stderr.log`,
    stderr_tail: 'warning\n',
    stdout_bytes: 3,
    stdout_log_path: `workers/${taskId}/logs/stdout.log`,
    stdout_tail: 'ok\n',
    suggested_model: 'haiku',
    summary: 'Done',
    task_id: taskId,
    title: 'Example task',
    verdict: {
      content_hash: 'sha256:verdict',
      engine: { cli: 'claude', model: 'sonnet' },
      issues: [],
      model_tier: 'standard',
      reasons: 'ok',
      reviewer: 'verifier',
      satisfied: true,
      task_id: taskId,
    },
    verify_output: 'ok',
    verify_rc: 0,
    worktree: `/tmp/${taskId}`,
  }
}

function fullSupervisorSnapshot(taskId: string): WorkerSupervisorSnapshot {
  return {
    attempt_id: 2,
    exit_code: null,
    logs: {
      stderr: `workers/${taskId}/logs/stderr.log`,
      stdout: `workers/${taskId}/logs/stdout.log`,
    },
    model_tier: 'frontier',
    offsets: {
      stderr: 7,
      stdout: 11,
    },
    pid: 202,
    restart_count: 1,
    signal: null,
    status: 'running',
    task_id: taskId,
    watchdog: {
      handling_detection: false,
      loop: {
        actions: [
          {
            normalized: '$ npm test',
            verbatim: '$ npm test',
          },
        ],
      },
      pending_detection: {
        idleMs: 1000,
        kind: 'progress-stall',
        lastProgressAtMs: 0,
      },
      progress: {
        attemptStartedAtMs: 1000,
        lastActionAtMs: 1000,
        lastOutputAtMs: 1200,
        lastProgressAtMs: 1200,
        outputBytes: 18,
        startedAtMs: 0,
      },
      retry: {
        attempts: 1,
        failureFingerprints: ['progress-stall:[]'],
      },
    },
  }
}
