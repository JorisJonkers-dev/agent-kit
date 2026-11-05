import { describe, expect, it } from 'vitest'

import { FakeWorkflows, FixedClock } from '../../../../test/model-gateway/harness.js'
import type { ModelGatewayDependencies } from './ports.js'
import { WorkflowEndCommand, WorkflowListCommand, WorkflowStartCommand } from './workflows.js'

/** The commands under test, over the workflows a test hands them and a clock that stands still. */
function deps(workflows: FakeWorkflows, clock = new FixedClock()): ModelGatewayDependencies {
  return { clock, workflows } as unknown as ModelGatewayDependencies
}

const STARTED = '2026-11-04T10:52:15.000Z'

describe('WorkflowStartCommand', () => {
  it('starts a workflow the first time it is named, and reports it running', async () => {
    const workflows = new FakeWorkflows()

    expect(await new WorkflowStartCommand(deps(workflows)).run({ name: 'spec-293' })).toEqual({
      status: 'ok',
      workflow: { ended_at: null, name: 'spec-293', started_at: STARTED, state: 'running' },
    })
    expect(await workflows.read()).toEqual([{ name: 'spec-293', started_at: STARTED }])
  })

  it('reports the workflow that is already running, without starting it again', async () => {
    const workflows = new FakeWorkflows([{ name: 'spec-293', started_at: '2026-11-04T09:00:00.000Z' }])

    expect((await new WorkflowStartCommand(deps(workflows)).run({ name: 'spec-293' })).workflow.started_at).toBe(
      '2026-11-04T09:00:00.000Z',
    )
  })

  it('refuses a workflow that ended, so a new piece of work needs a new name', async () => {
    const workflows = new FakeWorkflows([
      { ended_at: '2026-11-04T10:00:00.000Z', name: 'spec-293', started_at: '2026-11-04T09:00:00.000Z' },
    ])

    await expect(new WorkflowStartCommand(deps(workflows)).run({ name: 'spec-293' })).rejects.toThrow(
      'workflow spec-293 ended at 2026-11-04T10:00:00.000Z',
    )
  })

  it('refuses input that is not a JSON object', async () => {
    await expect(
      new WorkflowStartCommand(deps(new FakeWorkflows())).run([]),
    ).rejects.toThrow('the input must be a JSON object')
  })

  it('refuses input that names no workflow', async () => {
    await expect(new WorkflowStartCommand(deps(new FakeWorkflows())).run({})).rejects.toThrow(
      'a workflow command must name its workflow',
    )
  })
})

describe('WorkflowEndCommand', () => {
  it('ends a running workflow at this moment', async () => {
    const workflows = new FakeWorkflows([{ name: 'spec-293', started_at: '2026-11-04T09:00:00.000Z' }])

    expect(await new WorkflowEndCommand(deps(workflows)).run({ name: 'spec-293' })).toEqual({
      status: 'ok',
      workflow: { ended_at: STARTED, name: 'spec-293', started_at: '2026-11-04T09:00:00.000Z', state: 'ended' },
    })
  })

  it('keeps the moment a workflow already ended', async () => {
    const workflows = new FakeWorkflows([
      { ended_at: '2026-11-04T10:00:00.000Z', name: 'spec-293', started_at: '2026-11-04T09:00:00.000Z' },
    ])

    expect((await new WorkflowEndCommand(deps(workflows)).run({ name: 'spec-293' })).workflow.ended_at).toBe(
      '2026-11-04T10:00:00.000Z',
    )
  })

  it('refuses a workflow this machine never started', async () => {
    await expect(new WorkflowEndCommand(deps(new FakeWorkflows())).run({ name: 'spec-293' })).rejects.toThrow(
      'no workflow named spec-293',
    )
  })

  it('refuses input that is not a JSON object', async () => {
    await expect(
      new WorkflowEndCommand(deps(new FakeWorkflows())).run('spec-293'),
    ).rejects.toThrow('the input must be a JSON object')
  })
})

describe('WorkflowListCommand', () => {
  it('lists every workflow, with the 24-hour end worked out from the clock', async () => {
    const workflows = new FakeWorkflows([
      { name: 'spec-293', started_at: '2026-11-04T09:00:00.000Z' },
      { name: 'review', started_at: '2026-11-02T09:00:00.000Z' },
    ])

    expect(await new WorkflowListCommand(deps(workflows)).run({})).toEqual({
      status: 'ok',
      workflows: [
        { ended_at: null, name: 'spec-293', started_at: '2026-11-04T09:00:00.000Z', state: 'running' },
        { ended_at: '2026-11-03T09:00:00.000Z', name: 'review', started_at: '2026-11-02T09:00:00.000Z', state: 'ended' },
      ],
    })
  })

  it('lists nothing before the first workflow is started', async () => {
    expect(await new WorkflowListCommand(deps(new FakeWorkflows())).run({})).toEqual({ status: 'ok', workflows: [] })
  })

  it('refuses input that is not a JSON object', async () => {
    await expect(
      new WorkflowListCommand(deps(new FakeWorkflows())).run([]),
    ).rejects.toThrow('the input must be a JSON object')
  })
})
