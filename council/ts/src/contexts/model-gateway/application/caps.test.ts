import { describe, expect, it } from 'vitest'

import type { GatewayConfig } from '../domain/config.js'
import type { RunInput } from '../domain/run.js'
import { CapGuard, heldCostClass } from './caps.js'
import type { ModelGatewayDependencies } from './ports.js'

const CONFIG = { clients: {}, settings: { output_allowance_tokens: 1000 } } as unknown as GatewayConfig
const INPUT = { client: 'mystery', model: 'm', prompt: 'hi', workflow: 'w' } as unknown as RunInput

describe('heldCostClass', () => {
  it('refuses a client the configuration does not declare, rather than holding it to nothing', () => {
    expect(() => heldCostClass(INPUT, CONFIG)).toThrow(expect.objectContaining({ code: 'unknown-client' }) as unknown)
    expect(() => heldCostClass(INPUT, CONFIG)).toThrow(/mystery/)
  })

  it('refuses it before the guard estimates the run', () => {
    expect(() => new CapGuard({} as ModelGatewayDependencies).estimate(INPUT, undefined, CONFIG)).toThrow(
      expect.objectContaining({ code: 'unknown-client' }) as unknown,
    )
  })
})
