import { describe, expect, it } from 'vitest'

import { ModelGatewayError } from './config.js'
import {
  endWorkflow,
  parseWorkflowsFile,
  startWorkflow,
  workflowEnded,
  workflowEndsAt,
  workflowView,
  workflowViews,
  WORKFLOW_TTL_MS,
  type WorkflowRecord,
} from './workflows.js'

const STARTED = '2026-11-04T10:00:00.000Z'

/** The moment a workflow started, and a moment `hours` later. */
function at(hours: number): Date {
  return new Date(Date.parse(STARTED) + hours * 60 * 60 * 1000)
}

function running(name = 'spec-293'): WorkflowRecord {
  return { name, started_at: STARTED }
}

/** The code a call throws, for the tests that pin how an error is reported. */
function codeOf(run: () => unknown): string | undefined {
  try {
    run()
  } catch (error) {
    return error instanceof ModelGatewayError ? error.code : undefined
  }
  throw new Error('the call did not throw')
}

/** The message a call throws, for the tests that pin what an error says. */
function messageOf(run: () => unknown): string {
  try {
    run()
  } catch (error) {
    return error instanceof Error ? error.message : String(error)
  }
  throw new Error('the call did not throw')
}

describe('parseWorkflowsFile', () => {
  it('reads the workflows a file holds, in the order it holds them', () => {
    const text = JSON.stringify({
      version: 1,
      workflows: [running('one'), { ended_at: '2026-11-04T11:00:00.000Z', name: 'two', started_at: STARTED }],
    })

    expect(parseWorkflowsFile(text)).toEqual([
      { name: 'one', started_at: STARTED },
      { ended_at: '2026-11-04T11:00:00.000Z', name: 'two', started_at: STARTED },
    ])
  })

  it('reads a file that holds no workflow', () => {
    expect(parseWorkflowsFile('{"version":1,"workflows":[]}')).toEqual([])
  })

  it('refuses a file that is not JSON, naming what is wrong', () => {
    expect(codeOf(() => parseWorkflowsFile('{'))).toBe('invalid-config')
    expect(messageOf(() => parseWorkflowsFile('{'))).toContain('not JSON')
  })

  it('refuses a file that is not a JSON object', () => {
    expect(messageOf(() => parseWorkflowsFile('[]'))).toBe('the workflows file must be a JSON object')
  })

  it('refuses a workflow that is not an object', () => {
    expect(messageOf(() => parseWorkflowsFile('{"version":1,"workflows":["one"]}'))).toBe(
      'workflows[0] must be an object',
    )
  })

  it('refuses a file of another version', () => {
    expect(messageOf(() => parseWorkflowsFile('{"version":2,"workflows":[]}'))).toBe('version must be 1')
  })

  it('refuses a file whose workflows are not a list', () => {
    expect(messageOf(() => parseWorkflowsFile('{"version":1,"workflows":{}}'))).toBe('workflows must be a list')
  })

  it('refuses a workflow that names no name, or a moment that is not one', () => {
    expect(messageOf(() => parseWorkflowsFile('{"version":1,"workflows":[{"started_at":"x"}]}'))).toBe(
      'workflows[0].name must be a non-empty string',
    )
    expect(messageOf(() => parseWorkflowsFile('{"version":1,"workflows":[{"name":"one","started_at":"x"}]}'))).toBe(
      'workflows[0].started_at must be an ISO timestamp',
    )
    expect(
      messageOf(() => parseWorkflowsFile('{"version":1,"workflows":[{"ended_at":"x","name":"one","started_at":"2026-11-04T10:00:00.000Z"}]}')),
    ).toBe('workflows[0].ended_at must be an ISO timestamp')
  })
})

describe('workflowEnded', () => {
  it('is not ended while it runs, and ended once someone ended it', () => {
    expect(workflowEnded(running(), at(1))).toBe(false)
    expect(workflowEnded({ ended_at: '2026-11-04T10:30:00.000Z', name: 'one', started_at: STARTED }, at(1))).toBe(true)
  })

  it('counts as ended once its 24 hours have run out, without anyone ending it', () => {
    expect(workflowEnded(running(), new Date(Date.parse(STARTED) + WORKFLOW_TTL_MS - 1))).toBe(false)
    expect(workflowEnded(running(), new Date(Date.parse(STARTED) + WORKFLOW_TTL_MS))).toBe(true)
  })
})

describe('workflowView', () => {
  it('shows a running workflow with no end', () => {
    expect(workflowView(running(), at(1))).toEqual({
      ended_at: null,
      name: 'spec-293',
      started_at: STARTED,
      state: 'running',
    })
  })

  it('shows the moment someone ended it', () => {
    expect(workflowView({ ended_at: '2026-11-04T10:30:00.000Z', name: 'one', started_at: STARTED }, at(1))).toEqual({
      ended_at: '2026-11-04T10:30:00.000Z',
      name: 'one',
      started_at: STARTED,
      state: 'ended',
    })
  })

  it('shows the moment its 24 hours ran out when nobody ended it', () => {
    expect(workflowView(running(), at(30))).toEqual({
      ended_at: '2026-11-05T10:00:00.000Z',
      name: 'spec-293',
      started_at: STARTED,
      state: 'ended',
    })
  })

  it('shows every workflow, in the order the file holds them', () => {
    expect(workflowViews([running('one'), running('two')], at(1)).map((view) => view.name)).toEqual(['one', 'two'])
  })
})

describe('startWorkflow', () => {
  it('starts a workflow the first time it is named', () => {
    expect(startWorkflow([], 'spec-293', at(0))).toEqual({
      workflow: { name: 'spec-293', started_at: STARTED },
      workflows: [{ name: 'spec-293', started_at: STARTED }],
    })
  })

  it('leaves a workflow that is already running as it is', () => {
    const workflows = [running()]

    expect(startWorkflow(workflows, 'spec-293', at(1))).toEqual({ workflow: running(), workflows })
  })

  it('refuses a workflow that ended, naming when it ended', () => {
    const ended = [{ ended_at: '2026-11-04T10:30:00.000Z', name: 'spec-293', started_at: STARTED }]

    expect(codeOf(() => startWorkflow(ended, 'spec-293', at(1)))).toBe('workflow-ended')
    expect(messageOf(() => startWorkflow(ended, 'spec-293', at(1)))).toBe(
      'workflow spec-293 ended at 2026-11-04T10:30:00.000Z: a run may not join it, so start a new workflow',
    )
  })

  it('refuses a workflow whose 24 hours ran out, naming the moment it ended', () => {
    expect(messageOf(() => startWorkflow([running()], 'spec-293', at(30)))).toBe(
      'workflow spec-293 ended at 2026-11-05T10:00:00.000Z: a run may not join it, so start a new workflow',
    )
  })
})

describe('endWorkflow', () => {
  it('ends a running workflow at the moment it is ended', () => {
    const ended = { ended_at: '2026-11-04T12:00:00.000Z', name: 'spec-293', started_at: STARTED }

    expect(endWorkflow([running()], 'spec-293', at(2))).toEqual({ workflow: ended, workflows: [ended] })
  })

  it('leaves the other workflows as they are', () => {
    const other = running('other')

    expect(endWorkflow([running(), other], 'spec-293', at(2)).workflows[1]).toBe(other)
  })

  it('leaves a workflow that already ended at the moment it ended', () => {
    const ended = [{ ended_at: '2026-11-04T10:30:00.000Z', name: 'spec-293', started_at: STARTED }]

    expect(endWorkflow(ended, 'spec-293', at(2))).toEqual({ workflow: ended[0], workflows: ended })
  })

  it('refuses a workflow this machine never started', () => {
    expect(codeOf(() => endWorkflow([], 'nowhere', at(1)))).toBe('unknown-workflow')
    expect(messageOf(() => endWorkflow([], 'nowhere', at(1)))).toBe('no workflow named nowhere has started on this machine')
  })
})

describe('workflowEndsAt', () => {
  it('is the moment the 24 hours run out while the workflow runs', () => {
    expect(workflowEndsAt({ name: 'w', started_at: STARTED })).toEqual(at(24))
  })

  it('is the moment someone ended it when that came first', () => {
    expect(workflowEndsAt({ ended_at: at(2).toISOString(), name: 'w', started_at: STARTED })).toEqual(at(2))
  })
})
