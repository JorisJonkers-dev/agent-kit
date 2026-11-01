import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { harness, specIssue, ticketIssue, type FakeGhLinks, type Harness } from '../../../test/critique/harness.js'
import type { CheckTicketsResult, Refusal } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const AGENT_KIT = 'github.com/jorisjonkers-dev/agent-kit'

const CRITERIA = '## Acceptance criteria\n\n- [ ] One thing works.\n- [ ] Another thing works.\n'

/** A spec issue #167 whose tickets are #170, #171 and #172, linked as `links` says. */
async function github(links: FakeGhLinks, bodies: Readonly<Record<number, string>> = {}): Promise<Harness> {
  const ticket = (number: number): string => bodies[number] ?? `## Parent\n\n#167\n\n${CRITERIA}`
  const h = await harness({
    issues: {
      [`${AGENT_KIT}#167`]: specIssue(),
      [`${AGENT_KIT}#170`]: ticketIssue(170, 'Evidence checks', ticket(170)),
      [`${AGENT_KIT}#171`]: ticketIssue(171, 'Parallel fan-out', ticket(171)),
      [`${AGENT_KIT}#172`]: ticketIssue(172, 'Tripwire', ticket(172)),
    },
    links: { subIssues: { [`${AGENT_KIT}#167`]: [170, 171, 172] }, ...links },
    tracker: 'github',
  })
  open.push(h)
  return h
}

async function check(h: Harness, spec: string): Promise<CheckTicketsResult | Refusal> {
  return h.commands.checkTickets.run({ repo: h.repo, spec })
}

async function checked(h: Harness, spec: string): Promise<CheckTicketsResult> {
  const result = await check(h, spec)
  if (result.status === 'refused') throw new Error(`check-tickets refused: ${result.message}`)
  return result
}

describe('check-tickets with GitHub tickets', () => {
  it('fails on a dependency cycle between sub-issues and lists the tickets in it', async () => {
    const h = await github({
      blockedBy: { [`${AGENT_KIT}#170`]: [`${AGENT_KIT}#172`], [`${AGENT_KIT}#171`]: [`${AGENT_KIT}#170`], [`${AGENT_KIT}#172`]: [`${AGENT_KIT}#171`] },
    })

    const result = await checked(h, '#167')

    expect(result.status).toBe('failed')
    expect(result.cycles).toEqual([['#170', '#172', '#171']])
  })

  it('reports a blocker that is no sub-issue of the spec as dangling, without failing', async () => {
    const h = await github({
      blockedBy: { [`${AGENT_KIT}#171`]: [`${AGENT_KIT}#170`, `${AGENT_KIT}#175`, 'github.com/jorisjonkers-dev/workspace#12'] },
    })

    const result = await checked(h, '#167')

    expect(result.status).toBe('passed')
    expect(result.cycles).toEqual([])
    expect(result.dangling_blockers).toEqual([
      { blocker: '#175', ticket: '#171' },
      { blocker: 'github.com/jorisjonkers-dev/workspace#12', ticket: '#171' },
    ])
  })

  it('lists the spec user stories no ticket cites, without failing', async () => {
    const h = await github({}, { 170: `Delivers user story 1.\n\n${CRITERIA}` })

    const result = await checked(h, '#167')

    expect(result.status).toBe('passed')
    expect(result.coverage).toEqual({ status: 'checked', stories: [1, 3], uncited: [3] })
  })

  it('counts the acceptance criteria of each ticket', async () => {
    const h = await github({}, { 172: '## Acceptance criteria\n\n- [x] Done already.\n- [ ] Still open.\n* [ ] Starred.\n\nNot a criterion.\n' })

    const result = await checked(h, '#167')

    expect(result.tickets).toEqual([
      { blocked_by: [], criteria: 2, id: '#170', reference: `${AGENT_KIT}#170`, stories: [], title: 'Evidence checks' },
      { blocked_by: [], criteria: 2, id: '#171', reference: `${AGENT_KIT}#171`, stories: [], title: 'Parallel fan-out' },
      { blocked_by: [], criteria: 3, id: '#172', reference: `${AGENT_KIT}#172`, stories: [], title: 'Tripwire' },
    ])
  })

  it('reads the sub-issues and each ticket’s native blocked-by links in the spec’s repository', async () => {
    const h = await github({})

    await checked(h, '#167')

    expect(h.gh.linkRequests).toEqual([
      `sub-issues ${AGENT_KIT}#167`,
      `blocked-by ${AGENT_KIT}#170`,
      `blocked-by ${AGENT_KIT}#171`,
      `blocked-by ${AGENT_KIT}#172`,
    ])
  })

  it('refuses in one line when gh cannot read the sub-issues', async () => {
    const h = await github({ subIssues: { [`${AGENT_KIT}#167`]: 'HTTP 404: Not Found (https://api.github.com/repos/x/y/issues/167/sub_issues)' } })

    expect(await check(h, '#167')).toEqual({
      message: `cannot read the tickets of ${AGENT_KIT}#167 through gh: HTTP 404: Not Found (https://api.github.com/repos/x/y/issues/167/sub_issues)`,
      reason: 'tracker-unreachable',
      status: 'refused',
    })
  })

  it('refuses when gh cannot read the spec issue', async () => {
    const h = await github({})

    expect(await check(h, '#9999')).toMatchObject({ reason: 'tracker-unreachable', status: 'refused' })
    expect(h.gh.linkRequests).toEqual([])
  })
})

describe('check-tickets user-story coverage', () => {
  it('reports coverage as not checkable when no ticket cites a user story', async () => {
    const h = await github({})

    expect((await checked(h, '#167')).coverage).toEqual({ reason: 'no ticket cites a user story', status: 'not-checkable' })
  })

  it('reports coverage as not checkable when the spec numbers no user stories', async () => {
    const h = await github({}, { 170: 'Delivers user story 1.' })
    const spec = specIssue({ body: 'Widgets, with no stories.' })
    h.gh.serve(`${AGENT_KIT}#167`, spec)

    expect((await checked(h, '#167')).coverage).toEqual({ reason: 'the spec has no numbered user stories', status: 'not-checkable' })
  })

  it.each([
    ['user stories 1 and 3', [1, 3]],
    ['**User stories:** 1, 3', [1, 3]],
    ['User stories 1–3', [1, 2, 3]],
    ['(user story #3)', [3]],
    ['user stories 1, 2, and 3', [1, 2, 3]],
  ])('reads the citation %s', async (citation, cited) => {
    const h = await github({}, { 170: `Covers ${citation}.` })

    expect((await checked(h, '#167')).tickets[0]?.stories).toEqual(cited)
  })

  it('takes stories from the User Stories section only, at any depth of subheading', async () => {
    const h = await github({}, { 170: 'Delivers user story 2.' })
    h.gh.serve(
      `${AGENT_KIT}#167`,
      specIssue({
        body: '## Problem\n\n1. Not a story.\n\n## User Stories\n\n### Planning\n\n2. As the owner, I plan.\n   1. A nested step.\n\n```\n9. In a code block.\n```\n\n### Building\n\n4. As the owner, I build.\n\n## Implementation Decisions\n\n5. Not a story.\n',
      }),
    )

    expect((await checked(h, '#167')).coverage).toEqual({ status: 'checked', stories: [2, 4], uncited: [4] })
  })
})

/** A local ticket as `/to-tickets` writes it. */
function localTicket(title: string, blockedBy: string, extra = ''): string {
  return `# ${title}\n\n**What to build:** the behaviour.\n\n**Blocked by:** ${blockedBy}\n\n**Status:** ready-for-agent\n\n- [ ] One thing works.\n- [ ] Another thing works.\n${extra}`
}

/** The `.scratch/widgets/` spec with ticket files under `issues/`. */
async function local(tickets: Readonly<Record<string, string>>): Promise<Harness> {
  const h = await harness()
  open.push(h)
  for (const [name, text] of Object.entries(tickets)) {
    await h.writeText(join(h.repo, '.scratch', 'widgets', 'issues', name), text)
  }
  return h
}

const SPEC = '.scratch/widgets/spec.md'

describe('check-tickets with local-markdown tickets', () => {
  it('fails on a dependency cycle between ticket files and lists the tickets in it', async () => {
    const h = await local({
      '01-schema.md': localTicket('01: Schema', 'None (can start immediately)'),
      '02-api.md': localTicket('02: API', '01, 03'),
      '03-ui.md': localTicket('03: UI', '#2'),
    })

    const result = await checked(h, SPEC)

    expect(result.status).toBe('failed')
    expect(result.cycles).toEqual([['02', '03']])
  })

  it('lists every cycle of tickets that share members, each once', async () => {
    const h = await local({
      '01-schema.md': localTicket('01: Schema', '02'),
      '02-api.md': localTicket('02: API', '03, 01'),
      '03-ui.md': localTicket('03: UI', '02'),
    })

    expect((await checked(h, SPEC)).cycles).toEqual([
      ['01', '02'],
      ['02', '03'],
    ])
  })

  it('reports a ticket that blocks itself as a cycle of one', async () => {
    const h = await local({ '01-schema.md': localTicket('01: Schema', '01') })

    expect((await checked(h, SPEC)).cycles).toEqual([['01']])
  })

  it('reports a blocker that names no ticket of the spec as dangling, without failing', async () => {
    const h = await local({
      '01-schema.md': localTicket('01: Schema', 'None'),
      '02-api.md': localTicket('02: API', '01-schema.md, 07, Billing export'),
    })

    const result = await checked(h, SPEC)

    expect(result.status).toBe('passed')
    expect(result.dangling_blockers).toEqual([
      { blocker: '07', ticket: '02' },
      { blocker: 'Billing export', ticket: '02' },
    ])
  })

  it('resolves a blocker named by its ticket title', async () => {
    const h = await local({
      '01-schema.md': localTicket('01: Schema', 'None'),
      '02-api.md': localTicket('02: API', '"schema"'),
    })

    expect((await checked(h, SPEC)).tickets[1]?.blocked_by).toEqual(['01'])
  })

  it('reads blockers listed under a Blocked by heading', async () => {
    const h = await local({
      '01-schema.md': localTicket('01: Schema', 'None'),
      '02-api.md': localTicket('02: API', 'None'),
      '03-ui.md': '# 03: UI\n\n## Blocked by\n\n- 01\n- 02 and 04\n\nThe UI waits for both.\n',
    })

    expect((await checked(h, SPEC)).tickets[2]?.blocked_by).toEqual(['01', '02', '04'])
  })

  it('lists the spec user stories no ticket cites, without failing', async () => {
    const h = await local({ '01-schema.md': localTicket('01: Schema', 'None', '\nCovers user story 3.\n') })

    const result = await checked(h, SPEC)

    expect(result.status).toBe('passed')
    expect(result.coverage).toEqual({ status: 'checked', stories: [1, 3], uncited: [1] })
  })

  it('counts the criteria of a ticket with no criteria heading, leaving out its comments', async () => {
    const h = await local({
      '01-schema.md': localTicket('01: Schema', 'None', '\n## Comments\n\n- [ ] A checkbox in a comment.\n'),
    })

    expect((await checked(h, SPEC)).tickets).toEqual([
      { blocked_by: [], criteria: 2, id: '01', reference: '.scratch/widgets/issues/01-schema.md', stories: [], title: 'Schema' },
    ])
  })

  it('reads only the markdown files of the issues folder, in file-name order', async () => {
    const h = await local({ '02-api.md': '# API\n', '01-schema.md': 'No heading.\n', 'notes.txt': 'not a ticket', 'later.md': '# Later\n' })

    expect((await checked(h, SPEC)).tickets.map(({ id, title }) => ({ id, title }))).toEqual([
      { id: '01', title: '01-schema' },
      { id: '02', title: 'API' },
      { id: 'later', title: 'Later' },
    ])
  })

  it('reports no tickets, and nothing to check, when the spec has no issues folder', async () => {
    const h = await local({})

    expect(await checked(h, SPEC)).toEqual({
      coverage: { reason: 'no ticket cites a user story', status: 'not-checkable' },
      cycles: [],
      dangling_blockers: [],
      spec: { reference: SPEC, tracker: 'local-markdown' },
      status: 'passed',
      tickets: [],
    })
  })

  it('rejects a spec that is not a file in the repository', async () => {
    const h = await local({})

    await expect(check(h, '.scratch/missing/spec.md')).rejects.toThrow(/is not a file in the repository/)
  })
})

describe('check-tickets refusals', () => {
  it('refuses a repository whose remote is denied, before reading anything', async () => {
    const h = await harness({ remotes: [{ name: 'origin', url: 'https://github.com/nedap/widgets.git' }] })
    open.push(h)

    expect(await check(h, SPEC)).toMatchObject({ status: 'refused' })
  })

  it('refuses a tracker the critique cannot read', async () => {
    const h = await harness({ tracker: 'gitlab' })
    open.push(h)

    expect(await check(h, '#167')).toMatchObject({ reason: 'unsupported-tracker', status: 'refused' })
  })

  it('needs no family allowlist entry, since it sends nothing to a critic', async () => {
    const h = await local({ '01-schema.md': localTicket('01: Schema', 'None') })

    expect((await checked(h, SPEC)).tickets).toHaveLength(1)
  })

  it('rejects input without a spec', async () => {
    const h = await local({})

    await expect(h.commands.checkTickets.run({ repo: h.repo })).rejects.toThrow(/check-tickets input/)
  })
})
