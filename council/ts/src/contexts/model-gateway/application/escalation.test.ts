import { describe, expect, it } from 'vitest'

import { EscalatedRun, type AttemptedRun } from './escalation.js'
import type { ModelGatewayDependencies } from './ports.js'

function failed(reason: string): AttemptedRun {
  return { client: 'vibe', model: 'glm-5-3', output_path: '/runs/1/output.json', reason, run_id: '1', status: 'failed' }
}

describe('EscalatedRun', () => {
  it('returns a result whose reason is a refusal as it is, after one attempt, without retrying or climbing', async () => {
    const escalated = new EscalatedRun({} as ModelGatewayDependencies)
    const seen: string[] = []
    const input = { client: 'vibe', mode: 'read-only', model: 'glm-5-3', prompt: 'p', time_limit_s: 60, workflow: 'w' } as const

    const result = await escalated.run(input, (attempt) => {
      seen.push(attempt.prompt)
      return Promise.resolve(failed('cap-exceeded'))
    })

    expect(seen).toEqual(['p'])
    expect(result).toEqual({ ...failed('cap-exceeded'), attempts: [{ client: 'vibe', model: 'glm-5-3', reason: 'cap-exceeded', run_id: '1', status: 'failed' }] })
  })
})
