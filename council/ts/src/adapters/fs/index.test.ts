import { open, readFile, readdir, unlink, appendFile, mkdir, mkdtemp } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'

import { describe, expect, it } from 'vitest'

import type { ClockPort } from '../../ports/index.js'
import type { DesignLedger, RunState, Story, Task } from '../../domain/contracts/index.js'
import { FsRunStoreAdapter, normalizeLegacyRunDir, type WorkerResult } from './index.js'

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

    await store.writeState('run-a', state)
    await store.writeTasks('run-a', tasks)
    await store.writeStory('run-a', story)
    await store.writeDesignLedger('run-a', ledger)
    await store.writeWorkerResult('run-a', 'T1', result)

    await expect(store.readState('run-a')).resolves.toEqual(state)
    await expect(store.readTasks('run-a')).resolves.toEqual(tasks)
    await expect(store.readStory('run-a')).resolves.toEqual(story)
    await expect(store.readDesignLedger('run-a')).resolves.toEqual(ledger)
    await expect(store.readWorkerResult('run-a', 'T1')).resolves.toEqual(result)
    await expect(readFile(join(root, 'run-a', 'state.json'), 'utf8')).resolves.toBe(
      `${JSON.stringify(state, null, 2)}\n`,
    )
    const runFiles = await readdir(join(root, 'run-a'))
    expect(runFiles.filter((file) => file.includes('.tmp'))).toEqual([])
    await expect(readdir(join(root, 'run-a', 'workers', 'T1'))).resolves.toEqual(['result.json'])
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
        committed: 'yes',
        status: 'ok',
        task_id: 'T1',
      } as unknown as WorkerResult),
    ).rejects.toThrow('worker result.committed must be a boolean')

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
  it('normalizes T5 Python fixtures into typed v2 task graphs without renumbering ids', async () => {
    const fixtures = join(process.cwd(), 'test/fixtures/python-runs')

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
  return mkdtemp(join(tmpdir(), 'council-fs-store-'))
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
