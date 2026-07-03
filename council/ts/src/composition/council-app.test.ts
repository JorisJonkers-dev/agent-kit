import { describe, expect, it } from 'vitest'

import { CouncilApp } from './council-app.js'

describe('CouncilApp.recommend', () => {
  it('returns lens recommendations without touching IO adapters', async () => {
    const app = new CouncilApp({
      readText: () => Promise.reject(new Error('readText should not be called')),
      writeText: () => Promise.reject(new Error('writeText should not be called')),
    })

    const recommendation = await app.recommend({
      profile: {
        kind: 'api',
        risk: 'high',
        signals: ['timeout budget'],
        size: 'medium',
      },
    })

    expect(recommendation.lenses.length).toBeGreaterThan(0)
    expect(recommendation.workerCount).toBe(recommendation.lenses.length)
  })
})
