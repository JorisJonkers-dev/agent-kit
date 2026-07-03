import { describe, expect, it } from 'vitest'

import type {
  Amendment,
  ReviewVerdict,
  RoutingVerdict,
  RunState,
  Task,
} from '../../../shared-kernel/index.js'
import {
  amendmentEvent,
  planAtomicJsonWrite,
  planEventAppend,
  planEventsAppend,
  planStateWrite,
  planTasksWrite,
  reviewVerdictEvent,
  routingVerdictEvent,
  RUNSTORE_STATE_FILE,
  RUNSTORE_TASKS_FILE,
} from './index.js'

describe('runstore atomic JSON write plans', () => {
  it('plans state.json as a same-directory temp write followed by rename', () => {
    const state: RunState = {
      engine: {
        cli: 'codex',
        model: 'gpt-5',
      },
      stage: 'supervisor',
      rounds: 2,
    }

    const plan = planStateWrite('run-123', state, 'write-1')

    expect(plan).toEqual({
      kind: 'atomic-json-write',
      runId: 'run-123',
      target: RUNSTORE_STATE_FILE,
      finalPath: 'run-123/state.json',
      tempPath: 'run-123/.state.json.write-1.tmp',
      bytes: `${JSON.stringify(state, null, 2)}\n`,
      steps: [
        {
          kind: 'write-temp-file',
          path: 'run-123/.state.json.write-1.tmp',
          bytes: `${JSON.stringify(state, null, 2)}\n`,
        },
        {
          kind: 'sync-file',
          path: 'run-123/.state.json.write-1.tmp',
        },
        {
          kind: 'rename-file',
          fromPath: 'run-123/.state.json.write-1.tmp',
          toPath: 'run-123/state.json',
        },
        {
          kind: 'sync-directory',
          path: 'run-123',
        },
      ],
    })
  })

  it('plans tasks.json with array order preserved', () => {
    const tasks: readonly Task[] = [
      {
        id: 'T1',
        title: 'First task',
        objective: 'Do the first thing',
        output_format: 'Patch',
        paths: ['src/a.ts'],
        depends_on: [],
        difficulty: 'moderate',
        model: 'sonnet',
        verify: 'npm test',
        boundaries: 'Only src/a.ts',
      },
      {
        id: 'T2',
        title: 'Second task',
        objective: 'Do the second thing',
        output_format: 'Patch',
        paths: ['src/b.ts'],
        depends_on: ['T1'],
        difficulty: 'hard',
        model: 'opus',
        verify: 'npm test',
        boundaries: 'Only src/b.ts',
      },
    ]

    const plan = planTasksWrite('run-123', tasks, 'write-2')

    expect(plan.target).toBe(RUNSTORE_TASKS_FILE)
    expect(plan.finalPath).toBe('run-123/tasks.json')
    expect(plan.tempPath).toBe('run-123/.tasks.json.write-2.tmp')
    expect(plan.bytes).toBe(`${JSON.stringify(tasks, null, 2)}\n`)
  })

  it('supports the generic atomic JSON planner', () => {
    const state: RunState = {
      engine: {
        cli: 'codex',
        model: 'gpt-5',
      },
    }

    const plan = planAtomicJsonWrite({
      runId: 'generic-run',
      target: RUNSTORE_STATE_FILE,
      tempId: 'temp',
      value: state,
    })

    expect(plan.steps.map((step) => step.kind)).toEqual([
      'write-temp-file',
      'sync-file',
      'rename-file',
      'sync-directory',
    ])
  })

  it('rejects unsafe run IDs and temp IDs', () => {
    const state: RunState = {
      engine: {
        cli: 'codex',
        model: 'gpt-5',
      },
    }

    expect(() => planStateWrite('', state, 'tmp')).toThrow('runId must not be empty')
    expect(() => planStateWrite('run/../other', state, 'tmp')).toThrow(
      'runId must be a single path segment',
    )
    expect(() => planStateWrite('run', state, 'tmp\\other')).toThrow(
      'tempId must be a single path segment',
    )
    expect(() => planStateWrite('run', state, 'tmp\0other')).toThrow(
      'tempId must be a single path segment',
    )
  })

  it('rejects values that cannot be serialized as JSON', () => {
    expect(() =>
      planAtomicJsonWrite({
        runId: 'run',
        target: RUNSTORE_STATE_FILE,
        tempId: 'tmp',
        value: undefined as unknown as RunState,
      }),
    ).toThrow('runstore values must be JSON serializable')
  })
})

describe('runstore event append plans', () => {
  it('wraps known domain payloads as event records', () => {
    const review: ReviewVerdict = {
      engine: {
        cli: 'codex',
        model: 'gpt-5',
      },
      satisfied: false,
      reasons: 'Missing tests',
      issues: ['No coverage'],
      task_id: 'T1',
    }
    const routing: RoutingVerdict = {
      engine: {
        cli: 'claude',
        model: 'sonnet',
      },
      route: 'amend',
      reasons: 'Scope changed',
      task_id: 'T1',
    }
    const amendment: Amendment = {
      engine: {
        cli: 'codex',
        model: 'gpt-5',
      },
      id: 'A1',
      summary: 'Add tests',
    }

    expect(reviewVerdictEvent(review)).toEqual({
      type: 'review_verdict',
      payload: review,
    })
    expect(routingVerdictEvent(routing)).toEqual({
      type: 'routing_verdict',
      payload: routing,
    })
    expect(amendmentEvent(amendment)).toEqual({
      type: 'amendment',
      payload: amendment,
    })
  })

  it('plans a single event append inside the events lock', () => {
    const verdict: ReviewVerdict = {
      engine: {
        cli: 'codex',
        model: 'gpt-5',
      },
      satisfied: true,
      reasons: 'Done',
      issues: [],
    }
    const event = reviewVerdictEvent(verdict)

    const plan = planEventAppend('run-123', event)

    expect(plan).toEqual({
      kind: 'locked-event-append',
      runId: 'run-123',
      eventPath: 'run-123/events.jsonl',
      lockPath: 'run-123/events.jsonl.lock',
      events: [event],
      bytes: `${JSON.stringify(event)}\n`,
      steps: [
        {
          kind: 'acquire-lock',
          path: 'run-123/events.jsonl.lock',
        },
        {
          kind: 'append-file',
          path: 'run-123/events.jsonl',
          bytes: `${JSON.stringify(event)}\n`,
        },
        {
          kind: 'sync-file',
          path: 'run-123/events.jsonl',
        },
        {
          kind: 'release-lock',
          path: 'run-123/events.jsonl.lock',
        },
      ],
    })
  })

  it('plans batches as ordered JSON Lines in one lock window', () => {
    const first = amendmentEvent({
      engine: {
        cli: 'codex',
        model: 'gpt-5',
      },
      id: 'A1',
      summary: 'First',
    })
    const second = routingVerdictEvent({
      engine: {
        cli: 'claude',
        model: 'sonnet',
      },
      route: 'supervisor',
      reasons: 'Continue',
    })

    const plan = planEventsAppend({
      runId: 'run-123',
      events: [first, second],
    })

    expect(plan.bytes).toBe(`${JSON.stringify(first)}\n${JSON.stringify(second)}\n`)
    expect(plan.steps.map((step) => step.kind)).toEqual([
      'acquire-lock',
      'append-file',
      'sync-file',
      'release-lock',
    ])
  })

  it('rejects empty batches and unsafe run IDs', () => {
    const event = amendmentEvent({
      engine: {
        cli: 'codex',
        model: 'gpt-5',
      },
      id: 'A1',
      summary: 'First',
    })

    expect(() =>
      planEventsAppend({
        runId: 'run-123',
        events: [],
      }),
    ).toThrow('events must not be empty')
    expect(() => planEventAppend('run\\other', event)).toThrow(
      'runId must be a single path segment',
    )
  })
})
