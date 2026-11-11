import { mkdir, mkdtemp, readdir, readFile, realpath, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import type { BuildTrackerIssuesPort, BuildTrackerRepositoryPort, IssueRecord, RepositoryName } from '../contexts/build-tracker/index.js'
import { BUILD_TRACKER_USAGE, defaultBuildTrackerRuntime, runBuildTrackerCli, type BuildTrackerCliResult } from './index.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

const WIDGETS: RepositoryName = { host: 'github.com', owner: 'acme', repo: 'widgets' }
const GADGETS: RepositoryName = { host: 'github.com', owner: 'acme', repo: 'gadgets' }

interface FakeIssue {
  readonly title?: string
  readonly state?: 'open' | 'closed'
}

/** An issue tracker held in memory, keyed by `owner/repo#n`, that counts the issue reads in flight at once. */
class FakeTracker implements BuildTrackerIssuesPort {
  /** Every issue asked for, in the order asked: `owner/repo#n`. */
  readonly asked: string[] = []
  /** The most issue reads in flight at once. */
  peak = 0
  failure: string | undefined
  private inFlight = 0

  constructor(private readonly issues: Record<string, FakeIssue>) {}

  async issue(repository: RepositoryName, number: number): Promise<IssueRecord> {
    const ref = `${repository.owner}/${repository.repo}#${String(number)}`
    this.asked.push(ref)
    this.inFlight += 1
    this.peak = Math.max(this.peak, this.inFlight)
    try {
      // A macrotask, so a read the renderer does not wait for shows up as two in flight.
      await new Promise((resolve) => {
        setTimeout(resolve, 0)
      })
      if (this.failure !== undefined) throw new Error(this.failure)
      const issue = this.issues[ref]
      if (issue === undefined) throw new Error(`no issue ${ref}`)
      return { number, repository, state: issue.state ?? 'open', title: issue.title ?? '' }
    } finally {
      this.inFlight -= 1
    }
  }

  subIssues(): Promise<readonly IssueRecord[]> {
    return Promise.reject(new Error('a ticket file render reads no sub-issues'))
  }

  blockedBy(): Promise<readonly IssueRecord[]> {
    return Promise.reject(new Error('a ticket file render reads no blockers'))
  }
}

/** A git repository the fake knows: its root and the remote it reports. */
class FakeRepositories implements BuildTrackerRepositoryPort {
  constructor(
    private readonly root: string,
    private readonly remote: RepositoryName | undefined,
  ) {}

  locate(dir: string): ReturnType<BuildTrackerRepositoryPort['locate']> {
    if (dir !== this.root && !dir.startsWith(`${this.root}/`)) return Promise.resolve(undefined)
    return Promise.resolve({ mainRoot: this.root, prefix: '', remote: this.remote, root: this.root })
  }
}

/** One ticket of the fixture ticket file. */
interface FixtureTicket {
  readonly key: string
  readonly issue: number
  readonly branch: string
  readonly title: string
  readonly blockedBy: readonly string[]
}

/**
 * The fixture ticket file: spec 10, branches B0 and B1, and the tickets given,
 * each with the fields a ticket file carries beside the ones the renderer reads.
 */
function ticketsYaml(tickets: readonly FixtureTicket[]): string {
  const entries = tickets.map((ticket) =>
    [
      `  - key: ${ticket.key}`,
      `    issue: ${String(ticket.issue)}`,
      `    branch: ${ticket.branch}`,
      `    title: ${ticket.title}`,
      ticket.blockedBy.length === 0
        ? '    blocked_by: []'
        : `    blocked_by:\n${ticket.blockedBy.map((key) => `      - ${key}`).join('\n')}`,
      '    stories:\n      - 1',
      '    supersedes: []',
      '    lanes:\n      - all',
      '    hosts:\n      - linux',
      '    what: text',
      '    ac:\n      - one item',
    ].join('\n'),
  )
  return `spec: 10\nbranches:\n  B0:\n    title: B0 Base\n    issue: 20\n  B1:\n    title: B1 Next\n    issue: 21\ntickets:\n${entries.join('\n')}\n`
}

/** Two tickets on B0, the live check and one more on B1; the live check waits on B0-02. */
const FIXTURE: readonly FixtureTicket[] = [
  { blockedBy: [], branch: 'B0', issue: 11, key: 'B0-01', title: 'Store' },
  { blockedBy: ['B0-01'], branch: 'B0', issue: 12, key: 'B0-02', title: 'List' },
  { blockedBy: ['B0-02'], branch: 'B1', issue: 13, key: 'B1-01', title: 'Live check (live)' },
  { blockedBy: [], branch: 'B1', issue: 14, key: 'B1-02', title: 'Search' },
]

/** The fixture's issue tracker: the spec open, #11 closed, the rest open. */
function widgetIssues(): Record<string, FakeIssue> {
  return {
    'acme/widgets#10': { state: 'open', title: 'Widgets spec' },
    'acme/widgets#11': { state: 'closed', title: 'Store' },
    'acme/widgets#12': { state: 'open', title: 'List' },
    'acme/widgets#13': { state: 'open', title: 'Live check (live)' },
    'acme/widgets#14': { state: 'open', title: 'Search' },
  }
}

/** The issues the fixture's page is rendered from, in the order the renderer reads them. */
const ASKED = ['acme/widgets#10', 'acme/widgets#11', 'acme/widgets#12', 'acme/widgets#13', 'acme/widgets#14']

interface Workspace {
  /** The temp directory holding the repository, the state home and the page. */
  readonly dir: string
  /** The state home, empty unless a command writes a state. */
  readonly home: string
  readonly tickets: string
  readonly out: string
  readonly tracker: FakeTracker
  run(...argv: string[]): Promise<BuildTrackerCliResult>
  page(): Promise<string>
  writeTickets(text: string): Promise<void>
}

async function workspace(options: { issues?: Record<string, FakeIssue>; remote?: RepositoryName | undefined } = {}): Promise<Workspace> {
  const dir = await realpath(await mkdtemp(join(tmpdir(), 'build-tracker-tickets-')))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  const root = join(dir, 'widgets')
  const home = join(dir, 'state')
  await mkdir(root)
  await mkdir(home)
  const tickets = join(root, 'docs', 'specs', 'widgets', 'tickets.yaml')
  const out = join(dir, 'page.html')
  const tracker = new FakeTracker(options.issues ?? widgetIssues())
  const repositories = new FakeRepositories(root, 'remote' in options ? options.remote : WIDGETS)
  const runtime = defaultBuildTrackerRuntime({
    clock: { now: () => new Date('2026-11-03T12:00:00Z') },
    cwd: root,
    env: { XDG_STATE_HOME: home },
    issues: tracker,
    repositories,
  })
  return {
    dir,
    home,
    out,
    page: () => readFile(out, 'utf8'),
    run: (...argv) => runBuildTrackerCli(argv, runtime),
    tickets,
    tracker,
    writeTickets: async (text) => {
      await mkdir(dirname(tickets), { recursive: true })
      await writeFile(tickets, text)
    },
  }
}

/** The status label the page shows for ticket `n`, read from its list item. */
function shownAs(page: string, n: number): string | undefined {
  const item = new RegExp(`<li><span class="pill [a-z]+">([^<]+)</span><a href="[^"]*/issues/${String(n)}">`).exec(page)
  return item?.[1]
}

/** The title each ticket shows, in page order. */
function titles(page: string): (string | undefined)[] {
  return [...page.matchAll(/<span class="t">([^<]+)<\/span>/g)].map((match) => match[1])
}

/** Each section's heading and the tickets it lists, in page order. */
function sections(page: string): [string, number[]][] {
  return [...page.matchAll(/<section><h2>([^<]+) <small>[^<]*<\/small><\/h2>([\s\S]*?)<\/section>/g)].map((match) => [
    String(match[1]),
    [...String(match[2]).matchAll(/\/issues\/(\d+)"/g)].map((number) => Number(number[1])),
  ])
}

/** The Mermaid source as Mermaid reads it: the graph block's text, its HTML entities decoded. */
function graphSource(page: string): string {
  const block = /<pre class="mermaid">([\s\S]*?)<\/pre>/.exec(page)?.[1] ?? ''
  return block.replaceAll('&quot;', '"').replaceAll('&lt;', '<').replaceAll('&gt;', '>').replaceAll('&amp;', '&')
}

describe('build-tracker render --tickets', () => {
  it('writes one page for every ticket of the file, grouped by branch', async () => {
    const w = await workspace()
    await w.writeTickets(ticketsYaml(FIXTURE))

    const result = await w.run('render', '--tickets', w.tickets, '--out', w.out)

    expect(result.stderr).toBe('')
    expect(result.exitCode).toBe(0)
    const page = await w.page()
    expect(page).toContain('<h1>Widgets spec</h1>')
    expect(page).toContain('4 tickets in 2 branches · updated 2026-11-03 12:00 UTC')
    expect(sections(page)).toEqual([
      ['B0 Base', [11, 12]],
      ['B1 Next', [13, 14]],
    ])
    expect(page).toContain('<h2>B0 Base <small>1/2 merged</small></h2>')
    expect(page).toContain('<h2>B1 Next <small>0/2 merged</small></h2>')
    expect(titles(page)).toEqual(['B0-01 Store', 'B0-02 List', 'B1-01 Live check (live)', 'B1-02 Search'])
    for (const number of [11, 12, 13, 14]) {
      expect(page).toContain(`<a href="https://github.com/acme/widgets/issues/${String(number)}">#${String(number)}</a>`)
    }
  })

  it('shows a closed issue as Merged and an open one as Ready or Blocked', async () => {
    const w = await workspace()
    await w.writeTickets(ticketsYaml(FIXTURE))

    const result = await w.run('render', '--tickets', w.tickets, '--out', w.out)

    const page = await w.page()
    expect([11, 12, 13, 14].map((number) => shownAs(page, number))).toEqual(['Merged', 'Ready', 'Blocked', 'Ready'])
    expect(JSON.parse(result.stdout)).toEqual({ counts: { blocked: 1, merged: 1, ready: 2 }, out: w.out, status: 'rendered' })
  })

  it('draws the dependency graph from the ticket file blockers', async () => {
    const w = await workspace()
    await w.writeTickets(ticketsYaml(FIXTURE))

    await w.run('render', '--tickets', w.tickets, '--out', w.out)
    const graph = graphSource(await w.page())

    expect(graph).toContain('t11 --> t12')
    expect(graph).toContain('t12 --> t13')
    expect(graph.match(/-->/g)).toHaveLength(2)
  })

  it('marks the ticket whose title says it is a live check', async () => {
    const w = await workspace()
    await w.writeTickets(ticketsYaml(FIXTURE))

    await w.run('render', '--tickets', w.tickets, '--out', w.out)
    const page = await w.page()

    expect(page).toContain('<span class="t">B1-01 Live check (live)</span> <span class="live">live check</span>')
    expect(page.match(/class="live"/g)).toHaveLength(1)
  })

  it('reads the issues of --repo when it is given, and of the working directory remote when it is not', async () => {
    const named = await workspace({ remote: GADGETS })
    await named.writeTickets(ticketsYaml(FIXTURE))
    const remote = await workspace()
    await remote.writeTickets(ticketsYaml(FIXTURE))

    await named.run('render', '--tickets', named.tickets, '--repo', 'acme/widgets', '--out', named.out)
    await remote.run('render', '--tickets', remote.tickets, '--out', remote.out)

    expect(named.tracker.asked).toEqual(ASKED)
    expect(remote.tracker.asked).toEqual(ASKED)
    expect(await named.page()).toContain('<a href="https://github.com/acme/widgets/issues/11">#11</a>')
  })

  const refusals: [string, RepositoryName | undefined, readonly string[], string][] = [
    [
      'a working directory with no remote and no --repo',
      undefined,
      [],
      'the working directory has no git remote; name the repository with --repo owner/repo',
    ],
    ['a --repo that is no owner/repo', WIDGETS, ['--repo', 'widgets'], '--repo must be owner/repo, not widgets'],
  ]

  it.each(refusals)('refuses %s with invalid-spec', async (_name, remote, extra, message) => {
    const w = await workspace({ remote })
    await w.writeTickets(ticketsYaml(FIXTURE))

    const result = await w.run('render', '--tickets', w.tickets, ...extra, '--out', w.out)

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toEqual({ code: 'invalid-spec', message, status: 'error' })
    await expect(w.page()).rejects.toThrow(/ENOENT/)
  })

  it('refuses a ticket file it cannot read', async () => {
    const w = await workspace()

    const result = await w.run('render', '--tickets', join(w.dir, 'missing.yaml'), '--out', w.out)

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toMatchObject({ code: 'invalid-tickets', status: 'error' })
    expect(result.stderr).toMatch(/^build-tracker render: the ticket file at .*missing\.yaml cannot be read: ENOENT/)
    await expect(w.page()).rejects.toThrow(/ENOENT/)
  })

  it('refuses a ticket file whose ticket is on a branch the file does not define', async () => {
    const w = await workspace()
    await w.writeTickets(ticketsYaml([{ blockedBy: [], branch: 'B9', issue: 11, key: 'B0-01', title: 'Store' }]))

    const result = await w.run('render', '--tickets', w.tickets, '--out', w.out)

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toEqual({
      code: 'invalid-tickets',
      message: 'the ticket file is invalid: ticket B0-01 is on branch B9, which the file does not define',
      status: 'error',
    })
    await expect(w.page()).rejects.toThrow(/ENOENT/)
  })

  it('reports an issue tracker it cannot read', async () => {
    const w = await workspace()
    await w.writeTickets(ticketsYaml(FIXTURE))
    w.tracker.failure = 'HTTP 502'

    const result = await w.run('render', '--tickets', w.tickets, '--out', w.out)

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toEqual({
      code: 'tracker-unreachable',
      message: 'the issue tracker could not be read: HTTP 502',
      status: 'error',
    })
    await expect(w.page()).rejects.toThrow(/ENOENT/)
  })

  it("reads a relative --tickets path against the runtime's working directory", async () => {
    const w = await workspace()
    await w.writeTickets(ticketsYaml(FIXTURE))

    const result = await w.run('render', '--tickets', 'docs/specs/widgets/tickets.yaml', '--out', w.out)

    expect(result.exitCode).toBe(0)
    expect(await w.page()).toContain('<h1>Widgets spec</h1>')
  })

  it('writes no state file and leaves the state home empty', async () => {
    const w = await workspace()
    await w.writeTickets(ticketsYaml(FIXTURE))

    const result = await w.run('render', '--tickets', w.tickets, '--out', w.out)

    expect(result.exitCode).toBe(0)
    await expect(readdir(w.home)).resolves.toEqual([])
    expect((await readdir(w.dir)).sort()).toEqual(['page.html', 'state', 'widgets'])
  })

  it('reads at most eight issues at once, and every ticket of a long file', async () => {
    const tickets: FixtureTicket[] = Array.from({ length: 20 }, (_, index) => ({
      blockedBy: [],
      branch: index < 10 ? 'B0' : 'B1',
      issue: 100 + index,
      key: `${index < 10 ? 'B0' : 'B1'}-${String(index + 1).padStart(2, '0')}`,
      title: `Ticket ${String(index + 1)}`,
    }))
    const issues: Record<string, FakeIssue> = { 'acme/widgets#10': { state: 'open', title: 'Widgets spec' } }
    for (const ticket of tickets) issues[`acme/widgets#${String(ticket.issue)}`] = { state: 'open', title: ticket.title }
    const w = await workspace({ issues })
    await w.writeTickets(ticketsYaml(tickets))

    const result = await w.run('render', '--tickets', w.tickets, '--out', w.out)

    expect(result.exitCode).toBe(0)
    expect(w.tracker.peak).toBeLessThanOrEqual(8)
    expect(w.tracker.peak).toBeGreaterThan(1)
    expect(w.tracker.asked).toHaveLength(21)
    const page = await w.page()
    for (const ticket of tickets) {
      expect(page).toContain(`<a href="https://github.com/acme/widgets/issues/${String(ticket.issue)}">#${String(ticket.issue)}</a>`)
    }
  })
})

describe('build-tracker render --tickets arguments', () => {
  it.each([
    ['both --state and --tickets', ['render', '--state', 'state.json', '--tickets', 'tickets.yaml', '--out', 'page.html']],
    ['neither --state nor --tickets', ['render', '--out', 'page.html']],
    ['--repo with --state', ['render', '--state', 'state.json', '--repo', 'acme/widgets', '--out', 'page.html']],
    ['--repo twice', ['render', '--tickets', 'tickets.yaml', '--repo', 'acme/widgets', '--repo', 'acme/gadgets', '--out', 'page.html']],
  ])('prints its usage and exits 2 for %s', async (_name, argv) => {
    const result = await runBuildTrackerCli(argv, defaultBuildTrackerRuntime())

    expect(result).toEqual({ exitCode: 2, stderr: BUILD_TRACKER_USAGE, stdout: '' })
  })

  it('names --tickets in its usage', async () => {
    const result = await runBuildTrackerCli(['--help'], defaultBuildTrackerRuntime())

    expect(result).toEqual({ exitCode: 0, stderr: '', stdout: BUILD_TRACKER_USAGE })
    expect(BUILD_TRACKER_USAGE).toContain('build-tracker render (--state FILE | --tickets FILE [--repo OWNER/REPO]) --out FILE')
  })
})
