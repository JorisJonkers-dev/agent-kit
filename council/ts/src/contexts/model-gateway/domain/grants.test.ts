import { describe, expect, it } from 'vitest'

import type { LedgerEntry, LedgerRecord, ReservationRecord, SettledRecord } from './ledger.js'
import {
  GRANT_TTL_MS,
  grantId,
  grantRoom,
  grantViews,
  liveGrant,
  newGrant,
  noGrantMessage,
  parseGrantsFile,
  type Grant,
} from './grants.js'
import type { WorkflowRecord } from './workflows.js'

const NOW = new Date('2026-11-04T10:52:15.000Z')

function grant(overrides: Partial<Grant> = {}): Grant {
  return {
    answer: 'yes, up to five dollars',
    cap_usd: 5,
    client: 'codex',
    expires_at: '2026-11-05T10:52:15.000Z',
    id: 'grant-20261104T105215Z-abc123',
    question: 'may I spend up to $5 of nedap money on spec-293?',
    workflow: 'spec-293',
    written_at: '2026-11-04T10:52:15.000Z',
    ...overrides,
  }
}

function run(at: string, costUsd: number, overrides: Partial<LedgerRecord> = {}): LedgerRecord {
  return {
    at,
    client: 'codex',
    cost_class: 'metered-employer',
    cost_usd: costUsd,
    duration_s: 1,
    family: 'openai',
    mode: 'read-only',
    model: 'gpt-5.5',
    outcome: 'ok',
    repository: null,
    run_id: `run-${at}`,
    tokens: { cached: 0, input: 1, output: 1 },
    workflow: 'spec-293',
    ...overrides,
  }
}

function reservation(overrides: Partial<ReservationRecord> = {}): ReservationRecord {
  return {
    client: 'codex',
    cost_class: 'metered-employer',
    estimate_usd: 0.5,
    expires_at: '2026-11-04T11:08:15.000Z',
    kind: 'reservation',
    model: 'gpt-5.5',
    reserved_at: '2026-11-04T10:52:15.000Z',
    run_dir: '/tmp/run',
    run_id: 'run-reserved',
    workflow: 'spec-293',
    ...overrides,
  }
}

function settled(overrides: Partial<SettledRecord> = {}): SettledRecord {
  return {
    client: 'codex',
    cost_class: 'metered-employer',
    cost_usd: 0.4,
    kind: 'settled',
    reserved_at: '2026-11-04T10:52:15.000Z',
    run_id: 'run-settled',
    settled_at: '2026-11-04T10:53:15.000Z',
    workflow: 'spec-293',
    ...overrides,
  }
}

const RUNNING: WorkflowRecord = { name: 'spec-293', started_at: '2026-11-04T10:00:00.000Z' }

describe('parseGrantsFile', () => {
  it('reads the grants a file holds', () => {
    expect(parseGrantsFile(JSON.stringify({ grants: [grant()], version: 1 }))).toEqual([grant()])
  })

  it('reads a file that holds no grant', () => {
    expect(parseGrantsFile(JSON.stringify({ grants: [], version: 1 }))).toEqual([])
  })

  it('refuses a file that is not JSON, of another version, or whose grants are not grants', () => {
    expect(() => parseGrantsFile('{ not json')).toThrow(/not JSON/)
    expect(() => parseGrantsFile(JSON.stringify({ grants: [], version: 2 }))).toThrow(/version must be 1/)
    expect(() => parseGrantsFile(JSON.stringify({ grants: [7], version: 1 }))).toThrow(/grants\[0\] must be an object/)
    expect(() => parseGrantsFile(JSON.stringify({ grants: [grant({ id: '' })], version: 1 }))).toThrow(
      /grants\[0\]\.id must be a non-empty string/,
    )
    expect(() => parseGrantsFile(JSON.stringify({ grants: [grant({ cap_usd: 0 })], version: 1 }))).toThrow(
      /grants\[0\]\.cap_usd must be a number above 0/,
    )
    expect(() => parseGrantsFile(JSON.stringify({ grants: [grant({ expires_at: 'soon' })], version: 1 }))).toThrow(
      /grants\[0\]\.expires_at must be an ISO-8601 UTC timestamp/,
    )
    expect(() => parseGrantsFile(JSON.stringify({ grants: [grant({ ended_at: 'soon' })], version: 1 }))).toThrow(
      /grants\[0\]\.ended_at must be an ISO-8601 UTC timestamp/,
    )
    expect(() => parseGrantsFile(JSON.stringify({ grants: [grant({ question: '' })], version: 1 }))).toThrow(
      /grants\[0\]\.question must be a non-empty string/,
    )
    expect(() => parseGrantsFile(JSON.stringify({ grants: [grant({ answer: '' })], version: 1 }))).toThrow(
      /grants\[0\]\.answer must be a non-empty string/,
    )
    expect(() => parseGrantsFile(JSON.stringify({ grants: [grant({ workflow: '' })], version: 1 }))).toThrow(
      /grants\[0\]\.workflow must be a non-empty string/,
    )
    expect(() => parseGrantsFile(JSON.stringify({ grants: [grant({ client: '' })], version: 1 }))).toThrow(
      /grants\[0\]\.client must be a non-empty string/,
    )
    expect(() => parseGrantsFile(JSON.stringify({ grants: [grant({ written_at: 'soon' })], version: 1 }))).toThrow(
      /grants\[0\]\.written_at must be an ISO-8601 UTC timestamp/,
    )
    expect(() => parseGrantsFile(JSON.stringify({ grants: 'none', version: 1 }))).toThrow(/grants must be a list/)
    expect(() => parseGrantsFile('[]')).toThrow(/must be a JSON object/)
  })

  it.each([
    '2026-11-04 10:52:15',
    '2026-11-04T10:52:15',
    '2026-11-04T10:52:15+02:00',
    '2026-11-04T10:52:15.000+00:00',
    '2026-11-04',
    'Nov 4 2026 10:52:15 UTC',
    '2026-11-04T10:52Z',
    '2026-02-30T10:52:15Z',
    '2026-11-04T24:00:00Z',
    '2026-13-04T10:52:15Z',
    ' 2026-11-04T10:52:15Z',
  ])('refuses the timestamp %j, which is not a full ISO-8601 UTC one, naming the file field', (stamp) => {
    for (const field of ['expires_at', 'written_at', 'ended_at'] as const) {
      expect(() => parseGrantsFile(JSON.stringify({ grants: [grant({ [field]: stamp })], version: 1 }))).toThrow(
        new RegExp(`grants\\[0\\]\\.${field} must be an ISO-8601 UTC timestamp`),
      )
    }
  })

  it('refuses a timestamp that is not a string, such as epoch milliseconds', () => {
    expect(() => parseGrantsFile(JSON.stringify({ grants: [{ ...grant(), written_at: 1_793_000_000_000 }], version: 1 }))).toThrow(
      /grants\[0\]\.written_at must be an ISO-8601 UTC timestamp/,
    )
  })

  it.each(['2026-11-04T10:52:15Z', '2026-11-04T10:52:15.5Z', '2026-11-04T10:52:15.123Z', '2026-02-28T23:59:59.999Z'])(
    'reads the full ISO-8601 UTC timestamp %j',
    (stamp) => {
      expect(parseGrantsFile(JSON.stringify({ grants: [grant({ written_at: stamp })], version: 1 }))[0]?.written_at).toBe(stamp)
    },
  )
})

describe('newGrant', () => {
  it('ends 24 hours after it was written', () => {
    expect(newGrant({ ...grantInput(), workflowRecord: undefined, written_at: NOW })).toMatchObject({
      expires_at: new Date(NOW.getTime() + GRANT_TTL_MS).toISOString(),
      written_at: NOW.toISOString(),
    })
  })

  it('ends earlier when the workflow’s own 24 hours run out first', () => {
    const late: WorkflowRecord = { name: 'spec-293', started_at: '2026-11-04T10:00:00.000Z' }

    expect(newGrant({ ...grantInput(), workflowRecord: late, written_at: NOW }).expires_at).toBe('2026-11-05T10:00:00.000Z')
  })

  it('keeps the workflow’s own end when it is later than the grant’s 24 hours', () => {
    const early: WorkflowRecord = { name: 'spec-293', started_at: '2026-11-04T11:00:00.000Z' }

    expect(newGrant({ ...grantInput(), workflowRecord: early, written_at: NOW }).expires_at).toBe('2026-11-05T10:52:15.000Z')
  })

  it('refuses a workflow whose 24 hours already ran out, rather than writing a grant that is expired on arrival', () => {
    const ranOut: WorkflowRecord = { name: 'spec-293', started_at: '2026-11-03T10:00:00.000Z' }
    const exactly: WorkflowRecord = { name: 'spec-293', started_at: '2026-11-03T10:52:15.000Z' }

    for (const record of [ranOut, exactly]) {
      expect(() => newGrant({ ...grantInput(), workflowRecord: record, written_at: NOW })).toThrow(
        expect.objectContaining({ code: 'workflow-ended' }) as unknown,
      )
    }
  })

  it('refuses a workflow someone ended', () => {
    const ended: WorkflowRecord = { ended_at: '2026-11-04T10:30:00.000Z', name: 'spec-293', started_at: '2026-11-04T10:00:00.000Z' }

    expect(() => newGrant({ ...grantInput(), workflowRecord: ended, written_at: NOW })).toThrow(
      expect.objectContaining({ code: 'workflow-ended' }) as unknown,
    )
  })

  it('names itself after the moment it was written and a suffix that keeps two grants apart', () => {
    expect(grantId(NOW, 'abc123')).toBe('grant-20261104T105215Z-abc123')
  })
})

function grantInput(): Omit<Grant, 'expires_at' | 'written_at'> & { written_at: Date; workflowRecord: WorkflowRecord | undefined } {
  return {
    answer: 'yes',
    cap_usd: 5,
    client: 'codex',
    id: 'grant-20261104T105215Z-abc123',
    question: 'may I?',
    workflow: 'spec-293',
    workflowRecord: RUNNING,
    written_at: NOW,
  }
}

describe('grantRoom', () => {
  it('counts what the workflow spent since the grant, and what live reservations hold', () => {
    const entries: readonly LedgerEntry[] = [
      run('2026-11-04T10:00:00.000Z', 4),
      run('2026-11-04T10:55:00.000Z', 1.5),
      reservation(),
      reservation({ cost_class: 'metered-personal', run_id: 'other-class' }),
      reservation({ run_id: 'other-workflow', workflow: 'spec-294' }),
    ]

    expect(grantRoom(entries, grant())).toEqual({ grant: grant(), remaining_usd: 3, reserved_usd: 0.5, spent_usd: 1.5 })
  })

  it('counts only the runs and reservations of the client the grant names', () => {
    const entries: readonly LedgerEntry[] = [
      run('2026-11-04T10:55:00.000Z', 1),
      run('2026-11-04T10:56:00.000Z', 2, { client: 'opencode' }),
      reservation(),
      reservation({ client: 'opencode', run_id: 'other-client' }),
    ]

    expect(grantRoom(entries, grant())).toEqual({ grant: grant(), remaining_usd: 3.5, reserved_usd: 0.5, spent_usd: 1 })
  })

  it('counts a settled run that left no line of its own, and never counts a run twice', () => {
    const entries: readonly LedgerEntry[] = [
      run('2026-11-04T10:55:00.000Z', 1),
      settled({ cost_usd: 1 }),
      settled({ cost_usd: 0.25, reason: 'expired', run_id: 'run-died' }),
    ]

    expect(grantRoom(entries, grant())).toMatchObject({ spent_usd: 1.25 })
  })

  it('holds nothing more for a reservation whose run wrote its own line', () => {
    const entries: readonly LedgerEntry[] = [run('2026-11-04T10:55:00.000Z', 1), reservation({ run_id: 'run-2026-11-04T10:55:00.000Z' })]

    expect(grantRoom(entries, grant())).toMatchObject({ reserved_usd: 0, spent_usd: 1 })
  })
})

describe('grantViews', () => {
  it('lists a live grant with what it has left, and no lapsed one', () => {
    const views = grantViews([grant()], [run('2026-11-04T10:55:00.000Z', 1.5)], [RUNNING], NOW)

    expect(views).toEqual({
      grants: [{ ...grant(), remaining_usd: 3.5, reserved_usd: 0, spent_usd: 1.5 }],
      lapsed: [],
    })
  })

  it('lapses a grant whose workflow ended, naming that first', () => {
    const ended: WorkflowRecord = { ...RUNNING, ended_at: '2026-11-04T10:30:00.000Z' }

    expect(grantViews([grant()], [], [ended], NOW).lapsed).toEqual([
      { ...grant(), reason: 'workflow-ended', remaining_usd: 5, reserved_usd: 0, spent_usd: 0 },
    ])
  })

  it('lapses a grant whose workflow ran out its 24 hours', () => {
    const old: WorkflowRecord = { name: 'spec-293', started_at: '2026-11-03T09:00:00.000Z' }

    expect(grantViews([grant()], [], [old], NOW).lapsed[0]?.reason).toBe('workflow-ended')
  })

  it('lapses a grant after 24 hours', () => {
    const later = new Date('2026-11-05T10:52:15.000Z')
    const stillRunning: WorkflowRecord = { name: 'spec-293', started_at: '2026-11-04T11:00:00.000Z' }

    expect(grantViews([grant()], [], [stillRunning], later).lapsed[0]?.reason).toBe('expired')
  })

  it('lapses a grant whose cap is spent', () => {
    expect(grantViews([grant()], [run('2026-11-04T10:55:00.000Z', 5)], [RUNNING], NOW).lapsed[0]?.reason).toBe('cap-spent')
  })

  it('keeps a grant live while its cap is only reserved, and lapses it once settled spend reaches the cap', () => {
    const reserved = grantViews([grant()], [reservation({ estimate_usd: 5 })], [RUNNING], NOW)

    expect(reserved.lapsed).toEqual([])
    expect(reserved.grants[0]?.remaining_usd).toBe(0)
    expect(grantViews([grant()], [run('2026-11-04T10:55:00.000Z', 5)], [RUNNING], NOW).lapsed[0]?.reason).toBe('cap-spent')
  })

  it('lapses a grant someone ended', () => {
    expect(grantViews([grant({ ended_at: '2026-11-04T10:53:00.000Z' })], [], [RUNNING], NOW).lapsed[0]?.reason).toBe(
      'workflow-ended',
    )
  })

  it('lists a grant of a workflow this machine never started as live until its 24 hours run out', () => {
    expect(grantViews([grant()], [], [], NOW).grants).toHaveLength(1)
  })
})

describe('liveGrant', () => {
  it('finds the live grant of a workflow and client, and none when every grant lapsed', () => {
    expect(liveGrant([grant()], 'spec-293', 'codex', [], RUNNING, NOW)).toEqual(grant())
    expect(liveGrant([grant()], 'spec-294', 'codex', [], RUNNING, NOW)).toBeUndefined()
    expect(liveGrant([grant()], 'spec-293', 'codex', [run('2026-11-04T10:55:00.000Z', 5)], RUNNING, NOW)).toBeUndefined()
  })

  it('never lets a grant of one client cover another client of the same workflow', () => {
    expect(liveGrant([grant()], 'spec-293', 'opencode', [], RUNNING, NOW)).toBeUndefined()
  })

  it('takes the last live grant of a workflow, which is the one the owner wrote most recently', () => {
    const second = grant({ cap_usd: 9, id: 'grant-20261104T105315Z-def456' })

    expect(liveGrant([grant(), second], 'spec-293', 'codex', [], RUNNING, NOW)).toEqual(second)
  })
})

describe('liveGrant by written_at', () => {
  it('takes the live grant written last, whatever order the file holds them in', () => {
    const older = grant({ cap_usd: 3, id: 'grant-older', written_at: '2026-11-04T10:40:00.000Z' })
    const newer = grant({ cap_usd: 9, id: 'grant-newer', written_at: '2026-11-04T10:50:00.000Z' })

    expect(liveGrant([newer, older], 'spec-293', 'codex', [], RUNNING, NOW)).toEqual(newer)
    expect(liveGrant([older, newer], 'spec-293', 'codex', [], RUNNING, NOW)).toEqual(newer)
  })

  it('takes the later one in the file when two were written at the same moment', () => {
    const first = grant({ id: 'grant-first' })
    const second = grant({ id: 'grant-second' })

    expect(liveGrant([first, second], 'spec-293', 'codex', [], RUNNING, NOW)).toEqual(second)
  })

  it('ignores a newer grant that lapsed and takes the newest live one', () => {
    const older = grant({ id: 'grant-older', written_at: '2026-11-04T10:40:00.000Z' })
    const lapsed = grant({ expires_at: '2026-11-04T10:00:00.000Z', id: 'grant-lapsed', written_at: '2026-11-04T10:50:00.000Z' })

    expect(liveGrant([older, lapsed], 'spec-293', 'codex', [], RUNNING, NOW)).toEqual(older)
  })
})

describe('noGrantMessage', () => {
  it('quotes the workflow in the command it suggests, so a name with a quote cannot break out of it', () => {
    const message = noGrantMessage("it's", 'codex')

    expect(message).toContain(`"workflow":"it'\\''s"`)
    expect(message).toContain(`--json '{"workflow":"it'\\''s","client":"codex"`)
  })

  it('names the workflow, says no live grant covers it, and names the grant write call that would add one', () => {
    expect(noGrantMessage('spec-293', 'codex')).toBe(
      'no live grant covers workflow spec-293, so codex may not run: ask the owner, then add one with model-gateway grant write --json \'{"workflow":"spec-293","client":"codex","cap_usd":5,"question":"<what you asked the owner>","answer":"<what the owner answered>"}\'',
    )
  })
})
