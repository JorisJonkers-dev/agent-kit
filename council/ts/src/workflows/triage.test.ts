import { join } from 'node:path'

import { describe, expect, it } from 'vitest'

import { triageWorkflow, type TriageWorkflowWrite } from './triage.js'

const triage = {
  clarity: 'clear',
  kind: 'feature',
  landscape: 'brownfield',
  parallelism: 'high',
  risk: 'medium',
  size: 'medium',
} as const

describe('triage workflow', () => {
  it('returns the triage gate payload without filesystem coupling', async () => {
    const payload = await triageWorkflow({ triage })

    expect(payload).toMatchObject({
      council_worthy: true,
      input: triage,
      route: 'program',
      topology: 'parallel',
    })
  })

  it('writes triage.json when a run directory and write port are provided', async () => {
    const writes: TriageWorkflowWrite[] = []
    const payload = await triageWorkflow(
      { runDir: '/runs/run-a', signals: ['shared files'], triage },
      {
        writeText(path, text) {
          writes.push({ path, text })
          return Promise.resolve()
        },
      },
    )

    expect(writes).toEqual([
      {
        path: join('/runs/run-a', 'triage.json'),
        text: `${JSON.stringify(payload, null, 2)}\n`,
      },
    ])
    expect(JSON.parse(writes[0]?.text ?? '{}')).toEqual(payload)
  })

  it('does not write when only one side of the write contract is present', async () => {
    const writes: TriageWorkflowWrite[] = []
    const withRunDir = await triageWorkflow({ runDir: '/runs/run-a', triage })
    const withWriter = await triageWorkflow(
      { triage },
      {
        writeText(path, text) {
          writes.push({ path, text })
          return Promise.resolve()
        },
      },
    )

    expect(withRunDir.route).toBe('program')
    expect(withWriter.route).toBe('program')
    expect(writes).toEqual([])
  })
})
