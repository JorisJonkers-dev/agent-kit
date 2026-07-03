import { join } from 'node:path'

import { describe, expect, it } from 'vitest'

import { triageWorkflow, type TriageWorkflowWrite } from './triage.js'
import type { TriageInput } from '../contexts/triage/index.js'

const triage = {
  clarity: 'clear',
  kind: 'feature',
  landscape: 'brownfield',
  parallelism: 'high',
  risk: 'medium',
  size: 'medium',
} as const

const dogfoodProfiles = [
  {
    expected: {
      dagShape: 'single-minimal-task',
      route: 'direct',
      topology: 'single',
    },
    triage: {
      size: 'small',
      landscape: 'brownfield',
      kind: 'bugfix',
      risk: 'low',
      clarity: 'clear',
      parallelism: 'none',
    },
  },
  {
    expected: {
      dagShape: 'parallel-program-dag',
      route: 'program',
      topology: 'parallel',
    },
    triage: {
      size: 'large',
      landscape: 'greenfield',
      kind: 'feature',
      risk: 'high',
      clarity: 'needs-questions',
      parallelism: 'high',
    },
  },
  {
    expected: {
      dagShape: 'delta-task-dag',
      route: 'delta',
      topology: 'sequential',
    },
    triage: {
      size: 'medium',
      landscape: 'brownfield',
      kind: 'refactor',
      risk: 'high',
      clarity: 'needs-questions',
      parallelism: 'none',
    },
  },
] as const satisfies readonly {
  readonly expected: {
    readonly dagShape: string
    readonly route: string
    readonly topology: string
  }
  readonly triage: TriageInput
}[]

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

  it.each(dogfoodProfiles)('composes classifier output with lens recommendations for %#', async ({ expected, triage }) => {
    const payload = await triageWorkflow({
      signals: ['shared files', 'verification plan'],
      triage,
    })

    expect(payload).toMatchObject({
      classification: {
        dag_shape: expected.dagShape,
      },
      route: expected.route,
      topology: expected.topology,
    })
    expect(payload.lens_recommendation.recommended_lenses.length).toBeGreaterThan(0)
    expect(payload.lens_recommendation.considered_count).toBeGreaterThanOrEqual(
      payload.lens_recommendation.recommended_lenses.length,
    )
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
