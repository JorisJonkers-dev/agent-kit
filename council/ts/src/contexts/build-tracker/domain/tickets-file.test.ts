import { describe, expect, it } from 'vitest'

import { BuildTrackerError } from './state.js'
import { parseTicketsFile, stateFromTickets, type TicketsFile } from './tickets-file.js'

/** A source of `lines`, one per line. */
function yaml(...lines: readonly string[]): string {
  return lines.join('\n')
}

const REPOSITORY = { host: 'github.com', owner: 'acme', repo: 'widgets' }

/** A ticket file with the given branch lines and an empty ticket list. */
function fileWithBranches(...branches: readonly string[]): string {
  return yaml('spec: 100', 'branches:', ...branches, 'tickets: []')
}

/** A ticket file with one branch and the given tickets, each a list of `key: value` lines. */
function fileWithTickets(...tickets: readonly (readonly string[])[]): string {
  const lines = tickets.flatMap((fields) =>
    fields.map((field, index) => (index === 0 ? `  - ${field}` : `    ${field}`)),
  )
  return yaml(
    'spec: 100',
    'branches:',
    '  B0:',
    '    issue: 101',
    '    title: Foundation',
    'tickets:',
    ...lines,
  )
}

/** A ticket file with one branch and one ticket, from the ticket's `key: value` lines. */
function fileWithTicket(...fields: readonly string[]): string {
  return fileWithTickets(fields)
}

/** The error the parser refuses `text` with. */
function refusal(text: string): BuildTrackerError {
  try {
    parseTicketsFile(text)
  } catch (error) {
    if (error instanceof BuildTrackerError) return error
    throw error
  }
  throw new Error(`the parser accepted ${JSON.stringify(text)}`)
}

function expectRefused(text: string): void {
  expect(() => parseTicketsFile(text)).toThrow(BuildTrackerError)
  expect(refusal(text).code).toBe('invalid-tickets')
}

const VALID = yaml(
  'spec: 100',
  'labels:',
  '  - build-tracker',
  'branches:',
  '  B0:',
  '    issue: 101',
  '    title: Foundation',
  '    what: the repo and its checks',
  '  B1:',
  '    issue: 102',
  '    title: Widgets',
  '    what: the widget features',
  'tickets:',
  '  - key: T1',
  '    issue: 201',
  '    branch: B0',
  '    title: Set up the repo',
  '    blocked_by: []',
  '    stories: 3',
  '    ac: the repo builds',
  '  - key: T2',
  '    issue: 202',
  '    branch: B1',
  '    title: Add widgets',
  '    blocked_by:',
  '      - T1',
  '    stories: 2',
  '    ac: widgets render',
  '  - key: T3',
  '    issue: 203',
  '    branch: B1',
  '    title: Ship widgets',
  '    blocked_by:',
  '      - T1',
  '      - T2',
  '    stories: 1',
  '    ac: widgets ship',
)

describe('parseTicketsFile', () => {
  it('reads the spec, the branches and the tickets of a ticket file', () => {
    expect(parseTicketsFile(VALID)).toEqual({
      branches: [
        { id: 'B0', issue: 101, title: 'Foundation' },
        { id: 'B1', issue: 102, title: 'Widgets' },
      ],
      spec: 100,
      tickets: [
        { blockedBy: [], branch: 'B0', issue: 201, key: 'T1', title: 'Set up the repo' },
        { blockedBy: ['T1'], branch: 'B1', issue: 202, key: 'T2', title: 'Add widgets' },
        { blockedBy: ['T1', 'T2'], branch: 'B1', issue: 203, key: 'T3', title: 'Ship widgets' },
      ],
    })
  })
})

describe('parseTicketsFile refusals', () => {
  it('refuses a root that is not a mapping', () => {
    expectRefused(yaml('- a', '- b'))
  })

  it('refuses a spec that is missing or not a positive integer', () => {
    expectRefused(
      yaml('branches:', '  B0:', '    issue: 101', '    title: Foundation', 'tickets: []'),
    )
    expectRefused(
      yaml(
        'spec: 0',
        'branches:',
        '  B0:',
        '    issue: 101',
        '    title: Foundation',
        'tickets: []',
      ),
    )
    expectRefused(
      yaml(
        'spec: many',
        'branches:',
        '  B0:',
        '    issue: 101',
        '    title: Foundation',
        'tickets: []',
      ),
    )
  })

  it('refuses branches that are not a mapping', () => {
    expectRefused(yaml('spec: 100', 'branches: []', 'tickets: []'))
  })

  it('refuses a branch that is not a mapping', () => {
    expectRefused(fileWithBranches('  B0: 1'))
  })

  it('refuses a branch without an issue or a title', () => {
    expectRefused(fileWithBranches('  B0:', '    title: Foundation'))
    expectRefused(fileWithBranches('  B0:', '    issue: 101'))
  })

  it('refuses tickets that are not a list', () => {
    expectRefused(
      yaml(
        'spec: 100',
        'branches:',
        '  B0:',
        '    issue: 101',
        '    title: Foundation',
        'tickets: 1',
      ),
    )
  })

  it('refuses a ticket that is not a mapping', () => {
    expectRefused(fileWithTickets(['1']))
  })

  it('refuses a ticket without a key, a title, an issue or a branch', () => {
    expectRefused(fileWithTicket('issue: 201', 'branch: B0', 'title: One', 'blocked_by: []'))
    expectRefused(fileWithTicket('key: T1', 'issue: 201', 'branch: B0', 'blocked_by: []'))
    expectRefused(fileWithTicket('key: T1', 'branch: B0', 'title: One', 'blocked_by: []'))
    expectRefused(fileWithTicket('key: T1', 'issue: 201', 'title: One', 'blocked_by: []'))
  })

  it('refuses blocked_by that is not a list of ticket keys', () => {
    expectRefused(
      fileWithTicket('key: T1', 'issue: 201', 'branch: B0', 'title: One', 'blocked_by: 1'),
    )
    expectRefused(
      fileWithTicket('key: T1', 'issue: 201', 'branch: B0', 'title: One', 'blocked_by:', '  - 1'),
    )
  })

  it('refuses a duplicate key or a duplicate issue', () => {
    expectRefused(
      fileWithTickets(
        ['key: T1', 'issue: 201', 'branch: B0', 'title: One', 'blocked_by: []'],
        ['key: T1', 'issue: 202', 'branch: B0', 'title: Two', 'blocked_by: []'],
      ),
    )
    expectRefused(
      fileWithTickets(
        ['key: T1', 'issue: 201', 'branch: B0', 'title: One', 'blocked_by: []'],
        ['key: T2', 'issue: 201', 'branch: B0', 'title: Two', 'blocked_by: []'],
      ),
    )
  })

  it('refuses a ticket on an unknown branch or blocked by an unknown key', () => {
    expectRefused(
      fileWithTicket('key: T1', 'issue: 201', 'branch: B9', 'title: One', 'blocked_by: []'),
    )
    expectRefused(
      fileWithTicket('key: T1', 'issue: 201', 'branch: B0', 'title: One', 'blocked_by:', '  - T9'),
    )
  })
})

describe('stateFromTickets', () => {
  const FILE = yaml(
    'spec: 100',
    'branches:',
    '  B0:',
    '    issue: 101',
    '    title: Foundation',
    '  B1:',
    '    issue: 102',
    '    title: Widgets',
    'tickets:',
    '  - key: T3',
    '    issue: 203',
    '    branch: B1',
    '    title: Ship widgets (live)',
    '    blocked_by:',
    '      - T1',
    '  - key: T1',
    '    issue: 201',
    '    branch: B1',
    '    title: Set up the repo',
    '    blocked_by: []',
    '  - key: T2',
    '    issue: 202',
    '    branch: B1',
    '    title: Add widgets',
    '    blocked_by:',
    '      - T1',
  )

  it('makes a state of the file and the issue states, sorted and grouped by branch', () => {
    const states = new Map<number, 'open' | 'closed'>([
      [201, 'closed'],
      [202, 'open'],
    ])

    expect(stateFromTickets(parseTicketsFile(FILE), REPOSITORY, 'Widgets build', states)).toEqual({
      groups: [
        { name: 'Foundation', tickets: [] },
        { name: 'Widgets', tickets: [203, 201, 202] },
      ],
      milestones: [],
      spec: { kind: 'github', number: 100, repository: REPOSITORY },
      subtitle: '3 tickets in 2 branches',
      tickets: [
        {
          blockedBy: [],
          external: [],
          file: undefined,
          live: false,
          note: '',
          number: 201,
          status: 'merged',
          title: 'T1 Set up the repo',
        },
        {
          blockedBy: [201],
          external: [],
          file: undefined,
          live: false,
          note: '',
          number: 202,
          status: 'todo',
          title: 'T2 Add widgets',
        },
        {
          blockedBy: [201],
          external: [],
          file: undefined,
          live: true,
          note: '',
          number: 203,
          status: 'todo',
          title: 'T3 Ship widgets (live)',
        },
      ],
      title: 'Widgets build',
    })
  })

  it('maps a blocker the file does not name to issue 0', () => {
    const file: TicketsFile = {
      branches: [{ id: 'B0', issue: 101, title: 'Foundation' }],
      spec: 100,
      tickets: [{ blockedBy: ['T9'], branch: 'B0', issue: 201, key: 'T1', title: 'One' }],
    }

    const states = new Map<number, 'open' | 'closed'>()

    expect(stateFromTickets(file, REPOSITORY, 'Title', states).tickets[0]?.blockedBy).toEqual([0])
  })
})
