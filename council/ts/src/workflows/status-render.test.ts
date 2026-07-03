import { describe, expect, it } from 'vitest'

import type { RunTaskView, RunView } from './status.js'
import { renderRunStatusJson, renderRunStatusTable } from './status-render.js'

function task(input: Partial<RunTaskView> & Pick<RunTaskView, 'state' | 'taskId'>): RunTaskView {
  const { state, taskId, ...overrides } = input
  return {
    attempt: 0,
    blockedBy: [],
    dependenciesSatisfied: true,
    durationMs: 0,
    lastDetection: null,
    modelTier: null,
    pid: null,
    restarts: 0,
    startedAt: null,
    state,
    taskId,
    terminalStatus: null,
    title: `Task ${taskId}`,
    updatedAt: null,
    wave: 0,
    workerId: null,
    ...overrides,
  }
}

function runView(tasks: readonly RunTaskView[]): RunView {
  return {
    rollup: {
      countsByState: {
        blocked: 1,
        detected: 1,
        failed: 1,
        pending: 2,
        restarting: 1,
        running: 1,
        succeeded: 2,
      },
      criticalPath: ['ck-ready', 'ck-running', 'ck-detected'],
      elapsedMs: 665_000,
      readySet: ['ck-ready'],
      startedAt: '2026-07-03T10:00:00.000Z',
      updatedAt: '2026-07-03T10:10:30.000Z',
    },
    run: 'run-a',
    state: { stage: 'fanout' },
    tasks,
    waves: [
      ['ck-pending', 'ck-ready', 'ck-running'],
      ['ck-detected', 'ck-restarting', 'ck-succeeded'],
      ['ck-failed', 'ck-skipped', 'ck-blocked'],
    ],
  }
}

const COMPLETE_VIEW = runView([
  task({
    blockedBy: ['ck-root'],
    dependenciesSatisfied: false,
    state: 'pending',
    taskId: 'ck-pending',
    title: 'Waiting for dependency',
  }),
  task({
    state: 'pending',
    taskId: 'ck-ready',
    title: 'Ready task',
  }),
  task({
    attempt: 1,
    durationMs: 600_000,
    modelTier: 'sonnet',
    pid: 12345,
    startedAt: '2026-07-03T10:00:00.000Z',
    state: 'running',
    taskId: 'ck-running',
    title: 'Running task',
    updatedAt: '2026-07-03T10:05:00.000Z',
    workerId: 'worker-running',
  }),
  task({
    attempt: 2,
    durationMs: 125_000,
    lastDetection: 'progress-stall',
    pid: 456,
    restarts: 1,
    state: 'detected',
    taskId: 'ck-detected',
    title: 'Detected task',
    workerId: 'worker-detected',
    wave: 1,
  }),
  task({
    attempt: 3,
    durationMs: 95_000,
    lastDetection: 'budget-cap',
    restarts: 2,
    state: 'restarting',
    taskId: 'ck-restarting',
    title: 'Restarting task',
    wave: 1,
  }),
  task({
    durationMs: 30_000,
    state: 'succeeded',
    taskId: 'ck-succeeded',
    terminalStatus: 'ok',
    title: 'Succeeded task',
    wave: 1,
  }),
  task({
    durationMs: 45_000,
    state: 'failed',
    taskId: 'ck-failed',
    terminalStatus: 'failed',
    title: 'Failed task',
    wave: 2,
  }),
  task({
    state: 'succeeded',
    taskId: 'ck-skipped',
    terminalStatus: 'no-op',
    title: 'Skipped task',
    wave: 2,
  }),
  task({
    blockedBy: ['ck-failed'],
    dependenciesSatisfied: false,
    state: 'blocked',
    taskId: 'ck-blocked',
    title: 'Blocked task',
    wave: 2,
  }),
])

describe('renderRunStatusTable', () => {
  it('renders a compact deterministic table with rollups, waves, durations, ready set, and active detections', () => {
    expect(renderRunStatusTable(COMPLETE_VIEW)).toMatchInlineSnapshot(`
      "run run-a stage=fanout elapsed=11m05s started=2026-07-03T10:00:00.000Z updated=2026-07-03T10:10:30.000Z
      rollup counts=blocked:1 detected:1 failed:1 pending:1 ready:1 restarting:1 running:1 skipped:1 succeeded:1 ready=ck-ready critical=ck-ready>ck-running>ck-detected
      active detected=ck-detected(progress-stall) restarting=ck-restarting(budget-cap) running=ck-running(pid=12345)
      wave 0
      badge        task           duration  details
      [PENDING]    ck-pending     0s        Waiting for dependency; blocked-by=ck-root
      [READY]      ck-ready       0s        Ready task
      [RUNNING]    ck-running     10m00s    Running task; worker=worker-running; pid=12345; attempt=1; model=sonnet
      wave 1
      badge        task           duration  details
      [DETECTED]   ck-detected    2m05s     Detected task; worker=worker-detected; pid=456; attempt=2; restarts=1; detection=progress-stall
      [RESTART]    ck-restarting  1m35s     Restarting task; attempt=3; restarts=2; detection=budget-cap
      [OK]         ck-succeeded   30s       Succeeded task; terminal=ok
      wave 2
      badge        task           duration  details
      [FAILED]     ck-failed      45s       Failed task; terminal=failed
      [SKIPPED]    ck-skipped     0s        Skipped task; terminal=no-op
      [BLOCKED]    ck-blocked     0s        Blocked task; blocked-by=ck-failed"
    `)
  })

  it('renders deterministic ANSI colors only when explicitly requested', () => {
    expect(renderRunStatusTable(COMPLETE_VIEW, { color: true })).toMatchInlineSnapshot(`
      "run run-a stage=fanout elapsed=11m05s started=2026-07-03T10:00:00.000Z updated=2026-07-03T10:10:30.000Z
      rollup counts=blocked:1 detected:1 failed:1 pending:1 ready:1 restarting:1 running:1 skipped:1 succeeded:1 ready=ck-ready critical=ck-ready>ck-running>ck-detected
      active detected=ck-detected(progress-stall) restarting=ck-restarting(budget-cap) running=ck-running(pid=12345)
      wave 0
      badge        task           duration  details
      [90m[PENDING][0m    ck-pending     0s        Waiting for dependency; blocked-by=ck-root
      [36m[READY][0m      ck-ready       0s        Ready task
      [34m[RUNNING][0m    ck-running     10m00s    Running task; worker=worker-running; pid=12345; attempt=1; model=sonnet
      wave 1
      badge        task           duration  details
      [33m[DETECTED][0m   ck-detected    2m05s     Detected task; worker=worker-detected; pid=456; attempt=2; restarts=1; detection=progress-stall
      [35m[RESTART][0m    ck-restarting  1m35s     Restarting task; attempt=3; restarts=2; detection=budget-cap
      [32m[OK][0m         ck-succeeded   30s       Succeeded task; terminal=ok
      wave 2
      badge        task           duration  details
      [31m[FAILED][0m     ck-failed      45s       Failed task; terminal=failed
      [90m[SKIPPED][0m    ck-skipped     0s        Skipped task; terminal=no-op
      [31m[BLOCKED][0m    ck-blocked     0s        Blocked task; blocked-by=ck-failed"
    `)
  })

  it('renders empty rollups and ungrouped tasks without terminal assumptions', () => {
    const view = runView([
      task({
        durationMs: 3_661_000,
        state: 'budget-cap',
        taskId: 'ck-budget',
        title: 'Budget capped task',
        wave: null,
      }),
    ])

    expect(
      renderRunStatusTable({
        ...view,
        rollup: {
          countsByState: { 'budget-cap': 1 },
          criticalPath: [],
          elapsedMs: 0,
          readySet: [],
          startedAt: null,
          updatedAt: null,
        },
        waves: [],
      }),
    ).toMatchInlineSnapshot(`
      "run run-a stage=fanout elapsed=0s started=- updated=-
      rollup counts=budget-cap:1 ready=- critical=-
      active -
      wave ?
      badge      task       duration  details
      [BUDGET]   ck-budget  1h01m01s  Budget capped task"
    `)
  })
})

describe('renderRunStatusJson', () => {
  it('keeps empty RunView records stable in machine output', () => {
    expect(renderRunStatusJson({ ...COMPLETE_VIEW, state: {} })).toContain('"state": {}')
  })

  it('formats only serializable RunView data with stable object keys', () => {
    expect(renderRunStatusJson(COMPLETE_VIEW)).toMatchInlineSnapshot(`
      "{
        "rollup": {
          "countsByState": {
            "blocked": 1,
            "detected": 1,
            "failed": 1,
            "pending": 2,
            "restarting": 1,
            "running": 1,
            "succeeded": 2
          },
          "criticalPath": [
            "ck-ready",
            "ck-running",
            "ck-detected"
          ],
          "elapsedMs": 665000,
          "readySet": [
            "ck-ready"
          ],
          "startedAt": "2026-07-03T10:00:00.000Z",
          "updatedAt": "2026-07-03T10:10:30.000Z"
        },
        "run": "run-a",
        "state": {
          "stage": "fanout"
        },
        "tasks": [
          {
            "attempt": 0,
            "blockedBy": [
              "ck-root"
            ],
            "dependenciesSatisfied": false,
            "durationMs": 0,
            "lastDetection": null,
            "modelTier": null,
            "pid": null,
            "restarts": 0,
            "startedAt": null,
            "state": "pending",
            "taskId": "ck-pending",
            "terminalStatus": null,
            "title": "Waiting for dependency",
            "updatedAt": null,
            "wave": 0,
            "workerId": null
          },
          {
            "attempt": 0,
            "blockedBy": [],
            "dependenciesSatisfied": true,
            "durationMs": 0,
            "lastDetection": null,
            "modelTier": null,
            "pid": null,
            "restarts": 0,
            "startedAt": null,
            "state": "pending",
            "taskId": "ck-ready",
            "terminalStatus": null,
            "title": "Ready task",
            "updatedAt": null,
            "wave": 0,
            "workerId": null
          },
          {
            "attempt": 1,
            "blockedBy": [],
            "dependenciesSatisfied": true,
            "durationMs": 600000,
            "lastDetection": null,
            "modelTier": "sonnet",
            "pid": 12345,
            "restarts": 0,
            "startedAt": "2026-07-03T10:00:00.000Z",
            "state": "running",
            "taskId": "ck-running",
            "terminalStatus": null,
            "title": "Running task",
            "updatedAt": "2026-07-03T10:05:00.000Z",
            "wave": 0,
            "workerId": "worker-running"
          },
          {
            "attempt": 2,
            "blockedBy": [],
            "dependenciesSatisfied": true,
            "durationMs": 125000,
            "lastDetection": "progress-stall",
            "modelTier": null,
            "pid": 456,
            "restarts": 1,
            "startedAt": null,
            "state": "detected",
            "taskId": "ck-detected",
            "terminalStatus": null,
            "title": "Detected task",
            "updatedAt": null,
            "wave": 1,
            "workerId": "worker-detected"
          },
          {
            "attempt": 3,
            "blockedBy": [],
            "dependenciesSatisfied": true,
            "durationMs": 95000,
            "lastDetection": "budget-cap",
            "modelTier": null,
            "pid": null,
            "restarts": 2,
            "startedAt": null,
            "state": "restarting",
            "taskId": "ck-restarting",
            "terminalStatus": null,
            "title": "Restarting task",
            "updatedAt": null,
            "wave": 1,
            "workerId": null
          },
          {
            "attempt": 0,
            "blockedBy": [],
            "dependenciesSatisfied": true,
            "durationMs": 30000,
            "lastDetection": null,
            "modelTier": null,
            "pid": null,
            "restarts": 0,
            "startedAt": null,
            "state": "succeeded",
            "taskId": "ck-succeeded",
            "terminalStatus": "ok",
            "title": "Succeeded task",
            "updatedAt": null,
            "wave": 1,
            "workerId": null
          },
          {
            "attempt": 0,
            "blockedBy": [],
            "dependenciesSatisfied": true,
            "durationMs": 45000,
            "lastDetection": null,
            "modelTier": null,
            "pid": null,
            "restarts": 0,
            "startedAt": null,
            "state": "failed",
            "taskId": "ck-failed",
            "terminalStatus": "failed",
            "title": "Failed task",
            "updatedAt": null,
            "wave": 2,
            "workerId": null
          },
          {
            "attempt": 0,
            "blockedBy": [],
            "dependenciesSatisfied": true,
            "durationMs": 0,
            "lastDetection": null,
            "modelTier": null,
            "pid": null,
            "restarts": 0,
            "startedAt": null,
            "state": "succeeded",
            "taskId": "ck-skipped",
            "terminalStatus": "no-op",
            "title": "Skipped task",
            "updatedAt": null,
            "wave": 2,
            "workerId": null
          },
          {
            "attempt": 0,
            "blockedBy": [
              "ck-failed"
            ],
            "dependenciesSatisfied": false,
            "durationMs": 0,
            "lastDetection": null,
            "modelTier": null,
            "pid": null,
            "restarts": 0,
            "startedAt": null,
            "state": "blocked",
            "taskId": "ck-blocked",
            "terminalStatus": null,
            "title": "Blocked task",
            "updatedAt": null,
            "wave": 2,
            "workerId": null
          }
        ],
        "waves": [
          [
            "ck-pending",
            "ck-ready",
            "ck-running"
          ],
          [
            "ck-detected",
            "ck-restarting",
            "ck-succeeded"
          ],
          [
            "ck-failed",
            "ck-skipped",
            "ck-blocked"
          ]
        ]
      }"
    `)
  })
})
