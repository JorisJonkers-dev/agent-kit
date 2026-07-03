import { describe, expect, it } from 'vitest'

import {
  appendGuidance,
  emptyGuidanceState,
  guidanceForTask,
  pendingInbox,
  updateInboxStatus,
} from './index.js'

describe('guidance inbox', () => {
  it('appends inject, watchdog restart, and review fix-loop guidance with global and per-task ordering', () => {
    const first = appendGuidance(emptyGuidanceState(), {
      id: 'g1',
      task_id: 'T1',
      source: 'inject',
      message: 'narrow the implementation',
      created_at: '2026-07-03T08:00:00Z',
    })
    const second = appendGuidance(first.state, {
      id: 'g2',
      task_id: 'T2',
      source: 'watchdog_restart',
      message: 'resume from the last green step',
    })
    const third = appendGuidance(second.state, {
      id: 'g3',
      task_id: 'T1',
      source: 'review_fix_loop',
      message: 'address the review thread',
    })

    expect(first.entry).toEqual({
      id: 'g1',
      task_id: 'T1',
      source: 'inject',
      message: 'narrow the implementation',
      created_at: '2026-07-03T08:00:00Z',
      sequence: 1,
      task_sequence: 1,
    })
    expect(second.inbox_item).toEqual({
      guidance_id: 'g2',
      task_id: 'T2',
      sequence: 2,
      status: 'pending',
    })
    expect(third.state.next_sequence).toBe(4)
    expect(guidanceForTask(third.state, 'T1').map((entry) => entry.id)).toEqual(['g1', 'g3'])
    expect(guidanceForTask(third.state, 'missing')).toEqual([])
    expect(pendingInbox(third.state).map((item) => item.guidance_id)).toEqual(['g1', 'g2', 'g3'])
  })

  it('updates inbox status immutably while preserving sequencing', () => {
    const first = appendGuidance(emptyGuidanceState(), {
      id: 'g1',
      task_id: 'T1',
      source: 'inject',
      message: 'ship the smallest fix',
    })
    const second = appendGuidance(first.state, {
      id: 'g2',
      task_id: 'T1',
      source: 'review_fix_loop',
      message: 'rerun validation',
    })

    const delivered = updateInboxStatus(second.state, 'g1', 'delivered')
    const dismissed = updateInboxStatus(delivered, 'g2', 'dismissed')

    expect(second.state.inbox[0]?.status).toBe('pending')
    expect(pendingInbox(delivered).map((item) => item.guidance_id)).toEqual(['g2'])
    expect(pendingInbox(dismissed)).toEqual([])
  })

  it('rejects duplicate guidance ids and unknown inbox status targets', () => {
    const first = appendGuidance(emptyGuidanceState(), {
      id: 'g1',
      task_id: 'T1',
      source: 'inject',
      message: 'keep ids stable',
    })

    expect(() =>
      appendGuidance(first.state, {
        id: 'g1',
        task_id: 'T2',
        source: 'watchdog_restart',
        message: 'duplicate ids are ambiguous',
      }),
    ).toThrow('guidance id already exists: g1')
    expect(() => updateInboxStatus(first.state, 'missing', 'delivered')).toThrow(
      'guidance inbox item does not exist: missing',
    )
  })
})
